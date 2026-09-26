//! The data of `candidates`, its JSON Lines evidence stream, its envelope
//! coverage and the fixed prose for its failures (P08, ADR 0018).
//!
//! A candidate is a moment at which the screen changed, or a periodic sample
//! of a screen that did not, proposed so an agent need not look at every
//! frame. Each one is published as a `visual_candidate` evidence record: its
//! identity, the actual decoded frame time that represents it
//! (`representative_us`, which `frame get` will extract in P09), the span of
//! source time it stands for, why it was proposed, how long the screen held,
//! the size of the change that opened it (uncalibrated integers, for
//! ordering only) and a similarity hash that shows when a screen repeats.
//!
//! Every result also states its coverage. The `data` member lists the
//! analysed parts of the range and each typed gap; the envelope's frozen
//! `coverage` member carries the summary a generic consumer reads:
//! `truncated` when any part of the range has a gap, the merged gaps and the
//! distinct gap reasons. Such a result has status `partial` and still exits
//! 0: a later call continues `not_analyzed` parts.

use serde::Serialize;
use vsift_domain::{
    CoverageGapReason, SessionId, SourceSegmentId, TimeRange, VISUAL_CELL_MICROS,
    VISUAL_FRAME_HEIGHT, VISUAL_FRAME_WIDTH, VISUAL_SAMPLE_INTERVAL_MICROS, VISUAL_WINDOW_MICROS,
    VisualCandidate, VisualCoverageGap, VisualIndex, VisualWindow,
};

use crate::{
    CommandName, CoverageResponse, EvidenceEventResponse, LifecycleResponse, MAX_COVERAGE_RANGES,
    OperationResponse, TerminalEventResponse, stream::EvidenceStream, transcript::RangeData,
};

/// Envelope reason: more gaps exist than the envelope lists.
const GAPS_TRUNCATED_REASON: &str = "gap_list_truncated";

/// Fixed warning of a candidates result whose range has gaps.
pub const VISUAL_COVERAGE_WARNING: &str = "Part of the requested range has no visual candidates because it is not analysed yet or could not be analysed; coverage lists each gap and its reason. Run candidates again for the same range to continue not_analyzed and deadline_exceeded parts.";

/// Remediation when `FFmpeg` or `FFprobe` is missing for visual analysis.
pub const VISUAL_TOOLS_REMEDIATION: &str = "Visual candidates are found by decoding the video's frames with FFmpeg and FFprobe; Whisper and a model are not needed. Nothing was changed. Install or locate trusted builds, register them with setup configure ffmpeg|ffprobe --executable <path>, then run setup check.";

/// Remediation when the video has no video stream.
pub const NO_VIDEO_STREAM_REMEDIATION: &str = "The source has no video stream, so it has no visual candidates. Nothing was changed. Use transcript get or search for its speech instead.";

/// Remediation when a cursor is given for a session without a visual index.
pub const CANDIDATE_CURSOR_REMEDIATION: &str = "This session has no visual candidates yet, so the cursor cannot continue any page of it. Run candidates without --cursor first; pass back only the next_cursor it returns, with the same session and range.";

/// Everything a host presents about one page of candidates.
///
/// The fields borrow the engine's typed result, so every host presents the
/// same page identically.
#[derive(Clone, Debug)]
pub struct CandidatesPresentation<'a> {
    /// Session that was read.
    pub session_id: &'a SessionId,
    /// The index revision the page was read from.
    pub index: &'a VisualIndex,
    /// The whole-file source segment the candidates lie in.
    pub source_segment_id: &'a SourceSegmentId,
    /// The requested range.
    pub range: TimeRange,
    /// The requested range clipped to the video.
    pub searched: TimeRange,
    /// Candidates of the page, in time order.
    pub candidates: &'a [VisualCandidate],
    /// Token for the next page, absent on the last page.
    pub next_cursor: Option<&'a str>,
    /// Parts of the searched range analysed windows cover, merged.
    pub analysed: &'a [TimeRange],
    /// Typed gaps in the visual coverage of the searched range.
    pub gaps: &'a [VisualCoverageGap],
}

/// A window of the fixed 60 s grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct WindowData {
    ordinal: u32,
    from_us: u64,
    to_us: u64,
}

/// Size of the change that opened a candidate: uncalibrated integers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct ChangeData {
    changed_blocks: u8,
    max_block_delta: u8,
}

/// Width and height in pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct DimensionsData {
    width: u32,
    height: u32,
}

/// The analysis that produced a candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct AnalysisData {
    profile: &'static str,
    width: usize,
    height: usize,
}

