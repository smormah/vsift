//! What the dependency probe reports when its shared deadline passes
//! (issue #391).
//!
//! `setup check` reported a healthy `FFmpeg` and `FFprobe` as timed out on the
//! first run after a restart and advised installing them. Two facts were lost
//! in one `timed_out`: a tool that was started and gave no answer, and a tool
//! that was never started because an earlier probe had used up the one
//! deadline all the probes share. These tests pin both, with stand-in tools
//! whose behaviour does not depend on how fast the machine is: the stand-in
//! for a slow tool never answers at all, and whether a tool was started is
//! read from a file it writes when it runs, not from a clock.
//!
//! The stand-in is `testbin/smoke_fixture_tool.rs`, which takes its behaviour
//! from the name it is copied under (see that file): `hang` sleeps for five
//! minutes, `scribble` writes a file beside itself and answers like a healthy
//! tool.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use vsift_application::DependencyProbe;
use vsift_domain::{DependencyState, ProbeTimeout, ProbeTimeoutKind, RuntimeDependency};
use vsift_infrastructure::{
    DEFAULT_PROBE_TIMEOUT, ExecutableResolver, ExplicitProbePaths, ProcessDependencyProbe,
    ProcessSupervisor,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const FIXTURE_TOOL: &str = env!("CARGO_BIN_EXE_vsift-smoke-fixture");
/// The file a `scribble` stand-in writes beside itself when it is run.
const SCRIBBLE_FILE: &str = "vsift-smoke-scribble";
const OWNED_PREFIX: &str = "vsift-probe-deadline-";
/// Far more than a process that prints one line needs. It bounds the one
/// probe here that is expected to answer; no assertion depends on it being
/// short.
const GENEROUS: Duration = Duration::from_secs(60);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A folder of stand-in tools this test created and alone may remove, and an
/// empty folder beside it for a `PATH` search that finds nothing.
struct Staged {
    root: PathBuf,
}

impl Staged {
    fn new() -> TestResult<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root)?;
        fs::create_dir(root.join("tools"))?;
        fs::create_dir(root.join("empty"))?;
        Ok(Self { root })
    }

    fn tools(&self) -> PathBuf {
        self.root.join("tools")
    }

    /// A copy of the stand-in under `name`, which decides how it behaves.
    fn tool(&self, name: &str) -> TestResult<PathBuf> {
        let mut file_name = std::ffi::OsString::from(name);
        if let Some(extension) = Path::new(FIXTURE_TOOL).extension() {
            file_name.push(".");
            file_name.push(extension);
        }
        let path = self.tools().join(file_name);
        fs::copy(FIXTURE_TOOL, &path)?;
        Ok(path)
    }

    /// Whether a `scribble` stand-in in this folder has been run.
    fn a_tool_ran(&self) -> bool {
        self.tools().join(SCRIBBLE_FILE).exists()
    }

    /// A probe with these selections and one total deadline. Anything not
    /// selected is searched for in an empty folder, so it is missing whatever
    /// this machine's `PATH` holds.
    fn probe(
        &self,
        deadline: Duration,
        paths: ExplicitProbePaths,
    ) -> TestResult<ProcessDependencyProbe> {
        Ok(ProcessDependencyProbe::configured_with_explicit_paths(
            deadline,
            ExecutableResolver::from_directories([self.root.join("empty")])?,
            ProcessSupervisor::default(),
            paths,
        ))
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if self
            .root
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn timed_out(kind: ProbeTimeoutKind, shared_deadline: Duration) -> DependencyState {
    DependencyState::TimedOut(ProbeTimeout {
        kind,
        shared_deadline,
    })
}

/// The reported shape. The first tool is started and never answers; the
/// second, a healthy tool, is never started because the first used up the
/// deadline; a third that is not installed is still reported missing, so
/// "timed out" always means "found". Run again with time to spare, the second
/// tool answers: nothing was wrong with it.
#[tokio::test]
async fn a_tool_that_does_not_answer_leaves_the_next_one_unstarted_and_neither_is_missing()
-> TestResult {
    let staged = Staged::new()?;
    // Long enough that reaching the first probe cannot use it up on a loaded
    // machine; the stand-in would not answer in five minutes either.
    let deadline = Duration::from_secs(2);
    let paths = ExplicitProbePaths {
        ffmpeg: Some(staged.tool("ffmpeg-hang")?),
        ffprobe: Some(staged.tool("ffprobe-scribble")?),
        whisper: None,
    };

    let probe = staged.probe(deadline, paths.clone())?;
    assert_eq!(
        probe.probe(RuntimeDependency::Ffmpeg).await.state,
        timed_out(ProbeTimeoutKind::NoAnswer, deadline),
        "a tool that was started and gave no answer"
    );
    assert_eq!(
        probe.probe(RuntimeDependency::Ffprobe).await.state,
        timed_out(ProbeTimeoutKind::NotStarted, deadline),
        "a healthy tool whose turn came after the deadline"
    );
    assert!(
        !staged.a_tool_ran(),
        "a tool reported as not started was started"
    );
    assert_eq!(
        probe.probe(RuntimeDependency::Whisper).await.state,
        DependencyState::Missing,
        "a tool that is not there is missing, deadline or not"
    );

    let unhurried = staged.probe(
        GENEROUS,
        ExplicitProbePaths {
            ffmpeg: None,
            ..paths
        },
    )?;
    assert_eq!(
        unhurried.probe(RuntimeDependency::Ffprobe).await.state,
        DependencyState::Available {
            version: String::from("ffprobe version vsift-smoke-fixture Copyright (c) fixture"),
        }
    );
    assert!(staged.a_tool_ran());
    Ok(())
}

/// A deadline that has passed before the first probe starts nothing at all,
/// and says so; a timeout too large to be an instant on the clock was always
/// applied as none, and is reported as the zero it was applied as.
#[tokio::test]
async fn a_deadline_that_has_already_passed_starts_no_tool() -> TestResult {
    for applied in [Duration::ZERO, Duration::MAX] {
        let staged = Staged::new()?;
        let probe = staged.probe(
            applied,
            ExplicitProbePaths {
                ffmpeg: Some(staged.tool("ffmpeg-scribble")?),
                ffprobe: None,
                whisper: None,
            },
        )?;
        assert_eq!(
            probe.probe(RuntimeDependency::Ffmpeg).await.state,
            timed_out(ProbeTimeoutKind::NotStarted, Duration::ZERO),
            "{applied:?}"
        );
        assert!(!staged.a_tool_ran(), "{applied:?}");
    }
    Ok(())
}

/// Thirty seconds, raised from five in `0.2.1`: the figure the command line
/// and every host that passes the default apply to all the probes together.
#[test]
fn the_default_deadline_for_all_the_probes_is_thirty_seconds() {
    assert_eq!(DEFAULT_PROBE_TIMEOUT, Duration::from_secs(30));
}
