//! Conformance of the JSON Lines evidence stream (ADR 0016 decision 5): a
//! `transcript.get` page streamed as one evidence event per segment followed by
//! exactly one terminal event, against the published v1 schemas and the frozen
//! example `examples/transcript-get.events.jsonl`.
//!
//! The example is the first page (limit 2) of the F10 fixture imported from
//! `fixtures/corpus/transcripts/F10.srt` with the explicit +500 ms offset, the
//! same page as `examples/transcript-get.json` in its `--json` form.

use std::{
    collections::BTreeSet,
    fs, io,
    num::{NonZeroU16, NonZeroU32},
    path::PathBuf,
};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    AsrRevisionRequest, AsrTranscription, ImportedRevisionRequest, SuppliedTranscript,
    TranscriptPageRequest, build_asr_revision, build_imported_revision, page_transcript,
    whole_file_source_segment,
};
use vsift_contract::{
    CommandName, EventKind, EvidenceRecordType, LifecycleResponse, OperationResponse, ResultView,
    TerminalEventResponse, TranscriptEvidenceStream, TranscriptPresentation,
    transcript_get_response,
};
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, CueSource, CueText, CueTiming, FailureCode,
    ImportedCue, MediaTime, PageLimit, ParsedTranscript, SessionId, Sha256Hex, SidecarIdentity,
    SourceId, TimeRange, TranscriptFormat, TranscriptOffset, TranscriptRevision,
    TranscriptWarnings, plan_chunks,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const F10_SOURCE: &str =
    "src_sha256_d7ccece71288c5ff35d7b16c871a93a8ed48bbb7380617899575069b4d6545f4";
const F10_SRT_SHA256: &str = "2a4ee37d826754eac43da03e7cc63f718b16aed4d3ce163c380ebd8ab9630847";
const F10_SRT_BYTES: u64 = 245;
const EXPIRES_AT: &str = "2026-09-25T00:00:00Z";
/// Session expiry and "now" used to scope cursors, in microseconds.
const CURSOR_EXPIRES_US: u64 = 1_790_294_400_000_000;
const NOW_US: u64 = 1_790_208_000_000_000;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const STREAM_EXAMPLE: &str = "examples/transcript-get.events.jsonl";
/// The published schemas that define a JSON Lines event, one per event kind.
const EVENT_SCHEMAS: [&str; 5] = [
    "evidence-event.schema.json",
    "terminal-event.schema.json",
    "progress-event.schema.json",
    "lifecycle-event.schema.json",
    "result-event.schema.json",
];

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn read(relative_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(fs::read_to_string(schema_root().join(relative_path))?)
}

fn load(relative_path: &str) -> Result<Value, Box<dyn std::error::Error>> {
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

fn is_valid(schema_path: &str, instance: &Value) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&load(schema_path)?)?
        .is_valid(instance))
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema = load(schema_path)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// Validates one stream line against the schemas its `event` selects.
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
                validate(
                    "transcript-get-stream-data.schema.json",
                    &line["result"]["data"],
                )?;
            }
            Ok(())
        }
        _ => Err("a stream line has no published event kind".into()),
    }
}

