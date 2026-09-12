//! Opt-in cumulative real-provider checkpoint; later packet stages stay visible.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vsift_application::{InitializeSessionStorage, InitializeSessionStorageRequest};
use vsift_domain::{
    DurabilityRequirement, MediaSelection, MediaTime, OperationId, SessionId, TimeRange,
};
use vsift_infrastructure::{
    ExecutableResolver, FfmpegMedia, FilesystemSessionStore, HostIsolation,
    MediaProviderConformance, ProcessCancellation, ProcessRequest, ProcessSupervisor,
    ProcessWorkingDirectory, SourceSnapshot, SupervisorPolicy, TerminationReason,
    TrustedExecutable,
};

type TestResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum StageStatus {
    Passed,
    Failed,
    Blocked,
    NotImplemented,
}

#[derive(Serialize)]
struct StageReport {
    name: &'static str,
    status: StageStatus,
    elapsed_ms: u128,
    diagnostic: Option<String>,
}

#[derive(Serialize)]
struct ScenarioReport {
    fixture: &'static str,
    fixture_sha256: Option<String>,
    source_id: Option<String>,
    frame_sha256: Option<String>,
    audio_sha256: Option<String>,
    requested_frame_us: Option<u64>,
    actual_frame_us: Option<u64>,
    actual_audio_start_us: Option<u64>,
    stage: StageReport,
}

#[derive(Serialize)]
struct RunReport {
    schema_version: u8,
    checkpoint: &'static str,
    revision: Option<String>,
    dirty: Option<bool>,
    manifest_sha256: String,
    os: &'static str,
    architecture: &'static str,
    filesystem: String,
    resource_profile: &'static str,
    vsift_version: &'static str,
    ffmpeg_version: Option<String>,
    ffprobe_version: Option<String>,
    whisper_version: Option<String>,
    model_version: Option<String>,
    client_versions: Vec<String>,
    authorization: &'static str,
    scenarios: Vec<ScenarioReport>,
    future_stages: Vec<StageReport>,
    source_reference_validation: &'static str,
    coverage_gaps: Vec<String>,
    overall: StageStatus,
    complete_journey: StageStatus,
}

struct OwnedRoot(PathBuf);

#[derive(Deserialize)]
struct FixtureProvenance {
    fixtures: Vec<FixtureEntry>,
    media_variants: Vec<FixtureEntry>,
    #[serde(rename = "F11_variants")]
    malformed: Vec<MalformedEntry>,
}

#[derive(Deserialize)]
struct FixtureEntry {
    fixture: String,
    sha256: String,
}

#[derive(Deserialize)]
struct MalformedEntry {
    file: String,
    sha256: String,
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("vsift-p04-e2e-"))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

async fn tool_line(
    executable: TrustedExecutable,
    root: &Path,
    arguments: &[&str],
) -> Option<String> {
    let cwd = ProcessWorkingDirectory::new(root).ok()?;
    let request = ProcessRequest::new(executable, cwd, Duration::from_secs(5))
        .ok()?
        .with_arguments(arguments.iter().copied());
    let limit = NonZeroUsize::new(64 * 1024)?;
    let supervisor = ProcessSupervisor::new(
        SupervisorPolicy::new(limit, Duration::from_secs(1), Duration::from_secs(1)),
        HostIsolation::ProcessOnly,
    );
    let result = supervisor
        .run(request, ProcessCancellation::new())
        .await
        .ok()?;
    if result.termination != TerminationReason::Exited
        || !result.status.success()
        || !result.stdout.complete
    {
        return None;
    }
    std::str::from_utf8(&result.stdout.bytes).ok().map(|text| {
        text.lines()
            .next()
            .map_or_else(String::new, |line| line.chars().take(160).collect())
    })
}

