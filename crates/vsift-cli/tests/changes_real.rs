//! Opt-in: `changes` through the compiled binary on the synthetic F04
//! recording, whose order 1017 changes status at 10 s while every other cell
//! stays as it was (`fixtures/corpus/manifest.json`, F04-E04; ADR 0034).
//!
//! The binary registers the `FFmpeg` and `FFprobe` on `PATH`, opens F04 with
//! a plain `ingest` and asks for the changes of its first 14 s: the answer is
//! complete, a change brackets 10 s and its largest region is under 5% of the
//! frame, every frame a change cites is listed with its file, and the same
//! request again names the same changes, frames and regions.
//!
//! ```console
//! cargo test -p vsift-cli --locked --test changes_real -- --ignored --nocapture
//! ```
//!
//! Nothing is downloaded: F04 is committed, and the session is made in a
//! temporary folder and removed with it.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_infrastructure::ExecutableResolver;

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-changes-real-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const CLI_DEADLINE: Duration = Duration::from_mins(5);
const STATUS_EDIT_US: u64 = 10_000_000;

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

fn ids(values: &Value, member: &str) -> Result<Vec<String>, Box<dyn Error>> {
    values
        .as_array()
        .ok_or("not a list")?
        .iter()
        .map(|value| {
            value[member]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("no {member}").into())
        })
        .collect()
}

#[test]
#[ignore = "opt-in changes check on F04; needs ffmpeg and ffprobe on PATH"]
fn f04_status_edit_is_one_small_region_between_two_cited_frames() -> TestResult {
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

    let fixture = repository().join("fixtures/corpus/generated/F04.mp4");
    let (code, opened) = vsift(&base, &[OsStr::new("ingest"), fixture.as_os_str()])?;
    assert_eq!(code, Some(0), "{opened}");
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or("no session id")?
        .to_owned();

    let request: Vec<&OsStr> = [
        "changes",
        session.as_str(),
        "--from",
        "0",
        "--to",
        "14000000",
        "--limit",
        "20",
    ]
    .into_iter()
    .map(OsStr::new)
    .collect();
    let (code, first) = vsift(&base, &request)?;
    assert_eq!(code, Some(0), "{first}");
    validate("changes-data.schema.json", &first["data"])?;
    assert_eq!(first["status"], "complete", "{first}");
    let data = &first["data"];
    assert_eq!(data["next_cursor"], Value::Null);
    assert_eq!(data["resume"], Value::Null);
    assert_eq!(data["partial_reason"], Value::Null);

    let frames = ids(&data["frames"], "evidence_id")?;
    let files = ids(&data["files"], "evidence_id")?;
    assert_eq!(frames, files, "every frame has its file, in order");
    let changes = data["changes"].as_array().ok_or("no changes")?;
    for change in changes {
        validate("screen-change.schema.json", change)?;
        for side in ["before", "after"] {
            let id = change[side]["evidence_id"].as_str().ok_or("no id")?;
            assert!(frames.iter().any(|frame| frame == id), "{id} not listed");
        }
    }

    let status = changes
        .iter()
        .find(|change| {
            let from = change["change_window"]["from_us"]
                .as_u64()
                .unwrap_or(u64::MAX);
            let to = change["change_window"]["to_us"].as_u64().unwrap_or(0);
            from < STATUS_EDIT_US && STATUS_EDIT_US <= to
        })
        .ok_or("no change brackets the status edit at 10 s")?;
    assert_eq!(status["comparison"], "compared", "{status}");
    let after = status["after"]["evidence_id"].as_str().ok_or("no after")?;
    let frame = data["frames"]
        .as_array()
        .ok_or("no frames")?
        .iter()
        .find(|frame| frame["evidence_id"] == after)
        .ok_or("the after frame is not listed")?;
    let area = frame["image"]["width"].as_u64().ok_or("no width")?
        * frame["image"]["height"].as_u64().ok_or("no height")?;
    let largest = &status["regions"][0];
    let region = largest["width"].as_u64().ok_or("no region")?
        * largest["height"].as_u64().ok_or("no region")?;
    assert!(
        region * 20 < area,
        "the status edit should change under 5% of the frame, not {region} of {area} pixels"
    );

    let (code, again) = vsift(&base, &request)?;
    assert_eq!(code, Some(0), "{again}");
    assert_eq!(again["data"]["changes"], data["changes"]);
    assert_eq!(again["data"]["frames"], data["frames"]);
    Ok(())
}