/// One published `visual_candidate` evidence record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VisualCandidateData {
    candidate_id: String,
    source_id: String,
    source_segment_id: String,
    stream_index: u32,
    window: WindowData,
    representative_us: u64,
    span: RangeData,
    change_window: Option<RangeData>,
    reasons: Vec<&'static str>,
    stability: &'static str,
    change: Option<ChangeData>,
    visual_hash: String,
    sample_count: u16,
    displayed_dimensions: DimensionsData,
    analysis: AnalysisData,
}

impl VisualCandidateData {
    /// The candidate's identity, the record's upsert key.
    pub(crate) fn candidate_id(&self) -> &str {
        &self.candidate_id
    }

    /// Presents one candidate of `index`.
    #[must_use]
    pub fn new(
        index: &VisualIndex,
        source_segment_id: &SourceSegmentId,
        candidate: &VisualCandidate,
    ) -> Self {
        let representative = candidate.representative().as_micros();
        let ordinal = u32::try_from(representative / VISUAL_WINDOW_MICROS).unwrap_or(u32::MAX);
        let window = VisualWindow::new(ordinal, index.duration()).map_or(
            WindowData {
                ordinal,
                from_us: u64::from(ordinal).saturating_mul(VISUAL_WINDOW_MICROS),
                to_us: index.duration().as_micros(),
            },
            |window| WindowData {
                ordinal,
                from_us: window.range().start().as_micros(),
                to_us: window.range().end().as_micros(),
            },
        );
        let change_window = candidate.change().and_then(|change| {
            TimeRange::new(change.previous_sample(), candidate.representative())
                .ok()
                .map(RangeData::new)
        });
        let dimensions = index.displayed_dimensions();
        Self {
            candidate_id: candidate.id().as_str().to_owned(),
            source_id: index.source_id().as_str().to_owned(),
            source_segment_id: source_segment_id.as_str().to_owned(),
            stream_index: index.stream_index(),
            window,
            representative_us: representative,
            span: RangeData::new(candidate.span()),
            change_window,
            reasons: vec![candidate.reason().identifier()],
            stability: candidate.stability().identifier(),
            change: candidate.change().map(|change| ChangeData {
                changed_blocks: change.delta().changed_blocks(),
                max_block_delta: change.delta().max_block_delta(),
            }),
            visual_hash: candidate.hash().to_hex(),
            sample_count: candidate.sample_count(),
            displayed_dimensions: DimensionsData {
                width: dimensions.width(),
                height: dimensions.height(),
            },
            analysis: AnalysisData {
                profile: index.profile().identifier(),
                width: VISUAL_FRAME_WIDTH,
                height: VISUAL_FRAME_HEIGHT,
            },
        }
    }
}

/// The index revision a page was read from and its fixed sampling grid.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct IndexData {
    index_id: String,
    number: u32,
    profile: &'static str,
    duration_us: u64,
    window_us: u64,
    sample_interval_us: u64,
    coverage_interval_us: u64,
}

impl IndexData {
    fn new(index: &VisualIndex) -> Self {
        Self {
            index_id: index.id().as_str().to_owned(),
            number: index.number().get(),
            profile: index.profile().identifier(),
            duration_us: index.duration().as_micros(),
            window_us: VISUAL_WINDOW_MICROS,
            sample_interval_us: VISUAL_SAMPLE_INTERVAL_MICROS,
            coverage_interval_us: VISUAL_CELL_MICROS,
        }
    }
}

/// One typed gap in the visual coverage of the range.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct GapData {
    from_us: u64,
    to_us: u64,
    reason: &'static str,
    dropped_candidates: u16,
}

/// The coverage of the range: what is analysed and every typed gap.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct VisualCoverageData {
    searched_range: RangeData,
    analyzed: Vec<RangeData>,
    gaps: Vec<GapData>,
    ranges_truncated: bool,
}

impl VisualCoverageData {
    fn new(page: &CandidatesPresentation<'_>) -> Self {
        let gaps = merged_typed_gaps(page.gaps);
        let ranges_truncated =
            page.analysed.len() > MAX_COVERAGE_RANGES || gaps.len() > MAX_COVERAGE_RANGES;
        Self {
            searched_range: RangeData::new(page.searched),
            analyzed: page
                .analysed
                .iter()
                .take(MAX_COVERAGE_RANGES)
                .map(|range| RangeData::new(*range))
                .collect(),
            gaps: gaps.into_iter().take(MAX_COVERAGE_RANGES).collect(),
            ranges_truncated,
        }
    }
}

