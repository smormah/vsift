//! Opt-in: the production compatibility smoke on the real pinned Ubuntu tools.
//!
//! Only a disposable Ubuntu 24.04 x86-64 host runs this, through the manual
//! `P13 managed smoke` workflow or by hand:
//!
//! ```console
//! VSIFT_P13_REAL_TOOL_SMOKE=1 cargo test --release --locked -p vsift-infrastructure \
//!   --test p13_managed_smoke_real_tools -- --ignored --exact --nocapture
//! ```
//!
//! It downloads the three artifacts of the accepted catalogue (the pinned
//! `FFmpeg` build, whisper.cpp v1.9.2 and the base model) over HTTPS through the
//! production publisher transfer, which verifies each one's size and SHA-256
//! and sends only the neutral `VSift/0.1 managed setup` user agent. Each is
//! staged into a private temporary managed root through the reviewed payload
//! and runtime policy, then smoked together, unactivated, with the reviewed
//! fixture verifiers. A second smoke over the same stages with a changed
//! banner prefix is the negative control: it must fail at the banner and
//! discard every stage. Nothing is ever published or selected.

use std::{
    env,
    error::Error,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use vsift_application::{
    CompatibilitySmokeCheck, CompatibilitySmokeFailureReason, ManagedSetupAction, NoProgress,
    SmokeStageOutcome, StageDisposal, smoke_before_activation,
};
use vsift_domain::ManagedTarget;
use vsift_infrastructure::{
    HostIsolation, ManagedArtifactStore, ProcessCancellation, ReviewedFixtureVerifiers,
    ReviewedUbuntuAction, SmokeCompanions, StagedCompatibilitySmoke, accepted_ubuntu_catalogue,
    detect_managed_target, download_reviewed_publisher_artifact,
};

type TestResult = Result<(), Box<dyn Error>>;

const OPT_IN: &str = "VSIFT_P13_REAL_TOOL_SMOKE";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
#[ignore = "downloads the pinned publisher artifacts; run only on a disposable Ubuntu 24.04 x86-64 host with VSIFT_P13_REAL_TOOL_SMOKE=1"]
async fn pinned_ubuntu_tools_pass_the_smoke_before_activation() -> TestResult {
    if env::var(OPT_IN).as_deref() != Ok("1") {
        println!("skipped: set {OPT_IN}=1 to download and smoke the pinned tools");
        return Ok(());
    }
    if detect_managed_target() != ManagedTarget::Ubuntu2404X86_64 {
        return Err("the real-tool smoke runs only on Ubuntu 24.04 x86-64".into());
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let parent = TestRoot(env::temp_dir().join(format!(
        "vsift-p13-real-smoke-{}-{stamp}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )));
    fs::create_dir(&parent.0)?;
    let root = parent.0.join("managed");
    let store = ManagedArtifactStore::at(root.clone())?;
    let catalogue = accepted_ubuntu_catalogue()?;
    println!("catalogue revision {}", catalogue.revision);

    let mut candidates = Vec::new();
    for artifact in catalogue.artifacts.clone() {
        let action = ManagedSetupAction {
            id: format!("install-{}", artifact.component.identifier()),
            artifact,
        };
        let reviewed = ReviewedUbuntuAction::from_accepted_action(&action)?;
        let started = Instant::now();
        let staged = download_reviewed_publisher_artifact(
            &store,
            reviewed.publisher_source(),
            &ProcessCancellation::new(),
            &NoProgress,
        )
        .await?;
        println!(
            "{}: {} bytes downloaded and verified in {:.1} s",
            action.artifact.component.identifier(),
            action.artifact.integrity.bytes(),
            started.elapsed().as_secs_f64()
        );
        candidates.push(reviewed.stage_candidate(staged)?);
    }

    let smoke = StagedCompatibilitySmoke::new(
        catalogue.compatibility.clone(),
        HostIsolation::ProcessOnly,
        SmokeCompanions::default(),
        ReviewedFixtureVerifiers,
        ProcessCancellation::new(),
    );
    let started = Instant::now();
    let candidates = match smoke_before_activation(&smoke, candidates).await {
        SmokeStageOutcome::Passed(candidates) => candidates,
        SmokeStageOutcome::Failed { failure, stages } => {
            return Err(
                format!("the pinned tools failed the smoke: {failure:?}, {stages:?}").into(),
            );
        }
    };
    println!(
        "layout, banners, media fixture, speech fixture and recheck passed in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    assert!(!root.join("versions-v1").exists());
    assert!(!root.join("current-v1").exists());

    let mut wrong_banner = catalogue.compatibility;
    wrong_banner
        .expected_ffmpeg_version
        .push_str("-not-this-build");
    let control = StagedCompatibilitySmoke::new(
        wrong_banner,
        HostIsolation::ProcessOnly,
        SmokeCompanions::default(),
        ReviewedFixtureVerifiers,
        ProcessCancellation::new(),
    );
    let SmokeStageOutcome::Failed { failure, stages } =
        smoke_before_activation(&control, candidates).await
    else {
        return Err("a changed banner prefix did not fail the smoke".into());
    };
    assert_eq!(failure.check, CompatibilitySmokeCheck::Banner);
    assert_eq!(
        failure.reason,
        CompatibilitySmokeFailureReason::BannerMismatch
    );
    assert_eq!(stages.len(), 3);
    assert!(
        stages
            .iter()
            .all(|(_, disposal)| *disposal == StageDisposal::Discarded),
        "{stages:?}"
    );
    let remaining: Vec<String> = fs::read_dir(&root)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    assert_eq!(remaining, ["owner-v1"]);
    println!("negative control: banner mismatch, all three stages discarded, nothing activated");
    Ok(())
}
