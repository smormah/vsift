//! Opt-in real-`FFmpeg` checks of the P09 media primitives (V-01 and V-06 at
//! the adapter level; the evidence commands arrive in later P09 pull
//! requests).
//!
//! Needs `ffmpeg` and `ffprobe` on `PATH`:
//!
//! ```console
//! cargo test -p vsift-infrastructure --locked --test p09_media_primitives -- --ignored --nocapture
//! ```
//!
//! Expectations come from frozen truth only: the corpus manifest, the
//! generator's documented recipe (F01 is 20 fps with a keyframe every 40
//! frames; F09 keeps frames 0, 7, 14, ... plus 65, 145 and 239 of a 20 fps
//! grid behind a 2 s origin), the independent frame-timestamp lists that
//! `tools/verify_p04_fixtures.py` records with `ffprobe` in
//! `fixtures/corpus/generated/verification.json`, and the recorded P08
//! candidates. Pixels are compared by decoding with `FFmpeg` itself (no new
//! dependency). Clips built at run time use `FFmpeg`'s native `mpeg4` and
//! `aac` encoders, which every pinned build has.

mod candidate_recall;

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use candidate_recall::{RECORDED_FIXTURES, index_recorded, load_recorded, repository};
use serde_json::Value;
use vsift_application::{InitializeSessionStorage, InitializeSessionStorageRequest};
use vsift_domain::{
    CropRect, DurabilityRequirement, FrameDimensions, FrameListing, FrameSelection,
    FrameSelectionError, FrameTolerance, ListingTail, MediaDescription, MediaSelection, MediaTime,
    OperationId, SessionId, TimeRange, select_frame,
};
use vsift_infrastructure::{
    BoundSource, ExecutableResolver, ExtractedImage, FfmpegMedia, FilesystemSessionStore,
    HostIsolation, ImageRegion, MediaError, MediaProviderConformance, ProcessCancellation,
    SourceSnapshot, TrustedExecutable, WAV_HEADER_BYTES, max_frames_per_run,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-p09-media-";
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

/// A fixture staged into a fresh private session store.
struct Staged {
    root: OwnedRoot,
    store: FilesystemSessionStore,
    snapshot: SourceSnapshot,
    conformance: MediaProviderConformance,
}

impl Staged {
    async fn new(source: &Path) -> Built<Self> {
        let resolver = ExecutableResolver::from_current_path();
        let conformance = MediaProviderConformance::r0(
            resolver.resolve(OsStr::new("ffmpeg"))?,
            resolver.resolve(OsStr::new("ffprobe"))?,
        );
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
            source,
        )?;
        Ok(Self {
            root,
            store,
            snapshot,
            conformance,
        })
    }
}

/// The adapter over a staged store. A free function over the two fields, so
/// the snapshot can later be moved into a binding while the store is borrowed.
fn media<'a>(
    conformance: &MediaProviderConformance,
    store: &'a FilesystemSessionStore,
) -> FfmpegMedia<'a> {
    FfmpegMedia::new(conformance.clone(), HostIsolation::ProcessOnly, store)
}

fn fixture(name: &str) -> PathBuf {
    repository(&format!("fixtures/corpus/generated/{name}"))
}

const fn at(micros: u64) -> MediaTime {
    MediaTime::from_micros(micros)
}

fn range(start: u64, end: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(at(start), at(end))?)
}

fn video(description: &MediaDescription) -> Built<(MediaSelection, FrameDimensions)> {
    let (index, dimensions) = description.visual_video_stream()?;
    Ok((
        MediaSelection {
            video: Some(index),
            audio: None,
        },
        dimensions,
    ))
}

fn times(listing: &FrameListing) -> Vec<u64> {
    listing
        .frames()
        .iter()
        .map(|frame| frame.time.as_micros())
        .collect()
}

