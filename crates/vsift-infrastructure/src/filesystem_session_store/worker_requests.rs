//! Worker request records (P11 PR 3, ADR 0021 section 4): what a worker host
//! knows about each request it was given, keyed by the caller's operation id.
//!
//! ```text
//! worker-requests/<bucket>/<op>.json     record, replaced atomically
//! worker-requests/<bucket>/<op>.lock     owner lock: exclusive while a process runs it
//! worker-requests/<bucket>/.<op>.tmp     the staged record of a replacement
//! ```
//!
//! The bucket is the first byte of SHA-256 of the operation id, as two
//! lowercase hexadecimal digits, so no directory holds more than a
//! fraction of the at most [`MAX_REQUEST_RECORDS`] records.
//!
//! A record names the request's digest, its attempt, the session it created
//! or used and, while it runs, the canonical documents of its finished steps;
//! once the request has ended it holds only the canonical document of its
//! result and that document's SHA-256. This adapter treats the documents as
//! opaque bounded text: their shape is the contract's (`vsift-contract`'s
//! recorded steps and results), and a document whose digest does not match is
//! an integrity failure. A record holds no path, evidence text or provider
//! output.
//!
//! **Liveness.** As for P10 jobs, the owner lock is the only liveness
//! authority: a request is held exactly while a process holds its lock, so a
//! crashed owner frees it at once and a suspended one keeps it. Locks are
//! taken without waiting. A lock file is removed only by pruning, while
//! pruning holds it; a claimant that locked a lock file pruning removed
//! notices (the name no longer names the file it locked) and reports the
//! request as held, so two processes never own one operation id.
//!
//! **Writes** are staged, flushed and renamed over the record; in a durable
//! workspace the bucket is then synchronised, and every directory this module
//! creates is synchronised into its parent (FS-01). Readers that do not hold
//! the lock open the record with the retrying open, so a record met in the
//! middle of a replacement is read whole, old or new.

use std::{fs, num::NonZeroU32, path::Path};

use cap_fs_ext::{DirExt, MetadataExt};
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vsift_application::SessionStorageError;
use vsift_domain::{DurabilityRequirement, OperationId, SessionId, Sha256Hex};

use super::{
    FilesystemSessionStore, MetadataVersion, SESSIONS_DIRECTORY, STORAGE_SCHEMA_VERSION,
    StoredDurability, create_private_child_directory,
    jobs::{create_or_open, replace_file, sync_if_durable},
    map_committed_io, map_lock_error, map_storage_io, open_regular_file, open_replaced_file,
    read_bounded_to,
    stored::parse_versioned_json,
};
use crate::{
    fault_point::{FaultPlan, FaultPoint},
    file_lock::HeldFileLock,
};

/// The root directory of the records.
const REQUESTS_DIRECTORY: &str = "worker-requests";
/// Most records one workspace keeps (ADR 0021 section 4).
pub const MAX_REQUEST_RECORDS: usize = 4_096;
/// Largest record file: an ended request's result (at most 64 KiB as the
/// contract bounds it, a little more once escaped as JSON text) and its
/// identities.
pub const MAX_REQUEST_RECORD_BYTES: u64 = 192 * 1024;
/// Largest recorded step or result document: the contract's result bound.
pub const MAX_RECORDED_DOCUMENT_BYTES: usize = 65_536;
/// Most finished steps a running record lists: an ingest and eight steps.
pub const MAX_RECORDED_STEPS: usize = 9;
/// Most directory entries one bucket scan reads before it gives up.
const MAX_BUCKET_ENTRIES: usize = 3 * MAX_REQUEST_RECORDS;

