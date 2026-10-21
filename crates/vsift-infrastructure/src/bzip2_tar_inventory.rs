//! Read-only bzip2 decoding before bounded tar inventory validation.
//!
//! sherpa-onnx publishes every prebuilt archive, Windows included, as
//! `.tar.bz2` (ADR 0031, ADR 0032). The decoder is the `bzip2` crate over
//! `libbz2-rs-sys`, a Rust port of libbzip2 with no C build. Exactly one
//! bzip2 stream is reviewed: a second stream, or any byte after the first,
//! is refused rather than silently extending the tar stream. The tar reader
//! then applies the same inventory, link and selection policy as for the
//! gzip and XZ archives.

use std::{
    error::Error,
    fmt,
    io::{BufRead, BufReader, Read},
};

use bzip2::bufread::BzDecoder;
use cap_std::fs::Dir;

use crate::bounded_tar_inventory::cleanup_staged_files;
use crate::{
    ArchiveEntry, ArchiveInventoryBounds, ReviewedArchiveAlias, ReviewedArchiveFile,
    TarInventoryError, inspect_tar_inventory, inspect_tar_selected_files, stage_tar_selected_files,
};

/// Hard limit on compressed bzip2 bytes read from a verified artifact.
pub const MAX_BZIP2_ARCHIVE_BYTES: u64 = 268_435_456;

/// A typed reason a bzip2-compressed tar archive cannot proceed to
/// extraction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bzip2TarInventoryError {
    /// The reviewed compressed-input limit is zero or exceeds the hard ceiling.
    InvalidCompressedLimit,
    /// The compressed input exceeds its reviewed byte limit.
    CompressedInputTooLarge,
    /// Bytes remain after the single reviewed bzip2 stream.
    TrailingCompressedContent,
    /// bzip2 decoding or the expanded tar inventory failed.
    Tar(TarInventoryError),
}

impl fmt::Display for Bzip2TarInventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidCompressedLimit => "bzip2 input limit is invalid",
            Self::CompressedInputTooLarge => "bzip2 input exceeds reviewed limit",
            Self::TrailingCompressedContent => "bzip2 input follows its reviewed stream",
            Self::Tar(_) => "bzip2 or tar inventory failed review policy",
        })
    }
}

impl Error for Bzip2TarInventoryError {}

/// Decodes the one bzip2 stream and validates the complete expanded tar
/// inventory, writing nothing.
///
/// # Errors
///
/// Returns typed compressed-size, decoder or tar-inventory rejection.
pub fn inspect_bzip2_tar_inventory<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
) -> Result<Vec<ArchiveEntry>, Bzip2TarInventoryError> {
    inspect_bzip2_tar(
        source,
        max_compressed_bytes,
        max_tar_bytes,
        bounds,
        reviewed_aliases,
        None,
        None,
    )
}

/// Validates a bzip2/tar inventory and the exact bytes of the selected
/// regular files, writing nothing.
///
/// # Errors
///
/// Returns typed compressed-size, decoder, selection or tar rejection.
pub fn inspect_bzip2_tar_selected_files<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    selected_files: &[ReviewedArchiveFile<'_>],
) -> Result<Vec<ArchiveEntry>, Bzip2TarInventoryError> {
    inspect_bzip2_tar(
        source,
        max_compressed_bytes,
        max_tar_bytes,
        bounds,
        reviewed_aliases,
        Some(selected_files),
        None,
    )
}

/// Validates a bzip2/tar archive and stages the reviewed regular files
/// privately; any failure removes what this call staged.
///
/// # Errors
///
/// Returns typed compressed-size, decoder, selection, tar or staging
/// rejection.
pub fn stage_bzip2_tar_selected_files<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    selected_files: &[ReviewedArchiveFile<'_>],
    staging: &Dir,
) -> Result<Vec<ArchiveEntry>, Bzip2TarInventoryError> {
    inspect_bzip2_tar(
        source,
        max_compressed_bytes,
        max_tar_bytes,
        bounds,
        reviewed_aliases,
        Some(selected_files),
        Some(staging),
    )
}

