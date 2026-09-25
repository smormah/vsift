//! Conformance of the local-ASR contract: `transcript.retranscribe` results,
//! local-ASR and spliced transcript revisions, their segment records as a page
//! and as an evidence stream, and the fixed-prose failures, against the
//! published v1 schemas and frozen examples.
//!
//! The examples describe the F01 speech clip
//! (`fixtures/corpus/generated/F01-speech.mp4`): revision 1 is the whole clip
//! transcribed by the reviewed whisper.cpp v1.9.2 build with the pinned base
//! model, built from its recorded output
//! (`crates/vsift-infrastructure/tests/fixtures/whisper-1.9.2/F01.base.json`);
//! revision 2 retranscribes 5.5-6 s, where the clip is silent, so it carries
//! revision 1's segment and records a silent chunk and no new speech.

use std::{fs, io, num::NonZeroU16, num::NonZeroU32, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    AsrFailure, AsrFailureReason, AsrRevisionRequest, AsrStage, AsrTranscription,
    LocalAsrVerificationFailure, RevisionSplice, TranscriptPageRequest, build_asr_revision,
    page_transcript, whole_file_source_segment,
};
use vsift_contract::{
    LOCAL_ASR_MODEL_REMEDIATION, LOCAL_ASR_TOOLS_REMEDIATION, LifecycleResponse,
    NO_AUDIO_STREAM_REMEDIATION, NO_TRANSCRIPT_REMEDIATION, OperationResponse,
    TerminalEventResponse, TranscriptEvidenceStream, TranscriptPageData,
    TranscriptRetranscribeData, TranscriptRevisionData, UNKNOWN_REVISION_REMEDIATION,
    UNPINNED_MODEL_REMEDIATION, local_asr_failure_summary, local_asr_verification_summary,
    transcript_warning_messages,
};
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, ChunkTime, CueText, FailureCode, LanguageTag,
    MediaTime, PageLimit, ProviderChunkOutput, ProviderOutputError, ProviderSegment, ProviderToken,
    ProviderTokenKind, SessionId, Sha256Hex, SourceId, SourceSegment, TimeRange,
    TranscriptRevision, TranscriptRevisionError, TranscriptWarningKind, TranscriptWarnings,
    merge_chunks, plan_chunks, validate_chunk_output,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const F01_SPEECH_SOURCE: &str =
    "src_sha256_f8222a928243160c8dbf5b1a9bc24277e49ac47c11fe5526826462877678e881";
const WHISPER_SHA256: &str = "95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d";
const BASE_MODEL_SHA256: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";
const EXPIRES_AT: &str = "2026-09-25T00:00:00Z";
const CURSOR_EXPIRES_US: u64 = 1_790_294_400_000_000;
const NOW_US: u64 = 1_790_208_000_000_000;
const SECOND: u64 = 1_000_000;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn load(relative_path: &str) -> Built<Value> {
    Ok(serde_json::from_str(&fs::read_to_string(
        repository("schemas/v1").join(relative_path),
    )?)?)
}

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
            repository("schemas/v1").join(name),
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

fn range(from: u64, to: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from),
        MediaTime::from_micros(to),
    )?)
}

/// The reviewed build's recorded `-ojf` output for F01, read as the adapter
/// would read it: offsets in milliseconds, trimmed text, token probabilities.
fn recorded_f01_output() -> Built<ProviderChunkOutput> {
    let raw: Value = serde_json::from_slice(&fs::read(repository(
        "crates/vsift-infrastructure/tests/fixtures/whisper-1.9.2/F01.base.json",
    ))?)?;
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
            start: ChunkTime::from_millis(segment["offsets"]["from"].as_u64().ok_or("no from")?)
                .ok_or("time")?,
            end: ChunkTime::from_millis(segment["offsets"]["to"].as_u64().ok_or("no to")?)
                .ok_or("time")?,
            text: Some(CueText::new(text.clone(), text)?),
            tokens,
        });
    }
    Ok(ProviderChunkOutput {
        language: LanguageTag::parse("en").ok(),
        segments,
    })
}

