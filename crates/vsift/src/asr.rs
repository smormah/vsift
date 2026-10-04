//! Local speech recognition of a session's source: `transcript retranscribe`.
//!
//! [`Engine::retranscribe`] is the only way local ASR is requested (maintainer
//! decision D1, ADR 0017). `ingest` and `transcript get` never resolve
//! whisper.cpp or a model. The operation resolves every dependency and
//! identifies the model before anything runs, refuses a model that is not a
//! reviewed pinned profile (D5), runs the media-tool preflight and then the
//! local-ASR preflight, and only then holds the session, decodes speech from
//! its committed private source copy and recognises it in a private work
//! directory inside the session. The copy is bound for the whole run (issue
//! #148): hashed once when it is opened, compared by on-disk identity before
//! every `FFprobe` and `FFmpeg` call, and hashed again before the new revision
//! is committed with one generation publication. A failure, a changed copy or
//! a cancellation commits nothing.
//!
//! Since P10 the run is a recoverable job (ADR 0020): the application's
//! `run_retranscription` owns the retry table, the chunk checkpoints and the
//! exactly-once commit; this module resolves the request into the job's keys
//! and supplies the ports.

use std::{
    ffi::OsStr,
    future::Future,
    num::NonZeroU16,
    path::{Path, PathBuf},
    pin::Pin,
    time::Duration,
};

use vsift_application::{
    AsrFailure, AsrFailureReason, AsrStage, CachedMediaToolVerification, CommitGuard, JobRecord,
    JobReport, JobRequest, JobSpec, LocalAsrVerification, LocalAsrVerificationFailure,
    LocalAsrVerifier, MediaToolCheck, MediaToolFailure, MediaToolFingerprint,
    MediaToolPreflightFailure, MediaToolPreflightOutcome, MediaToolVerificationCache,
    OperationLookup, RecognitionScope, RecognizerIdentity, RetranscriptionPorts,
    RetranscriptionRun, RevisionStore, SessionStorageError, SourceProbeError, SpeechPcm,
    SpeechRecognitionError, SpeechRecognizer, TranscribeRangeRequest, job_id, lookup_operation,
    preflight_local_asr, recognition_key, retranscribe_operation_key, retranscribe_request_digest,
    retranscription_range, run_retranscription, whole_file_source_segment,
};
use vsift_domain::{
    AdmissionWait, AsrModelProfile, ChunkPlan, JobId, MediaSelection, MediaTime, OperationId,
    PlannedChunk, ProviderChunkOutput, RuntimeDependency, SessionId, SessionPhase, TimeRange,
    TranscriptRevision, plan_chunks, recognizer_threads,
};
use vsift_infrastructure::{
    BoundSource, ExecutableResolutionError, ExecutableResolver, FfmpegMedia, FfmpegSpeechAudio,
    FilesystemMediaToolVerificationCache, FilesystemSessionStore, FixtureAsrVerifier,
    LocalAsrFiles, MediaError, MediaProviderConformance, MediaToolVerificationAuthority,
    ProcessCancellation, ProcessWorkingDirectory, SessionStatus, SourceError, TokioRetryTimer,
    TrustedExecutable, WhisperCli, WhisperError, WhisperSpeechRecognizer, local_asr_fingerprint,
    media_tool_fingerprint, reviewed_compatibility_policy,
};

use crate::{
    engine::Engine,
    error::{EngineError, ExecutableRejection, SessionRootError, job_failure_code},
    managed::ManagedLookup,
    progress::ProgressObserver,
    sessions::SessionSnapshot,
    verification::Cancellation,
};

/// A request to transcribe a session's speech locally with whisper.cpp.
#[derive(Clone, Debug)]
pub struct RetranscribeRequest {
    /// Session whose committed source is transcribed.
    pub session: SessionId,
    /// Source range to transcribe; `None` transcribes the whole source.
    pub range: Option<RetranscribeRange>,
    /// The caller's operation id (maintainer decision D-1): a retry with the
    /// same id and request continues its job or returns its committed result
    /// without a new generation; the same id with another request is
    /// [`EngineError::IdempotencyConflict`]. `None` treats every request as
    /// new work (an identical one still continues an interrupted job).
    pub operation_id: Option<OperationId>,
    /// Signal that stops the run at its next provider boundary; nothing is
    /// committed after it fires, and the job stays resumable.
    pub cancellation: Cancellation,
    /// Receives `recognising_speech` progress in chunks (P11): 0 of the
    /// plan once it is made, then each chunk, reused ones included. A replay
    /// reports nothing.
    pub progress: ProgressObserver,
    /// How the run waits for its admission weight (its recognizer threads)
    /// when the root is busy: [`AdmissionWait::Immediate`] for interactive
    /// commands, which keep P10's two bounded retries, or a bounded wait of
    /// at most 60 s for a job host, after which the run answers
    /// [`EngineError::AdmissionBusy`] with a retry hint.
    pub admission: AdmissionWait,
}

