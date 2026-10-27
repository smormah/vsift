//! The guarded managed-install transaction (P13, ADR 0023 §3 step 3; ADR
//! 0036 section 4).
//!
//! For each component of an accepted plan the transaction fetches the
//! reviewed bytes (from the publisher, from a user's folder of the same
//! files, or a verified download an earlier run kept), verifies them by
//! size and SHA-256, stages them privately, prepares the runtime, runs the
//! compatibility smoke and only then publishes and selects it. Each
//! component activates atomically on its own, so a failure leaves every
//! component activated before it active, and a rerun of the same accepted
//! plan continues: a component whose reviewed version is already selected
//! is reported `already_current` and not fetched again.
//!
//! The core is two independent parts: video (the media tools) and speech
//! (the whisper.cpp program and its model). Their downloads run side by
//! side, and one failing does not stop the other's download. Activation is
//! in a fixed order, video first, because the speech smoke decodes its
//! fixture with video tools: when video failed and none are selected,
//! speech is reported `blocked` and its verified download is kept, so the
//! rerun after video is fixed verifies it again instead of downloading it.
//! Within a part, the first failure stops the part.
//!
//! The recognizer and its model need each other to be smoked: when the plan
//! installs both, they are staged together and smoked together (the model
//! is smoked with the staged whisper.cpp), then activated one after the
//! other. Every other component is smoked with the providers already
//! selected on the machine as companions. Nothing here decides how bytes
//! are fetched, staged or published: infrastructure does, behind
//! [`ManagedComponentInstaller`].
//!
//! A plan may also include optional components (ADR 0032), each a named
//! group of managed components. They are installed after every core
//! component, each group as a part of its own ([`install_managed_plan`]): a
//! core failure blocks them, but a failure of one never blocks or fails a
//! core component or another group, and the report keeps them apart, so a
//! proxy that blocks an optional download leaves `VSift` ready for local
//! videos and says so. The parts of one group that still need installing
//! are staged and smoked together, like the recognizer and its model.

use std::{
    future::{Future, poll_fn},
    pin::pin,
    task::Poll,
};

use vsift_domain::{FailureCode, ManagedComponent, ProgressStage, ProgressUpdate};

use crate::{
    CompatibilitySmoke, CompatibilitySmokeFailure, CompatibilitySmokeFailureReason, CorePart,
    ManagedSetupAction, OptionalComponent, ProgressSink, SmokeStageOutcome, StageDisposal,
    StagedManagedComponent, smoke_before_activation,
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
    /// Every core action of the plan, in order.
    pub components: Vec<ComponentInstallReport>,
    /// Every optional action of the plan, in order, after the core (#409);
    /// empty when the plan includes none.
    pub optional: Vec<ComponentInstallReport>,
}

impl ManagedInstallReport {
    /// The first core component failure that stopped the transaction, if
    /// any. An optional component's failure is never one.
    #[must_use]
    pub fn first_failure(&self) -> Option<(ManagedComponent, ComponentInstallFailure)> {
        first_failure_of(&self.components)
    }

    /// The public failure code of the transaction: that of its first core
    /// failure, or `None` when every core component is active, whatever
    /// happened to the optional part.
    #[must_use]
    pub fn failure_code(&self) -> Option<FailureCode> {
        self.first_failure()
            .and_then(|(_, failure)| failure.reason.failure_code())
    }

    /// The first optional component failure that stopped the optional
    /// part, if any. When the core failed, the optional part is blocked and
    /// this is `None`.
    #[must_use]
    pub fn optional_failure(&self) -> Option<(ManagedComponent, ComponentInstallFailure)> {
        first_failure_of(&self.optional)
    }

