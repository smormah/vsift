//! The guarded managed-install transaction (P13, ADR 0023 §3 step 3).
//!
//! An accepted plan's actions are installed one component at a time, in the
//! plan's dependency order (media tools, then the whisper.cpp CLI, then the
//! model). For each component the transaction fetches the reviewed bytes
//! (from the publisher, or from a user's folder of the same files), stages
//! them privately, prepares the runtime, runs the compatibility smoke and
//! only then publishes and selects it. Each component activates atomically
//! on its own, so a failure leaves every earlier component active and a
//! rerun of the same accepted plan continues where this one stopped: a
//! component whose reviewed version is already selected is reported
//! `already_current` and not fetched again.
//!
//! The recognizer and its model need each other to be smoked: when the plan
//! installs both, they are staged together and smoked together (the model
//! is smoked with the staged whisper.cpp), then activated one after the
//! other. Every other component is smoked with the providers already
//! selected on the machine as companions.
//!
//! The first component that fails stops the transaction; the components
//! after it are reported `blocked`, never attempted. Nothing here decides
//! how bytes are fetched, staged or published: infrastructure does, behind
//! [`ManagedComponentInstaller`].

use std::future::Future;

use vsift_domain::{FailureCode, ManagedComponent, ProgressStage, ProgressUpdate};

use crate::{
    CompatibilitySmoke, CompatibilitySmokeFailure, CompatibilitySmokeFailureReason,
    ManagedSetupAction, ProgressSink, SmokeStageOutcome, StageDisposal, StagedManagedComponent,
    smoke_before_activation,
};

/// Why a download from the reviewed publisher did not complete
/// (ADR 0023 decision H3). Reported under `DOWNLOAD_FAILED`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadFailureReason {
    /// The TLS handshake failed, for example on an untrusted certificate.
    Tls,
    /// The publisher redirected outside its reviewed route, to another host,
    /// with credentials, or too many times.
    RedirectPolicy,
    /// The publisher answered with something other than one complete
    /// `200 OK` body (a partial `206` included).
    HttpStatus,
    /// A proxy asked for authentication (`407`).
    ProxyAuth,
    /// No connection could be made or kept: no network, a refused or
    /// dropped connection, or a stalled transfer.
    Offline,
    /// The publisher's size differs from the reviewed size: a declared
    /// length, or a body shorter or longer than the review.
    Size,
}

impl DownloadFailureReason {
    /// Every reason, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Tls,
        Self::RedirectPolicy,
        Self::HttpStatus,
        Self::ProxyAuth,
        Self::Offline,
        Self::Size,
    ];

    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::RedirectPolicy => "redirect_policy",
            Self::HttpStatus => "http_status",
            Self::ProxyAuth => "proxy_auth",
            Self::Offline => "offline",
            Self::Size => "size",
        }
    }
}

/// The step of one component's installation that stopped it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallStep {
    /// Downloading the reviewed artifact from its publisher.
    Download,
    /// Importing the reviewed artifact from the user's `--artifact-dir`.
    Import,
    /// Extracting the verified artifact and preparing its private runtime.
    Stage,
    /// The compatibility smoke over the staged runtime.
    Smoke,
    /// Publishing and selecting the staged runtime.
    Activate,
}

impl InstallStep {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Import => "import",
            Self::Stage => "stage",
            Self::Smoke => "smoke",
            Self::Activate => "activate",
        }
    }
}

/// Why one component was not activated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallFailureReason {
    /// The download did not complete (`DOWNLOAD_FAILED`).
    Download(DownloadFailureReason),
    /// The artifact file named by the catalogue is not in the user's
    /// artifact folder (`INVALID_ARGUMENT`).
    ArtifactMissing,
    /// The file in the artifact folder is not a plain regular file: a
    /// link, a folder or a device (`INVALID_ARGUMENT`).
    ArtifactNotRegularFile,
    /// The complete bytes differ from the reviewed SHA-256
    /// (`INTEGRITY_FAILURE`); nothing of them was extracted or run.
    DigestMismatch,
    /// An imported file's size differs from the reviewed size
    /// (`INTEGRITY_FAILURE`).
    SizeMismatch,
    /// The verified artifact's contents, or a staged or published file,
    /// differ from the review; or the managed version already names other
    /// bytes (`INTEGRITY_FAILURE`).
    ReviewMismatch,
    /// Private managed storage could not be written or read (`STORAGE_IO`).
    Storage,
    /// The compatibility smoke failed at this check for this reason. A
    /// cancelled smoke is `CANCELLED`, one `VSift` could not prepare is
    /// `STORAGE_IO`, and every other is `MISSING_CAPABILITY`: the reviewed
    /// tools do not work on this machine.
    Smoke(CompatibilitySmokeFailure),
    /// The caller cancelled the installation (`CANCELLED`).
    Cancelled,
    /// Not activated because an earlier component of the plan failed or was
    /// cancelled first; carries no code of its own.
    Blocked,
}

