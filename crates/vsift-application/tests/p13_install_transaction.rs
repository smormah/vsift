//! P13 PR 4: the managed-install transaction through its application port
//! (D-02, D-03, D-06).
//!
//! A test catalogue produces the accepted plan's actions and a fake installer
//! records what it staged, smoked, discarded and activated. The tests pin the
//! order (media tools, then the whisper.cpp CLI with the model), that each
//! component activates on its own, that a rerun continues from the first
//! component not yet current, that the first failure stops the transaction,
//! and what each failure makes the command report.

use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

use vsift_application::{
    AcceptedManagedArtifact, AcceptedManagedCatalogue, ActivationFailure, CompatibilitySmoke,
    CompatibilitySmokeCheck, CompatibilitySmokeFailure, CompatibilitySmokeFailureReason,
    CompatibilitySmokeVerdict, ComponentInstallFailure, ComponentInstallOutcome,
    DownloadFailureReason, InstallFailureReason, InstallStep, ManagedComponentInstaller,
    ManagedPlanAvailability, ManagedSetupAction, ManagedSetupPlan, OptionalComponent,
    OptionalComponentDisposition, OptionalComponentSet, ProgressSink, ReviewedBanner,
    ReviewedCompatibilityPolicy, ReviewedManagedFile, RuntimeDiagnosis, SetupProfile,
    SetupSelectionState, StageDisposal, StageFailure, StageRetentionReason, StagedManagedComponent,
    install_managed_components, install_managed_plan, plan_managed_setup,
    plan_managed_setup_including,
};
use vsift_domain::{
    ArtifactIntegrity, DependencyState, DependencyStatus, FailureCode, ManagedArtifactFormat,
    ManagedComponent, ManagedTarget, ProgressStage, ProgressUpdate, RuntimeDependency,
    RuntimeReadiness, SharedLibraryName,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const MEDIA: ManagedComponent = ManagedComponent::MediaTools;
const WHISPER: ManagedComponent = ManagedComponent::WhisperCli;
const MODEL: ManagedComponent = ManagedComponent::WhisperModel;

/// Everything the fake installer did, in order.
#[derive(Debug, Default)]
struct Journal {
    staged: Vec<ManagedComponent>,
    smoked: Vec<Vec<ManagedComponent>>,
    discarded: Vec<ManagedComponent>,
    activated: Vec<ManagedComponent>,
}

#[derive(Debug)]
struct FakeCandidate {
    component: ManagedComponent,
    journal: Arc<Mutex<Journal>>,
}

impl StagedManagedComponent for FakeCandidate {
    fn component(&self) -> ManagedComponent {
        self.component
    }

    fn discard(self) -> StageDisposal {
        lock(&self.journal).discarded.push(self.component);
        StageDisposal::Discarded
    }
}

struct FakeSmoke {
    verdict: CompatibilitySmokeVerdict,
    journal: Arc<Mutex<Journal>>,
}

impl CompatibilitySmoke<FakeCandidate> for FakeSmoke {
    fn smoke(
        &self,
        candidates: &[FakeCandidate],
    ) -> impl Future<Output = CompatibilitySmokeVerdict> + Send {
        lock(&self.journal).smoked.push(
            candidates
                .iter()
                .map(|candidate| candidate.component)
                .collect(),
        );
        std::future::ready(self.verdict)
    }
}

/// A fake installer whose behaviour per component is set by each test.
struct FakeInstaller {
    journal: Arc<Mutex<Journal>>,
    current: Vec<ManagedComponent>,
    stage_failures: BTreeMap<u8, StageFailure>,
    activation_failures: BTreeMap<u8, ActivationFailure>,
    smoke_verdicts: Mutex<Vec<CompatibilitySmokeVerdict>>,
    cancelled: AtomicBool,
    cancel_after_staging: Option<ManagedComponent>,
}

impl FakeInstaller {
    fn new() -> Self {
        Self {
            journal: Arc::new(Mutex::new(Journal::default())),
            current: Vec::new(),
            stage_failures: BTreeMap::new(),
            activation_failures: BTreeMap::new(),
            smoke_verdicts: Mutex::new(Vec::new()),
            cancelled: AtomicBool::new(false),
            cancel_after_staging: None,
        }
    }

    fn journal(&self) -> std::sync::MutexGuard<'_, Journal> {
        lock(&self.journal)
    }
}

const fn key(component: ManagedComponent) -> u8 {
    match component {
        ManagedComponent::MediaTools => 0,
        ManagedComponent::WhisperCli => 1,
        ManagedComponent::WhisperModel => 2,
        ManagedComponent::JsRuntimeDeno => 3,
        ManagedComponent::YtDlp => 4,
    }
}

impl ManagedComponentInstaller for FakeInstaller {
    type Candidate = FakeCandidate;
    type Smoke = FakeSmoke;

    fn is_current(&self, action: &ManagedSetupAction) -> bool {
        self.current.contains(&action.artifact.component)
    }

