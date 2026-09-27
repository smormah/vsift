//! Recoverable jobs: operation keys, the job and checkpoint ports, and the
//! use cases that decide how a repeated request continues (P10, ADR 0020).
//!
//! A long operation runs as a *job* whose identity is derived from what it
//! computes, so running the same request again after a crash finds the same
//! job and continues it from its chunk checkpoints instead of starting over.
//! `transcript retranscribe` is the first such operation.
//!
//! **Keys.** Four digests identify a retranscription, each with its own
//! domain-separation prefix:
//!
//! - the *request digest* ([`retranscribe_request_digest`]) is what the caller
//!   asked for: the session, the command and its canonical parameters (the
//!   range, or none). An operation id is bound to it, so the same id with a
//!   different request is an [`JobRunError::IdempotencyConflict`];
//! - the *recognition key* ([`recognition_key`]) is everything that decides
//!   what the recognizer hears and how: session, source, audio stream,
//!   replaced range, chunk plan, decoding profile, recognizer identity and
//!   the local-ASR verification fingerprint. It leaves out the revision the
//!   run supersedes, so chunk checkpoints stay valid when only the base
//!   revision changes;
//! - the *operation key* ([`retranscribe_operation_key`], `opk_sha256_...`) is
//!   the recognition key and the base revision: the whole result;
//! - the *job id* ([`job_id`], `job_` and 32 hex digits) derives from the
//!   session and the operation key, so an identical request lands on the same
//!   job.
//!
//! **Liveness.** The operating-system lock on a job is the only authority on
//! whether it is live: a job whose record says it is running or committing
//! but whose lock can be taken was interrupted. There are no heartbeats,
//! timestamps or process ids to misjudge, and a suspended owner keeps its
//! lock (X-05).
//!
//! **Exactly-once commit.** Before publishing, a job records `committing` with
//! a deterministic commit operation id ([`commit_operation_id`]) and the
//! generation it observed. Recovery reconciles one way only ([`reconcile`]):
//! if the manifest chain holds that operation above that generation, the job
//! succeeded, whatever its record says; the chain is the source of truth.

mod run;
#[cfg(test)]
mod tests;

use std::{error::Error, fmt, time::Duration};

use vsift_domain::{
    AttemptFailure, ChunkCheckpoint, ChunkPlan, JobId, JobKind, JobState, JobTransitionError,
    OperationId, OperationKey, RecognitionKey, SessionId, Sha256Hex, SourceId, StorageGeneration,
    TimeRange, TranscriptRevisionId,
};

use crate::{
    MediaToolFingerprint, RecognizerIdentity, SessionStorageError,
    transcript::{derived_identity, sha256_hex},
};

pub use run::{
    CommitGuard, JobReport, JobRunError, RetranscriptionOutcome, RetranscriptionPorts,
    RetranscriptionRun, RetryTimer, RevisionStore, SessionHead, run_retranscription,
};

/// Command name bound into a retranscription's request digest.
const RETRANSCRIBE_COMMAND: &str = "transcript.retranscribe";
/// Hex digits of a derived job or commit operation identity.
const JOB_IDENTITY_HEX: usize = 32;
/// Most operation ids one job answers to.
pub const MAX_JOB_OPERATION_IDS: usize = 8;
/// Most jobs one session keeps.
pub const MAX_SESSION_JOBS: usize = 64;
/// Most recorded failures one job keeps: one per attempt.
pub const MAX_RECORDED_FAILURES: usize = vsift_domain::MAX_JOB_ATTEMPTS as usize;
/// How long a caller should wait before asking again about a live job.
pub const LIVE_JOB_RETRY_AFTER: Duration = vsift_domain::RETRY_MAX_DELAY;

/// Why a key could not be derived; only possible through an internal fault.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobKeyError {
    /// A derived value was not in canonical form.
    NotCanonical,
}

impl fmt::Display for JobKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a derived job key is not canonical")
    }
}

impl Error for JobKeyError {}

/// Digest of one public request: the session, the command and its canonical
/// parameters. An operation id is bound to it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestDigest(Sha256Hex);

impl RequestDigest {
    /// Wraps a stored digest.
    #[must_use]
    pub const fn new(digest: Sha256Hex) -> Self {
        Self(digest)
    }

