//! Recoverable jobs in session storage (P10, ADR 0020 section 4): the
//! [`JobStore`], [`JobOwner`], [`ChunkCheckpoints`], [`CommitLedger`] and
//! [`RevisionStore`] ports over the private root.
//!
//! ```text
//! sessions/<ses>/jobs/<job_id>/job.json          record, replaced atomically
//! sessions/<ses>/jobs/<job_id>/owner.lock        exclusive while an attempt runs
//! sessions/<ses>/jobs/<job_id>/state.lock        brief: serialises changes with cancel
//! sessions/<ses>/jobs/<job_id>/chunks/<n>.json   immutable chunk checkpoints
//! sessions/<ses>/jobs/by-operation/<op>.json     operation id -> job
//! job-index/<bucket>/<job_id>.json               job -> session
//! ```
//!
//! Everything under `jobs/` is a private, uncommitted stage file: never in a
//! manifest, never exported by `session retain`, removed with the session.
//! Every file is written to a staged name, flushed and renamed over its final
//! name; a durable session also synchronises the directory (FS-01).
//!
//! **Liveness and locks.** The owner lock is the only liveness authority: a
//! job is live exactly while some process holds it, so a suspended owner
//! keeps its job (X-05) and a crashed one frees it at once. Locks are always
//! taken without waiting, so no order can deadlock; the order used is the
//! session's shared lifetime hold (cleanup then stays busy), the job's owner
//! lock, then (inside a publication) admission, the lifetime hold again and
//! the writer lock. The state lock is taken for a few file operations only,
//! with a short bounded retry, and nothing waits while it is held.
//!
//! Every change goes through [`JobRecord::changed`] (the domain state graph)
//! on the record as stored under the state lock, fenced by the owner's epoch
//! and attempt, so a cancellation requested by another process between two
//! changes is seen and a stale owner cannot change the job.

use std::{fs, io::Write, path::Path, thread, time::Duration};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use sha2::{Digest, Sha256};
use vsift_application::{
    CancelRequest, CheckpointRead, CheckpointStoreError, ChunkCheckpoints, CommitLedger, JobChange,
    JobLiveness, JobOwner, JobRecord, JobSpec, JobStore, JobStoreError, JobView, MAX_SESSION_JOBS,
    RevisionStore, SessionHead, SessionStorageError,
};
use vsift_domain::{
    ChunkCheckpoint, JobId, JobState, MAX_PLANNED_CHUNKS, OperationId, SessionArtifactKind,
    SessionId, SessionPhase, StorageGeneration, TranscriptRevision, TranscriptRevisionId,
};

use super::{
    COORDINATION_DIRECTORY, FilesystemAdmissionPermit, FilesystemSessionStore,
    GENERATIONS_DIRECTORY, GenerationManifest, SESSIONS_DIRECTORY, StoredDurability,
    chain::read_committed_manifest,
    commit::sync_directory,
    create_private_child_directory,
    job_records::{
        MAX_CHECKPOINT_BYTES, StoredBinding, StoredJobIndex, checkpoint_name, checkpoint_ordinal,
        decode_checkpoint, decode_job, encode_checkpoint, encode_job,
    },
    map_lock_error, map_storage_io, open_regular_file, open_replaced_file, open_session_lock,
    read_bounded, read_bounded_manifest, read_bounded_to, sha256_hex,
    stored::{parse_versioned_json, read_versioned_json_file},
};
use crate::{
    encode_transcript_record,
    fault_point::{FaultPlan, FaultPoint},
    file_lock::HeldFileLock,
};

const JOBS_DIRECTORY: &str = "jobs";
const BY_OPERATION_DIRECTORY: &str = "by-operation";
const JOB_FILE: &str = "job.json";
const JOB_STAGED_FILE: &str = "job.json.tmp";
const OWNER_LOCK_FILE: &str = "owner.lock";
const STATE_LOCK_FILE: &str = "state.lock";
const CHUNKS_DIRECTORY: &str = "chunks";
const JOB_INDEX_DIRECTORY: &str = "job-index";
/// Most operation bindings one session keeps.
const MAX_SESSION_BINDINGS: usize = 256;
/// Most entries a scan of `jobs/` or `chunks/` reads before it gives up.
const MAX_SCANNED_ENTRIES: usize = 2 * MAX_PLANNED_CHUNKS;
/// How often, one millisecond apart, a busy state lock is tried again. Its
/// holders never wait while holding it, so it frees in far less.
const STATE_LOCK_ATTEMPTS: u32 = 500;

/// Exclusive ownership of one job: its owner lock, a shared hold on its
/// session's lifetime, and the record as of the last change.
pub struct FilesystemJobOwner {
    session_id: SessionId,
    jobs: Dir,
    job: Dir,
    chunks: Dir,
    durability: StoredDurability,
    plan: FaultPlan,
    record: JobRecord,
    _owner: HeldFileLock,
    _lifetime: HeldFileLock,
}

