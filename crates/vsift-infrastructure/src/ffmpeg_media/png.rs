//! Strict walker of the PNG sequence `FFmpeg`'s `image2pipe` writes.
//!
//! Evidence frames leave `FFmpeg` as consecutive PNG files on one pipe, with
//! no separator: the only way to split them is to walk each file's chunks.
//! The bytes are untrusted provider output that `VSift` stores and hands to
//! an agent as source evidence, so the walk accepts exactly what the
//! evidence profile asks `FFmpeg` for and nothing else: 8-bit RGB
//! (`format=rgb24`), not interlaced, at the expected size, with every
//! chunk's CRC checked. Pixels are never decoded here; the image content is
//! `FFmpeg`'s.

use flate2::Crc;
use vsift_domain::FrameDimensions;

use super::{MAX_FRAME_BYTES, MAX_FRAME_PIXELS, MediaError};

/// Most images one extraction run writes.
pub const MAX_IMAGES_PER_RUN: usize = 8;

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
/// IHDR length, and its fixed fields for 8-bit truecolour without
/// interlacing: bit depth 8, colour type 2 (RGB), compression 0, filter 0,
/// interlace 0.
const IHDR_LENGTH: usize = 13;
const IHDR_RGB8: [u8; 5] = [8, 2, 0, 0, 0];
/// PNG's own limit on a chunk length.
const MAX_CHUNK_LENGTH: u32 = 0x7fff_ffff;

/// Splits `stdout` into exactly `expected` PNG images of `dimensions`.
///
/// Published beside the other provider parsers for fuzzing (ADR 0016,
/// decision 6). Each image must be the PNG signature, an `IHDR` chunk first
/// (13 bytes: `dimensions`, 8-bit RGB, no interlace), one run of consecutive
/// `IDAT` chunks, ancillary chunks anywhere between, and `IEND` last. Every
/// chunk's CRC must match; unknown critical chunks (such as a palette),
/// bytes after the last `IEND`, and images beyond `expected` are rejected.
///
/// # Errors
///
/// Returns [`MediaError::OutputLimit`] beyond [`MAX_FRAME_BYTES`],
/// [`MediaError::InvalidDimensions`] for dimensions above 16 megapixels, and
/// [`MediaError::InvalidDecodedOutput`] for anything else; parsing never
/// partially succeeds.
pub fn parse_png_sequence(
    stdout: &[u8],
    expected: usize,
    dimensions: FrameDimensions,
) -> Result<Vec<Vec<u8>>, MediaError> {
    if stdout.len() > MAX_FRAME_BYTES {
        return Err(MediaError::OutputLimit);
    }
    if u64::from(dimensions.width()) * u64::from(dimensions.height()) > MAX_FRAME_PIXELS {
        return Err(MediaError::InvalidDimensions);
    }
    if expected == 0 || expected > MAX_IMAGES_PER_RUN {
        return Err(MediaError::InvalidDecodedOutput);
    }
    let mut images = Vec::with_capacity(expected);
    let mut rest = stdout;
    while !rest.is_empty() {
        if images.len() == expected {
            return Err(MediaError::InvalidDecodedOutput);
        }
        let length = image_length(rest, dimensions)?;
        let (image, tail) = rest
            .split_at_checked(length)
            .ok_or(MediaError::InvalidDecodedOutput)?;
        images.push(image.to_vec());
        rest = tail;
    }
    if images.len() == expected {
        Ok(images)
    } else {
        Err(MediaError::InvalidDecodedOutput)
    }
}

const fn invalid() -> MediaError {
    MediaError::InvalidDecodedOutput
}

/// Where a chunk may appear relative to the image data.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Stage {
    /// After `IHDR`, before the first `IDAT`.
    Header,
    /// Inside the run of consecutive `IDAT` chunks.
    Data,
    /// After the `IDAT` run, before `IEND`.
    Trailer,
}

