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
    LifecycleResponse, OpenData, OperationResponse, TerminalEventResponse, TranscriptPageData,
    TranscriptSegmentData, transcript_rejection_summary, transcript_warning_messages,
};
use vsift_domain::{
    CueSource, CueText, CueTiming, FailureCode, ImportedCue, MediaTime, PageLimit,
    ParsedTranscript, PublicationGuarantee, SessionId, SessionLifetime, SidecarIdentity, SourceId,
    StorageGeneration, TimeRange, TranscriptFormat, TranscriptImportError, TranscriptOffset,
    TranscriptRejection, TranscriptRevision, TranscriptWarnings,
};

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
        &TranscriptPageData::new(&session, revision, range, &segments, cursor.as_deref()),
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
