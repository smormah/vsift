//! The OCR runner adapter against a stand-in runner (ADR 0030): the process is
//! really started under the supervisor, given its frames as files in a private
//! folder, and its answers are read, refused or classified.

mod support;

use std::{fs, num::NonZeroU16, path::PathBuf, time::Instant};

use vsift_application::{EngineReading, ScreenTextImage, ScreenTextReadError};
use vsift_domain::FrameDimensions;
use vsift_infrastructure::{
    HostIsolation, OcrRunner, ProcessCancellation, ProcessWorkingDirectory, TrustedExecutable,
};

use support::PrivateStaging;

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const STAND_IN: &str = env!("CARGO_BIN_EXE_vsift-ocr-stand-in");

/// The stand-in staged as `ocr-<behaviour>` in its own private folder.
fn runner(staging: &PrivateStaging, behaviour: &str) -> Built<OcrRunner> {
    let name = format!("ocr-{behaviour}{}", std::env::consts::EXE_SUFFIX);
    let path: PathBuf = staging.path().join(name);
    fs::copy(STAND_IN, &path)?;
    Ok(OcrRunner::new(
        TrustedExecutable::explicit(&path)?,
        NonZeroU16::MIN,
        HostIsolation::ProcessOnly,
    ))
}

/// A PNG signature and `IHDR` head of `width` x `height`, then `text`: what the stand-in reads.
fn image(width: u32, height: u32, text: &str) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

#[tokio::test]
async fn the_runner_identifies_itself_and_reads_each_frame() -> TestResult {
    let tools = PrivateStaging::new("vsift-ocr-tools")?;
    let hand_over = PrivateStaging::new("vsift-ocr-hand-over")?;
    let runner = runner(&tools, "good")?;
    let identity = runner.identity(ProcessCancellation::new()).await?;
    assert_eq!(identity.runtime_version.as_str(), "1.31.0");
    assert_eq!(identity.recognizer_sha256.as_str(), "e".repeat(64));
    assert_eq!(identity.runner_sha256.as_str().len(), 64);

    let dimensions = FrameDimensions::new(1280, 720)?;
    let first = image(1280, 720, "Total population");
    let second = image(1280, 720, "Watchlist");
    let images = [
        ScreenTextImage {
            png: &first,
            dimensions,
        },
        ScreenTextImage {
            png: &second,
            dimensions,
        },
    ];
    let workspace = ProcessWorkingDirectory::new(hand_over.path())?;
    let readings = runner
        .read(&workspace, &images, ProcessCancellation::new())
        .await?;
    let texts: Vec<String> = readings
        .iter()
        .filter_map(|reading| match reading {
            EngineReading::Lines { lines, .. } => lines.first().map(|line| line.text.clone()),
            EngineReading::Rejected | EngineReading::Failed => None,
        })
        .collect();
    assert_eq!(
        texts,
        vec![String::from("Total population"), String::from("Watchlist")]
    );
    assert_eq!(
        hand_over.directory()?.entries()?.count(),
        0,
        "the hand-over files are removed after the run"
    );
    Ok(())
}

#[tokio::test]
async fn each_failing_runner_is_classified_and_leaves_no_files() -> TestResult {
    let tools = PrivateStaging::new("vsift-ocr-tools")?;
    let hand_over = PrivateStaging::new("vsift-ocr-hand-over")?;
    let workspace = ProcessWorkingDirectory::new(hand_over.path())?;
    let png = image(640, 360, "x");
    let images = [ScreenTextImage {
        png: &png,
        dimensions: FrameDimensions::new(640, 360)?,
    }; 2];
    for (behaviour, expected) in [
        ("garbage", ScreenTextReadError::UnparseableOutput),
        ("nomodel", ScreenTextReadError::ModelUnavailable),
        ("crash", ScreenTextReadError::AbnormalTermination),
        ("liar", ScreenTextReadError::UnparseableOutput),
    ] {
        let runner = runner(&tools, behaviour)?;
        let read = runner
            .read(&workspace, &images, ProcessCancellation::new())
            .await;
        assert_eq!(read, Err(expected), "{behaviour}");
        assert_eq!(fs::read_dir(hand_over.path())?.count(), 0, "{behaviour}");
    }
    Ok(())
}

#[tokio::test]
async fn a_cancelled_run_stops_the_runner() -> TestResult {
    let tools = PrivateStaging::new("vsift-ocr-tools")?;
    let hand_over = PrivateStaging::new("vsift-ocr-hand-over")?;
    let workspace = ProcessWorkingDirectory::new(hand_over.path())?;
    let runner = runner(&tools, "hang")?;
    let png = image(640, 360, "x");
    let images = [ScreenTextImage {
        png: &png,
        dimensions: FrameDimensions::new(640, 360)?,
    }];
    let cancellation = ProcessCancellation::new();
    let cancel = cancellation.clone();
    let started = Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        cancel.cancel();
    });
    let read = runner.read(&workspace, &images, cancellation).await;
    assert_eq!(read, Err(ScreenTextReadError::Cancelled));
    assert!(
        started.elapsed().as_secs() < 30,
        "the hanging runner was stopped, not waited for"
    );
    Ok(())
}
