//! How clocks on a screen move against the video (ADR 0029 decisions 10 to 13).
//!
//! This is step 2 of recognition, and the zone-free core of stage 2. Everything
//! rests on one number: a clock's **offset**, its reading minus the video's own
//! time at that frame. A clock that runs with the video keeps one offset. A
//! reading proves only an interval of offsets, because a minute face holds for
//! 60 s, so reads narrow it as they accumulate, and the width of the interval
//! is the clock's precision.
//!
//! Nothing here knows a zone. A clock's value is compared only with itself, so
//! two clocks in different zones are simply two clocks.

use crate::{
    ClockFace, ClockFaceRefusal, ClockFormat, ClockReading, ClockUnit, MIN_CLOCK_LINE_BASIS_POINTS,
    MediaTime, ScreenTextIndex, TextRegion, read_clock_face,
};

/// The version of the rules below: what a stored analysis was made by.
pub const CLOCK_RULES_VERSION: u32 = 1;
/// Fewest reads of a run that can make a clock.
pub const MIN_RUN_READS: usize = 3;
/// How late a screen may paint its clock, which widens every reading.
pub const PAINT_SLACK_MICROS: i64 = 250_000;
/// A jump in a clock's offset smaller than this is not reported.
pub const MIN_JUMP_MICROS: i64 = 2_000_000;
/// A jump at least this long, on a whole number of quarter hours, is another
/// clock in the same place and never a cut: two presenters' taskbars differ so.
pub const MIN_OTHER_CLOCK_JUMP_MICROS: i64 = 30 * 60 * 1_000_000;
/// The bracket a rollover search stops at: enough for the second.
pub const ROLLOVER_BRACKET_MICROS: u64 = 500_000;
/// Most frames a session's rollover searches read, across its calls.
pub const MAX_ROLLOVER_FRAMES: usize = 256;
/// Most places on the screen one analysis follows; more are only counted.
pub const MAX_CLOCK_PLACES: usize = 64;

const QUARTER_HOUR_MICROS: i64 = 15 * 60 * 1_000_000;
const DAY_MICROS: i64 = 86_400_000_000;

/// The half-open interval `[low, high)` of a clock's possible offsets, in
/// microseconds.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OffsetInterval {
    low: i64,
    high: i64,
}

impl OffsetInterval {
    /// The interval `[low, high)`; `None` if it holds nothing.
    #[must_use]
    pub const fn new(low: i64, high: i64) -> Option<Self> {
        if low < high {
            Some(Self { low, high })
        } else {
            None
        }
    }

    /// What one reading proves, widened for how late a screen paints.
    #[must_use]
    pub const fn of_reading(value_micros: i64, unit: ClockUnit, time: MediaTime) -> Self {
        // A media time is at most four hours, so it fits.
        let time = time.as_micros().cast_signed();
        Self {
            low: value_micros - time,
            high: value_micros + unit.as_micros() + PAINT_SLACK_MICROS - time,
        }
    }

    /// The lowest offset.
    #[must_use]
    pub const fn low(self) -> i64 {
        self.low
    }

    /// Just past the highest offset.
    #[must_use]
    pub const fn high(self) -> i64 {
        self.high
    }

    /// How far apart its ends are: the clock's precision.
    #[must_use]
    pub const fn width(self) -> i64 {
        self.high - self.low
    }

    /// Its middle.
    #[must_use]
    pub const fn middle(self) -> i64 {
        self.low + self.width() / 2
    }

    /// What the two have in common, if anything.
    #[must_use]
    pub fn meet(self, other: Self) -> Option<Self> {
        let met = Self {
            low: self.low.max(other.low),
            high: self.high.min(other.high),
        };
        (met.low < met.high).then_some(met)
    }

    /// Whether `offset` lies inside.
    #[must_use]
    pub const fn contains(self, offset: i64) -> bool {
        self.low <= offset && offset < self.high
    }
}

/// One clock face read at one moment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockRead {
    /// The frame's time.
    pub time: MediaTime,
    /// The frame's timestamp, which finds it again.
    pub pts: i64,
    /// Where on the frame the line lies.
    pub region: TextRegion,
    /// What it showed.
    pub face: ClockFace,
}

/// A read with the day the model decided it belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Tracked {
    read: ClockRead,
    /// The face's value, moved by whole days to continue its run.
    value: i64,
}

