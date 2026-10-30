//! Clocks on the screen: `clocks` (ADR 0029, stage 2).
//!
//! [`Engine::read_clocks`] finds the clocks in what `ocr` read, narrows them by
//! reading the frames around each rollover, and records what it found with the
//! session. It never runs `ocr`'s plan: with no index it says so. The clocks
//! themselves are derived again each time from the newest screen-text index, so
//! the search keeps no state: the frames it reads are ordinary frames of that
//! index, and a call that stops loses nothing.
//!
//! Only what a person states changes a clock time: `--screen-zone` and
//! `--accept-gap`. Everything else here reports (ADR 0029 decisions 13 and 16).

use std::{num::NonZeroU16, path::PathBuf};

use vsift_application::{
    DEFAULT_SCREEN_TEXT_FRAMES_PER_CALL, MAX_SCREEN_TEXT_FRAMES_PER_CALL, ScreenTextStop,
    SessionStorageError,
};
use vsift_domain::{
    ClockAnalysis, ClockConflict, ClockId, ClockRegister, ClockStatementError, GapId,
    MAX_ROLLOVER_FRAMES, MediaTime, OffsetObservation, ReadTarget, ScreenTextCoverage,
    ScreenTextIndex, SessionId, TimeAnchors, TimeRange, UtcOffset, analyse_clocks, rollover_frames,
};
use vsift_infrastructure::{FilesystemSessionStore, OcrRunner, SessionStatus};

use crate::{
    asr::{admission_fits, open_status},
    engine::Engine,
    error::{EngineError, SessionRootError},
    screen_text::{DEFAULT_OCR_THREADS, OcrError, ReadCall, analysed_plan},
    sessions::SessionSnapshot,
    verification::Cancellation,
};

/// Frames one `clocks` call reads unless the caller chooses.
pub const DEFAULT_CLOCK_FRAMES: u16 = DEFAULT_SCREEN_TEXT_FRAMES_PER_CALL;
/// How often a write that lost a race with another writer is tried again.
const MAX_COMMIT_ATTEMPTS: usize = 3;
/// The longest video: every plan of the whole video fits inside it.
const MAX_PLANNED_MICROS: u64 = 4 * 60 * 60 * 1_000_000;

/// A bounded request to find a session's clocks.
#[derive(Clone, Debug)]
pub struct ClocksRequest {
    /// The session.
    pub session: SessionId,
    /// Most frames this call reads, 0 to 200; `None` for 40. With 0 it reads
    /// nothing and needs no runner, so a statement never needs one.
    pub max_frames: Option<u16>,
    /// The runner executable for this call only; `None` resolves it.
    pub runner: Option<PathBuf>,
    /// Inference threads, as for `ocr`.
    pub threads: Option<NonZeroU16>,
    /// Clocks whose zone the user states.
    pub screen_zones: Vec<(ClockId, UtcOffset)>,
    /// Gaps the user accepts.
    pub accepted_gaps: Vec<GapId>,
    /// Stops the call at its next step; what was read before is committed.
    pub cancellation: Cancellation,
}

/// How far a session's clocks are known.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClocksState {
    /// `ocr` has not read the session.
    NotRun,
    /// `ocr` has frames still to read, or the search has reads still to make.
    Partial,
    /// Every planned frame is read and no search has a read left.
    Complete,
}

impl ClocksState {
    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::NotRun => "not_run",
            Self::Partial => "partial",
            Self::Complete => "complete",
        }
    }
}

/// What a `clocks` call found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClocksResults {
    session: SessionSnapshot,
    index: Option<ScreenTextIndex>,
    analysis: ClockAnalysis,
    register: ClockRegister,
    observations: Vec<OffsetObservation>,
    conflicts: Vec<ClockConflict>,
    read_now: usize,
    stop: Option<ScreenTextStop>,
    reads_pending: usize,
    frames_pending: bool,
    state: ClocksState,
}

impl ClocksResults {
    /// Session state observed after the call.
    #[must_use]
    pub const fn session(&self) -> &SessionSnapshot {
        &self.session
    }

    /// The newest screen-text index, if `ocr` has read the session.
    #[must_use]
    pub const fn index(&self) -> Option<&ScreenTextIndex> {
        self.index.as_ref()
    }

    /// What the clocks do.
    #[must_use]
    pub const fn analysis(&self) -> &ClockAnalysis {
        &self.analysis
    }

