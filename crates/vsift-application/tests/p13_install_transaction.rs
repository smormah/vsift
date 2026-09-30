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
    ManagedSetupAction, ProgressSink, ReviewedCompatibilityPolicy, ReviewedManagedFile,
    RuntimeDiagnosis, SetupProfile, SetupSelectionState, StageDisposal, StageFailure,
    StageRetentionReason, StagedManagedComponent, install_managed_components, plan_managed_setup,
};
use vsift_domain::{
    ArtifactIntegrity, DependencyState, DependencyStatus, FailureCode, ManagedArtifactFormat,
    ManagedComponent, ManagedTarget, ProgressStage, ProgressUpdate, RuntimeDependency,
    RuntimeReadiness,
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
        },
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
    assert!(journal.discarded.is_empty());
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
    assert!(journal.activated.is_empty());
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
    for (reason, code) in [
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
