//! Worker request records (P11 PR 3, ADR 0021 section 4): the owner lock is
//! the liveness authority, the record reads back exactly, a damaged record
//! is refused, and the workspace keeps a bounded number of records, pruning
//! only those whose session is gone.

use std::{fs, num::NonZeroU16, num::NonZeroU32, path::PathBuf};

use vsift_application::SessionStorageError;
use vsift_domain::{
    DurabilityRequirement, OperationId, SessionId, Sha256Hex, WorkspacePolicy, WorkspaceRetention,
};

use super::{
    FilesystemSessionStore, MAX_RECORDED_STEPS, RecordedRequestResult, RequestRecordWrite,
    SESSIONS_DIRECTORY, WorkerRequestClaim, WorkerRequestOwner, WorkerRequestRecord,
    decode_request_record, encode_request_record,
    tests::{Fixture, TestResult},
};

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OTHER_DIGEST: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn workspace(
    fixture: &Fixture,
) -> Result<(FilesystemSessionStore, PathBuf), Box<dyn std::error::Error>> {
    let root = fixture.path.join("workspace");
    let policy = WorkspacePolicy::new(
        DurabilityRequirement::Ephemeral,
        NonZeroU16::new(2).ok_or("zero")?,
        WorkspaceRetention::from_hours(2)?,
    )?;
    Ok((
        FilesystemSessionStore::provision_workspace(&root, policy)?,
        root,
    ))
}

fn operation(number: u32) -> Result<OperationId, Box<dyn std::error::Error>> {
    Ok(OperationId::parse(format!("op_{number:032x}"))?)
}

fn session(number: u32) -> Result<SessionId, Box<dyn std::error::Error>> {
    Ok(SessionId::parse(format!("ses_{number:032x}"))?)
}

