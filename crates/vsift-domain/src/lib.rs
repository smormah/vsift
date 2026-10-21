//! Domain concepts shared by `VSift` use cases.

#![forbid(unsafe_code)]

mod admission;
mod asr;
mod checkpoint;
mod evidence;
mod failure;
mod identity;
mod job;
mod media;
mod pagination;
mod progress;
mod provisioning;
mod retry;
mod search;
mod session;
mod source_origin;
mod spans;
mod storage;
mod timeline;
mod transcript;
mod visual;
mod worker;

use std::time::Duration;

pub use admission::{
    ADMISSION_RETRY_AFTER, AdmissionBudget, AdmissionBudgetError, AdmissionDecision, AdmissionWait,
    MAX_ADMISSION_WAIT, MAX_RECOGNIZER_THREADS, SINGLE_STAGE_WEIGHT, VISUAL_WINDOW_WEIGHT,
    recognizer_threads,
};
pub use asr::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, AsrSegmentDraft, ChunkPlan, ChunkPlanError,
    ChunkSegments, ChunkTime, DigestError, EarlierTextRule, MAX_CHUNK_WINDOW_MICROS,
    MAX_PLANNED_CHUNKS, MAX_PROVIDER_SEGMENTS, MAX_PROVIDER_TOKENS, MIN_RECOGNITION_MICROS,
    MIN_RECOGNITION_SAMPLES, MIN_SEGMENTS_FOR_REJECTION_RATIO, MergedSegment, MergedTranscript,
    PROVIDER_END_TOLERANCE_MICROS, PlannedChunk, ProviderChunkOutput, ProviderOutputError,
    ProviderSegment, ProviderToken, ProviderTokenKind, ReviewedAsrModel,
    SEAM_COVERAGE_TOLERANCE_MICROS, SPEECH_SAMPLE_RATE, Sha256Hex,
    UNUSABLE_CHUNK_SHARE_DENOMINATOR, ValidatedChunk, decoded_audio_range,
    is_below_recognition_floor, is_silent_pcm, merge_chunks, plan_chunks, rounds_to_no_pcm_sample,
    unusable_chunks_end_the_run, validate_chunk_output,
};
pub use checkpoint::{CheckpointOutcome, ChunkCheckpoint, RecognitionKey};
pub use evidence::{
    AUDIO_CLIP_SAMPLE_RATE, AudioRange, BurstCount, BurstExtent, BurstPlan, BurstRange, Confidence,
    ConfidenceError, ConfidenceOrigin, CropRegion, EvidenceDetail, EvidenceItem, EvidenceItemParts,
    EvidenceMedia, EvidenceMediaKind, EvidenceOperation, EvidenceProfile, EvidenceRecord,
    EvidenceRecordError, EvidenceRecordParts, EvidenceRequest, EvidenceSelection, EvidenceSubject,
    FrameListing, FrameRef, FrameSelection, FrameSelectionError, FrameTolerance, ListedFrame,
    ListingTail, MAX_AUDIO_CLIP_MICROS, MAX_BURST_FRAMES, MAX_BURST_RANGE_MICROS,
    MAX_FRAME_TOLERANCE_MICROS, MAX_LISTED_FRAMES, MAX_NEIGHBOUR_COUNT, MAX_RECORD_ITEMS,
    MAX_RECORD_SELECTIONS, NavigationError, NeighbourCount, NeighbourPlan, NeighbourStop,
    PartialReason, SelectedFrame, SelectionRole, SourceCheck, SpeakerLabel, SpeakerLabelError,
    TimeBase, plan_burst, plan_neighbours, select_frame,
};
pub use failure::{FailureClass, FailureCode, OperationStatus};
pub use identity::{
    ArtifactId, EvidenceId, IdentifierError, JobId, OperationId, OperationKey, SessionId, SourceId,
    SourceSegmentId, TranscriptRevisionId, TranscriptSegmentId, VisualCandidateId, VisualIndexId,
};
pub use job::{JobKind, JobState, JobTransitionError};
pub use media::{
    DisplayRotation, MediaDecodeSupport, MediaDescription, MediaSelection, MediaSelectionError,
    MediaStream, MediaStreamKind, VisualStreamError,
};
pub use pagination::{CursorError, CursorToken, PageLimit, PageLimitError, QueryDigest};
pub use progress::{ProgressStage, ProgressUnit, ProgressUpdate};
pub use provisioning::{
    ArtifactIntegrity, ArtifactIntegrityError, MAX_MANAGED_ARTIFACT_BYTES, MAX_MANAGED_KEY_BYTES,
    MAX_SHARED_LIBRARY_NAME_BYTES, ManagedArtifactFormat, ManagedComponent, ManagedTarget,
    ManagedVersionKey, ManagedVersionKeyError, SharedLibraryName, is_canonical_managed_key,
};
pub use retry::{
    AttemptFailure, Jitter, MAX_AUTOMATIC_RETRIES, MAX_JOB_ATTEMPTS, POISON_THRESHOLD,
    RETRY_BASE_DELAY, RETRY_MAX_DELAY, RetryClass, RetryDecision, RetryPolicy, poisoned_chunk,
};
pub use search::{
    CoverageBasis, MAX_SEARCH_QUERY_BYTES, MAX_SEARCH_TERMS, SearchCoverage, SearchHit,
    SearchMatch, SearchPosition, SearchQuery, SearchQueryRejection, SearchSlice,
    normalise_search_text, search_revision,
};
pub use session::{
    SessionArtifactKind, SessionLifetime, SessionLifetimePolicy, SessionPhase,
    SessionTransitionError, WorkspacePolicy, WorkspacePolicyError, WorkspaceRetention,
};
pub use source_origin::{
    AddressCheck, DEFAULT_DOWNLOAD_BYTES, DEFAULT_DOWNLOAD_SECONDS, DirectLinkOrigin,
    DownloadLimits, DownloadLimitsError, HostName, HostNameError, MAX_DOWNLOAD_BYTES,
    MAX_DOWNLOAD_SECONDS, MAX_HOST_NAME_BYTES, MAX_SOURCE_LINK_BYTES, MAX_SOURCE_REDIRECTS,
    SourceFetchReason, SourceLink, SourceOrigin, SourceOriginError,
};
pub use storage::{
    DurabilityRequirement, GenerationError, PublicationGuarantee, StorageGeneration,
};
pub use timeline::{
    CropParseError, CropRect, FrameDimensions, FrameTiming, GeometryError, MediaTime, StreamTime,
    TimeConversionError, TimeRange, TimeRangeError,
};
pub use transcript::{
    AlignedCue, AlignmentOrigin, CarriedFrom, CueMarkup, CueSource, CueText, CueTiming,
    ImportedCue, InheritedRevision, LanguageTag, LanguageTagError, MAX_CARRIED_UNTRANSCRIBED,
    MAX_CUE_TEXT_BYTES, MAX_SUPPLIED_TRANSCRIPT_BYTES, MAX_TRANSCRIPT_CUES,
    MAX_TRANSCRIPT_OFFSET_MICROS, MAX_UNPLACED_STRETCHES, ParsedTranscript, ProviderEndTrim,
    SegmentOrigin, SidecarIdentity, SourceSegment, SourceSegmentState, TranscriptFormat,
    TranscriptImportError, TranscriptOffset, TranscriptProvenance, TranscriptRejection,
    TranscriptRevision, TranscriptRevisionError, TranscriptRevisionParts, TranscriptSegment,
    TranscriptSegmentParts, TranscriptSlice, TranscriptWarning, TranscriptWarningKind,
    TranscriptWarnings, align_imported_cues,
};
pub use visual::{
    CandidateChange, CandidateDraft, CandidateOmission, CandidateReason, CandidateStability,
    CoverageGapReason, MAX_VISUAL_INDEX_DURATION_MICROS, MAX_WINDOW_CANDIDATES, MAX_WINDOW_SAMPLES,
    VISUAL_BLOCK_COLUMNS, VISUAL_BLOCK_ROWS, VISUAL_BLOCKS, VISUAL_CELL_MICROS, VISUAL_FRAME_BYTES,
    VISUAL_FRAME_HEIGHT, VISUAL_FRAME_WIDTH, VISUAL_LEAD_IN_MICROS, VISUAL_SAMPLE_INTERVAL_MICROS,
    VISUAL_WINDOW_MICROS, VisualAnalysisError, VisualCandidate, VisualChangePolicy,
    VisualCoverageGap, VisualDelta, VisualHash, VisualIndex, VisualIndexError, VisualIndexParts,
    VisualIndexProfile, VisualIndexWindow, VisualSample, VisualWindow, VisualWindowOutcome,
    WindowAnalysis, analyse_window, visual_window_count,
};
pub use worker::{
    MIN_STEP_BUDGET, RecordedRequest, RequestAdmission, StepRetry, admit_request, ends_request,
    step_may_start, step_retry,
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
    /// The dependency executable was found, but the diagnostic deadline passed
    /// before its probe gave a result. Nothing is known about whether it works.
    TimedOut(ProbeTimeout),
}

