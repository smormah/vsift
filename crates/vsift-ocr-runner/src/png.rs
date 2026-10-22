//! A strict PNG decoder for the frames `VSift` hands the runner.
//!
//! `VSift`'s evidence frames are PNG files that its own `FFmpeg` wrote in one
//! profile: 8-bit RGB, not interlaced, at most 16 megapixels (the frame
//! evidence contract). The runner accepts exactly that profile and nothing
//! else, so the only image format it parses is a small one, in safe Rust,
//! with every chunk's CRC checked and every size bounded before memory is
//! reserved. A palette, an alpha channel, 16-bit samples, interlacing and
//! unknown critical chunks are refused rather than converted.

use std::io::Read;

use flate2::{Crc, read::ZlibDecoder};

use crate::image::RgbImage;

/// Most pixels a frame may have: 16 megapixels, as for `VSift`'s frame evidence.
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;
/// Most bytes a PNG file may have before it is read.
pub const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
/// IHDR's fixed fields for 8-bit truecolour without interlacing: bit depth 8,
/// colour type 2 (RGB), compression 0, filter 0, interlace 0.
const IHDR_RGB8: [u8; 5] = [8, 2, 0, 0, 0];
const IHDR_LENGTH: usize = 13;
const MAX_CHUNK_LENGTH: u32 = 0x7fff_ffff;
const CHANNELS: usize = 3;

/// Why a PNG file was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PngError {
    /// The bytes are not a PNG file of the accepted profile.
    Invalid,
    /// The image is larger than [`MAX_PIXELS`] or the file than [`MAX_FILE_BYTES`].
    TooLarge,
    /// The image is 8-bit RGB in name but uses a feature the profile excludes.
    Unsupported,
}

impl std::fmt::Display for PngError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "the image is not a valid PNG file",
            Self::TooLarge => "the image is larger than the runner accepts",
            Self::Unsupported => "the image is not an 8-bit RGB PNG without interlacing",
        })
    }
}

impl std::error::Error for PngError {}

/// Decodes an 8-bit RGB PNG file into its pixels.
///
/// # Errors
///
/// [`PngError::TooLarge`] beyond the bounds, [`PngError::Unsupported`] for
/// another colour type, bit depth or interlacing, and [`PngError::Invalid`]
/// for anything else (a bad signature, CRC, chunk order, filter or length).
pub fn decode_rgb8(bytes: &[u8]) -> Result<RgbImage, PngError> {
    if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_FILE_BYTES) {
        return Err(PngError::TooLarge);
    }
    let rest = bytes.strip_prefix(SIGNATURE).ok_or(PngError::Invalid)?;
    let (kind, header, mut offset) = chunk_at(rest, 0)?;
    if &kind != b"IHDR" || header.len() != IHDR_LENGTH {
        return Err(PngError::Invalid);
    }
    let (width, height) = header_size(header)?;
    if header.get(8..13) != Some(IHDR_RGB8.as_slice()) {
        return Err(PngError::Unsupported);
    }
    let mut compressed = Vec::new();
    let mut state = DataState::Before;
    loop {
        let (kind, data, next) = chunk_at(rest, offset)?;
        offset = next;
        match &kind {
            b"IDAT" => {
                if state == DataState::After {
                    return Err(PngError::Invalid);
                }
                state = DataState::Within;
                compressed.extend_from_slice(data);
            }
            b"IEND" => {
                if state == DataState::Before || !data.is_empty() || offset != rest.len() {
                    return Err(PngError::Invalid);
                }
                break;
            }
            _ if is_critical(kind) => return Err(PngError::Unsupported),
            _ => {
                if state == DataState::Within {
                    state = DataState::After;
                }
            }
        }
    }
    let stride = width.checked_mul(CHANNELS).ok_or(PngError::TooLarge)?;
    let expected = stride
        .checked_add(1)
        .and_then(|row| row.checked_mul(height))
        .ok_or(PngError::TooLarge)?;
    let filtered = inflate_exactly(&compressed, expected)?;
    let pixels = unfilter(&filtered, stride, height)?;
    RgbImage::new(width, height, pixels).map_err(|_| PngError::Invalid)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataState {
    Before,
    Within,
    After,
}

/// A chunk's kind is critical when its first letter is upper case.
const fn is_critical(kind: [u8; 4]) -> bool {
    kind[0].is_ascii_uppercase()
}

fn header_size(header: &[u8]) -> Result<(usize, usize), PngError> {
    let field = |range: std::ops::Range<usize>| {
        header
            .get(range)
            .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            .map(u32::from_be_bytes)
            .ok_or(PngError::Invalid)
    };
    let width = field(0..4)?;
    let height = field(4..8)?;
    if width == 0 || height == 0 {
        return Err(PngError::Invalid);
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(PngError::TooLarge);
    }
    let width = usize::try_from(width).map_err(|_| PngError::TooLarge)?;
    let height = usize::try_from(height).map_err(|_| PngError::TooLarge)?;
    Ok((width, height))
}

