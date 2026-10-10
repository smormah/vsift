//! Conformance of the `search` contract (P08, ADR 0018): the `--json` result,
//! the `--events jsonl` stream of matching `transcript_segment` records ended
//! by one terminal event with the hit list, the envelope coverage and
//! `partial` status of an incompletely transcribed range, and the fixed prose
//! of a rejected query, against the published v1 schemas and the frozen
//! examples `examples/search.json` and `examples/search.events.jsonl`.
//!
//! The examples search the F10 fixture imported from
//! `fixtures/corpus/transcripts/F10.srt` with the explicit +500 ms offset for
//! `R-17`, which finds the segment F10-E01 cites.

use std::{fs, io, num::NonZeroU16, num::NonZeroU32, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    AsrRevisionRequest, AsrTranscription, ImportedRevisionRequest, SearchPageRequest,
    SuppliedTranscript, build_asr_revision, build_imported_revision, page_search,
    whole_file_source_segment,
};
use vsift_contract::{
    CommandName, EvidenceStream, LifecycleResponse, OperationResponse, ResultView,
    SearchEvidenceStream, SearchPresentation, UNTRANSCRIBED_SEARCH_WARNING,
    search_query_rejection_summary, search_response, search_response_with_view,
};
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, ChunkTime, CueSource, CueText, CueTiming,
    FailureCode, ImportedCue, MediaTime, PageLimit, ParsedTranscript, ProviderChunkOutput,
    ProviderSegment, ProviderToken, ProviderTokenKind, SearchQuery, SearchQueryRejection,
    SessionId, Sha256Hex, SidecarIdentity, SourceId, TimeRange, TranscriptFormat, TranscriptOffset,
    TranscriptRevision, TranscriptWarnings, merge_chunks, plan_chunks, validate_chunk_output,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const F10_SOURCE: &str =
    "src_sha256_d7ccece71288c5ff35d7b16c871a93a8ed48bbb7380617899575069b4d6545f4";
const F10_SRT_SHA256: &str = "2a4ee37d826754eac43da03e7cc63f718b16aed4d3ce163c380ebd8ab9630847";
const F10_SRT_BYTES: u64 = 245;
const DIGEST: &str = "95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d";
const EXPIRES_AT: &str = "2026-09-25T00:00:00Z";
const CURSOR_EXPIRES_US: u64 = 1_790_294_400_000_000;
const NOW_US: u64 = 1_790_208_000_000_000;
const SECOND: u64 = 1_000_000;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const JSON_EXAMPLE: &str = "examples/search.json";
const STREAM_EXAMPLE: &str = "examples/search.events.jsonl";

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn read(relative_path: &str) -> Built<String> {
    Ok(fs::read_to_string(schema_root().join(relative_path))?)
}

