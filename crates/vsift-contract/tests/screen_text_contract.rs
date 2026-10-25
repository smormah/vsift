//! Conformance of the on-screen text contract (OCR, ADR 0030): the `ocr`
//! result, complete and partial, and the `screen_text` member of a `search`
//! first page in its full, brief and stream forms, against the published v1
//! schemas and the frozen examples `examples/ocr.json`,
//! `examples/ocr.partial.json` and `examples/search.screen-text.json`.
//!
//! The index is built here: a 15 s video read every 5 s plus one visual
//! candidate at 6 s, whose frames at 5 s and 6 s show `Dialog R-17` in the
//! same place and whose frame at 10 s shows `Close R-17` elsewhere.

use std::{fs, io, num::NonZeroU32, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    ImportedRevisionRequest, ScreenTextStop, SearchPageRequest, SuppliedTranscript,
    build_imported_revision, page_search, screen_text_index_id, screen_text_line_id,
};
use vsift_contract::{
    EvidenceStream, LifecycleResponse, OCR_FAILED_REMEDIATION, OCR_RUNNER_REMEDIATION, ResultView,
    SCREEN_TEXT_PARTIAL_WARNING, ScreenTextHitPresentation, ScreenTextPresentation,
    ScreenTextSearchPresentation, ScreenTextUnavailable, SearchEvidenceStream, SearchPresentation,
    UnreadableIndex, UnreadableReason, screen_text_response, search_response,
    search_response_with_view,
};
use vsift_domain::{
    Confidence, ConfidenceOrigin, CoverageGapReason, CueSource, CueText, CueTiming,
    FrameDimensions, FrameRef, ImportedCue, MediaTime, PageLimit, ParsedTranscript, ProviderLabel,
    ReadTarget, ScreenText, ScreenTextCoverage, ScreenTextFrame, ScreenTextIndex,
    ScreenTextIndexParts, ScreenTextLine, ScreenTextOutcome, ScreenTextProfile, ScreenTextProvider,
    ScreenTextScope, SearchQuery, SessionId, Sha256Hex, SidecarIdentity, SourceId, TextRegion,
    TimeBase, TimeRange, TranscriptFormat, TranscriptOffset, TranscriptRevision,
    TranscriptWarnings, VisualCandidateId, VisualCoverageGap, plan_reads, screen_text_occurrences,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const SOURCE: &str = "src_sha256_d7ccece71288c5ff35d7b16c871a93a8ed48bbb7380617899575069b4d6545f4";
const EXPIRES_AT: &str = "2026-09-25T00:00:00Z";
const CURSOR_EXPIRES_US: u64 = 1_790_294_400_000_000;
const NOW_US: u64 = 1_790_208_000_000_000;
const SECOND: u64 = 1_000_000;
const PERIOD: u64 = 5 * SECOND;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const OCR_EXAMPLE: &str = "examples/ocr.json";
const OCR_PARTIAL_EXAMPLE: &str = "examples/ocr.partial.json";
const SEARCH_EXAMPLE: &str = "examples/search.screen-text.json";

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative_path: &str) -> Built<Value> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative_path),
    )?)?)
}

/// Writes `value` as the example when `VSIFT_REGENERATE_CONTRACT_EXAMPLES=1`.
fn regenerate(relative_path: &str, value: &Value) -> TestResult {
    if std::env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|value| value == "1") {
        let mut text = serde_json::to_string_pretty(value)?;
        text.push('\n');
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

fn range(from: u64, to: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from),
        MediaTime::from_micros(to),
    )?)
}

fn candidate() -> Built<VisualCandidateId> {
    Ok(VisualCandidateId::parse(format!("vcd_{}", "c".repeat(32)))?)
}

