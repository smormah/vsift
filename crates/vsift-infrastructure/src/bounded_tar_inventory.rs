//! Read-only tar inspection over a previously verified, unactivated artifact.

use std::{
    collections::HashSet,
    error::Error,
    fmt,
    io::{self, Read},
};

use sha2::{Digest, Sha256};
use vsift_domain::ArtifactIntegrity;

use crate::archive_inventory::safe_archive_path;
use crate::{
    ArchiveEntry, ArchiveEntryKind, ArchiveInventoryBounds, ArchiveInventoryError,
    ReviewedArchiveAlias, validate_archive_inventory,
};

/// Hard limit on the uncompressed tar stream, including headers and padding.
pub const MAX_TAR_STREAM_BYTES: u64 = 1_073_741_824;

/// One regular archive file selected by a reviewed source catalogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewedArchiveFile<'a> {
    /// Exact archive path, including its version-root directory.
    pub path: &'a str,
    /// Expected extracted size and SHA-256 of that regular file.
    pub integrity: ArtifactIntegrity,
}

/// A typed reason the tar stream cannot proceed to selected-file extraction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TarInventoryError {
    /// The reviewed stream limit is zero or exceeds the hard policy ceiling.
    InvalidStreamLimit,
    /// Tar headers or entry contents could not be read completely.
    Malformed(io::ErrorKind),
    /// The uncompressed tar stream exceeded its reviewed byte limit.
    StreamTooLarge,
    /// Nonzero content followed the tar end marker.
    TrailingContent,
    /// The complete entry metadata failed the provider-neutral policy.
    Inventory(ArchiveInventoryError),
    /// Selected-file paths are empty, unsafe, duplicate or over the entry cap.
    InvalidSelection,
    /// A selected regular file is absent from the complete archive.
    MissingSelectedFile,
    /// A selected entry has a different path, type, size or SHA-256.
    SelectedFileMismatch,
}

impl fmt::Display for TarInventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidStreamLimit => "tar stream limit is invalid",
            Self::Malformed(_) => "tar stream is unreadable or malformed",
            Self::StreamTooLarge => "tar stream exceeds reviewed limit",
            Self::TrailingContent => "tar stream contains data after its end marker",
            Self::Inventory(_) => "tar entry inventory failed review policy",
            Self::InvalidSelection => "reviewed tar file selection is invalid",
            Self::MissingSelectedFile => "tar archive lacks a reviewed file",
            Self::SelectedFileMismatch => "selected tar file differs from reviewed bytes",
        })
    }
}

impl Error for TarInventoryError {}

/// Reads and validates all raw tar headers without writing archive contents.
///
/// GNU long-name, PAX, sparse, hard-link and special headers fail closed through
/// the inventory policy. The byte limit includes headers, padding and trailing
/// zeros. We use sequential reads and drain every entry so a truncated file
/// cannot be skipped by seeking past the end of the source. The returned index
/// does not authorize extraction: contained staging and whole-artifact
/// provenance must still be checked separately.
///
/// # Errors
///
/// Returns typed stream, parser or inventory rejection before filesystem writes.
pub fn inspect_tar_inventory<Source: Read>(
    source: Source,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
) -> Result<Vec<ArchiveEntry>, TarInventoryError> {
    inspect_tar(source, max_tar_bytes, bounds, reviewed_aliases, &[])
}

