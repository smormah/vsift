//! A bounded reader for the zip archives of reviewed managed artifacts.
//!
//! Windows publishers ship zips: `FFmpeg`'s Windows build, whisper.cpp's
//! Windows build, ONNX Runtime and Deno (ADR 0032). The archive's own size
//! and SHA-256 are checked before this reader sees a byte, so in production
//! it reads reviewed bytes only. It is still written as a parser of hostile
//! input, because a re-pin, a test fixture or a fuzzer can hand it anything,
//! and it is fuzzed as one (`zip_inventory`).
//!
//! The policy is narrow on purpose, so that every reader of the same bytes
//! sees the same files (a zip can otherwise say different things to readers
//! that start from its local headers and readers that start from its
//! central directory):
//!
//! - **The whole file is accounted for.** The entries are laid end to end
//!   from offset 0: each local header, its data and, when the entry says so,
//!   its data descriptor, with nothing between them, then the central
//!   directory, then an end-of-central-directory record with no comment that
//!   ends the file. No prefix (a self-extractor), no gap, no overlap, no
//!   trailing byte.
//! - **Both headers agree.** Each local header names its entry byte for byte
//!   as the central directory does, with the same flags, method and (or, with
//!   a descriptor, zero) sizes and CRC-32.
//! - **Only what reviewed archives use.** Stored or deflated, unencrypted,
//!   one disk, no Zip64 (its markers and its extra field are refused), no
//!   comments, extra fields that tile their area exactly, and a made-by host
//!   whose attributes are understood (MS-DOS, NTFS, Unix, macOS).
//! - **Regular files and directories only.** A Unix mode of a link, a device
//!   or a FIFO is a special entry; a directory is stored and empty. Paths go
//!   through the provider-neutral inventory policy shared with the tar
//!   readers: printable ASCII, `/` separators, no `\`, `:`, `.` or `..`
//!   segment, no duplicate under ASCII case folding, and the reviewed entry
//!   and expanded-byte bounds.
//!
//! Only the selected files are expanded. Each is counted as it is produced
//! and stopped at its declared size, and is accepted only when its size,
//! CRC-32 and SHA-256 equal the review and the compressed stream used
//! exactly its declared bytes. Nothing is extracted by a name the archive
//! chose beyond the selected file's own last path segment, created new and
//! never following a link; any failure removes what this call created.

