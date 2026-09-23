//! Opt-in check of bring-your-own tool verification against real `FFmpeg`/`FFprobe`.
//!
//! Needs `ffmpeg` and `ffprobe` on `PATH`. Run with:
//! `cargo test -p vsift-infrastructure --locked --test p06_tool_verification -- --ignored`

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use vsift_application::{
    MediaToolCheck, MediaToolFailure, MediaToolVerification, MediaToolVerifier,
};
use vsift_infrastructure::{
    ExecutableResolver, FixtureMediaToolVerifier, HostIsolation, MediaProviderConformance,
    ProcessCancellation, TrustedExecutable, reviewed_compatibility_policy,
};

type TestResult = Result<(), Box<dyn Error>>;

/// Removes the test's own parent directory; the verifier must already have emptied it.
struct Parent(PathBuf);

impl Parent {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!(
            "vsift-p06-verify-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn is_empty(&self) -> Result<bool, Box<dyn Error>> {
        Ok(fs::read_dir(&self.0)?.next().is_none())
    }
}

impl Drop for Parent {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tools() -> Result<(TrustedExecutable, TrustedExecutable), Box<dyn Error>> {
    let resolver = ExecutableResolver::from_current_path();
    Ok((
        resolver.resolve(OsStr::new("ffmpeg"))?,
        resolver.resolve(OsStr::new("ffprobe"))?,
    ))
}

fn verifier(
    ffmpeg: TrustedExecutable,
    ffprobe: TrustedExecutable,
    parent: &Parent,
) -> Result<FixtureMediaToolVerifier, Box<dyn Error>> {
    Ok(FixtureMediaToolVerifier::new(
        MediaProviderConformance::r0(ffmpeg, ffprobe),
        HostIsolation::ProcessOnly,
        parent.0.clone(),
        reviewed_compatibility_policy()?,
        ProcessCancellation::new(),
    ))
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn working_tools_are_verified_and_leave_no_workspace() -> TestResult {
    let (ffmpeg, ffprobe) = tools()?;
    let parent = Parent::new("pass")?;

    let outcome = verifier(ffmpeg, ffprobe, &parent)?.verify().await;

    assert_eq!(outcome, MediaToolVerification::Verified);
    assert!(parent.is_empty()?, "verification workspace was not removed");
    Ok(())
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn a_tool_that_cannot_probe_fails_at_the_probe_check() -> TestResult {
    let (ffmpeg, _) = tools()?;
    let parent = Parent::new("fail")?;

    // FFmpeg in FFprobe's place responds to a version probe, which is all
    // `setup check` can see, but it cannot produce FFprobe's metadata.
    let outcome = verifier(ffmpeg.clone(), ffmpeg, &parent)?.verify().await;

    assert!(
        matches!(
            outcome,
            MediaToolVerification::Failed {
                check: MediaToolCheck::Probe,
                failure: MediaToolFailure::ProviderRejected
                    | MediaToolFailure::ProcessFailure
                    | MediaToolFailure::UnexpectedResult,
            }
        ),
        "unexpected outcome: {outcome:?}"
    );
    assert!(parent.is_empty()?, "verification workspace was not removed");
    Ok(())
}