/// A half-open source range to retranscribe, in microseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetranscribeRange {
    /// Inclusive source-timeline start.
    pub from_micros: u64,
    /// Exclusive source-timeline end.
    pub to_micros: u64,
}

/// What the recoverable job behind a retranscription did (P10, ADR 0020).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobSummary {
    job_id: JobId,
    operation_id: OperationId,
    resumed: bool,
    chunks_reused: u32,
    checkpoints_discarded: u32,
    replayed: bool,
    admission_wait: Duration,
}

impl JobSummary {
    pub(crate) fn from_report(report: JobReport) -> Self {
        Self {
            job_id: report.job_id,
            operation_id: report.operation_id,
            resumed: report.resumed,
            chunks_reused: report.chunks_reused,
            checkpoints_discarded: report.checkpoints_discarded,
            replayed: report.replayed,
            admission_wait: report.admission_wait,
        }
    }

    /// Time the run waited for admission capacity (a bounded
    /// [`AdmissionWait`] only; zero otherwise), which a worker reports as a
    /// step's `admission_wait_ms`.
    #[must_use]
    pub const fn admission_wait(&self) -> Duration {
        self.admission_wait
    }

    /// The job: derived from the session and the request's operation key,
    /// so running the same request again finds it.
    #[must_use]
    pub const fn job_id(&self) -> &JobId {
        &self.job_id
    }

    /// The operation the result is recorded under: the caller's operation
    /// id, or the commit's own when the caller gave none. A retry with it
    /// returns this result.
    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Whether an interrupted run of the job was continued.
    #[must_use]
    pub const fn resumed(&self) -> bool {
        self.resumed
    }

    /// Chunks taken from the interrupted run's checkpoints.
    #[must_use]
    pub const fn chunks_reused(&self) -> u32 {
        self.chunks_reused
    }

    /// Checkpoints found unusable and recognised again.
    #[must_use]
    pub const fn checkpoints_discarded(&self) -> u32 {
        self.checkpoints_discarded
    }

    /// Whether the revision is an earlier commit returned again, without a
    /// new generation.
    #[must_use]
    pub const fn replayed(&self) -> bool {
        self.replayed
    }
}

/// A committed retranscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetranscribeOutcome {
    session: SessionSnapshot,
    requested: Option<TimeRange>,
    revision: TranscriptRevision,
    job: JobSummary,
}

impl RetranscribeOutcome {
    /// Session state after the revision was committed.
    #[must_use]
    pub const fn session(&self) -> &SessionSnapshot {
        &self.session
    }

    /// The range the caller asked for, or `None` for the whole source.
    #[must_use]
    pub const fn requested(&self) -> Option<TimeRange> {
        self.requested
    }

    /// The new revision, now the session's newest (or, when replayed, the
    /// revision the earlier commit made).
    #[must_use]
    pub const fn revision(&self) -> &TranscriptRevision {
        &self.revision
    }

    /// What the job behind the result did.
    #[must_use]
    pub const fn job(&self) -> &JobSummary {
        &self.job
    }
}

/// Which recognizer a retranscription uses.
pub(crate) enum SelectedRecognizer<'a> {
    /// whisper.cpp with the configured model, run in a work directory.
    Whisper(WhisperCli),
    /// The embedding host's recognizer and verifier.
    Host(&'a HostAsr),
}

/// The closing verification before each commit attempt: the session's source
/// copy must still hold the committed bytes (issue #148).
struct SourceUnchanged<'a>(&'a BoundSource);

impl CommitGuard for SourceUnchanged<'_> {
    fn verify(&mut self) -> Result<(), SessionStorageError> {
        self.0
            .verify_unchanged()
            .map_err(|error| snapshot_storage_error(&error))
    }
}

