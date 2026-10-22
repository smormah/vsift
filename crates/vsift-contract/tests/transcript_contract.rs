//! Conformance of the P07 transcript contract: `ingest` with a supplied
//! transcript, `transcript.get` pages, transcript segment evidence records and
//! transcript rejection failures, against the published v1 schemas and frozen
//! examples.
//!
//! The examples describe the F10 fixture: its real source identity and the
//! real digest of `fixtures/corpus/transcripts/F10.srt`, imported with the
//! explicit +500 ms offset. Identities derive from content, so they are exact.

use std::{fs, io, num::NonZeroU32, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    ImportedRevisionRequest, OpenSessionOutcome, SuppliedTranscript, TranscriptPageRequest,
    build_imported_revision, page_transcript,
};
use vsift_contract::{
    LifecycleResponse, OpenData, OperationResponse, ResultView, TerminalEventResponse,
    TranscriptEvidenceStream, TranscriptPageData, TranscriptPresentation, TranscriptSegmentData,
    transcript_get_response, transcript_rejection_summary, transcript_warning_messages,
};
use vsift_domain::{
    CueSource, CueText, CueTiming, FailureCode, ImportedCue, MediaTime, PageLimit,
    ParsedTranscript, PublicationGuarantee, SearchCoverage, SessionId, SessionLifetime,
    SidecarIdentity, SourceId, SpeakerLabel, StorageGeneration, TimeRange, TranscriptFormat,
    TranscriptImportError, TranscriptOffset, TranscriptRejection, TranscriptRevision,
    TranscriptWarnings,
};

mod common;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const F10_SOURCE: &str =
    "src_sha256_d7ccece71288c5ff35d7b16c871a93a8ed48bbb7380617899575069b4d6545f4";
const F10_SOURCE_BYTES: u64 = 41_473;
const F10_SRT_SHA256: &str = "2a4ee37d826754eac43da03e7cc63f718b16aed4d3ce163c380ebd8ab9630847";
const F10_SRT_BYTES: u64 = 245;
const EXPIRES_AT: &str = "2026-09-25T00:00:00Z";
/// Session expiry and "now" used to scope cursors, in microseconds.
const CURSOR_EXPIRES_US: u64 = 1_790_294_400_000_000;
const NOW_US: u64 = 1_790_208_000_000_000;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative_path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative_path),
    )?)?)
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

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema = load(schema_path)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

fn f10_cues() -> Result<Vec<ImportedCue>, Box<dyn std::error::Error>> {
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
    Ok(cues)
}