/// A maximal sequence of reads whose intervals share a point.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClockRun {
    reads: Vec<Tracked>,
    offset: OffsetInterval,
}

impl ClockRun {
    /// The reads, in time order.
    pub fn reads(&self) -> impl ExactSizeIterator<Item = &ClockRead> {
        self.reads.iter().map(|tracked| &tracked.read)
    }

    /// The offsets every read allows.
    #[must_use]
    pub const fn offset(&self) -> OffsetInterval {
        self.offset
    }

    fn advances(&self) -> bool {
        self.reads
            .first()
            .is_some_and(|first| self.reads.iter().any(|read| read.value != first.value))
    }

    fn qualifies(&self) -> bool {
        self.reads.len() >= MIN_RUN_READS && self.advances()
    }

    fn first_time(&self) -> Option<MediaTime> {
        self.reads.first().map(|tracked| tracked.read.time)
    }

    fn last_time(&self) -> Option<MediaTime> {
        self.reads.last().map(|tracked| tracked.read.time)
    }
}

/// Reads whose offset departed from a clock and returned to the same interval
/// (ADR 0029 decision 13). The cause is not known: it may be a misread, or
/// another clock's turn in the same place. Its reads leave the track, are never
/// lost, and change nothing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClockExcursion {
    reads: Vec<ClockRead>,
}

impl ClockExcursion {
    /// The reads, in time order.
    #[must_use]
    pub fn reads(&self) -> &[ClockRead] {
        &self.reads
    }

    /// When it began and ended.
    #[must_use]
    pub fn span(&self) -> Option<(MediaTime, MediaTime)> {
        Some((self.reads.first()?.time, self.reads.last()?.time))
    }
}

/// A clock that held a value past its unit, then resumed: the screen stopped
/// while the video ran, so the video's time is still right.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClockFreeze {
    reads: Vec<ClockRead>,
}

impl ClockFreeze {
    /// The reads that stood still.
    #[must_use]
    pub fn reads(&self) -> &[ClockRead] {
        &self.reads
    }
}

/// What a clock's jump looks like.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClockJumpKind {
    /// The offset rose: time may be missing from the video, a cut or a paused
    /// recording. It changes no clock time until a person accepts it.
    Gap,
    /// The offset fell: a stretch repeated or moved, or a clock set back.
    Backward,
}

impl ClockJumpKind {
    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Gap => "gap",
            Self::Backward => "backward",
        }
    }
}

/// A change in a clock's offset between two of its runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockJump {
    /// What it looks like.
    pub kind: ClockJumpKind,
    /// The last read before it.
    pub before: ClockRead,
    /// The first read after it.
    pub after: ClockRead,
    /// How far the offset moved: at least `low`, less than `high`.
    pub jump: OffsetInterval,
}

/// Whether a face is a clock or an elapsed timer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClockKind {
    /// It tells the time of day.
    Clock,
    /// It read zero, or could have, during the recording: a meeting's timer. It
    /// gives no anchor, no offset observation and no gap.
    Counter,
}

impl ClockKind {
    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Clock => "clock",
            Self::Counter => "counter",
        }
    }
}

/// How a clock's rate compares with the video's, over its longest stretch
/// without a jump. A single run always fits a rate of 1.0, because its reads
/// share an offset by definition; a different rate shows as runs that creep
/// apart by steps too small to be jumps, so it is judged across them.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ClockRate {
    /// How long the stretch is on the video's time, in microseconds.
    pub span_micros: i64,
    /// How far the clock moved beyond the video over it (negative: behind).
    pub drift_micros: i64,
    /// Whether the drift is more than the faces' unit can explain, so a rate
    /// other than the video's is proved.
    pub differs: bool,
}

/// One clock on the screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenClock {
    format: ClockFormat,
    unit: ClockUnit,
    kind: ClockKind,
    runs: Vec<ClockRun>,
    excursions: Vec<ClockExcursion>,
    freezes: Vec<ClockFreeze>,
    jumps: Vec<ClockJump>,
}

impl ScreenClock {
    /// Its format class.
    #[must_use]
    pub const fn format(&self) -> ClockFormat {
        self.format
    }

    /// What its faces show.
    #[must_use]
    pub const fn unit(&self) -> ClockUnit {
        self.unit
    }

