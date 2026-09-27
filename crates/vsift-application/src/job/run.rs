//! The retranscription job: runs, resumes and commits one retranscription
//! exactly once (ADR 0020 sections 4-5).
//!
//! [`run_retranscription`] is the retry table of ADR 0020 section 4 made
//! executable. For one request it decides, in order:
//!
//! 1. with an operation id already bound to a job: a different request is an
//!    idempotency conflict (X-03); a succeeded job answers with its committed
//!    revision and no new generation (X-02); a live one is busy;
//! 2. it creates the job the request's keys name, or takes ownership of it
//!    (busy if another process owns it), reconciles it with the manifest
//!    chain, and restarts it if it had failed or been cancelled;
//! 3. it runs attempts: each admits, recognises the range from its
//!    checkpoints and the audio, assembles the revision, verifies the source
//!    again and commits under a deterministic operation id. Busy contention
//!    is retried at most twice with full-jitter backoff; other failures end
//!    the call with the job resumable, unless its chunk is poisoned or its
//!    attempts are used up.
//!
//! A commit that finds the session moved on re-reads its head. If the newest
//! revision is still the base, the commit is retried against the new
//! generation; if another revision superseded the base but re-widening the
//! request over it gives the same range, the recognised segments are spliced
//! onto the new newest revision; otherwise the job fails as superseded.

use std::{error::Error, fmt, future::Future, num::NonZeroU32, time::Duration};

use vsift_domain::{
    AttemptFailure, FailureCode, Jitter, JobId, JobState, OperationId, RetryDecision, RetryPolicy,
    SessionId, SourceId, StorageGeneration, TimeRange, TranscriptRevision, TranscriptRevisionId,
};

use super::{
    CommitLedger, JobChange, JobCommit, JobKeyError, JobOwner, JobRecord, JobSpec, JobStore,
    JobStoreError, OperationLookup, commit_operation_id, lookup_operation, reconcile,
};
use crate::{
    AsrCancellation, AsrFailure, AsrFailureReason, AsrRevisionRequest, AsrStage, AsrTranscription,
    SessionStorageError, SpeechAudioSource, SpeechRecognizer, TranscribeRangeRequest,
    TranscriptBuildError,
    asr::{AsrRunFailure, CheckpointScope, CheckpointUse, retranscription_range},
    build_asr_revision, transcribe_range_checkpointed,
};

/// A session's committed head as a commit sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionHead {
    /// The committed generation.
    pub generation: StorageGeneration,
    /// The session's newest transcript revision, if it has one.
    pub newest: Option<TranscriptRevision>,
}

/// Port through which a job reads and commits the session's revisions.
pub trait RevisionStore: Send + Sync {
    /// A held admission slot; dropping it releases the slot.
    type Permit: Send;

    /// Takes one admission slot of the session root without waiting.
    ///
    /// # Errors
    ///
    /// [`SessionStorageError::Busy`] when every slot is taken.
    fn admit(&self) -> Result<Self::Permit, SessionStorageError>;

    /// The committed head of an open, unexpired session.
    ///
    /// # Errors
    ///
    /// [`SessionStorageError::StateConflict`] for a closed or expired
    /// session, and storage failures.
    fn head(&self, session_id: &SessionId, now: u64) -> Result<SessionHead, SessionStorageError>;

    /// One committed revision by identity.
    ///
    /// # Errors
    ///
    /// As [`Self::head`].
    fn revision(
        &self,
        session_id: &SessionId,
        revision_id: &TranscriptRevisionId,
        now: u64,
    ) -> Result<Option<TranscriptRevision>, SessionStorageError>;

    /// Commits `revision` as the generation after `expected`, under
    /// `operation_id`.
    ///
    /// # Errors
    ///
    /// [`SessionStorageError::StateConflict`] when the session moved past
    /// `expected` or is closed, [`SessionStorageError::Busy`] under
    /// contention, and storage failures.
    fn publish_revision(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        expected: StorageGeneration,
        revision: &TranscriptRevision,
        now: u64,
    ) -> Result<StorageGeneration, SessionStorageError>;
}

/// Draws jitter and waits: the retry policy's only side effects.
pub trait RetryTimer: Send + Sync {
    /// A fresh uniform sample.
    fn jitter(&self) -> Jitter;

