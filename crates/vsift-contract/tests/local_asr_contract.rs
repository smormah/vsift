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

use std::{fs, future::Future, io, num::NonZeroU16, num::NonZeroU32, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    AsrCancellation, AsrFailure, AsrFailureReason, AsrRevisionRequest, AsrStage, AsrTranscription,
    LocalAsrVerificationFailure, RecognizerIdentity, Resumability, RevisionSplice,
    SpeechAudioError, SpeechAudioSource, SpeechPcm, SpeechRecognitionError, SpeechRecognizer,
    TranscribeRangeRequest, TranscriptPageRequest, UnusableChunk, UnusableChunks,
    build_asr_revision, page_transcript, transcribe_range, whole_file_source_segment,
};
use vsift_contract::{
    CANCELLATION_TOO_LATE_WARNING, CHECKPOINT_DISCARDED_WARNING, CommandName,
    IDEMPOTENCY_CONFLICT_REMEDIATION, JOB_BUSY_REMEDIATION, JOB_CANCELLED_REMEDIATION,
    JOB_INTERRUPTED_REMEDIATION, JOB_NOT_RESUMABLE_REMEDIATION, JOB_SESSION_NOT_OPEN_REMEDIATION,
    JobData, JobPresentation, JobResumeData, LOCAL_ASR_MODEL_REMEDIATION,
    LOCAL_ASR_TOOLS_REMEDIATION, LifecycleResponse, NO_AUDIO_STREAM_REMEDIATION,
    NO_TRANSCRIPT_REMEDIATION, OperationResponse, PROVIDER_CHUNKS_REJECTED_WARNING,
    ProgressEventResponse, ProgressReport, RESUMED_FROM_CHECKPOINT_WARNING, RetranscribeJob,
    SUPERSEDED_REMEDIATION, SessionJobData, SessionState, SessionStatusData, StatusData,
    TerminalEventResponse, TranscriptEvidenceStream, TranscriptPageData,
    TranscriptRetranscribeData, TranscriptRevisionData, UNKNOWN_JOB_REMEDIATION,
    UNKNOWN_REVISION_REMEDIATION, UNPINNED_MODEL_REMEDIATION, finish_retranscription,
    job_warning_messages, local_asr_failure_summary, local_asr_verification_summary,
    retranscription_gaps, transcript_warning_messages,
};
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, AttemptFailure, ChunkPlan, ChunkTime, CueText,
    FailureCode, JobId, JobKind, JobState, LanguageTag, MediaTime, OperationId, PageLimit,
    PlannedChunk, ProgressStage, ProgressUpdate, ProviderChunkOutput, ProviderOutputError,
    ProviderSegment, ProviderToken, ProviderTokenKind, SessionId, Sha256Hex, SourceId,
    SourceSegment, StorageGeneration, TimeRange, TranscriptRevision, TranscriptRevisionError,
    TranscriptRevisionId, TranscriptWarningKind, TranscriptWarnings, merge_chunks, plan_chunks,
    validate_chunk_output,
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
const JOB: &str = "job_8d3e1f0a2b4c6d8e0f1a2b3c4d5e6f70";
const OPERATION: &str = "op_4f0c2b8e9a1d3c5e7f60718293a4b5c6";

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
    job_response(revision, requested, &fresh_job()?)
}

fn fresh_job() -> Built<RetranscribeJob> {
    Ok(RetranscribeJob {
        job_id: JobId::parse(JOB)?,
        resumed: false,
        chunks_reused: 0,
        replayed: false,
    })
}

fn job_response(
    revision: &TranscriptRevision,
    requested: Option<TimeRange>,
    job: &RetranscribeJob,
) -> Built<OperationResponse<Value>> {
    Ok(OperationResponse::complete(
        "transcript.retranscribe",
        &TranscriptRetranscribeData::new(&SessionId::parse(SESSION)?, requested, revision, job),
    )?
    .with_operation_id(&OperationId::parse(OPERATION)?)
    .with_lifecycle(LifecycleResponse::ephemeral(EXPIRES_AT.to_owned()))
    .with_warnings(&transcript_warning_messages(revision)))
}

/// P10 PR 2: a resumed result names its job and reused chunks and warns in
/// fixed prose; a replayed one says so; both validate, and the terminal
/// event repeats the operation id.
#[test]
fn resumed_and_replayed_retranscriptions_validate() -> TestResult {
    let (_, second) = f01_revisions()?;
    let resumed = RetranscribeJob {
        resumed: true,
        chunks_reused: 1,
        ..fresh_job()?
    };
    let response = job_response(&second, None, &resumed)?
        .with_warnings(&job_warning_messages(resumed.chunks_reused, 1));
    let value = serde_json::to_value(&response)?;
    validate("operation-response.schema.json", &value)?;
    validate("transcript-retranscribe-data.schema.json", &value["data"])?;
    assert_eq!(value["operation_id"], OPERATION);
    assert_eq!(value["data"]["job"]["job_id"], JOB);
    assert_eq!(value["data"]["job"]["resumed"], true);
    assert_eq!(value["data"]["job"]["chunks_reused"], 1);
    let warnings = value["warnings"].as_array().ok_or("no warnings")?;
    assert!(warnings.contains(&Value::from(RESUMED_FROM_CHECKPOINT_WARNING)));
    assert!(warnings.contains(&Value::from(CHECKPOINT_DISCARDED_WARNING)));
    assert_eq!(job_warning_messages(0, 0), Vec::<&str>::new());

    let replayed = RetranscribeJob {
        replayed: true,
        ..fresh_job()?
    };
    let event = serde_json::to_value(TerminalEventResponse::new(job_response(
        &second, None, &replayed,
    )?))?;
    validate("terminal-event.schema.json", &event)?;
    validate(
        "transcript-retranscribe-data.schema.json",
        &event["result"]["data"],
    )?;
    assert_eq!(event["operation_id"], OPERATION);
    assert_eq!(event["result"]["data"]["job"]["replayed"], true);

    let mut invalid = value["data"].clone();
    invalid["job"]["job_id"] = Value::from("not-a-job");
    assert!(validate("transcript-retranscribe-data.schema.json", &invalid).is_err());
    Ok(())
}