impl DependencyState {
    /// Reports whether the initial executable probe succeeded, not full compatibility.
    #[must_use]
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    /// Reports whether the probe showed that the executable cannot be used as
    /// it is: none was found, or the one found failed.
    ///
    /// A probe that timed out shows neither, so it is not [`Self::is_available`]
    /// and not this either. The difference matters wherever an unusable tool is
    /// a reason to act: a healthy tool that was slow to start once must never
    /// be replaced or reinstalled as if it were missing (issue #391).
    #[must_use]
    pub const fn is_unusable(&self) -> bool {
        matches!(self, Self::Missing | Self::Unhealthy { .. })
    }

    /// Returns the stable state identifier used in machine-readable contracts.
    ///
    /// Both kinds of [`ProbeTimeout`] are the one published value `timed_out`.
    #[must_use]
    pub const fn identifier(&self) -> &'static str {
        match self {
            Self::Available { .. } => "available",
            Self::Missing => "missing",
            Self::Unhealthy { .. } => "unhealthy",
            Self::TimedOut(_) => "timed_out",
        }
    }
}

/// How a dependency probe came to have no result by the diagnostic deadline.
///
/// The probes of one diagnostic run one after another under a single shared
/// deadline, so a slow first probe can leave no time for the ones after it.
/// The two cases are one published state (`timed_out`) but different facts,
/// and advice that is true of one is false of the other: "it did not answer"
/// cannot be said of a tool that was never started.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeTimeoutKind {
    /// The executable was started and had not answered when the deadline
    /// passed; it was stopped.
    NoAnswer,
    /// The executable was found but never started, because the shared
    /// deadline had already passed when its turn came.
    NotStarted,
}

