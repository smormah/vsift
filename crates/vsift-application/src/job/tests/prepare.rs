//! The `prepare` job of `job start` (ADR 0033 section 5) over the in-memory
//! store: its identity, and how the way its streams end is recorded on it.

use std::sync::Mutex;

use vsift_domain::{AttemptFailure, FailureCode, JobId, JobMode, JobState, PrepareStream};

use super::{Built, MemoryStore, NOW, TestResult, operation, session};
use crate::{
    Clock, ClockError, JobCommit, JobStore, PrepareError, PrepareOutcome, PrepareStreams,
    StreamEnd, StreamStop, prepare_spec, retranscribe_request_digest, run_prepare,
};

struct FixedClock;

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> Result<u64, ClockError> {
        Ok(NOW)
    }
}

fn commit() -> Built<JobCommit> {
    Ok(JobCommit {
        operation_id: operation("op_0123456789abcdef0123456789abcdef")?,
        observed_generation: vsift_domain::StorageGeneration::from_value(3),
        revision_id: vsift_domain::TranscriptRevisionId::parse(
            "trv_0123456789abcdef0123456789abcdef",
        )?,
    })
}

/// The first window's commit, which the whole transcript's supersedes.
fn first_commit() -> Built<JobCommit> {
    Ok(JobCommit {
        operation_id: operation("op_1111111111111111aaaaaaaaaaaaaaaa")?,
        observed_generation: vsift_domain::StorageGeneration::from_value(2),
        revision_id: vsift_domain::TranscriptRevisionId::parse(
            "trv_1111111111111111aaaaaaaaaaaaaaaa",
        )?,
    })
}

fn stream_job() -> Built<JobId> {
    Ok(JobId::parse("job_fedcba9876543210fedcba9876543210")?)
}

/// Streams that end as they are told, and remember what they were asked.
struct Scripted {
    outcome: Mutex<Option<Result<JobCommit, StreamStop>>>,
    /// The `prepare` job to ask to stop while the stream runs, as `job cancel`
    /// from another process would.
    cancel_prepare: Option<(MemoryStore, JobId)>,
    ran: Mutex<Vec<PrepareStream>>,
    cancelled: Mutex<Vec<JobId>>,
    first_seen: Mutex<Vec<Option<vsift_domain::TimeRange>>>,
    /// How the wait for the job's turn ends (at once and well by default).
    wait: Mutex<Option<Result<(), StreamStop>>>,
}

impl Scripted {
    fn ending(outcome: Result<JobCommit, StreamStop>) -> Self {
        Self {
            outcome: Mutex::new(Some(outcome)),
            cancel_prepare: None,
            ran: Mutex::default(),
            cancelled: Mutex::default(),
            first_seen: Mutex::default(),
            wait: Mutex::default(),
        }
    }

    fn ran(&self) -> Vec<PrepareStream> {
        self.ran.lock().map(|ran| ran.clone()).unwrap_or_default()
    }

    fn cancelled(&self) -> Vec<JobId> {
        self.cancelled
            .lock()
            .map(|cancelled| cancelled.clone())
            .unwrap_or_default()
    }
}

impl PrepareStreams for Scripted {
    fn run(
        &self,
        stream: PrepareStream,
        first: Option<vsift_domain::TimeRange>,
    ) -> impl std::future::Future<Output = Result<StreamEnd, StreamStop>> + Send {
        if let Ok(mut ran) = self.ran.lock() {
            ran.push(stream);
        }
        if let Some((store, job)) = &self.cancel_prepare
            && let Ok(session) = session()
        {
            let _ = store.request_cancel(&session, job, NOW);
        }
        if let Ok(mut seen) = self.first_seen.lock() {
            seen.push(first);
        }
        std::future::ready(match stream {
            // The visual stream indexes, and the first window commits a
            // revision of its own that the whole transcript supersedes.
            PrepareStream::Visual => Ok(StreamEnd::Indexed),
            PrepareStream::FirstWindow => first_commit()
                .map(StreamEnd::Committed)
                .map_err(|_| StreamStop::Failed { failure: None }),
            PrepareStream::Transcript => self
                .outcome
                .lock()
                .ok()
                .and_then(|mut outcome| outcome.take())
                .unwrap_or(Err(StreamStop::Failed { failure: None }))
                .map(StreamEnd::Committed),
        })
    }

    fn before_start(
        &self,
        _mode: JobMode,
    ) -> impl std::future::Future<Output = Result<(), StreamStop>> + Send {
        std::future::ready(
            self.wait
                .lock()
                .ok()
                .and_then(|mut wait| wait.take())
                .unwrap_or(Ok(())),
        )
    }

    fn cancel(&self, job: &JobId) -> impl std::future::Future<Output = ()> + Send {
        if let Ok(mut cancelled) = self.cancelled.lock() {
            cancelled.push(job.clone());
        }
        std::future::ready(())
    }
}

/// A queued `prepare` job in the store, as `job start` leaves it.
fn queued(store: &MemoryStore, mode: JobMode) -> Built<JobId> {
    let spec = prepare_spec(&session()?, mode, None, None)?;
    let (owner, created) = store.open_or_create(&spec, NOW)?;
    assert!(created);
    drop(owner);
    Ok(spec.job_id)
}

