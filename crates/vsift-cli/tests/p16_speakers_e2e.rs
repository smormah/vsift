//! Opt-in speaker-label checkpoint (0.3.0, ADR 0031) with the real engine.
//!
//! Drives the compiled `vsift` binary as an agent would, with isolated per-user state
//! and an empty `PATH`: a synthetic two-voice conversation of the evaluation
//! (`SPK2-MF`, exact truth by sample position) is put in a video with `FFmpeg`, opened
//! with `ingest --transcript` (one cue per true turn, so each segment has one true
//! speaker), labelled by `transcript speakers` with the real sherpa-onnx tool and its
//! two models, read back with `transcript get`, named with `transcript name-speaker`,
//! and searched with `search --speaker`. Every answer is checked against its published
//! schema, and the labels against the truth.
//!
//! ```console
//! VSIFT_TEST_FFMPEG=<absolute ffmpeg>  VSIFT_TEST_FFPROBE=<absolute ffprobe>
//! VSIFT_TEST_DIARIZER=<absolute sherpa-onnx-offline-speaker-diarization>
//! VSIFT_TEST_SEGMENTATION_MODEL=<absolute pyannote segmentation 3.0 model.onnx>
//! VSIFT_TEST_EMBEDDING_MODEL=<absolute nemo_en_titanet_small.onnx>
//! VSIFT_TEST_SPEAKER_FIXTURES=<folder holding SPK2-MF.wav and SPK2-MF.truth.json>
//! cargo test --release -p vsift-cli --locked --test p16_speakers_e2e -- --ignored --nocapture
//! ```
//!
//! It runs in the hosted workflow `P16 speakers evaluation`, never on a development
//! machine: the tools are the pinned builds staged there, given by absolute path.

mod published_binary;

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    ffi::OsStr,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command as Process,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

const CLI_DEADLINE: Duration = Duration::from_mins(15);
const OWNED_PREFIX: &str = "vsift-p16-speakers-e2e-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const FIXTURE: &str = "SPK2-MF";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A temporary folder this test made, removed with its contents when it drops.
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
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn absolute(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.exists())
        .ok_or_else(|| {
            format!("set {name} to an absolute path of an existing file or folder").into()
        })
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

fn conforms(schema: &str, instance: &Value) -> TestResult {
    let definition: Value =
        serde_json::from_slice(&fs::read(repository().join("schemas/v1").join(schema))?)?;
    let validator = jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&definition)?;
    if let Err(error) = validator.validate(instance) {
        return Err(format!("{schema}: {error}").into());
    }
    Ok(())
}

/// A bounded `vsift` invocation with isolated per-user state and no ambient `PATH`.
fn vsift(base: &Path) -> Result<Command, Box<dyn Error>> {
    let mut command = published_binary::command()?;
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

fn run_json(command: &mut Command) -> Result<(Option<i32>, Value), Box<dyn Error>> {
    let output = command.output()?;
    Ok((
        output.status.code(),
        serde_json::from_slice(&output.stdout)?,
    ))
}

fn timestamp(micros: u64) -> String {
    let millis = micros / 1_000;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        millis / 3_600_000,
        millis / 60_000 % 60,
        millis / 1_000 % 60,
        millis % 1_000
    )
}

/// One SRT cue per true turn that does not overlap the cue before it, with the true
/// speaker of each cue's start time.
fn cues_from_truth(truth: &Value) -> Result<(String, BTreeMap<u64, String>), Box<dyn Error>> {
    let mut srt = String::new();
    let mut speakers = BTreeMap::new();
    let mut last_end = 0_u64;
    let mut ordinal = 0_u32;
    for turn in truth["turns"].as_array().ok_or("truth has no turns")? {
        let start = turn["start_us"].as_u64().ok_or("turn start")?;
        let end = turn["end_us"].as_u64().ok_or("turn end")?;
        if start < last_end {
            continue;
        }
        ordinal += 1;
        let text = turn["text"].as_str().ok_or("turn text")?;
        write!(
            srt,
            "{ordinal}\n{} --> {}\n{text}\n\n",
            timestamp(start),
            timestamp(end)
        )?;
        speakers.insert(
            start,
            turn["speaker"].as_str().ok_or("turn speaker")?.to_owned(),
        );
        last_end = end;
    }
    Ok((srt, speakers))
}

