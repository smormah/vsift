//! Pixels of one stored evidence PNG, for comparing two frames (ADR 0034).
//!
//! The walker in `png.rs` splits `FFmpeg`'s output and never decodes it. To
//! say where two cited frames differ, `VSift` reads their pixels itself, from
//! the very images it delivered, so anyone can check a region by comparing
//! the two files. The input is still provider output, so the decoder accepts
//! only what the evidence profile writes: one 8-bit RGB image without
//! interlacing, at the frame's displayed size, every chunk's CRC checked by
//! the same walk, and an image data stream that inflates to exactly one byte
//! per row plus three per pixel, never more. The size is checked from the
//! header before anything is inflated, and rows are inflated and unfiltered
//! one at a time, so the only large buffer is the decoded image itself.

use std::error::Error;
use std::fmt;
use std::io::Read;
use std::path::Path;

use flate2::read::ZlibDecoder;
use sha2::{Digest, Sha256};
use vsift_domain::FrameDimensions;

use super::png::{chunk_at, image_length};
use super::{MAX_FRAME_BYTES, MAX_FRAME_PIXELS};

const SIGNATURE_LENGTH: usize = 8;
/// RGB: three bytes per pixel, which is also the filters' byte distance.
const BYTES_PER_PIXEL: usize = 3;

/// Why a stored image's pixels could not be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PngPixelsError {
    /// The image is larger than any frame the extraction accepts (more than
    /// 16,000,000 pixels, or a file over 64 MiB); nothing was inflated.
    TooLarge,
    /// The bytes are not one image of the expected size and form.
    Invalid,
    /// The stored file could not be read, or its bytes no longer match the
    /// digest it was delivered with.
    Changed,
}

impl fmt::Display for PngPixelsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "the image is larger than any extracted frame",
            Self::Invalid => "the image is not an extracted frame",
            Self::Changed => "the stored image changed since it was delivered",
        })
    }
}

impl Error for PngPixelsError {}

/// Reads a delivered evidence PNG and decodes its pixels, after checking
/// its bytes against `sha256`, the lowercase hex digest it was delivered
/// with (ADR 0034 section 3).
///
/// # Errors
///
/// Returns [`PngPixelsError::TooLarge`] for a file over 64 MiB, read no
/// further; [`PngPixelsError::Changed`] for a file that cannot be read or
/// whose digest differs; and the errors of [`decode_png_pixels`].
pub fn read_png_pixels(
    path: &Path,
    sha256: &str,
    dimensions: FrameDimensions,
) -> Result<Vec<u8>, PngPixelsError> {
    // Read at most one byte past the bound, so the bound holds even if the
    // file grew after any earlier look at its size.
    let limit = u64::try_from(MAX_FRAME_BYTES).map_err(|_| PngPixelsError::TooLarge)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(limit.saturating_add(1)).read_to_end(&mut bytes))
        .map_err(|_| PngPixelsError::Changed)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(PngPixelsError::TooLarge);
    }
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(&bytes) {
        digest.push(char::from(HEX[usize::from(byte >> 4)]));
        digest.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    if digest != sha256 {
        return Err(PngPixelsError::Changed);
    }
    decode_png_pixels(&bytes, dimensions)
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Decodes the 8-bit RGB pixels of one evidence PNG of `dimensions`.
///
/// Published beside the PNG walker for fuzzing (ADR 0016, decision 6). The
/// result holds exactly `width * height * 3` bytes, row by row.
///
/// # Errors
///
/// Returns [`PngPixelsError::TooLarge`] for a file over 64 MiB or a header
/// over 16,000,000 pixels, checked before any inflate, and
/// [`PngPixelsError::Invalid`] for anything else: another size or form, a
/// broken chunk, image data that inflates to more or less than the image, or
/// an unknown row filter.
pub fn decode_png_pixels(
    bytes: &[u8],
    dimensions: FrameDimensions,
) -> Result<Vec<u8>, PngPixelsError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(PngPixelsError::TooLarge);
    }
    let (header_width, header_height) = header_size(bytes)?;
    if u64::from(header_width) * u64::from(header_height) > MAX_FRAME_PIXELS {
        return Err(PngPixelsError::TooLarge);
    }
    if image_length(bytes, dimensions).map_err(|_| PngPixelsError::Invalid)? != bytes.len() {
        return Err(PngPixelsError::Invalid);
    }
    let width = usize::try_from(dimensions.width()).map_err(|_| PngPixelsError::TooLarge)?;
    let height = usize::try_from(dimensions.height()).map_err(|_| PngPixelsError::TooLarge)?;
    let row_bytes = width
        .checked_mul(BYTES_PER_PIXEL)
        .ok_or(PngPixelsError::TooLarge)?;
    let total = row_bytes
        .checked_mul(height)
        .ok_or(PngPixelsError::TooLarge)?;
    let mut decoder = ZlibDecoder::new(ImageData::new(bytes)?);
    let mut pixels = Vec::with_capacity(total);
    let mut previous = vec![0_u8; row_bytes];
    let mut current = vec![0_u8; row_bytes];
    for _ in 0..height {
        let mut filter = [0_u8; 1];
        decoder
            .read_exact(&mut filter)
            .map_err(|_| PngPixelsError::Invalid)?;
        decoder
            .read_exact(&mut current)
            .map_err(|_| PngPixelsError::Invalid)?;
        unfilter(filter[0], &mut current, &previous)?;
        pixels.extend_from_slice(&current);
        std::mem::swap(&mut previous, &mut current);
    }
    // The stream must end here: reading on also checks its Adler-32.
    let mut rest = [0_u8; 1];
    match decoder.read(&mut rest) {
        Ok(0) => Ok(pixels),
        _ => Err(PngPixelsError::Invalid),
    }
}