/// The independent `ffprobe` frame times the fixture verifier recorded.
fn verified_frame_times(fixture: &str) -> Built<Vec<u64>> {
    let report: Value = serde_json::from_slice(&fs::read(repository(
        "fixtures/corpus/generated/verification.json",
    ))?)?;
    report["frame_timestamps_us"][fixture]
        .as_array()
        .ok_or("the verifier recorded no frame times for this fixture")?
        .iter()
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| "frame time is not an integer".into())
        })
        .collect()
}

/// Decodes a PNG with `FFmpeg` into packed 8-bit RGB.
fn decode_png(ffmpeg: &TrustedExecutable, root: &Path, png: &[u8]) -> Built<Vec<u8>> {
    let path = root.join("image.png");
    fs::write(&path, png)?;
    let output = Command::new(ffmpeg.path())
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .output()?;
    if !output.status.success() {
        return Err("FFmpeg could not decode the PNG".into());
    }
    Ok(output.stdout)
}

/// Decodes the displayed frame with this timestamp of `source`'s first video
/// stream into packed 8-bit RGB with `FFmpeg`'s own defaults (display
/// rotation applied).
fn decode_reference(ffmpeg: &TrustedExecutable, source: &Path, pts: i64) -> Built<Vec<u8>> {
    let output = Command::new(ffmpeg.path())
        .args(["-v", "error", "-nostdin", "-copyts", "-i"])
        .arg(source)
        .args(["-map", "0:v:0", "-vf"])
        .arg(format!("select=eq(pts\\,{pts})"))
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .output()?;
    if !output.status.success() {
        return Err("FFmpeg could not decode the reference frame".into());
    }
    Ok(output.stdout)
}

fn region(image: &[u8], width: u32, rect: CropRect) -> Built<Vec<u8>> {
    let width = usize::try_from(width)?;
    let mut bytes = Vec::new();
    for row in 0..usize::try_from(rect.height())? {
        let y = usize::try_from(rect.y())? + row;
        let start = (y * width + usize::try_from(rect.x())?) * 3;
        let end = start + usize::try_from(rect.width())? * 3;
        bytes.extend_from_slice(image.get(start..end).ok_or("region outside the image")?);
    }
    Ok(bytes)
}

/// Mean red, green and blue of packed 8-bit RGB.
fn mean_colour(rgb: &[u8]) -> Built<(u64, u64, u64)> {
    let pixels = u64::try_from(rgb.len() / 3)?;
    if pixels == 0 {
        return Err("empty image".into());
    }
    let mut sums = (0_u64, 0_u64, 0_u64);
    for pixel in rgb.as_chunks::<3>().0 {
        sums.0 += u64::from(pixel[0]);
        sums.1 += u64::from(pixel[1]);
        sums.2 += u64::from(pixel[2]);
    }
    Ok((sums.0 / pixels, sums.1 / pixels, sums.2 / pixels))
}

