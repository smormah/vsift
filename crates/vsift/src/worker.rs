//! Worker requests: the engine side of `job run` (P11 PR 3, ADR 0021
//! sections 2, 4 and 6).
//!
//! [`Engine::run_work_request`] runs one decoded [`WorkRequest`] in a worker
//! workspace. Each step is one existing engine operation, made idempotent:
//!
//! - **ingest** (the target): the session id is allocated and recorded in the
//!   request record before the copy starts; the source and a supplied
//!   transcript are opened inside the operator's input root, following no
//!   link. A continuation that finds the recorded session open adopts it; one
//!   that finds it unfinished records a new id and starts again, and the
//!   abandoned registration is removed by normal cleanup.
//! - **retranscribe** runs [`Engine::retranscribe`] under the operation id
//!   `op_` + 32 hexadecimal digits of SHA-256(`vsift.job-step.v1`, the
//!   request's operation id, the step index), so a retried request continues
//!   or replays its P10 job exactly once.
//! - **candidates** calls [`Engine::candidates`] until no window of the range
//!   is left unanalysed (each call analyses at most 30); what stays uncovered
//!   is the step's coverage and makes it `partial`.
//! - **retain** retains into `<bundle-root>/<bundle_name>` and records the
//!   bundle's manifest digest; a continuation that finds the directory
//!   accepts it only when it validates as this session's bundle.
//! - **close**: closing a closed session is success.
//!
//! **Order.** Everything that can refuse the request without writing runs
//! first: the workspace, a recorded result (replayed at once, with no other
//! check) or a conflicting digest, the durability, the bundle root, the
//! paths inside the input root and the target session. Only then is the
//! request claimed (its owner lock) and its record written, before any step.
//!
//! **Retries and deadline (X-09).** A step that fails with `BUSY` is run
//! again after a full-jitter backoff within the host's admission wait and
//! the request's deadline; nothing else is retried here. A step never starts
//! with less than [`MIN_STEP_BUDGET`](vsift_domain::MIN_STEP_BUDGET) of the
//! deadline left, and a step still running at the deadline is cancelled at
//! its next boundary: both end the request `DEADLINE_EXCEEDED`, resumable.
//!
//! **Shutdown (D4).** The run's `stop` signal stops it before its next step;
//! its `cancellation` stops the running step at its next boundary. A host
//! fires the first on a shutdown signal and the second when its drain time
//! is over. Either way the request stays resumable: its record keeps the
//! finished steps and the next delivery continues from the first unfinished
//! one.

use std::{
    num::{NonZeroU16, NonZeroU32},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use vsift_application::{RetryTimer, SessionStorageError, worker_step_operation_id};
use vsift_contract::{
    BundleSourceInclusion, FreeSpaceReserve, LifecycleResponse, MAX_REQUEST_DEADLINE_MS, Readiness,
    RequestDurability, RequestFailure, RequestRejection, ResourceLimits, ResultOrigin, StepOutputs,
    StepResult, StepTiming, WorkControls, WorkFailure, WorkRequest, WorkResult, WorkResultParts,
    WorkStep, WorkStepKind, WorkTarget, WorkerIsolation,
};
use vsift_domain::{
    ADMISSION_RETRY_AFTER, AdmissionWait, CoverageGapReason, DurabilityRequirement, FailureCode,
    JobId, ProgressStage, ProgressUpdate, RecordedRequest, RequestAdmission, SessionId,
    SessionPhase, Sha256Hex, StepRetry, TimeRange, VisualCoverageGap, WorkspacePolicy,
    admit_request, ends_request, step_may_start, step_retry,
};
use vsift_infrastructure::{
    ContainedPathError, DeadlineOutcome, FilesystemSessionStore, InputRoot, RecordedRequestResult,
    RequestRecordWrite, SessionRootProvisioning, TokioRetryTimer, WorkerRequestClaim,
    WorkerRequestOwner, WorkerRequestRecord, run_until_deadline, sleep_unless_cancelled,
};

use crate::{
    CandidatesRange, CandidatesRequest, RetranscribeRange, RetranscribeRequest,
    engine::{Engine, HostIsolation},
    error::{EngineError, SessionRootError, WorkerFailure},
    progress::{AdmissionWaiting, JobProgress, ProgressObserver},
    sessions::{IngestSource, IngestTranscript, PreparedIngest, SourceRetention},
    verification::Cancellation,
};

/// Most `candidates` calls one step makes: a source of the four-hour visual
/// bound needs eight at 30 windows each; the rest is headroom for windows a
/// call could not finish.
pub const MAX_CANDIDATE_CALLS: usize = 16;
/// The end of an unbounded candidates range: past any source, so the
/// operation clips it to the video.
const WHOLE_SOURCE_END: u64 = u64::MAX;

/// One worker request to run, with the host's controls.
pub struct WorkRequestRun {
    /// The decoded, validated request.
    pub request: WorkRequest,
    /// The operator's absolute input root: the only place the request's
    /// paths may name.
    pub input_root: PathBuf,
    /// The operator's absolute bundle root, needed by a `retain` step.
    pub bundle_root: Option<PathBuf>,
    /// How a step waits for admission capacity, and so how long a busy step
    /// is retried: [`AdmissionWait::Bounded`] (at most 60 s) for a worker
    /// host, [`AdmissionWait::Immediate`] to report contention at once.
    pub admission: AdmissionWait,
    /// The host's request concurrency, reported in the result's controls.
    pub concurrency: NonZeroU16,
    /// Fired by a shutdown: the request starts no further step.
    pub stop: Cancellation,
    /// Stops the running step at its next boundary (a shutdown once its
    /// drain time is over, or a second signal).
    pub cancellation: Cancellation,
    /// Receives `running_request` progress in steps, the recognition's
    /// chunk progress and admission waits.
    pub progress: ProgressObserver,
}

/// What a request came to: its result, and the typed failure that ended it
/// (for a host's remediation), if one did.
#[derive(Debug)]
pub struct WorkOutcome {
    result: WorkResult,
    cause: Option<EngineError>,
    stopped: bool,
}

impl WorkOutcome {
    /// The request's result (`job-result`).
    #[must_use]
    pub const fn result(&self) -> &WorkResult {
        &self.result
    }

    /// The typed failure behind the result's failure, when the request
    /// failed or was cancelled.
    #[must_use]
    pub const fn cause(&self) -> Option<&EngineError> {
        self.cause.as_ref()
    }

    /// Whether the host's shutdown (the run's `stop` or `cancellation`)
    /// ended the request before all its steps ran.
    #[must_use]
    pub const fn stopped_by_shutdown(&self) -> bool {
        self.stopped
    }

    /// The result and its cause.
    #[must_use]
    pub fn into_parts(self) -> (WorkResult, Option<EngineError>) {
        (self.result, self.cause)
    }
}

/// One step of a request as it runs: the ingest of an ingest target, then
/// each requested step with its index in the request.
#[derive(Clone, Debug)]
enum PlannedStep {
    Ingest,
    Requested { index: usize, step: WorkStep },
}

impl PlannedStep {
    const fn kind(&self) -> WorkStepKind {
        match self {
            Self::Ingest => WorkStepKind::Ingest,
            Self::Requested { step, .. } => step.kind(),
        }
    }
}

/// Everything a run needs across its steps.
struct RequestContext<'run> {
    run: &'run WorkRequestRun,
    store: FilesystemSessionStore,
    policy: WorkspacePolicy,
    digest: Sha256Hex,
    deadline: Instant,
    work: Cancellation,
    timer: TokioRetryTimer,
}

