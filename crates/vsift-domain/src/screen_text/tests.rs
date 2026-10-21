use std::num::NonZeroU32;

use super::{
    DEFAULT_READ_PERIOD_MICROS, MAX_SCREEN_TEXT_CHARS, ProviderLabel, ReadTarget, ScreenText,
    ScreenTextCoverage, ScreenTextError, ScreenTextFailure, ScreenTextFrame, ScreenTextIndex,
    ScreenTextIndexParts, ScreenTextLine, ScreenTextOutcome, ScreenTextProfile, ScreenTextProvider,
    ScreenTextScope, TextRegion, plan_reads,
};
use crate::{
    Confidence, ConfidenceOrigin, FrameDimensions, FrameRef, MediaTime, ScreenTextIndexId,
    ScreenTextLineId, SessionId, Sha256Hex, SourceId, TimeBase, TimeRange, VisualCandidateId,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type TestResultOf<T> = Result<T, Box<dyn std::error::Error>>;

const SECOND: u64 = 1_000_000;

fn digest(fill: char) -> TestResultOf<Sha256Hex> {
    Ok(Sha256Hex::parse(fill.to_string().repeat(64))?)
}

fn scope() -> TestResultOf<ScreenTextScope> {
    Ok(ScreenTextScope {
        session_id: SessionId::parse("ses_0123456789abcdef")?,
        source_id: SourceId::parse(format!("src_sha256_{}", "a".repeat(64)))?,
        stream_index: 0,
        profile: ScreenTextProfile::PpOcrV5LatinV1,
        provider: ScreenTextProvider {
            runner_sha256: digest('1')?,
            runner_version: ProviderLabel::parse("0.1.0")?,
            runtime_version: ProviderLabel::parse("1.31.0")?,
            detector_sha256: digest('2')?,
            recognizer_sha256: digest('3')?,
        },
    })
}

fn frame_ref(pts: i64) -> TestResultOf<FrameRef> {
    Ok(FrameRef {
        stream_index: 0,
        time_base: TimeBase::new(1, 30)?,
        pts,
        time: MediaTime::from_micros(u64::try_from(pts)? * SECOND / 30),
        dimensions: FrameDimensions::new(1280, 720)?,
    })
}

fn line(id: &str, text: &str) -> TestResultOf<ScreenTextLine> {
    Ok(ScreenTextLine {
        id: ScreenTextLineId::parse(id)?,
        region: TextRegion::new(10, 20, 300, 18, FrameDimensions::new(1280, 720)?)?,
        text: ScreenText::parse(text)?,
        confidence: Confidence::provider_score(9_850, ConfidenceOrigin::ProviderUncalibrated)?,
    })
}

fn candidate(suffix: char) -> TestResultOf<VisualCandidateId> {
    Ok(VisualCandidateId::parse(format!(
        "vcd_{}",
        suffix.to_string().repeat(32)
    ))?)
}

fn read_frame(
    pts: i64,
    target: ReadTarget,
    lines: Vec<ScreenTextLine>,
) -> TestResultOf<ScreenTextFrame> {
    Ok(ScreenTextFrame {
        frame: frame_ref(pts)?,
        targets: vec![target],
        outcome: ScreenTextOutcome::Read {
            lines,
            truncated: false,
        },
    })
}

fn index(
    frames: Vec<ScreenTextFrame>,
    unresolved: Vec<ReadTarget>,
) -> Result<ScreenTextIndex, ScreenTextError> {
    let scope = scope().map_err(|_| ScreenTextError::InvalidFrames)?;
    ScreenTextIndex::new(ScreenTextIndexParts {
        id: ScreenTextIndexId::parse("sti_0123456789abcdef0123456789abcdef")
            .map_err(|_| ScreenTextError::InvalidFrames)?,
        revision: NonZeroU32::MIN,
        scope,
        period_micros: DEFAULT_READ_PERIOD_MICROS,
        frames,
        unresolved,
    })
}

fn periodic(seconds: u64) -> ReadTarget {
    ReadTarget {
        time: MediaTime::from_micros(seconds * SECOND),
        candidate: None,
    }
}

#[test]
fn text_is_bounded_and_free_of_control_characters() {
    assert!(ScreenText::parse("Total 4,731,145").is_ok());
    assert!(ScreenText::parse("Página principal").is_ok());
    assert_eq!(ScreenText::parse("   "), Err(ScreenTextError::InvalidText));
    assert_eq!(
        ScreenText::parse("a\u{1b}[31m"),
        Err(ScreenTextError::InvalidText)
    );
    assert_eq!(
        ScreenText::parse("x".repeat(MAX_SCREEN_TEXT_CHARS + 1)),
        Err(ScreenTextError::InvalidText)
    );
    assert!(ScreenText::parse("é".repeat(MAX_SCREEN_TEXT_CHARS)).is_ok());
}

#[test]
fn a_region_must_lie_inside_its_frame() -> TestResult {
    let frame = FrameDimensions::new(100, 50)?;
    assert!(TextRegion::new(0, 0, 100, 50, frame).is_ok());
    assert!(TextRegion::new(1, 0, 100, 50, frame).is_err());
    assert!(TextRegion::new(0, 0, 0, 5, frame).is_err());
    assert!(TextRegion::new(u32::MAX, 0, 2, 5, frame).is_err());
    Ok(())
}

#[test]
fn two_regions_are_one_place_when_half_the_smaller_overlaps() -> TestResult {
    let frame = FrameDimensions::new(1280, 720)?;
    let label = TextRegion::new(100, 100, 200, 20, frame)?;
    assert!(label.same_place(TextRegion::new(103, 101, 198, 20, frame)?));
    assert!(!label.same_place(TextRegion::new(100, 125, 200, 20, frame)?));
    assert!(!label.same_place(TextRegion::new(250, 100, 200, 20, frame)?));
    Ok(())
}

#[test]
fn labels_are_short_and_plain() {
    assert!(ProviderLabel::parse("1.31.0").is_ok());
    assert!(ProviderLabel::parse("0.1.0+build.7").is_ok());
    assert!(ProviderLabel::parse("").is_err());
    assert!(ProviderLabel::parse("1.0 beta").is_err());
    assert!(ProviderLabel::parse("x".repeat(65)).is_err());
}

#[test]
fn a_plan_is_every_candidate_in_range_and_one_frame_a_period() -> TestResult {
    let range = TimeRange::new(
        MediaTime::from_micros(5 * SECOND),
        MediaTime::from_micros(31 * SECOND),
    )?;
    let candidates = [
        (candidate('a')?, MediaTime::from_micros(2 * SECOND)),
        (candidate('b')?, MediaTime::from_micros(12_500_000)),
        (candidate('c')?, MediaTime::from_micros(31 * SECOND)),
    ];
    let plan = plan_reads(range, &candidates, DEFAULT_READ_PERIOD_MICROS)?;
    let times: Vec<u64> = plan.iter().map(|target| target.time.as_micros()).collect();
    assert_eq!(
        times,
        vec![10 * SECOND, 12_500_000, 20 * SECOND, 30 * SECOND]
    );
    assert_eq!(
        plan.get(1).and_then(|target| target.candidate.clone()),
        Some(candidate('b')?)
    );
    assert!(plan_reads(range, &candidates, 999_999).is_err());
    assert!(plan_reads(range, &candidates, 600_000_001).is_err());
    Ok(())
}

#[test]
fn a_candidate_and_a_periodic_target_at_one_time_are_two_targets() -> TestResult {
    let range = TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(11 * SECOND),
    )?;
    let plan = plan_reads(
        range,
        &[(candidate('a')?, MediaTime::from_micros(10 * SECOND))],
        DEFAULT_READ_PERIOD_MICROS,
    )?;
    assert_eq!(plan.len(), 3);
    Ok(())
}

