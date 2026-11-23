//! Reading in rounds, over a fake stream and an engine that reads a known clock:
//! what `clocks` does between the index and the frames (ADR 0029 decision 12).

use std::{
    future::{Future, ready},
    sync::atomic::{AtomicUsize, Ordering},
};

use vsift_application::{
    EngineLine, EngineReading, EvidenceControl, EvidenceMediaError, EvidenceStop, ExtractedFrame,
    FrameExtractor, ScreenTextExtension, ScreenTextImage, ScreenTextReadError, ScreenTextReader,
    ScreenTextStop, VideoStreamFacts, merge_screen_text,
};
use vsift_domain::{
    FrameDimensions, FrameListing, ListedFrame, ListingTail, MediaTime, ProviderLabel, ReadTarget,
    ScreenTextIndex, ScreenTextProvider, SessionId, Sha256Hex, SourceId, TimeBase, TimeRange,
    analyse_clocks,
};

use super::{RoundsStart, read_rounds};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SECOND: u64 = 1_000_000;
/// Ten frames a second for five minutes.
const FRAMES: i64 = 3_000;
const PERIOD: u64 = 10 * SECOND;
/// The clock shows 14:00:25 at the start of the video, so it turns over at
/// 35 s, 95 s and so on.
const START_SECOND: u64 = 14 * 3_600 + 25;

fn frame_time(pts: i64) -> MediaTime {
    MediaTime::from_micros(u64::try_from(pts).unwrap_or(0) * SECOND / 10)
}

struct FakeVideo {
    facts: VideoStreamFacts,
}

impl FakeVideo {
    fn new() -> Built<Self> {
        Ok(Self {
            facts: VideoStreamFacts {
                stream_index: 0,
                time_base: TimeBase::new(1, 10)?,
                displayed: FrameDimensions::new(1920, 1080)?,
                duration: MediaTime::from_micros(300 * SECOND),
            },
        })
    }

    fn listing(&self, requested: TimeRange) -> Result<FrameListing, EvidenceMediaError> {
        let duration = self.facts.duration;
        let (end, tail) = if requested.end() >= duration {
            (duration, ListingTail::EndOfStream)
        } else {
            (requested.end(), ListingTail::MoreMayFollow)
        };
        let covered =
            TimeRange::new(requested.start(), end).map_err(|_| EvidenceMediaError::Invalid)?;
        let frames = (0..FRAMES)
            .map(|pts| ListedFrame {
                pts,
                time: frame_time(pts),
            })
            .filter(|frame| frame.time >= covered.start() && frame.time < covered.end())
            .collect();
        FrameListing::new(covered, frames, tail).map_err(|_| EvidenceMediaError::Invalid)
    }
}

impl FrameExtractor for FakeVideo {
    fn stream(&self) -> VideoStreamFacts {
        self.facts
    }

    fn max_frames_per_run(&self) -> usize {
        8
    }

    fn list_frames(
        &self,
        requested: TimeRange,
    ) -> impl Future<Output = Result<FrameListing, EvidenceMediaError>> + Send {
        ready(self.listing(requested))
    }

    fn frames(
        &self,
        pts: &[i64],
    ) -> impl Future<Output = Result<Vec<ExtractedFrame>, EvidenceMediaError>> + Send {
        ready(Ok(pts
            .iter()
            .map(|pts| ExtractedFrame {
                pts: *pts,
                time: frame_time(*pts),
                png: pts.to_string().into_bytes(),
            })
            .collect()))
    }

    fn crop(
        &self,
        _pts: i64,
        _rect: vsift_domain::CropRect,
    ) -> impl Future<Output = Result<ExtractedFrame, EvidenceMediaError>> + Send {
        ready(Err(EvidenceMediaError::Invalid))
    }
}

/// An engine that reads the clock off each frame, whose "image" is its number.
struct ClockReader {
    images: AtomicUsize,
}

