//! Conformance of the P11 worker contract (ADR 0021): `job-request`,
//! `job-result`, `job-batch-data` and `workspace-data` against the published
//! v1 schemas and their frozen examples.
//!
//! The requests are the frozen `job-request.json` (an F01 ingest into a
//! durable workspace: retranscription, candidates and a retained bundle) and
//! the two lines of `job-batch.requests.jsonl` (an F10 ingest with its
//! supplied transcript, and a retranscription of 5.5-6 s in an existing
//! session). The results are what a host presents for them.

use std::{
    collections::BTreeSet,
    fs, io,
    num::{NonZeroU16, NonZeroU32},
    path::PathBuf,
};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_contract::{
    BatchLine, BatchTermination, BundleName, CommandName, FreeSpaceReserve, JobBatchData,
    LifecycleResponse, MAX_RESULT_STEPS, MAX_WORK_RESULT_BYTES, OperationResponse,
    PARTIAL_REQUEST_WARNING, RecordedResultError, RequestDurability, RequestFailure,
    RequestRejection, ResourceLimits, ResultOrigin, StepOutputs, StepResult, StepStatus,
    StepTiming, WorkControls, WorkFailure, WorkRequest, WorkResult, WorkResultError,
    WorkResultParts, WorkStepKind, WorkerIsolation, WorkspaceData, WorkspaceInitOutcome,
    decode_batch_line, decode_work_request,
};
use vsift_domain::{
    CoverageGapReason, FailureCode, JobId, MediaTime, OperationId, PublicationGuarantee, SessionId,
    Sha256Hex, SourceId, StorageGeneration, TimeRange, TranscriptRevisionId, VisualCoverageGap,
    VisualIndexId,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const F01_SOURCE: &str =
    "src_sha256_f8222a928243160c8dbf5b1a9bc24277e49ac47c11fe5526826462877678e881";
const F10_SOURCE: &str =
    "src_sha256_d7ccece71288c5ff35d7b16c871a93a8ed48bbb7380617899575069b4d6545f4";
const JOB: &str = "job_8d3e1f0a2b4c6d8e0f1a2b3c4d5e6f70";
const EXPIRES_AT: &str = "2026-10-05T12:00:00Z";

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

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&load(schema_path)?)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

fn is_valid(schema_path: &str, instance: &Value) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&load(schema_path)?)?
        .is_valid(instance))
}

fn controls() -> Result<WorkControls, Box<dyn std::error::Error>> {
    Ok(WorkControls {
        isolation: WorkerIsolation::StrictLinux,
        admission_capacity: NonZeroU16::new(8).ok_or("zero")?,
        concurrency: NonZeroU16::new(2).ok_or("zero")?,
        resource_limits: ResourceLimits::HostCgroup,
        free_space_reserve: FreeSpaceReserve::Enforced,
    })
}

fn timing(elapsed_ms: u64, admission_wait_ms: u64) -> StepTiming {
    StepTiming {
        elapsed_ms,
        admission_wait_ms,
    }
}

fn frozen_request() -> Result<WorkRequest, Box<dyn std::error::Error>> {
    Ok(decode_work_request(
        read("examples/job-request.json")?.as_bytes(),
    )?)
}

/// The complete answer to `job-request.json`.
fn f01_steps() -> Result<Vec<StepResult>, Box<dyn std::error::Error>> {
    Ok(vec![
        StepResult::finished(
            &StepOutputs::Ingest {
                generation: StorageGeneration::from_value(1),
                revision_id: None,
            },
            timing(412, 0),
            None,
        ),
        StepResult::finished(
            &StepOutputs::Retranscribe {
                revision_id: TranscriptRevisionId::parse("trv_1f2e3d4c5b6a79880f1e2d3c4b5a6978")?,
                generation: StorageGeneration::from_value(2),
                chunks_reused: 0,
            },
            timing(21_530, 180),
            Some(&JobId::parse(JOB)?),
        ),
        StepResult::finished(
            &StepOutputs::Candidates {
                visual_index_id: VisualIndexId::parse("vix_31b0dcc02c3694707c889ba38d140c4f")?,
                generation: StorageGeneration::from_value(3),
                candidate_count: 4,
                gaps: Vec::new(),
            },
            timing(2_940, 0),
            None,
        ),
        StepResult::finished(
            &StepOutputs::Retain {
                bundle_name: BundleName::parse("f01-review")?,
                bundle_sha256: Sha256Hex::parse(
                    "3c9a0e5f7b2d4c6e8a1b3d5f7a9c1e3b5d7f9a1c3e5b7d9f1a3c5e7b9d1f3a5c",
                )?,
                artifact_count: 3,
            },
            timing(96, 0),
            None,
        ),
    ])
}

