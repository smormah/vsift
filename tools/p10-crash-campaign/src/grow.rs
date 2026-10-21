//! Retranscriptions that publish as they go (ADR 0033): the workload's `grow`
//! and `stop` operations, the continuation of an open revision a crash left,
//! and the acknowledgement of every part the moment it is committed.
//!
//! A growing run is a whole-source retranscription with
//! [`Publication::AsItGoes`] and no interval, so the stand-in source's four
//! chunks commit three parts and then the seal. Each part is acknowledged as
//! an `ACK` of kind `part` (with a `START` before it, under a sequence number
//! of its own) as soon as its generation is committed, so a crash after it is
//! held to it. A `stop` run asks for its job to be cancelled once its first
//! part is acknowledged, as `job cancel` from another process would; the run
//! then seals what it published as a stopped revision (D1).
//!
//! A crash can leave a session with an open revision. Every job operation on
//! such a session continues it instead of starting another: it resumes the
//! open revision's job to its seal, or stops it as `job cancel` of a job no
//! process runs does (the engine's `job_cancel`), at random; a job that has
//! already ended, or has nearly spent its attempts, is always stopped.

use std::{
    num::{NonZeroU16, NonZeroU32},
    sync::Mutex,
    time::Duration,
};

use vsift_application::{
    CancelOutcome, CancelRequest, CommitLedger, JobRequest, JobRunError, JobSpec, JobStore,
    JobStoreError, JobView, Publication, PublishedOpenRevision, RecognitionScope,
    RetranscriptionPorts, RetranscriptionRun, RevisionStore, SessionHead, SessionStorageError,
    TranscribeRangeRequest, cancel_job, job_id, job_status, recognition_key,
    retranscribe_operation_key, retranscribe_request_digest, retranscription_range,
    run_retranscription, seal_stopped_now, whole_file_source_segment,
};
use vsift_domain::{
    ChunkPlan, FailureCode, JobId, JobState, MediaTime, OperationId, RevisionState, SessionId,
    StorageGeneration, TranscriptRevision, TranscriptRevisionId, plan_chunks,
};
use vsift_infrastructure::{FilesystemSessionStore, MonotonicStopwatch, OpenRevisionRecord};

use crate::{
    error::CampaignError,
    protocol::{Ack, OperationKind},
    rng::SplitMix64,
    standins::{
        NeverStop, NoBackoff, SPEECH_MICROS, SourceUnchanged, StandInAudio, StandInRecognizer,
        recognizer_identity,
    },
    workload::{Acknowledger, OperationStop, committed, job_failure, storage, unix_nanos},
};

/// How the workload continues an open revision a crash left.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Continuation {
    /// Resumes its job to the seal.
    Resume,
    /// Stops it, as `job cancel` of a job no process runs does.
    Stop,
}

impl Continuation {
    /// One of the two, at random.
    pub fn drawn(rng: &mut SplitMix64) -> Self {
        if rng.below(2) == 0 {
            Self::Stop
        } else {
            Self::Resume
        }
    }
}

/// How a growing run ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ending {
    /// Sealed, with every chunk.
    Seal,
    /// Stopped once its first part is acknowledged, as `job cancel` stops it.
    StopAfterFirstPart,
}

/// Runs a growing retranscription of `session` and acknowledges it, or
/// continues the open revision a crash left there.
///
/// # Errors
///
/// An [`OperationStop`] when `VSift` answers with a typed failure or the
/// harness fails.
pub async fn grow(
    store: &FilesystemSessionStore,
    session: &SessionId,
    recognizer_delay: Duration,
    rng: &mut SplitMix64,
    seq: u64,
    acks: &mut Acknowledger<'_>,
    ending: Ending,
) -> Result<Ack, OperationStop> {
    let now = crate::workload::unix_seconds()?;
    if let Some(open) = store.read_open_transcript(session, now).map_err(storage)? {
        let continuation = Continuation::drawn(rng);
        return continue_open(
            store,
            session,
            &open,
            recognizer_delay,
            continuation,
            seq,
            acks,
        )
        .await;
    }
    run_growing(store, session, recognizer_delay, seq, acks, ending, now).await
}

