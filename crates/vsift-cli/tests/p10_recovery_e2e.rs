//! Opt-in P10 checkpoint: the recoverable mechanical run (X-01, X-02, X-03,
//! X-06, X-09 through the public CLI).
//!
//! Every stage drives the compiled `vsift` binary as a headless agent would,
//! with isolated per-user bases and an empty `PATH`: `FFmpeg`, `FFprobe`,
//! whisper.cpp and its model are registered with `setup configure`.
//! Expectations come from the frozen corpus truth (the F03 manifest entry and
//! its speech provenance) and from an uninterrupted control run, never from
//! the results being scored. The speech clip is F03's speech variant looped
//! nine times at run time without re-encoding (81 s: four 30 s chunks); the
//! long visual clip is F02 looped forty times (492 s: nine 60 s windows).
//! The stages:
//!
//! - `p10_local_asr_journey`: the P09 local-ASR journey on the looped clip:
//!   `transcript retranscribe` (the control run), `search` for the critical
//!   term, `candidates` within 10 s of the hit, `frame get --candidate`
//!   inside the critical event, `crop` of the changed cell, `audio` of the
//!   cited segment, then `session retain` and `bundle validate` with every
//!   citation checked against the frozen truth (spec item 7);
//! - `p10_kill_and_resume`: a retranscription killed once its first chunk
//!   checkpoint exists leaves its job `interrupted` (`job status`), and the
//!   same command resumes it to the control run's segments; afterwards a
//!   committed transcript artifact altered on disk is `INTEGRITY_FAILURE`;
//! - `p10_interrupt_and_job_resume`: Ctrl-C (`SIGINT` on Unix, the console
//!   helper's Ctrl-Break on Windows) during a retranscription after its
//!   first checkpoint: `CANCELLED` (exit 6) naming the session and job and
//!   suggesting `job resume <job>`, nothing committed, no provider left
//!   running; one checkpoint is then altered, and `job resume` discards it
//!   (`checkpoint_discarded`) and commits the control run's segments;
//! - `p10_job_cancel_twice`: `job cancel` twice while a retranscription runs
//!   in another process: the owner stops within the poll interval and its
//!   provider is stopped, the job ends `cancelled` with no checkpoint, the
//!   run answers `CANCELLED` and nothing is committed;
//! - `p10_operation_replay`: a retranscription with `--operation-id` killed
//!   as soon as its commit's pointer moved, before it could acknowledge
//!   (X-02 through the binary), is replayed by the same request and id
//!   without a new generation, and the same id with another range is
//!   `IDEMPOTENCY_CONFLICT` (X-03). The fault-injection kill at the pointer
//!   rename itself is covered by the storage kill test
//!   (`a_job_killed_at_every_job_point_resumes_or_replays_exactly_once`):
//!   the release CLI cannot carry the test-only `fault-injection` feature;
//! - `p10_interrupt_candidates`: an interruption halfway through a
//!   `candidates` call commits the windows it analysed and answers `partial`
//!   (or, when it lands before the first window ends, `CANCELLED`); a rerun
//!   completes and pages the control call's candidates.
//!
//! Clips are built without re-encoding (`-c copy`), so no encoder is needed.
//!
//! ```console
//! VSIFT_TEST_WHISPER_CLI=<abs> VSIFT_TEST_WHISPER_MODEL=<abs> \
//!   cargo test -p vsift-cli --locked --test p10_recovery_e2e -- --ignored --nocapture
//! ```
//!
//! On Windows the interruption stages use `tools/send-console-ctrl.ps1`
//! (PowerShell and .NET platform invoke). The run writes
//! `.vsift/e2e-runs/p10-<run-id>/report.json` and prints
//! `p10_recovery: passed` when every stage passed. A stage that cannot run
//! here is `blocked`, never `passed`, and the test fails unless every stage
//! passed.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command as Process, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::{Value, json};
use vsift_infrastructure::{ExecutableResolver, TrustedExecutable};

type TestResult = Result<(), Box<dyn Error>>;

/// Upper bound for one CLI invocation.
const CLI_DEADLINE: Duration = Duration::from_mins(10);
const OWNED_PREFIX: &str = "vsift-p10-recovery-e2e-";
const MAX_DIAGNOSTIC_CHARS: usize = 400;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SECOND: u64 = 1_000_000;
const MISSING_TOOLS: &str = "FFmpeg or FFprobe is not on PATH, or VSIFT_TEST_WHISPER_CLI and VSIFT_TEST_WHISPER_MODEL do not name an absolute whisper-cli and ggml model";
/// The journey's fixture, as in the P09 checkpoint: F03's speech variant,
/// the term the script says when the cell changes, the critical event that
/// shows it, and the cell (drawn at (850, 420), 280x70, by the frozen
/// generator recipe).
const JOURNEY_FIXTURE: &str = "F03";
const JOURNEY_FILE: &str = "F03-speech.mp4";
const JOURNEY_TERM: &str = "127.50";
const JOURNEY_EVENT: &str = "F03-E02";
const JOURNEY_CELL: [u64; 4] = [850, 420, 280, 70];
/// Loops of F03-speech (9 s) in the speech clip: 81 s, four R0 chunks.
const SPEECH_LOOPS: u32 = 8;
/// Loops of F02 (12 s) in the visual clip: 492 s, nine 60 s windows.
const VISUAL_LOOPS: u32 = 40;
const LEAD_LAG_US: u64 = 10 * SECOND;
/// Local speech recognition places segments within this much of the
/// generator's speech span (the P07 local-ASR checkpoint's tolerance).
const ASR_SPAN_TOLERANCE_US: u64 = SECOND;
/// An interrupted command must end within the providers' 5 s graceful and
/// 5 s forced budgets (SEC-04).
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(10);
/// After the command ended, nothing it started may still run after this.
const DESCENDANT_BUDGET: Duration = Duration::from_secs(10);

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

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

enum StageStop {
    Failed(String),
    Blocked(String),
}

impl<E: Error> From<E> for StageStop {
    fn from(error: E) -> Self {
        Self::Failed(bounded(&error.to_string()))
    }
}

type StageResult = Result<Value, StageStop>;