#[allow(clippy::too_many_lines)] // Keep the real source-to-artifact assertions together.
async fn scenario(
    root: &Path,
    name: &'static str,
    ffmpeg: TrustedExecutable,
    ffprobe: TrustedExecutable,
) -> TestResult<ScenarioReport> {
    let started = Instant::now();
    let fixture = root.join("fixtures/corpus/generated").join(name);
    let fixture_sha = digest(&fs::read(&fixture)?);
    let provenance: FixtureProvenance = serde_json::from_slice(&fs::read(
        root.join("fixtures/corpus/generated/provenance.json"),
    )?)?;
    let fixture_id = name.split('.').next().ok_or("fixture name is invalid")?;
    if !provenance
        .fixtures
        .iter()
        .chain(provenance.media_variants.iter())
        .any(|entry| entry.fixture == fixture_id && entry.sha256 == fixture_sha)
        && !provenance
            .malformed
            .iter()
            .any(|entry| entry.file == name && entry.sha256 == fixture_sha)
    {
        return Err("fixture hash differs from reviewed generator provenance".into());
    }
    let sequence = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let private_root =
        OwnedRoot(env::temp_dir().join(format!("vsift-p04-e2e-{}-{sequence}", std::process::id())));
    let store = FilesystemSessionStore::provision_default(&private_root.0)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    let store = FilesystemSessionStore::open_existing(&private_root.0)?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_abcdef0123456789")?,
        &fixture,
    )?;
    if snapshot.original_changed(&fixture)?
        || snapshot.id().as_str() != format!("src_sha256_{fixture_sha}")
    {
        return Err("fixture changed or source SHA-256 differs".into());
    }
    let source_id = snapshot.id().as_str().to_owned();
    let media = FfmpegMedia::new(
        MediaProviderConformance::r0(ffmpeg, ffprobe),
        HostIsolation::ProcessOnly,
        &store,
    );
    if name == "F11-truncated.mp4" {
        let rejection = media.probe(&snapshot, ProcessCancellation::new()).await;
        if !matches!(
            rejection,
            Err(vsift_infrastructure::MediaError::ProviderRejected
                | vsift_infrastructure::MediaError::InvalidMetadata
                | vsift_infrastructure::MediaError::InvalidDuration)
        ) {
            return Err(format!("truncated media rejection differed: {rejection:?}").into());
        }
        return Ok(ScenarioReport {
            fixture: name,
            fixture_sha256: Some(fixture_sha),
            source_id: Some(source_id),
            frame_sha256: None,
            audio_sha256: None,
            requested_frame_us: None,
            actual_frame_us: None,
            actual_audio_start_us: None,
            stage: StageReport {
                name: "p04_source_media",
                status: StageStatus::Passed,
                elapsed_ms: started.elapsed().as_millis(),
                diagnostic: Some("expected truncated-container rejection".to_owned()),
            },
        });
    }
    let description = media.probe(&snapshot, ProcessCancellation::new()).await?;
    let is_vfr = name == "F09.mkv";
    let requested = if is_vfr {
        3_250_000
    } else if name == "F10.mp4" {
        5_000_000
    } else {
        750_000
    };
    let audio_start = if is_vfr { 750_000 } else { 0 };
    let expected_audio_start = if name == "F01-audio-only.m4a" {
        64_000
    } else {
        audio_start
    };
    let selection = MediaSelection {
        video: if name == "F01-audio-only.m4a" {
            None
        } else {
            Some(0)
        },
        audio: if name == "F10.mp4" {
            None
        } else if name == "F01-audio-only.m4a" {
            Some(0)
        } else if name == "F01-multiple-audio.mp4" {
            Some(2)
        } else {
            Some(1)
        },
    };
    description.validate_selection(selection)?;
    let frame = if selection.video.is_some() {
        Some(
            media
                .frame(
                    &snapshot,
                    &description,
                    selection,
                    MediaTime::from_micros(requested),
                    500_000,
                    ProcessCancellation::new(),
                )
                .await?,
        )
    } else {
        None
    };
    if let Some(frame) = &frame {
        if frame.timing.actual().as_micros() != requested {
            return Err("observed frame PTS differs from frozen truth".into());
        }
        if name == "F01-rotation-90.mp4"
            && (frame.dimensions.width(), frame.dimensions.height()) != (720, 1280)
        {
            return Err("displayed orientation differs from portrait fixture truth".into());
        }
    }
    if is_vfr {
        if description.origin_micros != 2_000_000 || description.duration.as_micros() != 12_000_000
        {
            return Err("VFR origin or duration differs from frozen truth".into());
        }
        for (at, observed) in [(7_250_000, 7_250_000), (11_940_000, 11_950_000)] {
            let result = media
                .frame(
                    &snapshot,
                    &description,
                    selection,
                    MediaTime::from_micros(at),
                    150_000,
                    ProcessCancellation::new(),
                )
                .await?;
            if result.timing.actual().as_micros() != observed {
                return Err("VFR boundary/final frame PTS differs from truth".into());
            }
        }
        let out_of_range = media
            .frame(
                &snapshot,
                &description,
                selection,
                MediaTime::from_micros(12_000_000),
                500_000,
                ProcessCancellation::new(),
            )
            .await;
        if !matches!(
            out_of_range,
            Err(vsift_infrastructure::MediaError::NoFrameWithinTolerance)
        ) {
            return Err("out-of-range exact frame was not rejected".into());
        }
    }
    if name == "F10.mp4"
        && description
            .streams
            .iter()
            .any(|stream| stream.kind == vsift_domain::MediaStreamKind::Audio)
    {
        return Err("no-audio fixture unexpectedly reported an audio capability".into());
    }
    if name == "F01-audio-only.m4a"
        && description
            .streams
            .iter()
            .any(|stream| stream.kind == vsift_domain::MediaStreamKind::Video)
    {
        return Err("audio-only fixture unexpectedly reported a video capability".into());
    }
    if name == "F01-rotation-90.mp4"
        && description.streams[0].rotation != vsift_domain::DisplayRotation::Clockwise270
    {
        return Err("counter-clockwise display matrix was normalized incorrectly".into());
    }
    if name == "F01-multiple-audio.mp4"
        && description
            .streams
            .get(2)
            .and_then(|stream| stream.language.as_deref())
            != Some("spa")
    {
        return Err("explicit Spanish audio track was not preserved".into());
    }
    let audio = if selection.audio.is_some() {
        let range = TimeRange::new(
            MediaTime::from_micros(audio_start),
            MediaTime::from_micros(audio_start + 1_000_000),
        )?;
        let chunk = media
            .audio(
                &snapshot,
                &description,
                selection,
                range,
                ProcessCancellation::new(),
            )
            .await?;
        if chunk.actual_start.as_micros() != expected_audio_start {
            return Err("observed audio offset differs from truth".into());
        }
        Some(chunk)
    } else {
        None
    };
    Ok(ScenarioReport {
        fixture: name,
        fixture_sha256: Some(fixture_sha),
        source_id: Some(source_id),
        frame_sha256: frame.as_ref().map(|value| digest(&value.png)),
        audio_sha256: audio.as_ref().map(|value| digest(&value.pcm_s16le)),
        requested_frame_us: frame.as_ref().map(|_| requested),
        actual_frame_us: frame
            .as_ref()
            .map(|value| value.timing.actual().as_micros()),
        actual_audio_start_us: audio.as_ref().map(|value| value.actual_start.as_micros()),
        stage: StageReport {
            name: "p04_source_media",
            status: StageStatus::Passed,
            elapsed_ms: started.elapsed().as_millis(),
            diagnostic: None,
        },
    })
}