    /// A clock or a counter.
    #[must_use]
    pub const fn kind(&self) -> ClockKind {
        self.kind
    }

    /// Its runs, in time order.
    #[must_use]
    pub fn runs(&self) -> &[ClockRun] {
        &self.runs
    }

    /// The stretches that left and returned.
    #[must_use]
    pub fn excursions(&self) -> &[ClockExcursion] {
        &self.excursions
    }

    /// The stretches it stood still.
    #[must_use]
    pub fn freezes(&self) -> &[ClockFreeze] {
        &self.freezes
    }

    /// Its gaps and backward jumps.
    #[must_use]
    pub fn jumps(&self) -> &[ClockJump] {
        &self.jumps
    }

    /// How many reads the track kept.
    #[must_use]
    pub fn read_count(&self) -> usize {
        self.runs.iter().map(|run| run.reads.len()).sum()
    }

    /// The narrowest offset interval of any run: what the clock's precision is.
    #[must_use]
    pub fn tightest_offset(&self) -> Option<OffsetInterval> {
        self.runs
            .iter()
            .map(ClockRun::offset)
            .min_by_key(|offset| offset.width())
    }

    /// The first moment it was read.
    #[must_use]
    pub fn first_time(&self) -> Option<MediaTime> {
        self.runs.first().and_then(ClockRun::first_time)
    }

    /// The last moment it was read.
    #[must_use]
    pub fn last_time(&self) -> Option<MediaTime> {
        self.runs.last().and_then(ClockRun::last_time)
    }

    /// Its rate over its longest stretch without a jump; `None` for a clock of
    /// one run, which always fits the video's.
    #[must_use]
    pub fn rate(&self) -> Option<ClockRate> {
        let mut best: Option<ClockRate> = None;
        let mut start = 0;
        for end in 1..=self.runs.len() {
            if !self.breaks_between(self.runs.get(end - 1), self.runs.get(end)) {
                continue;
            }
            if let Some(rate) = self.stretch_rate(&self.runs[start..end])
                && best.is_none_or(|held| rate.span_micros > held.span_micros)
            {
                best = Some(rate);
            }
            start = end;
        }
        best
    }

    /// Whether a jump or a freeze lies between two runs, or no run follows: a
    /// rate is judged only across what ran on.
    fn breaks_between(&self, before: Option<&ClockRun>, after: Option<&ClockRun>) -> bool {
        let (Some(before), Some(after)) = (before, after) else {
            return true;
        };
        let (Some(from), Some(to)) = (before.last_time(), after.first_time()) else {
            return true;
        };
        self.jumps.iter().any(|jump| jump.after.time == to)
            || self
                .freezes
                .iter()
                .flat_map(|freeze| freeze.reads.iter())
                .any(|read| (from..=to).contains(&read.time))
    }

    fn stretch_rate(&self, runs: &[ClockRun]) -> Option<ClockRate> {
        if runs.len() < 2 {
            return None;
        }
        let first = runs.first()?.reads.first()?;
        let last = runs.last()?.reads.last()?;
        let span =
            last.read.time.as_micros().cast_signed() - first.read.time.as_micros().cast_signed();
        let drift = (last.value - first.value) - span;
        Some(ClockRate {
            span_micros: span,
            drift_micros: drift,
            differs: drift.abs() > self.unit.as_micros() + PAINT_SLACK_MICROS,
        })
    }

    /// Where the clock is best known: the first read of its tightest run, the
    /// reading it gives there when it has a date, and its offset interval.
    #[must_use]
    pub fn position(&self) -> Option<(MediaTime, Option<ClockReading>, OffsetInterval)> {
        let run = self.runs.iter().min_by_key(|run| run.offset.width())?;
        let first = run.reads.first()?;
        let time = first.read.time.as_micros().cast_signed();
        let moved = run.offset.low + time - first.value;
        let reading = match first.read.face.reading() {
            Some(reading) => Some(reading.shifted(moved)?),
            None => None,
        };
        Some((first.read.time, reading, run.offset))
    }

    /// The reading the clock gives as an anchor, and how precise it is. `None`
    /// for a counter, and for a clock with no date, which cannot be written as
    /// a date and time.
    #[must_use]
    pub fn anchor(&self) -> Option<(MediaTime, ClockReading, OffsetInterval)> {
        if self.kind == ClockKind::Counter {
            return None;
        }
        let (time, reading, offset) = self.position()?;
        Some((time, reading?, offset))
    }

