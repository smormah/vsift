//! Public CLI contract for `frame get`, `frame neighbours` and `frame burst`
//! (P09, ADR 0019).
//!
//! Every journey runs everywhere without `FFmpeg`. The configured "tools"
//! are plain files that cannot run, so a call that reached a provider fails;
//! a call that succeeds therefore proves it ran none. Evidence is seeded into
//! the session through the store exactly as an earlier real call would have
//! committed it (the application's extraction use cases over a stand-in
//! 20 fps stream, keyed with the provider fingerprint the binary derives for
//! the stand-ins), and the media-tool preflight pass the binary would have
//! recorded after verifying real tools is written to its private state. The
//! binary then answers identical requests from the committed records, with
//! the verified absolute paths of their files. The opt-in `p09_evidence_e2e`
//! checkpoint runs the commands against real `FFmpeg` instead.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fmt::Write as _,
    fs,
    future::{Future, ready},
    io,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    AudioExtractor, EvidenceBudget, EvidenceCall, EvidenceControl, EvidenceExtraction,
    EvidenceMediaError, EvidenceScope, EvidenceStop, ExtractedClip, ExtractedFrame, FrameAtRequest,
    FrameExtractor, VideoStreamFacts, extract_audio, extract_frame_at,
};
use vsift_contract::{
    BURST_RANGE_REMEDIATION, EVIDENCE_BUDGET_REMEDIATION, EVIDENCE_KIND_REMEDIATION,
    EVIDENCE_TOOLS_REMEDIATION, UNKNOWN_CANDIDATE_REMEDIATION, UNKNOWN_EVIDENCE_REMEDIATION,
};
use vsift_domain::{
    AudioRange, CropRect, EvidenceMediaKind, EvidenceProfile, FrameDimensions, FrameListing,
    FrameSelection, FrameTolerance, ListedFrame, ListingTail, MediaTime, OperationId, SessionId,
    Sha256Hex, SourceCheck, SourceId, TimeBase, TimeRange,
};
use vsift_infrastructure::{
    EvidenceMediaFile, FilesystemSessionStore, HostIsolation, MAX_EVIDENCE_ARTIFACTS,
    MediaProviderConformance, MediaToolVerificationAuthority, TrustedExecutable,
    encode_evidence_record, media_tool_fingerprint, reviewed_compatibility_policy,
    wav_from_pcm_s16le_mono,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-frame-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const FRAME_MICROS: u64 = 50_000;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Built<Self> {
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

/// The per-user `VSift` folder the binary derives from the test's `HOME`,
/// `LOCALAPPDATA` or `XDG_CONFIG_HOME`: macOS keeps it under
/// `Library/Application Support`.
fn config_root(base: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        base.join("Library/Application Support/vsift")
    }
    #[cfg(not(target_os = "macos"))]
    {
        base.join("vsift")
    }
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_str(&fs::read_to_string(
            repository("schemas/v1").join(name),
        )?)?)
    }
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema: Value = serde_json::from_str(&fs::read_to_string(
        repository("schemas/v1").join(schema_path),
    )?)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// Runs `vsift` with an isolated per-user base, the root's session store and
/// an empty `PATH`, so no media tool is found unless one is configured.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Built<Output> {
    let base = root.user_base();
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .output()?)
}

/// Parses a `--json` result, validating the envelope and a successful
/// result's data and items.
fn json(output: &Output) -> Built<Value> {
    assert!(output.stderr.is_empty(), "a JSON command wrote to stderr");
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    let evidence = value["command"]
        .as_str()
        .is_some_and(|command| command.starts_with("frame.") || command == "crop");
    if evidence && value["error"].is_null() {
        validate("frame-data.schema.json", &value["data"])?;
    }
    Ok(value)
}

