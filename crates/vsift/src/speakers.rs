//! Speaker labels: diarize a transcript revision, record verified names, and read the
//! labels beside transcript pages and search hits (ADR 0031).
//!
//! The engine runs the external diarization tool over the session's committed source
//! copy, as it runs whisper.cpp: the tool and its two model files are selected first,
//! the media tools are verified, the copy is bound and hashed, the audio is decoded by
//! the restricted `FFmpeg` adapter into the operation's private work folder, and the
//! record is committed only after the copy is verified again. A speaker record is
//! immutable; naming a label commits the next record of the same turns.

use std::{ffi::OsStr, num::NonZeroU32, path::PathBuf};

use vsift_application::{
    LabelSpeakersRequest, SessionStorageError, SpeakerLabelError, label_speakers, name_speaker,
};
use vsift_domain::{
    MediaSelection, MediaTime, SessionArtifactKind, SessionId, SpeakerCount, SpeakerRecord,
    SpeakerRecordParts, SpeakerTag, TimeRange, TranscriptRevision, TranscriptRevisionId,
    VerifiedName, recognizer_threads,
};
use vsift_infrastructure::{
    BoundSource, ExecutableResolutionError, ExecutableResolver, FfmpegMedia, FfmpegSherpaDiarizer,
    FilesystemSessionStore, ProcessWorkingDirectory, SessionStatus, SherpaOnnxDiarizer,
    TrustedExecutable, encode_speaker_record,
};

use crate::{
    asr::{admission_fits, probe_error, snapshot_storage_error},
    engine::{Engine, absolute_selection},
    error::{EngineError, ExecutableRejection, SessionRootError},
    sessions::SessionSnapshot,
    verification::Cancellation,
};

/// The name the diarization tool has in sherpa-onnx's builds; looked up on the
/// filtered `PATH` when no path is given. The models are never looked up there.
pub const DIARIZER_EXECUTABLE: &str = "sherpa-onnx-offline-speaker-diarization";

const MAX_COMMIT_ATTEMPTS: usize = 3;

/// The diarization tool and its two models, as the caller selected them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiarizerSelection {
    /// The tool; `None` looks for [`DIARIZER_EXECUTABLE`] on the filtered `PATH`.
    pub executable: Option<PathBuf>,
    /// The pyannote segmentation 3.0 model (`model.onnx`).
    pub segmentation_model: Option<PathBuf>,
    /// The speaker-embedding model (`NeMo` `TitaNet` small).
    pub embedding_model: Option<PathBuf>,
}

/// A half-open source range in microseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeakersRange {
    /// Inclusive start.
    pub from_micros: u64,
    /// Exclusive end.
    pub to_micros: u64,
}

/// A request to label a transcript revision's speakers.
#[derive(Clone, Debug)]
pub struct SpeakersRequest {
    /// The session.
    pub session: SessionId,
    /// The revision to label; `None` labels the newest.
    pub revision: Option<TranscriptRevisionId>,
    /// The range to hear; `None` hears the whole source (within the duration cap).
    pub range: Option<SpeakersRange>,
    /// The number of speakers, when the user knows it; `None` is the engine's
    /// automatic count.
    pub speakers: Option<u8>,
    /// The tool and models.
    pub diarizer: DiarizerSelection,
    /// Signal that stops the run before it commits.
    pub cancellation: Cancellation,
}

/// A request to record, or clear, the verified name of a speaker label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameSpeakerRequest {
    /// The session.
    pub session: SessionId,
    /// The revision whose newest speaker record is named; `None` is the newest
    /// revision.
    pub revision: Option<TranscriptRevisionId>,
    /// The label, such as `S2`.
    pub label: String,
    /// The name the user verified; `None` clears the label's name.
    pub name: Option<String>,
}

/// A speaker record with the revision it labels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpeakersView {
    session: SessionSnapshot,
    revision: TranscriptRevision,
    record: SpeakerRecord,
    records: usize,
}

impl SpeakersView {
    /// Session state observed when the record was committed or read.
    #[must_use]
    pub const fn session(&self) -> &SessionSnapshot {
        &self.session
    }

    /// The revision the record labels.
    #[must_use]
    pub const fn revision(&self) -> &TranscriptRevision {
        &self.revision
    }