fn f10_revision() -> Result<TranscriptRevision, Box<dyn std::error::Error>> {
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::Srt,
            None,
            f10_cues()?,
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

fn ingest_response(
    revision: &TranscriptRevision,
) -> Result<OperationResponse<Value>, Box<dyn std::error::Error>> {
    let opened = OpenSessionOutcome {
        session_id: SessionId::parse(SESSION)?,
        source_id: SourceId::parse(F10_SOURCE)?,
        source_bytes: F10_SOURCE_BYTES,
        generation: StorageGeneration::from_value(1),
        publication: PublicationGuarantee::ProcessCrashConsistent,
        lifetime: SessionLifetime::open(1_790_208_000)?,
    };
    Ok(OperationResponse::complete(
        "ingest",
        &OpenData::new(&opened, EXPIRES_AT.to_owned()).with_transcript(revision),
    )?
    .with_lifecycle(LifecycleResponse::ephemeral(EXPIRES_AT.to_owned()))
    .with_warnings(&transcript_warning_messages(revision)))
}

fn page_response(
    revision: &TranscriptRevision,
    from: u64,
    to: u64,
    limit: u16,
) -> Result<OperationResponse<Value>, Box<dyn std::error::Error>> {
    let session = SessionId::parse(SESSION)?;
    let range = TimeRange::new(MediaTime::from_micros(from), MediaTime::from_micros(to))?;
    let page = page_transcript(
        &session,
        revision,
        &TranscriptPageRequest {
            range,
            limit: PageLimit::new(limit)?,
            cursor: None,
        },
        CURSOR_EXPIRES_US,
        NOW_US,
    )?;
    let segments: Vec<_> = page.segments.into_iter().cloned().collect();
    let cursor = page.next_cursor.map(|cursor| cursor.encode());
    Ok(OperationResponse::complete(
        "transcript.get",
        &TranscriptPageData::new(&TranscriptPresentation {
            session_id: &session,
            revision,
            range,
            segments: &segments,
            next_cursor: cursor.as_deref(),
            coverage: &page.coverage,
            growth: vsift_contract::ReadGrowth::default(),
        }),
    )?
    .with_lifecycle(LifecycleResponse::ephemeral(EXPIRES_AT.to_owned())))
}

#[test]
fn ingest_with_a_supplied_transcript_matches_the_frozen_example() -> TestResult {
    let response = serde_json::to_value(ingest_response(&f10_revision()?)?)?;

    validate("operation-response.schema.json", &response)?;
    validate("ingest-data.schema.json", &response["data"])?;
    validate(
        "transcript-revision.schema.json",
        &response["data"]["transcript"],
    )?;
    assert_eq!(response, load("examples/ingest.transcript.json")?);
    Ok(())
}

#[test]
fn transcript_get_page_matches_the_frozen_example() -> TestResult {
    let response = serde_json::to_value(page_response(&f10_revision()?, 0, 12_000_000, 2)?)?;

    validate("operation-response.schema.json", &response)?;
    validate("transcript-get-data.schema.json", &response["data"])?;
    for item in response["data"]["items"]
        .as_array()
        .ok_or("items missing")?
    {
        validate("transcript-segment.schema.json", item)?;
    }
    assert_eq!(response, load("examples/transcript-get.json")?);

    let event = serde_json::to_value(TerminalEventResponse::new(page_response(
        &f10_revision()?,
        0,
        12_000_000,
        2,
    )?))?;
    validate("terminal-event.schema.json", &event)?;
    validate("operation-response.schema.json", &event["result"])?;
    Ok(())
}

/// A-09 citation: F10's truth window returns exactly the dialog segment.
#[test]
fn the_f10_truth_window_cites_the_dialog_segment() -> TestResult {
    let response =
        serde_json::to_value(page_response(&f10_revision()?, 5_000_000, 9_000_000, 20)?)?;
    let items = response["data"]["items"]
        .as_array()
        .ok_or("items missing")?;

    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["text"], "Dialog R-17 is displayed now.");
    assert_eq!(items[0]["start_us"], 5_000_000);
    assert_eq!(items[0]["end_us"], 9_000_000);
    assert_eq!(items[0]["alignment"]["cue_start_us"], 4_500_000);
    assert_eq!(items[0]["alignment"]["offset_us"], 500_000);
    assert!(response["data"]["next_cursor"].is_null());
    Ok(())
}

#[test]
fn published_transcript_examples_validate_against_their_schemas() -> TestResult {
    let ingest = load("examples/ingest.transcript.json")?;
    validate("operation-response.schema.json", &ingest)?;
    validate("ingest-data.schema.json", &ingest["data"])?;
    let page = load("examples/transcript-get.json")?;
    validate("operation-response.schema.json", &page)?;
    validate("transcript-get-data.schema.json", &page["data"])?;
    let rejected = load("examples/transcript-rejected.json")?;
    validate("operation-response.schema.json", &rejected)?;

    for (schema, instance) in [
        ("ingest-data.schema.json", &ingest["data"]),
        ("transcript-get-data.schema.json", &page["data"]),
        ("transcript-segment.schema.json", &page["data"]["items"][0]),
        ("transcript-revision.schema.json", &page["data"]["revision"]),
    ] {
        let mut extended = instance.clone();
        extended
            .as_object_mut()
            .ok_or("not an object")?
            .insert("unreviewed".to_owned(), Value::Bool(true));
        assert!(validate(schema, &extended).is_err(), "{schema}");
    }
    Ok(())
}