fn inspect_bzip2_tar<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    selected_files: Option<&[ReviewedArchiveFile<'_>]>,
    staging: Option<&Dir>,
) -> Result<Vec<ArchiveEntry>, Bzip2TarInventoryError> {
    if max_compressed_bytes == 0 || max_compressed_bytes > MAX_BZIP2_ARCHIVE_BYTES {
        return Err(Bzip2TarInventoryError::InvalidCompressedLimit);
    }
    // One additional byte distinguishes an oversized compressed artifact from
    // a stream exactly at its reviewed limit, whatever the buffer read ahead.
    // The single-stream decoder stops at the end of the first stream.
    let mut decoder = BzDecoder::new(BufReader::new(source.take(max_compressed_bytes + 1)));
    let inventory = match (selected_files, staging) {
        (Some(files), Some(directory)) => stage_tar_selected_files(
            &mut decoder,
            max_tar_bytes,
            bounds,
            reviewed_aliases,
            files,
            directory,
        ),
        (Some(files), None) => {
            inspect_tar_selected_files(&mut decoder, max_tar_bytes, bounds, reviewed_aliases, files)
        }
        (None, None) => {
            inspect_tar_inventory(&mut decoder, max_tar_bytes, bounds, reviewed_aliases)
        }
        (None, Some(_)) => Err(TarInventoryError::InvalidSelection),
    };
    let staging_completed = staging.is_some() && inventory.is_ok();
    let result = (|| {
        let mut buffered = decoder.into_inner();
        if buffered.get_ref().limit() == 0 {
            return Err(Bzip2TarInventoryError::CompressedInputTooLarge);
        }
        let entries = inventory.map_err(Bzip2TarInventoryError::Tar)?;
        // The tar reader drained the decoder to its stream end; anything
        // left, in the read-ahead buffer or after it, is a second stream or
        // trailing bytes.
        let remaining = buffered.fill_buf().map_err(|error| {
            Bzip2TarInventoryError::Tar(TarInventoryError::Malformed(error.kind()))
        })?;
        if !remaining.is_empty() {
            return Err(Bzip2TarInventoryError::TrailingCompressedContent);
        }
        Ok(entries)
    })();
    if result.is_err()
        && staging_completed
        && let (Some(directory), Some(files)) = (staging, selected_files)
    {
        cleanup_staged_files(directory, files).map_err(Bzip2TarInventoryError::Tar)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Cursor, Write},
    };

    use bzip2::{Compression, write::BzEncoder};
    use cap_std::fs::Dir;
    use tar::{Builder, EntryType, Header};

    use super::{
        Bzip2TarInventoryError, inspect_bzip2_tar_inventory, inspect_bzip2_tar_selected_files,
        stage_bzip2_tar_selected_files,
    };
    use crate::test_folder::TestFolder;
    use crate::{
        ArchiveInventoryBounds, ArchiveInventoryError, ReviewedArchiveAlias, ReviewedArchiveFile,
        TarInventoryError,
    };
    use vsift_domain::ArtifactIntegrity;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// A sherpa-onnx style archive: a library, its version link and a
    /// tool.
    fn tar() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut archive = Builder::new(Vec::new());
        let mut file = Header::new_gnu();
        file.set_path("root/lib/libtool.so.1")?;
        file.set_size(3);
        file.set_mode(0o755);
        file.set_cksum();
        archive.append(&file, Cursor::new(b"abc"))?;
        let mut link = Header::new_gnu();
        link.set_path("root/lib/libtool.so")?;
        link.set_size(0);
        link.set_mode(0o777);
        link.set_entry_type(EntryType::Symlink);
        link.set_link_name("libtool.so.1")?;
        link.set_cksum();
        archive.append(&link, Cursor::new([]))?;
        Ok(archive.into_inner()?)
    }

    fn compress(bytes: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut encoder = BzEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(bytes)?;
        Ok(encoder.finish()?)
    }

    fn bounds() -> Result<ArchiveInventoryBounds, ArchiveInventoryError> {
        ArchiveInventoryBounds::new(4, 100)
    }

    fn alias() -> [ReviewedArchiveAlias<'static>; 1] {
        [ReviewedArchiveAlias {
            path: "root/lib/libtool.so",
            target: "libtool.so.1",
        }]
    }

    fn selected() -> Result<[ReviewedArchiveFile<'static>; 1], Box<dyn std::error::Error>> {
        Ok([ReviewedArchiveFile {
            path: "root/lib/libtool.so.1",
            integrity: ArtifactIntegrity::from_sha256_hex(
                3,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            )?,
        }])
    }

    fn length(bytes: &[u8]) -> u64 {
        u64::try_from(bytes.len()).unwrap_or(u64::MAX)
    }

    #[test]
    fn a_reviewed_archive_is_inventoried_and_its_selection_staged_without_links() -> TestResult {
        let archive = compress(&tar()?)?;
        let entries = inspect_bzip2_tar_inventory(
            Cursor::new(&archive),
            length(&archive),
            4_096,
            bounds()?,
            &alias(),
        )?;
        assert_eq!(entries.len(), 2);

        let folder = TestFolder::new("vsift-bzip2-stage")?;
        let staging = Dir::open_ambient_dir(&*folder, cap_std::ambient_authority())?;
        stage_bzip2_tar_selected_files(
            Cursor::new(&archive),
            length(&archive),
            4_096,
            bounds()?,
            &alias(),
            &selected()?,
            &staging,
        )?;
        drop(staging);
        assert_eq!(fs::read(folder.join("libtool.so.1"))?, b"abc");
        assert_eq!(fs::read_dir(&*folder)?.count(), 1);
        Ok(())
    }

    /// The shape of the fuzz target's `one-file.tar.bz2` seed (`root/tool`,
    /// three bytes), which the replay test rebuilds and mirrors here.
    #[test]
    fn the_fuzz_seeds_one_file_shape_is_accepted() -> TestResult {
        let mut builder = Builder::new(Vec::new());
        let mut header = Header::new_gnu();
        header.set_path("root/tool")?;
        header.set_size(3);
        header.set_mode(0o600);
        header.set_cksum();
        builder.append(&header, Cursor::new(b"abc"))?;
        let archive = compress(&builder.into_inner()?)?;
        let entries = inspect_bzip2_tar_inventory(
            Cursor::new(&archive),
            length(&archive),
            4_096,
            bounds()?,
            &[],
        )?;
        assert_eq!(entries.len(), 1);
        Ok(())
    }

    #[test]
    fn a_second_stream_trailing_bytes_or_an_oversized_input_is_refused() -> TestResult {
        let single = compress(&tar()?)?;
        let mut concatenated = single.clone();
        concatenated.extend(compress(b"more")?);
        let mut trailing = single.clone();
        trailing.push(0);
        for archive in [&concatenated, &trailing] {
            assert_eq!(
                inspect_bzip2_tar_selected_files(
                    Cursor::new(archive),
                    length(archive),
                    4_096,
                    bounds()?,
                    &alias(),
                    &selected()?,
                ),
                Err(Bzip2TarInventoryError::TrailingCompressedContent)
            );
        }
        assert_eq!(
            inspect_bzip2_tar_inventory(
                Cursor::new(&single),
                length(&single) - 1,
                4_096,
                bounds()?,
                &alias()
            ),
            Err(Bzip2TarInventoryError::CompressedInputTooLarge)
        );
        for limit in [0, super::MAX_BZIP2_ARCHIVE_BYTES + 1] {
            assert_eq!(
                inspect_bzip2_tar_inventory(
                    Cursor::new(&single),
                    limit,
                    4_096,
                    bounds()?,
                    &alias()
                ),
                Err(Bzip2TarInventoryError::InvalidCompressedLimit)
            );
        }
        Ok(())
    }

    #[test]
    fn the_tar_policy_still_applies_inside_the_stream() -> TestResult {
        let expanded = tar()?;
        let archive = compress(&expanded)?;
        assert_eq!(
            inspect_bzip2_tar_inventory(
                Cursor::new(&archive),
                length(&archive),
                4_096,
                bounds()?,
                &[]
            ),
            Err(Bzip2TarInventoryError::Tar(TarInventoryError::Inventory(
                ArchiveInventoryError::UnreviewedLink
            )))
        );
        assert_eq!(
            inspect_bzip2_tar_inventory(
                Cursor::new(&archive),
                length(&archive),
                length(&expanded) - 1,
                bounds()?,
                &alias()
            ),
            Err(Bzip2TarInventoryError::Tar(
                TarInventoryError::StreamTooLarge
            ))
        );
        Ok(())
    }

    #[test]
    fn every_truncation_and_corruption_is_refused_without_panic() -> TestResult {
        let archive = compress(&tar()?)?;
        for cut in 0..archive.len() {
            let truncated = &archive[..cut];
            assert!(
                inspect_bzip2_tar_selected_files(
                    Cursor::new(truncated),
                    length(truncated).max(1),
                    4_096,
                    bounds()?,
                    &alias(),
                    &selected()?,
                )
                .is_err(),
                "cut at {cut}"
            );
        }
        for index in 0..archive.len() {
            let mut changed = archive.clone();
            changed[index] ^= 0x10;
            // bzip2 carries a CRC per block and per stream, so a change is
            // either refused or (in padding bits) leaves the same tar.
            if let Ok(entries) = inspect_bzip2_tar_selected_files(
                Cursor::new(&changed),
                length(&changed),
                4_096,
                bounds()?,
                &alias(),
                &selected()?,
            ) {
                assert_eq!(entries.len(), 2, "byte {index}");
            }
        }
        Ok(())
    }
}
