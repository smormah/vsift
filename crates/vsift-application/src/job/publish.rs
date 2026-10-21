//! Publishing a retranscription as it goes (ADR 0033).
//!
//! A run that publishes as it goes commits its revision in parts while it
//! runs: after a chunk, [`Publisher`] builds the open revision of what the run
//! has settled ([`crate::build_open_asr_revision`]) and commits it as the next
//! part, at most once every interval and seldom enough that the revision has
//! at most [`MAX_TRANSCRIPT_PARTS`] parts. The run seals the revision when it
//! ends; a run that is cancelled, or fails in a way it cannot resume from,
//! seals it as stopped at what it published ([`seal_stopped`]). A run that is
//! interrupted leaves it open, and a resumed run continues it.
//!
//! Publishing is advisory for the run: a part that cannot be committed now
//! (contention, a generation another command moved) is tried again after the
//! next chunk, and anything else stops the run publishing without stopping
//! the run, whose revision then appears at its seal as before.

use std::{num::NonZeroU32, sync::Mutex, time::Duration};

use vsift_domain::{
    JobId, MAX_EARLY_AUDIO_MICROS, MAX_TRANSCRIPT_PARTS, MERGE_RULES_VERSION, OperationId,
    TranscriptRevision,
};

use super::{JobKeyError, RetranscriptionRun, RetryTimer, RevisionStore};
use crate::{
    AsrRevisionRequest, RevisionSplice, RunObserver, RunSoFar, SessionStorageError,
    TranscriptBuildError, build_open_asr_revision, build_stopped_asr_revision,
    transcript::derived_identity,
};

/// Reads a monotonic clock: how long the process has run, for the interval
/// between two parts. Injected, like every clock a use case reads.
pub trait Stopwatch: Send + Sync {
    /// Time since an origin fixed for the process.
    fn elapsed(&self) -> Duration;
}

/// How a run publishes its revision before it ends (ADR 0033).
#[derive(Clone, Copy)]
pub enum Publication<'a> {
    /// Only when it ends: the revision is sealed in one generation, as before
    /// ADR 0033 (the quick and queued modes, a worker's step).
    AtTheSeal,
    /// As it goes: a part at most once every `interval` (read on
    /// `stopwatch`), and seldom enough that the revision has at most
    /// [`MAX_TRANSCRIPT_PARTS`] parts.
    AsItGoes {
        /// The least time between two parts.
        interval: Duration,
        /// The clock the interval is read on.
        stopwatch: &'a dyn Stopwatch,
    },
}

impl std::fmt::Debug for Publication<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AtTheSeal => formatter.write_str("AtTheSeal"),
            Self::AsItGoes { interval, .. } => formatter
                .debug_struct("AsItGoes")
                .field("interval", interval)
                .finish_non_exhaustive(),
        }
    }
}

/// Why a run stopped publishing; it goes on, and its revision appears at the
/// seal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishingStopped {
    /// A finished chunk's checkpoint could not be stored: a resumed run might
    /// hear that chunk differently than what would be published.
    CheckpointMissing,
    /// What the run settles now differs from what was published before (a
    /// resume with other merge rules); the run seals a stopped revision.
    Diverged,
    /// The session's newest revision is neither the run's base nor its own
    /// open revision.
    NotOurs,
    /// The revision has as many parts as it may.
    TooManyParts,
    /// A finished chunk's audio began earlier before its window than
    /// [`vsift_domain::MAX_EARLY_AUDIO_MICROS`], which what is settled relies
    /// on for the chunks still to come.
    AudioTooEarly,
    /// A part could not be built or committed for a reason a retry does not
    /// help.
    Failed,
}