    /// The lowercase hexadecimal digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

fn digest_lines(domain: &str, lines: &[&str]) -> Result<Sha256Hex, JobKeyError> {
    let mut material = String::from(domain);
    for line in lines {
        material.push('\n');
        material.push_str(line);
    }
    Sha256Hex::parse(sha256_hex(material.as_bytes())).map_err(|_| JobKeyError::NotCanonical)
}

fn range_text(range: Option<TimeRange>) -> String {
    range.map_or_else(
        || "range=none".to_owned(),
        |range| {
            format!(
                "range={}-{}",
                range.start().as_micros(),
                range.end().as_micros()
            )
        },
    )
}

/// The request digest of `transcript retranscribe` (`vsift.retranscribe-request.v1`).
///
/// # Errors
///
/// [`JobKeyError::NotCanonical`] only through an internal fault.
pub fn retranscribe_request_digest(
    session_id: &SessionId,
    range: Option<TimeRange>,
) -> Result<RequestDigest, JobKeyError> {
    digest_lines(
        "vsift.retranscribe-request.v1",
        &[
            session_id.as_str(),
            RETRANSCRIBE_COMMAND,
            &range_text(range),
        ],
    )
    .map(RequestDigest)
}

/// Everything that decides what one recognition run hears and how.
#[derive(Clone, Copy, Debug)]
pub struct RecognitionScope<'a> {
    /// Session the run belongs to.
    pub session_id: &'a SessionId,
    /// Source the run decodes.
    pub source_id: &'a SourceId,
    /// Original index of the decoded audio stream.
    pub audio_stream: u32,
    /// The range the run recognises: the request widened to whole segments.
    pub replaced_range: TimeRange,
    /// How the range is cut into chunks.
    pub plan: ChunkPlan,
    /// Provider build, model, decoding profile and threads.
    pub recognizer: &'a RecognizerIdentity,
    /// The local-ASR verification fingerprint of the setup, if one could be
    /// derived; it binds the tools, isolation, verifier and `VSift` version.
    pub verification: Option<&'a MediaToolFingerprint>,
}

/// The recognition key of `scope` (`vsift.recognition.v1`).
///
/// # Errors
///
/// [`JobKeyError::NotCanonical`] only through an internal fault.
pub fn recognition_key(scope: &RecognitionScope<'_>) -> Result<RecognitionKey, JobKeyError> {
    let recognizer = scope.recognizer;
    let fingerprint = scope.verification.map_or_else(
        || "none".to_owned(),
        |fingerprint| sha256_hex_of_digest(fingerprint.digest()),
    );
    digest_lines(
        "vsift.recognition.v1",
        &[
            scope.session_id.as_str(),
            scope.source_id.as_str(),
            &scope.audio_stream.to_string(),
            &scope.replaced_range.start().as_micros().to_string(),
            &scope.replaced_range.end().as_micros().to_string(),
            &scope.plan.window_us().to_string(),
            &scope.plan.overlap_us().to_string(),
            recognizer.decoding.identifier(),
            recognizer.provider.provider().identifier(),
            recognizer.provider.executable_sha256().as_str(),
            recognizer.model.profile().identifier(),
            recognizer.model.sha256().as_str(),
            &recognizer.threads.get().to_string(),
            &fingerprint,
        ],
    )
    .map(RecognitionKey::new)
}

fn sha256_hex_of_digest(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    text
}

/// The operation key of a retranscription (`vsift.retranscribe-op.v1`): its
/// recognition key and the revision it supersedes.
///
/// # Errors
///
/// [`JobKeyError::NotCanonical`] only through an internal fault.
pub fn retranscribe_operation_key(
    key: &RecognitionKey,
    base: Option<&TranscriptRevisionId>,
) -> Result<OperationKey, JobKeyError> {
    let digest = digest_lines(
        "vsift.retranscribe-op.v1",
        &[
            key.as_str(),
            base.map_or("none", TranscriptRevisionId::as_str),
        ],
    )?;
    OperationKey::from_sha256(digest.as_str()).map_err(|_| JobKeyError::NotCanonical)
}

/// The job id of `operation_key` in `session_id`: `job_` and the first 32 hex
/// digits of `sha256("vsift.job.v1\n" + session + "\n" + key)`.
///
/// # Errors
///
/// [`JobKeyError::NotCanonical`] only through an internal fault.
pub fn job_id(session_id: &SessionId, operation_key: &OperationKey) -> Result<JobId, JobKeyError> {
    let digest = digest_lines(
        "vsift.job.v1",
        &[session_id.as_str(), operation_key.as_str()],
    )?;
    JobId::parse(format!(
        "job_{}",
        digest.as_str().get(..JOB_IDENTITY_HEX).unwrap_or_default()
    ))
    .map_err(|_| JobKeyError::NotCanonical)
}

