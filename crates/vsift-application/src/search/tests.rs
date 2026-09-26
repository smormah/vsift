//! C-03 for search: page bounds, cursor scope, expiry and complete paging.

use std::{collections::BTreeSet, num::NonZeroU32};

use proptest::{
    collection::vec,
    prelude::{prop_assert, prop_assert_eq, proptest},
    sample::select,
};
use vsift_domain::{
    CueSource, CueText, CueTiming, CursorError, CursorToken, ImportedCue, MediaTime, PageLimit,
    ParsedTranscript, SearchMatch, SearchQuery, SessionId, SidecarIdentity, SourceId, TimeRange,
    TranscriptFormat, TranscriptOffset, TranscriptRevision, TranscriptWarnings,
};

use super::{SearchPageRequest, page_search, search_query_digest};
use crate::{
    ImportedRevisionRequest, SuppliedTranscript, TranscriptQueryError, build_imported_revision,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const EXPIRES: u64 = 2_000_000_000_000_000;
const NOW: u64 = 1_000_000_000_000_000;
const SECOND: u64 = 1_000_000;

fn session(suffix: &str) -> Built<SessionId> {
    Ok(SessionId::parse(format!("ses_{suffix}"))?)
}

/// An imported revision numbered `number` whose cue `n` (from 1) says
/// `texts[n - 1]`, starting at `n` seconds and lasting 0.9 s.
fn revision(texts: &[&str], number: u32) -> Built<TranscriptRevision> {
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
        session_id: &session("0123456789abcdef")?,
        source_id: &SourceId::from_sha256(DIGEST)?,
        source_duration: MediaTime::from_micros(duration),
        supplied: &supplied,
        offset: TranscriptOffset::ZERO,
        number: NonZeroU32::new(number).ok_or("zero")?,
    })?)
}

fn range(from: u64, to: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from),
        MediaTime::from_micros(to),
    )?)
}

fn request(query: &str, limit: u16, cursor: Option<String>) -> Built<SearchPageRequest> {
    Ok(SearchPageRequest {
        query: SearchQuery::parse(query)?,
        range: None,
        limit: PageLimit::new(limit)?,
        cursor,
    })
}

const TEXTS: [&str; 8] = [
    "Dialog R-17 is displayed now.",
    "dialog then r 17",
    "nothing to see",
    "R17 and the dialog",
    "17 r dialog",
    "open dialog R-17",
    "unrelated words",
    "dialog r17",
];

/// Pages `request` to the end, returning (tier, ordinal) of every hit.
fn all_pages(
    session_id: &SessionId,
    revision: &TranscriptRevision,
    mut request: SearchPageRequest,
) -> Built<Vec<(SearchMatch, u32)>> {
    let mut seen = Vec::new();
    loop {
        let page = page_search(session_id, revision, &request, EXPIRES, NOW)?;
        assert!(page.hits.len() <= usize::from(request.limit.get()));
        seen.extend(
            page.hits
                .iter()
                .map(|hit| (hit.tier(), hit.segment().ordinal())),
        );
        match page.next_cursor {
            Some(cursor) => request.cursor = Some(cursor.encode()),
            None => return Ok(seen),
        }
    }
}

/// C-03: limits 1 and 100 page completely; 0 and 101 are not page limits.
#[test]
fn page_limits_bound_every_page() -> TestResult {
    let session_id = session("0123456789abcdef")?;
    let revision = revision(&TEXTS, 1)?;
    let everything = all_pages(&session_id, &revision, request("dialog r 17", 100, None)?)?;
    assert_eq!(
        everything,
        [
            (SearchMatch::Phrase, 1),
            (SearchMatch::Phrase, 6),
            (SearchMatch::Phrase, 8),
            (SearchMatch::AllTerms, 2),
            (SearchMatch::AllTerms, 5),
        ]
    );
    assert_eq!(
        all_pages(&session_id, &revision, request("dialog r 17", 1, None)?)?,
        everything
    );
    assert!(PageLimit::new(0).is_err());
    assert!(PageLimit::new(PageLimit::MAX + 1).is_err());
    Ok(())
}

#[test]
fn an_empty_result_has_no_cursor_and_keeps_its_coverage() -> TestResult {
    let session_id = session("0123456789abcdef")?;
    let revision = revision(&TEXTS, 1)?;
    let page = page_search(
        &session_id,
        &revision,
        &request("absent", 20, None)?,
        EXPIRES,
        NOW,
    )?;
    assert!(page.hits.is_empty());
    assert!(page.next_cursor.is_none());
    assert!(page.coverage.is_complete());

    let mut windowed = request("dialog", 20, None)?;
    windowed.range = Some(range(2_950_000, 3_500_000)?);
    let page = page_search(&session_id, &revision, &windowed, EXPIRES, NOW)?;
    assert!(page.hits.is_empty(), "segment 3 has no dialog");
    Ok(())
}

