//! Opt-in: time anchoring (ADR 0029) reads real container tags.
//!
//! The pinned `FFmpeg` on `PATH` writes four one-second files: an MP4 with a
//! UTC `creation_time`; a MOV with the recording app's local-time tag
//! (`com.apple.quicktime.creationdate`, written as a metadata key; the pinned
//! build then writes no movie-header creation time, as hosted run 39525837912
//! showed); a Matroska file with `DateUTC`; and an MP4
//! whose `creation_time` is from 1990, which is no recording's time. The
//! compiled `vsift` registers the tools, records their verification with one
//! supplied-transcript ingest of F10, then opens each file with a plain
//! `ingest` and reports the anchors it read. Nothing is downloaded: the files
//! are made in a temporary folder and removed with it.
//!
//! ```console
//! cargo test --release -p vsift-cli --locked --test time_anchor_real_tags -- --ignored --nocapture
//! ```
//!
//! It runs in the hosted `P07 local ASR` workflow, where the pinned build is
//! on `PATH`.

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
use serde_json::{Value, json};
use vsift_infrastructure::{ExecutableResolver, TrustedExecutable};

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-time-anchor-real-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const CLI_DEADLINE: Duration = Duration::from_mins(5);
const CREATED: &str = "2026-10-21T13:03:13.000000Z";

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

/// A bounded `vsift` run with isolated per-user state and no ambient `PATH`.
fn vsift(base: &Path, arguments: &[&OsStr]) -> Result<Value, Box<dyn Error>> {
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
    if output.status.code() != Some(0) {
        return Err(format!("vsift {arguments:?} failed: {value}").into());
    }
    Ok(value)
}

/// Makes one tagged file with the pinned `FFmpeg`: explicit executable and
/// arguments, no shell.
fn make(ffmpeg: &TrustedExecutable, output: &Path, tags: &[&str]) -> TestResult {
    let mut command = Process::new(ffmpeg.path());
    command.args([
        "-v",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=64x64:rate=10:duration=1",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=16000:duration=1",
        "-shortest",
        "-c:v",
        "mpeg4",
        "-c:a",
        "aac",
    ]);
    command.args(tags).arg(output);
    let status = command.status()?;
    if !status.success() {
        return Err(format!("ffmpeg could not make {}", output.display()).into());
    }
    Ok(())
}

fn ingest(base: &Path, file: &Path, extra: &[&str]) -> Result<Value, Box<dyn Error>> {
    let mut arguments: Vec<&OsStr> = vec![OsStr::new("ingest"), file.as_os_str()];
    arguments.extend(extra.iter().map(OsStr::new));
    let opened = vsift(base, &arguments)?;
    validate("ingest-data.schema.json", &opened["data"])?;
    println!(
        "{}: {}",
        file.file_name().and_then(OsStr::to_str).unwrap_or("source"),
        opened["data"].get("time_anchor").unwrap_or(&Value::Null)
    );
    Ok(opened)
}

