//! The search across bundles without a filesystem: rank, paging, the cursor's
//! binding, every bound, a bundle that cannot be read, and cancellation.

use std::{cell::Cell, num::NonZeroU32};

use vsift_domain::{
    Confidence, ConfidenceOrigin, CueSource, CueText, CueTiming, CursorError, FrameDimensions,
    FrameRef, ImportedCue, MediaTime, PageLimit, ParsedTranscript, ProviderLabel, ReadTarget,
    ScreenText, ScreenTextFrame, ScreenTextIndex, ScreenTextIndexParts, ScreenTextLine,
    ScreenTextOutcome, ScreenTextProfile, ScreenTextProvider, ScreenTextScope, SearchMatch,
    SearchQuery, SessionId, Sha256Hex, SidecarIdentity, SourceId, TextRegion, TimeBase,
    TranscriptFormat, TranscriptOffset, TranscriptRevision, TranscriptSegment, TranscriptWarnings,
    VisualIndex,
};

use super::{
    BUNDLE_SEARCH_READ_BUDGET, BundleHitEvidence, BundleIdentity, BundleNotSearchedReason,
    BundleScreenText, BundleSearchError, BundleSearchInput, BundleSearchOutcome,
    BundleSessionEntry, BundleTranscript, BundleUnreadable, FoundBundle, FoundBundles,
    MAX_BUNDLE_MANIFEST_BYTES, MAX_BUNDLES_PER_SEARCH, OpenedBundle, search_bundles,
};
use crate::{
    ImportedRevisionRequest, SuppliedTranscript, build_imported_revision, screen_text_index_id,
    screen_text_line_id,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const NOW: u64 = 1_000_000_000_000_000;
const SECOND: u64 = 1_000_000;
const MIB: u64 = 1024 * 1024;

fn session() -> Built<SessionId> {
    Ok(SessionId::parse("ses_0123456789abcdef")?)
}

fn source() -> Built<SourceId> {
    Ok(SourceId::from_sha256(DIGEST)?)
}

/// A transcript whose cue `n` (from 1) says `texts[n - 1]` at `n` seconds.
fn revision(texts: &[&str]) -> Built<TranscriptRevision> {
    let mut cues = Vec::new();
    for (ordinal, text) in (1_u32..).zip(texts) {
        let start = u64::from(ordinal) * SECOND;
        cues.push(ImportedCue {
            source: CueSource::new(
                NonZeroU32::new(ordinal).ok_or("zero")?,
                NonZeroU32::new(ordinal * 4).ok_or("zero")?,
            ),
            timing: CueTiming::new(start, start + 900_000)?,
            text: CueText::new((*text).to_owned(), (*text).to_owned())?,
            speaker: None,
        });
    }
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::WebVtt,
            None,
            cues,
            TranscriptWarnings::default(),
        )?,
        sidecar: SidecarIdentity::new(DIGEST, 100)?,
    };
    let duration = (u64::try_from(texts.len())? + 2) * SECOND;
    Ok(build_imported_revision(ImportedRevisionRequest {
        session_id: &session()?,
        source_id: &source()?,
        source_duration: MediaTime::from_micros(duration),
        supplied: &supplied,
        offset: TranscriptOffset::ZERO,
        number: NonZeroU32::MIN,
    })?)
}