    /// The first read the track kept.
    #[must_use]
    pub fn first_read(&self) -> Option<&ClockRead> {
        self.runs
            .first()
            .and_then(|run| run.reads.first())
            .map(|tracked| &tracked.read)
    }

    /// Whether it was read on both sides of a moment and did not jump there:
    /// the cross-check on another clock's jump.
    #[must_use]
    pub fn spans_without_a_jump(&self, before: MediaTime, after: MediaTime) -> bool {
        let spans = self.first_time().is_some_and(|first| first <= before)
            && self.last_time().is_some_and(|last| last >= after);
        spans
            && !self
                .jumps
                .iter()
                .any(|jump| jump.before.time <= after && jump.after.time >= before)
    }

    /// Whether the two were read over a common stretch.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        match (
            self.first_time(),
            self.last_time(),
            other.first_time(),
            other.last_time(),
        ) {
            (Some(first), Some(last), Some(other_first), Some(other_last)) => {
                first <= other_last && other_first <= last
            }
            _ => false,
        }
    }

    fn adopt(&mut self, other: Self) {
        self.runs.extend(other.runs);
        self.runs.sort_by_key(ClockRun::first_time);
        self.excursions.extend(other.excursions);
        self.freezes.extend(other.freezes);
        self.jumps.extend(other.jumps);
    }
}

/// Lines that gave no clock, by why. A report's counts: an absent clock speaks
/// only for the frames read.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClockRefusals {
    /// Lines the grammar refused as too long.
    pub too_long: usize,
    /// Lines with several times in them.
    pub several_times: usize,
    /// Lines with a time scoring under the bound.
    pub low_score: usize,
    /// Faces in a place whose value never changed: a chat message, a heading.
    pub stood_still: usize,
    /// Faces in a place that moved, but not as a clock does: a countdown, a log.
    pub irregular: usize,
    /// Faces in places past [`MAX_CLOCK_PLACES`].
    pub places_past_bound: usize,
}

/// What the screen's clocks do.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClockAnalysis {
    /// The clocks, in order of first appearance.
    pub clocks: Vec<ScreenClock>,
    /// Lines that gave none.
    pub refusals: ClockRefusals,
}

impl ClockAnalysis {
    /// The times to read next, to narrow rollovers and jumps: the midpoint of
    /// each bracket still wider than [`ROLLOVER_BRACKET_MICROS`].
    ///
    /// The search keeps no state: this is a pure function of the reads, so it
    /// resumes from whatever the index holds. A minute clock's run searches its
    /// first and last rollover only, since one fixes the offset and two prove
    /// the rate. A bracket holding an excursion's or a freeze's read is left
    /// alone, because what lies inside it does not belong to the clock.
    #[must_use]
    pub fn next_reads(&self) -> Vec<MediaTime> {
        let mut times = Vec::new();
        for clock in &self.clocks {
            let outside: Vec<MediaTime> = clock
                .excursions
                .iter()
                .flat_map(|excursion| excursion.reads.iter())
                .chain(clock.freezes.iter().flat_map(|freeze| freeze.reads.iter()))
                .map(|read| read.time)
                .collect();
            if clock.unit == ClockUnit::Minute {
                for run in &clock.runs {
                    let rollovers: Vec<(MediaTime, MediaTime)> = run
                        .reads
                        .windows(2)
                        .filter(|pair| pair[1].value - pair[0].value == clock.unit.as_micros())
                        .map(|pair| (pair[0].read.time, pair[1].read.time))
                        .collect();
                    times.extend(
                        rollovers
                            .first()
                            .copied()
                            .and_then(|pair| midpoint(pair, &outside)),
                    );
                    if rollovers.len() > 1 {
                        times.extend(
                            rollovers
                                .last()
                                .copied()
                                .and_then(|pair| midpoint(pair, &outside)),
                        );
                    }
                }
            }
            for jump in &clock.jumps {
                times.extend(midpoint((jump.before.time, jump.after.time), &outside));
            }
        }
        times.sort();
        times.dedup();
        times
    }
}