fn scope() -> Built<ScreenTextScope> {
    let digest = |fill: char| Sha256Hex::parse(fill.to_string().repeat(64));
    Ok(ScreenTextScope {
        session_id: SessionId::parse(SESSION)?,
        source_id: SourceId::parse(SOURCE)?,
        stream_index: 0,
        profile: ScreenTextProfile::PpOcrV5LatinV1,
        provider: ScreenTextProvider {
            runner_sha256: digest('1')?,
            runner_version: ProviderLabel::parse("0.1.0")?,
            runtime_version: ProviderLabel::parse("1.31.0")?,
            detector_sha256: digest('2')?,
            recognizer_sha256: digest('3')?,
        },
    })
}

fn frame(pts: i64) -> Built<FrameRef> {
    Ok(FrameRef {
        stream_index: 0,
        time_base: TimeBase::new(1, 30)?,
        pts,
        time: MediaTime::from_micros(u64::try_from(pts)? * SECOND / 30),
        dimensions: FrameDimensions::new(1280, 720)?,
    })
}

/// A line as read: its region (x, y, width, height), text and score.
type ReadLine<'a> = ((u32, u32, u32, u32), &'a str, u16);

fn read_frame(
    scope: &ScreenTextScope,
    pts: i64,
    targets: Vec<ReadTarget>,
    lines: &[ReadLine<'_>],
) -> Built<ScreenTextFrame> {
    let mut read = Vec::new();
    for (ordinal, ((x, y, width, height), text, basis_points)) in (1..).zip(lines) {
        read.push(ScreenTextLine {
            id: screen_text_line_id(scope, pts, ordinal)?,
            region: TextRegion::new(*x, *y, *width, *height, FrameDimensions::new(1280, 720)?)?,
            text: ScreenText::parse(*text)?,
            confidence: Confidence::provider_score(
                *basis_points,
                ConfidenceOrigin::ProviderUncalibrated,
            )?,
        });
    }
    Ok(ScreenTextFrame {
        frame: frame(pts)?,
        targets,
        outcome: ScreenTextOutcome::Read {
            lines: read,
            truncated: false,
        },
    })
}

fn periodic(seconds: u64) -> ReadTarget {
    ReadTarget {
        time: MediaTime::from_micros(seconds * SECOND),
        candidate: None,
    }
}

/// The plan of the whole 15 s video: 0, 5 and 10 s, and the candidate at 6 s.
fn plan() -> Built<Vec<ReadTarget>> {
    Ok(plan_reads(
        range(0, 15 * SECOND)?,
        &[(candidate()?, MediaTime::from_micros(6 * SECOND))],
        PERIOD,
    )?)
}

/// The index after reading the frames of the plan up to `through_seconds`.
fn index(through_seconds: u64) -> Built<ScreenTextIndex> {
    let scope = scope()?;
    let dialog = ((420, 300, 360, 32), "Dialog R-17", 9_731);
    let mut frames = vec![read_frame(&scope, 0, vec![periodic(0)], &[])?];
    if through_seconds >= 5 {
        frames.push(read_frame(&scope, 150, vec![periodic(5)], &[dialog])?);
    }
    if through_seconds >= 6 {
        frames.push(read_frame(
            &scope,
            180,
            vec![ReadTarget {
                time: MediaTime::from_micros(6 * SECOND),
                candidate: Some(candidate()?),
            }],
            &[((424, 301, 356, 31), "Dialog R-17", 9_802)],
        )?);
    }
    if through_seconds >= 10 {
        frames.push(read_frame(
            &scope,
            300,
            vec![periodic(10)],
            &[
                ((40, 28, 300, 14), "Settings", 9_950),
                ((1_000, 660, 200, 24), "Close R-17", 8_412),
            ],
        )?);
    }
    let number = NonZeroU32::new(1).ok_or("revision")?;
    Ok(ScreenTextIndex::new(ScreenTextIndexParts {
        id: screen_text_index_id(&scope, number)?,
        revision: number,
        scope,
        period_micros: PERIOD,
        frames,
        unresolved: Vec::new(),
    })?)
}

fn lifecycle() -> LifecycleResponse {
    LifecycleResponse::ephemeral(EXPIRES_AT.to_owned())
}

#[test]
fn a_wholly_read_range_is_a_complete_ocr_result() -> TestResult {
    let index = index(10)?;
    let session = SessionId::parse(SESSION)?;
    let coverage = ScreenTextCoverage::of(&plan()?, Some(&index));
    let response = serde_json::to_value(screen_text_response(
        &ScreenTextPresentation {
            session_id: &session,
            index: Some(&index),
            range: range(0, 15 * SECOND)?,
            period_micros: PERIOD,
            coverage,
            candidates_complete: true,
            read_now: 2,
            stop: None,
            visual_gaps: &[],
        },
        lifecycle(),
    )?)?;
    regenerate(OCR_EXAMPLE, &response)?;
    validate("operation-response.schema.json", &response)?;
    validate("screen-text-data.schema.json", &response["data"])?;
    assert!(
        response == load(OCR_EXAMPLE)?,
        "the ocr result differs from {OCR_EXAMPLE}"
    );
    assert_eq!(response["command"], "ocr");
    assert_eq!(response["status"], "complete");
    assert_eq!(response["warnings"], serde_json::json!([]));
    assert_eq!(
        response["coverage"],
        serde_json::json!({"truncated": false, "gaps": [], "reasons": []})
    );
    let data = &response["data"];
    assert_eq!(data["coverage"]["planned"], 4);
    assert_eq!(data["coverage"]["read"], 4);
    assert_eq!(data["coverage"]["pending"], 0);
    assert_eq!(data["index"]["frames"], 4);
    assert_eq!(data["index"]["lines"], 4);
    assert_eq!(data["index"]["engine"]["profile"], "ppocrv5-latin-v1");
    assert_eq!(data["stopped"], Value::Null);
    Ok(())
}

#[test]
fn a_call_stopped_by_its_frame_limit_is_partial_and_names_the_stop() -> TestResult {
    let index = index(5)?;
    let session = SessionId::parse(SESSION)?;
    let gaps = [VisualCoverageGap {
        range: range(12 * SECOND, 15 * SECOND)?,
        reason: CoverageGapReason::DeadlineExceeded,
        dropped_candidates: 0,
    }];
    let response = serde_json::to_value(screen_text_response(
        &ScreenTextPresentation {
            session_id: &session,
            index: Some(&index),
            range: range(0, 15 * SECOND)?,
            period_micros: PERIOD,
            coverage: ScreenTextCoverage::of(&plan()?, Some(&index)),
            candidates_complete: false,
            read_now: 2,
            stop: Some(ScreenTextStop::FrameLimit),
            visual_gaps: &gaps,
        },
        lifecycle(),
    )?)?;
    regenerate(OCR_PARTIAL_EXAMPLE, &response)?;
    validate("operation-response.schema.json", &response)?;
    validate("screen-text-data.schema.json", &response["data"])?;
    assert!(
        response == load(OCR_PARTIAL_EXAMPLE)?,
        "the partial ocr result differs from {OCR_PARTIAL_EXAMPLE}"
    );
    assert_eq!(response["status"], "partial");
    assert_eq!(
        response["warnings"],
        serde_json::json!([SCREEN_TEXT_PARTIAL_WARNING])
    );
    assert_eq!(
        response["coverage"],
        serde_json::json!({
            "truncated": true,
            "gaps": ["12000000-15000000"],
            "reasons": ["frames_pending", "deadline_exceeded"]
        })
    );
    let data = &response["data"];
    assert_eq!(data["stopped"], "frame_limit");
    assert_eq!(data["coverage"]["read"], 2);
    assert_eq!(data["coverage"]["pending"], 2);
    assert_eq!(data["coverage"]["candidates_complete"], false);
    assert_eq!(data["candidate_gaps"][0]["reason"], "deadline_exceeded");
    Ok(())
}

#[test]
fn a_deadline_stop_with_frames_pending_is_partial() -> TestResult {
    let index = index(0)?;
    let session = SessionId::parse(SESSION)?;
    let response = serde_json::to_value(screen_text_response(
        &ScreenTextPresentation {
            session_id: &session,
            index: Some(&index),
            range: range(0, 15 * SECOND)?,
            period_micros: PERIOD,
            coverage: ScreenTextCoverage::of(&plan()?, Some(&index)),
            candidates_complete: true,
            read_now: 1,
            stop: Some(ScreenTextStop::Deadline),
            visual_gaps: &[],
        },
        lifecycle(),
    )?)?;
    validate("screen-text-data.schema.json", &response["data"])?;
    assert_eq!(response["status"], "partial");
    assert_eq!(response["data"]["stopped"], "deadline_exceeded");
    Ok(())
}

fn revision() -> Built<TranscriptRevision> {
    let cue = ImportedCue {
        source: CueSource::new(NonZeroU32::MIN, NonZeroU32::MIN),
        timing: CueTiming::new(4_500_000, 8_500_000)?,
        text: CueText::new(
            "Dialog R-17 is displayed now.".to_owned(),
            "Dialog R-17 is displayed now.".to_owned(),
        )?,
        speaker: None,
    };
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::Srt,
            None,
            vec![cue],
            TranscriptWarnings::default(),
        )?,
        sidecar: SidecarIdentity::new("a".repeat(64), 64)?,
    };
    Ok(build_imported_revision(ImportedRevisionRequest {
        session_id: &SessionId::parse(SESSION)?,
        source_id: &SourceId::parse(SOURCE)?,
        source_duration: MediaTime::from_micros(15 * SECOND),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(0)?,
        number: NonZeroU32::MIN,
    })?)
}