/// Splits `--events jsonl` stdout into validated lines.
fn stream(output: &Output, command: &str) -> Built<Vec<Value>> {
    assert!(
        output.stderr.is_empty(),
        "a JSON Lines command wrote to stderr"
    );
    let text = std::str::from_utf8(&output.stdout)?;
    let body = text
        .strip_suffix('\n')
        .ok_or("the stream does not end with a newline")?;
    let mut lines = Vec::new();
    for (sequence, line) in body.split('\n').enumerate() {
        let value: Value = serde_json::from_str(line)?;
        assert_eq!(value["sequence"], sequence, "sequence is not contiguous");
        assert_eq!(value["command"], command);
        match value["event"].as_str() {
            Some("evidence") => {
                validate("evidence-event.schema.json", &value)?;
                validate("frame-evidence.schema.json", &value["record"])?;
            }
            Some("terminal") => {
                validate("terminal-event.schema.json", &value)?;
                validate("operation-response.schema.json", &value["result"])?;
                if value["result"]["error"].is_null() {
                    validate("frame-stream-data.schema.json", &value["result"]["data"])?;
                }
            }
            _ => return Err("a stream line has no published event kind".into()),
        }
        lines.push(value);
    }
    let last = lines.last().ok_or("empty stream")?;
    assert_eq!(last["event"], "terminal", "the last line is not terminal");
    Ok(lines)
}

fn text(path: &Path) -> Built<&str> {
    Ok(path.to_str().ok_or("non-UTF-8 test path")?)
}

fn time_of(pts: i64) -> MediaTime {
    MediaTime::from_micros(u64::try_from(pts).unwrap_or(0) * FRAME_MICROS)
}

/// The start of an 8-bit RGB PNG of `width` x `height`, then `marker`: what
/// `bundle validate` and the store check of an image.
fn stand_in_png(width: u32, height: u32, marker: &str) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13_u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    png.extend_from_slice(marker.as_bytes());
    png
}

/// A 20 fps, 6 s, 64x36 stream standing in for what `FFmpeg` decoded in
/// an earlier call; only used to build the records that call committed.
struct SeedVideo(VideoStreamFacts);

impl SeedVideo {
    fn new() -> Built<Self> {
        Ok(Self(VideoStreamFacts {
            stream_index: 0,
            time_base: TimeBase::new(1, 20)?,
            displayed: FrameDimensions::new(64, 36)?,
            duration: MediaTime::from_micros(6_000_000),
        }))
    }
}

impl FrameExtractor for SeedVideo {
    fn stream(&self) -> VideoStreamFacts {
        self.0
    }

    fn max_frames_per_run(&self) -> usize {
        8
    }

    fn list_frames(
        &self,
        requested: TimeRange,
    ) -> impl Future<Output = Result<FrameListing, EvidenceMediaError>> + Send {
        let listing = (|| {
            let (end, tail) = if requested.end() >= self.0.duration {
                (self.0.duration, ListingTail::EndOfStream)
            } else {
                (requested.end(), ListingTail::MoreMayFollow)
            };
            let covered =
                TimeRange::new(requested.start(), end).map_err(|_| EvidenceMediaError::Invalid)?;
            let frames = (0..120_i64)
                .map(|pts| ListedFrame {
                    pts,
                    time: time_of(pts),
                })
                .filter(|frame| frame.time >= covered.start() && frame.time < covered.end())
                .collect();
            FrameListing::new(covered, frames, tail).map_err(|_| EvidenceMediaError::Invalid)
        })();
        ready(listing)
    }

    fn frames(
        &self,
        pts: &[i64],
    ) -> impl Future<Output = Result<Vec<ExtractedFrame>, EvidenceMediaError>> + Send {
        ready(Ok(pts
            .iter()
            .map(|value| ExtractedFrame {
                pts: *value,
                time: time_of(*value),
                png: stand_in_png(64, 36, &format!("frame {value}")),
            })
            .collect()))
    }

