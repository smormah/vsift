//! What changed on screen through the engine library (ADR 0034).
//!
//! The default tests run everywhere without `FFmpeg`: a session over a
//! stand-in source answers the request checks before any tool. One opt-in
//! test needs `FFmpeg` and `FFprobe` on `PATH` and reads the synthetic F04
//! recording, whose order 1017 changes status at 10 s while every other cell
//! stays as it was (`fixtures/corpus/manifest.json`, F04-E04):
//!
//! A second opt-in test generates a 12 s 720p video with FFmpeg whose two boxes
//! appear and disappear together every second, so every change has two
//! regions and one call reaches the 16-crop bound of `--crops`.
//!
//! `cargo test -p vsift --locked --test engine_screen_changes -- --ignored`

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use vsift::{
    Cancellation, CandidatesRange, Clock, ClockError, Engine, EngineConfig, EngineError,
    EnginePorts, FrameComparison, HostIsolation, IdentifierGenerationError, IdentifierSource,
    IngestRequest, MAX_CROPS_PER_CALL, ManagedRootLocation, OperationId, ScreenChangeCrops,
    ScreenChangeKind, ScreenChangesRequest, ScreenChangesResults, SessionId, SessionRootLocation,
    UserConfigurationLocation,
};
use vsift_domain::{EvidenceItem, EvidenceSubject, FrameDimensions};
use vsift_infrastructure::ExecutableResolver;

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

/// 2027-01-15T08:00:00Z: an arbitrary fixed start for deterministic expiry.
const T0: u64 = 1_800_000_000;
const OWNED_PREFIX: &str = "vsift-engine-screen-changes-test-";
const SECOND: u64 = 1_000_000;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct FixedClock;

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> Result<u64, ClockError> {
        Ok(T0)
    }
}

#[derive(Clone)]
struct SequentialIdentifiers(Arc<AtomicU64>);

impl IdentifierSource for SequentialIdentifiers {
    fn session_id(&self) -> Result<SessionId, IdentifierGenerationError> {
        SessionId::parse(format!(
            "ses_{:032x}",
            self.0.fetch_add(1, Ordering::SeqCst) + 1
        ))
        .map_err(|_| IdentifierGenerationError::NonCanonical)
    }

    fn operation_id(&self) -> Result<OperationId, IdentifierGenerationError> {
        OperationId::parse(format!(
            "op_{:032x}",
            self.0.fetch_add(1, Ordering::SeqCst) + 1
        ))
        .map_err(|_| IdentifierGenerationError::NonCanonical)
    }
}

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

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
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

struct Harness {
    engine: Engine,
    session: SessionId,
    // Last: on Windows a folder something still holds open cannot be removed.
    _root: OwnedRoot,
}

impl Harness {
    async fn open(source: Option<PathBuf>) -> Built<Self> {
        let root = OwnedRoot::new()?;
        let engine = Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(root.path("sessions")),
                user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
                managed_root: ManagedRootLocation::Explicit(root.path("managed")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            EnginePorts::new(
                FixedClock,
                SequentialIdentifiers(Arc::new(AtomicU64::new(0))),
            ),
        );
        let source = if let Some(source) = source {
            source
        } else {
            let stand_in = root.path("stand-in.mp4");
            fs::write(&stand_in, b"\0\0\0\x18ftypisomengine-changes")?;
            stand_in
        };
        let opened = engine
            .ingest(IngestRequest {
                source,
                transcript: None,
                cancellation: Cancellation::new(),
                durability: vsift::DurabilityRequirement::Ephemeral,
            })
            .await?;
        Ok(Self {
            engine,
            session: opened.session.session_id,
            _root: root,
        })
    }

    fn request(&self, from_seconds: u64, to_seconds: u64) -> ScreenChangesRequest {
        ScreenChangesRequest {
            session: self.session.clone(),
            range: CandidatesRange {
                from_micros: from_seconds * SECOND,
                to_micros: to_seconds * SECOND,
            },
            limit: None,
            cursor: None,
            crops: ScreenChangeCrops::NoCrops,
            cancellation: Cancellation::new(),
        }
    }
}

#[tokio::test]
async fn request_checks_answer_before_any_tool() -> TestResult {
    let harness = Harness::open(None).await?;
    let mut reversed = harness.request(10, 10);
    reversed.range.to_micros = 5 * SECOND;
    assert_eq!(
        harness.engine.screen_changes(reversed).await.err(),
        Some(EngineError::InvalidTimeRange)
    );
    let mut too_many = harness.request(0, 10);
    too_many.limit = Some(21);
    assert_eq!(
        harness.engine.screen_changes(too_many).await.err(),
        Some(EngineError::InvalidPageLimit)
    );
    let mut none = harness.request(0, 10);
    none.limit = Some(0);
    assert_eq!(
        harness.engine.screen_changes(none).await.err(),
        Some(EngineError::InvalidPageLimit)
    );
    // A cursor for a session with no visual index cannot continue anything,
    // and must not analyse.
    let mut continued = harness.request(0, 10);
    continued.cursor = Some("not-a-cursor".to_owned());
    assert_eq!(
        harness.engine.screen_changes(continued).await.err(),
        Some(EngineError::CandidateCursorWithoutIndex)
    );
    Ok(())
}