/// One search first page of `query` with the screen-text part `index` gives,
/// in the full, brief and stream forms.
struct Page {
    json: Value,
    brief: Value,
    terminal: Value,
}

fn search(query: &str, index: Option<&ScreenTextIndex>, limit: u16) -> Built<Page> {
    search_with(query, index, limit, None)
}

/// [`search`], with an index the member could not read.
fn search_with(
    query: &str,
    index: Option<&ScreenTextIndex>,
    limit: u16,
    unavailable: Option<ScreenTextUnavailable>,
) -> Built<Page> {
    let session = SessionId::parse(SESSION)?;
    let revision = revision()?;
    let query = SearchQuery::parse(query)?;
    let page = page_search(
        &session,
        &revision,
        &SearchPageRequest {
            query: query.clone(),
            range: None,
            limit: PageLimit::new(limit)?,
            cursor: None,
        },
        CURSOR_EXPIRES_US,
        NOW_US,
    )?;
    let occurrences = index.map_or_else(Vec::new, |index| {
        screen_text_occurrences(index, &query, None)
    });
    let candidates: Vec<Vec<VisualCandidateId>> = occurrences
        .iter()
        .map(|hit| {
            hit.first
                .targets
                .iter()
                .filter_map(|target| target.candidate.clone())
                .collect()
        })
        .collect();
    let hits = occurrences
        .iter()
        .zip(&candidates)
        .take(usize::from(limit))
        .map(|(hit, candidates)| ScreenTextHitPresentation {
            line: hit.line,
            frame: &hit.first.frame,
            candidates,
            last_time: hit.last_time,
            frames: hit.frames,
            tier: hit.tier,
        })
        .collect();
    let screen_text = ScreenTextSearchPresentation {
        index,
        hits,
        total_hits: occurrences.len(),
        coverage: index.map_or_else(ScreenTextCoverage::default, |index| {
            plan().map_or_else(
                |_| ScreenTextCoverage::default(),
                |plan| ScreenTextCoverage::of(&plan, Some(index)),
            )
        }),
        candidates_complete: index.is_some() && unavailable.is_none(),
        period_micros: index.map(ScreenTextIndex::period_micros),
        unavailable,
    };
    let next_cursor = page
        .next_cursor
        .as_ref()
        .map(vsift_domain::CursorToken::encode);
    let presentation = SearchPresentation {
        session_id: &session,
        revision: &revision,
        query: &query,
        range: None,
        hits: page
            .hits
            .iter()
            .map(|hit| (hit.segment(), hit.tier()))
            .collect(),
        next_cursor: next_cursor.as_deref(),
        coverage: &page.coverage,
        screen_text: Some(screen_text),
    };
    let json = serde_json::to_value(search_response(&presentation, lifecycle())?)?;
    let brief = serde_json::to_value(search_response_with_view(
        &presentation,
        lifecycle(),
        ResultView::Brief,
    )?)?;
    let stream = SearchEvidenceStream::new(&presentation, lifecycle())?;
    let terminal = serde_json::to_value(stream.terminal())?;
    Ok(Page {
        json,
        brief,
        terminal,
    })
}