    fn crop(
        &self,
        pts: i64,
        rect: CropRect,
    ) -> impl Future<Output = Result<ExtractedFrame, EvidenceMediaError>> + Send {
        ready(Ok(ExtractedFrame {
            pts,
            time: time_of(pts),
            png: stand_in_png(rect.width(), rect.height(), "crop"),
        }))
    }
}

struct SeedAudio;

impl AudioExtractor for SeedAudio {
    fn stream_index(&self) -> u32 {
        1
    }

    fn duration(&self) -> MediaTime {
        MediaTime::from_micros(6_000_000)
    }

    fn clip(
        &self,
        range: TimeRange,
    ) -> impl Future<Output = Result<ExtractedClip, EvidenceMediaError>> + Send {
        ready(
            wav_from_pcm_s16le_mono(&[0, 0, 1, 0])
                .map(|wav| ExtractedClip {
                    actual_start: range.start(),
                    wav,
                })
                .map_err(|_| EvidenceMediaError::Invalid),
        )
    }
}

struct Never;

impl EvidenceControl for Never {
    fn stop(&self) -> Option<EvidenceStop> {
        None
    }
}

/// One session of a stand-in source, opened through the binary, with
/// stand-in tools registered and trusted as the binary would trust real ones.
struct Harness {
    root: OwnedRoot,
    session: String,
    session_id: SessionId,
    source: SourceId,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl Harness {
    fn open() -> Built<Self> {
        let root = OwnedRoot::new()?;
        let source = root.path("stand-in.mp4");
        fs::write(&source, b"\0\0\0\x18ftypisomframe-cli-contract")?;
        let opened = json(&vsift(&root, &["ingest", text(&source)?, "--json"])?)?;
        let session = opened["data"]["session_id"]
            .as_str()
            .ok_or("no session")?
            .to_owned();
        let source_id = SourceId::parse(opened["data"]["source_id"].as_str().ok_or("no source")?)?;
        let ffmpeg = root.path("ffmpeg-stand-in.exe");
        let ffprobe = root.path("ffprobe-stand-in.exe");
        fs::write(&ffmpeg, b"not a program: ffmpeg")?;
        fs::write(&ffprobe, b"not a program: ffprobe")?;
        for (dependency, path) in [("ffmpeg", &ffmpeg), ("ffprobe", &ffprobe)] {
            let output = vsift(
                &root,
                &[
                    "setup",
                    "configure",
                    dependency,
                    "--executable",
                    text(path)?,
                    "--json",
                ],
            )?;
            assert_eq!(output.status.code(), Some(0), "setup configure failed");
        }
        Ok(Self {
            session_id: SessionId::parse(&session)?,
            root,
            session,
            source: source_id,
            ffmpeg,
            ffprobe,
        })
    }

    fn run(&self, arguments: &[&str]) -> Built<Output> {
        vsift(&self.root, arguments)
    }

    fn store(&self) -> Built<FilesystemSessionStore> {
        Ok(FilesystemSessionStore::open_existing(self.root.sessions())?)
    }

    /// The fingerprint the binary derives for the configured stand-ins: the
    /// reviewed fixture is its verifying authority.
    fn fingerprint(&self) -> Built<Sha256Hex> {
        let tools = MediaProviderConformance::r0(
            TrustedExecutable::explicit(&self.ffmpeg)?,
            TrustedExecutable::explicit(&self.ffprobe)?,
        );
        let fingerprint = media_tool_fingerprint(
            &tools,
            HostIsolation::ProcessOnly,
            &reviewed_compatibility_policy()?,
            MediaToolVerificationAuthority::ReviewedFixture,
        )
        .ok_or("no fingerprint")?;
        let mut hex = String::new();
        for byte in fingerprint.digest() {
            write!(hex, "{byte:02x}")?;
        }
        Ok(Sha256Hex::parse(hex)?)
    }

    /// Records the preflight pass the binary would have recorded after
    /// verifying real tools. The first evidence call creates the private
    /// state directory (and fails: the stand-ins cannot run).
    fn trust_tools(&self) -> TestResult {
        let first = json(&self.run(&["frame", "get", &self.session, "--at", "0", "--json"])?)?;
        assert_eq!(first["error"]["code"], "MISSING_CAPABILITY");
        let state = config_root(&self.root.user_base()).join("media-tool-verification");
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        fs::write(
            state.join("verified-v1.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 1,
                "entries": [{
                    "fingerprint": self.fingerprint()?.as_str(),
                    "verified_at_unix_seconds": now,
                }],
            }))?,
        )?;
        Ok(())
    }

    fn call<'a>(
        &'a self,
        fingerprint: &'a Sha256Hex,
        known: &'a BTreeSet<String>,
    ) -> EvidenceCall<'a, Never> {
        EvidenceCall {
            scope: EvidenceScope {
                session_id: &self.session_id,
                source_id: &self.source,
                profile: EvidenceProfile::P09R0,
                tool_fingerprint: Some(fingerprint),
            },
            source_check: SourceCheck::FullHash,
            budget: EvidenceBudget::per_call(MAX_EVIDENCE_ARTIFACTS, u64::MAX),
            known_media: known,
            control: &Never,
        }
    }

    /// What an earlier `frame get --at <at>` would have committed.
    async fn seeded_frame(&self, at: u64) -> Built<EvidenceExtraction> {
        let fingerprint = self.fingerprint()?;
        let known = BTreeSet::new();
        Ok(extract_frame_at(
            &self.call(&fingerprint, &known),
            &SeedVideo::new()?,
            FrameAtRequest {
                at: MediaTime::from_micros(at),
                selection: FrameSelection::AtOrAfter,
                tolerance: FrameTolerance::DEFAULT,
                candidate: None,
            },
        )
        .await?)
    }

    async fn seeded_audio(&self) -> Built<EvidenceExtraction> {
        let fingerprint = self.fingerprint()?;
        let known = BTreeSet::new();
        Ok(extract_audio(
            &self.call(&fingerprint, &known),
            &SeedAudio,
            AudioRange::new(TimeRange::new(
                MediaTime::from_micros(0),
                MediaTime::from_micros(1_000_000),
            )?)?,
        )
        .await?)
    }

    /// Commits an extraction's files and record, plus `fillers` images.
    fn seed(&self, extraction: &EvidenceExtraction, fillers: usize) -> TestResult {
        let store = self.store()?;
        let status = store.session_status(&self.session_id)?;
        let filler_bytes: Vec<Vec<u8>> = (0..fillers)
            .map(|number| stand_in_png(64, 36, &format!("filler {number}")))
            .collect();
        let mut media: Vec<EvidenceMediaFile<'_>> = extraction
            .media
            .iter()
            .map(|file| EvidenceMediaFile {
                kind: file.kind,
                bytes: &file.bytes,
            })
            .collect();
        media.extend(filler_bytes.iter().map(|bytes| EvidenceMediaFile {
            kind: EvidenceMediaKind::FramePng,
            bytes,
        }));
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        store.publish_evidence(
            &self.session_id,
            &OperationId::parse(format!("op_{:032x}", status.generation().value() + 1))?,
            status.generation(),
            &media,
            &encode_evidence_record(&extraction.record)?,
            None,
            now,
        )?;
        Ok(())
    }

    fn artifact_count(&self) -> Built<u64> {
        let value = json(&self.run(&["session", "status", &self.session, "--json"])?)?;
        value["data"]["artifact_count"]
            .as_u64()
            .ok_or_else(|| "no artifact count".into())
    }
}