impl InstallFailureReason {
    /// Stable machine-readable reason identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Download(reason) => reason.identifier(),
            Self::ArtifactMissing => "artifact_missing",
            Self::ArtifactNotRegularFile => "artifact_not_regular_file",
            Self::DigestMismatch => "digest_mismatch",
            Self::SizeMismatch => "size_mismatch",
            Self::ReviewMismatch => "review_mismatch",
            Self::Storage => "storage",
            Self::Smoke(failure) => failure.reason.identifier(),
            Self::Cancelled => "cancelled",
            Self::Blocked => "blocked",
        }
    }

    /// The public failure code this reason makes the command fail with, or
    /// `None` for a component blocked by an earlier one.
    #[must_use]
    pub const fn failure_code(self) -> Option<FailureCode> {
        Some(match self {
            Self::Download(_) => FailureCode::DownloadFailed,
            Self::ArtifactMissing | Self::ArtifactNotRegularFile => FailureCode::InvalidArgument,
            Self::DigestMismatch | Self::SizeMismatch | Self::ReviewMismatch => {
                FailureCode::IntegrityFailure
            }
            Self::Storage => FailureCode::StorageIo,
            Self::Smoke(failure) => match failure.reason {
                CompatibilitySmokeFailureReason::Cancelled => FailureCode::Cancelled,
                CompatibilitySmokeFailureReason::Preparation => FailureCode::StorageIo,
                _ => FailureCode::MissingCapability,
            },
            Self::Cancelled => FailureCode::Cancelled,
            Self::Blocked => return None,
        })
    }
}

/// One component's failure: the step it reached and why it stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComponentInstallFailure {
    /// The step that failed; `None` for a component that was never
    /// attempted.
    pub step: Option<InstallStep>,
    /// Why it failed.
    pub reason: InstallFailureReason,
}

impl ComponentInstallFailure {
    /// A failure at `step`.
    #[must_use]
    pub const fn at(step: InstallStep, reason: InstallFailureReason) -> Self {
        Self {
            step: Some(step),
            reason,
        }
    }

    const fn blocked() -> Self {
        Self {
            step: None,
            reason: InstallFailureReason::Blocked,
        }
    }
}

/// What the transaction did with one component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentInstallOutcome {
    /// The reviewed version was published and is now selected.
    Activated,
    /// The reviewed version was already selected; nothing was fetched.
    AlreadyCurrent,
    /// The component was not activated.
    Failed(ComponentInstallFailure),
}

impl ComponentInstallOutcome {
    /// Stable status identifier: `activated`, `already_current` or `failed`.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Activated => "activated",
            Self::AlreadyCurrent => "already_current",
            Self::Failed(_) => "failed",
        }
    }
}

/// One component of the accepted plan and what happened to it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentInstallReport {
    /// The reviewed component.
    pub component: ManagedComponent,
    /// The reviewed version the plan installs.
    pub version: String,
    /// What happened.
    pub outcome: ComponentInstallOutcome,
    /// What cleanup did with the component's private stage, when one was
    /// made and not activated.
    pub stage: Option<StageDisposal>,
}

/// The outcome of one accepted plan's installation, in plan order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedInstallReport {
    /// Every action of the plan, in order.
    pub components: Vec<ComponentInstallReport>,
}

impl ManagedInstallReport {
    /// The first component failure that stopped the transaction, if any.
    #[must_use]
    pub fn first_failure(&self) -> Option<(ManagedComponent, ComponentInstallFailure)> {
        self.components
            .iter()
            .find_map(|report| match report.outcome {
                ComponentInstallOutcome::Failed(failure)
                    if failure.reason != InstallFailureReason::Blocked =>
                {
                    Some((report.component, failure))
                }
                _ => None,
            })
    }