fn bounded(text: &str) -> String {
    text.chars().take(MAX_DIAGNOSTIC_CHARS).collect()
}

fn ensure(condition: bool, expectation: &str) -> Result<(), StageStop> {
    if condition {
        Ok(())
    } else {
        Err(StageStop::Failed(bounded(expectation)))
    }
}

fn failed(expectation: &str) -> StageStop {
    StageStop::Failed(bounded(expectation))
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> PathBuf {
    repository().join("fixtures/corpus/generated").join(name)
}

fn u64_of(value: &Value) -> Result<u64, StageStop> {
    value
        .as_u64()
        .ok_or_else(|| failed("an expected integer is missing"))
}

fn str_of(value: &Value) -> Result<String, StageStop> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| failed("an expected string is missing"))
}

/// The real tools this checkpoint needs.
struct Tools {
    ffmpeg: TrustedExecutable,
    ffprobe: TrustedExecutable,
    whisper: PathBuf,
    model: PathBuf,
}

impl Tools {
    fn discover() -> Option<Self> {
        let resolver = ExecutableResolver::from_current_path();
        let absolute = |name: &str| {
            env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute() && path.is_file())
        };
        Some(Self {
            ffmpeg: resolver.resolve(OsStr::new("ffmpeg")).ok()?,
            ffprobe: resolver.resolve(OsStr::new("ffprobe")).ok()?,
            whisper: absolute("VSIFT_TEST_WHISPER_CLI")?,
            model: absolute("VSIFT_TEST_WHISPER_MODEL")?,
        })
    }

    /// Loops `fixture` `loops` more times into `clip` without re-encoding.
    fn loop_clip(&self, name: &str, loops: u32, clip: &Path) -> Result<(), StageStop> {
        let output = Process::new(self.ffmpeg.path())
            .args(["-v", "error", "-nostdin", "-y", "-stream_loop"])
            .arg(loops.to_string())
            .arg("-i")
            .arg(fixture(name))
            .args(["-c", "copy"])
            .arg(clip)
            .output()?;
        ensure(
            output.status.success(),
            &format!(
                "ffmpeg could not loop {name}: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    }

    /// Decodes an image file to packed 8-bit RGB with `FFmpeg`.
    fn decode_image(&self, image: &Path) -> Result<Vec<u8>, StageStop> {
        let output = Process::new(self.ffmpeg.path())
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(image)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .output()?;
        ensure(output.status.success(), "FFmpeg could not decode an image")?;
        Ok(output.stdout)
    }
}

/// A bounded `vsift` invocation with isolated per-user state and no
/// ambient `PATH`.
fn vsift(base: &Path) -> Result<Command, StageStop> {
    let mut command = Command::cargo_bin("vsift")?;
    command
        .env("LOCALAPPDATA", base)
        .env("XDG_CONFIG_HOME", base)
        .env("HOME", base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(base.join("sessions"))
        .timeout(CLI_DEADLINE);
    Ok(command)
}

fn run_json(command: &mut Command) -> Result<(Option<i32>, Value), StageStop> {
    let output = command.output()?;
    ensure(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error",
    )?;
    let value: Value = serde_json::from_slice(&output.stdout)?;
    conforms("operation-response.schema.json", &value)?;
    Ok((output.status.code(), value))
}

/// A command that must succeed; returns its result and wall time.
fn ok_json(base: &Path, arguments: &[&str]) -> Result<(Value, Duration), StageStop> {
    let started = Instant::now();
    let (code, value) = run_json(vsift(base)?.args(arguments).arg("--json"))?;
    ensure(
        code == Some(0),
        &format!(
            "{arguments:?} failed: {} {}",
            value["error"]["code"], value["error"]["remediation"][0]["summary"]
        ),
    )?;
    Ok((value, started.elapsed()))
}

struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_slice(&fs::read(
            repository().join("schemas/v1").join(name),
        )?)?)
    }
}

fn conforms(schema: &str, instance: &Value) -> Result<(), StageStop> {
    let definition: Value =
        serde_json::from_slice(&fs::read(repository().join("schemas/v1").join(schema))?)?;
    let validator = jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&definition)
        .map_err(|error| failed(&error.to_string()))?;
    ensure(
        validator.is_valid(instance),
        &format!("an emitted document does not conform to {schema}"),
    )
}

/// Registers every tool in `base`.
fn prepare(base: &Path, tools: &Tools) -> Result<(), StageStop> {
    fs::create_dir_all(base)?;
    for (dependency, path) in [
        ("ffmpeg", tools.ffmpeg.path()),
        ("ffprobe", tools.ffprobe.path()),
        ("whisper", tools.whisper.as_path()),
    ] {
        let (code, _) = run_json(
            vsift(base)?
                .args(["setup", "configure", dependency, "--executable"])
                .arg(path)
                .arg("--json"),
        )?;
        ensure(code == Some(0), "setup configure did not succeed")?;
    }
    let (code, _) = run_json(
        vsift(base)?
            .args(["setup", "configure-model", "--file"])
            .arg(&tools.model)
            .arg("--json"),
    )?;
    ensure(code == Some(0), "setup configure-model did not succeed")
}

fn ingest(base: &Path, video: &Path) -> Result<String, StageStop> {
    let (code, opened) = run_json(vsift(base)?.arg("ingest").arg(video).arg("--json"))?;
    ensure(
        code == Some(0),
        &format!("ingest failed: {}", opened["error"]["code"]),
    )?;
    str_of(&opened["data"]["session_id"])
}

/// Every segment of a revision (or the newest) as (start, end, text),
/// read page by page with `transcript get`.
fn spoken(
    base: &Path,
    session: &str,
    revision: Option<&str>,
) -> Result<Vec<(u64, u64, String)>, StageStop> {
    let mut segments = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut arguments = vec![
            "transcript",
            "get",
            session,
            "--from",
            "0",
            "--to",
            "100000000",
            "--limit",
            "100",
        ];
        if let Some(revision) = revision {
            arguments.extend(["--revision", revision]);
        }
        let cursor_text;
        if let Some(next) = &cursor {
            cursor_text = next.clone();
            arguments.extend(["--cursor", cursor_text.as_str()]);
        }
        let (page, _) = ok_json(base, &arguments)?;
        for item in page["data"]["items"]
            .as_array()
            .ok_or_else(|| failed("no items"))?
        {
            segments.push((
                u64_of(&item["start_us"])?,
                u64_of(&item["end_us"])?,
                str_of(&item["text"])?,
            ));
        }
        match page["data"]["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => return Ok(segments),
        }
    }
}

