//! Opt-in live checks of the visual index over the synthetic corpus (P08).
//!
//! Needs `ffmpeg` and `ffprobe` on `PATH`:
//!
//! ```console
//! cargo test --release -p vsift-infrastructure --locked --test p08_candidates_fixtures -- --ignored --nocapture
//! ```
//!
//! - `p08_candidates_fixtures` decodes every visual fixture through
//!   `FfmpegMedia::visual_samples` exactly as an index extension does,
//!   scores the candidates against the frozen truth with the same gate as the
//!   always-run recorded-sample test, and reports the decode throughput in
//!   media seconds per wall-clock second.
//! - `records_visual_samples_and_reports_drift` decodes the same fixtures and
//!   reports how far the result differs from the committed recorded samples
//!   in `tests/data/visual_samples/`. With `VSIFT_RECORD_VISUAL_SAMPLES=1` it
//!   rewrites them, with the `FFmpeg` version and date as provenance; review
//!   the diff and the recall report before committing a re-recording.

mod candidate_recall;

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use candidate_recall::{
    Built, RECORDED_FIXTURES, RecordedFixture, RecordedWindow, index_recorded, load_recorded,
    manifest, recall_report, recorded_path, recorded_text, repository,
};
use serde_json::{Value, json};
use vsift_application::{
    ExtendVisualIndexRequest, InitializeSessionStorage, InitializeSessionStorageRequest,
    VisualIndexScope, VisualSampler, VisualSamplingError, extend_visual_index,
};
use vsift_domain::{
    DurabilityRequirement, MediaSelection, MediaTime, OperationId, SessionId, TimeRange,
    VisualIndex, VisualIndexProfile, VisualSample, VisualWindow,
};
use vsift_infrastructure::{
    BoundSource, ExecutableResolver, FfmpegMedia, FfmpegVisualSampler, FilesystemSessionStore,
    HostIsolation, MediaProviderConformance, ProcessCancellation, SourceSnapshot,
    TrustedExecutable,
};

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-p08-candidates-";
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for OwnedRoot {
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

fn tools() -> Built<(TrustedExecutable, TrustedExecutable)> {
    let resolver = ExecutableResolver::from_current_path();
    Ok((
        resolver.resolve(OsStr::new("ffmpeg"))?,
        resolver.resolve(OsStr::new("ffprobe"))?,
    ))
}

fn fixture_file(fixture: &str) -> String {
    let extension = if fixture == "F09" { "mkv" } else { "mp4" };
    format!("fixtures/corpus/generated/{fixture}.{extension}")
}

/// Indexes every window of one fixture through the real adapter
/// (`FfmpegVisualSampler` over a bound copy, as an index extension runs it);
/// returns the decoded samples, the index and the time spent indexing.
async fn decode_fixture(
    fixture: &str,
    conformance: &MediaProviderConformance,
) -> Built<(RecordedFixture, VisualIndex, Duration)> {
    let root = OwnedRoot::new()?;
    let workspace = root.0.join("workspace");
    let store = FilesystemSessionStore::provision_default(&workspace)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    let registration = store.register_session(
        &session_id,
        &OperationId::parse("op_0123456789abcdef")?,
        1_000,
    )?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    drop(registration);
    let store = FilesystemSessionStore::open_existing(&workspace)?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        &repository(&fixture_file(fixture)),
    )?;
    let media = FfmpegMedia::new(conformance.clone(), HostIsolation::ProcessOnly, &store);
    let description = media.probe(&snapshot, ProcessCancellation::new()).await?;
    let (video, displayed_dimensions) = description.visual_video_stream()?;
    let selection = MediaSelection {
        video: Some(video),
        audio: None,
    };
    let source_sha256 = snapshot
        .id()
        .as_str()
        .trim_start_matches("src_sha256_")
        .to_owned();
    let source_id = snapshot.id().clone();
    let bound = BoundSource::bind(snapshot)?;
    let sampler = Capturing {
        inner: FfmpegVisualSampler::new(
            &media,
            &bound,
            &description,
            selection,
            ProcessCancellation::new(),
        ),
        windows: Mutex::new(Vec::new()),
    };
    let started = Instant::now();
    let extension = extend_visual_index(
        ExtendVisualIndexRequest {
            scope: VisualIndexScope {
                session_id: &session_id,
                source_id: &source_id,
                stream_index: video,
                displayed_dimensions,
                duration: description.duration,
                profile: VisualIndexProfile::R0,
            },
            previous: None,
            range: TimeRange::new(MediaTime::from_micros(0), description.duration)?,
        },
        &sampler,
    )
    .await?;
    let elapsed = started.elapsed();
    if extension.stop.is_some() {
        return Err("the fixture was not indexed completely".into());
    }
    let index = extension.revision.ok_or("no revision")?;
    let windows = sampler
        .windows
        .into_inner()
        .map_err(|_| "capture lock poisoned")?;
    bound.release_verified()?;
    Ok((
        RecordedFixture {
            fixture: fixture.to_owned(),
            duration: description.duration,
            source_sha256,
            windows,
        },
        index,
        elapsed,
    ))
}

/// Passes windows through to the real adapter and keeps what it returned.
struct Capturing<S> {
    inner: S,
    windows: Mutex<Vec<RecordedWindow>>,
}