    /// Each optional component the plan includes, in plan order, with the
    /// first failure of its own part (`None` when every part of it is
    /// active, or when the core failed first and it was never attempted).
    #[must_use]
    pub fn optional_groups(&self) -> Vec<OptionalGroupOutcome> {
        let mut groups: Vec<OptionalGroupOutcome> = Vec::new();
        for report in &self.optional {
            let Some(group) = OptionalComponent::of(report.component) else {
                continue;
            };
            let installed = matches!(
                report.outcome,
                ComponentInstallOutcome::Activated | ComponentInstallOutcome::AlreadyCurrent
            );
            let failure = match report.outcome {
                ComponentInstallOutcome::Failed(failure)
                    if failure.reason != InstallFailureReason::Blocked =>
                {
                    Some((report.component, failure))
                }
                _ => None,
            };
            match groups.iter_mut().find(|outcome| outcome.group == group) {
                Some(outcome) => {
                    outcome.installed &= installed;
                    outcome.failure = outcome.failure.or(failure);
                }
                None => groups.push(OptionalGroupOutcome {
                    group,
                    installed,
                    failure,
                }),
            }
        }
        groups
    }

    /// Whether every optional component the plan includes is active.
    #[must_use]
    pub fn optional_installed(&self) -> bool {
        self.optional.iter().all(|report| {
            matches!(
                report.outcome,
                ComponentInstallOutcome::Activated | ComponentInstallOutcome::AlreadyCurrent
            )
        })
    }
}

/// What happened to one optional component of a plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionalGroupOutcome {
    /// The optional component.
    pub group: OptionalComponent,
    /// Whether every managed component of it is active now.
    pub installed: bool,
    /// The first failure of its own part; `None` when it is installed or
    /// was blocked by a core failure.
    pub failure: Option<(ManagedComponent, ComponentInstallFailure)>,
}

fn first_failure_of(
    reports: &[ComponentInstallReport],
) -> Option<(ManagedComponent, ComponentInstallFailure)> {
    reports.iter().find_map(|report| match report.outcome {
        ComponentInstallOutcome::Failed(failure)
            if failure.reason != InstallFailureReason::Blocked =>
        {
            Some((report.component, failure))
        }
        _ => None,
    })
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

/// What became of a verified download the transaction could not use yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadKeeping {
    /// It is kept for the next run, which verifies it again by its reviewed
    /// size and SHA-256 before it is used.
    Kept,
    /// It could not be kept, and cleanup did this with it instead.
    NotKept(StageDisposal),
}

/// The stage one piece of a plan has reached (ADR 0036 section 5): what a
/// person's progress line shows for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PieceStage {
    /// Not started yet.
    Waiting,
    /// Its reviewed bytes are arriving; [`PieceProgress::received`] counts
    /// them.
    Downloading,
    /// Its verified bytes are unpacked and checked against the review.
    Checking,
    /// Its compatibility smoke runs.
    Testing,
    /// Published and selected.
    Ready,
    /// Its reviewed version was already selected; nothing was fetched.
    AlreadyCurrent,
    /// Speech is downloaded and verified, and waits for working video tools
    /// to be tested with; its download is kept for the next run.
    WaitingForVideo,
    /// Not activated, for this reason.
    Failed(ComponentInstallFailure),
}

/// One observation of one piece of the plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PieceProgress {
    /// The piece.
    pub component: ManagedComponent,
    /// The stage it has reached.
    pub stage: PieceStage,
    /// The bytes of it received so far, of [`Self::total`].
    pub received: u64,
    /// Its reviewed size.
    pub total: u64,
}

/// Where the transaction reports each piece's stage and bytes (ADR 0036
/// section 5). It is a library-level observer for a person's progress
/// lines, beside the [`ProgressSink`] whose counts `--events jsonl` emits.
pub trait InstallObserver: Send + Sync {
    /// Receives one observation; called from the transaction's own task.
    fn piece(&self, progress: PieceProgress);
}

/// An observer that discards every observation.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoInstallObserver;

impl InstallObserver for NoInstallObserver {
    fn piece(&self, _progress: PieceProgress) {}
}