/// Continues the open revision `open` of `session` that a crash left, as
/// `continuation` says; a job that has ended, or has nearly spent its
/// attempts, is stopped.
///
/// # Errors
///
/// As [`grow`].
pub async fn continue_open(
    store: &FilesystemSessionStore,
    session: &SessionId,
    open: &OpenRevisionRecord,
    recognizer_delay: Duration,
    continuation: Continuation,
    seq: u64,
    acks: &mut Acknowledger<'_>,
) -> Result<Ack, OperationStop> {
    let now = crate::workload::unix_seconds()?;
    let record = job_status(store, session, &open.job_id, now)
        .map_err(job_store)?
        .record;
    // A job that has ended, or would end at its next attempt, is stopped:
    // resuming it again and again across crashes must not fail the workload.
    let ended = record.state.is_terminal()
        || record.attempt.saturating_add(1) >= vsift_domain::MAX_JOB_ATTEMPTS;
    if (ended || continuation == Continuation::Stop)
        && let Some(ack) = stop_open(store, session, &open.job_id, seq, now)?
    {
        return Ok(ack);
    }
    run_growing(
        store,
        session,
        recognizer_delay,
        seq,
        acks,
        Ending::Seal,
        now,
    )
    .await
}

/// Stops `job`'s open revision as `job cancel` of a job no process runs does:
/// the job is cancelled, then what it published is sealed as stopped. `None`
/// when the job can no longer be cancelled (its commit had begun): the caller
/// resumes it instead.
fn stop_open(
    store: &FilesystemSessionStore,
    session: &SessionId,
    job: &JobId,
    seq: u64,
    now: u64,
) -> Result<Option<Ack>, OperationStop> {
    let outcome = cancel_job(store, session, job, now).map_err(job_store)?;
    if !matches!(
        outcome,
        CancelOutcome::Cancelled
            | CancelOutcome::AlreadyEnded(JobState::Cancelled | JobState::Failed)
    ) {
        return Ok(None);
    }
    let view = job_status(store, session, job, now).map_err(job_store)?;
    seal_stopped_now(
        store,
        session,
        job,
        view.record.epoch,
        view.record.attempt,
        now,
    )
    .map_err(storage)?;
    sealed_ack(store, session, seq, now, RevisionState::Stopped).map(Some)
}

/// The acknowledgement of the revision `session` now ends in, which must be
/// in `state`.
fn sealed_ack(
    store: &FilesystemSessionStore,
    session: &SessionId,
    seq: u64,
    now: u64,
    state: RevisionState,
) -> Result<Ack, OperationStop> {
    let head = RevisionStore::head(store, session, now).map_err(storage)?;
    let revision = head
        .newest
        .filter(|newest| newest.state() == state)
        .ok_or(OperationStop::Failed(FailureCode::Internal))?;
    let (generation, manifest) = committed(store, session, head.generation)?;
    Ok(Ack {
        seq,
        unix_ns: unix_nanos()?,
        kind: match state {
            RevisionState::Stopped => OperationKind::Stop,
            RevisionState::Open | RevisionState::Sealed => OperationKind::Grow,
        },
        session: session.clone(),
        generation,
        manifest_sha256: manifest,
        artifacts: Vec::new(),
        revision: Some(revision.id().clone()),
        request: None,
    })
}

fn job_store(error: JobStoreError) -> OperationStop {
    OperationStop::Failed(FailureCode::from(vsift::EngineError::from(error)))
}

/// The whole-source job of `session` as its head stands: the job a growing
/// run starts, and the one that continues the open revision a crash left
/// (whose base is the revision the open one supersedes, as the engine reads
/// it).
pub struct WholeRun {
    /// The job.
    pub spec: JobSpec,
    /// The source segment of the stand-in speech.
    pub segment: vsift_domain::SourceSegment,
    /// The revision the run supersedes.
    pub base: Option<TranscriptRevision>,
    /// The generation the run starts from.
    pub observed: StorageGeneration,
    /// The job of the session's open revision, if it has one.
    pub open_job: Option<JobId>,
}

