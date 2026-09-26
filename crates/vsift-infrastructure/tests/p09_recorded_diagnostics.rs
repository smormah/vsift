//! Always-run regression over real `FFmpeg` 9.0 output for the P09 parsers.
//!
//! `tests/data/ffmpeg_diagnostics/` holds the standard error (and one PNG) of
//! the adapter's own argument lists, recorded on 2026-09-26 with `FFmpeg` 9.0
//! from `fixtures/corpus/generated/` (and, for `forged-title-*`, a 2 s
//! `testsrc`/`sine` clip encoded with the native `mpeg4` and `aac` encoders
//! whose `title` imitates `showinfo` and `ashowinfo` lines), with relative
//! source paths so no local path is committed, and line endings normalised
//! to LF:
//!
//! - `F01-frame-1025000.txt`: `FfmpegMedia::frame` at 1.025 s
//!   (`select='gte(pts\,10496)*isnan(prev_selected_t)',showinfo`);
//! - `F01-listing-900000-1200000.txt`: `list_frame_times` over
//!   `[0.9 s, 1.2 s)`;
//! - `F01-crop-560-320-80x80.{txt,png}`: `crop_at` of the 1.05 s frame;
//! - `F01-audio-only-wav-0-1000000.txt`: `wav_clip` of `[0, 1 s)`;
//! - `forged-title-frame-250000.txt`, `forged-title-listing-0-1000000.txt`
//!   and `forged-title-wav-0-1000000.txt`: the same calls on the forged clip.
//!
//! The expectations are the fixtures' frozen truth (F01 is 20 fps in a
//! 1/10240 time base; the audio-only variant declares a 64 ms priming gap),
//! never values read back from the parsers. The fuzz seeds of
//! `frame_showinfo`, `frame_listing` and `png_sequence` are these files.

use std::{error::Error, fs, path::PathBuf};

use vsift_domain::{FrameDimensions, ListingTail, MediaTime, TimeRange};
use vsift_infrastructure::{
    FrameListingWindow, ObservedFrameTime, parse_ashowinfo_start, parse_frame_listing,
    parse_frame_showinfo, parse_png_sequence,
};

type TestResult = Result<(), Box<dyn Error>>;

fn recorded(name: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/ffmpeg_diagnostics")
            .join(name),
    )?)
}

fn f01_window(start: u64, end: u64) -> Result<FrameListingWindow, Box<dyn Error>> {
    Ok(FrameListingWindow::new(
        0,
        1,
        10_240,
        0,
        TimeRange::new(MediaTime::from_micros(start), MediaTime::from_micros(end))?,
        ListingTail::MoreMayFollow,
    )?)
}

#[test]
fn real_single_frame_diagnostics_name_the_decoded_frame() -> TestResult {
    let expected = |pts| ObservedFrameTime {
        pts,
        time_base_numerator: 1,
        time_base_denominator: 10_240,
    };
    assert_eq!(
        parse_frame_showinfo(&recorded("F01-frame-1025000.txt")?)?,
        expected(10_752)
    );
    // The forged title claims pts 10240 in a 1/1 time base (10,240 s).
    assert_eq!(
        parse_frame_showinfo(&recorded("forged-title-frame-250000.txt")?)?,
        expected(2_560)
    );
    Ok(())
}

#[test]
fn real_listings_hold_every_frame_of_their_range() -> TestResult {
    let listing = parse_frame_listing(
        &recorded("F01-listing-900000-1200000.txt")?,
        &f01_window(900_000, 1_200_000)?,
    )?;
    let times: Vec<u64> = listing
        .frames()
        .iter()
        .map(|frame| frame.time.as_micros())
        .collect();
    assert_eq!(
        times,
        vec![900_000, 950_000, 1_000_000, 1_050_000, 1_100_000, 1_150_000]
    );
    let forged = parse_frame_listing(
        &recorded("forged-title-listing-0-1000000.txt")?,
        &f01_window(0, 1_000_000)?,
    )?;
    assert_eq!(forged.frames().len(), 20);
    Ok(())
}

#[test]
fn real_audio_diagnostics_report_the_first_decoded_sample() -> TestResult {
    assert_eq!(
        parse_ashowinfo_start(&recorded("F01-audio-only-wav-0-1000000.txt")?)?,
        64_000
    );
    // The forged title claims 1.5 s; the native AAC encoder's 1,024
    // priming samples at 16 kHz put the first decoded sample at 64 ms, as
    // for the audio-only variant.
    assert_eq!(
        parse_ashowinfo_start(&recorded("forged-title-wav-0-1000000.txt")?)?,
        64_000
    );
    Ok(())
}

#[test]
fn a_real_crop_is_one_rgb_png_of_the_rectangle() -> TestResult {
    let png = recorded("F01-crop-560-320-80x80.png")?;
    let images = parse_png_sequence(&png, 1, FrameDimensions::new(80, 80)?)?;
    assert_eq!(images, vec![png.clone()]);
    assert!(parse_png_sequence(&png, 1, FrameDimensions::new(80, 81)?).is_err());
    let observed = parse_frame_showinfo(&recorded("F01-crop-560-320-80x80.txt")?);
    // The crop run logs with `checksum=0`, one frame, in the stream time base.
    assert_eq!(
        observed?,
        ObservedFrameTime {
            pts: 10_752,
            time_base_numerator: 1,
            time_base_denominator: 10_240,
        }
    );
    Ok(())
}