/// The chunk at `offset`: its kind, its data and the offset after its CRC.
fn chunk_at(bytes: &[u8], offset: usize) -> Result<([u8; 4], &[u8], usize), PngError> {
    let length_bytes = bytes
        .get(offset..offset.checked_add(4).ok_or(PngError::Invalid)?)
        .ok_or(PngError::Invalid)?;
    let length =
        u32::from_be_bytes(<[u8; 4]>::try_from(length_bytes).map_err(|_| PngError::Invalid)?);
    if length > MAX_CHUNK_LENGTH {
        return Err(PngError::Invalid);
    }
    let length = usize::try_from(length).map_err(|_| PngError::Invalid)?;
    let kind_start = offset + 4;
    let data_start = kind_start + 4;
    let data_end = data_start.checked_add(length).ok_or(PngError::Invalid)?;
    let crc_end = data_end.checked_add(4).ok_or(PngError::Invalid)?;
    let kind = bytes
        .get(kind_start..data_start)
        .and_then(|kind| <[u8; 4]>::try_from(kind).ok())
        .ok_or(PngError::Invalid)?;
    let data = bytes.get(data_start..data_end).ok_or(PngError::Invalid)?;
    let stored = bytes
        .get(data_end..crc_end)
        .and_then(|crc| <[u8; 4]>::try_from(crc).ok())
        .map(u32::from_be_bytes)
        .ok_or(PngError::Invalid)?;
    let mut crc = Crc::new();
    crc.update(&kind);
    crc.update(data);
    if crc.sum() != stored {
        return Err(PngError::Invalid);
    }
    Ok((kind, data, crc_end))
}

/// Inflates the zlib stream to exactly `expected` bytes, reading at most one more.
fn inflate_exactly(compressed: &[u8], expected: usize) -> Result<Vec<u8>, PngError> {
    let limit = u64::try_from(expected)
        .ok()
        .and_then(|bytes| bytes.checked_add(1))
        .ok_or(PngError::TooLarge)?;
    let mut output = Vec::with_capacity(expected);
    ZlibDecoder::new(compressed)
        .take(limit)
        .read_to_end(&mut output)
        .map_err(|_| PngError::Invalid)?;
    if output.len() == expected {
        Ok(output)
    } else {
        Err(PngError::Invalid)
    }
}

/// Reverses PNG's per-row filters (None, Sub, Up, Average, Paeth).
fn unfilter(filtered: &[u8], stride: usize, height: usize) -> Result<Vec<u8>, PngError> {
    let mut pixels = vec![0_u8; stride * height];
    for row in 0..height {
        let start = row * (stride + 1);
        let filter = *filtered.get(start).ok_or(PngError::Invalid)?;
        let source = filtered
            .get(start + 1..start + 1 + stride)
            .ok_or(PngError::Invalid)?;
        let (done, current) = pixels.split_at_mut(row * stride);
        let current = current.get_mut(..stride).ok_or(PngError::Invalid)?;
        let previous = row
            .checked_sub(1)
            .and_then(|above| done.get(above * stride..));
        for index in 0..stride {
            let left = index
                .checked_sub(CHANNELS)
                .and_then(|at| current.get(at))
                .copied()
                .unwrap_or(0);
            let up = previous
                .and_then(|row| row.get(index))
                .copied()
                .unwrap_or(0);
            let up_left = index
                .checked_sub(CHANNELS)
                .and_then(|at| previous.and_then(|row| row.get(at)))
                .copied()
                .unwrap_or(0);
            let value = *source.get(index).ok_or(PngError::Invalid)?;
            let predicted = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => average(left, up),
                4 => paeth(left, up, up_left),
                _ => return Err(PngError::Invalid),
            };
            if let Some(slot) = current.get_mut(index) {
                *slot = value.wrapping_add(predicted);
            }
        }
    }
    Ok(pixels)
}

