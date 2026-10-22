//! The OCR runner adapter against a stand-in runner (ADR 0030): the process is
//! really started under the supervisor, given its frames as files in a private
//! folder, and its answers are read, refused or classified.

mod support;

use std::{
    fs,
    num::NonZeroU16,
    path::PathBuf,
    time::{Duration, Instant},
};

use vsift_application::{EngineReading, ScreenTextImage, ScreenTextReadError};
use vsift_application::{InitializeSessionStorage, InitializeSessionStorageRequest};
use vsift_domain::{
    DurabilityRequirement, FrameDimensions, OperationId, ScreenTextProvider, SessionId, Sha256Hex,
};
use vsift_infrastructure::{
    FfmpegMedia, FilesystemSessionStore, HostIsolation, MediaError, MediaProviderConformance,
    OcrRunner, ProcessCancellation, ProcessWorkingDirectory, SourceSnapshot, TrustedExecutable,
};

use support::PrivateStaging;

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const STAND_IN: &str = env!("CARGO_BIN_EXE_vsift-ocr-stand-in");
/// The media-tool stand-in; staged as `ffprobe-hang` it sleeps past any deadline.
const SMOKE_FIXTURE: &str = env!("CARGO_BIN_EXE_vsift-smoke-fixture");

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

/// The build every copy of the stand-in is (they all have the same bytes):
/// `good`'s identity.
async fn good_build(staging: &PrivateStaging) -> Built<ScreenTextProvider> {
    Ok(runner(staging, "good")?
        .identity(ProcessCancellation::new())
        .await?)
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
        .read(&workspace, &identity, &images, ProcessCancellation::new())
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
    let build = good_build(&tools).await?;
    for (behaviour, expected) in [
        ("garbage", ScreenTextReadError::UnparseableOutput),
        ("nomodel", ScreenTextReadError::ModelUnavailable),
        ("crash", ScreenTextReadError::AbnormalTermination),
        ("liar", ScreenTextReadError::UnparseableOutput),
        ("drift", ScreenTextReadError::BuildChanged),
    ] {
        let runner = runner(&tools, behaviour)?;
        let read = runner
            .read(&workspace, &build, &images, ProcessCancellation::new())
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
    let build = good_build(&tools).await?;
    let cancellation = ProcessCancellation::new();
    let cancel = cancellation.clone();
    let started = Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
    });
    let read = runner.read(&workspace, &build, &images, cancellation).await;
    assert_eq!(read, Err(ScreenTextReadError::Cancelled));
    assert!(
        started.elapsed().as_secs() < 30,
        "the hanging runner was stopped, not waited for"
    );
    Ok(())
}

/// RV-2 finding 3: a runner executable that is not the one whose digest the
/// build records (replaced between `describe` and a `read`) is refused before
/// it is started, and nothing is handed over.
#[tokio::test]
async fn an_executable_changed_since_describe_is_refused_before_it_runs() -> TestResult {
    let tools = PrivateStaging::new("vsift-ocr-tools")?;
    let hand_over = PrivateStaging::new("vsift-ocr-hand-over")?;
    let workspace = ProcessWorkingDirectory::new(hand_over.path())?;
    let build = ScreenTextProvider {
        runner_sha256: Sha256Hex::parse("0".repeat(64))?,
        ..good_build(&tools).await?
    };
    let png = image(640, 360, "x");
    let images = [ScreenTextImage {
        png: &png,
        dimensions: FrameDimensions::new(640, 360)?,
    }];
    let read = runner(&tools, "good")?
        .read(&workspace, &build, &images, ProcessCancellation::new())
        .await;
    assert_eq!(read, Err(ScreenTextReadError::BuildChanged));
    assert_eq!(fs::read_dir(hand_over.path())?.count(), 0);
    Ok(())
}

/// RV-2 finding 4: a runner bounded by its call runs for at most the time the
/// call has left, not its own 30 s and 20 s an image, and starts nothing once
/// the call has none left.
#[tokio::test]
async fn a_hanging_runner_ends_with_its_call() -> TestResult {
    let tools = PrivateStaging::new("vsift-ocr-tools")?;
    let hand_over = PrivateStaging::new("vsift-ocr-hand-over")?;
    let workspace = ProcessWorkingDirectory::new(hand_over.path())?;
    let build = good_build(&tools).await?;
    let png = image(640, 360, "x");
    let images = [ScreenTextImage {
        png: &png,
        dimensions: FrameDimensions::new(640, 360)?,
    }; 8];
    let started = Instant::now();
    let hanging = runner(&tools, "hang")?.with_call_deadline(started + Duration::from_secs(2));
    let read = hanging
        .read(&workspace, &build, &images, ProcessCancellation::new())
        .await;
    assert_eq!(read, Err(ScreenTextReadError::Deadline));
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the run ended with the call, not after its own 190 s: {:?}",
        started.elapsed()
    );
    assert_eq!(fs::read_dir(hand_over.path())?.count(), 0);

    let spent = runner(&tools, "good")?.with_call_deadline(Instant::now());
    let started = Instant::now();
    assert_eq!(
        spent.identity(ProcessCancellation::new()).await,
        Err(ScreenTextReadError::Deadline)
    );
    assert_eq!(
        spent
            .read(&workspace, &build, &images, ProcessCancellation::new())
            .await,
        Err(ScreenTextReadError::Deadline)
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "nothing was started"
    );
    Ok(())
}

/// RV-2 finding 4, the media tools: an `ocr` call bounds its frame listings
/// and extractions the same way. A provider that hangs (the smoke fixture
/// staged as `ffprobe-hang`, whose own deadline is 15 s) ends with the call,
/// and none starts once the call has no time left.
#[tokio::test]
async fn media_runs_end_with_their_call() -> TestResult {
    let sessions = PrivateStaging::new("vsift-ocr-sessions")?;
    let tools = PrivateStaging::new("vsift-ocr-media-tools")?;
    let store = FilesystemSessionStore::provision_default(sessions.path().join("root"))?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    let store = FilesystemSessionStore::open_existing(sessions.path().join("root"))?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/generated/F01.mp4"),
    )?;
    let hanging = tools
        .path()
        .join(format!("ffprobe-hang{}", std::env::consts::EXE_SUFFIX));
    fs::copy(SMOKE_FIXTURE, &hanging)?;
    let hanging = TrustedExecutable::explicit(&hanging)?;
    let conformance = MediaProviderConformance::r0(hanging.clone(), hanging);

    let started = Instant::now();
    let media = FfmpegMedia::new(conformance.clone(), HostIsolation::ProcessOnly, &store)
        .with_call_deadline(started + Duration::from_secs(2));
    let probed = media.probe(&snapshot, ProcessCancellation::new()).await;
    assert!(matches!(probed, Err(MediaError::Deadline)), "{probed:?}");
    assert!(
        started.elapsed() < Duration::from_secs(12),
        "the probe ended with the call, not after its own 15 s: {:?}",
        started.elapsed()
    );

    let spent = FfmpegMedia::new(conformance, HostIsolation::ProcessOnly, &store)
        .with_call_deadline(Instant::now());
    let started = Instant::now();
    let probed = spent.probe(&snapshot, ProcessCancellation::new()).await;
    assert!(matches!(probed, Err(MediaError::Deadline)), "{probed:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "nothing was started"
    );
    drop(snapshot);
    Ok(())
}
