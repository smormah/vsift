//! Integer timeline and pixel-geometry contracts.

use std::{error::Error, fmt};

const MICROS_PER_SECOND: i128 = 1_000_000;

/// A non-negative position on the normalized source presentation timeline.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MediaTime(u64);

impl MediaTime {
    /// Creates a timeline position from integer microseconds.
    #[must_use]
    pub const fn from_micros(value: u64) -> Self {
        Self(value)
    }

    /// Returns the normalized timeline position in microseconds.
    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }
}

/// A positive half-open media interval `[start, end)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeRange {
    start: MediaTime,
    end: MediaTime,
}

impl TimeRange {
    /// Creates a non-empty half-open range.
    ///
    /// # Errors
    ///
    /// Returns [`TimeRangeError`] unless `end` is strictly greater than `start`.
    pub fn new(start: MediaTime, end: MediaTime) -> Result<Self, TimeRangeError> {
        if end <= start {
            return Err(TimeRangeError::NotPositive);
        }
        Ok(Self { start, end })
    }

    /// Returns the inclusive start.
    #[must_use]
    pub const fn start(self) -> MediaTime {
        self.start
    }

    /// Returns the exclusive end.
    #[must_use]
    pub const fn end(self) -> MediaTime {
        self.end
    }

    /// Returns the duration in microseconds.
    #[must_use]
    pub const fn duration_micros(self) -> u64 {
        self.end.0 - self.start.0
    }
}

/// Why a media range was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeRangeError {
    /// End must be strictly greater than start.
    NotPositive,
}

impl fmt::Display for TimeRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("time range must have a positive duration")
    }
}

impl Error for TimeRangeError {}

/// Displayed frame dimensions after orientation metadata is applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameDimensions {
    width: u32,
    height: u32,
}

impl FrameDimensions {
    /// Creates positive frame dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError`] when either dimension is zero.
    pub fn new(width: u32, height: u32) -> Result<Self, GeometryError> {
        if width == 0 || height == 0 {
            return Err(GeometryError::ZeroSize);
        }
        Ok(Self { width, height })
    }

    /// Returns the displayed width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the displayed height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Pixel rectangle in an orientation-correct displayed source frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CropRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl CropRect {
    /// Creates a positive rectangle contained by `frame`.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError`] for zero size, overflow, or out-of-frame geometry.
    pub fn new(
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        frame: FrameDimensions,
    ) -> Result<Self, GeometryError> {
        if width == 0 || height == 0 {
            return Err(GeometryError::ZeroSize);
        }
        let right = x.checked_add(width).ok_or(GeometryError::Overflow)?;
        let bottom = y.checked_add(height).ok_or(GeometryError::Overflow)?;
        if right > frame.width || bottom > frame.height {
            return Err(GeometryError::OutsideFrame);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Returns the left coordinate.
    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }

    /// Returns the top coordinate.
    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }

    /// Returns the rectangle width.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the rectangle height.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Why frame dimensions or crop geometry was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryError {
    /// A width or height is zero.
    ZeroSize,
    /// Coordinate arithmetic overflowed.
    Overflow,
    /// The rectangle is not fully contained by the displayed frame.
    OutsideFrame,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ZeroSize => "frame and crop dimensions must be positive",
            Self::Overflow => "crop coordinate arithmetic overflowed",
            Self::OutsideFrame => "crop rectangle is outside the displayed frame",
        };
        formatter.write_str(message)
    }
}

impl Error for GeometryError {}

/// Original stream timestamp and the metadata required to normalize it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamTime {
    /// Zero-based input stream index.
    pub stream_index: u32,
    /// Presentation timestamp in stream time-base units.
    pub presentation_timestamp: i64,
    /// Positive time-base numerator.
    pub time_base_numerator: u32,
    /// Positive time-base denominator.
    pub time_base_denominator: u32,
    /// Signed offset from stream time to normalized source time.
    pub timeline_offset_micros: i64,
}

impl StreamTime {
    /// Converts stream time to the single normalized microsecond timeline.
    ///
    /// # Errors
    ///
    /// Returns [`TimeConversionError`] for invalid bases, overflow, or negative output.
    pub fn to_media_time(self) -> Result<MediaTime, TimeConversionError> {
        if self.time_base_numerator == 0 || self.time_base_denominator == 0 {
            return Err(TimeConversionError::InvalidTimeBase);
        }

        let scaled = i128::from(self.presentation_timestamp)
            .checked_mul(i128::from(self.time_base_numerator))
            .and_then(|value| value.checked_mul(MICROS_PER_SECOND))
            .ok_or(TimeConversionError::Overflow)?;
        let stream_micros = scaled / i128::from(self.time_base_denominator);
        let normalized = stream_micros
            .checked_add(i128::from(self.timeline_offset_micros))
            .ok_or(TimeConversionError::Overflow)?;
        let micros = u64::try_from(normalized).map_err(|_| TimeConversionError::OutsideTimeline)?;
        Ok(MediaTime::from_micros(micros))
    }
}