fn incomplete(
    name: &'static str,
    status: StageStatus,
    elapsed_ms: u128,
    diagnostic: &str,
) -> ScenarioReport {
    ScenarioReport {
        fixture: name,
        fixture_sha256: None,
        source_id: None,
        frame_sha256: None,
        audio_sha256: None,
        requested_frame_us: None,
        actual_frame_us: None,
        actual_audio_start_us: None,
        stage: StageReport {
            name: "p04_source_media",
            status,
            elapsed_ms,
            diagnostic: Some(diagnostic.chars().take(160).collect()),
        },
    }
}

fn future_stages() -> Vec<StageReport> {
    [
        "p05_session_lifecycle",
        "p06_dependency_remediation",
        "p07_supplied_transcript",
        "p07_local_asr",
        "p08_candidates_search",
        "p09_source_reinspection",
        "p10_recovery",
        "p11_worker_batch",
        "p12_agent_clients",
        "p13_distribution",
        "p14_release_qualification",
    ]
    .into_iter()
    .map(|name| StageReport {
        name,
        status: StageStatus::NotImplemented,
        elapsed_ms: 0,
        diagnostic: None,
    })
    .collect()
}

#[tokio::test]
#[ignore = "opt-in real FFmpeg/FFprobe checkpoint; reports to .vsift/e2e-runs"]
#[allow(clippy::too_many_lines)] // Always write a complete checkpoint report, including later stages.
async fn real_media_checkpoint() -> TestResult<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let report_dir = root
        .join(".vsift/e2e-runs")
        .join(format!("p04-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&report_dir)?;
    let resolver = ExecutableResolver::from_current_path();
    let ffmpeg = resolver.resolve(OsStr::new("ffmpeg"));
    let ffprobe = resolver.resolve(OsStr::new("ffprobe"));
    let git = resolver.resolve(OsStr::new("git")).ok();
    let revision = if let Some(executable) = git.clone() {
        tool_line(executable, &root, &["rev-parse", "HEAD"]).await
    } else {
        None
    };
    let dirty = if let Some(executable) = git {
        tool_line(executable, &root, &["status", "--porcelain"])
            .await
            .map(|line| !line.is_empty())
    } else {
        None
    };
    let ffmpeg_version = if let Some(executable) = ffmpeg.as_ref().ok().cloned() {
        tool_line(executable, &root, &["-version"]).await
    } else {
        None
    };
    let ffprobe_version = if let Some(executable) = ffprobe.as_ref().ok().cloned() {
        tool_line(executable, &root, &["-version"]).await
    } else {
        None
    };
    let mut scenarios = Vec::new();
    let mut failed = false;
    let blocked = ffmpeg.is_err() || ffprobe.is_err();
    for name in [
        "F01.mp4",
        "F09.mkv",
        "F10.mp4",
        "F01-rotation-90.mp4",
        "F01-audio-only.m4a",
        "F01-multiple-audio.mp4",
        "F11-truncated.mp4",
    ] {
        let started = Instant::now();
        let result = if let (Ok(ffmpeg), Ok(ffprobe)) = (&ffmpeg, &ffprobe) {
            scenario(&root, name, ffmpeg.clone(), ffprobe.clone()).await
        } else {
            failed = true;
            scenarios.push(incomplete(
                name,
                StageStatus::Blocked,
                0,
                "Supply FFmpeg and FFprobe on an explicit PATH; no dependency was installed",
            ));
            continue;
        };
        match result {
            Ok(report) => scenarios.push(report),
            Err(error) => {
                failed = true;
                scenarios.push(incomplete(
                    name,
                    StageStatus::Failed,
                    started.elapsed().as_millis(),
                    &error.to_string(),
                ));
            }
        }
    }
    let report = RunReport {
        schema_version: 1,
        checkpoint: "P04",
        revision,
        dirty,
        manifest_sha256: digest(&fs::read(root.join("fixtures/corpus/manifest.json"))?),
        os: env::consts::OS,
        architecture: env::consts::ARCH,
        filesystem: env::var("VSIFT_E2E_FILESYSTEM").map_or_else(
            |_| "uninspected".to_owned(),
            |value| {
                format!(
                    "operator reported: {}",
                    value.chars().take(60).collect::<String>()
                )
            },
        ),
        resource_profile: "desktop-safe: 20 GiB source, 4 h, 16 MP, 4 MiB probe, 64 MiB frame, 10 s PCM, 64 KiB diagnostics",
        vsift_version: env!("CARGO_PKG_VERSION"),
        ffmpeg_version,
        ffprobe_version,
        whisper_version: None,
        model_version: None,
        client_versions: Vec::new(),
        authorization: "opt-in cargo test invocation; no setup action",
        scenarios,
        future_stages: future_stages(),
        source_reference_validation: "SHA-256 and observed frame/audio PTS checked against frozen F01/F09 truth",
        coverage_gaps: vec![
            "P05-P14 stages are not implemented by this checkpoint".to_owned(),
            "Desktop provider process has no hard filesystem, network, or memory sandbox"
                .to_owned(),
        ],
        overall: if blocked {
            StageStatus::Blocked
        } else if failed {
            StageStatus::Failed
        } else {
            StageStatus::Passed
        },
        complete_journey: StageStatus::NotImplemented,
    };
    let output = report_dir.join("report.json");
    fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    println!("P04 checkpoint report: {}", output.display());
    if failed {
        Err("P04 checkpoint failed or was blocked; inspect its report".into())
    } else {
        Ok(())
    }
}
