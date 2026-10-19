//! Decoding a video's visual samples through the real adapter, as an index
//! extension does (P08). Test-only, shared by the opt-in live checks.
//!
//! `decode_file` stages a copy of the file in a throwaway session store,
//! probes it, and indexes every window through `FfmpegVisualSampler`, keeping
//! what each window's decode returned. The synthetic-corpus checks
//! (`p08_candidates_fixtures`) and the real-media measurement
//! (`p08_real_media_visual`) both use it, so they measure the same samples a
//! `candidates` call would.

// Each test binary that includes this module uses a different part of it.
#![allow(dead_code)]

use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use vsift_application::{
    ExtendVisualIndexRequest, InitializeSessionStorage, InitializeSessionStorageRequest,
    VisualIndexScope, VisualSampler, VisualSamplingError, extend_visual_index,
};
use vsift_domain::{
    DurabilityRequirement, FrameDimensions, MediaSelection, MediaTime, OperationId, SessionId,
    TimeRange, VisualIndex, VisualIndexProfile, VisualSample, VisualWindow,
};
use vsift_infrastructure::{
    BoundSource, ExecutableResolver, FfmpegMedia, FfmpegVisualSampler, FilesystemSessionStore,
    HostIsolation, MediaProviderConformance, ProcessCancellation, SourceSnapshot,
    TrustedExecutable,
};

use crate::candidate_recall::{Built, RecordedFixture, RecordedWindow};

const OWNED_PREFIX: &str = "vsift-p08-candidates-";
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A temporary folder this module created, removed when dropped (only when
/// its name carries the module's own prefix).
pub struct OwnedRoot(pub PathBuf);

impl OwnedRoot {
    /// Creates a new, empty folder under the system's temporary folder.
    pub fn new() -> Built<Self> {
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

/// `ffmpeg` and `ffprobe` from `PATH`.
pub fn tools() -> Built<(TrustedExecutable, TrustedExecutable)> {
    let resolver = ExecutableResolver::from_current_path();
    Ok((
        resolver.resolve(OsStr::new("ffmpeg"))?,
        resolver.resolve(OsStr::new("ffprobe"))?,
    ))
}

/// What one decode of a whole video gave.
pub struct Decoded {
    /// Every window's samples, lead-in included, with the source digest.
    pub recorded: RecordedFixture,
    /// The index the samples built.
    pub index: VisualIndex,
    /// Wall-clock time the indexing took.
    pub elapsed: Duration,
    /// The source's presentation origin (the probe's), in microseconds.
    pub origin_micros: i64,
    /// The sampled video stream's original index.
    pub video_index: u32,
    /// The sampled stream's displayed dimensions.
    pub dimensions: FrameDimensions,
}

/// Indexes every window of `file` through the real adapter
/// (`FfmpegVisualSampler` over a bound copy, as an index extension runs it).
pub async fn decode_file(
    file: &Path,
    label: &str,
    conformance: &MediaProviderConformance,
) -> Built<Decoded> {
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
        file,
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
        return Err(format!("{label} was not indexed completely").into());
    }
    let index = extension.revision.ok_or("no revision")?;
    let mut windows = sampler
        .windows
        .into_inner()
        .map_err(|_| "capture lock poisoned")?;
    windows.sort_by_key(|window| window.ordinal);
    bound.release_verified()?;
    Ok(Decoded {
        recorded: RecordedFixture {
            fixture: label.to_owned(),
            duration: description.duration,
            source_sha256,
            windows,
        },
        index,
        elapsed,
        origin_micros: description.origin_micros,
        video_index: video,
        dimensions: displayed_dimensions,
    })
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

/// The version words of an `ffmpeg -version` answer, without the build's
/// configuration.
pub fn ffmpeg_version(ffmpeg: &TrustedExecutable) -> Built<String> {
    let output = Command::new(ffmpeg.path()).arg("-version").output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next().ok_or("no version line")?;
    // Keep the version words only; builds append their configuration.
    Ok(line.split(" Copyright").next().unwrap_or(line).to_owned())
}

/// Today's civil date (UTC), `YYYY-MM-DD`.
pub fn today() -> Built<String> {
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
