//! Opt-in P14 checkpoint through the binary under test: hostile file names
//! and a sentinel environment (RQ-05; SEC-01, SEC-25; C-04, P-01, P-02).
//!
//! The other real-tool checkpoints never used a hostile name, and none looked at
//! what a tool child inherits. This one drives whichever `vsift` the checkpoint
//! override selects (`tests/published_binary/mod.rs`; without it, the Cargo-built
//! binary) with:
//!
//! - `p14_hostile_paths`: copies of F01 named with quotes, spaces, shell and
//!   glob characters, a leading dash, Unicode and (where the file system allows
//!   it) a newline, a tab and a backslash, ingested through the real `FFprobe` and
//!   `FFmpeg`. Each must open a session and deliver a frame, leave the original
//!   byte for byte unchanged, and create nothing beside it (no `pwned` file
//!   from a name that looks like a command). A name that starts with a dash is a
//!   typed `INVALID_ARGUMENT` unless it follows `--` or is written `./-name`,
//!   and a name that is not a file at all is a typed failure that runs nothing.
//! - `p14_sentinel_environment`: variables that look like secrets (a token, a
//!   cloud key, a password) are set in the environment of `vsift`, and a
//!   recorder, a copy of this very executable staged as `FFmpeg`, `FFprobe` and
//!   whisper.cpp, writes down every argument and environment variable it is
//!   started with. `setup check` and `ingest` start it; no recorded variable
//!   may carry a sentinel in its name or value, and no output of `vsift` may
//!   show one. A recorder that was never started proves nothing and fails.
//!
//! ```console
//! VSIFT_P14_INSTALLED_E2E=1 cargo test --release -p vsift-cli --locked \
//!   --test p14_installed_binary_e2e -- --ignored --nocapture
//! ```
//!
//! The target has no libtest harness (`harness = false` in `Cargo.toml`), because
//! the recorder is this executable started by `vsift` with `vsift`'s own argument
//! list, which libtest would reject; the file name decides which role it plays.
//! Without `VSIFT_P14_INSTALLED_E2E=1` it prints that it was skipped and succeeds,
//! so `cargo test --workspace` stays green; the opt-in is the only way it runs.
//! `FFmpeg` and `FFprobe` must be on `PATH` for the hostile-path stage, which is
//! `blocked` without them. The run writes `.vsift/e2e-runs/p14-installed-<id>/
//! report.json` and prints `p14_installed: passed`, `failed` or `blocked`.

mod published_binary;

use std::{
    env,
    error::Error,
    ffi::{OsStr, OsString},
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command as Process, ExitCode},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::{Value, json};
use vsift_infrastructure::{ExecutableResolver, TrustedExecutable};

const OPT_IN: &str = "VSIFT_P14_INSTALLED_E2E";
/// Upper bound for one CLI invocation; a prompt or hang is killed and fails.
const CLI_DEADLINE: Duration = Duration::from_secs(120);
const OWNED_PREFIX: &str = "vsift-p14-installed-e2e-";
const MAX_DIAGNOSTIC_CHARS: usize = 300;
/// The recorder's file name starts with this, then the role it plays.
const RECORDER_STEM_PREFIX: &str = "p14-recorder-";
const RECORDER_ROLES: [&str; 3] = ["ffmpeg", "ffprobe", "whisper"];
/// Values that must never appear in a tool child's environment or in any
/// output. Each is long and unlikely enough that a match is a leak, not luck.
const SENTINELS: [(&str, &str); 4] = [
    ("VSIFT_E2E_SENTINEL_TOKEN", "sentinel-token-6d1f9a7c42e8"),
    ("AWS_SECRET_ACCESS_KEY", "sentinel-cloud-key-b83e5f0a19d4"),
    ("GITHUB_TOKEN", "sentinel-vcs-token-27c4a6e1f593"),
    ("DB_PASSWORD", "sentinel-password-91a0d3b75e2c"),
];