/// A plain ingest keeps its existing shape: no transcript member is added.
#[test]
fn plain_ingest_data_is_unchanged_and_schema_valid() -> TestResult {
    let opened = OpenSessionOutcome {
        session_id: SessionId::parse(SESSION)?,
        source_id: SourceId::parse(F10_SOURCE)?,
        source_bytes: 10,
        generation: StorageGeneration::from_value(1),
        publication: PublicationGuarantee::ProcessCrashConsistent,
        lifetime: SessionLifetime::open(1_000)?,
    };
    let data = serde_json::to_value(OpenData::new(&opened, EXPIRES_AT.to_owned()))?;

    validate("ingest-data.schema.json", &data)?;
    assert_eq!(
        data,
        serde_json::json!({
            "session_id": SESSION,
            "source_id": F10_SOURCE,
            "source_bytes": 10,
            "generation": 1,
            "publication": "process_crash_consistent",
            "expires_at": EXPIRES_AT
        })
    );
    Ok(())
}

#[test]
fn transcript_rejection_failure_matches_the_frozen_example() -> TestResult {
    let response = serde_json::to_value(OperationResponse::failure_with_remediation(
        "ingest",
        FailureCode::InvalidSource,
        transcript_rejection_summary(TranscriptImportError::at_line(
            TranscriptRejection::InvalidTimestamp,
            6,
        )),
    ))?;

    validate("operation-response.schema.json", &response)?;
    assert_eq!(response, load("examples/transcript-rejected.json")?);
    Ok(())
}

#[test]
fn every_rejection_has_bounded_schema_valid_remediation() -> TestResult {
    for rejection in [
        TranscriptRejection::TooLarge,
        TranscriptRejection::UnsupportedEncoding,
        TranscriptRejection::InvalidUtf8,
        TranscriptRejection::ControlCharacter,
        TranscriptRejection::LineTooLong,
        TranscriptRejection::TooManyCues,
        TranscriptRejection::CueTextTooLong,
        TranscriptRejection::InvalidHeader,
        TranscriptRejection::InvalidCueNumber,
        TranscriptRejection::InvalidTimingLine,
        TranscriptRejection::InvalidTimestamp,
        TranscriptRejection::NonPositiveDuration,
        TranscriptRejection::OutOfOrder,
        TranscriptRejection::UntimedText,
        TranscriptRejection::NoCues,
        TranscriptRejection::OffsetOutOfRange,
        TranscriptRejection::NoCuesWithinSource,
    ] {
        let summary =
            transcript_rejection_summary(TranscriptImportError::at_line(rejection, u32::MAX));
        assert!(summary.contains(rejection.identifier()));
        let response = serde_json::to_value(OperationResponse::failure_with_remediation(
            "ingest",
            FailureCode::InvalidSource,
            summary,
        ))?;
        validate("operation-response.schema.json", &response)?;
    }
    Ok(())
}