/// The operation id attempt `attempt` of epoch `epoch` of `job` commits
/// under: deterministic, so recovery can look for it in the manifest chain.
///
/// # Errors
///
/// [`JobKeyError::NotCanonical`] only through an internal fault.
pub fn commit_operation_id(
    job: &JobId,
    epoch: u32,
    attempt: u32,
) -> Result<OperationId, JobKeyError> {
    OperationId::parse(derived_identity(
        "op_",
        "vsift.job-commit.v1",
        &[job.as_str(), &epoch.to_string(), &attempt.to_string()],
    ))
    .map_err(|_| JobKeyError::NotCanonical)
}

/// What a job was asked to do, as its record keeps it for a resume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobRequest {
    /// `transcript retranscribe`, of one range or (`None`) the whole source.
    Retranscribe {
        /// The requested range, before it is widened to whole segments.
        range: Option<TimeRange>,
    },
}

impl JobRequest {
    /// The kind of job the request runs as.
    #[must_use]
    pub const fn kind(self) -> JobKind {
        match self {
            Self::Retranscribe { .. } => JobKind::Retranscribe,
        }
    }
}

/// Everything that identifies a job before it exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobSpec {
    /// Session the job belongs to.
    pub session_id: SessionId,
    /// The job's identity, derived from `operation_key`.
    pub job_id: JobId,
    /// Digest of the public request.
    pub request_digest: RequestDigest,
    /// Digest of the whole result.
    pub operation_key: OperationKey,
    /// Digest of the recognition its checkpoints belong to.
    pub recognition_key: RecognitionKey,
    /// The request, kept so the job can be resumed by id.
    pub request: JobRequest,
}

/// The publication a job recorded before committing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobCommit {
    /// Operation id the generation is published under.
    pub operation_id: OperationId,
    /// The session generation the commit expects to follow.
    pub observed_generation: StorageGeneration,
    /// The revision the commit publishes.
    pub revision_id: TranscriptRevisionId,
}

/// A job's stored record.
///
/// Storage adapters decode it strictly and change it only through
/// [`JobChange`]s they apply with [`JobState::transition_to`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobRecord {
    /// The job's identity.
    pub job_id: JobId,
    /// Session the job belongs to.
    pub session_id: SessionId,
    /// Digest of the public request.
    pub request_digest: RequestDigest,
    /// Digest of the whole result.
    pub operation_key: OperationKey,
    /// Digest of the recognition its checkpoints belong to.
    pub recognition_key: RecognitionKey,
    /// The request, for a resume by id.
    pub request: JobRequest,
    /// Operation ids bound to this job, oldest first, at most
    /// [`MAX_JOB_OPERATION_IDS`].
    pub operation_ids: Vec<OperationId>,
    /// The recorded state; see [`JobView::effective_state`] for liveness.
    pub state: JobState,
    /// How often the job was restarted after a terminal failure or cancel.
    pub epoch: u32,
    /// Attempts started in this epoch, at most [`vsift_domain::MAX_JOB_ATTEMPTS`].
    pub attempt: u32,
    /// One entry per failed attempt of this epoch, oldest first.
    pub failures: Vec<AttemptFailure>,
    /// The publication recorded by `committing` and kept by `succeeded`.
    pub commit: Option<JobCommit>,
    /// When the job was created, in Unix seconds.
    pub created_at_unix_seconds: u64,
    /// When the record last changed, in Unix seconds.
    pub updated_at_unix_seconds: u64,
}

impl JobRecord {
    /// A new record for `spec`, queued.
    #[must_use]
    pub fn queued(spec: &JobSpec, now_unix_seconds: u64) -> Self {
        Self {
            job_id: spec.job_id.clone(),
            session_id: spec.session_id.clone(),
            request_digest: spec.request_digest.clone(),
            operation_key: spec.operation_key.clone(),
            recognition_key: spec.recognition_key.clone(),
            request: spec.request,
            operation_ids: Vec::new(),
            state: JobState::Queued,
            epoch: 0,
            attempt: 0,
            failures: Vec::new(),
            commit: None,
            created_at_unix_seconds: now_unix_seconds,
            updated_at_unix_seconds: now_unix_seconds,
        }
    }

