use proptest::prelude::{ProptestConfig, prop, prop_assert, prop_assert_eq, proptest};

use super::{
    CandidateReason, CandidateStability, MAX_WINDOW_CANDIDATES, VISUAL_BLOCKS, VISUAL_FRAME_BYTES,
    VisualCandidate, VisualChangePolicy, VisualHash, VisualIndexError, VisualIndexWindow,
    VisualSample, VisualWindow, VisualWindowOutcome, WindowAnalysis, analyse_window,
    visual_window_count,
};
use crate::{MediaTime, VisualCandidateId};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const POLICY: VisualChangePolicy = VisualChangePolicy::R0;

fn seconds(value: f64) -> MediaTime {
    // Test inputs are small, exact multiples of a microsecond.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "test times are small positive values"
    )]
    let micros = (value * 1_000_000.0).round() as u64;
    MediaTime::from_micros(micros)
}

/// A sample whose every block has `level`, except block `marker` at `marker_level`.
fn screen(time: MediaTime, level: u8, marker: usize, marker_level: u8) -> VisualSample {
    let mut blocks = [level; VISUAL_BLOCKS];
    if let Some(block) = blocks.get_mut(marker) {
        *block = marker_level;
    }
    let hash = VisualHash::from_bits(u64::from(level) << 8 | u64::from(marker_level));
    VisualSample::from_parts(time, blocks, hash)
}

fn uniform(time: MediaTime, level: u8) -> VisualSample {
    screen(time, level, 0, level)
}

fn window(ordinal: u32, duration_seconds: f64) -> Result<VisualWindow, VisualIndexError> {
    VisualWindow::new(ordinal, seconds(duration_seconds))
}

/// Samples at 0.5 s from `from` (inclusive) to `to` (exclusive), each drawn by `draw`.
fn grid(from: f64, to: f64, draw: impl Fn(f64) -> (u8, u8)) -> Vec<VisualSample> {
    let mut samples = Vec::new();
    let mut step = 0_u32;
    loop {
        let time = from + f64::from(step) * 0.5;
        if time >= to - 1e-9 {
            break;
        }
        let (level, marker) = draw(time);
        samples.push(screen(seconds(time), level, 7, marker));
        step += 1;
    }
    samples
}

fn identify(analysis: &WindowAnalysis) -> Result<VisualIndexWindow, Box<dyn std::error::Error>> {
    let mut candidates = Vec::new();
    for (position, draft) in analysis.candidates.iter().enumerate() {
        let id = VisualCandidateId::parse(format!(
            "vcd_{:016x}{:016x}",
            analysis.window.ordinal(),
            position
        ))?;
        candidates.push(VisualCandidate::new(id, *draft));
    }
    Ok(VisualIndexWindow::new(
        analysis.window,
        VisualWindowOutcome::Analysed {
            sample_count: analysis.sample_count,
            candidates,
            dropped_candidates: analysis.dropped_candidates,
            frameless_cells: analysis.frameless_cells.clone(),
        },
        POLICY,
    )?)
}

fn reasons(analysis: &WindowAnalysis) -> Vec<(u64, CandidateReason, CandidateStability)> {
    analysis
        .candidates
        .iter()
        .map(|draft| {
            (
                draft.representative.as_micros(),
                draft.reason,
                draft.stability,
            )
        })
        .collect()
}

#[test]
fn a_uniform_frame_has_uniform_blocks_and_a_zero_hash() {
    let sample = VisualSample::from_gray(MediaTime::from_micros(5), &[77; VISUAL_FRAME_BYTES]);
    assert!(sample.blocks().iter().all(|block| *block == 77));
    assert_eq!(sample.hash(), VisualHash::from_bits(0));
    assert_eq!(sample.hash().to_hex(), "0000000000000000");
    assert_eq!(sample.time(), MediaTime::from_micros(5));
}

#[test]
fn block_means_round_half_up_and_the_hash_marks_brighter_left_cells() -> TestResult {
    let mut pixels = [0_u8; VISUAL_FRAME_BYTES];
    // Top-left block: 32 of its 64 pixels are 1, so the mean 0.5 rounds to 1.
    for row in 0..4 {
        for column in 0..8 {
            if let Some(pixel) = pixels.get_mut(row * 128 + column) {
                *pixel = 1;
            }
        }
    }
    // Left 14 columns bright: the first hash cell of every row is brighter
    // than the second, so bit 7 of every row byte is set.
    for row in 0..72 {
        for column in 0..14 {
            if let Some(pixel) = pixels.get_mut(row * 128 + column) {
                *pixel = pixel.saturating_add(200);
            }
        }
    }
    let sample = VisualSample::from_gray(MediaTime::from_micros(0), &pixels);
    // 64 pixels at 200 plus 32 at 1: 12,832 / 64 = 200.5, rounded up.
    assert_eq!(sample.blocks().first(), Some(&201));
    // Columns 8..13 bright, 14..15 dark: 48 x 200 / 64 = 150.
    assert_eq!(sample.blocks().get(1), Some(&150));
    assert_eq!(sample.blocks().get(2), Some(&0));
    assert_eq!(sample.hash().to_hex(), "8080808080808080");
    assert_eq!(VisualHash::parse_hex("8080808080808080")?, sample.hash());
    assert!(VisualHash::parse_hex("8080808080808O80").is_err());
    assert!(VisualHash::parse_hex("80808080808080").is_err());
    Ok(())
}