fn run(source: &SourceSegment, covered: TimeRange, outcome: AsrChunkOutcome) -> Built<AsrRun> {
    let planned = plan_chunks(source.id(), covered, ChunkPlan::R0)?;
    Ok(AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(WHISPER_SHA256)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(BASE_MODEL_SHA256)?),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero")?,
        audio_stream: 1,
        chunks: planned
            .into_iter()
            .map(|chunk| AsrChunkRecord::new(chunk, outcome))
            .collect(),
    })?)
}

/// Revision 1 (the whole clip) and revision 2 (5.5-6 s, silent, spliced).
fn f01_revisions() -> Built<(TranscriptRevision, TranscriptRevision)> {
    let session = SessionId::parse(SESSION)?;
    let source_id = SourceId::parse(F01_SPEECH_SOURCE)?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(6 * SECOND))?;
    let audio = source.range();
    let whole = run(&source, audio, AsrChunkOutcome::Transcribed { audio })?;
    let first_chunk = whole.chunks().first().ok_or("no chunk")?.chunk().clone();
    let validated = validate_chunk_output(&first_chunk, audio, audio, recorded_f01_output()?)?;
    let (segments, language, warnings) = validated.into_parts();
    let merged = merge_chunks(&[segments]);
    let first = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription: AsrTranscription {
            run: whole,
            language,
            segments: merged.segments,
            warnings,
        },
        splice: None,
    })?;
    let tail = range(5_500_000, 6 * SECOND)?;
    let replaced = first.snap_to_segments(tail);
    let mut warnings = TranscriptWarnings::default();
    warnings.add(
        TranscriptWarningKind::SilentChunksSkipped,
        1,
        NonZeroU32::MIN,
    );
    let second = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::new(2).ok_or("zero")?,
        transcription: AsrTranscription {
            run: run(
                &source,
                replaced,
                AsrChunkOutcome::Silent { audio: replaced },
            )?,
            language: None,
            segments: Vec::new(),
            warnings,
        },
        splice: Some(RevisionSplice {
            base: &first,
            replaced_range: replaced,
        }),
    })?;
    Ok((first, second))
}

fn retranscribe_response(
    revision: &TranscriptRevision,
    requested: Option<TimeRange>,
) -> Built<OperationResponse<Value>> {
    Ok(OperationResponse::complete(
        "transcript.retranscribe",
        &TranscriptRetranscribeData::new(&SessionId::parse(SESSION)?, requested, revision),
    )?
    .with_lifecycle(LifecycleResponse::ephemeral(EXPIRES_AT.to_owned()))
    .with_warnings(&transcript_warning_messages(revision)))
}

fn page(revision: &TranscriptRevision) -> Built<(Vec<vsift_domain::TranscriptSegment>, TimeRange)> {
    let window = range(0, 6 * SECOND)?;
    let page = page_transcript(
        &SessionId::parse(SESSION)?,
        revision,
        &TranscriptPageRequest {
            range: window,
            limit: PageLimit::DEFAULT,
            cursor: None,
        },
        CURSOR_EXPIRES_US,
        NOW_US,
    )?;
    Ok((page.segments.into_iter().cloned().collect(), window))
}

fn page_response(revision: &TranscriptRevision) -> Built<OperationResponse<Value>> {
    let (segments, window) = page(revision)?;
    Ok(OperationResponse::complete(
        "transcript.get",
        &TranscriptPageData::new(
            &SessionId::parse(SESSION)?,
            revision,
            window,
            &segments,
            None,
        ),
    )?
    .with_lifecycle(LifecycleResponse::ephemeral(EXPIRES_AT.to_owned())))
}