/// Merges touching gaps of the same reason (budget gaps, which carry their
/// own dropped count, stay per window), in time order.
fn merged_typed_gaps(gaps: &[VisualCoverageGap]) -> Vec<GapData> {
    let mut merged: Vec<GapData> = Vec::new();
    for gap in gaps {
        let data = GapData {
            from_us: gap.range.start().as_micros(),
            to_us: gap.range.end().as_micros(),
            reason: gap.reason.identifier(),
            dropped_candidates: gap.dropped_candidates,
        };
        match merged.last_mut() {
            Some(last)
                if last.to_us == data.from_us
                    && last.reason == data.reason
                    && gap.reason != CoverageGapReason::CandidateBudgetExhausted =>
            {
                last.to_us = data.to_us;
            }
            _ => merged.push(data),
        }
    }
    merged
}

/// Data of a complete or partial `candidates` result: one bounded page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CandidatesData {
    session_id: String,
    range: RangeData,
    index: IndexData,
    coverage: VisualCoverageData,
    items: Vec<VisualCandidateData>,
    next_cursor: Option<String>,
}

impl CandidatesData {
    /// Presents one page of candidates.
    #[must_use]
    pub fn new(page: &CandidatesPresentation<'_>) -> Self {
        Self {
            session_id: page.session_id.as_str().to_owned(),
            range: RangeData::new(page.range),
            index: IndexData::new(page.index),
            coverage: VisualCoverageData::new(page),
            items: page
                .candidates
                .iter()
                .map(|candidate| {
                    VisualCandidateData::new(page.index, page.source_segment_id, candidate)
                })
                .collect(),
            next_cursor: page.next_cursor.map(str::to_owned),
        }
    }
}

/// Data of the terminal event that ends a `candidates` evidence stream: the
/// page without its items, which were the preceding evidence events, and
/// `record_count` saying how many there were.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CandidatesStreamData {
    session_id: String,
    range: RangeData,
    index: IndexData,
    coverage: VisualCoverageData,
    record_count: usize,
    next_cursor: Option<String>,
}

/// The envelope coverage of a candidates page: truncated when the range has
/// any gap, the gaps merged across reasons as `<from_us>-<to_us>`, and the
/// distinct reasons in taxonomy order.
fn envelope_coverage(gaps: &[VisualCoverageGap]) -> CoverageResponse {
    let mut ranges: Vec<TimeRange> = Vec::new();
    let mut ordered: Vec<TimeRange> = gaps.iter().map(|gap| gap.range).collect();
    ordered.sort_by_key(|range| (range.start(), range.end()));
    for range in ordered {
        match ranges.last_mut() {
            Some(last) if range.start() <= last.end() => {
                if let Ok(joined) = TimeRange::new(last.start(), last.end().max(range.end())) {
                    *last = joined;
                }
            }
            _ => ranges.push(range),
        }
    }
    let mut reasons: Vec<String> = CoverageGapReason::ALL
        .iter()
        .filter(|reason| gaps.iter().any(|gap| gap.reason == **reason))
        .map(|reason| reason.identifier().to_owned())
        .collect();
    if ranges.len() > MAX_COVERAGE_RANGES {
        reasons.push(GAPS_TRUNCATED_REASON.to_owned());
    }
    CoverageResponse::new(
        !ranges.is_empty(),
        ranges
            .iter()
            .take(MAX_COVERAGE_RANGES)
            .map(|range| format!("{}-{}", range.start().as_micros(), range.end().as_micros()))
            .collect(),
        reasons,
    )
}

/// Completes a candidates response with its lifecycle, coverage and, when
/// the range has gaps, the `partial` status and warning.
fn finish(
    response: OperationResponse<serde_json::Value>,
    gaps: &[VisualCoverageGap],
    lifecycle: LifecycleResponse,
) -> OperationResponse<serde_json::Value> {
    let response = response
        .with_lifecycle(lifecycle)
        .with_coverage(envelope_coverage(gaps));
    if gaps.is_empty() {
        response
    } else {
        response.with_warnings(&[VISUAL_COVERAGE_WARNING])
    }
}

/// The complete `candidates` result for one page.
///
/// # Errors
///
/// Returns the serialization error when the data cannot be represented as JSON.
pub fn candidates_response(
    page: &CandidatesPresentation<'_>,
    lifecycle: LifecycleResponse,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let response = OperationResponse::complete(
        CommandName::Candidates.identifier(),
        &CandidatesData::new(page),
    )?;
    Ok(finish(response, page.gaps, lifecycle))
}

/// One `candidates` page as a JSON Lines evidence stream.
///
/// The records are the page's candidates, in time order, as
/// `visual_candidate` evidence events numbered from 0; the terminal event
/// follows with the next sequence number and carries the coverage and
/// cursor. The stream holds at most the page limit of evidence events and
/// exactly one terminal event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidatesEvidenceStream {
    records: Vec<EvidenceEventResponse>,
    terminal: TerminalEventResponse,
}