/// A screen-text index with one read frame a second, the frame at `n` seconds
/// showing `texts[n]`.
fn index(texts: &[&str]) -> Built<ScreenTextIndex> {
    let digest = |fill: char| Sha256Hex::parse(fill.to_string().repeat(64));
    let scope = ScreenTextScope {
        session_id: session()?,
        source_id: source()?,
        stream_index: 0,
        profile: ScreenTextProfile::PpOcrV5LatinV1,
        provider: ScreenTextProvider {
            runner_sha256: digest('1')?,
            runner_version: ProviderLabel::parse("0.1.0")?,
            runtime_version: ProviderLabel::parse("1.31.0")?,
            detector_sha256: digest('2')?,
            recognizer_sha256: digest('3')?,
        },
    };
    let dimensions = FrameDimensions::new(1280, 720)?;
    let mut frames = Vec::new();
    for (pts, text) in (0_i64..).zip(texts) {
        let seconds = u64::try_from(pts)?;
        frames.push(ScreenTextFrame {
            frame: FrameRef {
                stream_index: 0,
                time_base: TimeBase::new(1, 1)?,
                pts,
                time: MediaTime::from_micros(seconds * SECOND),
                dimensions,
            },
            targets: vec![ReadTarget {
                time: MediaTime::from_micros(seconds * SECOND),
                candidate: None,
            }],
            outcome: ScreenTextOutcome::Read {
                lines: vec![ScreenTextLine {
                    id: screen_text_line_id(&scope, pts, 1)?,
                    region: TextRegion::new(10, 10, 200, 20, dimensions)?,
                    text: ScreenText::parse((*text).to_owned())?,
                    confidence: Confidence::provider_score(
                        9_000,
                        ConfidenceOrigin::ProviderUncalibrated,
                    )?,
                }],
                truncated: false,
            },
        });
    }
    Ok(ScreenTextIndex::new(ScreenTextIndexParts {
        id: screen_text_index_id(&scope, NonZeroU32::MIN)?,
        revision: NonZeroU32::MIN,
        scope,
        period_micros: SECOND,
        frames,
        unresolved: Vec::new(),
    })?)
}

/// A bundle held in memory. `calls` counts its record reads, and `cancel_at`
/// sets `cancelled` at the read of that number.
struct Stub<'a> {
    identity: BundleIdentity,
    bytes: u64,
    transcript: Result<Option<TranscriptRevision>, BundleUnreadable>,
    screen_text: Result<Option<ScreenTextIndex>, BundleUnreadable>,
    calls: &'a Cell<u32>,
    visual_reads: &'a Cell<u32>,
    cancel_at: u32,
    cancelled: &'a Cell<bool>,
}

impl Stub<'_> {
    fn read(&self) {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == self.cancel_at {
            self.cancelled.set(true);
        }
    }
}

impl OpenedBundle for Stub<'_> {
    fn identity(&self) -> &BundleIdentity {
        &self.identity
    }

    fn record_bytes(&self) -> u64 {
        self.bytes
    }

    fn read_transcript(&self) -> Result<Option<TranscriptRevision>, BundleUnreadable> {
        self.read();
        self.transcript.clone()
    }

    fn read_screen_text(&self) -> Result<Option<ScreenTextIndex>, BundleUnreadable> {
        self.read();
        self.screen_text.clone()
    }

    fn read_visual(&self) -> Result<Option<VisualIndex>, BundleUnreadable> {
        self.read();
        self.visual_reads.set(self.visual_reads.get() + 1);
        Ok(None)
    }
}

/// What a test's bundle holds.
#[derive(Clone)]
struct Held {
    name: String,
    digest: String,
    bytes: u64,
    transcript: Result<Option<TranscriptRevision>, BundleUnreadable>,
    screen_text: Result<Option<ScreenTextIndex>, BundleUnreadable>,
    open: Result<(), BundleUnreadable>,
}

fn held(name: &str, digest: char, texts: &[&str]) -> Built<Held> {
    Ok(Held {
        name: name.to_owned(),
        digest: digest.to_string().repeat(64),
        bytes: MIB,
        transcript: Ok(Some(revision(texts)?)),
        screen_text: Ok(None),
        open: Ok(()),
    })
}

/// A test's bundles and the counters of what the search did to them.
struct Fixture {
    bundles: Vec<Held>,
    opened: Cell<u32>,
    calls: Cell<u32>,
    visual_reads: Cell<u32>,
    cancelled: Cell<bool>,
    cancel_at: u32,
    read_budget: u64,
}