#[test]
fn a_search_first_page_cites_each_on_screen_hit_by_frame_time_and_region() -> TestResult {
    let index = index(10)?;
    let page = search("R-17", Some(&index), 20)?;
    regenerate(SEARCH_EXAMPLE, &page.json)?;
    validate("operation-response.schema.json", &page.json)?;
    validate("search-data.schema.json", &page.json["data"])?;
    assert!(
        page.json == load(SEARCH_EXAMPLE)?,
        "the search result differs from {SEARCH_EXAMPLE}"
    );
    // The envelope is the transcript's: on-screen text never changes it.
    assert_eq!(page.json["status"], "complete");
    let part = &page.json["data"]["screen_text"];
    assert_eq!(part["state"], "complete");
    assert_eq!(part["total_hits"], 2);
    // The dialog stayed in place over the frames at 5 s and 6 s: one hit.
    let first = &part["hits"][0];
    assert_eq!(first["text"], "Dialog R-17");
    assert_eq!(first["match"], "phrase");
    assert_eq!(first["frame"]["pts"], 150);
    assert_eq!(first["frame"]["time_us"], 5 * SECOND);
    assert_eq!(first["last_time_us"], 6 * SECOND);
    assert_eq!(first["frames"], 2);
    assert_eq!(
        first["region"],
        serde_json::json!({"x": 420, "y": 300, "width": 360, "height": 32})
    );
    assert_eq!(first["candidate_ids"], serde_json::json!([]));
    assert_eq!(
        first["confidence"],
        serde_json::json!({"value_basis_points": 9_731, "origin": "provider_uncalibrated"})
    );
    assert_eq!(part["hits"][1]["text"], "Close R-17");
    assert_eq!(part["hits"][1]["frame"]["time_us"], 10 * SECOND);

    validate("search-brief-data.schema.json", &page.brief["data"])?;
    assert_eq!(page.brief["data"]["screen_text"], *part);
    validate("terminal-event.schema.json", &page.terminal)?;
    validate(
        "search-stream-data.schema.json",
        &page.terminal["result"]["data"],
    )?;
    assert_eq!(page.terminal["result"]["data"]["screen_text"], *part);
    Ok(())
}