/// P10 PR 2 (D-4): the job failures a retranscription can end with validate
/// with their fixed-prose remediation, retry hint and affected job.
#[test]
fn job_failures_validate_with_their_hints() -> TestResult {
    let busy = OperationResponse::failure_with_remediation(
        "transcript.retranscribe",
        FailureCode::Busy,
        JOB_BUSY_REMEDIATION.to_owned(),
    )
    .with_retry_after(2_000)
    .with_affected_ids(&[JOB]);
    let conflict = OperationResponse::failure_with_remediation(
        "transcript.retranscribe",
        FailureCode::IdempotencyConflict,
        IDEMPOTENCY_CONFLICT_REMEDIATION.to_owned(),
    )
    .with_affected_ids(&[JOB]);
    let superseded = OperationResponse::failure_with_remediation(
        "transcript.retranscribe",
        FailureCode::Busy,
        SUPERSEDED_REMEDIATION.to_owned(),
    );
    for response in [busy, conflict, superseded] {
        let value = serde_json::to_value(TerminalEventResponse::new(response))?;
        validate("terminal-event.schema.json", &value)?;
        validate("operation-response.schema.json", &value["result"])?;
    }
    for prose in [
        CANCELLATION_TOO_LATE_WARNING,
        RESUMED_FROM_CHECKPOINT_WARNING,
        CHECKPOINT_DISCARDED_WARNING,
        IDEMPOTENCY_CONFLICT_REMEDIATION,
        JOB_BUSY_REMEDIATION,
        SUPERSEDED_REMEDIATION,
    ] {
        assert!(prose.len() <= 1_024 && !prose.contains('/') && !prose.contains('\\'));
    }
    Ok(())
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

/// P11: `transcript retranscribe --events jsonl` writes the job's chunk
/// progress (0 of 1 once planned, 1 of 1 after the chunk), then the terminal
/// event at the count of events before it, whose result is exactly the
/// `--json` result `transcript-retranscribe.json`.
#[test]
fn a_retranscription_stream_matches_the_frozen_example() -> TestResult {
    let (_, second) = f01_revisions()?;
    let job = JobId::parse(JOB)?;
    let command = CommandName::TranscriptRetranscribe;
    let mut lines = Vec::new();
    for completed in [0, 1] {
        let event = ProgressEventResponse::new(
            u64::try_from(lines.len())?,
            command,
            &ProgressReport {
                update: ProgressUpdate {
                    stage: ProgressStage::RecognisingSpeech,
                    completed,
                    total: Some(1),
                },
                job: Some(&job),
                request: None,
                dropped: 0,
            },
        );
        lines.push(serde_json::to_string(&event)?);
    }
    let terminal = TerminalEventResponse::at_sequence(
        retranscribe_response(&second, Some(range(5_500_000, 6 * SECOND)?))?,
        u64::try_from(lines.len())?,
    );
    lines.push(serde_json::to_string(&terminal)?);
    let mut produced = lines.join("\n");
    produced.push('\n');
    assert_eq!(
        produced,
        fs::read_to_string(repository(
            "schemas/v1/examples/transcript-retranscribe.events.jsonl"
        ))?
    );

    for (index, line) in produced.lines().enumerate() {
        let event: Value = serde_json::from_str(line)?;
        assert_eq!(event["sequence"], u64::try_from(index)?);
        let kind = event["event"].as_str().ok_or("event")?;
        validate(&format!("{kind}-event.schema.json"), &event)?;
        if kind == "terminal" {
            assert_eq!(
                event["result"],
                load("examples/transcript-retranscribe.json")?
            );
        }
    }
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
            AsrFailureReason::MalformedOutput(UnusableChunks {
                unusable: 4,
                answered: 5,
                planned: 8,
                first: UnusableChunk {
                    index: 0,
                    window: range(0, 30 * SECOND)?,
                    error: ProviderOutputError::InvalidTokenProbability,
                },
            }),
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

// ------------------------------------------------ #353: unusable chunks

/// The audio of a recording that is silent for its first `quiet` chunks and
/// speech after them, as a decoder would hand it over.
struct Recording {
    quiet: u32,
}

impl SpeechAudioSource for Recording {
    fn speech_pcm(
        &self,
        chunk: &PlannedChunk,
    ) -> impl Future<Output = Result<SpeechPcm, SpeechAudioError>> + Send {
        let level = if chunk.index() < self.quiet { 0 } else { 3_000 };
        std::future::ready(
            usize::try_from(chunk.window().duration_micros() / 1_000 * 16)
                .map_err(|_| SpeechAudioError::ResourceLimit)
                .map(|count| SpeechPcm {
                    actual_start: chunk.window().start(),
                    samples: vec![level; count],
                }),
        )
    }
}

/// A recogniser that answers the first chunk with `first` (the recorded F01
/// output) or, when it has none, with a segment that starts after the chunk's
/// audio ends, which no rule can place: what whisper.cpp did for the recording
/// of #353. Every other chunk it answers with one segment six seconds in when
/// `rest_described` and with the unplaceable one otherwise.
struct StandInRecogniser {
    first: Option<ProviderChunkOutput>,
    rest_described: bool,
}

impl SpeechRecognizer for StandInRecogniser {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send {
        std::future::ready(recognizer_identity().map_err(|_| SpeechRecognitionError::Io))
    }

    fn recognize(
        &self,
        chunk: &PlannedChunk,
        _pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        let answer = match (&self.first, chunk.index()) {
            (Some(output), 0) => Ok(output.clone()),
            (None, 0) => answer_after_the_audio(chunk),
            _ if self.rest_described => answer_six_seconds_in(chunk),
            _ => answer_after_the_audio(chunk),
        };
        std::future::ready(answer)
    }
}

/// One segment of two seconds, six seconds into the chunk.
fn answer_six_seconds_in(
    chunk: &PlannedChunk,
) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    let words = format!("words in chunk {}", chunk.index());
    Ok(ProviderChunkOutput {
        language: LanguageTag::parse("en").ok(),
        segments: vec![ProviderSegment {
            start: ChunkTime::from_millis(6_000).ok_or(SpeechRecognitionError::Io)?,
            end: ChunkTime::from_millis(8_000).ok_or(SpeechRecognitionError::Io)?,
            text: Some(CueText::new(words.clone(), words).map_err(|_| SpeechRecognitionError::Io)?),
            tokens: Vec::new(),
        }],
    })
}

fn answer_after_the_audio(
    chunk: &PlannedChunk,
) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    let words = "words that cannot be placed".to_owned();
    let past = chunk.window().duration_micros() / 1_000 + 1_000;
    Ok(ProviderChunkOutput {
        language: LanguageTag::parse("en").ok(),
        segments: vec![ProviderSegment {
            start: ChunkTime::from_millis(past).ok_or(SpeechRecognitionError::Io)?,
            end: ChunkTime::from_millis(past + 2_000).ok_or(SpeechRecognitionError::Io)?,
            text: Some(CueText::new(words.clone(), words).map_err(|_| SpeechRecognitionError::Io)?),
            tokens: Vec::new(),
        }],
    })
}

