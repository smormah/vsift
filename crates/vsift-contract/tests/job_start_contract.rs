//! `job start` and the progress of a `prepare` job through the v1 contract
//! (ADR 0033): the start's data, a `prepare` job's streams with their media
//! progress, and the `media` member of a progress event. Every answer
//! validates against the published schemas, and the frozen examples are what
//! the code gives.

use std::{fs, io, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::Resumability;
use vsift_contract::{
    CommandName, JobData, JobPresentation, JobRunner, JobStartData, OperationResponse,
    ProgressEventResponse, ProgressReport, StreamPresentation,
};
use vsift_domain::{
    JobId, JobKind, JobMode, JobState, MediaProgress, PrepareStream, ProgressStage, ProgressUpdate,
    SessionId,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const PREPARE_JOB: &str = "job_0123456789abcdef0123456789abcdef";
const STREAM_JOB: &str = "job_fedcba9876543210fedcba9876543210";
const SECOND: u64 = 1_000_000;

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

/// Compares `value` with the frozen example `name` (the parsed JSON).
fn assert_example(name: &str, value: &Value) -> TestResult {
    assert_eq!(*value, load(&format!("examples/{name}"))?, "{name}");
    Ok(())
}

/// Seven of 25 minutes recognised, 2.5 times real time, over four fresh
/// chunks: the estimate is 18 minutes over 2.5, 432 s.
fn media() -> MediaProgress {
    MediaProgress {
        from_us: 0,
        done_until_us: 420 * SECOND,
        to_us: Some(1_500 * SECOND),
        elapsed_us: 170 * SECOND,
        fresh_media_us: 400 * SECOND,
        fresh_elapsed_us: 160 * SECOND,
        fresh_chunks: 16,
    }
}

fn prepare_job(state: JobState, live: bool, streams: &[StreamPresentation<'_>]) -> Built<JobData> {
    let job = JobId::parse(PREPARE_JOB)?;
    let session = SessionId::parse(SESSION)?;
    Ok(JobData::new(&JobPresentation {
        job_id: &job,
        session_id: &session,
        kind: JobKind::Prepare,
        state,
        live_owner: live,
        resumability: Resumability::of(
            state,
            if live {
                vsift_application::JobLiveness::Owned
            } else {
                vsift_application::JobLiveness::Unowned
            },
            true,
        ),
        operation_id: None,
        requested: None,
        planned_chunks: None,
        checkpoints: 0,
        attempts: 1,
        result: None,
        failure: None,
        mode: Some(JobMode::Watched),
        streams,
    }))
}

fn running_stream(stream_job: &JobId, live: bool) -> StreamPresentation<'_> {
    StreamPresentation {
        stream: PrepareStream::Transcript,
        job_id: Some(stream_job),
        state: Some(JobState::Running),
        stage: Some(ProgressStage::RecognisingSpeech),
        chunks_done: Some(17),
        chunks_total: Some(60),
        published_until_us: Some(395 * SECOND),
        media: Some(media()),
        live,
    }
}

/// `job start` in the watched mode: the runner holds the job, which has not
/// recorded a stream yet; nothing ran here, so there is no outcome.
#[test]
fn a_watched_start_names_its_runner_and_no_outcome() -> TestResult {
    let data = JobStartData::new(
        prepare_job(JobState::Running, true, &[])?,
        "created",
        JobMode::Watched,
        "long_recording",
        JobRunner::Background("process_group"),
        None,
    );
    let response = serde_json::to_value(OperationResponse::complete(
        CommandName::JobStart.identifier(),
        &data,
    )?)?;
    validate("operation-response.schema.json", &response)?;
    validate("job-start-data.schema.json", &response["data"])?;
    assert_eq!(response["data"]["job"]["streams"], serde_json::json!([]));
    assert_example("job-start.watched.json", &response)?;

    // The schema holds a background runner to its detachment and a foreground
    // one to its outcome.
    let mut broken = response["data"].clone();
    broken["start"]["detachment"] = Value::Null;
    assert!(validate("job-start-data.schema.json", &broken).is_err());
    let mut broken = response["data"].clone();
    broken["start"]["runner"] = Value::from("foreground");
    broken["start"]["detachment"] = Value::Null;
    assert!(validate("job-start-data.schema.json", &broken).is_err());
    Ok(())
}

/// `job status` of a running `prepare` job: its stream's job, state, chunks,
/// how far it is published, and its media progress with the estimate and its
/// basis.
#[test]
fn a_running_prepare_job_reports_its_stream_in_media_time() -> TestResult {
    let stream_job = JobId::parse(STREAM_JOB)?;
    let streams = [running_stream(&stream_job, true)];
    let data = serde_json::to_value(prepare_job(JobState::Running, true, &streams)?)?;
    validate("job-data.schema.json", &data)?;
    let response = serde_json::to_value(OperationResponse::complete(
        CommandName::JobStatus.identifier(),
        &data,
    )?)?;
    assert_example("job-status.prepare.json", &response)?;
    let media = &data["streams"][0]["media"];
    assert_eq!(media["done_us"], 420 * SECOND);
    assert_eq!(media["percent"], 28);
    assert_eq!(media["speed"], 2.5);
    assert_eq!(media["estimate"]["remaining_ms"], 432_000);
    assert_eq!(media["estimate"]["basis"], "cumulative_measured_speed");

    // A stream no process runs keeps its figures but no estimate.
    let streams = [running_stream(&stream_job, false)];
    let data = serde_json::to_value(prepare_job(JobState::Interrupted, false, &streams)?)?;
    validate("job-data.schema.json", &data)?;
    assert_eq!(data["streams"][0]["media"]["estimate"], Value::Null);
    assert_eq!(data["streams"][0]["media"]["speed"], 2.5);

    // A retranscribe job carries neither a mode nor streams, as before.
    let job = JobId::parse(STREAM_JOB)?;
    let session = SessionId::parse(SESSION)?;
    let plain = serde_json::to_value(JobData::new(&JobPresentation {
        job_id: &job,
        session_id: &session,
        kind: JobKind::Retranscribe,
        state: JobState::Queued,
        live_owner: false,
        resumability: Resumability::NotStarted,
        operation_id: None,
        requested: None,
        planned_chunks: Some(3),
        checkpoints: 0,
        attempts: 0,
        result: None,
        failure: None,
        mode: None,
        streams: &[],
    }))?;
    validate("job-data.schema.json", &plain)?;
    assert!(plain.get("streams").is_none());
    assert!(plain["request"].get("mode").is_none());
    Ok(())
}

/// A `recognising_speech` progress event carries its media progress; other
/// stages, and updates without it, stay as they were.
#[test]
fn a_recognition_progress_event_carries_media_time() -> TestResult {
    let job = JobId::parse(STREAM_JOB)?;
    let event = |media| -> Built<Value> {
        let event = serde_json::to_value(ProgressEventResponse::new(
            4,
            CommandName::JobStart,
            &ProgressReport {
                update: ProgressUpdate {
                    stage: ProgressStage::RecognisingSpeech,
                    completed: 17,
                    total: Some(60),
                    media,
                },
                job: Some(&job),
                request: None,
                dropped: 0,
            },
        ))?;
        validate("progress-event.schema.json", &event)?;
        Ok(event)
    };
    let with = event(Some(media()))?;
    assert_eq!(with["media"]["total_us"], 1_500 * SECOND);
    assert_eq!(with["media"]["estimate"]["remaining_ms"], 432_000);
    let without = event(None)?;
    assert!(without.get("media").is_none());
    Ok(())
}
