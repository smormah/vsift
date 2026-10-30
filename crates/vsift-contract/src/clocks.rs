//! The data of `clocks` (ADR 0029, stage 2).
//!
//! Everything here is derived from text read off the screen, so every claim
//! comes with the frames that prove it: a person can open them with `frame get`.
//! An observation is an inference and says so. An excursion offers its two
//! explanations and chooses neither. Absence is never a claim: `no clock` speaks
//! only for the frames read, and the `refused` counts say what was set aside. A
//! clock's text is parsed into numbers and is never echoed.

use serde::Serialize;
use vsift_application::ScreenTextStop;
use vsift_domain::{
    AnchorConsistency, CLOCK_RULES_VERSION, ClockAnalysis, ClockConflict, ClockExcursion,
    ClockFreeze, ClockId, ClockJump, ClockRead, ClockRefusals, ClockRegister, MediaTime,
    OffsetObservation, ScreenClock, ScreenTextIndex, SessionId, TimeAnchors,
};

use crate::{
    CommandName, CoverageResponse, FRAMES_PENDING_REASON, LifecycleResponse, NOT_RUN_REASON,
    OperationResponse, TimeAnchorData, screen_text::IndexData,
};

/// The most stretches of one kind a clock lists; `*_total` counts them all.
const MAX_LISTED: usize = 64;

/// Envelope coverage reason: the search has reads still to make.
pub const READS_PENDING_REASON: &str = "reads_pending";

/// Fixed warning of a `clocks` result that is not complete.
pub const CLOCKS_PARTIAL_WARNING: &str = "Not everything is known about this session's clocks yet: frames ocr planned are unread, or the search around a clock's rollovers has reads left. Run ocr, then clocks, again to continue. A clock not listed may simply not have been read.";

/// Remediation of a statement about a clock or gap that cannot apply.
pub const CLOCK_RULES_REMEDIATION: &str = "The statement names a clock or gap this session does not have, or a zone that cannot apply: a counter has no zone, and a clock with no date needs an anchor that names an instant, which a session gets from ingest --recorded-at. Nothing was changed. Run clocks without the statement to see the ids. Pass only what the user stated: never an observation or a gap you inferred.";

/// Everything a host presents about one `clocks` call.
#[derive(Clone, Debug)]
pub struct ClocksPresentation<'a> {
    /// The session.
    pub session_id: &'a SessionId,
    /// `not_run`, `partial` or `complete`.
    pub state: &'static str,
    /// The newest screen-text index, if any.
    pub index: Option<&'a ScreenTextIndex>,
    /// What the clocks do.
    pub analysis: &'a ClockAnalysis,
    /// The clocks and gaps the session has named.
    pub register: &'a ClockRegister,
    /// The session's anchors after the call.
    pub anchors: &'a TimeAnchors,
    /// What each clock of unknown zone appears to be.
    pub observations: &'a [OffsetObservation],
    /// What disagrees.
    pub conflicts: &'a [ClockConflict],
    /// Frames this call read.
    pub read_now: usize,
    /// Why the call stopped early, if it did.
    pub stop: Option<ScreenTextStop>,
    /// How many reads the search would make next.
    pub reads_pending: usize,
    /// Whether `ocr` has planned frames still to read.
    pub frames_pending: bool,
}

