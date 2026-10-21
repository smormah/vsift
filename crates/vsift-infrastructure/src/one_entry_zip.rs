//! A bounded reader for a zip archive that holds exactly one regular file.
//!
//! The managed JavaScript runtime (Deno, ADR 0028 section 4) is published as
//! a zip of one executable. The archive's own SHA-256 is checked before this
//! reader sees a byte, so it reads reviewed bytes only; it is still written
//! as a parser of hostile input, because a re-pin, a test fixture or a fuzzer
//! can hand it anything, and it is fuzzed as one (`one_entry_zip`).
//!
//! The policy is narrow on purpose. The whole file must be accounted for:
//! one local header at offset 0, its data, an optional data descriptor, one
//! central directory record, and an end-of-central-directory record with no
//! comment that ends the file. Nothing may sit between them. The entry must
//! be stored or deflated, unencrypted, not Zip64, on disk 0, a regular file
//! (a Unix mode of a link, a directory or a device is refused), and named
//! exactly as reviewed. Its expanded bytes are counted as they are produced
//! and stop at the reviewed size, and the entry is accepted only when its
//! size, CRC-32 and SHA-256 all equal the review and the deflate stream used
//! exactly the declared compressed bytes. Nothing is extracted by a name the
//! archive chose: the caller names the output.

use std::{
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
use vsift_domain::ArtifactIntegrity;

use crate::bounded_tar_inventory::safe_staging_name;

/// The largest zip archive the reader accepts, the managed transfer limit.
pub const MAX_ZIP_ARCHIVE_BYTES: u64 = vsift_domain::MAX_MANAGED_ARTIFACT_BYTES;
/// The largest expanded entry the reader produces.
pub const MAX_ZIP_ENTRY_BYTES: u64 = crate::MAX_ARCHIVE_EXPANDED_BYTES;

const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const END_OF_CENTRAL_SIGNATURE: u32 = 0x0605_4b50;
const DATA_DESCRIPTOR_SIGNATURE: u32 = 0x0807_4b50;
const LOCAL_HEADER_BYTES: u64 = 30;
const CENTRAL_HEADER_BYTES: u64 = 46;
const END_OF_CENTRAL_BYTES: u64 = 22;
/// The longest entry name accepted, in bytes.
const MAX_NAME_BYTES: u16 = 256;
/// The longest extra field accepted in either header, in bytes.
const MAX_EXTRA_BYTES: u16 = 1024;
/// General-purpose flags the reader understands: the two deflate option
/// bits, the data-descriptor bit and the UTF-8 name bit. Any other bit
/// (encryption among them) is refused.
const ALLOWED_FLAGS: u16 = 0x0002 | 0x0004 | 0x0008 | 0x0800;
const DATA_DESCRIPTOR_FLAG: u16 = 0x0008;
/// The highest "version needed to extract" accepted: 2.0, deflate. Zip64
/// needs 4.5 and is refused.
const MAX_VERSION_NEEDED: u16 = 20;
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATED: u16 = 8;
const HOST_UNIX: u8 = 3;
const HOST_MS_DOS: u8 = 0;
const UNIX_TYPE_MASK: u32 = 0o170_000;
const UNIX_REGULAR: u32 = 0o100_000;
const MS_DOS_DIRECTORY: u32 = 0x10;

/// The one file a reviewed zip must hold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewedZipEntry<'a> {
    /// The entry's exact name in the archive.
    pub path: &'a str,
    /// Its expanded size and SHA-256.
    pub integrity: ArtifactIntegrity,
}

/// Why a zip archive cannot become a staged file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OneEntryZipError {
    /// The reviewed limits are zero or over the reader's hard ceilings.
    InvalidLimits,
    /// The archive is larger than its reviewed size.
    ArchiveTooLarge,
    /// A signature, length or offset is wrong, or bytes sit outside the
    /// records the archive declares.
    Malformed,
    /// The archive uses something the reader refuses: encryption, Zip64,
    /// several disks, a comment, an unknown flag or compression method.
    Unsupported,
    /// The archive does not hold exactly one regular file.
    NotOneRegularFile,
    /// The entry's name, size, CRC-32 or SHA-256 differs from the review,
    /// or its compressed stream does not end where it is declared to.
    EntryMismatch,
    /// The archive could not be read.
    Read(io::ErrorKind),
    /// The output name is unsafe, or the output file exists already.
    InvalidStaging,
    /// The output file could not be created, written or removed.
    StagingIo(io::ErrorKind),
}

