//! Conformance of the P11 event kinds (`progress`, `lifecycle`, `result`;
//! ADR 0021) against their published schemas and the frozen stream
//! `examples/job-batch.events.jsonl`: the `--events jsonl` form of a batch of
//! the two requests of `examples/job-batch.requests.jsonl`, whose terminal
//! data is `examples/job-batch.json`'s.

use std::{collections::BTreeSet, fs, io, num::NonZeroU16, num::NonZeroU32, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde::Serialize;
use serde_json::Value;
use vsift_contract::{
    BatchItemStatus, BatchLine, BatchTermination, CommandName, EventKind, FreeSpaceReserve,
    JobBatchData, LifecycleEventResponse, LifecycleKind, LifecycleReason, LifecycleResponse,
    MAX_EVENT_LINE_BYTES, OperationResponse, ProgressEventResponse, ProgressReport, Readiness,
    RequestEnd, RequestRef, RequestRejection, ResourceLimits, ResultEventResponse, ResultOrigin,
    StepOutputs, StepResult, StepTiming, TerminalEventResponse, WorkControls, WorkRequest,
    WorkResult, WorkResultParts, WorkerIsolation, decode_batch_line,
};
use vsift_domain::{
    FailureCode, JobId, OperationStatus, ProgressStage, ProgressUnit, ProgressUpdate,
    PublicationGuarantee, SessionId, SourceId, StorageGeneration, TranscriptRevisionId,
    VisualIndexId,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const F01_SOURCE: &str =
    "src_sha256_f8222a928243160c8dbf5b1a9bc24277e49ac47c11fe5526826462877678e881";
const F10_SOURCE: &str =
    "src_sha256_d7ccece71288c5ff35d7b16c871a93a8ed48bbb7380617899575069b4d6545f4";
const F10_SESSION: &str = "ses_7a1c3e5b9d0f2a4c6e8b1d3f5a7c9e0b";
const JOB: &str = "job_8d3e1f0a2b4c6d8e0f1a2b3c4d5e6f70";
const EXPIRES_AT: &str = "2026-09-29T12:00:00Z";
const STREAM_EXAMPLE: &str = "examples/job-batch.events.jsonl";

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn read(relative_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(fs::read_to_string(schema_root().join(relative_path))?)
}

fn load(relative_path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&read(relative_path)?)?)
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

/// Validates one line of a stream against the schema of its kind.
fn validate_line(line: &Value) -> TestResult {
    let kind = line["event"].as_str().ok_or("event missing")?;
    validate(&format!("{kind}-event.schema.json"), line)?;
    match kind {
        "terminal" => validate("operation-response.schema.json", &line["result"])?,
        "result" => validate("job-result.schema.json", &line["result"])?,
        _ => {}
    }
    Ok(())
}

fn requests() -> Result<Vec<WorkRequest>, Box<dyn std::error::Error>> {
    let mut requests = Vec::new();
    for line in read("examples/job-batch.requests.jsonl")?.lines() {
        if let BatchLine::Request(request) = decode_batch_line(line.as_bytes())? {
            requests.push(*request);
        }
    }
    Ok(requests)
}

fn controls() -> Result<WorkControls, Box<dyn std::error::Error>> {
    Ok(WorkControls {
        isolation: WorkerIsolation::ProcessOnly,
        admission_capacity: NonZeroU16::new(4).ok_or("zero")?,
        concurrency: NonZeroU16::new(2).ok_or("zero")?,
        resource_limits: ResourceLimits::NotEnforced,
        free_space_reserve: FreeSpaceReserve::NotEnforced,
    })
}

const fn timing(elapsed_ms: u64, admission_wait_ms: u64) -> StepTiming {
    StepTiming {
        elapsed_ms,
        admission_wait_ms,
    }
}

/// Line 1: the F10 ingest with its supplied transcript, its candidates and
/// the close.
fn f10_result(request: &WorkRequest) -> Result<WorkResult, Box<dyn std::error::Error>> {
    Ok(WorkResult::new(WorkResultParts {
        operation_id: request.operation_id(),
        request_digest: request.digest(),
        origin: ResultOrigin::Fresh,
        attempt: NonZeroU32::MIN,
        session_id: Some(&SessionId::parse(F10_SESSION)?),
        source_id: Some(&SourceId::parse(F10_SOURCE)?),
        publication: Some(PublicationGuarantee::ProcessCrashConsistent),
        lifecycle: Some(LifecycleResponse::ephemeral(EXPIRES_AT.to_owned())),
        steps: vec![
            StepResult::finished(
                &StepOutputs::Ingest {
                    generation: StorageGeneration::from_value(1),
                    revision_id: Some(TranscriptRevisionId::parse(
                        "trv_663ae41bbedc740b651fe61999d395b0",
                    )?),
                },
                timing(310, 0),
                None,
            ),
            StepResult::finished(
                &StepOutputs::Candidates {
                    visual_index_id: VisualIndexId::parse("vix_31b0dcc02c3694707c889ba38d140c4f")?,
                    generation: StorageGeneration::from_value(2),
                    candidate_count: 3,
                    gaps: Vec::new(),
                },
                timing(1_870, 420),
                None,
            ),
            StepResult::finished(
                &StepOutputs::Close {
                    generation: StorageGeneration::from_value(3),
                },
                timing(12, 0),
                None,
            ),
        ],
        failure: None,
        controls: controls()?,
    })?)
}

/// Line 2: the 5.5-6 s retranscription of the F01 session and its close.
fn f01_result(request: &WorkRequest) -> Result<WorkResult, Box<dyn std::error::Error>> {
    Ok(WorkResult::new(WorkResultParts {
        operation_id: request.operation_id(),
        request_digest: request.digest(),
        origin: ResultOrigin::Fresh,
        attempt: NonZeroU32::MIN,
        session_id: Some(&SessionId::parse(SESSION)?),
        source_id: Some(&SourceId::parse(F01_SOURCE)?),
        publication: Some(PublicationGuarantee::ProcessCrashConsistent),
        lifecycle: Some(LifecycleResponse::ephemeral(EXPIRES_AT.to_owned())),
        steps: vec![
            StepResult::finished(
                &StepOutputs::Retranscribe {
                    revision_id: TranscriptRevisionId::parse(
                        "trv_4210915bdac994cd08f04a7ad5d99475",
                    )?,
                    generation: StorageGeneration::from_value(2),
                    chunks_reused: 0,
                },
                timing(1_460, 0),
                Some(&JobId::parse(JOB)?),
            ),
            StepResult::finished(
                &StepOutputs::Close {
                    generation: StorageGeneration::from_value(3),
                },
                timing(9, 0),
                None,
            ),
        ],
        failure: None,
        controls: controls()?,
    })?)
}

/// Appends one event, numbered by its position, as one line.
struct Stream {
    lines: Vec<String>,
}

impl Stream {
    fn next(&self) -> u64 {
        u64::try_from(self.lines.len()).unwrap_or(u64::MAX)
    }

    fn push(&mut self, event: &impl Serialize) -> TestResult {
        self.lines.push(serde_json::to_string(event)?);
        Ok(())
    }
}

fn batch_stream() -> Result<String, Box<dyn std::error::Error>> {
    let requests = requests()?;
    let [first, second] = requests.as_slice() else {
        return Err("expected two requests".into());
    };
    let command = CommandName::JobBatch;
    let job = JobId::parse(JOB)?;
    let line_one = RequestRef {
        line: Some(1),
        operation_id: Some(first.operation_id()),
    };
    let line_two = RequestRef {
        line: Some(2),
        operation_id: Some(second.operation_id()),
    };
    let finished = RequestEnd {
        status: BatchItemStatus::Complete,
        code: None,
        rejection: None,
        progress_dropped: 0,
    };
    let mut stream = Stream { lines: Vec::new() };
    stream.push(&LifecycleEventResponse::started(
        stream.next(),
        command,
        Readiness {
            publication: PublicationGuarantee::ProcessCrashConsistent,
            isolation: WorkerIsolation::ProcessOnly,
            admission_capacity: NonZeroU16::new(4).ok_or("zero")?,
            concurrency: NonZeroU16::new(2).ok_or("zero")?,
        },
    ))?;
    stream.push(&LifecycleEventResponse::request_admitted(
        stream.next(),
        command,
        &line_one,
    ))?;
    stream.push(&LifecycleEventResponse::request_admitted(
        stream.next(),
        command,
        &line_two,
    ))?;
    for completed in [0, 1] {
        stream.push(&ProgressEventResponse::new(
            stream.next(),
            command,
            &ProgressReport {
                update: ProgressUpdate {
                    stage: ProgressStage::RecognisingSpeech,
                    completed,
                    total: Some(1),
                },
                job: Some(&job),
                request: Some(second.operation_id()),
                dropped: 0,
            },
        ))?;
    }
    stream.push(&ResultEventResponse::new(
        stream.next(),
        command,
        Some(2),
        f01_result(second)?,
    ))?;
    stream.push(&LifecycleEventResponse::request_finished(
        stream.next(),
        command,
        &line_two,
        finished,
    ))?;
    stream.push(&ResultEventResponse::new(
        stream.next(),
        command,
        Some(1),
        f10_result(first)?,
    ))?;
    stream.push(&LifecycleEventResponse::request_finished(
        stream.next(),
        command,
        &line_one,
        finished,
    ))?;
    stream.push(&LifecycleEventResponse::stopped(
        stream.next(),
        command,
        LifecycleReason::EndOfInput,
    ))?;
    let mut data = JobBatchData::new();
    data.record_result(2, second.operation_id(), OperationStatus::Complete, None)?;
    data.record_result(1, first.operation_id(), OperationStatus::Complete, None)?;
    let data = data.finish(BatchTermination::EndOfInput, None);
    stream.push(&TerminalEventResponse::at_sequence(
        OperationResponse::complete(command.identifier(), &data)?,
        stream.next(),
    ))?;
    let mut text = stream.lines.join("\n");
    text.push('\n');
    Ok(text)
}

#[test]
fn the_batch_stream_matches_its_frozen_example_byte_for_byte() -> TestResult {
    let produced = batch_stream()?;
    assert_eq!(produced, read(STREAM_EXAMPLE)?);

    let lines: Vec<Value> = produced
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    for (index, line) in lines.iter().enumerate() {
        validate_line(line)?;
        assert_eq!(line["sequence"], u64::try_from(index)?);
    }
    for line in produced.lines().take(lines.len() - 1) {
        assert!(line.len() < MAX_EVENT_LINE_BYTES);
    }
    let terminal = lines.last().ok_or("empty stream")?;
    assert_eq!(terminal["event"], "terminal");
    assert_eq!(
        terminal["result"],
        load("examples/job-batch.json")?,
        "the terminal result is the --json result"
    );
    validate("job-batch-data.schema.json", &terminal["result"]["data"])?;
    Ok(())
}

/// A reader written before P11 knows `evidence` and `terminal` only; it
/// skips the new kinds, still sees a contiguous sequence, and ends at the
/// terminal event.
#[test]
fn an_older_reader_skips_the_new_kinds_and_still_counts_them() -> TestResult {
    let known = ["evidence", "terminal"];
    let mut expected_sequence = 0_u64;
    let mut skipped = 0_usize;
    let mut terminal = None;
    for line in read(STREAM_EXAMPLE)?.lines() {
        let event: Value = serde_json::from_str(line)?;
        assert_eq!(event["sequence"], expected_sequence, "gap in the sequence");
        expected_sequence += 1;
        if !known.contains(&event["event"].as_str().ok_or("event")?) {
            skipped += 1;
            continue;
        }
        terminal = Some(event);
    }
    assert_eq!(skipped, 10);
    let terminal = terminal.ok_or("no terminal event")?;
    assert_eq!(terminal["event"], "terminal");
    assert_eq!(terminal["sequence"], expected_sequence - 1);
    Ok(())
}

#[test]
fn every_lifecycle_kind_and_reason_is_schema_valid() -> TestResult {
    let operation = vsift_domain::OperationId::parse("op_0123456789abcdef")?;
    let request = RequestRef {
        line: Some(7),
        operation_id: Some(&operation),
    };
    let command = CommandName::JobBatch;
    let events = [
        LifecycleEventResponse::admission_waiting(0, command, &request),
        LifecycleEventResponse::request_finished(
            1,
            command,
            &RequestRef {
                line: Some(8),
                operation_id: None,
            },
            RequestEnd {
                status: BatchItemStatus::Rejected,
                code: Some(FailureCode::InvalidArgument),
                rejection: Some(RequestRejection::Malformed),
                progress_dropped: 0,
            },
        ),
        LifecycleEventResponse::request_finished(
            2,
            command,
            &request,
            RequestEnd {
                status: BatchItemStatus::Cancelled,
                code: Some(FailureCode::Cancelled),
                rejection: None,
                progress_dropped: 12,
            },
        ),
        LifecycleEventResponse::draining(3, command, LifecycleReason::Shutdown),
        LifecycleEventResponse::draining(4, command, LifecycleReason::DrainTimeout),
        LifecycleEventResponse::stopped(5, command, BatchTermination::Shutdown.into()),
        LifecycleEventResponse::stopped(6, command, BatchTermination::LineLimit.into()),
        LifecycleEventResponse::stopped(7, command, BatchTermination::InputError.into()),
        LifecycleEventResponse::request_admitted(
            8,
            CommandName::JobRun,
            &RequestRef {
                line: None,
                operation_id: Some(&operation),
            },
        ),
    ];
    for event in &events {
        validate_line(&serde_json::to_value(event)?)?;
    }
    // A readiness on any kind but `started`, or a status on any kind but
    // `request_finished`, is refused.
    let mut forged = serde_json::to_value(&events[0])?;
    forged["status"] = Value::from("complete");
    assert!(!is_valid("lifecycle-event.schema.json", &forged)?);
    let mut forged = serde_json::to_value(&events[3])?;
    forged["readiness"] = serde_json::json!({
        "publication": "os_crash_durable",
        "isolation": "strict_linux",
        "admission_capacity": 1,
        "concurrency": 1
    });
    assert!(!is_valid("lifecycle-event.schema.json", &forged)?);
    Ok(())
}

#[test]
fn every_progress_stage_is_schema_valid_with_its_unit() -> TestResult {
    for stage in ProgressStage::ALL {
        let event = serde_json::to_value(ProgressEventResponse::new(
            0,
            CommandName::TranscriptRetranscribe,
            &ProgressReport {
                update: ProgressUpdate {
                    stage,
                    completed: 3,
                    total: None,
                },
                job: None,
                request: None,
                dropped: 0,
            },
        ))?;
        validate_line(&event)?;
        assert_eq!(event["unit"], stage.unit().identifier());
        let mut forged = event;
        let other = ProgressUnit::ALL
            .into_iter()
            .find(|unit| *unit != stage.unit())
            .ok_or("one unit")?;
        forged["unit"] = Value::from(other.identifier());
        assert!(!is_valid("progress-event.schema.json", &forged)?);
    }
    Ok(())
}

fn schema_enum(
    schema: &str,
    pointer: &str,
) -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
    let value = load(schema)?;
    let node = value
        .pointer(pointer)
        .ok_or_else(|| format!("{schema} has nothing at {pointer}"))?;
    let members = node
        .get("enum")
        .or_else(|| {
            node.get("oneOf")?
                .as_array()?
                .iter()
                .find_map(|branch| branch.get("enum"))
        })
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{schema}{pointer} has no enum"))?;
    Ok(members
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect())
}