fn load(relative_path: &str) -> Built<Value> {
    Ok(serde_json::from_str(&read(relative_path)?)?)
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

/// Validates one search stream line against the schemas its `event` selects.
fn validate_line(line: &Value) -> TestResult {
    match line["event"].as_str() {
        Some("evidence") => {
            validate("evidence-event.schema.json", line)?;
            validate("transcript-segment.schema.json", &line["record"])
        }
        Some("terminal") => {
            validate("terminal-event.schema.json", line)?;
            validate("operation-response.schema.json", &line["result"])?;
            if matches!(
                line["result"]["status"].as_str(),
                Some("complete" | "partial")
            ) {
                validate("search-stream-data.schema.json", &line["result"]["data"])?;
            }
            Ok(())
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

fn f10_revision() -> Built<TranscriptRevision> {
    let mut cues = Vec::new();
    for (ordinal, line, start, end, text) in [
        (
            1,
            2,
            500_000,
            3_500_000,
            "This synthetic sidecar is aligned with\nan explicit 500 millisecond offset.",
        ),
        (2, 7, 4_500_000, 8_500_000, "Dialog R-17 is displayed now."),
        (
            3,
            11,
            9_000_000,
            11_000_000,
            "End of the synthetic imported transcript.",
        ),
    ] {
        cues.push(ImportedCue {
            source: CueSource::new(
                NonZeroU32::new(ordinal).ok_or("zero ordinal")?,
                NonZeroU32::new(line).ok_or("zero line")?,
            ),
            timing: CueTiming::new(start, end)?,
            text: CueText::new(text.to_owned(), text.to_owned())?,
            speaker: None,
        });
    }
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::Srt,
            None,
            cues,
            TranscriptWarnings::default(),
        )?,
        sidecar: SidecarIdentity::new(F10_SRT_SHA256, F10_SRT_BYTES)?,
    };
    Ok(build_imported_revision(ImportedRevisionRequest {
        session_id: &SessionId::parse(SESSION)?,
        source_id: &SourceId::parse(F10_SOURCE)?,
        source_duration: MediaTime::from_micros(12_000_000),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?)
}

/// A local-ASR revision of a 12 s source whose one run transcribed only its
/// first 6 s, from the recorded F01 whisper.cpp output, so 6-12 s has no
/// transcript.
fn half_transcribed_revision() -> Built<TranscriptRevision> {
    let session = SessionId::parse(SESSION)?;
    let source_id = SourceId::from_sha256(DIGEST)?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(12 * SECOND))?;
    let audio = range(0, 6 * SECOND)?;
    let planned = plan_chunks(source.id(), audio, ChunkPlan::R0)?;
    let first = planned.first().ok_or("no chunk")?.clone();
    let raw: Value = serde_json::from_slice(&fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vsift-infrastructure/tests/fixtures/whisper-1.9.2/F01.base.json"),
    )?)?;
    let mut segments = Vec::new();
    for segment in raw["transcription"].as_array().ok_or("no segments")? {
        let text = segment["text"].as_str().ok_or("no text")?.trim().to_owned();
        let mut tokens = Vec::new();
        for token in segment["tokens"].as_array().ok_or("no tokens")? {
            tokens.push(ProviderToken {
                kind: ProviderTokenKind::classify(token["text"].as_str().ok_or("no token")?),
                probability: token["p"].as_f64().ok_or("no probability")?,
            });
        }
        segments.push(ProviderSegment {
            start: ChunkTime::from_millis(segment["offsets"]["from"].as_u64().ok_or("from")?)
                .ok_or("time")?,
            end: ChunkTime::from_millis(segment["offsets"]["to"].as_u64().ok_or("to")?)
                .ok_or("time")?,
            text: Some(CueText::new(text.clone(), text)?),
            tokens,
        });
    }
    let output = ProviderChunkOutput {
        language: None,
        segments,
    };
    let validated = validate_chunk_output(&first, audio, source.range(), output)?;
    let (chunk_segments, language, warnings) = validated.into_parts();
    let merged = merge_chunks(&[chunk_segments]);
    let run = AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero")?,
        audio_stream: 1,
        chunks: vec![AsrChunkRecord::new(
            first,
            AsrChunkOutcome::Transcribed { audio },
        )],
    })?;
    Ok(build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription: AsrTranscription {
            run,
            language,
            segments: merged.segments,
            unplaced: merged.unplaced,
            warnings,
        },
        splice: None,
    })?)
}