/// A dependency probe that gave no result by the diagnostic deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeTimeout {
    /// Whether the executable was started.
    pub kind: ProbeTimeoutKind,
    /// The one deadline every probe of the diagnostic shared, as the caller
    /// set it: the time allowed for all of them together, not for this probe
    /// alone. It is carried so that advice can name the figure the caller
    /// would have to raise.
    pub shared_deadline: Duration,
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
    /// Media and transcription executable probes passed; model compatibility is unverified.
    Ready,
    /// Media executable probes passed, but the transcription executable probe did not.
    Degraded,
    /// At least one core media executable probe did not pass.
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{DependencyState, ProbeTimeout, ProbeTimeoutKind};

    /// Issue #391: a probe that timed out is one published state whichever way
    /// it happened, and it is neither a usable tool nor one shown to be
    /// unusable, so nothing may treat it as missing.
    #[test]
    fn a_timed_out_probe_is_neither_available_nor_shown_unusable() {
        for kind in [ProbeTimeoutKind::NoAnswer, ProbeTimeoutKind::NotStarted] {
            let state = DependencyState::TimedOut(ProbeTimeout {
                kind,
                shared_deadline: Duration::from_secs(30),
            });
            assert_eq!(state.identifier(), "timed_out", "{kind:?}");
            assert!(!state.is_available(), "{kind:?}");
            assert!(!state.is_unusable(), "{kind:?}");
        }
        let available = DependencyState::Available {
            version: String::from("fixture 1"),
        };
        assert!(available.is_available() && !available.is_unusable());
        for unusable in [
            DependencyState::Missing,
            DependencyState::Unhealthy {
                message: String::from("process exited with exit code: 3"),
            },
        ] {
            assert!(!unusable.is_available() && unusable.is_unusable());
        }
    }
}