#[test]
fn a_session_ocr_has_not_read_says_not_run_and_finds_nothing() -> TestResult {
    let page = search("R-17", None, 20)?;
    validate("search-data.schema.json", &page.json["data"])?;
    let part = &page.json["data"]["screen_text"];
    assert_eq!(part["state"], "not_run");
    assert_eq!(part["index"], Value::Null);
    assert_eq!(part["hits"], serde_json::json!([]));
    assert_eq!(part["coverage"]["period_us"], Value::Null);
    assert_eq!(page.json["status"], "complete");
    Ok(())
}

const SEARCH_UNAVAILABLE_EXAMPLE: &str = "examples/search.screen-text-unavailable.json";

/// RV-2 finding 2: an index the member cannot read is named in it, with why,
/// and the search is the transcript's exactly as without OCR. A screen-text
/// index that cannot be read leaves the member `unavailable` with nothing
/// known; a visual index that cannot be read leaves the hits, `partial`.
#[test]
fn an_unreadable_index_is_named_in_the_member_and_never_changes_the_search() -> TestResult {
    let without = search("R-17", None, 20)?;
    let page = search_with(
        "R-17",
        None,
        20,
        Some(ScreenTextUnavailable {
            index: UnreadableIndex::ScreenText,
            reason: UnreadableReason::IntegrityFailure,
        }),
    )?;
    regenerate(SEARCH_UNAVAILABLE_EXAMPLE, &page.json)?;
    validate("operation-response.schema.json", &page.json)?;
    validate("search-data.schema.json", &page.json["data"])?;
    assert!(
        page.json == load(SEARCH_UNAVAILABLE_EXAMPLE)?,
        "the search result differs from {SEARCH_UNAVAILABLE_EXAMPLE}"
    );
    let part = &page.json["data"]["screen_text"];
    assert_eq!(part["state"], "unavailable");
    assert_eq!(
        part["unavailable"],
        serde_json::json!({"index": "screen_text", "reason": "integrity_failure"})
    );
    let mut transcript_only = page.json.clone();
    let mut expected = without.json.clone();
    for value in [&mut transcript_only, &mut expected] {
        if let Some(data) = value["data"].as_object_mut() {
            data.remove("screen_text");
        }
    }
    assert_eq!(transcript_only, expected);
    validate("search-brief-data.schema.json", &page.brief["data"])?;
    validate(
        "search-stream-data.schema.json",
        &page.terminal["result"]["data"],
    )?;

    let index = index(10)?;
    let page = search_with(
        "R-17",
        Some(&index),
        20,
        Some(ScreenTextUnavailable {
            index: UnreadableIndex::Visual,
            reason: UnreadableReason::UnsupportedVersion,
        }),
    )?;
    validate("search-data.schema.json", &page.json["data"])?;
    let part = &page.json["data"]["screen_text"];
    assert_eq!(part["state"], "partial");
    assert_eq!(part["total_hits"], 2);
    assert_eq!(
        part["unavailable"],
        serde_json::json!({"index": "visual", "reason": "unsupported_version"})
    );
    assert_eq!(page.json["status"], "complete");
    // A page whose indexes were read carries no `unavailable` member.
    assert!(
        search("R-17", Some(&index), 20)?.json["data"]["screen_text"]
            .get("unavailable")
            .is_none()
    );
    Ok(())
}