/// C-10 and untrusted text: imported segments state unknown confidence, and
/// any control character left in an original payload is replaced on output.
#[test]
fn segments_state_unknown_confidence_and_sanitize_original_text() -> TestResult {
    let mut cues = f10_cues()?;
    cues[1].text = CueText::new(
        "Dialog R-17 is displayed now.".to_owned(),
        "Dialog\tR-17 is <b>displayed</b> now.".to_owned(),
    )?;
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::Srt,
            None,
            cues,
            TranscriptWarnings::default(),
        )?,
        sidecar: SidecarIdentity::new(F10_SRT_SHA256, F10_SRT_BYTES)?,
    };
    let revision = build_imported_revision(ImportedRevisionRequest {
        session_id: &SessionId::parse(SESSION)?,
        source_id: &SourceId::parse(F10_SOURCE)?,
        source_duration: MediaTime::from_micros(12_000_000),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?;
    let segment = serde_json::to_value(TranscriptSegmentData::new(
        &revision,
        &revision.segments()[1],
    ))?;

    validate("transcript-segment.schema.json", &segment)?;
    assert_eq!(segment["confidence"]["value_basis_points"], Value::Null);
    assert_eq!(segment["confidence"]["origin"], "unavailable");
    assert_eq!(segment["markup"], "removed");
    assert_eq!(
        segment["original_text"],
        "Dialog\u{fffd}R-17 is <b>displayed</b> now."
    );
    assert_eq!(segment["speaker"], Value::Null);
    Ok(())
}

/// `display_text` (ADR 0008, note of 2026-09-29): every hidden character of
/// `text` written as `<U+XXXX>`, plain text unchanged, literal notation never
/// escaped again; `text` and `original_text` keep the characters raw. A
/// speaker label gets the same rendering in `display_label`.
#[test]
fn display_text_makes_hidden_characters_visible_and_keeps_text_raw() -> TestResult {
    let mut cues = f10_cues()?;
    cues[0].text = CueText::new(
        "Status: \u{202E}DELIAF\u{202C} build\u{200B} pass\u{200D}ed".to_owned(),
        "Status: &#x202E;DELIAF&#x202C; build&#x200B; pass&#8205;ed".to_owned(),
    )?;
    cues[1].text = CueText::new(
        "The slide literally reads <U+202E> and <U+".to_owned(),
        "The slide literally reads <U+202E> and <U+".to_owned(),
    )?;
    cues[1].speaker = Some(SpeakerLabel::parse("Admin\u{2066}istrator")?);
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::WebVtt,
            None,
            cues,
            TranscriptWarnings::default(),
        )?,
        sidecar: SidecarIdentity::new(F10_SRT_SHA256, F10_SRT_BYTES)?,
    };
    let revision = build_imported_revision(ImportedRevisionRequest {
        session_id: &SessionId::parse(SESSION)?,
        source_id: &SourceId::parse(F10_SOURCE)?,
        source_duration: MediaTime::from_micros(12_000_000),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?;
    let segments: Vec<Value> = revision
        .segments()
        .iter()
        .map(|segment| serde_json::to_value(TranscriptSegmentData::new(&revision, segment)))
        .collect::<Result<_, _>>()?;
    for segment in &segments {
        validate("transcript-segment.schema.json", segment)?;
    }

    let hidden = &segments[0];
    assert_eq!(
        hidden["text"],
        "Status: \u{202E}DELIAF\u{202C} build\u{200B} pass\u{200D}ed"
    );
    assert_eq!(
        hidden["display_text"],
        "Status: <U+202E>DELIAF<U+202C> build<U+200B> pass<U+200D>ed"
    );
    assert_eq!(
        hidden["original_text"],
        "Status: &#x202E;DELIAF&#x202C; build&#x200B; pass&#8205;ed"
    );

    let literal = &segments[1];
    assert_eq!(literal["display_text"], literal["text"]);
    assert_eq!(
        literal["display_text"],
        "The slide literally reads <U+202E> and <U+"
    );
    assert_eq!(literal["speaker"]["label"], "Admin\u{2066}istrator");
    assert_eq!(literal["speaker"]["display_label"], "Admin<U+2066>istrator");

    let plain = &segments[2];
    assert_eq!(plain["display_text"], plain["text"]);
    assert_eq!(
        plain["display_text"],
        "End of the synthetic imported transcript."
    );
    Ok(())
}

// --- The views and the coverage of a read (0.2.1: #365 and #373) ---------------

type Built<T> = Result<T, Box<dyn std::error::Error>>;

/// Whether `brief` is a projection of `full`: every member of `brief`, at every
/// depth, is a member of `full` with the same value, and an array is one when
/// each of its elements is. The views cannot drift apart because a brief member
/// is only ever a full member.
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

fn members(value: &Value) -> Built<Vec<&str>> {
    Ok(value
        .as_object()
        .ok_or("not an object")?
        .keys()
        .map(String::as_str)
        .collect())
}

/// One page of `revision` through `transcript_get_response`, as the CLI
/// presents it, in `view`.
fn view_response(
    revision: &TranscriptRevision,
    range: TimeRange,
    limit: u16,
    view: ResultView,
) -> Built<Value> {
    let session = SessionId::parse(SESSION)?;
    let page = page_transcript(
        &session,
        revision,
        &TranscriptPageRequest {
            range,
            limit: PageLimit::new(limit)?,
            cursor: None,
        },
        CURSOR_EXPIRES_US,
        NOW_US,
    )?;
    let segments: Vec<_> = page.segments.into_iter().cloned().collect();
    let cursor = page.next_cursor.map(|cursor| cursor.encode());
    Ok(serde_json::to_value(transcript_get_response(
        &TranscriptPresentation {
            session_id: &session,
            revision,
            range,
            segments: &segments,
            next_cursor: cursor.as_deref(),
            coverage: &page.coverage,
            growth: vsift_contract::ReadGrowth::default(),
        },
        LifecycleResponse::ephemeral(EXPIRES_AT.to_owned()),
        view,
    )?)?)
}

fn micros(from: u64, to: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from),
        MediaTime::from_micros(to),
    )?)
}

