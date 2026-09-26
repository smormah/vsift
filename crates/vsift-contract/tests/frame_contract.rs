//! Conformance of the frame and crop evidence contract (P09, ADR 0019): the
//! `--json` results of `frame get`, `frame neighbours` and `frame burst`, the
//! `--events jsonl` stream of `frame_evidence` records ended by one terminal
//! event, and the `partial` status of a call that stopped on a budget, against
//! the published v1 schemas and the frozen examples `examples/frame-get.json`,
//! `examples/frame-get.events.jsonl`, `examples/frame-neighbours.json` and
//! `examples/frame-burst.partial.json`.
//!
//! The records are built by the application's own extraction use cases over a
//! fake extractor that stands for the F01 fixture: 120 frames at 20 fps, one
//! every 50 ms, in F01's real 1/10240 time base, 1280x720, of the committed
//! F01 source. The images are short stand-in PNG byte strings, so their
//! digests are synthetic. Delivered paths are absolute on the machine that
//! runs a call; the examples name them under the neutral placeholder root
//! `/vsift-session-root`, with the real layout below it
//! (`sessions/<session>/artifacts/artifact-<sha256>.png`), and never a real
//! user's folder.
//!
//! After an intended contract change, run with
//! `VSIFT_REGENERATE_CONTRACT_EXAMPLES=1` to rewrite the examples, and review
//! the diff.

use std::{
    collections::BTreeSet,
    env,
    future::{Future, ready},
    io,
    path::PathBuf,
};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    EvidenceBudget, EvidenceCall, EvidenceControl, EvidenceExtraction, EvidenceMediaError,
    EvidenceScope, EvidenceStop, ExtractedFrame, FrameAtRequest, FrameExtractor, VideoStreamFacts,
    extract_burst, extract_frame_at, extract_neighbours,
};
use vsift_contract::{
    DeliveredEvidenceFile, EvidencePresentation, EvidencePresentationError, EvidenceStream,
    FrameEvidenceStream, LifecycleResponse, frame_response, partial_evidence_warning,
};
use vsift_domain::{
    BurstCount, BurstRange, CropRect, EvidenceProfile, EvidenceRecord, FrameDimensions,
    FrameListing, FrameSelection, FrameTolerance, ListedFrame, ListingTail, MediaTime,
    NeighbourCount, PartialReason, SessionId, Sha256Hex, SourceCheck, SourceId, TimeBase,
    TimeRange,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
/// The committed F01 fixture's SHA-256.
const F01_SHA256: &str = "65cec002d7dd8747e8ceb76f25270d35f38bfe354292e3c07bfa6169e2445070";
const FINGERPRINT: &str = "5f2d8f0c0e7b4f3a9a1c6d2e8b7a6f5e4d3c2b1a09f8e7d6c5b4a39281706f5e";
const EXPIRES_AT: &str = "2026-09-27T00:00:00Z";
/// The placeholder session root the examples' paths are written under.
const PLACEHOLDER_ROOT: &str = "/vsift-session-root";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const GET_EXAMPLE: &str = "examples/frame-get.json";
const GET_STREAM_EXAMPLE: &str = "examples/frame-get.events.jsonl";
const NEIGHBOURS_EXAMPLE: &str = "examples/frame-neighbours.json";
const BURST_EXAMPLE: &str = "examples/frame-burst.partial.json";
/// F01: 20 fps in a 1/10240 time base, so frame `n` has timestamp `512 n`.
const TICKS_PER_FRAME: i64 = 512;
const FRAME_MICROS: u64 = 50_000;
const FRAMES: i64 = 120;

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn read(relative_path: &str) -> Built<String> {
    Ok(std::fs::read_to_string(schema_root().join(relative_path))?)
}

fn load(relative_path: &str) -> Built<Value> {
    Ok(serde_json::from_str(&read(relative_path)?)?)
}

fn regenerating() -> bool {
    env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|value| value == "1")
}

fn regenerate(relative_path: &str, text: &str) -> TestResult {
    if regenerating() {
        std::fs::write(schema_root().join(relative_path), text)?;
    }
    Ok(())
}