impl Engine {
    /// Transcribes a session's speech, or one range of it, into a new revision.
    ///
    /// The whole source is transcribed when `range` is `None`. A bounded range
    /// is first widened to whole segments of the session's newest revision, and
    /// the result is a complete spliced revision: segments outside the range are
    /// carried from the newest revision with their original provenance, and the
    /// recognised segments fill the range (D3). The new revision becomes the
    /// newest, and so the default of `transcript get`; every earlier revision
    /// stays readable by identity. A run that hears no speech still commits a
    /// revision recording its chunk outcomes, with a `no_speech_recognised`
    /// warning.
    ///
    /// The run is a recoverable job (P10, ADR 0020). Its identity derives
    /// from the request, so rerunning the same request after an interruption
    /// (a crash, a cancellation, a failure) continues it from its chunk
    /// checkpoints and commits the revision an uninterrupted run would; a
    /// retry with the same operation id after a commit returns that commit.
    /// A session that moved during the run (a renewal, another revision) is
    /// followed rather than reported busy after all the work.
    ///
    /// # Errors
    ///
    /// Fails before running anything for an invalid range; with an
    /// operation id, then for a missing, closed or expired session and with
    /// [`EngineError::IdempotencyConflict`] or [`EngineError::JobBusy`] for an
    /// id bound to another request or to a live job (a committed one is
    /// replayed here, before any tool is needed); then for a missing tool or
    /// model, or a model that is not a reviewed pinned profile; then (without
    /// an operation id) for a missing, closed or expired session; then for a
    /// failed media-tool or local-ASR preflight; and during the run for a
    /// source without audio, a typed recognition failure, cancellation, a
    /// storage failure (including a source copy that changed), the same job
    /// running in another process, or a range another revision changed.
    /// Nothing is committed on failure, and the job stays resumable unless
    /// the error says otherwise. A cancellation by the caller before the
    /// commit is [`EngineError::JobInterrupted`], naming the job to resume; a
    /// `job cancel` from another process is noticed within
    /// [`vsift_infrastructure::JOB_CANCEL_POLL`], stops the running provider
    /// and ends the run with [`EngineError::JobCancelled`].
    #[allow(
        clippy::too_many_lines,
        reason = "The stage order is the contract; keep it visible in one place"
    )]
    pub async fn retranscribe(
        &self,
        request: RetranscribeRequest,
    ) -> Result<RetranscribeOutcome, EngineError> {
        let requested = request
            .range
            .map(|range| {
                TimeRange::new(
                    MediaTime::from_micros(range.from_micros),
                    MediaTime::from_micros(range.to_micros),
                )
                .map_err(|_| EngineError::InvalidTimeRange)
            })
            .transpose()?;
        let cancellation = request.cancellation.0.clone();
        let digest = retranscribe_request_digest(&request.session, requested)
            .map_err(|_| EngineError::JobInvariant)?;

        // 1. A retry with an operation id is answered first. A committed one
        // is replayed from its commit before any tool is resolved, checked or
        // run and before any hash (X-02); the same id with another request
        // is a conflict (X-03); a live job is busy. All three need only the
        // request digest and a read of the session, so a retry is answered
        // even where the tools have since gone. A request without an id
        // keeps the D5 order: nothing about the session is read before the
        // model is known to be a reviewed pinned profile.
        if let Some(operation) = &request.operation_id {
            let (store, now) = self.existing_store()?;
            let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
            store.read_transcript_head(&request.session, now)?;
            match lookup_operation(&store, &request.session, operation, &digest, now)? {
                OperationLookup::Replay(record) => {
                    return self.replayed(&store, requested, &record, operation.clone());
                }
                OperationLookup::Busy(job) => return Err(EngineError::JobBusy { job }),
                OperationLookup::Unbound | OperationLookup::Continue(_) => {}
            }
        }

        // 2. Resolve and identify everything before anything runs.
        let tools = self.local_asr_media_tools()?;
        let recognizer = self.select_recognizer(self.admission_capacity_hint())?;
        let identity = match &recognizer {
            SelectedRecognizer::Whisper(cli) => cli.recognizer_identity().await,
            SelectedRecognizer::Host(host) => host.recognizer.identity_boxed().await,
        }
        .map_err(|error| match error {
            SpeechRecognitionError::ModelUnavailable => EngineError::LocalAsrModelUnavailable,
            other => EngineError::LocalAsrFailed(AsrFailure {
                stage: AsrStage::RecognizerIdentity,
                reason: other.into(),
            }),
        })?;
        if identity.model.profile() == AsrModelProfile::Unreviewed {
            return Err(EngineError::LocalAsrModelNotPinned);
        }

        // 3. The session must exist and be open before minutes of work start.
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        // The newest revision and the generation come from one manifest, so
        // the commit below never builds on a base older than it expects.
        let (base, status) = store.read_transcript_head(&request.session, now)?;
        // A recognition reserves its threads of the root (X-07): one that
        // could never fit is refused before any work.
        admission_fits(&store, identity.threads)?;

        // 4. Prove the tools and the recognizer work before touching user media.
        self.ensure_media_tools_verified(&tools).await?;
        let verification = self
            .ensure_local_asr_verified(&tools, &recognizer, &identity, &cancellation)
            .await?;

        // 5. Hold the session's committed source and a private work directory.
        // The copy is hashed once here, compared by identity before each
        // provider call below, and hashed again before each commit (#148).
        let bound = BoundSource::open_committed(&store, &request.session, now)
            .map_err(|error| EngineError::Storage(snapshot_storage_error(&error)))?;
        let work = store.session_work_directory(&request.session, now)?;
        let media = FfmpegMedia::new(
            tools.clone(),
            self.config().host_isolation.into_infrastructure(),
            &store,
        );
        let description = media
            .probe(&bound, cancellation.clone())
            .await
            .map_err(|error| EngineError::SourceProbe(probe_error(&error)))?;
        let stream = description
            .speech_audio_stream()
            .ok_or(EngineError::NoAudioStream)?;
        let source = whole_file_source_segment(bound.snapshot().id(), description.duration)
            .map_err(|_| EngineError::SourceProbe(SourceProbeError::InvalidSource))?;
        if let Some(range) = requested
            && range.end() > source.range().end()
        {
            return Err(EngineError::RangeOutsideSource);
        }
        let replaced = retranscription_range(base.as_ref(), requested, source.range());
        // Reported by `job status` as progress; a plan that cannot be made
        // fails the run itself at planning.
        let planned_chunks = plan_chunks(source.id(), replaced, ChunkPlan::R0)
            .ok()
            .and_then(|chunks| u32::try_from(chunks.len()).ok());

        // 6. The job the request's keys name (ADR 0020 section 4).
        let key = recognition_key(&RecognitionScope {
            session_id: &request.session,
            source_id: bound.snapshot().id(),
            audio_stream: stream,
            replaced_range: replaced,
            plan: ChunkPlan::R0,
            recognizer: &identity,
            verification: verification.as_ref(),
        })
        .map_err(|_| EngineError::JobInvariant)?;
        let operation_key =
            retranscribe_operation_key(&key, base.as_ref().map(TranscriptRevision::id))
                .map_err(|_| EngineError::JobInvariant)?;
        let spec = JobSpec {
            session_id: request.session.clone(),
            job_id: job_id(&request.session, &operation_key)
                .map_err(|_| EngineError::JobInvariant)?,
            request_digest: digest,
            operation_key,
            recognition_key: key,
            request: JobRequest::Retranscribe { range: requested },
            planned_chunks,
        };

        // 7. Recognise from the job's checkpoints and the audio, assemble the
        // complete revision, verify the copy and commit exactly once. Each
        // attempt reserves the recognizer's threads of the root's capacity
        // while it recognises (SEC-20, X-07); that reservation also covers
        // the chunk decoding between recognitions, so the decoding adapter
        // takes none of its own.
        let speech_media = media.within_caller_admission();
        let audio = FfmpegSpeechAudio::new(
            &speech_media,
            &bound,
            &description,
            MediaSelection {
                video: None,
                audio: Some(stream),
            },
            cancellation.clone(),
        );
        let run = RetranscriptionRun {
            spec: &spec,
            operation_id: request.operation_id.as_ref(),
            transcribe: TranscribeRangeRequest {
                source_segment: &source,
                range: replaced,
                plan: ChunkPlan::R0,
                audio_stream: stream,
                expected: &identity,
            },
            source_id: bound.snapshot().id(),
            requested,
            base: base.as_ref(),
            observed: status.generation(),
            now,
            admission: request.admission,
        };
        let mut guard = SourceUnchanged(&bound);
        let selected = match &recognizer {
            SelectedRecognizer::Whisper(cli) => {
                let chunks = ProcessWorkingDirectory::new(work.path()).map_err(|_| {
                    EngineError::LocalAsrFailed(AsrFailure {
                        stage: AsrStage::Planning,
                        reason: AsrFailureReason::Workspace,
                    })
                })?;
                RunRecognizer::Whisper(WhisperSpeechRecognizer::new(
                    cli.clone(),
                    chunks,
                    cancellation.clone(),
                ))
            }
            SelectedRecognizer::Host(host) => {
                RunRecognizer::Host(HostRecognizerRef(host.recognizer.as_ref()))
            }
        };
        let progress = request.progress.for_job(&spec.job_id);
        let running = run_retranscription(
            run,
            RetranscriptionPorts {
                store: &store,
                audio: &audio,
                recognizer: &selected,
                cancellation: &cancellation,
                timer: &TokioRetryTimer,
                classify: job_failure_code,
                progress: &progress,
            },
            &mut guard,
        );
        // A `job cancel` from another process only records the request;
        // this run notices it within the poll interval and stops its
        // provider instead of finishing the chunk first.
        let outcome = store
            .watch_for_job_cancel(&spec.session_id, &spec.job_id, &cancellation, running)
            .await
            .map_err(|error| EngineError::from_job_run(error, &request.session))?;
        drop(audio);
        drop(work);
        drop(bound);
        let now = self.now_unix_seconds()?;
        let status = store.session_status(&request.session)?;
        Ok(RetranscribeOutcome {
            session: SessionSnapshot::observe(&status, now),
            requested,
            revision: outcome.revision,
            job: JobSummary::from_report(outcome.report),
        })
    }

    /// The committed result of a succeeded job, returned again without
    /// running or committing anything.
    fn replayed(
        &self,
        store: &FilesystemSessionStore,
        requested: Option<TimeRange>,
        record: &JobRecord,
        operation_id: OperationId,
    ) -> Result<RetranscribeOutcome, EngineError> {
        let now = self.now_unix_seconds()?;
        let commit = record.commit.as_ref().ok_or(EngineError::JobInvariant)?;
        let revision = store
            .revision(&record.session_id, &commit.revision_id, now)?
            .ok_or(EngineError::Storage(SessionStorageError::IntegrityFailure))?;
        let status = store.session_status(&record.session_id)?;
        Ok(RetranscribeOutcome {
            session: SessionSnapshot::observe(&status, now),
            requested,
            revision,
            job: JobSummary {
                job_id: record.job_id.clone(),
                operation_id,
                resumed: false,
                chunks_reused: 0,
                checkpoints_discarded: 0,
                replayed: true,
                admission_wait: Duration::ZERO,
            },
        })
    }

    /// Resolves `FFmpeg` and `FFprobe` for local ASR: a configured selection
    /// first, then the filtered `PATH`.
    fn local_asr_media_tools(&self) -> Result<MediaProviderConformance, EngineError> {
        self.media_tools().map_err(|error| match error {
            EngineError::MediaToolUnavailable(dependency) => {
                EngineError::LocalAsrToolUnavailable(dependency)
            }
            other => other,
        })
    }

    /// The host's recognizer when one was supplied, otherwise whisper.cpp
    /// (configured, then `whisper-cli` on the filtered `PATH`) with the
    /// configured model.
    fn select_recognizer(
        &self,
        capacity: NonZeroU16,
    ) -> Result<SelectedRecognizer<'_>, EngineError> {
        if let Some(host) = self.host_asr() {
            return Ok(SelectedRecognizer::Host(host));
        }
        let store = self.user_configuration()?;
        let mut managed = self.managed_lookup();
        let executable = resolve_recognizer(&mut managed, store.read()?.whisper)?;
        let (model, executable) = if let Some(model) = store.read_model()? {
            (model, executable)
        } else {
            let managed = managed.model().ok_or(EngineError::ModelNotSelected)?;
            // The recognizer keeps the managed model's version in use.
            (managed.path, executable.retaining(managed.hold))
        };
        Ok(SelectedRecognizer::Whisper(
            self.whisper_recognizer(executable, &model, capacity)?,
        ))
    }

    /// whisper.cpp with `model`, the host's isolation and the machine's
    /// recognizer threads capped by the root's admission `capacity`
    /// ([`recognizer_threads`]): the count is its admission weight and is
    /// recorded in every run's provenance, so the same audio recognised on
    /// a root of smaller capacity is a different run (known limit L-023).
    pub(crate) fn whisper_recognizer(
        &self,
        executable: TrustedExecutable,
        model: &Path,
        capacity: NonZeroU16,
    ) -> Result<WhisperCli, EngineError> {
        WhisperCli::new(
            executable,
            model,
            recognizer_threads(std::thread::available_parallelism().ok(), capacity),
            self.config().host_isolation.into_infrastructure(),
        )
        .map_err(|_: WhisperError| EngineError::LocalAsrModelUnavailable)
    }

    /// Ensures the selected recognizer transcribes the reviewed speech fixture
    /// before it touches user media, once per identity (see
    /// [`preflight_local_asr`]), and returns the fingerprint the pass is
    /// recorded under, which the recognition key binds.
    async fn ensure_local_asr_verified(
        &self,
        tools: &MediaProviderConformance,
        recognizer: &SelectedRecognizer<'_>,
        identity: &RecognizerIdentity,
        cancellation: &ProcessCancellation,
    ) -> Result<Option<MediaToolFingerprint>, EngineError> {
        let preflight = self.prepare_local_asr_preflight(tools, recognizer, identity)?;
        self.run_local_asr_preflight(&preflight, tools, recognizer, identity, cancellation)
            .await
            .map_err(EngineError::LocalAsrVerificationFailed)?;
        Ok(preflight.fingerprint)
    }

    /// Opens the per-user verification record and derives the fingerprint a
    /// local-ASR pass for this setup is recorded under. The verifier's
    /// authority is part of it: the reviewed fixture, or a host-supplied
    /// verifier or recognizer, never stand in for each other.
    ///
    /// # Errors
    ///
    /// Fails when the reviewed policy, the clock or the configuration cannot
    /// be read, and with a media-tool workspace failure when the per-user
    /// verification directory cannot be used.
    pub(crate) fn prepare_local_asr_preflight(
        &self,
        tools: &MediaProviderConformance,
        recognizer: &SelectedRecognizer<'_>,
        identity: &RecognizerIdentity,
    ) -> Result<LocalAsrPreflight, EngineError> {
        let policy =
            reviewed_compatibility_policy().map_err(|_| EngineError::ReviewedPolicyInvalid)?;
        let now = self.now_unix_seconds()?;
        let isolation = self.config().host_isolation.into_infrastructure();
        let state = self
            .user_configuration()?
            .media_tool_verification_state()
            .map_err(|_| {
                EngineError::MediaToolVerificationFailed(MediaToolPreflightFailure {
                    check: MediaToolCheck::Preparation,
                    failure: MediaToolFailure::Workspace,
                })
            })?;
        let media_authority = if self.host_media_tool_verifier().is_some() {
            MediaToolVerificationAuthority::HostSupplied
        } else {
            MediaToolVerificationAuthority::ReviewedFixture
        };
        let (files, authority) = match recognizer {
            SelectedRecognizer::Host(_) => (
                LocalAsrFiles::HostSupplied,
                MediaToolVerificationAuthority::HostSupplied,
            ),
            SelectedRecognizer::Whisper(cli) => (
                LocalAsrFiles::Selected {
                    whisper: cli.executable().path(),
                    model: cli.model(),
                },
                if self.host_local_asr_verifier().is_some() {
                    MediaToolVerificationAuthority::HostSupplied
                } else {
                    MediaToolVerificationAuthority::ReviewedFixture
                },
            ),
        };
        let fingerprint = media_tool_fingerprint(tools, isolation, &policy, media_authority)
            .and_then(|media| local_asr_fingerprint(&media, files, identity, isolation, authority));
        Ok(LocalAsrPreflight {
            state,
            fingerprint,
            now,
        })
    }

    /// Runs the local-ASR preflight with the verifier matching `recognizer`:
    /// the host's, a host-supplied replacement for the fixture, or the
    /// reviewed speech fixture.
    pub(crate) async fn run_local_asr_preflight(
        &self,
        preflight: &LocalAsrPreflight,
        tools: &MediaProviderConformance,
        recognizer: &SelectedRecognizer<'_>,
        identity: &RecognizerIdentity,
        cancellation: &ProcessCancellation,
    ) -> Result<MediaToolPreflightOutcome, LocalAsrVerificationFailure> {
        let LocalAsrPreflight {
            state,
            fingerprint,
            now,
        } = preflight;
        match recognizer {
            SelectedRecognizer::Host(host) => {
                let verifier = HostVerifierRef(host.verifier.as_ref());
                preflight_local_asr(&verifier, state, fingerprint.as_ref(), *now).await
            }
            SelectedRecognizer::Whisper(cli) => {
                if let Some(verifier) = self.host_local_asr_verifier() {
                    preflight_local_asr(&verifier, state, fingerprint.as_ref(), *now).await
                } else {
                    let verifier = FixtureAsrVerifier::new(
                        tools.clone(),
                        self.config().host_isolation.into_infrastructure(),
                        state.workspace_parent().to_path_buf(),
                        cli.clone(),
                        identity.clone(),
                        cancellation.clone(),
                    );
                    preflight_local_asr(&verifier, state, fingerprint.as_ref(), *now).await
                }
            }
        }
    }
}