fn midpoint((before, after): (MediaTime, MediaTime), outside: &[MediaTime]) -> Option<MediaTime> {
    let (from, to) = (before.as_micros(), after.as_micros());
    if to.saturating_sub(from) <= ROLLOVER_BRACKET_MICROS
        || outside.iter().any(|time| (before..=after).contains(time))
    {
        return None;
    }
    Some(MediaTime::from_micros(from + (to - from) / 2))
}

/// One place on the screen and the faces of one class read there.
struct Place {
    format: ClockFormat,
    unit: ClockUnit,
    region: TextRegion,
    reads: Vec<ClockRead>,
}

/// Every clock face of `index`, with what was refused.
#[must_use]
pub fn clock_reads(index: &ScreenTextIndex) -> (Vec<ClockRead>, ClockRefusals) {
    let mut reads = Vec::new();
    let mut refusals = ClockRefusals::default();
    for frame in index.frames() {
        for line in frame.lines() {
            match read_clock_face(line.text.as_str()) {
                Ok(face) => {
                    if line
                        .confidence
                        .basis_points()
                        .is_some_and(|score| score >= MIN_CLOCK_LINE_BASIS_POINTS)
                    {
                        reads.push(ClockRead {
                            time: frame.frame.time,
                            pts: frame.frame.pts,
                            region: line.region,
                            face,
                        });
                    } else {
                        refusals.low_score += 1;
                    }
                }
                Err(ClockFaceRefusal::TooLong) => refusals.too_long += 1,
                Err(ClockFaceRefusal::SeveralTimes) => refusals.several_times += 1,
                Err(ClockFaceRefusal::NoTime) => {}
            }
        }
    }
    (reads, refusals)
}

/// How many frames of `index` only a rollover search asked for: none of their
/// targets is a candidate or lies on the index's period.
#[must_use]
pub fn rollover_frames(index: &ScreenTextIndex) -> usize {
    let period = index.period_micros();
    index
        .frames()
        .iter()
        .filter(|frame| {
            frame.targets.iter().all(|target| {
                target.candidate.is_none() && !target.time.as_micros().is_multiple_of(period)
            })
        })
        .count()
}

/// What the clocks of `index` do, over a video of `duration`.
#[must_use]
pub fn analyse_clocks(index: &ScreenTextIndex, duration: MediaTime) -> ClockAnalysis {
    let (reads, refusals) = clock_reads(index);
    analyse_reads(reads, refusals, duration)
}

/// [`analyse_clocks`] over reads already found, in any order.
#[must_use]
pub fn analyse_reads(
    mut reads: Vec<ClockRead>,
    mut refusals: ClockRefusals,
    duration: MediaTime,
) -> ClockAnalysis {
    reads.sort_by_key(|read| (read.time, read.region.y(), read.region.x()));
    let mut places: Vec<Place> = Vec::new();
    for read in reads {
        let found = places.iter_mut().find(|place| {
            place.format == read.face.format()
                && place.unit == read.face.unit()
                && place.region.same_place(read.region)
        });
        if let Some(place) = found {
            place.region = read.region;
            place.reads.push(read);
        } else if places.len() < MAX_CLOCK_PLACES {
            places.push(Place {
                format: read.face.format(),
                unit: read.face.unit(),
                region: read.region,
                reads: vec![read],
            });
        } else {
            refusals.places_past_bound += 1;
        }
    }
    let mut clocks: Vec<ScreenClock> = Vec::new();
    for place in places {
        let (found, unplaced) = clocks_of_place(&place, duration);
        if found.is_empty() {
            let moved = place
                .reads
                .first()
                .is_some_and(|first| place.reads.iter().any(|read| read.face != first.face));
            if moved {
                refusals.irregular += unplaced;
            } else {
                refusals.stood_still += unplaced;
            }
        }
        for clock in found {
            join_or_add(&mut clocks, clock);
        }
    }
    clocks.sort_by_key(ScreenClock::first_time);
    ClockAnalysis { clocks, refusals }
}

/// One clock seen in two places, a window that moved, is one clock: same class
/// and the same offset. Two zones never meet here.
fn join_or_add(clocks: &mut Vec<ScreenClock>, clock: ScreenClock) {
    let meets = |held: &ScreenClock| {
        held.format == clock.format
            && held.unit == clock.unit
            && held.kind == clock.kind
            && matches!(
                (held.tightest_offset(), clock.tightest_offset()),
                (Some(left), Some(right)) if left.meet(right).is_some()
            )
    };
    if let Some(held) = clocks.iter_mut().find(|held| meets(held)) {
        held.adopt(clock);
    } else {
        clocks.push(clock);
    }
}

