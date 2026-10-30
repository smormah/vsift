//! The clocks a session has named (ADR 0029 decision 18).
//!
//! Clocks are derived again from the screen-text index each time, so their
//! order could change when a later `ocr` reads earlier frames, and a user's
//! confirmation names a clock. So a session keeps a small register: each clock
//! it has seen, by its class and its offset. A recomputed clock takes the id of
//! the entry it meets, and a new one takes the next id. Nothing about a clock's
//! time is stored here that the index does not give again, except what a person
//! said about it.

use std::{error::Error, fmt};

use crate::{
    AnchorSource, CivilDateTime, ClockAnalysis, ClockFormat, ClockGap, ClockId, ClockJump,
    ClockJumpKind, ClockKind, ClockReading, ClockUnit, GapId, GapState, MAX_CLOCK_GAPS,
    MAX_SCREEN_CLOCKS, MediaTime, OffsetInterval, ScreenAnchor, ScreenClock, TimeAnchor,
    TimeAnchors, UtcOffset,
};

const DAY_MICROS: i64 = 86_400_000_000;
const QUARTER_HOUR_MICROS: i64 = 15 * 60 * 1_000_000;
/// How far apart a gap may move and stay one gap, as reads narrow it.
const SAME_GAP_MICROS: u64 = 60_000_000;
/// Latitude of an offset observation, beyond the clock's own precision.
const OBSERVATION_LATITUDE_MICROS: i64 = 30_000_000;

/// One clock a session has named.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RegisteredClock {
    /// Its id.
    pub id: ClockId,
    /// Its format class.
    pub format: ClockFormat,
    /// What its faces show.
    pub unit: ClockUnit,
    /// Its offset, as last measured.
    pub offset: OffsetInterval,
    /// The zone the user confirmed for it, if they did.
    pub confirmed_offset: Option<UtcOffset>,
}

impl RegisteredClock {
    fn names(&self, clock: &ScreenClock) -> bool {
        self.format == clock.format()
            && self.unit == clock.unit()
            && clock
                .tightest_offset()
                .is_some_and(|offset| meets_modulo_a_day(self.offset, offset))
    }
}

/// A time of day has no date, so one clock can come out a whole day apart.
fn meets_modulo_a_day(left: OffsetInterval, right: OffsetInterval) -> bool {
    [-DAY_MICROS, 0, DAY_MICROS].iter().any(|days| {
        OffsetInterval::new(right.low() + days, right.high() + days)
            .is_some_and(|moved| left.meet(moved).is_some())
    })
}

/// The register after an analysis, and which id each of its clocks took.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClockRegister {
    entries: Vec<RegisteredClock>,
    gaps: Vec<ClockGap>,
    ids: Vec<Option<ClockId>>,
}

/// Why a statement about a clock or a gap was refused.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClockStatementError {
    /// The session has no clock of this id.
    UnknownClock,
    /// The clock is an elapsed timer, which has no zone.
    Counter,
    /// The clock has no date, and no absolute anchor gives it one.
    NoDate,
    /// The session has no gap of this id.
    UnknownGap,
}

impl fmt::Display for ClockStatementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnknownClock => "the session has no such clock",
            Self::Counter => "a counter has no zone",
            Self::NoDate => {
                "the clock has no date, and no anchor that names an instant gives it one"
            }
            Self::UnknownGap => "the session has no such gap",
        })
    }
}

impl Error for ClockStatementError {}

impl ClockRegister {
    /// Names the clocks and gaps of `analysis`, keeping every clock of
    /// `previous` and every gap a person accepted. A clock past
    /// [`MAX_SCREEN_CLOCKS`] takes no id: the report still lists it.
    #[must_use]
    pub fn of(
        previous: &[RegisteredClock],
        previous_gaps: &[ClockGap],
        analysis: &ClockAnalysis,
    ) -> Self {
        let mut entries = previous.to_vec();
        let mut ids = Vec::with_capacity(analysis.clocks.len());
        for clock in &analysis.clocks {
            let held = entries.iter_mut().find(|entry| entry.names(clock));
            let id = match (held, clock.tightest_offset()) {
                (Some(entry), Some(offset)) => {
                    entry.offset = offset;
                    Some(entry.id)
                }
                (None, Some(offset)) => next_id(&entries).inspect(|id| {
                    entries.push(RegisteredClock {
                        id: *id,
                        format: clock.format(),
                        unit: clock.unit(),
                        offset,
                        confirmed_offset: None,
                    });
                }),
                (_, None) => None,
            };
            ids.push(id);
        }
        let mut register = Self {
            entries,
            gaps: Vec::new(),
            ids,
        };
        register.gaps = register.named_gaps(previous_gaps, analysis);
        register
    }

