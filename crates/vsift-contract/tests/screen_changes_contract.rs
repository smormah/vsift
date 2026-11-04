//! Conformance of the `changes` contract (ADR 0034): the `--json` result, the
//! `--events jsonl` stream of `frame_evidence` then `screen_change` records
//! ended by one terminal event, the envelope coverage and `partial` status of
//! a range with gaps or a page that stopped, against the published v1
//! schemas and the frozen examples `examples/changes.json`,
//! `examples/changes.partial.json` and `examples/changes.events.jsonl`.
//!
//! The pages are planned by the application from synthetic visual indexes
//! (the same use cases the engine runs), and their compared changes stand
//! for what `frame get` and the region rule return: the frames are frame
//! evidence items of the session's source at the sampled times, with
//! stand-in digests, and the regions are written out. `changes.json` pages a
//! 14 s, 1440x900 recording whose screen changes at 7 s and at 10 s (the
//! second a status edit in one row, as on F04); `changes.partial.json` pages
//! a 150 s recording whose first window is analysed (a dialog from 5 s to
//! 9 s), second undecodable and third not analysed yet, stopped by the
//! session's evidence room after its first change. Delivered paths are named
//! under the neutral placeholder root `/vsift-session-root`.
//!
//! `changes.crops.json` is `changes.json` asked for with `--crops`: the
//! first change's one region and the status edit's two are cropped from both
//! frames, and the six crops follow the four frames.
//!
//! After an intended contract change, run with
//! `VSIFT_REGENERATE_CONTRACT_EXAMPLES=1` to rewrite the examples, and review
//! the diff.

use std::{env, fs, io, num::NonZeroU32, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    ComparedChange, CropStop, CropSummary, RegionCrops, ScreenChangeCrops, ScreenChangePageRequest,
    ScreenChangeStop, TakenFrame, VisualIndexScope, analysed_ranges, page_screen_changes,
    screen_change_id, screen_change_page_limit, visual_candidate_id, visual_coverage_gaps,
    visual_index_id,
};
use vsift_contract::{
    CHANGES_COVERAGE_WARNING, CHANGES_CROPS_STOPPED_WARNING, CHANGES_STOPPED_WARNING,
    DeliveredEvidenceFile, EvidenceStream, LifecycleResponse, ListedScreenChange,
    ScreenChangeResumePresentation, ScreenChangesEvidenceStream, ScreenChangesPresentation,
    screen_changes_response,
};
use vsift_domain::{
    ChangeRegion, CropRect, CropRegion, EvidenceId, EvidenceItem, EvidenceItemParts, EvidenceMedia,
    EvidenceMediaKind, EvidenceSubject, FrameComparison, FrameDimensions, FrameRef, MediaTime,
    MotionPart, PlannedScreenChange, RegionComparison, ScreenChangeId, ScreenChangeKind, SessionId,
    Sha256Hex, SourceCheck, SourceId, TimeAnchors, TimeBase, TimeRange, VISUAL_BLOCKS,
    VisualCandidate, VisualChangePolicy, VisualHash, VisualIndex, VisualIndexParts,
    VisualIndexProfile, VisualIndexWindow, VisualSample, VisualWindow, VisualWindowOutcome,
    analyse_window,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const SYNTHETIC_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const FINGERPRINT: &str = "5f2d8f0c0e7b4f3a9a1c6d2e8b7a6f5e4d3c2b1a09f8e7d6c5b4a39281706f5e";
const EXPIRES_AT: &str = "2026-09-25T00:00:00Z";
const NOW_US: u64 = 1_790_208_000_000_000;
const SECOND: u64 = 1_000_000;
/// A common MP4 video time base: 1/15360 s.
const TICKS_PER_SECOND: u64 = 15_360;
const PLACEHOLDER_ROOT: &str = "/vsift-session-root";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const JSON_EXAMPLE: &str = "examples/changes.json";
const PARTIAL_EXAMPLE: &str = "examples/changes.partial.json";
const STREAM_EXAMPLE: &str = "examples/changes.events.jsonl";
const CROPS_EXAMPLE: &str = "examples/changes.crops.json";

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative_path: &str) -> Built<Value> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative_path),
    )?)?)
}

fn regenerating() -> bool {
    env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|value| value == "1")
}

