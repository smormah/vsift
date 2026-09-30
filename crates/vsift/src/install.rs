//! `setup install`: the guarded managed-install transaction (P13, ADR 0023
//! §3 step 3).
//!
//! The order is fixed. The install guard is taken first and never waits (a
//! held guard is [`EngineError::ManagedInstallBusy`]). The plan is then
//! rebuilt from the machine as it is now and must equal the saved plan the
//! user reviewed, and the supplied digest must accept it; only then does the
//! application transaction install the plan's components, one at a time,
//! each fetched from its reviewed publisher (or the user's artifact folder),
//! verified by exact size and SHA-256, staged, smoked and activated on its
//! own. The compiled catalogue is the only trust anchor: the saved plan and
//! the artifact folder supply no URL, digest or file name.

use std::{path::PathBuf, time::Duration};

use vsift_application::{
    ManagedInstallReport, ManagedPlanAvailability, PlanAcceptanceError, ProgressSink,
    install_managed_components,
};
use vsift_contract::{InstallSource, SavedSetupPlan};
use vsift_domain::{FailureCode, ProgressUpdate, RuntimeDependency};
use vsift_infrastructure::{
    ActionAuthority, ExecutableResolver, ManagedArtifactError, ManagedArtifactSource,
    ManagedInstallerConfig, MediaProviderConformance, ReviewedFixtureVerifiers,
    ReviewedManagedInstaller, SmokeCompanionSource, SmokeCompanions, accepted_ubuntu_catalogue,
};

use crate::{
    asr::resolve_recognizer,
    engine::Engine,
    error::EngineError,
    progress::{JobProgress, ProgressObserver},
    setup::SetupPlanRequest,
    transcripts::resolve_media_tool,
    verification::Cancellation,
};

/// An accepted plan to install.
#[derive(Clone, Debug)]
pub struct SetupInstallRequest {
    /// The saved `setup plan --json` result the user reviewed.
    pub saved_plan: SavedSetupPlan,
    /// The digest the user accepted it by.
    pub accepted_digest: String,
    /// An absolute folder holding the plan's artifact files, for an offline
    /// install; `None` downloads each from its reviewed publisher.
    pub artifact_directory: Option<PathBuf>,
    /// Total deadline shared by the dependency probes that rebuild the plan.
    pub probe_timeout: Duration,
}

/// What one `setup install` did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupInstallOutcome {
    catalogue_revision: Option<String>,
    source: InstallSource,
    report: ManagedInstallReport,
}

impl SetupInstallOutcome {
    /// The accepted catalogue revision the plan installs from.
    #[must_use]
    pub fn catalogue_revision(&self) -> Option<&str> {
        self.catalogue_revision.as_deref()
    }

    /// Where the artifacts were read from.
    #[must_use]
    pub const fn source(&self) -> InstallSource {
        self.source
    }

    /// Each component of the plan, in order, and what happened to it.
    #[must_use]
    pub const fn report(&self) -> &ManagedInstallReport {
        &self.report
    }

    /// The public failure code of the transaction, or `None` when every
    /// component is installed.
    #[must_use]
    pub fn failure_code(&self) -> Option<FailureCode> {
        self.report.failure_code()
    }
}

