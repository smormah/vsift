//! D-06 through the application port: a failed compatibility smoke never
//! activates anything, and every staged component is either removed or
//! retained and reported.
//!
//! A test catalogue produces the accepted plan's actions; each action becomes
//! a fake staged component in a fake managed store. The fake smoke returns
//! every failure step and reason in turn, and the fake store records what was
//! staged, removed, retained and activated.

use std::{
    future::Future,
    sync::{Arc, Mutex, PoisonError},
};

use vsift_application::{
    AcceptedManagedArtifact, AcceptedManagedCatalogue, CompatibilitySmoke, CompatibilitySmokeCheck,
    CompatibilitySmokeFailure, CompatibilitySmokeFailureReason, CompatibilitySmokeVerdict,
    ManagedSetupAction, ReviewedCompatibilityPolicy, ReviewedManagedFile, RuntimeDiagnosis,
    SetupProfile, SetupSelectionState, SmokeStageOutcome, StageDisposal, StageRetentionReason,
    StagedManagedComponent, plan_managed_setup, smoke_before_activation,
};
use vsift_domain::{
    ArtifactIntegrity, DependencyState, DependencyStatus, ManagedArtifactFormat, ManagedComponent,
    ManagedTarget, RuntimeDependency, RuntimeReadiness,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const CHECKS: [CompatibilitySmokeCheck; 5] = [
    CompatibilitySmokeCheck::Layout,
    CompatibilitySmokeCheck::Banner,
    CompatibilitySmokeCheck::MediaFixture,
    CompatibilitySmokeCheck::SpeechFixture,
    CompatibilitySmokeCheck::Recheck,
];

const REASONS: [CompatibilitySmokeFailureReason; 13] = [
    CompatibilitySmokeFailureReason::MissingExecutable,
    CompatibilitySmokeFailureReason::WrongArchitecture,
    CompatibilitySmokeFailureReason::NotExecutable,
    CompatibilitySmokeFailureReason::BannerMismatch,
    CompatibilitySmokeFailureReason::OutputOverBound,
    CompatibilitySmokeFailureReason::DeadlineExceeded,
    CompatibilitySmokeFailureReason::UnexpectedExtraFile,
    CompatibilitySmokeFailureReason::ChangedContent,
    CompatibilitySmokeFailureReason::ProviderFailed,
    CompatibilitySmokeFailureReason::FixtureMismatch,
    CompatibilitySmokeFailureReason::Preparation,
    CompatibilitySmokeFailureReason::InvalidRequest,
    CompatibilitySmokeFailureReason::Cancelled,
];

/// What the fake managed store holds.
#[derive(Debug, Default)]
struct FakeStore {
    staged: Vec<ManagedComponent>,
    removed: Vec<ManagedComponent>,
    retained: Vec<ManagedComponent>,
    activated: Vec<ManagedComponent>,
}

/// How the fake stage behaves when cleanup reaches it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FakeOwnership {
    Proved,
    Unproved,
    UnexpectedEntry,
}

#[derive(Debug)]
struct FakeStage {
    component: ManagedComponent,
    ownership: FakeOwnership,
    store: Arc<Mutex<FakeStore>>,
}

impl StagedManagedComponent for FakeStage {
    fn component(&self) -> ManagedComponent {
        self.component
    }

    fn discard(self) -> StageDisposal {
        let mut store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
        match self.ownership {
            FakeOwnership::Proved => {
                store
                    .staged
                    .retain(|component| *component != self.component);
                store.removed.push(self.component);
                StageDisposal::Discarded
            }
            FakeOwnership::Unproved => {
                store.retained.push(self.component);
                StageDisposal::Retained(StageRetentionReason::OwnershipUnproved)
            }
            FakeOwnership::UnexpectedEntry => {
                store.retained.push(self.component);
                StageDisposal::Retained(StageRetentionReason::UnexpectedContent)
            }
        }
    }
}

struct FakeSmoke {
    verdict: CompatibilitySmokeVerdict,
    seen: Mutex<Vec<Vec<ManagedComponent>>>,
}