    /// Waits `delay`.
    fn sleep(&self, delay: Duration) -> impl Future<Output = ()> + Send;
}

/// The last check before a commit: everything the run read must be
/// unchanged (for a retranscription, the full hash of the session's source
/// copy, issue #148).
pub trait CommitGuard: Send {
    /// Verifies the inputs again.
    ///
    /// # Errors
    ///
    /// [`SessionStorageError::IntegrityFailure`] for a changed input, and
    /// storage failures; nothing is committed.
    fn verify(&mut self) -> Result<(), SessionStorageError>;
}

/// One retranscription request, as the engine resolved it.
#[derive(Clone, Copy, Debug)]
pub struct RetranscriptionRun<'a> {
    /// The job the request's keys name.
    pub spec: &'a JobSpec,
    /// The caller's operation id, if it supplied one.
    pub operation_id: Option<&'a OperationId>,
    /// What to recognise; its range is the replaced range.
    pub transcribe: TranscribeRangeRequest<'a>,
    /// Source the revision describes.
    pub source_id: &'a SourceId,
    /// The range the caller asked for, before widening.
    pub requested: Option<TimeRange>,
    /// The newest revision when the request was resolved: the base.
    pub base: Option<&'a TranscriptRevision>,
    /// The session generation observed with the base.
    pub observed: StorageGeneration,
    /// Current time in Unix seconds.
    pub now: u64,
}

/// The ports one run uses.
pub struct RetranscriptionPorts<'a, S, A, R, C, T> {
    /// Jobs, the manifest chain and revisions.
    pub store: &'a S,
    /// Speech audio of the bound source.
    pub audio: &'a A,
    /// The selected recognizer.
    pub recognizer: &'a R,
    /// The caller's cancellation.
    pub cancellation: &'a C,
    /// Backoff jitter and sleeping.
    pub timer: &'a T,
    /// The single public mapping from a failure to its code, owned by the
    /// engine; the retry policy and the poison rule decide by it.
    pub classify: fn(&JobRunError) -> FailureCode,
}

/// What a run tells its caller about the job behind it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobReport {
    /// The job.
    pub job_id: JobId,
    /// The operation id the result is recorded under: the caller's, or the
    /// commit's own when the caller supplied none.
    pub operation_id: OperationId,
    /// Whether the job had started before this request and continued.
    pub resumed: bool,
    /// Chunks taken from checkpoints stored before this request.
    pub chunks_reused: u32,
    /// Checkpoints found unusable and done again.
    pub checkpoints_discarded: u32,
    /// Whether the result is an earlier commit, returned without a new one.
    pub replayed: bool,
}

/// A committed (or replayed) retranscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetranscriptionOutcome {
    /// The revision, now the session's newest unless replayed.
    pub revision: TranscriptRevision,
    /// The job behind it.
    pub report: JobReport,
}

/// Why a retranscription job did not produce a revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobRunError {
    /// Another process owns the job right now.
    Busy {
        /// The live job.
        job: JobId,
    },
    /// The operation id is bound to a different request (D-4).
    IdempotencyConflict {
        /// The job the id is bound to.
        job: JobId,
    },
    /// The job ended in a state it cannot be resumed from.
    NotResumable {
        /// The job.
        job: JobId,
        /// Its state.
        state: JobState,
    },
    /// The job was cancelled through its job id before it committed.
    Cancelled {
        /// The job.
        job: JobId,
    },
    /// Another revision replaced part of the range during the run.
    Superseded {
        /// The failed job.
        job: JobId,
    },
    /// Recognition failed; the job stays resumable unless poisoned.
    Asr {
        /// The job.
        job: JobId,
        /// Stage, reason and chunk.
        failure: AsrRunFailure,
    },
    /// The revision could not be assembled; an internal fault.
    Assembly(TranscriptBuildError),
    /// A session storage failure.
    Storage(SessionStorageError),
    /// A job storage failure.
    Job(JobStoreError),
    /// A key could not be derived; an internal fault.
    Key(JobKeyError),
}