/// The operation id part `part` of attempt `attempt` of `job`'s epoch `epoch`
/// is committed under: deterministic, like the seal's.
///
/// # Errors
///
/// [`JobKeyError::NotCanonical`] only through an internal fault.
pub fn part_operation_id(
    job: &JobId,
    epoch: u32,
    attempt: u32,
    part: NonZeroU32,
) -> Result<OperationId, JobKeyError> {
    OperationId::parse(derived_identity(
        "op_",
        "vsift.job-part.v1",
        &[
            job.as_str(),
            &epoch.to_string(),
            &attempt.to_string(),
            &part.to_string(),
        ],
    ))
    .map_err(|_| JobKeyError::NotCanonical)
}

/// The operation id the stopped revision of attempt `attempt` of `job`'s epoch
/// `epoch` is sealed under.
///
/// # Errors
///
/// [`JobKeyError::NotCanonical`] only through an internal fault.
pub fn stop_operation_id(
    job: &JobId,
    epoch: u32,
    attempt: u32,
) -> Result<OperationId, JobKeyError> {
    OperationId::parse(derived_identity(
        "op_",
        "vsift.job-stop.v1",
        &[job.as_str(), &epoch.to_string(), &attempt.to_string()],
    ))
    .map_err(|_| JobKeyError::NotCanonical)
}

/// What a [`Publisher`] knows of the parts so far.
struct State {
    /// The open revision as its last part published it, and how many parts.
    published: Option<(TranscriptRevision, NonZeroU32)>,
    /// When the last part was committed, on the stopwatch.
    last_at: Option<Duration>,
    /// Finished chunks when the last part was committed.
    chunks_at_last: usize,
    /// Why publishing stopped, once it has.
    stopped: Option<PublishingStopped>,
}

/// Publishes a run's settled part after its chunks (ADR 0033).
pub(super) struct Publisher<'a, S> {
    run: &'a RetranscriptionRun<'a>,
    store: &'a S,
    publication: Publication<'a>,
    epoch: u32,
    attempt: u32,
    state: Mutex<State>,
}

impl<'a, S: RevisionStore> Publisher<'a, S> {
    /// A publisher for one attempt, which continues the parts the job's earlier
    /// attempts published.
    pub(super) fn new(
        run: &'a RetranscriptionRun<'a>,
        store: &'a S,
        publication: Publication<'a>,
        epoch: u32,
        attempt: u32,
    ) -> Self {
        let mut stopped = None;
        let published = match publication {
            Publication::AtTheSeal => None,
            Publication::AsItGoes { .. } => {
                match store.open_revision(&run.spec.session_id, run.now) {
                    Ok(Some(open)) if open.job_id != run.spec.job_id => {
                        stopped = Some(PublishingStopped::NotOurs);
                        None
                    }
                    Ok(Some(open)) => {
                        if open.merge_rules != MERGE_RULES_VERSION {
                            stopped = Some(PublishingStopped::Diverged);
                        }
                        Some((open.revision, open.parts))
                    }
                    Ok(None) => None,
                    Err(_) => {
                        stopped = Some(PublishingStopped::Failed);
                        None
                    }
                }
            }
        };
        Self {
            run,
            store,
            publication,
            epoch,
            attempt,
            state: Mutex::new(State {
                published,
                last_at: None,
                chunks_at_last: 0,
                stopped,
            }),
        }
    }

    /// The open revision published so far (by this attempt or an earlier one),
    /// and why publishing stopped, if it did.
    pub(super) fn outcome(&self) -> (Option<TranscriptRevision>, Option<PublishingStopped>) {
        self.state
            .lock()
            .map_or((None, Some(PublishingStopped::Failed)), |state| {
                (
                    state
                        .published
                        .as_ref()
                        .map(|(revision, _)| revision.clone()),
                    state.stopped,
                )
            })
    }