/// The per-user verification record and the fingerprint one local-ASR setup
/// is recorded under.
pub(crate) struct LocalAsrPreflight {
    state: FilesystemMediaToolVerificationCache,
    fingerprint: Option<MediaToolFingerprint>,
    now: u64,
}

impl LocalAsrPreflight {
    /// Whether a still-valid pass for exactly this setup is recorded.
    pub(crate) fn recorded(&self) -> bool {
        self.fingerprint.as_ref().is_some_and(|fingerprint| {
            self.state.lookup(fingerprint, self.now) == CachedMediaToolVerification::Verified
        })
    }
}

/// Resolves the whisper.cpp CLI: `configured` when given, otherwise the
/// managed version (with its hold), otherwise `whisper-cli` on the filtered
/// `PATH`.
pub(crate) fn resolve_recognizer(
    managed: &mut ManagedLookup<'_>,
    configured: Option<PathBuf>,
) -> Result<TrustedExecutable, EngineError> {
    if configured.is_none()
        && let Some(executable) = managed.executable(RuntimeDependency::Whisper)
    {
        return Ok(executable);
    }
    resolve_whisper(configured)
}

/// Resolves the whisper.cpp CLI: `selected` when given, otherwise
/// `whisper-cli` on the filtered `PATH`.
pub(crate) fn resolve_whisper(selected: Option<PathBuf>) -> Result<TrustedExecutable, EngineError> {
    match selected {
        Some(path) => TrustedExecutable::explicit(path),
        None => ExecutableResolver::from_current_path().resolve(OsStr::new("whisper-cli")),
    }
    .map_err(|error| match error {
        ExecutableResolutionError::NotFound => {
            EngineError::LocalAsrToolUnavailable(RuntimeDependency::Whisper)
        }
        other => EngineError::Executable(ExecutableRejection::from(&other)),
    })
}