fn frame_get<'a>(session: &'a str, at: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut arguments = vec!["frame", "get", session, "--at", at];
    arguments.extend_from_slice(extra);
    arguments
}

fn remediation(value: &Value) -> &Value {
    &value["error"]["remediation"][0]["summary"]
}

/// V-08 through the binary: an identical request is answered from its
/// committed record, with the verified absolute path of its file, without
/// running any provider (the stand-in tools cannot run) or writing anything.
#[tokio::test]
async fn an_identical_frame_request_is_reused_with_its_verified_file() -> TestResult {
    let harness = Harness::open()?;
    harness.trust_tools()?;
    let seeded = harness.seeded_frame(1_025_000).await?;
    harness.seed(&seeded, 0)?;
    let before = harness.artifact_count()?;

    let output = harness.run(&frame_get(&harness.session, "1025000", &["--json"]))?;
    assert_eq!(output.status.code(), Some(0));
    let value = json(&output)?;
    assert_eq!(value["command"], "frame.get");
    assert_eq!(value["status"], "complete");
    assert_eq!(value["lifecycle"]["mode"], "ephemeral");
    let data = &value["data"];
    assert_eq!(data["reused"], true);
    assert_eq!(data["request_key"], seeded.record.request_key().as_str());
    assert_eq!(data["selections"][0]["actual_us"], 1_050_000);
    assert_eq!(data["selections"][0]["delta_us"], 25_000);
    let item = seeded.record.items().first().ok_or("no item")?;
    assert_eq!(data["items"][0]["evidence_id"], item.id().as_str());
    let path = PathBuf::from(data["files"][0]["path"].as_str().ok_or("no path")?);
    assert!(path.is_absolute(), "a delivered path is not absolute");
    assert_eq!(
        fs::read(&path)?,
        seeded.media.first().ok_or("no image")?.bytes
    );
    assert_eq!(harness.artifact_count()?, before, "a reused call wrote");

    // Another time needs a provider, which the stand-ins cannot be.
    let other = json(&harness.run(&frame_get(&harness.session, "2000000", &["--json"]))?)?;
    assert!(other["error"]["code"].is_string());
    assert_eq!(harness.artifact_count()?, before);
    Ok(())
}

