//! Local speech recognition of a session's source: `transcript retranscribe`.
//!
//! [`Engine::retranscribe`] is the only way local ASR is requested (maintainer
//! decision D1, ADR 0017). `ingest` and `transcript get` never resolve
//! whisper.cpp or a model. The operation resolves every dependency and
//! identifies the model before anything runs, refuses a model that is not a
//! reviewed pinned profile (D5), runs the media-tool preflight and then the
//! local-ASR preflight, and only then holds the session, decodes speech from
//! its committed private source copy and recognises it in a private work
//! directory inside the session. The new revision is committed with one
//! generation publication; a failure or cancellation commits nothing.

use std::{
    ffi::OsStr,
    future::Future,
    num::{NonZeroU16, NonZeroU32},
    path::{Path, PathBuf},
    pin::Pin,
};

use vsift_application::{
    AsrFailure, AsrFailureReason, AsrRevisionRequest, AsrStage, CachedMediaToolVerification,
    LocalAsrVerification, LocalAsrVerificationFailure, LocalAsrVerifier, MediaToolCheck,
    MediaToolFailure, MediaToolFingerprint, MediaToolPreflightFailure, MediaToolPreflightOutcome,
    MediaToolVerificationCache, RecognizerIdentity, RevisionSplice, SessionStorageError,
    SourceProbeError, SpeechPcm, SpeechRecognitionError, SpeechRecognizer, TranscribeRangeRequest,
    build_asr_revision, preflight_local_asr, transcribe_range, whole_file_source_segment,
};
use vsift_domain::{
    AsrModelProfile, ChunkPlan, MediaSelection, MediaTime, PlannedChunk, ProviderChunkOutput,
    RuntimeDependency, SessionArtifactKind, SessionId, SessionPhase, TimeRange, TranscriptRevision,
};
use vsift_infrastructure::{
    ExecutableResolutionError, ExecutableResolver, FfmpegMedia, FfmpegSpeechAudio,
    FilesystemMediaToolVerificationCache, FilesystemSessionStore, FixtureAsrVerifier,
    LocalAsrFiles, MediaError, MediaProviderConformance, MediaToolVerificationAuthority,
    ProcessCancellation, ProcessWorkingDirectory, SessionStatus, SourceError, SourceSnapshot,
    TrustedExecutable, WhisperCli, WhisperError, WhisperSpeechRecognizer, encode_transcript_record,
    local_asr_fingerprint, media_tool_fingerprint, reviewed_compatibility_policy,
};

use crate::{
    engine::Engine,
    error::{EngineError, ExecutableRejection, SessionRootError},
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
    /// Signal that stops the run at its next provider boundary; nothing is
    /// committed after it fires.
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

/// A committed retranscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetranscribeOutcome {
    session: SessionSnapshot,
    requested: Option<TimeRange>,
    revision: TranscriptRevision,
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

    /// The new revision, now the session's newest.
    #[must_use]
    pub const fn revision(&self) -> &TranscriptRevision {
        &self.revision
    }
}

