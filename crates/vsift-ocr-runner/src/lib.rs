//! The OCR runner: text read off `VSift`'s evidence frames with `PP-OCRv5`'s models.
//!
//! `VSift` runs this program as an external engine under its process supervisor,
//! as it runs whisper.cpp (ADR 0030). It takes frame PNG files that `VSift`'s own
//! `FFmpeg` wrote, and writes one JSON object a line ([`output`]): every line of
//! text it read, with its box and the recogniser's confidence. No `VSift` crate
//! links it, and it links no `VSift` crate.
//!
//! The pipeline is the one the evaluation measured
//! (`docs/planning/ocr-engine-evaluation.md`): detect text with `PP-OCRv5` mobile's
//! detector ([`detection`]), cut out each box, read it with the Latin recogniser
//! ([`recognition`]), and keep lines of confidence 0.5 or more ([`pipeline`]).
//! Inference is behind [`pipeline::Inference`], so everything around the models
//! is tested without them.

#![forbid(unsafe_code)]

pub mod cli;
pub mod detection;
pub mod image;
pub mod onnx;
pub mod output;
pub mod pipeline;
pub mod png;
pub mod recognition;
pub mod tensor;

/// The fixed failure codes of the runner's `error` records and exit statuses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunnerFailure {
    /// The command line is not one the runner accepts.
    Usage,
    /// ONNX Runtime could not be loaded or initialised.
    RuntimeUnavailable,
    /// A model file is missing, too large, or not a model the runtime accepts.
    ModelUnavailable,
    /// An image could not be read: not a PNG of the accepted profile, or too large.
    ImageInvalid,
    /// The models ran but their output could not be read, or inference failed.
    InferenceFailed,
    /// The output could not be written.
    OutputFailed,
}

impl RunnerFailure {
    /// Every failure, in the order of their exit statuses.
    pub const ALL: [Self; 6] = [
        Self::Usage,
        Self::RuntimeUnavailable,
        Self::ModelUnavailable,
        Self::ImageInvalid,
        Self::InferenceFailed,
        Self::OutputFailed,
    ];

    /// The code written in an `error` record.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::ModelUnavailable => "model_unavailable",
            Self::ImageInvalid => "image_invalid",
            Self::InferenceFailed => "inference_failed",
            Self::OutputFailed => "output_failed",
        }
    }

    /// The exit status of a run that ends with this failure (an image that cannot be
    /// read does not end the run: its `error` record says so and the run goes on).
    #[must_use]
    pub const fn exit_status(self) -> u8 {
        match self {
            Self::Usage => 2,
            Self::RuntimeUnavailable => 3,
            Self::ModelUnavailable => 4,
            Self::ImageInvalid => 5,
            Self::InferenceFailed => 6,
            Self::OutputFailed => 7,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RunnerFailure;

    #[test]
    fn codes_and_exit_statuses_are_distinct() {
        let codes: std::collections::BTreeSet<&str> = RunnerFailure::ALL
            .iter()
            .map(|failure| failure.code())
            .collect();
        let statuses: std::collections::BTreeSet<u8> = RunnerFailure::ALL
            .iter()
            .map(|failure| failure.exit_status())
            .collect();
        assert_eq!(codes.len(), RunnerFailure::ALL.len());
        assert_eq!(statuses.len(), RunnerFailure::ALL.len());
        assert!(!statuses.contains(&0) && !statuses.contains(&1));
    }
}
