use vsift_domain::{ListingTail, MAX_LISTED_FRAMES, MediaTime, TimeRange};

use super::{
    FrameListingWindow, ObservedFrameTime, parse_ashowinfo_start, parse_frame_listing,
    parse_frame_showinfo,
};
use crate::MediaError;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const CONFIG: &str = "[Parsed_showinfo_1 @ 0] config in time_base: 1/10240, frame_rate: 20/1\n";

fn frame_line(number: usize, pts: i64) -> String {
    format!(
        "[Parsed_showinfo_1 @ 0] n:{number:>4} pts:{pts:>7} pts_time:x duration:    512 fmt:yuv420p s:1280x720 i:P iskey:0 type:P\n"
    )
}

/// A listing of F01 (20 fps, 1/10240, origin 0) over `[start, end)` microseconds.
fn f01_window(
    start: u64,
    end: u64,
    tail: ListingTail,
) -> Result<FrameListingWindow, Box<dyn std::error::Error>> {
    Ok(FrameListingWindow::new(
        0,
        1,
        10_240,
        0,
        TimeRange::new(MediaTime::from_micros(start), MediaTime::from_micros(end))?,
        tail,
    )?)
}

#[test]
fn a_single_frame_extraction_logs_exactly_one_frame() -> TestResult {
    let one = format!("{CONFIG}{}", frame_line(0, 10_752));
    assert_eq!(
        parse_frame_showinfo(one.as_bytes())?,
        ObservedFrameTime {
            pts: 10_752,
            time_base_numerator: 1,
            time_base_denominator: 10_240,
        }
    );
    for rejected in [
        format!("{CONFIG}{}{}", frame_line(0, 10_752), frame_line(1, 11_264)),
        frame_line(0, 10_752),
        CONFIG.to_owned(),
        format!("{CONFIG}{}", frame_line(1, 10_752)),
        format!("{CONFIG}  {}", frame_line(0, 10_752)),
        format!(
            "{CONFIG}{}",
            frame_line(0, 10_752).replace("s:1280x720", "s:1280y720")
        ),
        format!("{CONFIG}{}", frame_line(0, 10_752).replace("] n:", "]n:")),
    ] {
        assert!(
            parse_frame_showinfo(rejected.as_bytes()).is_err(),
            "{rejected}"
        );
    }
    assert!(matches!(
        parse_frame_showinfo(&vec![b' '; crate::MAX_DIAGNOSTIC_BYTES + 1]),
        Err(MediaError::OutputLimit)
    ));
    Ok(())
}

#[test]
fn a_forged_frame_line_breaks_the_numbering() {
    // A line forged through an untrusted string with an embedded line break
    // begins with the prefix; it adds a frame the real numbering does not
    // have, so the output is rejected rather than misread.
    let forged = format!(
        "[mov @ 0] unknown tag \n{}{CONFIG}{}",
        frame_line(0, 99_999),
        frame_line(0, 10_752)
    );
    assert!(parse_frame_showinfo(forged.as_bytes()).is_err());
    let after = format!("{CONFIG}{}{}", frame_line(0, 10_752), frame_line(0, 99_999));
    assert!(parse_frame_showinfo(after.as_bytes()).is_err());
}

#[test]
fn audio_starts_come_only_from_consistent_filter_lines() -> TestResult {
    let line = |number: usize, pts: i64, time: &str| {
        format!(
            "[Parsed_ashowinfo_1 @ 0] n:{number} pts:{pts} pts_time:{time} fmt:fltp channels:1 nb_samples:65536\n"
        )
    };
    let good = format!("{}{}", line(0, 1_024, "0.064"), line(1, 66_560, "4.16"));
    assert_eq!(parse_ashowinfo_start(good.as_bytes())?, 64_000);
    assert_eq!(
        parse_ashowinfo_start(line(0, 0, "-0.021333").as_bytes())?,
        -21_333
    );
    for rejected in [
        String::new(),
        line(1, 1_024, "0.064"),
        format!("{}{}", line(0, 1_024, "0.064"), line(0, 2_048, "0.128")),
        format!("{}{}", line(0, 1_024, "0.064"), line(2, 2_048, "0.128")),
        format!("{}{}", line(0, 2_048, "0.128"), line(1, 1_024, "0.064")),
        line(0, 1_024, "NOPTS"),
        line(0, 1_024, "0.064").replace("pts_time:0.064", ""),
        format!("    title : {}", line(0, 0, "9.5")),
    ] {
        assert!(
            parse_ashowinfo_start(rejected.as_bytes()).is_err(),
            "{rejected}"
        );
    }
    Ok(())
}

#[test]
fn listings_hold_every_frame_inside_their_bounds_in_the_stream_time_base() -> TestResult {
    let window = f01_window(1_000_000, 1_200_000, ListingTail::MoreMayFollow)?;
    assert_eq!((window.first_pts(), window.end_pts()), (10_240, 12_288));
    let mut lines = String::new();
    for (number, pts) in [10_240, 10_752, 11_264, 11_776].into_iter().enumerate() {
        lines.push_str(&frame_line(number, pts));
    }
    let listing = parse_frame_listing(format!("{CONFIG}{lines}").as_bytes(), &window)?;
    let times: Vec<u64> = listing
        .frames()
        .iter()
        .map(|frame| frame.time.as_micros())
        .collect();
    assert_eq!(times, vec![1_000_000, 1_050_000, 1_100_000, 1_150_000]);
    assert_eq!(listing.tail(), ListingTail::MoreMayFollow);
    assert!(parse_frame_listing(b"", &window)?.frames().is_empty());
    let mismatch = format!("{}{lines}", CONFIG.replace("1/10240", "1/1000"));
    assert!(matches!(
        parse_frame_listing(mismatch.as_bytes(), &window),
        Err(MediaError::TimeBaseMismatch)
    ));
    assert!(matches!(
        parse_frame_listing(lines.as_bytes(), &window),
        Err(MediaError::InvalidDecodedOutput)
    ));
    for outside in [10_239, 12_288] {
        let text = format!("{CONFIG}{}", frame_line(0, outside));
        assert!(parse_frame_listing(text.as_bytes(), &window).is_err());
    }
    Ok(())
}

#[test]
fn a_listing_beyond_its_frame_bound_is_cut_before_the_first_unlisted_frame() -> TestResult {
    let window = f01_window(0, 60_000_000, ListingTail::EndOfStream)?;
    let mut text = String::from(CONFIG);
    let mut pts = 0;
    for number in 0..=MAX_LISTED_FRAMES {
        text.push_str(&frame_line(number, pts));
        pts += 256;
    }
    let listing = parse_frame_listing(text.as_bytes(), &window)?;
    assert_eq!(listing.frames().len(), MAX_LISTED_FRAMES);
    assert_eq!(listing.tail(), ListingTail::MoreMayFollow);
    // A 40 fps stream: the 1,201st frame is at 30 s.
    assert_eq!(listing.covered().end().as_micros(), 30_000_000);
    text.push_str(&frame_line(MAX_LISTED_FRAMES + 1, pts));
    assert!(parse_frame_listing(text.as_bytes(), &window).is_err());
    Ok(())
}
