//! The data of `search`, its JSON Lines evidence stream, its envelope
//! coverage and the fixed prose for a rejected query.
//!
//! A search returns the matching transcript segments themselves: the same
//! published `transcript_segment` evidence records `transcript get` returns,
//! in rank order, with a parallel `hits` list saying how each one matched. A
//! search therefore creates no new evidence type, and an indexer that
//! consumes the `--events jsonl` stream upserts records it may already hold
//! (ADR 0018).
//!
//! Every result also states its coverage. The `data` member carries the
//! detail (`transcript_coverage`); the envelope's frozen `coverage` member
//! carries the summary a generic consumer reads: `truncated` when part of the
//! searched range has no transcript, the untranscribed gaps and the reasons.
//! Such a result has status `partial` and still exits 0.

use serde::Serialize;
use vsift_domain::{
    SearchCoverage, SearchMatch, SearchQuery, SearchQueryRejection, SessionId, TimeRange,
    TranscriptRevision, TranscriptSegment,
};

use crate::{
    CommandName, CoverageResponse, EvidenceEventResponse, LifecycleResponse, OperationResponse,
    ResultView, ScreenTextSearchData, ScreenTextSearchPresentation, TerminalEventResponse,
    TranscriptRevisionData, TranscriptSegmentData,
    growth::{
        PendingCoverData, ReadGrowth, SinceData, TRANSCRIPTION_PENDING_REASON,
        TRANSCRIPTION_PENDING_WARNING,
    },
    sanitize_untrusted_text,
    stream::EvidenceStream,
    transcript::{BriefTranscriptSegmentData, RangeData},
    view::BRIEF_VIEW,
};

/// Largest number of ranges in each `transcript_coverage` list and of gaps in
/// the envelope `coverage`; longer lists keep their first ranges in start
/// order and say they were cut.
pub const MAX_COVERAGE_RANGES: usize = 100;

/// Largest presented normalised query word. A query holds at most 256 bytes,
/// and lowercasing grows a character to at most three.
const MAX_PRESENTED_TERM_BYTES: usize = 1_024;

/// What search looks at: transcript text only, never on-screen text.
const SEARCH_SCOPE: &str = "transcript_text";

/// Envelope reason: part of the searched range has no transcript.
pub(crate) const UNTRANSCRIBED_REASON: &str = "untranscribed_range";

/// Envelope reason: more untranscribed gaps exist than the envelope lists.
const GAPS_TRUNCATED_REASON: &str = "gap_list_truncated";

/// Fixed warning of a search whose range is not fully transcribed.
pub const UNTRANSCRIBED_SEARCH_WARNING: &str = "Part of the searched range has no transcript, so words said there cannot be found; coverage lists the gaps. Run transcript retranscribe to transcribe them.";

/// Fixed warning of a search whose untranscribed gaps include a stretch where
/// the recogniser wrote words that no segment holds (the revision's
/// `local_asr.unplaced_ranges`; #359, #384), in place of
/// [`UNTRANSCRIBED_SEARCH_WARNING`].
///
/// Why a text of its own. The plain warning's advice, to retranscribe, does not
/// fit this cause: transcribing the range again cuts the audio at other points
/// and may cover it, but the recogniser may write the same words with the same
/// fault and give the same stretch back. So this one says that the stretch is
/// an upper bound (a search can call more time untranscribed than was lost),
/// that a retry may give it back, and what fills it for certain: a transcript
/// file supplied with `ingest --transcript`. It claims neither that such a file
/// exists nor that it covers the range. The plain text stays, word for word,
/// for every search whose gaps are all of other causes, so a client that reads
/// it is answered as before.
pub const UNPLACED_WORDS_SEARCH_WARNING: &str = "Part of the searched range has no transcript, so words said there cannot be found; coverage lists the gaps. Some of them are stretches where the recogniser wrote words VSift could not place in time (the revision's local_asr.unplaced_ranges); each is an upper bound, because the words took only part of it. Transcribing just that range again (transcript retranscribe with --from and --to) cuts the audio at other points and may fill it, but it may give the same stretch back; a transcript file you already have is the sure way: open the video with ingest --transcript.";