use std::{
    collections::HashSet,
    error::Error,
    fmt,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
#[cfg(unix)]
use cap_std::fs::OpenOptionsExt;
use cap_std::fs::{Dir, OpenOptions};
use flate2::{Crc, read::DeflateDecoder};
use sha2::{Digest, Sha256};

use crate::{
    ArchiveEntry, ArchiveEntryKind, ArchiveInventoryBounds, ArchiveInventoryError,
    ReviewedArchiveFile, archive_inventory::safe_archive_path,
    bounded_tar_inventory::safe_staging_name, validate_archive_inventory,
};

/// The largest zip archive the reader accepts: the managed transfer limit.
pub const MAX_ZIP_ARCHIVE_BYTES: u64 = vsift_domain::MAX_MANAGED_ARTIFACT_BYTES;

const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const END_OF_CENTRAL_SIGNATURE: u32 = 0x0605_4b50;
const DATA_DESCRIPTOR_SIGNATURE: u32 = 0x0807_4b50;
const LOCAL_HEADER_BYTES: u64 = 30;
const CENTRAL_HEADER_BYTES: usize = 46;
const END_OF_CENTRAL_BYTES: u64 = 22;
/// The longest entry name read, in bytes; the inventory policy then allows
/// at most 240.
const MAX_NAME_BYTES: u16 = 256;
/// The longest extra field accepted in either header, in bytes.
const MAX_EXTRA_BYTES: u16 = 1024;
/// The extra-field tag of Zip64's sizes, refused wherever it appears: a
/// reader that honours it and one that does not would read different sizes.
const ZIP64_EXTRA_TAG: u16 = 0x0001;
/// General-purpose flags the reader understands: the two deflate option
/// bits, the data-descriptor bit and the UTF-8 name bit. Any other bit
/// (encryption among them) is refused.
const ALLOWED_FLAGS: u16 = 0x0002 | 0x0004 | 0x0008 | 0x0800;
const DATA_DESCRIPTOR_FLAG: u16 = 0x0008;
/// The highest "version needed to extract" accepted: 2.0, deflate. Zip64
/// needs 4.5.
const MAX_VERSION_NEEDED: u16 = 20;
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATED: u16 = 8;
const HOST_MS_DOS: u8 = 0;
const HOST_UNIX: u8 = 3;
const HOST_NTFS: u8 = 10;
const HOST_MACOS: u8 = 19;
const UNIX_TYPE_MASK: u32 = 0o170_000;
const UNIX_REGULAR: u32 = 0o100_000;
const UNIX_DIRECTORY: u32 = 0o040_000;
const MS_DOS_DIRECTORY: u32 = 0x10;

/// A typed reason a zip archive cannot proceed to selected-file staging.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ZipInventoryError {
    /// The archive size is zero or over the reader's hard ceiling.
    InvalidLimits,
    /// The archive is larger than its reviewed size.
    ArchiveTooLarge,
    /// A signature, length or offset is wrong, the headers disagree, or
    /// bytes sit outside the records the archive declares.
    Malformed,
    /// The archive uses something the reader refuses: encryption, Zip64,
    /// several disks, a comment, an unknown flag, host or method.
    Unsupported,
    /// The complete entry index failed the provider-neutral policy.
    Inventory(ArchiveInventoryError),
    /// The reviewed selection is empty, unsafe, duplicated or over the
    /// entry bound.
    InvalidSelection,
    /// A selected regular file is absent from the archive.
    MissingSelectedFile,
    /// A selected entry has a different path, type, size, CRC-32 or
    /// SHA-256, or its compressed stream does not end where it is declared
    /// to.
    SelectedFileMismatch,
    /// The archive could not be read.
    Read(io::ErrorKind),
    /// The staging folder is not empty, or an output name exists already.
    InvalidStaging,
    /// A selected file could not be created, written, flushed or removed.
    StagingIo(io::ErrorKind),
}

impl fmt::Display for ZipInventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLimits => "zip archive limits are invalid",
            Self::ArchiveTooLarge => "zip archive exceeds its reviewed size",
            Self::Malformed => "zip archive is malformed",
            Self::Unsupported => "zip archive uses a refused feature",
            Self::Inventory(_) => "zip entry inventory failed review policy",
            Self::InvalidSelection => "reviewed zip file selection is invalid",
            Self::MissingSelectedFile => "zip archive lacks a reviewed file",
            Self::SelectedFileMismatch => "selected zip file differs from reviewed bytes",
            Self::Read(_) => "zip archive could not be read",
            Self::InvalidStaging => "selected zip staging directory is not empty",
            Self::StagingIo(_) => "selected zip file staging failed",
        })
    }
}

impl Error for ZipInventoryError {}

/// One entry as the central directory declares it, with where its data
/// starts once its local header has been checked.
#[derive(Clone, Debug)]
struct ZipEntry {
    path: String,
    kind: ArchiveEntryKind,
    flags: u16,
    method: u16,
    crc32: u32,
    compressed: u64,
    expanded: u64,
    local_offset: u64,
    data_start: u64,
}

/// Reads and checks the complete zip index of a source of exactly
/// `archive_bytes` bytes without expanding anything.
///
/// # Errors
///
/// The typed reason for the first rule the archive breaks.
pub fn inspect_zip_inventory<Source: Read + Seek>(
    mut source: Source,
    archive_bytes: u64,
    bounds: ArchiveInventoryBounds,
) -> Result<Vec<ArchiveEntry>, ZipInventoryError> {
    let entries = read_index(&mut source, archive_bytes, bounds)?;
    Ok(entries.into_iter().map(public_entry).collect())
}