    /// The record after `change`, or why the change is not legal.
    ///
    /// This is the single place the effect of every change is defined; a
    /// store applies it to the record it read under its state lock.
    ///
    /// # Errors
    ///
    /// Returns [`JobStoreError::Transition`] for a change the state graph
    /// forbids, and [`JobStoreError::Storage`] with
    /// [`SessionStorageError::CapacityExhausted`] for a job out of attempts.
    pub fn changed(&self, change: &JobChange, now: u64) -> Result<Self, JobStoreError> {
        let mut next = self.clone();
        // Never backwards, even if the clock is: a record must stay valid.
        next.updated_at_unix_seconds = now.max(self.updated_at_unix_seconds);
        match change {
            JobChange::Start => {
                next.state = self.state.transition_to(JobState::Running)?;
                if self.attempt >= vsift_domain::MAX_JOB_ATTEMPTS {
                    return Err(JobStoreError::Storage(
                        SessionStorageError::CapacityExhausted,
                    ));
                }
                next.attempt = self.attempt.saturating_add(1);
                next.commit = None;
            }
            JobChange::Commit(commit) => {
                // A commit retried after the session moved on keeps the
                // committing state and replaces what it expects to follow.
                if self.state != JobState::Committing {
                    next.state = self.state.transition_to(JobState::Committing)?;
                }
                next.commit = Some(commit.clone());
            }
            JobChange::Succeed => {
                next.state = self.state.transition_to(JobState::Succeeded)?;
            }
            JobChange::Interrupt(failure) => {
                next.state = self.state.transition_to(JobState::Interrupted)?;
                next.commit = None;
                if let Some(failure) = failure {
                    push_failure(&mut next.failures, *failure);
                }
            }
            JobChange::Fail(failure) => {
                next.state = self.state.transition_to(JobState::Failed)?;
                next.commit = None;
                if let Some(failure) = failure {
                    push_failure(&mut next.failures, *failure);
                }
            }
            JobChange::RequestCancel => {
                next.state = self.state.transition_to(JobState::Cancelling)?;
            }
            JobChange::Cancel => {
                next.state = self.state.transition_to(JobState::Cancelled)?;
                next.commit = None;
            }
            JobChange::Restart => {
                if !matches!(self.state, JobState::Failed | JobState::Cancelled) {
                    return Err(JobStoreError::Transition(JobTransitionError::new(
                        self.state,
                        JobState::Queued,
                    )));
                }
                next.state = JobState::Queued;
                next.epoch = self.epoch.checked_add(1).ok_or(JobStoreError::Storage(
                    SessionStorageError::CapacityExhausted,
                ))?;
                next.attempt = 0;
                next.failures.clear();
                next.commit = None;
            }
        }
        Ok(next)
    }
}

fn push_failure(failures: &mut Vec<AttemptFailure>, failure: AttemptFailure) {
    if failures.len() >= MAX_RECORDED_FAILURES {
        failures.remove(0);
    }
    failures.push(failure);
}

/// One change a job owner or a canceller applies to a job record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobChange {
    /// Starts an attempt: `queued` or `interrupted` to `running`.
    Start,
    /// Records the publication about to be made: `running` to `committing`,
    /// or a new expectation while already committing.
    Commit(JobCommit),
    /// The publication is in the manifest chain: `committing` to `succeeded`.
    /// Checkpoints are removed.
    Succeed,
    /// The attempt stopped and the job stays resumable.
    Interrupt(Option<AttemptFailure>),
    /// The job cannot continue. Checkpoints are removed.
    Fail(Option<AttemptFailure>),
    /// Asks a live owner to stop: `running` or `queued` to `cancelling`.
    RequestCancel,
    /// Cancellation won. Checkpoints are removed.
    Cancel,
    /// Starts a failed or cancelled job again in a new epoch, from nothing.
    Restart,
}

impl JobChange {
    /// Whether the change ends the job's use of its checkpoints.
    #[must_use]
    pub const fn discards_checkpoints(&self) -> bool {
        matches!(
            self,
            Self::Succeed | Self::Fail(_) | Self::Cancel | Self::Restart
        )
    }
}

/// Whether a job's owner lock is held by a live process right now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobLiveness {
    /// A process holds the owner lock: the job is live.
    Owned,
    /// No process holds it.
    Unowned,
}

/// A job's record and whether it is live, as one read observed them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobView {
    /// The stored record.
    pub record: JobRecord,
    /// Whether an owner holds the job.
    pub liveness: JobLiveness,
    /// How many chunk checkpoints the job holds.
    pub checkpoints: usize,
}

impl JobView {
    /// The state a caller should see: a record that names an owner which no
    /// longer holds the lock is interrupted, not running (recovery then
    /// decides whether a commit landed).
    #[must_use]
    pub const fn effective_state(&self) -> JobState {
        if self.record.state.claims_an_owner() && matches!(self.liveness, JobLiveness::Unowned) {
            JobState::Interrupted
        } else {
            self.record.state
        }
    }
}