fn average(left: u8, up: u8) -> u8 {
    // The mean of two bytes, rounded down, is a byte again.
    u8::try_from(u16::midpoint(u16::from(left), u16::from(up))).unwrap_or(u8::MAX)
}

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let estimate = i16::from(left) + i16::from(up) - i16::from(up_left);
    let to_left = (estimate - i16::from(left)).abs();
    let to_up = (estimate - i16::from(up)).abs();
    let to_up_left = (estimate - i16::from(up_left)).abs();
    if to_left <= to_up && to_left <= to_up_left {
        left
    } else if to_up <= to_up_left {
        up
    } else {
        up_left
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use flate2::{Compression, Crc, write::ZlibEncoder};

    use super::{PngError, decode_rgb8};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    pub(crate) fn chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
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

    /// An RGB PNG of `width` x `height` whose rows are filtered with `filter`.
    pub(crate) fn encode(
        width: u32,
        height: u32,
        pixels: &[u8],
        filter: u8,
        fields: [u8; 5],
    ) -> Vec<u8> {
        let stride = usize::try_from(width).unwrap_or(0) * 3;
        let mut raw = Vec::new();
        for (row_index, row) in pixels.chunks(stride.max(1)).enumerate() {
            raw.push(filter);
            let above = row_index
                .checked_sub(1)
                .and_then(|index| pixels.get(index * stride..index * stride + stride));
            for (index, value) in row.iter().enumerate() {
                let left = index
                    .checked_sub(3)
                    .and_then(|at| row.get(at))
                    .copied()
                    .unwrap_or(0);
                let up = above.and_then(|line| line.get(index)).copied().unwrap_or(0);
                let up_left = index
                    .checked_sub(3)
                    .and_then(|at| above.and_then(|line| line.get(at)))
                    .copied()
                    .unwrap_or(0);
                let predicted = match filter {
                    1 => left,
                    2 => up,
                    3 => super::average(left, up),
                    4 => super::paeth(left, up, up_left),
                    _ => 0,
                };
                raw.push(value.wrapping_sub(predicted));
            }
        }
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        let compressed = encoder
            .write_all(&raw)
            .and_then(|()| encoder.finish())
            .unwrap_or_default();
        let mut header = width.to_be_bytes().to_vec();
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&fields);
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend(chunk(*b"IHDR", &header));
        bytes.extend(chunk(*b"tEXt", b"Software\0test"));
        bytes.extend(chunk(*b"IDAT", &compressed));
        bytes.extend(chunk(*b"IEND", b""));
        bytes
    }

    fn sample(width: u32, height: u32) -> Vec<u8> {
        (0..width * height * 3)
            .map(|index| u8::try_from((index * 37 + 11) % 251).unwrap_or(0))
            .collect()
    }

    #[test]
    fn every_filter_decodes_to_the_same_pixels() -> TestResult {
        let pixels = sample(5, 4);
        for filter in 0..=4 {
            let image = decode_rgb8(&encode(5, 4, &pixels, filter, [8, 2, 0, 0, 0]))?;
            assert_eq!((image.width(), image.height()), (5, 4));
            assert_eq!(image.pixels(), pixels.as_slice(), "filter {filter}");
        }
        Ok(())
    }

    #[test]
    fn another_profile_is_refused_not_converted() {
        let pixels = sample(2, 2);
        for fields in [
            [16, 2, 0, 0, 0],
            [8, 6, 0, 0, 0],
            [8, 3, 0, 0, 0],
            [8, 2, 0, 0, 1],
        ] {
            assert_eq!(
                decode_rgb8(&encode(2, 2, &pixels, 0, fields)),
                Err(PngError::Unsupported)
            );
        }
    }

    #[test]
    fn a_damaged_file_is_refused() {
        let good = encode(2, 2, &sample(2, 2), 1, [8, 2, 0, 0, 0]);
        let mut crc = good.clone();
        if let Some(byte) = crc.get_mut(40) {
            *byte ^= 0x40;
        }
        let mut trailing = good.clone();
        trailing.push(0);
        let short = good.get(..good.len() - 3).unwrap_or_default().to_vec();
        let bad_filter = encode(2, 2, &sample(2, 2), 7, [8, 2, 0, 0, 0]);
        for bytes in [
            Vec::new(),
            b"not a png".to_vec(),
            crc,
            trailing,
            short,
            bad_filter,
        ] {
            assert!(decode_rgb8(&bytes).is_err());
        }
    }

    #[test]
    fn a_frame_beyond_the_pixel_bound_is_refused_before_inflating() {
        let header = [
            &8193_u32.to_be_bytes()[..],
            &8193_u32.to_be_bytes()[..],
            &[8, 2, 0, 0, 0][..],
        ]
        .concat();
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend(chunk(*b"IHDR", &header));
        bytes.extend(chunk(*b"IEND", b""));
        assert_eq!(decode_rgb8(&bytes), Err(PngError::TooLarge));
    }

    #[test]
    fn a_stream_that_inflates_past_its_rows_is_refused() {
        // The header says two rows; the data holds three.
        let long = encode(2, 2, &sample(2, 3), 0, [8, 2, 0, 0, 0]);
        assert_eq!(decode_rgb8(&long), Err(PngError::Invalid));
    }
}