type Failure = Box<dyn Error>;

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

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A temporary directory this checkpoint created and alone may remove.
struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, Failure> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!("{OWNED_PREFIX}{}-{stamp}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
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

/// The recorder's side: when this executable is started under a recorder name,
/// it appends its arguments and environment to a file beside itself, prints
/// nothing and succeeds.
fn run_as_recorder() -> Option<ExitCode> {
    let executable = env::current_exe().ok()?;
    if !executable
        .file_stem()?
        .to_str()?
        .starts_with(RECORDER_STEM_PREFIX)
    {
        return None;
    }
    let mut record = String::from("invocation\n");
    for argument in env::args_os().skip(1) {
        let _ = writeln!(record, "arg {}", argument.to_string_lossy());
    }
    for (name, value) in env::vars_os() {
        let _ = writeln!(
            record,
            "env {}={}",
            name.to_string_lossy(),
            value.to_string_lossy()
        );
    }
    let written = OpenOptions::new()
        .create(true)
        .append(true)
        .open(record_path(&executable))
        .and_then(|mut file| file.write_all(record.as_bytes()));
    Some(if written.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(70)
    })
}

/// The file name of a recorder staged for `role`, with this executable's extension.
fn recorder_file_name(role: &str, extension: Option<&OsStr>) -> OsString {
    let mut name = OsString::from(format!("{RECORDER_STEM_PREFIX}{role}"));
    if let Some(extension) = extension {
        name.push(".");
        name.push(extension);
    }
    name
}

/// Where a recorder writes: its own path with `.record` appended.
fn record_path(executable: &Path) -> PathBuf {
    let mut name = executable.as_os_str().to_owned();
    name.push(".record");
    PathBuf::from(name)
}

/// A bounded `vsift` invocation with isolated per-user state and no ambient `PATH`.
fn vsift(base: &Path) -> Result<Command, StageStop> {
    let mut command = published_binary::command()?;
    command
        .env("LOCALAPPDATA", base)
        .env("XDG_CONFIG_HOME", base)
        .env("XDG_DATA_HOME", base.join("data"))
        .env("XDG_CACHE_HOME", base.join("cache"))
        .env("HOME", base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(base.join("sessions"))
        .timeout(CLI_DEADLINE);
    Ok(command)
}

/// Runs a JSON command: exit code and the one result document.
fn run_json(command: &mut Command) -> Result<(Option<i32>, Value), StageStop> {
    let output = command.output()?;
    ensure(
        output.stderr.is_empty(),
        "a JSON command wrote to standard error",
    )?;
    Ok((
        output.status.code(),
        serde_json::from_slice(&output.stdout)?,
    ))
}

struct MediaTools {
    ffmpeg: TrustedExecutable,
    ffprobe: TrustedExecutable,
}

impl MediaTools {
    fn discover() -> Option<Self> {
        let resolver = ExecutableResolver::from_current_path();
        Some(Self {
            ffmpeg: resolver.resolve(OsStr::new("ffmpeg")).ok()?,
            ffprobe: resolver.resolve(OsStr::new("ffprobe")).ok()?,
        })
    }
}

/// Registers `path` as `dependency` in the base of `command`.
fn configure(base: &Path, dependency: &str, path: &Path) -> Result<(), StageStop> {
    let (code, result) = run_json(
        vsift(base)?
            .args(["setup", "configure", dependency, "--executable"])
            .arg(path)
            .arg("--json"),
    )?;
    ensure(
        code == Some(0),
        &format!("setup configure {dependency} failed: {}", result["error"]),
    )
}

/// The names to try on this system: what its file system lets a file be called.
fn hostile_names() -> Vec<&'static str> {
    let mut names = vec![
        "quote'single.mp4",
        "-leading-dash.mp4",
        "--help.mp4",
        "semi;colon & amp $(touch pwned) ^caret %PATH%.mp4",
        "\u{fc}n\u{ef} c\u{f6}d\u{e9} \u{52d5}\u{753b}.mp4",
        "[brackets]{braces}!bang#hash@at,comma=eq+plus~tilde.mp4",
    ];
    if cfg!(unix) {
        names.extend([
            "double\"quote.mp4",
            "new\nline.mp4",
            "tab\there.mp4",
            "back\\slash.mp4",
            "`backtick`.mp4",
            "star*glob?.mp4",
            "$HOME.mp4",
            "; touch pwned;.mp4",
        ]);
    }
    names
}