    /// The record, with every per-segment label derived.
    #[must_use]
    pub const fn record(&self) -> &SpeakerRecord {
        &self.record
    }

    /// How many speaker records the session holds, this one included.
    #[must_use]
    pub const fn records(&self) -> usize {
        self.records
    }
}

impl Engine {
    /// Diarizes a session's source and commits speaker labels for one of its
    /// transcript revisions.
    ///
    /// # Errors
    ///
    /// Fails for an invalid range or count, a missing tool or model
    /// (`MISSING_CAPABILITY`), a range longer than the cap (`RESOURCE_LIMIT`), a session
    /// without the revision, an engine failure, or a storage failure; nothing is
    /// committed then.
    #[allow(
        clippy::too_many_lines,
        reason = "The stage order is the contract; keep it visible in one place"
    )]
    pub async fn label_speakers(
        &self,
        request: SpeakersRequest,
    ) -> Result<SpeakersView, EngineError> {
        let range = request
            .range
            .map(|range| {
                TimeRange::new(
                    MediaTime::from_micros(range.from_micros),
                    MediaTime::from_micros(range.to_micros),
                )
                .map_err(|_| EngineError::InvalidTimeRange)
            })
            .transpose()?;
        let count = match request.speakers {
            None => SpeakerCount::Automatic,
            Some(count) => {
                SpeakerCount::given(count).map_err(|_| EngineError::InvalidSpeakerCount)?
            }
        };
        let cancellation = request.cancellation.0.clone();

        // 1. Select the tool, the models and the media tools before reading anything.
        let threads = recognizer_threads(
            std::thread::available_parallelism().ok(),
            self.admission_capacity_hint(),
        );
        let diarizer = select_diarizer(&request.diarizer, threads, self)?;
        let tools = self.local_asr_media_tools()?;

        // 2. The session and the revision must exist before any work starts.
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let revision = read_revision(&store, &request.session, request.revision.as_ref(), now)?;
        admission_fits(&store, threads)?;

        // 3. Prove the media tools work, then hold the committed copy and a private
        // work folder; the copy is hashed now and again before the commit (#148).
        self.ensure_media_tools_verified(&tools).await?;
        let bound = BoundSource::open_committed(&store, &request.session, now)
            .map_err(|error| EngineError::Storage(snapshot_storage_error(&error)))?;
        let work = store.session_work_directory(&request.session, now)?;
        let workspace = ProcessWorkingDirectory::new(work.path())
            .map_err(|_| EngineError::Storage(SessionStorageError::Io))?;
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
        if revision.source_id() != bound.snapshot().id() {
            return Err(EngineError::Storage(SessionStorageError::IntegrityFailure));
        }

        // 4. Diarize, holding the engine's threads of the root's capacity for the run.
        let permit = store
            .try_admit(threads.get())
            .map_err(EngineError::Storage)?;
        let existing = store.read_speaker_record(&request.session, None, now)?;
        let number = next_number(existing.records)?;
        // The run holds the engine's threads of the root's capacity; that reservation
        // also covers the audio decoding before it, so the decodes take none of their
        // own (as a transcription's do).
        let speech_media = media.within_caller_admission();
        let adapter = FfmpegSherpaDiarizer::new(
            &speech_media,
            &bound,
            &description,
            MediaSelection {
                video: None,
                audio: Some(stream),
            },
            &workspace,
            &diarizer,
            cancellation.clone(),
        );
        let labelled = label_speakers(
            LabelSpeakersRequest {
                session_id: &request.session,
                revision: &revision,
                source_duration: description.duration,
                range,
                count,
                number,
            },
            &adapter,
        )
        .await;
        drop(adapter);
        drop(permit);
        let record = labelled.map_err(|error| match error {
            SpeakerLabelError::Diarization(vsift_application::DiarizationRunError::Source)
                if bound.check_identity().is_err() =>
            {
                EngineError::Storage(SessionStorageError::IntegrityFailure)
            }
            other => EngineError::SpeakerLabels(other),
        })?;

        // 5. Commit only if the copy still holds the committed bytes.
        let snapshot = bound
            .release_verified()
            .map_err(|error| EngineError::Storage(snapshot_storage_error(&error)))?;
        let view = self.commit_speaker_record(&store, &request.session, &revision, record)?;
        drop(snapshot);
        drop(work);
        Ok(view)
    }

    /// Records, or clears, the verified name of a label of the newest speaker record of
    /// a revision, as the next speaker record.
    ///
    /// # Errors
    ///
    /// Fails for a label or name that does not parse, a revision without a speaker
    /// record, a label the record does not have, or a storage failure.
    pub fn name_speaker(&self, request: NameSpeakerRequest) -> Result<SpeakersView, EngineError> {
        let NameSpeakerRequest {
            session,
            revision,
            label,
            name,
        } = request;
        let tag = SpeakerTag::parse(&label).map_err(|_| EngineError::InvalidSpeakerLabel)?;
        let name = name
            .as_deref()
            .map(VerifiedName::parse)
            .transpose()
            .map_err(|_| EngineError::InvalidSpeakerName)?;
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let revision = read_revision(&store, &session, revision.as_ref(), now)?;
        let read = store.read_speaker_record(&session, Some(revision.id()), now)?;
        let parts = read.record.ok_or(EngineError::SpeakerRecordNotFound)?;
        let current = SpeakerRecord::derive(parts, &revision)
            .map_err(|_| EngineError::Storage(SessionStorageError::IntegrityFailure))?;
        let named = name_speaker(
            &session,
            &current,
            &revision,
            tag,
            name,
            next_number(read.records)?,
        )
        .map_err(EngineError::SpeakerLabels)?;
        self.commit_speaker_record(&store, &session, &revision, named)
    }

    /// The newest speaker record of the session's newest revision (or of `revision`),
    /// with its labels derived; `None` when the revision has none.
    ///
    /// # Errors
    ///
    /// Fails for a missing session or revision, or a record that fails its integrity
    /// checks.
    pub fn speakers(
        &self,
        session: &SessionId,
        revision: Option<&TranscriptRevisionId>,
    ) -> Result<Option<SpeakersView>, EngineError> {
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        let revision = read_revision(&store, session, revision, now)?;
        let read = store.read_speaker_record(session, Some(revision.id()), now)?;
        let Some(parts) = read.record else {
            return Ok(None);
        };
        let record = SpeakerRecord::derive(parts, &revision)
            .map_err(|_| EngineError::Storage(SessionStorageError::IntegrityFailure))?;
        Ok(Some(SpeakersView {
            session: SessionSnapshot::observe(&read.status, now),
            revision,
            record,
            records: read.records,
        }))
    }

    fn commit_speaker_record(
        &self,
        store: &FilesystemSessionStore,
        session: &SessionId,
        revision: &TranscriptRevision,
        mut record: SpeakerRecord,
    ) -> Result<SpeakersView, EngineError> {
        for _ in 0..MAX_COMMIT_ATTEMPTS {
            let now = self.now_unix_seconds()?;
            let read = store.read_speaker_record(session, None, now)?;
            let number = next_number(read.records)?;
            if record.parts().number != number {
                // Another writer committed a speaker record meanwhile: take the next
                // number (and with it the next identity) for the same content.
                record = renumbered(session, revision, &record, number)?;
            }
            let bytes = encode_speaker_record(session, record.parts())?;
            match store.publish_artifact(
                session,
                &self.new_operation_id()?,
                read.status.generation(),
                SessionArtifactKind::SpeakerRecord,
                &bytes,
                self.now_unix_seconds()?,
            ) {
                Ok(_) => {
                    let status: SessionStatus = crate::sessions::published_status(store, session)?;
                    return Ok(SpeakersView {
                        session: SessionSnapshot::observe(&status, self.now_unix_seconds()?),
                        revision: revision.clone(),
                        record,
                        records: read.records + 1,
                    });
                }
                Err(SessionStorageError::StateConflict) => {}
                Err(error) => return Err(EngineError::Storage(error)),
            }
        }
        Err(EngineError::Storage(SessionStorageError::Busy))
    }
}