/// Validates the full tar inventory and exact bytes of selected regular files.
///
/// The reviewed file list must be nonempty. No output file is created. This
/// does not substitute for whole-artifact verification or safe staging.
///
/// # Errors
///
/// Returns typed selection, parser, stream, metadata or digest rejection.
pub fn inspect_tar_selected_files<Source: Read>(
    source: Source,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    selected_files: &[ReviewedArchiveFile<'_>],
) -> Result<Vec<ArchiveEntry>, TarInventoryError> {
    if selected_files.is_empty() {
        return Err(TarInventoryError::InvalidSelection);
    }
    inspect_tar(
        source,
        max_tar_bytes,
        bounds,
        reviewed_aliases,
        selected_files,
    )
}

fn inspect_tar<Source: Read>(
    source: Source,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    selected_files: &[ReviewedArchiveFile<'_>],
) -> Result<Vec<ArchiveEntry>, TarInventoryError> {
    if max_tar_bytes == 0 || max_tar_bytes > MAX_TAR_STREAM_BYTES {
        return Err(TarInventoryError::InvalidStreamLimit);
    }
    if selected_files.len() > bounds.entries() {
        return Err(TarInventoryError::InvalidSelection);
    }
    let mut selected_paths = HashSet::new();
    for selected in selected_files {
        if !safe_archive_path(selected.path, false)
            || !selected_paths.insert(selected.path.to_ascii_lowercase())
        {
            return Err(TarInventoryError::InvalidSelection);
        }
    }
    let mut found_selected = vec![false; selected_files.len()];
    let mut archive = tar::Archive::new(source.take(max_tar_bytes + 1));
    let mut entries = Vec::new();
    let mut declared_bytes = 0_u64;
    for result in archive
        .entries()
        .map_err(|error| TarInventoryError::Malformed(error.kind()))?
        .raw(true)
    {
        let mut entry = result.map_err(|error| TarInventoryError::Malformed(error.kind()))?;
        if entries.len() >= bounds.entries() {
            return Err(TarInventoryError::Inventory(
                ArchiveInventoryError::TooManyEntries,
            ));
        }
        let metadata = read_entry_metadata(&entry)?;
        declared_bytes =
            declared_bytes
                .checked_add(metadata.bytes)
                .ok_or(TarInventoryError::Inventory(
                    ArchiveInventoryError::TooManyBytes,
                ))?;
        if declared_bytes > bounds.expanded_bytes() {
            return Err(TarInventoryError::Inventory(
                ArchiveInventoryError::TooManyBytes,
            ));
        }
        verify_entry_payload(&mut entry, &metadata, selected_files, &mut found_selected)?;
        entries.push(metadata);
    }
    let mut remaining = archive.into_inner();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = remaining
            .read(&mut buffer)
            .map_err(|error| TarInventoryError::Malformed(error.kind()))?;
        if read == 0 {
            break;
        }
        if remaining.limit() == 0 {
            return Err(TarInventoryError::StreamTooLarge);
        }
        if buffer[..read].iter().any(|byte| *byte != 0) {
            return Err(TarInventoryError::TrailingContent);
        }
    }
    validate_archive_inventory(&entries, reviewed_aliases, bounds)
        .map_err(TarInventoryError::Inventory)?;
    if found_selected.iter().any(|found| !found) {
        return Err(TarInventoryError::MissingSelectedFile);
    }
    Ok(entries)
}

fn read_entry_metadata<Source: Read>(
    entry: &tar::Entry<'_, Source>,
) -> Result<ArchiveEntry, TarInventoryError> {
    if entry.header().path_bytes().contains(&b'\\') {
        return Err(TarInventoryError::Inventory(
            ArchiveInventoryError::UnsafePath,
        ));
    }
    let encoded_path = entry.path_bytes();
    let path = std::str::from_utf8(&encoded_path)
        .map_err(|_| TarInventoryError::Inventory(ArchiveInventoryError::UnsafePath))?
        .to_owned();
    let kind = match entry.header().entry_type() {
        tar::EntryType::Regular => ArchiveEntryKind::Regular,
        tar::EntryType::Directory => ArchiveEntryKind::Directory,
        tar::EntryType::Symlink => {
            let encoded_target = entry.link_name_bytes().ok_or(TarInventoryError::Inventory(
                ArchiveInventoryError::UnreviewedLink,
            ))?;
            let target = std::str::from_utf8(&encoded_target)
                .map_err(|_| TarInventoryError::Inventory(ArchiveInventoryError::UnsafePath))?
                .to_owned();
            ArchiveEntryKind::SymbolicLink { target }
        }
        _ => {
            return Err(TarInventoryError::Inventory(
                ArchiveInventoryError::SpecialEntry,
            ));
        }
    };
    if !safe_archive_path(&path, kind == ArchiveEntryKind::Directory) {
        return Err(TarInventoryError::Inventory(
            ArchiveInventoryError::UnsafePath,
        ));
    }
    let bytes = entry.size();
    if kind != ArchiveEntryKind::Regular && bytes != 0 {
        return Err(TarInventoryError::Inventory(
            ArchiveInventoryError::UnexpectedContent,
        ));
    }
    Ok(ArchiveEntry { path, kind, bytes })
}

fn verify_entry_payload<Source: Read>(
    entry: &mut tar::Entry<'_, Source>,
    metadata: &ArchiveEntry,
    selected_files: &[ReviewedArchiveFile<'_>],
    found_selected: &mut [bool],
) -> Result<(), TarInventoryError> {
    let Some((position, selected)) = selected_files
        .iter()
        .enumerate()
        .find(|(_, selected)| selected.path.eq_ignore_ascii_case(&metadata.path))
    else {
        io::copy(entry, &mut io::sink())
            .map_err(|error| TarInventoryError::Malformed(error.kind()))?;
        return Ok(());
    };
    if selected.path != metadata.path
        || metadata.kind != ArchiveEntryKind::Regular
        || metadata.bytes != selected.integrity.bytes()
    {
        return Err(TarInventoryError::SelectedFileMismatch);
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = match entry.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(TarInventoryError::Malformed(error.kind())),
        };
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let observed: [u8; 32] = digest.finalize().into();
    if observed != selected.integrity.sha256() {
        return Err(TarInventoryError::SelectedFileMismatch);
    }
    found_selected[position] = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use tar::{Builder, EntryType, Header};

    use super::{
        ReviewedArchiveFile, TarInventoryError, inspect_tar_inventory, inspect_tar_selected_files,
    };
    use crate::{ArchiveInventoryBounds, ArchiveInventoryError, ReviewedArchiveAlias};
    use vsift_domain::ArtifactIntegrity;

    fn bounds() -> Result<ArchiveInventoryBounds, ArchiveInventoryError> {
        ArchiveInventoryBounds::new(4, 100)
    }

    fn fixture_tar() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut archive = Builder::new(Vec::new());
        let mut file = Header::new_gnu();
        file.set_path("root/lib.so.1")?;
        file.set_size(3);
        file.set_mode(0o600);
        file.set_cksum();
        archive.append(&file, Cursor::new(b"abc"))?;
        let mut link = Header::new_gnu();
        link.set_path("root/lib.so")?;
        link.set_size(0);
        link.set_mode(0o600);
        link.set_entry_type(EntryType::Symlink);
        link.set_link_name("lib.so.1")?;
        link.set_cksum();
        archive.append(&link, Cursor::new([]))?;
        Ok(archive.into_inner()?)
    }

    fn selected_file() -> Result<ReviewedArchiveFile<'static>, Box<dyn std::error::Error>> {
        Ok(ReviewedArchiveFile {
            path: "root/lib.so.1",
            integrity: ArtifactIntegrity::from_sha256_hex(
                3,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            )?,
        })
    }

    #[test]
    fn verifies_selected_regular_bytes_and_complete_inventory()
    -> Result<(), Box<dyn std::error::Error>> {
        let data = fixture_tar()?;
        let aliases = [ReviewedArchiveAlias {
            path: "root/lib.so",
            target: "lib.so.1",
        }];
        let files = [selected_file()?];
        let entries =
            inspect_tar_selected_files(Cursor::new(&data), 10_000, bounds()?, &aliases, &files)?;
        assert_eq!(entries.len(), 2);
        Ok(())
    }

    #[test]
    fn rejects_changed_missing_and_invalid_selected_files() -> Result<(), Box<dyn std::error::Error>>
    {
        let data = fixture_tar()?;
        let aliases = [ReviewedArchiveAlias {
            path: "root/lib.so",
            target: "lib.so.1",
        }];
        let valid = selected_file()?;
        let wrong_digest = ReviewedArchiveFile {
            path: valid.path,
            integrity: ArtifactIntegrity::from_sha256_hex(
                3,
                "a52d159f262b2c6ddb724a61840befc36eb30c88877a4030b65cbe86298449c9",
            )?,
        };
        assert_eq!(
            inspect_tar_selected_files(
                Cursor::new(&data),
                10_000,
                bounds()?,
                &aliases,
                &[wrong_digest]
            ),
            Err(TarInventoryError::SelectedFileMismatch)
        );
        let missing = ReviewedArchiveFile {
            path: "root/missing",
            integrity: valid.integrity,
        };
        assert_eq!(
            inspect_tar_selected_files(Cursor::new(&data), 10_000, bounds()?, &aliases, &[missing]),
            Err(TarInventoryError::MissingSelectedFile)
        );
        let duplicate = [valid, valid];
        assert_eq!(
            inspect_tar_selected_files(Cursor::new(&data), 10_000, bounds()?, &aliases, &duplicate),
            Err(TarInventoryError::InvalidSelection)
        );
        assert_eq!(
            inspect_tar_selected_files(Cursor::new(&data), 10_000, bounds()?, &aliases, &[]),
            Err(TarInventoryError::InvalidSelection)
        );
        Ok(())
    }

    #[test]
    fn reads_complete_regular_and_reviewed_link_headers() -> Result<(), Box<dyn std::error::Error>>
    {
        let data = fixture_tar()?;
        let entries = inspect_tar_inventory(
            Cursor::new(&data),
            10_000,
            bounds()?,
            &[ReviewedArchiveAlias {
                path: "root/lib.so",
                target: "lib.so.1",
            }],
        )?;
        assert_eq!(entries.len(), 2);
        Ok(())
    }

    #[test]
    fn rejects_truncated_or_appended_content() -> Result<(), Box<dyn std::error::Error>> {
        let data = fixture_tar()?;
        let aliases = [ReviewedArchiveAlias {
            path: "root/lib.so",
            target: "lib.so.1",
        }];
        let truncated = &data[..514];
        assert!(matches!(
            inspect_tar_inventory(Cursor::new(truncated), 10_000, bounds()?, &aliases),
            Err(TarInventoryError::Malformed(_))
        ));
        let mut appended = data;
        appended.extend_from_slice(b"evil");
        assert_eq!(
            inspect_tar_inventory(Cursor::new(appended), 10_000, bounds()?, &aliases),
            Err(TarInventoryError::TrailingContent)
        );
        Ok(())
    }

    #[test]
    fn bounds_entire_tar_stream_including_padding() -> Result<(), Box<dyn std::error::Error>> {
        let data = fixture_tar()?;
        let aliases = [ReviewedArchiveAlias {
            path: "root/lib.so",
            target: "lib.so.1",
        }];
        assert_eq!(
            inspect_tar_inventory(Cursor::new(data), 2_048, bounds()?, &aliases),
            Err(TarInventoryError::StreamTooLarge)
        );
        assert_eq!(
            inspect_tar_inventory(Cursor::new([]), 0, bounds()?, &[]),
            Err(TarInventoryError::InvalidStreamLimit)
        );
        Ok(())
    }

    #[test]
    fn raw_special_header_fails_before_extraction() -> Result<(), Box<dyn std::error::Error>> {
        let mut archive = Builder::new(Vec::new());
        let mut hard_link = Header::new_gnu();
        hard_link.set_path("root/linked")?;
        hard_link.set_size(0);
        hard_link.set_mode(0o600);
        hard_link.set_entry_type(EntryType::Link);
        hard_link.set_link_name("../outside")?;
        hard_link.set_cksum();
        archive.append(&hard_link, Cursor::new([]))?;
        let data = archive.into_inner()?;
        assert_eq!(
            inspect_tar_inventory(Cursor::new(data), 10_000, bounds()?, &[]),
            Err(TarInventoryError::Inventory(
                ArchiveInventoryError::SpecialEntry
            ))
        );
        Ok(())
    }

    #[test]
    fn pax_extension_cannot_hide_an_effective_entry() -> Result<(), Box<dyn std::error::Error>> {
        let mut archive = Builder::new(Vec::new());
        archive.append_pax_extensions([("path", b"root/hidden".as_slice())])?;
        let data = archive.into_inner()?;
        assert_eq!(
            inspect_tar_inventory(Cursor::new(data), 10_000, bounds()?, &[]),
            Err(TarInventoryError::Inventory(
                ArchiveInventoryError::SpecialEntry
            ))
        );
        Ok(())
    }
}