fn session_generation(base: &Path, session: &str) -> Result<u64, StageStop> {
    let (status, _) = ok_json(base, &["session", "status", session])?;
    u64_of(&status["data"]["generation"])
}

/// The session's only job, from `session status`.
fn only_job(base: &Path, session: &str) -> Result<String, StageStop> {
    let (status, _) = ok_json(base, &["session", "status", session])?;
    let jobs = status["data"]["jobs"]
        .as_array()
        .ok_or_else(|| failed("session status lists no jobs"))?;
    ensure(jobs.len() == 1, "the session does not hold exactly one job")?;
    str_of(&jobs[0]["job_id"])
}

fn job_status(base: &Path, job: &str) -> Result<Value, StageStop> {
    let (status, _) = ok_json(base, &["job", "status", job])?;
    conforms("job-data.schema.json", &status["data"])?;
    Ok(status["data"].clone())
}

/// The session's job directories under the root (`jobs/<job>`).
fn job_directory(base: &Path, session: &str, job: &str) -> PathBuf {
    base.join("sessions")
        .join("sessions")
        .join(session)
        .join("jobs")
        .join(job)
}

/// Every chunk checkpoint file of any job of the session.
fn checkpoints(base: &Path, session: &str) -> Vec<PathBuf> {
    let jobs = base
        .join("sessions")
        .join("sessions")
        .join(session)
        .join("jobs");
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(jobs) else {
        return found;
    };
    for entry in entries.flatten() {
        let Ok(chunks) = fs::read_dir(entry.path().join("chunks")) else {
            continue;
        };
        for chunk in chunks.flatten() {
            let path = chunk.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// A `vsift` run in the background, in a console of its own on Windows so
/// a console event reaches only it and its providers.
fn spawn(base: &Path, arguments: &[&str]) -> Result<Child, StageStop> {
    let mut command = Process::new(assert_cmd::cargo::cargo_bin("vsift"));
    command
        .env("LOCALAPPDATA", base)
        .env("XDG_CONFIG_HOME", base)
        .env("HOME", base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(base.join("sessions"))
        .args(arguments)
        .arg("--json")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    Ok(command.spawn()?)
}

/// The processes `parent` started that are still running.
fn children_of(parent: u32) -> Result<BTreeSet<u32>, StageStop> {
    #[cfg(windows)]
    let output = Process::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "Get-CimInstance Win32_Process -Filter 'ParentProcessId={parent}' | ForEach-Object {{ $_.ProcessId }}"
            ),
        ])
        .stdin(Stdio::null())
        .output()?;
    #[cfg(not(windows))]
    let output = Process::new("pgrep")
        .args(["-P", &parent.to_string()])
        .stdin(Stdio::null())
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|pid| pid.parse().ok())
        .collect())
}

/// Whether process `pid` still runs.
fn alive(pid: u32) -> Result<bool, StageStop> {
    #[cfg(windows)]
    let output = Process::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ 'alive' }}"),
        ])
        .stdin(Stdio::null())
        .output()?;
    #[cfg(not(windows))]
    let output = Process::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    #[cfg(windows)]
    return Ok(String::from_utf8_lossy(&output.stdout).contains("alive"));
    #[cfg(not(windows))]
    return Ok(output.status.success());
}

/// Sends the platform's interruption to `child`: `SIGINT` on Unix, a
/// console Ctrl-Break through the helper on Windows (Ctrl-Break, because a
/// process that inherited "ignore Ctrl-C" is never told about a Ctrl-C).
fn interrupt(child: &Child) -> Result<(), StageStop> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let status = Process::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(repository().join("tools/send-console-ctrl.ps1"))
            .args(["-ProcessId", &child.id().to_string(), "-Event", "CtrlBreak"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        ensure(status.code() == Some(0), "the console helper failed")
    }
    #[cfg(not(windows))]
    {
        let status = Process::new("kill")
            .args(["-s", "INT", &child.id().to_string()])
            .status()?;
        ensure(status.success(), "kill -s INT failed")
    }
}

/// What a background run ended with.
struct Ended {
    code: Option<i32>,
    result: Value,
    /// From the interruption (or the moment waiting began) to the exit.
    stopped_after: Duration,
    /// Every provider process seen running before it was stopped, or left
    /// behind with it as their parent.
    providers: BTreeSet<u32>,
}

/// The providers `child` is running now, waiting up to 10 s for one to
/// appear, so the check after the interruption is never vacuous.
fn running_providers(child: &mut Child) -> Result<BTreeSet<u32>, StageStop> {
    let started = Instant::now();
    loop {
        let providers = children_of(child.id())?;
        if !providers.is_empty() {
            return Ok(providers);
        }
        ensure(
            child.try_wait()?.is_none(),
            "the run ended before a provider was seen",
        )?;
        ensure(
            started.elapsed() < Duration::from_secs(10),
            "no provider process was seen running",
        )?;
        thread::sleep(Duration::from_millis(20));
    }
}