fn recognizer_identity() -> Built<RecognizerIdentity> {
    Ok(RecognizerIdentity {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(WHISPER_SHA256)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(BASE_MODEL_SHA256)?),
        decoding: AsrDecodingProfile::R0V1,
        threads: NonZeroU16::new(4).ok_or("zero")?,
    })
}

struct NotCancelled;

impl AsrCancellation for NotCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// A run over the whole of a `seconds`-long stand-in recording made by the
/// application's own use case, with the fakes above: what its numbers are is
/// what the code gives, not what a document says.
async fn transcribe_stand_in(
    seconds: u64,
    quiet: u32,
    recogniser: &StandInRecogniser,
) -> Built<(SourceSegment, Result<AsrTranscription, AsrFailure>)> {
    let source_id = SourceId::from_sha256(&"0123456789abcdef".repeat(4))?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(seconds * SECOND))?;
    let outcome = transcribe_range(
        TranscribeRangeRequest {
            source_segment: &source,
            range: source.range(),
            plan: ChunkPlan::R0,
            audio_stream: 1,
            expected: &recognizer_identity()?,
        },
        &Recording { quiet },
        recogniser,
        &NotCancelled,
    )
    .await;
    Ok((source, outcome))
}

/// The revision the run `transcription` over the whole of `source` makes as
/// revision `number`, superseding `base` when there is one (the whole source
/// is replaced, but for text kept inside a gap).
fn stand_in_revision(
    source: &SourceSegment,
    number: u32,
    transcription: AsrTranscription,
    base: Option<&TranscriptRevision>,
) -> Built<TranscriptRevision> {
    Ok(build_asr_revision(AsrRevisionRequest {
        session_id: &SessionId::parse(SESSION)?,
        source_id: &SourceId::from_sha256(&"0123456789abcdef".repeat(4))?,
        source_segment: source,
        number: NonZeroU32::new(number).ok_or("zero")?,
        transcription,
        splice: base.map(|base| RevisionSplice {
            base,
            replaced_range: source.range(),
        }),
    })?)
}

