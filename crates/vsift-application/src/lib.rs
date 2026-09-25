//! Application use cases and infrastructure ports for `VSift`.

#![forbid(unsafe_code)]

use std::future::Future;

use vsift_domain::{DependencyStatus, RuntimeCapability, RuntimeDependency, RuntimeReadiness};

mod asr;
mod clock;
mod identifiers;
mod provisioning;
mod session;
mod storage;
mod transcript;
mod verification;

pub use asr::{
    AsrCancellation, AsrFailure, AsrFailureReason, AsrRevisionRequest, AsrStage, AsrTranscription,
    RecognizerIdentity, RevisionSplice, SpeechAudioError, SpeechAudioSource, SpeechPcm,
    SpeechRecognitionError, SpeechRecognizer, TranscribeRangeRequest, build_asr_revision,
    transcribe_range,
};
pub use clock::{Clock, ClockError};
pub use identifiers::{IdentifierGenerationError, IdentifierSource};
pub use session::{
    ForegroundSessionPort, OpenSession, OpenSessionError, OpenSessionOutcome, OpenSessionRequest,
    StagedSessionSource,
};

pub use provisioning::{
    AcceptedManagedArtifact, AcceptedManagedCatalogue, ManagedPlanAvailability, ManagedSetupAction,
    ManagedSetupPlan, PlanAcceptanceError, ReviewedArchiveLimits, ReviewedArchiveLink,
    ReviewedArchiveSelection, ReviewedCompatibilityPolicy, ReviewedManagedFile,
    ReviewedRuntimeCopy, SetupDependencyDisposition, SetupModelDisposition, SetupProfile,
    SetupSelectionState, plan_managed_setup,
};
pub use storage::{
    AuthorizedSessionGenerationPublication, AuthorizedSessionStorageInitialization,
    InitializeSessionStorage, InitializeSessionStorageRequest, InitializedSessionStorage,
    PublishSessionGeneration, PublishSessionGenerationRequest, SessionStorageError, SessionStore,
    StorageCapabilities,
};
pub use transcript::{
    ImportedRevisionRequest, SourceDurationProbe, SourceProbeError, SuppliedTranscript,
    SuppliedTranscriptError, TranscriptBuildError, TranscriptImportRequest, TranscriptPage,
    TranscriptPageRequest, TranscriptQueryError, build_imported_revision, page_transcript,
    transcript_query_digest, transcript_segment_id, whole_file_source_segment,
};
pub use verification::{
    CachedMediaToolVerification, LocalAsrVerification, LocalAsrVerificationFailure,
    LocalAsrVerifier, MediaToolCheck, MediaToolFailure, MediaToolFingerprint,
    MediaToolPreflightFailure, MediaToolPreflightOutcome, MediaToolVerification,
    MediaToolVerificationCache, MediaToolVerifier, ModelVerification, VerificationRecord,
    VerificationRecordSkip, preflight_local_asr, preflight_media_tools,
};

/// Port used to inspect one specialist runtime dependency.
pub trait DependencyProbe: Send + Sync {
    /// Inspects a dependency without modifying the local machine.
    fn probe(&self, dependency: RuntimeDependency)
    -> impl Future<Output = DependencyStatus> + Send;
}

/// Complete result of the read-only runtime diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeDiagnosis {
    /// Aggregate executable-probe result, not full provider/model compatibility.
    pub readiness: RuntimeReadiness,
    /// Individual dependency results in stable display order.
    pub dependencies: Vec<DependencyStatus>,
}

/// One dependency's disposition when no managed artifact is qualified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnqualifiedPlanAction {
    /// An executable responded, but compatibility is still unverified.
    ProbeOnly,
    /// The user must locate or install a trusted executable outside managed setup.
    ManualSelection,
}

