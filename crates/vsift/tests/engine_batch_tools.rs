//! X-11 through the engine (P11 PR 4, ADR 0021 section 5): one batch mixes
//! a transcript import that succeeds, a malformed line, a recognition
//! cancelled mid-run with `job cancel`, a recognition whose stand-in
//! recognizer fails at its second chunk, and visual candidates over a
//! truncated video that end `partial`. Each line has its own outcome, and
//! the batch's outcome is maintainer decision D5's.
//!
//! Opt-in: it decodes real media with `FFmpeg` and `FFprobe` on `PATH` and
//! builds its long clips at run time (the native `mpeg4` encoder and AAC):
//!
//! `cargo test -p vsift --locked --test engine_batch_tools -- --ignored`

use std::{
    env,
    error::Error,
    ffi::{OsStr, OsString},
    fs,
    future::Future,
    num::NonZeroU16,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc as std_mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use tokio::sync::mpsc;
use vsift::{
    AdmissionBudget, AdmissionWait, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, BatchEvent, BatchLineEnd, Cancellation, ChunkTime, CueText,
    DurabilityRequirement, Engine, EngineConfig, EnginePorts, FailureClass, FailureCode,
    HostIsolation, JobId, LanguageTag, LocalAsrVerification, LocalAsrVerifier, ManagedRootLocation,
    MediaToolVerification, MediaToolVerifier, PlannedChunk, ProgressObserver, ProviderChunkOutput,
    ProviderSegment, ProviderToken, ProviderTokenKind, RecognizerIdentity, SessionRootLocation,
    Sha256Hex, SpeechPcm, SpeechRecognitionError, SpeechRecognizer, UserConfigurationLocation,
    WorkBatchRun, WorkspaceInitRequest, WorkspacePolicy, WorkspaceRetention,
};
use vsift_contract::{BatchOutcome, BatchTermination};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-engine-batch-tools-";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// How long the stand-in recognizer takes for one chunk.
const CHUNK_TIME: Duration = Duration::from_secs(2);
/// A full R0 chunk window.
const FULL_CHUNK_MICROS: u64 = 30_000_000;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct Layout(PathBuf);

impl Layout {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        let layout = Self(path);
        fs::create_dir_all(layout.inputs())?;
        fs::create_dir(layout.bundles())?;
        Ok(layout)
    }

    fn inputs(&self) -> PathBuf {
        self.0.join("inputs")
    }

    fn bundles(&self) -> PathBuf {
        self.0.join("bundles")
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus")
        .join(name)
}

/// Builds a `seconds`-long clip with a tone and a colour, with the native
/// `mpeg4` encoder and AAC.
fn build_clip(target: &Path, seconds: u32) -> TestResult {
    let duration = seconds.to_string();
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("sine=frequency=440:duration={duration}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("color=c=blue:s=320x240:r=10:d={duration}"))
        .args(["-shortest", "-c:v", "mpeg4", "-c:a", "aac"])
        .arg(OsString::from(target))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        return Err(format!("ffmpeg failed to build a {seconds} s clip").into());
    }
    Ok(())
}

fn identity() -> Result<RecognizerIdentity, SpeechRecognitionError> {
    let digest = Sha256Hex::parse(DIGEST).map_err(|_| SpeechRecognitionError::Io)?;
    Ok(RecognizerIdentity {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, digest.clone()),
        model: AsrModel::new(AsrModelProfile::Base, digest),
        decoding: AsrDecodingProfile::R0V1,
        threads: NonZeroU16::MIN,
    })
}

/// One segment per chunk, inside every window.
fn words(chunk: &PlannedChunk) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    let text = format!("words of chunk {}", chunk.index());
    let text = CueText::new(text.clone(), text).map_err(|_| SpeechRecognitionError::Io)?;
    let third = chunk.window().duration_micros() / 3_000;
    let start = 6_000.min(third);
    let length = 2_000.min(third);
    Ok(ProviderChunkOutput {
        language: LanguageTag::parse("en").ok(),
        segments: vec![ProviderSegment {
            start: ChunkTime::from_millis(start).ok_or(SpeechRecognitionError::Io)?,
            end: ChunkTime::from_millis(start + length).ok_or(SpeechRecognitionError::Io)?,
            text: Some(text),
            tokens: vec![ProviderToken {
                kind: ProviderTokenKind::Text,
                probability: 0.5,
            }],
        }],
    })
}

