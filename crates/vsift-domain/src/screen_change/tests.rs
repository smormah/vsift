use super::{
    ChangeRegion, FrameComparison, MAX_CHANGE_REGIONS, MotionPart, RegionComparison, RgbPixels,
    ScreenChangeError, ScreenChangeKind, compare_frames, window_changes,
};
use crate::{
    CandidateChange, CandidateDraft, CandidateReason, CandidateStability, FrameDimensions,
    MediaTime, TimeRange, VisualCandidate, VisualCandidateId, VisualDelta, VisualHash,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A grey frame of `width` x `height` at `level`.
fn frame(width: u32, height: u32, level: u8) -> Vec<u8> {
    vec![level; usize::try_from(width * height * 3).unwrap_or(0)]
}

/// Sets every channel of the pixels of `rect` (x, y, width, height) to `level`.
fn paint(bytes: &mut [u8], width: u32, rect: (u32, u32, u32, u32), level: u8) {
    let (x0, y0, w, h) = rect;
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let start = usize::try_from((y * width + x) * 3).unwrap_or(usize::MAX);
            if let Some(pixel) = bytes.get_mut(start..start + 3) {
                pixel.fill(level);
            }
        }
    }
}

fn compare(
    width: u32,
    height: u32,
    before: &[u8],
    after: &[u8],
) -> Result<FrameComparison, Box<dyn std::error::Error>> {
    let dimensions = FrameDimensions::new(width, height)?;
    Ok(compare_frames(
        RgbPixels::new(dimensions, before)?,
        RgbPixels::new(dimensions, after)?,
    ))
}

fn compared(
    width: u32,
    height: u32,
    before: &[u8],
    after: &[u8],
) -> Result<RegionComparison, Box<dyn std::error::Error>> {
    match compare(width, height, before, after)? {
        FrameComparison::Compared(regions) => Ok(regions),
        FrameComparison::DimensionsDiffer => Err("frames of one size were not compared".into()),
    }
}

fn rect(region: &ChangeRegion) -> (u32, u32, u32, u32) {
    (
        region.rect.x(),
        region.rect.y(),
        region.rect.width(),
        region.rect.height(),
    )
}

#[test]
fn identical_frames_have_no_region() -> TestResult {
    let pixels = frame(64, 48, 100);
    let result = compared(64, 48, &pixels, &pixels)?;
    assert_eq!(result.regions, Vec::new());
    assert_eq!(result.changed_area, None);
    assert_eq!(result.regions_left_out, 0);
    Ok(())
}

#[test]
fn a_channel_must_move_by_the_threshold() -> TestResult {
    let before = frame(32, 32, 100);
    let mut below = before.clone();
    paint(&mut below, 32, (0, 0, 8, 8), 131);
    assert_eq!(compared(32, 32, &before, &below)?.changed_area, None);
    let mut at = before.clone();
    paint(&mut at, 32, (0, 0, 8, 8), 132);
    assert_eq!(compared(32, 32, &before, &at)?.regions.len(), 1);
    // One channel alone is enough.
    let mut red = before.clone();
    for pixel in red.as_chunks_mut::<3>().0.iter_mut().take(4) {
        pixel[0] = 200;
    }
    assert_eq!(compared(32, 32, &before, &red)?.regions.len(), 1);
    Ok(())
}

#[test]
fn a_tile_needs_four_changed_pixels() -> TestResult {
    let before = frame(32, 32, 0);
    let mut three = before.clone();
    paint(&mut three, 32, (0, 0, 3, 1), 255);
    assert_eq!(compared(32, 32, &before, &three)?.changed_area, None);
    let mut four = before.clone();
    paint(&mut four, 32, (0, 0, 4, 1), 255);
    let result = compared(32, 32, &before, &four)?;
    assert_eq!(result.regions.len(), 1);
    assert_eq!(result.regions.first().map(rect), Some((0, 0, 16, 16)));
    assert_eq!(
        result.regions.first().map(|region| region.changed_pixels),
        Some(4)
    );
    Ok(())
}

#[test]
fn regions_are_tile_aligned_and_clipped_to_the_frame() -> TestResult {
    // 40x20: the last tile column and row are 8 and 4 pixels.
    let before = frame(40, 20, 0);
    let mut after = before.clone();
    paint(&mut after, 40, (34, 17, 6, 3), 255);
    let result = compared(40, 20, &before, &after)?;
    assert_eq!(result.regions.first().map(rect), Some((32, 16, 8, 4)));
    assert_eq!(result.changed_area.as_ref().map(rect), Some((32, 16, 8, 4)));
    Ok(())
}

