//! An 8-bit RGB image and the few operations the OCR pipeline needs.
//!
//! Resizing follows the bilinear interpolation of the pipeline the models were
//! measured with (pixel centres aligned, edges clamped), so that the tensors the
//! models see match the evaluation's (`docs/planning/ocr-engine-evaluation.md`).

use std::fmt;

/// An image size or crop that does not fit its image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageError;

impl fmt::Display for ImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the image size or region is invalid")
    }
}

impl std::error::Error for ImageError {}

/// An upright rectangle in pixels: its left, top, width and height.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: usize,
    /// Top edge.
    pub y: usize,
    /// Width, at least one.
    pub width: usize,
    /// Height, at least one.
    pub height: usize,
}

/// Pixels in RGB order, row by row, three bytes a pixel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbImage {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl RgbImage {
    /// An image of `width` x `height` from its RGB bytes.
    ///
    /// # Errors
    ///
    /// [`ImageError`] when a side is zero or `pixels` is not `width * height * 3` bytes.
    pub fn new(width: usize, height: usize, pixels: Vec<u8>) -> Result<Self, ImageError> {
        let expected = width
            .checked_mul(height)
            .and_then(|area| area.checked_mul(3))
            .ok_or(ImageError)?;
        if width == 0 || height == 0 || pixels.len() != expected {
            return Err(ImageError);
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// The RGB bytes.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The sample of `channel` (0 red, 1 green, 2 blue) at (`x`, `y`), clamped to the image.
    #[must_use]
    pub fn sample(&self, x: usize, y: usize, channel: usize) -> u8 {
        let x = x.min(self.width - 1);
        let y = y.min(self.height - 1);
        self.pixels
            .get((y * self.width + x) * 3 + channel.min(2))
            .copied()
            .unwrap_or(0)
    }

    /// The image resized to `width` x `height` by bilinear interpolation.
    ///
    /// # Errors
    ///
    /// [`ImageError`] when a target side is zero or the result would be too large.
    pub fn resized(&self, width: usize, height: usize) -> Result<Self, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError);
        }
        let columns = axis_weights(self.width, width);
        let rows = axis_weights(self.height, height);
        let mut pixels = Vec::with_capacity(
            width
                .checked_mul(height)
                .and_then(|area| area.checked_mul(3))
                .ok_or(ImageError)?,
        );
        for &(y0, y1, wy) in &rows {
            for &(x0, x1, wx) in &columns {
                for channel in 0..3 {
                    let top = lerp(
                        self.sample(x0, y0, channel),
                        self.sample(x1, y0, channel),
                        wx,
                    );
                    let bottom = lerp(
                        self.sample(x0, y1, channel),
                        self.sample(x1, y1, channel),
                        wx,
                    );
                    pixels.push(to_byte(top + (bottom - top) * wy));
                }
            }
        }
        Self::new(width, height, pixels)
    }

    /// The pixels inside `rect`.
    ///
    /// # Errors
    ///
    /// [`ImageError`] when `rect` is empty or reaches past the image.
    pub fn cropped(&self, rect: Rect) -> Result<Self, ImageError> {
        let right = rect.x.checked_add(rect.width).ok_or(ImageError)?;
        let bottom = rect.y.checked_add(rect.height).ok_or(ImageError)?;
        if rect.width == 0 || rect.height == 0 || right > self.width || bottom > self.height {
            return Err(ImageError);
        }
        let mut pixels = Vec::with_capacity(rect.width * rect.height * 3);
        for y in rect.y..bottom {
            let start = (y * self.width + rect.x) * 3;
            pixels.extend_from_slice(
                self.pixels
                    .get(start..start + rect.width * 3)
                    .ok_or(ImageError)?,
            );
        }
        Self::new(rect.width, rect.height, pixels)
    }

    /// The image turned a quarter turn anticlockwise (for a crop much taller than wide).
    #[must_use]
    pub fn rotated_anticlockwise(&self) -> Self {
        let mut pixels = Vec::with_capacity(self.pixels.len());
        for row in 0..self.width {
            let x = self.width - 1 - row;
            for y in 0..self.height {
                for channel in 0..3 {
                    pixels.push(self.sample(x, y, channel));
                }
            }
        }
        Self {
            width: self.height,
            height: self.width,
            pixels,
        }
    }
}

/// For each target position along one axis: the two source positions and the
/// weight of the second, with pixel centres aligned and the edges clamped.
fn axis_weights(source: usize, target: usize) -> Vec<(usize, usize, f32)> {
    let scale = ratio(source, target);
    let last = source - 1;
    (0..target)
        .map(|position| {
            let centre = (index_to_f32(position) + 0.5) * scale - 0.5;
            if centre <= 0.0 {
                return (0, 0, 0.0);
            }
            let low = floor_index(centre);
            if low >= last {
                return (last, last, 0.0);
            }
            (low, low + 1, centre - index_to_f32(low))
        })
        .collect()
}

