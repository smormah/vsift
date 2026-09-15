//! Read-only, memory-limited XZ decoding before bounded tar inspection.

use std::{
    error::Error,
    fmt,
    io::{self, Read, Take},
};

use cap_std::fs::Dir;
use lzma_rust2::{Action, Status, XzStream};

use crate::bounded_tar_inventory::cleanup_staged_files;
use crate::{
    ArchiveEntry, ArchiveInventoryBounds, ReviewedArchiveAlias, ReviewedArchiveFile,
    TarInventoryError, inspect_tar_inventory, inspect_tar_selected_files, stage_tar_selected_files,
};

/// Hard limit on compressed XZ bytes read from a verified artifact.
pub const MAX_XZ_ARCHIVE_BYTES: u64 = 134_217_728;
const XZ_MEMORY_LIMIT_KIB: u32 = 131_072;
const INPUT_BUFFER_BYTES: usize = 16 * 1024;

/// A typed reason an XZ-compressed tar cannot proceed to extraction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XzTarInventoryError {
    /// The reviewed compressed-input limit is zero or exceeds the hard ceiling.
    InvalidCompressedLimit,
    /// The compressed input exceeds its reviewed byte limit.
    CompressedInputTooLarge,
    /// Bytes remain after the single reviewed XZ stream.
    TrailingCompressedContent,
    /// XZ decoding or the expanded tar inventory failed.
    Tar(TarInventoryError),
}

impl fmt::Display for XzTarInventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidCompressedLimit => "XZ input limit is invalid",
            Self::CompressedInputTooLarge => "XZ input exceeds reviewed limit",
            Self::TrailingCompressedContent => "XZ input follows its reviewed stream",
            Self::Tar(_) => "XZ or tar inventory failed review policy",
        })
    }
}

impl Error for XzTarInventoryError {}

struct MemoryLimitedXzReader<Source: Read> {
    compressed: Take<Source>,
    decoder: XzStream,
    input: [u8; INPUT_BUFFER_BYTES],
    input_position: usize,
    input_length: usize,
    source_finished: bool,
    stream_finished: bool,
}

impl<Source: Read> MemoryLimitedXzReader<Source> {
    fn new(source: Take<Source>, memory_limit_kib: u32) -> Self {
        Self {
            compressed: source,
            // Only one XZ stream is reviewed. Concatenation is rejected after
            // decoding instead of silently extending the tar byte stream.
            decoder: XzStream::new_mem_limit(false, memory_limit_kib),
            input: [0_u8; INPUT_BUFFER_BYTES],
            input_position: 0,
            input_length: 0,
            source_finished: false,
            stream_finished: false,
        }
    }
}

impl<Source: Read> Read for MemoryLimitedXzReader<Source> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.stream_finished {
            return Ok(0);
        }
        loop {
            if self.input_position == self.input_length && !self.source_finished {
                self.input_length = self.compressed.read(&mut self.input)?;
                self.input_position = 0;
                self.source_finished = self.input_length == 0;
            }
            let action = if self.source_finished {
                Action::Finish
            } else {
                Action::Run
            };
            let result = self.decoder.process(
                &self.input[self.input_position..self.input_length],
                output,
                action,
            )?;
            self.input_position += result.bytes_consumed;
            if result.status == Status::StreamEnd {
                self.stream_finished = true;
            }
            if result.bytes_produced != 0 || self.stream_finished {
                return Ok(result.bytes_produced);
            }
            if result.bytes_consumed == 0
                && (self.source_finished || self.input_position != self.input_length)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "XZ decoder made no progress",
                ));
            }
        }
    }
}

/// Decodes one XZ stream with bounded dictionary memory and validates its tar.
///
/// The compressed input is limited to at most 128 MiB. The XZ decoder also
/// rejects blocks requiring more than 128 MiB of dictionary memory. The tar
/// reader independently caps all expanded headers, content and padding. This
/// performs no extraction or filesystem write and does not authorize an
/// unverified artifact for installation.
///
/// # Errors
///
/// Returns typed compressed-size, decoder, or tar-inventory rejection.
pub fn inspect_xz_tar_inventory<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
) -> Result<Vec<ArchiveEntry>, XzTarInventoryError> {
    inspect_xz_tar(
        source,
        max_compressed_bytes,
        max_tar_bytes,
        bounds,
        reviewed_aliases,
        None,
        None,
    )
}