fn f01_result(origin: ResultOrigin) -> Result<WorkResult, Box<dyn std::error::Error>> {
    let request = frozen_request()?;
    Ok(WorkResult::new(WorkResultParts {
        operation_id: request.operation_id(),
        request_digest: request.digest(),
        origin,
        attempt: NonZeroU32::MIN,
        session_id: Some(&SessionId::parse(SESSION)?),
        source_id: Some(&SourceId::parse(F01_SOURCE)?),
        publication: Some(PublicationGuarantee::OsCrashDurable),
        lifecycle: Some(LifecycleResponse::durable_worker(EXPIRES_AT.to_owned())),
        steps: f01_steps()?,
        failure: None,
        controls: controls()?,
    })?)
}

fn job_run(result: &WorkResult) -> Result<Value, Box<dyn std::error::Error>> {
    let operation = OperationId::parse(result.operation_id())?;
    let response = match result.status() {
        vsift_domain::OperationStatus::Partial => OperationResponse::partial(
            CommandName::JobRun.identifier(),
            result,
            PARTIAL_REQUEST_WARNING,
        )?,
        _ => OperationResponse::complete(CommandName::JobRun.identifier(), result)?,
    };
    Ok(serde_json::to_value(
        response
            .with_operation_id(&operation)
            .with_lifecycle(LifecycleResponse::durable_worker(EXPIRES_AT.to_owned())),
    )?)
}

#[test]
fn the_frozen_requests_decode_and_validate() -> TestResult {
    let request = load("examples/job-request.json")?;
    validate("job-request.schema.json", &request)?;
    let decoded = frozen_request()?;
    assert_eq!(decoded.durability(), RequestDurability::Durable);
    assert_eq!(
        decoded
            .steps()
            .iter()
            .map(vsift_contract::WorkStep::kind)
            .collect::<Vec<_>>(),
        [
            WorkStepKind::Retranscribe,
            WorkStepKind::Candidates,
            WorkStepKind::Retain
        ]
    );

    let batch = read("examples/job-batch.requests.jsonl")?;
    let mut operations = BTreeSet::new();
    for line in batch.lines() {
        validate("job-request.schema.json", &serde_json::from_str(line)?)?;
        let BatchLine::Request(request) = decode_batch_line(line.as_bytes())? else {
            return Err("a frozen batch line is blank".into());
        };
        assert!(operations.insert(request.operation_id().clone()));
    }
    assert_eq!(operations.len(), 2);
    Ok(())
}

/// The request schema refuses what the decoder refuses where a schema can
/// say it, and accepts nothing the decoder's shape check refuses.
#[test]
fn the_request_schema_is_strict() -> TestResult {
    let frozen = load("examples/job-request.json")?;
    let variants = [
        ("/unknown", Value::Bool(true)),
        ("/schema_version", Value::from("2")),
        ("/operation_id", Value::from("op_short")),
        ("/durability", Value::from("sometimes")),
        ("/deadline_ms", Value::from(0)),
        ("/target/ingest/source", Value::from("/etc/passwd")),
        ("/target/ingest/source", Value::from("a\\b")),
        ("/target/ingest/source", Value::from("C:clip.mp4")),
        ("/target/ingest/source", Value::from("a//b")),
        ("/steps/2/retain/bundle_name", Value::from("Bad Name")),
    ];
    for (pointer, value) in variants {
        let mut instance = frozen.clone();
        let (parent, member) = pointer.rsplit_once('/').ok_or("pointer")?;
        instance
            .pointer_mut(parent)
            .and_then(Value::as_object_mut)
            .ok_or("parent")?
            .insert(member.to_owned(), value);
        assert!(
            !is_valid("job-request.schema.json", &instance)?,
            "{pointer} was accepted"
        );
        assert!(
            decode_work_request(serde_json::to_vec(&instance)?.as_slice()).is_err(),
            "{pointer} was decoded"
        );
    }
    let mut missing = frozen;
    missing
        .pointer_mut("/target/ingest")
        .and_then(Value::as_object_mut)
        .ok_or("ingest")?
        .remove("transcript");
    assert!(!is_valid("job-request.schema.json", &missing)?);
    assert_eq!(
        decode_work_request(serde_json::to_vec(&missing)?.as_slice()).err(),
        Some(RequestRejection::Malformed)
    );
    Ok(())
}

