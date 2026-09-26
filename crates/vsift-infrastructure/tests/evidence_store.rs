//! Evidence records in session storage (P09 PR 2, ADR 0019): committing an
//! evidence call's media and record in one generation, reading them back,
//! verified media paths, the evidence sub-budget, and strict validation of
//! the evidence a retained bundle carries (ADR 0013 note of 2026-09-26).
//! Every record conforms to the published
//! `bundle-evidence-record.schema.json`, and the frozen example
//! `bundle-evidence-record.json` is the record of a `frame get` at 1.025 s
//! over the fake stream below.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fmt::Write as _,
    fs,
    future::{Future, ready},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use vsift_application::{
    AudioExtractor, CropRequest, EvidenceBudget, EvidenceCall, EvidenceControl, EvidenceExtraction,
    EvidenceMediaError, EvidenceScope, EvidenceStop, ExtractedClip, ExtractedFrame, FrameAtRequest,
    FrameExtractor, InitializeSessionStorage, InitializeSessionStorageRequest, SessionStorageError,
    VideoStreamFacts, extract_audio, extract_burst, extract_crop, extract_frame_at,
    extract_neighbours,
};
use vsift_domain::{
    AudioRange, BurstCount, BurstRange, CropRect, DurabilityRequirement, EvidenceProfile,
    FrameDimensions, FrameListing, FrameSelection, FrameTolerance, ListedFrame, ListingTail,
    MediaTime, NeighbourCount, OperationId, SessionArtifactKind, SessionId, Sha256Hex, SourceCheck,
    SourceId, StorageGeneration, TimeBase, TimeRange,
};
use vsift_infrastructure::{
    BundleSourcePolicy, EvidenceMediaFile, FilesystemSessionStore, MAX_EVIDENCE_ARTIFACTS,
    SourceSnapshot, decode_evidence_record, encode_evidence_record, wav_from_pcm_s16le_mono,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-evidence-store-test-";
const OPENED_AT: u64 = 1_000;
const FRAME_MICROS: u64 = 50_000;
const FINGERPRINT: &str = "5f2d8f0c0e7b4f3a9a1c6d2e8b7a6f5e4d3c2b1a09f8e7d6c5b4a39281706f5e";
const EXAMPLE_SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const EXAMPLE_SOURCE: &str =
    "src_sha256_2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae";

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
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// The start of an 8-bit RGB PNG of `width` x `height`, then `marker`; only
/// the header is read back by bundle validation.
fn fake_png(width: u32, height: u32, marker: &str) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13_u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    png.extend_from_slice(marker.as_bytes());
    png
}

/// A 20 fps, 6 s, 64x36 stream whose frame `n` has timestamp `n` in a 1/20
/// time base.
struct FakeVideo {
    facts: VideoStreamFacts,
}

impl FakeVideo {
    fn new() -> Built<Self> {
        Ok(Self {
            facts: VideoStreamFacts {
                stream_index: 0,
                time_base: TimeBase::new(1, 20)?,
                displayed: FrameDimensions::new(64, 36)?,
                duration: MediaTime::from_micros(6_000_000),
            },
        })
    }

    fn listing(&self, requested: TimeRange) -> Result<FrameListing, EvidenceMediaError> {
        let duration = self.facts.duration;
        let (end, tail) = if requested.end() >= duration {
            (duration, ListingTail::EndOfStream)
        } else {
            (requested.end(), ListingTail::MoreMayFollow)
        };
        let covered =
            TimeRange::new(requested.start(), end).map_err(|_| EvidenceMediaError::Invalid)?;
        let frames = (0..120_i64)
            .map(|pts| ListedFrame {
                pts,
                time: time_of(pts),
            })
            .filter(|frame| frame.time >= covered.start() && frame.time < covered.end())
            .collect();
        FrameListing::new(covered, frames, tail).map_err(|_| EvidenceMediaError::Invalid)
    }
}

fn time_of(pts: i64) -> MediaTime {
    MediaTime::from_micros(u64::try_from(pts).unwrap_or(0) * FRAME_MICROS)
}

impl FrameExtractor for FakeVideo {
    fn stream(&self) -> VideoStreamFacts {
        self.facts
    }

    fn max_frames_per_run(&self) -> usize {
        8
    }

    fn list_frames(
        &self,
        range: TimeRange,
    ) -> impl Future<Output = Result<FrameListing, EvidenceMediaError>> + Send {
        ready(self.listing(range))
    }