    fn named_gaps(&self, previous: &[ClockGap], analysis: &ClockAnalysis) -> Vec<ClockGap> {
        let mut gaps: Vec<ClockGap> = previous
            .iter()
            .filter(|gap| gap.state == GapState::Accepted)
            .copied()
            .collect();
        for (index, clock) in analysis.clocks.iter().enumerate() {
            if clock.kind() != ClockKind::Clock {
                continue;
            }
            let Some(id) = self.id_of(index) else {
                continue;
            };
            for jump in clock.jumps() {
                if jump.kind != ClockJumpKind::Gap {
                    continue;
                }
                let found = |gap: &ClockGap| same_gap(gap, jump);
                if let Some(held) = gaps.iter_mut().find(|gap| found(gap)) {
                    // Two clocks showing one gap are one gap; an accepted gap
                    // keeps what the person accepted, narrowed as reads narrow it.
                    if held.clock == id {
                        held.at = jump.after.time;
                        held.jump_low_micros = jump.jump.low();
                        held.jump_high_micros = jump.jump.high();
                    }
                    continue;
                }
                let reused = previous.iter().find(|gap| found(gap)).map(|gap| gap.id);
                let Some(gap_id) = reused.or_else(|| next_gap_id(&gaps, previous)) else {
                    continue;
                };
                gaps.push(ClockGap {
                    id: gap_id,
                    clock: id,
                    at: jump.after.time,
                    jump_low_micros: jump.jump.low(),
                    jump_high_micros: jump.jump.high(),
                    state: GapState::Detected,
                });
            }
        }
        gaps.sort_by_key(|gap| (gap.at, gap.id));
        gaps
    }

    /// Every clock the session has named.
    #[must_use]
    pub fn entries(&self) -> &[RegisteredClock] {
        &self.entries
    }

    /// Every gap, in timeline order.
    #[must_use]
    pub fn gaps(&self) -> &[ClockGap] {
        &self.gaps
    }

    /// The id of the analysis's clock at `index`, if it took one.
    #[must_use]
    pub fn id_of(&self, index: usize) -> Option<ClockId> {
        self.ids.get(index).copied().flatten()
    }

    /// Accepts a gap: what a person said, never what an observation suggests.
    ///
    /// # Errors
    ///
    /// [`ClockStatementError::UnknownGap`].
    pub fn accept_gap(&mut self, id: GapId) -> Result<(), ClockStatementError> {
        let gap = self
            .gaps
            .iter_mut()
            .find(|gap| gap.id == id)
            .ok_or(ClockStatementError::UnknownGap)?;
        gap.state = GapState::Accepted;
        Ok(())
    }

    /// Confirms a clock's zone: what a person said, never an observation.
    ///
    /// # Errors
    ///
    /// [`ClockStatementError::UnknownClock`], [`ClockStatementError::Counter`],
    /// or [`ClockStatementError::NoDate`] for a clock of no date when `anchors`
    /// holds no anchor that names an instant.
    pub fn confirm_zone(
        &mut self,
        id: ClockId,
        offset: UtcOffset,
        analysis: &ClockAnalysis,
        anchors: &TimeAnchors,
    ) -> Result<(), ClockStatementError> {
        let index = self
            .ids
            .iter()
            .position(|held| *held == Some(id))
            .ok_or(ClockStatementError::UnknownClock)?;
        let clock = analysis
            .clocks
            .get(index)
            .ok_or(ClockStatementError::UnknownClock)?;
        if clock.kind() == ClockKind::Counter {
            return Err(ClockStatementError::Counter);
        }
        if clock.first_read().is_some_and(|read| read.face.wraps())
            && absolute_anchor(anchors).is_none()
        {
            return Err(ClockStatementError::NoDate);
        }
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(ClockStatementError::UnknownClock)?;
        entry.confirmed_offset = Some(offset);
        Ok(())
    }