/// Width and height from the `IHDR` chunk, which the walker requires first.
fn header_size(bytes: &[u8]) -> Result<(u32, u32), PngPixelsError> {
    let field = |offset: usize| {
        bytes
            .get(offset..offset + 4)
            .and_then(|field| <[u8; 4]>::try_from(field).ok())
            .map(u32::from_be_bytes)
            .ok_or(PngPixelsError::Invalid)
    };
    // Signature, then the chunk length (4) and type (4), then the body.
    if bytes.get(SIGNATURE_LENGTH + 4..SIGNATURE_LENGTH + 8) != Some(b"IHDR".as_slice()) {
        return Err(PngPixelsError::Invalid);
    }
    Ok((field(SIGNATURE_LENGTH + 8)?, field(SIGNATURE_LENGTH + 12)?))
}

/// Reverses one row's filter in place, against the decoded row above it.
fn unfilter(filter: u8, row: &mut [u8], above: &[u8]) -> Result<(), PngPixelsError> {
    match filter {
        0 => {}
        1 => {
            for index in BYTES_PER_PIXEL..row.len() {
                let left = byte(row, index - BYTES_PER_PIXEL);
                add(row, index, left);
            }
        }
        2 => {
            for index in 0..row.len() {
                add(row, index, byte(above, index));
            }
        }
        3 => {
            for index in 0..row.len() {
                let left = left_of(row, index);
                // The PNG Average filter: floor((left + up) / 2).
                add(row, index, left.midpoint(byte(above, index)));
            }
        }
        4 => {
            for index in 0..row.len() {
                let left = left_of(row, index);
                let up = byte(above, index);
                let up_left = left_of(above, index);
                add(row, index, paeth(left, up, up_left));
            }
        }
        _ => return Err(PngPixelsError::Invalid),
    }
    Ok(())
}

fn byte(row: &[u8], index: usize) -> u8 {
    row.get(index).copied().unwrap_or(0)
}

fn left_of(row: &[u8], index: usize) -> u8 {
    index
        .checked_sub(BYTES_PER_PIXEL)
        .map_or(0, |left| byte(row, left))
}

fn add(row: &mut [u8], index: usize, value: u8) {
    if let Some(target) = row.get_mut(index) {
        *target = target.wrapping_add(value);
    }
}

/// The PNG specification's Paeth predictor.
fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let estimate = i16::from(left) + i16::from(up) - i16::from(up_left);
    let distance_left = (estimate - i16::from(left)).abs();
    let distance_up = (estimate - i16::from(up)).abs();
    let distance_up_left = (estimate - i16::from(up_left)).abs();
    if distance_left <= distance_up && distance_left <= distance_up_left {
        left
    } else if distance_up <= distance_up_left {
        up
    } else {
        up_left
    }
}

/// The image's consecutive `IDAT` chunk bodies, read as one stream.
struct ImageData<'a> {
    parts: Vec<&'a [u8]>,
    next: usize,
}

impl<'a> ImageData<'a> {
    /// Collects the `IDAT` bodies of an image the walker has accepted.
    fn new(bytes: &'a [u8]) -> Result<Self, PngPixelsError> {
        let mut parts = Vec::new();
        let mut offset = SIGNATURE_LENGTH;
        while offset < bytes.len() {
            let (kind, data, next) =
                chunk_at(bytes, offset).map_err(|_| PngPixelsError::Invalid)?;
            if kind == *b"IDAT" {
                parts.push(data);
            }
            offset = next;
        }
        Ok(Self { parts, next: 0 })
    }
}

impl Read for ImageData<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        while let Some(part) = self.parts.get_mut(self.next) {
            if part.is_empty() {
                self.next += 1;
                continue;
            }
            let taken = part.read(buffer)?;
            return Ok(taken);
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests;
