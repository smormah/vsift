//! Transcript revisions committed through session generations, read back,
//! retained in bundles and protected by integrity checks.

use std::{
    env,
    error::Error,
    fs,
    num::NonZeroU32,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use vsift_application::{
    ForegroundSessionPort, ImportedRevisionRequest, InitializeSessionStorage,
    InitializeSessionStorageRequest, OpenSessionError, SessionStorageError,
    build_imported_revision,
};
use vsift_domain::{
    DurabilityRequirement, MediaTime, OperationId, SessionId, StorageGeneration, TranscriptOffset,
    TranscriptRevision,
};
use vsift_infrastructure::{
    BundleSourcePolicy, FilesystemSessionStore, SourceSnapshot, decode_transcript_record,
    encode_transcript_record, read_supplied_transcript,
};

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-transcript-store-test-";
const OPENED_AT: u64 = 1_000;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
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

fn f10_sidecar() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/transcripts/F10.vtt")
}

/// An initialized session with a staged, not yet activated source.
struct Staged {
    root: OwnedRoot,
    workspace: PathBuf,
    store: FilesystemSessionStore,
    session_id: SessionId,
    snapshot: SourceSnapshot,
}

async fn staged() -> Result<Staged, Box<dyn Error>> {
    let root = OwnedRoot::new()?;
    let source = root.0.join("source.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomtranscript-store")?;
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
    Ok(Staged {
        root,
        workspace,
        store,
        session_id,
        snapshot,
    })
}

fn f10_revision(staged: &Staged) -> Result<TranscriptRevision, Box<dyn Error>> {
    let supplied = read_supplied_transcript(&f10_sidecar())?;
    Ok(build_imported_revision(ImportedRevisionRequest {
        session_id: &staged.session_id,
        source_id: staged.snapshot.id(),
        source_duration: MediaTime::from_micros(12_000_000),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?)
}

fn activate(
    staged: &Staged,
    revision: &TranscriptRevision,
) -> Result<StorageGeneration, OpenSessionError> {
    staged.store.activate_with_transcript(
        &staged.snapshot,
        &OperationId::parse("op_2222222222222222").map_err(|_| OpenSessionError::InvalidSource)?,
        StorageGeneration::INITIAL,
        OPENED_AT,
        revision,
    )
}

#[tokio::test]
async fn transcript_is_committed_with_activation_read_back_and_retained() -> TestResult {
    let staged = staged().await?;
    let revision = f10_revision(&staged)?;

    let generation = activate(&staged, &revision)?;
    assert_eq!(generation, StorageGeneration::from_value(1));

    let reopened = FilesystemSessionStore::open_existing(&staged.workspace)?;
    let status = reopened.session_status(&staged.session_id)?;
    assert_eq!(status.generation(), generation);
    assert_eq!(status.artifact_count(), 1);
    let (read, read_status) = reopened
        .read_transcript(&staged.session_id, OPENED_AT + 1)?
        .ok_or("transcript missing")?;
    assert_eq!(read, revision);
    assert_eq!(read_status, status);

    let bundle = staged.root.0.join("bundle");
    let retained = reopened.retain_bundle(
        &staged.session_id,
        &bundle,
        BundleSourcePolicy::EvidenceOnly,
    )?;
    assert_eq!(retained.artifact_count(), 1);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    assert_eq!(manifest["artifacts"][0]["kind"], "transcript_record");
    let name = manifest["artifacts"][0]["name"]
        .as_str()
        .ok_or("artifact name missing")?;
    assert_eq!(
        decode_transcript_record(&fs::read(bundle.join(name))?)?,
        revision
    );
    Ok(())
}

#[tokio::test]
async fn a_changed_transcript_record_is_an_integrity_failure() -> TestResult {
    let staged = staged().await?;
    let revision = f10_revision(&staged)?;
    activate(&staged, &revision)?;
    let artifacts = staged
        .workspace
        .join("sessions")
        .join(staged.session_id.as_str())
        .join("artifacts");
    let record = fs::read_dir(&artifacts)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .ok_or("record missing")?;
    let original = fs::read(&record)?;
    let tampered = String::from_utf8(original)?.replace("Dialog R-17", "Dialog R-18");
    fs::write(&record, tampered)?;

    assert_eq!(
        staged
            .store
            .read_transcript(&staged.session_id, OPENED_AT + 1)
            .map(|found| found.is_some()),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

#[tokio::test]
async fn sessions_without_transcripts_and_closed_or_expired_sessions_are_distinguished()
-> TestResult {
    let staged = staged().await?;
    let generation = staged.store.activate_source(
        &staged.snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        OPENED_AT,
    )?;
    assert_eq!(
        staged
            .store
            .read_transcript(&staged.session_id, OPENED_AT + 1)
            .map(|found| found.is_some()),
        Ok(false)
    );
    let status = staged.store.session_status(&staged.session_id)?;
    assert_eq!(
        staged
            .store
            .read_transcript(
                &staged.session_id,
                status.lifetime().expires_at_unix_seconds()
            )
            .map(|found| found.is_some()),
        Err(SessionStorageError::StateConflict)
    );
    drop(staged.snapshot);
    staged.store.close_session(
        &staged.session_id,
        &OperationId::parse("op_3333333333333333")?,
        generation,
    )?;
    assert_eq!(
        staged
            .store
            .read_transcript(&staged.session_id, OPENED_AT + 1)
            .map(|found| found.is_some()),
        Err(SessionStorageError::StateConflict)
    );
    Ok(())
}

#[tokio::test]
async fn stored_records_are_strict_and_versioned() -> TestResult {
    let staged = staged().await?;
    let revision = f10_revision(&staged)?;
    let encoded = encode_transcript_record(&revision)?;
    assert_eq!(decode_transcript_record(&encoded)?, revision);
    let value: serde_json::Value = serde_json::from_slice(&encoded)?;

    let mut future = value.clone();
    future["schema_version"] = serde_json::json!(2);
    assert_eq!(
        decode_transcript_record(&serde_json::to_vec(&future)?),
        Err(SessionStorageError::UnsupportedVersion)
    );

    for pointer in ["", "/segments/0", "/source_segment", "/sidecar"] {
        let mut extended = value.clone();
        extended
            .pointer_mut(pointer)
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("not an object")?
            .insert("unreviewed".to_owned(), serde_json::Value::Bool(true));
        assert_eq!(
            decode_transcript_record(&serde_json::to_vec(&extended)?),
            Err(SessionStorageError::IntegrityFailure),
            "{pointer}"
        );
    }

    // A range that is not the cue timing plus the offset is rejected by the domain.
    let mut shifted = value.clone();
    shifted["segments"][0]["start_us"] = serde_json::json!(1_000_001);
    assert_eq!(
        decode_transcript_record(&serde_json::to_vec(&shifted)?),
        Err(SessionStorageError::IntegrityFailure)
    );

    // Imported SRT/VTT segments cannot claim a speaker the format cannot state.
    let mut srt_speaker = value;
    srt_speaker["origin"] = serde_json::json!("imported_srt");
    srt_speaker["segments"][0]["speaker"] = serde_json::json!("Narrator");
    assert_eq!(
        decode_transcript_record(&serde_json::to_vec(&srt_speaker)?),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}