/// A 55 s recording of two chunks, the first transcribed from the recorded F01
/// output and the second unusable: what a recogniser's answer that cannot be
/// placed in its audio leaves behind, made by `transcribe_range`. (The
/// recording is a stand-in: the F01 clip's own output over a longer source, so
/// the example needs no real one.)
async fn unusable_chunk_revision() -> Built<TranscriptRevision> {
    let recogniser = StandInRecogniser {
        first: Some(recorded_f01_output()?),
        rest_described: false,
    };
    let (source, outcome) = transcribe_stand_in(55, 0, &recogniser).await?;
    let transcription = outcome.map_err(|failure| format!("the run failed: {failure:?}"))?;
    stand_in_revision(&source, 1, transcription, None)
}

/// What a run over the 2,080 s stand-in recording (83 chunks) gives when its
/// first three chunks are silent and the recogniser's answer for every other
/// chunk cannot be placed in its audio: the run stops as soon as it is certain
/// that more than half of the chunks it answered are unusable.
async fn most_chunks_unusable_failure() -> Built<AsrFailure> {
    let recogniser = StandInRecogniser {
        first: None,
        rest_described: false,
    };
    let (_, outcome) = transcribe_stand_in(2_080, 3, &recogniser).await?;
    Ok(outcome.err().ok_or("the run was expected to fail")?)
}

/// The same failure of a run over a short range: 55 s, two chunks, neither of
/// whose answers can be placed.
async fn short_range_failure() -> Built<AsrFailure> {
    let recogniser = StandInRecogniser {
        first: None,
        rest_described: false,
    };
    let (_, outcome) = transcribe_stand_in(55, 0, &recogniser).await?;
    Ok(outcome.err().ok_or("the run was expected to fail")?)
}

/// #353: a run in which the recogniser's answer for one chunk could not be used
/// commits its revision and answers `partial`: the envelope's coverage lists
/// the chunk's untranscribed range (what its neighbour's overlap did not
/// transcribe) under the published reason `untranscribed_range`, the data lists
/// the same range and counts the chunk, and the warning names it. It is a
/// success: the status says what is missing, and it validates against the
/// published schemas.
#[tokio::test]
async fn a_run_with_an_unusable_chunk_is_a_partial_result_that_names_its_gap() -> TestResult {
    let revision = unusable_chunk_revision().await?;
    let response = finish_retranscription(retranscribe_response(&revision, None)?, &revision);
    let value = serde_json::to_value(&response)?;
    validate("operation-response.schema.json", &value)?;
    validate("transcript-retranscribe-data.schema.json", &value["data"])?;
    assert_eq!(value["status"], "partial");
    assert!(value["error"].is_null());
    // The chunk's own 25-55 s window less the first 5 s the first chunk heard
    // (it ends at 30 s).
    assert_eq!(
        value["coverage"],
        serde_json::json!({
            "truncated": true,
            "gaps": ["30000000-55000000"],
            "reasons": ["untranscribed_range"],
        })
    );
    assert_eq!(
        value["data"]["untranscribed_ranges"],
        serde_json::json!([{"from_us": 30_000_000, "to_us": 55_000_000}])
    );
    let run = &value["data"]["revision"]["local_asr"];
    assert_eq!(run["chunk_count"], 2);
    assert_eq!(run["transcribed_chunks"], 1);
    assert_eq!(run["unusable_chunks"], 1);
    assert_eq!(run["silent_chunks"], 0);
    assert_eq!(
        value["data"]["revision"]["warnings"],
        serde_json::json!([{
            "code": "provider_chunks_rejected",
            "count": 1,
            "first_cue": 2,
            "excluded_cues": true,
        }])
    );
    assert!(
        value["warnings"]
            .as_array()
            .ok_or("no warnings")?
            .contains(&Value::from(PROVIDER_CHUNKS_REJECTED_WARNING))
    );
    assert_eq!(
        retranscription_gaps(&revision),
        [range(30 * SECOND, 55 * SECOND)?]
    );
    assert_example("transcript-retranscribe.partial.json", &value)?;

    // The stream form carries the same result as its terminal event.
    let event = serde_json::to_value(TerminalEventResponse::new(finish_retranscription(
        retranscribe_response(&revision, None)?,
        &revision,
    )))?;
    validate("terminal-event.schema.json", &event)?;
    assert_eq!(event["result"]["status"], "partial");

    // A search of the revision reports the same range as untranscribed, with
    // the same reason, and the recording's end as nothing was examined there.
    let coverage = vsift_domain::SearchCoverage::of(&revision, None);
    assert_eq!(coverage.untranscribed(), [range(30 * SECOND, 55 * SECOND)?]);
    assert!(coverage.no_speech().is_empty());
    Ok(())
}