    /// The public failure code of the transaction: that of its first
    /// failure, or `None` when every component is active.
    #[must_use]
    pub fn failure_code(&self) -> Option<FailureCode> {
        self.first_failure()
            .and_then(|(_, failure)| failure.reason.failure_code())
    }
}

/// A component that could not be staged, and what cleanup did with the
/// stage when one was made.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StageFailure {
    /// The failing step and reason.
    pub failure: ComponentInstallFailure,
    /// The stage's disposal, when a stage existed.
    pub stage: Option<StageDisposal>,
}

/// A staged component that could not be published and selected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivationFailure {
    /// Why activation failed.
    pub reason: InstallFailureReason,
    /// What cleanup did with the rest of the stage.
    pub stage: StageDisposal,
}

/// The infrastructure that fetches, stages, smokes and activates one
/// reviewed managed component.
///
/// Implementations hold the install guard for the whole transaction, verify
/// the complete bytes against the reviewed size and SHA-256 before anything
/// is extracted, and publish only a candidate the smoke passed. None of them
/// decides the order or what a failure means for the rest of the plan.
pub trait ManagedComponentInstaller: Send + Sync {
    /// A staged, unactivated component.
    type Candidate: StagedManagedComponent;
    /// The compatibility smoke for candidates.
    type Smoke: CompatibilitySmoke<Self::Candidate>;

    /// Whether `action`'s exact reviewed version is the one selected now.
    fn is_current(&self, action: &ManagedSetupAction) -> bool;

    /// Fetches `action`'s reviewed bytes, verifies them and stages them as
    /// an unactivated candidate with its private runtime prepared.
    fn stage(
        &self,
        action: &ManagedSetupAction,
    ) -> impl Future<Output = Result<Self::Candidate, StageFailure>> + Send;

    /// The smoke for the next candidates, with the providers selected on
    /// the machine at this moment as companions.
    fn smoke(&self) -> Self::Smoke;

    /// Publishes and selects `candidate`, which passed its smoke, then
    /// removes the rest of its stage.
    ///
    /// # Errors
    ///
    /// The reason activation failed, with what cleanup did with the stage.
    fn activate(
        &self,
        candidate: Self::Candidate,
        action: &ManagedSetupAction,
    ) -> Result<StageDisposal, ActivationFailure>;

    /// Whether the caller has cancelled the installation.
    fn is_cancelled(&self) -> bool;
}

/// Installs the accepted plan's `actions` in their order and reports each.
///
/// The caller holds the install guard and has revalidated the plan against
/// the current machine and the supplied acceptance digest immediately
/// before. `progress` receives one `installing_components` update as each
/// component finishes.
pub async fn install_managed_components<I>(
    installer: &I,
    actions: &[ManagedSetupAction],
    progress: &dyn ProgressSink,
) -> ManagedInstallReport
where
    I: ManagedComponentInstaller,
{
    let mut reports: Vec<ComponentInstallReport> = actions
        .iter()
        .map(|action| ComponentInstallReport {
            component: action.artifact.component,
            version: action.artifact.version.clone(),
            outcome: ComponentInstallOutcome::Failed(ComponentInstallFailure::blocked()),
            stage: None,
        })
        .collect();
    let total = u64::try_from(actions.len()).unwrap_or(u64::MAX);
    let mut finished = 0_u64;
    report_components(progress, finished, total);

    let mut stopped = false;
    for unit in install_units(installer, actions, &mut reports) {
        if stopped {
            break;
        }
        finished = finished.saturating_add(count(&unit.already_current));
        if unit.pending.is_empty() {
            report_components(progress, finished, total);
            continue;
        }
        stopped = !install_unit(installer, actions, &unit.pending, &mut reports).await;
        finished = finished.saturating_add(count(&unit.pending));
        report_components(progress, finished, total);
    }
    ManagedInstallReport {
        components: reports,
    }
}

/// Components installed together: the ones already current, and the ones
/// to fetch, stage, smoke together and activate in order.
struct InstallUnit {
    already_current: Vec<usize>,
    pending: Vec<usize>,
}