#[test]
fn hits_are_bounded_by_the_page_while_total_hits_counts_them_all() -> TestResult {
    let index = index(10)?;
    let page = search("R-17", Some(&index), 1)?;
    validate("search-data.schema.json", &page.json["data"])?;
    let part = &page.json["data"]["screen_text"];
    assert_eq!(part["hits"].as_array().map(Vec::len), Some(1));
    assert_eq!(part["total_hits"], 2);
    Ok(())
}

#[test]
fn a_partly_read_index_is_partial_in_search() -> TestResult {
    let index = index(5)?;
    let page = search("Dialog", Some(&index), 20)?;
    validate("search-data.schema.json", &page.json["data"])?;
    let part = &page.json["data"]["screen_text"];
    assert_eq!(part["state"], "partial");
    assert_eq!(part["coverage"]["pending"], 2);
    assert_eq!(page.json["status"], "complete");
    Ok(())
}

#[test]
fn screen_text_schemas_reject_unknown_fields_values_and_unbounded_text() -> TestResult {
    let ocr = load(OCR_EXAMPLE)?;
    let mut extended = ocr["data"].clone();
    extended["pixels"] = Value::Bool(true);
    assert!(!is_valid("screen-text-data.schema.json", &extended)?);
    let mut stopped = ocr["data"].clone();
    stopped["stopped"] = "tired".into();
    assert!(!is_valid("screen-text-data.schema.json", &stopped)?);
    let mut scoped = ocr["data"].clone();
    scoped["coverage"]["scope"] = "transcript_text".into();
    assert!(!is_valid("screen-text-data.schema.json", &scoped)?);

    let search = load(SEARCH_EXAMPLE)?;
    let mut long = search["data"].clone();
    long["screen_text"]["hits"][0]["text"] = "x".repeat(513).into();
    assert!(!is_valid("search-data.schema.json", &long)?);
    let mut calibrated = search["data"].clone();
    calibrated["screen_text"]["hits"][0]["confidence"]["origin"] = "provider_calibrated".into();
    assert!(!is_valid("search-data.schema.json", &calibrated)?);
    let mut state = search["data"].clone();
    state["screen_text"]["state"] = "unknown".into();
    assert!(!is_valid("search-data.schema.json", &state)?);
    let mut reason = load(SEARCH_UNAVAILABLE_EXAMPLE)?["data"].clone();
    reason["screen_text"]["unavailable"]["reason"] = "tired".into();
    assert!(!is_valid("search-data.schema.json", &reason)?);
    // Older pages, without the member, stay valid.
    let mut older = search["data"].clone();
    if let Some(data) = older.as_object_mut() {
        data.remove("screen_text");
    }
    assert!(is_valid("search-data.schema.json", &older)?);
    Ok(())
}