#[test]
fn the_r0_policy_needs_two_blocks_at_four_or_one_at_six() {
    let base = uniform(MediaTime::from_micros(0), 100);
    let noise = screen(MediaTime::from_micros(1), 100, 3, 101);
    let one_small = screen(MediaTime::from_micros(1), 100, 3, 105);
    let one_large = screen(MediaTime::from_micros(1), 100, 3, 106);
    let mut two = [100_u8; VISUAL_BLOCKS];
    two[0] = 104;
    two[1] = 96;
    let two = VisualSample::from_parts(MediaTime::from_micros(1), two, VisualHash::from_bits(1));
    assert!(!POLICY.is_change(POLICY.delta(&base, &noise)));
    assert!(!POLICY.is_change(POLICY.delta(&base, &one_small)));
    assert!(POLICY.is_change(POLICY.delta(&base, &one_large)));
    let delta = POLICY.delta(&base, &two);
    assert_eq!((delta.changed_blocks(), delta.max_block_delta()), (2, 4));
    assert!(POLICY.is_change(delta));
}

#[test]
fn windows_follow_the_fixed_grid_and_cells_follow_source_time() -> TestResult {
    assert_eq!(visual_window_count(seconds(61.0)), 2);
    assert_eq!(visual_window_count(seconds(60.0)), 1);
    let last = window(1, 61.0)?;
    assert_eq!(last.range().start(), seconds(60.0));
    assert_eq!(last.range().end(), seconds(61.0));
    assert_eq!(last.lead_in_start(), seconds(59.5));
    assert_eq!(last.cells().len(), 1);
    assert_eq!(window(0, 25.0)?.cells().len(), 3);
    assert_eq!(window(0, 25.0)?.lead_in_start(), seconds(0.0));
    assert!(window(2, 61.0).is_err());
    assert!(VisualWindow::new(0, MediaTime::from_micros(0)).is_err());
    Ok(())
}

#[test]
fn a_static_screen_gets_a_first_frame_and_one_candidate_per_ten_seconds() -> TestResult {
    let analysis = analyse_window(window(0, 25.0)?, &grid(0.0, 25.0, |_| (40, 40)), POLICY)?;
    assert_eq!(
        reasons(&analysis),
        vec![
            (0, CandidateReason::FirstFrame, CandidateStability::Settled),
            (
                10_000_000,
                CandidateReason::PeriodicCoverage,
                CandidateStability::Settled
            ),
            (
                20_000_000,
                CandidateReason::PeriodicCoverage,
                CandidateStability::Settled
            ),
        ]
    );
    assert_eq!(analysis.sample_count, 50);
    assert_eq!(
        analysis
            .candidates
            .iter()
            .map(|draft| draft.sample_count)
            .collect::<Vec<_>>(),
        vec![20, 20, 10]
    );
    assert!(
        analysis
            .candidates
            .iter()
            .all(|draft| draft.change.is_none())
    );
    identify(&analysis)?;
    Ok(())
}

#[test]
fn a_repeated_screen_is_a_separate_candidate_with_the_same_hash() -> TestResult {
    // F10's shape: WAITING, then a dialog from 5 s, WAITING again from 9 s.
    let analysis = analyse_window(
        window(0, 12.0)?,
        &grid(0.0, 12.0, |time| {
            if (5.0..9.0).contains(&time) {
                (40, 90)
            } else {
                (40, 40)
            }
        }),
        POLICY,
    )?;
    let times: Vec<u64> = analysis
        .candidates
        .iter()
        .map(|draft| draft.representative.as_micros())
        .collect();
    assert_eq!(times, vec![0, 5_000_000, 9_000_000, 10_000_000]);
    let hashes: Vec<VisualHash> = analysis.candidates.iter().map(|draft| draft.hash).collect();
    assert_eq!(hashes.first(), hashes.get(2));
    assert_ne!(hashes.first(), hashes.get(1));
    let change = analysis
        .candidates
        .get(1)
        .and_then(|draft| draft.change)
        .ok_or("change missing")?;
    assert_eq!(change.previous_sample(), seconds(4.5));
    assert_eq!(change.delta().max_block_delta(), 50);
    identify(&analysis)?;
    Ok(())
}