/// Why a job store operation failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStoreError {
    /// The session holds no such job.
    NotFound,
    /// The change is not legal from the stored state.
    Transition(JobTransitionError),
    /// This owner's attempt is no longer the job's current attempt, so it
    /// may not change the job (a stale attempt cannot commit).
    StaleOwner,
    /// A storage failure, including [`SessionStorageError::Busy`] for a job
    /// another process owns.
    Storage(SessionStorageError),
}

impl From<JobTransitionError> for JobStoreError {
    fn from(error: JobTransitionError) -> Self {
        Self::Transition(error)
    }
}

impl From<SessionStorageError> for JobStoreError {
    fn from(error: SessionStorageError) -> Self {
        Self::Storage(error)
    }
}

impl fmt::Display for JobStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("the session holds no such job"),
            Self::Transition(error) => error.fmt(formatter),
            Self::StaleOwner => formatter.write_str("the job has a newer attempt"),
            Self::Storage(error) => error.fmt(formatter),
        }
    }
}

impl Error for JobStoreError {}

/// What reading one chunk checkpoint found.
#[derive(Clone, Debug, PartialEq)]
pub enum CheckpointRead {
    /// No checkpoint was stored for the chunk.
    Absent,
    /// A checkpoint that decoded strictly and whose payload matched its digest.
    Found(ChunkCheckpoint),
    /// A checkpoint file that could not be used (truncated, forged, a newer
    /// version, over its bound); the store has already removed it.
    Unusable,
}

/// Why a checkpoint could not be stored. A run continues without it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointStoreError {
    /// The encoded checkpoint is larger than a checkpoint may be.
    TooLarge,
    /// The job already holds as many checkpoints as it may.
    TooMany,
    /// Writing, flushing or renaming it failed.
    Storage(SessionStorageError),
}

impl fmt::Display for CheckpointStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => formatter.write_str("the checkpoint is larger than its bound"),
            Self::TooMany => formatter.write_str("the job holds as many checkpoints as it may"),
            Self::Storage(error) => error.fmt(formatter),
        }
    }
}

impl Error for CheckpointStoreError {}

/// Port that keeps one job's chunk checkpoints.
///
/// Checkpoints are private stage files, never evidence. Storing one is an
/// optimisation: a run whose checkpoint could not be stored continues and
/// that chunk is simply done again after an interruption.
pub trait ChunkCheckpoints: Send + Sync {
    /// Reads the checkpoint of chunk `index`, removing it if it is unusable.
    fn load(&self, index: u32) -> CheckpointRead;

    /// Stores a checkpoint durably enough that a later run can read it.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointStoreError`]; the caller continues without it.
    fn store(&self, checkpoint: &ChunkCheckpoint) -> Result<(), CheckpointStoreError>;

    /// Removes the checkpoint of chunk `index`, best effort.
    fn discard(&self, index: u32);
}

/// No checkpoints: every chunk is decoded and recognised.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCheckpoints;

impl ChunkCheckpoints for NoCheckpoints {
    fn load(&self, _index: u32) -> CheckpointRead {
        CheckpointRead::Absent
    }

    fn store(&self, _checkpoint: &ChunkCheckpoint) -> Result<(), CheckpointStoreError> {
        Ok(())
    }

    fn discard(&self, _index: u32) {}
}

/// Exclusive ownership of one job, held until dropped.
///
/// The owner holds the job's operating-system lock, so no other process can
/// run or reconcile the job meanwhile. Every change is applied under the
/// job's short state lock to the record as stored, so a cancellation
/// requested by another process between two changes is seen.
pub trait JobOwner: ChunkCheckpoints {
    /// The record as of the owner's last change or read.
    fn record(&self) -> &JobRecord;

    /// Applies `change` to the stored record and keeps the result.
    ///
    /// # Errors
    ///
    /// [`JobStoreError::StaleOwner`] when the stored record belongs to
    /// another attempt, [`JobStoreError::Transition`] for an illegal change,
    /// and storage failures.
    fn apply(&mut self, change: &JobChange, now: u64) -> Result<(), JobStoreError>;

    /// Binds `operation_id` to this job, so a retry with it finds the job.
    ///
    /// # Errors
    ///
    /// Capacity when the job or session holds too many operation ids, and
    /// storage failures.
    fn bind_operation(&mut self, operation_id: &OperationId, now: u64)
    -> Result<(), JobStoreError>;