    fn frames(
        &self,
        pts: &[i64],
    ) -> impl Future<Output = Result<Vec<ExtractedFrame>, EvidenceMediaError>> + Send {
        ready(Ok(pts
            .iter()
            .map(|value| ExtractedFrame {
                pts: *value,
                time: time_of(*value),
                png: fake_png(64, 36, &format!("frame {value}")),
            })
            .collect()))
    }

    fn crop(
        &self,
        pts: i64,
        rect: CropRect,
    ) -> impl Future<Output = Result<ExtractedFrame, EvidenceMediaError>> + Send {
        ready(Ok(ExtractedFrame {
            pts,
            time: time_of(pts),
            png: fake_png(
                rect.width(),
                rect.height(),
                &format!("crop {pts} {} {}", rect.x(), rect.y()),
            ),
        }))
    }
}

struct FakeAudio;

impl AudioExtractor for FakeAudio {
    fn stream_index(&self) -> u32 {
        1
    }

    fn duration(&self) -> MediaTime {
        MediaTime::from_micros(6_000_000)
    }

    fn clip(
        &self,
        range: TimeRange,
    ) -> impl Future<Output = Result<ExtractedClip, EvidenceMediaError>> + Send {
        let samples = usize::try_from(range.duration_micros() / 62_500)
            .unwrap_or(1)
            .max(1);
        ready(
            wav_from_pcm_s16le_mono(&vec![0_u8; samples * 2])
                .map(|wav| ExtractedClip {
                    actual_start: MediaTime::from_micros(range.start().as_micros() + 64_000),
                    wav,
                })
                .map_err(|_| EvidenceMediaError::Invalid),
        )
    }
}

struct Never;

impl EvidenceControl for Never {
    fn stop(&self) -> Option<EvidenceStop> {
        None
    }
}

struct Opened {
    root: OwnedRoot,
    store: FilesystemSessionStore,
    session_id: SessionId,
    source_id: SourceId,
    fingerprint: Sha256Hex,
}

async fn open_session() -> Built<Opened> {
    let root = OwnedRoot::new()?;
    let source = root.0.join("source.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomevidence-store")?;
    let workspace = root.0.join("workspace");
    let store = FilesystemSessionStore::provision_default(&workspace)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    let registration = store.register_session(
        &session_id,
        &OperationId::parse("op_0123456789abcdef")?,
        OPENED_AT,
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
        &source,
    )?;
    store.activate_source(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        OPENED_AT,
    )?;
    let source_id = snapshot.id().clone();
    drop(snapshot);
    Ok(Opened {
        root,
        store,
        session_id,
        source_id,
        fingerprint: Sha256Hex::parse(FINGERPRINT)?,
    })
}

fn operation(number: u64) -> Built<OperationId> {
    Ok(OperationId::parse(format!("op_{number:016x}"))?)
}

fn range(start: u64, end: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(start),
        MediaTime::from_micros(end),
    )?)
}

fn frame_at(time: u64) -> FrameAtRequest {
    FrameAtRequest {
        at: MediaTime::from_micros(time),
        selection: FrameSelection::AtOrAfter,
        tolerance: FrameTolerance::MAX,
        candidate: None,
    }
}

impl Opened {
    fn call<'a>(&'a self, known: &'a BTreeSet<String>) -> EvidenceCall<'a, Never> {
        EvidenceCall {
            scope: EvidenceScope {
                session_id: &self.session_id,
                source_id: &self.source_id,
                profile: EvidenceProfile::P09R0,
                tool_fingerprint: Some(&self.fingerprint),
            },
            source_check: SourceCheck::FullHash,
            budget: EvidenceBudget::per_call(MAX_EVIDENCE_ARTIFACTS, u64::MAX),
            known_media: known,
            control: &Never,
        }
    }

    fn known(&self) -> Built<BTreeSet<String>> {
        Ok(self
            .store
            .read_evidence_records(&self.session_id, OPENED_AT + 1)?
            .known_media()
            .clone())
    }