/// The runs of one place, with times of day moved to continue them.
fn runs_of(reads: &[ClockRead]) -> Vec<ClockRun> {
    let mut runs: Vec<ClockRun> = Vec::new();
    for read in reads {
        let reference = runs.last().map(|run| run.offset.middle());
        let tracked = tracked_read(*read, reference);
        let interval = OffsetInterval::of_reading(tracked.value, read.face.unit(), read.time);
        match runs.last_mut() {
            Some(run) => {
                if let Some(met) = run.offset.meet(interval) {
                    run.offset = met;
                    run.reads.push(tracked);
                } else {
                    runs.push(ClockRun {
                        reads: vec![tracked],
                        offset: interval,
                    });
                }
            }
            None => runs.push(ClockRun {
                reads: vec![tracked],
                offset: interval,
            }),
        }
    }
    runs
}

/// A time of day moved by whole days to lie nearest `reference`, so a clock
/// passing midnight keeps its offset.
fn tracked_read(read: ClockRead, reference: Option<i64>) -> Tracked {
    let value = read.face.value_micros();
    match reference {
        Some(reference) if read.face.wraps() => {
            let middle = OffsetInterval::of_reading(value, read.face.unit(), read.time).middle();
            let days = (reference - middle + DAY_MICROS / 2).div_euclid(DAY_MICROS);
            Tracked {
                read,
                value: value + days * DAY_MICROS,
            }
        }
        _ => Tracked { read, value },
    }
}

/// Folds every stretch that left a run and returned to it.
fn fold_excursions(runs: Vec<ClockRun>) -> (Vec<ClockRun>, Vec<Vec<Tracked>>) {
    let mut kept: Vec<ClockRun> = Vec::new();
    let mut departures = Vec::new();
    let mut rest = runs.into_iter().collect::<std::collections::VecDeque<_>>();
    while let Some(mut run) = rest.pop_front() {
        // The nearest later run with this one's offset. One side must be long
        // enough to be a clock, or two stray reads could swallow the clock
        // that lies between them.
        while let Some(found) = rest.iter().position(|later| {
            run.offset.meet(later.offset).is_some()
                && run.reads.len().max(later.reads.len()) >= MIN_RUN_READS
        }) {
            let mut stretch: Vec<Tracked> = Vec::new();
            for between in rest.drain(..found) {
                stretch.extend(between.reads);
            }
            departures.push(stretch);
            let Some(later) = rest.pop_front() else { break };
            if let Some(met) = run.offset.meet(later.offset) {
                run.offset = met;
            }
            run.reads.extend(later.reads);
        }
        kept.push(run);
    }
    (kept, departures)
}

/// The clocks of one place, and how many reads gave none.
fn clocks_of_place(place: &Place, duration: MediaTime) -> (Vec<ScreenClock>, usize) {
    let (runs, departures) = fold_excursions(runs_of(&place.reads));
    let mut clocks: Vec<ScreenClock> = Vec::new();
    let mut between: Vec<Tracked> = Vec::new();
    let mut unplaced = 0;
    for run in runs {
        if !run.qualifies() {
            between.extend(run.reads.iter().copied());
            continue;
        }
        let stretch = std::mem::take(&mut between);
        let Some(clock) = clocks.last_mut() else {
            unplaced += stretch.len();
            clocks.push(new_clock(place, run));
            continue;
        };
        let (last, jump) = match clock.runs.last() {
            Some(previous) => (previous.reads.last().copied(), jump_between(previous, &run)),
            None => (None, None),
        };
        let Some(jump) = jump else {
            unplaced += stretch.len();
            continue;
        };
        if is_frozen(last, &stretch, place.unit) {
            clock.freezes.push(ClockFreeze {
                reads: stretch.iter().map(|tracked| tracked.read).collect(),
            });
            clock.runs.push(run);
            continue;
        }
        unplaced += stretch.len();
        if is_another_clock(jump.jump) {
            clocks.push(new_clock(place, run));
            continue;
        }
        if jump.jump.low >= MIN_JUMP_MICROS {
            clock.jumps.push(ClockJump {
                kind: ClockJumpKind::Gap,
                ..jump
            });
        } else if jump.jump.high <= -MIN_JUMP_MICROS {
            clock.jumps.push(ClockJump {
                kind: ClockJumpKind::Backward,
                ..jump
            });
        }
        clock.runs.push(run);
    }
    unplaced += between.len();
    for departure in departures {
        let Some(start) = departure.first().map(|tracked| tracked.read.time) else {
            continue;
        };
        // A stretch belongs to the clock whose run it left.
        let owner = clocks
            .iter_mut()
            .rev()
            .find(|clock| clock.first_time().is_some_and(|first| first < start));
        let Some(clock) = owner else {
            unplaced += departure.len();
            continue;
        };
        let last = clock
            .runs
            .iter()
            .flat_map(|run| run.reads.iter())
            .rfind(|tracked| tracked.read.time < start)
            .copied();
        let reads = departure.iter().map(|tracked| tracked.read).collect();
        if is_frozen(last, &departure, place.unit) {
            clock.freezes.push(ClockFreeze { reads });
        } else {
            clock.excursions.push(ClockExcursion { reads });
        }
    }
    for clock in &mut clocks {
        clock.kind = kind_of(clock, duration);
    }
    (clocks, unplaced)
}

