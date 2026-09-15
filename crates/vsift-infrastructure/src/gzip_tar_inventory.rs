//! Read-only gzip decoding before bounded tar inventory validation.

use std::{error::Error, fmt, io::Read};

use cap_std::fs::Dir;
use flate2::read::MultiGzDecoder;

use crate::bounded_tar_inventory::cleanup_staged_files;
use crate::{
    ArchiveEntry, ArchiveInventoryBounds, ReviewedArchiveAlias, ReviewedArchiveFile,
    TarInventoryError, inspect_tar_inventory, inspect_tar_selected_files, stage_tar_selected_files,
};

/// Hard limit on compressed gzip bytes read from a verified artifact.
pub const MAX_GZIP_ARCHIVE_BYTES: u64 = 268_435_456;

/// A typed reason a gzip-compressed tar archive cannot proceed to extraction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GzipTarInventoryError {
    /// The reviewed compressed-input limit is zero or exceeds the hard ceiling.
    InvalidCompressedLimit,
    /// The compressed input exceeds its reviewed byte limit.
    CompressedInputTooLarge,
    /// Gzip decoding or the expanded tar inventory failed.
    Tar(TarInventoryError),
}

impl fmt::Display for GzipTarInventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidCompressedLimit => "gzip input limit is invalid",
            Self::CompressedInputTooLarge => "gzip input exceeds reviewed limit",
            Self::Tar(_) => "gzip or tar inventory failed review policy",
        })
    }
}

impl Error for GzipTarInventoryError {}

/// Decodes all gzip members and validates the complete expanded tar inventory.
///
/// The compressed-input limit includes gzip headers and trailers. The tar
/// reader separately caps the entire expanded stream. This function neither
/// writes nor extracts archive contents; the caller must independently verify
/// whole-artifact provenance before using this read-only inspection result.
///
/// # Errors
///
/// Returns typed compressed-size, decoder, or tar-inventory rejection.
pub fn inspect_gzip_tar_inventory<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
) -> Result<Vec<ArchiveEntry>, GzipTarInventoryError> {
    inspect_gzip_tar(
        source,
        max_compressed_bytes,
        max_tar_bytes,
        bounds,
        reviewed_aliases,
        None,
        None,
    )
}

/// Validates a gzip/tar inventory and exact selected regular-file bytes.
///
/// # Errors
///
/// Returns typed compressed-size, decoder, selection or tar rejection.
pub fn inspect_gzip_tar_selected_files<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    selected_files: &[ReviewedArchiveFile<'_>],
) -> Result<Vec<ArchiveEntry>, GzipTarInventoryError> {
    inspect_gzip_tar(
        source,
        max_compressed_bytes,
        max_tar_bytes,
        bounds,
        reviewed_aliases,
        Some(selected_files),
        None,
    )
}

/// Validates a gzip/tar archive and stages reviewed regular files privately.
///
/// # Errors
///
/// Returns typed compressed-size, decoder, selection, tar or staging rejection.
pub fn stage_gzip_tar_selected_files<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    selected_files: &[ReviewedArchiveFile<'_>],
    staging: &Dir,
) -> Result<Vec<ArchiveEntry>, GzipTarInventoryError> {
    inspect_gzip_tar(
        source,
        max_compressed_bytes,
        max_tar_bytes,
        bounds,
        reviewed_aliases,
        Some(selected_files),
        Some(staging),
    )
}