/// Which recognizer a retranscription uses.
pub(crate) enum SelectedRecognizer<'a> {
    /// whisper.cpp with the configured model, run in a work directory.
    Whisper(WhisperCli),
    /// The embedding host's recognizer and verifier.
    Host(&'a HostAsr),
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
    /// # Errors
    ///
    /// Fails before running anything for an invalid range, a missing tool or
    /// model, or a model that is not a reviewed pinned profile; then for a
    /// missing, closed or expired session; then for a failed media-tool or
    /// local-ASR preflight; and during the run for a source without audio, a
    /// typed recognition failure, cancellation or a storage failure. A
    /// concurrent change to the session (a renewal or another revision) fails
    /// the commit with a busy storage error. Nothing is committed on failure.
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
        let (base, status) = match store.read_transcript(&request.session, now)? {
            Some((base, status)) => (Some(base), status),
            None => (None, open_status(&store, &request.session, now)?),
        };

        // 3. Prove the tools and the recognizer work before touching user media.
        self.ensure_media_tools_verified(&tools).await?;
        self.ensure_local_asr_verified(&tools, &recognizer, &identity, &cancellation)
            .await?;

        // 4. Hold the session's committed source and a private work directory.
        let snapshot = SourceSnapshot::open_committed(&store, &request.session, now)
            .map_err(|error| EngineError::Storage(snapshot_storage_error(&error)))?;
        let work = store.session_work_directory(&request.session, now)?;
        let media = FfmpegMedia::new(
            tools.clone(),
            self.config().host_isolation.into_infrastructure(),
            &store,
        );
        let description = media
            .probe(&snapshot, cancellation.clone())
            .await
            .map_err(|error| EngineError::SourceProbe(probe_error(&error)))?;
        let stream = description
            .speech_audio_stream()
            .ok_or(EngineError::NoAudioStream)?;
        let source = whole_file_source_segment(snapshot.id(), description.duration)
            .map_err(|_| EngineError::SourceProbe(SourceProbeError::InvalidSource))?;
        if let Some(range) = requested
            && range.end() > source.range().end()
        {
            return Err(EngineError::RangeOutsideSource);
        }
        let replaced = match (&base, requested) {
            (_, None) => source.range(),
            (Some(base), Some(range)) => base.snap_to_segments(range),
            (None, Some(range)) => range,
        };

        // 5. Recognise, then assemble the complete revision. One admission slot
        // of the session root is held for the whole run, so concurrent
        // retranscriptions cannot oversubscribe the machine (SEC-20); each
        // chunk's decoding takes its own slot as every media stage does.
        let _recognition = store.try_admit(1)?;
        let audio = FfmpegSpeechAudio::new(
            &media,
            &snapshot,
            &description,
            MediaSelection {
                video: None,
                audio: Some(stream),
            },
            cancellation.clone(),
        );
        let transcribe = TranscribeRangeRequest {
            source_segment: &source,
            range: replaced,
            plan: ChunkPlan::R0,
            audio_stream: stream,
            expected: &identity,
        };
        let transcription = match &recognizer {
            SelectedRecognizer::Whisper(cli) => {
                let chunks = ProcessWorkingDirectory::new(work.path()).map_err(|_| {
                    EngineError::LocalAsrFailed(AsrFailure {
                        stage: AsrStage::Planning,
                        reason: AsrFailureReason::Workspace,
                    })
                })?;
                let whisper =
                    WhisperSpeechRecognizer::new(cli.clone(), chunks, cancellation.clone());
                transcribe_range(transcribe, &audio, &whisper, &cancellation).await
            }
            SelectedRecognizer::Host(host) => {
                let supplied = HostRecognizerRef(host.recognizer.as_ref());
                transcribe_range(transcribe, &audio, &supplied, &cancellation).await
            }
        }
        .map_err(EngineError::LocalAsrFailed)?;
        let number = base
            .as_ref()
            .map_or(Some(1), |base| base.number().checked_add(1))
            .and_then(NonZeroU32::new)
            .ok_or(EngineError::Storage(SessionStorageError::CapacityExhausted))?;
        let revision = build_asr_revision(AsrRevisionRequest {
            session_id: &request.session,
            source_id: snapshot.id(),
            source_segment: &source,
            number,
            transcription,
            splice: base.as_ref().map(|base| RevisionSplice {
                base,
                replaced_range: replaced,
            }),
        })
        .map_err(EngineError::TranscriptAssembly)?;
        if cancellation.is_cancelled() {
            return Err(EngineError::LocalAsrFailed(AsrFailure {
                stage: AsrStage::Assembly,
                reason: AsrFailureReason::Cancelled,
            }));
        }

        // 6. Commit against the generation observed before the run.
        let record = encode_transcript_record(&revision)?;
        let committed = store.publish_artifact(
            &request.session,
            &self.new_operation_id()?,
            status.generation(),
            SessionArtifactKind::TranscriptRecord,
            &record,
            self.now_unix_seconds()?,
        );
        drop(work);
        drop(snapshot);
        let now = self.now_unix_seconds()?;
        match committed {
            Ok(_) => {}
            Err(SessionStorageError::StateConflict) => {
                // A session that is still open lost a race with another
                // writer (a renewal or another revision): retry later.
                return Err(match open_status(&store, &request.session, now) {
                    Ok(_) => EngineError::Storage(SessionStorageError::Busy),
                    Err(error) => error,
                });
            }
            Err(error) => return Err(EngineError::Storage(error)),
        }
        let status = store.session_status(&request.session)?;
        Ok(RetranscribeOutcome {
            session: SessionSnapshot::observe(&status, now),
            requested,
            revision,
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
    /// [`preflight_local_asr`]).
    async fn ensure_local_asr_verified(
        &self,
        tools: &MediaProviderConformance,
        recognizer: &SelectedRecognizer<'_>,
        identity: &RecognizerIdentity,
        cancellation: &ProcessCancellation,
    ) -> Result<(), EngineError> {
        let preflight = self.prepare_local_asr_preflight(tools, recognizer, identity)?;
        self.run_local_asr_preflight(&preflight, tools, recognizer, identity, cancellation)
            .await
            .map(|_| ())
            .map_err(EngineError::LocalAsrVerificationFailed)
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
fn open_status(
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
const fn snapshot_storage_error(error: &SourceError) -> SessionStorageError {
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
const fn probe_error(error: &MediaError) -> SourceProbeError {
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