/// Resolves sibling `$ref`s by their published identifier to the local copy.
struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_str(&std::fs::read_to_string(
            schema_root().join(name),
        )?)?)
    }
}

fn is_valid(schema_path: &str, instance: &Value) -> Built<bool> {
    Ok(jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&load(schema_path)?)?
        .is_valid(instance))
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&load(schema_path)?)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// Validates one frame stream line against the schemas its `event` selects.
fn validate_line(line: &Value) -> TestResult {
    match line["event"].as_str() {
        Some("evidence") => {
            validate("evidence-event.schema.json", line)?;
            validate("frame-evidence.schema.json", &line["record"])
        }
        Some("terminal") => {
            validate("terminal-event.schema.json", line)?;
            validate("operation-response.schema.json", &line["result"])?;
            validate("frame-stream-data.schema.json", &line["result"]["data"])
        }
        _ => Err("a stream line has no published event kind".into()),
    }
}

fn micros(value: u64) -> MediaTime {
    MediaTime::from_micros(value)
}

fn time_of(pts: i64) -> MediaTime {
    micros(u64::try_from(pts / TICKS_PER_FRAME).unwrap_or(0) * FRAME_MICROS)
}

/// A stand-in PNG: the signature and an `IHDR` of the image's size, then a
/// marker that makes each image's bytes distinct.
fn stand_in_png(width: u32, height: u32, marker: &str) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13_u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0]);
    png.extend_from_slice(marker.as_bytes());
    png
}

/// The F01 stand-in stream.
struct F01(VideoStreamFacts);

impl F01 {
    fn new() -> Built<Self> {
        Ok(Self(VideoStreamFacts {
            stream_index: 0,
            time_base: TimeBase::new(1, 10_240)?,
            displayed: FrameDimensions::new(1_280, 720)?,
            duration: micros(6_000_000),
        }))
    }

    fn listing(&self, requested: TimeRange) -> Result<FrameListing, EvidenceMediaError> {
        let (end, tail) = if requested.end() >= self.0.duration {
            (self.0.duration, ListingTail::EndOfStream)
        } else {
            (requested.end(), ListingTail::MoreMayFollow)
        };
        let covered =
            TimeRange::new(requested.start(), end).map_err(|_| EvidenceMediaError::Invalid)?;
        let frames = (0..FRAMES)
            .map(|frame| ListedFrame {
                pts: frame * TICKS_PER_FRAME,
                time: time_of(frame * TICKS_PER_FRAME),
            })
            .filter(|frame| frame.time >= covered.start() && frame.time < covered.end())
            .collect();
        FrameListing::new(covered, frames, tail).map_err(|_| EvidenceMediaError::Invalid)
    }
}

impl FrameExtractor for F01 {
    fn stream(&self) -> VideoStreamFacts {
        self.0
    }

    fn max_frames_per_run(&self) -> usize {
        8
    }

    fn list_frames(
        &self,
        range: TimeRange,
    ) -> impl Future<Output = Result<FrameListing, EvidenceMediaError>> + Send {
        ready(self.listing(range))
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
                png: stand_in_png(1_280, 720, &format!("F01 frame pts {value}")),
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
            png: stand_in_png(rect.width(), rect.height(), &format!("F01 crop pts {pts}")),
        }))
    }
}

struct Never;

impl EvidenceControl for Never {
    fn stop(&self) -> Option<EvidenceStop> {
        None
    }
}

struct Fixture {
    session: SessionId,
    source: SourceId,
    fingerprint: Sha256Hex,
    known: BTreeSet<String>,
    video: F01,
}

impl Fixture {
    fn new() -> Built<Self> {
        Ok(Self {
            session: SessionId::parse(SESSION)?,
            source: SourceId::from_sha256(F01_SHA256)?,
            fingerprint: Sha256Hex::parse(FINGERPRINT)?,
            known: BTreeSet::new(),
            video: F01::new()?,
        })
    }