fn png_size(image: &ExtractedImage) -> Option<(u32, u32)> {
    let width = u32::from_be_bytes(image.png.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(image.png.get(20..24)?.try_into().ok()?);
    Some((width, height))
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
#[allow(clippy::too_many_lines)] // One fixture's V-01 cases read best together.
async fn v01_f01_frames_are_selected_and_extracted_exactly() -> TestResult {
    let staged = Staged::new(&fixture("F01.mp4")).await?;
    let media = media(&staged.conformance, &staged.store);
    let cancel = ProcessCancellation::new;
    let description = media.probe(&staged.snapshot, cancel()).await?;
    let (selection, dimensions) = video(&description)?;
    let bound = BoundSource::bind(staged.snapshot)?;
    let whole = media
        .list_frame_times(
            &bound,
            &description,
            selection,
            range(0, 6_000_000)?,
            cancel(),
        )
        .await?;
    assert_eq!(whole.tail(), ListingTail::EndOfStream);
    assert_eq!(times(&whole), verified_frame_times("F01")?);
    assert_eq!(
        times(&whole),
        (0..120).map(|frame| frame * 50_000).collect::<Vec<_>>()
    );

    let any = FrameTolerance::MAX;
    let pick = |micros, policy| select_frame(&whole, at(micros), policy, any);
    let keyframe = pick(2_000_000, FrameSelection::AtOrAfter)?;
    let before_keyframe = pick(1_950_000, FrameSelection::AtOrAfter)?;
    let between = pick(1_025_000, FrameSelection::AtOrAfter)?;
    let last = pick(5_950_000, FrameSelection::AtOrAfter)?;
    assert_eq!(keyframe.timing.delta_micros(), 0);
    assert_eq!(before_keyframe.timing.delta_micros(), 0);
    assert_eq!(
        (between.timing.actual(), between.timing.delta_micros()),
        (at(1_050_000), 25_000)
    );
    assert_eq!(last.timing.actual(), at(5_950_000));
    assert_eq!(
        pick(5_970_000, FrameSelection::AtOrAfter),
        Err(FrameSelectionError::AfterFinalFrame)
    );
    let displayed = pick(5_970_000, FrameSelection::DisplayedAt)?;
    assert_eq!(
        (displayed.timing.actual(), displayed.timing.delta_micros()),
        (at(5_950_000), -20_000)
    );
    for policy in FrameSelection::ALL {
        assert_eq!(
            pick(6_000_000, policy),
            Err(FrameSelectionError::AtOrAfterEnd)
        );
    }
    assert!(matches!(
        media
            .list_frame_times(
                &bound,
                &description,
                selection,
                range(6_000_000, 6_500_000)?,
                cancel()
            )
            .await,
        Err(MediaError::InvalidFrameRequest)
    ));

    let chosen = [between, before_keyframe, keyframe, last].map(|selected| selected.frame.pts);
    let images = media
        .frames_at(&bound, &description, selection, &chosen, cancel())
        .await?;
    let extracted: Vec<(i64, u64)> = images
        .iter()
        .map(|image| (image.frame.pts, image.frame.time.as_micros()))
        .collect();
    assert_eq!(
        extracted,
        vec![
            (10_752, 1_050_000),
            (19_968, 1_950_000),
            (20_480, 2_000_000),
            (60_928, 5_950_000)
        ]
    );
    for image in &images {
        assert_eq!(image.region, ImageRegion::WholeFrame(dimensions));
        assert_eq!(png_size(image), Some((1_280, 720)));
    }
    // The static slide decodes to the same pixels at every frame.
    let first = decode_png(&staged.conformance.ffmpeg, &staged.root.0, &images[0].png)?;
    let reference = decode_reference(&staged.conformance.ffmpeg, &fixture("F01.mp4"), 10_752)?;
    assert_eq!(first, reference);

    // A timestamp that names no frame is reported, never replaced.
    assert!(matches!(
        media
            .frames_at(&bound, &description, selection, &[10_752, 10_753], cancel())
            .await,
        Err(MediaError::FrameNotFound)
    ));
    for invalid in [
        &[][..],
        &[20_480, 10_752],
        &[0, 512, 1_024, 1_536, 2_048, 2_560, 3_072, 3_584, 4_096],
    ] {
        assert!(matches!(
            media
                .frames_at(&bound, &description, selection, invalid, cancel())
                .await,
            Err(MediaError::InvalidFrameRequest)
        ));
    }
    assert_eq!(max_frames_per_run(dimensions), 8);

    // The single-frame call selects by integer timestamp too.
    let legacy = media
        .frame(
            &bound,
            &description,
            selection,
            at(1_025_000),
            100_000,
            cancel(),
        )
        .await?;
    assert_eq!(
        (legacy.timing.actual(), legacy.timing.delta_micros()),
        (at(1_050_000), 25_000)
    );
    bound.release_verified()?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn v01_f09_variable_frame_rate_and_origin() -> TestResult {
    let staged = Staged::new(&fixture("F09.mkv")).await?;
    let media = media(&staged.conformance, &staged.store);
    let cancel = ProcessCancellation::new;
    let description = media.probe(&staged.snapshot, cancel()).await?;
    assert_eq!(description.origin_micros, 2_000_000);
    let (selection, _) = video(&description)?;
    let bound = BoundSource::bind(staged.snapshot)?;
    let whole = media
        .list_frame_times(
            &bound,
            &description,
            selection,
            range(0, 12_000_000)?,
            cancel(),
        )
        .await?;
    assert_eq!(times(&whole), verified_frame_times("F09")?);
    let near = media
        .list_frame_times(
            &bound,
            &description,
            selection,
            range(3_000_000, 3_600_000)?,
            cancel(),
        )
        .await?;
    assert_eq!(times(&near), vec![3_150_000, 3_250_000, 3_500_000]);
    let selected = select_frame(
        &near,
        at(3_200_000),
        FrameSelection::AtOrAfter,
        FrameTolerance::MAX,
    )?;
    assert_eq!(
        (
            selected.frame.pts,
            selected.timing.actual(),
            selected.timing.delta_micros()
        ),
        (5_250, at(3_250_000), 50_000)
    );
    let displayed = select_frame(
        &near,
        at(3_200_000),
        FrameSelection::DisplayedAt,
        FrameTolerance::MAX,
    )?;
    assert_eq!(displayed.timing.actual(), at(3_150_000));
    let images = media
        .frames_at(
            &bound,
            &description,
            selection,
            &[selected.frame.pts],
            cancel(),
        )
        .await?;
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].frame.time, at(3_250_000));
    bound.release_verified()?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn v06_crops_of_the_rotated_variant_are_exact_displayed_pixels() -> TestResult {
    let source = fixture("F01-rotation-90.mp4");
    let staged = Staged::new(&source).await?;
    let media = media(&staged.conformance, &staged.store);
    let cancel = ProcessCancellation::new;
    let description = media.probe(&staged.snapshot, cancel()).await?;
    let (selection, dimensions) = video(&description)?;
    assert_eq!((dimensions.width(), dimensions.height()), (720, 1_280));
    let bound = BoundSource::bind(staged.snapshot)?;
    let listing = media
        .list_frame_times(
            &bound,
            &description,
            selection,
            range(0, 6_000_000)?,
            cancel(),
        )
        .await?;
    assert_eq!(times(&listing), verified_frame_times("F01-rotation-90")?);
    let pts = 20_480;
    let whole = media
        .frames_at(&bound, &description, selection, &[pts], cancel())
        .await?;
    assert_eq!(whole.first().and_then(png_size), Some((720, 1_280)));
    let reference = decode_reference(&staged.conformance.ffmpeg, &source, pts)?;
    assert_eq!(reference.len(), 720 * 1_280 * 3);
    let extracted = decode_png(
        &staged.conformance.ffmpeg,
        &staged.root.0,
        &whole.first().ok_or("no frame")?.png,
    )?;
    assert_eq!(extracted, reference);
    for rect in [
        CropRect::new(100, 200, 300, 150, dimensions)?,
        CropRect::new(0, 0, 720, 1_280, dimensions)?,
        CropRect::new(719, 1_279, 1, 1, dimensions)?,
        CropRect::new(420, 1_000, 300, 280, dimensions)?,
    ] {
        let crop = media
            .crop_at(&bound, &description, selection, pts, rect, cancel())
            .await?;
        assert_eq!(crop.region, ImageRegion::Crop(rect));
        assert_eq!(crop.frame.time, at(2_000_000));
        let pixels = decode_png(&staged.conformance.ffmpeg, &staged.root.0, &crop.png)?;
        assert_eq!(pixels, region(&reference, 720, rect)?, "{rect:?}");
    }
    // One pixel past the displayed frame is rejected before any I/O.
    assert!(CropRect::new(421, 1_000, 300, 280, dimensions).is_err());
    let encoded = FrameDimensions::new(1_280, 720)?;
    let landscape = CropRect::new(800, 0, 100, 100, encoded)?;
    assert!(matches!(
        media
            .crop_at(&bound, &description, selection, pts, landscape, cancel())
            .await,
        Err(MediaError::CropOutsideFrame)
    ));
    bound.release_verified()?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn v06_the_f03_cell_crop_turns_from_green_to_red() -> TestResult {
    let staged = Staged::new(&fixture("F03.mp4")).await?;
    let media = media(&staged.conformance, &staged.store);
    let cancel = ProcessCancellation::new;
    let description = media.probe(&staged.snapshot, cancel()).await?;
    let (selection, dimensions) = video(&description)?;
    let cell = CropRect::parse("850,420,280,70", dimensions)?;
    let bound = BoundSource::bind(staged.snapshot)?;
    let listing = media
        .list_frame_times(
            &bound,
            &description,
            selection,
            range(3_500_000, 4_500_000)?,
            cancel(),
        )
        .await?;
    let mut colours = Vec::new();
    for request in [3_900_000, 4_000_000] {
        let frame = select_frame(
            &listing,
            at(request),
            FrameSelection::AtOrAfter,
            FrameTolerance::EXACT,
        )?;
        let crop = media
            .crop_at(
                &bound,
                &description,
                selection,
                frame.frame.pts,
                cell,
                cancel(),
            )
            .await?;
        assert_eq!(png_size(&crop), Some((280, 70)));
        colours.push(mean_colour(&decode_png(
            &staged.conformance.ffmpeg,
            &staged.root.0,
            &crop.png,
        )?)?);
    }
    let [(red_before, green_before, _), (red_after, green_after, _)] = colours[..] else {
        return Err("expected two crops".into());
    };
    assert!(green_before > red_before + 40, "{colours:?}");
    assert!(red_after > green_after + 40, "{colours:?}");
    bound.release_verified()?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn every_recorded_candidate_is_extracted_at_its_own_time() -> TestResult {
    let mut checked = 0_usize;
    for name in RECORDED_FIXTURES {
        let file = if name == "F09" {
            "F09.mkv".to_owned()
        } else {
            format!("{name}.mp4")
        };
        let staged = Staged::new(&fixture(&file)).await?;
        let media = media(&staged.conformance, &staged.store);
        let cancel = ProcessCancellation::new;
        let description = media.probe(&staged.snapshot, cancel()).await?;
        let (selection, dimensions) = video(&description)?;
        let index = index_recorded(&load_recorded(name)?).await?;
        let bound = BoundSource::bind(staged.snapshot)?;
        let mut chosen = Vec::new();
        for candidate in index
            .windows()
            .iter()
            .flat_map(vsift_domain::VisualIndexWindow::candidates)
        {
            let time = candidate.representative().as_micros();
            let end = (time + 1_000_000).min(description.duration.as_micros());
            let listing = media
                .list_frame_times(&bound, &description, selection, range(time, end)?, cancel())
                .await?;
            let selected = select_frame(
                &listing,
                at(time),
                FrameSelection::AtOrAfter,
                FrameTolerance::EXACT,
            )?;
            assert_eq!(selected.timing.delta_micros(), 0, "{name} {time}");
            chosen.push(selected.frame.pts);
        }
        chosen.sort_unstable();
        chosen.dedup();
        for batch in chosen.chunks(max_frames_per_run(dimensions)) {
            let images = media
                .frames_at(&bound, &description, selection, batch, cancel())
                .await?;
            for (image, pts) in images.iter().zip(batch) {
                assert_eq!(image.frame.pts, *pts);
                assert_eq!(image.region, ImageRegion::WholeFrame(dimensions));
                assert_eq!(
                    png_size(image),
                    Some((dimensions.width(), dimensions.height()))
                );
                checked += 1;
            }
        }
        bound.release_verified()?;
    }
    println!("{checked} recorded candidates extracted at delta 0");
    assert_eq!(checked, 29);
    Ok(())
}

#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn wav_clips_report_the_first_decoded_sample() -> TestResult {
    for (file, request, expected_start) in [
        ("F01-audio-only.m4a", range(0, 1_000_000)?, 64_000),
        ("F09.mkv", range(0, 2_000_000)?, 750_000),
    ] {
        let staged = Staged::new(&fixture(file)).await?;
        let media = media(&staged.conformance, &staged.store);
        let cancel = ProcessCancellation::new;
        let description = media.probe(&staged.snapshot, cancel()).await?;
        let audio = description
            .streams
            .iter()
            .find(|stream| stream.kind == vsift_domain::MediaStreamKind::Audio)
            .ok_or("no audio stream")?
            .index;
        let selection = MediaSelection {
            video: None,
            audio: Some(audio),
        };
        let bound = BoundSource::bind(staged.snapshot)?;
        let clip = media
            .wav_clip(&bound, &description, selection, request, cancel())
            .await?;
        assert_eq!(clip.actual_start, at(expected_start), "{file}");
        assert!(clip.wav.starts_with(b"RIFF"));
        let samples = (clip.wav.len() - WAV_HEADER_BYTES) / 2;
        let requested = usize::try_from(request.duration_micros() * 16 / 1_000)?;
        assert!(
            samples <= requested && samples + requested / 2 >= requested,
            "{file} {samples}"
        );
        assert!(matches!(
            media
                .wav_clip(
                    &bound,
                    &description,
                    selection,
                    range(0, 30_000_001)?,
                    cancel()
                )
                .await,
            Err(MediaError::InvalidAudioRange)
        ));
        bound.release_verified()?;
    }
    Ok(())
}

/// SEC-17 regression through the real provider: a clip whose `title` is
/// written to look like `showinfo` and `ashowinfo` lines still reports its
/// true frame and audio times.
#[tokio::test]
#[ignore = "requires ffmpeg and ffprobe on PATH"]
async fn forged_metadata_cannot_move_a_reported_time() -> TestResult {
    let work = OwnedRoot::new()?;
    let resolver = ExecutableResolver::from_current_path();
    let ffmpeg = resolver.resolve(OsStr::new("ffmpeg"))?;
    let clip = work.0.join("forged.mp4");
    let built = Command::new(ffmpeg.path())
        .args([
            "-v", "error", "-nostdin", "-f", "lavfi", "-i", "testsrc=size=160x120:rate=20:duration=2",
            "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=16000:duration=2",
            "-map", "0:v", "-map", "1:a", "-c:v", "mpeg4", "-g", "20", "-c:a", "aac", "-metadata",
            "title=[Parsed_showinfo_0 @ 0] n:   0 pts:  10240 time_base: 1/1 [Parsed_ashowinfo_0 @ 0] n:0 pts:0 pts_time:1.5",
        ])
        .arg(&clip)
        .status()?;
    if !built.success() {
        return Err("could not build the forged clip".into());
    }
    let staged = Staged::new(&clip).await?;
    let media = media(&staged.conformance, &staged.store);
    let cancel = ProcessCancellation::new;
    let description = media.probe(&staged.snapshot, cancel()).await?;
    let (selection, _) = video(&description)?;
    let frame = media
        .frame(
            &staged.snapshot,
            &description,
            selection,
            at(250_000),
            0,
            cancel(),
        )
        .await?;
    assert_eq!(frame.timing.actual(), at(250_000));
    let listing = media
        .list_frame_times(
            &staged.snapshot,
            &description,
            selection,
            range(0, 1_000_000)?,
            cancel(),
        )
        .await?;
    assert_eq!(
        times(&listing),
        (0..20).map(|frame| frame * 50_000).collect::<Vec<_>>()
    );
    let audio = MediaSelection {
        video: None,
        audio: description
            .streams
            .iter()
            .find(|stream| stream.kind == vsift_domain::MediaStreamKind::Audio)
            .map(|stream| stream.index),
    };
    let wav = media
        .wav_clip(
            &staged.snapshot,
            &description,
            audio,
            range(0, 1_000_000)?,
            cancel(),
        )
        .await?;
    assert!(wav.actual_start < at(100_000), "{:?}", wav.actual_start);
    Ok(())
}