    /// Commits an extraction like the engine does.
    fn commit(&self, extraction: &EvidenceExtraction, number: u64) -> TestResult {
        let status = self.store.session_status(&self.session_id)?;
        let media: Vec<EvidenceMediaFile<'_>> = extraction
            .media
            .iter()
            .map(|file| EvidenceMediaFile {
                kind: file.kind,
                bytes: &file.bytes,
            })
            .collect();
        self.store.publish_evidence(
            &self.session_id,
            &operation(number)?,
            status.generation(),
            &media,
            &encode_evidence_record(&extraction.record)?,
            None,
            OPENED_AT + 1,
        )?;
        Ok(())
    }
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Whether `instance` conforms to the published bundle evidence record schema.
fn conforms_to_record_schema(instance: &serde_json::Value) -> Built<bool> {
    let schema: serde_json::Value = serde_json::from_slice(&fs::read(repository(
        "schemas/v1/bundle-evidence-record.schema.json",
    ))?)?;
    Ok(jsonschema::validator_for(&schema)?.is_valid(instance))
}

/// A `frame get` record of the example session is the frozen example; it
/// conforms to the schema, decodes back to the record, and the schema
/// rejects fields the record does not define.
#[tokio::test]
async fn a_frame_record_is_the_frozen_bundle_example() -> TestResult {
    let session = SessionId::parse(EXAMPLE_SESSION)?;
    let source = SourceId::parse(EXAMPLE_SOURCE)?;
    let fingerprint = Sha256Hex::parse(FINGERPRINT)?;
    let known = BTreeSet::new();
    let call = EvidenceCall {
        scope: EvidenceScope {
            session_id: &session,
            source_id: &source,
            profile: EvidenceProfile::P09R0,
            tool_fingerprint: Some(&fingerprint),
        },
        source_check: SourceCheck::FullHash,
        budget: EvidenceBudget::per_call(MAX_EVIDENCE_ARTIFACTS, u64::MAX),
        known_media: &known,
        control: &Never,
    };
    let extraction = extract_frame_at(&call, &FakeVideo::new()?, frame_at(1_025_000)).await?;
    let bytes = encode_evidence_record(&extraction.record)?;
    let record: serde_json::Value = serde_json::from_slice(&bytes)?;
    let example_path = "schemas/v1/examples/bundle-evidence-record.json";
    if env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|value| value == "1") {
        let mut text = serde_json::to_string_pretty(&record)?;
        text.push('\n');
        fs::write(repository(example_path), text)?;
    }
    let example: serde_json::Value = serde_json::from_slice(&fs::read(repository(example_path))?)?;
    assert!(
        record == example,
        "the frame record differs from {example_path}"
    );
    assert!(conforms_to_record_schema(&example)?);
    assert_eq!(
        decode_evidence_record(&serde_json::to_vec(&example)?, &session)?,
        extraction.record
    );
    let mut extended = example.clone();
    extended["items"][0]["path"] = serde_json::Value::String("frame.png".to_owned());
    assert!(!conforms_to_record_schema(&extended)?);
    Ok(())
}

/// Every operation's record conforms to the schema and holds no path.
#[tokio::test]
async fn every_operation_commits_a_conforming_record_without_paths() -> TestResult {
    let opened = open_session().await?;
    let video = FakeVideo::new()?;
    let known = opened.known()?;
    let frame = extract_frame_at(&opened.call(&known), &video, frame_at(3_000_000)).await?;
    opened.commit(&frame, 10)?;
    let anchor = frame.record.items().first().ok_or("no frame")?.clone();
    let known = opened.known()?;
    let neighbours = extract_neighbours(
        &opened.call(&known),
        &video,
        &anchor,
        NeighbourCount::new(2)?,
    )
    .await?;
    opened.commit(&neighbours, 11)?;
    let known = opened.known()?;
    let burst = extract_burst(
        &opened.call(&known),
        &video,
        BurstRange::new(range(5_000_000, 7_000_000)?)?,
        BurstCount::new(4)?,
    )
    .await?;
    opened.commit(&burst, 12)?;
    let known = opened.known()?;
    let crop = extract_crop(
        &opened.call(&known),
        &video,
        CropRequest {
            parent: &anchor,
            x: 8,
            y: 4,
            width: 32,
            height: 16,
        },
    )
    .await?;
    opened.commit(&crop, 13)?;
    let known = opened.known()?;
    let audio = extract_audio(
        &opened.call(&known),
        &FakeAudio,
        AudioRange::new(range(0, 2_000_000)?)?,
    )
    .await?;
    opened.commit(&audio, 14)?;

    let inventory = opened
        .store
        .read_evidence_records(&opened.session_id, OPENED_AT + 2)?;
    assert_eq!(inventory.records().len(), 5);
    let workspace = opened.root.0.to_string_lossy().into_owned();
    for record in inventory.records() {
        let bytes = encode_evidence_record(record)?;
        let text = String::from_utf8(bytes.clone())?;
        assert!(!text.contains(&workspace), "a record holds a path");
        assert!(!text.contains("op_"), "a record holds an operation id");
        assert!(conforms_to_record_schema(&serde_json::from_slice(&bytes)?)?);
    }
    // D2: each item's file is delivered by a verified absolute path.
    for item in inventory
        .records()
        .iter()
        .flat_map(vsift_domain::EvidenceRecord::items)
    {
        let media = item.media();
        let path = opened.store.verified_artifact_path(
            &opened.session_id,
            media.kind(),
            media.sha256().as_str(),
            media.bytes(),
            OPENED_AT + 2,
        )?;
        assert!(path.is_absolute());
        assert_eq!(
            u64::try_from(fs::read(&path)?.len())?,
            media.bytes(),
            "the delivered file is the committed one"
        );
    }
    let bundle = opened.root.0.join("bundle");
    opened.store.retain_bundle(
        &opened.session_id,
        &bundle,
        BundleSourcePolicy::EvidenceOnly,
    )?;
    FilesystemSessionStore::validate_bundle(&bundle)?;
    Ok(())
}