/// Checks the complete zip index and the exact bytes of every selected
/// regular file, writing nothing.
///
/// # Errors
///
/// The typed reason for the first rule the archive or a selected file
/// breaks.
pub fn inspect_zip_selected_files<Source: Read + Seek>(
    mut source: Source,
    archive_bytes: u64,
    bounds: ArchiveInventoryBounds,
    selected_files: &[ReviewedArchiveFile<'_>],
) -> Result<Vec<ArchiveEntry>, ZipInventoryError> {
    check_selection(selected_files, bounds)?;
    let entries = read_index(&mut source, archive_bytes, bounds)?;
    for (selected, entry) in match_selection(&entries, selected_files)? {
        copy_entry(&mut source, entry, selected, &mut io::sink())?;
    }
    Ok(entries.into_iter().map(public_entry).collect())
}

/// Checks the complete zip index and stages every selected regular file
/// under its last path segment in `staging`, which must be empty.
///
/// Each file is created new, private and without following a link; any
/// failure removes every file this call created. The staged files stay
/// unactivated and still need the runtime review and the smoke.
///
/// # Errors
///
/// The typed reason for the first rule the archive, a selected file or the
/// staging folder breaks.
pub fn stage_zip_selected_files(
    mut source: File,
    archive_bytes: u64,
    bounds: ArchiveInventoryBounds,
    selected_files: &[ReviewedArchiveFile<'_>],
    staging: &Dir,
) -> Result<Vec<ArchiveEntry>, ZipInventoryError> {
    check_selection(selected_files, bounds)?;
    ensure_empty_staging(staging)?;
    let entries = read_index(&mut source, archive_bytes, bounds)?;
    let matched = match_selection(&entries, selected_files)?;
    let mut created: Vec<&str> = Vec::with_capacity(matched.len());
    let result = (|| {
        for (selected, entry) in matched {
            let name = staging_name(selected.path);
            let mut options = OpenOptions::new();
            options
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file =
                staging
                    .open_with(name, &options)
                    .map_err(|error| match error.kind() {
                        io::ErrorKind::AlreadyExists => ZipInventoryError::InvalidStaging,
                        kind => ZipInventoryError::StagingIo(kind),
                    })?;
            created.push(name);
            copy_entry(&mut source, entry, selected, &mut file)?;
            file.sync_all()
                .map_err(|error| ZipInventoryError::StagingIo(error.kind()))?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        remove_created(staging, &created)?;
        return Err(error);
    }
    Ok(entries.into_iter().map(public_entry).collect())
}

fn public_entry(entry: ZipEntry) -> ArchiveEntry {
    ArchiveEntry {
        path: entry.path,
        kind: entry.kind,
        bytes: entry.expanded,
    }
}

fn staging_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn check_selection(
    selected_files: &[ReviewedArchiveFile<'_>],
    bounds: ArchiveInventoryBounds,
) -> Result<(), ZipInventoryError> {
    if selected_files.is_empty() || selected_files.len() > bounds.entries() {
        return Err(ZipInventoryError::InvalidSelection);
    }
    let mut paths = HashSet::new();
    let mut names = HashSet::new();
    for selected in selected_files {
        let name = staging_name(selected.path);
        if !safe_archive_path(selected.path, false)
            || !safe_staging_name(name)
            || !paths.insert(selected.path.to_ascii_lowercase())
            || !names.insert(name.to_ascii_lowercase())
        {
            return Err(ZipInventoryError::InvalidSelection);
        }
    }
    Ok(())
}

/// Pairs each selection with its entry, in archive order, so the archive is
/// read forwards.
fn match_selection<'entries, 'selection>(
    entries: &'entries [ZipEntry],
    selected_files: &'selection [ReviewedArchiveFile<'selection>],
) -> Result<
    Vec<(
        &'selection ReviewedArchiveFile<'selection>,
        &'entries ZipEntry,
    )>,
    ZipInventoryError,
> {
    let mut matched = Vec::with_capacity(selected_files.len());
    for selected in selected_files {
        let Some(entry) = entries
            .iter()
            .find(|entry| entry.path.eq_ignore_ascii_case(selected.path))
        else {
            return Err(ZipInventoryError::MissingSelectedFile);
        };
        if entry.path != selected.path
            || entry.kind != ArchiveEntryKind::Regular
            || entry.expanded != selected.integrity.bytes()
        {
            return Err(ZipInventoryError::SelectedFileMismatch);
        }
        matched.push((selected, entry));
    }
    matched.sort_by_key(|(_, entry)| entry.local_offset);
    Ok(matched)
}

fn ensure_empty_staging(staging: &Dir) -> Result<(), ZipInventoryError> {
    let mut entries = staging
        .read_dir(".")
        .map_err(|error| ZipInventoryError::StagingIo(error.kind()))?;
    match entries.next() {
        Some(Ok(_)) => Err(ZipInventoryError::InvalidStaging),
        Some(Err(error)) => Err(ZipInventoryError::StagingIo(error.kind())),
        None => Ok(()),
    }
}

fn remove_created(staging: &Dir, created: &[&str]) -> Result<(), ZipInventoryError> {
    let mut failure = None;
    for name in created.iter().rev() {
        if let Err(error) = staging.remove_file(name)
            && error.kind() != io::ErrorKind::NotFound
            && failure.is_none()
        {
            failure = Some(error.kind());
        }
    }
    failure.map_or(Ok(()), |kind| Err(ZipInventoryError::StagingIo(kind)))
}

/// Reads the end record, the central directory and every local header, and
/// checks the layout and the inventory policy.
fn read_index<Source: Read + Seek>(
    source: &mut Source,
    archive_bytes: u64,
    bounds: ArchiveInventoryBounds,
) -> Result<Vec<ZipEntry>, ZipInventoryError> {
    if archive_bytes == 0 || archive_bytes > MAX_ZIP_ARCHIVE_BYTES {
        return Err(ZipInventoryError::InvalidLimits);
    }
    let actual_bytes = source
        .seek(SeekFrom::End(0))
        .map_err(|error| ZipInventoryError::Read(error.kind()))?;
    if actual_bytes > archive_bytes {
        return Err(ZipInventoryError::ArchiveTooLarge);
    }
    if actual_bytes != archive_bytes || archive_bytes < END_OF_CENTRAL_BYTES {
        return Err(ZipInventoryError::Malformed);
    }
    let end_start = archive_bytes - END_OF_CENTRAL_BYTES;
    let (count, central_offset, central_size) = read_end_of_central(source, end_start, bounds)?;
    let mut entries = read_central_directory(source, central_offset, central_size, count)?;

    // The entries must tile the bytes before the central directory exactly,
    // in offset order, from offset 0.
    entries.sort_by_key(|entry| entry.local_offset);
    let mut cursor = 0_u64;
    for index in 0..entries.len() {
        if entries[index].local_offset != cursor {
            return Err(ZipInventoryError::Malformed);
        }
        let data_start = read_local_header(source, &entries[index])?;
        let data_end = data_start
            .checked_add(entries[index].compressed)
            .ok_or(ZipInventoryError::Malformed)?;
        let next = entries
            .get(index + 1)
            .map_or(central_offset, |entry| entry.local_offset);
        let records_end = if entries[index].flags & DATA_DESCRIPTOR_FLAG == 0 {
            data_end
        } else {
            read_data_descriptor(source, data_end, next, &entries[index])?
        };
        if records_end != next {
            return Err(ZipInventoryError::Malformed);
        }
        entries[index].data_start = data_start;
        cursor = next;
    }
    if cursor != central_offset {
        return Err(ZipInventoryError::Malformed);
    }
    let index: Vec<ArchiveEntry> = entries.iter().cloned().map(public_entry).collect();
    validate_archive_inventory(&index, &[], bounds).map_err(ZipInventoryError::Inventory)?;
    Ok(entries)
}

/// The end-of-central-directory record, which must end the file with no
/// comment; returns the entry count and the central directory's place.
fn read_end_of_central<Source: Read + Seek>(
    source: &mut Source,
    end_start: u64,
    bounds: ArchiveInventoryBounds,
) -> Result<(usize, u64, u64), ZipInventoryError> {
    let record = read_at::<22, _>(source, end_start)?;
    if le32(&record, 0) != END_OF_CENTRAL_SIGNATURE {
        return Err(ZipInventoryError::Malformed);
    }
    let disk = le16(&record, 4);
    let central_disk = le16(&record, 6);
    let entries_on_disk = le16(&record, 8);
    let entries = le16(&record, 10);
    let central_size = le32(&record, 12);
    let central_offset = le32(&record, 16);
    let comment_length = le16(&record, 20);
    if disk == u16::MAX
        || central_disk == u16::MAX
        || entries_on_disk == u16::MAX
        || entries == u16::MAX
        || central_size == u32::MAX
        || central_offset == u32::MAX
    {
        // The Zip64 markers.
        return Err(ZipInventoryError::Unsupported);
    }
    if disk != 0 || central_disk != 0 || comment_length != 0 || entries_on_disk != entries {
        return Err(ZipInventoryError::Unsupported);
    }
    if entries == 0 {
        return Err(ZipInventoryError::Inventory(ArchiveInventoryError::Empty));
    }
    let count = usize::from(entries);
    if count > bounds.entries() {
        return Err(ZipInventoryError::Inventory(
            ArchiveInventoryError::TooManyEntries,
        ));
    }
    let central_offset = u64::from(central_offset);
    let central_size = u64::from(central_size);
    if central_offset
        .checked_add(central_size)
        .is_none_or(|end| end != end_start)
    {
        return Err(ZipInventoryError::Malformed);
    }
    Ok((count, central_offset, central_size))
}

/// Every central directory record, which together must fill the central
/// directory exactly.
fn read_central_directory<Source: Read + Seek>(
    source: &mut Source,
    central_offset: u64,
    central_size: u64,
    count: usize,
) -> Result<Vec<ZipEntry>, ZipInventoryError> {
    let longest_record = CENTRAL_HEADER_BYTES + usize::from(MAX_NAME_BYTES + MAX_EXTRA_BYTES);
    let size = usize::try_from(central_size).map_err(|_| ZipInventoryError::Malformed)?;
    if size > count.saturating_mul(longest_record) || size < count * CENTRAL_HEADER_BYTES {
        return Err(ZipInventoryError::Malformed);
    }
    let directory = read_bytes(source, central_offset, size)?;
    let mut position = 0_usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let (entry, length) = parse_central_record(directory.get(position..).unwrap_or_default())?;
        entries.push(entry);
        position += length;
    }
    if position != size {
        return Err(ZipInventoryError::Malformed);
    }
    Ok(entries)
}

/// One central directory record at the start of `record`, with its length.
fn parse_central_record(record: &[u8]) -> Result<(ZipEntry, usize), ZipInventoryError> {
    if record.len() < CENTRAL_HEADER_BYTES || le32(record, 0) != CENTRAL_HEADER_SIGNATURE {
        return Err(ZipInventoryError::Malformed);
    }
    let [_, host] = le16(record, 4).to_le_bytes();
    let version_needed = le16(record, 6);
    let flags = le16(record, 8);
    let method = le16(record, 10);
    let crc32 = le32(record, 16);
    let compressed = le32(record, 20);
    let expanded = le32(record, 24);
    let name_length = le16(record, 28);
    let extra_length = le16(record, 30);
    let comment_length = le16(record, 32);
    let start_disk = le16(record, 34);
    let external = le32(record, 38);
    let local_offset = le32(record, 42);
    check_features(version_needed, flags, method)?;
    if compressed == u32::MAX || expanded == u32::MAX || local_offset == u32::MAX {
        return Err(ZipInventoryError::Unsupported);
    }
    if comment_length != 0 || start_disk != 0 {
        return Err(ZipInventoryError::Unsupported);
    }
    if name_length == 0 || name_length > MAX_NAME_BYTES || extra_length > MAX_EXTRA_BYTES {
        return Err(ZipInventoryError::Malformed);
    }
    let name_end = CENTRAL_HEADER_BYTES + usize::from(name_length);
    let length = name_end + usize::from(extra_length);
    let (Some(name), Some(extra)) = (
        record.get(CENTRAL_HEADER_BYTES..name_end),
        record.get(name_end..length),
    ) else {
        return Err(ZipInventoryError::Malformed);
    };
    check_extra(extra)?;
    let path = std::str::from_utf8(name)
        .map_err(|_| ZipInventoryError::Inventory(ArchiveInventoryError::UnsafePath))?
        .to_owned();
    let kind = entry_kind(&path, host, external)?;
    if kind == ArchiveEntryKind::Directory
        && (method != METHOD_STORED || compressed != 0 || expanded != 0)
    {
        return Err(ZipInventoryError::Inventory(
            ArchiveInventoryError::UnexpectedContent,
        ));
    }
    if method == METHOD_STORED && compressed != expanded {
        return Err(ZipInventoryError::Malformed);
    }
    Ok((
        ZipEntry {
            path,
            kind,
            flags,
            method,
            crc32,
            compressed: u64::from(compressed),
            expanded: u64::from(expanded),
            local_offset: u64::from(local_offset),
            data_start: 0,
        },
        length,
    ))
}

/// The entry's kind from its name and its made-by host's attributes.
fn entry_kind(path: &str, host: u8, external: u32) -> Result<ArchiveEntryKind, ZipInventoryError> {
    let directory_name = path.ends_with('/');
    match host {
        HOST_UNIX | HOST_MACOS => {
            let mode = (external >> 16) & UNIX_TYPE_MASK;
            match (mode, directory_name) {
                (0 | UNIX_REGULAR, false) => Ok(ArchiveEntryKind::Regular),
                (0 | UNIX_DIRECTORY, true) => Ok(ArchiveEntryKind::Directory),
                (UNIX_REGULAR | UNIX_DIRECTORY, _) => Err(ZipInventoryError::Malformed),
                _ => Ok(ArchiveEntryKind::Other),
            }
        }
        HOST_MS_DOS | HOST_NTFS => {
            let directory_attribute = external & MS_DOS_DIRECTORY != 0;
            match (directory_attribute, directory_name) {
                (false, false) => Ok(ArchiveEntryKind::Regular),
                (_, true) => Ok(ArchiveEntryKind::Directory),
                (true, false) => Err(ZipInventoryError::Malformed),
            }
        }
        _ => Err(ZipInventoryError::Unsupported),
    }
}

fn check_features(version_needed: u16, flags: u16, method: u16) -> Result<(), ZipInventoryError> {
    if version_needed > MAX_VERSION_NEEDED
        || flags & !ALLOWED_FLAGS != 0
        || !matches!(method, METHOD_STORED | METHOD_DEFLATED)
    {
        return Err(ZipInventoryError::Unsupported);
    }
    Ok(())
}

/// An extra field must be whole blocks (a tag, a length and that many
/// bytes) with no Zip64 block.
fn check_extra(mut extra: &[u8]) -> Result<(), ZipInventoryError> {
    while !extra.is_empty() {
        if extra.len() < 4 {
            return Err(ZipInventoryError::Malformed);
        }
        let tag = le16(extra, 0);
        let length = usize::from(le16(extra, 2));
        if tag == ZIP64_EXTRA_TAG {
            return Err(ZipInventoryError::Unsupported);
        }
        extra = extra
            .get(4 + length..)
            .ok_or(ZipInventoryError::Malformed)?;
    }
    Ok(())
}

/// The local header of `entry`, which must agree with its central record;
/// returns where the entry's data starts.
fn read_local_header<Source: Read + Seek>(
    source: &mut Source,
    entry: &ZipEntry,
) -> Result<u64, ZipInventoryError> {
    let header = read_at::<30, _>(source, entry.local_offset)?;
    if le32(&header, 0) != LOCAL_HEADER_SIGNATURE {
        return Err(ZipInventoryError::Malformed);
    }
    let version_needed = le16(&header, 4);
    let flags = le16(&header, 6);
    let method = le16(&header, 8);
    let crc32 = le32(&header, 14);
    let compressed = le32(&header, 18);
    let expanded = le32(&header, 22);
    let name_length = le16(&header, 26);
    let extra_length = le16(&header, 28);
    check_features(version_needed, flags, method)?;
    if flags != entry.flags
        || method != entry.method
        || usize::from(name_length) != entry.path.len()
        || extra_length > MAX_EXTRA_BYTES
    {
        return Err(ZipInventoryError::Malformed);
    }
    // Without a data descriptor the local header carries the values; with
    // one, a writer that streamed may have left them zero.
    let agrees = |local: u32, central: u64| {
        u64::from(local) == central || (flags & DATA_DESCRIPTOR_FLAG != 0 && local == 0)
    };
    if !agrees(crc32, u64::from(entry.crc32))
        || !agrees(compressed, entry.compressed)
        || !agrees(expanded, entry.expanded)
    {
        return Err(ZipInventoryError::Malformed);
    }
    let name_at = entry.local_offset + LOCAL_HEADER_BYTES;
    let name = read_bytes(source, name_at, usize::from(name_length))?;
    if name != entry.path.as_bytes() {
        return Err(ZipInventoryError::Malformed);
    }
    let extra_at = name_at + u64::from(name_length);
    check_extra(&read_bytes(source, extra_at, usize::from(extra_length))?)?;
    Ok(extra_at + u64::from(extra_length))
}

/// The data descriptor after an entry's data, with or without its
/// signature, which must end exactly at `next`; returns where it ends.
fn read_data_descriptor<Source: Read + Seek>(
    source: &mut Source,
    data_end: u64,
    next: u64,
    entry: &ZipEntry,
) -> Result<u64, ZipInventoryError> {
    let room = next
        .checked_sub(data_end)
        .ok_or(ZipInventoryError::Malformed)?;
    let fields_at = match room {
        16 => {
            let signature = read_at::<4, _>(source, data_end)?;
            if le32(&signature, 0) != DATA_DESCRIPTOR_SIGNATURE {
                return Err(ZipInventoryError::Malformed);
            }
            data_end + 4
        }
        12 => data_end,
        _ => return Err(ZipInventoryError::Malformed),
    };
    let fields = read_at::<12, _>(source, fields_at)?;
    if le32(&fields, 0) != entry.crc32
        || u64::from(le32(&fields, 4)) != entry.compressed
        || u64::from(le32(&fields, 8)) != entry.expanded
    {
        return Err(ZipInventoryError::Malformed);
    }
    Ok(data_end + room)
}

/// Expands one selected entry into `output`, stopping at its declared size,
/// and checks its size, CRC-32, SHA-256 and that the compressed stream used
/// exactly its declared bytes.
fn copy_entry<Source: Read + Seek>(
    source: &mut Source,
    entry: &ZipEntry,
    selected: &ReviewedArchiveFile<'_>,
    output: &mut dyn Write,
) -> Result<(), ZipInventoryError> {
    source
        .seek(SeekFrom::Start(entry.data_start))
        .map_err(|error| ZipInventoryError::Read(error.kind()))?;
    let compressed = source.take(entry.compressed);
    let mut digest = Sha256::new();
    let mut crc = Crc::new();
    let mut written = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut sink = |chunk: &[u8]| -> Result<(), ZipInventoryError> {
        written = u64::try_from(chunk.len())
            .ok()
            .and_then(|length| written.checked_add(length))
            .ok_or(ZipInventoryError::SelectedFileMismatch)?;
        if written > entry.expanded {
            return Err(ZipInventoryError::SelectedFileMismatch);
        }
        digest.update(chunk);
        crc.update(chunk);
        output
            .write_all(chunk)
            .map_err(|error| ZipInventoryError::StagingIo(error.kind()))
    };
    let consumed = if entry.method == METHOD_STORED {
        let mut reader = compressed;
        pump(&mut reader, &mut buffer, &mut sink)?;
        entry.compressed - reader.limit()
    } else {
        let mut decoder = DeflateDecoder::new(compressed);
        pump(&mut decoder, &mut buffer, &mut sink)?;
        decoder.total_in()
    };
    let observed: [u8; 32] = digest.finalize().into();
    if written != entry.expanded
        || consumed != entry.compressed
        || crc.sum() != entry.crc32
        || written != selected.integrity.bytes()
        || observed != selected.integrity.sha256()
    {
        return Err(ZipInventoryError::SelectedFileMismatch);
    }
    Ok(())
}

fn pump(
    reader: &mut dyn Read,
    buffer: &mut [u8],
    sink: &mut dyn FnMut(&[u8]) -> Result<(), ZipInventoryError>,
) -> Result<(), ZipInventoryError> {
    loop {
        let read = match reader.read(buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            // A corrupt deflate stream is an entry that differs from the
            // review; a short archive cannot be, as its length was checked.
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidInput
                        | io::ErrorKind::InvalidData
                        | io::ErrorKind::UnexpectedEof
                ) =>
            {
                return Err(ZipInventoryError::SelectedFileMismatch);
            }
            Err(error) => return Err(ZipInventoryError::Read(error.kind())),
        };
        if read == 0 {
            return Ok(());
        }
        sink(buffer.get(..read).unwrap_or_default())?;
    }
}

fn read_at<const N: usize, Source: Read + Seek>(
    source: &mut Source,
    offset: u64,
) -> Result<[u8; N], ZipInventoryError> {
    let mut bytes = [0_u8; N];
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| ZipInventoryError::Read(error.kind()))?;
    source
        .read_exact(&mut bytes)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => ZipInventoryError::Malformed,
            kind => ZipInventoryError::Read(kind),
        })?;
    Ok(bytes)
}

fn read_bytes<Source: Read + Seek>(
    source: &mut Source,
    offset: u64,
    length: usize,
) -> Result<Vec<u8>, ZipInventoryError> {
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| ZipInventoryError::Read(error.kind()))?;
    let mut bytes = vec![0_u8; length];
    source
        .read_exact(&mut bytes)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => ZipInventoryError::Malformed,
            kind => ZipInventoryError::Read(kind),
        })?;
    Ok(bytes)
}

/// The little-endian `u16` at `at`; zero past the end, which the callers'
/// length checks never reach.
fn le16(bytes: &[u8], at: usize) -> u16 {
    match bytes.get(at..at + 2) {
        Some(&[low, high]) => u16::from_le_bytes([low, high]),
        _ => 0,
    }
}

/// The little-endian `u32` at `at`; zero past the end, which the callers'
/// length checks never reach.
fn le32(bytes: &[u8], at: usize) -> u32 {
    match bytes.get(at..at + 4) {
        Some(&[a, b, c, d]) => u32::from_le_bytes([a, b, c, d]),
        _ => 0,
    }
}

#[cfg(test)]
mod tests;