fn f10_revision() -> Result<TranscriptRevision, Box<dyn std::error::Error>> {
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

/// One page of the F10 revision in both published forms: the `--json`
/// result and the `--events jsonl` stream serialized line by line.
struct Page {
    json: Value,
    lines: Vec<String>,
}

fn page(
    revision: &TranscriptRevision,
    from: u64,
    to: u64,
    limit: u16,
    cursor: Option<String>,
) -> Result<Page, Box<dyn std::error::Error>> {
    let session = SessionId::parse(SESSION)?;
    let range = TimeRange::new(MediaTime::from_micros(from), MediaTime::from_micros(to))?;
    let page = page_transcript(
        &session,
        revision,
        &TranscriptPageRequest {
            range,
            limit: PageLimit::new(limit)?,
            cursor,
        },
        CURSOR_EXPIRES_US,
        NOW_US,
    )?;
    let segments: Vec<_> = page.segments.into_iter().cloned().collect();
    let next_cursor = page.next_cursor.map(|cursor| cursor.encode());
    let lifecycle = || LifecycleResponse::ephemeral(EXPIRES_AT.to_owned());
    let presentation = TranscriptPresentation {
        session_id: &session,
        revision,
        range,
        segments: &segments,
        next_cursor: next_cursor.as_deref(),
        coverage: &page.coverage,
        time_anchors: &vsift_domain::TimeAnchors::none(),
    };
    let json = serde_json::to_value(transcript_get_response(
        &presentation,
        lifecycle(),
        ResultView::Full,
    )?)?;
    let stream = TranscriptEvidenceStream::new(&presentation, lifecycle())?;
    let mut lines = stream
        .records()
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()?;
    lines.push(serde_json::to_string(stream.terminal())?);
    Ok(Page { json, lines })
}

fn parsed(lines: &[String]) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    Ok(lines
        .iter()
        .map(|line| serde_json::from_str(line))
        .collect::<Result<Vec<Value>, _>>()?)
}

#[test]
fn the_stream_matches_the_frozen_example_byte_for_byte() -> TestResult {
    let page = page(&f10_revision()?, 0, 12_000_000, 2, None)?;
    let example = read(STREAM_EXAMPLE)?;

    let mut emitted = page.lines.join("\n");
    emitted.push('\n');
    assert!(
        emitted == example.replace("\r\n", "\n"),
        "the emitted stream differs from {STREAM_EXAMPLE}"
    );
    for line in parsed(&page.lines)? {
        validate_line(&line)?;
    }
    Ok(())
}

/// #365 and #373: a stream is the stream the published `0.1.0` wrote, byte for
/// byte in every record, with only the terminal data's new member added. The
/// brief view is never a stream's: its records are published evidence an index
/// stores by key, and they stay complete.
#[test]
fn a_stream_is_what_was_published_but_for_the_new_terminal_member() -> TestResult {
    let page = page(&f10_revision()?, 0, 12_000_000, 2, None)?;
    let published = fs::read_to_string(
        schema_root().join("frozen/v0.1.0/examples/transcript-get.events.jsonl"),
    )?;
    let published: Vec<&str> = published.lines().collect();
    assert_eq!(page.lines.len(), published.len());
    let (terminal, records) = page.lines.split_last().ok_or("empty stream")?;
    for (emitted, frozen) in records.iter().zip(&published) {
        assert_eq!(emitted, frozen);
    }
    let mut terminal: Value = serde_json::from_str(terminal)?;
    terminal["result"]["data"]
        .as_object_mut()
        .ok_or("terminal data is not an object")?
        .remove("transcript_coverage")
        .ok_or("the new member is missing")?;
    assert_eq!(
        terminal,
        serde_json::from_str::<Value>(published.last().ok_or("empty stream")?)?
    );
    Ok(())
}

#[test]
fn every_line_of_the_frozen_example_validates() -> TestResult {
    let example = read(STREAM_EXAMPLE)?;
    let lines: Vec<&str> = example.lines().collect();

    assert_eq!(lines.len(), 3);
    for line in lines {
        validate_line(&serde_json::from_str(line)?)?;
    }
    Ok(())
}