/// Waits for a background run, then checks that none of the `providers`
/// seen running before it was stopped (nor any process still naming it as
/// parent) outlives it by more than [`DESCENDANT_BUDGET`].
fn finish(
    mut child: Child,
    since: Instant,
    budget: Duration,
    mut providers: BTreeSet<u32>,
) -> Result<Ended, StageStop> {
    let pid = child.id();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if since.elapsed() > budget {
            let _ = child.kill();
            return Err(failed("the run outlived its budget"));
        }
        thread::sleep(Duration::from_millis(100));
    };
    let stopped_after = since.elapsed();
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .ok_or_else(|| failed("no stdout"))?
        .read_to_string(&mut stdout)?;
    ensure(
        stdout.lines().count() == 1,
        &format!("not exactly one terminal result: {stdout}"),
    )?;
    let result: Value = serde_json::from_str(stdout.trim_end())?;
    conforms("operation-response.schema.json", &result)?;
    // On Windows an orphan keeps its parent's id; on Unix it is re-parented.
    providers.extend(children_of(pid)?);
    let deadline = Instant::now() + DESCENDANT_BUDGET;
    loop {
        let mut running = Vec::new();
        for provider in &providers {
            if alive(*provider)? {
                running.push(*provider);
            }
        }
        if running.is_empty() {
            break;
        }
        ensure(
            Instant::now() < deadline,
            &format!("providers still running after the command ended: {running:?}"),
        )?;
        thread::sleep(Duration::from_millis(200));
    }
    Ok(Ended {
        code: status.code(),
        result,
        stopped_after,
        providers,
    })
}

/// Waits until the session holds a chunk checkpoint, or the run ended.
fn wait_for_checkpoint(base: &Path, session: &str, child: &mut Child) -> Result<(), StageStop> {
    let started = Instant::now();
    loop {
        if !checkpoints(base, session).is_empty() {
            return Ok(());
        }
        ensure(
            child.try_wait()?.is_none(),
            "the run ended before its first checkpoint",
        )?;
        ensure(started.elapsed() < CLI_DEADLINE, "no checkpoint appeared")?;
        thread::sleep(Duration::from_millis(20));
    }
}

/// The frozen truth of the journey's fixture.
struct JourneyTruth {
    duration: u64,
    speech: (u64, u64),
    term_words: (u64, u64),
    event: (u64, u64),
}

fn journey_truth() -> Result<JourneyTruth, StageStop> {
    let manifest: Value = serde_json::from_slice(&fs::read(
        repository().join("fixtures/corpus/manifest.json"),
    )?)?;
    let entry = manifest["fixtures"]
        .as_array()
        .and_then(|fixtures| fixtures.iter().find(|entry| entry["id"] == JOURNEY_FIXTURE))
        .ok_or_else(|| failed("fixture missing from the manifest"))?;
    let event = entry["events"]
        .as_array()
        .and_then(|events| events.iter().find(|event| event["id"] == JOURNEY_EVENT))
        .ok_or_else(|| failed("the journey's event is missing from the manifest"))?;
    ensure(
        event["critical"] == true,
        "the journey's event is not critical",
    )?;
    let provenance: Value = serde_json::from_slice(&fs::read(fixture("speech-provenance.json"))?)?;
    let variant = provenance["assembly"]["variants"]
        .as_array()
        .and_then(|variants| {
            variants
                .iter()
                .find(|variant| variant["fixture"] == JOURNEY_FIXTURE)
        })
        .ok_or_else(|| failed("the journey's fixture is missing from speech provenance"))?;
    let word = variant["tts_word_timings"]
        .as_array()
        .and_then(|words| words.iter().find(|word| word["text"] == JOURNEY_TERM))
        .ok_or_else(|| failed("the term has no word timing"))?;
    Ok(JourneyTruth {
        duration: u64_of(&entry["duration_us"])?,
        speech: (
            u64_of(&variant["speech_start_us"])?,
            u64_of(&variant["speech_end_us"])?,
        ),
        term_words: (u64_of(&word["start_us"])?, u64_of(&word["end_us"])?),
        event: (u64_of(&event["start_us"])?, u64_of(&event["end_us"])?),
    })
}

fn mean_colour(rgb: &[u8]) -> Result<(u64, u64, u64), StageStop> {
    let pixels = u64::try_from(rgb.len() / 3)?;
    ensure(pixels > 0, "an image is empty")?;
    let mut sums = (0_u64, 0_u64, 0_u64);
    for pixel in rgb.as_chunks::<3>().0 {
        sums.0 += u64::from(pixel[0]);
        sums.1 += u64::from(pixel[1]);
        sums.2 += u64::from(pixel[2]);
    }
    Ok((sums.0 / pixels, sums.1 / pixels, sums.2 / pixels))
}

fn delivered(result: &Value) -> Result<PathBuf, StageStop> {
    let path = PathBuf::from(str_of(&result["data"]["files"][0]["path"])?);
    ensure(path.is_file(), "a delivered file does not exist")?;
    Ok(path)
}

/// Every artifact of a retained bundle of one kind.
fn bundle_records(bundle: &Path, kind: &str) -> Result<Vec<Value>, StageStop> {
    let manifest: Value = serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    let mut records = Vec::new();
    for artifact in manifest["artifacts"]
        .as_array()
        .ok_or_else(|| failed("artifacts missing"))?
    {
        if artifact["kind"] != kind {
            continue;
        }
        let name = str_of(&artifact["name"])?;
        ensure(
            name.starts_with("artifact-") && !name.contains(['/', '\\']),
            "a bundle artifact has an unexpected name",
        )?;
        records.push(serde_json::from_slice(&fs::read(bundle.join(name))?)?);
    }
    Ok(records)
}

/// The shared clips and the control run every later stage compares with.
struct Control {
    speech_clip: PathBuf,
    visual_clip: PathBuf,
    segments: Vec<(u64, u64, String)>,
}