impl Fixture {
    fn new(bundles: Vec<Held>) -> Self {
        Self {
            bundles,
            opened: Cell::new(0),
            calls: Cell::new(0),
            visual_reads: Cell::new(0),
            cancelled: Cell::new(false),
            cancel_at: 0,
            read_budget: BUNDLE_SEARCH_READ_BUDGET,
        }
    }

    fn search(
        &self,
        query: &str,
        limit: u16,
        cursor: Option<&str>,
    ) -> Built<Result<BundleSearchOutcome<String>, BundleSearchError>> {
        let found = FoundBundles {
            bundles: self
                .bundles
                .iter()
                .enumerate()
                .map(|(at, bundle)| FoundBundle {
                    name: bundle.name.clone(),
                    location: at,
                })
                .collect(),
            skipped_entries: 0,
        };
        let query = SearchQuery::parse(query)?;
        let session = session()?;
        let source = source()?;
        Ok(search_bundles(
            &BundleSearchInput {
                found: &found,
                query: &query,
                limit: PageLimit::new(limit)?,
                cursor,
                read_budget: self.read_budget,
                now_micros: NOW,
            },
            |at: &usize| {
                self.opened.set(self.opened.get() + 1);
                let bundle = self.bundles.get(*at).ok_or(BundleUnreadable::Unreadable)?;
                bundle.open?;
                Ok(Stub {
                    identity: BundleIdentity {
                        session_id: session.clone(),
                        source_id: source.clone(),
                        source_included: false,
                        manifest_sha256: bundle.digest.clone(),
                    },
                    bytes: bundle.bytes,
                    transcript: bundle.transcript.clone(),
                    screen_text: bundle.screen_text.clone(),
                    calls: &self.calls,
                    visual_reads: &self.visual_reads,
                    cancel_at: self.cancel_at,
                    cancelled: &self.cancelled,
                })
            },
            |_: &TranscriptRevision, segment: &TranscriptSegment| segment.text().text().to_owned(),
            || self.cancelled.get(),
        ))
    }
}

fn texts(outcome: &BundleSearchOutcome<String>) -> Vec<String> {
    outcome
        .hits
        .iter()
        .map(|hit| {
            let text = match &hit.evidence {
                BundleHitEvidence::Transcript(text) => text.clone(),
                BundleHitEvidence::ScreenText(hit) => format!("screen {}", hit.line.text.as_str()),
            };
            format!("{}: {text}", hit.bundle)
        })
        .collect()
}

/// Every phrase comes before any all-terms hit, whichever bundle holds it, and
/// the order inside a tier is the order the bundles were found in.
#[test]
fn hits_rank_by_tier_then_bundle_then_time() -> TestResult {
    let fixture = Fixture::new(vec![
        held(
            "call-1",
            'a',
            &["limit of the refund", "the refund limit is 50"],
        )?,
        held("call-2", 'b', &["refund limit raised"])?,
    ]);
    let outcome = fixture.search("refund limit", 20, None)??;
    assert_eq!(
        texts(&outcome),
        [
            "call-1: the refund limit is 50",
            "call-2: refund limit raised",
            "call-1: limit of the refund",
        ]
    );
    assert_eq!(outcome.total_hits, 3);
    assert!(!outcome.has_more);
    assert_eq!(outcome.counts.searched, 2);
    let first = outcome.hits.first().ok_or("no hit")?;
    assert_eq!(first.position.tier(), SearchMatch::Phrase);
    assert_eq!(first.identity.manifest_sha256, "a".repeat(64));
    assert_eq!(first.time, MediaTime::from_micros(2 * SECOND));
    Ok(())
}