    /// A call that checked the copy with `check` and may take `slots` more
    /// session evidence artifacts.
    fn call(&self, check: SourceCheck, slots: usize) -> EvidenceCall<'_, Never> {
        EvidenceCall {
            scope: EvidenceScope {
                session_id: &self.session,
                source_id: &self.source,
                profile: EvidenceProfile::P09R0,
                tool_fingerprint: Some(&self.fingerprint),
            },
            source_check: check,
            budget: EvidenceBudget::per_call(slots, u64::MAX),
            known_media: &self.known,
            control: &Never,
        }
    }

    /// `frame get <session> --at <at>` with the defaults: at-or-after, 1 s.
    async fn frame_get(&self, at: u64) -> Built<EvidenceExtraction> {
        Ok(extract_frame_at(
            &self.call(SourceCheck::FullHash, 160),
            &self.video,
            FrameAtRequest {
                at: micros(at),
                selection: FrameSelection::AtOrAfter,
                tolerance: FrameTolerance::DEFAULT,
                candidate: None,
            },
        )
        .await?)
    }
}

/// The delivered path of each item under the placeholder root.
fn placeholder_paths(record: &EvidenceRecord) -> Vec<PathBuf> {
    record
        .items()
        .iter()
        .map(|item| {
            // Written with `/` on every platform, so the examples do not
            // depend on the machine that regenerates them.
            PathBuf::from(format!(
                "{PLACEHOLDER_ROOT}/sessions/{SESSION}/artifacts/artifact-{}.png",
                item.media().sha256().as_str()
            ))
        })
        .collect()
}

fn delivered<'a>(
    record: &'a EvidenceRecord,
    paths: &'a [PathBuf],
) -> Vec<DeliveredEvidenceFile<'a>> {
    record
        .items()
        .iter()
        .zip(paths)
        .map(|(item, path)| DeliveredEvidenceFile {
            evidence_id: item.id(),
            kind: item.media().kind(),
            path,
        })
        .collect()
}

fn lifecycle() -> LifecycleResponse {
    LifecycleResponse::ephemeral(EXPIRES_AT.to_owned())
}

/// A result's `--json` form, with the examples' placeholder paths.
fn json_of(record: &EvidenceRecord, reused: bool) -> Built<Value> {
    let paths = placeholder_paths(record);
    let files = delivered(record, &paths);
    let presentation = EvidencePresentation {
        record,
        reused,
        files: &files,
    };
    Ok(serde_json::to_value(frame_response(
        &presentation,
        lifecycle(),
    )?)?)
}

/// A result's `--events jsonl` lines.
fn lines_of(record: &EvidenceRecord) -> Built<Vec<String>> {
    let paths = placeholder_paths(record);
    let files = delivered(record, &paths);
    let presentation = EvidencePresentation {
        record,
        reused: false,
        files: &files,
    };
    let stream = FrameEvidenceStream::new(&presentation, lifecycle())?;
    let mut lines = stream
        .records()
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()?;
    lines.push(serde_json::to_string(stream.terminal())?);
    Ok(lines)
}

fn pretty(value: &Value) -> Built<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(value)?))
}