#[test]
fn a_complete_run_matches_its_frozen_example() -> TestResult {
    let result = f01_result(ResultOrigin::Fresh)?;
    let data = serde_json::to_value(&result)?;
    validate("job-result.schema.json", &data)?;
    assert_eq!(data["request_digest"], frozen_request()?.digest().as_str());
    assert_eq!(data["status"], "complete");

    let response = job_run(&result)?;
    validate("operation-response.schema.json", &response)?;
    assert_eq!(response, load("examples/job-run.json")?);
    Ok(())
}

/// A replay returns the recorded result, marked `replayed`, under the same
/// operation id and digest.
#[test]
fn a_replayed_run_matches_its_frozen_example() -> TestResult {
    let response = job_run(&f01_result(ResultOrigin::Replayed)?)?;
    validate("operation-response.schema.json", &response)?;
    validate("job-result.schema.json", &response["data"])?;
    assert_eq!(response["data"]["replayed"], true);
    assert_eq!(response, load("examples/job-run.replayed.json")?);

    let fresh = job_run(&f01_result(ResultOrigin::Fresh)?)?;
    let mut expected = fresh;
    expected["data"]["replayed"] = Value::Bool(true);
    assert_eq!(response, expected);
    Ok(())
}

/// Candidates that still hold an undecodable gap after the whole range
/// was analysed make the step, and so the request, `partial`.
#[test]
fn a_partial_run_matches_its_frozen_example() -> TestResult {
    let request = decode_work_request(
        br#"{"schema_version":"1","operation_id":"op_2a4c6e8f0b1d3f5a7c9e1b3d5f7a9c1e","durability":"durable","target":{"session_id":"ses_0123456789abcdef0123456789abcdef"},"steps":[{"candidates":{"range":{"from_us":0,"to_us":150000000}}}]}"#,
    )?;
    let gap = VisualCoverageGap {
        range: TimeRange::new(
            MediaTime::from_micros(60_000_000),
            MediaTime::from_micros(120_000_000),
        )?,
        reason: CoverageGapReason::Undecodable,
        dropped_candidates: 0,
    };
    let result = WorkResult::new(WorkResultParts {
        operation_id: request.operation_id(),
        request_digest: request.digest(),
        origin: ResultOrigin::Fresh,
        attempt: NonZeroU32::new(2).ok_or("zero")?,
        session_id: Some(&SessionId::parse(SESSION)?),
        source_id: Some(&SourceId::parse(F01_SOURCE)?),
        publication: Some(PublicationGuarantee::OsCrashDurable),
        lifecycle: Some(LifecycleResponse::durable_worker(EXPIRES_AT.to_owned())),
        steps: vec![StepResult::finished(
            &StepOutputs::Candidates {
                visual_index_id: VisualIndexId::parse("vix_5278827a5eb80d85a88983f2e69a5cba")?,
                generation: StorageGeneration::from_value(4),
                candidate_count: 3,
                gaps: vec![gap],
            },
            timing(9_870, 1_250),
            None,
        )],
        failure: None,
        controls: controls()?,
    })?;
    assert_eq!(result.status(), vsift_domain::OperationStatus::Partial);
    let response = job_run(&result)?;
    validate("operation-response.schema.json", &response)?;
    validate("job-result.schema.json", &response["data"])?;
    assert_eq!(response["status"], "partial");
    assert_eq!(response["data"]["steps"][0]["coverage"]["truncated"], true);
    assert_eq!(response, load("examples/job-run.partial.json")?);
    Ok(())
}

