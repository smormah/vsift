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

    /// Swaps positive dimensions for a quarter-turn display orientation.
    #[must_use]
    pub const fn quarter_turn(self) -> Self {
        Self {
            width: self.height,
            height: self.width,
        }
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

    /// Returns the rectangle's size as the dimensions of the image it cuts out.
    #[must_use]
    pub const fn dimensions(self) -> FrameDimensions {
        FrameDimensions {
            width: self.width,
            height: self.height,
        }
    }

    /// Parses the public `x,y,width,height` form and validates it against
    /// the displayed frame it will be cut from.
    ///
    /// The form is exactly four unsigned decimal integers separated by
    /// commas: no sign, whitespace, leading zero (except `0` itself) or
    /// anything else, so one spelling names one rectangle and nothing a
    /// caller writes reaches a provider as text. Validation is the same as
    /// [`CropRect::new`] and happens before any I/O.
    ///
    /// # Errors
    ///
    /// Returns [`CropParseError::Malformed`] for any other text and
    /// [`CropParseError::Geometry`] for a zero-size, overflowing or
    /// out-of-frame rectangle.
    pub fn parse(text: &str, frame: FrameDimensions) -> Result<Self, CropParseError> {
        let mut values = [0_u32; 4];
        let mut parts = text.split(',');
        for value in &mut values {
            *value = parts
                .next()
                .and_then(parse_crop_coordinate)
                .ok_or(CropParseError::Malformed)?;
        }
        if parts.next().is_some() {
            return Err(CropParseError::Malformed);
        }
        let [x, y, width, height] = values;
        Self::new(x, y, width, height, frame).map_err(CropParseError::Geometry)
    }

    /// Maps `inner`, a rectangle of the image this crop cut out, back to the
    /// coordinates of the frame this crop was cut from.
    ///
    /// A crop of a crop is recorded against the original frame so its
    /// lineage names source pixels, never pixels of a derived image.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::OutsideFrame`] when `inner` is not contained
    /// by this crop's own dimensions, and [`GeometryError::Overflow`] if the
    /// translated coordinates cannot be represented.
    pub fn compose(self, inner: Self) -> Result<Self, GeometryError> {
        let within = Self::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height,
            self.dimensions(),
        )?;
        Ok(Self {
            x: self
                .x
                .checked_add(within.x)
                .ok_or(GeometryError::Overflow)?,
            y: self
                .y
                .checked_add(within.y)
                .ok_or(GeometryError::Overflow)?,
            width: within.width,
            height: within.height,
        })
    }
}

/// Reads one crop coordinate: ASCII digits only, no leading zero.
fn parse_crop_coordinate(text: &str) -> Option<u32> {
    let canonical = !text.is_empty()
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if canonical { text.parse().ok() } else { None }
}

/// Why a textual crop rectangle was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CropParseError {
    /// The text is not exactly four canonical unsigned integers separated by commas.
    Malformed,
    /// The rectangle is empty, overflows or leaves the displayed frame.
    Geometry(GeometryError),
}

impl fmt::Display for CropParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => formatter.write_str("crop must be written as x,y,width,height"),
            Self::Geometry(error) => error.fmt(formatter),
        }
    }
}

impl Error for CropParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Malformed => None,
            Self::Geometry(error) => Some(error),
        }
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

    /// Returns the earliest timestamp of a stream, in its own time base,
    /// whose normalized time ([`StreamTime::to_media_time`]) is at or after
    /// `time`.
    ///
    /// Frame requests are selected by comparing integer stream timestamps
    /// with this bound instead of comparing decimal seconds: a decimal
    /// comparison is evaluated in floating point by the provider, so a frame
    /// whose normalized time equals the request exactly (for example a visual
    /// candidate's representative time) could be skipped or kept depending
    /// on rounding. Because [`StreamTime::to_media_time`] truncates, several
    /// timestamps of a time base finer than a microsecond can share one
    /// normalized time; the smallest of them is returned, so every frame at
    /// or after `time` has a timestamp at or above the result and every
    /// earlier frame one below it.
    ///
    /// # Errors
    ///
    /// Returns [`TimeConversionError::InvalidTimeBase`] for a zero numerator
    /// or denominator and [`TimeConversionError::Overflow`] when the bound
    /// does not fit a 64-bit timestamp.
    pub fn first_at_or_after(
        stream_index: u32,
        time_base_numerator: u32,
        time_base_denominator: u32,
        timeline_offset_micros: i64,
        time: MediaTime,
    ) -> Result<Self, TimeConversionError> {
        if time_base_numerator == 0 || time_base_denominator == 0 {
            return Err(TimeConversionError::InvalidTimeBase);
        }
        // Normalized time is trunc(pts * scale / denominator) + offset, so
        // the bound is the smallest pts whose truncated stream micros reach
        // `target`. Truncation rounds toward zero, which is a floor for
        // non-negative quotients and a ceiling for negative ones.
        let scale = i128::from(time_base_numerator) * MICROS_PER_SECOND;
        let denominator = i128::from(time_base_denominator);
        let target = i128::from(time.as_micros()) - i128::from(timeline_offset_micros);
        let bound = if target > 0 {
            // floor(p * scale / d) >= target  <=>  p * scale >= target * d.
            ceiling_division(target * denominator, scale)
        } else {
            // A non-negative p always qualifies; a negative one qualifies
            // while its quotient stays above target - 1.
            floor_division((target - 1) * denominator, scale) + 1
        };
        let presentation_timestamp =
            i64::try_from(bound).map_err(|_| TimeConversionError::Overflow)?;
        Ok(Self {
            stream_index,
            presentation_timestamp,
            time_base_numerator,
            time_base_denominator,
            timeline_offset_micros,
        })
    }
}