/// Read-only check-first result while the managed catalogue has no accepted builds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnqualifiedSetupPlan {
    /// Executable-probe aggregate only.
    pub readiness: RuntimeReadiness,
    /// Per-dependency status and permitted next action in stable order.
    pub dependencies: Vec<(DependencyStatus, UnqualifiedPlanAction)>,
}

impl UnqualifiedSetupPlan {
    /// Interprets a completed diagnosis without offering an unreviewed download.
    #[must_use]
    pub fn from_diagnosis(diagnosis: RuntimeDiagnosis) -> Self {
        Self {
            readiness: diagnosis.readiness,
            dependencies: diagnosis
                .dependencies
                .into_iter()
                .map(|status| {
                    let action = if status.state.is_available() {
                        UnqualifiedPlanAction::ProbeOnly
                    } else {
                        UnqualifiedPlanAction::ManualSelection
                    };
                    (status, action)
                })
                .collect(),
        }
    }
}

/// Read-only use case that diagnoses `VSift`'s local runtime.
pub struct DiagnoseRuntime<P> {
    probe: P,
}

impl<P> DiagnoseRuntime<P>
where
    P: DependencyProbe,
{
    /// Creates the use case with its dependency probe.
    pub const fn new(probe: P) -> Self {
        Self { probe }
    }

    /// Inspects all known dependencies and calculates aggregate readiness.
    pub async fn execute(&self) -> RuntimeDiagnosis {
        let mut dependencies = Vec::with_capacity(RuntimeDependency::ALL.len());

        for dependency in RuntimeDependency::ALL {
            dependencies.push(self.probe.probe(dependency).await);
        }

        let media_ready = dependencies
            .iter()
            .filter(|status| status.dependency.capability() == RuntimeCapability::MediaProcessing)
            .all(|status| status.state.is_available());

        let transcription_ready = dependencies.iter().any(|status| {
            status.dependency.capability() == RuntimeCapability::Transcription
                && status.state.is_available()
        });

        let readiness = match (media_ready, transcription_ready) {
            (false, _) => RuntimeReadiness::Blocked,
            (true, false) => RuntimeReadiness::Degraded,
            (true, true) => RuntimeReadiness::Ready,
        };

        RuntimeDiagnosis {
            readiness,
            dependencies,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::ready;

    use super::{DependencyProbe, DiagnoseRuntime};
    use vsift_domain::{DependencyState, DependencyStatus, RuntimeDependency, RuntimeReadiness};

    struct FakeProbe {
        unavailable: Vec<RuntimeDependency>,
    }

    impl DependencyProbe for FakeProbe {
        fn probe(
            &self,
            dependency: RuntimeDependency,
        ) -> impl Future<Output = DependencyStatus> + Send {
            let state = if self.unavailable.contains(&dependency) {
                DependencyState::Missing
            } else {
                DependencyState::Available {
                    version: String::from("test version"),
                }
            };

            ready(DependencyStatus { dependency, state })
        }
    }

    #[tokio::test]
    async fn reports_blocked_when_media_processing_is_missing() {
        let use_case = DiagnoseRuntime::new(FakeProbe {
            unavailable: vec![RuntimeDependency::Ffmpeg],
        });

        let diagnosis = use_case.execute().await;

        assert_eq!(diagnosis.readiness, RuntimeReadiness::Blocked);
    }

    #[tokio::test]
    async fn reports_degraded_when_only_transcription_is_missing() {
        let use_case = DiagnoseRuntime::new(FakeProbe {
            unavailable: vec![RuntimeDependency::Whisper],
        });

        let diagnosis = use_case.execute().await;

        assert_eq!(diagnosis.readiness, RuntimeReadiness::Degraded);
    }

    #[tokio::test]
    async fn reports_ready_when_every_dependency_is_available() {
        let use_case = DiagnoseRuntime::new(FakeProbe {
            unavailable: Vec::new(),
        });

        let diagnosis = use_case.execute().await;

        assert_eq!(diagnosis.readiness, RuntimeReadiness::Ready);
    }
}