/// One request's record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerRequestRecord {
    /// The caller's operation id: the record's key.
    pub operation_id: OperationId,
    /// The digest of the request first accepted under the id.
    pub request_digest: Sha256Hex,
    /// The attempt that wrote the record, from 1; a continuation of an
    /// interrupted request counts one more.
    pub attempt: NonZeroU32,
    /// When the first attempt accepted the request.
    pub created_at_unix_seconds: u64,
    /// When the record last changed.
    pub updated_at_unix_seconds: u64,
    /// The session the request created or uses; an ingest target's is
    /// allocated and recorded before its copy starts.
    pub session_id: Option<SessionId>,
    /// While the request runs: the canonical documents of its finished
    /// steps, in order.
    pub steps: Vec<String>,
    /// Once the request has ended: its result.
    pub result: Option<RecordedRequestResult>,
}

/// The canonical document of an ended request's result and its SHA-256.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedRequestResult {
    document: String,
    sha256: Sha256Hex,
}

impl RecordedRequestResult {
    /// Records `document` with its digest.
    ///
    /// # Errors
    ///
    /// [`SessionStorageError::CapacityExhausted`] for a document over
    /// [`MAX_RECORDED_DOCUMENT_BYTES`].
    pub fn new(document: String) -> Result<Self, SessionStorageError> {
        if document.len() > MAX_RECORDED_DOCUMENT_BYTES {
            return Err(SessionStorageError::CapacityExhausted);
        }
        let sha256 = document_sha256(&document)?;
        Ok(Self { document, sha256 })
    }

    /// The canonical result document.
    #[must_use]
    pub fn document(&self) -> &str {
        &self.document
    }

    /// Its SHA-256: the same for every replay of the result.
    #[must_use]
    pub const fn sha256(&self) -> &Sha256Hex {
        &self.sha256
    }
}

fn document_sha256(document: &str) -> Result<Sha256Hex, SessionStorageError> {
    Sha256Hex::parse(super::sha256_hex(document.as_bytes()))
        .map_err(|_| SessionStorageError::IntegrityFailure)
}

/// Which write of a request's life a record replacement is: each passes its
/// own fault point after the record is in place (and synchronised).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestRecordWrite {
    /// An attempt accepted the request (a first attempt, or a continuation).
    Accept,
    /// A step finished.
    Step,
    /// The request ended; the record now holds its result.
    Complete,
}

impl RequestRecordWrite {
    const fn fault_point(self) -> FaultPoint {
        match self {
            Self::Accept => FaultPoint::RequestAccept,
            Self::Step => FaultPoint::RequestStep,
            Self::Complete => FaultPoint::RequestComplete,
        }
    }
}

/// The outcome of claiming a request.
pub enum WorkerRequestClaim {
    /// This process holds the request now; the record, if any, was read
    /// under the lock.
    Owned(WorkerRequestOwner),
    /// Another process holds the request; its record as it stands, if one
    /// was written yet.
    HeldElsewhere(Option<WorkerRequestRecord>),
}

/// Exclusive ownership of one request: its lock and its bucket.
pub struct WorkerRequestOwner {
    operation_id: OperationId,
    root: Dir,
    requests: Dir,
    bucket: Dir,
    durability: StoredDurability,
    plan: FaultPlan,
    record: Option<WorkerRequestRecord>,
    capacity: usize,
    _lock: HeldFileLock,
}

impl FilesystemSessionStore {
    /// The durability request records follow: the workspace's.
    fn request_durability(&self) -> StoredDurability {
        match self
            .workspace_policy()
            .map(vsift_domain::WorkspacePolicy::durability)
        {
            Some(DurabilityRequirement::Durable) => StoredDurability::Durable,
            Some(DurabilityRequirement::Ephemeral) | None => StoredDurability::Ephemeral,
        }
    }

    /// Reads a request's record without taking or probing its lock and
    /// without creating anything; `None` when there is none.
    ///
    /// # Errors
    ///
    /// Storage failures, and [`SessionStorageError::IntegrityFailure`] for a
    /// record that is not a valid record of `operation_id`.
    pub fn read_worker_request(
        &self,
        operation_id: &OperationId,
    ) -> Result<Option<WorkerRequestRecord>, SessionStorageError> {
        self.revalidate_root()?;
        let Some(requests) = open_existing(&self.root, REQUESTS_DIRECTORY)? else {
            return Ok(None);
        };
        let Some(bucket) = open_existing(&requests, &bucket_name(operation_id))? else {
            return Ok(None);
        };
        read_record(&bucket, operation_id)
    }