/// The infrastructure that fetches, stages, smokes and activates one
/// reviewed managed component.
///
/// Implementations hold the install guard for the whole transaction, verify
/// the complete bytes against the reviewed size and SHA-256 before anything
/// is extracted, and publish only a candidate the smoke passed. None of them
/// decides the order or what a failure means for the rest of the plan.
pub trait ManagedComponentInstaller: Send + Sync {
    /// A component's verified bytes, not yet unpacked.
    type Download: StagedManagedComponent;
    /// A staged, unactivated component.
    type Candidate: StagedManagedComponent;
    /// The compatibility smoke for candidates.
    type Smoke: CompatibilitySmoke<Self::Candidate>;

    /// Whether `action`'s exact reviewed version is the one selected now.
    fn is_current(&self, action: &ManagedSetupAction) -> bool;

    /// Fetches `action`'s reviewed bytes and verifies them against its
    /// reviewed size and SHA-256: a download kept by an earlier run, verified
    /// again, or else a fresh download or import. `progress` receives the
    /// `fetching_artifact` observations of this one artifact.
    fn fetch(
        &self,
        action: &ManagedSetupAction,
        progress: &dyn ProgressSink,
    ) -> impl Future<Output = Result<Self::Download, StageFailure>> + Send;

    /// Unpacks a verified download and stages it as an unactivated
    /// candidate with its private runtime prepared.
    ///
    /// # Errors
    ///
    /// The step and reason staging failed, with what cleanup did.
    fn stage(
        &self,
        download: Self::Download,
        action: &ManagedSetupAction,
    ) -> Result<Self::Candidate, StageFailure>;

    /// Keeps a verified download for a later run of the same plan.
    fn keep(&self, download: Self::Download, action: &ManagedSetupAction) -> DownloadKeeping;

    /// Whether working video tools are selected on the machine now, as the
    /// speech smoke needs them for its companion.
    fn has_media_tools(&self) -> bool;

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

/// Installs the accepted plan's core `actions` and reports each; the plan
/// includes no optional component.
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
    install_managed_plan(installer, actions, &[], progress).await
}

/// Installs an accepted plan: the core first, then each optional
/// component's actions as a part of its own (ADR 0032); see
/// [`install_observed_managed_plan`].
pub async fn install_managed_plan<I>(
    installer: &I,
    core: &[ManagedSetupAction],
    optional: &[ManagedSetupAction],
    progress: &dyn ProgressSink,
) -> ManagedInstallReport
where
    I: ManagedComponentInstaller,
{
    install_observed_managed_plan(installer, core, optional, progress, &NoInstallObserver).await
}

/// Installs an accepted plan, reporting each piece to `observer`.
///
/// The core is two independent parts (ADR 0036 section 4): video (the media
/// tools) and speech (the whisper.cpp program and its model). Their
/// downloads run side by side, one download per part at a time, and one
/// failing never stops the other's download and verification. Activation
/// stays one atomic step per component, video first, so that speech is
/// smoked with the video just activated. When video failed and no working
/// video tools are selected, speech cannot be tested: it is reported
/// `blocked` and its verified download is kept for the next run.
///
/// Optional components are installed only once the core is complete, each
/// as a part of its own: one failing stops only its own part, and
/// [`ManagedInstallReport::failure_code`] stays that of the core. The
/// components of one optional component that still need installing are
/// staged, smoked and activated as one unit. `optional` is in plan order,
/// each optional component's actions together; `progress` counts every
/// action as it finishes.
pub async fn install_observed_managed_plan<I>(
    installer: &I,
    core: &[ManagedSetupAction],
    optional: &[ManagedSetupAction],
    progress: &dyn ProgressSink,
    observer: &dyn InstallObserver,
) -> ManagedInstallReport
where
    I: ManagedComponentInstaller,
{
    let mut core_reports = blocked_reports(core);
    let mut optional_reports = blocked_reports(optional);
    let total = u64::try_from(core.len().saturating_add(optional.len())).unwrap_or(u64::MAX);
    let mut counter = Counter {
        progress,
        finished: 0,
        total,
    };
    counter.report();
    for action in core.iter().chain(optional) {
        observe(observer, action, PieceStage::Waiting);
    }

    let core_installed =
        install_core(installer, core, &mut core_reports, observer, &mut counter).await;
    if core_installed {
        let mut start = 0;
        while start < optional.len() {
            let group = OptionalComponent::of(optional[start].artifact.component);
            let length = optional[start..]
                .iter()
                .take_while(|action| OptionalComponent::of(action.artifact.component) == group)
                .count();
            let end = start + length.max(1);
            let mut part = Part::of(&optional[start..end], &mut optional_reports[start..end]);
            let pending = part.mark_current(installer, observer, &mut counter);
            let fetched = fetch_all(installer, part.actions, &pending, progress, observer).await;
            let downloads = part.settle_fetch(fetched, observer, &mut counter);
            if let Some(downloads) = downloads {
                part.install_downloads(
                    installer,
                    downloads,
                    Grouping::Together,
                    observer,
                    &mut counter,
                )
                .await;
            }
            start = end;
        }
    }
    ManagedInstallReport {
        components: core_reports,
        optional: optional_reports,
    }
}