/// A session's jobs directory with the shared lifetime hold that keeps
/// cleanup away while it is used.
struct SessionJobs {
    lifetime: HeldFileLock,
    session: Dir,
    jobs: Option<Dir>,
}

impl FilesystemSessionStore {
    /// Takes a shared hold on `session_id`'s lifetime and opens its
    /// directory and, if present (or `create`), its `jobs/`.
    fn session_jobs_directory(
        &self,
        session_id: &SessionId,
        create: bool,
    ) -> Result<Option<SessionJobs>, JobStoreError> {
        self.revalidate_root()?;
        let coordination = self
            .root
            .open_dir_nofollow(COORDINATION_DIRECTORY)
            .map_err(map_storage_io)?;
        let lifetime =
            HeldFileLock::try_shared(open_session_lock(&coordination, session_id, "lifetime")?)
                .map_err(map_lock_error)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        if !sessions
            .try_exists(session_id.as_str())
            .map_err(map_storage_io)?
        {
            return Ok(None);
        }
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let exists = session.try_exists(JOBS_DIRECTORY).map_err(map_storage_io)?;
        if !exists && create {
            create_directory(&session, JOBS_DIRECTORY)?;
        }
        let jobs = if exists || create {
            Some(
                session
                    .open_dir_nofollow(JOBS_DIRECTORY)
                    .map_err(|_| SessionStorageError::IntegrityFailure)?,
            )
        } else {
            None
        };
        Ok(Some(SessionJobs {
            lifetime,
            session,
            jobs,
        }))
    }

    /// The durability of an open, unexpired session, which job files follow.
    fn open_session_durability(
        &self,
        session: &Dir,
        session_id: &SessionId,
        now: Option<u64>,
    ) -> Result<StoredDurability, SessionStorageError> {
        let committed = read_committed_manifest(session, session_id, self.chain_check())?;
        if let Some(now) = now {
            let record = committed
                .manifest
                .lifecycle
                .as_ref()
                .ok_or(SessionStorageError::StateConflict)?;
            let status = record.to_status(session_id.clone(), committed.manifest.generation)?;
            if status.phase() != SessionPhase::Open || status.lifetime().expired(now) {
                return Err(SessionStorageError::StateConflict);
            }
        }
        Ok(committed.manifest.durability)
    }