impl fmt::Display for JobRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy { .. } => formatter.write_str("the job is running in another process"),
            Self::IdempotencyConflict { .. } => {
                formatter.write_str("the operation id is bound to a different request")
            }
            Self::NotResumable { state, .. } => write!(
                formatter,
                "the job is {} and cannot be resumed",
                state.identifier()
            ),
            Self::Cancelled { .. } => formatter.write_str("the job was cancelled"),
            Self::Superseded { .. } => {
                formatter.write_str("another revision replaced part of the range")
            }
            Self::Asr { failure, .. } => failure.failure.fmt(formatter),
            Self::Assembly(error) => {
                write!(formatter, "the revision could not be assembled: {error:?}")
            }
            Self::Storage(error) => error.fmt(formatter),
            Self::Job(error) => error.fmt(formatter),
            Self::Key(error) => error.fmt(formatter),
        }
    }
}

impl Error for JobRunError {}

impl JobRunError {
    /// The job the failure concerns, when there is one.
    #[must_use]
    pub const fn job(&self) -> Option<&JobId> {
        match self {
            Self::Busy { job }
            | Self::IdempotencyConflict { job }
            | Self::NotResumable { job, .. }
            | Self::Cancelled { job }
            | Self::Superseded { job }
            | Self::Asr { job, .. } => Some(job),
            Self::Assembly(_) | Self::Storage(_) | Self::Job(_) | Self::Key(_) => None,
        }
    }
}

/// Why one attempt stopped.
enum AttemptStop {
    /// Cancellation requested through the job won; the job is cancelled.
    CancelledByRequest,
    /// The attempt failed at `chunk` (if it was working on one).
    Failed {
        error: JobRunError,
        chunk: Option<u32>,
    },
}

impl AttemptStop {
    const fn failed(error: JobRunError) -> Self {
        Self::Failed { error, chunk: None }
    }
}

/// Runs, resumes or replays one retranscription job (see the module
/// documentation for the order of decisions).
///
/// # Errors
///
/// [`JobRunError`]; after a failure the job is interrupted and resumable
/// unless the error says otherwise (a poisoned chunk, used-up attempts or a
/// superseded range fail it; a cancellation through the job cancels it).
pub async fn run_retranscription<S, A, R, C, T, G>(
    run: RetranscriptionRun<'_>,
    ports: RetranscriptionPorts<'_, S, A, R, C, T>,
    guard: &mut G,
) -> Result<RetranscriptionOutcome, JobRunError>
where
    S: JobStore + CommitLedger + RevisionStore,
    A: SpeechAudioSource,
    R: SpeechRecognizer,
    C: AsrCancellation,
    T: RetryTimer,
    G: CommitGuard,
{
    let spec = run.spec;
    let store = ports.store;
    if let Some(operation) = run.operation_id {
        match lookup_operation(
            store,
            &spec.session_id,
            operation,
            &spec.request_digest,
            run.now,
        )? {
            OperationLookup::Replay(record) => {
                return replay(store, record.as_ref(), operation.clone(), run.now);
            }
            OperationLookup::Busy(job) => return Err(JobRunError::Busy { job }),
            OperationLookup::Unbound | OperationLookup::Continue(_) => {}
        }
    }

    let (mut owner, created) = store
        .open_or_create(spec, run.now)
        .map_err(|error| owned_or_busy(error, &spec.job_id))?;
    reconcile(&mut owner, store, run.now).map_err(JobRunError::Job)?;
    match owner.record().state {
        JobState::Succeeded => {
            // Defensive: a commit that landed without its record update.
            let record = owner.record().clone();
            let operation = match run.operation_id {
                Some(operation) => {
                    owner
                        .bind_operation(operation, run.now)
                        .map_err(JobRunError::Job)?;
                    operation.clone()
                }
                None => committed_operation(&record)?,
            };
            return replay(store, &record, operation, run.now);
        }
        JobState::Failed | JobState::Cancelled => owner
            .apply(&JobChange::Restart, run.now)
            .map_err(JobRunError::Job)?,
        JobState::Queued
        | JobState::Running
        | JobState::Committing
        | JobState::Interrupted
        | JobState::Cancelling => {}
    }
    let resumed = !created && owner.record().attempt > 0;
    if let Some(operation) = run.operation_id {
        owner
            .bind_operation(operation, run.now)
            .map_err(JobRunError::Job)?;
    }

    let mut retries = 0_u32;
    let mut first_use: Option<CheckpointUse> = None;
    let mut discarded = 0_u32;
    loop {
        owner
            .apply(&JobChange::Start, run.now)
            .map_err(JobRunError::Job)?;
        let attempt = attempt_once(&run, &ports, &mut owner, guard, &mut retries).await;
        match attempt {
            Ok((revision, usage, commit_operation)) => {
                let first = first_use.unwrap_or(usage);
                discarded = discarded.saturating_add(usage.discarded);
                if run.operation_id.is_none() {
                    // Best effort: lets the commit's own id replay the result.
                    let _ = owner.bind_operation(&commit_operation, run.now);
                }
                return Ok(RetranscriptionOutcome {
                    revision,
                    report: JobReport {
                        job_id: spec.job_id.clone(),
                        operation_id: run.operation_id.cloned().unwrap_or(commit_operation),
                        resumed: resumed || first.reused > 0,
                        chunks_reused: first.reused,
                        checkpoints_discarded: discarded,
                        replayed: false,
                    },
                });
            }
            Err((stop, usage)) => {
                if let Some(usage) = usage {
                    first_use.get_or_insert(usage);
                    discarded = discarded.saturating_add(usage.discarded);
                }
                let delay = settle_failure(&run, &ports, &mut owner, stop, retries)?;
                retries = retries.saturating_add(1);
                ports.timer.sleep(delay).await;
            }
        }
    }
}

