//! Visual-index records in session storage (P08 PR 3): committing revisions,
//! reading the newest, the per-session record limit, retained bundles, and
//! strict validation of the records a bundle carries (ADR 0013 note of
//! 2026-09-26). Every record a bundle carries conforms to the published
//! `bundle-visual-index-record.schema.json`, and the frozen example
//! `bundle-visual-index-record.json` is the F02 index built from its
//! recorded samples (P08 PR 4).

mod candidate_recall;

use std::{
    env,
    error::Error,
    fmt::Write as _,
    fs,
    future::Future,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use vsift_application::{
    ExtendVisualIndexRequest, InitializeSessionStorage, InitializeSessionStorageRequest,
    SessionStorageError, VisualIndexScope, VisualSampler, VisualSamplingError, extend_visual_index,
};
use vsift_domain::{
    DurabilityRequirement, FrameDimensions, MediaTime, OperationId, SessionArtifactKind, SessionId,
    StorageGeneration, TimeRange, VISUAL_BLOCKS, VisualHash, VisualIndex, VisualIndexProfile,
    VisualSample, VisualWindow,
};
use vsift_infrastructure::{
    BundleSourcePolicy, FilesystemSessionStore, MAX_VISUAL_INDEX_RECORDS, SourceSnapshot,
    encode_visual_index_record,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-visual-index-store-test-";
const OPENED_AT: u64 = 1_000;
const SECOND: u64 = 1_000_000;
const DURATION: u64 = 150 * SECOND;

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

/// A slide that changes every 20 s.
struct Slides;

impl VisualSampler for Slides {
    fn window_samples(
        &self,
        window: VisualWindow,
    ) -> impl Future<Output = Result<Vec<VisualSample>, VisualSamplingError>> + Send {
        let mut samples = Vec::new();
        let mut time = window.lead_in_start().as_micros();
        while time < window.range().end().as_micros() {
            let level = u8::try_from(40 + (time / (20 * SECOND)) * 20).unwrap_or(u8::MAX);
            samples.push(VisualSample::from_parts(
                MediaTime::from_micros(time),
                [level; VISUAL_BLOCKS],
                VisualHash::from_bits(u64::from(level)),
            ));
            time += SECOND / 2;
        }
        std::future::ready(Ok(samples))
    }
}

struct Opened {
    root: OwnedRoot,
    workspace: PathBuf,
    store: FilesystemSessionStore,
    session_id: SessionId,
    source_id: vsift_domain::SourceId,
    generation: StorageGeneration,
}

async fn open_session() -> Built<Opened> {
    let root = OwnedRoot::new()?;
    let source = root.0.join("source.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomvisual-index-store")?;
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
        root,
        workspace,
        store,
        session_id,
        source_id,
        generation,
    })
}

async fn extended(
    session: &Opened,
    previous: Option<&VisualIndex>,
    to_seconds: u64,
) -> Built<VisualIndex> {
    let scope = VisualIndexScope {
        session_id: &session.session_id,
        source_id: &session.source_id,
        stream_index: 0,
        displayed_dimensions: FrameDimensions::new(1280, 720)?,
        duration: MediaTime::from_micros(DURATION),
        profile: VisualIndexProfile::R0,
    };
    let range = TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(to_seconds * SECOND),
    )?;
    Ok(extend_visual_index(
        ExtendVisualIndexRequest {
            scope,
            previous,
            range,
        },
        &Slides,
    )
    .await?
    .revision
    .ok_or("no revision")?)
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Whether `instance` conforms to the published bundle visual-index record schema.
fn conforms_to_record_schema(instance: &serde_json::Value) -> Built<bool> {
    let schema: serde_json::Value = serde_json::from_slice(&fs::read(repository(
        "schemas/v1/bundle-visual-index-record.schema.json",
    ))?)?;
    Ok(jsonschema::validator_for(&schema)?.is_valid(instance))
}

/// The stored record of F02's index (from its recorded samples) is the
/// frozen example; it conforms to the schema, decodes back to the index,
/// and the schema rejects fields the record does not define.
#[tokio::test]
async fn the_f02_record_is_the_frozen_bundle_example() -> TestResult {
    let recorded = candidate_recall::load_recorded("F02")?;
    let index = candidate_recall::index_recorded(&recorded).await?;
    let session = SessionId::parse(candidate_recall::SCORING_SESSION)?;
    let bytes = encode_visual_index_record(&session, &index)?;
    let record: serde_json::Value = serde_json::from_slice(&bytes)?;
    let example_path = "schemas/v1/examples/bundle-visual-index-record.json";
    if env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|value| value == "1") {
        let mut text = serde_json::to_string_pretty(&record)?;
        text.push('\n');
        fs::write(repository(example_path), text)?;
    }
    let example: serde_json::Value = serde_json::from_slice(&fs::read(repository(example_path))?)?;
    assert!(
        record == example,
        "the F02 record differs from {example_path}"
    );
    assert!(conforms_to_record_schema(&example)?);
    assert_eq!(
        vsift_infrastructure::decode_visual_index_record(&serde_json::to_vec(&example)?, &session)?,
        index
    );
    let mut extended = example.clone();
    extended["windows"][0]["candidates"][0]["thumbnail"] = serde_json::Value::Bool(true);
    assert!(!conforms_to_record_schema(&extended)?);
    Ok(())
}