/// The records are exactly the `--json` page items, in order, followed by one
/// terminal event whose data is the page without its items.
#[test]
fn records_are_the_page_items_then_one_terminal_event() -> TestResult {
    let page = page(&f10_revision()?, 0, 12_000_000, 2, None)?;
    let lines = parsed(&page.lines)?;
    let items = page.json["data"]["items"]
        .as_array()
        .ok_or("items missing")?;

    assert_eq!(lines.len(), items.len() + 1);
    for (index, (line, item)) in lines.iter().zip(items).enumerate() {
        assert_eq!(line["event"], "evidence");
        assert_eq!(line["sequence"], index);
        assert_eq!(line["command"], "transcript.get");
        assert_eq!(line["record_type"], "transcript_segment");
        assert!(
            line["key"] == item["segment_id"],
            "record {index}: the key is not its segment identity"
        );
        assert!(
            &line["record"] == item,
            "record {index} differs from the page item"
        );
    }
    let terminal = lines.last().ok_or("terminal missing")?;
    assert_eq!(terminal["event"], "terminal");
    assert_eq!(terminal["sequence"], items.len());
    let data = &terminal["result"]["data"];
    assert_eq!(data["record_count"], items.len());
    assert!(data.get("items").is_none());
    for member in [
        "session_id",
        "revision",
        "range",
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

/// A local-recognition revision of a 55 s recording with no segment, whose
/// first chunk (0-30 s) was transcribed and whose second (25-55 s) could not be
/// read: 30-55 s has no transcript.
fn revision_with_a_gap() -> Result<TranscriptRevision, Box<dyn std::error::Error>> {
    let session = SessionId::parse(SESSION)?;
    let source_id = SourceId::from_sha256(&"0123456789abcdef".repeat(4))?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(55_000_000))?;
    let digest = Sha256Hex::parse("ab".repeat(32))?;
    let planned = plan_chunks(source.id(), source.range(), ChunkPlan::R0)?;
    assert_eq!(planned.len(), 2);
    let run = AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, digest.clone()),
        model: AsrModel::new(AsrModelProfile::Base, digest),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero")?,
        audio_stream: 1,
        chunks: planned
            .into_iter()
            .enumerate()
            .map(|(index, chunk)| {
                let audio = chunk.window();
                let outcome = if index == 0 {
                    AsrChunkOutcome::Transcribed { audio }
                } else {
                    AsrChunkOutcome::Unusable { audio }
                };
                AsrChunkRecord::new(chunk, outcome)
            })
            .collect(),
    })?;
    Ok(build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription: AsrTranscription {
            run,
            language: None,
            segments: Vec::new(),
            unplaced: Vec::new(),
            warnings: TranscriptWarnings::default(),
        },
        splice: None,
    })?)
}