    /// Claims a request: takes its owner lock without waiting and reads its
    /// record under it, creating the directories and lock file it needs.
    ///
    /// # Errors
    ///
    /// Storage failures, and [`SessionStorageError::IntegrityFailure`] for a
    /// damaged record.
    pub fn claim_worker_request(
        &self,
        operation_id: &OperationId,
    ) -> Result<WorkerRequestClaim, SessionStorageError> {
        self.revalidate_root()?;
        let durability = self.request_durability();
        let requests = open_or_create(&self.root, REQUESTS_DIRECTORY, durability)?;
        let bucket = open_or_create(&requests, &bucket_name(operation_id), durability)?;
        let lock_name = lock_name(operation_id);
        let lock_file = create_or_open(&bucket, &lock_name)?;
        let locked_identity = identity(&lock_file)?;
        let lock = match HeldFileLock::try_exclusive(lock_file.into_std()) {
            Ok(lock) => lock,
            Err(fs::TryLockError::WouldBlock) => {
                return Ok(WorkerRequestClaim::HeldElsewhere(read_record(
                    &bucket,
                    operation_id,
                )?));
            }
            Err(error) => return Err(map_lock_error(error)),
        };
        // Pruning removes a lock file while holding it; a claimant that
        // opened it just before and locked it just after holds nothing.
        let current = open_regular_file(&bucket, &lock_name, true)
            .ok()
            .map(|file| identity(&file))
            .transpose()?;
        if current != Some(locked_identity) {
            lock.release().map_err(map_storage_io)?;
            return Ok(WorkerRequestClaim::HeldElsewhere(read_record(
                &bucket,
                operation_id,
            )?));
        }
        let record = read_record(&bucket, operation_id)?;
        Ok(WorkerRequestClaim::Owned(WorkerRequestOwner {
            operation_id: operation_id.clone(),
            root: self.root.try_clone().map_err(map_storage_io)?,
            requests,
            bucket,
            durability,
            plan: FaultPlan::from_environment(),
            record,
            capacity: MAX_REQUEST_RECORDS,
            _lock: lock,
        }))
    }
}

impl WorkerRequestOwner {
    /// A smaller record cap, so a test can fill a workspace.
    #[cfg(test)]
    pub(super) const fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    /// The record as of the last write (or as read when claimed).
    #[must_use]
    pub const fn record(&self) -> Option<&WorkerRequestRecord> {
        self.record.as_ref()
    }

    /// Replaces the record. The first record of an operation id first makes
    /// room: at [`MAX_REQUEST_RECORDS`] the records of sessions that no
    /// longer exist are removed, and if none is, the workspace is full.
    ///
    /// # Errors
    ///
    /// [`SessionStorageError::CapacityExhausted`] when the workspace holds
    /// its maximum of records and none can be pruned, or when the record is
    /// over its bounds; [`SessionStorageError::IntegrityFailure`] for a
    /// record of another operation id; storage failures.
    pub fn write(
        &mut self,
        record: WorkerRequestRecord,
        write: RequestRecordWrite,
    ) -> Result<(), SessionStorageError> {
        if record.operation_id != self.operation_id {
            return Err(SessionStorageError::IntegrityFailure);
        }
        let bytes = encode_request_record(&record)?;
        if self.record.is_none() {
            self.make_room()?;
        }
        replace_file(
            &self.bucket,
            &record_name(&self.operation_id),
            &staged_name(&self.operation_id),
            &bytes,
            self.durability,
            None,
        )?;
        self.plan.reach(write.fault_point());
        self.record = Some(record);
        Ok(())
    }

