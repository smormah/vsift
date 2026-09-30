//! Public CLI contract for `frame get`, `frame neighbours`, `frame burst`,
//! `crop` and `audio` (P09, ADR 0019).
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
    AudioExtractor, CropRequest, EvidenceBudget, EvidenceCall, EvidenceControl, EvidenceExtraction,
    EvidenceMediaError, EvidenceScope, EvidenceStop, ExtractedClip, ExtractedFrame, FrameAtRequest,
    FrameExtractor, VideoStreamFacts, extract_audio, extract_crop, extract_frame_at,
};
use vsift_contract::{
    AUDIO_RANGE_REMEDIATION, BURST_RANGE_REMEDIATION, CROP_OUTSIDE_REMEDIATION,
    EVIDENCE_BUDGET_REMEDIATION, EVIDENCE_KIND_REMEDIATION, EVIDENCE_TOOLS_REMEDIATION,
    UNKNOWN_CANDIDATE_REMEDIATION, UNKNOWN_EVIDENCE_REMEDIATION, is_hidden_character,
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

/// An owned temporary folder, with the session root at `sessions` below it.
struct OwnedRoot {
    path: PathBuf,
    sessions: PathBuf,
}

impl OwnedRoot {
    fn new() -> Built<Self> {
        Self::with_sessions(Path::new("private sessions"))
    }

    /// An owned folder whose session root is `sessions` below it; the
    /// session root's parent folders are created, the root itself is not.
    fn with_sessions(sessions: &Path) -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        let sessions = path.join(sessions);
        let root = Self { path, sessions };
        if let Some(parent) = root.sessions.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(root)
    }

    fn path(&self, child: &str) -> PathBuf {
        self.path.join(child)
    }

    fn sessions(&self) -> PathBuf {
        self.sessions.clone()
    }

    fn user_base(&self) -> PathBuf {
        self.path("user")
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.path);
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
    let command = value["command"].as_str().unwrap_or_default();
    if value["error"].is_null() {
        if command.starts_with("frame.") || command == "crop" {
            validate("frame-data.schema.json", &value["data"])?;
        } else if command == "audio" {
            validate("audio-data.schema.json", &value["data"])?;
        }
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
                let record = if command == "audio" {
                    "audio-evidence.schema.json"
                } else {
                    "frame-evidence.schema.json"
                };
                validate(record, &value["record"])?;
            }
            Some("terminal") => {
                validate("terminal-event.schema.json", &value)?;
                validate("operation-response.schema.json", &value["result"])?;
                if value["result"]["error"].is_null() {
                    let data = if command == "audio" {
                        "audio-stream-data.schema.json"
                    } else {
                        "frame-stream-data.schema.json"
                    };
                    validate(data, &value["result"]["data"])?;
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
        Self::open_in(OwnedRoot::new()?)
    }

    fn open_in(root: OwnedRoot) -> Built<Self> {
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

    /// What an earlier `crop` of `parent` would have committed.
    async fn seeded_crop(
        &self,
        parent: &vsift_domain::EvidenceItem,
        (x, y, width, height): (u32, u32, u32, u32),
    ) -> Built<EvidenceExtraction> {
        let fingerprint = self.fingerprint()?;
        let known = BTreeSet::new();
        Ok(extract_crop(
            &self.call(&fingerprint, &known),
            &SeedVideo::new()?,
            CropRequest {
                parent,
                x,
                y,
                width,
                height,
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

/// The Windows extended-length prefix (ADR 0019 D2, 2026-09-29 note).
#[cfg(windows)]
const EXTENDED_LENGTH_PREFIX: &str = r"\\?\";

/// The legacy Win32 path limit in UTF-16 code units, including the NUL.
#[cfg(windows)]
const WINDOWS_MAX_PATH: usize = 260;

/// Seeds one frame in `root`'s session, answers an identical `frame get`
/// through the binary and returns the harness (which owns the folder), the
/// delivered path's text and the frame's committed bytes.
#[cfg(windows)]
async fn delivered_frame_path(root: OwnedRoot) -> Built<(Harness, String, Vec<u8>)> {
    let harness = Harness::open_in(root)?;
    harness.trust_tools()?;
    let seeded = harness.seeded_frame(1_025_000).await?;
    harness.seed(&seeded, 0)?;
    let value = json(&harness.run(&frame_get(&harness.session, "1025000", &["--json"]))?)?;
    assert_eq!(value["status"], "complete");
    assert_eq!(value["data"]["reused"], true);
    let path = value["data"]["files"][0]["path"]
        .as_str()
        .ok_or("no path")?
        .to_owned();
    let bytes = seeded.media.first().ok_or("no image")?.bytes.clone();
    Ok((harness, path, bytes))
}

/// #210: under a session root of ordinary length, a delivered Windows path
/// is the plain absolute form `C:\...` (which agent file tools accept), and
/// it names exactly the file the engine verified: canonicalising it gives
/// back the extended-length form of the same text.
#[cfg(windows)]
#[tokio::test]
async fn a_short_session_root_delivers_a_plain_windows_path() -> TestResult {
    let (_harness, path, bytes) = delivered_frame_path(OwnedRoot::new()?).await?;
    assert!(
        !path.starts_with(EXTENDED_LENGTH_PREFIX),
        "a short path kept the extended-length form"
    );
    assert!(
        path.encode_utf16().count() < WINDOWS_MAX_PATH,
        "the test's session root is too long for a plain path"
    );
    let mut drive = path.chars();
    assert!(
        drive
            .next()
            .is_some_and(|letter| letter.is_ascii_alphabetic())
            && drive.next() == Some(':')
            && drive.next() == Some('\\'),
        "a delivered path does not start with a drive"
    );
    assert!(Path::new(&path).is_absolute());
    assert_eq!(
        fs::canonicalize(&path)?,
        PathBuf::from(format!("{EXTENDED_LENGTH_PREFIX}{path}")),
        "the plain path names another file"
    );
    assert_eq!(fs::read(&path)?, bytes);
    Ok(())
}

/// #210: when the plain form would reach `MAX_PATH`, the delivered path
/// keeps the extended-length form `\\?\C:\...`, the documented fallback that
/// is exact at any length.
#[cfg(windows)]
#[tokio::test]
async fn a_session_root_beyond_max_path_keeps_the_extended_length_form() -> TestResult {
    let padding = "p".repeat(120);
    let sessions = Path::new(&padding).join(&padding).join("private sessions");
    let (harness, path, bytes) = delivered_frame_path(OwnedRoot::with_sessions(&sessions)?).await?;
    let plain = path
        .strip_prefix(EXTENDED_LENGTH_PREFIX)
        .ok_or("a long path lost the extended-length form")?;
    assert!(
        plain.encode_utf16().count() >= WINDOWS_MAX_PATH,
        "the padded path is not beyond MAX_PATH"
    );
    assert_eq!(fs::canonicalize(&path)?, PathBuf::from(&path));
    assert_eq!(fs::read(&path)?, bytes);

    // L-016 (P13 PR 2b): human output shows the exact path alone on its
    // line, followed once by the note on how to open the extended form.
    let human = harness.run(&frame_get(&harness.session, "1025000", &[]))?;
    assert_eq!(human.status.code(), Some(0));
    let text = human_text(&human)?;
    assert!(
        text.lines().any(|line| line == format!("    {path}")),
        "{text}"
    );
    assert_eq!(text.matches("extended-length form").count(), 1, "{text}");
    assert!(text.contains("Copy-Item -LiteralPath"), "{text}");
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

    // Without --json, the result is readable text (P13 PR 2b): each item
    // by its identity, its file's path alone on its line, and no JSON.
    let human = harness.run(&frame_get(&harness.session, "1025000", &[]))?;
    assert_eq!(human.status.code(), Some(0));
    let text = human_text(&human)?;
    assert!(!text.contains("\"command\""), "{text}");
    for item in items {
        let identity = item["evidence_id"].as_str().ok_or("no identity")?;
        assert!(
            text.lines().any(|line| line.starts_with(identity)),
            "{text}"
        );
    }
    let path = page["data"]["files"][0]["path"].as_str().ok_or("no path")?;
    assert!(
        text.lines().any(|line| line == format!("    {path}")),
        "the path is not alone on its line: {text}"
    );
    Ok(())
}

/// Checks a successful human run: nothing on stderr, and stdout holds no
/// control character but line breaks, no raw hidden character, no terminal
/// link and no line longer than a diagnostic (SEC-T02's terminal rules).
fn human_text(output: &Output) -> Built<String> {
    assert!(
        output.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout.clone())?;
    for character in text.chars() {
        assert!(
            character == '\n' || !character.is_control(),
            "control U+{:04X} in {text:?}",
            u32::from(character)
        );
        assert!(
            !is_hidden_character(character),
            "raw hidden U+{:04X} in {text:?}",
            u32::from(character)
        );
    }
    assert!(!text.contains("\u{1b}]8;"), "a terminal link: {text:?}");
    for line in text.lines() {
        assert!(line.len() <= 4_095, "a line of {} bytes", line.len());
    }
    Ok(text)
}

/// The session root of the SEC-T02 human rerun: a right-to-left override
/// and a zero-width space everywhere, and where the platform allows them in
/// a name an OSC-8 link, an ANSI colour, a line break and a C1 control that
/// would forge a line of their own.
fn hostile_sessions() -> PathBuf {
    #[cfg(windows)]
    let parent = "roo\u{202e}ts\u{200b}";
    #[cfg(not(windows))]
    let parent = "roo\u{202e}ts\u{200b}\u{1b}]8;;https://example.invalid\u{7}x\u{1b}[31m\nForged: line\u{85}";
    Path::new(parent).join("private sessions")
}

/// SEC-T02 over human output (P13 PR 2b; L-016, L-073): under a session
/// root holding hidden characters (and controls where a name can hold
/// them), `frame get`, `crop` and `audio` without `--json` keep every
/// delivered path on its own line, inert, flagged as not shown exactly, and
/// never let the root forge a line.
#[tokio::test]
async fn sec_t02_hostile_session_root_paths_stay_inert_in_human_output() -> TestResult {
    let harness = Harness::open_in(OwnedRoot::with_sessions(&hostile_sessions())?)?;
    harness.trust_tools()?;
    let frame = harness.seeded_frame(1_025_000).await?;
    harness.seed(&frame, 0)?;
    let parent = frame.record.items().first().ok_or("no frame")?;
    let crop = harness.seeded_crop(parent, (8, 4, 20, 10)).await?;
    harness.seed(&crop, 0)?;
    let audio = harness.seeded_audio().await?;
    harness.seed(&audio, 0)?;
    let parent = parent.id().as_str().to_owned();
    let commands: [Vec<&str>; 3] = [
        frame_get(&harness.session, "1025000", &[]),
        vec!["crop", &harness.session, &parent, "--rect", "8,4,20,10"],
        vec!["audio", &harness.session, "--from", "0", "--to", "1000000"],
    ];
    for arguments in commands {
        let context = arguments.join(" ");
        let output = harness.run(&arguments)?;
        assert_eq!(output.status.code(), Some(0), "{context}");
        let text = human_text(&output)?;
        let lines: Vec<&str> = text.lines().collect();
        let shown = lines
            .iter()
            .position(|line| line.contains("roo<U+202E>ts<U+200B>"))
            .ok_or_else(|| format!("{context}: no path line: {text}"))?;
        assert!(lines[shown].starts_with("    "), "{context}: {text}");
        assert!(
            lines[shown - 1].starts_with("  File ("),
            "{context}: {text}"
        );
        assert!(
            text.contains("is not shown exactly"),
            "{context}: no note: {text}"
        );
        assert!(
            !lines.iter().any(|line| line.starts_with("Forged")),
            "{context}: a forged line: {text}"
        );
        #[cfg(not(windows))]
        assert!(
            lines[shown].contains("\u{fffd}]8;;https://example.invalid\u{fffd}x\u{fffd}[31m\u{fffd}Forged: line\u{fffd}"),
            "{context}: {text}"
        );
    }
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

/// A committed crop and a committed clip are answered from their records,
/// with their verified files, through `--json` and `--events jsonl`.
#[tokio::test]
async fn identical_crop_and_audio_requests_are_reused() -> TestResult {
    let harness = Harness::open()?;
    harness.trust_tools()?;
    let frame = harness.seeded_frame(1_025_000).await?;
    harness.seed(&frame, 0)?;
    let parent = frame.record.items().first().ok_or("no frame")?;
    let crop = harness.seeded_crop(parent, (8, 4, 20, 10)).await?;
    harness.seed(&crop, 0)?;
    let audio = harness.seeded_audio().await?;
    harness.seed(&audio, 0)?;
    let before = harness.artifact_count()?;

    let crop_arguments = [
        "crop",
        harness.session.as_str(),
        parent.id().as_str(),
        "--rect",
        "8,4,20,10",
    ];
    let mut json_arguments = crop_arguments.to_vec();
    json_arguments.push("--json");
    let value = json(&harness.run(&json_arguments)?)?;
    assert_eq!(value["command"], "crop");
    let data = &value["data"];
    assert_eq!(data["reused"], true);
    assert_eq!(data["operation"], "crop");
    assert_eq!(data["items"][0]["kind"], "crop");
    assert_eq!(
        data["items"][0]["crop"]["parent_evidence_id"],
        parent.id().as_str()
    );
    assert_eq!(
        (
            &data["items"][0]["image"]["width"],
            &data["items"][0]["image"]["height"]
        ),
        (&Value::from(20), &Value::from(10))
    );
    let path = PathBuf::from(data["files"][0]["path"].as_str().ok_or("no path")?);
    assert_eq!(fs::read(&path)?, crop.media.first().ok_or("no crop")?.bytes);
    let mut stream_arguments = crop_arguments.to_vec();
    stream_arguments.extend(["--events", "jsonl"]);
    let lines = stream(&harness.run(&stream_arguments)?, "crop")?;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["record_type"], "frame_evidence");

    let audio_arguments = [
        "audio",
        harness.session.as_str(),
        "--from",
        "0",
        "--to",
        "1000000",
    ];
    let mut json_arguments = audio_arguments.to_vec();
    json_arguments.push("--json");
    let value = json(&harness.run(&json_arguments)?)?;
    assert_eq!(value["command"], "audio");
    assert_eq!(value["data"]["reused"], true);
    assert_eq!(value["data"]["range_clipped"], false);
    assert_eq!(value["data"]["files"][0]["media_type"], "audio/wav");
    let path = PathBuf::from(
        value["data"]["files"][0]["path"]
            .as_str()
            .ok_or("no path")?,
    );
    assert_eq!(
        fs::read(&path)?,
        audio.media.first().ok_or("no clip")?.bytes
    );
    let mut stream_arguments = audio_arguments.to_vec();
    stream_arguments.extend(["--events", "jsonl"]);
    let lines = stream(&harness.run(&stream_arguments)?, "audio")?;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["record_type"], "audio_evidence");
    assert_eq!(lines[1]["result"]["data"]["record_count"], 1);
    assert_eq!(harness.artifact_count()?, before, "a reused call wrote");
    Ok(())
}

/// Crop parents and rectangles are checked against the session's records
/// before any provider runs.
#[tokio::test]
async fn crops_outside_their_parent_or_of_a_clip_are_invalid_arguments() -> TestResult {
    let harness = Harness::open()?;
    harness.trust_tools()?;
    let frame = harness.seeded_frame(1_025_000).await?;
    harness.seed(&frame, 0)?;
    let audio = harness.seeded_audio().await?;
    harness.seed(&audio, 0)?;
    let parent = frame
        .record
        .items()
        .first()
        .ok_or("no frame")?
        .id()
        .as_str()
        .to_owned();
    let clip = audio
        .record
        .items()
        .first()
        .ok_or("no clip")?
        .id()
        .as_str()
        .to_owned();
    let before = harness.artifact_count()?;
    for (evidence, rect, summary) in [
        (parent.as_str(), "60,0,5,1", CROP_OUTSIDE_REMEDIATION),
        (parent.as_str(), "0,0,64,37", CROP_OUTSIDE_REMEDIATION),
        (clip.as_str(), "0,0,1,1", EVIDENCE_KIND_REMEDIATION),
        (
            "evd_ffffffffffffffffffffffffffffffff",
            "0,0,1,1",
            UNKNOWN_EVIDENCE_REMEDIATION,
        ),
    ] {
        let output =
            harness.run(&["crop", &harness.session, evidence, "--rect", rect, "--json"])?;
        assert_eq!(output.status.code(), Some(2), "{rect}");
        let value = json(&output)?;
        assert_eq!(value["command"], "crop");
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
        assert_eq!(remediation(&value), summary);
    }
    // The whole parent is a valid rectangle; it needs a provider, which the
    // stand-ins cannot be, so it fails later without writing anything.
    let whole = json(&harness.run(&[
        "crop",
        &harness.session,
        &parent,
        "--rect",
        "0,0,64,36",
        "--json",
    ])?)?;
    assert_ne!(whole["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(harness.artifact_count()?, before);
    Ok(())
}

/// The crop and audio grammar: a canonical `x,y,width,height` with a
/// positive size, and a range of at most 30 s checked before any tool.
#[tokio::test]
async fn the_crop_and_audio_grammar_and_ranges_are_checked_first() -> TestResult {
    let root = OwnedRoot::new()?;
    let session = "ses_0123456789abcdef";
    let evidence = "evd_0123456789abcdef";
    for rect in [
        "1,2,3",
        "1,2,3,4,5",
        "01,2,3,4",
        "0,0,0,5",
        "0,0,5,0",
        "-1,0,1,1",
        "+1,0,1,1",
        "1, 2,3,4",
        "4294967295,0,1,1",
        "a,b,c,d",
    ] {
        let output = vsift(
            &root,
            &["crop", session, evidence, "--rect", rect, "--json"],
        )?;
        assert_eq!(output.status.code(), Some(2), "{rect}");
        assert_eq!(json(&output)?["command"], "parse", "{rect}");
    }
    for arguments in [
        vec!["crop", session, evidence, "--json"],
        vec!["crop", evidence, "--rect", "0,0,1,1", "--json"],
        vec!["audio", session, "--from", "0", "--json"],
    ] {
        let output = vsift(&root, &arguments)?;
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert_eq!(json(&output)?["command"], "parse", "{arguments:?}");
    }
    let long = vsift(
        &root,
        &[
            "audio", session, "--from", "0", "--to", "30000001", "--json",
        ],
    )?;
    assert_eq!(long.status.code(), Some(2));
    let value = json(&long)?;
    assert_eq!(value["command"], "audio");
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(remediation(&value), AUDIO_RANGE_REMEDIATION);
    for (from, to) in [("5", "5"), ("10", "5")] {
        let output = vsift(
            &root,
            &["audio", session, "--from", from, "--to", to, "--json"],
        )?;
        assert_eq!(output.status.code(), Some(2), "{from}-{to}");
        assert_eq!(json(&output)?["error"]["code"], "INVALID_ARGUMENT");
    }

    // Without media tools a clip names what to install.
    let source = root.path("stand-in.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomaudio-no-tools")?;
    let opened = json(&vsift(&root, &["ingest", text(&source)?, "--json"])?)?;
    let session = opened["data"]["session_id"].as_str().ok_or("no session")?;
    let output = vsift(
        &root,
        &["audio", session, "--from", "0", "--to", "1000000", "--json"],
    )?;
    assert_eq!(output.status.code(), Some(2));
    let value = json(&output)?;
    assert_eq!(value["error"]["code"], "MISSING_CAPABILITY");
    assert_eq!(remediation(&value), EVIDENCE_TOOLS_REMEDIATION);
    Ok(())
}

/// The generations at which [`s11_warm_reuse_as_the_manifest_chain_grows`]
/// measures (#164).
const CHAIN_MEASUREMENT_GENERATIONS: [u64; 4] = [1, 64, 256, 1_024];
/// Warm calls timed at each of those generations.
const CHAIN_MEASUREMENT_SAMPLES: usize = 20;

/// The 95th percentile of `samples`, in milliseconds (nearest rank).
fn p95_ms(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    let rank = (samples.len() * 95).div_ceil(100).max(1);
    samples.get(rank - 1).copied().unwrap_or(f64::NAN)
}

/// S-11 and #164, opt-in and recorded (not gated): the p95 of a warm, reused
/// `frame get` through the binary as the session's manifest chain grows to
/// 1,024 generations. Renewals add one generation each, as in the P09
/// performance record; the evidence is seeded, so no provider runs.
///
/// ```console
/// cargo test -p vsift-cli --release --locked --test evidence_cli_contract -- --ignored --nocapture s11_warm_reuse
/// ```
#[tokio::test]
#[ignore = "opt-in #164 measurement; run with --release --ignored --nocapture"]
async fn s11_warm_reuse_as_the_manifest_chain_grows() -> TestResult {
    measure_warm_reuse(0, "S11_CHAIN").await
}

/// The same measurement with the session's evidence budget full (ADR 0020
/// D-2): 384 evidence artifacts, so every generation's manifest is about
/// 75 KiB instead of 1 KiB; the slope must stay about zero.
///
/// ```console
/// cargo test -p vsift-cli --release --locked --test evidence_cli_contract -- --ignored --nocapture s11_warm_reuse_with
/// ```
#[tokio::test]
#[ignore = "opt-in D-2 measurement; run with --release --ignored --nocapture"]
async fn s11_warm_reuse_with_a_full_evidence_budget() -> TestResult {
    measure_warm_reuse(MAX_EVIDENCE_ARTIFACTS - 3, "S11_FULL").await
}

/// Seeds one frame and `fillers` more evidence files, then times warm reused
/// `frame get` calls as renewals grow the chain, printing rows labelled
/// `label`.
async fn measure_warm_reuse(fillers: usize, label: &str) -> TestResult {
    let harness = Harness::open()?;
    harness.trust_tools()?;
    let seeded = harness.seeded_frame(1_025_000).await?;
    harness.seed(&seeded, fillers)?;
    let store = harness.store()?;
    let mut rows = Vec::new();
    for target in CHAIN_MEASUREMENT_GENERATIONS {
        let mut generation = store.session_status(&harness.session_id)?.generation();
        while generation.value() < target {
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let operation = OperationId::parse(format!(
                "op_c4a1{:028x}",
                generation.value().saturating_add(1)
            ))?;
            generation = store.renew_session(&harness.session_id, &operation, generation, now)?;
        }
        let mut samples = Vec::with_capacity(CHAIN_MEASUREMENT_SAMPLES);
        for _ in 0..CHAIN_MEASUREMENT_SAMPLES {
            let started = std::time::Instant::now();
            let output = harness.run(&frame_get(&harness.session, "1025000", &["--json"]))?;
            samples.push(started.elapsed().as_secs_f64() * 1_000.0);
            assert_eq!(output.status.code(), Some(0), "a warm call failed");
            assert_eq!(
                json(&output)?["data"]["reused"],
                true,
                "a warm call was not reused"
            );
        }
        let p95 = p95_ms(&mut samples);
        println!(
            "{label} generation={} p95_ms={p95:.1} min_ms={:.1}",
            generation.value(),
            samples.first().copied().unwrap_or(f64::NAN)
        );
        rows.push((generation.value(), p95));
    }
    if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
        let generations = last.0.saturating_sub(first.0).max(1);
        #[allow(
            clippy::cast_precision_loss,
            reason = "a generation count of at most 4,096 is exact in f64"
        )]
        let slope = (last.1 - first.1) / generations as f64;
        println!(
            "{label} slope_ms_per_generation={slope:.3} build={}",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
        );
    }
    Ok(())
}