/// A local-ASR revision of a 12 s source whose one run transcribed its first
/// 6 s and in which the recogniser wrote words that no segment holds (#359): two
/// segments kept, at 0.6 to 0.9 s and 3.0 to 3.5 s, and between them, at 1.0 s,
/// one rejected for a reversed range.
fn revision_with_words_that_no_segment_holds() -> Built<TranscriptRevision> {
    let session = SessionId::parse(SESSION)?;
    let source_id = SourceId::from_sha256(DIGEST)?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(12 * SECOND))?;
    let audio = range(0, 6 * SECOND)?;
    let planned = plan_chunks(source.id(), audio, ChunkPlan::R0)?;
    let first = planned.first().ok_or("no chunk")?.clone();
    let segment = |from: u64, to: u64, words: &str| -> Built<ProviderSegment> {
        Ok(ProviderSegment {
            start: ChunkTime::from_millis(from).ok_or("time")?,
            end: ChunkTime::from_millis(to).ok_or("time")?,
            text: Some(CueText::new(words.to_owned(), words.to_owned())?),
            tokens: Vec::new(),
        })
    };
    let output = ProviderChunkOutput {
        language: None,
        segments: vec![
            segment(600, 900, "the build is healthy")?,
            segment(1_000, 900, "error code E-409")?,
            segment(3_000, 3_500, "and the queue is empty")?,
        ],
    };
    let validated = validate_chunk_output(&first, audio, source.range(), output)?;
    let (chunk_segments, language, warnings) = validated.into_parts();
    let merged = merge_chunks(&[chunk_segments]);
    let run = AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero")?,
        audio_stream: 1,
        chunks: vec![AsrChunkRecord::new(
            first,
            AsrChunkOutcome::Transcribed { audio },
        )],
    })?;
    Ok(build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription: AsrTranscription {
            run,
            language,
            segments: merged.segments,
            unplaced: merged.unplaced,
            warnings,
        },
        splice: None,
    })?)
}

/// One page of `query` over `revision` in its published forms: the full
/// `--json` result, the brief one (`--view brief`) and the evidence stream.
struct Page {
    json: Value,
    brief: Value,
    lines: Vec<String>,
}