/// `frame get --at 1025000` over F01: the first frame at or after the time is
/// the one at 1.05 s, 25 ms later, whole and at native size.
#[tokio::test]
async fn frame_get_matches_the_frozen_example() -> TestResult {
    let fixture = Fixture::new()?;
    let extraction = fixture.frame_get(1_025_000).await?;
    let value = json_of(&extraction.record, false)?;
    regenerate(GET_EXAMPLE, &pretty(&value)?)?;
    validate("operation-response.schema.json", &value)?;
    validate("frame-data.schema.json", &value["data"])?;
    assert!(
        value == load(GET_EXAMPLE)?,
        "the frame get result differs from {GET_EXAMPLE}"
    );
    assert_eq!(value["command"], "frame.get");
    assert_eq!(value["status"], "complete");
    assert_eq!(value["warnings"], serde_json::json!([]));
    let data = &value["data"];
    assert_eq!(
        data["request"],
        serde_json::json!({"at_us": 1_025_000, "policy": "at_or_after", "tolerance_us": 1_000_000, "candidate_id": null})
    );
    assert_eq!(data["selections"][0]["actual_us"], 1_050_000);
    assert_eq!(data["selections"][0]["delta_us"], 25_000);
    assert_eq!(data["selections"][0]["role"], "requested");
    let item = &data["items"][0];
    assert_eq!(item["kind"], "frame");
    assert_eq!(item["frame"]["pts"], 10_752);
    assert_eq!(
        (&item["image"]["width"], &item["image"]["height"]),
        (&Value::from(1_280), &Value::from(720))
    );
    assert_eq!(data["files"][0]["evidence_id"], item["evidence_id"]);
    assert_eq!(data["files"][0]["media_type"], "image/png");
    assert!(
        data["files"][0]["path"]
            .as_str()
            .is_some_and(|path| path.starts_with(PLACEHOLDER_ROOT)),
        "the example names a real folder"
    );
    assert_eq!(data["source_check"], "full_hash");
    assert_eq!(data["partial_reason"], Value::Null);
    Ok(())
}

/// The stream carries each item as a `frame_evidence` event keyed by its
/// identity, then the terminal event with everything else and the count.
#[tokio::test]
async fn frame_get_stream_matches_the_frozen_example_byte_for_byte() -> TestResult {
    let fixture = Fixture::new()?;
    let extraction = fixture.frame_get(1_025_000).await?;
    let json = json_of(&extraction.record, false)?;
    let mut emitted = lines_of(&extraction.record)?.join("\n");
    emitted.push('\n');
    regenerate(GET_STREAM_EXAMPLE, &emitted)?;
    let example = read(GET_STREAM_EXAMPLE)?;
    assert!(
        emitted == example.replace("\r\n", "\n"),
        "the emitted stream differs from {GET_STREAM_EXAMPLE}"
    );
    let lines: Vec<Value> = example
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    for line in &lines {
        validate_line(line)?;
    }
    let items = json["data"]["items"].as_array().ok_or("no items")?;
    let (terminal, records) = lines.split_last().ok_or("empty stream")?;
    assert_eq!(records.len(), items.len());
    for ((record, item), sequence) in records.iter().zip(items).zip(0_u64..) {
        assert_eq!(record["record_type"], "frame_evidence");
        assert_eq!(record["command"], "frame.get");
        assert_eq!(record["key"], item["evidence_id"]);
        assert_eq!(&record["record"], item);
        assert_eq!(record["sequence"], sequence);
    }
    assert_eq!(terminal["sequence"], items.len());
    let data = &terminal["result"]["data"];
    assert_eq!(data["record_count"], items.len());
    assert_eq!(data["files"], json["data"]["files"]);
    assert_eq!(data["selections"], json["data"]["selections"]);
    assert_eq!(terminal["result"]["status"], "complete");
    Ok(())
}

/// Two frames on each side of the first frame: none precede it, so the
/// before side stops at the start of the stream.
#[tokio::test]
async fn frame_neighbours_match_the_frozen_example() -> TestResult {
    let fixture = Fixture::new()?;
    let first = fixture.frame_get(0).await?;
    let anchor = first.record.items().first().ok_or("no anchor")?;
    let extraction = extract_neighbours(
        &fixture.call(SourceCheck::Identity, 160),
        &fixture.video,
        anchor,
        NeighbourCount::new(2)?,
    )
    .await?;
    let value = json_of(&extraction.record, false)?;
    regenerate(NEIGHBOURS_EXAMPLE, &pretty(&value)?)?;
    validate("operation-response.schema.json", &value)?;
    validate("frame-data.schema.json", &value["data"])?;
    assert!(
        value == load(NEIGHBOURS_EXAMPLE)?,
        "the neighbours result differs from {NEIGHBOURS_EXAMPLE}"
    );
    let data = &value["data"];
    assert_eq!(value["command"], "frame.neighbours");
    assert_eq!(
        data["neighbours"],
        serde_json::json!({"before_stop": "start_of_stream", "after_stop": null})
    );
    let actual: Vec<u64> = data["selections"]
        .as_array()
        .ok_or("no selections")?
        .iter()
        .filter_map(|selection| selection["actual_us"].as_u64())
        .collect();
    assert_eq!(actual, vec![50_000, 100_000]);
    assert!(data["selections"].as_array().is_some_and(|all| {
        all.iter()
            .all(|selection| selection["role"] == "after" && selection["requested_us"] == 0)
    }));
    assert_eq!(data["request"]["anchor_evidence_id"], anchor.id().as_str());
    assert_eq!(data["source_check"], "identity");
    Ok(())
}

