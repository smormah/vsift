//! Identifying a yt-dlp or JavaScript runtime the user registered (ADR 0028
//! section 4.6), with the smoke's stand-in program staged under the names it
//! takes its behaviour from (`<tool>-<behaviour>`, see the stand-in's
//! documentation). Nothing here reaches a network.

use std::{
    env,
    error::Error,
    fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use vsift_infrastructure::{
    HostIsolation, ProcessCancellation, TrustedExecutable, UserOnlineTool, UserOnlineToolFault,
    identify_user_online_tool,
};

type TestResult = Result<(), Box<dyn Error>>;

const FIXTURE_TOOL: &str = env!("CARGO_BIN_EXE_vsift-smoke-fixture");

/// A temporary folder holding staged copies of the stand-in, removed on drop.
struct Folder(PathBuf);

impl Folder {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!(
            "vsift-user-online-tool-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(path.join("workspaces"))?;
        Ok(Self(path))
    }

    fn workspaces(&self) -> PathBuf {
        self.0.join("workspaces")
    }

    /// The stand-in staged as `<name>`, which selects its behaviour.
    fn stage(&self, name: &str) -> Result<TrustedExecutable, Box<dyn Error>> {
        let path = self
            .0
            .join(format!("{name}{}", env::consts::EXE_SUFFIX));
        fs::copy(FIXTURE_TOOL, &path)?;
        Ok(TrustedExecutable::explicit(path)?)
    }

    /// Whether the private folders the identification made are gone.
    fn workspaces_are_empty(&self) -> Result<bool, Box<dyn Error>> {
        Ok(fs::read_dir(self.workspaces())?.next().is_none())
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

async fn identify(
    folder: &Folder,
    tool: UserOnlineTool,
    executable: &TrustedExecutable,
    time_left: Duration,
) -> Result<vsift_infrastructure::UserOnlineToolFacts, UserOnlineToolFault> {
    identify_user_online_tool(
        tool,
        executable,
        &folder.workspaces(),
        HostIsolation::ProcessOnly,
        time_left,
        ProcessCancellation::new(),
    )
    .await
}

fn is_hex_digest(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// A runtime that answers with a line beginning `deno ` is identified, with
/// the digest of the file, and its private folder is removed afterwards.
#[tokio::test]
async fn a_runtime_that_prints_deno_is_identified_with_its_digest() -> TestResult {
    let folder = Folder::new()?;
    let executable = folder.stage("deno-good")?;
    let facts = identify(
        &folder,
        UserOnlineTool::JsRuntime,
        &executable,
        Duration::from_secs(20),
    )
    .await?;
    assert!(facts.version_line.starts_with("deno "), "{facts:?}");
    assert!(is_hex_digest(facts.sha256.as_str()));
    assert!(folder.workspaces_are_empty()?);
    Ok(())
}

/// A yt-dlp whose line is not a date version, and a program that prints no
/// version line at all, are not accepted: the line is checked untrusted text.
/// (A runtime that prints another name is refused by the CLI contract test,
/// whose stand-in is the build's own binary.)
#[tokio::test]
async fn a_line_of_the_wrong_shape_is_not_a_version() -> TestResult {
    let folder = Folder::new()?;
    // The stand-in named `yt-dlp-good` prints `yt-dlp version <build> ...`.
    let not_a_date = folder.stage("yt-dlp-good")?;
    assert!(matches!(
        identify(
            &folder,
            UserOnlineTool::YtDlp,
            &not_a_date,
            Duration::from_secs(20)
        )
        .await,
        Err(UserOnlineToolFault::UnexpectedOutput)
    ));
    // `node-good` answers `--version` with nothing: no version line.
    let not_deno = folder.stage("node-good")?;
    assert!(matches!(
        identify(
            &folder,
            UserOnlineTool::JsRuntime,
            &not_deno,
            Duration::from_secs(20)
        )
        .await,
        Err(UserOnlineToolFault::UnexpectedOutput)
    ));
    assert!(folder.workspaces_are_empty()?);
    Ok(())
}

/// A program that fails, or that prints without end, is a typed fault.
#[tokio::test]
async fn a_failing_or_flooding_program_is_a_typed_fault() -> TestResult {
    let folder = Folder::new()?;
    let failing = folder.stage("deno-fail")?;
    assert!(matches!(
        identify(
            &folder,
            UserOnlineTool::JsRuntime,
            &failing,
            Duration::from_secs(20)
        )
        .await,
        Err(UserOnlineToolFault::Failed)
    ));
    let flooding = folder.stage("deno-flood")?;
    assert!(matches!(
        identify(
            &folder,
            UserOnlineTool::JsRuntime,
            &flooding,
            Duration::from_secs(20)
        )
        .await,
        Err(UserOnlineToolFault::OutputOverBound)
    ));
    Ok(())
}

/// The run stays inside the time it is given: a program that hangs ends at
/// the deadline of what is left of `setup check`'s shared one, not at the
/// 30 seconds of its own limit, and with nothing left it is not started.
#[tokio::test]
async fn a_hanging_program_stays_inside_the_time_it_is_given() -> TestResult {
    let folder = Folder::new()?;
    let hanging = folder.stage("deno-hang")?;
    let started = Instant::now();
    let result = identify(
        &folder,
        UserOnlineTool::JsRuntime,
        &hanging,
        Duration::from_secs(2),
    )
    .await;
    assert!(
        matches!(result, Err(UserOnlineToolFault::Deadline)),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "took {:?}",
        started.elapsed()
    );
    assert!(folder.workspaces_are_empty()?);

    let started = Instant::now();
    let none_left = identify(
        &folder,
        UserOnlineTool::JsRuntime,
        &hanging,
        Duration::ZERO,
    )
    .await;
    assert!(matches!(none_left, Err(UserOnlineToolFault::Deadline)));
    assert!(started.elapsed() < Duration::from_secs(2));
    // Nothing was started, so no private folder was made.
    assert!(folder.workspaces_are_empty()?);
    Ok(())
}