#[test]
fn tiles_one_tile_apart_join_and_two_apart_do_not() -> TestResult {
    let before = frame(128, 16, 0);
    let mut joined = before.clone();
    paint(&mut joined, 128, (0, 0, 4, 4), 255);
    paint(&mut joined, 128, (32, 0, 4, 4), 255);
    let result = compared(128, 16, &before, &joined)?;
    assert_eq!(result.regions.len(), 1);
    assert_eq!(result.regions.first().map(rect), Some((0, 0, 48, 16)));
    let mut apart = before.clone();
    paint(&mut apart, 128, (0, 0, 4, 4), 255);
    paint(&mut apart, 128, (48, 0, 4, 4), 255);
    let result = compared(128, 16, &before, &apart)?;
    assert_eq!(result.regions.len(), 2);
    assert_eq!(result.changed_area.as_ref().map(rect), Some((0, 0, 64, 16)));
    assert_eq!(
        result.changed_area.as_ref().map(|area| area.changed_pixels),
        Some(32)
    );
    Ok(())
}

#[test]
fn diagonal_tiles_join() -> TestResult {
    let before = frame(32, 32, 0);
    let mut after = before.clone();
    paint(&mut after, 32, (12, 12, 4, 4), 255);
    paint(&mut after, 32, (16, 16, 4, 4), 255);
    let result = compared(32, 32, &before, &after)?;
    assert_eq!(result.regions.first().map(rect), Some((0, 0, 32, 32)));
    assert_eq!(result.regions.len(), 1);
    Ok(())
}

#[test]
fn only_the_largest_regions_are_kept_and_the_rest_counted() -> TestResult {
    // Twelve separate tiles, three tiles apart, each with a different size.
    let width = 48 * 12;
    let before = frame(width, 16, 0);
    let mut after = before.clone();
    for index in 0..12 {
        paint(&mut after, width, (index * 48, 0, 4, 1 + index), 255);
    }
    let result = compared(width, 16, &before, &after)?;
    assert_eq!(result.regions.len(), MAX_CHANGE_REGIONS);
    assert_eq!(result.regions_left_out, 4);
    let sizes: Vec<u64> = result
        .regions
        .iter()
        .map(|region| region.changed_pixels)
        .collect();
    assert_eq!(sizes, vec![48, 44, 40, 36, 32, 28, 24, 20]);
    let total: u64 = (0..12_u64).map(|index| 4 * (1 + index)).sum();
    assert_eq!(
        result.changed_area.as_ref().map(|area| area.changed_pixels),
        Some(total)
    );
    Ok(())
}

#[test]
fn equal_regions_are_ordered_top_then_left() -> TestResult {
    let before = frame(96, 96, 0);
    let mut after = before.clone();
    paint(&mut after, 96, (64, 0, 4, 1), 255);
    paint(&mut after, 96, (0, 64, 4, 1), 255);
    paint(&mut after, 96, (0, 0, 4, 1), 255);
    let result = compared(96, 96, &before, &after)?;
    let corners: Vec<(u32, u32)> = result
        .regions
        .iter()
        .map(|region| (region.rect.x(), region.rect.y()))
        .collect();
    assert_eq!(corners, vec![(0, 0), (64, 0), (0, 64)]);
    Ok(())
}

#[test]
fn frames_of_different_sizes_are_not_compared() -> TestResult {
    let small = frame(16, 16, 0);
    let large = frame(32, 16, 0);
    let result = compare_frames(
        RgbPixels::new(FrameDimensions::new(16, 16)?, &small)?,
        RgbPixels::new(FrameDimensions::new(32, 16)?, &large)?,
    );
    assert_eq!(result, FrameComparison::DimensionsDiffer);
    Ok(())
}

#[test]
fn a_buffer_must_match_its_dimensions() -> TestResult {
    let pixels = frame(16, 16, 0);
    let dimensions = FrameDimensions::new(16, 15)?;
    assert_eq!(
        RgbPixels::new(dimensions, &pixels).err(),
        Some(ScreenChangeError::PixelBufferSize)
    );
    Ok(())
}

fn time(seconds_tenths: u64) -> MediaTime {
    MediaTime::from_micros(seconds_tenths * 100_000)
}