/// The committed status of an open, unexpired session.
pub(crate) fn open_status(
    store: &FilesystemSessionStore,
    session: &SessionId,
    now: u64,
) -> Result<SessionStatus, EngineError> {
    let status = store.session_status(session)?;
    if status.phase() != SessionPhase::Open || status.lifetime().expired(now) {
        return Err(EngineError::Storage(SessionStorageError::StateConflict));
    }
    Ok(status)
}

/// Refuses work whose admission weight exceeds the root's whole capacity:
/// it could never be admitted, so it fails with `RESOURCE_LIMIT` before any
/// work instead of waiting or retrying (X-07).
pub(crate) fn admission_fits(
    store: &FilesystemSessionStore,
    weight: NonZeroU16,
) -> Result<(), EngineError> {
    let capacity = store.admission_capacity();
    if weight.get() > capacity {
        return Err(EngineError::AdmissionExceedsCapacity {
            weight: weight.get(),
            capacity,
        });
    }
    Ok(())
}

/// Storage failure for a committed source copy that could not be reopened.
pub(crate) const fn snapshot_storage_error(error: &SourceError) -> SessionStorageError {
    match error {
        SourceError::Storage(storage) => *storage,
        SourceError::SnapshotChanged
        | SourceError::IdentityFailure
        | SourceError::UnsupportedContainer
        | SourceError::TooLarge => SessionStorageError::IntegrityFailure,
        SourceError::InvalidPath
        | SourceError::NotRegularFile
        | SourceError::SymbolicLink
        | SourceError::Deadline
        | SourceError::ChangedDuringStage
        | SourceError::Cancelled
        | SourceError::Io(_) => SessionStorageError::Io,
    }
}

