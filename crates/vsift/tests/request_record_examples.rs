//! The committed example worker request records
//! (`crates/vsift-infrastructure/tests/data/worker-requests/`) are exactly
//! what a worker host writes for known values, and read back to them (P11
//! PR 3, ADR 0021 section 4).
//!
//! They exist so the `request_record` fuzz target can seed from committed
//! repository files, as the harness's seed-provenance rules require, and they
//! pin the stored format: the record (the infrastructure's encoder) and the
//! recorded step and result documents inside it (the contract's canonical
//! form). A change to either fails here until the examples are regenerated
//! and reviewed.
//!
//! Regenerate with `VSIFT_REGENERATE_REQUEST_EXAMPLES=1` and review the diff.

use std::{
    env,
    error::Error,
    fs,
    num::{NonZeroU16, NonZeroU32},
    path::PathBuf,
};

use vsift_contract::{
    BundleName, FreeSpaceReserve, LifecycleResponse, ResourceLimits, ResultOrigin, StepOutputs,
    StepResult, StepTiming, WorkControls, WorkRequest, WorkResult, WorkResultParts,
    WorkerIsolation, decode_work_request,
};
use vsift_domain::{
    JobId, PublicationGuarantee, SessionId, Sha256Hex, SourceId, StorageGeneration,
    TranscriptRevisionId, VisualIndexId,
};
use vsift_infrastructure::{
    RecordedRequestResult, WorkerRequestRecord, decode_request_record, encode_request_record,
};

type Built<T> = Result<T, Box<dyn Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const SOURCE: &str = "src_sha256_f8222a928243160c8dbf5b1a9bc24277e49ac47c11fe5526826462877678e881";
const CREATED: u64 = 1_790_208_000;

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../vsift-infrastructure/tests/data/worker-requests")
}

/// The frozen request of the contract's examples.
fn frozen_request() -> Built<WorkRequest> {
    let text = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/v1/examples/job-request.json"),
    )?;
    Ok(decode_work_request(&text)?)
}

fn timing(elapsed_ms: u64, admission_wait_ms: u64) -> StepTiming {
    StepTiming {
        elapsed_ms,
        admission_wait_ms,
    }
}

/// The steps of the frozen request's complete run (as `job-run.json`).
fn steps() -> Built<Vec<StepResult>> {
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
            Some(&JobId::parse("job_8d3e1f0a2b4c6d8e0f1a2b3c4d5e6f70")?),
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

fn documents(steps: &[StepResult]) -> Built<Vec<String>> {
    steps
        .iter()
        .map(|step| Ok(String::from_utf8(step.recorded_bytes()?)?))
        .collect()
}

/// A request interrupted after its ingest and retranscription, on its
/// second attempt.
fn running() -> Built<WorkerRequestRecord> {
    let request = frozen_request()?;
    let mut finished = steps()?;
    finished.truncate(2);
    Ok(WorkerRequestRecord {
        operation_id: request.operation_id().clone(),
        request_digest: Sha256Hex::parse(request.digest().as_str())?,
        attempt: NonZeroU32::new(2).ok_or("zero")?,
        created_at_unix_seconds: CREATED,
        updated_at_unix_seconds: CREATED + 40,
        session_id: Some(SessionId::parse(SESSION)?),
        steps: documents(&finished)?,
        result: None,
    })
}

/// The same request once it ended: its recorded result only.
fn ended() -> Built<WorkerRequestRecord> {
    let request = frozen_request()?;
    let result = WorkResult::new(WorkResultParts {
        operation_id: request.operation_id(),
        request_digest: request.digest(),
        origin: ResultOrigin::Fresh,
        attempt: NonZeroU32::new(2).ok_or("zero")?,
        session_id: Some(&SessionId::parse(SESSION)?),
        source_id: Some(&SourceId::parse(SOURCE)?),
        publication: Some(PublicationGuarantee::OsCrashDurable),
        lifecycle: Some(LifecycleResponse::durable_worker(
            "2026-10-05T12:00:00Z".to_owned(),
        )),
        steps: steps()?,
        failure: None,
        controls: WorkControls {
            isolation: WorkerIsolation::StrictLinux,
            admission_capacity: NonZeroU16::new(8).ok_or("zero")?,
            concurrency: NonZeroU16::MIN,
            resource_limits: ResourceLimits::HostCgroup,
            free_space_reserve: FreeSpaceReserve::Enforced,
        },
    })?;
    let mut record = running()?;
    record.steps.clear();
    record.updated_at_unix_seconds = CREATED + 60;
    record.result = Some(RecordedRequestResult::new(String::from_utf8(
        result.recorded_bytes()?,
    )?)?);
    Ok(record)
}

#[test]
fn the_example_request_records_are_what_a_worker_writes() -> Result<(), Box<dyn Error>> {
    let regenerate = env::var_os("VSIFT_REGENERATE_REQUEST_EXAMPLES").is_some();
    for (name, record) in [
        ("request-record.running.json", running()?),
        ("request-record.ended.json", ended()?),
    ] {
        let path = examples().join(name);
        let encoded = encode_request_record(&record)?;
        if regenerate {
            fs::create_dir_all(examples())?;
            fs::write(&path, &encoded)?;
        }
        let committed = fs::read(&path)?;
        assert_eq!(committed, encoded, "{name} is not what the encoder writes");
        assert_eq!(
            decode_request_record(&committed, &record.operation_id)?,
            record,
            "{name}"
        );
        for step in &record.steps {
            StepResult::decode_recorded(step.as_bytes())?;
        }
        if let Some(result) = &record.result {
            WorkResult::decode_recorded(result.document().as_bytes())?;
        }
    }
    Ok(())
}