/// A segment and a line at the same time: the segment first; a later line
/// after an earlier one, whatever its kind.
#[test]
fn transcript_and_on_screen_hits_interleave_by_time() -> TestResult {
    let mut bundle = held("call-1", 'a', &["refund", "other", "refund again"])?;
    bundle.screen_text = Ok(Some(index(&["nothing", "refund", "nothing", "x refund"])?));
    let fixture = Fixture::new(vec![bundle]);
    let outcome = fixture.search("refund", 20, None)??;
    assert_eq!(
        texts(&outcome),
        [
            "call-1: refund",
            "call-1: screen refund",
            "call-1: refund again",
            "call-1: screen x refund",
        ]
    );
    Ok(())
}

/// Pages over a mixed set neither repeat nor skip a hit, and `total_hits` is
/// the same on each.
#[test]
fn pages_neither_repeat_nor_skip() -> TestResult {
    let fixture = Fixture::new(vec![
        held("a", 'a', &["one", "two one", "one three"])?,
        held("b", 'b', &["four", "one"])?,
        held("c", 'c', &["one one", "five"])?,
    ]);
    let whole = texts(&fixture.search("one", 100, None)??);
    assert_eq!(whole.len(), 5);
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = fixture.search("one", 2, cursor.as_deref())??;
        assert_eq!(page.total_hits, 5);
        seen.extend(texts(&page));
        if !page.has_more {
            break;
        }
        cursor = Some(page.hits.last().ok_or("no hit")?.continuation.clone());
    }
    assert_eq!(seen, whole);
    Ok(())
}

/// Any change to the set of bundles rejects a cursor rather than restarting:
/// another bundle, one fewer, and another query.
#[test]
fn a_cursor_is_bound_to_the_set_and_the_query() -> TestResult {
    let bundles = vec![held("a", 'a', &["one", "one"])?, held("b", 'b', &["one"])?];
    let first = Fixture::new(bundles.clone()).search("one", 1, None)??;
    let cursor = first.hits.last().ok_or("no hit")?.continuation.clone();

    let mut replaced = bundles.clone();
    if let Some(bundle) = replaced.get_mut(1) {
        bundle.digest = "c".repeat(64);
    }
    assert_eq!(
        Fixture::new(replaced)
            .search("one", 1, Some(&cursor))?
            .err(),
        Some(BundleSearchError::Cursor(CursorError::WrongQuery))
    );

    let fewer = bundles.get(..1).ok_or("no bundle")?.to_vec();
    assert_eq!(
        Fixture::new(fewer).search("one", 1, Some(&cursor))?.err(),
        Some(BundleSearchError::Cursor(CursorError::WrongGeneration))
    );

    assert_eq!(
        Fixture::new(bundles.clone())
            .search("two", 1, Some(&cursor))?
            .err(),
        Some(BundleSearchError::Cursor(CursorError::WrongQuery))
    );
    Fixture::new(bundles).search("one", 1, Some(&cursor))??;
    Ok(())
}

/// A token that is not a cursor costs no read at all.
#[test]
fn a_malformed_cursor_is_refused_before_a_bundle_is_opened() -> TestResult {
    let fixture = Fixture::new(vec![held("a", 'a', &["one"])?]);
    assert!(matches!(
        fixture.search("one", 1, Some("not-a-cursor"))?,
        Err(BundleSearchError::Cursor(_))
    ));
    assert_eq!(fixture.opened.get(), 0);
    Ok(())
}

/// With no bundle searched there is nothing a cursor could continue.
#[test]
fn a_cursor_with_nothing_searched_is_refused() -> TestResult {
    let bundles = vec![held("a", 'a', &["one", "one"])?];
    let first = Fixture::new(bundles.clone()).search("one", 1, None)??;
    let cursor = first.hits.last().ok_or("no hit")?.continuation.clone();
    let mut broken = bundles;
    if let Some(bundle) = broken.get_mut(0) {
        bundle.open = Err(BundleUnreadable::IntegrityFailure);
    }
    assert_eq!(
        Fixture::new(broken).search("one", 1, Some(&cursor))?.err(),
        Some(BundleSearchError::Cursor(CursorError::WrongSession))
    );
    Ok(())
}