/// The warning of a search that is not complete: the one for unplaced words
/// when a gap overlaps a stretch the revision lists as unplaced, else the plain
/// one.
fn untranscribed_search_warning(
    revision: &TranscriptRevision,
    coverage: &SearchCoverage,
) -> &'static str {
    let unplaced = revision.unplaced();
    let overlaps = coverage.untranscribed().iter().any(|gap| {
        unplaced
            .iter()
            .any(|stretch| stretch.start() < gap.end() && gap.start() < stretch.end())
    });
    if overlaps {
        UNPLACED_WORDS_SEARCH_WARNING
    } else {
        UNTRANSCRIBED_SEARCH_WARNING
    }
}

/// Everything a host presents about one page of search results.
///
/// The fields borrow the engine's typed result, so every host presents the
/// same page identically.
#[derive(Clone, Debug)]
pub struct SearchPresentation<'a> {
    /// Session that was searched.
    pub session_id: &'a SessionId,
    /// The revision that was searched.
    pub revision: &'a TranscriptRevision,
    /// The normalised query.
    pub query: &'a SearchQuery,
    /// The requested range, if any.
    pub range: Option<TimeRange>,
    /// Hits in rank order, each segment with the tier it matched in.
    pub hits: Vec<(&'a TranscriptSegment, SearchMatch)>,
    /// Token for the next page, absent on the last page.
    pub next_cursor: Option<&'a str>,
    /// Coverage of the searched range.
    pub coverage: &'a SearchCoverage,
    /// The on-screen text part of a first page (ADR 0030); `None` on a page a
    /// cursor continues, which leaves the member out.
    pub screen_text: Option<ScreenTextSearchPresentation<'a>>,
    /// What a search of a growing transcript adds (ADR 0033); [`Default`] for
    /// a sealed revision searched without `--since`.
    pub growth: ReadGrowth<'a>,
}

/// How the query was read: its normalised words, in query order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct SearchQueryData {
    terms: Vec<String>,
}

impl SearchQueryData {
    pub(crate) fn new(query: &SearchQuery) -> Self {
        Self {
            terms: query
                .terms()
                .iter()
                .map(|term| sanitize_untrusted_text(term, MAX_PRESENTED_TERM_BYTES))
                .collect(),
        }
    }
}

/// One hit: the segment's identity and how it matched.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SearchHitData {
    segment_id: String,
    #[serde(rename = "match")]
    tier: &'static str,
}

/// The detailed coverage of the searched range, clipped to the source.
///
/// `search` and `transcript get` both present it, from the one domain rule
/// ([`SearchCoverage::of`]), so a range has the same gaps whichever command
/// reads it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct TranscriptCoverageData {
    basis: &'static str,
    scope: &'static str,
    searched_range: Option<RangeData>,
    transcribed_ranges: Vec<RangeData>,
    untranscribed_ranges: Vec<RangeData>,
    no_speech_ranges: Vec<RangeData>,
    ranges_truncated: bool,
    /// What an open revision will still publish (ADR 0033); absent when
    /// nothing is pending, so every other answer is presented as before.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pending_ranges: Vec<RangeData>,
    /// The earlier revision that holds text over part of the pending ranges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_covered_by: Option<PendingCoverData>,
}

impl TranscriptCoverageData {
    /// The coverage of a read, with what a growing transcript adds.
    pub(crate) fn with_growth(coverage: &SearchCoverage, growth: ReadGrowth<'_>) -> Self {
        let mut data = Self::new(coverage);
        data.pending_ranges = bounded(coverage.pending());
        data.pending_covered_by = growth.pending_covered_by.and_then(PendingCoverData::new);
        data
    }

    pub(crate) fn new(coverage: &SearchCoverage) -> Self {
        let lists = [
            coverage.transcribed(),
            coverage.untranscribed(),
            coverage.no_speech(),
        ];
        let ranges_truncated = lists.iter().any(|list| list.len() > MAX_COVERAGE_RANGES);
        Self {
            basis: coverage.basis().identifier(),
            scope: SEARCH_SCOPE,
            searched_range: coverage.searched().map(RangeData::new),
            transcribed_ranges: bounded(coverage.transcribed()),
            untranscribed_ranges: bounded(coverage.untranscribed()),
            no_speech_ranges: bounded(coverage.no_speech()),
            ranges_truncated,
            pending_ranges: Vec::new(),
            pending_covered_by: None,
        }
    }
}

