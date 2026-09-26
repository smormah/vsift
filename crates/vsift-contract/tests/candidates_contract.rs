//! Conformance of the `candidates` contract (P08, ADR 0018): the `--json`
//! result, the `--events jsonl` stream of `visual_candidate` records ended by
//! one terminal event, the envelope coverage and `partial` status of a range
//! with gaps, against the published v1 schemas and the frozen examples
//! `examples/candidates.json`, `examples/candidates.partial.json` and
//! `examples/candidates.events.jsonl`.
//!
//! `candidates.json` and the stream page the F02 fixture (three slides,
//! A-B-A) indexed from the samples real `FFmpeg` decoded from it
//! (`crates/vsift-infrastructure/tests/data/visual_samples/F02.json`), so the
//! example shows exactly what an agent receives for that video.
//! `candidates.partial.json` pages a synthetic 150 s video whose first window
//! is analysed, second undecodable and third not analysed yet.
//!
//! After an intended contract change, run with
//! `VSIFT_REGENERATE_CONTRACT_EXAMPLES=1` to rewrite the examples, and review
//! the diff.

use std::{env, fs, io, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    CandidatePageRequest, VisualExtensionStop, VisualIndexScope, analysed_ranges, page_candidates,
    visual_candidate_id, visual_coverage_gaps, visual_index_id, whole_file_source_segment,
};
use vsift_contract::{
    CandidatesEvidenceStream, CandidatesPresentation, EvidenceStream, LifecycleResponse,
    VISUAL_COVERAGE_WARNING, candidates_response,
};
use vsift_domain::{
    FrameDimensions, MediaTime, PageLimit, SessionId, SourceId, TimeRange, VISUAL_BLOCKS,
    VisualCandidate, VisualChangePolicy, VisualHash, VisualIndex, VisualIndexParts,
    VisualIndexProfile, VisualIndexWindow, VisualSample, VisualWindow, VisualWindowOutcome,
    analyse_window,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const SYNTHETIC_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const EXPIRES_AT: &str = "2026-09-25T00:00:00Z";
const CURSOR_EXPIRES_US: u64 = 1_790_294_400_000_000;
const NOW_US: u64 = 1_790_208_000_000_000;
const SECOND: u64 = 1_000_000;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const JSON_EXAMPLE: &str = "examples/candidates.json";
const PARTIAL_EXAMPLE: &str = "examples/candidates.partial.json";
const STREAM_EXAMPLE: &str = "examples/candidates.events.jsonl";

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn read(relative_path: &str) -> Built<String> {
    Ok(fs::read_to_string(schema_root().join(relative_path))?)
}

fn load(relative_path: &str) -> Built<Value> {
    Ok(serde_json::from_str(&read(relative_path)?)?)
}

fn regenerating() -> bool {
    env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|value| value == "1")
}

/// Writes `text` over an example when regenerating.
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

/// Validates one candidates stream line against the schemas its `event` selects.
fn validate_line(line: &Value) -> TestResult {
    match line["event"].as_str() {
        Some("evidence") => {
            validate("evidence-event.schema.json", line)?;
            validate("visual-candidate.schema.json", &line["record"])
        }
        Some("terminal") => {
            validate("terminal-event.schema.json", line)?;
            validate("operation-response.schema.json", &line["result"])?;
            validate(
                "candidates-stream-data.schema.json",
                &line["result"]["data"],
            )
        }
        _ => Err("a stream line has no published event kind".into()),
    }
}

fn range(from: u64, to: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from),
        MediaTime::from_micros(to),
    )?)
}

fn hex_byte(text: &str) -> Built<u8> {
    Ok(u8::from_str_radix(text, 16)?)
}

/// The recorded F02 samples: its duration, source digest and each window's
/// samples, lead-in included.
/// Each recorded window's ordinal and samples.
type RecordedWindows = Vec<(u32, Vec<VisualSample>)>;

fn recorded_f02() -> Built<(MediaTime, String, RecordedWindows)> {
    let value: Value = serde_json::from_slice(&fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vsift-infrastructure/tests/data/visual_samples/F02.json"),
    )?)?;
    let mut windows = Vec::new();
    for window in value["windows"].as_array().ok_or("no windows")? {
        let mut samples = Vec::new();
        for sample in window["samples"].as_array().ok_or("no samples")? {
            let text = sample["blocks"].as_str().ok_or("no blocks")?;
            let mut blocks = [0_u8; VISUAL_BLOCKS];
            for (position, block) in blocks.iter_mut().enumerate() {
                *block = hex_byte(
                    text.get(position * 2..position * 2 + 2)
                        .ok_or("short blocks")?,
                )?;
            }
            samples.push(VisualSample::from_parts(
                MediaTime::from_micros(sample["time_us"].as_u64().ok_or("no time")?),
                blocks,
                VisualHash::parse_hex(sample["hash"].as_str().ok_or("no hash")?)?,
            ));
        }
        windows.push((
            u32::try_from(window["ordinal"].as_u64().ok_or("no ordinal")?)?,
            samples,
        ));
    }
    Ok((
        MediaTime::from_micros(value["duration_us"].as_u64().ok_or("no duration")?),
        value["source_sha256"]
            .as_str()
            .ok_or("no source")?
            .to_owned(),
        windows,
    ))
}