    /// The request the run's revisions are built from.
    fn request(&self, transcription: crate::AsrTranscription) -> Option<AsrRevisionRequest<'_>> {
        let number = self
            .run
            .base
            .map_or(Some(1), |base| base.number().checked_add(1))
            .and_then(NonZeroU32::new)?;
        Some(AsrRevisionRequest {
            session_id: &self.run.spec.session_id,
            source_id: self.run.source_id,
            source_segment: self.run.transcribe.source_segment,
            number,
            transcription,
            splice: self.run.base.map(|base| RevisionSplice {
                base,
                replaced_range: self.run.transcribe.range,
            }),
        })
    }

    /// Builds and commits the next part, if it is due; see the module.
    fn publish(&self, so_far: &RunSoFar<'_>, state: &mut State) -> Result<(), PublishingStopped> {
        let Publication::AsItGoes {
            interval,
            stopwatch,
        } = self.publication
        else {
            return Ok(());
        };
        if !so_far.all_checkpointed() {
            return Err(PublishingStopped::CheckpointMissing);
        }
        if !so_far.audio_began_in_time(MAX_EARLY_AUDIO_MICROS) {
            return Err(PublishingStopped::AudioTooEarly);
        }
        let now = stopwatch.elapsed();
        let per_part = so_far.planned().div_ceil(MAX_TRANSCRIPT_PARTS).max(1);
        if let Some(last) = state.last_at
            && (now.saturating_sub(last) < interval
                || so_far.finished().saturating_sub(state.chunks_at_last) < per_part)
        {
            return Ok(());
        }
        let (transcription, settled_until) = so_far
            .open(MAX_EARLY_AUDIO_MICROS)
            .map_err(|_| PublishingStopped::Failed)?;
        let request = self
            .request(transcription)
            .ok_or(PublishingStopped::Failed)?;
        let open = build_open_asr_revision(request, settled_until)
            .map_err(|_: TranscriptBuildError| PublishingStopped::Failed)?;
        let until = open.published_until();
        let (count, published_until, parts) = match &state.published {
            Some((published, parts)) => (
                published.segments().len(),
                published.published_until(),
                parts.get(),
            ),
            None => (0, None, 0),
        };
        if open.segments().len() < count || until < published_until {
            // A resumed run is catching up with what was published.
            return Ok(());
        }
        if let Some((published, _)) = &state.published
            && (open.id() != published.id()
                || open.segments().get(..count) != Some(published.segments()))
        {
            return Err(PublishingStopped::Diverged);
        }
        if open.segments().len() == count && until == published_until {
            return Ok(());
        }
        let part = parts
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .ok_or(PublishingStopped::TooManyParts)?;
        if usize::try_from(part.get()).map_or(true, |part| part > MAX_TRANSCRIPT_PARTS) {
            return Err(PublishingStopped::TooManyParts);
        }
        let session = &self.run.spec.session_id;
        let head = self
            .store
            .head(session, self.run.now)
            .map_err(|_| PublishingStopped::Failed)?;
        let newest = head.newest.as_ref().map(TranscriptRevision::id);
        if newest != self.run.base.map(TranscriptRevision::id) && newest != Some(open.id()) {
            return Err(PublishingStopped::NotOurs);
        }
        let operation = part_operation_id(&self.run.spec.job_id, self.epoch, self.attempt, part)
            .map_err(|_| PublishingStopped::Failed)?;
        match self.store.publish_part(
            session,
            &operation,
            head.generation,
            &open,
            &self.run.spec.job_id,
            part,
            count,
            self.run.now,
        ) {
            Ok(_) => {
                state.published = Some((open, part));
                state.last_at = Some(now);
                state.chunks_at_last = so_far.finished();
                Ok(())
            }
            // Another command moved the generation, or the writer was busy:
            // the next chunk tries again.
            Err(SessionStorageError::Busy | SessionStorageError::StateConflict) => Ok(()),
            Err(SessionStorageError::CapacityExhausted) => Err(PublishingStopped::TooManyParts),
            Err(_) => Err(PublishingStopped::Failed),
        }
    }
}

impl<S: RevisionStore> RunObserver for Publisher<'_, S> {
    fn chunk_finished(&self, run: &RunSoFar<'_>) {
        // The last chunk is published by the seal.
        if run.next().is_none() {
            return;
        }
        // A poisoned lock only loses advisory publication.
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.stopped.is_some() {
            return;
        }
        if let Err(stopped) = self.publish(run, &mut state) {
            state.stopped = Some(stopped);
        }
    }
}