/// Data of a complete `search` result: one bounded page of hits.
///
/// `items` are the matching segments as published transcript evidence
/// records, in rank order; `hits` names each item's segment and how it
/// matched, in the same order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SearchData {
    session_id: String,
    revision: TranscriptRevisionData,
    query: SearchQueryData,
    range: Option<RangeData>,
    items: Vec<TranscriptSegmentData>,
    hits: Vec<SearchHitData>,
    next_cursor: Option<String>,
    transcript_coverage: TranscriptCoverageData,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen_text: Option<ScreenTextSearchData>,
    /// What to search on from (ADR 0033): present over an open revision and
    /// for a `--since` search.
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<SinceData>,
}

impl SearchData {
    /// Presents one page of search results.
    #[must_use]
    pub fn new(page: &SearchPresentation<'_>) -> Self {
        Self {
            session_id: page.session_id.as_str().to_owned(),
            revision: TranscriptRevisionData::new(page.revision),
            query: SearchQueryData::new(page.query),
            range: page.range.map(RangeData::new),
            items: page
                .hits
                .iter()
                .map(|(segment, _)| TranscriptSegmentData::new(page.revision, segment))
                .collect(),
            hits: hit_data(page),
            next_cursor: page.next_cursor.map(str::to_owned),
            transcript_coverage: TranscriptCoverageData::with_growth(page.coverage, page.growth),
            screen_text: page.screen_text.as_ref().map(ScreenTextSearchData::new),
            since: page.growth.since.map(SinceData::new),
        }
    }
}

/// Data of a complete `search` result in the brief view (`--view brief`,
/// #365): [`SearchData`] with each item reduced to what a reader cites and
/// places (`BriefTranscriptSegmentData`), and a `view` member saying so.
///
/// Everything else is the full page's own, made by the same code: the
/// revision block that holds what every item repeats, the parallel `hits`,
/// the cursor and the coverage. A brief page never leaves out the coverage:
/// a gap is never taken as absence, in either view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SearchBriefData {
    session_id: String,
    view: &'static str,
    revision: TranscriptRevisionData,
    query: SearchQueryData,
    range: Option<RangeData>,
    items: Vec<BriefTranscriptSegmentData>,
    hits: Vec<SearchHitData>,
    next_cursor: Option<String>,
    transcript_coverage: TranscriptCoverageData,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen_text: Option<ScreenTextSearchData>,
    since: Option<SinceData>,
}

impl SearchBriefData {
    /// Presents one page of search results in the brief view.
    #[must_use]
    pub fn new(page: &SearchPresentation<'_>) -> Self {
        Self::from(&SearchData::new(page))
    }
}

impl From<&SearchData> for SearchBriefData {
    /// Projects the full page, so a brief member can only be a full member.
    fn from(full: &SearchData) -> Self {
        Self {
            session_id: full.session_id.clone(),
            view: BRIEF_VIEW,
            revision: full.revision.clone(),
            query: full.query.clone(),
            range: full.range,
            items: full
                .items
                .iter()
                .map(BriefTranscriptSegmentData::from)
                .collect(),
            hits: full.hits.clone(),
            next_cursor: full.next_cursor.clone(),
            transcript_coverage: full.transcript_coverage.clone(),
            screen_text: full.screen_text.clone(),
            since: full.since.clone(),
        }
    }
}

/// Data of the terminal event that ends a `search` evidence stream: the page
/// without its items, which were the preceding evidence events, and
/// `record_count` saying how many there were.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SearchStreamData {
    session_id: String,
    revision: TranscriptRevisionData,
    query: SearchQueryData,
    range: Option<RangeData>,
    hits: Vec<SearchHitData>,
    record_count: usize,
    next_cursor: Option<String>,
    transcript_coverage: TranscriptCoverageData,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen_text: Option<ScreenTextSearchData>,
    since: Option<SinceData>,
}

fn hit_data(page: &SearchPresentation<'_>) -> Vec<SearchHitData> {
    page.hits
        .iter()
        .map(|(segment, tier)| SearchHitData {
            segment_id: segment.id().as_str().to_owned(),
            tier: tier.identifier(),
        })
        .collect()
}