/// Analyses and identifies one window's samples.
fn analysed(
    scope: &VisualIndexScope<'_>,
    ordinal: u32,
    samples: &[VisualSample],
) -> Built<VisualIndexWindow> {
    let window = VisualWindow::new(ordinal, scope.duration)?;
    let analysis = analyse_window(window, samples, VisualChangePolicy::R0)?;
    let mut candidates = Vec::new();
    for draft in analysis.candidates {
        candidates.push(VisualCandidate::new(
            visual_candidate_id(scope, ordinal, draft.representative)?,
            draft,
        ));
    }
    Ok(VisualIndexWindow::new(
        window,
        VisualWindowOutcome::Analysed {
            sample_count: analysis.sample_count,
            candidates,
            dropped_candidates: analysis.dropped_candidates,
            frameless_cells: analysis.frameless_cells,
        },
        VisualChangePolicy::R0,
    )?)
}

fn index(scope: &VisualIndexScope<'_>, windows: Vec<VisualIndexWindow>) -> Built<VisualIndex> {
    let number = std::num::NonZeroU32::MIN;
    Ok(VisualIndex::new(VisualIndexParts {
        id: visual_index_id(scope, number)?,
        number,
        source_id: scope.source_id.clone(),
        stream_index: scope.stream_index,
        displayed_dimensions: scope.displayed_dimensions,
        duration: scope.duration,
        profile: scope.profile,
        windows,
    })?)
}

/// The F02 index, from its recorded samples (1280x720, one video stream).
fn f02_index(session: &SessionId) -> Built<VisualIndex> {
    let (duration, digest, recorded) = recorded_f02()?;
    let source = SourceId::from_sha256(&digest)?;
    let scope = VisualIndexScope {
        session_id: session,
        source_id: &source,
        stream_index: 0,
        displayed_dimensions: FrameDimensions::new(1280, 720)?,
        duration,
        profile: VisualIndexProfile::R0,
    };
    let mut windows = Vec::new();
    for (ordinal, samples) in &recorded {
        windows.push(analysed(&scope, *ordinal, samples)?);
    }
    index(&scope, windows)
}

/// A synthetic 150 s index: window 0 analysed (a dialog from 5 s to 9 s),
/// window 1 undecodable, window 2 not analysed.
fn partial_index(session: &SessionId) -> Built<VisualIndex> {
    let source = SourceId::from_sha256(SYNTHETIC_DIGEST)?;
    let scope = VisualIndexScope {
        session_id: session,
        source_id: &source,
        stream_index: 0,
        displayed_dimensions: FrameDimensions::new(1440, 900)?,
        duration: MediaTime::from_micros(150 * SECOND),
        profile: VisualIndexProfile::R0,
    };
    let samples: Vec<VisualSample> = (0..120_u64)
        .map(|step| {
            let time = step * SECOND / 2;
            let level = if (5 * SECOND..9 * SECOND).contains(&time) {
                90
            } else {
                40
            };
            VisualSample::from_parts(
                MediaTime::from_micros(time),
                [level; VISUAL_BLOCKS],
                VisualHash::from_bits(u64::from(level)),
            )
        })
        .collect();
    let undecodable = VisualIndexWindow::new(
        VisualWindow::new(1, scope.duration)?,
        VisualWindowOutcome::Undecodable,
        VisualChangePolicy::R0,
    )?;
    index(&scope, vec![analysed(&scope, 0, &samples)?, undecodable])
}

/// One page in both published forms.
struct Page {
    json: Value,
    lines: Vec<String>,
}