/// A slow stand-in recognizer: every chunk takes [`CHUNK_TIME`], and the
/// second chunk of a source whose second window is shorter than a full
/// chunk (the 50 s clip; the 70 s clip's is full) always fails as a
/// provider failure.
#[derive(Clone)]
struct StandInRecognizer;

impl SpeechRecognizer for StandInRecognizer {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send {
        std::future::ready(identity())
    }

    fn recognize(
        &self,
        chunk: &PlannedChunk,
        _pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        let fails = chunk.index() == 1 && chunk.window().duration_micros() < FULL_CHUNK_MICROS;
        let output = if fails {
            Err(SpeechRecognitionError::ProviderFailed)
        } else {
            words(chunk)
        };
        async move {
            tokio::time::sleep(CHUNK_TIME).await;
            output
        }
    }
}

#[derive(Clone)]
struct PassingAsr;

impl LocalAsrVerifier for PassingAsr {
    fn verify(&self) -> impl Future<Output = LocalAsrVerification> + Send {
        std::future::ready(LocalAsrVerification::Verified)
    }
}

#[derive(Clone)]
struct PassingMediaTools;

impl MediaToolVerifier for PassingMediaTools {
    fn verify(&self) -> impl Future<Output = MediaToolVerification> + Send {
        std::future::ready(MediaToolVerification::Verified)
    }
}

fn ingest(operation: &str, source: &str, transcript: &str, steps: &str) -> String {
    format!(
        r#"{{"schema_version":"1","operation_id":"{operation}","durability":"ephemeral","target":{{"ingest":{{"source":"{source}","transcript":{transcript}}}}},"steps":[{steps}]}}"#
    )
}

/// Lower is more severe: D5's order 7 > 1 > 5 > 3 > 2 > cancelled > 4,
/// written out here independently of the contract's implementation.
fn severity(code: FailureCode) -> usize {
    [
        FailureClass::StorageOrIo,
        FailureClass::Internal,
        FailureClass::Limit,
        FailureClass::Source,
        FailureClass::UsageOrCapability,
        FailureClass::Cancelled,
        FailureClass::Retryable,
    ]
    .iter()
    .position(|class| *class == code.class())
    .unwrap_or(usize::MAX)
}