#[test]
fn a_screen_seen_in_one_sample_is_transient() -> TestResult {
    // F06's shape: a tooltip visible for one sample at 4.5 s.
    let analysis = analyse_window(
        window(0, 10.0)?,
        &grid(0.0, 10.0, |time| {
            if (4.4..4.6).contains(&time) {
                (40, 200)
            } else {
                (40, 40)
            }
        }),
        POLICY,
    )?;
    assert_eq!(
        reasons(&analysis),
        vec![
            (0, CandidateReason::FirstFrame, CandidateStability::Settled),
            (
                4_500_000,
                CandidateReason::VisualChange,
                CandidateStability::Transient
            ),
            (
                5_000_000,
                CandidateReason::VisualChange,
                CandidateStability::Settled
            ),
        ]
    );
    identify(&analysis)?;
    Ok(())
}

#[test]
fn consecutive_changes_collapse_into_motion_then_settle() -> TestResult {
    // Scrolling from 3 s to 5 s: every sample differs from the one before.
    let analysis = analyse_window(
        window(0, 10.0)?,
        &grid(0.0, 10.0, |time| {
            if time < 3.0 {
                (40, 40)
            } else if time < 5.0 {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "small test levels"
                )]
                let level = (60.0 + (time - 3.0) * 40.0) as u8;
                (level, level)
            } else {
                (160, 160)
            }
        }),
        POLICY,
    )?;
    assert_eq!(
        reasons(&analysis),
        vec![
            (0, CandidateReason::FirstFrame, CandidateStability::Settled),
            (
                3_000_000,
                CandidateReason::MotionStart,
                CandidateStability::InMotion
            ),
            (
                5_000_000,
                CandidateReason::SettledAfterMotion,
                CandidateStability::Settled
            ),
        ]
    );
    assert_eq!(
        analysis.candidates.get(1).map(|draft| draft.sample_count),
        Some(4)
    );
    identify(&analysis)?;
    Ok(())
}

#[test]
fn motion_until_the_window_end_stays_in_motion() -> TestResult {
    let analysis = analyse_window(
        window(0, 4.0)?,
        &grid(0.0, 4.0, |time| {
            if time < 2.0 {
                (40, 40)
            } else {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "small test levels"
                )]
                let level = (60.0 + time * 20.0) as u8;
                (level, level)
            }
        }),
        POLICY,
    )?;
    assert_eq!(
        reasons(&analysis),
        vec![
            (0, CandidateReason::FirstFrame, CandidateStability::Settled),
            (
                2_000_000,
                CandidateReason::MotionStart,
                CandidateStability::InMotion
            ),
        ]
    );
    identify(&analysis)?;
    Ok(())
}

#[test]
fn a_lead_in_sample_decides_how_a_later_window_opens() -> TestResult {
    let later = window(1, 90.0)?;
    let mut unchanged = vec![uniform(seconds(59.5), 40)];
    unchanged.extend(grid(60.0, 90.0, |_| (40, 40)));
    let analysis = analyse_window(later, &unchanged, POLICY)?;
    assert_eq!(
        analysis
            .candidates
            .first()
            .map(|draft| (draft.reason, draft.change)),
        Some((CandidateReason::PeriodicCoverage, None))
    );
    assert_eq!(analysis.sample_count, 60);
    identify(&analysis)?;

    let mut changed = vec![uniform(seconds(59.5), 90)];
    changed.extend(grid(60.0, 90.0, |_| (40, 40)));
    let analysis = analyse_window(later, &changed, POLICY)?;
    let first = analysis.candidates.first().ok_or("no candidate")?;
    assert_eq!(first.reason, CandidateReason::VisualChange);
    assert_eq!(
        first.change.map(super::CandidateChange::previous_sample),
        Some(seconds(59.5))
    );
    identify(&analysis)?;
    Ok(())
}

#[test]
fn cells_without_frames_are_gaps_and_an_empty_window_is_all_gaps() -> TestResult {
    let analysis = analyse_window(window(0, 60.0)?, &grid(0.0, 10.0, |_| (40, 40)), POLICY)?;
    assert_eq!(analysis.frameless_cells.len(), 5);
    assert_eq!(analysis.candidates.len(), 1);
    identify(&analysis)?;

    let empty = analyse_window(window(0, 30.0)?, &[], POLICY)?;
    assert_eq!(empty.sample_count, 0);
    assert!(empty.candidates.is_empty());
    assert_eq!(empty.frameless_cells.len(), 3);
    identify(&empty)?;
    Ok(())
}