/// Past the 64th bundle nothing is opened, however many there are: this is what
/// stops a folder of thousands from reading thousands of manifests.
#[test]
fn bundles_past_the_limit_are_never_opened() -> TestResult {
    let bundles = (0..70)
        .map(|at| held(&format!("call-{at:02}"), 'a', &["one"]))
        .collect::<Built<Vec<_>>>()?;
    let fixture = Fixture::new(bundles);
    let outcome = fixture.search("one", 1, None)??;
    assert_eq!(fixture.opened.get(), u32::try_from(MAX_BUNDLES_PER_SEARCH)?);
    assert_eq!(outcome.counts.found, 70);
    assert_eq!(outcome.counts.not_searched, 6);
    assert_eq!(outcome.not_searched.len(), 6);
    assert!(
        outcome
            .not_searched
            .iter()
            .all(|bundle| bundle.reason == BundleNotSearchedReason::BundleLimit)
    );
    assert_eq!(
        outcome
            .not_searched
            .first()
            .map(|bundle| bundle.bundle.as_str()),
        Some("call-64")
    );
    Ok(())
}

/// The worst case for the budget: every bundle at the record limits (24 + 48 +
/// 8 MiB). It stops at 1 GiB, manifests counted, and what does not fit is
/// named; declared sizes are charged before any record is read.
#[test]
fn the_budget_stops_bundles_at_the_record_limits() -> TestResult {
    let bundles = (0..MAX_BUNDLES_PER_SEARCH)
        .map(|at| {
            let mut bundle = held(&format!("call-{at:02}"), 'a', &["one"])?;
            bundle.digest = format!("{at:064x}");
            bundle.bytes = 80 * MIB;
            Ok(bundle)
        })
        .collect::<Built<Vec<_>>>()?;
    let fixture = Fixture::new(bundles);
    let outcome = fixture.search("one", 1, None)??;
    assert_eq!(outcome.counts.searched, 12);
    assert_eq!(outcome.counts.not_searched, MAX_BUNDLES_PER_SEARCH - 12);
    assert!(
        outcome
            .not_searched
            .iter()
            .all(|bundle| bundle.reason == BundleNotSearchedReason::ReadBudget)
    );
    let manifests = u64::from(fixture.opened.get()) * MAX_BUNDLE_MANIFEST_BYTES;
    assert!(manifests + 12 * 80 * MIB <= BUNDLE_SEARCH_READ_BUDGET);
    Ok(())
}

/// Every manifest counts: with room for only three, a fourth is never opened.
#[test]
fn manifests_count_against_the_budget() -> TestResult {
    let bundles = vec![
        held("a", 'a', &["one"])?,
        held("b", 'b', &["one"])?,
        held("c", 'c', &["one"])?,
        held("d", 'd', &["one"])?,
    ];
    let mut fixture = Fixture::new(bundles);
    for bundle in &mut fixture.bundles {
        bundle.bytes = 1;
    }
    fixture.read_budget = 3 * MAX_BUNDLE_MANIFEST_BYTES + 3;
    let outcome = fixture.search("one", 1, None)??;
    assert_eq!(fixture.opened.get(), 3);
    assert_eq!(outcome.counts.searched, 3);
    assert_eq!(
        outcome.not_searched.first().map(|bundle| bundle.reason),
        Some(BundleNotSearchedReason::ReadBudget)
    );
    Ok(())
}

/// A bundle that cannot be opened is listed, with why, and the rest are searched.
#[test]
fn a_bundle_that_cannot_be_opened_is_listed_and_the_search_goes_on() -> TestResult {
    let mut broken = held("broken", 'a', &["one"])?;
    broken.open = Err(BundleUnreadable::NotPrivate);
    let fixture = Fixture::new(vec![broken, held("good", 'b', &["one"])?]);
    let outcome = fixture.search("one", 20, None)??;
    assert_eq!(texts(&outcome), ["good: one"]);
    assert_eq!(outcome.counts.unavailable, 1);
    assert_eq!(
        outcome.sessions.first(),
        Some(&BundleSessionEntry::Unavailable {
            bundle: String::from("broken"),
            reason: BundleUnreadable::NotPrivate,
        })
    );
    Ok(())
}