    /// Takes the owner lock of an existing job directory and reads its record.
    fn own(
        &self,
        opened: SessionJobs,
        session_id: &SessionId,
        job_id: &JobId,
        spec: Option<(&JobSpec, u64)>,
    ) -> Result<(FilesystemJobOwner, bool), JobStoreError> {
        let jobs = opened.jobs.ok_or(JobStoreError::NotFound)?;
        let durability =
            self.open_session_durability(&opened.session, session_id, spec.map(|(_, now)| now))?;
        let plan = FaultPlan::from_environment();
        if spec.is_some() && !jobs.try_exists(job_id.as_str()).map_err(map_storage_io)? {
            self.make_room(&jobs)?;
            match create_private_child_directory(&jobs, Path::new(job_id.as_str())) {
                Ok(()) => sync_if_durable(durability, &jobs)?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(map_storage_io(error).into()),
            }
        }
        if !jobs.try_exists(job_id.as_str()).map_err(map_storage_io)? {
            return Err(JobStoreError::NotFound);
        }
        let job = jobs
            .open_dir_nofollow(job_id.as_str())
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let owner_file = if spec.is_some() {
            create_or_open(&job, OWNER_LOCK_FILE)?
        } else {
            open_regular_file(&job, OWNER_LOCK_FILE, true).map_err(|_| JobStoreError::NotFound)?
        };
        let owner = HeldFileLock::try_exclusive(owner_file.into_std()).map_err(map_lock_error)?;
        if spec.is_some() {
            drop(create_or_open(&job, STATE_LOCK_FILE)?);
        }
        let (record, created) = with_state_lock(&job, || {
            match (read_record(&job, job_id, session_id)?, spec) {
                (Some(record), Some((spec, _))) => {
                    if record.request_digest != spec.request_digest
                        || record.operation_key != spec.operation_key
                        || record.recognition_key != spec.recognition_key
                        || record.request != spec.request
                    {
                        return Err(SessionStorageError::IntegrityFailure.into());
                    }
                    // A creation that stopped before its index entry.
                    self.index_job(job_id, session_id, durability)?;
                    Ok((record, false))
                }
                (Some(record), None) => Ok((record, false)),
                (None, Some((spec, now))) => {
                    let record = JobRecord::queued(spec, now);
                    write_record(&job, &record, durability)?;
                    plan.reach(FaultPoint::JobCreate);
                    self.index_job(job_id, session_id, durability)?;
                    plan.reach(FaultPoint::JobIndex);
                    Ok((record, true))
                }
                (None, None) => Err(JobStoreError::NotFound),
            }
        })?;
        if !job.try_exists(CHUNKS_DIRECTORY).map_err(map_storage_io)? {
            create_directory(&job, CHUNKS_DIRECTORY)?;
            sync_if_durable(durability, &job)?;
        }
        let chunks = job
            .open_dir_nofollow(CHUNKS_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        Ok((
            FilesystemJobOwner {
                session_id: session_id.clone(),
                jobs,
                job,
                chunks,
                durability,
                plan,
                record,
                _owner: owner,
                _lifetime: opened.lifetime,
            },
            created,
        ))
    }

    /// Makes room for one more job: at [`MAX_SESSION_JOBS`] the oldest ended
    /// job that no process owns and no caller-supplied operation id pins is
    /// removed with its index entry and its commit's own binding.
    fn make_room(&self, jobs: &Dir) -> Result<(), JobStoreError> {
        let names = job_names(jobs)?;
        if names.len() < MAX_SESSION_JOBS {
            return Ok(());
        }
        let mut oldest: Option<(u64, JobRecord)> = None;
        for name in names {
            let Ok(job_id) = JobId::parse(name.as_str()) else {
                continue;
            };
            let Ok(job) = jobs.open_dir_nofollow(job_id.as_str()) else {
                continue;
            };
            let Ok(Some(record)) = read_record_unverified_session(&job, &job_id) else {
                continue;
            };
            if !record.state.is_terminal()
                || !record.operation_ids.is_empty()
                || liveness(&job)? == JobLiveness::Owned
            {
                continue;
            }
            if oldest
                .as_ref()
                .is_none_or(|(updated, _)| record.updated_at_unix_seconds < *updated)
            {
                oldest = Some((record.updated_at_unix_seconds, record));
            }
        }
        let (_, record) = oldest.ok_or(SessionStorageError::CapacityExhausted)?;
        if let Some(commit) = &record.commit {
            remove_binding_if(jobs, &commit.operation_id, &record.job_id);
        }
        jobs.remove_dir_all(record.job_id.as_str())
            .map_err(map_storage_io)?;
        self.unindex_job(&record.job_id);
        Ok(())
    }

    /// Writes the root index entry of a job, unless it already names the session.
    fn index_job(
        &self,
        job_id: &JobId,
        session_id: &SessionId,
        durability: StoredDurability,
    ) -> Result<(), SessionStorageError> {
        if !self
            .root
            .try_exists(JOB_INDEX_DIRECTORY)
            .map_err(map_storage_io)?
        {
            create_directory(&self.root, JOB_INDEX_DIRECTORY)?;
        }
        let index = self
            .root
            .open_dir_nofollow(JOB_INDEX_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let bucket_name = job_bucket(job_id);
        if !index.try_exists(&bucket_name).map_err(map_storage_io)? {
            create_directory(&index, &bucket_name)?;
            sync_if_durable(durability, &index)?;
        }
        let bucket = index
            .open_dir_nofollow(&bucket_name)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let name = format!("{}.json", job_id.as_str());
        if bucket.try_exists(&name).map_err(map_storage_io)?
            && read_versioned_json_file::<StoredJobIndex>(&bucket, &name)
                .ok()
                .and_then(|entry| entry.session())
                .as_ref()
                == Some(session_id)
        {
            return Ok(());
        }
        let bytes = serde_json::to_vec(&StoredJobIndex::new(session_id))
            .map_err(|_| SessionStorageError::Io)?;
        replace_file(
            &bucket,
            &name,
            &format!(".{}.tmp", job_id.as_str()),
            &bytes,
            durability,
            None,
        )
    }

    /// Removes a job's root index entry, best effort.
    fn unindex_job(&self, job_id: &JobId) {
        let _ = self
            .root
            .open_dir_nofollow(JOB_INDEX_DIRECTORY)
            .and_then(|index| index.open_dir_nofollow(job_bucket(job_id)))
            .and_then(|bucket| bucket.remove_file(format!("{}.json", job_id.as_str())));
    }

    /// Removes the index entries of every job of a session directory about
    /// to be removed, best effort (`clean_session`).
    pub(super) fn unindex_session_jobs(&self, session: &Dir) {
        let Ok(jobs) = session.open_dir_nofollow(JOBS_DIRECTORY) else {
            return;
        };
        let Ok(names) = job_names(&jobs) else {
            return;
        };
        for name in names {
            if let Ok(job_id) = JobId::parse(name) {
                self.unindex_job(&job_id);
            }
        }
    }

    /// One job's view, from an opened `jobs/` directory.
    fn view(
        jobs: &Dir,
        session_id: &SessionId,
        job_id: &JobId,
    ) -> Result<Option<JobView>, JobStoreError> {
        if !jobs.try_exists(job_id.as_str()).map_err(map_storage_io)? {
            return Ok(None);
        }
        let job = jobs
            .open_dir_nofollow(job_id.as_str())
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let Some(record) = read_record(&job, job_id, session_id)? else {
            return Ok(None);
        };
        Ok(Some(JobView {
            record,
            liveness: liveness(&job)?,
            checkpoints: count_checkpoints(&job)?,
        }))
    }
}

impl JobStore for FilesystemSessionStore {
    type Owner = FilesystemJobOwner;

    fn job(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
    ) -> Result<Option<JobView>, JobStoreError> {
        let Some(opened) = self.session_jobs_directory(session_id, false)? else {
            return Ok(None);
        };
        let Some(jobs) = &opened.jobs else {
            return Ok(None);
        };
        Self::view(jobs, session_id, job_id)
    }

    fn job_session(&self, job_id: &JobId) -> Result<Option<SessionId>, JobStoreError> {
        self.revalidate_root()?;
        if !self
            .root
            .try_exists(JOB_INDEX_DIRECTORY)
            .map_err(map_storage_io)?
        {
            return Ok(None);
        }
        let index = self
            .root
            .open_dir_nofollow(JOB_INDEX_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let bucket_name = job_bucket(job_id);
        if !index.try_exists(&bucket_name).map_err(map_storage_io)? {
            return Ok(None);
        }
        let bucket = index
            .open_dir_nofollow(&bucket_name)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let name = format!("{}.json", job_id.as_str());
        if !bucket.try_exists(&name).map_err(map_storage_io)? {
            return Ok(None);
        }
        let entry = read_versioned_json_file::<StoredJobIndex>(&bucket, &name)?;
        entry
            .session()
            .map(Some)
            .ok_or(SessionStorageError::IntegrityFailure.into())
    }

    fn job_for_operation(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
    ) -> Result<Option<JobId>, JobStoreError> {
        let Some(opened) = self.session_jobs_directory(session_id, false)? else {
            return Ok(None);
        };
        let Some(jobs) = &opened.jobs else {
            return Ok(None);
        };
        if !jobs
            .try_exists(BY_OPERATION_DIRECTORY)
            .map_err(map_storage_io)?
        {
            return Ok(None);
        }
        let bindings = jobs
            .open_dir_nofollow(BY_OPERATION_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let name = binding_name(operation_id);
        if !bindings.try_exists(&name).map_err(map_storage_io)? {
            return Ok(None);
        }
        let binding = read_versioned_json_file::<StoredBinding>(&bindings, &name)?;
        binding
            .job(operation_id)
            .map(Some)
            .ok_or(SessionStorageError::IntegrityFailure.into())
    }

    fn session_jobs(&self, session_id: &SessionId) -> Result<Vec<JobView>, JobStoreError> {
        let Some(opened) = self.session_jobs_directory(session_id, false)? else {
            return Ok(Vec::new());
        };
        let Some(jobs) = &opened.jobs else {
            return Ok(Vec::new());
        };
        let mut views = Vec::new();
        for name in job_names(jobs)? {
            let job_id = JobId::parse(name).map_err(|_| SessionStorageError::IntegrityFailure)?;
            if let Some(view) = Self::view(jobs, session_id, &job_id)? {
                views.push(view);
            }
        }
        views.sort_by(|left, right| left.record.job_id.cmp(&right.record.job_id));
        Ok(views)
    }

    fn open_or_create(
        &self,
        spec: &JobSpec,
        now: u64,
    ) -> Result<(Self::Owner, bool), JobStoreError> {
        let opened = self
            .session_jobs_directory(&spec.session_id, true)?
            .ok_or(SessionStorageError::StateConflict)?;
        self.own(opened, &spec.session_id, &spec.job_id, Some((spec, now)))
    }

    fn acquire(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
    ) -> Result<Self::Owner, JobStoreError> {
        let opened = self
            .session_jobs_directory(session_id, false)?
            .ok_or(JobStoreError::NotFound)?;
        self.own(opened, session_id, job_id, None)
            .map(|(owner, _)| owner)
    }

    fn request_cancel(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
        now: u64,
    ) -> Result<CancelRequest, JobStoreError> {
        let opened = self
            .session_jobs_directory(session_id, false)?
            .ok_or(JobStoreError::NotFound)?;
        let jobs = opened.jobs.as_ref().ok_or(JobStoreError::NotFound)?;
        if !jobs.try_exists(job_id.as_str()).map_err(map_storage_io)? {
            return Err(JobStoreError::NotFound);
        }
        let job = jobs
            .open_dir_nofollow(job_id.as_str())
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let durability = self.open_session_durability(&opened.session, session_id, None)?;
        with_state_lock(&job, || {
            let record = read_record(&job, job_id, session_id)?.ok_or(JobStoreError::NotFound)?;
            if record.state.is_terminal() {
                return Ok(CancelRequest::Terminal(record.state));
            }
            if liveness(&job)? == JobLiveness::Unowned {
                return Ok(CancelRequest::NotLive);
            }
            match record.state {
                JobState::Committing => Ok(CancelRequest::TooLate),
                JobState::Cancelling => Ok(CancelRequest::AlreadyRequested),
                JobState::Queued | JobState::Running => {
                    let next = record.changed(&JobChange::RequestCancel, now)?;
                    write_record(&job, &next, durability)?;
                    Ok(CancelRequest::Requested)
                }
                // An owner that holds an interrupted job is about to start
                // or end it; ask again in a moment.
                JobState::Interrupted
                | JobState::Succeeded
                | JobState::Failed
                | JobState::Cancelled => Err(SessionStorageError::Busy.into()),
            }
        })
    }
}

impl FilesystemJobOwner {
    fn checkpoint_path(index: u32) -> Option<(u32, String)> {
        let ordinal = checkpoint_ordinal(index)?;
        Some((ordinal, checkpoint_name(ordinal)))
    }

    /// Removes every checkpoint; the first removal passes
    /// [`FaultPoint::CheckpointDeletion`].
    fn delete_checkpoints(&self) -> Result<(), SessionStorageError> {
        let mut first = true;
        for name in entry_names(&self.chunks)? {
            self.chunks.remove_file(&name).map_err(map_storage_io)?;
            if first {
                self.plan.reach(FaultPoint::CheckpointDeletion);
                first = false;
            }
        }
        sync_if_durable(self.durability, &self.chunks)
    }
}

impl ChunkCheckpoints for FilesystemJobOwner {
    fn load(&self, index: u32) -> CheckpointRead {
        let Some((ordinal, name)) = Self::checkpoint_path(index) else {
            return CheckpointRead::Absent;
        };
        match self.chunks.try_exists(&name) {
            Ok(true) => {}
            Ok(false) | Err(_) => return CheckpointRead::Absent,
        }
        let decoded = open_regular_file(&self.chunks, &name, false)
            .ok()
            .and_then(|file| read_bounded_to(file, MAX_CHECKPOINT_BYTES).ok())
            .and_then(|bytes| decode_checkpoint(&bytes, ordinal));
        if let Some(checkpoint) = decoded {
            CheckpointRead::Found(checkpoint)
        } else {
            // Never guessed at: removed and done again (S-08).
            let _ = self.chunks.remove_file(&name);
            CheckpointRead::Unusable
        }
    }

    fn store(&self, checkpoint: &ChunkCheckpoint) -> Result<(), CheckpointStoreError> {
        self.plan.reach(FaultPoint::ChunkRecognised);
        let (ordinal, name) =
            Self::checkpoint_path(checkpoint.index()).ok_or(CheckpointStoreError::TooMany)?;
        if usize::try_from(ordinal).map_or(true, |ordinal| ordinal > MAX_PLANNED_CHUNKS) {
            return Err(CheckpointStoreError::TooMany);
        }
        let bytes = encode_checkpoint(checkpoint)
            .ok_or(CheckpointStoreError::Storage(SessionStorageError::Io))?;
        if u64::try_from(bytes.len()).map_or(true, |size| size > MAX_CHECKPOINT_BYTES) {
            return Err(CheckpointStoreError::TooLarge);
        }
        replace_file(
            &self.chunks,
            &name,
            &format!(".{ordinal:05}.tmp"),
            &bytes,
            self.durability,
            Some((&self.plan, CHECKPOINT_POINTS)),
        )
        .map_err(CheckpointStoreError::Storage)
    }

    fn discard(&self, index: u32) {
        if let Some((_, name)) = Self::checkpoint_path(index) {
            let _ = self.chunks.remove_file(name);
        }
    }
}

/// The fault points of a checkpoint's write, flush and rename.
const CHECKPOINT_POINTS: [FaultPoint; 3] = [
    FaultPoint::CheckpointWrite,
    FaultPoint::CheckpointFlush,
    FaultPoint::CheckpointRename,
];

impl JobOwner for FilesystemJobOwner {
    fn record(&self) -> &JobRecord {
        &self.record
    }

    fn apply(&mut self, change: &JobChange, now: u64) -> Result<(), JobStoreError> {
        let next = with_state_lock(&self.job, || {
            let stored = read_record(&self.job, &self.record.job_id, &self.session_id)?
                .ok_or(SessionStorageError::IntegrityFailure)?;
            if stored.epoch != self.record.epoch || stored.attempt != self.record.attempt {
                return Err(JobStoreError::StaleOwner);
            }
            let next = stored.changed(change, now)?;
            write_record(&self.job, &next, self.durability)?;
            match change {
                JobChange::Commit(_) => self.plan.reach(FaultPoint::JobCommitting),
                JobChange::Succeed => self.plan.reach(FaultPoint::JobSucceeded),
                JobChange::Start
                | JobChange::Interrupt(_)
                | JobChange::Fail(_)
                | JobChange::RequestCancel
                | JobChange::Cancel
                | JobChange::Restart => {}
            }
            Ok(next)
        })?;
        self.record = next;
        if change.discards_checkpoints() {
            // The record already says the checkpoints are unused; failing to
            // remove one leaves a file the next restart or cleanup removes.
            let _ = self.delete_checkpoints();
        }
        Ok(())
    }

    fn bind_operation(
        &mut self,
        operation_id: &OperationId,
        now: u64,
    ) -> Result<(), JobStoreError> {
        if !self
            .jobs
            .try_exists(BY_OPERATION_DIRECTORY)
            .map_err(map_storage_io)?
        {
            create_directory(&self.jobs, BY_OPERATION_DIRECTORY)?;
            sync_if_durable(self.durability, &self.jobs)?;
        }
        let bindings = self
            .jobs
            .open_dir_nofollow(BY_OPERATION_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let name = binding_name(operation_id);
        if !bindings.try_exists(&name).map_err(map_storage_io)?
            && entry_names(&bindings)?.len() >= MAX_SESSION_BINDINGS
        {
            return Err(SessionStorageError::CapacityExhausted.into());
        }
        let next = with_state_lock(&self.job, || {
            let mut stored = read_record(&self.job, &self.record.job_id, &self.session_id)?
                .ok_or(SessionStorageError::IntegrityFailure)?;
            if stored.epoch != self.record.epoch || stored.attempt != self.record.attempt {
                return Err(JobStoreError::StaleOwner);
            }
            // The commit's own id is an alias of the job's result and pins
            // nothing; a caller's id keeps the job from being pruned.
            let alias = stored
                .commit
                .as_ref()
                .is_some_and(|commit| commit.operation_id == *operation_id);
            if !alias && !stored.operation_ids.contains(operation_id) {
                if stored.operation_ids.len() >= vsift_application::MAX_JOB_OPERATION_IDS {
                    return Err(SessionStorageError::CapacityExhausted.into());
                }
                stored.operation_ids.push(operation_id.clone());
                stored.updated_at_unix_seconds = now.max(stored.updated_at_unix_seconds);
                write_record(&self.job, &stored, self.durability)?;
            }
            let bytes = serde_json::to_vec(&StoredBinding::new(operation_id, &stored.job_id))
                .map_err(|_| SessionStorageError::Io)?;
            replace_file(
                &bindings,
                &name,
                &format!(".{}.tmp", operation_id.as_str()),
                &bytes,
                self.durability,
                None,
            )?;
            Ok(stored)
        })?;
        self.record = next;
        Ok(())
    }

    fn cancel_requested(&self) -> bool {
        read_record(&self.job, &self.record.job_id, &self.session_id)
            .ok()
            .flatten()
            .is_some_and(|record| record.state == JobState::Cancelling)
    }
}

impl CommitLedger for FilesystemSessionStore {
    fn committed_generation(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        above: StorageGeneration,
    ) -> Result<Option<StorageGeneration>, SessionStorageError> {
        let _hold = self.acquire_read(session_id)?;
        let session = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?
            .open_dir_nofollow(session_id.as_str())
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let head = read_committed_manifest(&session, session_id, self.chain_check())?;
        let generations = session
            .open_dir_nofollow(GENERATIONS_DIRECTORY)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        // Walk down from the verified head along the digest links, so every
        // generation looked at is one the head's chain fixes.
        let mut manifest = head.manifest;
        while manifest.generation > above.value() {
            if manifest.operation_id == operation_id.as_str() {
                return Ok(Some(StorageGeneration::from_value(manifest.generation)));
            }
            let expected = manifest
                .previous_manifest_sha256
                .clone()
                .ok_or(SessionStorageError::IntegrityFailure)?;
            let previous = manifest
                .generation
                .checked_sub(1)
                .ok_or(SessionStorageError::IntegrityFailure)?;
            let file = open_regular_file(&generations, &format!("{previous}.json"), false)
                .map_err(|_| SessionStorageError::IntegrityFailure)?;
            let bytes =
                read_bounded_manifest(file).map_err(|_| SessionStorageError::IntegrityFailure)?;
            if sha256_hex(&bytes) != expected {
                return Err(SessionStorageError::IntegrityFailure);
            }
            let next: GenerationManifest = parse_versioned_json(&bytes)?;
            if next.generation != previous || next.session_id != session_id.as_str() {
                return Err(SessionStorageError::IntegrityFailure);
            }
            manifest = next;
        }
        Ok(None)
    }
}

impl RevisionStore for FilesystemSessionStore {
    type Permit = FilesystemAdmissionPermit;

    fn admit(&self) -> Result<Self::Permit, SessionStorageError> {
        self.try_admit(1)
    }

    fn head(&self, session_id: &SessionId, now: u64) -> Result<SessionHead, SessionStorageError> {
        // One manifest: a revision committed between two reads would pair a
        // head without it with a generation that holds it.
        let (newest, status) = self.read_transcript_head(session_id, now)?;
        Ok(SessionHead {
            generation: status.generation(),
            newest,
        })
    }

    fn revision(
        &self,
        session_id: &SessionId,
        revision_id: &TranscriptRevisionId,
        now: u64,
    ) -> Result<Option<TranscriptRevision>, SessionStorageError> {
        Ok(self
            .read_transcript_revision(session_id, revision_id, now)?
            .map(|(revision, _)| revision))
    }

    fn publish_revision(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        expected: StorageGeneration,
        revision: &TranscriptRevision,
        now: u64,
    ) -> Result<StorageGeneration, SessionStorageError> {
        let record = encode_transcript_record(revision)?;
        self.publish_artifact(
            session_id,
            operation_id,
            expected,
            SessionArtifactKind::TranscriptRecord,
            &record,
            now,
        )
    }
}

/// Reads a job record, or `None` when the job directory holds none yet (a
/// creation that stopped before its record).
///
/// The record is replaced by rename under the state lock while other
/// processes (status, cancel requests, the owner's cancellation check) read
/// it without that lock, so it is opened with [`open_replaced_file`]: a
/// reader that meets a replacement retries rather than reporting the job
/// missing or damaged.
fn read_record(
    job: &Dir,
    job_id: &JobId,
    session_id: &SessionId,
) -> Result<Option<JobRecord>, JobStoreError> {
    let Some(file) = open_record(job)? else {
        return Ok(None);
    };
    let bytes = read_bounded(file).map_err(|_| SessionStorageError::IntegrityFailure)?;
    decode_job(&bytes, job_id, session_id)
        .map(Some)
        .map_err(Into::into)
}

/// Reads a record whose session is not known (pruning): the session it names
/// is taken from the record itself and checked by the key derivation.
fn read_record_unverified_session(
    job: &Dir,
    job_id: &JobId,
) -> Result<Option<JobRecord>, JobStoreError> {
    #[derive(serde::Deserialize)]
    struct SessionProbe {
        session_id: String,
    }

    let Some(file) = open_record(job)? else {
        return Ok(None);
    };
    let bytes = read_bounded(file).map_err(|_| SessionStorageError::IntegrityFailure)?;
    let probe: SessionProbe =
        serde_json::from_slice(&bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    let session_id =
        SessionId::parse(probe.session_id).map_err(|_| SessionStorageError::IntegrityFailure)?;
    decode_job(&bytes, job_id, &session_id)
        .map(Some)
        .map_err(Into::into)
}

/// Opens `job.json`, or `None` when it does not exist.
fn open_record(job: &Dir) -> Result<Option<cap_std::fs::File>, JobStoreError> {
    match open_replaced_file(job, JOB_FILE, false) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(SessionStorageError::IntegrityFailure.into()),
    }
}

/// Replaces `job.json` atomically.
fn write_record(
    job: &Dir,
    record: &JobRecord,
    durability: StoredDurability,
) -> Result<(), SessionStorageError> {
    let bytes = encode_job(record)?;
    replace_file(job, JOB_FILE, JOB_STAGED_FILE, &bytes, durability, None)
}

/// Writes `bytes` to `staged` (created new, after removing a leftover),
/// flushes it, renames it over `name` and, for a durable session,
/// synchronises the directory. `points` names the write, flush and rename
/// fault points and the plan that counts them.
fn replace_file(
    directory: &Dir,
    name: &str,
    staged: &str,
    bytes: &[u8],
    durability: StoredDurability,
    points: Option<(&FaultPlan, [FaultPoint; 3])>,
) -> Result<(), SessionStorageError> {
    let reach = |index: usize| {
        if let Some((plan, points)) = points {
            plan.reach(points[index]);
        }
    };
    if directory.try_exists(staged).map_err(map_storage_io)? {
        directory.remove_file(staged).map_err(map_storage_io)?;
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = directory
        .open_with(staged, &options)
        .map_err(map_storage_io)?;
    file.write_all(bytes).map_err(map_storage_io)?;
    reach(0);
    file.sync_all().map_err(map_storage_io)?;
    drop(file);
    reach(1);
    directory
        .rename(staged, directory, name)
        .map_err(map_storage_io)?;
    reach(2);
    sync_if_durable(durability, directory)
}

fn sync_if_durable(
    durability: StoredDurability,
    directory: &Dir,
) -> Result<(), SessionStorageError> {
    if matches!(durability, StoredDurability::Durable) {
        sync_directory(directory).map_err(map_storage_io)?;
    }
    Ok(())
}

/// Takes the job's state lock, retrying briefly, runs `action` and releases
/// the lock before returning.
fn with_state_lock<T>(
    job: &Dir,
    action: impl FnOnce() -> Result<T, JobStoreError>,
) -> Result<T, JobStoreError> {
    let mut attempts = 0;
    let lock = loop {
        let file = open_regular_file(job, STATE_LOCK_FILE, true)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        match HeldFileLock::try_exclusive(file.into_std()) {
            Ok(lock) => break lock,
            Err(fs::TryLockError::WouldBlock) if attempts < STATE_LOCK_ATTEMPTS => {
                attempts += 1;
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(map_lock_error(error).into()),
        }
    };
    let result = action();
    lock.release().map_err(map_storage_io)?;
    result
}

/// Whether a process holds the job's owner lock right now. The probe takes
/// a shared lock for an instant, so an acquisition racing with it may be
/// told busy, which is safe and retryable.
fn liveness(job: &Dir) -> Result<JobLiveness, SessionStorageError> {
    if !job.try_exists(OWNER_LOCK_FILE).map_err(map_storage_io)? {
        return Ok(JobLiveness::Unowned);
    }
    let file = open_regular_file(job, OWNER_LOCK_FILE, true)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    match HeldFileLock::try_shared(file.into_std()) {
        Ok(probe) => {
            probe.release().map_err(map_storage_io)?;
            Ok(JobLiveness::Unowned)
        }
        Err(fs::TryLockError::WouldBlock) => Ok(JobLiveness::Owned),
        Err(fs::TryLockError::Error(error)) => Err(map_storage_io(error)),
    }
}

fn count_checkpoints(job: &Dir) -> Result<usize, SessionStorageError> {
    if !job.try_exists(CHUNKS_DIRECTORY).map_err(map_storage_io)? {
        return Ok(0);
    }
    let chunks = job
        .open_dir_nofollow(CHUNKS_DIRECTORY)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    Ok(entry_names(&chunks)?
        .iter()
        .filter(|name| {
            name.strip_suffix(".json").is_some_and(|ordinal| {
                ordinal.len() == 5 && ordinal.bytes().all(|b| b.is_ascii_digit())
            })
        })
        .count())
}

/// The names in a directory, at most [`MAX_SCANNED_ENTRIES`].
fn entry_names(directory: &Dir) -> Result<Vec<String>, SessionStorageError> {
    let mut names = Vec::new();
    for entry in directory.entries().map_err(map_storage_io)? {
        let entry = entry.map_err(map_storage_io)?;
        if names.len() >= MAX_SCANNED_ENTRIES {
            return Err(SessionStorageError::CapacityExhausted);
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        names.push(name);
    }
    Ok(names)
}

/// The job directory names in `jobs/`; anything but jobs and the binding
/// directory is an integrity failure.
fn job_names(jobs: &Dir) -> Result<Vec<String>, SessionStorageError> {
    let mut names = Vec::new();
    for name in entry_names(jobs)? {
        if name == BY_OPERATION_DIRECTORY {
            continue;
        }
        JobId::parse(name.as_str()).map_err(|_| SessionStorageError::IntegrityFailure)?;
        names.push(name);
    }
    names.sort();
    Ok(names)
}

fn binding_name(operation_id: &OperationId) -> String {
    format!("{}.json", operation_id.as_str())
}

/// Removes the binding of `operation_id` if it still points at `job_id`.
fn remove_binding_if(jobs: &Dir, operation_id: &OperationId, job_id: &JobId) {
    let Ok(bindings) = jobs.open_dir_nofollow(BY_OPERATION_DIRECTORY) else {
        return;
    };
    let name = binding_name(operation_id);
    let bound = read_versioned_json_file::<StoredBinding>(&bindings, &name)
        .ok()
        .and_then(|binding| binding.job(operation_id));
    if bound.as_ref() == Some(job_id) {
        let _ = bindings.remove_file(name);
    }
}

fn job_bucket(job_id: &JobId) -> String {
    let digest = Sha256::digest(job_id.as_str().as_bytes());
    format!("{:02x}", digest[0])
}

fn create_directory(parent: &Dir, name: &str) -> Result<(), SessionStorageError> {
    match create_private_child_directory(parent, Path::new(name)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(map_storage_io(error)),
    }
}

/// Opens a lock anchor, creating it empty if it does not exist yet.
fn create_or_open(directory: &Dir, name: &str) -> Result<cap_std::fs::File, SessionStorageError> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .follow(FollowSymlinks::No);
    let file = directory
        .open_with(name, &options)
        .map_err(map_storage_io)?;
    let metadata = file.metadata().map_err(map_storage_io)?;
    if !metadata.is_file() || !super::has_one_link(&metadata) {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(file)
}