/// Records how a failed attempt ends the job and decides whether to retry.
///
/// A cancellation requested through the job cancels it. Otherwise the
/// failure is recorded with its chunk: the job fails for a superseded range,
/// used-up attempts or a poisoned chunk, and is interrupted (resumable)
/// otherwise. Returns the backoff before an automatic retry, or the error
/// that ends the call.
fn settle_failure<S, A, R, C, T>(
    run: &RetranscriptionRun<'_>,
    ports: &RetranscriptionPorts<'_, S, A, R, C, T>,
    owner: &mut S::Owner,
    stop: AttemptStop,
    retries: u32,
) -> Result<Duration, JobRunError>
where
    S: JobStore,
    T: RetryTimer,
{
    let job = &run.spec.job_id;
    let (error, chunk) = match stop {
        AttemptStop::CancelledByRequest => return Err(JobRunError::Cancelled { job: job.clone() }),
        AttemptStop::Failed { error, chunk } => (error, chunk),
    };
    if owner.cancel_requested() {
        owner
            .apply(&JobChange::Cancel, run.now)
            .map_err(JobRunError::Job)?;
        return Err(JobRunError::Cancelled { job: job.clone() });
    }
    let code = (ports.classify)(&error);
    let failure = AttemptFailure { chunk, code };
    let decision = RetryPolicy::R0.decide(code, retries, None, ports.timer.jitter());
    let mut history = owner.record().failures.clone();
    history.push(failure);
    let ends = matches!(error, JobRunError::Superseded { .. })
        || owner.record().attempt >= vsift_domain::MAX_JOB_ATTEMPTS
        || vsift_domain::poisoned_chunk(&history).is_some();
    let change = if ends {
        JobChange::Fail(Some(failure))
    } else {
        JobChange::Interrupt(Some(failure))
    };
    owner.apply(&change, run.now).map_err(JobRunError::Job)?;
    match decision {
        RetryDecision::RetryAfter(delay) if !ends => Ok(delay),
        RetryDecision::RetryAfter(_) | RetryDecision::LeaveToCaller | RetryDecision::Never => {
            Err(error)
        }
    }
}

/// A job whose owner lock is held elsewhere is busy; anything else is a
/// job-store failure.
fn owned_or_busy(error: JobStoreError, job: &JobId) -> JobRunError {
    match error {
        JobStoreError::Storage(SessionStorageError::Busy) => JobRunError::Busy { job: job.clone() },
        other => JobRunError::Job(other),
    }
}

/// The operation a succeeded job committed under.
fn committed_operation(record: &JobRecord) -> Result<OperationId, JobRunError> {
    record
        .commit
        .as_ref()
        .map(|commit| commit.operation_id.clone())
        .ok_or(JobRunError::Storage(SessionStorageError::IntegrityFailure))
}