impl CandidatesEvidenceStream {
    /// Presents one page of candidates as an evidence stream.
    ///
    /// # Errors
    ///
    /// Returns the serialization error when the terminal data cannot be
    /// represented as JSON.
    pub fn new(
        page: &CandidatesPresentation<'_>,
        lifecycle: LifecycleResponse,
    ) -> Result<Self, serde_json::Error> {
        let command = CommandName::Candidates;
        let records: Vec<_> = (0_u64..)
            .zip(page.candidates)
            .map(|(sequence, candidate)| {
                EvidenceEventResponse::visual_candidate(
                    sequence,
                    command,
                    VisualCandidateData::new(page.index, page.source_segment_id, candidate),
                )
            })
            .collect();
        let data = CandidatesStreamData {
            session_id: page.session_id.as_str().to_owned(),
            range: RangeData::new(page.range),
            index: IndexData::new(page.index),
            coverage: VisualCoverageData::new(page),
            record_count: records.len(),
            next_cursor: page.next_cursor.map(str::to_owned),
        };
        let result = finish(
            OperationResponse::complete(command.identifier(), &data)?,
            page.gaps,
            lifecycle,
        );
        let terminal_sequence = u64::try_from(records.len()).unwrap_or(u64::MAX);
        Ok(Self {
            records,
            terminal: TerminalEventResponse::at_sequence(result, terminal_sequence),
        })
    }
}

impl EvidenceStream for CandidatesEvidenceStream {
    fn records(&self) -> &[EvidenceEventResponse] {
        &self.records
    }

    fn terminal(&self) -> &TerminalEventResponse {
        &self.terminal
    }
}

#[cfg(test)]
mod tests {
    use vsift_domain::{CoverageGapReason, MediaTime, TimeRange, VisualCoverageGap};

    use super::{envelope_coverage, merged_typed_gaps};

    fn gap(
        from: u64,
        to: u64,
        reason: CoverageGapReason,
    ) -> Result<VisualCoverageGap, Box<dyn std::error::Error>> {
        Ok(VisualCoverageGap {
            range: TimeRange::new(MediaTime::from_micros(from), MediaTime::from_micros(to))?,
            reason,
            dropped_candidates: 0,
        })
    }

    #[test]
    fn complete_coverage_is_not_truncated() -> Result<(), Box<dyn std::error::Error>> {
        let value = serde_json::to_value(envelope_coverage(&[]))?;
        assert_eq!(
            value,
            serde_json::json!({"truncated": false, "gaps": [], "reasons": []})
        );
        Ok(())
    }

    /// Touching gaps of one reason merge in `data`; the envelope merges
    /// overlapping gaps of any reason and lists each reason once, in
    /// taxonomy order.
    #[test]
    fn gaps_merge_by_reason_in_data_and_by_range_in_the_envelope()
    -> Result<(), Box<dyn std::error::Error>> {
        let gaps = [
            gap(0, 60, CoverageGapReason::NotAnalyzed)?,
            gap(60, 120, CoverageGapReason::NotAnalyzed)?,
            gap(120, 130, CoverageGapReason::Undecodable)?,
            gap(125, 128, CoverageGapReason::NoDecodedFrame)?,
            gap(200, 210, CoverageGapReason::NoDecodedFrame)?,
        ];
        let data = merged_typed_gaps(&gaps);
        assert_eq!(data.len(), 4);
        assert_eq!((data[0].from_us, data[0].to_us), (0, 120));
        let value = serde_json::to_value(envelope_coverage(&gaps))?;
        assert_eq!(value["truncated"], true);
        assert_eq!(value["gaps"], serde_json::json!(["0-130", "200-210"]));
        assert_eq!(
            value["reasons"],
            serde_json::json!(["not_analyzed", "undecodable", "no_decoded_frame"])
        );
        Ok(())
    }

    #[test]
    fn envelope_gap_lists_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let gaps: Vec<VisualCoverageGap> = (0..150_u64)
            .map(|index| {
                gap(
                    index * 10,
                    index * 10 + 5,
                    CoverageGapReason::NoDecodedFrame,
                )
            })
            .collect::<Result<_, _>>()?;
        let value = serde_json::to_value(envelope_coverage(&gaps))?;
        assert_eq!(value["gaps"].as_array().map(Vec::len), Some(100));
        assert_eq!(
            value["reasons"],
            serde_json::json!(["no_decoded_frame", "gap_list_truncated"])
        );
        Ok(())
    }
}