fn identifiers<T: Copy>(all: &[T], name: fn(T) -> &'static str) -> BTreeSet<String> {
    all.iter().map(|item| name(*item).to_owned()).collect()
}

/// Every enum an event publishes is exactly the contract's.
#[test]
fn published_event_enums_match_the_contract() -> TestResult {
    assert_eq!(
        schema_enum("progress-event.schema.json", "/properties/stage")?,
        identifiers(&ProgressStage::ALL, ProgressStage::identifier)
    );
    assert_eq!(
        schema_enum("progress-event.schema.json", "/properties/unit")?,
        identifiers(&ProgressUnit::ALL, ProgressUnit::identifier)
    );
    assert_eq!(
        schema_enum("lifecycle-event.schema.json", "/properties/kind")?,
        identifiers(&LifecycleKind::ALL, LifecycleKind::identifier)
    );
    assert_eq!(
        schema_enum("lifecycle-event.schema.json", "/properties/reason")?,
        identifiers(&LifecycleReason::ALL, LifecycleReason::identifier)
    );
    assert_eq!(
        schema_enum("lifecycle-event.schema.json", "/properties/status")?,
        identifiers(&BatchItemStatus::ALL, BatchItemStatus::identifier)
    );
    Ok(())
}

/// Only progress may be dropped for a slow reader.
#[test]
fn only_progress_is_droppable() {
    for kind in EventKind::ALL {
        assert_eq!(kind.droppable(), kind == EventKind::Progress, "{kind:?}");
    }
}

/// Every string member of the new event schemas is an enum, a constant or
/// a bounded pattern: nothing free-form can reach a supervisor's logs.
#[test]
fn every_string_member_of_the_new_events_is_bounded() -> TestResult {
    fn check(node: &Value, path: &str) -> Result<(), String> {
        if let Some(object) = node.as_object() {
            if object.get("type") == Some(&Value::from("string"))
                && !(object.contains_key("enum") || object.contains_key("const"))
                && !(object.contains_key("pattern") && object.contains_key("maxLength"))
            {
                return Err(format!("{path} is an unbounded string"));
            }
            for (key, value) in object {
                check(value, &format!("{path}/{key}"))?;
            }
        } else if let Some(items) = node.as_array() {
            for (index, value) in items.iter().enumerate() {
                check(value, &format!("{path}/{index}"))?;
            }
        }
        Ok(())
    }
    for schema in [
        "progress-event.schema.json",
        "lifecycle-event.schema.json",
        "result-event.schema.json",
    ] {
        let mut value = load(schema)?;
        if let Some(object) = value.as_object_mut() {
            // Documentation members are not instance constraints.
            object.remove("description");
            object.remove("title");
        }
        check(&value, schema)?;
    }
    Ok(())
}