/// Installs the core's video and speech parts; `true` when every core
/// component is active.
async fn install_core<I: ManagedComponentInstaller>(
    installer: &I,
    core: &[ManagedSetupAction],
    reports: &mut [ComponentInstallReport],
    observer: &dyn InstallObserver,
    counter: &mut Counter<'_>,
) -> bool {
    let split = core
        .iter()
        .position(|action| CorePart::of(action.artifact.component) == Some(CorePart::Speech))
        .unwrap_or(core.len());
    let (video_actions, speech_actions) = core.split_at(split);
    let (video_reports, speech_reports) = reports.split_at_mut(split);
    let mut video = Part::of(video_actions, video_reports);
    let mut speech = Part::of(speech_actions, speech_reports);
    let video_pending = video.mark_current(installer, observer, counter);
    let speech_pending = speech.mark_current(installer, observer, counter);

    let progress = counter.progress;
    let (video_fetched, speech_fetched) = join(
        fetch_all(installer, video.actions, &video_pending, progress, observer),
        fetch_all(
            installer,
            speech.actions,
            &speech_pending,
            progress,
            observer,
        ),
    )
    .await;

    let video_installed = match video.settle_fetch(video_fetched, observer, counter) {
        Some(downloads) => {
            video
                .install_downloads(installer, downloads, Grouping::Core, observer, counter)
                .await
        }
        None => false,
    };
    let Some(downloads) = speech.settle_fetch(speech_fetched, observer, counter) else {
        return false;
    };
    if downloads.is_empty() {
        return video_installed;
    }
    if !video_installed && !installer.has_media_tools() {
        if installer.is_cancelled() {
            speech.fail_downloads(downloads, cancelled(), observer, counter);
        } else {
            speech.keep_downloads(installer, downloads, observer);
        }
        return false;
    }
    speech
        .install_downloads(installer, downloads, Grouping::Core, observer, counter)
        .await
        && video_installed
}

/// How a part's pending components are grouped into units.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Grouping {
    /// The core: the recognizer and its model form one unit when both still
    /// need installing; every other component is a unit of its own.
    Core,
    /// An optional component: every part of it that still needs installing
    /// forms one unit, smoked together.
    Together,
}

fn blocked_reports(actions: &[ManagedSetupAction]) -> Vec<ComponentInstallReport> {
    actions
        .iter()
        .map(|action| ComponentInstallReport {
            component: action.artifact.component,
            version: action.artifact.version.clone(),
            outcome: ComponentInstallOutcome::Failed(ComponentInstallFailure::blocked()),
            stage: None,
        })
        .collect()
}

/// The `installing_components` count, reported as each component finishes.
struct Counter<'sink> {
    progress: &'sink dyn ProgressSink,
    finished: u64,
    total: u64,
}

impl Counter<'_> {
    fn finish(&mut self, components: usize) {
        self.finished = self
            .finished
            .saturating_add(u64::try_from(components).unwrap_or(u64::MAX));
        self.report();
    }

    fn report(&self) {
        self.progress.report(ProgressUpdate {
            stage: ProgressStage::InstallingComponents,
            completed: self.finished.min(self.total),
            total: Some(self.total),
            media: None,
        });
    }
}