fn page(
    session: &SessionId,
    index: &VisualIndex,
    requested: TimeRange,
    limit: u16,
    stop: Option<VisualExtensionStop>,
) -> Built<Page> {
    let page = page_candidates(
        session,
        index,
        &CandidatePageRequest {
            range: requested,
            limit: PageLimit::new(limit)?,
            cursor: None,
        },
        CURSOR_EXPIRES_US,
        NOW_US,
    )?;
    let next_cursor = page
        .next_cursor
        .as_ref()
        .map(vsift_domain::CursorToken::encode);
    let candidates: Vec<VisualCandidate> = page.candidates.into_iter().cloned().collect();
    let segment = whole_file_source_segment(index.source_id(), index.duration())?;
    let analysed = analysed_ranges(index, requested);
    let gaps = visual_coverage_gaps(Some(index), index.duration(), requested, stop);
    let presentation = CandidatesPresentation {
        session_id: session,
        index,
        source_segment_id: segment.id(),
        range: requested,
        searched: TimeRange::new(requested.start(), requested.end().min(index.duration()))?,
        candidates: &candidates,
        next_cursor: next_cursor.as_deref(),
        analysed: &analysed,
        gaps: &gaps,
    };
    let lifecycle = || LifecycleResponse::ephemeral(EXPIRES_AT.to_owned());
    let json = serde_json::to_value(candidates_response(&presentation, lifecycle())?)?;
    let stream = CandidatesEvidenceStream::new(&presentation, lifecycle())?;
    let mut lines = stream
        .records()
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()?;
    lines.push(serde_json::to_string(stream.terminal())?);
    Ok(Page { json, lines })
}

fn f02_page() -> Built<Page> {
    let session = SessionId::parse(SESSION)?;
    let index = f02_index(&session)?;
    page(&session, &index, range(0, 12 * SECOND)?, 20, None)
}

fn partial_page() -> Built<Page> {
    let session = SessionId::parse(SESSION)?;
    let index = partial_index(&session)?;
    page(&session, &index, range(0, 200 * SECOND)?, 2, None)
}

#[test]
fn the_f02_result_matches_the_frozen_example() -> TestResult {
    let page = f02_page()?;
    regenerate(
        JSON_EXAMPLE,
        &format!("{}\n", serde_json::to_string_pretty(&page.json)?),
    )?;
    validate("operation-response.schema.json", &page.json)?;
    validate("candidates-data.schema.json", &page.json["data"])?;
    assert!(
        page.json == load(JSON_EXAMPLE)?,
        "the candidates result differs from {JSON_EXAMPLE}"
    );
    assert_eq!(page.json["status"], "complete");
    assert_eq!(
        page.json["coverage"],
        serde_json::json!({"truncated": false, "gaps": [], "reasons": []})
    );
    let items = page.json["data"]["items"]
        .as_array()
        .ok_or("no items")?
        .clone();
    // F02's frozen truth: slide A [0, 4 s), B [4, 8 s), A again [8, 12 s).
    for (start, end) in [(0, 4), (4, 8), (8, 12)] {
        assert!(
            items.iter().any(|item| item["representative_us"]
                .as_u64()
                .is_some_and(|time| (start * SECOND..end * SECOND).contains(&time))),
            "no candidate in [{start} s, {end} s)"
        );
    }
    // The unchanged third slide is still represented in its next 10 s cell,
    // by a separate candidate with the same visual hash.
    let periodic = items
        .iter()
        .find(|item| item["reasons"] == serde_json::json!(["periodic_coverage"]))
        .ok_or("no periodic candidate")?;
    assert!(
        items
            .iter()
            .any(|item| item["visual_hash"] == periodic["visual_hash"]
                && item["candidate_id"] != periodic["candidate_id"]
                && item["reasons"] == serde_json::json!(["visual_change"]))
    );
    for item in &items {
        validate("visual-candidate.schema.json", item)?;
    }
    Ok(())
}

#[test]
fn the_stream_matches_the_frozen_example_byte_for_byte() -> TestResult {
    let page = f02_page()?;
    let mut emitted = page.lines.join("\n");
    emitted.push('\n');
    regenerate(STREAM_EXAMPLE, &emitted)?;
    let example = read(STREAM_EXAMPLE)?;
    assert!(
        emitted == example.replace("\r\n", "\n"),
        "the emitted stream differs from {STREAM_EXAMPLE}"
    );
    let lines: Vec<Value> = example
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    for line in &lines {
        validate_line(line)?;
    }
    // The records are the --json items, keyed by candidate identity, and the
    // terminal event counts them.
    let items = page.json["data"]["items"].as_array().ok_or("no items")?;
    let (terminal, records) = lines.split_last().ok_or("empty stream")?;
    assert_eq!(records.len(), items.len());
    for ((record, item), sequence) in records.iter().zip(items).zip(0_u64..) {
        assert_eq!(record["record_type"], "visual_candidate");
        assert_eq!(record["key"], item["candidate_id"]);
        assert_eq!(&record["record"], item);
        assert_eq!(record["sequence"], sequence);
    }
    assert_eq!(terminal["sequence"], items.len());
    assert_eq!(terminal["result"]["data"]["record_count"], items.len());
    assert_eq!(terminal["result"]["status"], page.json["status"]);
    assert_eq!(terminal["result"]["coverage"], page.json["coverage"]);
    Ok(())
}