    fn stage(
        &self,
        action: &ManagedSetupAction,
    ) -> impl Future<Output = Result<Self::Candidate, StageFailure>> + Send {
        let component = action.artifact.component;
        let result = if let Some(failure) = self.stage_failures.get(&key(component)) {
            Err(*failure)
        } else {
            self.journal().staged.push(component);
            if self.cancel_after_staging == Some(component) {
                self.cancelled.store(true, Ordering::SeqCst);
            }
            Ok(FakeCandidate {
                component,
                journal: Arc::clone(&self.journal),
            })
        };
        std::future::ready(result)
    }

    fn smoke(&self) -> Self::Smoke {
        let mut verdicts = self
            .smoke_verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let verdict = if verdicts.is_empty() {
            CompatibilitySmokeVerdict::Passed
        } else {
            verdicts.remove(0)
        };
        FakeSmoke {
            verdict,
            journal: Arc::clone(&self.journal),
        }
    }

    fn activate(
        &self,
        candidate: Self::Candidate,
        action: &ManagedSetupAction,
    ) -> Result<StageDisposal, ActivationFailure> {
        let component = action.artifact.component;
        assert_eq!(candidate.component, component);
        if let Some(failure) = self.activation_failures.get(&key(component)) {
            return Err(*failure);
        }
        self.journal().activated.push(component);
        Ok(StageDisposal::Discarded)
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct RecordedProgress {
    updates: Mutex<Vec<ProgressUpdate>>,
}

impl ProgressSink for RecordedProgress {
    fn report(&self, update: ProgressUpdate) {
        self.updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(update);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn integrity(fill: char) -> Result<ArtifactIntegrity, Box<dyn std::error::Error>> {
    Ok(ArtifactIntegrity::from_sha256_hex(
        10,
        &fill.to_string().repeat(64),
    )?)
}

fn test_catalogue() -> Result<AcceptedManagedCatalogue, Box<dyn std::error::Error>> {
    let mut artifacts = Vec::new();
    for (component, fill, executable) in [
        (MEDIA, 'a', true),
        (WHISPER, 'b', true),
        (MODEL, 'c', false),
    ] {
        artifacts.push(AcceptedManagedArtifact {
            component,
            version: format!("{}-fixture", component.identifier()),
            publisher: String::from("fixture publisher"),
            source_url: format!("https://publisher.invalid/{}", component.identifier()),
            integrity: integrity(fill)?,
            format: ManagedArtifactFormat::RawFile,
            archive_limits: None,
            selected_files: Vec::new(),
            archive_links: Vec::new(),
            runtime_copies: Vec::new(),
            licence: String::from("MIT"),
            notice_url: String::from("https://publisher.invalid/notice"),
            source_code_url: String::from("https://publisher.invalid/source"),
            trust_limit: String::from("fixture evidence only"),
            files: vec![ReviewedManagedFile {
                name: format!("{}-file", component.identifier()),
                integrity: integrity(fill)?,
                executable,
            }],
        });
    }
    Ok(AcceptedManagedCatalogue {
        revision: String::from("p13-install-fixture-r1"),
        target: ManagedTarget::Ubuntu2404X86_64,
        stop_new_plans_at: 2_000,
        stop_new_plans_date: String::from("fixture-date"),
        artifacts,
        compatibility: ReviewedCompatibilityPolicy {
            fixture: integrity('f')?,
            expected_ffmpeg_version: String::from("ffmpeg version fixture"),
            expected_ffprobe_version: String::from("ffprobe version fixture"),
            stream_limit_bytes: 64 * 1024,
            audio_file_limit_bytes: 256 * 1024,
            transcript_file_limit_bytes: 64 * 1024,
            media_deadline_seconds: 60,
            inference_deadline_seconds: 180,
            audio_sample_rate_hz: 16_000,
            audio_channels: 1,
            optional_banners: Vec::new(),
        },
        optional_artifacts: Vec::new(),
        superseded: Vec::new(),
    })
}

/// The accepted plan's actions on a host missing `missing`, with no
/// configured model.
fn accepted_actions(
    missing: &[RuntimeDependency],
) -> Result<Vec<ManagedSetupAction>, Box<dyn std::error::Error>> {
    let plan = plan_managed_setup(
        SetupProfile::Desktop,
        RuntimeDiagnosis {
            readiness: RuntimeReadiness::Blocked,
            dependencies: RuntimeDependency::ALL
                .into_iter()
                .map(|dependency| DependencyStatus {
                    dependency,
                    state: if missing.contains(&dependency) {
                        DependencyState::Missing
                    } else {
                        DependencyState::Available {
                            version: String::from("fixture 1"),
                        }
                    },
                })
                .collect(),
        },
        ManagedTarget::Ubuntu2404X86_64,
        SetupSelectionState::default(),
        1_000,
        Some(test_catalogue()?),
    );
    let digest = plan.digest.clone().ok_or("the test plan has no digest")?;
    plan.validate_acceptance(&digest)
        .map_err(|error| format!("{error:?}"))?;
    Ok(plan.actions)
}

fn outcomes(report: &vsift_application::ManagedInstallReport) -> Vec<ComponentInstallOutcome> {
    report
        .components
        .iter()
        .map(|component| component.outcome)
        .collect()
}

const fn failed(
    step: Option<InstallStep>,
    reason: InstallFailureReason,
) -> ComponentInstallOutcome {
    ComponentInstallOutcome::Failed(ComponentInstallFailure { step, reason })
}

const BLOCKED: ComponentInstallOutcome = failed(None, InstallFailureReason::Blocked);

/// A fresh plan activates each component in dependency order, smoking the
/// media tools alone and the recognizer together with its model, and
/// reports progress as each component finishes.
#[tokio::test]
async fn a_fresh_plan_activates_every_component_in_dependency_order() -> TestResult {
    let actions = accepted_actions(&RuntimeDependency::ALL)?;
    let installer = FakeInstaller::new();
    let progress = RecordedProgress::default();

    let report = install_managed_components(&installer, &actions, &progress).await;

    assert_eq!(
        outcomes(&report),
        [ComponentInstallOutcome::Activated; 3],
        "{report:?}"
    );
    assert_eq!(report.failure_code(), None);
    let journal = installer.journal();
    assert_eq!(journal.staged, [MEDIA, WHISPER, MODEL]);
    assert_eq!(journal.smoked, [vec![MEDIA], vec![WHISPER, MODEL]]);
    assert_eq!(journal.activated, [MEDIA, WHISPER, MODEL]);
    assert_eq!(journal.discarded, [] as [vsift_domain::ManagedComponent; 0]);
    assert!(
        report
            .components
            .iter()
            .all(|component| component.stage == Some(StageDisposal::Discarded))
    );
    let updates = lock(&progress.updates).clone();
    assert!(updates.iter().all(
        |update| update.stage == ProgressStage::InstallingComponents && update.total == Some(3)
    ));
    let completed: Vec<u64> = updates.iter().map(|update| update.completed).collect();
    assert_eq!(completed, [0, 1, 3]);
    Ok(())
}

/// A rerun of the same accepted plan after the media tools activated skips
/// them (`already_current`, never fetched) and continues with the rest.
#[tokio::test]
async fn a_rerun_continues_after_the_components_already_current() -> TestResult {
    let actions = accepted_actions(&RuntimeDependency::ALL)?;
    let mut installer = FakeInstaller::new();
    installer.current = vec![MEDIA];

    let report =
        install_managed_components(&installer, &actions, &RecordedProgress::default()).await;

    assert_eq!(
        outcomes(&report),
        [
            ComponentInstallOutcome::AlreadyCurrent,
            ComponentInstallOutcome::Activated,
            ComponentInstallOutcome::Activated,
        ]
    );
    assert_eq!(report.components[0].stage, None);
    {
        let journal = installer.journal();
        assert_eq!(journal.staged, [WHISPER, MODEL]);
        assert_eq!(journal.activated, [WHISPER, MODEL]);
    }

    // Everything current: nothing is fetched, smoked or activated.
    let mut installer = FakeInstaller::new();
    installer.current = vec![MEDIA, WHISPER, MODEL];
    let report =
        install_managed_components(&installer, &actions, &RecordedProgress::default()).await;
    assert_eq!(
        outcomes(&report),
        [ComponentInstallOutcome::AlreadyCurrent; 3]
    );
    let journal = installer.journal();
    assert!(journal.staged.is_empty() && journal.smoked.is_empty());
    assert_eq!(journal.activated, [] as [vsift_domain::ManagedComponent; 0]);
    Ok(())
}

/// With the recognizer already current, the model is smoked alone, with
/// the selected recognizer as its companion.
#[tokio::test]
async fn a_model_alone_is_smoked_on_its_own() -> TestResult {
    let actions = accepted_actions(&[RuntimeDependency::Whisper])?;
    assert_eq!(actions.len(), 2);
    let mut installer = FakeInstaller::new();
    installer.current = vec![WHISPER];

    let report =
        install_managed_components(&installer, &actions, &RecordedProgress::default()).await;

    assert_eq!(
        outcomes(&report),
        [
            ComponentInstallOutcome::AlreadyCurrent,
            ComponentInstallOutcome::Activated,
        ]
    );
    assert_eq!(installer.journal().smoked, [vec![MODEL]]);
    Ok(())
}

/// D-03: a download that fails leaves the earlier component active, reports
/// its typed reason, and blocks the rest without fetching them.
#[tokio::test]
async fn a_failed_download_stops_the_transaction_and_keeps_earlier_components() -> TestResult {
    let actions = accepted_actions(&RuntimeDependency::ALL)?;
    for reason in DownloadFailureReason::ALL {
        let mut installer = FakeInstaller::new();
        installer.stage_failures.insert(
            key(WHISPER),
            StageFailure {
                failure: ComponentInstallFailure::at(
                    InstallStep::Download,
                    InstallFailureReason::Download(reason),
                ),
                stage: None,
            },
        );

        let report =
            install_managed_components(&installer, &actions, &RecordedProgress::default()).await;

        assert_eq!(
            outcomes(&report),
            [
                ComponentInstallOutcome::Activated,
                failed(
                    Some(InstallStep::Download),
                    InstallFailureReason::Download(reason)
                ),
                BLOCKED,
            ],
            "{reason:?}"
        );
        assert_eq!(report.failure_code(), Some(FailureCode::DownloadFailed));
        let journal = installer.journal();
        assert_eq!(journal.staged, [MEDIA]);
        assert_eq!(journal.activated, [MEDIA]);
    }
    Ok(())
}

/// When the model cannot be fetched, the recognizer staged for the same
/// smoke is discarded and reported blocked with its stage's disposal.
#[tokio::test]
async fn a_companion_that_cannot_be_fetched_discards_the_staged_recognizer() -> TestResult {
    let actions = accepted_actions(&RuntimeDependency::ALL)?;
    let mut installer = FakeInstaller::new();
    installer.stage_failures.insert(
        key(MODEL),
        StageFailure {
            failure: ComponentInstallFailure::at(
                InstallStep::Download,
                InstallFailureReason::DigestMismatch,
            ),
            stage: Some(StageDisposal::Discarded),
        },
    );

    let report =
        install_managed_components(&installer, &actions, &RecordedProgress::default()).await;

    assert_eq!(
        outcomes(&report),
        [
            ComponentInstallOutcome::Activated,
            BLOCKED,
            failed(
                Some(InstallStep::Download),
                InstallFailureReason::DigestMismatch
            ),
        ]
    );
    assert_eq!(report.components[1].stage, Some(StageDisposal::Discarded));
    assert_eq!(report.failure_code(), Some(FailureCode::IntegrityFailure));
    let journal = installer.journal();
    assert_eq!(journal.discarded, [WHISPER]);
    // D-02: bytes that failed their digest never reach activation.
    assert_eq!(journal.activated, [MEDIA]);
    Ok(())
}

/// D-06: a failed smoke reports every candidate of its unit with the
/// check, the reason and the stage's disposal; nothing of it activates.
#[tokio::test]
async fn a_failed_smoke_reports_each_candidate_and_activates_none() -> TestResult {
    let actions = accepted_actions(&RuntimeDependency::ALL)?;
    let libgomp = SharedLibraryName::parse("libgomp.so.1").ok_or("a library name")?;
    for (reason, code) in [
        // #256: a tool the loader could not start is a missing capability like
        // any other provider failure; the library name only travels with it.
        (
            CompatibilitySmokeFailureReason::MissingSharedLibrary(libgomp),
            FailureCode::MissingCapability,
        ),
        (
            CompatibilitySmokeFailureReason::FixtureMismatch,
            FailureCode::MissingCapability,
        ),
        (
            CompatibilitySmokeFailureReason::BannerMismatch,
            FailureCode::MissingCapability,
        ),
        (
            CompatibilitySmokeFailureReason::Cancelled,
            FailureCode::Cancelled,
        ),
        (
            CompatibilitySmokeFailureReason::Preparation,
            FailureCode::StorageIo,
        ),
    ] {
        let failure = CompatibilitySmokeFailure {
            check: CompatibilitySmokeCheck::SpeechFixture,
            reason,
        };
        let installer = FakeInstaller::new();
        *lock(&installer.smoke_verdicts) = vec![
            CompatibilitySmokeVerdict::Passed,
            CompatibilitySmokeVerdict::Failed(failure),
        ];

        let report =
            install_managed_components(&installer, &actions, &RecordedProgress::default()).await;

        let smoked = failed(
            Some(InstallStep::Smoke),
            InstallFailureReason::Smoke(failure),
        );
        assert_eq!(
            outcomes(&report),
            [ComponentInstallOutcome::Activated, smoked, smoked]
        );
        assert_eq!(report.failure_code(), Some(code), "{reason:?}");
        assert_eq!(report.components[1].stage, Some(StageDisposal::Discarded));
        assert_eq!(report.components[2].stage, Some(StageDisposal::Discarded));
        let journal = installer.journal();
        assert_eq!(journal.activated, [MEDIA]);
        assert_eq!(journal.discarded, [WHISPER, MODEL]);
    }
    Ok(())
}

/// A failed activation keeps the components activated before it, reports
/// its reason and disposal, and discards the rest of its unit.
#[tokio::test]
async fn a_failed_activation_discards_the_rest_of_its_unit() -> TestResult {
    let actions = accepted_actions(&RuntimeDependency::ALL)?;
    let mut installer = FakeInstaller::new();
    installer.activation_failures.insert(
        key(WHISPER),
        ActivationFailure {
            reason: InstallFailureReason::Storage,
            stage: StageDisposal::Retained(StageRetentionReason::StorageFailure),
        },
    );

    let report =
        install_managed_components(&installer, &actions, &RecordedProgress::default()).await;

    assert_eq!(
        outcomes(&report),
        [
            ComponentInstallOutcome::Activated,
            failed(Some(InstallStep::Activate), InstallFailureReason::Storage),
            BLOCKED,
        ]
    );
    assert_eq!(
        report.components[1].stage,
        Some(StageDisposal::Retained(
            StageRetentionReason::StorageFailure
        ))
    );
    assert_eq!(report.components[2].stage, Some(StageDisposal::Discarded));
    assert_eq!(report.failure_code(), Some(FailureCode::StorageIo));
    assert_eq!(installer.journal().activated, [MEDIA]);
    Ok(())
}

/// Cancellation between components stops before the next one is fetched:
/// it is reported cancelled and the rest blocked.
#[tokio::test]
async fn cancellation_stops_before_the_next_component() -> TestResult {
    let actions = accepted_actions(&RuntimeDependency::ALL)?;
    let mut installer = FakeInstaller::new();
    installer.cancel_after_staging = Some(MEDIA);

    let report =
        install_managed_components(&installer, &actions, &RecordedProgress::default()).await;

    assert_eq!(
        outcomes(&report),
        [
            ComponentInstallOutcome::Activated,
            failed(None, InstallFailureReason::Cancelled),
            BLOCKED,
        ]
    );
    assert_eq!(report.failure_code(), Some(FailureCode::Cancelled));
    assert_eq!(installer.journal().staged, [MEDIA]);
    Ok(())
}

/// Every reason names the public failure code the command reports, and
/// every identifier is a distinct lower snake-case word.
#[test]
fn every_failure_reason_has_one_code_and_identifier() {
    let smoke = CompatibilitySmokeFailure {
        check: CompatibilitySmokeCheck::Banner,
        reason: CompatibilitySmokeFailureReason::BannerMismatch,
    };
    let mut table = vec![
        (
            InstallFailureReason::ArtifactMissing,
            Some(FailureCode::InvalidArgument),
        ),
        (
            InstallFailureReason::ArtifactNotRegularFile,
            Some(FailureCode::InvalidArgument),
        ),
        (
            InstallFailureReason::DigestMismatch,
            Some(FailureCode::IntegrityFailure),
        ),
        (
            InstallFailureReason::SizeMismatch,
            Some(FailureCode::IntegrityFailure),
        ),
        (
            InstallFailureReason::ReviewMismatch,
            Some(FailureCode::IntegrityFailure),
        ),
        (InstallFailureReason::Storage, Some(FailureCode::StorageIo)),
        (
            InstallFailureReason::Smoke(smoke),
            Some(FailureCode::MissingCapability),
        ),
        (
            InstallFailureReason::Cancelled,
            Some(FailureCode::Cancelled),
        ),
        (InstallFailureReason::Blocked, None),
    ];
    table.extend(DownloadFailureReason::ALL.into_iter().map(|reason| {
        (
            InstallFailureReason::Download(reason),
            Some(FailureCode::DownloadFailed),
        )
    }));
    let mut identifiers = Vec::new();
    for (reason, code) in table {
        assert_eq!(reason.failure_code(), code, "{reason:?}");
        let identifier = reason.identifier();
        assert!(
            identifier
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
            "{identifier}"
        );
        identifiers.push(identifier);
    }
    let count = identifiers.len();
    identifiers.sort_unstable();
    identifiers.dedup();
    assert_eq!(identifiers.len(), count);
}

const DENO: ManagedComponent = ManagedComponent::JsRuntimeDeno;
const YT_DLP: ManagedComponent = ManagedComponent::YtDlp;

/// The test catalogue with the two optional components (#409).
fn optional_catalogue() -> Result<AcceptedManagedCatalogue, Box<dyn std::error::Error>> {
    let mut catalogue = test_catalogue()?;
    for (component, fill) in [(DENO, 'd'), (YT_DLP, 'e')] {
        catalogue.optional_artifacts.push(AcceptedManagedArtifact {
            component,
            version: format!("{}-fixture", component.identifier()),
            publisher: String::from("fixture publisher"),
            source_url: format!("https://publisher.invalid/{}", component.identifier()),
            integrity: integrity(fill)?,
            format: ManagedArtifactFormat::RawFile,
            archive_limits: None,
            selected_files: Vec::new(),
            archive_links: Vec::new(),
            runtime_copies: Vec::new(),
            licence: String::from("GPL-3.0-or-later"),
            notice_url: String::from("https://publisher.invalid/notice"),
            source_code_url: String::from("https://publisher.invalid/source"),
            trust_limit: String::from("fixture evidence only"),
            files: vec![ReviewedManagedFile {
                name: format!("{}-file", component.identifier()),
                integrity: integrity(fill)?,
                executable: true,
            }],
        });
        catalogue
            .compatibility
            .optional_banners
            .push(ReviewedBanner {
                component,
                first_line_prefix: format!("{} fixture", component.identifier()),
            });
    }
    Ok(catalogue)
}

fn diagnosis(missing: &[RuntimeDependency]) -> RuntimeDiagnosis {
    RuntimeDiagnosis {
        readiness: RuntimeReadiness::Blocked,
        dependencies: RuntimeDependency::ALL
            .into_iter()
            .map(|dependency| DependencyStatus {
                dependency,
                state: if missing.contains(&dependency) {
                    DependencyState::Missing
                } else {
                    DependencyState::Available {
                        version: String::from("fixture 1"),
                    }
                },
            })
            .collect(),
    }
}

fn optional_plan(
    profile: SetupProfile,
    missing: &[RuntimeDependency],
    include: OptionalComponentSet,
    target: ManagedTarget,
) -> Result<ManagedSetupPlan, Box<dyn std::error::Error>> {
    Ok(plan_managed_setup_including(
        profile,
        diagnosis(missing),
        target,
        SetupSelectionState::default(),
        include,
        1_000,
        Some(optional_catalogue()?),
    ))
}

const YT_DLP_ONLY: OptionalComponentSet =
    OptionalComponentSet::EMPTY.with(OptionalComponent::OnlineLinks);

/// #409: the default desktop plan offers yt-dlp and installs nothing of it;
/// leaving it out changes nothing the digest binds, and the plan says the
/// operator must decide.
#[test]
fn the_default_desktop_plan_offers_the_optional_component_and_installs_none_of_it() -> TestResult {
    let plan = optional_plan(
        SetupProfile::Desktop,
        &RuntimeDependency::ALL,
        OptionalComponentSet::EMPTY,
        ManagedTarget::Ubuntu2404X86_64,
    )?;
    assert_eq!(plan.optional.len(), 1);
    let offer = &plan.optional[0];
    assert_eq!(offer.component, OptionalComponent::OnlineLinks);
    assert_eq!(offer.disposition, OptionalComponentDisposition::NotIncluded);
    assert_eq!(
        offer
            .artifacts
            .iter()
            .map(|artifact| artifact.component)
            .collect::<Vec<_>>(),
        [DENO, YT_DLP]
    );
    assert_eq!(offer.download_bytes(), 20);
    assert_eq!(offer.actions, [] as [ManagedSetupAction; 0]);
    assert!(plan.operator_decision_required());
    assert_eq!(plan.optional_actions().count(), 0);
    assert!(
        plan.actions
            .iter()
            .all(|action| !action.artifact.component.is_optional())
    );

    let without_offer = plan_managed_setup(
        SetupProfile::Desktop,
        diagnosis(&RuntimeDependency::ALL),
        ManagedTarget::Ubuntu2404X86_64,
        SetupSelectionState::default(),
        1_000,
        Some(test_catalogue()?),
    );
    assert_eq!(plan.digest, without_offer.digest);
    Ok(())
}

/// `--include yt_dlp` adds the runtime and yt-dlp, in that order, after the
/// core; the digest changes, so the plan without them does not accept it.
#[test]
fn including_yt_dlp_adds_the_runtime_and_yt_dlp_and_changes_the_digest() -> TestResult {
    let left_out = optional_plan(
        SetupProfile::Desktop,
        &RuntimeDependency::ALL,
        OptionalComponentSet::EMPTY,
        ManagedTarget::Ubuntu2404X86_64,
    )?;
    let included = optional_plan(
        SetupProfile::Desktop,
        &RuntimeDependency::ALL,
        YT_DLP_ONLY,
        ManagedTarget::Ubuntu2404X86_64,
    )?;
    assert_eq!(
        included.optional[0].disposition,
        OptionalComponentDisposition::Included
    );
    assert_eq!(
        included
            .optional_actions()
            .map(|action| (action.id.as_str(), action.artifact.component))
            .collect::<Vec<_>>(),
        [
            ("install-js_runtime_deno", DENO),
            ("install-yt_dlp", YT_DLP)
        ]
    );
    assert_eq!(included.actions, left_out.actions);
    assert_eq!(included.included(), YT_DLP_ONLY);
    assert!(included.operator_decision_required());
    let (Some(included_digest), Some(left_out_digest)) = (&included.digest, &left_out.digest)
    else {
        return Err("both plans have digests".into());
    };
    assert_ne!(included_digest, left_out_digest);
    assert!(included.validate_acceptance(left_out_digest).is_err());
    assert!(included.validate_acceptance(included_digest).is_ok());

    // A changed banner is a changed plan.
    let mut catalogue = optional_catalogue()?;
    catalogue.compatibility.optional_banners[1].first_line_prefix = String::from("other");
    let changed = plan_managed_setup_including(
        SetupProfile::Desktop,
        diagnosis(&RuntimeDependency::ALL),
        ManagedTarget::Ubuntu2404X86_64,
        SetupSelectionState::default(),
        YT_DLP_ONLY,
        1_000,
        Some(catalogue),
    );
    assert_ne!(changed.digest, included.digest);

    // With every core tool present, the optional part alone is a plan to
    // install, not "nothing required".
    let only_optional = optional_plan(
        SetupProfile::Desktop,
        &[],
        YT_DLP_ONLY,
        ManagedTarget::Ubuntu2404X86_64,
    )?;
    assert_eq!(
        only_optional.availability,
        ManagedPlanAvailability::Qualified
    );
    assert!(only_optional.digest.is_some());
    Ok(())
}

/// The worker plan neither includes nor lists it unless asked; an
/// unavailable host lists it as unavailable and asks no decision.
#[test]
fn the_worker_plan_and_an_unavailable_host_do_not_offer_it() -> TestResult {
    let worker = optional_plan(
        SetupProfile::Worker,
        &RuntimeDependency::ALL,
        OptionalComponentSet::EMPTY,
        ManagedTarget::Ubuntu2404X86_64,
    )?;
    assert_eq!(worker.optional, []);
    assert!(!worker.operator_decision_required());
    let asked = optional_plan(
        SetupProfile::Worker,
        &RuntimeDependency::ALL,
        YT_DLP_ONLY,
        ManagedTarget::Ubuntu2404X86_64,
    )?;
    assert_eq!(
        asked.optional[0].disposition,
        OptionalComponentDisposition::Included
    );

    for include in [OptionalComponentSet::EMPTY, YT_DLP_ONLY] {
        let elsewhere = optional_plan(
            SetupProfile::Desktop,
            &RuntimeDependency::ALL,
            include,
            ManagedTarget::WindowsX86_64,
        )?;
        assert_eq!(
            elsewhere.optional[0].disposition,
            OptionalComponentDisposition::Unavailable
        );
        assert_eq!(elsewhere.optional[0].artifacts, []);
        assert!(!elsewhere.operator_decision_required());
        assert_eq!(elsewhere.optional_actions().count(), 0);
    }
    // An expired catalogue cannot plan it either.
    let expired = plan_managed_setup_including(
        SetupProfile::Desktop,
        diagnosis(&RuntimeDependency::ALL),
        ManagedTarget::Ubuntu2404X86_64,
        SetupSelectionState::default(),
        YT_DLP_ONLY,
        3_000,
        Some(optional_catalogue()?),
    );
    assert_eq!(
        expired.optional[0].disposition,
        OptionalComponentDisposition::Unavailable
    );
    Ok(())
}

/// A catalogue whose optional part names a component twice, lacks a banner
/// or holds a core component is invalid and plans nothing. (The order of the
/// optional artifacts does not matter: a plan installs an optional
/// component's parts in that component's own order.)
#[test]
fn an_inconsistent_optional_catalogue_is_invalid() -> TestResult {
    let mut reversed = optional_catalogue()?;
    let first = reversed.optional_artifacts[0].clone();
    reversed.optional_artifacts.push(first);
    let mut no_banner = optional_catalogue()?;
    no_banner.compatibility.optional_banners.pop();
    let mut core_in_optional = optional_catalogue()?;
    core_in_optional.optional_artifacts[0].component = MEDIA;
    for catalogue in [reversed, no_banner, core_in_optional] {
        let plan = plan_managed_setup_including(
            SetupProfile::Desktop,
            diagnosis(&RuntimeDependency::ALL),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            YT_DLP_ONLY,
            1_000,
            Some(catalogue),
        );
        assert_eq!(plan.availability, ManagedPlanAvailability::CatalogueInvalid);
        assert_eq!(plan.digest, None);
        assert_eq!(plan.optional_actions().count(), 0);
    }
    Ok(())
}

type PlanParts = (Vec<ManagedSetupAction>, Vec<ManagedSetupAction>);

fn included_plan_actions() -> Result<PlanParts, Box<dyn std::error::Error>> {
    let plan = optional_plan(
        SetupProfile::Desktop,
        &RuntimeDependency::ALL,
        YT_DLP_ONLY,
        ManagedTarget::Ubuntu2404X86_64,
    )?;
    let optional = plan.optional_actions().cloned().collect();
    Ok((plan.actions, optional))
}

fn optional_outcomes(
    report: &vsift_application::ManagedInstallReport,
) -> Vec<ComponentInstallOutcome> {
    report
        .optional
        .iter()
        .map(|component| component.outcome)
        .collect()
}

/// The optional part installs after the core, its two components staged
/// and smoked together, and progress counts both parts.
#[tokio::test]
async fn the_optional_part_installs_after_the_core_and_together() -> TestResult {
    let (core, optional) = included_plan_actions()?;
    let installer = FakeInstaller::new();
    let progress = RecordedProgress::default();

    let report = install_managed_plan(&installer, &core, &optional, &progress).await;

    assert_eq!(outcomes(&report), [ComponentInstallOutcome::Activated; 3]);
    assert_eq!(
        optional_outcomes(&report),
        [ComponentInstallOutcome::Activated; 2]
    );
    assert_eq!(report.failure_code(), None);
    assert_eq!(report.optional_failure(), None);
    assert!(report.optional_installed());
    let journal = installer.journal();
    assert_eq!(journal.staged, [MEDIA, WHISPER, MODEL, DENO, YT_DLP]);
    assert_eq!(
        journal.smoked,
        [vec![MEDIA], vec![WHISPER, MODEL], vec![DENO, YT_DLP]]
    );
    assert_eq!(journal.activated, [MEDIA, WHISPER, MODEL, DENO, YT_DLP]);
    let completed: Vec<(u64, Option<u64>)> = lock(&progress.updates)
        .iter()
        .map(|update| (update.completed, update.total))
        .collect();
    assert_eq!(
        completed,
        [(0, Some(5)), (1, Some(5)), (3, Some(5)), (5, Some(5))]
    );
    Ok(())
}

/// #409: a blocked optional download never fails or blocks the core. The
/// core is active, the transaction has no failure code, and the optional
/// part reports the typed reason with the runtime's stage discarded.
#[tokio::test]
async fn an_optional_failure_never_fails_the_core() -> TestResult {
    let (core, optional) = included_plan_actions()?;
    for reason in DownloadFailureReason::ALL {
        let mut installer = FakeInstaller::new();
        installer.stage_failures.insert(
            key(YT_DLP),
            StageFailure {
                failure: ComponentInstallFailure::at(
                    InstallStep::Download,
                    InstallFailureReason::Download(reason),
                ),
                stage: None,
            },
        );

        let report =
            install_managed_plan(&installer, &core, &optional, &RecordedProgress::default()).await;

        assert_eq!(outcomes(&report), [ComponentInstallOutcome::Activated; 3]);
        assert_eq!(report.failure_code(), None, "{reason:?}");
        let yt_dlp_failed = failed(
            Some(InstallStep::Download),
            InstallFailureReason::Download(reason),
        );
        assert_eq!(optional_outcomes(&report), [BLOCKED, yt_dlp_failed]);
        assert_eq!(report.optional[0].stage, Some(StageDisposal::Discarded));
        assert_eq!(
            report.optional_failure().map(|(component, _)| component),
            Some(YT_DLP)
        );
        assert!(!report.optional_installed());
        let journal = installer.journal();
        assert_eq!(journal.activated, [MEDIA, WHISPER, MODEL]);
        assert_eq!(journal.discarded, [DENO]);
    }

    // A failed optional smoke is the same: the core stays installed.
    let installer = FakeInstaller::new();
    let failure = CompatibilitySmokeFailure {
        check: CompatibilitySmokeCheck::Banner,
        reason: CompatibilitySmokeFailureReason::BannerMismatch,
    };
    *lock(&installer.smoke_verdicts) = vec![
        CompatibilitySmokeVerdict::Passed,
        CompatibilitySmokeVerdict::Passed,
        CompatibilitySmokeVerdict::Failed(failure),
    ];
    let report =
        install_managed_plan(&installer, &core, &optional, &RecordedProgress::default()).await;
    let smoked = failed(
        Some(InstallStep::Smoke),
        InstallFailureReason::Smoke(failure),
    );
    assert_eq!(report.failure_code(), None);
    assert_eq!(optional_outcomes(&report), [smoked, smoked]);
    assert_eq!(installer.journal().discarded, [DENO, YT_DLP]);
    Ok(())
}

/// A core failure is a failure exactly as before, and the optional part is
/// blocked: nothing of it is fetched.
#[tokio::test]
async fn a_core_failure_blocks_the_optional_part() -> TestResult {
    let (core, optional) = included_plan_actions()?;
    let mut installer = FakeInstaller::new();
    installer.stage_failures.insert(
        key(MEDIA),
        StageFailure {
            failure: ComponentInstallFailure::at(
                InstallStep::Download,
                InstallFailureReason::Download(DownloadFailureReason::ProxyAuth),
            ),
            stage: None,
        },
    );

    let report =
        install_managed_plan(&installer, &core, &optional, &RecordedProgress::default()).await;

    assert_eq!(report.failure_code(), Some(FailureCode::DownloadFailed));
    assert_eq!(optional_outcomes(&report), [BLOCKED, BLOCKED]);
    assert_eq!(report.optional_failure(), None);
    assert_eq!(installer.journal().staged, [] as [ManagedComponent; 0]);
    Ok(())
}

/// A rerun with the optional components current fetches nothing of them;
/// with only the runtime current, yt-dlp is staged and smoked alone.
#[tokio::test]
async fn a_rerun_skips_optional_components_already_current() -> TestResult {
    let (core, optional) = included_plan_actions()?;
    let mut installer = FakeInstaller::new();
    installer.current = vec![MEDIA, WHISPER, MODEL, DENO];

    let report =
        install_managed_plan(&installer, &core, &optional, &RecordedProgress::default()).await;

    assert_eq!(
        optional_outcomes(&report),
        [
            ComponentInstallOutcome::AlreadyCurrent,
            ComponentInstallOutcome::Activated
        ]
    );
    assert_eq!(installer.journal().smoked, [vec![YT_DLP]]);
    Ok(())
}