    /// Whether another process has asked this job to stop (the stored state
    /// is `cancelling`). A read failure counts as no request.
    fn cancel_requested(&self) -> bool;
}

/// What a cancellation request found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelRequest {
    /// A live owner was asked to stop; it will stop at its next chunk or
    /// before its commit.
    Requested,
    /// A live owner had already been asked to stop.
    AlreadyRequested,
    /// A live owner is committing: cancellation is too late.
    TooLate,
    /// The job is already terminal in this state.
    Terminal(JobState),
    /// No process owns the job; the caller must acquire it to cancel it.
    NotLive,
}

/// Port that keeps a session's jobs.
pub trait JobStore: Send + Sync {
    /// Exclusive ownership of one job.
    type Owner: JobOwner;

    /// The job's record and liveness, if the session holds it.
    ///
    /// # Errors
    ///
    /// Storage failures, including integrity and version failures of the record.
    fn job(&self, session_id: &SessionId, job_id: &JobId)
    -> Result<Option<JobView>, JobStoreError>;

    /// The session a job belongs to, from the root's job index.
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn job_session(&self, job_id: &JobId) -> Result<Option<SessionId>, JobStoreError>;

    /// The job an operation id is bound to in `session_id`, if any.
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn job_for_operation(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
    ) -> Result<Option<JobId>, JobStoreError>;

    /// Every job of a session, in identity order.
    ///
    /// # Errors
    ///
    /// Storage failures.
    fn session_jobs(&self, session_id: &SessionId) -> Result<Vec<JobView>, JobStoreError>;

    /// Creates the job `spec` names if the session does not hold it yet, then
    /// takes ownership of it. Returns the owner and whether it was created.
    ///
    /// # Errors
    ///
    /// [`SessionStorageError::Busy`] when another process owns the job, a
    /// record whose keys differ from `spec` as an integrity failure, and
    /// [`SessionStorageError::CapacityExhausted`] when the session holds
    /// [`MAX_SESSION_JOBS`] jobs none of which can be pruned.
    fn open_or_create(
        &self,
        spec: &JobSpec,
        now: u64,
    ) -> Result<(Self::Owner, bool), JobStoreError>;

    /// Takes ownership of an existing job.
    ///
    /// # Errors
    ///
    /// [`JobStoreError::NotFound`], [`SessionStorageError::Busy`] when
    /// another process owns it, and storage failures.
    fn acquire(&self, session_id: &SessionId, job_id: &JobId)
    -> Result<Self::Owner, JobStoreError>;

    /// Asks the job's live owner to stop, under the job's state lock.
    ///
    /// # Errors
    ///
    /// [`JobStoreError::NotFound`] and storage failures.
    fn request_cancel(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
        now: u64,
    ) -> Result<CancelRequest, JobStoreError>;
}

/// Port that tells whether an operation was committed, from the manifest
/// chain, which is the source of truth for a commit.
pub trait CommitLedger: Send + Sync {
    /// The generation above `above` that `operation_id` committed, if any.
    ///
    /// # Errors
    ///
    /// Storage failures, including a chain that fails validation.
    fn committed_generation(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        above: StorageGeneration,
    ) -> Result<Option<StorageGeneration>, SessionStorageError>;
}

/// What reconciling an owned job with the manifest chain found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reconciled {
    /// The record already described the job; nothing changed.
    Unchanged,
    /// The job's commit is in the chain: it is now `succeeded`.
    Committed,
    /// The previous owner ended without committing: the job is now
    /// `interrupted` (or `failed`, if it was out of attempts).
    Interrupted,
    /// The previous owner ended after a cancellation was requested: the job
    /// is now `cancelled`.
    Cancelled,
}

/// Reconciles a job whose previous owner is gone with the manifest chain.
///
/// It runs one way only: a job recorded as committing whose commit operation
/// appears in the chain above the generation it observed has succeeded, even
/// though its record says otherwise (the later record write is best effort).
/// A job recorded as committing without that, or as running, was
/// interrupted; one recorded as cancelling is cancelled.
///
/// # Errors
///
/// Storage and ledger failures; the record is left as it was.
pub fn reconcile<O, L>(owner: &mut O, ledger: &L, now: u64) -> Result<Reconciled, JobStoreError>
where
    O: JobOwner,
    L: CommitLedger + ?Sized,
{
    let record = owner.record();
    match record.state {
        JobState::Committing => {
            let landed = match &record.commit {
                Some(commit) => ledger
                    .committed_generation(
                        &record.session_id,
                        &commit.operation_id,
                        commit.observed_generation,
                    )?
                    .is_some(),
                None => false,
            };
            if landed {
                owner.apply(&JobChange::Succeed, now)?;
                Ok(Reconciled::Committed)
            } else {
                end_interrupted(owner, now)?;
                Ok(Reconciled::Interrupted)
            }
        }
        JobState::Running => {
            end_interrupted(owner, now)?;
            Ok(Reconciled::Interrupted)
        }
        JobState::Cancelling => {
            owner.apply(&JobChange::Cancel, now)?;
            Ok(Reconciled::Cancelled)
        }
        JobState::Queued
        | JobState::Interrupted
        | JobState::Succeeded
        | JobState::Failed
        | JobState::Cancelled => Ok(Reconciled::Unchanged),
    }
}