/// A step that fails ends the request: the steps after it did not start,
/// and the request's failure names the step.
#[test]
fn a_failed_step_ends_the_request() -> TestResult {
    let request = frozen_request()?;
    let mut steps = f01_steps()?;
    steps.truncate(1);
    steps.push(StepResult::failed(
        WorkStepKind::Retranscribe,
        timing(120_000, 60_000),
        Some(&JobId::parse(JOB)?),
        WorkFailure::new(FailureCode::Busy, Some(2_000)),
    ));
    steps.push(StepResult::not_started(WorkStepKind::Candidates));
    steps.push(StepResult::not_started(WorkStepKind::Retain));
    let parts = WorkResultParts {
        operation_id: request.operation_id(),
        request_digest: request.digest(),
        origin: ResultOrigin::Fresh,
        attempt: NonZeroU32::MIN,
        session_id: Some(&SessionId::parse(SESSION)?),
        source_id: Some(&SourceId::parse(F01_SOURCE)?),
        publication: Some(PublicationGuarantee::OsCrashDurable),
        lifecycle: None,
        steps,
        failure: None,
        controls: controls()?,
    };
    let result = WorkResult::new(parts.clone())?;
    let data = serde_json::to_value(&result)?;
    validate("job-result.schema.json", &data)?;
    assert_eq!(data["status"], "failed");
    assert_eq!(data["failure"]["code"], "BUSY");
    assert_eq!(data["failure"]["retry_after_ms"], 2_000);
    assert_eq!(data["failure"]["step"], 1);
    assert_eq!(data["steps"][2]["status"], "not_started");
    assert_eq!(result.failure_code(), Some(FailureCode::Busy));

    // A step that ran after the failure is a host defect.
    let mut inconsistent = parts;
    inconsistent.steps[2] = f01_steps()?.remove(2);
    assert_eq!(
        WorkResult::new(inconsistent).err(),
        Some(WorkResultError::Inconsistent)
    );
    Ok(())
}

#[test]
fn cancellation_and_rejection_are_reported_as_such() -> TestResult {
    let request = frozen_request()?;
    let base = |steps: Vec<StepResult>, failure| WorkResultParts {
        operation_id: request.operation_id(),
        request_digest: request.digest(),
        origin: ResultOrigin::Fresh,
        attempt: NonZeroU32::MIN,
        session_id: None,
        source_id: None,
        publication: None,
        lifecycle: None,
        steps,
        failure,
        controls: WorkControls {
            isolation: WorkerIsolation::ProcessOnly,
            admission_capacity: NonZeroU16::MIN,
            concurrency: NonZeroU16::MIN,
            resource_limits: ResourceLimits::NotEnforced,
            free_space_reserve: FreeSpaceReserve::NotEnforced,
        },
    };
    let cancelled = WorkResult::new(base(
        vec![StepResult::failed(
            WorkStepKind::Ingest,
            timing(5, 0),
            None,
            WorkFailure::new(FailureCode::Cancelled, Some(10)),
        )],
        None,
    ))?;
    let data = serde_json::to_value(&cancelled)?;
    validate("job-result.schema.json", &data)?;
    assert_eq!(data["status"], "cancelled");
    assert_eq!(data["steps"][0]["status"], "cancelled");
    // A hint is kept only for a retryable code.
    assert_eq!(data["failure"]["retry_after_ms"], Value::Null);

    let rejected = WorkResult::new(base(
        Vec::new(),
        Some(RequestFailure::Rejected(
            RequestRejection::WorkspaceNotDurable,
        )),
    ))?;
    let data = serde_json::to_value(&rejected)?;
    validate("job-result.schema.json", &data)?;
    assert_eq!(data["status"], "failed");
    assert_eq!(data["failure"]["code"], "INVALID_ARGUMENT");
    assert_eq!(data["failure"]["rejection"], "workspace_not_durable");
    assert_eq!(data["failure"]["step"], Value::Null);
    assert_eq!(data["steps"], Value::Array(Vec::new()));

    let refused_steps = WorkResult::new(base(
        vec![StepResult::not_started(WorkStepKind::Ingest)],
        Some(RequestFailure::NotStarted(WorkFailure::new(
            FailureCode::IsolationUnavailable,
            None,
        ))),
    ))?;
    assert_eq!(
        refused_steps.status(),
        vsift_domain::OperationStatus::Failed
    );
    assert_eq!(
        WorkResult::new(base(
            f01_steps()?,
            Some(RequestFailure::NotStarted(WorkFailure::new(
                FailureCode::Busy,
                None
            )))
        ))
        .err(),
        Some(WorkResultError::Inconsistent)
    );
    Ok(())
}