/// The request record as this attempt keeps it.
struct Progress {
    owner: WorkerRequestOwner,
    attempt: NonZeroU32,
    created_at: u64,
    session: Option<SessionId>,
    steps: Vec<StepResult>,
}

/// A step that failed, with what it had done.
struct StepFailed {
    error: EngineError,
    timing: StepTiming,
    job: Option<JobId>,
}

/// What one run of a step produced.
struct StepDone {
    outputs: StepOutputs,
    admission_wait: Duration,
    job: Option<JobId>,
}

/// A failed run of a step, with the job it ran, if any.
struct StepError {
    error: EngineError,
    job: Option<JobId>,
}

impl From<EngineError> for StepError {
    fn from(error: EngineError) -> Self {
        Self { error, job: None }
    }
}

impl Engine {
    /// Runs one worker request in the worker workspace this engine's
    /// session root names (see the module documentation).
    ///
    /// It never fails outright: every outcome, refusals included, is a
    /// [`WorkResult`] (`complete`, `partial`, `failed` or `cancelled`), with
    /// the typed failure behind a failure in [`WorkOutcome::cause`].
    pub async fn run_work_request(&self, run: WorkRequestRun) -> WorkOutcome {
        let started = Instant::now();
        let limit = run.request.deadline().map_or(
            MAX_REQUEST_DEADLINE_MS,
            vsift_contract::RequestDeadline::as_millis,
        );
        let deadline = started + Duration::from_millis(limit);
        let Ok(digest) = Sha256Hex::parse(run.request.digest().as_str()) else {
            return self.refused(&run, None, EngineError::JobInvariant);
        };
        let (store, policy) = match self.worker_store() {
            Ok(opened) => opened,
            Err(error) => return self.refused(&run, None, error),
        };
        let context = RequestContext {
            run: &run,
            store,
            policy,
            digest,
            deadline,
            work: run.cancellation.child(),
            timer: TokioRetryTimer,
        };
        let mut outcome = self.run_in_workspace(&context).await;
        outcome.stopped = outcome.result.failure_code().is_some()
            && (run.stop.is_cancelled() || run.cancellation.is_cancelled());
        if outcome.result.failure_code().is_none() {
            outcome.cause = None;
        }
        outcome
    }

    /// What a worker host states when it starts (O-03): the guarantee the
    /// workspace's sessions get, the isolation it runs under, its admission
    /// capacity and the host's `concurrency`.
    ///
    /// # Errors
    ///
    /// As a request would fail: a root that is missing, unreadable or not a
    /// worker workspace.
    pub fn worker_readiness(&self, concurrency: NonZeroU16) -> Result<Readiness, EngineError> {
        let (store, policy) = self.worker_store()?;
        let isolation = match self.config().host_isolation {
            HostIsolation::ProcessOnly => WorkerIsolation::ProcessOnly,
            #[cfg(target_os = "linux")]
            HostIsolation::StrictLinux => WorkerIsolation::StrictLinux,
        };
        Ok(Readiness {
            publication: policy.durability().required_guarantee(),
            isolation,
            admission_capacity: NonZeroU16::new(store.admission_capacity())
                .unwrap_or(NonZeroU16::MIN),
            concurrency,
        })
    }