/// The newest speaker record of `revision`, with its labels derived; `None` when the
/// revision has none.
pub(crate) fn newest_record(
    store: &FilesystemSessionStore,
    session: &SessionId,
    revision: &TranscriptRevision,
    now: u64,
) -> Result<Option<SpeakerRecord>, EngineError> {
    let read = store.read_speaker_record(session, Some(revision.id()), now)?;
    read.record
        .map(|parts| {
            SpeakerRecord::derive(parts, revision)
                .map_err(|_| EngineError::Storage(SessionStorageError::IntegrityFailure))
        })
        .transpose()
}

/// The label a search filter names: `S2`, or a name the user verified for a label
/// (compared exactly, after trimming).
pub(crate) fn resolve_speaker(
    record: Option<&SpeakerRecord>,
    wanted: &str,
) -> Result<SpeakerTag, EngineError> {
    let record = record.ok_or(EngineError::SpeakerRecordNotFound)?;
    if let Ok(tag) = SpeakerTag::parse(wanted.trim()) {
        if record.turns().iter().any(|turn| turn.tag() == tag) {
            return Ok(tag);
        }
        return Err(EngineError::SpeakerLabels(SpeakerLabelError::Record(
            vsift_domain::DiarizationError::NameForUnknownLabel,
        )));
    }
    let wanted = wanted.trim();
    record
        .speakers()
        .into_iter()
        .map(vsift_domain::SpeakerSummary::tag)
        .find(|tag| {
            record
                .name(*tag)
                .is_some_and(|name| name.as_str() == wanted)
        })
        .ok_or(EngineError::SpeakerLabels(SpeakerLabelError::Record(
            vsift_domain::DiarizationError::NameForUnknownLabel,
        )))
}

