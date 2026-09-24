//! Public CLI contract for the automatic media-tool preflight.
//!
//! `ingest --transcript` probes the video with `FFprobe`, so it first verifies
//! the selected `FFmpeg`/`FFprobe` pair against the reviewed built-in fixture. A
//! pair that fails is reported with a typed code and a fixed-prose remediation
//! naming the failed check, and nothing is written. Plain files registered as
//! tools fail everywhere without real tools; `FFmpeg` registered as `FFprobe`
//! needs a real `ffmpeg` on `PATH` and is opt-in:
//!
//! `cargo test -p vsift-cli --locked --test media_tool_preflight_cli_contract -- --ignored`

use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-preflight-cli-";
const PROCESS_FAILURE_MARKER: &str = "at the probe step (process_failure).";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }

    fn sessions(&self) -> PathBuf {
        self.path("private sessions")
    }

    fn user_base(&self) -> PathBuf {
        self.path("user")
    }

    fn verification_state(&self) -> PathBuf {
        self.user_base()
            .join("vsift")
            .join("media-tool-verification")
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.path(name);
        fs::write(&path, bytes)?;
        Ok(path)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema: Value =
        serde_json::from_str(&fs::read_to_string(schema_root().join(schema_path))?)?;
    jsonschema::validator_for(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// Runs `vsift` with an isolated per-user base and the root's session store.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    let base = root.user_base();
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .output()?)
}

fn path_text(path: &Path) -> Result<&str, Box<dyn Error>> {
    Ok(path.to_str().ok_or("non-UTF-8 test path")?)
}

fn configure(root: &OwnedRoot, dependency: &str, executable: &Path) -> TestResult {
    let output = vsift(
        root,
        &[
            "setup",
            "configure",
            dependency,
            "--executable",
            path_text(executable)?,
            "--json",
        ],
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("setup configure {dependency} failed: {output:?}").into())
    }
}

fn ingest_arguments<'a>(source: &'a str, sidecar: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut arguments = vec!["ingest", source, "--transcript", sidecar];
    arguments.extend_from_slice(extra);
    arguments
}

/// Checks the failed-preflight result shape shared by every scenario.
fn assert_preflight_failure(value: &Value, marker: &str) -> Result<(), Box<dyn Error>> {
    validate("operation-response.schema.json", value)?;
    assert_eq!(value["command"], "ingest");
    assert_eq!(value["status"], "failed");
    assert_eq!(value["data"], Value::Null);
    assert_eq!(value["error"]["code"], "MISSING_CAPABILITY");
    assert_eq!(value["error"]["retryable"], false);
    let remediation = value["error"]["remediation"]
        .as_array()
        .ok_or("remediation missing")?;
    assert_eq!(remediation.len(), 1);
    assert_eq!(remediation[0]["required_authority"], "none");
    assert_eq!(remediation[0]["command"], Value::Null);
    let summary = remediation[0]["summary"]
        .as_str()
        .ok_or("summary missing")?;
    assert!(summary.contains(marker), "{summary}");
    assert!(summary.contains("Nothing was changed."), "{summary}");
    assert!(summary.contains("setup configure"), "{summary}");
    Ok(())
}

#[test]
fn an_unusable_tool_fails_the_preflight_before_mutation_with_typed_remediation() -> TestResult {
    let root = OwnedRoot::new()?;
    let stand_in = root.write("not-a-program.exe", b"plain text, not a program")?;
    configure(&root, "ffmpeg", &stand_in)?;
    configure(&root, "ffprobe", &stand_in)?;
    let source = root.write("original.mp4", b"\0\0\0\x18ftypisomsource-content")?;
    let sidecar = root.write("captions.srt", b"1\n00:00:00,500 --> 00:00:01,000\nx\n")?;
    let source = path_text(&source)?;
    let sidecar = path_text(&sidecar)?;

    let output = vsift(&root, &ingest_arguments(source, sidecar, &["--json"]))?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_preflight_failure(&value, PROCESS_FAILURE_MARKER)?;
    assert!(!root.sessions().exists(), "a session root was created");
    assert!(
        !root.verification_state().join("verified-v1.json").exists(),
        "a failed verification was recorded"
    );

    // The same failure as a JSON Lines terminal event and as human text.
    let events = vsift(
        &root,
        &ingest_arguments(source, sidecar, &["--events", "jsonl"]),
    )?;
    assert_eq!(events.status.code(), Some(2));
    let event: Value = serde_json::from_slice(&events.stdout)?;
    validate("terminal-event.schema.json", &event)?;
    assert_preflight_failure(&event["result"], PROCESS_FAILURE_MARKER)?;

    let human = vsift(&root, &ingest_arguments(source, sidecar, &[]))?;
    assert_eq!(human.status.code(), Some(2));
    let diagnostic = String::from_utf8(human.stderr)?;
    assert!(diagnostic.contains(PROCESS_FAILURE_MARKER), "{diagnostic}");
    assert!(
        !diagnostic.contains(path_text(&stand_in)?),
        "a path was echoed"
    );
    assert!(!root.sessions().exists());
    Ok(())
}

#[test]
fn plain_ingest_does_not_run_the_preflight() -> TestResult {
    let root = OwnedRoot::new()?;
    let stand_in = root.write("not-a-program.exe", b"plain text, not a program")?;
    configure(&root, "ffmpeg", &stand_in)?;
    configure(&root, "ffprobe", &stand_in)?;
    let source = root.write("original.mp4", b"\0\0\0\x18ftypisomsource-content")?;

    let output = vsift(&root, &["ingest", path_text(&source)?, "--json"])?;

    assert_eq!(output.status.code(), Some(0));
    assert!(!root.verification_state().exists());
    Ok(())
}

fn find_on_path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    let file_name = format!("{name}{}", env::consts::EXE_SUFFIX);
    env::split_paths(&env::var_os("PATH").ok_or("PATH is not set")?)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(&file_name))
        .find(|candidate| Path::is_file(candidate))
        .ok_or_else(|| format!("{name} is not on PATH").into())
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Opt-in: `FFmpeg` registered as `FFprobe` answers a version probe, which is
/// all `setup check` sees, but fails the preflight at the probe check.
#[test]
#[ignore = "requires ffmpeg on PATH"]
fn ffmpeg_registered_as_ffprobe_fails_at_the_probe_check() -> TestResult {
    let root = OwnedRoot::new()?;
    let ffmpeg = find_on_path("ffmpeg")?;
    configure(&root, "ffmpeg", &ffmpeg)?;
    configure(&root, "ffprobe", &ffmpeg)?;
    let video = repository("fixtures/corpus/generated/F10.mp4");
    let sidecar = repository("fixtures/corpus/transcripts/F10.srt");

    let output = vsift(
        &root,
        &ingest_arguments(
            path_text(&video)?,
            path_text(&sidecar)?,
            &["--transcript-offset", "500000", "--json"],
        ),
    )?;

    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    assert_preflight_failure(&value, "at the probe step (")?;
    assert!(!root.sessions().exists());
    assert!(!root.verification_state().join("verified-v1.json").exists());
    Ok(())
}