    /// The session root as a worker workspace, which it must already be.
    fn worker_store(&self) -> Result<(FilesystemSessionStore, WorkspacePolicy), EngineError> {
        let root = self.session_root_path()?;
        let store = self
            .open_session_store(&root, SessionRootProvisioning::ExistingOnly)?
            .ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let policy = store
            .workspace_policy()
            .ok_or(EngineError::Worker(WorkerFailure::WorkspaceRequired))?;
        Ok((store, policy))
    }

    async fn run_in_workspace(&self, context: &RequestContext<'_>) -> WorkOutcome {
        let run = context.run;
        match self.admit(context) {
            Ok(Admitted::Replay(result)) => self.replay(context, &result),
            Ok(Admitted::Run(progress)) => self.run_steps(context, *progress).await,
            Err(Refusal::Rejected(rejection)) => self.rejected(run, context, rejection),
            Err(Refusal::Failed(error)) => self.refused(run, Some(context), error),
        }
    }

    /// Everything before the first step: the checks that need no write in
    /// their contract order, then the claim and the attempt's record.
    #[allow(
        clippy::too_many_lines,
        reason = "The refusal order is the contract; keep it visible in one place"
    )]
    fn admit(&self, context: &RequestContext<'_>) -> Result<Admitted, Refusal> {
        let run = context.run;
        let request = &run.request;
        let operation_id = request.operation_id();

        // 1. A known operation id is answered from its record first: a
        // recorded result is replayed before anything else is checked (the
        // inputs may be gone), and another request's digest is a conflict.
        let peeked = context
            .store
            .read_worker_request(operation_id)
            .map_err(EngineError::from)?;
        if let Some(record) = &peeked {
            if record.request_digest != context.digest {
                return Err(EngineError::Worker(WorkerFailure::Conflict).into());
            }
            if let Some(result) = &record.result {
                return Ok(Admitted::Replay(result.clone()));
            }
        }

        // 2. Refusals that need no write.
        if request.durability() == RequestDurability::Durable
            && context.policy.durability() == DurabilityRequirement::Ephemeral
        {
            return Err(Refusal::Rejected(RequestRejection::WorkspaceNotDurable));
        }
        let retains = request
            .steps()
            .iter()
            .any(|step| matches!(step, WorkStep::Retain { .. }));
        if retains
            && !run
                .bundle_root
                .as_deref()
                .is_some_and(|root| root.is_absolute() && root.is_dir())
        {
            return Err(EngineError::Worker(WorkerFailure::BundleRootRequired).into());
        }
        let ingest_recorded = peeked
            .as_ref()
            .is_some_and(|record| !record.steps.is_empty());
        match request.target() {
            WorkTarget::Ingest { source, transcript } if !ingest_recorded => {
                let root = open_input_root(&run.input_root)?;
                root.open_file(source.as_str()).map_err(Refusal::from)?;
                if let Some(transcript) = transcript {
                    root.open_file(transcript.path().as_str())
                        .map_err(Refusal::from)?;
                }
            }
            WorkTarget::Ingest { .. } => {}
            WorkTarget::Session(session) => {
                if context
                    .store
                    .indexed_session_status(session)
                    .map_err(EngineError::from)?
                    .is_none()
                {
                    return Err(EngineError::Worker(WorkerFailure::SessionNotFound).into());
                }
            }
        }
        if run.stop.is_cancelled() {
            return Err(EngineError::Worker(WorkerFailure::Stopped).into());
        }

        // 3. Claim the request; its record, read under the lock, decides.
        let owner = match context
            .store
            .claim_worker_request(operation_id)
            .map_err(EngineError::from)?
        {
            WorkerRequestClaim::Owned(owner) => owner,
            WorkerRequestClaim::HeldElsewhere(record) => {
                let held = record
                    .as_ref()
                    .map(|record| recorded(record, &context.digest));
                return Err(EngineError::Worker(match admit_request(held, true) {
                    RequestAdmission::Conflict => WorkerFailure::Conflict,
                    RequestAdmission::Start
                    | RequestAdmission::Replay
                    | RequestAdmission::Continue
                    | RequestAdmission::Busy => WorkerFailure::Busy,
                })
                .into());
            }
        };
        let admission = admit_request(
            owner
                .record()
                .map(|record| recorded(record, &context.digest)),
            false,
        );
        let mut progress = match admission {
            RequestAdmission::Replay => {
                let result = owner
                    .record()
                    .and_then(|record| record.result.clone())
                    .ok_or(EngineError::JobInvariant)?;
                return Ok(Admitted::Replay(result));
            }
            RequestAdmission::Conflict => {
                return Err(EngineError::Worker(WorkerFailure::Conflict).into());
            }
            RequestAdmission::Busy => {
                return Err(EngineError::Worker(WorkerFailure::Busy).into());
            }
            RequestAdmission::Start => Progress {
                owner,
                attempt: NonZeroU32::MIN,
                created_at: self.now_unix_seconds()?,
                session: None,
                steps: Vec::new(),
            },
            RequestAdmission::Continue => {
                let record = owner.record().ok_or(EngineError::JobInvariant)?;
                let steps = decode_steps(&record.steps)?;
                Progress {
                    attempt: record.attempt.saturating_add(1),
                    created_at: record.created_at_unix_seconds,
                    session: record.session_id.clone(),
                    steps,
                    owner,
                }
            }
        };

        // 4. The session: a session target's own; an ingest target's
        // recorded id, adopted when that session opened, else a new one.
        let mut adopted = None;
        match request.target() {
            WorkTarget::Session(target) => progress.session = Some(target.clone()),
            WorkTarget::Ingest { transcript, .. } if progress.steps.is_empty() => {
                adopted = progress.session.as_ref().and_then(|recorded| {
                    self.adoptable_ingest(context, recorded, transcript.is_some())
                });
                if adopted.is_none() {
                    progress.session = Some(self.new_session_id()?);
                }
            }
            WorkTarget::Ingest { .. } => {}
        }
        self.write_record(context, &mut progress, RequestRecordWrite::Accept)?;
        if let Some(ingest) = adopted {
            progress.steps.push(ingest);
            self.write_record(context, &mut progress, RequestRecordWrite::Step)?;
        }
        Ok(Admitted::Run(Box::new(progress)))
    }

    /// Runs the steps from the first unfinished one and presents the
    /// result; an ended request records it for every replay.
    async fn run_steps(&self, context: &RequestContext<'_>, mut progress: Progress) -> WorkOutcome {
        let run = context.run;
        let plan = plan(&run.request);
        let total = u64::try_from(plan.len()).unwrap_or(u64::MAX);
        let mut cause = None;
        report_steps(&run.progress, progress.steps.len(), total);
        while let Some(planned) = plan.get(progress.steps.len()) {
            let kind = planned.kind();
            if run.stop.is_cancelled() || context.work.is_cancelled() {
                progress.steps.push(StepResult::failed(
                    kind,
                    StepTiming::default(),
                    None,
                    WorkFailure::new(FailureCode::Cancelled, None),
                ));
                cause = Some(EngineError::Worker(WorkerFailure::Stopped));
                break;
            }
            if !step_may_start(context.deadline.saturating_duration_since(Instant::now())) {
                progress.steps.push(StepResult::failed(
                    kind,
                    StepTiming::default(),
                    None,
                    WorkFailure::new(FailureCode::DeadlineExceeded, None),
                ));
                cause = Some(EngineError::Worker(WorkerFailure::DeadlineExceeded));
                break;
            }
            match self
                .run_step_retrying(context, &mut progress, planned)
                .await
            {
                Ok(step) => {
                    progress.steps.push(step);
                    if let Err(error) =
                        self.write_record(context, &mut progress, RequestRecordWrite::Step)
                    {
                        // The step's work is committed but the record could
                        // not say so; the next delivery adopts or replays
                        // it. The step is reported as the failure.
                        progress.steps.pop();
                        progress.steps.push(StepResult::failed(
                            kind,
                            StepTiming::default(),
                            None,
                            failure_of(&error),
                        ));
                        cause = Some(error);
                        break;
                    }
                    report_steps(&run.progress, progress.steps.len(), total);
                }
                Err(failed) => {
                    progress.steps.push(StepResult::failed(
                        kind,
                        failed.timing,
                        failed.job.as_ref(),
                        failure_of(&failed.error),
                    ));
                    cause = Some(failed.error);
                    break;
                }
            }
        }
        let mut steps = progress.steps.clone();
        for planned in plan.iter().skip(steps.len()) {
            steps.push(StepResult::not_started(planned.kind()));
        }
        let result = match self.present(context, progress.attempt, progress.session.as_ref(), steps)
        {
            Ok(result) => result,
            Err(error) => return self.refused(run, Some(context), error),
        };
        if ends_request(result.failure_code())
            && let Err(error) = self.record_result(context, &mut progress, &result)
        {
            // The work is committed; only its record failed. The result
            // stands, and the next delivery finds every step recorded and
            // records the result then.
            cause.get_or_insert(error);
        }
        WorkOutcome {
            result,
            cause,
            stopped: false,
        }
    }

    /// Runs one step, again after a jittered backoff while it answers
    /// `BUSY` within the admission wait and the deadline (X-09).
    async fn run_step_retrying(
        &self,
        context: &RequestContext<'_>,
        progress: &mut Progress,
        planned: &PlannedStep,
    ) -> Result<StepResult, StepFailed> {
        let run = context.run;
        let started = Instant::now();
        let mut waited = Duration::ZERO;
        let mut polls = 0_u32;
        loop {
            let session = progress.session.clone();
            let (ran, deadline) = run_until_deadline(
                context.deadline,
                &context.work.0,
                // Boxed: the operations a step maps to are large futures.
                Box::pin(self.run_step(context, planned, session.as_ref())),
            )
            .await;
            let failed = match ran {
                Ok(done) => {
                    return Ok(StepResult::finished(
                        &done.outputs,
                        StepTiming {
                            elapsed_ms: millis(started.elapsed()),
                            admission_wait_ms: millis(waited.saturating_add(done.admission_wait)),
                        },
                        done.job.as_ref(),
                    ));
                }
                Err(failed) => failed,
            };
            let stop = |error: EngineError, waited: Duration| StepFailed {
                error,
                timing: StepTiming {
                    elapsed_ms: millis(started.elapsed()),
                    admission_wait_ms: millis(waited),
                },
                job: failed.job.clone(),
            };
            if deadline == DeadlineOutcome::Passed {
                return Err(stop(
                    EngineError::Worker(WorkerFailure::DeadlineExceeded),
                    waited,
                ));
            }
            // A recognition waits for its admission inside the job (the
            // same bounded wait); its `BUSY` after that wait is reported.
            let retryable = !matches!(failed.error, EngineError::AdmissionBusy { .. })
                || planned.kind() != WorkStepKind::Retranscribe;
            let decision = if retryable {
                step_retry(
                    failed.error.failure_code(),
                    polls,
                    waited,
                    context.deadline.saturating_duration_since(Instant::now()),
                    run.admission,
                    context.timer.jitter(),
                )
            } else {
                StepRetry::Report
            };
            match decision {
                StepRetry::Report => return Err(stop(failed.error.clone(), waited)),
                StepRetry::DeadlineExceeded => {
                    return Err(stop(
                        EngineError::Worker(WorkerFailure::DeadlineExceeded),
                        waited,
                    ));
                }
                StepRetry::After(delay) => {
                    if polls == 0 {
                        run.progress.admission_waiting(&AdmissionWaiting {
                            job: failed.job.clone(),
                            weight: NonZeroU16::MIN,
                        });
                    }
                    if !sleep_unless_cancelled(delay, &context.work.0).await {
                        // A shutdown (or the host) cancelled the step while
                        // it waited to try again.
                        return Err(stop(EngineError::Worker(WorkerFailure::Stopped), waited));
                    }
                    waited = waited.saturating_add(delay);
                    polls = polls.saturating_add(1);
                    if matches!(planned, PlannedStep::Ingest) {
                        // A failed ingest may have registered its session:
                        // the next try opens a new one, recorded first.
                        progress.session =
                            Some(self.new_session_id().map_err(|error| stop(error, waited))?);
                        self.write_record(context, progress, RequestRecordWrite::Accept)
                            .map_err(|error| stop(error, waited))?;
                    }
                }
            }
        }
    }

    /// Runs one step once.
    async fn run_step(
        &self,
        context: &RequestContext<'_>,
        planned: &PlannedStep,
        session: Option<&SessionId>,
    ) -> Result<StepDone, StepError> {
        let run = context.run;
        let session = session.ok_or(EngineError::JobInvariant)?;
        match planned {
            PlannedStep::Ingest => self.ingest_step(context, session).await,
            PlannedStep::Requested { index, step } => match step {
                WorkStep::Retranscribe { range } => {
                    let operation_id = worker_step_operation_id(run.request.operation_id(), *index)
                        .map_err(|_| EngineError::JobInvariant)?;
                    let outcome = self
                        .retranscribe(RetranscribeRequest {
                            session: session.clone(),
                            range: range.map(|range| RetranscribeRange {
                                from_micros: range.start().as_micros(),
                                to_micros: range.end().as_micros(),
                            }),
                            operation_id: Some(operation_id),
                            cancellation: context.work.clone(),
                            progress: run.progress.clone(),
                            admission: run.admission,
                        })
                        .await
                        .map_err(|error| StepError {
                            job: error.affected_job().cloned(),
                            error,
                        })?;
                    Ok(StepDone {
                        outputs: StepOutputs::Retranscribe {
                            revision_id: outcome.revision().id().clone(),
                            generation: outcome.session().generation(),
                            chunks_reused: outcome.job().chunks_reused(),
                        },
                        admission_wait: outcome.job().admission_wait(),
                        job: Some(outcome.job().job_id().clone()),
                    })
                }
                WorkStep::Candidates { range } => {
                    self.candidates_step(context, session, *range).await
                }
                WorkStep::Retain { bundle, source } => {
                    let root = run
                        .bundle_root
                        .as_deref()
                        .ok_or(EngineError::Worker(WorkerFailure::BundleRootRequired))?;
                    let retention = match source {
                        BundleSourceInclusion::SourceIncluded => SourceRetention::IncludeSource,
                        BundleSourceInclusion::EvidenceOnly => SourceRetention::EvidenceOnly,
                    };
                    let summary =
                        self.retain_step(session, &root.join(bundle.as_str()), retention)?;
                    Ok(StepDone {
                        outputs: StepOutputs::Retain {
                            bundle_name: bundle.clone(),
                            bundle_sha256: Sha256Hex::parse(summary.manifest_sha256())
                                .map_err(|_| EngineError::JobInvariant)?,
                            artifact_count: u32::try_from(summary.artifact_count())
                                .unwrap_or(u32::MAX),
                        },
                        admission_wait: Duration::ZERO,
                        job: None,
                    })
                }
                WorkStep::Close => {
                    let current = self.session_status(session)?;
                    let closed = if current.phase() == SessionPhase::Closed {
                        current
                    } else {
                        self.close_session(session)?
                    };
                    Ok(StepDone {
                        outputs: StepOutputs::Close {
                            generation: closed.generation(),
                        },
                        admission_wait: Duration::ZERO,
                        job: None,
                    })
                }
            },
        }
    }

    /// The ingest of an ingest target into `session`, from files opened
    /// again inside the input root for this try.
    async fn ingest_step(
        &self,
        context: &RequestContext<'_>,
        session: &SessionId,
    ) -> Result<StepDone, StepError> {
        let run = context.run;
        let WorkTarget::Ingest { source, transcript } = run.request.target() else {
            return Err(EngineError::JobInvariant.into());
        };
        // The copy's admission first: a busy root is then retried before
        // any session is registered.
        let admission = context.store.try_admit(1).map_err(EngineError::from)?;
        let root = open_input_root(&run.input_root).map_err(EngineError::from)?;
        let source = root
            .open_file(source.as_str())
            .map_err(|error| EngineError::from(InputFailure::from(error)))?;
        let transcript = match transcript {
            Some(transcript) => Some(IngestTranscript::Contained {
                file: root
                    .open_file(transcript.path().as_str())
                    .map_err(|error| EngineError::from(InputFailure::from(error)))?,
                offset_micros: transcript.offset().as_micros(),
            }),
            None => None,
        };
        let outcome = self
            .ingest_prepared(PreparedIngest {
                source: IngestSource::Contained {
                    file: source,
                    admission: Some(admission),
                },
                transcript,
                cancellation: context.work.clone(),
                durability: match run.request.durability() {
                    RequestDurability::Durable => DurabilityRequirement::Durable,
                    RequestDurability::Ephemeral => DurabilityRequirement::Ephemeral,
                },
                session_id: Some(session.clone()),
            })
            .await?;
        Ok(StepDone {
            outputs: StepOutputs::Ingest {
                generation: outcome.session.generation,
                revision_id: outcome
                    .transcript
                    .as_ref()
                    .map(|revision| revision.id().clone()),
            },
            admission_wait: Duration::ZERO,
            job: None,
        })
    }

    /// Candidates of the range, analysed until no window is left
    /// unanalysed, at most [`MAX_CANDIDATE_CALLS`] calls.
    async fn candidates_step(
        &self,
        context: &RequestContext<'_>,
        session: &SessionId,
        range: Option<TimeRange>,
    ) -> Result<StepDone, StepError> {
        let range = CandidatesRange {
            from_micros: range.map_or(0, |range| range.start().as_micros()),
            to_micros: range.map_or(WHOLE_SOURCE_END, |range| range.end().as_micros()),
        };
        let mut calls = 0;
        loop {
            let results = self
                .candidates(CandidatesRequest {
                    session: session.clone(),
                    range,
                    limit: None,
                    cursor: None,
                    cancellation: context.work.clone(),
                })
                .await?;
            calls += 1;
            let unanalysed = results.gaps().iter().any(|gap| {
                matches!(
                    gap.reason,
                    CoverageGapReason::NotAnalyzed | CoverageGapReason::DeadlineExceeded
                )
            });
            if unanalysed && context.work.is_cancelled() {
                // Windows analysed before the stop are committed; the rest
                // wait for the next delivery.
                return Err(EngineError::Worker(WorkerFailure::Stopped).into());
            }
            if !unanalysed || results.analysed_now() == 0 || calls >= MAX_CANDIDATE_CALLS {
                let gaps: Vec<VisualCoverageGap> = results.gaps().to_vec();
                return Ok(StepDone {
                    outputs: StepOutputs::Candidates {
                        visual_index_id: results.index().id().clone(),
                        generation: results.session().generation(),
                        candidate_count: u32::try_from(
                            results.index().candidates_in(results.searched()).count(),
                        )
                        .unwrap_or(u32::MAX),
                        gaps,
                    },
                    admission_wait: Duration::ZERO,
                    job: None,
                });
            }
        }
    }

    /// Retains `session` into `output`, or accepts the bundle already
    /// there when it validates as this session's bundle with the requested
    /// source inclusion and the session's artifacts (a try that stopped
    /// after retaining but before its record said so).
    fn retain_step(
        &self,
        session: &SessionId,
        output: &Path,
        retention: SourceRetention,
    ) -> Result<crate::BundleSummary, EngineError> {
        if !output.exists() {
            return self.retain_session(session, output, retention);
        }
        let mismatch = EngineError::Worker(WorkerFailure::BundleMismatch);
        let bundle = self.validate_bundle(output).map_err(|_| mismatch.clone())?;
        let current = self.session_status(session)?;
        if bundle.session_id() != session
            || bundle.source_id() != current.source_id()
            || bundle.source_retention() != retention
            || bundle.artifact_count() != current.artifact_count()
        {
            return Err(mismatch);
        }
        Ok(bundle)
    }

    /// The ingest step a continuation adopts: the recorded session opened
    /// (the previous attempt stopped after its activation but before its
    /// record said so), so it is this request's session. `None` when it did
    /// not open.
    fn adoptable_ingest(
        &self,
        context: &RequestContext<'_>,
        session: &SessionId,
        with_transcript: bool,
    ) -> Option<StepResult> {
        let status = context.store.indexed_session_status(session).ok()??;
        if status.phase() != SessionPhase::Open {
            return None;
        }
        let revision_id = if with_transcript {
            let now = self.now_unix_seconds().ok()?;
            let (newest, _) = context.store.read_transcript_head(session, now).ok()?;
            Some(newest?.id().clone())
        } else {
            None
        };
        Some(StepResult::finished(
            &StepOutputs::Ingest {
                generation: status.generation(),
                revision_id,
            },
            StepTiming::default(),
            None,
        ))
    }

    /// Replaces the request's record with this attempt's state.
    fn write_record(
        &self,
        context: &RequestContext<'_>,
        progress: &mut Progress,
        write: RequestRecordWrite,
    ) -> Result<(), EngineError> {
        let now = self.now_unix_seconds()?;
        let steps = progress
            .steps
            .iter()
            .map(|step| {
                step.recorded_bytes()
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .ok_or(EngineError::JobInvariant)
            })
            .collect::<Result<Vec<_>, _>>()?;
        progress.owner.write(
            WorkerRequestRecord {
                operation_id: context.run.request.operation_id().clone(),
                request_digest: context.digest.clone(),
                attempt: progress.attempt,
                created_at_unix_seconds: progress.created_at,
                updated_at_unix_seconds: now.max(progress.created_at),
                session_id: progress.session.clone(),
                steps,
                result: None,
            },
            write,
        )?;
        Ok(())
    }

    /// Records the result of an ended request.
    fn record_result(
        &self,
        context: &RequestContext<'_>,
        progress: &mut Progress,
        result: &WorkResult,
    ) -> Result<(), EngineError> {
        let document = result
            .recorded_bytes()
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or(EngineError::JobInvariant)?;
        let recorded = RecordedRequestResult::new(document)?;
        let now = self.now_unix_seconds()?;
        progress.owner.write(
            WorkerRequestRecord {
                operation_id: context.run.request.operation_id().clone(),
                request_digest: context.digest.clone(),
                attempt: progress.attempt,
                created_at_unix_seconds: progress.created_at,
                updated_at_unix_seconds: now.max(progress.created_at),
                session_id: progress.session.clone(),
                steps: Vec::new(),
                result: Some(recorded),
            },
            RequestRecordWrite::Complete,
        )?;
        Ok(())
    }

    /// A recorded result, returned without new work (`replayed: true`).
    fn replay(&self, context: &RequestContext<'_>, result: &RecordedRequestResult) -> WorkOutcome {
        match WorkResult::decode_recorded(result.document().as_bytes()) {
            Ok(recorded) => WorkOutcome {
                result: recorded.into_replayed(),
                cause: None,
                stopped: false,
            },
            Err(_) => self.refused(
                context.run,
                Some(context),
                EngineError::Storage(SessionStorageError::IntegrityFailure),
            ),
        }
    }

    /// Presents a request that ran (some of) its steps.
    fn present(
        &self,
        context: &RequestContext<'_>,
        attempt: NonZeroU32,
        session: Option<&SessionId>,
        steps: Vec<StepResult>,
    ) -> Result<WorkResult, EngineError> {
        let run = context.run;
        // A session is named once it exists: an ingest that stopped before
        // its session opened names none.
        let status = session.and_then(|session| self.session_status(session).ok());
        let session = status.as_ref().and(session);
        let publication = status
            .as_ref()
            .map(|_| context.policy.durability().required_guarantee());
        WorkResult::new(WorkResultParts {
            operation_id: run.request.operation_id(),
            request_digest: run.request.digest(),
            origin: ResultOrigin::Fresh,
            attempt,
            session_id: session,
            source_id: status.as_ref().map(crate::SessionSnapshot::source_id),
            publication,
            lifecycle: status
                .as_ref()
                .and_then(|status| LifecycleResponse::of_session(status.lifetime())),
            steps,
            failure: None,
            controls: self.controls(run, Some(context)),
        })
        .map_err(|_| EngineError::JobInvariant)
    }

    /// A request that ran no step: refused, or failed before its first.
    fn refused(
        &self,
        run: &WorkRequestRun,
        context: Option<&RequestContext<'_>>,
        error: EngineError,
    ) -> WorkOutcome {
        let failure = failure_of(&error);
        self.not_started(
            run,
            context,
            RequestFailure::NotStarted(failure),
            Some(error),
        )
    }

    /// A request refused with one of the contract's rejections.
    fn rejected(
        &self,
        run: &WorkRequestRun,
        context: &RequestContext<'_>,
        rejection: RequestRejection,
    ) -> WorkOutcome {
        self.not_started(
            run,
            Some(context),
            RequestFailure::Rejected(rejection),
            None,
        )
    }

    fn not_started(
        &self,
        run: &WorkRequestRun,
        context: Option<&RequestContext<'_>>,
        failure: RequestFailure,
        cause: Option<EngineError>,
    ) -> WorkOutcome {
        WorkOutcome {
            result: WorkResult::refused(
                run.request.operation_id(),
                run.request.digest(),
                failure,
                self.controls(run, context),
            ),
            cause,
            stopped: false,
        }
    }

    /// The controls a request ran under.
    fn controls(&self, run: &WorkRequestRun, context: Option<&RequestContext<'_>>) -> WorkControls {
        let (isolation, resource_limits) = match self.config().host_isolation {
            HostIsolation::ProcessOnly => {
                (WorkerIsolation::ProcessOnly, ResourceLimits::NotEnforced)
            }
            #[cfg(target_os = "linux")]
            HostIsolation::StrictLinux => {
                (WorkerIsolation::StrictLinux, ResourceLimits::HostCgroup)
            }
        };
        let capacity = context
            .and_then(|context| NonZeroU16::new(context.store.admission_capacity()))
            .unwrap_or_else(|| self.admission_capacity_hint());
        // Only a copy into a workspace checks the reserve, and only on Unix
        // (P11 PR 2).
        let copies = matches!(run.request.target(), WorkTarget::Ingest { .. });
        WorkControls {
            isolation,
            admission_capacity: capacity,
            concurrency: run.concurrency,
            resource_limits,
            free_space_reserve: if copies && cfg!(unix) {
                FreeSpaceReserve::Enforced
            } else {
                FreeSpaceReserve::NotEnforced
            },
        }
    }
}

