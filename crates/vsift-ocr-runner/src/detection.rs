//! Text detection: the detector's input tensor, and its probability map turned into boxes.
//!
//! The detector (`PP-OCR`'s DB model) answers, for every pixel of a resized copy of
//! the frame, the probability that it lies on text. The boxes are the connected
//! regions of that map above a threshold, kept when their mean probability is
//! high enough and grown back to the text's full height ("unclipped"), as the
//! model was trained to expect. Screen text is upright, so the boxes are upright
//! rectangles; rotated text is not a goal of this runner (ADR 0030).

use std::fmt;

use crate::{
    image::{Rect, RgbImage, floor_index, index_to_f32, ratio, round_index},
    tensor::Tensor,
};

/// The detector's settings: those of the pipeline the evaluation measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectionSettings {
    /// The shorter side is enlarged to at least this many pixels.
    pub limit_side: usize,
    /// A frame whose longer side exceeds this is first reduced to it.
    pub max_side: usize,
    /// A map pixel is text above this probability.
    pub threshold: f32,
    /// A region is kept when its mean probability reaches this.
    pub box_threshold: f32,
    /// Most regions considered per frame.
    pub max_candidates: usize,
    /// How far a region is grown, relative to its area over its perimeter.
    pub unclip_ratio: f32,
    /// Boxes whose tops lie within this many pixels are one line, read left to right.
    pub line_tolerance: usize,
}

impl DetectionSettings {
    /// `PP-OCRv5`'s settings as the evaluation ran them.
    pub const PP_OCR_V5: Self = Self {
        limit_side: 736,
        max_side: 2000,
        threshold: 0.3,
        box_threshold: 0.5,
        max_candidates: 1000,
        unclip_ratio: 1.6,
        line_tolerance: 10,
    };
}

/// Why detection could not run or its output could not be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetectionError {
    /// The frame is too small or too large to resize for the detector.
    InvalidSize,
    /// The detector's output is not a probability map of the expected shape.
    InvalidOutput,
}

impl fmt::Display for DetectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSize => "the frame cannot be resized for the text detector",
            Self::InvalidOutput => {
                "the text detector's output is not a probability map of the expected shape"
            }
        })
    }
}

impl std::error::Error for DetectionError {}

/// The sizes needed to map the detector's boxes back onto the frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DetectionScale {
    frame_width: usize,
    frame_height: usize,
    map_width: usize,
    map_height: usize,
}

/// A detected box on the frame and its mean text probability.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectedBox {
    /// The box in the frame's own pixels.
    pub rect: Rect,
    /// The mean probability of text inside the region (0 to 1).
    pub score: f32,
}

/// Most pixels of the detector's map, checked before any of it is reserved:
/// 8 Mi pixels, a 96 MiB input tensor. [`detection_map_size`] keeps every
/// map within `max_side`² (4 M pixels) before its sides are rounded to
/// multiples of 32, so only a frame of an aspect ratio of about 1,000 to 1
/// or more, whose shorter side rounds up from 16 pixels to 32, can reach it.
pub const MAX_DETECTION_MAP_PIXELS: usize = 8 * 1024 * 1024;

/// The detector's map size for a frame of `width` × `height`: the frame
/// reduced so its longer side is at most `max_side`, then enlarged so its
/// shorter side reaches `limit_side`, each side rounded to a multiple of 32.
///
/// The enlargement is capped so the map's area stays within `max_side`²
/// (RV-2 finding 1). Without the cap a frame of an extreme aspect ratio,
/// which the PNG decoder's 16-megapixel bound lets through (128 × 125,000,
/// say), was enlarged from a 2-pixel shorter side to 736, a map of about 500
/// million pixels and gigabytes of tensors. The cap was chosen over refusing
/// such a frame, and over capping the longer side at `max_side`, because it
/// leaves the measured pipeline's map unchanged for every frame of an aspect
/// ratio up to about 7.4 to 1 (ultra-wide 32:9 screens included) while it
/// bounds the memory of every other frame as the reduction bounds a large one.
///
/// # Errors
///
/// [`DetectionError::InvalidSize`] when a side would be zero, or the map
/// would exceed [`MAX_DETECTION_MAP_PIXELS`].
pub fn detection_map_size(
    width: usize,
    height: usize,
    settings: &DetectionSettings,
) -> Result<(usize, usize), DetectionError> {
    let longer = width.max(height);
    let reduce = if longer > settings.max_side {
        ratio(settings.max_side, longer)
    } else {
        1.0
    };
    let reduced = |length: usize| floor_index(index_to_f32(length) * reduce);
    let (reduced_width, reduced_height) = (reduced(width), reduced(height));
    if reduced_width == 0 || reduced_height == 0 {
        return Err(DetectionError::InvalidSize);
    }
    let shorter = index_to_f32(width.min(height)) * reduce;
    let wanted = if shorter < index_to_f32(settings.limit_side) {
        index_to_f32(settings.limit_side) / shorter
    } else {
        1.0
    };
    // Both reduced sides are at most `max_side`, so the cap is never below 1.
    let ceiling = index_to_f32(settings.max_side) * index_to_f32(settings.max_side);
    let cap = (ceiling / (index_to_f32(reduced_width) * index_to_f32(reduced_height))).sqrt();
    let enlarge = wanted.min(cap);
    let side = |reduced: usize| {
        let scaled = floor_index(index_to_f32(reduced) * enlarge);
        round_index(round_half_even(index_to_f32(scaled) / 32.0)) * 32
    };
    let (map_width, map_height) = (side(reduced_width), side(reduced_height));
    let within = map_width
        .checked_mul(map_height)
        .is_some_and(|pixels| pixels <= MAX_DETECTION_MAP_PIXELS);
    if map_width == 0 || map_height == 0 || !within {
        return Err(DetectionError::InvalidSize);
    }
    Ok((map_width, map_height))
}