impl Engine {
    /// Installs an accepted managed setup plan.
    ///
    /// # Errors
    ///
    /// Fails before any change when the artifact folder is not absolute, the
    /// host has no managed installation, the managed root cannot be used or
    /// is held by another installation, or the saved plan or digest does not
    /// accept the current plan. A component that fails during the
    /// transaction is not an error: it is reported in the outcome, with every
    /// component activated before it still active.
    pub async fn install_setup(
        &self,
        request: SetupInstallRequest,
        cancellation: &Cancellation,
        progress: ProgressObserver,
    ) -> Result<SetupInstallOutcome, EngineError> {
        let profile = request
            .saved_plan
            .profile()
            .map_err(EngineError::SavedPlanRejected)?;
        let source = match &request.artifact_directory {
            Some(directory) if directory.is_absolute() => {
                ManagedArtifactSource::Directory(directory.clone())
            }
            Some(_) => return Err(EngineError::ArtifactDirectoryNotAbsolute),
            None => ManagedArtifactSource::Publisher,
        };
        // Nothing is created on a host that has nothing to install.
        if self.managed_install_availability()? != ManagedPlanAvailability::Qualified {
            return Err(EngineError::PlanAcceptance(
                PlanAcceptanceError::ManagedUnavailable,
            ));
        }
        let store = self
            .managed_store()
            .ok_or(EngineError::ManagedStorageUnavailable)?;
        let guard = store.try_install_guard().map_err(|error| match error {
            ManagedArtifactError::Busy => EngineError::ManagedInstallBusy,
            ManagedArtifactError::Unavailable
            | ManagedArtifactError::UnsafeStorage
            | ManagedArtifactError::Io
            | ManagedArtifactError::Transfer(_) => EngineError::ManagedStorageUnavailable,
        })?;
        let current = self
            .plan_setup(SetupPlanRequest {
                profile,
                probe_timeout: request.probe_timeout,
            })
            .await?;
        current.validate_acceptance(&request.saved_plan, &request.accepted_digest)?;
        let plan = current.authority();
        let catalogue =
            accepted_ubuntu_catalogue().map_err(|_| EngineError::ReviewedPolicyInvalid)?;
        let install_source = match source {
            ManagedArtifactSource::Publisher => InstallSource::Publisher,
            ManagedArtifactSource::Directory(_) => InstallSource::ArtifactDirectory,
        };
        let sink = OperationProgress(&progress);
        let installer = ReviewedManagedInstaller::new(
            ManagedInstallerConfig {
                store,
                source,
                authority: ActionAuthority::ReviewedCatalogue,
                policy: catalogue.compatibility,
                host_isolation: self.config().host_isolation.into_infrastructure(),
                verifiers: ReviewedFixtureVerifiers,
                companions: SelectedCompanions { engine: self },
                cancellation: cancellation.0.clone(),
            },
            &guard,
            &sink,
        );
        let report = install_managed_components(&installer, &plan.actions, &sink).await;
        drop(installer);
        drop(guard);
        Ok(SetupInstallOutcome {
            catalogue_revision: plan.catalogue_revision.clone(),
            source: install_source,
            report,
        })
    }
}

/// The observer of one operation as the application's progress port.
struct OperationProgress<'observer>(&'observer ProgressObserver);

impl ProgressSink for OperationProgress<'_> {
    fn report(&self, update: ProgressUpdate) {
        self.0.report(&JobProgress { job: None, update });
    }
}

/// Smoke companions resolved in the order every command uses: configured,
/// then managed, then the filtered `PATH` (the model: configured, then
/// managed). The install guard is held throughout, so no managed version a
/// companion names can be removed during the smoke.
struct SelectedCompanions<'engine> {
    engine: &'engine Engine,
}

impl SmokeCompanionSource for SelectedCompanions<'_> {
    fn companions(&self) -> SmokeCompanions {
        let Ok(store) = self.engine.user_configuration() else {
            return SmokeCompanions::default();
        };
        let configured = store.read().unwrap_or_default();
        let resolver = ExecutableResolver::from_current_path();
        let mut managed = self.engine.managed_lookup();
        let ffmpeg = resolve_media_tool(
            &resolver,
            &mut managed,
            configured.ffmpeg,
            RuntimeDependency::Ffmpeg,
        );
        let ffprobe = resolve_media_tool(
            &resolver,
            &mut managed,
            configured.ffprobe,
            RuntimeDependency::Ffprobe,
        );
        let media = ffmpeg
            .ok()
            .zip(ffprobe.ok())
            .map(|(ffmpeg, ffprobe)| MediaProviderConformance::r0(ffmpeg, ffprobe));
        let whisper = resolve_recognizer(&mut managed, configured.whisper).ok();
        let model = match store.read_model() {
            Ok(Some(model)) => Some(model),
            Ok(None) | Err(_) => managed.model().map(|model| model.path),
        };
        SmokeCompanions {
            media,
            whisper,
            model,
        }
    }
}