/// The first [`MAX_COVERAGE_RANGES`] of `ranges`, in start order.
pub(crate) fn bounded(ranges: &[TimeRange]) -> Vec<RangeData> {
    ranges
        .iter()
        .take(MAX_COVERAGE_RANGES)
        .map(|range| RangeData::new(*range))
        .collect()
}

/// The envelope coverage of a search: truncated when part of the searched
/// range has no transcript (`gaps`, merged and in start order), with the
/// gaps as `<from_us>-<to_us>`.
pub(crate) fn envelope_coverage(gaps: &[TimeRange]) -> CoverageResponse {
    envelope_coverage_with(gaps, &[])
}

/// [`envelope_coverage`] of a read of an open revision (ADR 0033): the reason
/// `transcription_pending` follows `untranscribed_range`.
pub(crate) fn envelope_coverage_pending(gaps: &[TimeRange]) -> CoverageResponse {
    envelope_coverage_with(gaps, &[TRANSCRIPTION_PENDING_REASON])
}

fn envelope_coverage_with(gaps: &[TimeRange], more: &[&str]) -> CoverageResponse {
    let mut reasons = Vec::new();
    if !gaps.is_empty() {
        reasons.push(UNTRANSCRIBED_REASON.to_owned());
        reasons.extend(more.iter().map(|&reason| reason.to_owned()));
    }
    if gaps.len() > MAX_COVERAGE_RANGES {
        reasons.push(GAPS_TRUNCATED_REASON.to_owned());
    }
    CoverageResponse::new(
        !gaps.is_empty(),
        gaps.iter()
            .take(MAX_COVERAGE_RANGES)
            .map(|gap| format!("{}-{}", gap.start().as_micros(), gap.end().as_micros()))
            .collect(),
        reasons,
    )
}

/// Completes a search response with its lifecycle, coverage and, when the
/// searched range is not fully transcribed, the `partial` status and warning.
fn finish(
    response: OperationResponse<serde_json::Value>,
    revision: &TranscriptRevision,
    coverage: &SearchCoverage,
    lifecycle: LifecycleResponse,
) -> OperationResponse<serde_json::Value> {
    let pending = !coverage.pending().is_empty();
    let response = response
        .with_lifecycle(lifecycle)
        .with_coverage(if pending {
            envelope_coverage_pending(coverage.untranscribed())
        } else {
            envelope_coverage(coverage.untranscribed())
        });
    if coverage.is_complete() {
        response
    } else if !pending {
        response.with_warnings(&[untranscribed_search_warning(revision, coverage)])
    } else if coverage.untranscribed() == coverage.pending() {
        // An open revision (ADR 0033) whose only gap is what is pending.
        response.with_warnings(&[TRANSCRIPTION_PENDING_WARNING])
    } else {
        response.with_warnings(&[
            untranscribed_search_warning(revision, coverage),
            TRANSCRIPTION_PENDING_WARNING,
        ])
    }
}

/// The complete `search` result for one page, in the full view.
///
/// # Errors
///
/// Returns the serialization error when the data cannot be represented as JSON.
pub fn search_response(
    page: &SearchPresentation<'_>,
    lifecycle: LifecycleResponse,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    search_response_with_view(page, lifecycle, ResultView::Full)
}

/// The complete `search` result for one page in `view`.
///
/// The view changes only the items and the `view` member: the coverage, the
/// `partial` status and the warning are the same in both, from the one
/// `finish`.
///
/// # Errors
///
/// Returns the serialization error when the data cannot be represented as JSON.
pub fn search_response_with_view(
    page: &SearchPresentation<'_>,
    lifecycle: LifecycleResponse,
    view: ResultView,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let command = CommandName::Search.identifier();
    let response = match view {
        ResultView::Full => OperationResponse::complete(command, &SearchData::new(page))?,
        ResultView::Brief => OperationResponse::complete(command, &SearchBriefData::new(page))?,
    };
    Ok(finish(response, page.revision, page.coverage, lifecycle))
}

/// One `search` page as a JSON Lines evidence stream.
///
/// The records are the matching segments, in rank order, as
/// `transcript_segment` evidence events numbered from 0; the terminal event
/// follows with the next sequence number and carries the hit list, coverage
/// and cursor. The stream holds at most the page limit of evidence events and
/// exactly one terminal event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchEvidenceStream {
    records: Vec<EvidenceEventResponse>,
    terminal: TerminalEventResponse,
}