fn state(store: &MemoryStore, job: &JobId) -> Built<JobState> {
    Ok(store
        .job(&session()?, job)?
        .ok_or("no such job")?
        .record
        .state)
}

#[test]
fn a_prepare_job_derives_from_its_session_mode_and_base() -> TestResult {
    let session = session()?;
    let base = vsift_domain::TranscriptRevisionId::parse("trv_0123456789abcdef0123456789abcdef")?;
    let watched = prepare_spec(&session, JobMode::Watched, None, None)?;
    assert_eq!(
        watched,
        prepare_spec(&session, JobMode::Watched, None, None)?
    );
    assert_eq!(watched.request.kind(), vsift_domain::JobKind::Prepare);
    assert_eq!(watched.request.mode(), Some(JobMode::Watched));
    assert_eq!(watched.planned_chunks, None);
    let quick = prepare_spec(&session, JobMode::Quick, None, None)?;
    let later = prepare_spec(&session, JobMode::Watched, None, Some(&base))?;
    assert_ne!(watched.job_id, quick.job_id);
    assert_ne!(watched.request_digest, quick.request_digest);
    assert_ne!(watched.job_id, later.job_id);
    // The base changes the work, not the request an operation id binds.
    assert_eq!(watched.request_digest, later.request_digest);
    // Never the digest of a retranscription of the same session.
    assert_ne!(
        watched.request_digest,
        retranscribe_request_digest(&session, None)?
    );
    Ok(())
}

#[tokio::test]
async fn a_prepare_job_records_the_commit_of_its_stream() -> TestResult {
    let store = MemoryStore::default();
    let job = queued(&store, JobMode::Watched)?;
    let streams = Scripted::ending(Ok(commit()?));
    let outcome = run_prepare(&store, &session()?, &job, &FixedClock, &streams).await?;
    assert_eq!(outcome, PrepareOutcome::Succeeded(commit()?));
    assert_eq!(
        streams.ran(),
        [PrepareStream::Visual, PrepareStream::Transcript]
    );
    let record = store.job(&session()?, &job)?.ok_or("no such job")?.record;
    assert_eq!(record.state, JobState::Succeeded);
    assert_eq!(record.commit, Some(commit()?));
    // Asked again, it answers with its commit and runs nothing.
    let again = Scripted::ending(Err(StreamStop::Failed { failure: None }));
    let outcome = run_prepare(&store, &session()?, &job, &FixedClock, &again).await?;
    assert_eq!(outcome, PrepareOutcome::AlreadySucceeded(commit()?));
    assert_eq!(again.ran(), [] as [PrepareStream; 0]);
    Ok(())
}

#[tokio::test]
async fn an_interrupted_stream_leaves_the_prepare_job_resumable() -> TestResult {
    let store = MemoryStore::default();
    let job = queued(&store, JobMode::Watched)?;
    let failure = AttemptFailure {
        chunk: Some(2),
        code: FailureCode::Busy,
    };
    let interrupted = Scripted::ending(Err(StreamStop::Interrupted {
        job: Some(stream_job()?),
        failure: Some(failure),
    }));
    let outcome = run_prepare(&store, &session()?, &job, &FixedClock, &interrupted).await?;
    assert!(matches!(
        outcome,
        PrepareOutcome::Stopped(StreamStop::Interrupted { .. })
    ));
    assert_eq!(interrupted.cancelled(), [] as [JobId; 0]);
    let record = store.job(&session()?, &job)?.ok_or("no such job")?.record;
    assert_eq!(record.state, JobState::Interrupted);
    assert_eq!(record.failures, [failure]);
    // A resume runs the stream again and succeeds.
    let resumed = Scripted::ending(Ok(commit()?));
    let outcome = run_prepare(&store, &session()?, &job, &FixedClock, &resumed).await?;
    assert_eq!(outcome, PrepareOutcome::Succeeded(commit()?));
    assert_eq!(
        store.job(&session()?, &job)?.ok_or("gone")?.record.attempt,
        2
    );
    Ok(())
}

/// `job cancel` of a running `prepare` job stops its stream; the stream's job,
/// which stops interrupted, is cancelled so that what it published is sealed.
#[tokio::test]
async fn a_cancel_while_a_stream_runs_cancels_the_stream_job_too() -> TestResult {
    let store = MemoryStore::default();
    let job = queued(&store, JobMode::Watched)?;
    let streams = Scripted {
        cancel_prepare: Some((store.clone(), job.clone())),
        ..Scripted::ending(Err(StreamStop::Interrupted {
            job: Some(stream_job()?),
            failure: None,
        }))
    };
    let outcome = run_prepare(&store, &session()?, &job, &FixedClock, &streams).await?;
    assert_eq!(outcome, PrepareOutcome::Stopped(StreamStop::Cancelled));
    assert_eq!(streams.cancelled(), [stream_job()?]);
    assert_eq!(state(&store, &job)?, JobState::Cancelled);
    Ok(())
}