#[test]
fn an_index_keeps_frames_in_timestamp_order_with_unique_lines() -> TestResult {
    let first = read_frame(
        30,
        periodic(1),
        vec![line("stl_0000000000000001", "Watchlist")?],
    )?;
    let second = read_frame(60, periodic(2), vec![line("stl_0000000000000002", "Talk")?])?;
    assert!(index(vec![first.clone(), second.clone()], Vec::new()).is_ok());
    assert_eq!(
        index(vec![second.clone(), first.clone()], Vec::new()),
        Err(ScreenTextError::InvalidFrames)
    );
    let mut repeated = second.clone();
    repeated.outcome = first.outcome.clone();
    assert_eq!(
        index(vec![first.clone(), repeated], Vec::new()),
        Err(ScreenTextError::DuplicateLine)
    );
    let mut untargeted = second;
    untargeted.targets.clear();
    assert_eq!(
        index(vec![first, untargeted], Vec::new()),
        Err(ScreenTextError::InvalidFrames)
    );
    Ok(())
}

#[test]
fn coverage_counts_read_failed_unresolved_and_pending_targets() -> TestResult {
    let range = TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(40 * SECOND),
    )?;
    let plan = plan_reads(range, &[], DEFAULT_READ_PERIOD_MICROS)?;
    assert_eq!(
        ScreenTextCoverage::of(&plan, None),
        ScreenTextCoverage {
            planned: 4,
            pending: 4,
            ..ScreenTextCoverage::default()
        }
    );
    let read = read_frame(
        1,
        periodic(0),
        vec![line("stl_0000000000000001", "Search")?],
    )?;
    let failed = ScreenTextFrame {
        frame: frame_ref(300)?,
        targets: vec![periodic(10)],
        outcome: ScreenTextOutcome::Failed(ScreenTextFailure::ImageRejected),
    };
    let built = index(vec![read, failed], vec![periodic(30)])?;
    let coverage = ScreenTextCoverage::of(&plan, Some(&built));
    assert_eq!(
        coverage,
        ScreenTextCoverage {
            planned: 4,
            read: 1,
            failed: 1,
            unresolved: 1,
            pending: 1
        }
    );
    assert!(!coverage.is_complete());
    assert!(
        built.attempted(&periodic(30))
            && built.attempted(&periodic(10))
            && !built.attempted(&periodic(20))
    );
    assert_eq!(
        built
            .frame_at_pts(300)
            .map(ScreenTextFrame::lines)
            .map(<[ScreenTextLine]>::len),
        Some(0)
    );
    Ok(())
}