/// How the checks before the first step end.
enum Admitted {
    /// The request ended earlier: its recorded result.
    Replay(RecordedRequestResult),
    /// The request runs, from the record this attempt wrote.
    Run(Box<Progress>),
}

/// Why a request does not run.
enum Refusal {
    /// One of the contract's rejections.
    Rejected(RequestRejection),
    /// Any other failure before the first step.
    Failed(EngineError),
}

impl From<EngineError> for Refusal {
    fn from(error: EngineError) -> Self {
        Self::Failed(error)
    }
}

impl From<ContainedPathError> for Refusal {
    fn from(error: ContainedPathError) -> Self {
        match InputFailure::from(error) {
            InputFailure::Rejected(rejection) => Self::Rejected(rejection),
            InputFailure::Engine(error) => Self::Failed(error),
        }
    }
}

impl From<InputFailure> for Refusal {
    fn from(failure: InputFailure) -> Self {
        match failure {
            InputFailure::Rejected(rejection) => Self::Rejected(rejection),
            InputFailure::Engine(error) => Self::Failed(error),
        }
    }
}

/// Why the request's inputs could not be opened inside the input root.
enum InputFailure {
    /// A refusal the contract names (`path_outside_input_root`).
    Rejected(RequestRejection),
    /// Any other failure.
    Engine(EngineError),
}