fn inspect_gzip_tar<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    selected_files: Option<&[ReviewedArchiveFile<'_>]>,
    staging: Option<&Dir>,
) -> Result<Vec<ArchiveEntry>, GzipTarInventoryError> {
    if max_compressed_bytes == 0 || max_compressed_bytes > MAX_GZIP_ARCHIVE_BYTES {
        return Err(GzipTarInventoryError::InvalidCompressedLimit);
    }

    // One additional byte distinguishes an oversized compressed artifact from
    // a stream exactly at its reviewed limit, even if the decoder buffers input.
    let mut decoder = MultiGzDecoder::new(source.take(max_compressed_bytes + 1));
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
        let mut compressed = decoder.into_inner();
        if compressed.limit() == 0 {
            return Err(GzipTarInventoryError::CompressedInputTooLarge);
        }
        // Inspecting the expanded tar drains the decoder, but an early inventory
        // rejection can leave input unread. Do not turn such rejection into success.
        let entries = inventory.map_err(GzipTarInventoryError::Tar)?;
        let mut probe = [0_u8; 1];
        if compressed.read(&mut probe).map_err(|error| {
            GzipTarInventoryError::Tar(TarInventoryError::Malformed(error.kind()))
        })? != 0
        {
            return Err(GzipTarInventoryError::CompressedInputTooLarge);
        }
        Ok(entries)
    })();
    if result.is_err()
        && staging_completed
        && let (Some(directory), Some(files)) = (staging, selected_files)
    {
        cleanup_staged_files(directory, files).map_err(GzipTarInventoryError::Tar)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use std::{
        fmt::Write as _,
        fs,
        io::{self, Cursor, Write},
        path::PathBuf,
    };

    use cap_std::fs::Dir;
    use flate2::{Compression, write::GzEncoder};
    use tar::{Builder, Header};

    use super::{
        GzipTarInventoryError, inspect_gzip_tar_inventory, inspect_gzip_tar_selected_files,
        stage_gzip_tar_selected_files,
    };
    use crate::{ArchiveInventoryBounds, ReviewedArchiveFile, TarInventoryError};
    use vsift_domain::ArtifactIntegrity;

    fn fixture() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut tar = Builder::new(Vec::new());
        let mut header = Header::new_gnu();
        header.set_path("root/tool")?;
        header.set_size(3);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append(&header, Cursor::new(b"abc"))?;
        let tar_bytes = tar.into_inner()?;
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(&tar_bytes)?;
        Ok(gzip.finish()?)
    }

    fn bounds() -> Result<ArchiveInventoryBounds, Box<dyn std::error::Error>> {
        Ok(ArchiveInventoryBounds::new(4, 100)?)
    }

    fn selected() -> Result<[ReviewedArchiveFile<'static>; 1], Box<dyn std::error::Error>> {
        Ok([ReviewedArchiveFile {
            path: "root/tool",
            integrity: ArtifactIntegrity::from_sha256_hex(
                3,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            )?,
        }])
    }

    fn staging_directory() -> Result<(PathBuf, Dir), Box<dyn std::error::Error>> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| io::Error::other("test random source unavailable"))?;
        let mut suffix = String::with_capacity(32);
        for byte in random {
            write!(&mut suffix, "{byte:02x}")?;
        }
        let path = std::env::temp_dir().join(format!("vsift-p06-gzip-stage-{suffix}"));
        fs::create_dir(&path)?;
        let directory = Dir::open_ambient_dir(&path, cap_std::ambient_authority())?;
        Ok((path, directory))
    }

    #[test]
    fn validates_complete_compressed_tar_without_writing() -> Result<(), Box<dyn std::error::Error>>
    {
        let gzip = fixture()?;
        let entries = inspect_gzip_tar_inventory(
            Cursor::new(&gzip),
            gzip.len() as u64,
            10_000,
            bounds()?,
            &[],
        )?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "root/tool");
        Ok(())
    }

    #[test]
    fn verifies_selected_file_inside_gzip() -> Result<(), Box<dyn std::error::Error>> {
        let gzip = fixture()?;
        let selected = selected()?;
        let entries = inspect_gzip_tar_selected_files(
            Cursor::new(&gzip),
            gzip.len() as u64,
            10_000,
            bounds()?,
            &[],
            &selected,
        )?;
        assert_eq!(entries.len(), 1);
        Ok(())
    }

    #[test]
    fn stages_selected_file_and_cleans_it_after_hidden_gzip_content()
    -> Result<(), Box<dyn std::error::Error>> {
        let gzip = fixture()?;
        let selected = selected()?;
        let (path, directory) = staging_directory()?;
        stage_gzip_tar_selected_files(
            Cursor::new(&gzip),
            gzip.len() as u64,
            10_000,
            bounds()?,
            &[],
            &selected,
            &directory,
        )?;
        assert_eq!(fs::read(path.join("tool"))?, b"abc");
        drop(directory);
        fs::remove_dir_all(&path)?;

        let mut appended = gzip;
        let mut second = GzEncoder::new(Vec::new(), Compression::default());
        second.write_all(b"hidden")?;
        appended.extend_from_slice(&second.finish()?);
        let (path, directory) = staging_directory()?;
        assert!(
            stage_gzip_tar_selected_files(
                Cursor::new(&appended),
                appended.len() as u64,
                10_000,
                bounds()?,
                &[],
                &selected,
                &directory,
            )
            .is_err()
        );
        assert!(fs::read_dir(&path)?.next().is_none());
        drop(directory);
        fs::remove_dir_all(path)?;

        let (path, directory) = staging_directory()?;
        fs::write(path.join("tool"), b"preexisting")?;
        assert_eq!(
            stage_gzip_tar_selected_files(
                Cursor::new(&appended),
                appended.len() as u64,
                10_000,
                bounds()?,
                &[],
                &selected,
                &directory,
            ),
            Err(GzipTarInventoryError::Tar(
                TarInventoryError::InvalidStaging
            ))
        );
        assert_eq!(fs::read(path.join("tool"))?, b"preexisting");
        drop(directory);
        fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn rejects_corrupt_truncated_and_oversized_streams() -> Result<(), Box<dyn std::error::Error>> {
        let gzip = fixture()?;
        assert_eq!(
            inspect_gzip_tar_inventory(Cursor::new(&gzip), 0, 10_000, bounds()?, &[]),
            Err(GzipTarInventoryError::InvalidCompressedLimit)
        );
        assert_eq!(
            inspect_gzip_tar_inventory(Cursor::new(&gzip), 4, 10_000, bounds()?, &[]),
            Err(GzipTarInventoryError::CompressedInputTooLarge)
        );
        let truncated = &gzip[..gzip.len() - 4];
        assert!(matches!(
            inspect_gzip_tar_inventory(Cursor::new(truncated), 10_000, 10_000, bounds()?, &[]),
            Err(GzipTarInventoryError::Tar(TarInventoryError::Malformed(_)))
        ));
        assert!(
            inspect_gzip_tar_inventory(Cursor::new(&gzip), 10_000, 512, bounds()?, &[]).is_err()
        );
        let mut corrupt = gzip;
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0x80;
        assert!(matches!(
            inspect_gzip_tar_inventory(Cursor::new(corrupt), 10_000, 10_000, bounds()?, &[]),
            Err(GzipTarInventoryError::Tar(TarInventoryError::Malformed(_)))
        ));
        Ok(())
    }

    #[test]
    fn rejects_a_second_gzip_member_after_tar_end() -> Result<(), Box<dyn std::error::Error>> {
        let mut gzip = fixture()?;
        let mut second = GzEncoder::new(Vec::new(), Compression::default());
        second.write_all(b"hidden")?;
        gzip.extend_from_slice(&second.finish()?);
        assert_eq!(
            inspect_gzip_tar_inventory(
                Cursor::new(&gzip),
                gzip.len() as u64,
                10_000,
                bounds()?,
                &[]
            ),
            Err(GzipTarInventoryError::Tar(
                TarInventoryError::TrailingContent
            ))
        );
        Ok(())
    }
}
