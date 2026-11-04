//! What changed on screen, measured on real screen recordings (ADR 0034).
//!
//! `changes` is run over the whole of each fetched item, page by page: a
//! cursor while one continues, a fresh call from `resume` when the analysed
//! part ends, and a new session when the evidence room is full. The report
//! holds numbers only (item ids, pinned digests, counts, times and shares),
//! never a pixel, a path or text:
//!
//! - how many changes, a minute, and how many calls and sessions they took;
//! - regions per change, the share of changes with no region at the
//!   threshold, the share of the frame the largest region covers, and how
//!   many regions are small (fewer than [`SMALL_REGION_PIXELS`] changed
//!   pixels), which on the synthetic corpus are codec noise around text;
//! - with a person's `labels.changes_us` in the manifest, the share of
//!   labelled changes a listed change's window holds (exactly, and within a
//!   second), and the listed changes no label falls near. Without labels the
//!   report says the score waits for them.
//!
//! Opt-in and hosted: the `P14 real media` workflow runs it with
//! `mode: changes` (never on a development machine). With `ffmpeg` and
//! `ffprobe` on `PATH`:
//!
//! ```console
//! VSIFT_REAL_MEDIA_DIR=<the fetched items> VSIFT_SCREEN_CHANGES_OUT=<file.json> \
//!   [VSIFT_REAL_MEDIA_ITEMS="RM01 RM04 RM10"] \
//!   cargo test -p vsift --locked --test real_media_screen_changes \
//!   -- --ignored --exact measures_screen_changes_on_real_recordings --nocapture
//! ```

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use vsift::{
    Cancellation, CandidatesRange, Engine, EngineConfig, EnginePorts, FrameComparison,
    HostIsolation, IngestRequest, ManagedRootLocation, ScreenChangeCrops, ScreenChangeKind,
    ScreenChangeResumeReason, ScreenChangesRequest, ScreenChangesResults, SessionId,
    SessionRootLocation, UserConfigurationLocation,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

/// A region with fewer changed pixels than this is counted as small.
const SMALL_REGION_PIXELS: u64 = 32;
/// How far a label may lie outside a change's window and still count loosely.
const LOOSE_MICROS: u64 = 1_000_000;
/// Most calls for one item: a guard against a loop that makes no progress.
const MAX_CALLS: usize = 2_000;
const OWNED_PREFIX: &str = "vsift-real-media-changes-";

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

/// One listed change, as numbers.
#[derive(Clone, Debug, PartialEq)]
struct Listed {
    before_us: u64,
    after_us: u64,
    motion: bool,
    regions: usize,
    regions_left_out: u32,
    small_regions: usize,
    largest_share: f64,
    no_region: bool,
}

fn listed_of(results: &ScreenChangesResults) -> Vec<Listed> {
    let dimensions = results.index().displayed_dimensions();
    let frame = (u64::from(dimensions.width()) * u64::from(dimensions.height())).max(1);
    results
        .changes()
        .iter()
        .map(|item| {
            let change = &item.compared.change;
            let (regions, left_out, small, largest, no_region) = match &item.compared.comparison {
                FrameComparison::Compared(found) => (
                    found.regions.len(),
                    found.regions_left_out,
                    found
                        .regions
                        .iter()
                        .filter(|region| region.changed_pixels < SMALL_REGION_PIXELS)
                        .count(),
                    found.regions.first().map_or(0.0, |region| {
                        ratio(
                            u64::from(region.rect.width()) * u64::from(region.rect.height()),
                            frame,
                        )
                    }),
                    found.changed_area.is_none(),
                ),
                FrameComparison::DimensionsDiffer => (0, 0, 0, 0.0, false),
            };
            Listed {
                before_us: change.before.as_micros(),
                after_us: change.after.as_micros(),
                motion: matches!(change.kind, ScreenChangeKind::Motion(_)),
                regions,
                regions_left_out: left_out,
                small_regions: small,
                largest_share: largest,
                no_region,
            }
        })
        .collect()
}

#[allow(
    clippy::cast_precision_loss,
    reason = "a share of counts far below 2^52, for a report"
)]
fn ratio(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// The labelled changes a listed change's window holds, exactly and within
/// [`LOOSE_MICROS`], and the listed changes no label falls near.
fn score(listed: &[Listed], labels: &[u64]) -> Value {
    let holds = |change: &Listed, label: u64, slack: u64| {
        change.before_us.saturating_sub(slack) < label && label <= change.after_us + slack
    };
    let exact = labels
        .iter()
        .filter(|label| listed.iter().any(|change| holds(change, **label, 0)))
        .count();
    let loose = labels
        .iter()
        .filter(|label| {
            listed
                .iter()
                .any(|change| holds(change, **label, LOOSE_MICROS))
        })
        .count();
    let unlabelled = listed
        .iter()
        .filter(|change| {
            !labels
                .iter()
                .any(|label| holds(change, *label, LOOSE_MICROS))
        })
        .count();
    json!({
        "labels": labels.len(),
        "found_exact": exact,
        "found_within_1s": loose,
        "recall_exact": ratio(exact as u64, labels.len() as u64),
        "recall_within_1s": ratio(loose as u64, labels.len() as u64),
        "listed_without_label_within_1s": unlabelled,
    })
}

fn quantiles(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    let at = |share: f64| {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "an index into a short sorted list"
        )]
        let index = ((values.len().saturating_sub(1)) as f64 * share).round() as usize;
        values.get(index).copied()
    };
    json!({ "p50": at(0.5), "p90": at(0.9), "max": at(1.0) })
}