impl From<ContainedPathError> for InputFailure {
    fn from(error: ContainedPathError) -> Self {
        match error {
            ContainedPathError::Link => Self::Rejected(RequestRejection::PathOutsideInputRoot),
            ContainedPathError::Invalid => Self::Rejected(RequestRejection::InvalidPath),
            ContainedPathError::NotFound => {
                Self::Engine(EngineError::Worker(WorkerFailure::InputNotFound))
            }
            ContainedPathError::NotRegularFile => {
                Self::Engine(EngineError::Worker(WorkerFailure::InputNotRegularFile))
            }
            ContainedPathError::Io => {
                Self::Engine(EngineError::Worker(WorkerFailure::InputUnreadable))
            }
        }
    }
}

impl From<InputFailure> for EngineError {
    fn from(failure: InputFailure) -> Self {
        match failure {
            // Met only when a path turned into a link between the request's
            // checks and its step: the step fails as not found.
            InputFailure::Rejected(_) => Self::Worker(WorkerFailure::InputNotFound),
            InputFailure::Engine(error) => error,
        }
    }
}

fn open_input_root(path: &Path) -> Result<InputRoot, InputFailure> {
    InputRoot::open(path)
        .map_err(|_| InputFailure::Engine(EngineError::Worker(WorkerFailure::InputRootUnavailable)))
}