/// Ends an attempt whose owner is gone: interrupted, or failed when the job
/// has used every attempt or is poisoned.
fn end_interrupted<O: JobOwner>(owner: &mut O, now: u64) -> Result<(), JobStoreError> {
    let record = owner.record();
    let exhausted = record.attempt >= vsift_domain::MAX_JOB_ATTEMPTS
        || vsift_domain::poisoned_chunk(&record.failures).is_some();
    let change = if exhausted {
        JobChange::Fail(None)
    } else {
        JobChange::Interrupt(None)
    };
    owner.apply(&change, now)
}

/// How a request that carries an operation id relates to earlier work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationLookup {
    /// The id is not bound to any job in the session.
    Unbound,
    /// The id's job succeeded: its committed result answers the request.
    Replay(Box<JobRecord>),
    /// The id's job is live in another process.
    Busy(JobId),
    /// The id's job can continue (or restart); the request runs.
    Continue(JobId),
}

/// Looks up the job an operation id is bound to, before any work runs.
///
/// This is the retry table of ADR 0020 section 4 for a request with an
/// operation id: the same id with a different request digest is a conflict
/// (X-03); a succeeded job answers with its committed result (X-02); a live
/// one is busy; anything else continues. An interrupted job whose commit
/// landed is reconciled first, so a crash between the pointer and the job
/// record still replays rather than committing twice.
///
/// # Errors
///
/// [`JobRunError::IdempotencyConflict`], and store or ledger failures.
pub fn lookup_operation<S>(
    store: &S,
    session_id: &SessionId,
    operation_id: &OperationId,
    request_digest: &RequestDigest,
    now: u64,
) -> Result<OperationLookup, JobRunError>
where
    S: JobStore + CommitLedger,
{
    let Some(job_id) = store
        .job_for_operation(session_id, operation_id)
        .map_err(JobRunError::Job)?
    else {
        return Ok(OperationLookup::Unbound);
    };
    let Some(view) = store.job(session_id, &job_id).map_err(JobRunError::Job)? else {
        return Ok(OperationLookup::Unbound);
    };
    if view.record.request_digest != *request_digest {
        return Err(JobRunError::IdempotencyConflict { job: job_id });
    }
    match view.effective_state() {
        JobState::Succeeded => Ok(OperationLookup::Replay(Box::new(view.record))),
        _ if view.liveness == JobLiveness::Owned => Ok(OperationLookup::Busy(job_id)),
        JobState::Interrupted if view.record.state == JobState::Committing => {
            match store.acquire(session_id, &job_id) {
                Ok(mut owner) => {
                    reconcile(&mut owner, store, now).map_err(JobRunError::Job)?;
                    if owner.record().state == JobState::Succeeded {
                        Ok(OperationLookup::Replay(Box::new(owner.record().clone())))
                    } else {
                        Ok(OperationLookup::Continue(job_id))
                    }
                }
                Err(JobStoreError::Storage(SessionStorageError::Busy)) => {
                    Ok(OperationLookup::Busy(job_id))
                }
                Err(error) => Err(JobRunError::Job(error)),
            }
        }
        _ => Ok(OperationLookup::Continue(job_id)),
    }
}

/// What cancelling a job achieved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelOutcome {
    /// The job is cancelled now; its checkpoints are gone.
    Cancelled,
    /// A live owner was asked to stop and will cancel at its next boundary.
    Requested,
    /// Cancellation came too late: the job is committing or committed.
    TooLate(JobState),
    /// The job had already ended in this state; nothing changed.
    AlreadyEnded(JobState),
}

