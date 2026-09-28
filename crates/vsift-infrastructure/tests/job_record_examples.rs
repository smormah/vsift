//! The committed example job records and chunk checkpoints
//! (`tests/data/jobs/`) are exactly what the store writes for known values,
//! and read back to them (P10's private job storage, ADR 0020 section 4).
//!
//! They exist so the `job_record` and `chunk_checkpoint` fuzz targets (issue
//! #180, P11 PR 1) can seed from committed repository files, as the harness's
//! seed-provenance rules require, and they pin the stored format: a change to
//! the encoder fails here until the examples are regenerated and reviewed.
//!
//! Regenerate with `VSIFT_REGENERATE_JOB_EXAMPLES=1` and review the diff.

use std::{env, error::Error, fs, path::PathBuf};

use vsift_application::{JobCommit, JobRecord, JobRequest, job_id, retranscribe_request_digest};
use vsift_domain::{
    AttemptFailure, CheckpointOutcome, ChunkCheckpoint, ChunkTime, CueText, FailureCode, JobState,
    LanguageTag, MediaTime, OperationId, OperationKey, PlannedChunk, ProviderChunkOutput,
    ProviderSegment, ProviderToken, ProviderTokenKind, RecognitionKey, SessionId, Sha256Hex,
    SourceSegmentId, StorageGeneration, TimeRange, TranscriptRevisionId,
};
use vsift_infrastructure::{
    decode_chunk_checkpoint, decode_job_record, encode_chunk_checkpoint, encode_job_record,
};

type Built<T> = Result<T, Box<dyn Error>>;

/// The session every example job belongs to.
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const OPERATION_KEY: &str =
    "opk_sha256_5d1e2c3b4a5968778695a4b3c2d1e0f1a2b3c4d5e6f708192a3b4c5d6e7f8091";
const RECOGNITION_KEY: &str = "3a5c7e9b1d2f4a6c8e0b2d4f6a8c0e1b3d5f7a9c1e3b5d7f9a1c3e5b7d9f1a3c";
const CREATED: u64 = 1_790_208_000;

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/jobs")
}

fn range(from: u64, to: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from),
        MediaTime::from_micros(to),
    )?)
}

fn base_record(state: JobState) -> Built<JobRecord> {
    let session = SessionId::parse(SESSION)?;
    let operation_key = OperationKey::parse(OPERATION_KEY)?;
    let requested = Some(range(5_500_000, 6_000_000)?);
    Ok(JobRecord {
        job_id: job_id(&session, &operation_key)?,
        request_digest: retranscribe_request_digest(&session, requested)?,
        session_id: session,
        operation_key,
        recognition_key: RecognitionKey::new(Sha256Hex::parse(RECOGNITION_KEY)?),
        request: JobRequest::Retranscribe { range: requested },
        planned_chunks: Some(1),
        operation_ids: vec![OperationId::parse("op_4f0c2b8e9a1d3c5e7f60718293a4b5c6")?],
        state,
        epoch: 0,
        attempt: 1,
        failures: Vec::new(),
        commit: None,
        created_at_unix_seconds: CREATED,
        updated_at_unix_seconds: CREATED + 12,
    })
}

/// A succeeded job with its commit.
fn succeeded() -> Built<JobRecord> {
    let mut record = base_record(JobState::Succeeded)?;
    record.commit = Some(JobCommit {
        operation_id: OperationId::parse("op_c0ffee00c0ffee00c0ffee00c0ffee00")?,
        observed_generation: StorageGeneration::from_value(1),
        revision_id: TranscriptRevisionId::parse("trv_4210915bdac994cd08f04a7ad5d99475")?,
    });
    Ok(record)
}

/// An interrupted job after two failed attempts.
fn interrupted() -> Built<JobRecord> {
    let mut record = base_record(JobState::Interrupted)?;
    record.attempt = 2;
    record.failures = vec![
        AttemptFailure {
            chunk: Some(0),
            code: FailureCode::Busy,
        },
        AttemptFailure {
            chunk: None,
            code: FailureCode::Cancelled,
        },
    ];
    Ok(record)
}

fn chunk() -> Built<PlannedChunk> {
    Ok(PlannedChunk::new(
        SourceSegmentId::parse("sgm_0123456789abcdef")?,
        0,
        range(0, 30_000_000)?,
    ))
}

fn recognised() -> Built<ChunkCheckpoint> {
    let window = range(0, 30_000_000)?;
    Ok(ChunkCheckpoint::new(
        RecognitionKey::new(Sha256Hex::parse(RECOGNITION_KEY)?),
        &chunk()?,
        CheckpointOutcome::Recognised {
            audio: window,
            output: ProviderChunkOutput {
                language: LanguageTag::parse("en").ok(),
                segments: vec![ProviderSegment {
                    start: ChunkTime::from_micros(1_000_000),
                    end: ChunkTime::from_micros(3_400_000),
                    text: Some(CueText::new(
                        "The service status is healthy.".to_owned(),
                        "The service status is healthy.".to_owned(),
                    )?),
                    tokens: vec![
                        ProviderToken {
                            kind: ProviderTokenKind::Special,
                            probability: 0.999_5,
                        },
                        ProviderToken {
                            kind: ProviderTokenKind::Text,
                            probability: 0.1 + 0.2,
                        },
                    ],
                }],
            },
        },
    ))
}

fn silent() -> Built<ChunkCheckpoint> {
    Ok(ChunkCheckpoint::new(
        RecognitionKey::new(Sha256Hex::parse(RECOGNITION_KEY)?),
        &chunk()?,
        CheckpointOutcome::Silent {
            audio: range(0, 6_000_000)?,
        },
    ))
}

/// Compares `encoded` with the committed example, or rewrites it.
fn matches_example(name: &str, encoded: &[u8]) -> Built<()> {
    let path = examples().join(name);
    if env::var("VSIFT_REGENERATE_JOB_EXAMPLES").is_ok_and(|value| value == "1") {
        fs::create_dir_all(examples())?;
        fs::write(&path, encoded)?;
    }
    if fs::read(&path)? != encoded {
        return Err(format!("{name} is not what the encoder writes").into());
    }
    Ok(())
}

#[test]
fn the_example_job_records_are_what_the_store_writes() -> Built<()> {
    for (name, record) in [
        ("job-record.succeeded.json", succeeded()?),
        ("job-record.interrupted.json", interrupted()?),
    ] {
        let encoded = encode_job_record(&record)?;
        matches_example(name, &encoded)?;
        assert_eq!(
            decode_job_record(&encoded, &record.job_id, &record.session_id)?,
            record
        );
    }
    Ok(())
}

#[test]
fn the_example_checkpoints_are_what_a_run_stores() -> Built<()> {
    for (name, checkpoint) in [
        ("checkpoint.recognised.json", recognised()?),
        ("checkpoint.silent.json", silent()?),
    ] {
        let encoded = encode_chunk_checkpoint(&checkpoint).ok_or("not encoded")?;
        matches_example(name, &encoded)?;
        assert_eq!(decode_chunk_checkpoint(&encoded, 1), Some(checkpoint));
        assert_eq!(decode_chunk_checkpoint(&encoded, 2), None);
    }
    Ok(())
}