impl WholeRun {
    /// The whole-source job of `session` now.
    ///
    /// # Errors
    ///
    /// As [`grow`].
    pub fn of(
        store: &FilesystemSessionStore,
        session: &SessionId,
        now: u64,
    ) -> Result<Self, OperationStop> {
        let status = store.session_status(session).map_err(storage)?;
        let segment =
            whole_file_source_segment(status.source_id(), MediaTime::from_micros(SPEECH_MICROS))
                .map_err(|_| CampaignError::StandIn)?;
        let head = RevisionStore::head(store, session, now).map_err(storage)?;
        let (base, open_job) = match head.newest {
            Some(open) if open.state() == RevisionState::Open => {
                let job = store
                    .read_open_transcript(session, now)
                    .map_err(storage)?
                    .map(|open| open.job_id);
                let base = match open.supersedes() {
                    Some(id) => Some(
                        store
                            .read_transcript_revision(session, id, now)
                            .map_err(storage)?
                            .map(|(revision, _)| revision)
                            .ok_or(OperationStop::Failed(FailureCode::IntegrityFailure))?,
                    ),
                    None => None,
                };
                (base, job)
            }
            other => (other, None),
        };
        let replaced = retranscription_range(base.as_ref(), None, segment.range());
        let identity = recognizer_identity().map_err(|_| CampaignError::StandIn)?;
        let key = recognition_key(&RecognitionScope {
            session_id: session,
            source_id: status.source_id(),
            audio_stream: 1,
            replaced_range: replaced,
            plan: ChunkPlan::R0,
            recognizer: &identity,
            verification: None,
        })
        .map_err(CampaignError::JobKey)?;
        let operation_key =
            retranscribe_operation_key(&key, base.as_ref().map(TranscriptRevision::id))
                .map_err(CampaignError::JobKey)?;
        let planned = plan_chunks(segment.id(), replaced, ChunkPlan::R0)
            .map_err(|_| CampaignError::StandIn)?;
        let spec = JobSpec {
            session_id: session.clone(),
            job_id: job_id(session, &operation_key).map_err(CampaignError::JobKey)?,
            request_digest: retranscribe_request_digest(session, None)
                .map_err(CampaignError::JobKey)?,
            operation_key,
            recognition_key: key,
            request: JobRequest::Retranscribe {
                range: None,
                share: vsift_domain::RecognitionShare::Whole,
            },
            planned_chunks: Some(u32::try_from(planned.len()).map_err(|_| CampaignError::StandIn)?),
        };
        Ok(Self {
            spec,
            segment,
            base,
            observed: head.generation,
            open_job,
        })
    }
}

/// Runs a growing retranscription of `session` from its head, ending as
/// `ending` says, and acknowledges each part and the seal.
///
/// # Errors
///
/// As [`grow`].
pub async fn run_growing(
    store: &FilesystemSessionStore,
    session: &SessionId,
    recognizer_delay: Duration,
    seq: u64,
    acks: &mut Acknowledger<'_>,
    ending: Ending,
    now: u64,
) -> Result<Ack, OperationStop> {
    let whole = WholeRun::of(store, session, now)?;
    if whole
        .open_job
        .as_ref()
        .is_some_and(|job| *job != whole.spec.job_id)
    {
        // Only a whole-source growing run publishes parts, so the open
        // revision is always this job's.
        return Err(CampaignError::ForeignOpenRevision.into());
    }
    let status = store.session_status(session).map_err(storage)?;
    let identity = recognizer_identity().map_err(|_| CampaignError::StandIn)?;
    let acknowledging = Acknowledging {
        store,
        acks: Mutex::new(acks),
        ending,
        job: whole.spec.job_id.clone(),
        now,
        state: Mutex::new(PartState::default()),
    };
    let stopwatch = MonotonicStopwatch::started();
    let replaced = retranscription_range(whole.base.as_ref(), None, whole.segment.range());
    let outcome = run_retranscription(
        RetranscriptionRun {
            spec: &whole.spec,
            operation_id: None,
            transcribe: TranscribeRangeRequest {
                source_segment: &whole.segment,
                range: replaced,
                plan: ChunkPlan::R0,
                audio_stream: 1,
                expected: &identity,
            },
            source_id: status.source_id(),
            requested: None,
            base: whole.base.as_ref(),
            observed: whole.observed,
            now,
            admission: vsift_domain::AdmissionWait::Immediate,
        },
        RetranscriptionPorts {
            store: &acknowledging,
            audio: &StandInAudio,
            recognizer: &StandInRecognizer {
                delay: recognizer_delay,
            },
            cancellation: &NeverStop,
            timer: &NoBackoff,
            classify: job_failure,
            progress: &vsift_application::NoProgress,
            publication: Publication::AsItGoes {
                interval: Duration::ZERO,
            },
            stopwatch: &stopwatch,
            seed: None,
        },
        &mut SourceUnchanged,
    )
    .await;
    let failure = acknowledging
        .state
        .into_inner()
        .map_err(|_| CampaignError::StandIn)?
        .failure;
    if let Some(stop) = failure {
        return Err(stop);
    }
    match outcome {
        Ok(_) => sealed_ack(store, session, seq, now, RevisionState::Sealed),
        Err(JobRunError::Cancelled { .. }) if ending == Ending::StopAfterFirstPart => {
            // The run seals what it published (best effort); a seal it could
            // not commit is committed here, as a later `job cancel` would.
            if let Some(open) = store.read_open_transcript(session, now).map_err(storage)? {
                let view = job_status(store, session, &open.job_id, now).map_err(job_store)?;
                seal_stopped_now(
                    store,
                    session,
                    &open.job_id,
                    view.record.epoch,
                    view.record.attempt,
                    now,
                )
                .map_err(storage)?;
            }
            sealed_ack(store, session, seq, now, RevisionState::Stopped)
        }
        Err(error) => Err(OperationStop::Failed(job_failure(&error))),
    }
}