#[tokio::test]
async fn a_stream_cancelled_elsewhere_or_failed_ends_the_prepare_job() -> TestResult {
    let store = MemoryStore::default();
    let job = queued(&store, JobMode::Watched)?;
    let streams = Scripted::ending(Err(StreamStop::Cancelled));
    run_prepare(&store, &session()?, &job, &FixedClock, &streams).await?;
    assert_eq!(state(&store, &job)?, JobState::Cancelled);

    let store = MemoryStore::default();
    let job = queued(&store, JobMode::Quick)?;
    let streams = Scripted::ending(Err(StreamStop::Failed { failure: None }));
    run_prepare(&store, &session()?, &job, &FixedClock, &streams).await?;
    assert_eq!(state(&store, &job)?, JobState::Failed);
    // A failed job is not resumed; `job start` starts it again.
    let again = Scripted::ending(Ok(commit()?));
    let error = run_prepare(&store, &session()?, &job, &FixedClock, &again).await;
    assert_eq!(
        error,
        Err(PrepareError::NotResumable {
            job: job.clone(),
            state: JobState::Failed
        })
    );
    assert_eq!(again.ran(), [] as [PrepareStream; 0]);
    Ok(())
}

#[tokio::test]
async fn a_prepare_job_another_process_runs_is_busy() -> TestResult {
    let store = MemoryStore::default();
    let job = queued(&store, JobMode::Watched)?;
    let held = store.acquire(&session()?, &job)?;
    let streams = Scripted::ending(Ok(commit()?));
    let error = run_prepare(&store, &session()?, &job, &FixedClock, &streams).await;
    assert_eq!(error, Err(PrepareError::Busy { job: job.clone() }));
    assert_eq!(streams.ran(), [] as [PrepareStream; 0]);
    drop(held);
    Ok(())
}

/// D5: a job with a first window runs it before the visual stream and the
/// transcript, hands it the window, and records the whole transcript's
/// commit, not the window's.
#[tokio::test]
async fn a_first_window_runs_first_and_the_whole_transcript_is_recorded() -> TestResult {
    let store = MemoryStore::default();
    let window = vsift_domain::TimeRange::new(
        vsift_domain::MediaTime::from_micros(25_000_000),
        vsift_domain::MediaTime::from_micros(85_000_000),
    )?;
    let spec = prepare_spec(&session()?, JobMode::Watched, Some(window), None)?;
    assert_eq!(spec.request.first(), Some(window));
    assert_ne!(
        spec.job_id,
        prepare_spec(&session()?, JobMode::Watched, None, None)?.job_id
    );
    drop(store.open_or_create(&spec, NOW)?);
    let streams = Scripted::ending(Ok(commit()?));
    let outcome = run_prepare(&store, &session()?, &spec.job_id, &FixedClock, &streams).await?;
    assert_eq!(outcome, PrepareOutcome::Succeeded(commit()?));
    assert_eq!(
        streams.ran(),
        [
            PrepareStream::FirstWindow,
            PrepareStream::Visual,
            PrepareStream::Transcript
        ]
    );
    assert!(
        streams
            .first_seen
            .lock()
            .map_err(|_| "poisoned")?
            .iter()
            .all(|seen| *seen == Some(window))
    );
    Ok(())
}

/// ADR 0033 section 6: a queued job stays queued while it waits its turn; a
/// cancellation then cancels it, and a stopped runner leaves it queued for the
/// next runner, which starts it once its turn comes.
#[tokio::test]
async fn a_queued_job_waits_queued_and_a_cancel_while_waiting_cancels_it() -> TestResult {
    let store = MemoryStore::default();
    let job = queued(&store, JobMode::Queued)?;
    let stopped = Scripted {
        wait: Mutex::new(Some(Err(StreamStop::Interrupted {
            job: None,
            failure: None,
        }))),
        ..Scripted::ending(Ok(commit()?))
    };
    let outcome = run_prepare(&store, &session()?, &job, &FixedClock, &stopped).await?;
    assert!(matches!(
        outcome,
        PrepareOutcome::Stopped(StreamStop::Interrupted { .. })
    ));
    assert_eq!(stopped.ran(), [] as [PrepareStream; 0]);
    assert_eq!(state(&store, &job)?, JobState::Queued);
    let record = store.job(&session()?, &job)?.ok_or("no job")?.record;
    assert_eq!(record.attempt, 0, "a wait is no attempt");

    let cancelled = Scripted {
        wait: Mutex::new(Some(Err(StreamStop::Cancelled))),
        ..Scripted::ending(Ok(commit()?))
    };
    let outcome = run_prepare(&store, &session()?, &job, &FixedClock, &cancelled).await?;
    assert_eq!(outcome, PrepareOutcome::Stopped(StreamStop::Cancelled));
    assert_eq!(state(&store, &job)?, JobState::Cancelled);

    // Its turn comes: the next job runs its streams.
    let store = MemoryStore::default();
    let job = queued(&store, JobMode::Queued)?;
    let turn = Scripted::ending(Ok(commit()?));
    let outcome = run_prepare(&store, &session()?, &job, &FixedClock, &turn).await?;
    assert_eq!(outcome, PrepareOutcome::Succeeded(commit()?));
    Ok(())
}
