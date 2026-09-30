//! The reviewed managed-component installer behind the application's
//! install transaction (P13, ADR 0023 §3 step 3).
//!
//! It holds the root's install guard for the whole transaction. For one
//! accepted action it binds the action to the compiled catalogue's exact
//! literals, fetches the artifact (from the publisher over HTTPS, or from
//! the user's `--artifact-dir`) into a private stage while checking the
//! reviewed size and SHA-256, extracts the reviewed selection, prepares the
//! runtime, hands the candidate to PR 3's compatibility smoke and, when the
//! smoke passed, publishes and selects it. Bytes that fail their digest are
//! discarded before anything is extracted, so they can never reach
//! publication (D-02).

use std::{fmt, path::PathBuf};

use vsift_application::{
    ActivationFailure, ComponentInstallFailure, DownloadFailureReason, InstallFailureReason,
    InstallStep, ManagedComponentInstaller, ManagedSetupAction, ProgressSink,
    ReviewedCompatibilityPolicy, StageFailure,
};

use crate::{
    ArtifactTransferError, HostIsolation, ManagedArtifactError, ManagedArtifactStore,
    ManagedCandidateError, ManagedCandidateFailure, ManagedInstallGuard, ManagedPayloadError,
    ManagedRuntimeIdentity, ManagedRuntimeLayoutError, ManagedRuntimePublicationError,
    ProcessCancellation, PublisherTransferError, ReviewedUbuntuAction, SmokeCompanions,
    SmokeFixtureVerifiers, StagedCompatibilitySmoke, StagedManagedCandidate,
    download_reviewed_publisher_artifact,
    offline_artifact_import::{ArtifactImportError, import_reviewed_artifact},
};

/// Where the reviewed bytes come from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedArtifactSource {
    /// Each artifact is downloaded from its reviewed publisher over HTTPS.
    Publisher,
    /// Each artifact is imported from this absolute folder, by the file
    /// name the catalogue gives it (`setup install --artifact-dir`).
    Directory(PathBuf),
}

/// How an accepted action becomes transport and staging authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionAuthority {
    /// The compiled Ubuntu catalogue: every action must equal its reviewed
    /// literals exactly.
    ReviewedCatalogue,
    /// Development builds only (`install-test-hooks`): each action's
    /// artifact is served by a local test server at `<base_url>/<file
    /// name>`, optionally through `proxy`. The action's size, SHA-256 and
    /// layout are still checked; only the publisher route is replaced.
    #[cfg(any(test, feature = "install-test-hooks"))]
    Loopback {
        /// `http://127.0.0.1:<port>` or `https://127.0.0.1:<port>`.
        base_url: String,
        /// An explicit proxy in place of the system settings.
        proxy: Option<String>,
    },
}

impl ActionAuthority {
    fn resolve(&self, action: &ManagedSetupAction) -> Option<ReviewedUbuntuAction> {
        match self {
            Self::ReviewedCatalogue => ReviewedUbuntuAction::from_accepted_action(action).ok(),
            #[cfg(any(test, feature = "install-test-hooks"))]
            Self::Loopback { base_url, proxy } => {
                ReviewedUbuntuAction::loopback_for_tests(action, base_url, proxy.as_deref()).ok()
            }
        }
    }
}

/// Resolves the providers selected on the machine, for a smoke whose
/// candidates need a companion they do not include.
pub trait SmokeCompanionSource: Send + Sync {
    /// The selected media tools, recognizer and model at this moment.
    fn companions(&self) -> SmokeCompanions;
}

/// Everything one installation transaction runs with.
pub struct ManagedInstallerConfig<V, C> {
    /// The managed root.
    pub store: ManagedArtifactStore,
    /// Where the bytes come from.
    pub source: ManagedArtifactSource,
    /// How actions are bound to reviewed source.
    pub authority: ActionAuthority,
    /// The accepted plan's compatibility policy.
    pub policy: ReviewedCompatibilityPolicy,
    /// Host isolation for every smoke provider run.
    pub host_isolation: HostIsolation,
    /// The smoke's fixture verifiers.
    pub verifiers: V,
    /// Where companions come from.
    pub companions: C,
    /// The command's cancellation.
    pub cancellation: ProcessCancellation,
}