/// One unreadable record loses that part only: the screen still answers, and
/// the coverage says what was lost.
#[test]
fn an_unreadable_transcript_still_gives_the_on_screen_hits() -> TestResult {
    let mut bundle = held("call", 'a', &["one"])?;
    bundle.transcript = Err(BundleUnreadable::IntegrityFailure);
    bundle.screen_text = Ok(Some(index(&["one"])?));
    let fixture = Fixture::new(vec![bundle]);
    let outcome = fixture.search("one", 20, None)??;
    assert_eq!(texts(&outcome), ["call: screen one"]);
    let Some(BundleSessionEntry::Searched(coverage)) = outcome.sessions.first() else {
        return Err("not searched".into());
    };
    assert_eq!(
        coverage.transcript,
        BundleTranscript::Unavailable(BundleUnreadable::IntegrityFailure)
    );
    assert!(coverage.transcript.has_gap());
    Ok(())
}

/// A bundle found twice is searched once; its hits are not doubled.
#[test]
fn the_same_bundle_found_twice_is_searched_once() -> TestResult {
    let fixture = Fixture::new(vec![
        held("copy", 'a', &["one"])?,
        held("original", 'a', &["one"])?,
    ]);
    let outcome = fixture.search("one", 20, None)??;
    assert_eq!(outcome.total_hits, 1);
    assert_eq!(outcome.counts.searched, 1);
    assert_eq!(outcome.counts.duplicates, 1);
    assert_eq!(outcome.sessions.len(), 1);
    Ok(())
}

/// A bundle retained without `ocr` is counted, and costs no visual-index read.
#[test]
fn on_screen_gaps_are_counted_and_cost_no_read() -> TestResult {
    let mut read = held("read", 'a', &["one"])?;
    read.screen_text = Ok(Some(index(&["one"])?));
    let fixture = Fixture::new(vec![held("not-run", 'b', &["one"])?, read]);
    let outcome = fixture.search("one", 20, None)??;
    // Both are incomplete: the second has no visual index, so its plan is not
    // known; only it paid for a visual read.
    assert_eq!(outcome.counts.screen_text_incomplete, 2);
    assert_eq!(fixture.visual_reads.get(), 1);
    let Some(BundleSessionEntry::Searched(coverage)) = outcome.sessions.first() else {
        return Err("not searched".into());
    };
    assert_eq!(coverage.screen_text, BundleScreenText::NotRun);
    Ok(())
}

/// A cancellation is honoured between bundles.
#[test]
fn a_cancellation_stops_between_bundles() -> TestResult {
    let mut first = held("a", 'a', &["one"])?;
    first.screen_text = Ok(Some(index(&["one"])?));
    let mut fixture = Fixture::new(vec![first, held("b", 'b', &["one"])?]);
    // Set by the first bundle's last read, its visual index, after the checks
    // between its records: the check before the second bundle stops the search.
    fixture.cancel_at = 3;
    assert_eq!(
        fixture.search("one", 20, None)?.err(),
        Some(BundleSearchError::Cancelled)
    );
    assert_eq!(fixture.opened.get(), 1);
    Ok(())
}

/// A cancellation is honoured between the records of one bundle.
#[test]
fn a_cancellation_stops_between_records() -> TestResult {
    let mut fixture = Fixture::new(vec![held("a", 'a', &["one"])?]);
    fixture.cancel_at = 1;
    assert_eq!(
        fixture.search("one", 20, None)?.err(),
        Some(BundleSearchError::Cancelled)
    );
    assert_eq!(fixture.calls.get(), 1);
    Ok(())
}