/// #353: a run that could not read part of the range it replaced keeps what the
/// earlier revision had there, and says so. A 55 s stand-in recording is
/// transcribed in full (revision 1: the recorded F01 speech in the first
/// chunk's first seconds, one segment in the second chunk), then again with the
/// first chunk unusable (revision 2). The result lists the whole of the part
/// that run did not transcribe, 0-25 s, because that is what the run did; the
/// words revision 1 had in it are kept as they were, carried from revision 1,
/// so a search of revision 2 still finds them and reports only what lies
/// around them as untranscribed. The two lists differ on purpose, and the
/// warning says why.
#[tokio::test]
async fn text_kept_in_an_unreadable_part_is_listed_as_not_transcribed_by_the_run() -> TestResult {
    let source_id = SourceId::from_sha256(&"0123456789abcdef".repeat(4))?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(55 * SECOND))?;
    let described = StandInRecogniser {
        first: Some(recorded_f01_output()?),
        rest_described: true,
    };
    let (_, whole) = transcribe_stand_in(55, 0, &described).await?;
    let first = stand_in_revision(&source, 1, whole.map_err(|f| format!("{f:?}"))?, None)?;
    let unreadable_start = StandInRecogniser {
        first: None,
        rest_described: true,
    };
    let (_, again) = transcribe_stand_in(55, 0, &unreadable_start).await?;
    let second = stand_in_revision(
        &source,
        2,
        again.map_err(|f| format!("{f:?}"))?,
        Some(&first),
    )?;

    // The first chunk's window (0-30 s) less the second's (25-55 s).
    let not_transcribed = [range(0, 25 * SECOND)?];
    assert_eq!(retranscription_gaps(&second), not_transcribed);
    let kept: Vec<TimeRange> = second
        .segments()
        .iter()
        .filter(|segment| segment.carried_from().is_some())
        .map(vsift_domain::TranscriptSegment::range)
        .collect();
    assert!(!kept.is_empty(), "revision 1's speech was not kept");
    assert!(
        kept.iter()
            .all(|range| range.end().as_micros() <= 25 * SECOND)
    );
    assert!(
        second
            .segments()
            .iter()
            .any(|segment| segment.carried_from().is_none()),
        "the second chunk's own words are missing"
    );

    let response = finish_retranscription(retranscribe_response(&second, None)?, &second);
    let value = serde_json::to_value(&response)?;
    validate("operation-response.schema.json", &value)?;
    validate("transcript-retranscribe-data.schema.json", &value["data"])?;
    assert_eq!(value["status"], "partial");
    assert_eq!(value["coverage"]["gaps"], serde_json::json!(["0-25000000"]));
    assert_eq!(
        value["data"]["untranscribed_ranges"],
        serde_json::json!([{"from_us": 0, "to_us": 25_000_000}])
    );
    assert_eq!(
        value["data"]["revision"]["carried_segment_count"],
        kept.len()
    );
    let warnings = value["warnings"].as_array().ok_or("no warnings")?;
    assert!(warnings.contains(&Value::from(PROVIDER_CHUNKS_REJECTED_WARNING)));
    assert!(
        PROVIDER_CHUNKS_REJECTED_WARNING
            .contains("kept whole as it was, unless this run's own text overlaps it")
    );

    // A search of the revision counts the kept words as transcribed: it lists
    // less than the run did not transcribe, and nothing under the kept words.
    let coverage = vsift_domain::SearchCoverage::of(&second, None);
    assert!(!coverage.untranscribed().is_empty());
    assert_ne!(coverage.untranscribed(), not_transcribed);
    for gap in coverage.untranscribed() {
        assert!(gap.end().as_micros() <= 25 * SECOND, "{gap:?}");
        for kept in &kept {
            assert!(
                gap.end() <= kept.start() || kept.end() <= gap.start(),
                "{gap:?} overlaps the kept words at {kept:?}"
            );
        }
    }
    Ok(())
}

/// A run with no unusable chunk is presented exactly as it always was: the
/// complete result, `coverage` null, and none of the members #353 added.
#[test]
fn a_run_without_an_unusable_chunk_is_presented_exactly_as_before() -> TestResult {
    let (first, second) = f01_revisions()?;
    for (revision, requested) in [
        (&first, None),
        (&second, Some(range(5_500_000, 6 * SECOND)?)),
    ] {
        let response =
            finish_retranscription(retranscribe_response(revision, requested)?, revision);
        let value = serde_json::to_value(&response)?;
        assert_eq!(value["status"], "complete");
        assert!(value["coverage"].is_null());
        assert!(value["data"].get("untranscribed_ranges").is_none());
        assert!(
            value["data"]["revision"]["local_asr"]
                .get("unusable_chunks")
                .is_none()
        );
        assert!(retranscription_gaps(revision).is_empty());
    }
    // And the frozen example, byte for byte, is still what is produced.
    let (_, second) = f01_revisions()?;
    let response = serde_json::to_value(finish_retranscription(
        retranscribe_response(&second, Some(range(5_500_000, 6 * SECOND)?))?,
        &second,
    ))?;
    assert_eq!(response, load("examples/transcript-retranscribe.json")?);
    Ok(())
}

/// The failure response of `failure`, as `transcript retranscribe` answers it
/// for the job `JOB`.
fn unusable_output_response(failure: AsrFailure) -> Built<Value> {
    let response = OperationResponse::failure_with_remediation(
        "transcript.retranscribe",
        FailureCode::MissingCapability,
        local_asr_failure_summary(failure),
    )
    .with_affected_ids(&[JOB]);
    let value = serde_json::to_value(&response)?;
    validate("operation-response.schema.json", &value)?;
    validate(
        "terminal-event.schema.json",
        &serde_json::to_value(TerminalEventResponse::new(response))?,
    )?;
    Ok(value)
}