    /// The clocks and gaps the session has named.
    #[must_use]
    pub const fn register(&self) -> &ClockRegister {
        &self.register
    }

    /// What each clock of unknown zone appears to be: inferences, never applied.
    #[must_use]
    pub fn observations(&self) -> &[OffsetObservation] {
        &self.observations
    }

    /// What disagrees, unresolved.
    #[must_use]
    pub fn conflicts(&self) -> &[ClockConflict] {
        &self.conflicts
    }

    /// Frames this call read.
    #[must_use]
    pub const fn read_now(&self) -> usize {
        self.read_now
    }

    /// Why this call stopped before its search was done, if it did.
    #[must_use]
    pub const fn stop(&self) -> Option<ScreenTextStop> {
        self.stop
    }

    /// How many reads the search would make next.
    #[must_use]
    pub const fn reads_pending(&self) -> usize {
        self.reads_pending
    }

    /// Whether `ocr` has planned frames still to read.
    #[must_use]
    pub const fn frames_pending(&self) -> bool {
        self.frames_pending
    }

    /// How far the clocks are known.
    #[must_use]
    pub const fn state(&self) -> ClocksState {
        self.state
    }
}

/// The video as far as `index` has read it: all that a counter's rule needs,
/// and it costs no tool.
fn read_duration(index: &ScreenTextIndex) -> MediaTime {
    let last = index
        .frames()
        .last()
        .map_or(0, |frame| frame.frame.time.as_micros());
    MediaTime::from_micros(last.saturating_add(1))
}

/// The search's next reads over `index`, stopping at the session's bound.
fn next_targets(index: &ScreenTextIndex) -> Vec<ReadTarget> {
    let room = MAX_ROLLOVER_FRAMES.saturating_sub(rollover_frames(index));
    analyse_clocks(index, read_duration(index))
        .next_reads()
        .into_iter()
        .take(room)
        .map(|time| ReadTarget {
            time,
            candidate: None,
        })
        .collect()
}

/// Whether every planned frame of the whole video is read.
fn frames_pending(
    store: &FilesystemSessionStore,
    session: &SessionId,
    index: &ScreenTextIndex,
    now: u64,
) -> bool {
    let whole = TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(MAX_PLANNED_MICROS),
    );
    let Ok(whole) = whole else {
        return true;
    };
    analysed_plan(store, session, whole, index.period_micros(), now).map_or(
        true,
        |(plan, _, _, awaiting_analysis)| {
            !ScreenTextCoverage {
                awaiting_analysis,
                ..ScreenTextCoverage::of(&plan, Some(index))
            }
            .is_complete()
        },
    )
}

/// What a call answers before `ocr` has read the session.
fn clocks_not_run(status: &SessionStatus, now: u64) -> ClocksResults {
    let analysis = ClockAnalysis::default();
    let register = ClockRegister::of(
        status.screen_clocks(),
        status.time_anchors().gaps(),
        &analysis,
    );
    ClocksResults {
        session: SessionSnapshot::observe(status, now),
        index: None,
        analysis,
        register,
        observations: Vec::new(),
        conflicts: Vec::new(),
        read_now: 0,
        stop: None,
        reads_pending: 0,
        frames_pending: true,
        state: ClocksState::NotRun,
    }
}

impl Engine {
    /// Finds a session's clocks, and records what it found and what the request
    /// states (see the module documentation).
    ///
    /// # Errors
    ///
    /// Fails for an invalid budget or thread count; a missing root or session,
    /// or a closed or expired one; a statement about a clock or gap the session
    /// does not have, or a zone that cannot apply ([`EngineError::Clocks`]); and
    /// whatever `ocr` fails for when the call reads frames.
    pub async fn read_clocks(&self, request: ClocksRequest) -> Result<ClocksResults, EngineError> {
        let session = request.session.clone();
        self.read_clocks_unnamed(request)
            .await
            .map_err(|error| self.name_running_transcription(&session, error))
    }

