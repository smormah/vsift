use std::{collections::BTreeSet, num::NonZeroU32};

use vsift_domain::{
    Confidence, ConfidenceOrigin, CursorError, FrameDimensions, FrameRef, MediaTime, PageLimit,
    ProviderLabel, ReadTarget, ScreenText, ScreenTextFrame, ScreenTextIndex, ScreenTextIndexParts,
    ScreenTextLine, ScreenTextOutcome, ScreenTextProfile, ScreenTextProvider, ScreenTextScope,
    SearchQuery, SessionId, Sha256Hex, SourceId, TextRegion, TimeBase, TimeRange,
};

use super::{ScreenTextPageRequest, page_screen_text};
use crate::{screen_text_index_id, screen_text_line_id};

type Built<T> = Result<T, Box<dyn std::error::Error>>;
type TestResult = Built<()>;

const SECOND: u64 = 1_000_000;
const EXPIRES: u64 = 2_000_000_000_000_000;
const NOW: u64 = 1_000_000_000_000_000;

fn session() -> Built<SessionId> {
    Ok(SessionId::parse("ses_0123456789abcdef")?)
}

/// An index of `frames` read frames, one a second, each showing `Invoice <n>`
/// at a place of its own, and `Total due` in one place on every frame.
fn index(frames: i64, revision: u32) -> Built<ScreenTextIndex> {
    let digest = |fill: char| Sha256Hex::parse(fill.to_string().repeat(64));
    let scope = ScreenTextScope {
        session_id: session()?,
        source_id: SourceId::parse(format!("src_sha256_{}", "b".repeat(64)))?,
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
    let mut read = Vec::new();
    for pts in 0..frames {
        let seconds = u64::try_from(pts)?;
        let line = |ordinal: usize, region: (u32, u32), text: String| -> Built<ScreenTextLine> {
            Ok(ScreenTextLine {
                id: screen_text_line_id(&scope, pts, ordinal)?,
                region: TextRegion::new(region.0, region.1, 200, 20, dimensions)?,
                text: ScreenText::parse(text)?,
                confidence: Confidence::provider_score(
                    9_000,
                    ConfidenceOrigin::ProviderUncalibrated,
                )?,
            })
        };
        read.push(ScreenTextFrame {
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
                lines: vec![
                    line(1, (10, 10), String::from("Total due"))?,
                    line(
                        2,
                        (10, 40 + u32::try_from(pts)? * 25),
                        format!("Invoice {}", 4400 + pts),
                    )?,
                ],
                truncated: false,
            },
        });
    }
    let number = NonZeroU32::new(revision).ok_or("revision")?;
    Ok(ScreenTextIndex::new(ScreenTextIndexParts {
        id: screen_text_index_id(&scope, number)?,
        revision: number,
        scope,
        period_micros: SECOND,
        frames: read,
        unresolved: Vec::new(),
    })?)
}

fn request(query: &str, limit: u16, cursor: Option<String>) -> Built<ScreenTextPageRequest> {
    Ok(ScreenTextPageRequest {
        query: SearchQuery::parse(query)?,
        range: None,
        limit: PageLimit::new(limit)?,
        cursor,
    })
}

#[test]
fn pages_cover_every_occurrence_once_in_rank_order() -> TestResult {
    let index = index(12, 1)?;
    let session = session()?;
    let mut cursor = None;
    let mut seen = Vec::new();
    loop {
        let page = page_screen_text(
            &session,
            &index,
            &request("invoice", 5, cursor)?,
            EXPIRES,
            NOW,
        )?;
        assert_eq!(page.total_hits, 12);
        seen.extend(page.hits.iter().map(|hit| hit.first.frame.pts));
        match page.next_cursor {
            Some(next) => cursor = Some(next.encode()),
            None => break,
        }
    }
    assert_eq!(seen, (0..12).collect::<Vec<_>>());
    assert_eq!(seen.iter().collect::<BTreeSet<_>>().len(), 12);
    Ok(())
}

#[test]
fn a_line_held_in_one_place_is_one_occurrence_over_its_frames() -> TestResult {
    let index = index(12, 1)?;
    let page = page_screen_text(
        &session()?,
        &index,
        &request("total due", 20, None)?,
        EXPIRES,
        NOW,
    )?;
    assert_eq!(page.total_hits, 1);
    let hit = page.hits.first().ok_or("no hit")?;
    assert_eq!((hit.frames, hit.last_time.as_micros()), (12, 11 * SECOND));
    assert!(page.next_cursor.is_none());
    Ok(())
}

#[test]
fn a_cursor_of_another_query_range_revision_or_session_is_rejected() -> TestResult {
    let first = index(12, 1)?;
    let session = session()?;
    let page = page_screen_text(
        &session,
        &first,
        &request("invoice", 5, None)?,
        EXPIRES,
        NOW,
    )?;
    let cursor = page.next_cursor.ok_or("no cursor")?.encode();
    let other_query = page_screen_text(
        &session,
        &first,
        &request("total", 5, Some(cursor.clone()))?,
        EXPIRES,
        NOW,
    );
    assert_eq!(other_query.err(), Some(CursorError::WrongQuery));
    let mut ranged = request("invoice", 5, Some(cursor.clone()))?;
    ranged.range = Some(TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(5 * SECOND),
    )?);
    assert_eq!(
        page_screen_text(&session, &first, &ranged, EXPIRES, NOW).err(),
        Some(CursorError::WrongQuery)
    );
    let second = index(12, 2)?;
    assert_eq!(
        page_screen_text(
            &session,
            &second,
            &request("invoice", 5, Some(cursor.clone()))?,
            EXPIRES,
            NOW
        )
        .err(),
        Some(CursorError::WrongGeneration)
    );
    assert_eq!(
        page_screen_text(
            &session,
            &first,
            &request("invoice", 5, Some(cursor))?,
            EXPIRES,
            EXPIRES
        )
        .err(),
        Some(CursorError::Expired)
    );
    Ok(())
}

#[test]
fn a_range_searches_only_its_frames() -> TestResult {
    let index = index(12, 1)?;
    let mut ranged = request("invoice", 20, None)?;
    ranged.range = Some(TimeRange::new(
        MediaTime::from_micros(3 * SECOND),
        MediaTime::from_micros(6 * SECOND),
    )?);
    let page = page_screen_text(&session()?, &index, &ranged, EXPIRES, NOW)?;
    let frames: Vec<i64> = page.hits.iter().map(|hit| hit.first.frame.pts).collect();
    assert_eq!(frames, vec![3, 4, 5]);
    Ok(())
}