/// Cancels a job (the `CancelJob` use case).
///
/// Cancellation is serialized with the commit under the job's state lock: a
/// cancel that wins leaves no generation, one that loses reports the committed
/// result (`TooLate`), and a repeated cancel is idempotent. A job no process
/// owns is acquired, reconciled with the chain (its commit may have landed),
/// then cancelled.
///
/// # Errors
///
/// [`JobStoreError::NotFound`], [`SessionStorageError::Busy`] when an owner
/// appeared meanwhile, and storage failures.
pub fn cancel_job<S>(
    store: &S,
    session_id: &SessionId,
    job_id: &JobId,
    now: u64,
) -> Result<CancelOutcome, JobStoreError>
where
    S: JobStore + CommitLedger,
{
    match store.request_cancel(session_id, job_id, now)? {
        CancelRequest::Requested | CancelRequest::AlreadyRequested => {
            return Ok(CancelOutcome::Requested);
        }
        CancelRequest::TooLate => return Ok(CancelOutcome::TooLate(JobState::Committing)),
        CancelRequest::Terminal(JobState::Succeeded) => {
            return Ok(CancelOutcome::TooLate(JobState::Succeeded));
        }
        CancelRequest::Terminal(state) => return Ok(CancelOutcome::AlreadyEnded(state)),
        CancelRequest::NotLive => {}
    }
    let mut owner = store.acquire(session_id, job_id)?;
    match reconcile(&mut owner, store, now)? {
        Reconciled::Committed => return Ok(CancelOutcome::TooLate(JobState::Succeeded)),
        Reconciled::Cancelled => return Ok(CancelOutcome::Cancelled),
        Reconciled::Unchanged | Reconciled::Interrupted => {}
    }
    match owner.record().state {
        JobState::Queued | JobState::Interrupted => {
            owner.apply(&JobChange::Cancel, now)?;
            Ok(CancelOutcome::Cancelled)
        }
        JobState::Succeeded => Ok(CancelOutcome::TooLate(JobState::Succeeded)),
        state @ (JobState::Failed | JobState::Cancelled) => Ok(CancelOutcome::AlreadyEnded(state)),
        // Reconciliation moved every owner-claiming state on.
        state @ (JobState::Running | JobState::Committing | JobState::Cancelling) => Err(
            JobStoreError::Transition(JobTransitionError::new(state, JobState::Cancelled)),
        ),
    }
}

/// Reads a job's current state (the `JobStatusQuery` use case).
///
/// A job no process owns whose record still names an owner is reconciled
/// first, so the answer never reports a crashed job as running and never
/// reports a landed commit as interrupted. A live job is reported as its
/// owner last recorded it.
///
/// # Errors
///
/// [`JobStoreError::NotFound`] and storage failures.
pub fn job_status<S>(
    store: &S,
    session_id: &SessionId,
    job_id: &JobId,
    now: u64,
) -> Result<JobView, JobStoreError>
where
    S: JobStore + CommitLedger,
{
    let view = store
        .job(session_id, job_id)?
        .ok_or(JobStoreError::NotFound)?;
    if !view.record.state.claims_an_owner() || view.liveness == JobLiveness::Owned {
        return Ok(view);
    }
    match store.acquire(session_id, job_id) {
        Ok(mut owner) => {
            reconcile(&mut owner, store, now)?;
            drop(owner);
            store
                .job(session_id, job_id)?
                .ok_or(JobStoreError::NotFound)
        }
        // An owner appeared between the read and the acquisition: it is live.
        Err(JobStoreError::Storage(SessionStorageError::Busy)) => store
            .job(session_id, job_id)?
            .ok_or(JobStoreError::NotFound),
        Err(error) => Err(error),
    }
}

/// Whether a job may be resumed by id (the `ResumeJob` use case's check):
/// only an interrupted or queued job can; a live one is busy.
///
/// Resuming runs the job's recorded request again, which finds the same job
/// through its keys and continues from its checkpoints.
///
/// # Errors
///
/// [`JobRunError::Busy`] for a live job, [`JobRunError::NotResumable`] for a
/// terminal one, and store failures.
pub fn resumable_request<S>(
    store: &S,
    session_id: &SessionId,
    job_id: &JobId,
    now: u64,
) -> Result<JobRecord, JobRunError>
where
    S: JobStore + CommitLedger,
{
    let view = job_status(store, session_id, job_id, now).map_err(JobRunError::Job)?;
    if view.liveness == JobLiveness::Owned && view.record.state.claims_an_owner() {
        return Err(JobRunError::Busy {
            job: job_id.clone(),
        });
    }
    match view.record.state {
        JobState::Queued | JobState::Interrupted => Ok(view.record),
        state => Err(JobRunError::NotResumable {
            job: job_id.clone(),
            state,
        }),
    }
}