/// The detector's input for `frame`: BGR, normalised to -1..1, sides multiples of 32.
///
/// # Errors
///
/// [`DetectionError::InvalidSize`] when the map cannot be sized ([`detection_map_size`]).
pub fn detection_input(
    frame: &RgbImage,
    settings: &DetectionSettings,
) -> Result<(Tensor, DetectionScale), DetectionError> {
    let (width, height) = (frame.width(), frame.height());
    let (map_width, map_height) = detection_map_size(width, height, settings)?;
    let resized = frame
        .resized(map_width, map_height)
        .map_err(|_| DetectionError::InvalidSize)?;
    let tensor = bgr_tensor(&[&resized], map_width).map_err(|_| DetectionError::InvalidSize)?;
    Ok((
        tensor,
        DetectionScale {
            frame_width: width,
            frame_height: height,
            map_width,
            map_height,
        },
    ))
}

/// One NCHW tensor of `images`, BGR, each normalised to -1..1 and padded on the
/// right with zeros to `width` (every image must be `width` or narrower and of one height).
pub(crate) fn bgr_tensor(
    images: &[&RgbImage],
    width: usize,
) -> Result<Tensor, crate::tensor::ShapeError> {
    let height = images.first().map_or(0, |image| image.height());
    let plane = height * width;
    let mut data = vec![0.0_f32; images.len() * 3 * plane];
    for (index, image) in images.iter().enumerate() {
        if image.height() != height || image.width() > width {
            return Err(crate::tensor::ShapeError);
        }
        for (channel, source) in [2_usize, 1, 0].into_iter().enumerate() {
            let base = (index * 3 + channel) * plane;
            for y in 0..height {
                for x in 0..image.width() {
                    if let Some(slot) = data.get_mut(base + y * width + x) {
                        *slot = f32::from(image.sample(x, y, source)) / 127.5 - 1.0;
                    }
                }
            }
        }
    }
    Tensor::new(vec![images.len(), 3, height, width], data)
}

/// The boxes of `map`, the detector's output for the input `scale` describes, in reading order.
///
/// # Errors
///
/// [`DetectionError::InvalidOutput`] when `map` is not `[1, 1, height, width]` of the input.
pub fn detected_boxes(
    map: &Tensor,
    scale: &DetectionScale,
    settings: &DetectionSettings,
) -> Result<Vec<DetectedBox>, DetectionError> {
    if map.shape() != [1, 1, scale.map_height, scale.map_width] {
        return Err(DetectionError::InvalidOutput);
    }
    let (width, height) = (scale.map_width, scale.map_height);
    let probabilities = map.data();
    if probabilities.iter().any(|value| !value.is_finite()) {
        return Err(DetectionError::InvalidOutput);
    }
    let text: Vec<bool> = probabilities
        .iter()
        .map(|value| *value > settings.threshold)
        .collect();
    let dilated = dilate(&text, width, height);
    let mut boxes = Vec::new();
    for region in regions(&dilated, width, height)
        .into_iter()
        .take(settings.max_candidates)
    {
        if let Some(detected) = keep_region(region, probabilities, scale, settings) {
            boxes.push(detected);
        }
    }
    Ok(reading_order(boxes, settings.line_tolerance))
}