/// Why stream time could not be normalized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeConversionError {
    /// A rational time base used zero for its numerator or denominator.
    InvalidTimeBase,
    /// Intermediate or output arithmetic exceeded the supported range.
    Overflow,
    /// The normalized timestamp would be before the source timeline.
    OutsideTimeline,
}

impl fmt::Display for TimeConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidTimeBase => "stream time base must be positive",
            Self::Overflow => "stream timestamp conversion overflowed",
            Self::OutsideTimeline => "stream timestamp is outside the normalized timeline",
        };
        formatter.write_str(message)
    }
}

impl Error for TimeConversionError {}

/// Requested and source-observed timestamps for one extracted frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameTiming {
    requested: MediaTime,
    actual: MediaTime,
    delta_micros: i64,
}

impl FrameTiming {
    /// Creates timing metadata without losing a negative or positive delta.
    ///
    /// # Errors
    ///
    /// Returns [`TimeConversionError`] when the signed delta cannot fit in `i64`.
    pub fn new(requested: MediaTime, actual: MediaTime) -> Result<Self, TimeConversionError> {
        let delta = i128::from(actual.as_micros()) - i128::from(requested.as_micros());
        let delta_micros = i64::try_from(delta).map_err(|_| TimeConversionError::Overflow)?;
        Ok(Self {
            requested,
            actual,
            delta_micros,
        })
    }

    /// Returns the caller's requested time.
    #[must_use]
    pub const fn requested(self) -> MediaTime {
        self.requested
    }

    /// Returns the timestamp of the frame actually returned by the provider.
    #[must_use]
    pub const fn actual(self) -> MediaTime {
        self.actual
    }

    /// Returns `actual - requested` in microseconds.
    #[must_use]
    pub const fn delta_micros(self) -> i64 {
        self.delta_micros
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::{prop_assert, prop_assert_eq, proptest};

    use super::{CropRect, FrameDimensions, FrameTiming, MediaTime, StreamTime, TimeRange};

    #[test]
    fn rejects_empty_reversed_and_overflowing_geometry() {
        assert!(TimeRange::new(MediaTime::from_micros(5), MediaTime::from_micros(5)).is_err());
        assert!(TimeRange::new(MediaTime::from_micros(6), MediaTime::from_micros(5)).is_err());
        assert!(FrameDimensions::new(0, 10).is_err());

        let frame = FrameDimensions::new(1920, 1080).ok();
        assert!(frame.is_some());
        if let Some(frame) = frame {
            assert!(CropRect::new(1919, 0, 2, 1, frame).is_err());
            assert!(CropRect::new(u32::MAX, 0, 2, 1, frame).is_err());
            assert!(CropRect::new(0, 0, 0, 1, frame).is_err());
            assert!(CropRect::new(0, 0, 1920, 1080, frame).is_ok());
        }
    }

    #[test]
    fn preserves_requested_and_actual_frame_times() {
        let timing = FrameTiming::new(MediaTime::from_micros(2_000), MediaTime::from_micros(1_750));
        assert_eq!(timing.map(FrameTiming::delta_micros), Ok(-250));
    }

    proptest! {
        #[test]
        fn microsecond_time_base_round_trips(value in 0_u64..=9_223_372_036_854_775_807_u64) {
            let converted = i64::try_from(value);
            prop_assert!(converted.is_ok());
            let Ok(presentation_timestamp) = converted else {
                return Ok(());
            };
            let stream_time = StreamTime {
                stream_index: 0,
                presentation_timestamp,
                time_base_numerator: 1,
                time_base_denominator: 1_000_000,
                timeline_offset_micros: 0,
            };

            prop_assert_eq!(stream_time.to_media_time().map(MediaTime::as_micros), Ok(value));
        }

        #[test]
        fn full_frame_crop_is_always_contained(
            width in 1_u32..=100_000,
            height in 1_u32..=100_000
        ) {
            let dimensions = FrameDimensions::new(width, height);
            prop_assert_eq!(
                dimensions.and_then(|frame| CropRect::new(0, 0, width, height, frame)),
                dimensions.map(|_| CropRect { x: 0, y: 0, width, height })
            );
        }
    }
}