/// One part of a plan: its actions and their reports, in plan order.
struct Part<'plan, 'report> {
    actions: &'plan [ManagedSetupAction],
    reports: &'report mut [ComponentInstallReport],
}

/// A part's fetches: every pending component's verified download, or the
/// first that failed, with the downloads made before it.
type Fetched<D> = Result<Vec<(usize, D)>, FetchFailure<D>>;

struct FetchFailure<D> {
    index: usize,
    failure: StageFailure,
    fetched: Vec<(usize, D)>,
}

impl<'plan> Part<'plan, '_> {
    fn of<'report>(
        actions: &'plan [ManagedSetupAction],
        reports: &'report mut [ComponentInstallReport],
    ) -> Part<'plan, 'report> {
        Part { actions, reports }
    }

    /// Marks every already current component and returns the others.
    fn mark_current<I: ManagedComponentInstaller>(
        &mut self,
        installer: &I,
        observer: &dyn InstallObserver,
        counter: &mut Counter<'_>,
    ) -> Vec<usize> {
        let mut pending = Vec::with_capacity(self.actions.len());
        for (index, action) in self.actions.iter().enumerate() {
            if installer.is_current(action) {
                self.reports[index].outcome = ComponentInstallOutcome::AlreadyCurrent;
                observe(observer, action, PieceStage::AlreadyCurrent);
                counter.finish(1);
            } else {
                pending.push(index);
            }
        }
        pending
    }

    /// Records a part's fetches: on a failure, the failed component's
    /// report, the downloads made before it discarded, and `None`.
    fn settle_fetch<D: StagedManagedComponent>(
        &mut self,
        fetched: Fetched<D>,
        observer: &dyn InstallObserver,
        counter: &mut Counter<'_>,
    ) -> Option<Vec<(usize, D)>> {
        match fetched {
            Ok(downloads) => Some(downloads),
            Err(FetchFailure {
                index,
                failure,
                fetched,
            }) => {
                self.record_failure(index, failure.failure, failure.stage, observer);
                counter.finish(1);
                for (fetched_index, download) in fetched {
                    self.reports[fetched_index].stage = Some(download.discard());
                }
                None
            }
        }
    }

    fn record_failure(
        &mut self,
        index: usize,
        failure: ComponentInstallFailure,
        stage: Option<StageDisposal>,
        observer: &dyn InstallObserver,
    ) {
        self.reports[index].outcome = ComponentInstallOutcome::Failed(failure);
        self.reports[index].stage = stage;
        observe(observer, &self.actions[index], PieceStage::Failed(failure));
    }

    /// Fails every download with `failure`, discarding each.
    fn fail_downloads<D: StagedManagedComponent>(
        &mut self,
        downloads: Vec<(usize, D)>,
        failure: ComponentInstallFailure,
        observer: &dyn InstallObserver,
        counter: &mut Counter<'_>,
    ) {
        let count = downloads.len();
        for (index, download) in downloads {
            let disposal = download.discard();
            self.record_failure(index, failure, Some(disposal), observer);
        }
        counter.finish(count);
    }

    /// Keeps every download for the next run; each stays `blocked`.
    fn keep_downloads<I: ManagedComponentInstaller>(
        &mut self,
        installer: &I,
        downloads: Vec<(usize, I::Download)>,
        observer: &dyn InstallObserver,
    ) {
        for (index, download) in downloads {
            let action = &self.actions[index];
            match installer.keep(download, action) {
                DownloadKeeping::Kept => {
                    observe(observer, action, PieceStage::WaitingForVideo);
                }
                DownloadKeeping::NotKept(disposal) => {
                    self.reports[index].stage = Some(disposal);
                    observe(
                        observer,
                        action,
                        PieceStage::Failed(ComponentInstallFailure::blocked()),
                    );
                }
            }
        }
    }

    /// Stages, smokes and activates the part's downloads, unit by unit, in
    /// order; `false` when a failure stopped the part.
    async fn install_downloads<I: ManagedComponentInstaller>(
        &mut self,
        installer: &I,
        downloads: Vec<(usize, I::Download)>,
        grouping: Grouping,
        observer: &dyn InstallObserver,
        counter: &mut Counter<'_>,
    ) -> bool {
        let mut units = units_of(self.actions, downloads, grouping).into_iter();
        while let Some(unit) = units.next() {
            let count = unit.len();
            let unit_installed = self.install_unit(installer, unit, observer).await;
            counter.finish(count);
            if !unit_installed {
                for rest in units {
                    for (index, download) in rest {
                        self.reports[index].stage = Some(download.discard());
                    }
                }
                return false;
            }
        }
        true
    }

    /// Stages, smokes and activates one unit; `false` when it failed.
    async fn install_unit<I: ManagedComponentInstaller>(
        &mut self,
        installer: &I,
        unit: Vec<(usize, I::Download)>,
        observer: &dyn InstallObserver,
    ) -> bool {
        let mut candidates = Vec::with_capacity(unit.len());
        let mut remaining = unit.into_iter();
        while let Some((index, download)) = remaining.next() {
            let action = &self.actions[index];
            let staged = if installer.is_cancelled() {
                Err(StageFailure {
                    failure: cancelled(),
                    stage: Some(download.discard()),
                })
            } else {
                observe(observer, action, PieceStage::Checking);
                installer.stage(download, action)
            };
            match staged {
                Ok(candidate) => candidates.push((index, candidate)),
                Err(failure) => {
                    self.record_failure(index, failure.failure, failure.stage, observer);
                    for (staged_index, candidate) in candidates {
                        self.reports[staged_index].stage = Some(candidate.discard());
                    }
                    for (rest, download) in remaining {
                        self.reports[rest].stage = Some(download.discard());
                    }
                    return false;
                }
            }
        }

        let (indices, candidates): (Vec<usize>, Vec<I::Candidate>) = candidates.into_iter().unzip();
        for index in &indices {
            observe(observer, &self.actions[*index], PieceStage::Testing);
        }
        let smoke = installer.smoke();
        let passed = match smoke_before_activation(&smoke, candidates).await {
            SmokeStageOutcome::Passed(passed) => passed,
            SmokeStageOutcome::Failed { failure, stages } => {
                for (index, (_, disposal)) in indices.iter().zip(stages) {
                    self.record_failure(
                        *index,
                        ComponentInstallFailure::at(
                            InstallStep::Smoke,
                            InstallFailureReason::Smoke(failure),
                        ),
                        Some(disposal),
                        observer,
                    );
                }
                return false;
            }
        };

        let mut remaining = indices.into_iter().zip(passed);
        while let Some((index, candidate)) = remaining.next() {
            let action = &self.actions[index];
            match installer.activate(candidate, action) {
                Ok(disposal) => {
                    self.reports[index].outcome = ComponentInstallOutcome::Activated;
                    self.reports[index].stage = Some(disposal);
                    observe(observer, action, PieceStage::Ready);
                }
                Err(failure) => {
                    self.record_failure(
                        index,
                        ComponentInstallFailure::at(InstallStep::Activate, failure.reason),
                        Some(failure.stage),
                        observer,
                    );
                    for (blocked, candidate) in remaining {
                        self.reports[blocked].stage = Some(candidate.discard());
                    }
                    return false;
                }
            }
        }
        true
    }
}