impl fmt::Display for OneEntryZipError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLimits => "zip reader limits are invalid",
            Self::ArchiveTooLarge => "zip archive exceeds its reviewed size",
            Self::Malformed => "zip archive is malformed",
            Self::Unsupported => "zip archive uses a refused feature",
            Self::NotOneRegularFile => "zip archive does not hold exactly one regular file",
            Self::EntryMismatch => "zip entry differs from the review",
            Self::Read(_) => "zip archive could not be read",
            Self::InvalidStaging => "zip output name or folder is invalid",
            Self::StagingIo(_) => "zip output could not be written",
        })
    }
}

impl Error for OneEntryZipError {}

/// The entry as the central directory declares it.
#[derive(Clone, Copy, Debug)]
struct CentralEntry {
    flags: u16,
    method: u16,
    crc32: u32,
    compressed: u64,
    expanded: u64,
    name_length: u16,
}

/// Reads the one entry of a zip `source` of exactly `archive_bytes` bytes
/// into `output`, checking it against `entry`.
///
/// `max_expanded_bytes` is the reviewed bound on the entry's expanded size;
/// the entry's reviewed size must not exceed it. `output` receives at most
/// the reviewed size; on any error it may hold a prefix, which the caller
/// discards.
///
/// # Errors
///
/// A typed reason for the first rule the archive breaks.
pub fn read_one_entry_zip<Source: Read + Seek>(
    mut source: Source,
    archive_bytes: u64,
    max_expanded_bytes: u64,
    entry: ReviewedZipEntry<'_>,
    output: &mut dyn Write,
) -> Result<(), OneEntryZipError> {
    let expected_bytes = entry.integrity.bytes();
    if archive_bytes == 0
        || archive_bytes > MAX_ZIP_ARCHIVE_BYTES
        || max_expanded_bytes == 0
        || max_expanded_bytes > MAX_ZIP_ENTRY_BYTES
        || expected_bytes > max_expanded_bytes
        || entry.path.is_empty()
        || entry.path.len() > usize::from(MAX_NAME_BYTES)
    {
        return Err(OneEntryZipError::InvalidLimits);
    }
    let actual_bytes = source
        .seek(SeekFrom::End(0))
        .map_err(|error| OneEntryZipError::Read(error.kind()))?;
    if actual_bytes > archive_bytes {
        return Err(OneEntryZipError::ArchiveTooLarge);
    }
    if actual_bytes != archive_bytes
        || archive_bytes < LOCAL_HEADER_BYTES + CENTRAL_HEADER_BYTES + END_OF_CENTRAL_BYTES
    {
        return Err(OneEntryZipError::Malformed);
    }

    let central_offset = read_end_of_central(&mut source, archive_bytes)?;
    let central = read_central_entry(&mut source, central_offset, archive_bytes, entry)?;
    if central.expanded != expected_bytes {
        return Err(OneEntryZipError::EntryMismatch);
    }
    let data_start = read_local_header(&mut source, &central, entry)?;
    let data_end = data_start
        .checked_add(central.compressed)
        .ok_or(OneEntryZipError::Malformed)?;
    let records_end = if central.flags & DATA_DESCRIPTOR_FLAG == 0 {
        data_end
    } else {
        read_data_descriptor(&mut source, data_end, central_offset, &central)?
    };
    if records_end != central_offset {
        return Err(OneEntryZipError::Malformed);
    }
    copy_entry(&mut source, data_start, &central, entry.integrity, output)
}