/// What the parts of one growing run did.
#[derive(Default)]
struct PartState {
    /// Parts committed and acknowledged.
    acknowledged: u32,
    /// The first failure: the harness's, or a typed one of an acknowledgement.
    failure: Option<OperationStop>,
}

/// The session store as a growing run sees it: every part it commits is
/// acknowledged at once, and a `stop` run asks for its job to be cancelled
/// after its first.
struct Acknowledging<'s, 'a, 'c> {
    store: &'s FilesystemSessionStore,
    acks: Mutex<&'a mut Acknowledger<'c>>,
    ending: Ending,
    job: JobId,
    now: u64,
    state: Mutex<PartState>,
}

impl Acknowledging<'_, '_, '_> {
    /// Acknowledges the part committed as `generation`; asks for the job to
    /// be cancelled after the first part of a `stop` run.
    fn acknowledge(
        &self,
        part_seq: u64,
        session: &SessionId,
        generation: StorageGeneration,
        open: &TranscriptRevisionId,
    ) -> Result<(), OperationStop> {
        let (generation, manifest) = committed(self.store, session, generation)?;
        let ack = Ack {
            seq: part_seq,
            unix_ns: unix_nanos()?,
            kind: OperationKind::Part,
            session: session.clone(),
            generation,
            manifest_sha256: manifest,
            artifacts: Vec::new(),
            revision: Some(open.clone()),
            request: None,
        };
        self.acks
            .lock()
            .map_err(|_| CampaignError::StandIn)?
            .acknowledge(&ack)?;
        let first = {
            let mut state = self.state.lock().map_err(|_| CampaignError::StandIn)?;
            state.acknowledged += 1;
            state.acknowledged == 1
        };
        if first && self.ending == Ending::StopAfterFirstPart {
            self.store
                .request_cancel(session, &self.job, self.now)
                .map_err(job_store)?;
        }
        Ok(())
    }

    fn record_failure(&self, failure: OperationStop) {
        if let Ok(mut state) = self.state.lock()
            && state.failure.is_none()
        {
            state.failure = Some(failure);
        }
    }
}

impl JobStore for Acknowledging<'_, '_, '_> {
    type Owner = <FilesystemSessionStore as JobStore>::Owner;

    fn job(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
    ) -> Result<Option<JobView>, JobStoreError> {
        self.store.job(session_id, job_id)
    }

    fn job_session(&self, job_id: &JobId) -> Result<Option<SessionId>, JobStoreError> {
        self.store.job_session(job_id)
    }

    fn job_for_operation(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
    ) -> Result<Option<JobId>, JobStoreError> {
        self.store.job_for_operation(session_id, operation_id)
    }

    fn session_jobs(&self, session_id: &SessionId) -> Result<Vec<JobView>, JobStoreError> {
        JobStore::session_jobs(self.store, session_id)
    }

    fn open_or_create(
        &self,
        spec: &JobSpec,
        now: u64,
    ) -> Result<(Self::Owner, bool), JobStoreError> {
        self.store.open_or_create(spec, now)
    }

    fn acquire(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
    ) -> Result<Self::Owner, JobStoreError> {
        self.store.acquire(session_id, job_id)
    }

    fn request_cancel(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
        now: u64,
    ) -> Result<CancelRequest, JobStoreError> {
        self.store.request_cancel(session_id, job_id, now)
    }
}

impl CommitLedger for Acknowledging<'_, '_, '_> {
    fn committed_generation(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        above: StorageGeneration,
    ) -> Result<Option<StorageGeneration>, SessionStorageError> {
        self.store
            .committed_generation(session_id, operation_id, above)
    }
}