/// Validates an XZ/tar inventory and exact selected regular-file bytes.
///
/// # Errors
///
/// Returns typed compressed-size, decoder, selection or tar rejection.
pub fn inspect_xz_tar_selected_files<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    selected_files: &[ReviewedArchiveFile<'_>],
) -> Result<Vec<ArchiveEntry>, XzTarInventoryError> {
    inspect_xz_tar(
        source,
        max_compressed_bytes,
        max_tar_bytes,
        bounds,
        reviewed_aliases,
        Some(selected_files),
        None,
    )
}

/// Validates an XZ/tar archive and stages reviewed regular files privately.
///
/// # Errors
///
/// Returns typed compressed-size, decoder, selection, tar or staging rejection.
pub fn stage_xz_tar_selected_files<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    selected_files: &[ReviewedArchiveFile<'_>],
    staging: &Dir,
) -> Result<Vec<ArchiveEntry>, XzTarInventoryError> {
    inspect_xz_tar(
        source,
        max_compressed_bytes,
        max_tar_bytes,
        bounds,
        reviewed_aliases,
        Some(selected_files),
        Some(staging),
    )
}

fn inspect_xz_tar<Source: Read>(
    source: Source,
    max_compressed_bytes: u64,
    max_tar_bytes: u64,
    bounds: ArchiveInventoryBounds,
    reviewed_aliases: &[ReviewedArchiveAlias<'_>],
    selected_files: Option<&[ReviewedArchiveFile<'_>]>,
    staging: Option<&Dir>,
) -> Result<Vec<ArchiveEntry>, XzTarInventoryError> {
    if max_compressed_bytes == 0 || max_compressed_bytes > MAX_XZ_ARCHIVE_BYTES {
        return Err(XzTarInventoryError::InvalidCompressedLimit);
    }
    let mut reader =
        MemoryLimitedXzReader::new(source.take(max_compressed_bytes + 1), XZ_MEMORY_LIMIT_KIB);
    let inventory = match (selected_files, staging) {
        (Some(files), Some(directory)) => stage_tar_selected_files(
            &mut reader,
            max_tar_bytes,
            bounds,
            reviewed_aliases,
            files,
            directory,
        ),
        (Some(files), None) => {
            inspect_tar_selected_files(&mut reader, max_tar_bytes, bounds, reviewed_aliases, files)
        }
        (None, None) => inspect_tar_inventory(&mut reader, max_tar_bytes, bounds, reviewed_aliases),
        (None, Some(_)) => Err(TarInventoryError::InvalidSelection),
    };
    let staging_completed = staging.is_some() && inventory.is_ok();
    let result =
        (|| {
            if reader.compressed.limit() == 0 {
                return Err(XzTarInventoryError::CompressedInputTooLarge);
            }
            let entries = inventory.map_err(XzTarInventoryError::Tar)?;
            if !reader.stream_finished || reader.input_position != reader.input_length {
                return Err(XzTarInventoryError::TrailingCompressedContent);
            }
            let mut probe = [0_u8; 1];
            if reader.compressed.read(&mut probe).map_err(|error| {
                XzTarInventoryError::Tar(TarInventoryError::Malformed(error.kind()))
            })? != 0
            {
                return Err(XzTarInventoryError::TrailingCompressedContent);
            }
            Ok(entries)
        })();
    if result.is_err()
        && staging_completed
        && let (Some(directory), Some(files)) = (staging, selected_files)
    {
        cleanup_staged_files(directory, files).map_err(XzTarInventoryError::Tar)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use std::{
        fmt::Write as _,
        fs,
        io::{self, Cursor, Read, Write},
        path::PathBuf,
    };

    use cap_std::fs::Dir;
    use lzma_rust2::{XzOptions, XzWriter};
    use tar::{Builder, Header};

    use super::{
        MemoryLimitedXzReader, XzTarInventoryError, inspect_xz_tar_inventory,
        inspect_xz_tar_selected_files, stage_xz_tar_selected_files,
    };
    use crate::{ArchiveInventoryBounds, ReviewedArchiveFile};
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
        let mut xz = XzWriter::new(Vec::new(), XzOptions::with_preset(1))?;
        xz.write_all(&tar_bytes)?;
        Ok(xz.finish()?)
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
        let path = std::env::temp_dir().join(format!("vsift-p06-xz-stage-{suffix}"));
        fs::create_dir(&path)?;
        let directory = Dir::open_ambient_dir(&path, cap_std::ambient_authority())?;
        Ok((path, directory))
    }

    #[test]
    fn validates_one_complete_xz_tar() -> Result<(), Box<dyn std::error::Error>> {
        let xz = fixture()?;
        let entries =
            inspect_xz_tar_inventory(Cursor::new(&xz), xz.len() as u64, 10_000, bounds()?, &[])?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "root/tool");
        Ok(())
    }

    #[test]
    fn verifies_selected_file_inside_xz() -> Result<(), Box<dyn std::error::Error>> {
        let xz = fixture()?;
        let selected = selected()?;
        let entries = inspect_xz_tar_selected_files(
            Cursor::new(&xz),
            xz.len() as u64,
            10_000,
            bounds()?,
            &[],
            &selected,
        )?;
        assert_eq!(entries.len(), 1);
        Ok(())
    }

    #[test]
    fn stages_selected_file_and_cleans_it_after_trailing_xz_content()
    -> Result<(), Box<dyn std::error::Error>> {
        let xz = fixture()?;
        let selected = selected()?;
        let (path, directory) = staging_directory()?;
        stage_xz_tar_selected_files(
            Cursor::new(&xz),
            xz.len() as u64,
            10_000,
            bounds()?,
            &[],
            &selected,
            &directory,
        )?;
        assert_eq!(fs::read(path.join("tool"))?, b"abc");
        drop(directory);
        fs::remove_dir_all(&path)?;

        let mut appended = xz;
        appended.extend_from_slice(b"hidden");
        let (path, directory) = staging_directory()?;
        assert_eq!(
            stage_xz_tar_selected_files(
                Cursor::new(&appended),
                10_000,
                10_000,
                bounds()?,
                &[],
                &selected,
                &directory,
            ),
            Err(XzTarInventoryError::TrailingCompressedContent)
        );
        assert!(fs::read_dir(&path)?.next().is_none());
        drop(directory);
        fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn rejects_limits_truncation_and_trailing_input() -> Result<(), Box<dyn std::error::Error>> {
        let xz = fixture()?;
        assert_eq!(
            inspect_xz_tar_inventory(Cursor::new(&xz), 0, 10_000, bounds()?, &[]),
            Err(XzTarInventoryError::InvalidCompressedLimit)
        );
        assert_eq!(
            inspect_xz_tar_inventory(Cursor::new(&xz), 4, 10_000, bounds()?, &[]),
            Err(XzTarInventoryError::CompressedInputTooLarge)
        );
        let truncated = &xz[..xz.len() - 4];
        assert!(
            inspect_xz_tar_inventory(Cursor::new(truncated), 10_000, 10_000, bounds()?, &[])
                .is_err()
        );
        let mut appended = xz;
        appended.extend_from_slice(b"hidden");
        assert_eq!(
            inspect_xz_tar_inventory(Cursor::new(appended), 10_000, 10_000, bounds()?, &[]),
            Err(XzTarInventoryError::TrailingCompressedContent)
        );
        Ok(())
    }

    #[test]
    fn rejects_an_xz_dictionary_above_memory_budget() -> Result<(), Box<dyn std::error::Error>> {
        let xz = fixture()?;
        let mut reader = MemoryLimitedXzReader::new(Cursor::new(xz).take(10_000), 1);
        let mut output = Vec::new();
        assert!(reader.read_to_end(&mut output).is_err());
        Ok(())
    }
}