/// Walks one image at the start of `bytes` and returns its length.
fn image_length(bytes: &[u8], dimensions: FrameDimensions) -> Result<usize, MediaError> {
    if !bytes.starts_with(SIGNATURE) {
        return Err(invalid());
    }
    let mut offset = SIGNATURE.len();
    let mut stage = Stage::Header;
    let mut first = true;
    loop {
        let (kind, data, next) = chunk_at(bytes, offset)?;
        offset = next;
        if first {
            if kind != *b"IHDR" || !header_matches(data, dimensions) {
                return Err(invalid());
            }
            first = false;
            continue;
        }
        stage = match (&kind, stage) {
            (b"IEND", Stage::Data | Stage::Trailer) if data.is_empty() => return Ok(offset),
            (b"IDAT", Stage::Header | Stage::Data) => Stage::Data,
            (b"IDAT" | b"IEND" | b"IHDR", _) => return Err(invalid()),
            // An ancillary chunk (first letter lowercase) may appear anywhere;
            // any other critical chunk, such as a palette, is not RGB output.
            (other, current) if other[0].is_ascii_lowercase() => {
                if current == Stage::Data {
                    Stage::Trailer
                } else {
                    current
                }
            }
            _ => return Err(invalid()),
        };
    }
}

/// Reads the chunk at `offset`: its type, its data and the offset after it.
fn chunk_at(bytes: &[u8], offset: usize) -> Result<([u8; 4], &[u8], usize), MediaError> {
    let header = bytes
        .get(offset..offset.checked_add(8).ok_or_else(invalid)?)
        .ok_or_else(invalid)?;
    let (length, kind) = header.split_at(4);
    let length = u32::from_be_bytes(length.try_into().map_err(|_| invalid())?);
    let kind: [u8; 4] = kind.try_into().map_err(|_| invalid())?;
    if length > MAX_CHUNK_LENGTH || !kind.iter().all(u8::is_ascii_alphabetic) {
        return Err(invalid());
    }
    let length = usize::try_from(length).map_err(|_| invalid())?;
    let data_start = offset + 8;
    let data_end = data_start.checked_add(length).ok_or_else(invalid)?;
    let crc_end = data_end.checked_add(4).ok_or_else(invalid)?;
    let data = bytes.get(data_start..data_end).ok_or_else(invalid)?;
    let stored = bytes.get(data_end..crc_end).ok_or_else(invalid)?;
    let mut crc = Crc::new();
    crc.update(&kind);
    crc.update(data);
    if crc.sum().to_be_bytes() != *stored {
        return Err(invalid());
    }
    Ok((kind, data, crc_end))
}

/// Whether an `IHDR` body describes `dimensions` as 8-bit RGB without interlacing.
fn header_matches(data: &[u8], dimensions: FrameDimensions) -> bool {
    if data.len() != IHDR_LENGTH {
        return false;
    }
    let (size, fields) = data.split_at(8);
    let (width, height) = size.split_at(4);
    let read = |bytes: &[u8]| bytes.try_into().ok().map(u32::from_be_bytes);
    read(width) == Some(dimensions.width())
        && read(height) == Some(dimensions.height())
        && fields == IHDR_RGB8
}

#[cfg(test)]
mod tests {
    use flate2::Crc;
    use vsift_domain::FrameDimensions;