impl RevisionStore for Acknowledging<'_, '_, '_> {
    type Permit = <FilesystemSessionStore as RevisionStore>::Permit;
    type Demand = <FilesystemSessionStore as RevisionStore>::Demand;

    fn admit(&self, weight: NonZeroU16) -> Result<Self::Permit, SessionStorageError> {
        self.store.admit(weight)
    }

    fn admit_queued(&self, weight: NonZeroU16) -> Result<Self::Permit, SessionStorageError> {
        self.store.admit_queued(weight)
    }

    fn others_waiting(&self) -> bool {
        self.store.others_waiting()
    }

    fn demand_capacity(&self) -> Option<Self::Demand> {
        self.store.demand_capacity()
    }

    fn queued_work_holds(&self) -> bool {
        self.store.queued_work_holds()
    }

    fn head(&self, session_id: &SessionId, now: u64) -> Result<SessionHead, SessionStorageError> {
        RevisionStore::head(self.store, session_id, now)
    }

    fn revision(
        &self,
        session_id: &SessionId,
        revision_id: &TranscriptRevisionId,
        now: u64,
    ) -> Result<Option<TranscriptRevision>, SessionStorageError> {
        self.store.revision(session_id, revision_id, now)
    }

    fn open_revision(
        &self,
        session_id: &SessionId,
        now: u64,
    ) -> Result<Option<PublishedOpenRevision>, SessionStorageError> {
        self.store.open_revision(session_id, now)
    }

    fn publish_part(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        expected: StorageGeneration,
        open: &TranscriptRevision,
        job: &JobId,
        part: NonZeroU32,
        first: usize,
        now: u64,
    ) -> Result<StorageGeneration, SessionStorageError> {
        // A part is an operation of its own on the wire: started before it
        // touches the store, acknowledged once committed, and never a failure
        // line (the run tries a part that was not committed again).
        let started = self
            .acks
            .lock()
            .map_or(Err(CampaignError::StandIn), |mut acks| {
                let part_seq = acks.next_seq();
                acks.start(part_seq, OperationKind::Part).map(|()| part_seq)
            });
        let part_seq = started.unwrap_or_else(|error| {
            self.record_failure(error.into());
            0
        });
        let committed = self.store.publish_part(
            session_id,
            operation_id,
            expected,
            open,
            job,
            part,
            first,
            now,
        )?;
        if let Err(failure) = self.acknowledge(part_seq, session_id, committed, open.id()) {
            self.record_failure(failure);
        }
        Ok(committed)
    }

    fn publish_revision(
        &self,
        session_id: &SessionId,
        operation_id: &OperationId,
        expected: StorageGeneration,
        revision: &TranscriptRevision,
        now: u64,
    ) -> Result<StorageGeneration, SessionStorageError> {
        self.store
            .publish_revision(session_id, operation_id, expected, revision, now)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::OsStr,
        fs,
        path::PathBuf,
        process::{Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use vsift::{
        Cancellation, Engine, EngineConfig, EnginePorts, HostIsolation, IngestRequest,
        ManagedRootLocation, SessionRootLocation, UserConfigurationLocation,
    };
    use vsift_domain::{
        DurabilityRequirement, MediaTime, PublicationGuarantee, RevisionState, SessionId,
        TranscriptRevisionId,
    };
    use vsift_infrastructure::{FAULT_EXIT_CODE, FAULT_POINT_VARIABLE, FilesystemSessionStore};

    use super::{Continuation, Ending, continue_open, run_growing};
    use crate::{
        protocol::{Ack, OperationKind, parse_events},
        rng::SplitMix64,
        verify::{check_newest_revision, verify},
        workload::{Acknowledger, Channel, MAX_ACK_BYTES, OperationStop, read_text, unix_seconds},
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;
    type Built<T> = Result<T, Box<dyn std::error::Error>>;

    const OWNED_PREFIX: &str = "vsift-campaign-grow-test-";
    const CHILD_ROOT: &str = "VSIFT_GROW_TEST_ROOT";
    const CHILD_SESSION: &str = "VSIFT_GROW_TEST_SESSION";
    const CHILD_MODE: &str = "VSIFT_GROW_TEST_MODE";
    const CHILD_INITIAL: &str = "VSIFT_GROW_TEST_INITIAL";
    const CHILD_SEQ: &str = "VSIFT_GROW_TEST_SEQ";

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    /// A folder of the test's own, removed when the test ends: the session
    /// root, the scratch space and the acknowledgement log beside the root.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Built<Self> {
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "{OWNED_PREFIX}{}-{stamp}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }

        fn root(&self) -> PathBuf {
            self.0.join("root")
        }

        fn acks(&self) -> PathBuf {
            acks_beside(&self.root())
        }

        /// An ordinary (crash-consistent) session of a stand-in source.
        async fn session(&self) -> Built<SessionId> {
            let engine = Engine::new(
                EngineConfig {
                    session_root: SessionRootLocation::Explicit(self.root()),
                    user_configuration: UserConfigurationLocation::Explicit(self.0.join("config")),
                    managed_root: ManagedRootLocation::Explicit(self.0.join("managed")),
                    host_isolation: HostIsolation::ProcessOnly,
                },
                EnginePorts::system(),
            );
            let source = self.0.join("source.mp4");
            let mut bytes = b"\0\0\0\x18ftypisomvsift-campaign-grow".to_vec();
            let mut rng = SplitMix64::new(7);
            while bytes.len() < 16 * 1024 {
                bytes.extend_from_slice(&rng.next().to_le_bytes());
            }
            fs::write(&source, &bytes)?;
            let opened = engine
                .ingest(IngestRequest {
                    source,
                    transcript: None,
                    cancellation: Cancellation::new(),
                    durability: DurabilityRequirement::Ephemeral,
                })
                .await?;
            Ok(opened.session.session_id)
        }

        fn store(&self) -> Built<FilesystemSessionStore> {
            Ok(FilesystemSessionStore::open_existing(self.root())?)
        }

        fn acknowledged(&self) -> Built<Vec<Ack>> {
            if !self.acks().exists() {
                return Ok(Vec::new());
            }
            let events = parse_events(&read_text(&self.acks(), MAX_ACK_BYTES)?);
            if !events.malformed.is_empty() {
                return Err(format!("malformed acknowledgements {:?}", events.malformed).into());
            }
            Ok(events.acks)
        }

        /// The verifier finds nothing lost or damaged.
        fn verified(&self, context: &str) -> TestResult {
            let acks = self.acknowledged()?;
            let findings = verify(
                &self.root(),
                &acks,
                unix_seconds()?,
                PublicationGuarantee::ProcessCrashConsistent,
            );
            if !findings.clean() {
                return Err(format!("{context}: {:?}", findings.lines("VERIFY")).into());
            }
            Ok(())
        }

        /// The session's newest revision's state.
        fn newest_state(&self, session: &SessionId) -> Built<Option<RevisionState>> {
            let (newest, _) = self
                .store()?
                .read_transcript_head(session, unix_seconds()?)?;
            Ok(newest.map(|newest| newest.state()))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if self
                .0
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with(OWNED_PREFIX))
            {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    /// Where the acknowledgement log of the session root `root` lives.
    fn acks_beside(root: &std::path::Path) -> PathBuf {
        root.with_file_name("acks.log")
    }

    fn kinds(acks: &[Ack]) -> Vec<OperationKind> {
        acks.iter().map(|ack| ack.kind).collect()
    }

    fn stop_text(stop: OperationStop) -> String {
        match stop {
            OperationStop::Failed(code) => code.identifier().to_owned(),
            OperationStop::Harness(error) => error.to_string(),
        }
    }

    /// Runs one growing operation in this process and acknowledges it.
    async fn operation(fixture: &Fixture, session: &SessionId, ending: Ending) -> TestResult {
        let store = fixture.store()?;
        let mut channel = Channel::open(&fixture.acks(), false)?;
        let mut acks = Acknowledger::new(&mut channel, None, 1);
        let seq = acks.next_seq();
        let now = unix_seconds()?;
        let ack = run_growing(&store, session, Duration::ZERO, seq, &mut acks, ending, now)
            .await
            .map_err(stop_text)?;
        acks.acknowledge(&ack)?;
        Ok(())
    }

    /// ADR 0033: a growing run over the stand-in's four chunks acknowledges
    /// three parts as they are committed, then its seal; every part is still
    /// held after the seal retires its file, and the sealed revision is, byte
    /// for byte, what a seal-only run makes.
    #[tokio::test]
    async fn a_growing_run_acknowledges_three_parts_then_its_seal() -> TestResult {
        let fixture = Fixture::new()?;
        let session = fixture.session().await?;
        operation(&fixture, &session, Ending::Seal).await?;
        let acks = fixture.acknowledged()?;
        assert_eq!(
            kinds(&acks),
            [
                OperationKind::Part,
                OperationKind::Part,
                OperationKind::Part,
                OperationKind::Grow
            ]
        );
        // One open revision, sealed under its own identity.
        let revisions: Vec<&TranscriptRevisionId> = acks
            .iter()
            .filter_map(|ack| ack.revision.as_ref())
            .collect();
        assert!(revisions.windows(2).all(|pair| pair[0] == pair[1]));
        let generations: Vec<u64> = acks.iter().map(|ack| ack.generation).collect();
        assert!(generations.windows(2).all(|pair| pair[0] < pair[1]));
        fixture.verified("after the seal")?;
        assert_eq!(fixture.newest_state(&session)?, Some(RevisionState::Sealed));
        check_newest_revision(&fixture.store()?, &session, unix_seconds()?)?;
        Ok(())
    }

    /// ADR 0033, D1: a growing run whose job is cancelled after its first
    /// part seals what it published as a stopped revision, which keeps the
    /// part and is acknowledged as `stop`; a later run over it seals.
    #[tokio::test]
    async fn a_stopped_run_keeps_what_it_published() -> TestResult {
        let fixture = Fixture::new()?;
        let session = fixture.session().await?;
        operation(&fixture, &session, Ending::StopAfterFirstPart).await?;
        let acks = fixture.acknowledged()?;
        assert_eq!(kinds(&acks), [OperationKind::Part, OperationKind::Stop]);
        fixture.verified("after the stop")?;
        assert_eq!(
            fixture.newest_state(&session)?,
            Some(RevisionState::Stopped)
        );
        operation(&fixture, &session, Ending::Seal).await?;
        fixture.verified("after a run over the stopped revision")?;
        assert_eq!(fixture.newest_state(&session)?, Some(RevisionState::Sealed));
        Ok(())
    }

    /// The verifier's seal-only run also makes what the workload's ordinary
    /// retranscriptions commit, over whole and partial ranges and over a base,
    /// so the campaign holds every newest revision to it.
    #[tokio::test]
    async fn the_seal_only_run_matches_the_ordinary_retranscriptions() -> TestResult {
        let fixture = Fixture::new()?;
        let session = fixture.session().await?;
        let config = crate::workload::WorkloadConfig {
            root: fixture.root(),
            scratch: fixture.0.join("scratch"),
            ack_out: fixture.acks(),
            drain: false,
            mark_device: None,
            dmsetup: PathBuf::from("dmsetup"),
            known_acks: None,
            first_seq: 1,
            max_ops: None,
            stop_after_failures: None,
            seed: 3,
            recognizer_delay: Duration::ZERO,
            rotate_after: 40,
            source_kib: (16, 32),
            mix: crate::workload::Mix::All,
        };
        let mut rng = SplitMix64::new(11);
        let mut channel = Channel::open(&fixture.acks(), false)?;
        let mut acks = Acknowledger::new(&mut channel, None, 1);
        for _ in 0..6 {
            let seq = acks.next_seq();
            let ack = crate::workload::retranscribe_or_continue(
                &config, &session, &mut rng, seq, &mut acks,
            )
            .await
            .map_err(stop_text)?;
            acks.acknowledge(&ack)?;
            check_newest_revision(&fixture.store()?, &session, unix_seconds()?)?;
        }
        fixture.verified("after the retranscriptions")?;
        Ok(())
    }

    /// What a session's newest revision has published: its segments and the
    /// time it is published until (the end of time once sealed).
    fn published(fixture: &Fixture, session: &SessionId) -> Built<(usize, MediaTime)> {
        let (newest, _) = fixture
            .store()?
            .read_transcript_head(session, unix_seconds()?)?;
        Ok(newest.map_or((0, MediaTime::from_micros(0)), |newest| {
            (
                newest.segments().len(),
                newest
                    .published_until()
                    .unwrap_or(MediaTime::from_micros(u64::MAX)),
            )
        }))
    }

    /// The child of the kill tests: continues the session's open revision as
    /// `VSIFT_GROW_TEST_MODE` says, or starts a growing run while the session
    /// still ends in the revision it began with, stopping wherever
    /// `VSIFT_FAULT_POINT` says; with nothing left to do it ends at once.
    #[tokio::test]
    #[ignore = "internal child entry launched by the kill tests"]
    async fn growing_child() -> TestResult {
        let root = PathBuf::from(env::var_os(CHILD_ROOT).ok_or("missing root")?);
        let session = SessionId::parse(env::var(CHILD_SESSION)?)?;
        let stopping = env::var(CHILD_MODE)? == "stop";
        let initial = env::var(CHILD_INITIAL)?;
        let first_seq: u64 = env::var(CHILD_SEQ)?.parse()?;
        let store = FilesystemSessionStore::open_existing(&root)?;
        let mut channel = Channel::open(&acks_beside(&root), false)?;
        let mut acks = Acknowledger::new(&mut channel, None, first_seq);
        let seq = acks.next_seq();
        let now = unix_seconds()?;
        let outcome = if let Some(open) = store.read_open_transcript(&session, now)? {
            let continuation = if stopping {
                Continuation::Stop
            } else {
                Continuation::Resume
            };
            continue_open(
                &store,
                &session,
                &open,
                Duration::ZERO,
                continuation,
                seq,
                &mut acks,
            )
            .await
        } else {
            let (newest, _) = store.read_transcript_head(&session, now)?;
            let newest = newest.map_or_else(|| "-".to_owned(), |newest| newest.id().to_string());
            if newest != initial {
                return Ok(());
            }
            let ending = if stopping {
                Ending::StopAfterFirstPart
            } else {
                Ending::Seal
            };
            run_growing(
                &store,
                &session,
                Duration::ZERO,
                seq,
                &mut acks,
                ending,
                now,
            )
            .await
        };
        let ack = outcome.map_err(stop_text)?;
        acks.acknowledge(&ack)?;
        Ok(())
    }

    /// Runs the child once, stopping at `point` if it gets there; says whether
    /// it did.
    fn child(
        fixture: &Fixture,
        session: &SessionId,
        mode: &str,
        seq: u64,
        point: Option<&str>,
    ) -> Built<bool> {
        let mut command = Command::new(env::current_exe()?);
        command
            .args([
                "--exact",
                "grow::tests::growing_child",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ROOT, fixture.root())
            .env(CHILD_SESSION, session.as_str())
            .env(CHILD_MODE, mode)
            .env(CHILD_INITIAL, "-")
            .env(CHILD_SEQ, seq.to_string())
            .env_remove(FAULT_POINT_VARIABLE)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(point) = point {
            command.env(FAULT_POINT_VARIABLE, point);
        }
        let output = command.output()?;
        match output.status.code() {
            Some(FAULT_EXIT_CODE) => Ok(true),
            Some(0) => Ok(false),
            other => Err(format!(
                "{point:?}: the child ended with {other:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into()),
        }
    }

    /// Where a part, the seal or a stopped seal is killed: before its first
    /// file is in place, with its manifest in place but not its pointer,
    /// committed but not yet synchronised, committed; and, for the seal, with
    /// the job saying it commits.
    const POINTS: [&str; 5] = [
        "artifact-install",
        "manifest-rename",
        "pointer-rename",
        "pointer-directory-sync",
        "job-committing",
    ];

    /// Moves a killed run on by one chunk: the child stops when its first
    /// chunk is recognised, before its checkpoint, after committing what the
    /// chunks before allow.
    const ADVANCE: &str = "chunk-recognised:1";

    /// ADR 0033, the crash campaign's open revisions at the unit level: a
    /// growing run killed at each point of each part's commit and of its
    /// seal, and resumed, leaves after every kill a state the verifier holds
    /// (every part acknowledged is kept; an open revision rebuilt from its
    /// parts is a prefix of the seal-only revision; what is published never
    /// goes back), and ends in the revision a seal-only run makes, byte for
    /// byte.
    #[tokio::test]
    async fn a_growing_run_killed_at_every_commit_resumes_to_the_same_seal() -> TestResult {
        for point in POINTS {
            let fixture = Fixture::new()?;
            let session = fixture.session().await?;
            let mut previous = published(&fixture, &session)?;
            let mut finished = false;
            for round in 1..=12_u64 {
                let fault = if round % 2 == 1 { point } else { ADVANCE };
                let killed = child(&fixture, &session, "grow", round * 1_000, Some(fault))?;
                fixture.verified(&format!("{point}, round {round}"))?;
                let now = published(&fixture, &session)?;
                assert!(
                    now.0 >= previous.0 && now.1 >= previous.1,
                    "{point}, round {round}: what is published went back"
                );
                previous = now;
                if !killed {
                    finished = true;
                    break;
                }
            }
            assert!(finished, "{point}: the run never finished");
            assert_eq!(
                fixture.newest_state(&session)?,
                Some(RevisionState::Sealed),
                "{point}"
            );
            check_newest_revision(&fixture.store()?, &session, unix_seconds()?)
                .map_err(|what| format!("{point}: {what}"))?;
            let parts = fixture
                .acknowledged()?
                .iter()
                .filter(|ack| ack.kind == OperationKind::Part)
                .count();
            // A kill after a part's commit and before its acknowledgement
            // leaves it committed but unacknowledged, which is allowed; no
            // part is ever committed twice.
            assert!(parts <= 3, "{point}: {parts} parts acknowledged");
        }
        Ok(())
    }

    /// ADR 0033, D1, at the unit level: a run stopped after its first part,
    /// killed at each point of that part's commit, of the run's own stopped
    /// seal and of the stopped seal a later `job cancel` commits, ends in a
    /// stopped revision that keeps every part acknowledged.
    #[tokio::test]
    async fn a_stopped_run_killed_at_every_commit_keeps_what_it_published() -> TestResult {
        for point in POINTS {
            let fixture = Fixture::new()?;
            let session = fixture.session().await?;
            for round in 1..=3_u64 {
                if !child(&fixture, &session, "stop", round * 1_000, Some(point))? {
                    break;
                }
                fixture.verified(&format!("{point}, round {round}"))?;
            }
            // Whatever is left is finished without a kill.
            child(&fixture, &session, "stop", 9_000, None)?;
            fixture.verified(&format!("{point}, at the end"))?;
            assert_eq!(
                fixture.newest_state(&session)?,
                Some(RevisionState::Stopped),
                "{point}"
            );
            check_newest_revision(&fixture.store()?, &session, unix_seconds()?)
                .map_err(|what| format!("{point}: {what}"))?;
        }
        Ok(())
    }
}