/// Groups a part's downloads into units, in order, as `grouping` says.
fn units_of<D>(
    actions: &[ManagedSetupAction],
    downloads: Vec<(usize, D)>,
    grouping: Grouping,
) -> Vec<Vec<(usize, D)>> {
    match grouping {
        Grouping::Together if downloads.is_empty() => Vec::new(),
        Grouping::Together => vec![downloads],
        Grouping::Core => {
            let has = |component| {
                downloads
                    .iter()
                    .any(|(index, _)| actions[*index].artifact.component == component)
            };
            let joined = has(ManagedComponent::WhisperCli) && has(ManagedComponent::WhisperModel);
            let mut units: Vec<Vec<(usize, D)>> = Vec::with_capacity(downloads.len());
            for (index, download) in downloads {
                let component = actions[index].artifact.component;
                let speech = matches!(
                    component,
                    ManagedComponent::WhisperCli | ManagedComponent::WhisperModel
                );
                match units.last_mut() {
                    Some(unit)
                        if joined
                            && speech
                            && unit.iter().any(|(first, _)| {
                                actions[*first].artifact.component == ManagedComponent::WhisperCli
                            }) =>
                    {
                        unit.push((index, download));
                    }
                    _ => units.push(vec![(index, download)]),
                }
            }
            units
        }
    }
}