/// Stage 1: the P09 local-ASR journey on the looped speech clip; its
/// retranscription is the control run.
#[allow(
    clippy::too_many_lines,
    reason = "The journey's steps and their checks read best in one place"
)]
fn journey_stage(base: &Path, tools: &Tools, control: &mut Option<Control>) -> StageResult {
    let truth = journey_truth()?;
    let speech_clip = base.join("F03-speech-looped.mp4");
    tools.loop_clip(JOURNEY_FILE, SPEECH_LOOPS, &speech_clip)?;
    let visual_clip = base.join("F02-looped.mp4");
    tools.loop_clip("F02.mp4", VISUAL_LOOPS, &visual_clip)?;
    let session = ingest(base, &speech_clip)?;
    let (retranscribed, retranscribe_time) =
        ok_json(base, &["transcript", "retranscribe", &session])?;
    conforms(
        "transcript-retranscribe-data.schema.json",
        &retranscribed["data"],
    )?;
    let chunks = u64_of(&retranscribed["data"]["revision"]["local_asr"]["chunk_count"])?;
    ensure(
        chunks >= 3,
        &format!("the clip has {chunks} chunks, fewer than three"),
    )?;
    let revision = str_of(&retranscribed["data"]["revision"]["revision_id"])?;
    let job = str_of(&retranscribed["data"]["job"]["job_id"])?;
    let status = job_status(base, &job)?;
    ensure(
        status["state"] == "succeeded"
            && status["result"]["revision_id"] == revision.as_str()
            && status["progress"]["chunks_total"] == chunks,
        "job status does not report the committed run",
    )?;

    // Search: the term's segment in the first loop lies in the speech span.
    let (search, _) = ok_json(base, &["search", &session, "--query", JOURNEY_TERM])?;
    conforms("search-data.schema.json", &search["data"])?;
    let mut hits = Vec::new();
    for item in search["data"]["items"]
        .as_array()
        .ok_or_else(|| failed("no search items"))?
    {
        hits.push((u64_of(&item["start_us"])?, u64_of(&item["end_us"])?));
    }
    hits.sort_unstable();
    let (segment_start, segment_end) = *hits
        .first()
        .ok_or_else(|| failed("the term was not found"))?;
    ensure(
        hits.len() >= 3,
        &format!("the term was heard in only {} loops", hits.len()),
    )?;
    let tolerance = ASR_SPAN_TOLERANCE_US;
    ensure(
        segment_start + tolerance >= truth.speech.0
            && segment_end <= truth.speech.1 + tolerance
            && segment_start < truth.term_words.1 + tolerance
            && segment_end + tolerance > truth.term_words.0,
        &format!(
            "the first cited segment [{segment_start}, {segment_end}) is not inside the speech span {:?} over the term {:?}",
            truth.speech, truth.term_words
        ),
    )?;

    // Candidates near the hit; one inside the critical event of the first loop.
    let window = (
        segment_start.saturating_sub(LEAD_LAG_US),
        (segment_start + LEAD_LAG_US).min(truth.duration),
    );
    let (page, _) = ok_json(
        base,
        &[
            "candidates",
            &session,
            "--from",
            &window.0.to_string(),
            "--to",
            &window.1.to_string(),
            "--limit",
            "100",
        ],
    )?;
    conforms("candidates-data.schema.json", &page["data"])?;
    let candidate = page["data"]["items"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|item| {
                item["representative_us"]
                    .as_u64()
                    .is_some_and(|time| (truth.event.0..truth.event.1).contains(&time))
            })
        })
        .ok_or_else(|| failed("no candidate near the hit lies inside the event"))?
        .clone();
    let candidate_id = str_of(&candidate["candidate_id"])?;
    let representative = u64_of(&candidate["representative_us"])?;

    // The candidate's own frame, the cell cropped from it, the cited audio.
    let (frame, _) = ok_json(
        base,
        &["frame", "get", &session, "--candidate", &candidate_id],
    )?;
    conforms("frame-data.schema.json", &frame["data"])?;
    let frame_id = str_of(&frame["data"]["items"][0]["evidence_id"])?;
    ensure(
        frame["data"]["selections"][0]["delta_us"] == 0
            && u64_of(&frame["data"]["selections"][0]["actual_us"])? == representative,
        "the candidate's frame is not its own",
    )?;
    let [x, y, width, height] = JOURNEY_CELL;
    let rect = format!("{x},{y},{width},{height}");
    let (crop, _) = ok_json(base, &["crop", &session, &frame_id, "--rect", &rect])?;
    conforms("frame-data.schema.json", &crop["data"])?;
    let (red, green, blue) = mean_colour(&tools.decode_image(&delivered(&crop)?)?)?;
    ensure(
        red > green + 40,
        &format!("the cell is not red after the change: ({red}, {green}, {blue})"),
    )?;
    let audio_to = segment_end.min(segment_start + 30 * SECOND);
    let (audio, _) = ok_json(
        base,
        &[
            "audio",
            &session,
            "--from",
            &segment_start.to_string(),
            "--to",
            &audio_to.to_string(),
        ],
    )?;
    conforms("audio-data.schema.json", &audio["data"])?;

    // Retain and validate, lineage included (spec item 7).
    let bundle = base.join("bundle");
    let (_, retain_time) = ok_json(
        base,
        &[
            "session",
            "retain",
            &session,
            "--output",
            &bundle.to_string_lossy(),
        ],
    )?;
    let (validated, _) = ok_json(base, &["bundle", "validate", &bundle.to_string_lossy()])?;
    let transcripts = bundle_records(&bundle, "transcript_record")?;
    let evidence_records = bundle_records(&bundle, "evidence_record")?;
    for record in &transcripts {
        conforms("bundle-transcript-record.schema.json", record)?;
    }
    for record in &evidence_records {
        conforms("bundle-evidence-record.schema.json", record)?;
    }
    let cited = transcripts.iter().any(|record| {
        record["segments"].as_array().is_some_and(|segments| {
            segments.iter().any(|segment| {
                segment["start_us"] == segment_start && segment["end_us"] == segment_end
            })
        })
    });
    let lineage = evidence_records
        .iter()
        .any(|record| record["request"]["frame_get"]["candidate_id"] == candidate_id.as_str())
        && evidence_records
            .iter()
            .any(|record| record["request"]["crop"]["parent_evidence_id"] == frame_id.as_str());
    ensure(
        transcripts.len() == 1 && evidence_records.len() == 3 && cited && lineage,
        &format!(
            "the bundle does not carry the cited segment and the frame, crop and clip lineage: {} transcripts, {} evidence records, cited {cited}, lineage {lineage}",
            transcripts.len(),
            evidence_records.len()
        ),
    )?;

    let segments = spoken(base, &session, Some(&revision))?;
    *control = Some(Control {
        speech_clip,
        visual_clip,
        segments,
    });
    Ok(json!({
        "clip": format!("{JOURNEY_FILE} looped {SPEECH_LOOPS} more times (-c copy)"),
        "chunks": chunks,
        "retranscribe_ms": retranscribe_time.as_millis(),
        "term_hits": hits.len(),
        "segment_us": [segment_start, segment_end],
        "speech_span_us": [truth.speech.0, truth.speech.1],
        "candidate": {"id": candidate_id, "representative_us": representative},
        "event_us": [truth.event.0, truth.event.1],
        "crop_mean_rgb": [red, green, blue],
        "bundle_artifacts": validated["data"]["artifact_count"],
        "retain_ms": retain_time.as_millis(),
    }))
}