/// Checks the archive with [`read_one_entry_zip`] and writes its entry,
/// created new and private, as `staging_name` in `staging`.
///
/// A failure removes the file this call created.
///
/// # Errors
///
/// As [`read_one_entry_zip`], plus an unsafe or existing output name and
/// output I/O failures.
pub fn stage_one_entry_zip(
    source: File,
    archive_bytes: u64,
    max_expanded_bytes: u64,
    entry: ReviewedZipEntry<'_>,
    staging: &Dir,
    staging_name: &str,
) -> Result<(), OneEntryZipError> {
    if !safe_output_name(staging_name) {
        return Err(OneEntryZipError::InvalidStaging);
    }
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file =
        staging
            .open_with(staging_name, &options)
            .map_err(|error| match error.kind() {
                io::ErrorKind::AlreadyExists => OneEntryZipError::InvalidStaging,
                kind => OneEntryZipError::StagingIo(kind),
            })?;
    let written = read_one_entry_zip(source, archive_bytes, max_expanded_bytes, entry, &mut file)
        .and_then(|()| {
            file.sync_all()
                .map_err(|error| OneEntryZipError::StagingIo(error.kind()))
        });
    drop(file);
    if let Err(error) = written {
        return match staging.remove_file(staging_name) {
            Ok(()) => Err(error),
            Err(removal) if removal.kind() == io::ErrorKind::NotFound => Err(error),
            Err(removal) => Err(OneEntryZipError::StagingIo(removal.kind())),
        };
    }
    Ok(())
}

fn safe_output_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        && safe_staging_name(name)
}

/// The end-of-central-directory record, which must end the file with no
/// comment; returns the central directory's offset.
fn read_end_of_central<Source: Read + Seek>(
    source: &mut Source,
    archive_bytes: u64,
) -> Result<u64, OneEntryZipError> {
    let start = archive_bytes - END_OF_CENTRAL_BYTES;
    let record = read_at::<22, _>(source, start)?;
    if le32(&record, 0) != END_OF_CENTRAL_SIGNATURE {
        return Err(OneEntryZipError::Malformed);
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
        return Err(OneEntryZipError::Unsupported);
    }
    if disk != 0 || central_disk != 0 || comment_length != 0 {
        return Err(OneEntryZipError::Unsupported);
    }
    if entries != 1 || entries_on_disk != 1 {
        return Err(OneEntryZipError::NotOneRegularFile);
    }
    let central_offset = u64::from(central_offset);
    if central_offset
        .checked_add(u64::from(central_size))
        .is_none_or(|end| end != start)
    {
        return Err(OneEntryZipError::Malformed);
    }
    Ok(central_offset)
}

/// The one central directory record, which must fill the central
/// directory exactly and describe the reviewed regular file at offset 0.
fn read_central_entry<Source: Read + Seek>(
    source: &mut Source,
    central_offset: u64,
    archive_bytes: u64,
    entry: ReviewedZipEntry<'_>,
) -> Result<CentralEntry, OneEntryZipError> {
    let record = read_at::<46, _>(source, central_offset)?;
    if le32(&record, 0) != CENTRAL_HEADER_SIGNATURE {
        return Err(OneEntryZipError::Malformed);
    }
    let [_, host] = le16(&record, 4).to_le_bytes();
    let version_needed = le16(&record, 6);
    let flags = le16(&record, 8);
    let method = le16(&record, 10);
    let crc32 = le32(&record, 16);
    let compressed = le32(&record, 20);
    let expanded = le32(&record, 24);
    let name_length = le16(&record, 28);
    let extra_length = le16(&record, 30);
    let comment_length = le16(&record, 32);
    let start_disk = le16(&record, 34);
    let external = le32(&record, 38);
    let local_offset = le32(&record, 42);
    check_features(version_needed, flags, method)?;
    if compressed == u32::MAX || expanded == u32::MAX || local_offset == u32::MAX {
        return Err(OneEntryZipError::Unsupported);
    }
    if comment_length != 0 || start_disk != 0 {
        return Err(OneEntryZipError::Unsupported);
    }
    if name_length == 0 || name_length > MAX_NAME_BYTES || extra_length > MAX_EXTRA_BYTES {
        return Err(OneEntryZipError::Malformed);
    }
    let record_end =
        central_offset + CENTRAL_HEADER_BYTES + u64::from(name_length) + u64::from(extra_length);
    if record_end != archive_bytes - END_OF_CENTRAL_BYTES || local_offset != 0 {
        return Err(OneEntryZipError::Malformed);
    }
    let is_regular = match host {
        HOST_UNIX => {
            let mode = external >> 16;
            mode & UNIX_TYPE_MASK == UNIX_REGULAR || mode & UNIX_TYPE_MASK == 0
        }
        HOST_MS_DOS => external & MS_DOS_DIRECTORY == 0,
        _ => false,
    };
    let name = read_bytes(
        source,
        central_offset + CENTRAL_HEADER_BYTES,
        usize::from(name_length),
    )?;
    if !is_regular || name.ends_with(b"/") {
        return Err(OneEntryZipError::NotOneRegularFile);
    }
    if name != entry.path.as_bytes() {
        return Err(OneEntryZipError::EntryMismatch);
    }
    if method == METHOD_STORED && compressed != expanded {
        return Err(OneEntryZipError::Malformed);
    }
    Ok(CentralEntry {
        flags,
        method,
        crc32,
        compressed: u64::from(compressed),
        expanded: u64::from(expanded),
        name_length,
    })
}