#[test]
fn the_budget_keeps_coverage_and_the_largest_changes() -> TestResult {
    // A new screen every second: 60 two-sample states, all changes.
    let analysis = analyse_window(
        window(0, 60.0)?,
        &grid(0.0, 60.0, |time| {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "small test levels"
            )]
            let second = time as u8;
            (
                40,
                if second.is_multiple_of(2) {
                    40
                } else {
                    60 + second * 3
                },
            )
        }),
        POLICY,
    )?;
    assert_eq!(analysis.candidates.len(), MAX_WINDOW_CANDIDATES);
    assert_eq!(analysis.dropped_candidates, 60 - 32);
    let window_value = identify(&analysis)?;
    assert_eq!(window_value.candidates().len(), 32);
    assert_eq!(
        analysis.candidates.first().map(|draft| draft.reason),
        Some(CandidateReason::FirstFrame)
    );
    assert_eq!(
        analysis
            .candidates
            .iter()
            .map(|draft| usize::from(draft.sample_count))
            .sum::<usize>(),
        120
    );
    Ok(())
}

#[test]
fn rejects_unordered_outside_and_excess_samples() -> TestResult {
    let target = window(1, 120.0)?;
    let early = vec![uniform(seconds(59.4), 1)];
    assert!(analyse_window(target, &early, POLICY).is_err());
    let late = vec![uniform(seconds(120.0), 1)];
    assert!(analyse_window(target, &late, POLICY).is_err());
    let unordered = vec![uniform(seconds(61.0), 1), uniform(seconds(61.0), 1)];
    assert!(analyse_window(target, &unordered, POLICY).is_err());
    let many: Vec<VisualSample> = (0..123_u64)
        .map(|step| uniform(MediaTime::from_micros(60_000_000 + step * 400_000), 1))
        .collect();
    assert!(analyse_window(target, &many, POLICY).is_err());
    Ok(())
}

#[test]
fn validation_rejects_tampered_windows() -> TestResult {
    let analysis = analyse_window(
        window(0, 12.0)?,
        &grid(
            0.0,
            12.0,
            |time| if time >= 5.0 { (40, 90) } else { (40, 40) },
        ),
        POLICY,
    )?;
    identify(&analysis)?;

    let mut shifted = analysis.clone();
    if let Some(draft) = shifted.candidates.get_mut(1) {
        draft.representative = seconds(5.5);
    }
    assert!(identify(&shifted).is_err());

    let mut weak = analysis.clone();
    if let Some(draft) = weak.candidates.get_mut(1) {
        draft.change = Some(super::CandidateChange::new(
            seconds(4.5),
            super::VisualDelta::new(1, 5)?,
        ));
    }
    assert!(identify(&weak).is_err());

    let mut uncovered = analysis.clone();
    uncovered.candidates.pop();
    if let Some(last) = uncovered.candidates.last_mut() {
        last.span = crate::TimeRange::new(last.representative, seconds(12.0))?;
        last.sample_count += 4;
    }
    assert!(identify(&uncovered).is_err());

    let mut miscounted = analysis;
    miscounted.sample_count += 1;
    assert!(identify(&miscounted).is_err());
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn any_sample_sequence_yields_valid_bounded_deterministic_coverage(
        ordinal in 0_u32..3,
        tail_seconds in 1_u64..60,
        gaps in prop::collection::vec(500_000_u64..3_000_000, 1..121),
        screens in prop::collection::vec(0_u8..6, 121),
        with_lead in prop::bool::ANY,
    ) {
        let duration = MediaTime::from_micros(u64::from(ordinal) * 60_000_000 + tail_seconds * 1_000_000);
        let Ok(target) = VisualWindow::new(ordinal, duration) else {
            return Ok(());
        };
        let mut samples = Vec::new();
        let mut time = if with_lead && ordinal > 0 {
            target.lead_in_start().as_micros()
        } else {
            target.range().start().as_micros()
        };
        for (gap, level) in gaps.iter().zip(&screens) {
            if time >= target.range().end().as_micros() {
                break;
            }
            samples.push(uniform(MediaTime::from_micros(time), level * 30));
            time += gap;
        }
        let first = analyse_window(target, &samples, POLICY);
        let second = analyse_window(target, &samples, POLICY);
        prop_assert_eq!(&first, &second);
        let Ok(analysis) = first else {
            return Ok(());
        };
        prop_assert!(analysis.candidates.len() <= MAX_WINDOW_CANDIDATES);
        prop_assert!(identify(&analysis).is_ok());
        for cell in target.cells() {
            let has_sample = samples.iter().any(|sample| super::contains(cell, sample.time()));
            let has_candidate = analysis
                .candidates
                .iter()
                .any(|draft| super::contains(cell, draft.representative));
            prop_assert_eq!(has_sample, has_candidate);
            prop_assert_eq!(!has_sample, analysis.frameless_cells.contains(&cell));
        }
    }
}