#[test]
fn ocr_remediations_name_the_runner_and_ask_rather_than_search() {
    assert!(OCR_RUNNER_REMEDIATION.contains("setup configure ocr-runner"));
    assert!(OCR_RUNNER_REMEDIATION.contains("ask the user"));
    assert!(OCR_FAILED_REMEDIATION.contains("Nothing was committed"));
}

const OCR_SEARCH_EXAMPLE: &str = "examples/ocr-search.json";
const OCR_SEARCH_NOT_RUN_EXAMPLE: &str = "examples/ocr-search.not-run.json";

/// One `ocr search` page of `query` over `index` (none: `ocr` has not run).
fn ocr_search(
    query: &str,
    index: Option<&ScreenTextIndex>,
    limit: u16,
    cursor: Option<String>,
) -> Built<Value> {
    let session = SessionId::parse(SESSION)?;
    let query = SearchQuery::parse(query)?;
    let page = match index {
        Some(index) => Some(vsift_application::page_screen_text(
            &session,
            index,
            &vsift_application::ScreenTextPageRequest {
                query: query.clone(),
                range: None,
                limit: PageLimit::new(limit)?,
                cursor,
            },
            CURSOR_EXPIRES_US,
            NOW_US,
        )?),
        None => None,
    };
    let candidates: Vec<Vec<VisualCandidateId>> = page
        .iter()
        .flat_map(|page| page.hits.iter())
        .map(|hit| {
            hit.first
                .targets
                .iter()
                .filter_map(|target| target.candidate.clone())
                .collect()
        })
        .collect();
    let hits = page
        .iter()
        .flat_map(|page| page.hits.iter())
        .zip(&candidates)
        .map(|(hit, candidates)| ScreenTextHitPresentation {
            line: hit.line,
            frame: &hit.first.frame,
            candidates,
            last_time: hit.last_time,
            frames: hit.frames,
            tier: hit.tier,
        })
        .collect();
    let next_cursor = page
        .as_ref()
        .and_then(|page| page.next_cursor.as_ref())
        .map(vsift_domain::CursorToken::encode);
    let coverage = match index {
        Some(index) => ScreenTextCoverage::of(&plan()?, Some(index)),
        None => ScreenTextCoverage::default(),
    };
    Ok(serde_json::to_value(
        vsift_contract::screen_text_search_response(
            &vsift_contract::ScreenTextSearchPagePresentation {
                session_id: &session,
                query: &query,
                range: None,
                part: ScreenTextSearchPresentation {
                    index,
                    hits,
                    total_hits: page.as_ref().map_or(0, |page| page.total_hits),
                    coverage,
                    candidates_complete: index.is_some(),
                    period_micros: index.map(ScreenTextIndex::period_micros),
                    unavailable: None,
                },
                next_cursor: next_cursor.as_deref(),
            },
            lifecycle(),
        )?,
    )?)
}