fn fixture(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus/generated")
        .join(relative)
}

fn media_tools_on_path() -> bool {
    let resolver = ExecutableResolver::from_current_path();
    resolver.resolve(OsStr::new("ffmpeg")).is_ok()
        && resolver.resolve(OsStr::new("ffprobe")).is_ok()
}

/// F04: order 1017's status change at 10 s is one small region, compared
/// between two exact frames that `frame get` stored. (F04's scroll is drawn
/// with the pixels of the state before it, #159, so it is no change here.) A repeat names the same changes, frames
/// and regions without storing anything new.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn f04_a_status_change_is_one_small_region_between_two_exact_frames() -> TestResult {
    if !media_tools_on_path() {
        return Err("FFmpeg and FFprobe must be on PATH".into());
    }
    let harness = Harness::open(Some(fixture("F04.mp4"))).await?;
    let mut request = harness.request(0, 14);
    request.limit = Some(20);
    let first = harness.engine.screen_changes(request.clone()).await?;
    assert_eq!(first.next_cursor(), None);
    assert_eq!(first.resume(), None);
    assert_eq!(first.stop(), None);
    let dimensions = first.index().displayed_dimensions();
    let frame_area = u64::from(dimensions.width()) * u64::from(dimensions.height());

    let status = first
        .changes()
        .iter()
        .find(|item| {
            let change = &item.compared.change;
            change.before.as_micros() < 10 * SECOND && 10 * SECOND <= change.after.as_micros()
        })
        .ok_or("no change at the status edit")?;
    assert_eq!(status.compared.change.kind, ScreenChangeKind::Change);
    let FrameComparison::Compared(regions) = &status.compared.comparison else {
        return Err("the status change was not compared".into());
    };
    // The largest region is the status row: a small part of the frame. The
    // re-encoded text around it may add regions of a few pixels each, which
    // come after it (ADR 0034 section 3; the hosted run measures them on real
    // recordings).
    let largest = regions
        .regions
        .first()
        .ok_or("the status change has no region")?;
    let changed = u64::from(largest.rect.width()) * u64::from(largest.rect.height());
    assert!(
        changed * 20 < frame_area,
        "the status edit should change under 5% of the frame, not {changed} of {frame_area} pixels"
    );
    assert!(
        regions
            .regions
            .iter()
            .skip(1)
            .all(|region| region.changed_pixels * 10 < largest.changed_pixels)
    );
    for item in first.changes() {
        assert!(item.compared.before.time < item.compared.after.time);
        assert!(item.id.as_str().starts_with("chg_"));
    }
    assert_eq!(first.frames().len(), first.files().len());

    let again = harness.engine.screen_changes(request.clone()).await?;
    assert_eq!(again.changes(), first.changes());
    assert_eq!(again.frames(), first.frames());

    // With crops, the status row is cut out of both exact frames at the
    // region's own size, and the listing is the same as without.
    request.crops = ScreenChangeCrops::LargestRegions;
    let cropped = harness.engine.screen_changes(request).await?;
    assert_eq!(cropped.changes().len(), first.changes().len());
    assert_eq!(cropped.next_cursor(), first.next_cursor());
    assert_eq!(cropped.stop(), None);
    let summary = cropped.crops().ok_or("no crop summary")?;
    assert_eq!(summary.stop, None);
    assert_eq!(summary.regions_without_crop, 0);
    let status = cropped
        .changes()
        .iter()
        .find(|item| item.id == status.id)
        .ok_or("the status change is not listed with crops")?;
    let crops = status
        .compared
        .crops
        .first()
        .ok_or("the status row has no crops")?;
    for crop in [&crops.before, &crops.after] {
        let crop = crop.as_ref().ok_or("a crop is missing")?;
        assert_eq!(
            item_dimensions(&cropped, crop)?,
            FrameDimensions::new(largest.rect.width(), largest.rect.height())?
        );
    }
    assert_crops_follow_frames(&cropped, summary.made)?;
    Ok(())
}

/// The displayed size of a listed evidence item.
fn item_dimensions(
    results: &ScreenChangesResults,
    id: &vsift::EvidenceId,
) -> Built<FrameDimensions> {
    results
        .frames()
        .iter()
        .find(|item| item.id() == id)
        .and_then(|item| item.subject().image_dimensions())
        .ok_or_else(|| "a crop is not listed with its size".into())
}

