use std::io::Write;

use flate2::Compression;
use flate2::Crc;
use flate2::write::ZlibEncoder;
use vsift_domain::FrameDimensions;

use super::{PngPixelsError, decode_png_pixels, paeth};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_be_bytes());
    bytes.extend_from_slice(&kind);
    bytes.extend_from_slice(data);
    let mut crc = Crc::new();
    crc.update(&kind);
    crc.update(data);
    bytes.extend_from_slice(&crc.sum().to_be_bytes());
    bytes
}

/// A PNG of `width` x `height` whose image data is `stream`, cut into
/// `IDAT` chunks of at most `part` bytes.
fn png(width: u32, height: u32, stream: &[u8], part: usize) -> Vec<u8> {
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(chunk(*b"IHDR", &header));
    bytes.extend(chunk(*b"tEXt", b"Software\0test"));
    for piece in stream.chunks(part.max(1)) {
        bytes.extend(chunk(*b"IDAT", piece));
    }
    bytes.extend(chunk(*b"IEND", &[]));
    bytes
}

fn deflate(raw: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(raw)?;
    encoder.finish()
}

/// Applies PNG filter `filter` to `row` against the unfiltered row above.
fn filtered(filter: u8, row: &[u8], above: &[u8]) -> Vec<u8> {
    let at =
        |bytes: &[u8], index: Option<usize>| index.and_then(|i| bytes.get(i)).copied().unwrap_or(0);
    row.iter()
        .enumerate()
        .map(|(index, value)| {
            let left = at(row, index.checked_sub(3));
            let up = at(above, Some(index));
            let up_left = at(above, index.checked_sub(3));
            let prediction = match filter {
                1 => left,
                2 => up,
                3 => left.midpoint(up),
                4 => paeth(left, up, up_left),
                _ => 0,
            };
            value.wrapping_sub(prediction)
        })
        .collect()
}

/// A deterministic image: each byte a mix of its position.
fn image(width: u32, height: u32) -> Vec<u8> {
    let length = usize::try_from(width * height * 3).unwrap_or(0);
    (0..length)
        .map(|index| u8::try_from((index * 37 + index / 7) % 251).unwrap_or(0))
        .collect()
}

/// Encodes `pixels` with the filter `filters[row % filters.len()]` per row.
fn encode(width: u32, pixels: &[u8], filters: &[u8]) -> Vec<u8> {
    let row_bytes = usize::try_from(width * 3).unwrap_or(1);
    let mut raw = Vec::new();
    let mut above = vec![0_u8; row_bytes];
    for (row, line) in pixels.chunks(row_bytes).enumerate() {
        let filter = filters
            .get(row % filters.len().max(1))
            .copied()
            .unwrap_or(0);
        raw.push(filter);
        raw.extend(filtered(filter, line, &above));
        above = line.to_vec();
    }
    raw
}

#[test]
fn every_filter_round_trips() -> TestResult {
    let (width, height) = (7, 5);
    let pixels = image(width, height);
    let dimensions = FrameDimensions::new(width, height)?;
    for filters in [&[0][..], &[1], &[2], &[3], &[4], &[0, 1, 2, 3, 4]] {
        let bytes = png(
            width,
            height,
            &deflate(&encode(width, &pixels, filters))?,
            9,
        );
        assert_eq!(
            decode_png_pixels(&bytes, dimensions)?,
            pixels,
            "filters {filters:?}"
        );
    }
    Ok(())
}

#[test]
fn an_unknown_filter_is_refused() -> TestResult {
    let pixels = image(4, 2);
    let mut raw = encode(4, &pixels, &[0]);
    if let Some(filter) = raw.first_mut() {
        *filter = 5;
    }
    let bytes = png(4, 2, &deflate(&raw)?, 64);
    assert_eq!(
        decode_png_pixels(&bytes, FrameDimensions::new(4, 2)?),
        Err(PngPixelsError::Invalid)
    );
    Ok(())
}

#[test]
fn image_data_must_be_exactly_the_image() -> TestResult {
    let pixels = image(4, 2);
    let raw = encode(4, &pixels, &[0]);
    let dimensions = FrameDimensions::new(4, 2)?;
    let mut longer = raw.clone();
    longer.push(0);
    let shorter = raw.get(..raw.len() - 1).unwrap_or_default();
    for stream in [deflate(&longer)?, deflate(shorter)?] {
        assert_eq!(
            decode_png_pixels(&png(4, 2, &stream, 64), dimensions),
            Err(PngPixelsError::Invalid)
        );
    }
    // A stream whose checksum is broken.
    let mut broken = deflate(&raw)?;
    if let Some(last) = broken.last_mut() {
        *last ^= 1;
    }
    assert_eq!(
        decode_png_pixels(&png(4, 2, &broken, 64), dimensions),
        Err(PngPixelsError::Invalid)
    );
    Ok(())
}

#[test]
fn another_size_or_a_broken_chunk_is_refused() -> TestResult {
    let pixels = image(4, 2);
    let bytes = png(4, 2, &deflate(&encode(4, &pixels, &[0]))?, 64);
    assert_eq!(
        decode_png_pixels(&bytes, FrameDimensions::new(4, 3)?),
        Err(PngPixelsError::Invalid)
    );
    let mut broken = bytes.clone();
    if let Some(byte) = broken.get_mut(40) {
        *byte ^= 0x40;
    }
    assert_eq!(
        decode_png_pixels(&broken, FrameDimensions::new(4, 2)?),
        Err(PngPixelsError::Invalid)
    );
    let mut trailing = bytes;
    trailing.push(0);
    assert_eq!(
        decode_png_pixels(&trailing, FrameDimensions::new(4, 2)?),
        Err(PngPixelsError::Invalid)
    );
    Ok(())
}

#[test]
fn a_frame_too_large_is_refused_before_inflating() -> TestResult {
    // 5000 x 4000 is 20 megapixels. Its image data is not a zlib stream at
    // all, so the answer is TooLarge only if nothing was inflated.
    let bytes = png(5000, 4000, b"not a zlib stream", 64);
    assert_eq!(
        decode_png_pixels(&bytes, FrameDimensions::new(5000, 4000)?),
        Err(PngPixelsError::TooLarge)
    );
    let huge = vec![0_u8; super::MAX_FRAME_BYTES + 1];
    assert_eq!(
        decode_png_pixels(&huge, FrameDimensions::new(4, 2)?),
        Err(PngPixelsError::TooLarge)
    );
    Ok(())
}

#[test]
fn paeth_follows_the_specification() {
    assert_eq!(paeth(10, 20, 15), 15);
    assert_eq!(paeth(10, 10, 10), 10);
    assert_eq!(paeth(200, 10, 5), 200);
    assert_eq!(paeth(5, 200, 10), 200);
}