#[test]
fn ocr_search_pages_every_on_screen_hit_without_a_transcript() -> TestResult {
    let index = index(10)?;
    let first = ocr_search("R-17", Some(&index), 1, None)?;
    regenerate(OCR_SEARCH_EXAMPLE, &first)?;
    validate("operation-response.schema.json", &first)?;
    validate("ocr-search-data.schema.json", &first["data"])?;
    assert!(
        first == load(OCR_SEARCH_EXAMPLE)?,
        "the ocr search result differs from {OCR_SEARCH_EXAMPLE}"
    );
    assert_eq!(first["command"], "ocr.search");
    assert_eq!(first["status"], "complete");
    let data = &first["data"];
    assert_eq!(data["state"], "complete");
    assert_eq!(data["total_hits"], 2);
    assert_eq!(data["hits"][0]["text"], "Dialog R-17");
    let cursor = data["next_cursor"].as_str().ok_or("no cursor")?.to_owned();
    let second = ocr_search("R-17", Some(&index), 1, Some(cursor))?;
    validate("ocr-search-data.schema.json", &second["data"])?;
    assert_eq!(second["data"]["hits"][0]["text"], "Close R-17");
    assert_eq!(second["data"]["next_cursor"], Value::Null);
    assert_eq!(second["data"]["total_hits"], 2);
    Ok(())
}

#[test]
fn ocr_search_before_ocr_is_partial_and_says_not_run() -> TestResult {
    let response = ocr_search("R-17", None, 20, None)?;
    regenerate(OCR_SEARCH_NOT_RUN_EXAMPLE, &response)?;
    validate("operation-response.schema.json", &response)?;
    validate("ocr-search-data.schema.json", &response["data"])?;
    assert!(
        response == load(OCR_SEARCH_NOT_RUN_EXAMPLE)?,
        "the ocr search result differs from {OCR_SEARCH_NOT_RUN_EXAMPLE}"
    );
    assert_eq!(response["status"], "partial");
    assert_eq!(response["data"]["state"], "not_run");
    assert_eq!(
        response["coverage"],
        serde_json::json!({"truncated": true, "gaps": [], "reasons": ["not_run"]})
    );
    assert_eq!(
        response["warnings"],
        serde_json::json!([vsift_contract::OCR_SEARCH_PARTIAL_WARNING])
    );
    Ok(())
}

#[test]
fn a_partly_read_ocr_search_names_what_is_missing() -> TestResult {
    let index = index(5)?;
    let response = ocr_search("Dialog", Some(&index), 20, None)?;
    validate("ocr-search-data.schema.json", &response["data"])?;
    assert_eq!(response["status"], "partial");
    assert_eq!(
        response["coverage"]["reasons"],
        serde_json::json!(["frames_pending"])
    );
    Ok(())
}

/// ADR 0033's analysed-windows-only call: reads waiting for their windows
/// keep the result partial, under their own reason and count.
#[test]
fn reads_awaiting_analysis_are_counted_apart_and_keep_the_result_partial() -> TestResult {
    let index = index(10)?;
    let session = SessionId::parse(SESSION)?;
    let response = serde_json::to_value(screen_text_response(
        &ScreenTextPresentation {
            session_id: &session,
            index: Some(&index),
            range: range(0, 15 * SECOND)?,
            period_micros: PERIOD,
            coverage: ScreenTextCoverage {
                awaiting_analysis: 3,
                ..ScreenTextCoverage::of(&plan()?, Some(&index))
            },
            candidates_complete: true,
            read_now: 0,
            stop: None,
            visual_gaps: &[],
        },
        lifecycle(),
    )?)?;
    validate("screen-text-data.schema.json", &response["data"])?;
    assert_eq!(response["status"], "partial");
    assert_eq!(response["data"]["coverage"]["awaiting_analysis"], 3);
    assert_eq!(response["data"]["coverage"]["pending"], 0);
    assert_eq!(
        response["coverage"]["reasons"],
        serde_json::json!([vsift_contract::FRAMES_AWAITING_ANALYSIS_REASON])
    );
    Ok(())
}