/// #373: the terminal event of a stream states the coverage and the status of
/// the range that was asked for, as the page does, **not** of the whole
/// recording. The recording's gap is 30-55 s: a stream of 0-25 s is complete
/// whatever lies beyond it, one that reaches into the gap lists only its own
/// part, and one that runs past the end of the video is clipped to it.
#[test]
fn a_stream_states_the_coverage_and_status_of_its_own_range() -> TestResult {
    let revision = revision_with_a_gap()?;
    let second = 1_000_000;
    for (from, to, gaps) in [
        (0, 25, vec![]),
        (0, 30, vec![]),
        (0, 31, vec!["30000000-31000000"]),
        (20, 40, vec!["30000000-40000000"]),
        (40, 50, vec!["40000000-50000000"]),
        (54, 60, vec!["54000000-55000000"]),
    ] {
        let page = page(&revision, from * second, to * second, 100, None)?;
        let lines = parsed(&page.lines)?;
        let terminal = &lines.last().ok_or("terminal missing")?["result"];
        validate(
            "terminal-event.schema.json",
            lines.last().ok_or("terminal")?,
        )?;
        let label = format!("{from}-{to} s");
        let named: Vec<&str> = terminal["coverage"]["gaps"]
            .as_array()
            .map(|gaps| gaps.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        assert_eq!(named, gaps, "{label}");
        if gaps.is_empty() {
            assert_eq!(terminal["status"], "complete", "{label}");
            assert!(terminal["coverage"].is_null(), "{label}");
        } else {
            assert_eq!(terminal["status"], "partial", "{label}");
        }
        let untranscribed: Vec<String> =
            terminal["data"]["transcript_coverage"]["untranscribed_ranges"]
                .as_array()
                .ok_or("no untranscribed list")?
                .iter()
                .map(|range| format!("{}-{}", range["from_us"], range["to_us"]))
                .collect();
        assert_eq!(untranscribed, gaps, "{label}");
        // The searched range is the range asked for, clipped to the video.
        assert_eq!(
            terminal["data"]["transcript_coverage"]["searched_range"]["to_us"],
            to.min(55) * second,
            "{label}"
        );
        // And the page says the same, member for member.
        for member in ["status", "warnings", "error", "coverage", "lifecycle"] {
            assert_eq!(terminal[member], page.json[member], "{label}: {member}");
        }
        assert_eq!(
            terminal["data"]["transcript_coverage"], page.json["data"]["transcript_coverage"],
            "{label}"
        );
    }
    Ok(())
}

/// Following each terminal cursor visits every segment exactly once and ends
/// with a terminal event whose cursor is null.
#[test]
fn terminal_cursors_continue_the_stream_until_the_range_is_exhausted() -> TestResult {
    let revision = f10_revision()?;
    let mut cursor = None;
    let mut keys = Vec::new();
    let mut pages = 0;
    loop {
        let page = page(&revision, 0, 12_000_000, 2, cursor)?;
        pages += 1;
        let lines = parsed(&page.lines)?;
        let (terminal, records) = lines.split_last().ok_or("empty stream")?;
        for record in records {
            validate_line(record)?;
            keys.push(record["key"].as_str().ok_or("key missing")?.to_owned());
        }
        validate_line(terminal)?;
        match terminal["result"]["data"]["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }

    assert_eq!(pages, 2);
    assert_eq!(keys.len(), 3);
    assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 3);
    let all = parsed(&page(&revision, 0, 12_000_000, 100, None)?.lines)?;
    let expected: Vec<&Value> = all
        .iter()
        .filter(|line| line["event"] == "evidence")
        .map(|line| &line["key"])
        .collect();
    assert!(
        keys.iter()
            .map(|key| Value::from(key.as_str()))
            .collect::<Vec<_>>()
            == expected.into_iter().cloned().collect::<Vec<_>>(),
        "paged keys differ from a single full page, in order"
    );
    Ok(())
}

/// A range that no segment intersects is a complete stream of one line.
#[test]
fn an_empty_range_is_a_single_terminal_event() -> TestResult {
    let page = page(&f10_revision()?, 4_000_000, 5_000_000, 20, None)?;
    let lines = parsed(&page.lines)?;

    assert_eq!(lines.len(), 1);
    validate_line(&lines[0])?;
    assert_eq!(lines[0]["event"], "terminal");
    assert_eq!(lines[0]["sequence"], 0);
    assert_eq!(lines[0]["result"]["status"], "complete");
    assert_eq!(lines[0]["result"]["data"]["record_count"], 0);
    assert!(lines[0]["result"]["data"]["next_cursor"].is_null());
    Ok(())
}

/// The stream is bounded by the page limit: at most `limit` records.
#[test]
fn the_stream_never_exceeds_the_page_limit() -> TestResult {
    let revision = f10_revision()?;
    for limit in 1..=3 {
        let lines = parsed(&page(&revision, 0, 12_000_000, limit, None)?.lines)?;
        let records = lines
            .iter()
            .filter(|line| line["event"] == "evidence")
            .count();
        assert_eq!(records, usize::from(limit));
        assert_eq!(lines.len(), records + 1);
        assert_eq!(
            lines
                .iter()
                .filter(|line| line["event"] == "terminal")
                .count(),
            1
        );
    }
    Ok(())
}

#[test]
fn evidence_and_stream_schemas_reject_unknown_fields_and_mismatched_keys() -> TestResult {
    let example = read(STREAM_EXAMPLE)?;
    let lines = example
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<Value>, _>>()?;
    let record = lines.first().ok_or("record missing")?;
    let terminal = lines.last().ok_or("terminal missing")?;

    let mut extended = record.clone();
    extended
        .as_object_mut()
        .ok_or("not an object")?
        .insert("unreviewed".to_owned(), Value::Bool(true));
    assert!(!is_valid("evidence-event.schema.json", &extended)?);

    let mut extended_record = record.clone();
    extended_record["record"]
        .as_object_mut()
        .ok_or("not an object")?
        .insert("unreviewed".to_owned(), Value::Bool(true));
    assert!(!is_valid("evidence-event.schema.json", &extended_record)?);

    let mut foreign_key = record.clone();
    foreign_key["key"] = Value::from("evd_0123456789abcdef");
    assert!(!is_valid("evidence-event.schema.json", &foreign_key)?);

    let mut unknown_type = record.clone();
    unknown_type["record_type"] = Value::from("frame_image");
    assert!(!is_valid("evidence-event.schema.json", &unknown_type)?);

    // A transcript segment labelled as another published record type fails
    // that type's key pattern and record schema.
    let mut mislabelled = record.clone();
    mislabelled["record_type"] = Value::from("visual_candidate");
    assert!(!is_valid("evidence-event.schema.json", &mislabelled)?);

    let mut as_terminal = record.clone();
    as_terminal["event"] = Value::from("terminal");
    assert!(!is_valid("evidence-event.schema.json", &as_terminal)?);

    let mut data = terminal["result"]["data"].clone();
    data.as_object_mut()
        .ok_or("not an object")?
        .insert("items".to_owned(), Value::Array(Vec::new()));
    assert!(!is_valid("transcript-get-stream-data.schema.json", &data)?);
    Ok(())
}

/// The members of a string `enum` or the value of a `const` at `pointer`.
fn published_identifiers(
    schema: &str,
    pointer: &str,
) -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
    let value = load(schema)?;
    let node = value
        .pointer(pointer)
        .ok_or_else(|| format!("{schema} has nothing at {pointer}"))?;
    let members: Vec<&Value> = match (node.get("enum"), node.get("const")) {
        (Some(Value::Array(members)), None) => members.iter().collect(),
        (None, Some(constant)) => vec![constant],
        _ => return Err(format!("{schema}{pointer} is neither an enum nor a const").into()),
    };
    members
        .into_iter()
        .map(|member| {
            member
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{schema}{pointer} has a non-string member").into())
        })
        .collect()
}

/// Every event kind has exactly one published event schema, and every
/// published event schema is produced by an event kind.
#[test]
fn event_kinds_and_published_event_schemas_match() -> TestResult {
    let mut published = BTreeSet::new();
    for schema in EVENT_SCHEMAS {
        let kinds = published_identifiers(schema, "/properties/event")?;
        assert_eq!(kinds.len(), 1, "{schema} must fix one event kind");
        published.extend(kinds);
    }
    let produced: BTreeSet<String> = EventKind::ALL
        .into_iter()
        .map(|kind| kind.identifier().to_owned())
        .collect();

    assert_eq!(produced, published);
    Ok(())
}

/// The published `record_type` enum is exactly the record types the contract
/// produces, and each has a schema branch that constrains its record.
#[test]
fn record_types_and_the_published_record_type_enum_match() -> TestResult {
    let published = published_identifiers("evidence-event.schema.json", "/properties/record_type")?;
    let produced: BTreeSet<String> = EvidenceRecordType::ALL
        .into_iter()
        .map(|record_type| record_type.identifier().to_owned())
        .collect();
    assert_eq!(produced, published);

    let schema = load("evidence-event.schema.json")?;
    let branches = schema["allOf"].as_array().ok_or("allOf missing")?;
    for record_type in EvidenceRecordType::ALL {
        let constrained = branches.iter().any(|branch| {
            branch["if"]["properties"]["record_type"]["const"] == record_type.identifier()
                && branch["then"]["properties"]["record"]["$ref"].is_string()
        });
        assert!(
            constrained,
            "{} has no record schema",
            record_type.identifier()
        );
    }
    Ok(())
}

/// Failures stay one terminal event at sequence 0, whatever the output mode.
#[test]
fn a_failed_stream_is_one_terminal_failure_event() -> TestResult {
    let event = serde_json::to_value(TerminalEventResponse::new(OperationResponse::failure(
        CommandName::TranscriptGet.identifier(),
        FailureCode::InvalidArgument,
    )))?;

    validate_line(&event)?;
    assert_eq!(event["sequence"], 0);
    assert_eq!(event["result"]["error"]["code"], "INVALID_ARGUMENT");
    Ok(())
}