/// The stage `p14_hostile_paths`; see the module documentation.
fn hostile_paths(root: &OwnedRoot, tools: Option<&MediaTools>) -> StageResult {
    let tools = tools.ok_or_else(|| {
        StageStop::Blocked(String::from(
            "FFmpeg or FFprobe is not on PATH: put trusted builds there and rerun",
        ))
    })?;
    let base = root.0.join("hostile");
    let inputs = base.join("inputs");
    fs::create_dir_all(&inputs)?;
    configure(&base, "ffmpeg", tools.ffmpeg.path())?;
    configure(&base, "ffprobe", tools.ffprobe.path())?;
    let original = fs::read(repository().join("fixtures/corpus/generated/F01.mp4"))?;
    let mut tried = Vec::new();
    for name in hostile_names() {
        let source = inputs.join(name);
        fs::write(&source, &original)?;
        let (code, opened) = run_json(vsift(&base)?.args(["ingest", "--json", "--"]).arg(&source))?;
        ensure(
            code == Some(0),
            &format!("ingest of {name:?} failed: {}", opened["error"]),
        )?;
        let session = opened["data"]["session_id"]
            .as_str()
            .ok_or_else(|| StageStop::Failed(format!("ingest of {name:?} opened no session")))?;
        let (code, frame) =
            run_json(vsift(&base)?.args(["frame", "get", session, "--at", "0", "--json"]))?;
        ensure(
            code == Some(0),
            &format!("frame get after {name:?} failed: {}", frame["error"]),
        )?;
        ensure(
            fs::read(&source)? == original,
            &format!("the original {name:?} changed"),
        )?;
        tried.push(name);
    }
    // A leading dash is data only behind `--` or as `./-name`; bare, it is an
    // option the parser refuses with its typed code, and nothing is opened.
    let (code, undashed) = run_json(vsift(&base)?.current_dir(&inputs).args([
        "ingest",
        "--json",
        "-leading-dash.mp4",
    ]))?;
    ensure(
        code == Some(2) && undashed["error"]["code"] == "INVALID_ARGUMENT",
        &format!("a bare leading dash was not a typed refusal: {code:?}"),
    )?;
    let (code, relative) = run_json(vsift(&base)?.current_dir(&inputs).args([
        "ingest",
        "--json",
        "./-leading-dash.mp4",
    ]))?;
    ensure(
        code == Some(0),
        &format!("./-leading-dash.mp4 did not open: {}", relative["error"]),
    )?;
    // Names that are not files: typed failures, and nothing they say runs.
    for text in ["; touch pwned;", "$(touch pwned)", "`touch pwned`", "-i"] {
        let (code, refused) = run_json(
            vsift(&base)?
                .current_dir(&inputs)
                .args(["ingest", "--json", "--", text]),
        )?;
        ensure(
            code != Some(0) && refused["error"]["code"].is_string(),
            &format!("{text:?} was not refused with a typed code"),
        )?;
    }
    for place in [&base, &inputs, &root.0] {
        ensure(
            !place.join("pwned").exists(),
            "a file named by a hostile name was created",
        )?;
    }
    // Counted, not compared by name: a file system may store a name in another
    // Unicode normalisation than the one it was created with.
    ensure(
        fs::read_dir(&inputs)?.count() == tried.len(),
        "the input folder holds something other than the copies this stage made",
    )?;
    Ok(json!({
        "names_tried": tried.len(),
        "names": tried.iter().map(|name| name.escape_debug().to_string()).collect::<Vec<_>>(),
        "bare_leading_dash": "INVALID_ARGUMENT, exit 2",
        "dot_slash_leading_dash": "opened",
        "not_files_refused_typed": 4,
    }))
}

