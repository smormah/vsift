//! Domain concepts shared by `VSift` use cases.

#![forbid(unsafe_code)]

mod evidence;
mod failure;
mod identity;
mod job;
mod pagination;
mod timeline;

pub use evidence::{
    Confidence, ConfidenceError, ConfidenceOrigin, SpeakerLabel, SpeakerLabelError,
};
pub use failure::{FailureClass, FailureCode, OperationStatus};
pub use identity::{
    ArtifactId, EvidenceId, IdentifierError, JobId, OperationId, OperationKey, SessionId, SourceId,
};
pub use job::{JobState, JobTransitionError};
pub use pagination::{CursorError, CursorToken, PageLimit, PageLimitError, QueryDigest};
pub use timeline::{
    CropRect, FrameDimensions, FrameTiming, GeometryError, MediaTime, StreamTime,
    TimeConversionError, TimeRange, TimeRangeError,
};

/// A specialist runtime dependency that provides one of `VSift`'s capabilities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeDependency {
    /// `FFmpeg` performs media conversion and extraction.
    Ffmpeg,
    /// `FFprobe` reads media metadata without decoding the complete source.
    Ffprobe,
    /// A Whisper-compatible CLI performs local speech-to-text transcription.
    Whisper,
}

impl RuntimeDependency {
    /// All dependencies understood by the initial runtime diagnostic.
    pub const ALL: [Self; 3] = [Self::Ffmpeg, Self::Ffprobe, Self::Whisper];

    /// Returns the stable identifier used in machine-readable contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
            Self::Whisper => "whisper",
        }
    }

    /// Returns the human-readable dependency name.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Ffmpeg => "FFmpeg",
            Self::Ffprobe => "FFprobe",
            Self::Whisper => "Whisper",
        }
    }

    /// Returns the capability supplied by this dependency.
    #[must_use]
    pub const fn capability(self) -> RuntimeCapability {
        match self {
            Self::Ffmpeg | Self::Ffprobe => RuntimeCapability::MediaProcessing,
            Self::Whisper => RuntimeCapability::Transcription,
        }
    }
}

/// A user-facing capability supplied by one or more runtime dependencies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeCapability {
    /// Media probing, audio extraction, and frame extraction.
    MediaProcessing,
    /// Local speech-to-text transcription.
    Transcription,
}

impl RuntimeCapability {
    /// Returns the stable identifier used in machine-readable contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::MediaProcessing => "media_processing",
            Self::Transcription => "transcription",
        }
    }
}

/// The observed state of one runtime dependency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyState {
    /// The dependency responded successfully.
    Available {
        /// A bounded first line identifying the detected tool or version.
        version: String,
    },
    /// The dependency executable could not be found.
    Missing,
    /// The dependency was found but did not respond successfully.
    Unhealthy {
        /// A bounded diagnostic safe to present to the caller.
        message: String,
    },
    /// The dependency did not respond before the diagnostic deadline.
    TimedOut,
}

impl DependencyState {
    /// Reports whether the dependency is ready for use.
    #[must_use]
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    /// Returns the stable state identifier used in machine-readable contracts.
    #[must_use]
    pub const fn identifier(&self) -> &'static str {
        match self {
            Self::Available { .. } => "available",
            Self::Missing => "missing",
            Self::Unhealthy { .. } => "unhealthy",
            Self::TimedOut => "timed_out",
        }
    }
}

/// The diagnostic result for one dependency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyStatus {
    /// Dependency that was inspected.
    pub dependency: RuntimeDependency,
    /// State observed by the dependency probe.
    pub state: DependencyState,
}

/// Aggregate readiness of the local `VSift` runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeReadiness {
    /// Media processing and local transcription are available.
    Ready,
    /// Core media processing is available, but an optional capability is missing.
    Degraded,
    /// Core media processing is unavailable.
    Blocked,
}

impl RuntimeReadiness {
    /// Returns the stable identifier used in machine-readable contracts.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Blocked => "blocked",
        }
    }
}