/// The true speaker of the segment that starts at `start_us` (SRT keeps millisecond
/// times, so the nearest cue start within 1 ms).
fn true_speaker(speakers: &BTreeMap<u64, String>, start_us: u64) -> Option<&str> {
    speakers
        .range(start_us.saturating_sub(1_000)..=start_us + 1_000)
        .next()
        .map(|(_, speaker)| speaker.as_str())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "One journey in order: a split would hide which stage failed"
)]
#[ignore = "needs the real diarization engine, its models and FFmpeg; runs in P16 speakers evaluation"]
fn speaker_labels_follow_the_true_voices_end_to_end() -> TestResult {
    let ffmpeg = absolute("VSIFT_TEST_FFMPEG")?;
    let ffprobe = absolute("VSIFT_TEST_FFPROBE")?;
    let diarizer = absolute("VSIFT_TEST_DIARIZER")?;
    let segmentation = absolute("VSIFT_TEST_SEGMENTATION_MODEL")?;
    let embedding = absolute("VSIFT_TEST_EMBEDDING_MODEL")?;
    let fixtures = absolute("VSIFT_TEST_SPEAKER_FIXTURES")?;
    let root = OwnedRoot::new()?;
    let base = root.0.join("user");
    fs::create_dir(&base)?;

    // The conversation in a small video, and its true turns as an SRT.
    let truth: Value =
        serde_json::from_slice(&fs::read(fixtures.join(format!("{FIXTURE}.truth.json")))?)?;
    let (srt, speakers) = cues_from_truth(&truth)?;
    let transcript = root.0.join("conversation.srt");
    fs::write(&transcript, srt)?;
    let video = root.0.join("conversation.mp4");
    let encoded = Process::new(&ffmpeg)
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("color=c=gray:s=320x240:r=5")
        .arg("-i")
        .arg(fixtures.join(format!("{FIXTURE}.wav")))
        .args(["-shortest", "-c:v", "mpeg4", "-c:a", "aac", "-b:a", "128k"])
        .arg(&video)
        .status()?;
    if !encoded.success() {
        return Err("FFmpeg could not make the conversation video".into());
    }

    for (dependency, path) in [("ffmpeg", &ffmpeg), ("ffprobe", &ffprobe)] {
        let (code, _) = run_json(
            vsift(&base)?
                .args(["setup", "configure", dependency, "--executable"])
                .arg(path)
                .arg("--json"),
        )?;
        assert_eq!(code, Some(0), "setup configure {dependency}");
    }
    let (code, opened) = run_json(
        vsift(&base)?
            .arg("ingest")
            .arg(&video)
            .arg("--transcript")
            .arg(&transcript)
            .arg("--json"),
    )?;
    assert_eq!(code, Some(0), "ingest: {opened}");
    let session = opened["data"]["session_id"]
        .as_str()
        .ok_or("ingest gave no session")?
        .to_owned();

    // Label with the user's count, then read every segment back.
    let (code, labelled) = run_json(
        vsift(&base)?
            .args([
                "transcript",
                "speakers",
                &session,
                "--speakers",
                "2",
                "--diarizer",
            ])
            .arg(&diarizer)
            .arg("--segmentation-model")
            .arg(&segmentation)
            .arg("--embedding-model")
            .arg(&embedding)
            .arg("--json"),
    )?;
    assert_eq!(code, Some(0), "transcript speakers: {labelled}");
    conforms("operation-response.schema.json", &labelled)?;
    conforms("transcript-speakers-data.schema.json", &labelled["data"])?;
    let labels = labelled["data"]["speakers"]["labels"]
        .as_array()
        .ok_or("no labels")?;
    assert_eq!(labels.len(), 2, "two voices, two labels: {labelled}");

    let duration = truth["duration_us"].as_u64().ok_or("duration")?;
    let (code, page) = run_json(
        vsift(&base)?
            .args(["transcript", "get", &session, "--from", "0", "--to"])
            .arg(duration.to_string())
            .args(["--limit", "100", "--json"]),
    )?;
    assert_eq!(code, Some(0), "transcript get: {page}");
    conforms("transcript-get-data.schema.json", &page["data"])?;
    let items = page["data"]["items"].as_array().ok_or("no items")?;
    let mut votes: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut labelled_items = 0_u32;
    for item in items {
        let start = item["start_us"].as_u64().ok_or("start")?;
        let truth_speaker = true_speaker(&speakers, start).ok_or("a segment has no true turn")?;
        if let Some(label) = item["speaker_label"]["label"].as_str() {
            labelled_items += 1;
            *votes
                .entry((truth_speaker.to_owned(), label.to_owned()))
                .or_insert(0) += 1;
        }
    }
    // The best one-to-one mapping of true speakers to labels, and how many agree.
    let vote = |truth: &str, label: &str| {
        votes
            .get(&(truth.to_owned(), label.to_owned()))
            .copied()
            .unwrap_or_default()
    };
    let agree = (vote("A", "S1") + vote("B", "S2")).max(vote("A", "S2") + vote("B", "S1"));
    let total = u32::try_from(items.len())?;
    println!(
        "{FIXTURE}: {total} segments, {labelled_items} labelled, {agree} agree with the truth"
    );
    assert!(
        labelled_items * 10 >= total * 8,
        "at least 80% of segments labelled"
    );
    assert!(
        agree * 10 >= labelled_items * 9,
        "at least 90% of labels right"
    );

    // Name S1 and find its segments by the name.
    let (code, named) = run_json(vsift(&base)?.args([
        "transcript",
        "name-speaker",
        &session,
        "--label",
        "S1",
        "--name",
        "Example Caller",
        "--json",
    ]))?;
    assert_eq!(code, Some(0), "name-speaker: {named}");
    conforms("transcript-speakers-data.schema.json", &named["data"])?;
    assert_eq!(
        named["data"]["speakers"]["labels"][0]["verified_name"],
        "Example Caller"
    );
    let (code, found) = run_json(vsift(&base)?.args([
        "search",
        &session,
        "--query",
        "export",
        "--speaker",
        "Example Caller",
        "--json",
    ]))?;
    assert_eq!(code, Some(0), "search --speaker: {found}");
    conforms("search-data.schema.json", &found["data"])?;
    assert_eq!(found["data"]["query"]["speaker"], "S1");
    for item in found["data"]["items"].as_array().ok_or("no items")? {
        assert_eq!(item["speaker_label"]["label"], "S1");
    }

    // The engine's own count finds two voices too.
    let (code, automatic) = run_json(
        vsift(&base)?
            .args(["transcript", "speakers", &session, "--diarizer"])
            .arg(&diarizer)
            .arg("--segmentation-model")
            .arg(&segmentation)
            .arg("--embedding-model")
            .arg(&embedding)
            .arg("--json"),
    )?;
    assert_eq!(code, Some(0), "automatic count: {automatic}");
    println!(
        "{FIXTURE}: the automatic count found {} labels",
        automatic["data"]["speakers"]["labels"]
            .as_array()
            .map_or(0, Vec::len)
    );
    Ok(())
}