/// #353: the failure of a run whose recogniser answered unusably for most
/// chunks keeps its published code, names the job in `affected_ids`, and says in
/// its remediation which reason, how many chunks and where the first one is;
/// the job it names is failed, not resumable. The numbers are those of the
/// application's own run over an 83-chunk stand-in recording whose first three
/// chunks are silent: it stops at the 41st answer, the first moment it is
/// certain that more than half of the answers are unusable.
#[tokio::test]
async fn most_chunks_unusable_is_a_truthful_failure_with_its_job() -> TestResult {
    let failure = most_chunks_unusable_failure().await?;
    assert_eq!(
        failure,
        AsrFailure {
            stage: AsrStage::OutputValidation,
            reason: AsrFailureReason::MalformedOutput(UnusableChunks {
                unusable: 41,
                answered: 41,
                planned: 83,
                first: UnusableChunk {
                    index: 3,
                    window: range(75 * SECOND, 105 * SECOND)?,
                    error: ProviderOutputError::TooManyRejectedSegments,
                },
            }),
        }
    );
    let value = unusable_output_response(failure)?;
    // The published code and its safe message are unchanged.
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], "MISSING_CAPABILITY");
    assert_eq!(value["error"]["affected_ids"], serde_json::json!([JOB]));
    let summary = value["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("no remediation")?;
    for named in [
        "(malformed_output)",
        "too_many_rejected_segments",
        "41 of the 41 audio chunks",
        "chunk 4 of 83",
        "0:01:15 to 0:01:45",
        "75000000 to 105000000 microseconds",
        "could not be transcribed reliably",
        "ingest --transcript <file>",
    ] {
        assert!(summary.contains(named), "{named}: {summary}");
    }
    assert!(!summary.contains("different range"), "{summary}");
    assert_example("retranscribe-unusable-output.json", &value)?;

    // The job is failed, so `job status` says it cannot be resumed, with the
    // chunk and code of the failure.
    let data = f01_job(
        JobState::Failed,
        Resumability::Failed,
        None,
        Some(AttemptFailure {
            chunk: Some(3),
            code: FailureCode::MissingCapability,
        }),
        0,
    )?;
    let status = serde_json::to_value(OperationResponse::complete("job.status", &data)?)?;
    validate("operation-response.schema.json", &status)?;
    validate("job-data.schema.json", &status["data"])?;
    assert_eq!(status["data"]["state"], "failed");
    assert_eq!(status["data"]["resumable"], false);
    assert_eq!(status["data"]["resumable_reason"], "failed");
    assert_eq!(status["data"]["failure"]["code"], "MISSING_CAPABILITY");
    Ok(())
}