impl ClockReader {
    fn provider() -> Result<ScreenTextProvider, ScreenTextReadError> {
        let digest = |fill: char| {
            Sha256Hex::parse(fill.to_string().repeat(64))
                .map_err(|_| ScreenTextReadError::ProviderFailed)
        };
        Ok(ScreenTextProvider {
            runner_sha256: digest('a')?,
            runner_version: ProviderLabel::parse("0.1.0")
                .map_err(|_| ScreenTextReadError::ProviderFailed)?,
            runtime_version: ProviderLabel::parse("1.31.0")
                .map_err(|_| ScreenTextReadError::ProviderFailed)?,
            detector_sha256: digest('d')?,
            recognizer_sha256: digest('e')?,
        })
    }

    fn reading(image: &ScreenTextImage<'_>) -> EngineReading {
        let pts: u64 = String::from_utf8_lossy(image.png).parse().unwrap_or(0);
        let shown = START_SECOND + pts / 10;
        EngineReading::Lines {
            lines: vec![EngineLine {
                x: 1800,
                y: 1040,
                width: 60,
                height: 14,
                text: format!("{:02}:{:02}", shown / 3_600, shown / 60 % 60),
                confidence_basis_points: 9_500,
            }],
            truncated: false,
        }
    }
}

impl ScreenTextReader for ClockReader {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<ScreenTextProvider, ScreenTextReadError>> + Send {
        ready(Self::provider())
    }

    fn max_images_per_run(&self) -> usize {
        8
    }

    fn read(
        &self,
        _build: &ScreenTextProvider,
        images: &[ScreenTextImage<'_>],
    ) -> impl Future<Output = Result<Vec<EngineReading>, ScreenTextReadError>> + Send {
        self.images.fetch_add(images.len(), Ordering::SeqCst);
        ready(Ok(images.iter().map(Self::reading).collect()))
    }
}

struct Running;

impl EvidenceControl for Running {
    fn stop(&self) -> Option<EvidenceStop> {
        None
    }
}

fn grid() -> Vec<ReadTarget> {
    (0..30)
        .map(|step| ReadTarget {
            time: MediaTime::from_micros(step * PERIOD),
            candidate: None,
        })
        .collect()
}

fn search_targets(index: &ScreenTextIndex) -> Vec<ReadTarget> {
    analyse_clocks(index, MediaTime::from_micros(300 * SECOND))
        .next_reads()
        .into_iter()
        .map(|time| ReadTarget {
            time,
            candidate: None,
        })
        .collect()
}

struct Call {
    session: SessionId,
    source: SourceId,
}

impl Call {
    fn new() -> Built<Self> {
        Ok(Self {
            session: SessionId::parse("ses_0123456789abcdef")?,
            source: SourceId::parse(format!("src_sha256_{}", "a".repeat(64)))?,
        })
    }

    fn start(&self, max_frames: u16) -> RoundsStart<'_> {
        RoundsStart {
            session_id: &self.session,
            source_id: &self.source,
            previous: None,
            max_frames,
            revisions_left: 64,
            period: PERIOD,
        }
    }
}

fn committed(extension: &ScreenTextExtension) -> Built<ScreenTextIndex> {
    merge_screen_text(None, extension, PERIOD)?.ok_or_else(|| "nothing to commit".into())
}

/// One call reads the grid, then narrows, each round seeing the one before.
#[tokio::test]
async fn rounds_narrow_a_rollover_and_end_in_one_commit() -> TestResult {
    let call = Call::new()?;
    let reader = ClockReader {
        images: AtomicUsize::new(0),
    };
    let mut plans = 0;
    let extension = read_rounds(
        call.start(200),
        &FakeVideo::new()?,
        &reader,
        &Running,
        &mut |held: Option<&ScreenTextIndex>| {
            plans += 1;
            held.map_or_else(grid, search_targets)
        },
    )
    .await?;
    assert_eq!(extension.stop, None);
    assert!(plans > 2, "a search takes more than one round: {plans}");
    let index = committed(&extension)?;
    assert_eq!(index.revision().get(), 1, "the rounds are one revision");
    let analysis = analyse_clocks(&index, MediaTime::from_micros(300 * SECOND));
    let [clock] = analysis.clocks.as_slice() else {
        return Err("no single clock".into());
    };
    let offset = clock.tightest_offset().ok_or("no offset")?;
    let truth = i64::try_from(START_SECOND * SECOND)?;
    assert!(offset.contains(truth), "{offset:?}");
    assert!(offset.width() <= 1_000_000, "{offset:?}");
    assert_eq!(analysis.next_reads(), Vec::new(), "the search has ended");
    Ok(())
}