/// Fetches each pending component of a part in order, one at a time,
/// stopping at the first failure.
async fn fetch_all<I: ManagedComponentInstaller>(
    installer: &I,
    actions: &[ManagedSetupAction],
    pending: &[usize],
    progress: &dyn ProgressSink,
    observer: &dyn InstallObserver,
) -> Fetched<I::Download> {
    let mut fetched = Vec::with_capacity(pending.len());
    for &index in pending {
        let action = &actions[index];
        let result = if installer.is_cancelled() {
            Err(StageFailure {
                failure: cancelled(),
                stage: None,
            })
        } else {
            observe(observer, action, PieceStage::Downloading);
            let sink = PieceSink {
                progress,
                observer,
                component: action.artifact.component,
            };
            installer.fetch(action, &sink).await
        };
        match result {
            Ok(download) => fetched.push((index, download)),
            Err(failure) => {
                return Err(FetchFailure {
                    index,
                    failure,
                    fetched,
                });
            }
        }
    }
    Ok(fetched)
}

/// The progress port one fetch reports to: its `fetching_artifact`
/// observations go to the caller's port unchanged, and to the observer as
/// the piece's bytes.
struct PieceSink<'run> {
    progress: &'run dyn ProgressSink,
    observer: &'run dyn InstallObserver,
    component: ManagedComponent,
}

impl ProgressSink for PieceSink<'_> {
    fn report(&self, update: ProgressUpdate) {
        if update.stage == ProgressStage::FetchingArtifact {
            self.observer.piece(PieceProgress {
                component: self.component,
                stage: PieceStage::Downloading,
                received: update.completed,
                total: update.total.unwrap_or(update.completed),
            });
        }
        self.progress.report(update);
    }
}

fn observe(observer: &dyn InstallObserver, action: &ManagedSetupAction, stage: PieceStage) {
    let total = action.artifact.integrity.bytes();
    let received = match stage {
        PieceStage::Waiting | PieceStage::Downloading | PieceStage::Failed(_) => 0,
        PieceStage::Checking
        | PieceStage::Testing
        | PieceStage::Ready
        | PieceStage::AlreadyCurrent
        | PieceStage::WaitingForVideo => total,
    };
    observer.piece(PieceProgress {
        component: action.artifact.component,
        stage,
        received,
        total,
    });
}

const fn cancelled() -> ComponentInstallFailure {
    ComponentInstallFailure {
        step: None,
        reason: InstallFailureReason::Cancelled,
    }
}

/// Runs two futures side by side on the caller's task and returns both
/// outputs. The application stays free of an async runtime, so this is the
/// whole of the concurrency: each future is polled whenever the task wakes,
/// and neither is polled again once it is ready.
async fn join<A: Future, B: Future>(first: A, second: B) -> (A::Output, B::Output) {
    let mut first = pin!(first);
    let mut second = pin!(second);
    let mut first_output = None;
    let mut second_output = None;
    poll_fn(|context| {
        if first_output.is_none()
            && let Poll::Ready(output) = first.as_mut().poll(context)
        {
            first_output = Some(output);
        }
        if second_output.is_none()
            && let Poll::Ready(output) = second.as_mut().poll(context)
        {
            second_output = Some(output);
        }
        if first_output.is_some() && second_output.is_some() {
            match (first_output.take(), second_output.take()) {
                (Some(first), Some(second)) => Poll::Ready((first, second)),
                _ => Poll::Pending,
            }
        } else {
            Poll::Pending
        }
    })
    .await
}