#[test]
fn a_bounded_retranscription_matches_the_frozen_example() -> TestResult {
    let (first, second) = f01_revisions()?;
    let response = serde_json::to_value(retranscribe_response(
        &second,
        Some(range(5_500_000, 6 * SECOND)?),
    )?)?;
    validate("operation-response.schema.json", &response)?;
    validate(
        "transcript-retranscribe-data.schema.json",
        &response["data"],
    )?;
    let revision = &response["data"]["revision"];
    assert_eq!(revision["supersedes"], first.id().as_str());
    assert_eq!(revision["carried_segment_count"], 1);
    assert_eq!(response["data"]["recognised_segment_count"], 0);
    assert_eq!(revision["sidecar"], Value::Null);
    assert_eq!(response, load("examples/transcript-retranscribe.json")?);

    let event = serde_json::to_value(TerminalEventResponse::new(retranscribe_response(
        &second, None,
    )?))?;
    validate("terminal-event.schema.json", &event)?;
    validate(
        "transcript-retranscribe-data.schema.json",
        &event["result"]["data"],
    )?;
    Ok(())
}

#[test]
fn reading_a_spliced_revision_matches_the_frozen_example() -> TestResult {
    let (first, second) = f01_revisions()?;
    let response = serde_json::to_value(page_response(&second)?)?;
    validate("operation-response.schema.json", &response)?;
    validate("transcript-get-data.schema.json", &response["data"])?;
    let item = &response["data"]["items"][0];
    validate("transcript-segment.schema.json", item)?;
    assert_eq!(item["carried_from"]["revision_id"], first.id().as_str());
    assert_eq!(
        item["carried_from"]["segment_id"],
        first.segments()[0].id().as_str()
    );
    assert_ne!(item["segment_id"], first.segments()[0].id().as_str());
    assert_eq!(item["alignment"]["origin"], "local_asr");
    assert_eq!(item["cue"], Value::Null);
    assert_eq!(item["language"], "en");
    assert_eq!(response, load("examples/transcript-get.asr.json")?);

    // The older revision stays readable and its own record has no carried_from.
    let older = serde_json::to_value(page_response(&first)?)?;
    validate("transcript-get-data.schema.json", &older["data"])?;
    assert!(older["data"]["items"][0].get("carried_from").is_none());
    assert_eq!(older["data"]["revision"]["supersedes"], Value::Null);
    Ok(())
}

#[test]
fn a_spliced_revision_streams_schema_valid_evidence() -> TestResult {
    let (_, second) = f01_revisions()?;
    let (segments, window) = page(&second)?;
    let stream = TranscriptEvidenceStream::new(
        &SessionId::parse(SESSION)?,
        &second,
        window,
        &segments,
        None,
        LifecycleResponse::ephemeral(EXPIRES_AT.to_owned()),
    )?;
    for record in stream.records() {
        let value = serde_json::to_value(record)?;
        validate("evidence-event.schema.json", &value)?;
        assert_eq!(value["key"], value["record"]["segment_id"]);
    }
    let terminal = serde_json::to_value(stream.terminal())?;
    validate("terminal-event.schema.json", &terminal)?;
    validate(
        "transcript-get-stream-data.schema.json",
        &terminal["result"]["data"],
    )?;
    Ok(())
}

#[test]
fn local_asr_revisions_and_segments_satisfy_their_schemas() -> TestResult {
    let (first, second) = f01_revisions()?;
    let summary = serde_json::to_value(TranscriptRevisionData::new(&first))?;
    validate("transcript-revision.schema.json", &summary)?;
    assert_eq!(summary["local_asr"]["transcribed_chunks"], 1);
    assert_eq!(summary["replaced_range"], Value::Null);
    let (segments, window) = page(&first)?;
    let response = serde_json::to_value(OperationResponse::complete(
        "transcript.get",
        &TranscriptPageData::new(&SessionId::parse(SESSION)?, &first, window, &segments, None),
    )?)?;
    let item = &response["data"]["items"][0];
    validate("transcript-segment.schema.json", item)?;
    assert_eq!(item["alignment"]["chunk_index"], 0);
    assert_eq!(item["alignment"]["provider_end_us"], 5_260_000);
    assert_eq!(item["alignment"]["model_profile"], "base");
    assert_eq!(item["confidence"]["origin"], "provider_uncalibrated");
    assert_eq!(
        item["text"],
        "The service status is healthy and the build is 2048."
    );

    // Local-ASR members never validate on an import, and an import's shape
    // never validates as local ASR.
    let mut forged = serde_json::to_value(TranscriptRevisionData::new(&second))?;
    forged["alignment"] = serde_json::json!({"origin": "imported_srt", "offset_us": 0});
    assert!(validate("transcript-revision.schema.json", &forged).is_err());
    let mut cueless = item.clone();
    cueless["cue"] = serde_json::json!({"ordinal": 1, "line": 1});
    assert!(validate("transcript-segment.schema.json", &cueless).is_err());
    Ok(())
}