impl<S: VisualSampler> VisualSampler for Capturing<S> {
    async fn window_samples(
        &self,
        window: VisualWindow,
    ) -> Result<Vec<VisualSample>, VisualSamplingError> {
        let samples = self.inner.window_samples(window).await?;
        if let Ok(mut windows) = self.windows.lock() {
            windows.push(RecordedWindow {
                ordinal: window.ordinal(),
                samples: samples.clone(),
            });
        }
        Ok(samples)
    }
}

fn ffmpeg_version(ffmpeg: &TrustedExecutable) -> Built<String> {
    let output = Command::new(ffmpeg.path()).arg("-version").output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next().ok_or("no version line")?;
    // Keep the version words only; builds append their configuration.
    Ok(line.split(" Copyright").next().unwrap_or(line).to_owned())
}

fn today() -> Built<String> {
    let days = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() / 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = i64::try_from(days)? + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}

/// Counts samples whose time, blocks or hash differ from the committed ones.
fn drift(committed: &RecordedFixture, fresh: &RecordedFixture) -> Value {
    let committed_samples: Vec<&VisualSample> = committed
        .windows
        .iter()
        .flat_map(|window| window.samples.iter())
        .collect();
    let fresh_samples: Vec<&VisualSample> = fresh
        .windows
        .iter()
        .flat_map(|window| window.samples.iter())
        .collect();
    let paired = committed_samples.iter().zip(&fresh_samples);
    let time_drift = paired
        .clone()
        .filter(|(left, right)| left.time() != right.time())
        .count();
    let block_drift = paired
        .clone()
        .filter(|(left, right)| left.blocks() != right.blocks())
        .count();
    let largest_block_delta = paired
        .clone()
        .flat_map(|(left, right)| {
            left.blocks()
                .iter()
                .zip(right.blocks())
                .map(|(a, b)| a.abs_diff(*b))
        })
        .max()
        .unwrap_or(0);
    let hash_drift = paired
        .filter(|(left, right)| left.hash() != right.hash())
        .count();
    json!({
        "fixture": fresh.fixture,
        "source_changed": committed.source_sha256 != fresh.source_sha256,
        "committed_samples": committed_samples.len(),
        "fresh_samples": fresh_samples.len(),
        "time_drift": time_drift,
        "block_drift": block_drift,
        "largest_block_delta": largest_block_delta,
        "hash_drift": hash_drift,
    })
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn records_visual_samples_and_reports_drift() -> TestResult {
    let (ffmpeg, ffprobe) = tools()?;
    let provenance = json!({
        "ffmpeg": ffmpeg_version(&ffmpeg)?,
        "operation": "FfmpegMedia::visual_samples",
        "profile": "r0-visual-v1: 60 s windows with a 0.5 s lead-in, select >= 0.5 s apart, scale=128:72:flags=area, format=gray, showinfo, -fps_mode passthrough, rawvideo gray",
        "recorded": today()?,
    });
    let conformance = MediaProviderConformance::r0(ffmpeg, ffprobe);
    let write = env::var("VSIFT_RECORD_VISUAL_SAMPLES").is_ok_and(|value| value == "1");
    let mut report = Vec::new();
    for fixture in RECORDED_FIXTURES {
        let (fresh, _, _) = decode_fixture(fixture, &conformance).await?;
        match load_recorded(fixture) {
            Ok(committed) => report.push(drift(&committed, &fresh)),
            Err(_) => report.push(json!({ "fixture": fixture, "committed": null })),
        }
        if write {
            let path = recorded_path(fixture);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, recorded_text(&fresh, &provenance)?)?;
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "provenance": provenance,
            "written": write,
            "drift": report,
        }))?
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn p08_candidates_fixtures() -> TestResult {
    let (ffmpeg, ffprobe) = tools()?;
    let conformance = MediaProviderConformance::r0(ffmpeg, ffprobe);
    let truth = manifest()?;
    let mut indexed = Vec::new();
    let mut decode_time = Duration::ZERO;
    for fixture in RECORDED_FIXTURES {
        let (fresh, index, elapsed) = decode_fixture(fixture, &conformance).await?;
        decode_time += elapsed;
        // The live index must equal the one the recorded-sample replay builds
        // from the same samples, apart from the session its identities name.
        let replayed = index_recorded(&fresh).await?;
        if replayed.windows().len() != index.windows().len()
            || replayed
                .windows()
                .iter()
                .zip(index.windows())
                .any(|(left, right)| {
                    left.candidates().len() != right.candidates().len()
                        || left
                            .candidates()
                            .iter()
                            .zip(right.candidates())
                            .any(|(a, b)| a.draft() != b.draft())
                })
        {
            return Err(format!("{fixture}: the live index differs from its replay").into());
        }
        indexed.push((fresh, index));
    }
    let report = recall_report(&truth, &indexed)?;
    let media_seconds = indexed
        .iter()
        .map(|(recorded, _)| recorded.duration.as_micros())
        .sum::<u64>();
    let throughput = f64::from(u32::try_from(media_seconds / 1_000)?)
        / 1_000.0
        / decode_time.as_secs_f64().max(f64::EPSILON);
    let mut json = report.json;
    json["throughput_media_seconds_per_second"] = json!(throughput);
    json["decode_seconds"] = json!(decode_time.as_secs_f64());
    println!("{}", serde_json::to_string_pretty(&json)?);
    assert!(report.failures.is_empty(), "{:#?}", report.failures);
    Ok(())
}