    /// Makes room for one more record (see [`Self::write`]).
    fn make_room(&self) -> Result<(), SessionStorageError> {
        let buckets = entry_names(&self.requests)?;
        let mut count = 0_usize;
        for name in &buckets {
            count += record_names(&open_bucket(&self.requests, name)?)?.len();
        }
        if count < self.capacity {
            return Ok(());
        }
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_committed_io)?;
        let mut pruned = 0_usize;
        for name in &buckets {
            let bucket = open_bucket(&self.requests, name)?;
            for operation_id in record_names(&bucket)? {
                if operation_id == self.operation_id {
                    continue;
                }
                if prune_if_orphaned(&bucket, &operation_id, &sessions)? {
                    pruned += 1;
                }
            }
            if pruned > 0 {
                sync_if_durable(self.durability, &bucket)?;
            }
        }
        if count.saturating_sub(pruned) < self.capacity {
            Ok(())
        } else {
            Err(SessionStorageError::CapacityExhausted)
        }
    }
}

/// Removes the record of `operation_id` and its lock file if no process
/// holds it and the session it names no longer exists (or it names none).
fn prune_if_orphaned(
    bucket: &Dir,
    operation_id: &OperationId,
    sessions: &Dir,
) -> Result<bool, SessionStorageError> {
    let lock_name = lock_name(operation_id);
    let lock = match open_regular_file(bucket, &lock_name, true) {
        Ok(file) => match HeldFileLock::try_exclusive(file.into_std()) {
            Ok(lock) => Some(lock),
            Err(fs::TryLockError::WouldBlock) => return Ok(false),
            Err(error) => return Err(map_lock_error(error)),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(map_storage_io(error)),
    };
    // A record that cannot be read names nothing that can be proven gone;
    // it stays, and counts.
    let Ok(Some(record)) = read_record(bucket, operation_id) else {
        return Ok(false);
    };
    let session_exists = match &record.session_id {
        Some(session) => sessions
            .try_exists(session.as_str())
            .map_err(map_storage_io)?,
        None => false,
    };
    if session_exists {
        return Ok(false);
    }
    bucket
        .remove_file(record_name(operation_id))
        .map_err(map_storage_io)?;
    if lock.is_some() {
        bucket.remove_file(&lock_name).map_err(map_storage_io)?;
    }
    if let Some(lock) = lock {
        lock.release().map_err(map_storage_io)?;
    }
    Ok(true)
}

/// The identity of an open file: device and file index.
fn identity(file: &cap_std::fs::File) -> Result<(u64, u64), SessionStorageError> {
    let metadata = file.metadata().map_err(map_storage_io)?;
    Ok((MetadataExt::dev(&metadata), MetadataExt::ino(&metadata)))
}

fn bucket_name(operation_id: &OperationId) -> String {
    let digest = Sha256::digest(operation_id.as_str().as_bytes());
    format!("{:02x}", digest[0])
}

fn record_name(operation_id: &OperationId) -> String {
    format!("{}.json", operation_id.as_str())
}

fn lock_name(operation_id: &OperationId) -> String {
    format!("{}.lock", operation_id.as_str())
}

fn staged_name(operation_id: &OperationId) -> String {
    format!(".{}.tmp", operation_id.as_str())
}

/// Opens `name` in `parent` if it exists.
fn open_existing(parent: &Dir, name: &str) -> Result<Option<Dir>, SessionStorageError> {
    if !parent.try_exists(name).map_err(map_storage_io)? {
        return Ok(None);
    }
    parent
        .open_dir_nofollow(name)
        .map(Some)
        .map_err(map_committed_io)
}

/// Opens `name` in `parent`, creating it (and synchronising `parent` in a
/// durable workspace) first if it is missing.
fn open_or_create(
    parent: &Dir,
    name: &str,
    durability: StoredDurability,
) -> Result<Dir, SessionStorageError> {
    if !parent.try_exists(name).map_err(map_storage_io)? {
        match create_private_child_directory(parent, Path::new(name)) {
            Ok(()) => sync_if_durable(durability, parent)?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(map_storage_io(error)),
        }
    }
    parent.open_dir_nofollow(name).map_err(map_committed_io)
}

/// Opens one bucket named in `worker-requests/`; any other name is damage.
fn open_bucket(requests: &Dir, name: &str) -> Result<Dir, SessionStorageError> {
    let valid = name.len() == 2
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !valid {
        return Err(SessionStorageError::IntegrityFailure);
    }
    requests.open_dir_nofollow(name).map_err(map_committed_io)
}

/// The names in a directory, at most [`MAX_BUCKET_ENTRIES`].
fn entry_names(directory: &Dir) -> Result<Vec<String>, SessionStorageError> {
    let mut names = Vec::new();
    for entry in directory.entries().map_err(map_storage_io)? {
        let entry = entry.map_err(map_storage_io)?;
        if names.len() >= MAX_BUCKET_ENTRIES {
            return Err(SessionStorageError::CapacityExhausted);
        }
        names.push(
            entry
                .file_name()
                .into_string()
                .map_err(|_| SessionStorageError::IntegrityFailure)?,
        );
    }
    names.sort();
    Ok(names)
}

/// The operation ids whose records a bucket holds. Lock files and staged
/// records are expected; any other name is damage.
fn record_names(bucket: &Dir) -> Result<Vec<OperationId>, SessionStorageError> {
    let mut records = Vec::new();
    for name in entry_names(bucket)? {
        if let Some(operation) = name.strip_suffix(".json") {
            records.push(
                OperationId::parse(operation).map_err(|_| SessionStorageError::IntegrityFailure)?,
            );
        } else if let Some(operation) = name.strip_suffix(".lock").or_else(|| {
            name.strip_prefix('.')
                .and_then(|staged| staged.strip_suffix(".tmp"))
        }) {
            OperationId::parse(operation).map_err(|_| SessionStorageError::IntegrityFailure)?;
        } else {
            return Err(SessionStorageError::IntegrityFailure);
        }
    }
    Ok(records)
}

/// Reads the record of `operation_id` from its bucket, or `None`.
fn read_record(
    bucket: &Dir,
    operation_id: &OperationId,
) -> Result<Option<WorkerRequestRecord>, SessionStorageError> {
    let file = match open_replaced_file(bucket, &record_name(operation_id), false) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(map_committed_io(error)),
    };
    let bytes = read_bounded_to(file, MAX_REQUEST_RECORD_BYTES).map_err(map_committed_io)?;
    decode_request_record(&bytes, operation_id).map(Some)
}

