//! Opt-in cumulative P05 real-media-to-retained-session checkpoint.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    num::NonZeroUsize,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use sha2::{Digest, Sha256};
use vsift_application::{InitializeSessionStorage, InitializeSessionStorageRequest};
use vsift_domain::{
    DurabilityRequirement, MediaSelection, MediaTime, OperationId, SessionArtifactKind, SessionId,
    StorageGeneration, TimeRange,
};
use vsift_infrastructure::{
    BundleSourcePolicy, CleanOutcome, ExecutableResolver, FfmpegMedia, FilesystemSessionStore,
    HostIsolation, MediaProviderConformance, ProcessCancellation, ProcessRequest,
    ProcessSupervisor, ProcessWorkingDirectory, SourceSnapshot, SupervisorPolicy,
    TerminationReason, TrustedExecutable,
};

type TestResult = Result<(), Box<dyn Error>>;

struct OwnedRoot(PathBuf);

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("vsift-p05-e2e-"))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

async fn tool_line(
    executable: TrustedExecutable,
    root: &std::path::Path,
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
    std::str::from_utf8(&result.stdout.bytes).ok().map(|value| {
        value
            .lines()
            .next()
            .map_or_else(String::new, |line| line.chars().take(160).collect())
    })
}

#[tokio::test]
#[ignore = "opt-in native FFmpeg/FFprobe P05 checkpoint; reports to .vsift/e2e-runs"]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the cumulative real-media and session assertions in one checkpoint"
)]
async fn real_media_session_checkpoint() -> TestResult {
    let started = Instant::now();
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_dir = repository
        .join(".vsift/e2e-runs")
        .join(format!("p05-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    let source = repository.join("fixtures/corpus/generated/F01.mp4");
    let source_digest = digest(&fs::read(&source)?);
    let temp =
        OwnedRoot(env::temp_dir().join(format!("vsift-p05-e2e-{}-{stamp}", std::process::id())));
    let workspace = temp.0.join("workspace");
    fs::create_dir(&temp.0)?;
    let resolver = ExecutableResolver::from_current_path();
    let ffmpeg = resolver.resolve(OsStr::new("ffmpeg"))?;
    let ffprobe = resolver.resolve(OsStr::new("ffprobe"))?;
    let ffmpeg_version = tool_line(ffmpeg.clone(), &repository, &["-version"]).await;
    let ffprobe_version = tool_line(ffprobe.clone(), &repository, &["-version"]).await;
    let git = resolver.resolve(OsStr::new("git")).ok();
    let revision = if let Some(executable) = git.clone() {
        tool_line(executable, &repository, &["rev-parse", "HEAD"]).await
    } else {
        None
    };
    let dirty = if let Some(executable) = git {
        tool_line(executable, &repository, &["status", "--porcelain"])
            .await
            .map(|line| !line.is_empty())
    } else {
        None
    };
    let store = FilesystemSessionStore::provision_default(&workspace)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let registration = store.register_session(
        &session_id,
        &OperationId::parse("op_0123456789abcdef")?,
        now,
    )?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    let store = FilesystemSessionStore::open_existing(&workspace)?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        &source,
    )?;
    assert_eq!(
        snapshot.id().as_str(),
        format!("src_sha256_{source_digest}")
    );
    let mut generation = store.activate_source(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        now,
    )?;
    drop(registration);
    let media = FfmpegMedia::new(
        MediaProviderConformance::r0(ffmpeg, ffprobe),
        HostIsolation::ProcessOnly,
        &store,
    );
    let description = media.probe(&snapshot, ProcessCancellation::new()).await?;
    let selection = MediaSelection {
        video: Some(0),
        audio: Some(1),
    };
    description.validate_selection(selection)?;
    let frame = media
        .frame(
            &snapshot,
            &description,
            selection,
            MediaTime::from_micros(750_000),
            500_000,
            ProcessCancellation::new(),
        )
        .await?;
    let audio = media
        .audio(
            &snapshot,
            &description,
            selection,
            TimeRange::new(MediaTime::from_micros(0), MediaTime::from_micros(1_000_000))?,
            ProcessCancellation::new(),
        )
        .await?;
    assert_eq!(frame.timing.actual().as_micros(), 750_000);
    generation = store.publish_artifact(
        &session_id,
        &OperationId::parse("op_3333333333333333")?,
        generation,
        SessionArtifactKind::FramePng,
        &frame.png,
        now,
    )?;
    generation = store.publish_artifact(
        &session_id,
        &OperationId::parse("op_4444444444444444")?,
        generation,
        SessionArtifactKind::AudioPcm,
        &audio.pcm_s16le,
        now,
    )?;
    let status = store.session_status(&session_id)?;
    assert_eq!(status.generation(), generation);
    assert_eq!(status.artifact_count(), 2);
    let evidence = temp.0.join("evidence-only");
    let portable = temp.0.join("portable");
    let evidence_status =
        store.retain_bundle(&session_id, &evidence, BundleSourcePolicy::EvidenceOnly)?;
    let portable_status =
        store.retain_bundle(&session_id, &portable, BundleSourcePolicy::IncludeSource)?;
    assert_eq!(evidence_status.artifact_count(), 2);
    assert_eq!(portable_status.artifact_count(), 2);
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&evidence)?,
        evidence_status
    );
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&portable)?,
        portable_status
    );
    assert!(!evidence.join("source.media").exists());
    assert_eq!(
        digest(&fs::read(portable.join("source.media"))?),
        source_digest
    );
    drop(snapshot);
    store.close_session(
        &session_id,
        &OperationId::parse("op_5555555555555555")?,
        generation,
    )?;
    assert_eq!(
        store.clean_session(&session_id, now, false)?,
        CleanOutcome::Removed
    );
    assert_eq!(digest(&fs::read(&source)?), source_digest);
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&portable)?,
        portable_status
    );
    let future_stages: Vec<_> = [
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
    .map(|name| json!({"name": name, "status": "not_implemented"}))
    .collect();
    let report = json!({
        "schema_version": 1,
        "checkpoint": "P05",
        "revision": revision,
        "dirty": dirty,
        "fixture_manifest_sha256": digest(&fs::read(repository.join("fixtures/corpus/manifest.json"))?),
        "fixture": "F01.mp4",
        "fixture_sha256": source_digest,
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "filesystem": env::var("VSIFT_E2E_FILESYSTEM").map_or_else(
            |_| "uninspected".to_owned(),
            |value| format!("operator reported: {}", value.chars().take(60).collect::<String>()),
        ),
        "resource_profile": "desktop-safe: 20 GiB source, 10 GiB evidence, 64 MiB frame, 4 MiB audio, bounded process and manifest output",
        "vsift_version": env!("CARGO_PKG_VERSION"),
        "ffmpeg_version": ffmpeg_version,
        "ffprobe_version": ffprobe_version,
        "whisper_version": null,
        "model_version": null,
        "client_versions": [],
        "authorization": "opt-in cargo test invocation; no setup or persistent registration",
        "source_id": status.source_id().as_str(),
        "frame_actual_time_us": frame.timing.actual().as_micros(),
        "frame_sha256": digest(&frame.png),
        "audio_sha256": digest(&audio.pcm_s16le),
        "p04_source_media": {"status": "passed"},
        "p05_session_lifecycle": {"status": "passed", "elapsed_ms": started.elapsed().as_millis()},
        "retained_source_included": true,
        "retained_evidence_only": true,
        "source_preserved": true,
        "source_reference_validation": "fixture, private source and retained source SHA-256 matched; actual frame PTS checked",
        "coverage_gaps": [
            "P06-P14 stages are not implemented by this checkpoint",
            "Desktop provider process has no hard filesystem, network or memory sandbox",
            "Strict OS/storage crash durability is not qualified"
        ],
        "future_stages": future_stages,
        "overall": "passed",
        "complete_journey": "not_implemented",
    });
    let report_path = run_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("P05 checkpoint report: {}", report_path.display());
    Ok(())
}