/// A cursor may be used again and yields the same page; queries that
/// normalise to the same words are one query and share cursors.
#[test]
fn cursors_are_reusable_and_bound_to_the_normalised_query() -> TestResult {
    let session_id = session("0123456789abcdef")?;
    let revision = revision(&TEXTS, 1)?;
    let first = page_search(
        &session_id,
        &revision,
        &request("R-17", 2, None)?,
        EXPIRES,
        NOW,
    )?;
    let cursor = first.next_cursor.ok_or("expected a cursor")?.encode();
    let again = |query: &str| -> Built<Vec<u32>> {
        Ok(page_search(
            &session_id,
            &revision,
            &request(query, 2, Some(cursor.clone()))?,
            EXPIRES,
            NOW,
        )?
        .hits
        .iter()
        .map(|hit| hit.segment().ordinal())
        .collect())
    };
    let second = again("R-17")?;
    assert_eq!(again("R-17")?, second);
    assert_eq!(again("r17")?, second);
    assert!(!second.is_empty());
    Ok(())
}

#[test]
fn cursors_from_another_query_session_revision_or_time_are_rejected() -> TestResult {
    let session_id = session("0123456789abcdef")?;
    let revision = revision(&TEXTS, 1)?;
    let first = page_search(
        &session_id,
        &revision,
        &request("dialog", 1, None)?,
        EXPIRES,
        NOW,
    )?;
    let cursor = first.next_cursor.ok_or("expected a cursor")?.encode();
    let continued = |request: SearchPageRequest,
                     session_id: &SessionId,
                     revision: &TranscriptRevision,
                     now: u64| {
        page_search(session_id, revision, &request, EXPIRES, now).map(|page| page.hits.len())
    };
    let with_cursor = |query: &str| request(query, 1, Some(cursor.clone()));

    assert_eq!(
        continued(with_cursor("dialog")?, &session_id, &revision, NOW),
        Ok(1)
    );
    assert_eq!(
        continued(with_cursor("dialog r17")?, &session_id, &revision, NOW),
        Err(TranscriptQueryError::Cursor(CursorError::WrongQuery))
    );
    let mut ranged = with_cursor("dialog")?;
    ranged.range = Some(range(0, 5 * SECOND)?);
    assert_eq!(
        continued(ranged, &session_id, &revision, NOW),
        Err(TranscriptQueryError::Cursor(CursorError::WrongQuery))
    );
    assert_eq!(
        continued(
            with_cursor("dialog")?,
            &session("fedcba9876543210")?,
            &revision,
            NOW
        ),
        Err(TranscriptQueryError::Cursor(CursorError::WrongSession))
    );
    assert_eq!(
        continued(
            with_cursor("dialog")?,
            &session_id,
            &self::revision(&TEXTS, 2)?,
            NOW
        ),
        Err(TranscriptQueryError::Cursor(CursorError::WrongGeneration))
    );
    assert_eq!(
        continued(with_cursor("dialog")?, &session_id, &revision, EXPIRES),
        Err(TranscriptQueryError::Cursor(CursorError::Expired))
    );
    // A transcript.get cursor is not a search cursor.
    let transcript_cursor = CursorToken::new(
        session_id.clone(),
        1,
        crate::transcript_query_digest(revision.id(), range(0, 10 * SECOND)?)?,
        "1",
        EXPIRES,
    )?;
    assert_eq!(
        continued(
            request("dialog", 1, Some(transcript_cursor.encode()))?,
            &session_id,
            &revision,
            NOW
        ),
        Err(TranscriptQueryError::Cursor(CursorError::WrongQuery))
    );
    assert_eq!(
        continued(
            request("dialog", 1, Some("v1|not-a-cursor".to_owned()))?,
            &session_id,
            &revision,
            NOW
        ),
        Err(TranscriptQueryError::Cursor(CursorError::Malformed))
    );
    Ok(())
}