/// Answers with a succeeded job's committed revision: nothing runs and
/// nothing is committed (X-02).
fn replay<S: RevisionStore>(
    store: &S,
    record: &JobRecord,
    operation_id: OperationId,
    now: u64,
) -> Result<RetranscriptionOutcome, JobRunError> {
    let commit = record
        .commit
        .as_ref()
        .ok_or(JobRunError::Storage(SessionStorageError::IntegrityFailure))?;
    let revision = store
        .revision(&record.session_id, &commit.revision_id, now)
        .map_err(JobRunError::Storage)?
        .ok_or(JobRunError::Storage(SessionStorageError::IntegrityFailure))?;
    Ok(RetranscriptionOutcome {
        revision,
        report: JobReport {
            job_id: record.job_id.clone(),
            operation_id,
            resumed: false,
            chunks_reused: 0,
            checkpoints_discarded: 0,
            replayed: true,
        },
    })
}

/// The caller's cancellation, or a cancellation requested through the job.
struct JobAwareCancellation<'a, C, O> {
    caller: &'a C,
    owner: &'a O,
}

impl<C: AsrCancellation, O: JobOwner> AsrCancellation for JobAwareCancellation<'_, C, O> {
    fn is_cancelled(&self) -> bool {
        self.caller.is_cancelled() || self.owner.cancel_requested()
    }
}

type AttemptResult =
    Result<(TranscriptRevision, CheckpointUse, OperationId), (AttemptStop, Option<CheckpointUse>)>;

/// One attempt: admission, recognition from checkpoints and audio, assembly,
/// the closing verification and the commit.
async fn attempt_once<S, A, R, C, T, G>(
    run: &RetranscriptionRun<'_>,
    ports: &RetranscriptionPorts<'_, S, A, R, C, T>,
    owner: &mut S::Owner,
    guard: &mut G,
    retries: &mut u32,
) -> AttemptResult
where
    S: JobStore + CommitLedger + RevisionStore,
    A: SpeechAudioSource,
    R: SpeechRecognizer,
    C: AsrCancellation,
    T: RetryTimer,
    G: CommitGuard,
{
    let job = &run.spec.job_id;
    let permit = ports
        .store
        .admit()
        .map_err(|error| (AttemptStop::failed(JobRunError::Storage(error)), None))?;
    let recognised = {
        let shared: &S::Owner = owner;
        let cancellation = JobAwareCancellation {
            caller: ports.cancellation,
            owner: shared,
        };
        transcribe_range_checkpointed(
            run.transcribe,
            CheckpointScope {
                checkpoints: shared,
                key: &run.spec.recognition_key,
            },
            ports.audio,
            ports.recognizer,
            &cancellation,
        )
        .await
    };
    drop(permit);
    let (transcription, usage) = match recognised {
        Ok(result) => result,
        Err(failure) => {
            let chunk = failure.chunk;
            let stop = AttemptStop::Failed {
                error: JobRunError::Asr {
                    job: job.clone(),
                    failure,
                },
                chunk,
            };
            return Err((stop, None));
        }
    };
    let with_usage = |stop: AttemptStop| (stop, Some(usage));
    if ports.cancellation.is_cancelled() {
        return Err(with_usage(AttemptStop::failed(JobRunError::Asr {
            job: job.clone(),
            failure: AsrFailure {
                stage: AsrStage::Assembly,
                reason: AsrFailureReason::Cancelled,
            }
            .into(),
        })));
    }
    let revision = assemble(run, run.base, &transcription)
        .map_err(|error| with_usage(AttemptStop::failed(JobRunError::Assembly(error))))?;
    guard
        .verify()
        .map_err(|error| with_usage(AttemptStop::failed(JobRunError::Storage(error))))?;
    let operation = commit_operation_id(job, owner.record().epoch, owner.record().attempt)
        .map_err(|error| with_usage(AttemptStop::failed(JobRunError::Key(error))))?;
    commit(
        run,
        ports,
        owner,
        revision,
        &transcription,
        &operation,
        retries,
    )
    .await
    .map(|revision| (revision, usage, operation))
    .map_err(with_usage)
}

