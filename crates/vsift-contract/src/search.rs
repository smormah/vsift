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
    TerminalEventResponse, TranscriptRevisionData, TranscriptSegmentData, sanitize_untrusted_text,
    stream::EvidenceStream, transcript::RangeData,
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
const UNTRANSCRIBED_REASON: &str = "untranscribed_range";

/// Envelope reason: more untranscribed gaps exist than the envelope lists.
const GAPS_TRUNCATED_REASON: &str = "gap_list_truncated";

/// Fixed warning of a search whose range is not fully transcribed.
pub const UNTRANSCRIBED_SEARCH_WARNING: &str = "Part of the searched range has no transcript, so words said there cannot be found; coverage lists the gaps. Run transcript retranscribe to transcribe them.";

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
}

/// How the query was read: its normalised words, in query order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SearchQueryData {
    terms: Vec<String>,
}

impl SearchQueryData {
    fn new(query: &SearchQuery) -> Self {
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
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct TranscriptCoverageData {
    basis: &'static str,
    scope: &'static str,
    searched_range: Option<RangeData>,
    transcribed_ranges: Vec<RangeData>,
    untranscribed_ranges: Vec<RangeData>,
    no_speech_ranges: Vec<RangeData>,
    ranges_truncated: bool,
}

impl TranscriptCoverageData {
    fn new(coverage: &SearchCoverage) -> Self {
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
            transcript_coverage: TranscriptCoverageData::new(page.coverage),
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
fn bounded(ranges: &[TimeRange]) -> Vec<RangeData> {
    ranges
        .iter()
        .take(MAX_COVERAGE_RANGES)
        .map(|range| RangeData::new(*range))
        .collect()
}

/// The envelope coverage of a search: truncated when part of the searched
/// range has no transcript (`gaps`, merged and in start order), with the
/// gaps as `<from_us>-<to_us>`.
fn envelope_coverage(gaps: &[TimeRange]) -> CoverageResponse {
    let mut reasons = Vec::new();
    if !gaps.is_empty() {
        reasons.push(UNTRANSCRIBED_REASON.to_owned());
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
    coverage: &SearchCoverage,
    lifecycle: LifecycleResponse,
) -> OperationResponse<serde_json::Value> {
    let response = response
        .with_lifecycle(lifecycle)
        .with_coverage(envelope_coverage(coverage.untranscribed()));
    if coverage.is_complete() {
        response
    } else {
        response.with_warnings(&[UNTRANSCRIBED_SEARCH_WARNING])
    }
}

/// The complete `search` result for one page.
///
/// # Errors
///
/// Returns the serialization error when the data cannot be represented as JSON.
pub fn search_response(
    page: &SearchPresentation<'_>,
    lifecycle: LifecycleResponse,
) -> Result<OperationResponse<serde_json::Value>, serde_json::Error> {
    let response =
        OperationResponse::complete(CommandName::Search.identifier(), &SearchData::new(page))?;
    Ok(finish(response, page.coverage, lifecycle))
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
            transcript_coverage: TranscriptCoverageData::new(page.coverage),
        };
        let result = finish(
            OperationResponse::complete(command.identifier(), &data)?,
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