fn new_clock(place: &Place, run: ClockRun) -> ScreenClock {
    ScreenClock {
        format: place.format,
        unit: place.unit,
        kind: ClockKind::Clock,
        runs: vec![run],
        excursions: Vec::new(),
        freezes: Vec::new(),
        jumps: Vec::new(),
    }
}

/// A jump between two runs, its kind still to be decided.
fn jump_between(before: &ClockRun, after: &ClockRun) -> Option<ClockJump> {
    Some(ClockJump {
        kind: ClockJumpKind::Gap,
        before: before.reads.last()?.read,
        after: after.reads.first()?.read,
        jump: OffsetInterval {
            low: after.offset.low - before.offset.high,
            high: after.offset.high - before.offset.low,
        },
    })
}

/// Whether `stretch` held the value of the read before it for longer than one
/// face can show one value: the clock stood still.
fn is_frozen(before: Option<Tracked>, stretch: &[Tracked], unit: ClockUnit) -> bool {
    let (Some(before), Some(last)) = (before, stretch.last()) else {
        return false;
    };
    let held =
        last.read.time.as_micros().cast_signed() - before.read.time.as_micros().cast_signed();
    let same_face = |tracked: &Tracked| {
        tracked.value.rem_euclid(DAY_MICROS) == before.value.rem_euclid(DAY_MICROS)
    };
    held > unit.as_micros() && stretch.iter().all(same_face)
}

/// A jump of a whole number of quarter hours, of at least half an hour, within
/// its own width: most likely another machine's clock in the same place.
fn is_another_clock(jump: OffsetInterval) -> bool {
    let middle = jump.middle();
    let quarters = (middle + QUARTER_HOUR_MICROS / 2).div_euclid(QUARTER_HOUR_MICROS);
    let nearest = quarters * QUARTER_HOUR_MICROS;
    middle.abs() >= MIN_OTHER_CLOCK_JUMP_MICROS && (middle - nearest).abs() <= jump.width() / 2
}

/// A counter is a moving seconds face of no date and no AM or PM that shows
/// hour `00`, or whose offset lies in `(−duration, 0]`: it read zero during the
/// recording. The cost is a real clock near midnight (L-213).
fn kind_of(clock: &ScreenClock, duration: MediaTime) -> ClockKind {
    let mut reads = clock.runs.iter().flat_map(|run| run.reads.iter());
    let could_be = clock
        .runs
        .first()
        .and_then(|run| run.reads.first())
        .is_some_and(|tracked| tracked.read.face.could_be_a_counter());
    if !could_be {
        return ClockKind::Clock;
    }
    let shows_hour_zero = reads.any(|tracked| tracked.read.face.second_of_day() < 3_600);
    let duration = duration.as_micros().cast_signed();
    let read_zero = clock
        .runs
        .iter()
        .any(|run| run.offset.high > -duration && run.offset.low <= 0);
    if shows_hour_zero || read_zero {
        ClockKind::Counter
    } else {
        ClockKind::Clock
    }
}

#[cfg(test)]
mod tests;
