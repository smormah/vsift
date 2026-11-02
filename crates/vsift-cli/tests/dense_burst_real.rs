//! Opt-in: a `frame burst` over a range too dense for one frame listing gets
//! the typed `outside_listing` answer on real video, never "the file may be
//! damaged" (L-015, #172 stays open).
//!
//! The pinned `FFmpeg` on `PATH` writes two small 70 s files of 30 fps video
//! (64x64, no audio): one at a constant rate and one that drops every eleventh
//! frame, so its rate varies. A 60 s range of either holds more than the 1,200
//! frames one listing covers. `FFmpeg`'s `-frames:v` limit is applied after the
//! filter graph, so on `FFmpeg` 9.0 the listing run logs one `showinfo` line
//! more than the limit; the reader used to reject that as damaged output. The
//! compiled `vsift` registers the tools, opens each file with a plain
//! `ingest`, asks for the dense burst, and then for a burst of the same file
//! that one listing does cover.
//!
//! ```console
//! cargo test --release -p vsift-cli --locked --test dense_burst_real -- --ignored --nocapture
//! ```
//!
//! It runs in the hosted `P07 local ASR` workflow, where the pinned build is
//! on `PATH`. Nothing is downloaded: the files are made in a temporary folder
//! and removed with it.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    process::Command as Process,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_infrastructure::{ExecutableResolver, TrustedExecutable};

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-dense-burst-real-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const CLI_DEADLINE: Duration = Duration::from_mins(5);

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

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
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

fn validate(schema: &str, instance: &Value) -> TestResult {
    let definition: Value =
        serde_json::from_slice(&fs::read(repository().join("schemas/v1").join(schema))?)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&definition)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema}: {error}")))?;
    Ok(())
}

/// A bounded `vsift` run with isolated per-user state and no ambient `PATH`:
/// the exit code and the validated response.
fn vsift(base: &Path, arguments: &[&OsStr]) -> Result<(Option<i32>, Value), Box<dyn Error>> {
    let output = Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", base)
        .env("XDG_CONFIG_HOME", base)
        .env("XDG_CACHE_HOME", base)
        .env("HOME", base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(base.join("sessions"))
        .args(arguments)
        .arg("--json")
        .timeout(CLI_DEADLINE)
        .output()?;
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    Ok((output.status.code(), value))
}

/// Makes one 70 s, 30 fps clip with the pinned `FFmpeg`: explicit executable
/// and arguments, no shell. `filter` drops frames to vary the rate.
fn make(ffmpeg: &TrustedExecutable, output: &Path, filter: Option<&str>) -> TestResult {
    let mut command = Process::new(ffmpeg.path());
    command.args([
        "-v",
        "error",
        "-nostdin",
        "-y",
        "-threads",
        "2",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=64x64:rate=30:duration=70",
    ]);
    if let Some(filter) = filter {
        command.args(["-vf", filter, "-fps_mode", "vfr"]);
    }
    command.args(["-c:v", "mpeg4"]).arg(output);
    let status = command.status()?;
    if !status.success() {
        return Err(format!("ffmpeg could not make {}", output.display()).into());
    }
    Ok(())
}

fn ingest(base: &Path, file: &Path) -> Result<String, Box<dyn Error>> {
    let (code, opened) = vsift(base, &[OsStr::new("ingest"), file.as_os_str()])?;
    if code != Some(0) {
        return Err(format!("ingest failed: {opened}").into());
    }
    validate("ingest-data.schema.json", &opened["data"])?;
    Ok(opened["data"]["session_id"]
        .as_str()
        .ok_or("no session id")?
        .to_owned())
}

fn burst(
    base: &Path,
    session: &str,
    from: &str,
    to: &str,
) -> Result<(Option<i32>, Value), Box<dyn Error>> {
    let arguments: Vec<&OsStr> = [
        "frame",
        "burst",
        session,
        "--from",
        from,
        "--to",
        to,
        "--max-frames",
        "6",
    ]
    .into_iter()
    .map(OsStr::new)
    .collect();
    vsift(base, &arguments)
}

#[test]
#[ignore = "opt-in dense burst check; needs the pinned ffmpeg and ffprobe on PATH"]
fn a_dense_burst_gets_outside_listing_not_a_damaged_file_error() -> TestResult {
    let resolver = ExecutableResolver::from_current_path();
    let ffmpeg = resolver.resolve(OsStr::new("ffmpeg"))?;
    let ffprobe = resolver.resolve(OsStr::new("ffprobe"))?;
    let root = OwnedRoot::new()?;
    let base = root.0.join("user");
    fs::create_dir(&base)?;
    for (dependency, path) in [("ffmpeg", ffmpeg.path()), ("ffprobe", ffprobe.path())] {
        let (code, value) = vsift(
            &base,
            &[
                OsStr::new("setup"),
                OsStr::new("configure"),
                OsStr::new(dependency),
                OsStr::new("--executable"),
                path.as_os_str(),
            ],
        )?;
        assert_eq!(code, Some(0), "{value}");
    }

    let constant = root.0.join("constant.mp4");
    make(&ffmpeg, &constant, None)?;
    let varying = root.0.join("varying.mp4");
    make(&ffmpeg, &varying, Some("select='gt(mod(n,11),0)'"))?;

    for (name, file) in [("constant rate", &constant), ("varying rate", &varying)] {
        let session = ingest(&base, file)?;

        // 60 s of 30 fps video is at least 1,600 frames: more than one listing.
        let (code, dense) = burst(&base, &session, "5000000", "65000000")?;
        assert_eq!(code, Some(2), "{name}: {dense}");
        assert_eq!(
            dense["error"]["code"], "INVALID_ARGUMENT",
            "{name}: {dense}"
        );
        let summary = dense["error"]["remediation"][0]["summary"]
            .as_str()
            .ok_or("no remediation")?;
        assert!(
            summary.contains("too dense there for one frame listing")
                && !summary.contains("damaged"),
            "{name}: {summary}"
        );

        // 20 s is 600 frames, which one listing covers.
        let (code, covered) = burst(&base, &session, "5000000", "25000000")?;
        assert_eq!(code, Some(0), "{name}: {covered}");
        validate("frame-data.schema.json", &covered["data"])?;
        assert_eq!(
            covered["data"]["items"].as_array().map(Vec::len),
            Some(6),
            "{name}"
        );
    }
    Ok(())
}