/// A burst over 4-8 s of the 6 s video is clipped to 4-6 s; the session had
/// room for four images and the record only, so the call returned the
/// first four frames and is `partial` with the typed reason.
#[tokio::test]
async fn a_partial_burst_matches_the_frozen_example() -> TestResult {
    let fixture = Fixture::new()?;
    let extraction = extract_burst(
        &fixture.call(SourceCheck::Identity, 5),
        &fixture.video,
        BurstRange::new(TimeRange::new(micros(4_000_000), micros(8_000_000))?)?,
        BurstCount::new(12)?,
    )
    .await?;
    let value = json_of(&extraction.record, false)?;
    regenerate(BURST_EXAMPLE, &pretty(&value)?)?;
    validate("operation-response.schema.json", &value)?;
    validate("frame-data.schema.json", &value["data"])?;
    assert!(
        value == load(BURST_EXAMPLE)?,
        "the burst result differs from {BURST_EXAMPLE}"
    );
    assert_eq!(value["command"], "frame.burst");
    assert_eq!(value["status"], "partial");
    assert_eq!(
        value["warnings"],
        serde_json::json!([partial_evidence_warning(
            PartialReason::SessionEvidenceBudget
        )])
    );
    let data = &value["data"];
    assert_eq!(data["partial_reason"], "session_evidence_budget");
    assert_eq!(
        data["burst"],
        serde_json::json!({
            "planned": {"start_us": 4_000_000, "end_us": 6_000_000},
            "extent": "clipped_at_end_of_stream",
            "targets": 12,
            "distinct": 12
        })
    );
    assert_eq!(data["items"].as_array().map(Vec::len), Some(4));
    assert_eq!(data["files"].as_array().map(Vec::len), Some(4));
    assert!(
        data["selections"]
            .as_array()
            .is_some_and(|all| all.iter().all(|selection| selection["role"] == "target"))
    );
    Ok(())
}

/// A reused result is the committed result with `reused` set, and nothing
/// else differs.
#[tokio::test]
async fn a_reused_result_differs_only_in_reused() -> TestResult {
    let fixture = Fixture::new()?;
    let extraction = fixture.frame_get(1_025_000).await?;
    let fresh = json_of(&extraction.record, false)?;
    let mut reused = json_of(&extraction.record, true)?;
    assert_eq!(reused["data"]["reused"], true);
    reused["data"]["reused"] = Value::Bool(false);
    assert_eq!(reused, fresh);
    Ok(())
}

/// Files must be exactly one per item, in item order and of the item's kind.
#[tokio::test]
async fn files_that_do_not_match_the_items_are_refused() -> TestResult {
    let fixture = Fixture::new()?;
    let extraction = fixture.frame_get(1_025_000).await?;
    let record = &extraction.record;
    let presentation = |files: &[DeliveredEvidenceFile<'_>]| {
        frame_response(
            &EvidencePresentation {
                record,
                reused: false,
                files,
            },
            lifecycle(),
        )
    };
    assert!(matches!(
        presentation(&[]),
        Err(EvidencePresentationError::FileMismatch)
    ));
    let other = fixture.frame_get(2_000_000).await?;
    let path = PathBuf::from(PLACEHOLDER_ROOT);
    let foreign = DeliveredEvidenceFile {
        evidence_id: other.record.items().first().ok_or("no item")?.id(),
        kind: vsift_domain::EvidenceMediaKind::FramePng,
        path: &path,
    };
    assert!(matches!(
        presentation(&[foreign]),
        Err(EvidencePresentationError::FileMismatch)
    ));
    Ok(())
}