#[tokio::test]
async fn the_budget_holds_across_rounds_and_a_later_call_continues() -> TestResult {
    let call = Call::new()?;
    let reader = ClockReader {
        images: AtomicUsize::new(0),
    };
    let extension = read_rounds(
        call.start(32),
        &FakeVideo::new()?,
        &reader,
        &Running,
        &mut |held: Option<&ScreenTextIndex>| held.map_or_else(grid, search_targets),
    )
    .await?;
    assert_eq!(reader.images.load(Ordering::SeqCst), 32);
    let first = committed(&extension)?;
    let mut start = call.start(40);
    start.previous = Some(&first);
    let later = read_rounds(
        start,
        &FakeVideo::new()?,
        &reader,
        &Running,
        &mut |held: Option<&ScreenTextIndex>| held.map_or_else(Vec::new, search_targets),
    )
    .await?;
    let second = merge_screen_text(Some(&first), &later, PERIOD)?.ok_or("nothing to commit")?;
    let analysis = analyse_clocks(&second, MediaTime::from_micros(300 * SECOND));
    assert_eq!(
        analysis.next_reads(),
        Vec::new(),
        "the later call finished it"
    );
    Ok(())
}

/// A target that lands on a frame an earlier round of the same call read joins
/// it, or the index would not know it was attempted.
#[tokio::test]
async fn a_later_round_can_join_a_frame_an_earlier_one_read() -> TestResult {
    let call = Call::new()?;
    let reader = ClockReader {
        images: AtomicUsize::new(0),
    };
    let on_a_frame = ReadTarget {
        time: MediaTime::from_micros(1_100_000),
        candidate: None,
    };
    let just_before = ReadTarget {
        time: MediaTime::from_micros(1_050_000),
        candidate: None,
    };
    let mut round = 0;
    let extension = read_rounds(
        call.start(40),
        &FakeVideo::new()?,
        &reader,
        &Running,
        &mut |_: Option<&ScreenTextIndex>| {
            round += 1;
            match round {
                1 => vec![on_a_frame.clone()],
                2 => vec![just_before.clone()],
                _ => Vec::new(),
            }
        },
    )
    .await?;
    let index = committed(&extension)?;
    assert_eq!(index.frames().len(), 1);
    assert!(index.attempted(&on_a_frame));
    assert!(index.attempted(&just_before));
    Ok(())
}

/// `ocr` plans once, and an empty plan still answers: its path is unchanged.
#[tokio::test]
async fn a_single_plan_reads_once_and_an_empty_one_still_answers() -> TestResult {
    let call = Call::new()?;
    let reader = ClockReader {
        images: AtomicUsize::new(0),
    };
    let mut once = Some(grid());
    let extension = read_rounds(
        call.start(40),
        &FakeVideo::new()?,
        &reader,
        &Running,
        &mut |_: Option<&ScreenTextIndex>| once.take().unwrap_or_default(),
    )
    .await?;
    assert_eq!(extension.frames.len(), 30);
    assert_eq!(extension.stop, None);

    let mut none = Some(Vec::new());
    let empty = read_rounds(
        call.start(40),
        &FakeVideo::new()?,
        &reader,
        &Running,
        &mut |_: Option<&ScreenTextIndex>| none.take().unwrap_or_default(),
    )
    .await?;
    assert_eq!(empty.frames.len(), 0);
    assert_eq!(empty.stop, None::<ScreenTextStop>);
    Ok(())
}
