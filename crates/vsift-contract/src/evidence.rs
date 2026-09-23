//! Evidence metadata shapes frozen ahead of the packets that produce them.
//!
//! P01 fixed how confidence and frame timing appear in public output before P07
//! and P09 emit them, so later evidence records cannot drift from the reviewed
//! representation.

use serde::Serialize;
use vsift_domain::{Confidence, ConfidenceOrigin, FrameTiming};

/// Public confidence representation that leaves unknown values as JSON `null`.
///
/// A provider that reports no score must not look calibrated, so the origin is
/// always stated beside the value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ConfidenceResponse {
    value_basis_points: Option<u16>,
    origin: &'static str,
}

impl From<Confidence> for ConfidenceResponse {
    fn from(value: Confidence) -> Self {
        let origin = match value.origin() {
            ConfidenceOrigin::Unavailable => "unavailable",
            ConfidenceOrigin::ProviderUncalibrated => "provider_uncalibrated",
            ConfidenceOrigin::ProviderCalibrated => "provider_calibrated",
        };
        Self {
            value_basis_points: value.basis_points(),
            origin,
        }
    }
}

/// Public frame timing with separate requested and provider-observed positions.
///
/// Decoders snap to real frames, so the requested time and the frame actually
/// returned differ; both are reported, with their signed difference, rather than
/// pretending the request was exact.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct FrameTimingResponse {
    #[serde(rename = "requested_time_us")]
    requested: u64,
    #[serde(rename = "actual_time_us")]
    actual: u64,
    #[serde(rename = "delta_us")]
    delta: i64,
}

impl From<FrameTiming> for FrameTimingResponse {
    fn from(value: FrameTiming) -> Self {
        Self {
            requested: value.requested().as_micros(),
            actual: value.actual().as_micros(),
            delta: value.delta_micros(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use vsift_domain::{Confidence, FrameTiming, MediaTime};

    use super::{ConfidenceResponse, FrameTimingResponse};

    #[test]
    fn unknown_confidence_and_frame_delta_remain_explicit() -> Result<(), Box<dyn std::error::Error>>
    {
        let confidence = serde_json::to_value(ConfidenceResponse::from(Confidence::unknown()))?;
        let timing =
            FrameTiming::new(MediaTime::from_micros(2_000), MediaTime::from_micros(1_750))?;
        let timing = serde_json::to_value(FrameTimingResponse::from(timing))?;

        assert_eq!(confidence["value_basis_points"], Value::Null);
        assert_eq!(confidence["origin"], "unavailable");
        assert_eq!(timing["requested_time_us"], 2_000);
        assert_eq!(timing["actual_time_us"], 1_750);
        assert_eq!(timing["delta_us"], -250);
        Ok(())
    }
}