/// Groups the plan into units, marking every already current component.
///
/// The whisper.cpp CLI and the model form one unit when both still need
/// installing, because neither can be smoked without the other; every
/// other component is a unit of its own.
fn install_units<I: ManagedComponentInstaller>(
    installer: &I,
    actions: &[ManagedSetupAction],
    reports: &mut [ComponentInstallReport],
) -> Vec<InstallUnit> {
    let mut pending = Vec::with_capacity(actions.len());
    let mut units: Vec<InstallUnit> = Vec::with_capacity(actions.len());
    for (index, action) in actions.iter().enumerate() {
        if installer.is_current(action) {
            reports[index].outcome = ComponentInstallOutcome::AlreadyCurrent;
            units.push(InstallUnit {
                already_current: vec![index],
                pending: Vec::new(),
            });
        } else {
            pending.push(index);
            units.push(InstallUnit {
                already_current: Vec::new(),
                pending: vec![index],
            });
        }
    }
    let whisper = pending
        .iter()
        .position(|&index| actions[index].artifact.component == ManagedComponent::WhisperCli);
    let model = pending
        .iter()
        .position(|&index| actions[index].artifact.component == ManagedComponent::WhisperModel);
    if let (Some(whisper), Some(model)) = (whisper, model) {
        let whisper_index = pending[whisper];
        let model_index = pending[model];
        units.retain(|unit| unit.pending != [model_index]);
        if let Some(unit) = units
            .iter_mut()
            .find(|unit| unit.pending == [whisper_index])
        {
            unit.pending.push(model_index);
        }
    }
    units
}

/// Stages, smokes and activates one unit; `false` when it stopped the
/// transaction.
async fn install_unit<I: ManagedComponentInstaller>(
    installer: &I,
    actions: &[ManagedSetupAction],
    unit: &[usize],
    reports: &mut [ComponentInstallReport],
) -> bool {
    let mut candidates = Vec::with_capacity(unit.len());
    for &index in unit {
        let staged = if installer.is_cancelled() {
            Err(StageFailure {
                failure: ComponentInstallFailure {
                    step: None,
                    reason: InstallFailureReason::Cancelled,
                },
                stage: None,
            })
        } else {
            installer.stage(&actions[index]).await
        };
        match staged {
            Ok(candidate) => candidates.push((index, candidate)),
            Err(failure) => {
                reports[index].outcome = ComponentInstallOutcome::Failed(failure.failure);
                reports[index].stage = failure.stage;
                for (staged_index, candidate) in candidates {
                    reports[staged_index].stage = Some(candidate.discard());
                }
                return false;
            }
        }
    }

    let (indices, candidates): (Vec<usize>, Vec<I::Candidate>) = candidates.into_iter().unzip();
    let smoke = installer.smoke();
    let passed = match smoke_before_activation(&smoke, candidates).await {
        SmokeStageOutcome::Passed(passed) => passed,
        SmokeStageOutcome::Failed { failure, stages } => {
            for (index, (_, disposal)) in indices.iter().zip(stages) {
                reports[*index].outcome =
                    ComponentInstallOutcome::Failed(ComponentInstallFailure::at(
                        InstallStep::Smoke,
                        InstallFailureReason::Smoke(failure),
                    ));
                reports[*index].stage = Some(disposal);
            }
            return false;
        }
    };

    let mut remaining = indices.into_iter().zip(passed);
    while let Some((index, candidate)) = remaining.next() {
        match installer.activate(candidate, &actions[index]) {
            Ok(disposal) => {
                reports[index].outcome = ComponentInstallOutcome::Activated;
                reports[index].stage = Some(disposal);
            }
            Err(failure) => {
                reports[index].outcome = ComponentInstallOutcome::Failed(
                    ComponentInstallFailure::at(InstallStep::Activate, failure.reason),
                );
                reports[index].stage = Some(failure.stage);
                for (blocked, candidate) in remaining {
                    reports[blocked].stage = Some(candidate.discard());
                }
                return false;
            }
        }
    }
    true
}

fn count(indices: &[usize]) -> u64 {
    u64::try_from(indices.len()).unwrap_or(u64::MAX)
}

fn report_components(progress: &dyn ProgressSink, finished: u64, total: u64) {
    progress.report(ProgressUpdate {
        stage: ProgressStage::InstallingComponents,
        completed: finished.min(total),
        total: Some(total),
    });
}
