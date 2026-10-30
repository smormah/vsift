//! The clocks a session has named (ADR 0029 decision 18).
//!
//! Clocks are derived again from the screen-text index each time, so their
//! order could change when a later `ocr` reads earlier frames, and a user's
//! confirmation names a clock. So a session keeps a small register: each clock
//! it has seen, by its class and its offset. A recomputed clock takes the id of
//! the entry it meets, and a new one takes the next id. Nothing about a clock's
//! time is stored here that the index does not give again, except what a person
//! said about it.

use crate::{
    AnchorSource, ClockAnalysis, ClockFormat, ClockId, ClockKind, ClockUnit, MAX_SCREEN_CLOCKS,
    OffsetInterval, ScreenAnchor, ScreenClock, TimeAnchor, UtcOffset,
};

const DAY_MICROS: i64 = 86_400_000_000;

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
    ids: Vec<Option<ClockId>>,
}

impl ClockRegister {
    /// Names the clocks of `analysis`, keeping every entry of `previous`. A
    /// clock past [`MAX_SCREEN_CLOCKS`] takes no id: the report still lists it.
    #[must_use]
    pub fn of(previous: &[RegisteredClock], analysis: &ClockAnalysis) -> Self {
        let mut entries = previous.to_vec();
        let mut ids = Vec::with_capacity(analysis.clocks.len());
        for clock in &analysis.clocks {
            let held = entries.iter_mut().find(|entry| entry.names(clock));
            let id = match (held, clock.tightest_offset()) {
                (Some(entry), Some(offset)) => {
                    entry.offset = offset;
                    Some(entry.id)
                }
                (None, Some(offset)) => next_id(&entries).map(|id| {
                    entries.push(RegisteredClock {
                        id,
                        format: clock.format(),
                        unit: clock.unit(),
                        offset,
                        confirmed_offset: None,
                    });
                    id
                }),
                (_, None) => None,
            };
            ids.push(id);
        }
        Self { entries, ids }
    }

    /// Every clock the session has named.
    #[must_use]
    pub fn entries(&self) -> &[RegisteredClock] {
        &self.entries
    }

    /// The id of the analysis's clock at `index`, if it took one.
    #[must_use]
    pub fn id_of(&self, index: usize) -> Option<ClockId> {
        self.ids.get(index).copied().flatten()
    }

    /// The anchors the analysis's clocks give: a dated clock's reading, under
    /// the source its zone allows. A clock of no date gives none, and a counter
    /// none (ADR 0029 decisions 10 and 15).
    #[must_use]
    pub fn screen_anchors(&self, analysis: &ClockAnalysis) -> Vec<TimeAnchor> {
        analysis
            .clocks
            .iter()
            .enumerate()
            .filter(|(_, clock)| clock.kind() == ClockKind::Clock)
            .filter_map(|(index, clock)| {
                let id = self.id_of(index)?;
                let (time, reading, offset) = clock.anchor()?;
                let source = match clock.format() {
                    ClockFormat::DatedWithZone => AnchorSource::ScreenTimestamp,
                    ClockFormat::Dated => AnchorSource::ScreenClock,
                    ClockFormat::TwentyFourHour | ClockFormat::TwelveHour => return None,
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

#[cfg(test)]
mod tests;