/// Probe failure of the committed source, as for supplied-transcript import.
pub(crate) const fn probe_error(error: &MediaError) -> SourceProbeError {
    match error {
        MediaError::CapacityUnavailable => SourceProbeError::Busy,
        MediaError::Deadline => SourceProbeError::Deadline,
        MediaError::Cancelled => SourceProbeError::Cancelled,
        MediaError::OutputLimit => SourceProbeError::ResourceLimit,
        MediaError::Process(_) | MediaError::Request(_) | MediaError::InvalidSourcePath => {
            SourceProbeError::Io
        }
        _ => SourceProbeError::InvalidSource,
    }
}

/// A recognizer and its verifier supplied by the embedding host.
pub(crate) struct HostAsr {
    recognizer: Box<dyn HostSpeechRecognizer>,
    verifier: Box<dyn HostLocalAsrVerifier>,
}

impl HostAsr {
    /// The host recognizer's reported identity.
    pub(crate) async fn identity(&self) -> Result<RecognizerIdentity, SpeechRecognitionError> {
        self.recognizer.identity_boxed().await
    }

    pub(crate) fn new(
        recognizer: impl SpeechRecognizer + 'static,
        verifier: impl LocalAsrVerifier + 'static,
    ) -> Self {
        Self {
            recognizer: Box::new(recognizer),
            verifier: Box::new(verifier),
        }
    }
}