fn read_revision(
    store: &FilesystemSessionStore,
    session: &SessionId,
    revision: Option<&TranscriptRevisionId>,
    now: u64,
) -> Result<TranscriptRevision, EngineError> {
    match revision {
        None => store
            .read_transcript(session, now)?
            .map(|(revision, _)| revision)
            .ok_or(EngineError::TranscriptUnavailable),
        Some(id) => store
            .read_transcript_revision(session, id, now)?
            .map(|(revision, _)| revision)
            .ok_or(EngineError::TranscriptRevisionNotFound),
    }
}

fn next_number(records: usize) -> Result<NonZeroU32, EngineError> {
    u32::try_from(records)
        .ok()
        .and_then(|count| count.checked_add(1))
        .and_then(NonZeroU32::new)
        .ok_or(EngineError::Storage(SessionStorageError::CapacityExhausted))
}

fn renumbered(
    session: &SessionId,
    revision: &TranscriptRevision,
    record: &SpeakerRecord,
    number: NonZeroU32,
) -> Result<SpeakerRecord, EngineError> {
    let mut parts: SpeakerRecordParts = record.parts().clone();
    parts.number = number;
    parts.id =
        vsift_application::speaker_record_id(session, revision.source_id(), revision.id(), number)
            .map_err(EngineError::SpeakerLabels)?;
    SpeakerRecord::derive(parts, revision)
        .map_err(|error| EngineError::SpeakerLabels(SpeakerLabelError::Record(error)))
}

/// The tool from its path or the filtered `PATH`, and both models from their paths.
fn select_diarizer(
    selection: &DiarizerSelection,
    threads: std::num::NonZeroU16,
    engine: &Engine,
) -> Result<SherpaOnnxDiarizer, EngineError> {
    let executable = match &selection.executable {
        Some(path) => TrustedExecutable::explicit(absolute_selection(path)?),
        None => ExecutableResolver::from_current_path().resolve(OsStr::new(DIARIZER_EXECUTABLE)),
    }
    .map_err(|error| match error {
        ExecutableResolutionError::NotFound => EngineError::DiarizerUnavailable,
        other => EngineError::Executable(ExecutableRejection::from(&other)),
    })?;
    let (Some(segmentation), Some(embedding)) =
        (&selection.segmentation_model, &selection.embedding_model)
    else {
        return Err(EngineError::DiarizerUnavailable);
    };
    SherpaOnnxDiarizer::new(
        executable,
        &absolute_selection(segmentation)?,
        &absolute_selection(embedding)?,
        threads,
        engine.config().host_isolation.into_infrastructure(),
    )
    .map_err(|_| EngineError::DiarizerUnavailable)
}