/// #353: when only a few chunks were answered, the same failure does not claim
/// the whole recording could not be transcribed: it says the stretch could not
/// be read and that a slightly different range, which cuts the audio at other
/// points, may work, beside the transcript the user can supply. The numbers are
/// those of the application's own run over a 55 s stand-in recording.
#[tokio::test]
async fn few_chunks_unusable_says_the_stretch_could_not_be_read() -> TestResult {
    let failure = short_range_failure().await?;
    assert_eq!(
        failure,
        AsrFailure {
            stage: AsrStage::OutputValidation,
            reason: AsrFailureReason::MalformedOutput(UnusableChunks {
                unusable: 2,
                answered: 2,
                planned: 2,
                first: UnusableChunk {
                    index: 0,
                    window: range(0, 30 * SECOND)?,
                    error: ProviderOutputError::TooManyRejectedSegments,
                },
            }),
        }
    );
    let value = unusable_output_response(failure)?;
    assert_eq!(value["error"]["code"], "MISSING_CAPABILITY");
    let summary = value["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("no remediation")?;
    for named in [
        "2 of the 2 audio chunks",
        "chunk 1 of 2",
        "This stretch of the recording could not be transcribed.",
        "Try a slightly different range with --from and --to",
        "ingest --transcript <file>",
    ] {
        assert!(summary.contains(named), "{named}: {summary}");
    }
    assert!(!summary.contains("reliably"), "{summary}");
    assert!(!summary.contains("will not change"), "{summary}");
    assert_example("retranscribe-unusable-output.short-range.json", &value)?;
    Ok(())
}

/// Every warning kind has exactly one published code in each schema that lists
/// them, in the domain's order, so a new kind cannot be written to a record
/// that its schema does not know.
#[test]
fn every_warning_kind_is_published_in_each_schema_that_lists_them() -> TestResult {
    let identifiers: Vec<&str> = TranscriptWarningKind::ALL
        .into_iter()
        .map(TranscriptWarningKind::identifier)
        .collect();
    assert_eq!(identifiers.len(), 13);
    let revision = load("transcript-revision.schema.json")?;
    let listed: Vec<&str> =
        revision["properties"]["warnings"]["items"]["properties"]["code"]["enum"]
            .as_array()
            .ok_or("no code enum")?
            .iter()
            .filter_map(Value::as_str)
            .collect();
    assert_eq!(listed, identifiers);
    let record = load("bundle-transcript-record.schema.json")?;
    let stored: Vec<&str> = record["$defs"]["v2"]["properties"]["warnings"]["items"]["properties"]
        ["kind"]["enum"]
        .as_array()
        .ok_or("no kind enum")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(stored, identifiers);
    assert_eq!(
        record["$defs"]["v2"]["properties"]["warnings"]["maxItems"],
        u64::try_from(identifiers.len())?
    );
    // The most warnings a revision can carry is one per kind.
    assert_eq!(
        revision["properties"]["warnings"]["maxItems"],
        u64::try_from(identifiers.len())?
    );
    Ok(())
}

// ------------------------------------------------ P10 PR 3: the job surface

/// Compares `value` with the frozen example `name`: the parsed JSON values, not
/// the bytes. Member order and white space of an example are not part of the
/// contract (a consumer reads JSON), and the examples are formatted for a person
/// to read, so a change of either is not a change of what the example says; a
/// change of any value, member or sentence is.
fn assert_example(name: &str, value: &Value) -> TestResult {
    assert_eq!(*value, load(&format!("examples/{name}"))?, "{name}");
    Ok(())
}

/// The F01 retranscription job of the frozen examples, as `job status`
/// presents it in `state` with `resumability`.
fn f01_job(
    state: JobState,
    resumability: Resumability,
    result: Option<(&TranscriptRevisionId, StorageGeneration)>,
    failure: Option<AttemptFailure>,
    checkpoints: usize,
) -> Built<JobData> {
    let job = JobId::parse(JOB)?;
    let session = SessionId::parse(SESSION)?;
    let operation = OperationId::parse(OPERATION)?;
    Ok(JobData::new(&JobPresentation {
        job_id: &job,
        session_id: &session,
        kind: JobKind::Retranscribe,
        state,
        live_owner: resumability == Resumability::LiveOwner,
        resumability,
        operation_id: Some(&operation),
        requested: Some(range(5_500_000, 6 * SECOND)?),
        planned_chunks: Some(1),
        checkpoints,
        attempts: 1,
        result,
        failure,
    }))
}

/// `job status` of a job whose run a Ctrl-C stopped: interrupted,
/// resumable, with the failure that ended its attempt.
#[test]
fn an_interrupted_job_status_matches_the_frozen_example() -> TestResult {
    let data = f01_job(
        JobState::Interrupted,
        Resumability::Interrupted,
        None,
        Some(AttemptFailure {
            chunk: Some(0),
            code: FailureCode::Cancelled,
        }),
        0,
    )?;
    let response = serde_json::to_value(OperationResponse::complete("job.status", &data)?)?;
    validate("operation-response.schema.json", &response)?;
    validate("job-data.schema.json", &response["data"])?;
    assert_eq!(response["data"]["resumable"], true);
    assert_eq!(response["data"]["failure"]["code"], "CANCELLED");
    assert_example("job-status.json", &response)
}

/// `job cancel` of a job that had committed: its result stands, with the
/// warning `cancellation_too_late`.
#[test]
fn a_late_cancel_matches_the_frozen_example() -> TestResult {
    let (_, second) = f01_revisions()?;
    let data = f01_job(
        JobState::Succeeded,
        Resumability::Succeeded,
        Some((second.id(), StorageGeneration::from_value(3))),
        None,
        0,
    )?;
    let response = serde_json::to_value(
        OperationResponse::complete("job.cancel", &data)?
            .with_warnings(&[CANCELLATION_TOO_LATE_WARNING]),
    )?;
    validate("operation-response.schema.json", &response)?;
    validate("job-data.schema.json", &response["data"])?;
    assert_eq!(
        response["data"]["result"]["revision_id"],
        second.id().as_str()
    );
    assert_example("job-cancel.json", &response)
}

/// `job resume` of the interrupted job: the job afterwards and the
/// retranscription it committed from its checkpoint.
#[test]
fn a_resumed_job_matches_the_frozen_example() -> TestResult {
    let (_, second) = f01_revisions()?;
    let job = f01_job(
        JobState::Succeeded,
        Resumability::Succeeded,
        Some((second.id(), StorageGeneration::from_value(3))),
        None,
        0,
    )?;
    let resumed = RetranscribeJob {
        resumed: true,
        chunks_reused: 1,
        ..fresh_job()?
    };
    let outcome = TranscriptRetranscribeData::new(
        &SessionId::parse(SESSION)?,
        Some(range(5_500_000, 6 * SECOND)?),
        &second,
        &resumed,
    );
    let mut warnings = transcript_warning_messages(&second);
    warnings.extend(job_warning_messages(1, 0));
    let response = serde_json::to_value(
        OperationResponse::complete("job.resume", &JobResumeData::new(job, outcome))?
            .with_operation_id(&OperationId::parse(OPERATION)?)
            .with_lifecycle(LifecycleResponse::ephemeral(EXPIRES_AT.to_owned()))
            .with_warnings(&warnings),
    )?;
    validate("operation-response.schema.json", &response)?;
    validate("job-resume-data.schema.json", &response["data"])?;
    validate("job-data.schema.json", &response["data"]["job"])?;
    validate(
        "transcript-retranscribe-data.schema.json",
        &response["data"]["outcome"],
    )?;
    assert_eq!(response["data"]["outcome"]["job"]["chunks_reused"], 1);
    assert_example("job-resume.json", &response)
}

/// A retranscription stopped by Ctrl-C: `CANCELLED`, the session and job in
/// `affected_ids`, and a remediation that suggests `job resume <job>` as an
/// executable and argument array.
#[test]
fn an_interrupted_retranscription_matches_the_frozen_example() -> TestResult {
    let response = OperationResponse::failure_with_suggested_command(
        "transcript.retranscribe",
        FailureCode::Cancelled,
        JOB_INTERRUPTED_REMEDIATION.to_owned(),
        &["job", "resume", JOB],
    )
    .with_affected_ids(&[SESSION, JOB]);
    let value = serde_json::to_value(&response)?;
    validate("operation-response.schema.json", &value)?;
    assert_eq!(value["status"], "cancelled");
    assert_eq!(
        value["error"]["affected_ids"],
        serde_json::json!([SESSION, JOB])
    );
    let command = &value["error"]["remediation"][0]["command"];
    assert_eq!(command["executable"], "vsift");
    assert_eq!(
        command["arguments"],
        serde_json::json!(["job", "resume", JOB])
    );
    let event = serde_json::to_value(TerminalEventResponse::new(response))?;
    validate("terminal-event.schema.json", &event)?;
    assert_example("retranscribe-cancelled.json", &value)
}

/// Every state, resumability and failure code a job can report gives
/// schema-valid job data, and malformed members are rejected.
#[test]
fn every_job_state_and_reason_is_schema_valid() -> TestResult {
    let (_, second) = f01_revisions()?;
    for state in JobState::ALL {
        for resumability in Resumability::ALL {
            let result = (state == JobState::Succeeded)
                .then(|| (second.id(), StorageGeneration::from_value(2)));
            let data = serde_json::to_value(f01_job(state, resumability, result, None, 1)?)?;
            validate("job-data.schema.json", &data)?;
            assert_eq!(data["resumable"], resumability.resumable());
        }
    }
    for code in FailureCode::ALL {
        let data = serde_json::to_value(f01_job(
            JobState::Failed,
            Resumability::Failed,
            None,
            Some(AttemptFailure { chunk: None, code }),
            0,
        )?)?;
        validate("job-data.schema.json", &data)?;
        assert_eq!(data["failure"]["retryable"], code.retryable());
    }
    let valid = serde_json::to_value(f01_job(
        JobState::Queued,
        Resumability::NotStarted,
        None,
        None,
        0,
    )?)?;
    for (member, bad) in [
        ("job_id", Value::from("job-1")),
        ("state", Value::from("paused")),
        ("resumable_reason", Value::from("maybe")),
        ("operation_id", Value::from("op-1")),
        ("path", Value::from("C:/private")),
    ] {
        let mut invalid = valid.clone();
        invalid[member] = bad;
        assert!(
            validate("job-data.schema.json", &invalid).is_err(),
            "{member}"
        );
    }
    for prose in [
        JOB_INTERRUPTED_REMEDIATION,
        JOB_SESSION_NOT_OPEN_REMEDIATION,
        UNKNOWN_JOB_REMEDIATION,
        JOB_NOT_RESUMABLE_REMEDIATION,
        JOB_CANCELLED_REMEDIATION,
    ] {
        assert!(prose.len() <= 1_024 && !prose.contains('/') && !prose.contains('\\'));
    }
    // L-070 regression: a renewal only extends an open session, so the
    // remediation for a closed or expired one must not send the caller to
    // `session renew`, which would be refused.
    assert!(!JOB_SESSION_NOT_OPEN_REMEDIATION.contains("session renew"));
    assert!(JOB_SESSION_NOT_OPEN_REMEDIATION.contains("ingest"));
    Ok(())
}

/// `session status` adds its newest jobs and whether it holds more; every
/// earlier member is unchanged.
#[test]
fn session_status_lists_jobs_additively() -> TestResult {
    let status = StatusData {
        session_id: SESSION.to_owned(),
        state: SessionState::Open,
        source_id: F01_SPEECH_SOURCE.to_owned(),
        source_bytes: 1,
        artifact_count: 2,
        artifact_bytes: 3,
        generation: 3,
        expires_at: EXPIRES_AT.to_owned(),
    };
    let plain = serde_json::to_value(&status)?;
    let job = SessionJobData::new(
        &JobId::parse(JOB)?,
        JobKind::Retranscribe,
        JobState::Interrupted,
        false,
        Resumability::Interrupted,
    );
    let listed = serde_json::to_value(SessionStatusData::new(status, vec![job], false))?;
    for (member, value) in plain.as_object().ok_or("not an object")? {
        assert_eq!(&listed[member], value, "{member}");
    }
    assert_eq!(listed["jobs_truncated"], false);
    assert_eq!(listed["jobs"][0]["job_id"], JOB);
    assert_eq!(listed["jobs"][0]["resumable_reason"], "interrupted");
    let job_data = load("job-data.schema.json")?;
    for member in [
        "job_id",
        "kind",
        "state",
        "live_owner",
        "resumable",
        "resumable_reason",
    ] {
        assert!(job_data["properties"].get(member).is_some(), "{member}");
    }
    Ok(())
}