type IdentityFuture<'a> =
    Pin<Box<dyn Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send + 'a>>;
type RecognitionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send + 'a>>;
type LocalAsrVerificationFuture<'a> =
    Pin<Box<dyn Future<Output = LocalAsrVerification> + Send + 'a>>;

/// Object-safe form of [`SpeechRecognizer`]; see the media-tool verifier's
/// form in `engine.rs` for why hosts are type-erased.
trait HostSpeechRecognizer: Send + Sync {
    fn identity_boxed(&self) -> IdentityFuture<'_>;
    fn recognize_boxed<'a>(
        &'a self,
        chunk: &'a PlannedChunk,
        pcm: &'a SpeechPcm,
    ) -> RecognitionFuture<'a>;
}

impl<Recognizer: SpeechRecognizer> HostSpeechRecognizer for Recognizer {
    fn identity_boxed(&self) -> IdentityFuture<'_> {
        Box::pin(self.identity())
    }

    fn recognize_boxed<'a>(
        &'a self,
        chunk: &'a PlannedChunk,
        pcm: &'a SpeechPcm,
    ) -> RecognitionFuture<'a> {
        Box::pin(self.recognize(chunk, pcm))
    }
}

struct HostRecognizerRef<'a>(&'a dyn HostSpeechRecognizer);