#[test]
fn profiles_round_trip_their_identifiers() {
    for profile in ScreenTextProfile::ALL {
        assert_eq!(
            ScreenTextProfile::from_identifier(profile.identifier()),
            Some(profile)
        );
    }
    assert_eq!(ScreenTextProfile::from_identifier("tesseract"), None);
}

mod search {
    use super::{TestResult, frame_ref, index, line, periodic, read_frame};
    use crate::{
        FrameDimensions, MediaTime, PageLimit, ScreenText, ScreenTextFailure, ScreenTextFrame,
        ScreenTextLine, ScreenTextLineId, ScreenTextOutcome, SearchMatch, SearchQuery, TextRegion,
        TimeRange, search_screen_text,
    };

    fn placed(id: &str, text: &str, y: u32) -> Result<ScreenTextLine, Box<dyn std::error::Error>> {
        let mut placed = line(id, text)?;
        placed.region = TextRegion::new(10, y, 300, 18, FrameDimensions::new(1280, 720)?)?;
        placed.text = ScreenText::parse(text)?;
        placed.id = ScreenTextLineId::parse(id)?;
        Ok(placed)
    }

    #[test]
    fn a_label_on_consecutive_frames_is_one_occurrence_with_its_span() -> TestResult {
        let frames = vec![
            read_frame(
                30,
                periodic(1),
                vec![placed("stl_0000000000000001", "Total population", 20)?],
            )?,
            ScreenTextFrame {
                frame: frame_ref(45)?,
                targets: vec![periodic(2)],
                outcome: ScreenTextOutcome::Failed(ScreenTextFailure::EngineFailed),
            },
            read_frame(
                60,
                periodic(3),
                vec![placed("stl_0000000000000002", "total  Population", 22)?],
            )?,
            read_frame(
                90,
                periodic(4),
                vec![placed("stl_0000000000000003", "Search", 20)?],
            )?,
            read_frame(
                120,
                periodic(5),
                vec![placed("stl_0000000000000004", "Total population", 20)?],
            )?,
        ];
        let built = index(frames, Vec::new())?;
        let slice = search_screen_text(
            &built,
            &SearchQuery::parse("total population")?,
            None,
            None,
            PageLimit::DEFAULT,
        );
        assert_eq!(
            slice.hits.len(),
            2,
            "the label disappears for one read frame, so it occurs twice"
        );
        let first = slice.hits.first().ok_or("no hit")?;
        assert_eq!(
            (first.first.frame.pts, first.frames, first.last_time),
            (30, 2, MediaTime::from_micros(2_000_000))
        );
        assert_eq!(first.tier, SearchMatch::Phrase);
        assert!(!slice.has_more);
        Ok(())
    }