fn running(
    operation_id: &OperationId,
    session_id: Option<SessionId>,
) -> Result<WorkerRequestRecord, Box<dyn std::error::Error>> {
    Ok(WorkerRequestRecord {
        operation_id: operation_id.clone(),
        request_digest: Sha256Hex::parse(DIGEST)?,
        attempt: NonZeroU32::MIN,
        created_at_unix_seconds: 1_000,
        updated_at_unix_seconds: 1_000,
        session_id,
        steps: vec![r#"{"kind":"ingest"}"#.to_owned()],
        result: None,
    })
}

fn owned(
    store: &FilesystemSessionStore,
    operation_id: &OperationId,
) -> Result<WorkerRequestOwner, Box<dyn std::error::Error>> {
    match store.claim_worker_request(operation_id)? {
        WorkerRequestClaim::Owned(owner) => Ok(owner),
        WorkerRequestClaim::HeldElsewhere(_) => Err("the request was held elsewhere".into()),
    }
}

/// The owner lock decides liveness: a second claim while the first owner
/// holds the request sees it held (with its record), and once the owner is
/// gone the next claim owns it and reads the record it left.
#[test]
fn the_owner_lock_decides_who_runs_a_request() -> TestResult {
    let fixture = Fixture::new()?;
    let (store, _) = workspace(&fixture)?;
    let operation_id = operation(1)?;
    assert_eq!(store.read_worker_request(&operation_id)?, None);

    let mut owner = owned(&store, &operation_id)?;
    assert_eq!(owner.record(), None);
    let record = running(&operation_id, Some(session(7)?))?;
    owner.write(record.clone(), RequestRecordWrite::Accept)?;
    assert_eq!(owner.record(), Some(&record));

    let second = FilesystemSessionStore::open_existing(fixture.path.join("workspace"))?;
    match second.claim_worker_request(&operation_id)? {
        WorkerRequestClaim::HeldElsewhere(seen) => assert_eq!(seen, Some(record.clone())),
        WorkerRequestClaim::Owned(_) => return Err("a held request was claimed twice".into()),
    }
    assert_eq!(
        second.read_worker_request(&operation_id)?,
        Some(record.clone())
    );
    drop(owner);

    let continued = owned(&second, &operation_id)?;
    assert_eq!(continued.record(), Some(&record));
    Ok(())
}

/// An ended record keeps only its result, whose digest is checked when it
/// is read back.
#[test]
fn an_ended_record_keeps_its_result_and_its_digest() -> TestResult {
    let fixture = Fixture::new()?;
    let (store, root) = workspace(&fixture)?;
    let operation_id = operation(2)?;
    let mut owner = owned(&store, &operation_id)?;
    owner.write(
        running(&operation_id, Some(session(1)?))?,
        RequestRecordWrite::Accept,
    )?;
    let result = RecordedRequestResult::new(r#"{"status":"complete"}"#.to_owned())?;
    let mut ended = running(&operation_id, Some(session(1)?))?;
    ended.steps.clear();
    ended.result = Some(result.clone());
    ended.updated_at_unix_seconds = 1_010;
    owner.write(ended.clone(), RequestRecordWrite::Complete)?;
    drop(owner);
    let read = store
        .read_worker_request(&operation_id)?
        .ok_or("no record")?;
    assert_eq!(read, ended);
    assert_eq!(
        read.result
            .as_ref()
            .map(|result| result.sha256().as_str().to_owned()),
        Some(super::sha256_hex(br#"{"status":"complete"}"#))
    );

    // A result changed on disk no longer matches its digest.
    let bucket = fs::read_dir(root.join("worker-requests"))?
        .next()
        .ok_or("no bucket")??
        .path();
    let file = bucket.join(format!("{}.json", operation_id.as_str()));
    let text = fs::read_to_string(&file)?;
    fs::write(&file, text.replace("complete", "partial!"))?;
    assert_eq!(
        store.read_worker_request(&operation_id).err(),
        Some(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

/// The codec round-trips and refuses everything that is not a valid record
/// of exactly this operation id.
#[test]
fn the_record_codec_is_strict() -> TestResult {
    let operation_id = operation(3)?;
    let record = running(&operation_id, Some(session(3)?))?;
    let bytes = encode_request_record(&record)?;
    assert_eq!(decode_request_record(&bytes, &operation_id)?, record);
    assert_eq!(
        decode_request_record(&bytes, &operation(4)?).err(),
        Some(SessionStorageError::IntegrityFailure)
    );
    let text = String::from_utf8(bytes)?;
    for (from, to) in [
        ("\"attempt\":1", "\"attempt\":0"),
        ("\"steps\":", "\"extra\":1,\"steps\":"),
        (DIGEST, OTHER_DIGEST.get(..63).ok_or("digest")?),
        (
            "\"updated_at_unix_seconds\":1000",
            "\"updated_at_unix_seconds\":999",
        ),
        ("\"result_sha256\":null", "\"result_sha256\":\"00\""),
        ("\"session_id\":\"ses_", "\"session_id\":\"src_"),
    ] {
        let changed = text.replacen(from, to, 1);
        assert_ne!(changed, text, "{from}");
        assert_eq!(
            decode_request_record(changed.as_bytes(), &operation_id).err(),
            Some(SessionStorageError::IntegrityFailure),
            "{from}"
        );
    }
    assert_eq!(
        decode_request_record(
            text.replacen("\"schema_version\":1", "\"schema_version\":2", 1)
                .as_bytes(),
            &operation_id
        )
        .err(),
        Some(SessionStorageError::UnsupportedVersion)
    );

    let mut both = record.clone();
    both.result = Some(RecordedRequestResult::new("{}".to_owned())?);
    assert_eq!(
        encode_request_record(&both).err(),
        Some(SessionStorageError::IntegrityFailure)
    );
    let mut many = record;
    many.steps = vec!["{}".to_owned(); MAX_RECORDED_STEPS + 1];
    assert_eq!(
        encode_request_record(&many).err(),
        Some(SessionStorageError::CapacityExhausted)
    );
    Ok(())
}

/// ADR 0021 section 4: at the cap, the records of sessions that are gone
/// are pruned first; a record whose session still exists, or that a process
/// holds, is kept, and with nothing to prune the workspace is full.
#[test]
fn a_full_workspace_prunes_only_records_whose_session_is_gone() -> TestResult {
    let fixture = Fixture::new()?;
    let (store, root) = workspace(&fixture)?;
    let sessions = root.join(SESSIONS_DIRECTORY);
    // Three records: one whose session exists, one held by a live owner
    // (session gone), one orphaned.
    let kept = operation(10)?;
    let held = operation(11)?;
    let orphan = operation(12)?;
    fs::create_dir(sessions.join(session(10)?.as_str()))?;
    for (operation_id, session_id) in [(&kept, session(10)?), (&orphan, session(12)?)] {
        let mut owner = owned(&store, operation_id)?;
        owner.write(
            running(operation_id, Some(session_id))?,
            RequestRecordWrite::Accept,
        )?;
    }
    let mut live = owned(&store, &held)?;
    live.write(
        running(&held, Some(session(11)?))?,
        RequestRecordWrite::Accept,
    )?;

    // A fourth request with room for three prunes the orphan only.
    let fourth = operation(13)?;
    let mut owner = owned(&store, &fourth)?.with_capacity(3);
    owner.write(
        running(&fourth, Some(session(13)?))?,
        RequestRecordWrite::Accept,
    )?;
    assert_eq!(store.read_worker_request(&orphan)?, None);
    assert!(store.read_worker_request(&kept)?.is_some());
    assert!(store.read_worker_request(&held)?.is_some());
    drop(owner);

    // Nothing left to prune: full.
    fs::create_dir(sessions.join(session(13)?.as_str()))?;
    let fifth = operation(14)?;
    let mut full = owned(&store, &fifth)?.with_capacity(3);
    assert_eq!(
        full.write(
            running(&fifth, Some(session(14)?))?,
            RequestRecordWrite::Accept
        )
        .err(),
        Some(SessionStorageError::CapacityExhausted)
    );
    assert_eq!(store.read_worker_request(&fifth)?, None);
    drop(live);
    Ok(())
}