fn lerp(from: u8, to: u8, weight: f32) -> f32 {
    let from = f32::from(from);
    from + (f32::from(to) - from) * weight
}

/// `numerator / denominator` as a float; both are image sides, far below 2^24.
#[must_use]
pub fn ratio(numerator: usize, denominator: usize) -> f32 {
    index_to_f32(numerator) / index_to_f32(denominator)
}

/// An image coordinate as a float. Sides are bounded by the PNG decoder's 16
/// megapixels, so every coordinate is exact in an `f32` (below 2^24).
#[must_use]
pub fn index_to_f32(value: usize) -> f32 {
    u16::try_from(value).map_or_else(
        |_| u32::try_from(value).map_or(f32::MAX, precise_f32),
        f32::from,
    )
}

#[expect(
    clippy::cast_precision_loss,
    reason = "image coordinates stay below 2^24, where an f32 is exact"
)]
fn precise_f32(value: u32) -> f32 {
    value as f32
}

/// The largest index not above `value`, for a non-negative, finite `value`.
#[must_use]
pub fn floor_index(value: f32) -> usize {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    float_to_index(value.floor())
}

/// The nearest index to `value`, for a non-negative, finite `value`.
#[must_use]
pub fn round_index(value: f32) -> usize {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    float_to_index(value.round())
}

/// The smallest index not below `value`.
#[must_use]
pub fn ceil_index(value: f32) -> usize {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    float_to_index(value.ceil())
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "callers pass a whole, non-negative value; anything above u32::MAX saturates"
)]
fn float_to_index(whole: f32) -> usize {
    usize::try_from(whole.min(4_294_967_040.0) as u32).unwrap_or(usize::MAX)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to 0..=255 and rounded first"
)]
fn to_byte(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::{Rect, RgbImage, ceil_index, floor_index, round_index};

    type TestResult = Result<(), Box<dyn std::error::Error>>;
    type TestResultOf<T> = Result<T, Box<dyn std::error::Error>>;

    fn gradient(width: usize, height: usize) -> TestResultOf<RgbImage> {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                pixels.extend([
                    u8::try_from(x * 10).unwrap_or(255),
                    u8::try_from(y * 10).unwrap_or(255),
                    7,
                ]);
            }
        }
        Ok(RgbImage::new(width, height, pixels)?)
    }

    #[test]
    fn a_size_that_does_not_match_its_bytes_is_refused() {
        assert!(RgbImage::new(2, 2, vec![0; 11]).is_err());
        assert!(RgbImage::new(0, 2, Vec::new()).is_err());
    }

    #[test]
    fn resizing_to_the_same_size_keeps_every_pixel() -> TestResult {
        let image = gradient(6, 4)?;
        assert_eq!(image.resized(6, 4)?, image);
        Ok(())
    }

    #[test]
    fn doubling_interpolates_between_neighbours() -> TestResult {
        let image = gradient(2, 1)?;
        let doubled = image.resized(4, 1)?;
        // Centres at -0.25, 0.25, 0.75 and 1.25 of the source: clamped, a quarter, three quarters, clamped.
        let reds: Vec<u8> = (0..4).map(|x| doubled.sample(x, 0, 0)).collect();
        assert_eq!(reds, vec![0, 3, 8, 10]);
        Ok(())
    }

    #[test]
    fn a_crop_is_the_pixels_inside_its_rectangle() -> TestResult {
        let image = gradient(5, 5)?;
        let crop = image.cropped(Rect {
            x: 1,
            y: 2,
            width: 3,
            height: 2,
        })?;
        assert_eq!((crop.width(), crop.height()), (3, 2));
        assert_eq!((crop.sample(0, 0, 0), crop.sample(0, 0, 1)), (10, 20));
        assert!(
            image
                .cropped(Rect {
                    x: 4,
                    y: 0,
                    width: 2,
                    height: 1
                })
                .is_err()
        );
        assert!(
            image
                .cropped(Rect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 1
                })
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn a_quarter_turn_anticlockwise_moves_the_right_edge_to_the_top() -> TestResult {
        let image = gradient(3, 2)?;
        let turned = image.rotated_anticlockwise();
        assert_eq!((turned.width(), turned.height()), (2, 3));
        assert_eq!(turned.sample(0, 0, 0), image.sample(2, 0, 0));
        assert_eq!(turned.sample(1, 2, 1), image.sample(0, 1, 1));
        Ok(())
    }

    #[test]
    fn float_indices_floor_round_and_ceil_without_going_negative() {
        assert_eq!(
            (floor_index(2.7), round_index(2.5), ceil_index(2.1)),
            (2, 3, 3)
        );
        assert_eq!(
            (floor_index(-1.0), round_index(f32::NAN), ceil_index(-0.5)),
            (0, 0, 0)
        );
    }
}