/// Floor of `numerator / denominator` for a positive denominator.
const fn floor_division(numerator: i128, denominator: i128) -> i128 {
    let quotient = numerator / denominator;
    if numerator % denominator < 0 {
        quotient - 1
    } else {
        quotient
    }
}

/// Ceiling of `numerator / denominator` for a positive denominator.
const fn ceiling_division(numerator: i128, denominator: i128) -> i128 {
    let quotient = numerator / denominator;
    if numerator % denominator > 0 {
        quotient + 1
    } else {
        quotient
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

    use super::{
        CropParseError, CropRect, FrameDimensions, FrameTiming, GeometryError, MediaTime,
        StreamTime, TimeConversionError, TimeRange,
    };

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
    fn crops_parse_only_in_their_canonical_form() {
        let frame = FrameDimensions::new(1440, 900);
        assert!(frame.is_ok());
        let Ok(frame) = frame else {
            return;
        };
        assert_eq!(
            CropRect::parse("850,420,280,70", frame),
            CropRect::new(850, 420, 280, 70, frame).map_err(CropParseError::Geometry)
        );
        assert!(CropRect::parse("0,0,1440,900", frame).is_ok());
        assert!(CropRect::parse("1439,899,1,1", frame).is_ok());
        for text in [
            "",
            "1,2,3",
            "1,2,3,4,5",
            " 1,2,3,4",
            "1, 2,3,4",
            "+1,2,3,4",
            "-1,2,3,4",
            "01,2,3,4",
            "1,2,3,4,",
            "1;2;3;4",
            "0x1,2,3,4",
            "4294967296,0,1,1",
            "1,2,3,٤",
        ] {
            assert_eq!(
                CropRect::parse(text, frame),
                Err(CropParseError::Malformed),
                "{text}"
            );
        }
        assert_eq!(
            CropRect::parse("1440,0,1,1", frame),
            Err(CropParseError::Geometry(GeometryError::OutsideFrame))
        );
        assert_eq!(
            CropRect::parse("1,0,1440,900", frame),
            Err(CropParseError::Geometry(GeometryError::OutsideFrame))
        );
        assert_eq!(
            CropRect::parse("0,0,0,10", frame),
            Err(CropParseError::Geometry(GeometryError::ZeroSize))
        );
        assert_eq!(
            CropRect::parse("4294967295,0,1,1", frame),
            Err(CropParseError::Geometry(GeometryError::Overflow))
        );
    }

    #[test]
    fn a_crop_of_a_crop_names_source_pixels() {
        let composed = FrameDimensions::new(1440, 900).and_then(|frame| {
            let outer = CropRect::new(100, 200, 300, 150, frame)?;
            let inner = CropRect::new(10, 20, 290, 130, outer.dimensions())?;
            outer.compose(inner)
        });
        assert_eq!(
            composed.map(|rect| (rect.x(), rect.y(), rect.width(), rect.height())),
            Ok((110, 220, 290, 130))
        );
        let escaping = FrameDimensions::new(1440, 900).and_then(|frame| {
            let outer = CropRect::new(100, 200, 300, 150, frame)?;
            let inner = CropRect::new(10, 20, 291, 130, frame)?;
            outer.compose(inner)
        });
        assert_eq!(escaping, Err(GeometryError::OutsideFrame));
    }

    #[test]
    fn integer_bounds_select_exactly_the_frames_at_or_after_a_time() {
        // F01: 20 fps in a 1/10240 time base, origin 0.
        let bound = |micros| {
            StreamTime::first_at_or_after(0, 1, 10_240, 0, MediaTime::from_micros(micros))
                .map(|time| time.presentation_timestamp)
        };
        assert_eq!(bound(1_050_000), Ok(10_752));
        assert_eq!(bound(1_025_000), Ok(10_496));
        assert_eq!(bound(0), Ok(0));
        // F09: 1/1000 with a 2 s origin: 3.25 s normalized is pts 5250.
        assert_eq!(
            StreamTime::first_at_or_after(
                0,
                1,
                1_000,
                -2_000_000,
                MediaTime::from_micros(3_250_000)
            )
            .map(|time| time.presentation_timestamp),
            Ok(5_250)
        );
        // A time base finer than a microsecond: 10 timestamps share each
        // microsecond and the earliest is returned.
        assert_eq!(
            StreamTime::first_at_or_after(0, 1, 10_000_000, 0, MediaTime::from_micros(7))
                .map(|time| time.presentation_timestamp),
            Ok(70)
        );
        // A negative origin offset (streams starting before zero).
        assert_eq!(
            StreamTime::first_at_or_after(0, 1, 1_000, 500_000, MediaTime::from_micros(0))
                .map(|time| time.presentation_timestamp),
            Ok(-500)
        );
        assert_eq!(
            StreamTime::first_at_or_after(0, 0, 1_000, 0, MediaTime::from_micros(0)),
            Err(TimeConversionError::InvalidTimeBase)
        );
        assert_eq!(
            StreamTime::first_at_or_after(0, 1, u32::MAX, 0, MediaTime::from_micros(u64::MAX)),
            Err(TimeConversionError::Overflow)
        );
    }

    #[test]
    fn preserves_requested_and_actual_frame_times() {
        let timing = FrameTiming::new(MediaTime::from_micros(2_000), MediaTime::from_micros(1_750));
        assert_eq!(timing.map(FrameTiming::delta_micros), Ok(-250));
    }

    proptest! {
        #[test]
        fn the_integer_bound_is_the_smallest_timestamp_at_or_after_the_time(
            numerator in 1_u32..=1_001,
            denominator in 1_u32..=90_000_000,
            offset in -10_000_000_000_i64..=10_000_000_000,
            micros in 0_u64..=100_000_000_000,
        ) {
            let time = MediaTime::from_micros(micros);
            let bound = StreamTime::first_at_or_after(3, numerator, denominator, offset, time);
            prop_assert!(bound.is_ok());
            let Ok(bound) = bound else {
                return Ok(());
            };
            prop_assert_eq!(bound.stream_index, 3);
            prop_assert!(bound.to_media_time().is_ok_and(|actual| actual >= time));
            let previous = StreamTime {
                presentation_timestamp: bound.presentation_timestamp - 1,
                ..bound
            };
            // The timestamp before the bound is earlier than the time or
            // before the normalized timeline altogether.
            prop_assert!(previous.to_media_time().map_or(true, |earlier| earlier < time));
        }

        #[test]
        fn parsed_crops_are_contained_and_compose_inside_the_frame(
            width in 1_u32..=4_096,
            height in 1_u32..=4_096,
            x in 0_u32..=5_000,
            y in 0_u32..=5_000,
            crop_width in 0_u32..=5_000,
            crop_height in 0_u32..=5_000,
            inner in (0_u32..=5_000, 0_u32..=5_000, 0_u32..=5_000, 0_u32..=5_000),
        ) {
            let frame = FrameDimensions::new(width, height);
            prop_assert!(frame.is_ok());
            let Ok(frame) = frame else {
                return Ok(());
            };
            let parsed = CropRect::parse(&format!("{x},{y},{crop_width},{crop_height}"), frame);
            let contained = crop_width > 0
                && crop_height > 0
                && u64::from(x) + u64::from(crop_width) <= u64::from(width)
                && u64::from(y) + u64::from(crop_height) <= u64::from(height);
            prop_assert_eq!(parsed.is_ok(), contained);
            if let Ok(outer) = parsed {
                let (inner_x, inner_y, inner_width, inner_height) = inner;
                let child = CropRect::new(inner_x, inner_y, inner_width, inner_height, outer.dimensions());
                let composed = child.and_then(|child| outer.compose(child));
                prop_assert_eq!(composed.is_ok(), child.is_ok());
                if let Ok(rect) = composed {
                    prop_assert!(rect.x() >= outer.x() && rect.y() >= outer.y());
                    prop_assert!(rect.x() + rect.width() <= outer.x() + outer.width());
                    prop_assert!(rect.y() + rect.height() <= outer.y() + outer.height());
                    prop_assert!(CropRect::new(rect.x(), rect.y(), rect.width(), rect.height(), frame).is_ok());
                }
            }
        }

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