fn check_features(version_needed: u16, flags: u16, method: u16) -> Result<(), OneEntryZipError> {
    if version_needed > MAX_VERSION_NEEDED
        || flags & !ALLOWED_FLAGS != 0
        || !matches!(method, METHOD_STORED | METHOD_DEFLATED)
    {
        return Err(OneEntryZipError::Unsupported);
    }
    Ok(())
}

/// The local header at offset 0, which must agree with the central record;
/// returns where the entry's data starts.
fn read_local_header<Source: Read + Seek>(
    source: &mut Source,
    central: &CentralEntry,
    entry: ReviewedZipEntry<'_>,
) -> Result<u64, OneEntryZipError> {
    let header = read_at::<30, _>(source, 0)?;
    if le32(&header, 0) != LOCAL_HEADER_SIGNATURE {
        return Err(OneEntryZipError::Malformed);
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
    if flags != central.flags || method != central.method || name_length != central.name_length {
        return Err(OneEntryZipError::Malformed);
    }
    if extra_length > MAX_EXTRA_BYTES {
        return Err(OneEntryZipError::Malformed);
    }
    // Without a data descriptor the local header carries the values; with
    // one, a writer that streamed may have left them zero.
    let agrees = |local: u32, central: u64| {
        u64::from(local) == central || (flags & DATA_DESCRIPTOR_FLAG != 0 && local == 0)
    };
    if !agrees(crc32, u64::from(central.crc32))
        || !agrees(compressed, central.compressed)
        || !agrees(expanded, central.expanded)
    {
        return Err(OneEntryZipError::Malformed);
    }
    let name = read_bytes(source, LOCAL_HEADER_BYTES, usize::from(name_length))?;
    if name != entry.path.as_bytes() {
        return Err(OneEntryZipError::EntryMismatch);
    }
    Ok(LOCAL_HEADER_BYTES + u64::from(name_length) + u64::from(extra_length))
}

/// The data descriptor after the entry's data, with or without its
/// signature; returns where it ends.
fn read_data_descriptor<Source: Read + Seek>(
    source: &mut Source,
    data_end: u64,
    central_offset: u64,
    central: &CentralEntry,
) -> Result<u64, OneEntryZipError> {
    let room = central_offset
        .checked_sub(data_end)
        .ok_or(OneEntryZipError::Malformed)?;
    let (fields_at, length) = match room {
        16 => {
            let signature = read_at::<4, _>(source, data_end)?;
            if le32(&signature, 0) != DATA_DESCRIPTOR_SIGNATURE {
                return Err(OneEntryZipError::Malformed);
            }
            (data_end + 4, 16)
        }
        12 => (data_end, 12),
        _ => return Err(OneEntryZipError::Malformed),
    };
    let fields = read_at::<12, _>(source, fields_at)?;
    if le32(&fields, 0) != central.crc32
        || u64::from(le32(&fields, 4)) != central.compressed
        || u64::from(le32(&fields, 8)) != central.expanded
    {
        return Err(OneEntryZipError::Malformed);
    }
    Ok(data_end + length)
}

/// Expands the entry into `output`, stopping at the declared size, and
/// checks its size, CRC-32, SHA-256 and that the compressed stream used
/// exactly its declared bytes.
fn copy_entry<Source: Read + Seek>(
    source: &mut Source,
    data_start: u64,
    central: &CentralEntry,
    integrity: ArtifactIntegrity,
    output: &mut dyn Write,
) -> Result<(), OneEntryZipError> {
    source
        .seek(SeekFrom::Start(data_start))
        .map_err(|error| OneEntryZipError::Read(error.kind()))?;
    let compressed = source.take(central.compressed);
    let mut digest = Sha256::new();
    let mut crc = Crc::new();
    let mut written = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut sink = |chunk: &[u8]| -> Result<(), OneEntryZipError> {
        written = written
            .checked_add(u64::try_from(chunk.len()).map_err(|_| OneEntryZipError::EntryMismatch)?)
            .ok_or(OneEntryZipError::EntryMismatch)?;
        if written > central.expanded {
            return Err(OneEntryZipError::EntryMismatch);
        }
        digest.update(chunk);
        crc.update(chunk);
        output
            .write_all(chunk)
            .map_err(|error| OneEntryZipError::StagingIo(error.kind()))
    };
    let consumed = if central.method == METHOD_STORED {
        let mut reader = compressed;
        pump(&mut reader, &mut buffer, &mut sink)?;
        central.compressed - reader.limit()
    } else {
        let mut decoder = DeflateDecoder::new(compressed);
        pump(&mut decoder, &mut buffer, &mut sink)?;
        decoder.total_in()
    };
    let observed: [u8; 32] = digest.finalize().into();
    if written != central.expanded
        || consumed != central.compressed
        || crc.sum() != central.crc32
        || written != integrity.bytes()
        || observed != integrity.sha256()
    {
        return Err(OneEntryZipError::EntryMismatch);
    }
    Ok(())
}

fn pump(
    reader: &mut dyn Read,
    buffer: &mut [u8],
    sink: &mut dyn FnMut(&[u8]) -> Result<(), OneEntryZipError>,
) -> Result<(), OneEntryZipError> {
    loop {
        let read = match reader.read(buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            // A corrupt deflate stream is an entry that differs from the
            // review; a short archive cannot be, as its length was checked.
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
                return Err(OneEntryZipError::EntryMismatch);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
                ) =>
            {
                return Err(OneEntryZipError::EntryMismatch);
            }
            Err(error) => return Err(OneEntryZipError::Read(error.kind())),
        };
        if read == 0 {
            return Ok(());
        }
        sink(&buffer[..read])?;
    }
}

fn read_at<const N: usize, Source: Read + Seek>(
    source: &mut Source,
    offset: u64,
) -> Result<[u8; N], OneEntryZipError> {
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| OneEntryZipError::Read(error.kind()))?;
    let mut bytes = [0_u8; N];
    source
        .read_exact(&mut bytes)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => OneEntryZipError::Malformed,
            kind => OneEntryZipError::Read(kind),
        })?;
    Ok(bytes)
}

fn read_bytes<Source: Read + Seek>(
    source: &mut Source,
    offset: u64,
    length: usize,
) -> Result<Vec<u8>, OneEntryZipError> {
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| OneEntryZipError::Read(error.kind()))?;
    let mut bytes = vec![0_u8; length];
    source
        .read_exact(&mut bytes)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => OneEntryZipError::Malformed,
            kind => OneEntryZipError::Read(kind),
        })?;
    Ok(bytes)
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[cfg(test)]
mod tests;