/// #365: the brief page is the full page with each item reduced to what a
/// reader cites and places. The envelope and every member outside the items are
/// the full page's own, value for value; each brief item is a projection of its
/// full item; the full answer names no view and the brief one says so.
#[test]
fn the_brief_page_is_a_projection_of_the_full_page() -> TestResult {
    let revision = f10_revision()?;
    let range = micros(0, 12_000_000)?;
    let full = view_response(&revision, range, 2, ResultView::Full)?;
    let brief = view_response(&revision, range, 2, ResultView::Brief)?;

    validate("operation-response.schema.json", &brief)?;
    validate("transcript-get-brief-data.schema.json", &brief["data"])?;
    assert!(full["data"].get("view").is_none());
    assert_eq!(brief["data"]["view"], "brief");
    // The default view is the answer every release gave, plus the new member.
    assert_eq!(
        full,
        serde_json::to_value(page_response(&revision, 0, 12_000_000, 2)?)?
    );

    for (member, value) in full.as_object().ok_or("not an object")? {
        if member != "data" {
            assert_eq!(&brief[member], value, "envelope member {member}");
        }
    }
    for (member, value) in full["data"].as_object().ok_or("not an object")? {
        if member != "items" {
            assert_eq!(&brief["data"][member], value, "data member {member}");
        }
    }
    let (full_items, brief_items) = (
        full["data"]["items"].as_array().ok_or("items missing")?,
        brief["data"]["items"].as_array().ok_or("items missing")?,
    );
    assert_eq!(full_items.len(), 2);
    assert_eq!(brief_items.len(), full_items.len());
    for (brief_item, full_item) in brief_items.iter().zip(full_items) {
        validate("transcript-segment-brief.schema.json", brief_item)?;
        assert!(is_projection_of(brief_item, full_item));
        // An item with no speaker names none, rather than a null.
        assert_eq!(
            members(brief_item)?,
            ["display_text", "end_us", "segment_id", "start_us"]
        );
    }
    // What the brief item leaves out is in the revision block, once.
    assert_eq!(
        brief["data"]["revision"]["revision_id"],
        full_items[0]["revision_id"]
    );
    assert_eq!(
        brief["data"]["revision"]["source_id"],
        full_items[0]["source_id"]
    );
    assert_eq!(
        brief["data"]["revision"]["source_segments"][0]["source_segment_id"],
        full_items[0]["source_segment_id"]
    );
    Ok(())
}

/// #365: the brief page of fifty imported cues (the committed example's items
/// repeated to fifty, `common::page_of_fifty`) is under three tenths of the full
/// page: 35,511 and 9,251 characters, 26.1%.
#[test]
fn a_brief_page_of_fifty_imported_cues_has_the_measured_share() -> TestResult {
    let revision = f10_revision()?;
    let range = micros(0, 12_000_000)?;
    let full = common::page_of_fifty(&view_response(&revision, range, 2, ResultView::Full)?)?;
    let brief = common::page_of_fifty(&view_response(&revision, range, 2, ResultView::Brief)?)?;
    assert!(brief * 10 < full * 3, "{brief} of {full}");
    Ok(())
}