/// The stored shape of a record (`schema_version` 1).
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRequestRecord {
    schema_version: u16,
    operation_id: String,
    request_digest: String,
    attempt: u32,
    created_at_unix_seconds: u64,
    updated_at_unix_seconds: u64,
    session_id: Option<String>,
    steps: Vec<String>,
    result: Option<String>,
    result_sha256: Option<String>,
}

impl MetadataVersion for StoredRequestRecord {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

/// Encodes a request record exactly as the store writes one.
///
/// # Errors
///
/// [`SessionStorageError::CapacityExhausted`] for a record over its bounds
/// (more than [`MAX_RECORDED_STEPS`] steps, a document over
/// [`MAX_RECORDED_DOCUMENT_BYTES`], a file over
/// [`MAX_REQUEST_RECORD_BYTES`]); [`SessionStorageError::IntegrityFailure`]
/// for a record that lists steps and a result, or that changed before it was
/// created.
pub fn encode_request_record(record: &WorkerRequestRecord) -> Result<Vec<u8>, SessionStorageError> {
    if record.steps.len() > MAX_RECORDED_STEPS
        || record
            .steps
            .iter()
            .any(|step| step.len() > MAX_RECORDED_DOCUMENT_BYTES)
    {
        return Err(SessionStorageError::CapacityExhausted);
    }
    if (record.result.is_some() && !record.steps.is_empty())
        || record.updated_at_unix_seconds < record.created_at_unix_seconds
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let stored = StoredRequestRecord {
        schema_version: STORAGE_SCHEMA_VERSION,
        operation_id: record.operation_id.as_str().to_owned(),
        request_digest: record.request_digest.as_str().to_owned(),
        attempt: record.attempt.get(),
        created_at_unix_seconds: record.created_at_unix_seconds,
        updated_at_unix_seconds: record.updated_at_unix_seconds,
        session_id: record
            .session_id
            .as_ref()
            .map(|session| session.as_str().to_owned()),
        steps: record.steps.clone(),
        result: record.result.as_ref().map(|result| result.document.clone()),
        result_sha256: record
            .result
            .as_ref()
            .map(|result| result.sha256.as_str().to_owned()),
    };
    let bytes = serde_json::to_vec(&stored).map_err(|_| SessionStorageError::Io)?;
    if u64::try_from(bytes.len()).map_or(true, |size| size > MAX_REQUEST_RECORD_BYTES) {
        return Err(SessionStorageError::CapacityExhausted);
    }
    Ok(bytes)
}

/// Decodes the record of `operation_id` (`worker-requests/<bucket>/<op>.json`
/// v1) exactly as the store reads one back.
///
/// Public for the `request_record` fuzz target (ADR 0016 decision 6): the
/// record is private storage, read only after ownership and link checks, but
/// it is still untrusted input to its decoder.
///
/// # Errors
///
/// [`SessionStorageError::UnsupportedVersion`] for a newer record and
/// [`SessionStorageError::IntegrityFailure`] for anything that is not a
/// valid record of exactly this operation id: an unknown or missing member,
/// an identifier outside its grammar, more than [`MAX_RECORDED_STEPS`] steps,
/// a document over [`MAX_RECORDED_DOCUMENT_BYTES`], steps beside a result,
/// or a result whose digest does not match.
pub fn decode_request_record(
    bytes: &[u8],
    operation_id: &OperationId,
) -> Result<WorkerRequestRecord, SessionStorageError> {
    if u64::try_from(bytes.len()).map_or(true, |size| size > MAX_REQUEST_RECORD_BYTES) {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let stored: StoredRequestRecord = parse_versioned_json(bytes)?;
    let integrity = |valid: bool| {
        if valid {
            Ok(())
        } else {
            Err(SessionStorageError::IntegrityFailure)
        }
    };
    integrity(stored.operation_id == operation_id.as_str())?;
    let request_digest = Sha256Hex::parse(stored.request_digest)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let attempt = NonZeroU32::new(stored.attempt).ok_or(SessionStorageError::IntegrityFailure)?;
    integrity(stored.updated_at_unix_seconds >= stored.created_at_unix_seconds)?;
    let session_id = stored
        .session_id
        .map(SessionId::parse)
        .transpose()
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    integrity(
        stored.steps.len() <= MAX_RECORDED_STEPS
            && stored
                .steps
                .iter()
                .all(|step| step.len() <= MAX_RECORDED_DOCUMENT_BYTES),
    )?;
    let result = match (stored.result, stored.result_sha256) {
        (None, None) => None,
        (Some(document), Some(sha256)) => {
            integrity(stored.steps.is_empty() && document.len() <= MAX_RECORDED_DOCUMENT_BYTES)?;
            let computed = document_sha256(&document)?;
            integrity(computed.as_str() == sha256)?;
            Some(RecordedRequestResult {
                document,
                sha256: computed,
            })
        }
        (Some(_), None) | (None, Some(_)) => return Err(SessionStorageError::IntegrityFailure),
    };
    Ok(WorkerRequestRecord {
        operation_id: operation_id.clone(),
        request_digest,
        attempt,
        created_at_unix_seconds: stored.created_at_unix_seconds,
        updated_at_unix_seconds: stored.updated_at_unix_seconds,
        session_id,
        steps: stored.steps,
        result,
    })
}
