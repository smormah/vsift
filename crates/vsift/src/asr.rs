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
    AsrModelProfile, ChunkPlan, JobId, MediaSelection, MediaTime, OperationId, PlannedChunk,
    ProviderChunkOutput, RuntimeDependency, SessionId, SessionPhase, TimeRange, TranscriptRevision,
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
    sessions::SessionSnapshot,
    verification::Cancellation,
};

/// Most recognizer threads a run asks for. More gives little on the pinned
/// base model and would starve the rest of the machine.
const MAX_RECOGNIZER_THREADS: u16 = 8;

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
        }
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
    /// Fails before running anything for an invalid range, a missing tool or
    /// model, or a model that is not a reviewed pinned profile; then for a
    /// missing, closed or expired session; then with
    /// [`EngineError::IdempotencyConflict`] or [`EngineError::JobBusy`] for an
    /// operation id bound to another request or to a live job; then for a
    /// failed media-tool or local-ASR preflight; and during the run for a
    /// source without audio, a typed recognition failure, cancellation, a
    /// storage failure (including a source copy that changed), the same job
    /// running in another process, or a range another revision changed.
    /// Nothing is committed on failure, and the job stays resumable unless
    /// the error says otherwise.
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

        // 1. Resolve and identify everything before anything runs.
        let tools = self.local_asr_media_tools()?;
        let recognizer = self.select_recognizer()?;
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

        // 2. The session must exist and be open before minutes of work start.
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        // The newest revision and the generation come from one manifest, so
        // the commit below never builds on a base older than it expects.
        let (base, status) = store.read_transcript_head(&request.session, now)?;

        // 3. A retry whose operation id already committed is answered from
        // the commit, before any check or hash runs (X-02); the same id with
        // another request is a conflict (X-03).
        if let Some(operation) = &request.operation_id {
            match lookup_operation(&store, &request.session, operation, &digest, now)? {
                OperationLookup::Replay(record) => {
                    return self.replayed(&store, requested, &record, operation.clone());
                }
                OperationLookup::Busy(job) => return Err(EngineError::JobBusy { job }),
                OperationLookup::Unbound | OperationLookup::Continue(_) => {}
            }
        }

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
        };

        // 7. Recognise from the job's checkpoints and the audio, assemble the
        // complete revision, verify the copy and commit exactly once. The job
        // holds one admission slot of the root while it recognises (SEC-20);
        // each chunk's decoding takes its own slot as every media stage does.
        let audio = FfmpegSpeechAudio::new(
            &media,
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
        };
        let mut guard = SourceUnchanged(&bound);
        let outcome = match &recognizer {
            SelectedRecognizer::Whisper(cli) => {
                let chunks = ProcessWorkingDirectory::new(work.path()).map_err(|_| {
                    EngineError::LocalAsrFailed(AsrFailure {
                        stage: AsrStage::Planning,
                        reason: AsrFailureReason::Workspace,
                    })
                })?;
                let whisper =
                    WhisperSpeechRecognizer::new(cli.clone(), chunks, cancellation.clone());
                run_retranscription(
                    run,
                    RetranscriptionPorts {
                        store: &store,
                        audio: &audio,
                        recognizer: &whisper,
                        cancellation: &cancellation,
                        timer: &TokioRetryTimer,
                        classify: job_failure_code,
                    },
                    &mut guard,
                )
                .await
            }
            SelectedRecognizer::Host(host) => {
                let supplied = HostRecognizerRef(host.recognizer.as_ref());
                run_retranscription(
                    run,
                    RetranscriptionPorts {
                        store: &store,
                        audio: &audio,
                        recognizer: &supplied,
                        cancellation: &cancellation,
                        timer: &TokioRetryTimer,
                        classify: job_failure_code,
                    },
                    &mut guard,
                )
                .await
            }
        }
        .map_err(EngineError::from)?;
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
    fn select_recognizer(&self) -> Result<SelectedRecognizer<'_>, EngineError> {
        if let Some(host) = self.host_asr() {
            return Ok(SelectedRecognizer::Host(host));
        }
        let store = self.user_configuration()?;
        let executable = resolve_whisper(store.read()?.whisper)?;
        let model = store.read_model()?.ok_or(EngineError::ModelNotSelected)?;
        Ok(SelectedRecognizer::Whisper(
            self.whisper_recognizer(executable, &model)?,
        ))
    }

    /// whisper.cpp with `model`, the machine's recognizer threads and the
    /// host's isolation.
    pub(crate) fn whisper_recognizer(
        &self,
        executable: TrustedExecutable,
        model: &Path,
    ) -> Result<WhisperCli, EngineError> {
        WhisperCli::new(
            executable,
            model,
            recognizer_threads(),
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

/// Recognizer threads: the machine's parallelism, at most
/// [`MAX_RECOGNIZER_THREADS`]. The count is recorded in every run's provenance.
fn recognizer_threads() -> NonZeroU16 {
    std::thread::available_parallelism()
        .ok()
        .and_then(|threads| u16::try_from(threads.get()).ok())
        .map_or(4, |threads| threads.min(MAX_RECOGNIZER_THREADS))
        .try_into()
        .unwrap_or(NonZeroU16::MIN)
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
        | SourceError::Deadline
        | SourceError::ChangedDuringStage
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