/// Builds the revision the run's transcription makes over `base`.
fn assemble(
    run: &RetranscriptionRun<'_>,
    base: Option<&TranscriptRevision>,
    transcription: &AsrTranscription,
) -> Result<TranscriptRevision, TranscriptBuildError> {
    let number = base
        .map_or(Some(1), |base| base.number().checked_add(1))
        .and_then(NonZeroU32::new)
        .ok_or(TranscriptBuildError::Invalid(
            vsift_domain::TranscriptRevisionError::InvalidAsrRun,
        ))?;
    build_asr_revision(AsrRevisionRequest {
        session_id: &run.spec.session_id,
        source_id: run.source_id,
        source_segment: run.transcribe.source_segment,
        number,
        transcription: transcription.clone(),
        splice: base.map(|base| crate::RevisionSplice {
            base,
            replaced_range: run.transcribe.range,
        }),
    })
}

/// Records `committing`, publishes, and follows the session if it moved.
#[allow(
    clippy::too_many_lines,
    reason = "The commit, rebase and reconciliation order is the contract; keep it in one place"
)]
async fn commit<S, A, R, C, T>(
    run: &RetranscriptionRun<'_>,
    ports: &RetranscriptionPorts<'_, S, A, R, C, T>,
    owner: &mut S::Owner,
    revision: TranscriptRevision,
    transcription: &AsrTranscription,
    operation: &OperationId,
    retries: &mut u32,
) -> Result<TranscriptRevision, AttemptStop>
where
    S: JobStore + CommitLedger + RevisionStore,
    T: RetryTimer,
{
    let store = ports.store;
    let session = &run.spec.session_id;
    let job = &run.spec.job_id;
    let mut expected = run.observed;
    let mut revision = revision;
    let mut base_id = run.base.map(|base| base.id().clone());
    loop {
        let recorded = owner.apply(
            &JobChange::Commit(JobCommit {
                operation_id: operation.clone(),
                observed_generation: expected,
                revision_id: revision.id().clone(),
            }),
            run.now,
        );
        if let Err(error) = recorded {
            if owner.cancel_requested() {
                // Cancellation won the state lock before the commit: nothing
                // is published (a cancel that wins leaves no generation).
                owner
                    .apply(&JobChange::Cancel, run.now)
                    .map_err(|error| AttemptStop::failed(JobRunError::Job(error)))?;
                return Err(AttemptStop::CancelledByRequest);
            }
            return Err(AttemptStop::failed(JobRunError::Job(error)));
        }
        let published = store.publish_revision(session, operation, expected, &revision, run.now);
        let Err(error) = published else {
            // The chain holds the commit; the record update is best effort
            // (recovery reconciles from the chain).
            let _ = owner.apply(&JobChange::Succeed, run.now);
            return Ok(revision);
        };
        // A failure after the pointer moved still committed: the chain,
        // not the error, says whether the operation landed.
        if let Ok(Some(_)) = store.committed_generation(session, operation, expected) {
            let _ = owner.apply(&JobChange::Succeed, run.now);
            return Ok(revision);
        }
        if !matches!(
            error,
            SessionStorageError::StateConflict | SessionStorageError::Busy
        ) {
            return Err(AttemptStop::failed(JobRunError::Storage(error)));
        }
        // The session moved (a renewal or another revision) or its writer
        // was busy: retry within the policy's budget.
        let decision =
            RetryPolicy::R0.decide(FailureCode::Busy, *retries, None, ports.timer.jitter());
        let RetryDecision::RetryAfter(delay) = decision else {
            return Err(AttemptStop::failed(JobRunError::Storage(
                SessionStorageError::Busy,
            )));
        };
        *retries = retries.saturating_add(1);
        ports.timer.sleep(delay).await;
        let head = store
            .head(session, run.now)
            .map_err(|error| AttemptStop::failed(JobRunError::Storage(error)))?;
        let newest_id = head.newest.as_ref().map(|newest| newest.id().clone());
        if newest_id != base_id {
            // Another revision superseded the base. The recognised range is
            // still right only if widening the request over the new newest
            // revision gives the same range; then splice onto it.
            let source = run.transcribe.source_segment.range();
            let range = retranscription_range(head.newest.as_ref(), run.requested, source);
            if range != run.transcribe.range {
                return Err(AttemptStop::failed(JobRunError::Superseded {
                    job: job.clone(),
                }));
            }
            revision = assemble(run, head.newest.as_ref(), transcription)
                .map_err(|error| AttemptStop::failed(JobRunError::Assembly(error)))?;
            base_id = newest_id;
        }
        expected = head.generation;
    }
}