/// `--events jsonl` writes each item as a `frame_evidence` event keyed by
/// its identity, then one terminal event with the rest and the count.
#[tokio::test]
async fn a_frame_stream_is_the_items_then_one_terminal_event() -> TestResult {
    let harness = Harness::open()?;
    harness.trust_tools()?;
    let seeded = harness.seeded_frame(1_025_000).await?;
    harness.seed(&seeded, 0)?;
    let page = json(&harness.run(&frame_get(&harness.session, "1025000", &["--json"]))?)?;
    let output = harness.run(&frame_get(
        &harness.session,
        "1025000",
        &["--events", "jsonl"],
    ))?;
    assert_eq!(output.status.code(), Some(0));
    let lines = stream(&output, "frame.get")?;
    let (terminal, records) = lines.split_last().ok_or("empty")?;
    let items = page["data"]["items"].as_array().ok_or("items")?;
    assert_eq!(records.len(), items.len());
    for (record, item) in records.iter().zip(items) {
        assert_eq!(record["record_type"], "frame_evidence");
        assert_eq!(record["key"], item["evidence_id"]);
        assert_eq!(&record["record"], item);
    }
    let data = &terminal["result"]["data"];
    assert_eq!(data["record_count"], items.len());
    assert_eq!(data["files"], page["data"]["files"]);
    assert_eq!(data["reused"], true);
    assert_eq!(terminal["result"]["status"], "complete");

    // Without --json, the result is the same document, indented.
    let human = harness.run(&frame_get(&harness.session, "1025000", &[]))?;
    assert_eq!(human.status.code(), Some(0));
    let text = String::from_utf8(human.stdout)?;
    assert!(text.contains("\n  \"command\": \"frame.get\""));
    let value: Value = serde_json::from_str(&text)?;
    validate("frame-data.schema.json", &value["data"])?;
    Ok(())
}