impl SearchEvidenceStream {
    /// Presents one page of search results as an evidence stream.
    ///
    /// # Errors
    ///
    /// Returns the serialization error when the terminal data cannot be
    /// represented as JSON.
    pub fn new(
        page: &SearchPresentation<'_>,
        lifecycle: LifecycleResponse,
    ) -> Result<Self, serde_json::Error> {
        let command = CommandName::Search;
        let records: Vec<_> = (0_u64..)
            .zip(&page.hits)
            .map(|(sequence, (segment, _))| {
                EvidenceEventResponse::transcript_segment(sequence, command, page.revision, segment)
            })
            .collect();
        let data = SearchStreamData {
            session_id: page.session_id.as_str().to_owned(),
            revision: TranscriptRevisionData::new(page.revision),
            query: SearchQueryData::new(page.query),
            range: page.range.map(RangeData::new),
            hits: hit_data(page),
            record_count: records.len(),
            next_cursor: page.next_cursor.map(str::to_owned),
            transcript_coverage: TranscriptCoverageData::with_growth(page.coverage, page.growth),
            screen_text: page.screen_text.as_ref().map(ScreenTextSearchData::new),
            since: page.growth.since.map(SinceData::new),
        };
        let result = finish(
            OperationResponse::complete(command.identifier(), &data)?,
            page.revision,
            page.coverage,
            lifecycle,
        );
        let terminal_sequence = u64::try_from(records.len()).unwrap_or(u64::MAX);
        Ok(Self {
            records,
            terminal: TerminalEventResponse::at_sequence(result, terminal_sequence),
        })
    }
}

impl EvidenceStream for SearchEvidenceStream {
    fn records(&self) -> &[EvidenceEventResponse] {
        &self.records
    }

    fn terminal(&self) -> &TerminalEventResponse {
        &self.terminal
    }
}

/// Structured remediation for a rejected search query.
///
/// The summary is fixed prose chosen by the typed reason; it never repeats
/// the query.
#[must_use]
pub fn search_query_rejection_summary(rejection: SearchQueryRejection) -> String {
    let advice = match rejection {
        SearchQueryRejection::Empty => "Give --query at least one letter or digit to search for.",
        SearchQueryRejection::TooLong => "Shorten --query to at most 256 bytes.",
        SearchQueryRejection::TooManyTerms => {
            "Use at most 16 words in --query; search for the most distinctive words."
        }
        SearchQueryRejection::ControlCharacter => {
            "Remove control characters, including tabs and line breaks, from --query."
        }
    };
    format!(
        "The search query was rejected ({}). {advice}",
        rejection.identifier()
    )
}

#[cfg(test)]
mod tests {
    use vsift_domain::{MediaTime, TimeRange};

    use super::{MAX_COVERAGE_RANGES, bounded, envelope_coverage};

    fn gaps(count: u64) -> Result<Vec<TimeRange>, Box<dyn std::error::Error>> {
        (0..count)
            .map(|index| {
                Ok(TimeRange::new(
                    MediaTime::from_micros(index * 10),
                    MediaTime::from_micros(index * 10 + 5),
                )?)
            })
            .collect()
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

    /// More gaps than the envelope lists keep the first 100 in start order
    /// and say so with a distinct reason.
    #[test]
    fn gap_lists_are_bounded_and_say_when_they_are_cut() -> Result<(), Box<dyn std::error::Error>> {
        let exact = serde_json::to_value(envelope_coverage(&gaps(100)?))?;
        assert_eq!(exact["gaps"].as_array().map(Vec::len), Some(100));
        assert_eq!(exact["reasons"], serde_json::json!(["untranscribed_range"]));

        let many = gaps(150)?;
        let value = serde_json::to_value(envelope_coverage(&many))?;
        assert_eq!(value["truncated"], true);
        assert_eq!(
            value["gaps"].as_array().map(Vec::len),
            Some(MAX_COVERAGE_RANGES)
        );
        assert_eq!(value["gaps"][0], "0-5");
        assert_eq!(value["gaps"][99], "990-995");
        assert_eq!(
            value["reasons"],
            serde_json::json!(["untranscribed_range", "gap_list_truncated"])
        );
        assert_eq!(bounded(&many).len(), MAX_COVERAGE_RANGES);
        Ok(())
    }
}