/// The largest result the model allows stays within 64 KiB and schema-valid.
#[test]
fn the_largest_result_is_bounded() -> TestResult {
    let request = frozen_request()?;
    let mut steps = Vec::new();
    let gaps: Vec<VisualCoverageGap> = (0_u64..200)
        .map(|index| {
            Ok(VisualCoverageGap {
                range: TimeRange::new(
                    MediaTime::from_micros(index * 2_000_000_000_000),
                    MediaTime::from_micros(index * 2_000_000_000_000 + 1_000_000_000_000),
                )?,
                reason: CoverageGapReason::NoDecodedFrame,
                dropped_candidates: 0,
            })
        })
        .collect::<Result<_, vsift_domain::TimeRangeError>>()?;
    for _ in 0..MAX_RESULT_STEPS {
        steps.push(StepResult::finished(
            &StepOutputs::Candidates {
                visual_index_id: VisualIndexId::parse(format!("vix_{}", "f".repeat(64)))?,
                generation: StorageGeneration::from_value(u64::MAX),
                candidate_count: u32::MAX,
                gaps: gaps.clone(),
            },
            timing(u64::MAX, u64::MAX),
            Some(&JobId::parse(format!("job_{}", "a".repeat(64)))?),
        ));
    }
    let result = WorkResult::new(WorkResultParts {
        operation_id: &OperationId::parse(format!("op_{}", "b".repeat(64)))?,
        request_digest: request.digest(),
        origin: ResultOrigin::Replayed,
        attempt: NonZeroU32::MAX,
        session_id: Some(&SessionId::parse(format!("ses_{}", "c".repeat(64)))?),
        source_id: Some(&SourceId::parse(F10_SOURCE)?),
        publication: Some(PublicationGuarantee::ProcessCrashConsistent),
        lifecycle: Some(LifecycleResponse::durable_worker(EXPIRES_AT.to_owned())),
        steps: steps.clone(),
        failure: None,
        controls: controls()?,
    })?;
    let bytes = serde_json::to_vec(&result)?;
    assert!(
        bytes.len() <= MAX_WORK_RESULT_BYTES,
        "{} bytes",
        bytes.len()
    );
    validate("job-result.schema.json", &serde_json::from_slice(&bytes)?)?;

    steps.push(StepResult::not_started(WorkStepKind::Close));
    assert_eq!(
        WorkResult::new(WorkResultParts {
            operation_id: request.operation_id(),
            request_digest: request.digest(),
            origin: ResultOrigin::Fresh,
            attempt: NonZeroU32::MIN,
            session_id: None,
            source_id: None,
            publication: None,
            lifecycle: None,
            steps,
            failure: None,
            controls: controls()?,
        })
        .err(),
        Some(WorkResultError::TooManySteps)
    );
    Ok(())
}

/// Every step and result enum is exactly what the schema publishes.
#[test]
fn published_enums_match_the_contract() -> TestResult {
    let schema = load("job-result.schema.json")?;
    let listed = |pointer: &str| -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
        schema
            .pointer(pointer)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{pointer} missing"))?
            .iter()
            .map(|member| {
                member
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "non-string member".into())
            })
            .collect()
    };
    let identifiers =
        |items: &mut dyn Iterator<Item = &'static str>| items.map(str::to_owned).collect();
    assert_eq!(
        listed("/$defs/step/properties/kind/enum")?,
        identifiers(&mut WorkStepKind::ALL.iter().map(|kind| kind.identifier()))
    );
    assert_eq!(
        listed("/$defs/step/properties/status/enum")?,
        identifiers(&mut StepStatus::ALL.iter().map(|status| status.identifier()))
    );
    assert_eq!(
        listed("/$defs/rejection/enum")?,
        identifiers(
            &mut RequestRejection::ALL
                .iter()
                .map(|rejection| rejection.identifier())
        )
    );
    assert_eq!(
        listed("/$defs/failure_code/enum")?,
        identifiers(&mut FailureCode::ALL.iter().map(|code| code.identifier()))
    );
    assert_eq!(
        listed("/properties/controls/properties/isolation/enum")?,
        identifiers(
            &mut WorkerIsolation::ALL
                .iter()
                .map(|isolation| isolation.identifier())
        )
    );
    assert_eq!(
        listed("/properties/controls/properties/resource_limits/enum")?,
        identifiers(&mut ResourceLimits::ALL.iter().map(|limits| limits.identifier()))
    );
    assert_eq!(
        listed("/properties/controls/properties/free_space_reserve/enum")?,
        identifiers(
            &mut FreeSpaceReserve::ALL
                .iter()
                .map(|reserve| reserve.identifier())
        )
    );
    let batch = load("job-batch-data.schema.json")?;
    let termination: BTreeSet<String> = batch
        .pointer("/properties/termination_reason/enum")
        .and_then(Value::as_array)
        .ok_or("termination_reason")?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    assert_eq!(
        termination,
        identifiers(
            &mut BatchTermination::ALL
                .iter()
                .map(|reason| reason.identifier())
        )
    );
    Ok(())
}