/// Stage 2: a run killed after its first checkpoint is interrupted and the
/// same command resumes it to the control segments; then a damaged
/// committed artifact is `INTEGRITY_FAILURE`.
fn kill_stage(base: &Path, control: &Control) -> StageResult {
    let session = ingest(base, &control.speech_clip)?;
    let mut child = spawn(base, &["transcript", "retranscribe", &session])?;
    wait_for_checkpoint(base, &session, &mut child)?;
    child.kill()?;
    let _ = child.wait()?;
    let job = only_job(base, &session)?;
    let interrupted = job_status(base, &job)?;
    ensure(
        interrupted["state"] == "interrupted"
            && interrupted["resumable"] == true
            && interrupted["live_owner"] == false,
        &format!("a killed run's job is not interrupted: {interrupted}"),
    )?;
    let kept = u64_of(&interrupted["progress"]["chunks_checkpointed"])?;
    ensure(kept >= 1, "the killed run kept no checkpoint")?;
    ensure(
        session_generation(base, &session)? == 1,
        "the killed run committed",
    )?;

    let (resumed, resume_time) = ok_json(base, &["transcript", "retranscribe", &session])?;
    ensure(
        resumed["data"]["job"]["job_id"] == job.as_str()
            && resumed["data"]["job"]["resumed"] == true
            && u64_of(&resumed["data"]["job"]["chunks_reused"])? >= 1,
        "the rerun did not resume the killed job",
    )?;
    let segments = spoken(base, &session, None)?;
    ensure(
        segments == control.segments,
        "the resumed revision's segments differ from the control run's",
    )?;

    // Damage the committed transcript record: reads fail closed.
    let artifacts = base
        .join("sessions")
        .join("sessions")
        .join(&session)
        .join("artifacts");
    let record = fs::read_dir(&artifacts)?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with("artifact-"))
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
        .ok_or_else(|| failed("no committed transcript record"))?;
    let mut bytes = fs::read(&record)?;
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x20;
    fs::write(&record, bytes)?;
    let (code, damaged) = run_json(
        vsift(base)?
            .args([
                "transcript",
                "get",
                &session,
                "--from",
                "0",
                "--to",
                "100000000",
            ])
            .arg("--json"),
    )?;
    ensure(
        code != Some(0) && damaged["error"]["code"] == "INTEGRITY_FAILURE",
        &format!("a damaged artifact is not an integrity failure: {damaged}"),
    )?;
    Ok(json!({
        "job": job,
        "checkpoints_after_kill": kept,
        "chunks_reused": resumed["data"]["job"]["chunks_reused"],
        "resume_ms": resume_time.as_millis(),
        "segments": segments.len(),
        "damaged_artifact": "INTEGRITY_FAILURE",
    }))
}

/// Stage 3: an interruption after the first checkpoint cancels the run
/// (`CANCELLED`, the session and job named, `job resume` suggested) and
/// leaves no provider running; a damaged checkpoint is then discarded by
/// `job resume`, which commits the control segments.
fn interrupt_stage(base: &Path, control: &Control) -> StageResult {
    let session = ingest(base, &control.speech_clip)?;
    let mut child = spawn(base, &["transcript", "retranscribe", &session])?;
    wait_for_checkpoint(base, &session, &mut child)?;
    let providers = running_providers(&mut child)?;
    let sent = Instant::now();
    interrupt(&child)?;
    let ended = finish(child, sent, SHUTDOWN_BUDGET, providers)?;
    let error = &ended.result["error"];
    ensure(
        ended.code == Some(6)
            && ended.result["status"] == "cancelled"
            && error["code"] == "CANCELLED",
        &format!(
            "the interrupted run did not answer CANCELLED: {}",
            ended.result
        ),
    )?;
    let job = str_of(&error["affected_ids"][1])?;
    ensure(
        error["affected_ids"][0] == session.as_str()
            && error["remediation"][0]["command"]["executable"] == "vsift"
            && error["remediation"][0]["command"]["arguments"] == json!(["job", "resume", job]),
        "the cancellation does not name the session and job and suggest job resume",
    )?;
    ensure(
        session_generation(base, &session)? == 1,
        "the interrupted run committed",
    )?;
    let status = job_status(base, &job)?;
    ensure(
        status["state"] == "interrupted" && status["failure"]["code"] == "CANCELLED",
        &format!("the interrupted job is not interrupted by a cancellation: {status}"),
    )?;

    // Damage the first checkpoint's payload; resume discards and redoes it.
    let first = checkpoints(base, &session)
        .into_iter()
        .next()
        .ok_or_else(|| failed("the interrupted run kept no checkpoint"))?;
    let mut bytes = fs::read(&first)?;
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x01;
    fs::write(&first, bytes)?;
    let (resumed, resume_time) = ok_json(base, &["job", "resume", &job])?;
    conforms("job-resume-data.schema.json", &resumed["data"])?;
    let warned = resumed["warnings"].as_array().is_some_and(|warnings| {
        warnings.iter().any(|warning| {
            warning
                .as_str()
                .is_some_and(|text| text.starts_with("checkpoint_discarded:"))
        })
    });
    ensure(
        resumed["data"]["job"]["state"] == "succeeded" && warned,
        "job resume did not commit with the damaged checkpoint discarded",
    )?;
    let segments = spoken(base, &session, None)?;
    ensure(
        segments == control.segments,
        "the resumed revision's segments differ from the control run's",
    )?;
    Ok(json!({
        "job": job,
        "interruption": if cfg!(windows) { "console Ctrl-Break (helper)" } else { "SIGINT" },
        "stopped_after_ms": ended.stopped_after.as_millis(),
        "providers_seen": ended.providers.len(),
        "resume_ms": resume_time.as_millis(),
        "checkpoint_discarded": true,
    }))
}

