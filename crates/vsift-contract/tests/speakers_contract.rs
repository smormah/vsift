//! Conformance of speaker labels (0.3.0, ADR 0031): `transcript.speakers` and
//! `transcript.name-speaker` results, the `speakers` block and per-segment
//! `speaker_label` of `transcript.get` and `search`, against the published v1 schemas
//! and frozen examples.
//!
//! The revision is the F10 fixture's imported transcript (three segments), as in
//! `transcript_contract.rs`. The turns are made up: S1 holds the first segment, S2
//! the second, and the third has two voices, so the answer is partial.
//!
//! After an intended contract change, run with
//! `VSIFT_REGENERATE_CONTRACT_EXAMPLES=1` to rewrite the examples, and review the diff.

use std::{collections::BTreeMap, env, fs, io, num::NonZeroU32, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    ImportedRevisionRequest, SearchPageRequest, SpeakerFilter, SuppliedTranscript,
    TranscriptPageRequest, build_imported_revision, page_search_by_speaker, page_transcript,
    speaker_record_id,
};
use vsift_contract::{
    CommandName, LifecycleResponse, ResultView, SearchPresentation, SpeakersPresentation,
    TranscriptPresentation, UNLABELLED_SEGMENTS_WARNING, search_response_with_speakers,
    search_response_with_view, speakers_response, transcript_get_response,
    transcript_get_response_with_speakers,
};
use vsift_domain::{
    CueSource, CueText, CueTiming, DiarizationEngine, DiarizationProvenance, DiarizedTurn,
    EngineSpeaker, ImportedCue, MediaTime, PageLimit, ParsedTranscript, SearchQuery, SessionId,
    Sha256Hex, SidecarIdentity, SourceId, SpeakerCount, SpeakerLabellingProfile, SpeakerRecord,
    SpeakerRecordParts, SpeakerTag, TimeRange, TranscriptFormat, TranscriptOffset,
    TranscriptRevision, TranscriptWarnings, VerifiedName,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const F10_SOURCE: &str =
    "src_sha256_d7ccece71288c5ff35d7b16c871a93a8ed48bbb7380617899575069b4d6545f4";
const F10_SRT_SHA256: &str = "2a4ee37d826754eac43da03e7cc63f718b16aed4d3ce163c380ebd8ab9630847";
const ENGINE_SHA256: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SEGMENTATION_SHA256: &str =
    "220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079";
const EMBEDDING_SHA256: &str = "ad4a1802485d8b34c722d2a9d04249662f2ece5d28a7a039063ca22f515a789e";
const EXPIRES_AT: &str = "2026-09-25T00:00:00Z";
const CURSOR_EXPIRES_US: u64 = 1_790_294_400_000_000;
const NOW_US: u64 = 1_790_208_000_000_000;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";

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

/// Compares `value` with a frozen example, or rewrites the example when regenerating.
fn matches_example(relative_path: &str, value: &Value) -> TestResult {
    if regenerating() {
        let mut text = serde_json::to_string_pretty(value)?;
        text.push('\n');
        fs::write(schema_root().join(relative_path), text)?;
    }
    assert_eq!(value, &load(relative_path)?, "{relative_path}");
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

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&load(schema_path)?)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

fn f10_revision() -> Built<TranscriptRevision> {
    let mut cues = Vec::new();
    for (ordinal, line, start, end, text) in [
        (
            1,
            2,
            500_000,
            3_500_000,
            "This synthetic sidecar is aligned with an explicit 500 millisecond offset.",
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
        sidecar: SidecarIdentity::new(F10_SRT_SHA256, 245)?,
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

fn turn(start: u64, end: u64, speaker: u16) -> Built<DiarizedTurn> {
    Ok(DiarizedTurn::new(
        TimeRange::new(MediaTime::from_micros(start), MediaTime::from_micros(end))?,
        EngineSpeaker::new(speaker),
    ))
}

/// S1 holds the first segment, S2 the second; the third is half each.
fn record(revision: &TranscriptRevision, named: bool, number: u32) -> Built<SpeakerRecord> {
    let session = SessionId::parse(SESSION)?;
    let number = NonZeroU32::new(number).ok_or("zero number")?;
    let mut names = BTreeMap::new();
    if named {
        names.insert(
            SpeakerTag::parse("S2")?,
            VerifiedName::parse("Alice Example")?,
        );
    }
    Ok(SpeakerRecord::derive(
        SpeakerRecordParts {
            id: speaker_record_id(&session, revision.source_id(), revision.id(), number)?,
            number,
            source_id: revision.source_id().clone(),
            revision_id: revision.id().clone(),
            diarized_range: TimeRange::new(
                MediaTime::from_micros(0),
                MediaTime::from_micros(12_000_000),
            )?,
            count: SpeakerCount::given(2)?,
            profile: SpeakerLabellingProfile::V1,
            provenance: DiarizationProvenance::new(
                DiarizationEngine::SherpaOnnx,
                Sha256Hex::parse(ENGINE_SHA256)?,
                Sha256Hex::parse(SEGMENTATION_SHA256)?,
                Sha256Hex::parse(EMBEDDING_SHA256)?,
            ),
            turns: vec![
                turn(400_000, 3_600_000, 3)?,
                turn(4_500_000, 8_400_000, 7)?,
                turn(9_000_000, 10_000_000, 3)?,
                turn(10_000_000, 11_000_000, 7)?,
            ],
            names,
        },
        revision,
    )?)
}

fn lifecycle() -> LifecycleResponse {
    LifecycleResponse::ephemeral(EXPIRES_AT.to_owned())
}

#[test]
fn transcript_speakers_matches_its_schema_and_example_and_is_partial() -> TestResult {
    let revision = f10_revision()?;
    let session = SessionId::parse(SESSION)?;
    let record = record(&revision, false, 1)?;
    let response = serde_json::to_value(speakers_response(
        CommandName::TranscriptSpeakers,
        &SpeakersPresentation {
            session_id: &session,
            revision: &revision,
            record: &record,
            records: 1,
        },
        lifecycle(),
    )?)?;
    validate("operation-response.schema.json", &response)?;
    validate("transcript-speakers-data.schema.json", &response["data"])?;
    assert_eq!(response["command"], "transcript.speakers");
    assert_eq!(response["status"], "partial");
    assert_eq!(response["warnings"][0], UNLABELLED_SEGMENTS_WARNING);
    let labels = &response["data"]["speakers"]["labels"];
    assert_eq!(labels[0]["display"], "S1, by voice");
    assert_eq!(labels[1]["label"], "S2");
    assert_eq!(
        response["data"]["speakers"]["coverage"]["no_confident_speaker_segments"],
        1
    );
    matches_example("examples/transcript-speakers.json", &response)
}

#[test]
fn a_named_label_reads_as_the_users_statement() -> TestResult {
    let revision = f10_revision()?;
    let session = SessionId::parse(SESSION)?;
    let record = record(&revision, true, 2)?;
    let response = serde_json::to_value(speakers_response(
        CommandName::TranscriptNameSpeaker,
        &SpeakersPresentation {
            session_id: &session,
            revision: &revision,
            record: &record,
            records: 2,
        },
        lifecycle(),
    )?)?;
    validate("operation-response.schema.json", &response)?;
    validate("transcript-speakers-data.schema.json", &response["data"])?;
    let label = &response["data"]["speakers"]["labels"][1];
    assert_eq!(label["verified_name"], "Alice Example");
    assert_eq!(label["display"], "Alice Example (S2, named by the user)");
    Ok(())
}

fn page(
    revision: &TranscriptRevision,
) -> Built<(
    Vec<vsift_domain::TranscriptSegment>,
    vsift_domain::SearchCoverage,
)> {
    let session = SessionId::parse(SESSION)?;
    let page = page_transcript(
        &session,
        revision,
        &TranscriptPageRequest {
            range: TimeRange::new(
                MediaTime::from_micros(0),
                MediaTime::from_micros(12_000_000),
            )?,
            limit: PageLimit::new(20)?,
            cursor: None,
        },
        CURSOR_EXPIRES_US,
        NOW_US,
    )?;
    Ok((page.segments.into_iter().cloned().collect(), page.coverage))
}

#[test]
fn transcript_get_carries_labels_only_when_the_revision_has_them() -> TestResult {
    let revision = f10_revision()?;
    let session = SessionId::parse(SESSION)?;
    let (segments, coverage) = page(&revision)?;
    let presentation = TranscriptPresentation {
        session_id: &session,
        revision: &revision,
        range: TimeRange::new(
            MediaTime::from_micros(0),
            MediaTime::from_micros(12_000_000),
        )?,
        segments: &segments,
        next_cursor: None,
        coverage: &coverage,
    };
    // Without speaker labels the answer is exactly what it always was.
    let plain = serde_json::to_value(transcript_get_response(
        &presentation,
        lifecycle(),
        ResultView::Full,
    )?)?;
    let without = serde_json::to_value(transcript_get_response_with_speakers(
        &presentation,
        None,
        lifecycle(),
        ResultView::Full,
    )?)?;
    assert_eq!(plain, without);
    assert!(plain["data"].get("speakers").is_none());
    assert!(plain["data"]["items"][0].get("speaker_label").is_none());

    let record = record(&revision, true, 2)?;
    let full = serde_json::to_value(transcript_get_response_with_speakers(
        &presentation,
        Some(&record),
        lifecycle(),
        ResultView::Full,
    )?)?;
    validate("operation-response.schema.json", &full)?;
    validate("transcript-get-data.schema.json", &full["data"])?;
    let items = full["data"]["items"].as_array().ok_or("items missing")?;
    for item in items {
        validate("transcript-segment.schema.json", item)?;
    }
    assert_eq!(items[0]["speaker_label"]["display"], "S1, by voice");
    assert_eq!(
        items[1]["speaker_label"]["display"],
        "Alice Example (S2, named by the user)"
    );
    assert_eq!(items[2]["speaker_label"]["label"], Value::Null);
    assert_eq!(items[2]["speaker_label"]["no_confident_speaker"], "mixed");
    assert_eq!(full["warnings"][0], UNLABELLED_SEGMENTS_WARNING);
    matches_example("examples/transcript-get.speakers.json", &full)?;

    let brief = serde_json::to_value(transcript_get_response_with_speakers(
        &presentation,
        Some(&record),
        lifecycle(),
        ResultView::Brief,
    )?)?;
    validate("transcript-get-brief-data.schema.json", &brief["data"])?;
    for item in brief["data"]["items"].as_array().ok_or("items missing")? {
        validate("transcript-segment-brief.schema.json", item)?;
    }
    assert_eq!(brief["data"]["items"][1]["speaker_label"]["label"], "S2");
    Ok(())
}

#[test]
fn a_search_filtered_by_speaker_finds_only_that_speakers_segments() -> TestResult {
    let revision = f10_revision()?;
    let session = SessionId::parse(SESSION)?;
    let record = record(&revision, true, 2)?;
    // "synthetic" is in the first and the third segment; S1 holds only the first.
    let query = SearchQuery::parse("synthetic")?;
    let request = SearchPageRequest {
        query: query.clone(),
        range: None,
        limit: PageLimit::new(20)?,
        cursor: None,
    };
    let tag = SpeakerTag::parse("S1")?;
    let filtered = page_search_by_speaker(
        &session,
        &revision,
        &request,
        Some(SpeakerFilter {
            record: &record,
            tag,
        }),
        CURSOR_EXPIRES_US,
        NOW_US,
    )?;
    assert_eq!(filtered.hits.len(), 1);
    let presentation = SearchPresentation {
        session_id: &session,
        revision: &revision,
        query: &query,
        range: None,
        hits: filtered
            .hits
            .iter()
            .map(|hit| (hit.segment(), hit.tier()))
            .collect(),
        next_cursor: None,
        coverage: &filtered.coverage,
    };
    let response = serde_json::to_value(search_response_with_speakers(
        &presentation,
        Some(&record),
        Some(tag),
        lifecycle(),
        ResultView::Full,
    )?)?;
    validate("operation-response.schema.json", &response)?;
    validate("search-data.schema.json", &response["data"])?;
    assert_eq!(response["data"]["query"]["speaker"], "S1");
    assert_eq!(response["data"]["items"][0]["speaker_label"]["label"], "S1");
    matches_example("examples/search.speakers.json", &response)?;

    let unfiltered = page_search_by_speaker(
        &session,
        &revision,
        &request,
        None,
        CURSOR_EXPIRES_US,
        NOW_US,
    )?;
    assert_eq!(unfiltered.hits.len(), 2);
    let plain = SearchPresentation {
        hits: unfiltered
            .hits
            .iter()
            .map(|hit| (hit.segment(), hit.tier()))
            .collect(),
        coverage: &unfiltered.coverage,
        ..presentation
    };
    let before = serde_json::to_value(search_response_with_view(
        &plain,
        lifecycle(),
        ResultView::Full,
    )?)?;
    assert!(before["data"].get("speakers").is_none());
    assert!(before["data"]["query"].get("speaker").is_none());
    Ok(())
}