/// The `--json` form of the frozen batch: both requests complete.
#[test]
fn a_batch_summary_matches_its_frozen_example() -> TestResult {
    let batch = read("examples/job-batch.requests.jsonl")?;
    let mut operations = Vec::new();
    for line in batch.lines() {
        if let BatchLine::Request(request) = decode_batch_line(line.as_bytes())? {
            operations.push(request.operation_id().clone());
        }
    }
    let [first, second] = operations.as_slice() else {
        return Err("expected two requests".into());
    };
    let mut data = JobBatchData::new();
    // In the order the requests finished.
    data.record_result(2, second, vsift_domain::OperationStatus::Complete, None)?;
    data.record_result(1, first, vsift_domain::OperationStatus::Complete, None)?;
    let data = data.finish(BatchTermination::EndOfInput, None);
    let value = serde_json::to_value(&data)?;
    validate("job-batch-data.schema.json", &value)?;
    let response = serde_json::to_value(OperationResponse::complete(
        CommandName::JobBatch.identifier(),
        &data,
    )?)?;
    validate("operation-response.schema.json", &response)?;
    assert_eq!(response, load("examples/job-batch.json")?);

    let mut rejected = JobBatchData::new();
    rejected.record_rejection(3, None, RequestRejection::TooLarge)?;
    let rejected = rejected.finish(BatchTermination::Shutdown, Some(4));
    validate(
        "job-batch-data.schema.json",
        &serde_json::to_value(&rejected)?,
    )?;
    Ok(())
}

#[test]
fn a_workspace_initialisation_matches_its_frozen_example() -> TestResult {
    let data = WorkspaceData::new(
        RequestDurability::Durable,
        NonZeroU16::new(8).ok_or("zero")?,
        vsift_contract::DEFAULT_SESSION_RETENTION_SECONDS,
        WorkspaceInitOutcome::Created,
    )?;
    let value = serde_json::to_value(&data)?;
    validate("workspace-data.schema.json", &value)?;
    let response = serde_json::to_value(OperationResponse::complete(
        "session.init-workspace",
        &data,
    )?)?;
    validate("operation-response.schema.json", &response)?;
    assert_eq!(response, load("examples/workspace-init.json")?);

    let mut forged = value;
    forged["publication"] = Value::from("process_crash_consistent");
    assert!(!is_valid("workspace-data.schema.json", &forged)?);
    Ok(())
}

/// Durable sessions report their own guarantee since P10 PR 4.
#[test]
fn ingest_data_admits_a_durable_publication() -> TestResult {
    let schema = load("ingest-data.schema.json")?;
    let listed: BTreeSet<&str> = schema["properties"]["publication"]["enum"]
        .as_array()
        .ok_or("publication")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        listed,
        BTreeSet::from([
            PublicationGuarantee::ProcessCrashConsistent.identifier(),
            PublicationGuarantee::OsCrashDurable.identifier()
        ])
    );
    Ok(())
}