/// Everything a recorder wrote: one block per start.
struct Recording {
    starts: usize,
    variables: Vec<(String, String)>,
    arguments: Vec<String>,
}

fn read_recording(executable: &Path) -> Result<Recording, StageStop> {
    let text = match fs::read_to_string(record_path(executable)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut starts = 0;
    let mut variables = Vec::new();
    let mut arguments = Vec::new();
    for line in text.lines() {
        if line == "invocation" {
            starts += 1;
        } else if let Some(entry) = line.strip_prefix("env ") {
            // A value may hold a line break, whose rest this line parser sees
            // as an unrecognised line; the sentinel scan below covers the text.
            let (name, value) = entry.split_once('=').unwrap_or((entry, ""));
            variables.push((name.to_owned(), value.to_owned()));
        } else if let Some(argument) = line.strip_prefix("arg ") {
            arguments.push(argument.to_owned());
        }
    }
    Ok(Recording {
        starts,
        variables,
        arguments,
    })
}

/// Stages a recorder for each tool role and registers it in `base`, then proves
/// the recorder can see a leak: started directly with a sentinel, it writes the
/// variable down, so an empty record under `vsift` means the child did not have it.
fn stage_recorders(base: &Path, tools: &Path) -> Result<Vec<(&'static str, PathBuf)>, StageStop> {
    let this = env::current_exe()?;
    let extension = this.extension().map(OsStr::to_owned);
    let mut recorders = Vec::new();
    for role in RECORDER_ROLES {
        let staged = tools.join(recorder_file_name(role, extension.as_deref()));
        fs::copy(&this, &staged)?;
        configure(base, role, &staged)?;
        recorders.push((role, staged));
    }
    let control = tools.join(recorder_file_name("control", extension.as_deref()));
    fs::copy(&this, &control)?;
    let (control_name, control_value) = SENTINELS[0];
    let status = Process::new(&control)
        .env(control_name, control_value)
        .status()?;
    let seen = read_recording(&control)?;
    ensure(
        status.success()
            && seen.starts == 1
            && seen
                .variables
                .iter()
                .any(|(name, value)| name == control_name && value == control_value),
        "the recorder did not write down a variable it was started with: the check cannot see a leak",
    )?;
    Ok(recorders)
}

/// Reads what each recorder was started with: every role was started at least
/// once, and no argument, variable name or value holds a sentinel.
fn audit_recorders(recorders: &[(&'static str, PathBuf)]) -> Result<Value, StageStop> {
    let mut starts = serde_json::Map::new();
    for (role, executable) in recorders {
        let recording = read_recording(executable)?;
        ensure(
            recording.starts > 0,
            &format!("the {role} recorder was never started, so nothing was checked"),
        )?;
        for (name, value) in &recording.variables {
            for (sentinel_name, sentinel_value) in SENTINELS {
                ensure(
                    !name.contains(sentinel_name) && !value.contains(sentinel_value),
                    &format!("the {role} child inherited {sentinel_name}"),
                )?;
            }
        }
        ensure(
            recording
                .arguments
                .iter()
                .all(|argument| SENTINELS.iter().all(|(_, value)| !argument.contains(value))),
            &format!("a sentinel reached the {role} child's arguments"),
        )?;
        starts.insert(
            (*role).to_owned(),
            json!({
                "starts": recording.starts,
                "variables_seen": recording.variables.len(),
            }),
        );
    }
    Ok(Value::Object(starts))
}

/// The stage `p14_sentinel_environment`; see the module documentation.
fn sentinel_environment(root: &OwnedRoot) -> StageResult {
    let base = root.0.join("sentinel");
    let tools = base.join("tools");
    fs::create_dir_all(&tools)?;
    let recorders = stage_recorders(&base, &tools)?;
    let fixture = repository()
        .join("fixtures/corpus/generated/F01.mp4")
        .canonicalize()?;
    let with_sentinels = |command: &mut Command| {
        for (name, value) in SENTINELS {
            command.env(name, value);
        }
    };
    let mut command = vsift(&base)?;
    with_sentinels(&mut command);
    let check = command.args(["setup", "check", "--json"]).output()?;
    // `ingest` copies the source and starts no tool; `frame get` does, with
    // the recorder standing in for FFprobe and FFmpeg, and ends in a typed
    // failure for want of a real one.
    let mut command = vsift(&base)?;
    with_sentinels(&mut command);
    let ingest = command.args(["ingest", "--json"]).arg(&fixture).output()?;
    let opened: Value = serde_json::from_slice(&ingest.stdout)?;
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or_else(|| StageStop::Failed(String::from("ingest opened no session")))?;
    let mut command = vsift(&base)?;
    with_sentinels(&mut command);
    let frame = command
        .args(["frame", "get", session, "--at", "0", "--json"])
        .output()?;
    // What vsift itself printed must hold no sentinel either.
    for (what, bytes) in [
        ("setup check", &check.stdout),
        ("setup check (stderr)", &check.stderr),
        ("ingest", &ingest.stdout),
        ("ingest (stderr)", &ingest.stderr),
        ("frame get", &frame.stdout),
        ("frame get (stderr)", &frame.stderr),
    ] {
        let text = String::from_utf8_lossy(bytes);
        for (name, value) in SENTINELS {
            ensure(
                !text.contains(value) && !text.contains(name),
                &format!("{what} output shows {name}"),
            )?;
        }
    }
    Ok(json!({
        "sentinels": SENTINELS.map(|(name, _)| name),
        "control_recorded_a_variable": true,
        "recorders": audit_recorders(&recorders)?,
        "setup_check_exit": check.status.code(),
        "ingest_exit": ingest.status.code(),
        "frame_get_exit": frame.status.code(),
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

fn checkpoint() -> Result<String, Failure> {
    let started = Instant::now();
    let root = OwnedRoot::new()?;
    let tools = MediaTools::discover();
    let mut stages = Vec::new();
    let clock = Instant::now();
    stages.push(stage(
        "p14_hostile_paths",
        clock,
        hostile_paths(&root, tools.as_ref()),
    ));
    let clock = Instant::now();
    stages.push(stage(
        "p14_sentinel_environment",
        clock,
        sentinel_environment(&root),
    ));
    let any = |wanted: &str| stages.iter().any(|entry| entry["status"] == wanted);
    let overall = if any("failed") {
        "failed"
    } else if any("blocked") {
        "blocked"
    } else {
        "passed"
    };
    let report = json!({
        "schema_version": 1,
        "checkpoint": "P14 installed-binary checks (hostile paths, sentinel environment)",
        "binary_under_test": published_binary::report()?,
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "resource_profile": "each CLI call killed after 120 s; empty PATH; per-user state in a temporary base",
        "stages": stages,
        "overall": overall,
        "elapsed_ms": started.elapsed().as_millis(),
    });
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository()
        .canonicalize()?
        .join(".vsift/e2e-runs")
        .join(format!("p14-installed-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    println!("P14 installed-binary report: {}", report_path.display());
    println!("p14_installed: {overall}");
    Ok(overall.to_owned())
}

fn main() -> ExitCode {
    if let Some(code) = run_as_recorder() {
        return code;
    }
    if env::var_os(OPT_IN).is_none() {
        println!(
            "p14_installed: skipped (opt-in: set {OPT_IN}=1 and, for a published binary, {})",
            published_binary::BINARY_VARIABLE
        );
        return ExitCode::SUCCESS;
    }
    match checkpoint() {
        Ok(overall) if overall == "passed" => ExitCode::SUCCESS,
        Ok(overall) => {
            eprintln!("P14 installed-binary checkpoint {overall}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("P14 installed-binary checkpoint could not run: {error}");
            ExitCode::FAILURE
        }
    }
}