/// X-11: a mixed batch reports independent outcomes (see the module
/// documentation), and its outcome is the most severe failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "opt-in: needs FFmpeg and FFprobe on PATH"]
#[allow(
    clippy::too_many_lines,
    reason = "One X-11 scenario: its five lines and their outcomes read best in one place"
)]
async fn a_mixed_batch_reports_independent_outcomes() -> TestResult {
    let layout = Layout::new()?;
    fs::copy(
        fixture("generated/F10.mp4"),
        layout.inputs().join("F10.mp4"),
    )?;
    fs::copy(
        fixture("transcripts/F10.srt"),
        layout.inputs().join("F10.srt"),
    )?;
    build_clip(&layout.inputs().join("long70.mp4"), 70)?;
    build_clip(&layout.inputs().join("long50.mp4"), 50)?;
    let bytes = fs::read(fixture("generated/F05.mp4"))?;
    fs::write(
        layout.inputs().join("truncated.mp4"),
        bytes.get(..bytes.len() * 6 / 10).unwrap_or_default(),
    )?;
    let lines = [
        ingest(
            "op_x11import000000000000000001",
            "F10.mp4",
            r#"{"path":"F10.srt","offset_us":500000}"#,
            r#"{"close":{}}"#,
        ),
        r#"{"schema_version":"1","operation_id":"#.to_owned(),
        ingest(
            "op_x11cancelled00000000000003",
            "long70.mp4",
            "null",
            r#"{"retranscribe":{"range":null}}"#,
        ),
        ingest(
            "op_x11failing000000000000004",
            "long50.mp4",
            "null",
            r#"{"retranscribe":{"range":null}}"#,
        ),
        ingest(
            "op_x11partial000000000000005",
            "truncated.mp4",
            "null",
            r#"{"candidates":{"range":null}}"#,
        ),
    ];
    let requests = layout.0.join("requests.jsonl");
    fs::write(&requests, lines.join("\n"))?;

    let engine = Arc::new(Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(layout.0.join("workspace")),
            user_configuration: UserConfigurationLocation::Explicit(layout.0.join("config")),
            managed_root: ManagedRootLocation::Explicit(layout.0.join("managed")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::system()
            .with_media_tool_verifier(PassingMediaTools)
            .with_speech_recognizer(StandInRecognizer, PassingAsr),
    ));
    engine.init_workspace(WorkspaceInitRequest {
        policy: WorkspacePolicy::new(
            DurabilityRequirement::Ephemeral,
            NonZeroU16::new(8).ok_or("zero")?,
            WorkspaceRetention::from_hours(2)?,
        )?,
    })?;

    // The job of line 3, reported by its progress, is cancelled by id.
    let (job_sender, job_receiver) = std_mpsc::channel::<JobId>();
    let progress: vsift::BatchProgress = Arc::new(move |line, _| {
        let sender = job_sender.clone();
        ProgressObserver::new(move |progress| {
            if line == 3
                && let Some(job) = &progress.job
            {
                let _ = sender.send(job.clone());
            }
        })
    });
    let (events, mut receiver) = mpsc::channel(8);
    let run = WorkBatchRun {
        requests,
        input_root: layout.inputs(),
        bundle_root: Some(layout.bundles()),
        admission: AdmissionWait::Bounded(AdmissionBudget::new(Duration::from_secs(60))?),
        concurrency: NonZeroU16::new(4).ok_or("zero")?,
        stop: Cancellation::new(),
        cancellation: Cancellation::new(),
        progress,
        events,
    };
    let batch = tokio::spawn(Arc::clone(&engine).run_work_batch(run));
    let canceller = {
        let engine = Arc::clone(&engine);
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let job = job_receiver
                .recv_timeout(Duration::from_secs(120))
                .map_err(|_| "line 3 never reported its job".to_owned())?;
            engine
                .job_cancel(&job)
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    };
    let mut results = std::collections::BTreeMap::new();
    while let Some(event) = receiver.recv().await {
        if let BatchEvent::Finished {
            line,
            end: BatchLineEnd::Ran(outcome),
        } = event
        {
            results.insert(line, serde_json::to_value(outcome.result())?);
        }
    }
    canceller.await??;
    let summary = batch.await??;
    assert_eq!(summary.termination(), BatchTermination::EndOfInput);
    let value = serde_json::to_value(&summary)?;
    let status = |line: u32| -> Built<Value> {
        value["items"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["line"] == line))
            .map(|item| item["status"].clone())
            .ok_or_else(|| format!("no item for line {line}: {value}").into())
    };
    assert_eq!(status(1)?, "complete", "{value} {results:?}");
    assert_eq!(status(2)?, "rejected", "{value}");
    assert_eq!(status(3)?, "cancelled", "{value} {:?}", results.get(&3));
    assert_eq!(status(4)?, "failed", "{value} {:?}", results.get(&4));
    assert_eq!(status(5)?, "partial", "{value} {:?}", results.get(&5));
    let imported = results.get(&1).ok_or("no result for line 1")?;
    assert!(imported["steps"][0]["outputs"]["revision_id"].is_string());
    let failing = results.get(&4).ok_or("no result for line 4")?;
    assert_eq!(failing["steps"][1]["kind"], "retranscribe");
    assert_eq!(failing["steps"][1]["status"], "failed");
    let partial = results.get(&5).ok_or("no result for line 5")?;
    assert_eq!(partial["steps"][1]["status"], "partial");
    assert!(
        partial["steps"][1]["coverage"]["reasons"]
            .as_array()
            .is_some_and(|reasons| !reasons.is_empty())
    );

    // D5: the most severe failure among the lines decides.
    let codes: Vec<FailureCode> = value["items"]
        .as_array()
        .ok_or("no items")?
        .iter()
        .filter_map(|item| item["code"].as_str())
        .filter_map(|code| {
            FailureCode::ALL
                .iter()
                .copied()
                .find(|known| known.identifier() == code)
        })
        .collect();
    let worst = codes
        .iter()
        .copied()
        .min_by_key(|code| severity(*code))
        .ok_or("no failure")?;
    assert_eq!(summary.outcome(), BatchOutcome::Failed(worst), "{value}");
    Ok(())
}