/// A region's inclusive pixel bounds on the map.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Region {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}

fn keep_region(
    region: Region,
    probabilities: &[f32],
    scale: &DetectionScale,
    settings: &DetectionSettings,
) -> Option<DetectedBox> {
    let width = index_to_f32(region.right - region.left);
    let height = index_to_f32(region.bottom - region.top);
    if width.min(height) < 3.0 {
        return None;
    }
    let score = mean_inside(probabilities, scale.map_width, region);
    if score < settings.box_threshold {
        return None;
    }
    let distance = width * height * settings.unclip_ratio / (2.0 * (width + height));
    if (width + 2.0 * distance).min(height + 2.0 * distance) < 5.0 {
        return None;
    }
    let to_frame_x = index_to_f32(scale.frame_width) / index_to_f32(scale.map_width);
    let to_frame_y = index_to_f32(scale.frame_height) / index_to_f32(scale.map_height);
    let last_x = index_to_f32(scale.frame_width - 1);
    let last_y = index_to_f32(scale.frame_height - 1);
    let x0 =
        round_half_even((index_to_f32(region.left) - distance) * to_frame_x).clamp(0.0, last_x);
    let x1 =
        round_half_even((index_to_f32(region.right) + distance) * to_frame_x).clamp(0.0, last_x);
    let y0 = round_half_even((index_to_f32(region.top) - distance) * to_frame_y).clamp(0.0, last_y);
    let y1 =
        round_half_even((index_to_f32(region.bottom) + distance) * to_frame_y).clamp(0.0, last_y);
    let rect = Rect {
        x: round_index(x0),
        y: round_index(y0),
        width: round_index(x1 - x0),
        height: round_index(y1 - y0),
    };
    if rect.width <= 3 || rect.height <= 3 {
        return None;
    }
    Some(DetectedBox { rect, score })
}

fn mean_inside(probabilities: &[f32], width: usize, region: Region) -> f32 {
    let mut sum = 0.0_f64;
    let mut count = 0_u32;
    for y in region.top..=region.bottom {
        for x in region.left..=region.right {
            if let Some(value) = probabilities.get(y * width + x) {
                sum += f64::from(*value);
                count = count.saturating_add(1);
            }
        }
    }
    if count == 0 {
        return 0.0;
    }
    narrow(sum / f64::from(count))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a mean of probabilities lies in 0..=1"
)]
fn narrow(value: f64) -> f32 {
    value as f32
}

/// The map dilated by a 2x2 square: a pixel is set when it or its neighbour to the
/// left, above or above-left is.
fn dilate(text: &[bool], width: usize, height: usize) -> Vec<bool> {
    let at = |x: usize, y: usize| text.get(y * width + x).copied().unwrap_or(false);
    let mut dilated = vec![false; text.len()];
    for y in 0..height {
        for x in 0..width {
            let left = x > 0 && at(x - 1, y);
            let up = y > 0 && at(x, y - 1);
            let up_left = x > 0 && y > 0 && at(x - 1, y - 1);
            if let Some(slot) = dilated.get_mut(y * width + x) {
                *slot = at(x, y) || left || up || up_left;
            }
        }
    }
    dilated
}

/// The 8-connected regions of `set`, in the order their first pixel is met row by row.
fn regions(set: &[bool], width: usize, height: usize) -> Vec<Region> {
    let mut seen = vec![false; set.len()];
    let mut found = Vec::new();
    let mut stack = Vec::new();
    for start in 0..set.len() {
        if !set.get(start).copied().unwrap_or(false) || seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut region = Region {
            left: start % width,
            top: start / width,
            right: start % width,
            bottom: start / width,
        };
        if let Some(mark) = seen.get_mut(start) {
            *mark = true;
        }
        stack.push(start);
        while let Some(index) = stack.pop() {
            let (x, y) = (index % width, index / width);
            region.left = region.left.min(x);
            region.right = region.right.max(x);
            region.top = region.top.min(y);
            region.bottom = region.bottom.max(y);
            for (dx, dy) in [
                (-1_isize, -1_isize),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                let (Some(nx), Some(ny)) = (x.checked_add_signed(dx), y.checked_add_signed(dy))
                else {
                    continue;
                };
                if nx >= width || ny >= height {
                    continue;
                }
                let next = ny * width + nx;
                if set.get(next).copied().unwrap_or(false)
                    && !seen.get(next).copied().unwrap_or(true)
                {
                    if let Some(mark) = seen.get_mut(next) {
                        *mark = true;
                    }
                    stack.push(next);
                }
            }
        }
        found.push(region);
    }
    found
}