/// Two requests resolving to one frame commit one file; committing a file
/// the session holds keeps it, and a name held by another kind fails.
#[tokio::test]
async fn identical_files_are_kept_and_conflicting_ones_fail() -> TestResult {
    let opened = open_session().await?;
    let video = FakeVideo::new()?;
    let known = opened.known()?;
    let first = extract_frame_at(&opened.call(&known), &video, frame_at(1_025_000)).await?;
    opened.commit(&first, 10)?;
    let known = opened.known()?;
    let second = extract_frame_at(&opened.call(&known), &video, frame_at(1_040_000)).await?;
    assert!(second.media.is_empty());
    opened.commit(&second, 11)?;
    let status = opened.store.session_status(&opened.session_id)?;
    assert_eq!(status.artifact_count(), 3, "one image and two records");

    // Offering the same image again keeps the one entry.
    let image = first.media.first().ok_or("no image")?;
    opened.store.publish_evidence(
        &opened.session_id,
        &operation(12)?,
        status.generation(),
        &[EvidenceMediaFile {
            kind: image.kind,
            bytes: &image.bytes,
        }],
        &encode_evidence_record(&first.record)?,
        None,
        OPENED_AT + 1,
    )?;
    let status = opened.store.session_status(&opened.session_id)?;
    assert_eq!(status.artifact_count(), 3);

    // A record whose name a visual-index record already holds conflicts.
    let bytes = b"{\"shared\":\"name\"}".to_vec();
    let generation = opened.store.publish_artifact(
        &opened.session_id,
        &operation(13)?,
        status.generation(),
        SessionArtifactKind::VisualIndexRecord,
        &bytes,
        OPENED_AT + 1,
    )?;
    assert_eq!(
        opened.store.publish_evidence(
            &opened.session_id,
            &operation(14)?,
            generation,
            &[],
            &bytes,
            None,
            OPENED_AT + 1,
        ),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

/// ADR 0019 D4: evidence takes at most 160 of the session's 256 artifacts.
#[tokio::test]
async fn the_evidence_sub_budget_is_enforced() -> TestResult {
    let opened = open_session().await?;
    let video = FakeVideo::new()?;
    // One call's image and record, and fillers up to two slots short of the
    // sub-budget, in one generation.
    let known = opened.known()?;
    let first = extract_frame_at(&opened.call(&known), &video, frame_at(0)).await?;
    let fillers: Vec<Vec<u8>> = (0..MAX_EVIDENCE_ARTIFACTS - 4)
        .map(|number| fake_png(64, 36, &format!("filler {number}")))
        .collect();
    let mut media: Vec<EvidenceMediaFile<'_>> = first
        .media
        .iter()
        .map(|file| EvidenceMediaFile {
            kind: file.kind,
            bytes: &file.bytes,
        })
        .collect();
    media.extend(fillers.iter().map(|bytes| EvidenceMediaFile {
        kind: vsift_domain::EvidenceMediaKind::FramePng,
        bytes,
    }));
    opened.store.publish_evidence(
        &opened.session_id,
        &operation(100)?,
        opened
            .store
            .session_status(&opened.session_id)?
            .generation(),
        &media,
        &encode_evidence_record(&first.record)?,
        None,
        OPENED_AT + 1,
    )?;
    let known = opened.known()?;
    let inventory = opened
        .store
        .read_evidence_records(&opened.session_id, OPENED_AT + 1)?;
    assert_eq!(inventory.evidence_slots_left(), 2);
    let burst = extract_burst(
        &EvidenceCall {
            budget: EvidenceBudget::per_call(inventory.evidence_slots_left(), u64::MAX),
            ..opened.call(&known)
        },
        &video,
        BurstRange::new(range(1_000_000, 2_000_000)?)?,
        BurstCount::new(4)?,
    )
    .await?;
    assert_eq!(
        burst.media.len(),
        1,
        "one image and the record fill the budget"
    );
    opened.commit(&burst, 999)?;
    let inventory = opened
        .store
        .read_evidence_records(&opened.session_id, OPENED_AT + 1)?;
    assert_eq!(inventory.evidence_slots_left(), 0);
    let status = inventory.status();
    assert_eq!(
        opened.store.publish_artifact(
            &opened.session_id,
            &operation(1000)?,
            status.generation(),
            SessionArtifactKind::FramePng,
            &fake_png(64, 36, "one too many"),
            OPENED_AT + 1,
        ),
        Err(SessionStorageError::CapacityExhausted)
    );
    // Other artifacts still fit: the limit is the evidence sub-budget.
    opened.store.publish_artifact(
        &opened.session_id,
        &operation(1001)?,
        status.generation(),
        SessionArtifactKind::VisualIndexRecord,
        b"{\"other\":true}",
        OPENED_AT + 1,
    )?;
    Ok(())
}

/// A retained bundle with a frame, a crop of it and an audio clip.
async fn bundled() -> Built<(Opened, PathBuf)> {
    let opened = open_session().await?;
    let video = FakeVideo::new()?;
    let known = opened.known()?;
    let frame = extract_frame_at(&opened.call(&known), &video, frame_at(2_000_000)).await?;
    opened.commit(&frame, 10)?;
    let parent = frame.record.items().first().ok_or("no frame")?.clone();
    let known = opened.known()?;
    let crop = extract_crop(
        &opened.call(&known),
        &video,
        CropRequest {
            parent: &parent,
            x: 4,
            y: 2,
            width: 16,
            height: 8,
        },
    )
    .await?;
    opened.commit(&crop, 11)?;
    let known = opened.known()?;
    let audio = extract_audio(
        &opened.call(&known),
        &FakeAudio,
        AudioRange::new(range(1_000_000, 3_000_000)?)?,
    )
    .await?;
    opened.commit(&audio, 12)?;
    let bundle = opened.root.0.join("bundle");
    opened.store.retain_bundle(
        &opened.session_id,
        &bundle,
        BundleSourcePolicy::EvidenceOnly,
    )?;
    FilesystemSessionStore::validate_bundle(&bundle)?;
    Ok((opened, bundle))
}

fn sha256_hex(bytes: &[u8]) -> Built<String> {
    let mut digest = String::new();
    for byte in Sha256::digest(bytes) {
        write!(digest, "{byte:02x}")?;
    }
    Ok(digest)
}

/// The bundle's manifest and the entries of one kind.
fn manifest(bundle: &Path) -> Built<serde_json::Value> {
    Ok(serde_json::from_slice(&fs::read(
        bundle.join("bundle.json"),
    )?)?)
}

fn entry_names(manifest: &serde_json::Value, kind: &str) -> Vec<String> {
    manifest["artifacts"]
        .as_array()
        .map(|artifacts| {
            artifacts
                .iter()
                .filter(|artifact| artifact["kind"] == kind)
                .filter_map(|artifact| artifact["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Replaces one artifact of the bundle with `bytes` under `kind`, keeping
/// the manifest's digests consistent.
fn replace_artifact(bundle: &Path, old_name: &str, kind: &str, bytes: &[u8]) -> Built<String> {
    let mut manifest = manifest(bundle)?;
    let extension = match kind {
        "frame_png" => "png",
        "audio_wav" => "wav",
        _ => "json",
    };
    let digest = sha256_hex(bytes)?;
    let new_name = format!("artifact-{digest}.{extension}");
    let entry = manifest["artifacts"]
        .as_array_mut()
        .and_then(|artifacts| {
            artifacts
                .iter_mut()
                .find(|artifact| artifact["name"] == old_name)
        })
        .ok_or("no such artifact")?;
    entry["kind"] = kind.into();
    entry["name"] = new_name.clone().into();
    entry["sha256"] = digest.into();
    entry["bytes"] = bytes.len().into();
    fs::remove_file(bundle.join(old_name))?;
    fs::write(bundle.join(&new_name), bytes)?;
    fs::write(bundle.join("bundle.json"), serde_json::to_vec(&manifest)?)?;
    Ok(new_name)
}

/// Removes one artifact from the bundle and its manifest.
fn remove_artifact(bundle: &Path, name: &str) -> TestResult {
    let mut manifest = manifest(bundle)?;
    manifest["artifacts"]
        .as_array_mut()
        .ok_or("no artifacts")?
        .retain(|artifact| artifact["name"] != name);
    fs::remove_file(bundle.join(name))?;
    fs::write(bundle.join("bundle.json"), serde_json::to_vec(&manifest)?)?;
    Ok(())
}

fn assert_rejected(bundle: &Path) {
    assert_eq!(
        FilesystemSessionStore::validate_bundle(bundle).map(|status| status.artifact_count()),
        Err(SessionStorageError::IntegrityFailure)
    );
}

/// The record whose request is `operation`, by name.
fn record_named(bundle: &Path, operation: &str) -> Built<(String, serde_json::Value)> {
    for name in entry_names(&manifest(bundle)?, "evidence_record") {
        let record: serde_json::Value = serde_json::from_slice(&fs::read(bundle.join(&name))?)?;
        if record["request"].get(operation).is_some() {
            return Ok((name, record));
        }
    }
    Err(format!("no {operation} record").into())
}

#[tokio::test]
async fn a_bundle_missing_an_evidence_file_is_rejected() -> TestResult {
    let (_opened, bundle) = bundled().await?;
    let audio = entry_names(&manifest(&bundle)?, "audio_wav");
    remove_artifact(&bundle, audio.first().ok_or("no clip")?)?;
    assert_rejected(&bundle);
    Ok(())
}

#[tokio::test]
async fn a_bundle_with_a_tampered_evidence_file_is_rejected() -> TestResult {
    let (_opened, bundle) = bundled().await?;
    let images = entry_names(&manifest(&bundle)?, "frame_png");
    let name = images.first().ok_or("no image")?;
    let mut bytes = fs::read(bundle.join(name))?;
    bytes.push(0);
    replace_artifact(&bundle, name, "frame_png", &bytes)?;
    assert_rejected(&bundle);
    Ok(())
}

#[tokio::test]
async fn a_bundle_whose_file_has_another_kind_is_rejected() -> TestResult {
    let (_opened, bundle) = bundled().await?;
    let clips = entry_names(&manifest(&bundle)?, "audio_wav");
    let name = clips.first().ok_or("no clip")?;
    let bytes = fs::read(bundle.join(name))?;
    replace_artifact(&bundle, name, "frame_png", &bytes)?;
    assert_rejected(&bundle);
    Ok(())
}

#[tokio::test]
async fn a_bundle_whose_image_header_disagrees_with_its_item_is_rejected() -> TestResult {
    let (_opened, bundle) = bundled().await?;
    let (record_name, mut record) = record_named(&bundle, "frame_get")?;
    let image_name = format!(
        "artifact-{}.png",
        record["items"][0]["media"]["sha256"]
            .as_str()
            .ok_or("no digest")?
    );
    // A consistent image of another size: the manifest and the record's
    // digest agree with it, only the header disagrees with the item.
    let other = fake_png(63, 36, "frame 40");
    let new_image = replace_artifact(&bundle, &image_name, "frame_png", &other)?;
    record["items"][0]["media"]["sha256"] = new_image
        .trim_start_matches("artifact-")
        .trim_end_matches(".png")
        .into();
    record["items"][0]["media"]["bytes"] = other.len().into();
    replace_artifact(
        &bundle,
        &record_name,
        "evidence_record",
        &serde_json::to_vec(&record)?,
    )?;
    assert_rejected(&bundle);
    Ok(())
}

#[tokio::test]
async fn a_bundle_without_a_crop_parent_is_rejected() -> TestResult {
    let (_opened, bundle) = bundled().await?;
    let (frame_record, _) = record_named(&bundle, "frame_get")?;
    remove_artifact(&bundle, &frame_record)?;
    assert_rejected(&bundle);
    Ok(())
}

#[tokio::test]
async fn an_edited_record_identity_is_rejected() -> TestResult {
    let (_opened, bundle) = bundled().await?;
    let (name, mut record) = record_named(&bundle, "crop")?;
    record["request"]["crop"]["rect"]["x"] = 5.into();
    replace_artifact(
        &bundle,
        &name,
        "evidence_record",
        &serde_json::to_vec(&record)?,
    )?;
    assert_rejected(&bundle);
    Ok(())
}