fn summary(listed: &[Listed], duration_us: u64) -> Value {
    let count = listed.len() as u64;
    let with_small = listed
        .iter()
        .filter(|change| change.small_regions > 0)
        .count() as u64;
    let regions: usize = listed.iter().map(|change| change.regions).sum();
    let small: usize = listed.iter().map(|change| change.small_regions).sum();
    json!({
        "changes": count,
        "per_minute": ratio(count * 60_000_000, duration_us),
        "motion": listed.iter().filter(|change| change.motion).count(),
        "no_region_at_threshold": listed.iter().filter(|change| change.no_region).count(),
        "regions_per_change": quantiles(listed.iter().map(|change| {
            #[allow(clippy::cast_precision_loss, reason = "a count of at most eight")]
            let regions = change.regions as f64;
            regions
        }).collect()),
        "changes_with_regions_left_out": listed.iter().filter(|change| change.regions_left_out > 0).count(),
        "regions": regions,
        "small_regions": small,
        "share_of_regions_small": ratio(small as u64, regions as u64),
        "share_of_changes_with_a_small_region": ratio(with_small, count),
        "largest_region_frame_share": quantiles(listed.iter().map(|change| change.largest_share).collect()),
    })
}

struct Item {
    id: String,
    /// The pinned digest the file must have; `None` for a corpus fixture.
    sha256: Option<String>,
    file: PathBuf,
    labels: Option<Vec<u64>>,
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn fetched_items(media: &Path, wanted: &[String]) -> Built<(Vec<Item>, Vec<String>)> {
    let manifest: Value = serde_json::from_slice(&fs::read(repository(
        "tools/p14-campaigns/real-media/manifest.json",
    ))?)?;
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    for item in manifest["items"]
        .as_array()
        .ok_or("the manifest has no items")?
    {
        let id = item["id"].as_str().ok_or("an item has no id")?.to_owned();
        if !wanted.is_empty() && !wanted.contains(&id) {
            continue;
        }
        let (Some("approved"), Some(file_name), Some(sha256)) = (
            item["status"].as_str(),
            item["file_name"].as_str(),
            item["sha256"].as_str(),
        ) else {
            skipped.push(format!("{id}: not approved and pinned"));
            continue;
        };
        let file = media.join(&id).join(file_name);
        if !file.is_file() {
            skipped.push(format!("{id}: not fetched"));
            continue;
        }
        let labels = item["labels"]["changes_us"]
            .as_array()
            .map(|times| times.iter().filter_map(Value::as_u64).collect());
        items.push(Item {
            id,
            sha256: Some(sha256.to_owned()),
            file,
            labels,
        });
    }
    Ok((items, skipped))
}

fn engine(root: &OwnedRoot) -> Engine {
    Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(root.0.join("sessions")),
            user_configuration: UserConfigurationLocation::Explicit(root.0.join("config")),
            managed_root: ManagedRootLocation::Explicit(root.0.join("managed")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::system(),
    )
}

async fn open_session(engine: &Engine, item: &Item) -> Built<SessionId> {
    let opened = engine
        .ingest(IngestRequest {
            source: item.file.clone(),
            transcript: None,
            cancellation: Cancellation::new(),
            durability: vsift::DurabilityRequirement::Ephemeral,
        })
        .await?;
    if let Some(pinned) = &item.sha256
        && opened.session.source_id.as_str() != format!("src_sha256_{pinned}")
    {
        return Err(format!("{}: the fetched file is not the pinned one", item.id).into());
    }
    Ok(opened.session.session_id)
}

async fn measure_item(item: &Item) -> Built<Value> {
    let root = OwnedRoot::new()?;
    let engine = engine(&root);
    let started = Instant::now();
    let mut session = open_session(&engine, item).await?;
    let mut sessions = 1;
    let mut from = 0;
    let mut cursor: Option<String> = None;
    let mut listed: Vec<Listed> = Vec::new();
    let mut stops = Vec::new();
    let mut duration_us;
    let mut calls = 0;
    loop {
        calls += 1;
        if calls > MAX_CALLS {
            return Err(format!("{}: no progress after {MAX_CALLS} calls", item.id).into());
        }
        let results = engine
            .screen_changes(ScreenChangesRequest {
                session: session.clone(),
                range: CandidatesRange {
                    from_micros: from,
                    to_micros: u64::MAX / 2,
                },
                limit: Some(20),
                cursor: cursor.take(),
                crops: ScreenChangeCrops::NoCrops,
                cancellation: Cancellation::new(),
            })
            .await?;
        duration_us = results.index().duration().as_micros();
        for change in listed_of(&results) {
            if !listed.iter().any(|known| known.after_us == change.after_us) {
                listed.push(change);
            }
        }
        if let Some(next) = results.next_cursor() {
            cursor = Some(next.to_owned());
            continue;
        }
        let Some(resume) = results.resume() else {
            break;
        };
        stops.push(resume.reason.identifier());
        if resume.from.as_micros() < from {
            return Err(format!("{}: resume went backwards", item.id).into());
        }
        if resume.reason == ScreenChangeResumeReason::SessionEvidenceBudget {
            session = open_session(&engine, item).await?;
            sessions += 1;
        }
        from = resume.from.as_micros();
    }
    listed.sort_by_key(|change| change.after_us);
    Ok(json!({
        "id": item.id,
        "sha256": item.sha256,
        "duration_us": duration_us,
        "calls": calls,
        "sessions": sessions,
        "seconds": started.elapsed().as_secs_f64(),
        "resume_reasons": stops,
        "summary": summary(&listed, duration_us),
        "score": item.labels.as_ref().map_or_else(
            || json!("waits for the maintainer's change-time labels"),
            |labels| score(&listed, labels),
        ),
        "change_times_us": listed.iter().map(|change| [change.before_us, change.after_us]).collect::<Vec<_>>(),
    }))
}

#[tokio::test]
#[ignore = "hosted only: requires the fetched real-media items and FFmpeg on PATH"]
async fn measures_screen_changes_on_real_recordings() -> TestResult {
    let media = PathBuf::from(env::var("VSIFT_REAL_MEDIA_DIR")?);
    let output = PathBuf::from(env::var("VSIFT_SCREEN_CHANGES_OUT")?);
    if !media.is_absolute() || !output.is_absolute() {
        return Err("VSIFT_REAL_MEDIA_DIR and VSIFT_SCREEN_CHANGES_OUT must be absolute".into());
    }
    let wanted: Vec<String> = env::var("VSIFT_REAL_MEDIA_ITEMS")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let (items, skipped) = fetched_items(&media, &wanted)?;
    if items.is_empty() {
        return Err(format!("no fetched item to measure ({skipped:?})").into());
    }
    let mut measured = Vec::new();
    for item in &items {
        let report = measure_item(item).await?;
        println!(
            "{}: calls {} sessions {} seconds {:.1} {}",
            item.id,
            report["calls"],
            report["sessions"],
            report["seconds"].as_f64().unwrap_or_default(),
            serde_json::to_string(&report["summary"])?
        );
        measured.push(report);
    }
    let report = json!({
        "profile": "screen-change-regions-v1",
        "small_region_pixels": SMALL_REGION_PIXELS,
        "skipped": skipped,
        "items": measured,
    });
    fs::write(&output, format!("{}\n", serde_json::to_string(&report)?))?;
    Ok(())
}

/// The same loop over the synthetic F04 (14 s): the control, cheap enough to
/// run on a development machine with `FFmpeg` on `PATH`. Its status edit at
/// 10 s is labelled from the corpus truth (F04-E04).
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn measures_screen_changes_on_the_synthetic_corpus() -> TestResult {
    let item = Item {
        id: "F04".to_owned(),
        sha256: None,
        file: repository("fixtures/corpus/generated/F04.mp4"),
        labels: Some(vec![10_000_000]),
    };
    let report = measure_item(&item).await?;
    println!("{}", serde_json::to_string(&report)?);
    assert_eq!(report["sessions"], 1);
    assert_eq!(report["score"]["found_exact"], 1);
    assert!(
        report["summary"]["changes"]
            .as_u64()
            .is_some_and(|count| count >= 1)
    );
    Ok(())
}

fn change(before_us: u64, after_us: u64) -> Listed {
    Listed {
        before_us,
        after_us,
        motion: false,
        regions: 1,
        regions_left_out: 0,
        small_regions: 0,
        largest_share: 0.1,
        no_region: false,
    }
}

#[test]
fn a_label_is_found_in_a_change_window_exactly_or_within_a_second() {
    let listed = [
        change(9_500_000, 10_000_000),
        change(20_000_000, 20_500_000),
    ];
    let report = score(&listed, &[10_000_000, 20_800_000, 40_000_000]);
    assert_eq!(report["labels"], 3);
    assert_eq!(report["found_exact"], 1);
    assert_eq!(report["found_within_1s"], 2);
    assert_eq!(report["listed_without_label_within_1s"], 0);
    let report = score(&listed, &[]);
    assert_eq!(report["listed_without_label_within_1s"], 2);
    assert_eq!(report["recall_exact"], 0.0);
}

#[test]
fn the_summary_counts_small_regions_and_missing_ones() {
    let mut noisy = change(0, 500_000);
    noisy.small_regions = 2;
    noisy.regions = 3;
    let mut empty = change(1_000_000, 1_500_000);
    empty.no_region = true;
    empty.regions = 0;
    let report = summary(&[noisy, empty], 60_000_000);
    assert_eq!(report["changes"], 2);
    assert_eq!(report["per_minute"], 2.0);
    assert_eq!(report["small_regions"], 2);
    assert_eq!(report["regions"], 3);
    assert_eq!(report["share_of_regions_small"], 2.0 / 3.0);
    assert_eq!(report["no_region_at_threshold"], 1);
    assert_eq!(report["share_of_changes_with_a_small_region"], 0.5);
}