/// The page's frames come first and its `made` crops after them, each with
/// one file, in the same order.
fn assert_crops_follow_frames(results: &ScreenChangesResults, made: usize) -> TestResult {
    let frames = results.frames();
    assert_eq!(frames.len(), results.files().len());
    let whole = frames
        .len()
        .checked_sub(made)
        .ok_or("fewer items than crops")?;
    let is_crop = |item: &EvidenceItem| matches!(item.subject(), EvidenceSubject::Crop { .. });
    assert!(frames.iter().take(whole).all(|item| !is_crop(item)));
    assert!(frames.iter().skip(whole).all(is_crop));
    for (item, file) in frames.iter().zip(results.files()) {
        assert_eq!(item.id(), file.evidence_id());
    }
    // Each record once, though a crop may be named by two regions.
    let mut ids: Vec<&str> = frames.iter().map(|item| item.id().as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), frames.len());
    Ok(())
}

/// Generates 12 s of a 1280x720 screen whose two boxes, far apart (more
/// than 40 tiles, where the region rule joins tiles at most 2 apart), are
/// shown in every even second and hidden in every odd one: a change every
/// second, each with two regions of about the same size. The size is a real
/// screen recording's, because each crop decodes a frame of it again.
fn two_box_video(root: &OwnedRoot) -> Built<PathBuf> {
    let video = root.path("two-boxes.mp4");
    let shown = "enable='lt(mod(t,2),1)'";
    let graph = format!(
        "color=c=0x404040:s=1280x720:r=10:d=12,\
         drawbox=x=64:y=64:w=384:h=256:color=white:t=fill:{shown},\
         drawbox=x=832:y=400:w=384:h=256:color=white:t=fill:{shown}"
    );
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &graph,
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&video)
        .status()?;
    if !status.success() {
        return Err("FFmpeg could not generate the two-box video".into());
    }
    Ok(video)
}

/// One call with `--crops` over a page of changes that each have two
/// regions reaches the 16-crop bound: eight regions are cropped from both
/// frames, the rest keep their rectangles, and the page lists the same
/// changes as without crops. Its wall time and the evidence it stored are
/// printed for the pull request (the planner's ruling of 2026-11-04).
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn one_call_reaches_the_sixteen_crop_bound() -> TestResult {
    if !media_tools_on_path() {
        return Err("FFmpeg and FFprobe must be on PATH".into());
    }
    let generated = OwnedRoot::new()?;
    let harness = Harness::open(Some(two_box_video(&generated)?)).await?;
    let mut request = harness.request(0, 12);
    request.limit = Some(10);
    let listed = harness.engine.screen_changes(request.clone()).await?;
    let before = harness
        .engine
        .session_status(&harness.session)?
        .artifact_count();

    request.crops = ScreenChangeCrops::LargestRegions;
    let started = Instant::now();
    let cropped = harness.engine.screen_changes(request).await?;
    let elapsed = started.elapsed();
    let after = harness
        .engine
        .session_status(&harness.session)?
        .artifact_count();
    println!(
        "one --crops call: {} changes, {} crops, {:.1} s, {} evidence artifacts stored (session {} -> {})",
        cropped.changes().len(),
        cropped.crops().map_or(0, |summary| summary.made),
        elapsed.as_secs_f64(),
        after.saturating_sub(before),
        before,
        after
    );

    // The same changes; both pages go on by cursor, and the cursors differ
    // because each is bound to whether crops were asked for.
    let ids = |results: &ScreenChangesResults| {
        results
            .changes()
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&cropped), ids(&listed));
    assert!(cropped.next_cursor().is_some() && listed.next_cursor().is_some());
    assert_ne!(cropped.next_cursor(), listed.next_cursor());
    assert_eq!(cropped.stop(), None);
    let summary = cropped.crops().ok_or("no crop summary")?;
    assert_eq!(summary.made, MAX_CROPS_PER_CALL);
    assert_eq!(summary.stop, Some(vsift::CropStop::CropLimit));
    let two_regions = cropped
        .changes()
        .iter()
        .filter(|item| match &item.compared.comparison {
            FrameComparison::Compared(found) => found.regions.len() >= 2,
            FrameComparison::DimensionsDiffer => false,
        })
        .count();
    assert!(
        two_regions > 4,
        "only {two_regions} changes have two regions"
    );
    assert_eq!(summary.regions_without_crop, 2 * two_regions - 8);
    for item in cropped.changes().iter().take(4) {
        assert!(
            item.compared
                .crops
                .iter()
                .all(vsift::RegionCrops::is_complete)
        );
    }
    assert_crops_follow_frames(&cropped, summary.made)?;
    // A crop takes at most two artifacts, its record and its image. The after
    // frame of one change is the before frame of the next, so the after crop
    // of a region is the next change's before crop of it, an exact repeat
    // that is reused and takes none.
    let stored = after.saturating_sub(before);
    assert!(stored > 0 && stored < 2 * MAX_CROPS_PER_CALL, "{stored}");
    Ok(())
}