#[test]
#[ignore = "opt-in time anchoring check; needs the pinned ffmpeg and ffprobe on PATH"]
#[allow(
    clippy::too_many_lines,
    reason = "one journey: make the files, verify the tools once, read every file's anchors"
)]
fn plain_ingest_reads_the_recording_time_tags_of_real_files() -> TestResult {
    let resolver = ExecutableResolver::from_current_path();
    let ffmpeg = resolver.resolve(OsStr::new("ffmpeg"))?;
    let ffprobe = resolver.resolve(OsStr::new("ffprobe"))?;
    let root = OwnedRoot::new()?;
    let base = root.0.join("user");
    fs::create_dir(&base)?;
    for (dependency, path) in [("ffmpeg", ffmpeg.path()), ("ffprobe", ffprobe.path())] {
        vsift(
            &base,
            &[
                OsStr::new("setup"),
                OsStr::new("configure"),
                OsStr::new(dependency),
                OsStr::new("--executable"),
                path.as_os_str(),
            ],
        )?;
    }

    let created = format!("creation_time={CREATED}");
    let mp4 = root.0.join("created.mp4");
    make(&ffmpeg, &mp4, &["-metadata", &created])?;
    let mov = root.0.join("recorder.mov");
    make(
        &ffmpeg,
        &mov,
        &[
            "-movflags",
            "use_metadata_tags",
            "-metadata",
            "com.apple.quicktime.creationdate=2026-10-21T14:03:12+0100",
            "-metadata",
            &created,
        ],
    )?;
    let mkv = root.0.join("created.mkv");
    make(&ffmpeg, &mkv, &["-metadata", &created])?;
    let old = root.0.join("old.mp4");
    make(
        &ffmpeg,
        &old,
        &["-metadata", "creation_time=1990-01-01T00:00:00Z"],
    )?;

    // Before any verification is recorded, a plain ingest reads no tag.
    let unverified = ingest(&base, &mp4, &[])?;
    assert!(
        unverified["data"].get("time_anchor").is_none(),
        "a plain ingest probed unverified tools: {unverified}"
    );

    // One supplied-transcript ingest verifies the tools and records the pass.
    let f10 = repository().join("fixtures/corpus/generated/F10.mp4");
    let srt = repository().join("fixtures/corpus/transcripts/F10.srt");
    let srt = srt.to_str().ok_or("non-UTF-8 repository path")?;
    ingest(
        &base,
        &f10,
        &["--transcript", srt, "--transcript-offset", "500000"],
    )?;

    let read = ingest(&base, &mp4, &[])?;
    assert_eq!(
        read["data"]["time_anchor"],
        json!({
            "consistency": "single",
            "source": "container_creation_time",
            "confidence": "medium",
            "recording_start": {"utc": CREATED, "local": null, "utc_offset": null},
            "anchors": [{
                "source": "container_creation_time",
                "confidence": "medium",
                "media_time_us": 0,
                "time": {"utc": CREATED, "local": null, "utc_offset": null}
            }]
        }),
        "MP4 creation_time"
    );

    let read = ingest(&base, &mov, &[])?;
    let anchor = &read["data"]["time_anchor"];
    assert_eq!(anchor["source"], "quicktime_creationdate", "{read}");
    assert_eq!(
        anchor["recording_start"],
        json!({
            "utc": "2026-10-21T13:03:12.000000Z",
            "local": "2026-10-21T14:03:12.000000",
            "utc_offset": "+01:00"
        })
    );
    let tags = anchor["anchors"].as_array().map_or(0, Vec::len);
    let expected = if tags == 1 { "single" } else { "consistent" };
    assert_eq!(anchor["consistency"], expected, "{read}");

    // The operator's statement agrees with the recorder's tag.
    let read = ingest(&base, &mov, &["--recorded-at", "2026-10-21T14:03:12+01:00"])?;
    let anchor = &read["data"]["time_anchor"];
    assert_eq!(anchor["source"], "operator", "{read}");
    assert_eq!(anchor["consistency"], "consistent", "{read}");
    assert_eq!(anchor["anchors"].as_array().map(Vec::len), Some(tags + 1));

    // A statement an hour off conflicts, and still decides.
    let read = ingest(&base, &mov, &["--recorded-at", "2026-10-21T15:03:12+01:00"])?;
    let anchor = &read["data"]["time_anchor"];
    assert_eq!(anchor["source"], "operator", "{read}");
    assert_eq!(anchor["consistency"], "conflicting", "{read}");

    let read = ingest(&base, &mkv, &[])?;
    assert_eq!(
        read["data"]["time_anchor"]["recording_start"]["utc"], CREATED,
        "Matroska DateUTC: {read}"
    );

    let read = ingest(&base, &old, &[])?;
    assert!(
        read["data"].get("time_anchor").is_none(),
        "a 1990 creation time was taken as a recording time: {read}"
    );

    let id = read["data"]["session_id"]
        .as_str()
        .ok_or("no session id")?
        .to_owned();
    let status = vsift(
        &base,
        &[OsStr::new("session"), OsStr::new("status"), OsStr::new(&id)],
    )?;
    assert!(status["data"].get("time_anchor").is_none());
    Ok(())
}