/// P11 PR 3: a request record keeps finished steps and an ended result as
/// their canonical bytes; each reads back to exactly what was recorded,
/// a replay changes only `replayed`, and a changed or foreign document is
/// refused.
#[test]
fn recorded_steps_and_results_read_back_exactly() -> TestResult {
    let fresh = f01_result(ResultOrigin::Fresh)?;
    let bytes = fresh.recorded_bytes()?;
    let decoded = WorkResult::decode_recorded(&bytes)?;
    assert_eq!(decoded, fresh);
    assert_eq!(
        serde_json::to_value(decoded.into_replayed())?,
        serde_json::to_value(f01_result(ResultOrigin::Replayed)?)?
    );
    // A replayed result records as the fresh one did.
    assert_eq!(f01_result(ResultOrigin::Replayed)?.recorded_bytes()?, bytes);
    for step in fresh.steps() {
        let recorded = step.recorded_bytes()?;
        assert_eq!(&StepResult::decode_recorded(&recorded)?, step);
    }

    // Failed and rejected results round-trip too.
    let request = frozen_request()?;
    let mut steps = f01_steps()?;
    steps.truncate(1);
    steps.push(StepResult::failed(
        WorkStepKind::Retranscribe,
        timing(12, 3),
        Some(&JobId::parse(JOB)?),
        WorkFailure::new(FailureCode::Busy, Some(2_000)),
    ));
    steps.push(StepResult::not_started(WorkStepKind::Candidates));
    let failed = WorkResult::new(WorkResultParts {
        operation_id: request.operation_id(),
        request_digest: request.digest(),
        origin: ResultOrigin::Fresh,
        attempt: NonZeroU32::new(3).ok_or("zero")?,
        session_id: Some(&SessionId::parse(SESSION)?),
        source_id: None,
        publication: None,
        lifecycle: Some(LifecycleResponse::ephemeral(EXPIRES_AT.to_owned())),
        steps,
        failure: None,
        controls: controls()?,
    })?;
    assert_eq!(
        WorkResult::decode_recorded(&failed.recorded_bytes()?)?,
        failed
    );
    let rejected = WorkResult::new(WorkResultParts {
        operation_id: request.operation_id(),
        request_digest: request.digest(),
        origin: ResultOrigin::Fresh,
        attempt: NonZeroU32::MIN,
        session_id: None,
        source_id: None,
        publication: None,
        lifecycle: None,
        steps: Vec::new(),
        failure: Some(RequestFailure::Rejected(
            RequestRejection::PathOutsideInputRoot,
        )),
        controls: controls()?,
    })?;
    assert_eq!(
        WorkResult::decode_recorded(&rejected.recorded_bytes()?)?,
        rejected
    );

    let text = String::from_utf8(bytes)?;
    for (from, to) in [
        ("\"replayed\":false", "\"replayed\":true"),
        ("\"status\":\"complete\"", "\"status\":\"partial\""),
        ("\"attempt\":1", "\"attempt\":0"),
        ("\"kind\":\"ingest\"", "\"kind\":\"search\""),
        ("\"isolation\":\"strict_linux\"", "\"isolation\":\"none\""),
        (
            "\"admission_wait_ms\":180,",
            "\"admission_wait_ms\":180,\"extra\":1,",
        ),
        ("{\"operation_id\"", " {\"operation_id\""),
        ("\"generation\":1,", "\"generation\":01,"),
    ] {
        let changed = text.replacen(from, to, 1);
        assert_ne!(changed, text, "{from}");
        assert!(
            WorkResult::decode_recorded(changed.as_bytes()).is_err(),
            "{from} -> {to} was accepted"
        );
    }
    // Member order is part of the canonical form.
    let moved = text
        .replacen(
            "\"operation_id\":\"op_5b1e0c7a9d2f4e6b8a3c1d0e9f7a6b5c\",",
            "",
            1,
        )
        .replacen(
            "\"status\":\"complete\",",
            "\"status\":\"complete\",\"operation_id\":\"op_5b1e0c7a9d2f4e6b8a3c1d0e9f7a6b5c\",",
            1,
        );
    assert_eq!(
        WorkResult::decode_recorded(moved.as_bytes()).err(),
        Some(RecordedResultError::NotCanonical)
    );
    assert_eq!(
        WorkResult::decode_recorded(&vec![b' '; MAX_WORK_RESULT_BYTES + 1]).err(),
        Some(RecordedResultError::TooLarge)
    );
    Ok(())
}