#[test]
fn forged_positions_are_rejected() -> TestResult {
    let session_id = session("0123456789abcdef")?;
    let revision = revision(&TEXTS, 1)?;
    let query = SearchQuery::parse("dialog")?;
    let digest = search_query_digest(revision.id(), None, &query)?;
    for key in [
        "1", "0-1", "3-1", "1-0", "1-01", "1-9", "01-1", "1-1-1", "a-1",
    ] {
        let token = CursorToken::new(session_id.clone(), 1, digest.clone(), key, EXPIRES)?;
        let result = page_search(
            &session_id,
            &revision,
            &request("dialog", 20, Some(token.encode()))?,
            EXPIRES,
            NOW,
        );
        assert_eq!(
            result.map(|page| page.hits.len()),
            Err(TranscriptQueryError::Cursor(
                CursorError::InvalidLastItemKey
            )),
            "{key}"
        );
    }
    for key in ["1-8", "2-8"] {
        let token = CursorToken::new(session_id.clone(), 1, digest.clone(), key, EXPIRES)?;
        assert!(
            page_search(
                &session_id,
                &revision,
                &request("dialog", 20, Some(token.encode()))?,
                EXPIRES,
                NOW,
            )
            .is_ok(),
            "{key}"
        );
    }
    Ok(())
}

/// S-11 in memory: a revision at the 20,000-segment import bound pages
/// completely at limits 1, 20 and 100 with no gap or duplicate. The engine's
/// opt-in `engine_search` S-11 test repeats this through the store and
/// measures each page.
#[test]
fn s11_a_20000_segment_revision_pages_completely() -> TestResult {
    let texts: Vec<String> = (1_u32..=20_000)
        .map(|ordinal| {
            if ordinal % 400 == 0 {
                format!("Dialog R-17 marker {ordinal}")
            } else if ordinal % 1_000 == 500 {
                format!("17 then r reversed {ordinal}")
            } else {
                format!("segment {ordinal} of the long synthetic transcript")
            }
        })
        .collect();
    let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
    let session_id = session("0123456789abcdef")?;
    let revision = revision(&borrowed, 1)?;
    let mut expected: Vec<(SearchMatch, u32)> = (1_u32..=20_000)
        .filter(|ordinal| ordinal % 400 == 0)
        .map(|ordinal| (SearchMatch::Phrase, ordinal))
        .collect();
    expected.extend(
        (1_u32..=20_000)
            .filter(|ordinal| ordinal % 1_000 == 500)
            .map(|ordinal| (SearchMatch::AllTerms, ordinal)),
    );
    for limit in [1, 20, 100] {
        let paged = all_pages(&session_id, &revision, request("r 17", limit, None)?)?;
        assert_eq!(paged, expected, "limit {limit}");
    }
    Ok(())
}

const WORDS: [&str; 6] = ["alpha", "beta", "R-17", "r", "17", "gamma"];

proptest! {
    /// C-03: in a fixed revision, following cursors at any limit visits every
    /// hit exactly once, in the order of one unlimited page.
    #[test]
    fn paging_has_no_gaps_or_duplicates(
        texts in vec(vec(select(WORDS.to_vec()), 1..6), 1..40),
        query in vec(select(WORDS.to_vec()), 1..3),
        limit in 1_u16..=100,
        windowed in proptest::bool::ANY,
    ) {
        let texts: Vec<String> = texts.iter().map(|words| words.join(" ")).collect();
        let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
        let (Ok(session_id), Ok(revision), Ok(query)) = (
            session("0123456789abcdef"),
            revision(&borrowed, 1),
            SearchQuery::parse(&query.join(" ")),
        ) else {
            return Err(proptest::test_runner::TestCaseError::fail("setup"));
        };
        let window = if windowed { range(5 * SECOND, 25 * SECOND).ok() } else { None };
        let (Ok(limit), Ok(unlimited)) = (PageLimit::new(limit), PageLimit::new(100)) else {
            return Err(proptest::test_runner::TestCaseError::fail("limit"));
        };
        let paged = all_pages(&session_id, &revision, SearchPageRequest {
            query: query.clone(), range: window, limit, cursor: None,
        });
        let single = page_search(&session_id, &revision, &SearchPageRequest {
            query, range: window, limit: unlimited, cursor: None,
        }, EXPIRES, NOW);
        let (Ok(paged), Ok(single)) = (paged, single) else {
            return Err(proptest::test_runner::TestCaseError::fail("paging failed"));
        };
        let expected: Vec<(SearchMatch, u32)> = single
            .hits
            .iter()
            .map(|hit| (hit.tier(), hit.segment().ordinal()))
            .collect();
        let distinct: BTreeSet<u32> = paged.iter().map(|(_, ordinal)| *ordinal).collect();
        prop_assert_eq!(distinct.len(), paged.len());
        prop_assert_eq!(&paged, &expected);
        let mut sorted = paged.clone();
        sorted.sort_unstable();
        prop_assert!(sorted == paged, "hits are not in rank order");
    }
}