    /// The anchors the analysis's clocks give (ADR 0029 decisions 15 and 16). A
    /// confirmed clock gives an instant. An unconfirmed one gives what its face
    /// says, and one of no date gives none, because a date is never borrowed
    /// silently. A counter gives none.
    #[must_use]
    pub fn screen_anchors(
        &self,
        analysis: &ClockAnalysis,
        anchors: &TimeAnchors,
    ) -> Vec<TimeAnchor> {
        analysis
            .clocks
            .iter()
            .enumerate()
            .filter(|(_, clock)| clock.kind() == ClockKind::Clock)
            .filter_map(|(index, clock)| {
                let id = self.id_of(index)?;
                let confirmed = self
                    .entries
                    .iter()
                    .find(|entry| entry.id == id)
                    .and_then(|entry| entry.confirmed_offset);
                let (time, reading, offset) = clock.position()?;
                let (source, reading) = match (confirmed, clock.format()) {
                    (Some(zone), _) => (
                        AnchorSource::ScreenClockConfirmed,
                        confirmed_reading(clock, reading, time, zone, anchors)?,
                    ),
                    (None, ClockFormat::DatedWithZone) => (AnchorSource::ScreenTimestamp, reading?),
                    (None, ClockFormat::Dated) => (AnchorSource::ScreenClock, reading?),
                    (None, ClockFormat::TwentyFourHour | ClockFormat::TwelveHour) => return None,
                };
                Some(TimeAnchor::on_screen(
                    source,
                    time,
                    reading,
                    ScreenAnchor {
                        clock: id,
                        precision_micros: offset.width().unsigned_abs(),
                    },
                ))
            })
            .collect()
    }

    /// What each clock of unknown zone appears to be, against the anchor that
    /// names an instant (ADR 0029 decision 16). Always an inference, and never
    /// applied to a clock time.
    #[must_use]
    pub fn observations(
        &self,
        analysis: &ClockAnalysis,
        anchors: &TimeAnchors,
    ) -> Vec<OffsetObservation> {
        let Some(anchor) = absolute_anchor(anchors) else {
            return Vec::new();
        };
        analysis
            .clocks
            .iter()
            .enumerate()
            .filter(|(_, clock)| {
                clock.kind() == ClockKind::Clock && clock.format() != ClockFormat::DatedWithZone
            })
            .filter_map(|(index, clock)| {
                let id = self.id_of(index)?;
                let (time, _, offset) = clock.position()?;
                let shown = i64::try_from(time.as_micros()).ok()?;
                let instant = anchor.reading_at(time)?.utc_unix_micros()?;
                let wraps = clock.first_read()?.face.wraps();
                let difference = |value: i64| {
                    let apart = value + shown - instant;
                    if wraps {
                        centred_on_a_day(apart)
                    } else {
                        apart
                    }
                };
                let measured = difference(offset.middle());
                let width = offset.width();
                Some(OffsetObservation {
                    clock: id,
                    anchor: anchor.source(),
                    measured_micros: measured,
                    uncertainty_micros: width / 2,
                    fits: fitting_offsets(measured, width / 2, wraps),
                })
            })
            .collect()
    }

    /// What disagrees, unresolved (ADR 0029 decision 14).
    #[must_use]
    pub fn conflicts(&self, analysis: &ClockAnalysis) -> Vec<ClockConflict> {
        let mut conflicts = Vec::new();
        let flagged = |clock: &ScreenClock| clock.rate().filter(|rate| rate.differs);
        for (index, clock) in analysis.clocks.iter().enumerate() {
            if clock.kind() != ClockKind::Clock {
                continue;
            }
            for jump in clock.jumps() {
                if jump.kind == ClockJumpKind::Backward {
                    conflicts.push(ClockConflict::Backward {
                        clock: self.id_of(index),
                        at: jump.after.time,
                    });
                }
                for (other_index, other) in analysis.clocks.iter().enumerate() {
                    if other_index != index
                        && other.kind() == ClockKind::Clock
                        && other.spans_without_a_jump(jump.before.time, jump.after.time)
                    {
                        conflicts.push(ClockConflict::JumpNotShared {
                            clock: self.id_of(index),
                            other: self.id_of(other_index),
                            at: jump.after.time,
                        });
                    }
                }
            }
            for (other_index, other) in analysis.clocks.iter().enumerate().skip(index + 1) {
                if other.kind() != ClockKind::Clock || !clock.overlaps(other) {
                    continue;
                }
                let rates = (flagged(clock), flagged(other));
                if !matches!(rates, (None, None)) {
                    conflicts.push(ClockConflict::RatesDiffer {
                        clock: self.id_of(index),
                        other: self.id_of(other_index),
                    });
                }
            }
        }
        conflicts
    }
}

/// How a clock of unknown zone compares with an anchor that names an instant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OffsetObservation {
    /// The clock.
    pub clock: ClockId,
    /// The anchor it was measured against.
    pub anchor: AnchorSource,
    /// How far the clock runs from that anchor's UTC, in microseconds.
    pub measured_micros: i64,
    /// How far that can be from the truth.
    pub uncertainty_micros: i64,
    /// The UTC offsets it fits; empty when it fits none, which usually means
    /// the anchor is not the recording's start. A time of day fits modulo 24
    /// hours, so it can fit two.
    pub fits: Vec<UtcOffset>,
}