/// Top to bottom; boxes whose tops lie within `tolerance` of the one before are a line, left to right.
fn reading_order(mut boxes: Vec<DetectedBox>, tolerance: usize) -> Vec<DetectedBox> {
    boxes.sort_by_key(|detected| detected.rect.y);
    let mut line = 0_usize;
    let mut previous_top = None;
    let mut keyed: Vec<(usize, usize, DetectedBox)> = Vec::with_capacity(boxes.len());
    for detected in boxes {
        if previous_top.is_some_and(|top| detected.rect.y - top >= tolerance) {
            line += 1;
        }
        previous_top = Some(detected.rect.y);
        keyed.push((line, detected.rect.x, detected));
    }
    keyed.sort_by_key(|(line, x, _)| (*line, *x));
    keyed.into_iter().map(|(_, _, detected)| detected).collect()
}

/// Rounds to the nearest whole number, halves to even, as the measured pipeline did.
#[must_use]
pub fn round_half_even(value: f32) -> f32 {
    let rounded = value.round();
    // A value exactly halfway rounds away from zero; step back when that is odd.
    let halfway = ((value - value.trunc()).abs() - 0.5).abs() < f32::EPSILON;
    if halfway && (rounded % 2.0).abs() > 0.5 {
        rounded - value.signum()
    } else {
        rounded
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DetectionError, DetectionSettings, MAX_DETECTION_MAP_PIXELS, Tensor, detected_boxes,
        detection_input, detection_map_size, round_half_even,
    };
    use crate::image::{Rect, RgbImage};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn a_720p_frame_is_enlarged_to_a_multiple_of_32_with_its_short_side_at_least_736() -> TestResult
    {
        let frame = RgbImage::new(1280, 720, vec![255; 1280 * 720 * 3])?;
        let (tensor, _) = detection_input(&frame, &DetectionSettings::PP_OCR_V5)?;
        assert_eq!(tensor.shape(), &[1, 3, 736, 1312]);
        // White normalises to 1.
        assert!(
            tensor
                .data()
                .iter()
                .all(|value| (*value - 1.0).abs() < 1e-6)
        );
        let large = RgbImage::new(1280, 1024, vec![0; 1280 * 1024 * 3])?;
        assert_eq!(
            detection_input(&large, &DetectionSettings::PP_OCR_V5)?
                .0
                .shape(),
            &[1, 3, 1024, 1280]
        );
        Ok(())
    }

    /// RV-2 finding 1: a frame the PNG decoder accepts (16.0 megapixels) but of
    /// an extreme aspect ratio was enlarged to a map of about 500 million
    /// pixels. Its map is now within `max_side`² before rounding, and within
    /// the fixed bound after; an ordinary or ultra-wide screen's map is the
    /// measured pipeline's, uncapped.
    #[test]
    fn a_frame_of_an_extreme_aspect_ratio_gets_a_bounded_map() -> TestResult {
        let settings = DetectionSettings::PP_OCR_V5;
        for (width, height) in [(128, 125_000), (125_000, 128), (1, 2000), (4, 4_000_000)] {
            match detection_map_size(width, height, &settings) {
                Ok((map_width, map_height)) => {
                    let pixels = map_width * map_height;
                    assert!(
                        pixels <= MAX_DETECTION_MAP_PIXELS,
                        "{width} x {height}: {map_width} x {map_height}"
                    );
                }
                Err(error) => assert_eq!(error, DetectionError::InvalidSize),
            }
        }
        assert_eq!(
            detection_map_size(128, 125_000, &settings)?,
            (64, 63_232),
            "the 2 x 2000 reduction is enlarged only to 4 M pixels"
        );
        assert_eq!(detection_map_size(1280, 720, &settings)?, (1312, 736));
        assert_eq!(detection_map_size(3840, 1080, &settings)?, (2624, 736));
        assert_eq!(detection_map_size(3840, 2160, &settings)?, (1984, 1120));
        Ok(())
    }

    #[test]
    fn channels_are_handed_over_blue_first() -> TestResult {
        let frame = RgbImage::new(1, 1, vec![255, 0, 0])?;
        let tensor = super::bgr_tensor(&[&frame], 2)?;
        assert_eq!(tensor.shape(), &[1, 3, 1, 2]);
        // Blue -1, padding 0; green -1, padding 0; red 1, padding 0.
        assert_eq!(tensor.data(), &[-1.0, 0.0, -1.0, 0.0, 1.0, 0.0]);
        Ok(())
    }

    fn map_with(
        width: usize,
        height: usize,
        blocks: &[(usize, usize, usize, usize, f32)],
    ) -> Result<Tensor, Box<dyn std::error::Error>> {
        let mut data = vec![0.0_f32; width * height];
        for &(left, top, right, bottom, value) in blocks {
            for y in top..=bottom {
                for x in left..=right {
                    if let Some(slot) = data.get_mut(y * width + x) {
                        *slot = value;
                    }
                }
            }
        }
        Ok(Tensor::new(vec![1, 1, height, width], data)?)
    }

    #[test]
    fn regions_become_grown_boxes_in_reading_order() -> TestResult {
        let frame = RgbImage::new(736, 736, vec![0; 736 * 736 * 3])?;
        let (_, scale) = detection_input(&frame, &DetectionSettings::PP_OCR_V5)?;
        // Two words on one line (the right one a few pixels higher) and one line below.
        let map = map_with(
            736,
            736,
            &[
                (300, 102, 360, 112, 0.9),
                (100, 100, 200, 110, 0.9),
                (100, 300, 400, 320, 0.8),
            ],
        )?;
        let boxes = detected_boxes(&map, &scale, &DetectionSettings::PP_OCR_V5)?;
        let xs: Vec<usize> = boxes.iter().map(|detected| detected.rect.x).collect();
        assert_eq!(boxes.len(), 3);
        assert!(xs[0] < xs[1], "the first line reads left to right: {xs:?}");
        assert!(boxes[2].rect.y > boxes[0].rect.y);
        // The region (100..=200 after dilation, 100..=110 plus one) grows by area * 1.6 / perimeter.
        let first = boxes[0].rect;
        assert!(
            first.x < 100
                && first.y < 100
                && first.x + first.width > 201
                && first.y + first.height > 111,
            "{first:?}"
        );
        // The mean is taken over the dilated region, one pixel wider and taller than the
        // marked block, as the measured pipeline scores it: 0.9 * 101 * 11 / (102 * 12).
        assert!((boxes[0].score - 0.817).abs() < 0.01, "{}", boxes[0].score);
        Ok(())
    }

    #[test]
    fn faint_tiny_and_malformed_regions_are_dropped() -> TestResult {
        let frame = RgbImage::new(736, 736, vec![0; 736 * 736 * 3])?;
        let (_, scale) = detection_input(&frame, &DetectionSettings::PP_OCR_V5)?;
        let faint = map_with(736, 736, &[(100, 100, 200, 110, 0.35)])?;
        let tiny = map_with(736, 736, &[(100, 100, 101, 130, 0.9)])?;
        assert_eq!(
            detected_boxes(&faint, &scale, &DetectionSettings::PP_OCR_V5)?.len(),
            0
        );
        assert_eq!(
            detected_boxes(&tiny, &scale, &DetectionSettings::PP_OCR_V5)?.len(),
            0
        );
        let wrong_shape = Tensor::new(vec![1, 1, 10, 10], vec![0.0; 100])?;
        assert!(detected_boxes(&wrong_shape, &scale, &DetectionSettings::PP_OCR_V5).is_err());
        let not_a_number = map_with(736, 736, &[(1, 1, 1, 1, f32::NAN)])?;
        assert!(detected_boxes(&not_a_number, &scale, &DetectionSettings::PP_OCR_V5).is_err());
        Ok(())
    }

    #[test]
    fn boxes_are_mapped_back_onto_the_frame_and_clamped_to_it() -> TestResult {
        let frame = RgbImage::new(1280, 720, vec![0; 1280 * 720 * 3])?;
        let (_, scale) = detection_input(&frame, &DetectionSettings::PP_OCR_V5)?;
        let map = map_with(1312, 736, &[(1250, 700, 1311, 735, 0.9)])?;
        let boxes = detected_boxes(&map, &scale, &DetectionSettings::PP_OCR_V5)?;
        let Some(only) = boxes.first() else {
            return Err("no box".into());
        };
        assert!(
            only.rect.x + only.rect.width <= 1279 && only.rect.y + only.rect.height <= 719,
            "{:?}",
            only.rect
        );
        assert_ne!(
            only.rect,
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0
            }
        );
        Ok(())
    }

    #[test]
    fn halves_round_to_even() {
        assert_eq!(
            [
                round_half_even(22.5),
                round_half_even(23.5),
                round_half_even(2.4),
                round_half_even(-0.5)
            ],
            [22.0, 24.0, 2.0, 0.0]
        );
    }
}