/// Stage 4: `job cancel` twice while the run is live.
fn cancel_stage(base: &Path, control: &Control) -> StageResult {
    let session = ingest(base, &control.speech_clip)?;
    let mut child = spawn(base, &["transcript", "retranscribe", &session])?;
    wait_for_checkpoint(base, &session, &mut child)?;
    let job = only_job(base, &session)?;
    let providers = running_providers(&mut child)?;
    let asked = Instant::now();
    let (first, _) = ok_json(base, &["job", "cancel", &job])?;
    let (second, _) = ok_json(base, &["job", "cancel", &job])?;
    for answer in [&first, &second] {
        conforms("job-data.schema.json", &answer["data"])?;
        ensure(
            matches!(
                answer["data"]["state"].as_str(),
                Some("cancelling" | "cancelled")
            ),
            &format!("job cancel did not cancel: {answer}"),
        )?;
    }
    let ended = finish(child, asked, SHUTDOWN_BUDGET, providers)?;
    ensure(
        ended.code == Some(6) && ended.result["error"]["code"] == "CANCELLED",
        &format!(
            "the cancelled run did not answer CANCELLED: {}",
            ended.result
        ),
    )?;
    let status = job_status(base, &job)?;
    ensure(
        status["state"] == "cancelled"
            && status["progress"]["chunks_checkpointed"] == 0
            && checkpoints(base, &session).is_empty(),
        &format!("the job is not cancelled without checkpoints: {status}"),
    )?;
    ensure(
        session_generation(base, &session)? == 1,
        "the cancelled run committed",
    )?;
    ensure(
        !job_directory(base, &session, &job).join("chunks").exists()
            || checkpoints(base, &session).is_empty(),
        "checkpoints remain",
    )?;
    Ok(json!({
        "job": job,
        "first_answer": first["data"]["state"],
        "second_answer": second["data"]["state"],
        "stopped_after_ms": ended.stopped_after.as_millis(),
        "providers_seen": ended.providers.len(),
    }))
}

/// Stage 5: X-02 and X-03 through the binary.
fn replay_stage(base: &Path, control: &Control) -> StageResult {
    const OPERATION: &str = "op_p10e2ereplay0123456789abcdef";
    let session = ingest(base, &control.speech_clip)?;
    let pointer = base
        .join("sessions")
        .join("sessions")
        .join(&session)
        .join("current.json");
    let before = fs::read(&pointer)?;
    let request = [
        "transcript",
        "retranscribe",
        session.as_str(),
        "--from",
        "0",
        "--to",
        "20000000",
        "--operation-id",
        OPERATION,
    ];
    let mut child = spawn(base, &request)?;
    let started = Instant::now();
    loop {
        if fs::read(&pointer).is_ok_and(|now| now != before) {
            break;
        }
        if child.try_wait()?.is_some() {
            break;
        }
        ensure(started.elapsed() < CLI_DEADLINE, "the run never committed")?;
        thread::sleep(Duration::from_millis(1));
    }
    // Kill it as soon as the pointer moved, before it can acknowledge.
    let _ = child.kill();
    let _ = child.wait()?;
    let job = only_job(base, &session)?;
    let left_as = job_status(base, &job)?["state"].clone();
    let generation = session_generation(base, &session)?;
    ensure(generation == 2, "the commit did not land exactly once")?;

    let (replayed, replay_time) = ok_json(base, &request)?;
    ensure(
        replayed["operation_id"] == OPERATION
            && replayed["data"]["job"]["replayed"] == true
            && replayed["data"]["job"]["job_id"] == job.as_str(),
        "the retry was not replayed",
    )?;
    ensure(
        session_generation(base, &session)? == generation,
        "the replay made a new generation",
    )?;
    let (code, conflict) = run_json(vsift(base)?.args([
        "transcript",
        "retranscribe",
        &session,
        "--from",
        "0",
        "--to",
        "10000000",
        "--operation-id",
        OPERATION,
        "--json",
    ]))?;
    ensure(
        code == Some(2)
            && conflict["error"]["code"] == "IDEMPOTENCY_CONFLICT"
            && conflict["error"]["affected_ids"] == json!([job]),
        &format!("the reused id was not a conflict: {conflict}"),
    )?;
    ensure(
        session_generation(base, &session)? == generation,
        "the conflict changed the session",
    )?;
    Ok(json!({
        "job": job,
        "state_after_kill": left_as,
        "replay_ms": replay_time.as_millis(),
        "fault_injection": "the pointer-rename kill point is exercised by the storage kill test; the release CLI cannot carry the fault-injection feature, so this stage kills the binary as the pointer moves",
    }))
}

/// Stage 6: an interruption halfway through `candidates` keeps what was
/// analysed; a rerun completes it.
fn candidates_stage(base: &Path, control: &Control) -> StageResult {
    let range = ["--from", "0", "--to", "600000000", "--limit", "100"];
    let reference = ingest(base, &control.visual_clip)?;
    let mut full = vec!["candidates", reference.as_str()];
    full.extend(range);
    let (complete, control_time) = ok_json(base, &full)?;
    ensure(
        complete["status"] == "complete",
        "the control candidates call was not complete",
    )?;
    let control_times: Vec<u64> = complete["data"]["items"]
        .as_array()
        .ok_or_else(|| failed("no control items"))?
        .iter()
        .filter_map(|item| item["representative_us"].as_u64())
        .collect();

    let session = ingest(base, &control.visual_clip)?;
    let mut call = vec!["candidates", session.as_str()];
    call.extend(range);
    let mut child = spawn(base, &call)?;
    thread::sleep(control_time / 2);
    let providers = running_providers(&mut child)?;
    let sent = Instant::now();
    interrupt(&child)?;
    let ended = finish(child, sent, SHUTDOWN_BUDGET, providers)?;
    let outcome = if ended.code == Some(0) {
        ensure(
            ended.result["status"] == "partial",
            &format!("the interrupted call is not partial: {}", ended.result),
        )?;
        conforms("candidates-data.schema.json", &ended.result["data"])?;
        "partial"
    } else {
        ensure(
            ended.code == Some(6) && ended.result["error"]["code"] == "CANCELLED",
            &format!(
                "the interrupted call is neither partial nor cancelled: {}",
                ended.result
            ),
        )?;
        "cancelled"
    };
    let (rerun, rerun_time) = ok_json(base, &call)?;
    let rerun_times: Vec<u64> = rerun["data"]["items"]
        .as_array()
        .ok_or_else(|| failed("no rerun items"))?
        .iter()
        .filter_map(|item| item["representative_us"].as_u64())
        .collect();
    ensure(
        rerun["status"] == "complete" && rerun_times == control_times,
        "the rerun did not complete to the control call's candidates",
    )?;
    Ok(json!({
        "clip": format!("F02.mp4 looped {VISUAL_LOOPS} more times (-c copy)"),
        "interruption": if cfg!(windows) { "console Ctrl-Break (helper)" } else { "SIGINT" },
        "control_ms": control_time.as_millis(),
        "interrupted_after_ms": (control_time / 2).as_millis(),
        "outcome": outcome,
        "stopped_after_ms": ended.stopped_after.as_millis(),
        "providers_seen": ended.providers.len(),
        "rerun_ms": rerun_time.as_millis(),
        "candidates": control_times.len(),
    }))
}