    use super::{MAX_IMAGES_PER_RUN, parse_png_sequence};
    use crate::MediaError;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = u32::try_from(data.len())
            .unwrap_or(0)
            .to_be_bytes()
            .to_vec();
        bytes.extend_from_slice(&kind);
        bytes.extend_from_slice(data);
        let mut crc = Crc::new();
        crc.update(&kind);
        crc.update(data);
        bytes.extend_from_slice(&crc.sum().to_be_bytes());
        bytes
    }

    fn header(width: u32, height: u32, fields: [u8; 5]) -> Vec<u8> {
        let mut data = width.to_be_bytes().to_vec();
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&fields);
        chunk(*b"IHDR", &data)
    }

    /// A structurally valid 2x1 RGB PNG built from `chunks` after the signature.
    fn png(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        for part in chunks {
            bytes.extend_from_slice(part);
        }
        bytes
    }

    fn valid() -> Vec<u8> {
        png(&[
            header(2, 1, [8, 2, 0, 0, 0]),
            chunk(*b"pHYs", &[0; 9]),
            chunk(*b"IDAT", b"\x78\x01"),
            chunk(*b"IDAT", b"\x01\x02"),
            chunk(*b"tEXt", b"x"),
            chunk(*b"IEND", b""),
        ])
    }

    #[test]
    fn splits_consecutive_images_of_the_expected_size() -> TestResult {
        let dimensions = FrameDimensions::new(2, 1)?;
        let mut two = valid();
        two.extend_from_slice(&valid());
        let images = parse_png_sequence(&two, 2, dimensions)?;
        assert_eq!(images, vec![valid(), valid()]);
        assert_eq!(parse_png_sequence(&valid(), 1, dimensions)?.len(), 1);
        Ok(())
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One table of every rejected departure reads best in one place.
    fn rejects_every_departure_from_the_evidence_profile() -> TestResult {
        let dimensions = FrameDimensions::new(2, 1)?;
        let data = chunk(*b"IDAT", b"\x78\x01");
        let end = chunk(*b"IEND", b"");
        let mut corrupt_crc = valid();
        if let Some(byte) = corrupt_crc.last_mut() {
            *byte ^= 1;
        }
        let mut trailing = valid();
        trailing.push(0);
        let mut two = valid();
        two.extend_from_slice(&valid());
        for (bytes, expected) in [
            (Vec::new(), 1),
            (valid(), 0),
            (valid(), 2),
            (two, 1),
            (trailing, 1),
            (corrupt_crc, 1),
            (
                valid()
                    .get(..valid().len() - 1)
                    .unwrap_or_default()
                    .to_vec(),
                1,
            ),
            (
                png(&[header(3, 1, [8, 2, 0, 0, 0]), data.clone(), end.clone()]),
                1,
            ),
            (
                png(&[header(2, 1, [16, 2, 0, 0, 0]), data.clone(), end.clone()]),
                1,
            ),
            (
                png(&[header(2, 1, [8, 6, 0, 0, 0]), data.clone(), end.clone()]),
                1,
            ),
            (
                png(&[header(2, 1, [8, 2, 0, 0, 1]), data.clone(), end.clone()]),
                1,
            ),
            (
                png(&[data.clone(), header(2, 1, [8, 2, 0, 0, 0]), end.clone()]),
                1,
            ),
            (png(&[header(2, 1, [8, 2, 0, 0, 0]), end.clone()]), 1),
            (
                png(&[
                    header(2, 1, [8, 2, 0, 0, 0]),
                    chunk(*b"PLTE", &[0; 3]),
                    data.clone(),
                    end.clone(),
                ]),
                1,
            ),
            (
                png(&[
                    header(2, 1, [8, 2, 0, 0, 0]),
                    data.clone(),
                    chunk(*b"tEXt", b"x"),
                    data.clone(),
                    end.clone(),
                ]),
                1,
            ),
            (
                png(&[
                    header(2, 1, [8, 2, 0, 0, 0]),
                    data.clone(),
                    chunk(*b"IEND", b"x"),
                ]),
                1,
            ),
            (
                png(&[
                    header(2, 1, [8, 2, 0, 0, 0]),
                    data.clone(),
                    chunk(*b"ID1T", b""),
                ]),
                1,
            ),
            (
                png(&[
                    header(2, 1, [8, 2, 0, 0, 0]),
                    data,
                    chunk(*b"IHDR", &[0; 13]),
                    end,
                ]),
                1,
            ),
        ] {
            assert!(
                parse_png_sequence(&bytes, expected, dimensions).is_err(),
                "{expected} {bytes:?}"
            );
        }
        assert!(matches!(
            parse_png_sequence(&valid(), MAX_IMAGES_PER_RUN + 1, dimensions),
            Err(MediaError::InvalidDecodedOutput)
        ));
        assert!(matches!(
            parse_png_sequence(&valid(), 1, FrameDimensions::new(4_001, 4_000)?),
            Err(MediaError::InvalidDimensions)
        ));
        assert!(matches!(
            parse_png_sequence(&vec![0; super::MAX_FRAME_BYTES + 1], 1, dimensions),
            Err(MediaError::OutputLimit)
        ));
        Ok(())
    }
}