/// ADR 0019 D4: a session without room for a record and one file fails
/// with `RESOURCE_LIMIT` and the remediation to retain it and open a new
/// one, before any provider runs; what it holds is still answered.
#[tokio::test]
async fn an_exhausted_evidence_budget_names_its_remediation() -> TestResult {
    let harness = Harness::open()?;
    harness.trust_tools()?;
    let seeded = harness.seeded_frame(1_025_000).await?;
    harness.seed(&seeded, MAX_EVIDENCE_ARTIFACTS - 3)?;
    let output = harness.run(&frame_get(&harness.session, "3000000", &["--json"]))?;
    assert_eq!(output.status.code(), Some(5));
    let value = json(&output)?;
    assert_eq!(value["error"]["code"], "RESOURCE_LIMIT");
    assert_eq!(remediation(&value), EVIDENCE_BUDGET_REMEDIATION);
    let reused = json(&harness.run(&frame_get(&harness.session, "1025000", &["--json"]))?)?;
    assert_eq!(reused["data"]["reused"], true);
    Ok(())
}

/// Parents are looked up in the session's records before any provider
/// runs: an unknown identity and an audio clip are typed invalid arguments
/// with their remediation, and an unknown candidate needs no tool at all.
#[tokio::test]
async fn unknown_or_unsuitable_parents_and_candidates_are_invalid_arguments() -> TestResult {
    let harness = Harness::open()?;
    let candidate = json(&harness.run(&[
        "frame",
        "get",
        &harness.session,
        "--candidate",
        "vcd_0123456789abcdef",
        "--json",
    ])?)?;
    assert_eq!(candidate["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(remediation(&candidate), UNKNOWN_CANDIDATE_REMEDIATION);

    harness.trust_tools()?;
    let audio = harness.seeded_audio().await?;
    harness.seed(&audio, 0)?;
    let clip = audio
        .record
        .items()
        .first()
        .ok_or("no clip")?
        .id()
        .as_str()
        .to_owned();
    for (evidence, code, summary) in [
        (
            "evd_ffffffffffffffffffffffffffffffff",
            "INVALID_ARGUMENT",
            UNKNOWN_EVIDENCE_REMEDIATION,
        ),
        (clip.as_str(), "INVALID_ARGUMENT", EVIDENCE_KIND_REMEDIATION),
    ] {
        let output = harness.run(&["frame", "neighbours", &harness.session, evidence, "--json"])?;
        assert_eq!(output.status.code(), Some(2), "{evidence}");
        let value = json(&output)?;
        assert_eq!(value["command"], "frame.neighbours");
        assert_eq!(value["error"]["code"], code);
        assert_eq!(remediation(&value), summary);
    }
    Ok(())
}

/// Without media tools an evidence call is a missing capability whose
/// remediation names what to install, and nothing is written.
#[tokio::test]
async fn frames_without_media_tools_name_what_to_install() -> TestResult {
    let root = OwnedRoot::new()?;
    let source = root.path("stand-in.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomno-tools")?;
    let opened = json(&vsift(&root, &["ingest", text(&source)?, "--json"])?)?;
    let session = opened["data"]["session_id"].as_str().ok_or("no session")?;
    for arguments in [
        frame_get(session, "0", &["--json"]),
        vec![
            "frame", "burst", session, "--from", "0", "--to", "1000000", "--json",
        ],
    ] {
        let output = vsift(&root, &arguments)?;
        assert_eq!(output.status.code(), Some(2));
        let value = json(&output)?;
        assert_eq!(value["error"]["code"], "MISSING_CAPABILITY");
        assert_eq!(remediation(&value), EVIDENCE_TOOLS_REMEDIATION);
    }
    let status = json(&vsift(&root, &["session", "status", session, "--json"])?)?;
    assert_eq!(status["data"]["artifact_count"], 0);
    Ok(())
}

/// The grammar enforces the D6 shapes and limits before anything runs:
/// exactly one of `--at` and `--candidate`, policy and tolerance only with
/// a time, counts 1..=20 and frames 1..=100, a tolerance of at most 10 s.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "One table of every grammar rejection reads best in one place"
)]
async fn the_grammar_rejects_what_d6_does_not_define() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = "ses_0123456789abcdef";
    let evidence = "evd_0123456789abcdef";
    let rejected: [&[&str]; 14] = [
        &["frame", "get", session],
        &[
            "frame",
            "get",
            session,
            "--at",
            "5",
            "--candidate",
            "vcd_0123456789abcdef",
        ],
        &[
            "frame",
            "get",
            session,
            "--candidate",
            "vcd_0123456789abcdef",
            "--select",
            "displayed-at",
        ],
        &[
            "frame",
            "get",
            session,
            "--candidate",
            "vcd_0123456789abcdef",
            "--tolerance-us",
            "0",
        ],
        &[
            "frame",
            "get",
            session,
            "--at",
            "5",
            "--tolerance-us",
            "10000001",
        ],
        &["frame", "get", session, "--at", "5", "--select", "nearest"],
        &["frame", "get", session, "--at", "-5"],
        &[
            "frame",
            "get",
            session,
            "--candidate",
            "evd_0123456789abcdef",
        ],
        &["frame", "neighbours", session, evidence, "--count", "0"],
        &["frame", "neighbours", session, evidence, "--count", "21"],
        &["frame", "neighbours", evidence],
        &[
            "frame",
            "burst",
            session,
            "--from",
            "0",
            "--to",
            "1",
            "--max-frames",
            "0",
        ],
        &[
            "frame",
            "burst",
            session,
            "--from",
            "0",
            "--to",
            "1",
            "--max-frames",
            "101",
        ],
        &["frame", "burst", session, "--from", "0"],
    ];
    for arguments in rejected {
        let mut arguments = arguments.to_vec();
        arguments.push("--json");
        let output = vsift(&root, &arguments)?;
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        let value = json(&output)?;
        assert_eq!(value["command"], "parse", "{arguments:?}");
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT", "{arguments:?}");
    }
    for arguments in [
        frame_get(
            session,
            "5",
            &["--tolerance-us", "10000000", "--select", "displayed-at"],
        ),
        vec!["frame", "neighbours", session, evidence, "--count", "20"],
        vec![
            "frame",
            "burst",
            session,
            "--from",
            "0",
            "--to",
            "60000000",
            "--max-frames",
            "100",
        ],
    ] {
        let mut arguments = arguments;
        arguments.push("--json");
        let value = json(&vsift(&root, &arguments)?)?;
        assert_ne!(value["command"], "parse", "{arguments:?}");
    }
    Ok(())
}