fn candidate(
    serial: u32,
    reason: CandidateReason,
    at: u64,
    previous: Option<u64>,
) -> Result<VisualCandidate, Box<dyn std::error::Error>> {
    let change = previous
        .map(|before| {
            VisualDelta::new(3, 20).map(|delta| CandidateChange::new(time(before), delta))
        })
        .transpose()?;
    Ok(VisualCandidate::new(
        VisualCandidateId::parse(format!("vcd_{serial:032}"))?,
        CandidateDraft {
            reason,
            representative: time(at),
            span: TimeRange::new(time(at), time(at + 5))?,
            sample_count: 1,
            stability: CandidateStability::Settled,
            change,
            hash: VisualHash::from_bits(u64::from(serial)),
        },
    ))
}

fn ids(serials: &[u32]) -> Result<Vec<VisualCandidateId>, Box<dyn std::error::Error>> {
    serials
        .iter()
        .map(|serial| Ok(VisualCandidateId::parse(format!("vcd_{serial:032}"))?))
        .collect()
}

#[test]
fn first_frames_and_coverage_are_not_changes_and_a_change_is_one() -> TestResult {
    let candidates = vec![
        candidate(1, CandidateReason::FirstFrame, 0, None)?,
        candidate(2, CandidateReason::VisualChange, 20, Some(15))?,
        candidate(3, CandidateReason::PeriodicCoverage, 100, None)?,
    ];
    let changes = window_changes(0, &candidates, true);
    assert_eq!(changes.len(), 1);
    let change = changes.first().ok_or("one change")?;
    assert_eq!(change.kind, ScreenChangeKind::Change);
    assert_eq!(change.candidates, ids(&[2])?);
    assert_eq!((change.before, change.after), (time(15), time(20)));
    Ok(())
}

#[test]
fn a_motion_run_is_one_change_across_coverage_inside_it() -> TestResult {
    let candidates = vec![
        candidate(1, CandidateReason::FirstFrame, 0, None)?,
        candidate(2, CandidateReason::MotionStart, 20, Some(15))?,
        candidate(3, CandidateReason::PeriodicCoverage, 100, None)?,
        candidate(4, CandidateReason::SettledAfterMotion, 150, Some(145))?,
        candidate(5, CandidateReason::VisualChange, 300, Some(295))?,
    ];
    let changes = window_changes(2, &candidates, true);
    let kinds: Vec<ScreenChangeKind> = changes.iter().map(|change| change.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ScreenChangeKind::Motion(MotionPart::Run),
            ScreenChangeKind::Change
        ]
    );
    let run = changes.first().ok_or("a run")?;
    assert_eq!(run.candidates, ids(&[2, 4])?);
    assert_eq!((run.before, run.after), (time(15), time(150)));
    assert_eq!(run.window_ordinal, 2);
    Ok(())
}

#[test]
fn a_run_still_moving_at_the_window_end_is_its_start_alone() -> TestResult {
    let candidates = vec![
        candidate(1, CandidateReason::FirstFrame, 0, None)?,
        candidate(2, CandidateReason::MotionStart, 500, Some(495))?,
    ];
    let changes = window_changes(0, &candidates, true);
    let change = changes.first().ok_or("one change")?;
    assert_eq!(change.kind, ScreenChangeKind::Motion(MotionPart::Start));
    assert!(!MotionPart::Start.settled());
    assert_eq!((change.before, change.after), (time(495), time(500)));
    Ok(())
}

#[test]
fn a_window_that_dropped_candidates_never_pairs_a_run() -> TestResult {
    // The budget kept a start and a settled candidate, which may belong to
    // two different runs; each is a change of its own.
    let candidates = vec![
        candidate(1, CandidateReason::FirstFrame, 0, None)?,
        candidate(2, CandidateReason::MotionStart, 20, Some(15))?,
        candidate(4, CandidateReason::SettledAfterMotion, 150, Some(145))?,
    ];
    let changes = window_changes(0, &candidates, false);
    let kinds: Vec<ScreenChangeKind> = changes.iter().map(|change| change.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ScreenChangeKind::Motion(MotionPart::Start),
            ScreenChangeKind::Motion(MotionPart::End)
        ]
    );
    let end = changes.get(1).ok_or("the end")?;
    assert_eq!((end.before, end.after), (time(145), time(150)));
    assert!(MotionPart::End.settled());
    Ok(())
}

#[test]
fn a_settled_candidate_whose_start_was_dropped_stands_alone() -> TestResult {
    let candidates = vec![
        candidate(1, CandidateReason::FirstFrame, 0, None)?,
        candidate(4, CandidateReason::SettledAfterMotion, 150, Some(145))?,
    ];
    let changes = window_changes(0, &candidates, false);
    assert_eq!(
        changes.first().map(|change| change.kind),
        Some(ScreenChangeKind::Motion(MotionPart::End))
    );
    Ok(())
}