/// The recognizer one retranscription runs with, chosen before it starts,
/// so the job runs through a single call whichever it is.
enum RunRecognizer<'a> {
    /// whisper.cpp in the session's work directory.
    Whisper(WhisperSpeechRecognizer),
    /// The embedding host's recognizer.
    Host(HostRecognizerRef<'a>),
}

impl SpeechRecognizer for RunRecognizer<'_> {
    async fn identity(&self) -> Result<RecognizerIdentity, SpeechRecognitionError> {
        match self {
            Self::Whisper(whisper) => whisper.identity().await,
            Self::Host(host) => host.identity().await,
        }
    }

    async fn recognize(
        &self,
        chunk: &PlannedChunk,
        pcm: &SpeechPcm,
    ) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
        match self {
            Self::Whisper(whisper) => whisper.recognize(chunk, pcm).await,
            Self::Host(host) => host.recognize(chunk, pcm).await,
        }
    }
}

impl SpeechRecognizer for HostRecognizerRef<'_> {
    async fn identity(&self) -> Result<RecognizerIdentity, SpeechRecognitionError> {
        self.0.identity_boxed().await
    }

    async fn recognize(
        &self,
        chunk: &PlannedChunk,
        pcm: &SpeechPcm,
    ) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
        self.0.recognize_boxed(chunk, pcm).await
    }
}

/// Object-safe form of [`LocalAsrVerifier`].
pub(crate) trait HostLocalAsrVerifier: Send + Sync {
    fn verify_boxed(&self) -> LocalAsrVerificationFuture<'_>;
}

impl<Verifier: LocalAsrVerifier> HostLocalAsrVerifier for Verifier {
    fn verify_boxed(&self) -> LocalAsrVerificationFuture<'_> {
        Box::pin(self.verify())
    }
}

pub(crate) struct HostVerifierRef<'a>(pub(crate) &'a dyn HostLocalAsrVerifier);

impl LocalAsrVerifier for HostVerifierRef<'_> {
    fn verify(&self) -> impl Future<Output = LocalAsrVerification> + Send {
        self.0.verify_boxed()
    }
}