fn stage(name: &str, started: Instant, result: StageResult) -> Value {
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Ok(evidence) => json!({
            "name": name, "status": "passed", "elapsed_ms": elapsed_ms, "evidence": evidence,
        }),
        Err(StageStop::Failed(diagnostic)) => json!({
            "name": name, "status": "failed", "elapsed_ms": elapsed_ms, "diagnostic": diagnostic,
        }),
        Err(StageStop::Blocked(remediation)) => json!({
            "name": name, "status": "blocked", "elapsed_ms": elapsed_ms, "remediation": remediation,
        }),
    }
}

/// Runs a later stage in its own base, once the control run exists.
fn with_control(
    root: &OwnedRoot,
    name: &str,
    tools: Option<&Tools>,
    control: Option<&Control>,
    run: impl FnOnce(&Path, &Control) -> StageResult,
) -> StageResult {
    let Some(tools) = tools else {
        return Err(StageStop::Blocked(MISSING_TOOLS.to_owned()));
    };
    let Some(control) = control else {
        return Err(StageStop::Blocked(
            "p10_local_asr_journey did not pass, so there is no control run to compare with"
                .to_owned(),
        ));
    };
    let base = root.path(name);
    prepare(&base, tools)?;
    run(&base, control)
}

#[test]
#[ignore = "opt-in P10 recovery checkpoint; needs ffmpeg/ffprobe on PATH, VSIFT_TEST_WHISPER_CLI and VSIFT_TEST_WHISPER_MODEL; reports to .vsift/e2e-runs"]
fn recoverable_mechanical_run() -> TestResult {
    let started = Instant::now();
    let repository = repository().canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository
        .join(".vsift/e2e-runs")
        .join(format!("p10-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let root = OwnedRoot::new()?;
    let tools = Tools::discover();
    let mut stages = Vec::new();

    let clock = Instant::now();
    let mut control = None;
    let result = match tools.as_ref() {
        Some(tools) => {
            let base = root.path("journey");
            prepare(&base, tools).and_then(|()| journey_stage(&base, tools, &mut control))
        }
        None => Err(StageStop::Blocked(MISSING_TOOLS.to_owned())),
    };
    stages.push(stage("p10_local_asr_journey", clock, result));

    for (name, run) in [
        (
            "p10_kill_and_resume",
            kill_stage as fn(&Path, &Control) -> StageResult,
        ),
        ("p10_interrupt_and_job_resume", interrupt_stage),
        ("p10_job_cancel_twice", cancel_stage),
        ("p10_operation_replay", replay_stage),
        ("p10_interrupt_candidates", candidates_stage),
    ] {
        let clock = Instant::now();
        let result = with_control(&root, name, tools.as_ref(), control.as_ref(), run);
        stages.push(stage(name, clock, result));
    }

    let status_of = |wanted: &str| stages.iter().any(|entry| entry["status"] == wanted);
    let overall = if status_of("failed") {
        "failed"
    } else if status_of("blocked") {
        "blocked"
    } else {
        "passed"
    };
    stages.push(json!({"name": "p10_recovery", "status": overall}));
    let future_stages: Vec<_> = [
        "p11_worker_batch",
        "p12_agent_clients",
        "p13_distribution",
        "p14_release_qualification",
    ]
    .into_iter()
    .map(|name| json!({"name": name, "status": "not_implemented"}))
    .collect();
    let manifest: Value =
        serde_json::from_slice(&fs::read(repository.join("fixtures/corpus/manifest.json"))?)?;
    let report = json!({
        "schema_version": 1,
        "checkpoint": "P10 recoverable mechanical run",
        "fixture_manifest": {
            "schema_version": manifest["schema_version"],
            "corpus_id": manifest["corpus_id"],
        },
        "fixtures": "F03-speech.mp4 looped to 81 s and F02.mp4 looped to 492 s at run time without re-encoding; F03's manifest entry and speech provenance",
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "resource_profile": "each CLI call killed after 600 s; empty PATH; interrupted commands must end within 10 s and leave no provider running 10 s later",
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "authorization": "opt-in cargo test invocation; setup configure writes only to isolated temporary per-user bases; no install, download or network access",
        "prior_checkpoints": ["P09: p09_evidence_e2e"],
        "stages": stages,
        "coverage_gaps": [
            "The kill at the pointer rename uses the storage crate's fault points; the binary is killed as the pointer moves instead",
            "On Windows the interruption is a console Ctrl-Break: a process that inherited 'ignore Ctrl-C' never sees a Ctrl-C",
            "No OS or storage crash: durable publication is P10 PR 4's campaign"
        ],
        "future_stages": future_stages,
        "overall": overall,
        "complete_journey": "not_implemented",
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("P10 recovery checkpoint report: {}", report_path.display());
    println!("p10_recovery: {overall}");
    if overall == "passed" {
        Ok(())
    } else {
        Err(format!(
            "P10 recovery checkpoint {overall}; see {}",
            report_path.display()
        )
        .into())
    }
}