/// Seals `job`'s open revision as stopped at what it published (ADR 0033, D1),
/// if the session holds one, under the stop operation of the job's attempt,
/// waiting a little between tries while another command holds the session.
/// Returns the stopped revision, or `None` when the job published nothing.
///
/// # Errors
///
/// The storage failure that left the revision open; a later cancellation or
/// resume tries again.
pub async fn seal_stopped<S: RevisionStore, T: RetryTimer>(
    store: &S,
    timer: &T,
    session: &vsift_domain::SessionId,
    job: &JobId,
    epoch: u32,
    attempt: u32,
    now: u64,
) -> Result<Option<TranscriptRevision>, SessionStorageError> {
    let Some((stopped, operation)) = stopped_of(store, session, job, epoch, attempt, now)? else {
        return Ok(None);
    };
    let mut last = SessionStorageError::Busy;
    for wait in [0_u64, 50, 200, 800] {
        if wait > 0 {
            timer.sleep(Duration::from_millis(wait)).await;
        }
        match try_seal(store, session, &operation, &stopped, now) {
            Ok(true) => return Ok(Some(stopped)),
            Ok(false) => last = SessionStorageError::Busy,
            Err(error) => return Err(error),
        }
    }
    Err(last)
}

/// [`seal_stopped`] without waiting between its tries, for a caller that is
/// not on an asynchronous task (`job cancel` of a job no process runs).
///
/// # Errors
///
/// As [`seal_stopped`].
pub fn seal_stopped_now<S: RevisionStore>(
    store: &S,
    session: &vsift_domain::SessionId,
    job: &JobId,
    epoch: u32,
    attempt: u32,
    now: u64,
) -> Result<Option<TranscriptRevision>, SessionStorageError> {
    let Some((stopped, operation)) = stopped_of(store, session, job, epoch, attempt, now)? else {
        return Ok(None);
    };
    for _ in 0..4 {
        if try_seal(store, session, &operation, &stopped, now)? {
            return Ok(Some(stopped));
        }
    }
    Err(SessionStorageError::Busy)
}

/// The stopped revision of `job`'s open revision and the operation it is
/// sealed under, or `None` when the session holds no open revision of `job`.
fn stopped_of<S: RevisionStore>(
    store: &S,
    session: &vsift_domain::SessionId,
    job: &JobId,
    epoch: u32,
    attempt: u32,
    now: u64,
) -> Result<Option<(TranscriptRevision, OperationId)>, SessionStorageError> {
    let Some(open) = store.open_revision(session, now)? else {
        return Ok(None);
    };
    if open.job_id != *job {
        return Ok(None);
    }
    let base = match open.revision.supersedes() {
        Some(id) => Some(
            store
                .revision(session, id, now)?
                .ok_or(SessionStorageError::IntegrityFailure)?,
        ),
        None => None,
    };
    let stopped = build_stopped_asr_revision(&open.revision, base.as_ref())
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let operation = stop_operation_id(job, epoch, attempt)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    Ok(Some((stopped, operation)))
}

/// One try at the stop seal: `true` once the stopped revision is the
/// session's newest, `false` when another command moved the session or held
/// its writer, so the caller tries again.
fn try_seal<S: RevisionStore>(
    store: &S,
    session: &vsift_domain::SessionId,
    operation: &OperationId,
    stopped: &TranscriptRevision,
    now: u64,
) -> Result<bool, SessionStorageError> {
    let head = store.head(session, now)?;
    // A seal that landed before an error was reported is in the head.
    if head.newest.as_ref().is_some_and(|newest| {
        newest.id() == stopped.id() && newest.state() == vsift_domain::RevisionState::Stopped
    }) {
        return Ok(true);
    }
    match store.publish_revision(session, operation, head.generation, stopped, now) {
        Ok(_) => Ok(true),
        Err(SessionStorageError::Busy | SessionStorageError::StateConflict) => Ok(false),
        Err(error) => Err(error),
    }
}