/// The production [`ManagedComponentInstaller`].
pub struct ReviewedManagedInstaller<'run, V, C> {
    config: ManagedInstallerConfig<V, C>,
    guard: &'run ManagedInstallGuard,
    progress: &'run dyn ProgressSink,
}

impl<V, C> fmt::Debug for ReviewedManagedInstaller<'_, V, C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReviewedManagedInstaller")
            .field("source", &self.config.source)
            .field("authority", &self.config.authority)
            .finish_non_exhaustive()
    }
}

impl<'run, V, C> ReviewedManagedInstaller<'run, V, C> {
    /// An installer over `config` that publishes under `guard`, which must
    /// be the install guard of `config.store`, and reports fetch progress
    /// to `progress`.
    #[must_use]
    pub const fn new(
        config: ManagedInstallerConfig<V, C>,
        guard: &'run ManagedInstallGuard,
        progress: &'run dyn ProgressSink,
    ) -> Self {
        Self {
            config,
            guard,
            progress,
        }
    }

    const fn fetch_step(&self) -> InstallStep {
        match self.config.source {
            ManagedArtifactSource::Publisher => InstallStep::Download,
            ManagedArtifactSource::Directory(_) => InstallStep::Import,
        }
    }
}

impl<V, C> ManagedComponentInstaller for ReviewedManagedInstaller<'_, V, C>
where
    V: SmokeFixtureVerifiers + Clone,
    C: SmokeCompanionSource,
{
    type Candidate = StagedManagedCandidate;
    type Smoke = StagedCompatibilitySmoke<V>;

    fn is_current(&self, action: &ManagedSetupAction) -> bool {
        let component = action.artifact.component.identifier();
        let Ok(identity) = ManagedRuntimeIdentity::new(component, action.artifact.version.clone())
        else {
            return false;
        };
        matches!(
            self.config.store.open_selected_runtime(component),
            Ok(Some(runtime)) if runtime.identity() == &identity
        )
    }

    async fn stage(
        &self,
        action: &ManagedSetupAction,
    ) -> Result<StagedManagedCandidate, StageFailure> {
        let step = self.fetch_step();
        let fetch_failure = |reason| StageFailure {
            failure: ComponentInstallFailure::at(step, reason),
            stage: None,
        };
        let Some(route) = self.config.authority.resolve(action) else {
            return Err(fetch_failure(InstallFailureReason::ReviewMismatch));
        };
        let integrity = route.publisher_source().integrity();
        let staged = match &self.config.source {
            ManagedArtifactSource::Publisher => download_reviewed_publisher_artifact(
                &self.config.store,
                route.publisher_source(),
                &self.config.cancellation,
                self.progress,
            )
            .await
            .map_err(|error| fetch_failure(transfer_reason(&error)))?,
            ManagedArtifactSource::Directory(directory) => {
                let Some(file_name) = route.publisher_source().file_name() else {
                    return Err(fetch_failure(InstallFailureReason::ReviewMismatch));
                };
                import_reviewed_artifact(
                    &self.config.store,
                    directory,
                    file_name,
                    integrity,
                    &self.config.cancellation,
                    self.progress,
                )
                .await
                .map_err(|error| fetch_failure(import_reason(&error)))?
            }
        };
        route
            .stage_candidate(staged)
            .map_err(|error: ManagedCandidateError| StageFailure {
                failure: ComponentInstallFailure::at(
                    InstallStep::Stage,
                    candidate_reason(&error.failure),
                ),
                stage: Some(error.stage),
            })
    }

    fn smoke(&self) -> StagedCompatibilitySmoke<V> {
        StagedCompatibilitySmoke::new(
            self.config.policy.clone(),
            self.config.host_isolation,
            self.config.companions.companions(),
            self.config.verifiers.clone(),
            self.config.cancellation.clone(),
        )
    }

    fn activate(
        &self,
        candidate: StagedManagedCandidate,
        action: &ManagedSetupAction,
    ) -> Result<vsift_application::StageDisposal, ActivationFailure> {
        let Ok(identity) = ManagedRuntimeIdentity::new(
            action.artifact.component.identifier(),
            action.artifact.version.clone(),
        ) else {
            return Err(ActivationFailure {
                reason: InstallFailureReason::ReviewMismatch,
                stage: candidate.discard(),
            });
        };
        let (published, stage) = candidate.publish_and_select(self.guard, &identity);
        match published {
            Ok(runtime) => {
                drop(runtime);
                Ok(stage)
            }
            Err(error) => Err(ActivationFailure {
                reason: publication_reason(&error),
                stage,
            }),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.config.cancellation.is_cancelled()
    }
}

/// The reason of a download that failed.
fn transfer_reason(error: &PublisherTransferError) -> InstallFailureReason {
    match error {
        PublisherTransferError::Failed(reason) => InstallFailureReason::Download(*reason),
        PublisherTransferError::Cancelled => InstallFailureReason::Cancelled,
        PublisherTransferError::Staging(ManagedArtifactError::Transfer(
            ArtifactTransferError::Truncated | ArtifactTransferError::Oversized,
        )) => InstallFailureReason::Download(DownloadFailureReason::Size),
        PublisherTransferError::Staging(error) => staging_reason(error),
    }
}

/// The reason of an offline import that failed.
fn import_reason(error: &ArtifactImportError) -> InstallFailureReason {
    match error {
        ArtifactImportError::Missing => InstallFailureReason::ArtifactMissing,
        ArtifactImportError::NotRegularFile => InstallFailureReason::ArtifactNotRegularFile,
        ArtifactImportError::SizeMismatch
        | ArtifactImportError::Staging(ManagedArtifactError::Transfer(
            ArtifactTransferError::Truncated | ArtifactTransferError::Oversized,
        )) => InstallFailureReason::SizeMismatch,
        ArtifactImportError::Unreadable => InstallFailureReason::Storage,
        ArtifactImportError::Cancelled => InstallFailureReason::Cancelled,
        ArtifactImportError::Staging(error) => staging_reason(error),
    }
}

/// A staging failure: a digest mismatch is integrity, anything else is
/// private storage.
const fn staging_reason(error: &ManagedArtifactError) -> InstallFailureReason {
    match error {
        ManagedArtifactError::Transfer(ArtifactTransferError::DigestMismatch) => {
            InstallFailureReason::DigestMismatch
        }
        ManagedArtifactError::Transfer(
            ArtifactTransferError::Truncated | ArtifactTransferError::Oversized,
        ) => InstallFailureReason::SizeMismatch,
        ManagedArtifactError::Transfer(
            ArtifactTransferError::SourceIo(_) | ArtifactTransferError::DestinationIo(_),
        )
        | ManagedArtifactError::Unavailable
        | ManagedArtifactError::UnsafeStorage
        | ManagedArtifactError::Busy
        | ManagedArtifactError::Io => InstallFailureReason::Storage,
    }
}

/// A verified artifact whose contents or layout could not be staged.
const fn candidate_reason(failure: &ManagedCandidateFailure) -> InstallFailureReason {
    match failure {
        ManagedCandidateFailure::Payload(ManagedPayloadError::Storage(_))
        | ManagedCandidateFailure::Runtime(ManagedRuntimeLayoutError::Storage(_)) => {
            InstallFailureReason::Storage
        }
        ManagedCandidateFailure::InvalidReview
        | ManagedCandidateFailure::IntegrityMismatch
        | ManagedCandidateFailure::Payload(_)
        | ManagedCandidateFailure::Runtime(_) => InstallFailureReason::ReviewMismatch,
    }
}

/// A publication that failed: changed bytes or a conflicting version are
/// integrity; the rest is storage.
const fn publication_reason(error: &ManagedRuntimePublicationError) -> InstallFailureReason {
    match error {
        ManagedRuntimePublicationError::InvalidIdentity
        | ManagedRuntimePublicationError::VersionConflict
        | ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::Transfer(_) | ManagedArtifactError::UnsafeStorage,
        ) => InstallFailureReason::ReviewMismatch,
        ManagedRuntimePublicationError::Storage(
            ManagedArtifactError::Unavailable
            | ManagedArtifactError::Busy
            | ManagedArtifactError::Io,
        ) => InstallFailureReason::Storage,
    }
}