impl FakeSmoke {
    fn new(verdict: CompatibilitySmokeVerdict) -> Self {
        Self {
            verdict,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Vec<ManagedComponent>> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CompatibilitySmoke<FakeStage> for FakeSmoke {
    fn smoke(
        &self,
        candidates: &[FakeStage],
    ) -> impl Future<Output = CompatibilitySmokeVerdict> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(candidates.iter().map(|stage| stage.component).collect());
        std::future::ready(self.verdict)
    }
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
        (ManagedComponent::MediaTools, 'a', true),
        (ManagedComponent::WhisperCli, 'b', true),
        (ManagedComponent::WhisperModel, 'c', false),
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
        revision: String::from("d06-fixture-r1"),
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

/// The accepted plan's actions for a host that has none of the tools.
fn accepted_actions() -> Result<Vec<ManagedSetupAction>, Box<dyn std::error::Error>> {
    let plan = plan_managed_setup(
        SetupProfile::Desktop,
        RuntimeDiagnosis {
            readiness: RuntimeReadiness::Blocked,
            dependencies: RuntimeDependency::ALL
                .into_iter()
                .map(|dependency| DependencyStatus {
                    dependency,
                    state: DependencyState::Missing,
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
    assert_eq!(plan.actions.len(), 3);
    Ok(plan.actions)
}

fn stage_all(
    actions: &[ManagedSetupAction],
    ownership: FakeOwnership,
    store: &Arc<Mutex<FakeStore>>,
) -> Vec<FakeStage> {
    actions
        .iter()
        .map(|action| {
            store
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .staged
                .push(action.artifact.component);
            FakeStage {
                component: action.artifact.component,
                ownership,
                store: Arc::clone(store),
            }
        })
        .collect()
}

/// Every failure step and reason, with provable and unprovable stages: no
/// component is ever activated, provable stages are removed, and the others
/// are retained and reported with their reason.
#[tokio::test]
async fn every_failed_smoke_discards_or_reports_and_never_activates() -> TestResult {
    let actions = accepted_actions()?;
    let components: Vec<ManagedComponent> = actions
        .iter()
        .map(|action| action.artifact.component)
        .collect();
    for ownership in [
        FakeOwnership::Proved,
        FakeOwnership::Unproved,
        FakeOwnership::UnexpectedEntry,
    ] {
        for check in CHECKS {
            for reason in REASONS {
                let store = Arc::new(Mutex::new(FakeStore::default()));
                let failure = CompatibilitySmokeFailure { check, reason };
                let smoke = FakeSmoke::new(CompatibilitySmokeVerdict::Failed(failure));
                let outcome =
                    smoke_before_activation(&smoke, stage_all(&actions, ownership, &store)).await;

                let SmokeStageOutcome::Failed {
                    failure: reported,
                    stages,
                } = outcome
                else {
                    return Err(format!("{check:?}/{reason:?} did not fail").into());
                };
                assert_eq!(reported, failure);
                assert_eq!(smoke.calls(), vec![components.clone()]);
                let expected = match ownership {
                    FakeOwnership::Proved => StageDisposal::Discarded,
                    FakeOwnership::Unproved => {
                        StageDisposal::Retained(StageRetentionReason::OwnershipUnproved)
                    }
                    FakeOwnership::UnexpectedEntry => {
                        StageDisposal::Retained(StageRetentionReason::UnexpectedContent)
                    }
                };
                assert_eq!(
                    stages,
                    components
                        .iter()
                        .map(|component| (*component, expected))
                        .collect::<Vec<_>>()
                );
                let store = store.lock().unwrap_or_else(PoisonError::into_inner);
                assert!(store.activated.is_empty(), "{check:?}/{reason:?}");
                match ownership {
                    FakeOwnership::Proved => {
                        assert!(store.staged.is_empty());
                        assert_eq!(store.removed, components);
                        assert!(store.retained.is_empty());
                    }
                    FakeOwnership::Unproved | FakeOwnership::UnexpectedEntry => {
                        assert_eq!(store.staged, components);
                        assert!(store.removed.is_empty());
                        assert_eq!(store.retained, components);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Each accepted action smoked on its own fails and is cleaned up alone.
#[tokio::test]
async fn a_single_component_failure_cleans_only_that_stage() -> TestResult {
    for action in accepted_actions()? {
        let store = Arc::new(Mutex::new(FakeStore::default()));
        let smoke = FakeSmoke::new(CompatibilitySmokeVerdict::Failed(
            CompatibilitySmokeFailure {
                check: CompatibilitySmokeCheck::Banner,
                reason: CompatibilitySmokeFailureReason::BannerMismatch,
            },
        ));
        let outcome = smoke_before_activation(
            &smoke,
            stage_all(std::slice::from_ref(&action), FakeOwnership::Proved, &store),
        )
        .await;
        let component = action.artifact.component;
        assert!(matches!(
            outcome,
            SmokeStageOutcome::Failed { ref stages, .. }
                if *stages == [(component, StageDisposal::Discarded)]
        ));
        let store = store.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(store.removed, [component]);
        assert!(store.activated.is_empty());
    }
    Ok(())
}

/// A passing smoke returns the candidates untouched: still staged, not
/// removed, not activated. Publication is a separate step.
#[tokio::test]
async fn a_passing_smoke_returns_unactivated_candidates() -> TestResult {
    let actions = accepted_actions()?;
    let store = Arc::new(Mutex::new(FakeStore::default()));
    let smoke = FakeSmoke::new(CompatibilitySmokeVerdict::Passed);
    let outcome =
        smoke_before_activation(&smoke, stage_all(&actions, FakeOwnership::Proved, &store)).await;
    let SmokeStageOutcome::Passed(candidates) = outcome else {
        return Err("the smoke did not pass".into());
    };
    let components: Vec<ManagedComponent> = candidates
        .iter()
        .map(StagedManagedComponent::component)
        .collect();
    let store_view = store.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(store_view.staged, components);
    assert!(store_view.removed.is_empty());
    assert!(store_view.retained.is_empty());
    assert!(store_view.activated.is_empty());
    Ok(())
}

/// An empty request or a component staged twice is refused before any
/// smoke runs, and whatever was staged is still cleaned up.
#[tokio::test]
async fn invalid_requests_run_no_smoke_and_clean_up() -> TestResult {
    let actions = accepted_actions()?;
    let smoke = FakeSmoke::new(CompatibilitySmokeVerdict::Passed);
    let empty: Vec<FakeStage> = Vec::new();
    assert!(matches!(
        smoke_before_activation(&smoke, empty).await,
        SmokeStageOutcome::Failed {
            failure: CompatibilitySmokeFailure {
                check: CompatibilitySmokeCheck::Layout,
                reason: CompatibilitySmokeFailureReason::InvalidRequest,
            },
            ref stages,
        } if stages.is_empty()
    ));
    let store = Arc::new(Mutex::new(FakeStore::default()));
    let first = actions.first().ok_or("no action")?;
    let twice = [first.clone(), first.clone()];
    let outcome =
        smoke_before_activation(&smoke, stage_all(&twice, FakeOwnership::Proved, &store)).await;
    assert!(matches!(
        outcome,
        SmokeStageOutcome::Failed {
            failure: CompatibilitySmokeFailure {
                reason: CompatibilitySmokeFailureReason::InvalidRequest,
                ..
            },
            ref stages,
        } if stages.len() == 2
            && stages.iter().all(|(_, disposal)| *disposal == StageDisposal::Discarded)
    ));
    assert!(smoke.calls().is_empty());
    assert!(
        store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .activated
            .is_empty()
    );
    Ok(())
}

#[test]
fn identifiers_are_stable_and_distinct() {
    let reasons: Vec<&str> = REASONS.iter().map(|reason| reason.identifier()).collect();
    let checks: Vec<&str> = CHECKS.iter().map(|check| check.identifier()).collect();
    for list in [&reasons, &checks] {
        for (index, identifier) in list.iter().enumerate() {
            assert!(!list[index + 1..].contains(identifier), "{identifier}");
        }
    }
    assert_eq!(reasons[0], "missing_executable");
    assert_eq!(checks[4], "recheck");
    assert_eq!(
        StageRetentionReason::OwnershipUnproved.identifier(),
        "ownership_unproved"
    );
}