/// Something the clocks disagree about. It is reported and never resolved.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClockConflict {
    /// Two clocks over one stretch run at different rates.
    RatesDiffer {
        /// One clock.
        clock: Option<ClockId>,
        /// The other.
        other: Option<ClockId>,
    },
    /// One clock jumps where another spans the moment without jumping.
    JumpNotShared {
        /// The clock that jumped.
        clock: Option<ClockId>,
        /// The clock that did not.
        other: Option<ClockId>,
        /// Where.
        at: MediaTime,
    },
    /// A clock's offset fell: a stretch repeated or moved, or it was set back.
    Backward {
        /// The clock.
        clock: Option<ClockId>,
        /// Where.
        at: MediaTime,
    },
}

/// Whether a stored gap and a jump found now are the same gap: their lengths
/// overlap and they lie within [`SAME_GAP_MICROS`]. Reads narrow both.
fn same_gap(gap: &ClockGap, jump: &ClockJump) -> bool {
    let apart = gap.at.as_micros().abs_diff(jump.after.time.as_micros());
    let held = OffsetInterval::new(gap.jump_low_micros, gap.jump_high_micros);
    apart <= SAME_GAP_MICROS && held.is_some_and(|held| held.meet(jump.jump).is_some())
}

fn absolute_anchor(anchors: &TimeAnchors) -> Option<&TimeAnchor> {
    anchors.anchors().iter().find(|anchor| {
        !anchor.source().is_on_screen() && anchor.reading().utc_unix_micros().is_some()
    })
}

/// `apart` moved by whole days to lie within half a day of zero.
const fn centred_on_a_day(apart: i64) -> i64 {
    (apart + DAY_MICROS / 2).rem_euclid(DAY_MICROS) - DAY_MICROS / 2
}

/// The offsets a difference fits: the nearest multiple of 15 minutes within
/// its uncertainty plus [`OBSERVATION_LATITUDE_MICROS`], within ±14:00; a clock
/// with no date also a day either way.
fn fitting_offsets(measured: i64, uncertainty: i64, wraps: bool) -> Vec<UtcOffset> {
    let days: &[i64] = if wraps {
        &[-DAY_MICROS, 0, DAY_MICROS]
    } else {
        &[0]
    };
    days.iter()
        .filter_map(|day| {
            let shifted = measured + day;
            let quarters = (shifted + QUARTER_HOUR_MICROS / 2).div_euclid(QUARTER_HOUR_MICROS);
            let nearest = quarters * QUARTER_HOUR_MICROS;
            if (shifted - nearest).abs() > uncertainty + OBSERVATION_LATITUDE_MICROS {
                return None;
            }
            UtcOffset::from_seconds(i32::try_from(nearest / 1_000_000).ok()?).ok()
        })
        .collect()
}

/// The instant a confirmed clock names. A clock of no date takes the date on
/// which its reading lies nearest the absolute anchor, so any remaining
/// difference shows as a disagreement.
fn confirmed_reading(
    clock: &ScreenClock,
    reading: Option<ClockReading>,
    time: MediaTime,
    zone: UtcOffset,
    anchors: &TimeAnchors,
) -> Option<ClockReading> {
    if let Some(reading) = reading {
        return ClockReading::zoned(reading.local()?, zone).ok();
    }
    let instant = absolute_anchor(anchors)?
        .reading_at(time)?
        .utc_unix_micros()?;
    let (_, _, offset) = clock.position()?;
    let shown = i64::try_from(time.as_micros()).ok()?;
    let of_day = (offset.low() + shown).rem_euclid(DAY_MICROS);
    let local = instant + i64::from(zone.as_seconds()) * 1_000_000;
    let midnight = local.div_euclid(DAY_MICROS) * DAY_MICROS;
    let nearest = [-DAY_MICROS, 0, DAY_MICROS]
        .into_iter()
        .map(|day| midnight + day + of_day)
        .min_by_key(|value| value.abs_diff(local))?;
    ClockReading::zoned(CivilDateTime::from_unix_micros(nearest)?, zone).ok()
}

fn next_id(entries: &[RegisteredClock]) -> Option<ClockId> {
    if entries.len() >= MAX_SCREEN_CLOCKS {
        return None;
    }
    let highest = entries
        .iter()
        .map(|entry| entry.id.ordinal())
        .max()
        .unwrap_or(0);
    ClockId::new(highest + 1)
}

fn next_gap_id(gaps: &[ClockGap], previous: &[ClockGap]) -> Option<GapId> {
    if gaps.len() >= MAX_CLOCK_GAPS {
        return None;
    }
    let highest = gaps
        .iter()
        .chain(previous)
        .map(|gap| gap.id.ordinal())
        .max()
        .unwrap_or(0);
    GapId::new(highest + 1)
}

#[cfg(test)]
mod tests;