/// A delivered path that is not valid UTF-8 is a typed error, never a lossy
/// string that names another file.
#[cfg(unix)]
#[tokio::test]
async fn a_non_utf8_path_is_a_typed_error() -> TestResult {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let fixture = Fixture::new()?;
    let extraction = fixture.frame_get(1_025_000).await?;
    let item = extraction.record.items().first().ok_or("no item")?;
    let path = PathBuf::from(OsString::from_vec(b"/root/\xff.png".to_vec()));
    let files = [DeliveredEvidenceFile {
        evidence_id: item.id(),
        kind: item.media().kind(),
        path: &path,
    }];
    assert!(matches!(
        frame_response(
            &EvidencePresentation {
                record: &extraction.record,
                reused: false,
                files: &files,
            },
            lifecycle(),
        ),
        Err(EvidencePresentationError::NonUtf8Path)
    ));
    Ok(())
}

#[tokio::test]
async fn published_shapes_reject_what_they_do_not_define() -> TestResult {
    let fixture = Fixture::new()?;
    let extraction = fixture.frame_get(1_025_000).await?;
    let value = json_of(&extraction.record, false)?;
    let data = &value["data"];
    let item = data["items"][0].clone();

    let mut extended = item.clone();
    extended
        .as_object_mut()
        .ok_or("not an object")?
        .insert("path".to_owned(), Value::from("/tmp/frame.png"));
    assert!(!is_valid("frame-evidence.schema.json", &extended)?);

    let mut frame_with_crop = item.clone();
    frame_with_crop["crop"] = serde_json::json!({
        "parent_evidence_id": item["evidence_id"], "x": 0, "y": 0, "width": 1, "height": 1,
        "frame_x": 0, "frame_y": 0
    });
    assert!(!is_valid("frame-evidence.schema.json", &frame_with_crop)?);

    let mut scaled = item.clone();
    scaled["image"]["media_type"] = Value::from("image/jpeg");
    assert!(!is_valid("frame-evidence.schema.json", &scaled)?);

    let mut with_neighbours = data.clone();
    with_neighbours["neighbours"] = serde_json::json!({"before_stop": null, "after_stop": null});
    assert!(!is_valid("frame-data.schema.json", &with_neighbours)?);

    let mut two_requested = data.clone();
    let selection = data["selections"][0].clone();
    two_requested["selections"] = Value::Array(vec![selection.clone(), selection]);
    assert!(!is_valid("frame-data.schema.json", &two_requested)?);

    let mut wrong_tolerance = data.clone();
    wrong_tolerance["request"]["tolerance_us"] = Value::from(10_000_001);
    assert!(!is_valid("frame-data.schema.json", &wrong_tolerance)?);

    let mut with_count = data.clone();
    with_count
        .as_object_mut()
        .ok_or("not an object")?
        .insert("record_count".to_owned(), Value::from(1));
    assert!(!is_valid("frame-data.schema.json", &with_count)?);

    let lines: Vec<Value> = lines_of(&extraction.record)?
        .iter()
        .map(|line| serde_json::from_str(line))
        .collect::<Result<_, _>>()?;
    let record = lines.first().ok_or("no record")?;
    let mut foreign_key = record.clone();
    foreign_key["key"] = Value::from("vcd_0123456789abcdef");
    assert!(!is_valid("evidence-event.schema.json", &foreign_key)?);
    let mut mislabelled = record.clone();
    mislabelled["record_type"] = Value::from("visual_candidate");
    assert!(!is_valid("evidence-event.schema.json", &mislabelled)?);

    let terminal = lines.last().ok_or("no terminal")?;
    let mut with_items = terminal["result"]["data"].clone();
    with_items
        .as_object_mut()
        .ok_or("not an object")?
        .insert("items".to_owned(), Value::Array(Vec::new()));
    assert!(!is_valid("frame-stream-data.schema.json", &with_items)?);
    Ok(())
}
