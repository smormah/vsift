//! Screen-text index records in session storage (OCR, ADR 0030): committing
//! revisions, reading the newest, retained bundles, and strict validation of
//! the records a bundle carries. Every record a bundle carries conforms to the
//! published `bundle-screen-text-index-record.schema.json`, and the frozen
//! example `bundle-screen-text-index-record.json` is the record of the sample
//! index built here.

use std::{
    env,
    error::Error,
    fmt::Write as _,
    fs,
    num::NonZeroU32,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use vsift_application::{
    InitializeSessionStorage, InitializeSessionStorageRequest, SessionStorageError,
    screen_text_index_id, screen_text_line_id,
};
use vsift_domain::{
    Confidence, ConfidenceOrigin, DEFAULT_READ_PERIOD_MICROS, DurabilityRequirement,
    FrameDimensions, FrameRef, MediaTime, OperationId, ProviderLabel, ReadTarget, ScreenText,
    ScreenTextFailure, ScreenTextFrame, ScreenTextIndex, ScreenTextIndexParts, ScreenTextLine,
    ScreenTextOutcome, ScreenTextProfile, ScreenTextProvider, ScreenTextScope, SessionArtifactKind,
    SessionId, Sha256Hex, SourceId, StorageGeneration, TextRegion, TimeBase, VisualCandidateId,
};
use vsift_infrastructure::{
    BundleSourcePolicy, FilesystemSessionStore, SourceSnapshot, decode_screen_text_index_record,
    encode_screen_text_index_record,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-screen-text-store-test-";
const OPENED_AT: u64 = 1_000;
const SECOND: u64 = 1_000_000;
const EXAMPLE: &str = "schemas/v1/examples/bundle-screen-text-index-record.json";
const SCHEMA: &str = "schemas/v1/bundle-screen-text-index-record.schema.json";

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

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Whether `instance` conforms to the published record schema.
fn conforms_to_record_schema(instance: &serde_json::Value) -> Built<bool> {
    let schema: serde_json::Value = serde_json::from_slice(&fs::read(repository(SCHEMA))?)?;
    Ok(jsonschema::validator_for(&schema)?.is_valid(instance))
}

fn scope(session: &SessionId, source: &SourceId) -> Built<ScreenTextScope> {
    let digest = |fill: char| Sha256Hex::parse(fill.to_string().repeat(64));
    Ok(ScreenTextScope {
        session_id: session.clone(),
        source_id: source.clone(),
        stream_index: 0,
        profile: ScreenTextProfile::PpOcrV5LatinV1,
        provider: ScreenTextProvider {
            runner_sha256: digest('1')?,
            runner_version: ProviderLabel::parse("0.1.0")?,
            runtime_version: ProviderLabel::parse("1.31.0")?,
            detector_sha256: digest('2')?,
            recognizer_sha256: digest('3')?,
        },
    })
}

fn frame(pts: i64) -> Built<FrameRef> {
    Ok(FrameRef {
        stream_index: 0,
        time_base: TimeBase::new(1, 30)?,
        pts,
        time: MediaTime::from_micros(u64::try_from(pts)? * SECOND / 30),
        dimensions: FrameDimensions::new(1280, 720)?,
    })
}

fn line(
    scope: &ScreenTextScope,
    pts: i64,
    ordinal: usize,
    region: (u32, u32, u32, u32),
    text: &str,
    basis_points: u16,
) -> Built<ScreenTextLine> {
    let (x, y, width, height) = region;
    Ok(ScreenTextLine {
        id: screen_text_line_id(scope, pts, ordinal)?,
        region: TextRegion::new(x, y, width, height, FrameDimensions::new(1280, 720)?)?,
        text: ScreenText::parse(text)?,
        confidence: Confidence::provider_score(
            basis_points,
            ConfidenceOrigin::ProviderUncalibrated,
        )?,
    })
}

/// Revision 1 reads the frame at 1 s; revision 2 adds an unreadable frame at
/// 10 s and a target at 60 s with no frame.
fn sample(session: &SessionId, source: &SourceId, revision: u32) -> Built<ScreenTextIndex> {
    let scope = scope(session, source)?;
    let mut frames = vec![ScreenTextFrame {
        frame: frame(30)?,
        targets: vec![
            ReadTarget {
                time: MediaTime::from_micros(SECOND),
                candidate: None,
            },
            ReadTarget {
                time: MediaTime::from_micros(SECOND),
                candidate: Some(VisualCandidateId::parse(format!("vcd_{}", "c".repeat(32)))?),
            },
        ],
        outcome: ScreenTextOutcome::Read {
            lines: vec![
                line(&scope, 30, 1, (40, 28, 300, 14), "Invoice total", 9_912)?,
                line(&scope, 30, 2, (40, 60, 200, 14), "4,731,145", 8_001)?,
            ],
            truncated: false,
        },
    }];
    let mut unresolved = Vec::new();
    if revision > 1 {
        frames.push(ScreenTextFrame {
            frame: frame(300)?,
            targets: vec![ReadTarget {
                time: MediaTime::from_micros(10 * SECOND),
                candidate: None,
            }],
            outcome: ScreenTextOutcome::Failed(ScreenTextFailure::ImageRejected),
        });
        unresolved.push(ReadTarget {
            time: MediaTime::from_micros(60 * SECOND),
            candidate: None,
        });
    }
    let number = NonZeroU32::new(revision).ok_or("revision")?;
    Ok(ScreenTextIndex::new(ScreenTextIndexParts {
        id: screen_text_index_id(&scope, number)?,
        revision: number,
        scope,
        period_micros: DEFAULT_READ_PERIOD_MICROS,
        frames,
        unresolved,
    })?)
}

fn example_session() -> Built<(SessionId, SourceId)> {
    Ok((
        SessionId::parse("ses_0123456789abcdef")?,
        SourceId::parse(format!("src_sha256_{}", "b".repeat(64)))?,
    ))
}

/// The record of the sample index is the frozen example; it conforms to the
/// schema, decodes back to the index, and the schema rejects fields the
/// record does not define and a read frame without its lines.
#[test]
fn the_sample_record_is_the_frozen_bundle_example() -> TestResult {
    let (session, source) = example_session()?;
    let index = sample(&session, &source, 2)?;
    let record: serde_json::Value =
        serde_json::from_slice(&encode_screen_text_index_record(&index)?)?;
    if env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|value| value == "1") {
        let mut text = serde_json::to_string_pretty(&record)?;
        text.push('\n');
        fs::write(repository(EXAMPLE), text)?;
    }
    let example: serde_json::Value = serde_json::from_slice(&fs::read(repository(EXAMPLE))?)?;
    assert!(
        record == example,
        "the sample record differs from {EXAMPLE}"
    );
    assert!(conforms_to_record_schema(&example)?);
    assert_eq!(
        decode_screen_text_index_record(&serde_json::to_vec(&example)?, &session)?,
        index
    );
    let mut extended = example.clone();
    extended["frames"][0]["lines"][0]["pixels"] = serde_json::Value::Bool(true);
    assert!(!conforms_to_record_schema(&extended)?);
    let mut lineless = example.clone();
    if let Some(frame) = lineless["frames"][0].as_object_mut() {
        frame.remove("lines");
    }
    assert!(!conforms_to_record_schema(&lineless)?);
    let mut failed_with_lines = example;
    failed_with_lines["frames"][1]["lines"] = serde_json::json!([]);
    assert!(!conforms_to_record_schema(&failed_with_lines)?);
    Ok(())
}

struct Opened {
    workspace: PathBuf,
    store: FilesystemSessionStore,
    session_id: SessionId,
    source_id: SourceId,
    generation: StorageGeneration,
    // Last: fields drop in order, and on Windows a folder something still
    // holds open cannot be removed (#423).
    root: OwnedRoot,
}

async fn open_session() -> Built<Opened> {
    let root = OwnedRoot::new()?;
    let source = root.0.join("source.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomscreen-text-store")?;
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
    let generation = store.activate_source(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        OPENED_AT,
    )?;
    let source_id = snapshot.id().clone();
    drop(snapshot);
    Ok(Opened {
        workspace,
        store,
        session_id,
        source_id,
        generation,
        root,
    })
}

fn operation(number: u64) -> Built<OperationId> {
    Ok(OperationId::parse(format!("op_{number:016x}"))?)
}

fn publish(session: &mut Opened, number: u64, index: &ScreenTextIndex) -> TestResult {
    session.generation = session.store.publish_artifact(
        &session.session_id,
        &operation(number)?,
        session.generation,
        SessionArtifactKind::ScreenTextIndexRecord,
        &encode_screen_text_index_record(index)?,
        OPENED_AT + 1,
    )?;
    Ok(())
}

/// Rewrites the bundle's last artifact with `edit` and its manifest entry
/// with the new name, size and digest, so only the record's own checks can
/// catch what was changed.
fn rewrite_last_artifact(
    bundle: &std::path::Path,
    edit: impl FnOnce(&mut serde_json::Value),
) -> TestResult {
    let manifest_path = bundle.join("bundle.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    let entry = manifest["artifacts"]
        .as_array_mut()
        .and_then(|artifacts| artifacts.last_mut())
        .ok_or("no artifact")?;
    let old_name = entry["name"].as_str().ok_or("no name")?.to_owned();
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(bundle.join(&old_name))?)?;
    edit(&mut record);
    let bytes = serde_json::to_vec(&record)?;
    let mut digest = String::new();
    for byte in Sha256::digest(&bytes) {
        write!(digest, "{byte:02x}")?;
    }
    let new_name = format!("artifact-{digest}.json");
    fs::remove_file(bundle.join(&old_name))?;
    fs::write(bundle.join(&new_name), &bytes)?;
    entry["name"] = new_name.into();
    entry["sha256"] = digest.into();
    entry["bytes"] = bytes.len().into();
    fs::write(&manifest_path, serde_json::to_vec(&manifest)?)?;
    Ok(())
}

#[tokio::test]
async fn revisions_are_committed_the_newest_is_read_and_bundles_validate_them() -> TestResult {
    let mut session = open_session().await?;
    let (none, _) = session
        .store
        .read_screen_text_index(&session.session_id, OPENED_AT + 1)?;
    assert_eq!(none, None);
    let first = sample(&session.session_id, &session.source_id, 1)?;
    publish(&mut session, 10, &first)?;
    let second = sample(&session.session_id, &session.source_id, 2)?;
    publish(&mut session, 11, &second)?;

    let store = FilesystemSessionStore::open_existing(&session.workspace)?;
    let (newest, status) = store.read_screen_text_index(&session.session_id, OPENED_AT + 2)?;
    assert_eq!(newest, Some(second));
    assert_eq!(status.source_id(), &session.source_id);

    let bundle = session.root.0.join("bundle");
    store.retain_bundle(
        &session.session_id,
        &bundle,
        BundleSourcePolicy::EvidenceOnly,
    )?;
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&bundle)?.artifact_count(),
        2
    );
    for entry in fs::read_dir(&bundle)? {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("artifact-"))
        {
            let record: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
            assert!(
                conforms_to_record_schema(&record)?,
                "a bundled record does not conform to its schema"
            );
        }
    }

    // With every digest made to agree, an edited line identity or a region
    // outside its frame is still refused.
    let copy = session.root.0.join("bundle-identity");
    store.retain_bundle(&session.session_id, &copy, BundleSourcePolicy::EvidenceOnly)?;
    rewrite_last_artifact(&copy, |record| {
        record["frames"][0]["lines"][1]["id"] = "stl_0000000000000000".into();
    })?;
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&copy),
        Err(SessionStorageError::IntegrityFailure)
    );
    let copy = session.root.0.join("bundle-region");
    store.retain_bundle(&session.session_id, &copy, BundleSourcePolicy::EvidenceOnly)?;
    rewrite_last_artifact(&copy, |record| {
        record["frames"][0]["lines"][0]["x"] = 1_200.into();
    })?;
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&copy),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

#[tokio::test]
async fn a_record_of_another_source_is_an_integrity_failure() -> TestResult {
    let mut session = open_session().await?;
    let other = SourceId::parse(format!("src_sha256_{}", "d".repeat(64)))?;
    let foreign = sample(&session.session_id, &other, 1)?;
    publish(&mut session, 10, &foreign)?;
    assert!(matches!(
        session
            .store
            .read_screen_text_index(&session.session_id, OPENED_AT + 2),
        Err(SessionStorageError::IntegrityFailure)
    ));
    Ok(())
}