fn operation(number: u64) -> Built<OperationId> {
    Ok(OperationId::parse(format!("op_{number:016x}"))?)
}

#[tokio::test]
async fn revisions_are_committed_the_newest_is_read_and_bundles_validate_them() -> TestResult {
    let mut session = open_session().await?;
    assert_eq!(
        session
            .store
            .read_visual_index(&session.session_id, OPENED_AT + 1)?,
        None
    );
    let first = extended(&session, None, 60).await?;
    session.generation = session.store.publish_artifact(
        &session.session_id,
        &operation(10)?,
        session.generation,
        SessionArtifactKind::VisualIndexRecord,
        &encode_visual_index_record(&session.session_id, &first)?,
        OPENED_AT + 1,
    )?;
    let second = extended(&session, Some(&first), 150).await?;
    session.generation = session.store.publish_artifact(
        &session.session_id,
        &operation(11)?,
        session.generation,
        SessionArtifactKind::VisualIndexRecord,
        &encode_visual_index_record(&session.session_id, &second)?,
        OPENED_AT + 1,
    )?;
    let store = FilesystemSessionStore::open_existing(&session.workspace)?;
    let (newest, status) = store
        .read_visual_index(&session.session_id, OPENED_AT + 2)?
        .ok_or("no index")?;
    assert_eq!(newest, second);
    assert_eq!(status.source_id(), &session.source_id);
    assert_eq!(newest.window(0), first.window(0));

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
            && path
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            let record: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
            assert!(
                conforms_to_record_schema(&record)?,
                "a bundled record does not conform to its schema"
            );
        }
    }

    // Rewrite a record and its manifest entry together: the digests agree,
    // but the record claims a span its candidates do not have.
    let manifest_path = bundle.join("bundle.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    let entry = manifest["artifacts"]
        .as_array_mut()
        .and_then(|artifacts| artifacts.last_mut())
        .ok_or("no artifact")?;
    let old_name = entry["name"].as_str().ok_or("no name")?.to_owned();
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(bundle.join(&old_name))?)?;
    record["windows"][0]["candidates"][0]["span_end_us"] = (3 * SECOND).into();
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
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&bundle),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

#[tokio::test]
async fn a_record_of_another_session_is_an_integrity_failure() -> TestResult {
    let session = open_session().await?;
    let index = extended(&session, None, 60).await?;
    let other = SessionId::parse("ses_fedcba9876543210")?;
    session.store.publish_artifact(
        &session.session_id,
        &operation(10)?,
        session.generation,
        SessionArtifactKind::VisualIndexRecord,
        &encode_visual_index_record(&other, &index)?,
        OPENED_AT + 1,
    )?;
    assert_eq!(
        session
            .store
            .read_visual_index(&session.session_id, OPENED_AT + 2),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

#[tokio::test]
async fn a_session_holds_at_most_sixty_four_index_records() -> TestResult {
    let mut session = open_session().await?;
    for number in 0..MAX_VISUAL_INDEX_RECORDS {
        session.generation = session.store.publish_artifact(
            &session.session_id,
            &operation(100 + u64::try_from(number)?)?,
            session.generation,
            SessionArtifactKind::VisualIndexRecord,
            format!("{{\"record\":{number}}}").as_bytes(),
            OPENED_AT + 1,
        )?;
    }
    assert_eq!(
        session.store.publish_artifact(
            &session.session_id,
            &operation(999)?,
            session.generation,
            SessionArtifactKind::VisualIndexRecord,
            b"{\"record\":\"one too many\"}",
            OPENED_AT + 1,
        ),
        Err(SessionStorageError::CapacityExhausted)
    );
    // Other evidence is still accepted: the limit is per kind.
    session.store.publish_artifact(
        &session.session_id,
        &operation(1000)?,
        session.generation,
        SessionArtifactKind::AudioPcm,
        &[0, 0],
        OPENED_AT + 1,
    )?;
    Ok(())
}