/// What a record says about the arriving request.
fn recorded(record: &WorkerRequestRecord, digest: &Sha256Hex) -> RecordedRequest {
    RecordedRequest {
        same_digest: record.request_digest == *digest,
        ended: record.result.is_some(),
    }
}

/// The finished steps a record lists.
fn decode_steps(steps: &[String]) -> Result<Vec<StepResult>, EngineError> {
    steps
        .iter()
        .map(|step| {
            StepResult::decode_recorded(step.as_bytes())
                .map_err(|_| EngineError::Storage(SessionStorageError::IntegrityFailure))
        })
        .collect()
}

/// The steps of a request in the order they run.
fn plan(request: &WorkRequest) -> Vec<PlannedStep> {
    let ingest = matches!(request.target(), WorkTarget::Ingest { .. });
    ingest
        .then_some(PlannedStep::Ingest)
        .into_iter()
        .chain(
            request
                .steps()
                .iter()
                .enumerate()
                .map(|(index, step)| PlannedStep::Requested {
                    index,
                    step: step.clone(),
                }),
        )
        .collect()
}

fn report_steps(observer: &ProgressObserver, completed: usize, total: u64) {
    observer.report(&JobProgress {
        job: None,
        update: ProgressUpdate {
            stage: ProgressStage::RunningRequest,
            completed: u64::try_from(completed).unwrap_or(u64::MAX),
            total: Some(total),
        },
    });
}

/// A failure as a result reports it. Every `BUSY` carries a retry hint:
/// contention a step met directly (an admission slot, a session writer) has
/// none of its own, and a supervisor needs one to back off.
fn failure_of(error: &EngineError) -> WorkFailure {
    let code = error.failure_code();
    let hint = error.retry_after_ms().or_else(|| {
        (code == FailureCode::Busy)
            .then(|| u64::try_from(ADMISSION_RETRY_AFTER.as_millis()).unwrap_or(u64::MAX))
    });
    WorkFailure::new(code, hint)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