#[test]
fn every_local_asr_failure_is_a_schema_valid_bounded_failure() -> TestResult {
    let reasons = [
        (AsrFailureReason::InvalidRange, FailureCode::InvalidArgument),
        (AsrFailureReason::TooManyChunks, FailureCode::ResourceLimit),
        (
            AsrFailureReason::ModelChanged,
            FailureCode::MissingCapability,
        ),
        (
            AsrFailureReason::MalformedOutput(ProviderOutputError::InvalidTokenProbability),
            FailureCode::MissingCapability,
        ),
        (
            AsrFailureReason::AbnormalTermination,
            FailureCode::ResourceLimit,
        ),
        (AsrFailureReason::Deadline, FailureCode::DeadlineExceeded),
        (AsrFailureReason::Cancelled, FailureCode::Cancelled),
        (AsrFailureReason::Workspace, FailureCode::StorageIo),
        (
            AsrFailureReason::InvalidRun(TranscriptRevisionError::InvalidAsrRun),
            FailureCode::Internal,
        ),
    ];
    for (reason, code) in reasons {
        let failure = AsrFailure {
            stage: AsrStage::Recognition,
            reason,
        };
        for summary in [
            local_asr_failure_summary(failure),
            local_asr_verification_summary(LocalAsrVerificationFailure::Transcription(failure)),
        ] {
            let response = OperationResponse::failure_with_remediation(
                "transcript.retranscribe",
                code,
                summary,
            );
            let value = serde_json::to_value(&response)?;
            validate("operation-response.schema.json", &value)?;
            let event = serde_json::to_value(TerminalEventResponse::new(response))?;
            validate("terminal-event.schema.json", &event)?;
        }
    }
    for summary in [
        LOCAL_ASR_TOOLS_REMEDIATION,
        LOCAL_ASR_MODEL_REMEDIATION,
        UNPINNED_MODEL_REMEDIATION,
        NO_AUDIO_STREAM_REMEDIATION,
        NO_TRANSCRIPT_REMEDIATION,
        UNKNOWN_REVISION_REMEDIATION,
    ] {
        let value = serde_json::to_value(OperationResponse::failure_with_remediation(
            "transcript.retranscribe",
            FailureCode::MissingCapability,
            summary.to_owned(),
        ))?;
        validate("operation-response.schema.json", &value)?;
    }
    Ok(())
}

#[test]
fn published_local_asr_examples_validate_against_their_schemas() -> TestResult {
    let retranscribed = load("examples/transcript-retranscribe.json")?;
    validate("operation-response.schema.json", &retranscribed)?;
    validate(
        "transcript-retranscribe-data.schema.json",
        &retranscribed["data"],
    )?;
    let page = load("examples/transcript-get.asr.json")?;
    validate("operation-response.schema.json", &page)?;
    validate("transcript-get-data.schema.json", &page["data"])?;
    let record = load("examples/bundle-transcript-record.asr.json")?;
    validate("bundle-transcript-record.schema.json", &record)?;
    let imported = load("examples/bundle-transcript-record.json")?;
    validate("bundle-transcript-record.schema.json", &imported)?;
    for (schema, instance) in [
        (
            "transcript-retranscribe-data.schema.json",
            &retranscribed["data"],
        ),
        ("transcript-segment.schema.json", &page["data"]["items"][0]),
        (
            "transcript-revision.schema.json",
            &retranscribed["data"]["revision"],
        ),
        ("bundle-transcript-record.schema.json", &record),
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