/// A range with an analysed, an undecodable and an unanalysed window is a
/// `partial` result: every gap is typed in `data`, the envelope lists them
/// merged with their distinct reasons, and the range past the video's end is
/// clipped.
#[test]
fn a_range_with_gaps_is_partial_and_matches_the_frozen_example() -> TestResult {
    let page = partial_page()?;
    regenerate(
        PARTIAL_EXAMPLE,
        &format!("{}\n", serde_json::to_string_pretty(&page.json)?),
    )?;
    validate("operation-response.schema.json", &page.json)?;
    validate("candidates-data.schema.json", &page.json["data"])?;
    assert!(
        page.json == load(PARTIAL_EXAMPLE)?,
        "the partial result differs from {PARTIAL_EXAMPLE}"
    );
    assert_eq!(page.json["status"], "partial");
    assert_eq!(
        page.json["warnings"],
        serde_json::json!([VISUAL_COVERAGE_WARNING])
    );
    assert_eq!(
        page.json["coverage"],
        serde_json::json!({
            "truncated": true,
            "gaps": ["60000000-150000000"],
            "reasons": ["not_analyzed", "undecodable"]
        })
    );
    let coverage = &page.json["data"]["coverage"];
    assert_eq!(
        coverage["searched_range"],
        serde_json::json!({"from_us": 0, "to_us": 150_000_000})
    );
    assert_eq!(
        coverage["analyzed"],
        serde_json::json!([{"from_us": 0, "to_us": 60_000_000}])
    );
    assert_eq!(coverage["gaps"][0]["reason"], "undecodable");
    assert_eq!(coverage["gaps"][1]["reason"], "not_analyzed");
    assert_eq!(page.json["data"]["items"].as_array().map(Vec::len), Some(2));
    assert!(page.json["data"]["next_cursor"].is_string());
    Ok(())
}

/// A deadline stop reports the window it stopped at as `deadline_exceeded`
/// rather than `not_analyzed`.
#[test]
fn a_deadline_stop_is_reported_as_its_own_gap() -> TestResult {
    let session = SessionId::parse(SESSION)?;
    let index = partial_index(&session)?;
    let page = page(
        &session,
        &index,
        range(0, 150 * SECOND)?,
        20,
        Some(VisualExtensionStop::Deadline { ordinal: 2 }),
    )?;
    validate("candidates-data.schema.json", &page.json["data"])?;
    assert_eq!(
        page.json["coverage"]["reasons"],
        serde_json::json!(["deadline_exceeded", "undecodable"])
    );
    Ok(())
}

#[test]
fn published_shapes_reject_what_they_do_not_define() -> TestResult {
    let page = f02_page()?;
    let item = page.json["data"]["items"][0].clone();

    let mut extended = item.clone();
    extended
        .as_object_mut()
        .ok_or("not an object")?
        .insert("thumbnail".to_owned(), Value::Bool(true));
    assert!(!is_valid("visual-candidate.schema.json", &extended)?);

    let mut calibrated = item.clone();
    calibrated["stability"] = Value::from("stable");
    assert!(!is_valid("visual-candidate.schema.json", &calibrated)?);

    let mut no_reason = item.clone();
    no_reason["reasons"] = Value::Array(Vec::new());
    assert!(!is_valid("visual-candidate.schema.json", &no_reason)?);

    let records: Vec<Value> = page
        .lines
        .iter()
        .map(|line| serde_json::from_str(line))
        .collect::<Result<_, _>>()?;
    let record = records.first().ok_or("no record")?;
    let mut foreign_key = record.clone();
    foreign_key["key"] = Value::from("tsg_0123456789abcdef");
    assert!(!is_valid("evidence-event.schema.json", &foreign_key)?);

    let mut with_count = page.json["data"].clone();
    with_count
        .as_object_mut()
        .ok_or("not an object")?
        .insert("record_count".to_owned(), Value::from(1));
    assert!(!is_valid("candidates-data.schema.json", &with_count)?);

    let terminal = records.last().ok_or("no terminal")?;
    let mut with_items = terminal["result"]["data"].clone();
    with_items
        .as_object_mut()
        .ok_or("not an object")?
        .insert("items".to_owned(), Value::Array(Vec::new()));
    assert!(!is_valid(
        "candidates-stream-data.schema.json",
        &with_items
    )?);
    Ok(())
}