    async fn read_clocks_unnamed(
        &self,
        request: ClocksRequest,
    ) -> Result<ClocksResults, EngineError> {
        let max_frames = request.max_frames.unwrap_or(DEFAULT_CLOCK_FRAMES);
        if max_frames > MAX_SCREEN_TEXT_FRAMES_PER_CALL {
            return Err(EngineError::Ocr(OcrError::InvalidBudget));
        }
        let (store, now) = self.existing_store()?;
        let store = store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        open_status(&store, &request.session, now)?;
        let (previous, status) =
            store.read_screen_text_index(&request.session, self.now_unix_seconds()?)?;
        let Some(held) = previous.as_ref() else {
            if let Some(statement) = request.statement() {
                return Err(statement);
            }
            return Ok(clocks_not_run(&status, now));
        };
        let wants_reads = max_frames > 0 && !next_targets(held).is_empty();
        let (index, read_now, stop) = if wants_reads {
            let threads = request
                .threads
                .unwrap_or(NonZeroU16::new(DEFAULT_OCR_THREADS).unwrap_or(NonZeroU16::MIN));
            admission_fits(&store, threads)?;
            let runner = match self.host_screen_text_reader() {
                Some(_) => None,
                None => Some(OcrRunner::new(
                    self.resolve_ocr_runner(request.runner.clone())?,
                    threads,
                    self.config().host_isolation.into_infrastructure(),
                )),
            };
            let period = held.period_micros();
            let done = self
                .read_in_rounds(
                    ReadCall {
                        store: &store,
                        session: &request.session,
                        previous,
                        status,
                        runner,
                        max_frames,
                        period,
                        cancellation: request.cancellation.0.clone(),
                        now,
                    },
                    |index| index.map_or_else(Vec::new, next_targets),
                )
                .await?;
            (done.index, done.read_now, done.extension.stop)
        } else {
            (previous, 0, None)
        };
        let index = index.ok_or(EngineError::Storage(SessionStorageError::IntegrityFailure))?;
        let analysis = analyse_clocks(&index, read_duration(&index));
        let (register, anchors) = self.record_clocks(&store, &request, &analysis)?;
        let now = self.now_unix_seconds()?;
        let status = crate::sessions::published_status(&store, &request.session)?;
        let reads_pending = next_targets(&index).len();
        let frames_pending = frames_pending(&store, &request.session, &index, now);
        let state = if reads_pending == 0 && !frames_pending {
            ClocksState::Complete
        } else {
            ClocksState::Partial
        };
        Ok(ClocksResults {
            session: SessionSnapshot::observe(&status, now),
            observations: register.observations(&analysis, &anchors),
            conflicts: register.conflicts(&analysis),
            index: Some(index),
            analysis,
            register,
            read_now,
            stop,
            reads_pending,
            frames_pending,
            state,
        })
    }

    /// Records the clocks, the gaps and what the request states, on a fresh
    /// status each try, so nothing a concurrent writer did is lost. Nothing is
    /// written when nothing changed.
    fn record_clocks(
        &self,
        store: &FilesystemSessionStore,
        request: &ClocksRequest,
        analysis: &ClockAnalysis,
    ) -> Result<(ClockRegister, TimeAnchors), EngineError> {
        for _ in 0..MAX_COMMIT_ATTEMPTS {
            let status = crate::sessions::published_status(store, &request.session)?;
            let held = status.time_anchors();
            let mut register = ClockRegister::of(status.screen_clocks(), held.gaps(), analysis);
            for (clock, offset) in &request.screen_zones {
                register
                    .confirm_zone(*clock, *offset, analysis, held)
                    .map_err(EngineError::Clocks)?;
            }
            for gap in &request.accepted_gaps {
                register.accept_gap(*gap).map_err(EngineError::Clocks)?;
            }
            let anchors = held
                .with_screen_anchors(register.screen_anchors(analysis, held))
                .with_gaps(register.gaps().to_vec());
            if anchors == *held && register.entries() == status.screen_clocks() {
                return Ok((register, anchors));
            }
            match store.publish_clocks(
                &request.session,
                &self.new_operation_id()?,
                status.generation(),
                &anchors,
                register.entries(),
                self.now_unix_seconds()?,
            ) {
                Ok(_) => return Ok((register, anchors)),
                Err(SessionStorageError::StateConflict) => {}
                Err(error) => return Err(EngineError::Storage(error)),
            }
        }
        Err(EngineError::Storage(SessionStorageError::Busy))
    }
}

impl ClocksRequest {
    /// The refusal of a statement about a session with nothing to state it of.
    fn statement(&self) -> Option<EngineError> {
        if !self.screen_zones.is_empty() {
            return Some(EngineError::Clocks(ClockStatementError::UnknownClock));
        }
        if !self.accepted_gaps.is_empty() {
            return Some(EngineError::Clocks(ClockStatementError::UnknownGap));
        }
        None
    }
}