/// Ranges are checked before any tool: over 60 s is `INVALID_ARGUMENT` with
/// the remediation to find moments with `candidates`; an empty or reversed
/// range is invalid too.
#[tokio::test]
async fn burst_ranges_are_checked_before_any_tool() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = "ses_0123456789abcdef";
    let long = vsift(
        &root,
        &[
            "frame", "burst", session, "--from", "0", "--to", "60000001", "--json",
        ],
    )?;
    assert_eq!(long.status.code(), Some(2));
    let value = json(&long)?;
    assert_eq!(value["command"], "frame.burst");
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(remediation(&value), BURST_RANGE_REMEDIATION);
    for (from, to) in [("5", "5"), ("10", "5")] {
        let output = vsift(
            &root,
            &[
                "frame", "burst", session, "--from", from, "--to", to, "--json",
            ],
        )?;
        assert_eq!(output.status.code(), Some(2), "{from}-{to}");
        assert_eq!(json(&output)?["error"]["code"], "INVALID_ARGUMENT");
    }
    // A failure in JSON Lines mode is the usual single terminal event.
    let events = vsift(
        &root,
        &[
            "frame", "burst", session, "--from", "0", "--to", "60000001", "--events", "jsonl",
        ],
    )?;
    let lines = stream(&events, "frame.burst")?;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["result"]["error"]["code"], "INVALID_ARGUMENT");
    Ok(())
}