/// An exact place on a frame: a claim's proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct ReadData {
    time_us: u64,
    pts: i64,
    region: RegionData,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct RegionData {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl ReadData {
    fn new(read: &ClockRead) -> Self {
        Self {
            time_us: read.time.as_micros(),
            pts: read.pts,
            region: RegionData {
                x: read.region.x(),
                y: read.region.y(),
                width: read.region.width(),
                height: read.region.height(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[allow(
    clippy::struct_field_names,
    reason = "The names are the published JSON, and every time in it says its unit"
)]
struct OffsetData {
    low_us: i64,
    high_us: i64,
    precision_us: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct RateData {
    span_us: i64,
    drift_us: i64,
    differs: bool,
}

/// Reads that left a clock and returned. The cause is not known.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ExcursionData {
    from_us: u64,
    to_us: u64,
    reads: usize,
    first: ReadData,
    last: ReadData,
    explanations: [&'static str; 2],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct FreezeData {
    from_us: u64,
    to_us: u64,
    reads: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct JumpData {
    kind: &'static str,
    gap_id: Option<String>,
    before: ReadData,
    after: ReadData,
    jump_us: i64,
    uncertainty_us: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ClockData {
    clock_id: Option<String>,
    kind: &'static str,
    format: &'static str,
    unit: &'static str,
    reads: usize,
    first_us: Option<u64>,
    last_us: Option<u64>,
    offset: Option<OffsetData>,
    confirmed_utc_offset: Option<String>,
    rate: Option<RateData>,
    first: Option<ReadData>,
    excursions: Vec<ExcursionData>,
    excursions_total: usize,
    freezes: Vec<FreezeData>,
    jumps: Vec<JumpData>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ObservationData {
    clock_id: String,
    anchor: &'static str,
    /// Always true: an observation is never applied to a clock time.
    inference: bool,
    measured_us: i64,
    uncertainty_us: i64,
    fits: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ConflictData {
    kind: &'static str,
    clock_id: Option<String>,
    other_clock_id: Option<String>,
    at_us: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct RefusedData {
    too_long: usize,
    several_times: usize,
    low_score: usize,
    stood_still: usize,
    irregular: usize,
    places_past_bound: usize,
}

impl RefusedData {
    const fn new(refusals: ClockRefusals) -> Self {
        Self {
            too_long: refusals.too_long,
            several_times: refusals.several_times,
            low_score: refusals.low_score,
            stood_still: refusals.stood_still,
            irregular: refusals.irregular,
            places_past_bound: refusals.places_past_bound,
        }
    }
}

/// Data of a complete or partial `clocks` result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ClocksData {
    session_id: String,
    state: &'static str,
    rules_version: u32,
    index: Option<IndexData>,
    read_now: usize,
    stopped: Option<&'static str>,
    reads_pending: usize,
    clocks: Vec<ClockData>,
    observations: Vec<ObservationData>,
    conflicts: Vec<ConflictData>,
    refused: RefusedData,
    time_anchor: Option<TimeAnchorData>,
}

fn excursion_data(excursion: &ClockExcursion) -> Option<ExcursionData> {
    let first = excursion.reads().first()?;
    let last = excursion.reads().last()?;
    Some(ExcursionData {
        from_us: first.time.as_micros(),
        to_us: last.time.as_micros(),
        reads: excursion.reads().len(),
        first: ReadData::new(first),
        last: ReadData::new(last),
        explanations: ["misread", "another_clock"],
    })
}

fn freeze_data(freeze: &ClockFreeze) -> Option<FreezeData> {
    Some(FreezeData {
        from_us: freeze.reads().first()?.time.as_micros(),
        to_us: freeze.reads().last()?.time.as_micros(),
        reads: freeze.reads().len(),
    })
}

fn jump_data(jump: &ClockJump, register: &ClockRegister) -> JumpData {
    let gap = register
        .gaps()
        .iter()
        .find(|gap| gap.at == jump.after.time)
        .map(|gap| gap.id.to_string());
    JumpData {
        kind: jump.kind.identifier(),
        gap_id: gap,
        before: ReadData::new(&jump.before),
        after: ReadData::new(&jump.after),
        jump_us: jump.jump.middle(),
        uncertainty_us: jump.jump.width() / 2,
    }
}

fn clock_data(index: usize, clock: &ScreenClock, register: &ClockRegister) -> ClockData {
    let id = register.id_of(index);
    let confirmed = id
        .and_then(|id| register.entries().iter().find(|entry| entry.id == id))
        .and_then(|entry| entry.confirmed_offset);
    ClockData {
        clock_id: id.map(|id| id.to_string()),
        kind: clock.kind().identifier(),
        format: clock.format().identifier(),
        unit: clock.unit().identifier(),
        reads: clock.read_count(),
        first_us: clock.first_time().map(MediaTime::as_micros),
        last_us: clock.last_time().map(MediaTime::as_micros),
        offset: clock.tightest_offset().map(|offset| OffsetData {
            low_us: offset.low(),
            high_us: offset.high(),
            precision_us: offset.width(),
        }),
        confirmed_utc_offset: confirmed.map(|offset| offset.to_string()),
        rate: clock.rate().map(|rate| RateData {
            span_us: rate.span_micros,
            drift_us: rate.drift_micros,
            differs: rate.differs,
        }),
        first: clock.first_read().map(ReadData::new),
        excursions: clock
            .excursions()
            .iter()
            .take(MAX_LISTED)
            .filter_map(excursion_data)
            .collect(),
        excursions_total: clock.excursions().len(),
        freezes: clock
            .freezes()
            .iter()
            .take(MAX_LISTED)
            .filter_map(freeze_data)
            .collect(),
        jumps: clock
            .jumps()
            .iter()
            .take(MAX_LISTED)
            .map(|jump| jump_data(jump, register))
            .collect(),
    }
}

fn id_text(id: Option<ClockId>) -> Option<String> {
    id.map(|id| id.to_string())
}

fn conflict_data(conflict: &ClockConflict) -> ConflictData {
    match *conflict {
        ClockConflict::RatesDiffer { clock, other } => ConflictData {
            kind: "rates_differ",
            clock_id: id_text(clock),
            other_clock_id: id_text(other),
            at_us: None,
        },
        ClockConflict::JumpNotShared { clock, other, at } => ConflictData {
            kind: "jump_not_shared",
            clock_id: id_text(clock),
            other_clock_id: id_text(other),
            at_us: Some(at.as_micros()),
        },
        ClockConflict::Backward { clock, at } => ConflictData {
            kind: "backward",
            clock_id: id_text(clock),
            other_clock_id: None,
            at_us: Some(at.as_micros()),
        },
    }
}

impl ClocksData {
    /// Presents one `clocks` call.
    #[must_use]
    pub fn new(call: &ClocksPresentation<'_>) -> Self {
        let mut conflicts: Vec<ConflictData> = call.conflicts.iter().map(conflict_data).collect();
        if call.anchors.consistency() == Some(AnchorConsistency::Conflicting) {
            conflicts.insert(
                0,
                ConflictData {
                    kind: "instants_disagree",
                    clock_id: None,
                    other_clock_id: None,
                    at_us: None,
                },
            );
        }
        Self {
            session_id: call.session_id.as_str().to_owned(),
            state: call.state,
            rules_version: CLOCK_RULES_VERSION,
            index: call.index.map(IndexData::new),
            read_now: call.read_now,
            stopped: call.stop.map(ScreenTextStop::identifier),
            reads_pending: call.reads_pending,
            clocks: call
                .analysis
                .clocks
                .iter()
                .enumerate()
                .map(|(index, clock)| clock_data(index, clock, call.register))
                .collect(),
            observations: call
                .observations
                .iter()
                .map(|observation| ObservationData {
                    clock_id: observation.clock.to_string(),
                    anchor: observation.anchor.identifier(),
                    inference: true,
                    measured_us: observation.measured_micros,
                    uncertainty_us: observation.uncertainty_micros,
                    fits: observation.fits.iter().map(ToString::to_string).collect(),
                })
                .collect(),
            conflicts,
            refused: RefusedData::new(call.analysis.refusals),
            time_anchor: TimeAnchorData::new(call.anchors),
        }
    }
}

/// Truncated until the session has been read and the search is done.
fn envelope_coverage(call: &ClocksPresentation<'_>) -> CoverageResponse {
    let mut reasons = Vec::new();
    if call.index.is_none() {
        reasons.push(NOT_RUN_REASON.to_owned());
    } else {
        if call.frames_pending {
            reasons.push(FRAMES_PENDING_REASON.to_owned());
        }
        if call.reads_pending > 0 {
            reasons.push(READS_PENDING_REASON.to_owned());
        }
    }
    CoverageResponse::new(!reasons.is_empty(), Vec::new(), reasons)
}

/// The `clocks` result: `partial`, with a warning, until the session has been
/// read and the search is done.
///
/// # Errors
///
/// Returns the serialization error when the data cannot be represented as JSON.
pub fn clocks_response(
    call: &ClocksPresentation<'_>,
    lifecycle: LifecycleResponse,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let data = ClocksData::new(call);
    let coverage = envelope_coverage(call);
    let complete = call.index.is_some() && !call.frames_pending && call.reads_pending == 0;
    let response = OperationResponse::complete(CommandName::Clocks.identifier(), &data)?
        .with_lifecycle(lifecycle)
        .with_coverage(coverage);
    Ok(if complete {
        response
    } else {
        response.with_warnings(&[CLOCKS_PARTIAL_WARNING])
    })
}