fn regenerate(relative_path: &str, text: &str) -> TestResult {
    if regenerating() {
        fs::write(schema_root().join(relative_path), text)?;
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
        Ok(serde_json::from_str(&fs::read_to_string(
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

/// Validates a `--json` result and every change and frame it lists.
fn validate_result(value: &Value) -> TestResult {
    validate("operation-response.schema.json", value)?;
    validate("changes-data.schema.json", &value["data"])?;
    for change in value["data"]["changes"].as_array().ok_or("no changes")? {
        validate("screen-change.schema.json", change)?;
    }
    for frame in value["data"]["frames"].as_array().ok_or("no frames")? {
        validate("frame-evidence.schema.json", frame)?;
    }
    Ok(())
}

/// Validates one stream line against the schemas its `event` selects.
fn validate_line(line: &Value) -> TestResult {
    match line["event"].as_str() {
        Some("evidence") => {
            validate("evidence-event.schema.json", line)?;
            match line["record_type"].as_str() {
                Some("frame_evidence") => validate("frame-evidence.schema.json", &line["record"]),
                Some("screen_change") => validate("screen-change.schema.json", &line["record"]),
                _ => Err("a stream record has an unexpected type".into()),
            }
        }
        Some("terminal") => {
            validate("terminal-event.schema.json", line)?;
            validate("operation-response.schema.json", &line["result"])?;
            validate("changes-stream-data.schema.json", &line["result"]["data"])
        }
        _ => Err("a stream line has no published event kind".into()),
    }
}

fn micros(value: u64) -> MediaTime {
    MediaTime::from_micros(value)
}

fn range(from: u64, to: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(micros(from), micros(to))?)
}

fn pretty(value: &Value) -> Built<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(value)?))
}

/// One synthetic recording: its index and the session it was opened in.
struct Recording {
    session: SessionId,
    index: VisualIndex,
}

impl Recording {
    /// Indexes `windows` of a `duration` recording at `dimensions`, each
    /// analysed window from samples of the grey level `level(time)`.
    fn new(
        duration: u64,
        dimensions: FrameDimensions,
        analysed: &[u32],
        undecodable: &[u32],
        level: impl Fn(u64) -> u8,
    ) -> Built<Self> {
        let session = SessionId::parse(SESSION)?;
        let source = SourceId::from_sha256(SYNTHETIC_DIGEST)?;
        let scope = VisualIndexScope {
            session_id: &session,
            source_id: &source,
            stream_index: 0,
            displayed_dimensions: dimensions,
            duration: micros(duration),
            profile: VisualIndexProfile::R0,
        };
        let mut windows = Vec::new();
        for ordinal in analysed {
            let window = VisualWindow::new(*ordinal, scope.duration)?;
            let mut samples = Vec::new();
            let mut time = window.lead_in_start().as_micros();
            while time < window.range().end().as_micros() {
                let grey = level(time);
                samples.push(VisualSample::from_parts(
                    micros(time),
                    [grey; VISUAL_BLOCKS],
                    VisualHash::from_bits(u64::from(grey)),
                ));
                time += SECOND / 2;
            }
            let analysis = analyse_window(window, &samples, VisualChangePolicy::R0)?;
            let mut candidates = Vec::new();
            for draft in analysis.candidates {
                candidates.push(VisualCandidate::new(
                    visual_candidate_id(&scope, *ordinal, draft.representative)?,
                    draft,
                ));
            }
            windows.push(VisualIndexWindow::new(
                window,
                VisualWindowOutcome::Analysed {
                    sample_count: analysis.sample_count,
                    candidates,
                    dropped_candidates: analysis.dropped_candidates,
                    frameless_cells: analysis.frameless_cells,
                },
                VisualChangePolicy::R0,
            )?);
        }
        for ordinal in undecodable {
            windows.push(VisualIndexWindow::new(
                VisualWindow::new(*ordinal, scope.duration)?,
                VisualWindowOutcome::Undecodable,
                VisualChangePolicy::R0,
            )?);
        }
        let number = NonZeroU32::MIN;
        let index = VisualIndex::new(VisualIndexParts {
            id: visual_index_id(&scope, number)?,
            number,
            source_id: source.clone(),
            stream_index: 0,
            displayed_dimensions: dimensions,
            duration: scope.duration,
            profile: VisualIndexProfile::R0,
            windows,
        })?;
        Ok(Self { session, index })
    }

    /// The 14 s recording of `changes.json`: one screen until 7 s, another
    /// until 10 s, a third after it.
    fn status_edit() -> Built<Self> {
        Self::new(
            14 * SECOND,
            FrameDimensions::new(1440, 900)?,
            &[0],
            &[],
            |time| match time {
                time if time < 7 * SECOND => 40,
                time if time < 10 * SECOND => 90,
                _ => 140,
            },
        )
    }

    /// The 150 s recording of `changes.partial.json`.
    fn dialog_then_gaps() -> Built<Self> {
        Self::new(
            150 * SECOND,
            FrameDimensions::new(1440, 900)?,
            &[0],
            &[1],
            |time| {
                if (5 * SECOND..9 * SECOND).contains(&time) {
                    90
                } else {
                    40
                }
            },
        )
    }

    fn dimensions(&self) -> FrameDimensions {
        self.index.displayed_dimensions()
    }
}

/// A synthetic SHA-256 in lowercase hex.
fn stand_in_digest(material: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    Sha256::digest(material.as_bytes())
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// The frame evidence item `frame get` would store for the sampled frame at
/// `time`, with a stand-in digest.
fn frame_item(dimensions: FrameDimensions, time: MediaTime) -> Built<EvidenceItem> {
    let micros = time.as_micros();
    let digest = stand_in_digest(&format!("stand-in png at {micros}"));
    let identity = stand_in_digest(&format!("stand-in frame at {micros}"));
    Ok(EvidenceItem::new(EvidenceItemParts {
        id: EvidenceId::parse(format!("evd_{}", identity.get(..32).ok_or("short digest")?))?,
        subject: EvidenceSubject::Frame(FrameRef {
            stream_index: 0,
            time_base: TimeBase::new(1, u32::try_from(TICKS_PER_SECOND)?)?,
            pts: i64::try_from(micros * TICKS_PER_SECOND / SECOND)?,
            time,
            dimensions,
        }),
        media: EvidenceMedia::new(
            EvidenceMediaKind::FramePng,
            Sha256Hex::parse(digest)?,
            48_000 + micros / 1_000,
        )?,
        tool_fingerprint: Some(Sha256Hex::parse(FINGERPRINT)?),
        source_check: SourceCheck::Identity,
    })?)
}

fn taken(item: &EvidenceItem, dimensions: FrameDimensions) -> Built<TakenFrame> {
    let EvidenceSubject::Frame(frame) = item.subject() else {
        return Err("not a frame".into());
    };
    Ok(TakenFrame {
        evidence_id: item.id().clone(),
        time: frame.time,
        dimensions,
    })
}

fn region(
    dimensions: FrameDimensions,
    (x, y, width, height): (u32, u32, u32, u32),
    changed_pixels: u64,
) -> Built<ChangeRegion> {
    Ok(ChangeRegion {
        rect: CropRect::new(x, y, width, height, dimensions)?,
        changed_pixels,
    })
}

/// A page's changes compared with `comparisons` (one per planned change, in
/// order), its distinct frames in order of first use and their identities.
struct ComparedPage {
    changes: Vec<(ScreenChangeId, ComparedChange)>,
    frames: Vec<EvidenceItem>,
    /// Crops made by `--crops`, in the order they were made.
    crop_items: Vec<EvidenceItem>,
}

fn compare(
    recording: &Recording,
    planned: &[PlannedScreenChange],
    comparisons: Vec<FrameComparison>,
) -> Built<ComparedPage> {
    let dimensions = recording.dimensions();
    let mut frames: Vec<EvidenceItem> = Vec::new();
    let mut changes = Vec::new();
    for (change, comparison) in planned.iter().zip(comparisons) {
        let mut take = |time: MediaTime| -> Built<TakenFrame> {
            let item = frame_item(dimensions, time)?;
            let frame = taken(&item, dimensions)?;
            if !frames.iter().any(|known| known.id() == item.id()) {
                frames.push(item);
            }
            Ok(frame)
        };
        let before = take(change.before)?;
        let after = take(change.after)?;
        changes.push((
            screen_change_id(&recording.session, &recording.index, change)?,
            ComparedChange {
                change: change.clone(),
                before,
                after,
                comparison,
                crops: Vec::new(),
            },
        ));
    }
    Ok(ComparedPage {
        changes,
        frames,
        crop_items: Vec::new(),
    })
}

/// The crop evidence item `crop` would store for `rect` of `parent`, with a
/// stand-in digest.
fn crop_item(parent: &EvidenceItem, rect: CropRect) -> Built<EvidenceItem> {
    let EvidenceSubject::Frame(frame) = parent.subject() else {
        return Err("not a frame".into());
    };
    let material = format!(
        "{} {},{},{},{}",
        parent.id().as_str(),
        rect.x(),
        rect.y(),
        rect.width(),
        rect.height()
    );
    let digest = stand_in_digest(&format!("stand-in crop png of {material}"));
    let identity = stand_in_digest(&format!("stand-in crop of {material}"));
    Ok(EvidenceItem::new(EvidenceItemParts {
        id: EvidenceId::parse(format!("evd_{}", identity.get(..32).ok_or("short digest")?))?,
        subject: EvidenceSubject::Crop {
            frame: frame.clone(),
            region: CropRegion {
                parent: parent.id().clone(),
                rect,
                frame_rect: rect,
            },
        },
        media: EvidenceMedia::new(
            EvidenceMediaKind::FramePng,
            Sha256Hex::parse(digest)?,
            4_000 + u64::from(rect.width()),
        )?,
        tool_fingerprint: Some(Sha256Hex::parse(FINGERPRINT)?),
        source_check: SourceCheck::Identity,
    })?)
}

/// Crops the first two regions of each change, before then after, in page
/// order, keeping the first `made` crops (as a call that stopped there).
fn with_crops(mut page: ComparedPage, made: usize) -> Built<ComparedPage> {
    let mut count = 0;
    for (_, change) in &mut page.changes {
        let FrameComparison::Compared(found) = &change.comparison else {
            continue;
        };
        let mut crops = Vec::new();
        for region in found.regions.iter().take(2) {
            let mut entry = RegionCrops::default();
            for (frame, slot) in [
                (&change.before, &mut entry.before),
                (&change.after, &mut entry.after),
            ] {
                if count < made {
                    let parent = page
                        .frames
                        .iter()
                        .find(|item| item.id() == &frame.evidence_id)
                        .ok_or("an unlisted parent")?;
                    let item = crop_item(parent, region.rect)?;
                    *slot = Some(item.id().clone());
                    page.crop_items.push(item);
                    count += 1;
                }
            }
            crops.push(entry);
        }
        change.crops = crops;
    }
    Ok(page)
}

fn placeholder_paths(frames: &[EvidenceItem]) -> Vec<PathBuf> {
    frames
        .iter()
        .map(|item| {
            PathBuf::from(format!(
                "{PLACEHOLDER_ROOT}/sessions/{SESSION}/artifacts/artifact-{}.png",
                item.media().sha256().as_str()
            ))
        })
        .collect()
}

fn lifecycle() -> LifecycleResponse {
    LifecycleResponse::ephemeral(EXPIRES_AT.to_owned())
}

/// One page in both published forms.
struct Rendered {
    json: Value,
    lines: Vec<String>,
}

#[derive(Clone, Copy)]
struct PageOptions<'a> {
    requested: TimeRange,
    listed: usize,
    stop: Option<ScreenChangeStop>,
    resume: Option<ScreenChangeResumePresentation>,
    next_cursor: Option<&'a str>,
    anchors: &'a TimeAnchors,
    crops: Option<CropSummary>,
}

fn render(recording: &Recording, page: &ComparedPage, options: PageOptions<'_>) -> Built<Rendered> {
    let listed: Vec<ListedScreenChange<'_>> = page
        .changes
        .iter()
        .take(options.listed)
        .map(|(id, compared)| ListedScreenChange { id, compared })
        .collect();
    // Only the frames of the listed changes, as the engine takes them, then
    // with crops the crops, which are made after every frame.
    let mut frames: Vec<EvidenceItem> = page
        .frames
        .iter()
        .filter(|frame| {
            listed.iter().any(|change| {
                change.compared.before.evidence_id == *frame.id()
                    || change.compared.after.evidence_id == *frame.id()
            })
        })
        .cloned()
        .collect();
    if options.crops.is_some() {
        frames.extend(page.crop_items.iter().cloned());
    }
    let paths = placeholder_paths(&frames);
    let files: Vec<DeliveredEvidenceFile<'_>> = frames
        .iter()
        .zip(&paths)
        .map(|(item, path)| DeliveredEvidenceFile {
            evidence_id: item.id(),
            kind: item.media().kind(),
            path,
        })
        .collect();
    let index = &recording.index;
    let analysed = analysed_ranges(index, options.requested);
    let gaps = visual_coverage_gaps(Some(index), index.duration(), options.requested, None);
    let presentation = ScreenChangesPresentation {
        session_id: &recording.session,
        index,
        range: options.requested,
        searched: TimeRange::new(
            options.requested.start(),
            options.requested.end().min(index.duration()),
        )?,
        changes: &listed,
        frames: &frames,
        files: &files,
        next_cursor: options.next_cursor,
        resume: options.resume,
        analysed: &analysed,
        gaps: &gaps,
        stop: options.stop,
        time_anchors: options.anchors,
        crops: options.crops,
    };
    let json = serde_json::to_value(screen_changes_response(&presentation, lifecycle())?)?;
    let stream = ScreenChangesEvidenceStream::new(&presentation, lifecycle())?;
    let mut lines = stream
        .records()
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()?;
    lines.push(serde_json::to_string(stream.terminal())?);
    Ok(Rendered { json, lines })
}

/// The planned changes of a request, as the engine pages them.
fn planned(recording: &Recording, requested: TimeRange) -> Built<Vec<PlannedScreenChange>> {
    Ok(page_screen_changes(
        &recording.session,
        &recording.index,
        &ScreenChangePageRequest {
            range: requested,
            limit: screen_change_page_limit(None)?,
            cursor: None,
            crops: ScreenChangeCrops::NoCrops,
        },
        NOW_US,
    )?
    .changes)
}

/// `changes.json`: two changes; the status edit at 10 s is one row and a
/// few pixels of re-encoded text around it.
fn status_edit_page() -> Built<(Recording, ComparedPage, TimeRange)> {
    let recording = Recording::status_edit()?;
    let dimensions = recording.dimensions();
    let requested = range(0, 14 * SECOND)?;
    let changes = planned(&recording, requested)?;
    assert_eq!(changes.len(), 2, "the synthetic recording changes twice");
    let page = compare(
        &recording,
        &changes,
        vec![
            FrameComparison::Compared(RegionComparison {
                regions: vec![region(dimensions, (0, 64, 1440, 772), 1_041_600)?],
                regions_left_out: 0,
                changed_area: Some(region(dimensions, (0, 64, 1440, 772), 1_041_600)?),
            }),
            FrameComparison::Compared(RegionComparison {
                regions: vec![
                    region(dimensions, (1008, 384, 400, 48), 7_140)?,
                    region(dimensions, (224, 608, 32, 16), 25)?,
                ],
                regions_left_out: 0,
                changed_area: Some(region(dimensions, (224, 384, 1184, 240), 7_165)?),
            }),
        ],
    )?;
    Ok((recording, page, requested))
}

fn status_edit_rendered(anchors: &TimeAnchors) -> Built<Rendered> {
    let (recording, page, requested) = status_edit_page()?;
    render(
        &recording,
        &page,
        PageOptions {
            requested,
            listed: 2,
            stop: None,
            resume: None,
            next_cursor: None,
            anchors,
            crops: None,
        },
    )
}

#[test]
fn a_complete_page_matches_the_frozen_example() -> TestResult {
    let rendered = status_edit_rendered(&TimeAnchors::none())?;
    let value = rendered.json;
    regenerate(JSON_EXAMPLE, &pretty(&value)?)?;
    validate_result(&value)?;
    assert!(
        value == load(JSON_EXAMPLE)?,
        "the changes result differs from {JSON_EXAMPLE}"
    );
    assert_eq!(value["command"], "changes");
    assert_eq!(value["status"], "complete");
    assert_eq!(value["warnings"], serde_json::json!([]));
    assert_eq!(
        value["coverage"],
        serde_json::json!({"truncated": false, "gaps": [], "reasons": []})
    );
    let data = &value["data"];
    assert_eq!(
        data["profile"],
        serde_json::json!({
            "identifier": "screen-change-regions-v1",
            "pixel_level": 32,
            "tile": 16,
            "tile_pixels": 4,
            "join_distance": 2,
            "max_regions": 8
        })
    );
    let changes = data["changes"].as_array().ok_or("changes")?;
    assert_eq!(changes.len(), 2);
    let status = &changes[1];
    assert_eq!(status["kind"], "change");
    assert_eq!(status["motion"], Value::Null);
    assert_eq!(status["settled"], Value::Null);
    assert_eq!(
        status["change_window"],
        serde_json::json!({"from_us": 9_500_000, "to_us": 10_000_000})
    );
    assert_eq!(status["before"]["time_us"], 9_500_000);
    assert_eq!(status["after"]["time_us"], 10_000_000);
    assert_eq!(status["comparison"], "compared");
    assert_eq!(status["no_region_at_threshold"], false);
    // Consecutive changes share no frame here: four distinct frames, each
    // listed once with its file, in order of first use.
    let frames = data["frames"].as_array().ok_or("frames")?;
    let files = data["files"].as_array().ok_or("files")?;
    assert_eq!(frames.len(), 4);
    assert_eq!(files.len(), 4);
    for change in changes {
        for side in ["before", "after"] {
            let id = &change[side]["evidence_id"];
            assert!(frames.iter().any(|frame| &frame["evidence_id"] == id));
            assert!(files.iter().any(|file| &file["evidence_id"] == id));
        }
    }
    for (frame, file) in frames.iter().zip(files) {
        assert_eq!(frame["evidence_id"], file["evidence_id"]);
        assert_eq!(frame["profile"], "p09-r0-v1");
        assert_eq!(file["media_type"], "image/png");
        assert!(
            file["path"]
                .as_str()
                .is_some_and(|path| path.starts_with(PLACEHOLDER_ROOT))
        );
    }
    assert_eq!(data["partial_reason"], Value::Null);
    assert_eq!(data["next_cursor"], Value::Null);
    assert_eq!(data["resume"], Value::Null);
    Ok(())
}

/// The stream carries the frames as `frame_evidence` events keyed by their
/// identity, then each change as a `screen_change` event keyed by its
/// `change_id`, then the terminal event with everything else and the count.
#[test]
fn the_stream_matches_the_frozen_example_byte_for_byte() -> TestResult {
    let rendered = status_edit_rendered(&TimeAnchors::none())?;
    let text = format!("{}\n", rendered.lines.join("\n"));
    regenerate(STREAM_EXAMPLE, &text)?;
    let lines: Vec<Value> = rendered
        .lines
        .iter()
        .map(|line| serde_json::from_str(line))
        .collect::<Result<_, _>>()?;
    for (sequence, line) in lines.iter().enumerate() {
        validate_line(line)?;
        assert_eq!(line["sequence"], sequence);
        assert_eq!(line["command"], "changes");
    }
    let data = &rendered.json["data"];
    let frames = data["frames"].as_array().ok_or("frames")?;
    let changes = data["changes"].as_array().ok_or("changes")?;
    let (terminal, records) = lines.split_last().ok_or("empty")?;
    assert_eq!(records.len(), frames.len() + changes.len());
    for (record, frame) in records.iter().zip(frames) {
        assert_eq!(record["record_type"], "frame_evidence");
        assert_eq!(record["key"], frame["evidence_id"]);
        assert_eq!(&record["record"], frame);
    }
    for (record, change) in records.iter().skip(frames.len()).zip(changes) {
        assert_eq!(record["record_type"], "screen_change");
        assert_eq!(record["key"], change["change_id"]);
        assert_eq!(&record["record"], change);
    }
    let result = &terminal["result"];
    assert_eq!(result["status"], "complete");
    assert_eq!(result["data"]["record_count"], records.len());
    assert_eq!(result["data"]["files"], data["files"]);
    assert!(result["data"].get("changes").is_none());
    assert!(result["data"].get("frames").is_none());
    let committed = fs::read_to_string(schema_root().join(STREAM_EXAMPLE))?;
    assert!(
        committed == text,
        "the changes stream differs from {STREAM_EXAMPLE}"
    );
    Ok(())
}

/// A page stopped by the session's evidence room after its first change,
/// over a range with an undecodable and a not-analysed window: `partial`
/// with both fixed warnings, no cursor, `resume` at the first change not
/// listed, and envelope coverage that names the gaps and the stop.
#[test]
fn a_stopped_page_over_gaps_matches_the_frozen_example() -> TestResult {
    let recording = Recording::dialog_then_gaps()?;
    let dimensions = recording.dimensions();
    let requested = range(0, 150 * SECOND)?;
    let changes = planned(&recording, requested)?;
    assert_eq!(changes.len(), 2, "the dialog opens and closes");
    let dialog = region(dimensions, (400, 260, 640, 380), 201_344)?;
    let page = compare(
        &recording,
        &changes,
        vec![
            FrameComparison::Compared(RegionComparison {
                regions: vec![dialog],
                regions_left_out: 0,
                changed_area: Some(dialog),
            }),
            FrameComparison::Compared(RegionComparison {
                regions: vec![dialog],
                regions_left_out: 0,
                changed_area: Some(dialog),
            }),
        ],
    )?;
    let not_listed = changes.get(1).ok_or("no second change")?.after;
    let value = render(
        &recording,
        &page,
        PageOptions {
            requested,
            listed: 1,
            stop: Some(ScreenChangeStop::SessionEvidenceBudget),
            resume: Some(ScreenChangeResumePresentation {
                from: not_listed,
                reason: "session_evidence_budget",
            }),
            next_cursor: None,
            anchors: &TimeAnchors::none(),
            crops: None,
        },
    )?
    .json;
    regenerate(PARTIAL_EXAMPLE, &pretty(&value)?)?;
    validate_result(&value)?;
    assert!(
        value == load(PARTIAL_EXAMPLE)?,
        "the partial changes result differs from {PARTIAL_EXAMPLE}"
    );
    assert_eq!(value["status"], "partial");
    assert_eq!(
        value["warnings"],
        serde_json::json!([CHANGES_COVERAGE_WARNING, CHANGES_STOPPED_WARNING])
    );
    assert_eq!(value["coverage"]["truncated"], true);
    assert_eq!(
        value["coverage"]["gaps"],
        serde_json::json!(["9000000-150000000"])
    );
    assert_eq!(
        value["coverage"]["reasons"],
        serde_json::json!(["not_analyzed", "undecodable", "session_evidence_budget"])
    );
    let data = &value["data"];
    assert_eq!(data["changes"].as_array().map(Vec::len), Some(1));
    assert_eq!(data["frames"].as_array().map(Vec::len), Some(2));
    assert_eq!(data["partial_reason"], "session_evidence_budget");
    assert_eq!(data["next_cursor"], Value::Null);
    assert_eq!(
        data["resume"],
        serde_json::json!({"from_us": 9_000_000, "reason": "session_evidence_budget"})
    );
    Ok(())
}

/// A page with only gaps and no stop is `partial` with the coverage warning
/// alone, and its stream's terminal event says the same.
#[test]
fn gaps_alone_make_a_page_partial_with_one_warning() -> TestResult {
    let recording = Recording::dialog_then_gaps()?;
    let requested = range(60 * SECOND, 150 * SECOND)?;
    let page = compare(&recording, &[], Vec::new())?;
    let rendered = render(
        &recording,
        &page,
        PageOptions {
            requested,
            listed: 0,
            stop: None,
            resume: Some(ScreenChangeResumePresentation {
                from: micros(120 * SECOND),
                reason: "not_analyzed",
            }),
            next_cursor: None,
            anchors: &TimeAnchors::none(),
            crops: None,
        },
    )?;
    validate_result(&rendered.json)?;
    assert_eq!(rendered.json["status"], "partial");
    assert_eq!(
        rendered.json["warnings"],
        serde_json::json!([CHANGES_COVERAGE_WARNING])
    );
    assert_eq!(rendered.json["data"]["changes"], serde_json::json!([]));
    assert_eq!(rendered.json["data"]["files"], serde_json::json!([]));
    let terminal: Value = serde_json::from_str(rendered.lines.last().ok_or("no line")?)?;
    validate_line(&terminal)?;
    assert_eq!(rendered.lines.len(), 1);
    assert_eq!(terminal["result"]["status"], "partial");
    assert_eq!(terminal["result"]["data"]["record_count"], 0);
    Ok(())
}

/// A motion run, a run start and end, a change with no region at the
/// threshold and one between frames of different sizes are all published as
/// the schema says, and the schema refuses what contradicts them.
#[test]
fn every_kind_and_comparison_is_published_as_the_schema_says() -> TestResult {
    let recording = Recording::status_edit()?;
    let dimensions = recording.dimensions();
    let base = planned(&recording, range(0, 14 * SECOND)?)?;
    let template = base.first().ok_or("no change")?;
    let with_kind = |kind: ScreenChangeKind| PlannedScreenChange {
        kind,
        ..template.clone()
    };
    let kinds = [
        with_kind(ScreenChangeKind::Motion(MotionPart::Run)),
        with_kind(ScreenChangeKind::Motion(MotionPart::Start)),
        with_kind(ScreenChangeKind::Motion(MotionPart::End)),
        with_kind(ScreenChangeKind::Change),
    ];
    let page = compare(
        &recording,
        &kinds,
        vec![
            FrameComparison::Compared(RegionComparison {
                regions: vec![region(dimensions, (0, 0, 16, 16), 4)?],
                regions_left_out: 3,
                changed_area: Some(region(dimensions, (0, 0, 1440, 900), 400)?),
            }),
            FrameComparison::Compared(RegionComparison {
                regions: Vec::new(),
                regions_left_out: 0,
                changed_area: None,
            }),
            FrameComparison::Compared(RegionComparison {
                regions: Vec::new(),
                regions_left_out: 0,
                changed_area: None,
            }),
            FrameComparison::DimensionsDiffer,
        ],
    )?;
    let value = render(
        &recording,
        &page,
        PageOptions {
            requested: range(0, 14 * SECOND)?,
            listed: 4,
            stop: None,
            resume: None,
            next_cursor: None,
            anchors: &TimeAnchors::none(),
            crops: None,
        },
    )?
    .json;
    validate_result(&value)?;
    let changes = value["data"]["changes"].as_array().ok_or("changes")?;
    let facts: Vec<(&Value, &Value, &Value, &Value, &Value)> = changes
        .iter()
        .map(|change| {
            (
                &change["kind"],
                &change["motion"],
                &change["settled"],
                &change["comparison"],
                &change["no_region_at_threshold"],
            )
        })
        .collect();
    assert_eq!(
        serde_json::to_value(&facts)?,
        serde_json::json!([
            ["motion", "run", true, "compared", false],
            ["motion", "start", false, "compared", true],
            ["motion", "end", true, "compared", true],
            ["change", null, null, "dimensions_differ", false]
        ])
    );
    assert_eq!(changes[0]["regions_left_out"], 3);
    assert_eq!(changes[3]["changed_area"], Value::Null);

    let mut contradictory = changes[3].clone();
    contradictory["motion"] = Value::from("run");
    assert!(!is_valid("screen-change.schema.json", &contradictory)?);
    let mut unsettled = changes[0].clone();
    unsettled["settled"] = Value::Null;
    assert!(!is_valid("screen-change.schema.json", &unsettled)?);
    let mut uncompared = changes[3].clone();
    uncompared["no_region_at_threshold"] = Value::from(true);
    assert!(!is_valid("screen-change.schema.json", &uncompared)?);
    let mut extra = changes[1].clone();
    extra["meaning"] = Value::from("a dialog opened");
    assert!(!is_valid("screen-change.schema.json", &extra)?);
    Ok(())
}

/// Time anchoring (ADR 0029): each change's before and after carry the clock
/// time of their frame, the same as the frame's own record states.
#[test]
fn before_and_after_carry_their_frames_clock_time() -> TestResult {
    let anchors = TimeAnchors::new(vec![vsift_domain::TimeAnchor::new(
        vsift_domain::AnchorSource::QuicktimeCreationDate,
        micros(0),
        vsift_domain::ClockReading::parse("2026-10-21T09:30:00-05:00")?,
    )])?;
    let rendered = status_edit_rendered(&anchors)?;
    validate_result(&rendered.json)?;
    let data = &rendered.json["data"];
    let frames = data["frames"].as_array().ok_or("frames")?;
    for change in data["changes"].as_array().ok_or("changes")? {
        for side in ["before", "after"] {
            let frame = frames
                .iter()
                .find(|frame| frame["evidence_id"] == change[side]["evidence_id"])
                .ok_or("an unlisted frame")?;
            assert!(change[side]["clock_time"].is_object());
            assert_eq!(change[side]["clock_time"], frame["clock_time"]);
        }
    }
    assert_eq!(
        data["changes"][1]["after"]["clock_time"]["at"]["utc"],
        "2026-10-21T14:30:10.000000Z"
    );
    Ok(())
}

/// Files that are not exactly one per frame, in frame order, are refused.
#[test]
fn files_that_do_not_match_the_frames_are_refused() -> TestResult {
    let (recording, page, requested) = status_edit_page()?;
    let listed: Vec<ListedScreenChange<'_>> = page
        .changes
        .iter()
        .map(|(id, compared)| ListedScreenChange { id, compared })
        .collect();
    let paths = placeholder_paths(&page.frames);
    let mut files: Vec<DeliveredEvidenceFile<'_>> = page
        .frames
        .iter()
        .zip(&paths)
        .map(|(item, path)| DeliveredEvidenceFile {
            evidence_id: item.id(),
            kind: item.media().kind(),
            path,
        })
        .collect();
    files.reverse();
    let anchors = TimeAnchors::none();
    let presentation = ScreenChangesPresentation {
        session_id: &recording.session,
        index: &recording.index,
        range: requested,
        searched: requested,
        changes: &listed,
        frames: &page.frames,
        files: &files,
        next_cursor: None,
        resume: None,
        analysed: &[],
        gaps: &[],
        stop: None,
        time_anchors: &anchors,
        crops: None,
    };
    assert!(screen_changes_response(&presentation, lifecycle()).is_err());
    assert!(ScreenChangesEvidenceStream::new(&presentation, lifecycle()).is_err());
    Ok(())
}

/// The status-edit page asked for with `--crops`, keeping `made` crops and
/// reporting `summary`.
fn status_edit_with_crops(made: usize, summary: CropSummary) -> Built<Rendered> {
    let (recording, page, requested) = status_edit_page()?;
    let page = with_crops(page, made)?;
    render(
        &recording,
        &page,
        PageOptions {
            requested,
            listed: 2,
            stop: None,
            resume: None,
            next_cursor: None,
            anchors: &TimeAnchors::none(),
            crops: Some(summary),
        },
    )
}

/// Without `--crops` nothing of crops is present: no `crops` member, no crop
/// in any region, no crop among the frames. The frozen slice-1 examples
/// above pin the rest byte for byte.
#[test]
fn a_page_without_crops_has_no_crop_member_anywhere() -> TestResult {
    let rendered = status_edit_rendered(&TimeAnchors::none())?;
    let data = &rendered.json["data"];
    assert!(data.get("crops").is_none());
    for change in data["changes"].as_array().ok_or("changes")? {
        for region in change["regions"].as_array().ok_or("regions")? {
            assert!(region.get("before_crop").is_none());
            assert!(region.get("after_crop").is_none());
        }
    }
    assert!(
        data["frames"]
            .as_array()
            .ok_or("frames")?
            .iter()
            .all(|frame| frame["kind"] == "frame")
    );
    let terminal: Value = serde_json::from_str(rendered.lines.last().ok_or("no terminal")?)?;
    assert!(terminal["result"]["data"].get("crops").is_none());
    Ok(())
}

/// With `--crops`, each change's two largest regions name their before and
/// after crops, the crops follow the frames with one file each, and the
/// stream carries them as `frame_evidence` events before the changes.
#[test]
fn a_page_with_crops_matches_the_frozen_example() -> TestResult {
    let rendered = status_edit_with_crops(
        6,
        CropSummary {
            made: 6,
            regions_without_crop: 0,
            stop: None,
        },
    )?;
    let value = rendered.json;
    regenerate(CROPS_EXAMPLE, &pretty(&value)?)?;
    validate_result(&value)?;
    assert!(
        value == load(CROPS_EXAMPLE)?,
        "the changes --crops result differs from {CROPS_EXAMPLE}"
    );
    assert_eq!(value["status"], "complete");
    assert_eq!(value["warnings"], serde_json::json!([]));
    let data = &value["data"];
    assert_eq!(
        data["crops"],
        serde_json::json!({"limit": 16, "made": 6, "regions_without_crop": 0, "stop": null})
    );
    let frames = data["frames"].as_array().ok_or("frames")?;
    let files = data["files"].as_array().ok_or("files")?;
    assert_eq!(frames.len(), 10);
    assert_eq!(files.len(), 10);
    assert!(frames.iter().take(4).all(|frame| frame["kind"] == "frame"));
    assert!(frames.iter().skip(4).all(|frame| frame["kind"] == "crop"));
    for (frame, file) in frames.iter().zip(files) {
        assert_eq!(frame["evidence_id"], file["evidence_id"]);
    }
    for change in data["changes"].as_array().ok_or("changes")? {
        for region in change["regions"].as_array().ok_or("regions")? {
            for (side, crop) in [("before", "before_crop"), ("after", "after_crop")] {
                let item = frames
                    .iter()
                    .find(|frame| frame["evidence_id"] == region[crop])
                    .ok_or("a crop is not listed")?;
                assert_eq!(
                    item["crop"]["parent_evidence_id"],
                    change[side]["evidence_id"]
                );
                assert_eq!(item["crop"]["x"], region["x"]);
                assert_eq!(item["crop"]["width"], region["width"]);
            }
        }
    }
    // The stream: ten frame_evidence events, then the two changes, then the
    // terminal event with the crops summary and the count.
    let lines: Vec<Value> = rendered
        .lines
        .iter()
        .map(|line| serde_json::from_str(line))
        .collect::<Result<_, _>>()?;
    for line in &lines {
        validate_line(line)?;
    }
    assert_eq!(lines.len(), 13);
    let terminal = lines.last().ok_or("no terminal")?;
    assert_eq!(terminal["result"]["data"]["record_count"], 12);
    assert_eq!(terminal["result"]["data"]["crops"], data["crops"]);
    Ok(())
}

/// A crop stop for the room, the deadline or a cancellation leaves the
/// result complete, with `partial_reason` null and coverage whole, adds the
/// fixed crop warning, and keeps the crops made before it; the 16-crop bound
/// adds no warning (the planner's ruling of 2026-11-04).
#[test]
fn a_crop_stopped_page_stays_complete() -> TestResult {
    for stop in [
        CropStop::SessionEvidenceBudget,
        CropStop::DeadlineExceeded,
        CropStop::Cancelled,
        CropStop::CropLimit,
    ] {
        // Three crops: the first change's region, then only the status
        // row's before crop; the status row lacks its after crop and the
        // small region has none.
        let rendered = status_edit_with_crops(
            3,
            CropSummary {
                made: 3,
                regions_without_crop: 2,
                stop: Some(stop),
            },
        )?;
        let value = rendered.json;
        validate_result(&value)?;
        assert_eq!(value["status"], "complete", "{stop:?}");
        assert_eq!(value["data"]["partial_reason"], Value::Null);
        assert_eq!(value["data"]["resume"], Value::Null);
        assert_eq!(
            value["coverage"],
            serde_json::json!({"truncated": false, "gaps": [], "reasons": []})
        );
        assert_eq!(value["data"]["crops"]["stop"], stop.identifier());
        let warnings = if stop.is_interrupted() {
            serde_json::json!([CHANGES_CROPS_STOPPED_WARNING])
        } else {
            serde_json::json!([])
        };
        assert_eq!(value["warnings"], warnings, "{stop:?}");
        let status = &value["data"]["changes"][1]["regions"];
        assert!(status[0]["before_crop"].is_string());
        assert!(status[0].get("after_crop").is_none());
        assert!(status[1].get("before_crop").is_none());
        assert_eq!(
            value["data"]["frames"].as_array().ok_or("frames")?.len(),
            4 + 3
        );
        for line in &rendered.lines {
            validate_line(&serde_json::from_str(line)?)?;
        }
    }
    assert!(CHANGES_CROPS_STOPPED_WARNING.len() <= 1024);
    Ok(())
}

/// The crop members are published only where the schema puts them: never on
/// `changed_area`, never with a value that is not an evidence identity, and
/// `crops` only with its four members.
#[test]
fn the_crop_members_are_published_as_the_schema_says() -> TestResult {
    let rendered = status_edit_with_crops(
        6,
        CropSummary {
            made: 6,
            regions_without_crop: 0,
            stop: None,
        },
    )?;
    let changes = rendered.json["data"]["changes"]
        .as_array()
        .ok_or("changes")?
        .clone();
    let status = changes.get(1).ok_or("the status change")?;
    assert!(is_valid("screen-change.schema.json", status)?);
    let mut on_area = status.clone();
    on_area["changed_area"]["before_crop"] = status["regions"][0]["before_crop"].clone();
    assert!(!is_valid("screen-change.schema.json", &on_area)?);
    let mut not_an_id = status.clone();
    not_an_id["regions"][0]["after_crop"] = Value::from("crop-1");
    assert!(!is_valid("screen-change.schema.json", &not_an_id)?);
    let mut data = rendered.json["data"].clone();
    assert!(is_valid("changes-data.schema.json", &data)?);
    data["crops"]["limit"] = Value::from(17);
    assert!(!is_valid("changes-data.schema.json", &data)?);
    data["crops"]["limit"] = Value::from(16);
    data["crops"]["stop"] = Value::from("partial");
    assert!(!is_valid("changes-data.schema.json", &data)?);
    Ok(())
}