/// #365: a page of fifty segments, the `standard` profile's page size, in
/// both views. The brief page is a fraction of the full page (the issue: a
/// client cut a 54,000-character page at about 43,000 and lost its last items
/// and its cursor).
#[test]
fn a_brief_page_of_fifty_segments_is_a_fraction_of_the_full_page() -> TestResult {
    let mut cues = Vec::new();
    for ordinal in 1..=50_u32 {
        let start = u64::from(ordinal) * 2_000_000;
        let text = format!(
            "Step {ordinal} of the walkthrough opens the settings page and checks the build."
        );
        cues.push(ImportedCue {
            source: CueSource::new(
                NonZeroU32::new(ordinal).ok_or("zero ordinal")?,
                NonZeroU32::new(ordinal * 4).ok_or("zero line")?,
            ),
            timing: CueTiming::new(start, start + 1_800_000)?,
            text: CueText::new(text.clone(), text)?,
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
    let revision = build_imported_revision(ImportedRevisionRequest {
        session_id: &SessionId::parse(SESSION)?,
        source_id: &SourceId::parse(F10_SOURCE)?,
        source_duration: MediaTime::from_micros(120_000_000),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(0)?,
        number: NonZeroU32::MIN,
    })?;
    let range = micros(0, 120_000_000)?;
    let full = view_response(&revision, range, 50, ResultView::Full)?;
    let brief = view_response(&revision, range, 50, ResultView::Brief)?;
    assert_eq!(full["data"]["items"].as_array().map(Vec::len), Some(50));
    assert_eq!(brief["data"]["items"].as_array().map(Vec::len), Some(50));
    let (full_size, brief_size) = (
        serde_json::to_string(&full)?.len(),
        serde_json::to_string(&brief)?.len(),
    );
    assert!(
        brief_size * 3 < full_size,
        "a brief page of 50 is {brief_size} characters, the full page {full_size}"
    );
    Ok(())
}

/// A speaker is shown by its `display_label` and its origin, never its raw
/// label; the text is `display_text`, with the hidden characters visible; the
/// raw `text`, `original_text` and `label` are in the full view only.
#[test]
fn a_brief_item_quotes_display_text_and_the_display_label_only() -> TestResult {
    let mut cues = f10_cues()?;
    cues[0].text = CueText::new(
        "Status: \u{202E}DELIAF\u{202C} build".to_owned(),
        "Status: &#x202E;DELIAF&#x202C; build".to_owned(),
    )?;
    cues[1].speaker = Some(SpeakerLabel::parse("Admin\u{2066}istrator")?);
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::WebVtt,
            None,
            cues,
            TranscriptWarnings::default(),
        )?,
        sidecar: SidecarIdentity::new(F10_SRT_SHA256, F10_SRT_BYTES)?,
    };
    let revision = build_imported_revision(ImportedRevisionRequest {
        session_id: &SessionId::parse(SESSION)?,
        source_id: &SourceId::parse(F10_SOURCE)?,
        source_duration: MediaTime::from_micros(12_000_000),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?;
    let range = micros(0, 12_000_000)?;
    let brief = view_response(&revision, range, 20, ResultView::Brief)?;
    validate("transcript-get-brief-data.schema.json", &brief["data"])?;
    let items = brief["data"]["items"].as_array().ok_or("items missing")?;
    assert_eq!(
        items[0]["display_text"],
        "Status: <U+202E>DELIAF<U+202C> build"
    );
    assert_eq!(
        items[1]["speaker"],
        serde_json::json!({
            "display_label": "Admin<U+2066>istrator",
            "origin": "imported_webvtt_voice",
        })
    );
    // The raw forms are not in the answer at all, anywhere.
    let text = serde_json::to_string(&brief)?;
    for raw in [
        "\u{202E}",
        "\u{2066}",
        "\\u202e",
        "\\u2066",
        "original_text",
    ] {
        assert!(!text.contains(raw), "{raw:?} is in the brief answer");
    }
    // A brief item is a projection of the full item, speaker included.
    let full = view_response(&revision, range, 20, ResultView::Full)?;
    let full_items = full["data"]["items"].as_array().ok_or("items missing")?;
    for (brief_item, full_item) in items.iter().zip(full_items) {
        assert!(is_projection_of(brief_item, full_item));
    }
    Ok(())
}

/// #373: an imported transcript is taken to cover the whole source, so a read
/// of a range with no cue in it is complete: it says what it covers and that
/// the file's completeness is not verified (`basis`), and keeps `coverage`
/// null as the answer always had. A range past the end of the video is
/// clipped to it; one wholly outside it searches nothing and is complete.
#[test]
fn an_imported_transcript_covers_the_whole_source() -> TestResult {
    let revision = f10_revision()?;
    for (from, to, searched) in [
        // No cue lies in 3.5-4.5 s (cues: 0.5-3.5, 4.5-8.5, 9-11 s).
        (3_500_000, 4_500_000, Some((3_500_000, 4_500_000))),
        // The source is 12 s long: the request is clipped to it.
        (11_000_000, 20_000_000, Some((11_000_000, 12_000_000))),
        // Wholly past the end: nothing to cover.
        (20_000_000, 30_000_000, None),
    ] {
        let response = view_response(&revision, micros(from, to)?, 20, ResultView::Full)?;
        validate("operation-response.schema.json", &response)?;
        validate("transcript-get-data.schema.json", &response["data"])?;
        assert_eq!(response["status"], "complete", "{from}-{to}");
        assert!(response["coverage"].is_null(), "{from}-{to}");
        assert_eq!(response["warnings"], serde_json::json!([]), "{from}-{to}");
        let coverage = &response["data"]["transcript_coverage"];
        assert_eq!(coverage["basis"], "supplied_transcript");
        assert_eq!(coverage["scope"], "transcript_text");
        assert_eq!(coverage["untranscribed_ranges"], serde_json::json!([]));
        assert_eq!(coverage["no_speech_ranges"], serde_json::json!([]));
        assert_eq!(coverage["ranges_truncated"], false);
        if let Some((start, end)) = searched {
            let range = serde_json::json!({"from_us": start, "to_us": end});
            assert_eq!(coverage["searched_range"], range);
            assert_eq!(coverage["transcribed_ranges"], serde_json::json!([range]));
        } else {
            assert!(coverage["searched_range"].is_null());
            assert_eq!(coverage["transcribed_ranges"], serde_json::json!([]));
        }
    }
    Ok(())
}

/// #373: the member is always there, on the page of any range and in the
/// terminal data of the stream, and the stream's records are untouched by it.
#[test]
fn the_stream_states_the_coverage_its_page_does() -> TestResult {
    let revision = f10_revision()?;
    let range = micros(0, 12_000_000)?;
    let session = SessionId::parse(SESSION)?;
    let stream = TranscriptEvidenceStream::new(
        &TranscriptPresentation {
            session_id: &session,
            revision: &revision,
            range,
            segments: &revision.segments()[..2],
            next_cursor: None,
            coverage: &SearchCoverage::of(&revision, Some(range)),
            growth: vsift_contract::ReadGrowth::default(),
        },
        LifecycleResponse::ephemeral(EXPIRES_AT.to_owned()),
    )?;
    let terminal = serde_json::to_value(stream.terminal())?;
    validate("terminal-event.schema.json", &terminal)?;
    validate(
        "transcript-get-stream-data.schema.json",
        &terminal["result"]["data"],
    )?;
    let page = view_response(&revision, range, 2, ResultView::Full)?;
    assert_eq!(
        terminal["result"]["data"]["transcript_coverage"],
        page["data"]["transcript_coverage"]
    );
    assert_eq!(terminal["result"]["status"], "complete");
    // The records are the full segments, byte for byte, in every case.
    for (record, segment) in stream.records().iter().zip(revision.segments()) {
        let value = serde_json::to_value(record)?;
        assert_eq!(
            value["record"],
            serde_json::to_value(TranscriptSegmentData::new(&revision, segment))?
        );
        assert_eq!(value["key"], value["record"]["segment_id"]);
    }
    Ok(())
}

/// The new members are listed in the published schemas, and a view the
/// contract names is a view the brief schemas allow.
#[test]
fn the_published_schemas_describe_the_views_and_the_coverage() -> TestResult {
    let brief = load("transcript-get-brief-data.schema.json")?;
    assert_eq!(brief["properties"]["view"]["const"], "brief");
    assert!(
        brief["required"]
            .as_array()
            .ok_or("no required list")?
            .contains(&Value::from("transcript_coverage"))
    );
    for view in ResultView::ALL {
        // `full` is the absence of the member: only `brief` names itself.
        assert_eq!(
            brief["properties"]["view"]["const"] == view.identifier(),
            view == ResultView::Brief
        );
    }
    // The 0.1.0 answers have no coverage and stay valid against the schema:
    // the member is always emitted, and the schema cannot require it.
    let mut old = load("examples/transcript-get.json")?;
    old["data"]
        .as_object_mut()
        .ok_or("data is not an object")?
        .remove("transcript_coverage");
    validate("transcript-get-data.schema.json", &old["data"])?;
    Ok(())
}