fn search(
    revision: &TranscriptRevision,
    query: &str,
    window: Option<TimeRange>,
    limit: u16,
) -> Built<Page> {
    let session = SessionId::parse(SESSION)?;
    let query = SearchQuery::parse(query)?;
    let page = page_search(
        &session,
        revision,
        &SearchPageRequest {
            query: query.clone(),
            range: window,
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
    let presentation = SearchPresentation {
        session_id: &session,
        revision,
        query: &query,
        range: window,
        hits: page
            .hits
            .iter()
            .map(|hit| (hit.segment(), hit.tier()))
            .collect(),
        next_cursor: next_cursor.as_deref(),
        coverage: &page.coverage,
    };
    let lifecycle = || LifecycleResponse::ephemeral(EXPIRES_AT.to_owned());
    let json = serde_json::to_value(search_response(&presentation, lifecycle())?)?;
    let brief = serde_json::to_value(search_response_with_view(
        &presentation,
        lifecycle(),
        ResultView::Brief,
    )?)?;
    let stream = SearchEvidenceStream::new(&presentation, lifecycle())?;
    let mut lines = stream
        .records()
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()?;
    lines.push(serde_json::to_string(stream.terminal())?);
    Ok(Page { json, brief, lines })
}

fn parsed(lines: &[String]) -> Built<Vec<Value>> {
    Ok(lines
        .iter()
        .map(|line| serde_json::from_str(line))
        .collect::<Result<Vec<Value>, _>>()?)
}

#[test]
fn the_json_result_matches_the_frozen_example() -> TestResult {
    let page = search(&f10_revision()?, "R-17", None, 20)?;
    validate("operation-response.schema.json", &page.json)?;
    validate("search-data.schema.json", &page.json["data"])?;
    assert!(
        page.json == load(JSON_EXAMPLE)?,
        "the search result differs from {JSON_EXAMPLE}"
    );
    let data = &page.json["data"];
    assert_eq!(page.json["status"], "complete");
    assert_eq!(
        page.json["coverage"],
        serde_json::json!({"truncated": false, "gaps": [], "reasons": []})
    );
    assert_eq!(data["query"]["terms"], serde_json::json!(["r17"]));
    assert_eq!(data["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(data["items"][0]["text"], "Dialog R-17 is displayed now.");
    // F10-E01's frozen truth window.
    assert_eq!(data["items"][0]["start_us"], 5_000_000);
    assert_eq!(data["items"][0]["end_us"], 9_000_000);
    assert_eq!(
        data["hits"][0]["segment_id"],
        data["items"][0]["segment_id"]
    );
    assert_eq!(data["hits"][0]["match"], "phrase");
    assert_eq!(data["transcript_coverage"]["basis"], "supplied_transcript");
    assert_eq!(data["transcript_coverage"]["scope"], "transcript_text");
    Ok(())
}

#[test]
fn the_stream_matches_the_frozen_example_byte_for_byte() -> TestResult {
    let page = search(&f10_revision()?, "R-17", None, 20)?;
    let example = read(STREAM_EXAMPLE)?;
    let mut emitted = page.lines.join("\n");
    emitted.push('\n');
    assert!(
        emitted == example.replace("\r\n", "\n"),
        "the emitted stream differs from {STREAM_EXAMPLE}"
    );
    for line in example.lines() {
        validate_line(&serde_json::from_str(line)?)?;
    }
    Ok(())
}

/// "dialog r 17" is the same phrase spelled as spoken; it cites the same
/// segment as "R-17".
#[test]
fn a_spoken_spelling_finds_the_same_segment() -> TestResult {
    let revision = f10_revision()?;
    let written = search(&revision, "R-17", None, 20)?;
    let spoken = search(&revision, "dialog r 17", None, 20)?;
    assert_eq!(spoken.json["data"]["hits"], written.json["data"]["hits"]);
    assert_eq!(
        spoken.json["data"]["query"]["terms"],
        serde_json::json!(["dialog", "r", "17"])
    );
    Ok(())
}

/// The records are exactly the `--json` items, in rank order, followed by one
/// terminal event whose data is the page without its items.
#[test]
fn records_are_the_items_then_one_terminal_event_with_the_hits() -> TestResult {
    let page = search(&f10_revision()?, "synthetic", None, 20)?;
    let lines = parsed(&page.lines)?;
    let items = page.json["data"]["items"]
        .as_array()
        .ok_or("items missing")?;
    assert_eq!(items.len(), 2);
    assert_eq!(lines.len(), items.len() + 1);
    for (index, (line, item)) in lines.iter().zip(items).enumerate() {
        validate_line(line)?;
        assert_eq!(line["sequence"], index);
        assert_eq!(line["command"], "search");
        assert_eq!(line["record_type"], "transcript_segment");
        assert!(line["key"] == item["segment_id"], "key {index}");
        assert!(&line["record"] == item, "record {index}");
    }
    let terminal = lines.last().ok_or("terminal missing")?;
    validate_line(terminal)?;
    assert_eq!(terminal["sequence"], items.len());
    let data = &terminal["result"]["data"];
    assert_eq!(data["record_count"], items.len());
    assert!(data.get("items").is_none());
    for member in [
        "session_id",
        "revision",
        "query",
        "range",
        "hits",
        "next_cursor",
        "transcript_coverage",
    ] {
        assert!(
            data[member] == page.json["data"][member],
            "terminal data {member} differs from the page"
        );
    }
    for member in ["status", "warnings", "error", "coverage", "lifecycle"] {
        assert!(
            terminal["result"][member] == page.json[member],
            "terminal result {member} differs from the page result"
        );
    }
    Ok(())
}

#[test]
fn a_search_without_hits_is_complete_and_streams_one_terminal_event() -> TestResult {
    let page = search(&f10_revision()?, "absent", Some(range(0, 12 * SECOND)?), 20)?;
    validate("search-data.schema.json", &page.json["data"])?;
    assert_eq!(page.json["status"], "complete");
    assert_eq!(page.json["data"]["items"], serde_json::json!([]));
    assert_eq!(page.json["data"]["hits"], serde_json::json!([]));
    assert!(page.json["data"]["next_cursor"].is_null());
    let lines = parsed(&page.lines)?;
    assert_eq!(lines.len(), 1);
    validate_line(&lines[0])?;
    assert_eq!(lines[0]["sequence"], 0);
    assert_eq!(lines[0]["result"]["data"]["record_count"], 0);
    Ok(())
}

/// A range the transcript does not fully cover is a `partial` success whose
/// envelope coverage names the untranscribed gap.
#[test]
fn an_incompletely_transcribed_range_is_a_partial_result() -> TestResult {
    let revision = half_transcribed_revision()?;
    let page = search(&revision, "build 2,048", None, 20)?;
    validate("operation-response.schema.json", &page.json)?;
    validate("search-data.schema.json", &page.json["data"])?;
    assert_eq!(page.json["status"], "partial");
    assert!(page.json["error"].is_null());
    assert_eq!(
        page.json["coverage"],
        serde_json::json!({
            "truncated": true,
            "gaps": ["6000000-12000000"],
            "reasons": ["untranscribed_range"],
        })
    );
    assert_eq!(
        page.json["warnings"],
        serde_json::json!([UNTRANSCRIBED_SEARCH_WARNING])
    );
    let coverage = &page.json["data"]["transcript_coverage"];
    assert_eq!(coverage["basis"], "local_asr");
    assert_eq!(
        coverage["transcribed_ranges"],
        serde_json::json!([{"from_us": 0, "to_us": 6_000_000}])
    );
    assert_eq!(
        coverage["untranscribed_ranges"],
        serde_json::json!([{"from_us": 6_000_000, "to_us": 12_000_000}])
    );
    assert_eq!(coverage["ranges_truncated"], false);
    for line in parsed(&page.lines)? {
        validate_line(&line)?;
    }
    let terminal = parsed(&page.lines)?.pop().ok_or("terminal missing")?;
    assert_eq!(terminal["result"]["status"], "partial");
    assert_eq!(terminal["result"]["coverage"], page.json["coverage"]);

    // Restricted to the transcribed part, the same search is complete.
    let inside = search(&revision, "build 2,048", Some(range(0, 6 * SECOND)?), 20)?;
    assert_eq!(inside.json["status"], "complete");
    assert_eq!(inside.json["coverage"]["truncated"], false);
    Ok(())
}

/// #359: words the recogniser wrote and no segment holds are not covered by the
/// window they were written in, so a search over them is `partial`, names the
/// stretch as a gap with the published reason, and says in the revision it
/// presents (`local_asr.unplaced_ranges`) where the words may lie; a search that
/// stays clear of the stretch is complete. Without such words the same run is
/// covered whole, as it always was.
#[test]
fn a_search_over_words_that_no_segment_holds_is_partial_and_names_the_stretch() -> TestResult {
    let revision = revision_with_words_that_no_segment_holds()?;
    let page = search(&revision, "queue is empty", None, 20)?;
    validate("operation-response.schema.json", &page.json)?;
    validate("search-data.schema.json", &page.json["data"])?;
    assert_eq!(page.json["status"], "partial");
    assert_eq!(
        page.json["coverage"],
        serde_json::json!({
            "truncated": true,
            "gaps": ["1000000-3000000", "6000000-12000000"],
            "reasons": ["untranscribed_range"],
        })
    );
    let coverage = &page.json["data"]["transcript_coverage"];
    assert_eq!(
        coverage["transcribed_ranges"],
        serde_json::json!([
            {"from_us": 0, "to_us": 1_000_000},
            {"from_us": 3_000_000, "to_us": 6_000_000},
        ])
    );
    assert_eq!(
        page.json["data"]["revision"]["local_asr"]["unplaced_ranges"],
        serde_json::json!([{"from_us": 1_000_000, "to_us": 3_000_000}])
    );
    // The word that was kept is found; the one that was rejected is not.
    assert_eq!(page.json["data"]["hits"].as_array().map(Vec::len), Some(1));
    let lost = search(&revision, "E-409", None, 20)?;
    assert_eq!(lost.json["data"]["hits"].as_array().map(Vec::len), Some(0));
    assert_eq!(lost.json["status"], "partial");
    for line in parsed(&page.lines)? {
        validate_line(&line)?;
    }
    // Clear of the stretch, within what was transcribed, it is complete.
    let clear = search(
        &revision,
        "queue is empty",
        Some(range(3 * SECOND, 6 * SECOND)?),
        20,
    )?;
    assert_eq!(clear.json["status"], "complete");
    // Over the stretch it is not.
    let over = search(&revision, "queue", Some(range(SECOND, 2 * SECOND)?), 20)?;
    assert_eq!(over.json["status"], "partial");
    assert_eq!(
        over.json["coverage"]["gaps"],
        serde_json::json!(["1000000-2000000"])
    );
    Ok(())
}

#[test]
fn rejected_queries_have_fixed_remediation_naming_the_reason() -> TestResult {
    for rejection in SearchQueryRejection::ALL {
        let summary = search_query_rejection_summary(rejection);
        assert!(summary.contains(rejection.identifier()), "{summary}");
        assert!(summary.len() <= 1024);
        let response = serde_json::to_value(OperationResponse::failure_with_remediation(
            CommandName::Search.identifier(),
            FailureCode::InvalidArgument,
            summary,
        ))?;
        validate("operation-response.schema.json", &response)?;
        assert_eq!(response["error"]["code"], "INVALID_ARGUMENT");
    }
    Ok(())
}

#[test]
fn search_schemas_reject_unknown_fields_and_values() -> TestResult {
    let example = load(JSON_EXAMPLE)?;
    let data = &example["data"];

    let mut extended = data.clone();
    extended
        .as_object_mut()
        .ok_or("not an object")?
        .insert("unreviewed".to_owned(), Value::Bool(true));
    assert!(!is_valid("search-data.schema.json", &extended)?);

    let mut unknown_match = data.clone();
    unknown_match["hits"][0]["match"] = Value::from("fuzzy");
    assert!(!is_valid("search-data.schema.json", &unknown_match)?);

    let mut unknown_basis = data.clone();
    unknown_basis["transcript_coverage"]["basis"] = Value::from("guessed");
    assert!(!is_valid("search-data.schema.json", &unknown_basis)?);

    let mut other_scope = data.clone();
    other_scope["transcript_coverage"]["scope"] = Value::from("on_screen_text");
    assert!(!is_valid("search-data.schema.json", &other_scope)?);

    let mut with_count = data.clone();
    with_count
        .as_object_mut()
        .ok_or("not an object")?
        .insert("record_count".to_owned(), Value::from(1));
    assert!(!is_valid("search-data.schema.json", &with_count)?);

    let stream = read(STREAM_EXAMPLE)?;
    let terminal: Value = serde_json::from_str(stream.lines().last().ok_or("empty")?)?;
    let mut with_items = terminal["result"]["data"].clone();
    with_items
        .as_object_mut()
        .ok_or("not an object")?
        .insert("items".to_owned(), Value::Array(Vec::new()));
    assert!(!is_valid("search-stream-data.schema.json", &with_items)?);
    Ok(())
}

/// Whether `brief` is a projection of `full`: every member of `brief`, at every
/// depth, is a member of `full` with the same value, and an array is one when
/// each of its elements is.
fn is_projection_of(brief: &Value, full: &Value) -> bool {
    match (brief, full) {
        (Value::Object(brief), Value::Object(full)) => brief.iter().all(|(member, value)| {
            full.get(member)
                .is_some_and(|other| is_projection_of(value, other))
        }),
        (Value::Array(brief), Value::Array(full)) => {
            brief.len() == full.len()
                && brief
                    .iter()
                    .zip(full)
                    .all(|(brief, full)| is_projection_of(brief, full))
        }
        (brief, full) => brief == full,
    }
}

/// #365: the brief result is its frozen example. The full result names no view,
/// the brief one does, and what the brief one leaves out of each item is in the
/// revision block once.
#[test]
fn the_brief_result_matches_the_frozen_example() -> TestResult {
    let page = search(&f10_revision()?, "R-17", None, 20)?;
    validate("operation-response.schema.json", &page.brief)?;
    validate("search-brief-data.schema.json", &page.brief["data"])?;
    assert!(
        page.brief == load("examples/search.brief.json")?,
        "the brief search result differs from examples/search.brief.json"
    );
    assert!(page.json["data"].get("view").is_none());
    assert_eq!(page.brief["data"]["view"], "brief");
    for item in page.brief["data"]["items"]
        .as_array()
        .ok_or("items missing")?
    {
        validate("transcript-segment-brief.schema.json", item)?;
    }
    assert_eq!(
        page.brief["data"]["items"][0]["display_text"],
        "Dialog R-17 is displayed now."
    );
    assert_eq!(
        page.brief["data"]["revision"]["revision_id"],
        page.json["data"]["items"][0]["revision_id"]
    );
    Ok(())
}

/// #365: the two views are one result. Everything but the items, the `view`
/// member and the shape of an item is the full result's, value for value, for a
/// complete search and for a partial one; each brief item is a projection of
/// the full item at the same rank, and the hits that name them are unchanged.
#[test]
fn the_brief_result_is_a_projection_of_the_full_result() -> TestResult {
    for (revision, query) in [
        (f10_revision()?, "synthetic"),
        (half_transcribed_revision()?, "build 2,048"),
    ] {
        let page = search(&revision, query, None, 20)?;
        for member in ["schema_version", "command", "status", "warnings", "error"] {
            assert_eq!(page.brief[member], page.json[member], "{member}");
        }
        assert_eq!(page.brief["coverage"], page.json["coverage"]);
        assert_eq!(page.brief["lifecycle"], page.json["lifecycle"]);
        for (member, value) in page.json["data"].as_object().ok_or("data")? {
            if member != "items" {
                assert_eq!(&page.brief["data"][member], value, "data member {member}");
            }
        }
        let (brief_items, full_items) = (
            page.brief["data"]["items"].as_array().ok_or("items")?,
            page.json["data"]["items"].as_array().ok_or("items")?,
        );
        assert!(!full_items.is_empty());
        assert_eq!(brief_items.len(), full_items.len());
        for (brief_item, full_item) in brief_items.iter().zip(full_items) {
            assert!(is_projection_of(brief_item, full_item));
        }
        for (hit, item) in page.brief["data"]["hits"]
            .as_array()
            .ok_or("hits")?
            .iter()
            .zip(brief_items)
        {
            assert_eq!(hit["segment_id"], item["segment_id"]);
        }
    }
    Ok(())
}

/// The brief view never leaves out a gap: a partial search is partial, with the
/// same coverage and warning, in either view.
#[test]
fn a_partial_brief_result_states_its_gap_as_the_full_one_does() -> TestResult {
    let page = search(&half_transcribed_revision()?, "build 2,048", None, 20)?;
    validate("operation-response.schema.json", &page.brief)?;
    validate("search-brief-data.schema.json", &page.brief["data"])?;
    assert_eq!(page.brief["status"], "partial");
    assert_eq!(
        page.brief["coverage"],
        serde_json::json!({
            "truncated": true,
            "gaps": ["6000000-12000000"],
            "reasons": ["untranscribed_range"],
        })
    );
    assert_eq!(
        page.brief["warnings"],
        serde_json::json!([UNTRANSCRIBED_SEARCH_WARNING])
    );
    assert_eq!(
        page.brief["data"]["transcript_coverage"],
        page.json["data"]["transcript_coverage"]
    );
    Ok(())
}

/// The full result and the stream are what they were: the new view changes
/// neither, and a stream's records are the full items.
#[test]
fn the_full_result_and_the_stream_are_unchanged_by_the_brief_view() -> TestResult {
    let page = search(&f10_revision()?, "synthetic", None, 20)?;
    let lines = parsed(&page.lines)?;
    for (line, item) in lines
        .iter()
        .zip(page.json["data"]["items"].as_array().ok_or("items")?)
    {
        assert_eq!(&line["record"], item);
        assert!(line["record"].get("view").is_none());
    }
    let terminal = lines.last().ok_or("terminal")?;
    assert!(terminal["result"]["data"].get("view").is_none());
    assert!(page.json["data"].get("view").is_none());
    Ok(())
}