    #[test]
    fn the_same_words_elsewhere_on_the_frame_are_another_occurrence() -> TestResult {
        let frames = vec![
            read_frame(
                30,
                periodic(1),
                vec![placed("stl_0000000000000001", "Download", 20)?],
            )?,
            read_frame(
                60,
                periodic(2),
                vec![placed("stl_0000000000000002", "Download", 400)?],
            )?,
        ];
        let built = index(frames, Vec::new())?;
        let slice = search_screen_text(
            &built,
            &SearchQuery::parse("download")?,
            None,
            None,
            PageLimit::DEFAULT,
        );
        assert_eq!(slice.hits.len(), 2);
        Ok(())
    }

    #[test]
    fn hits_rank_by_tier_page_by_position_and_respect_the_window() -> TestResult {
        let frames = vec![
            read_frame(
                30,
                periodic(1),
                vec![placed("stl_0000000000000001", "population total", 20)?],
            )?,
            read_frame(
                60,
                periodic(2),
                vec![placed("stl_0000000000000002", "total population", 300)?],
            )?,
            read_frame(
                90,
                periodic(3),
                vec![placed("stl_0000000000000003", "total population x", 500)?],
            )?,
        ];
        let built = index(frames, Vec::new())?;
        let query = SearchQuery::parse("total population")?;
        let page = search_screen_text(&built, &query, None, None, PageLimit::new(1)?);
        let first = page.hits.first().ok_or("no hit")?;
        assert_eq!(
            (first.tier, first.first.frame.pts),
            (SearchMatch::Phrase, 60)
        );
        assert!(page.has_more);
        let next = search_screen_text(
            &built,
            &query,
            None,
            Some(first.position()),
            PageLimit::new(5)?,
        );
        let ranked: Vec<(SearchMatch, i64)> = next
            .hits
            .iter()
            .map(|hit| (hit.tier, hit.first.frame.pts))
            .collect();
        assert_eq!(
            ranked,
            vec![(SearchMatch::Phrase, 90), (SearchMatch::AllTerms, 30)]
        );
        let window = TimeRange::new(
            MediaTime::from_micros(1_500_000),
            MediaTime::from_micros(2_500_000),
        )?;
        let windowed = search_screen_text(&built, &query, Some(window), None, PageLimit::DEFAULT);
        assert_eq!(
            windowed
                .hits
                .iter()
                .map(|hit| hit.first.frame.pts)
                .collect::<Vec<_>>(),
            vec![60]
        );
        Ok(())
    }
}
