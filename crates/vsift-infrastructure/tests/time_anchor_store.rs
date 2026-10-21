//! Time anchoring (ADR 0029) on the real filesystem adapter: the anchors an
//! open stores are read back by every later status, survive later
//! generations, and a session without anchors is recorded exactly as before.

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use vsift_application::{InitializeSessionStorage, InitializeSessionStorageRequest};
use vsift_domain::{
    AnchorSource, ClockReading, DurabilityRequirement, MediaTime, OperationId, SessionId,
    StorageGeneration, TimeAnchor, TimeAnchors,
};
use vsift_infrastructure::{FilesystemSessionStore, SourceSnapshot};

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-time-anchor-test-";

struct OwnedRoot(PathBuf);

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path =
            env::temp_dir().join(format!("{PREFIX}{}-{stamp}-{sequence}", std::process::id()));
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
            .is_some_and(|name| name.starts_with(PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// Opens one session over a stub source with `anchors`; returns the store,
/// the session and the activation's generation.
async fn open_with(
    root: &Path,
    anchors: &TimeAnchors,
) -> Result<(FilesystemSessionStore, SessionId, StorageGeneration), Box<dyn Error>> {
    let source = root.join("source.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomtest")?;
    let workspace = root.join("workspace");
    let store = FilesystemSessionStore::provision_default(&workspace)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    let _registration = store.register_session(
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
    let store = FilesystemSessionStore::open_existing(&workspace)?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        &source,
    )?;
    let generation = store.activate_anchored_source(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        1_000,
        anchors,
    )?;
    drop(snapshot);
    Ok((store, session_id, generation))
}

fn anchors() -> Result<TimeAnchors, Box<dyn Error>> {
    let start = MediaTime::from_micros(0);
    Ok(TimeAnchors::new(vec![
        TimeAnchor::new(
            AnchorSource::Operator,
            start,
            ClockReading::parse("2026-10-21T14:03:12+01:00")?,
        ),
        TimeAnchor::new(
            AnchorSource::ContainerCreationTime,
            start,
            ClockReading::parse("2026-10-21T13:03:13Z")?,
        ),
    ])?)
}

/// Every committed generation's manifest text, in generation order.
fn manifests(root: &Path, session_id: &SessionId) -> Result<Vec<String>, Box<dyn Error>> {
    let directory = root
        .join("workspace")
        .join("sessions")
        .join(session_id.as_str())
        .join("generations");
    let mut names: Vec<PathBuf> = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    names.sort();
    names
        .iter()
        .map(|path| fs::read_to_string(path).map_err(Into::into))
        .collect()
}

#[tokio::test]
async fn stored_anchors_are_read_back_and_kept_by_later_generations() -> TestResult {
    let root = OwnedRoot::new()?;
    let anchors = anchors()?;
    let (store, session_id, generation) = open_with(&root.0, &anchors).await?;

    assert_eq!(store.session_status(&session_id)?.time_anchors(), &anchors);
    store.renew_session(
        &session_id,
        &OperationId::parse("op_3333333333333333")?,
        generation,
        1_060,
    )?;
    let reopened = FilesystemSessionStore::open_existing(root.0.join("workspace"))?;
    assert_eq!(
        reopened.session_status(&session_id)?.time_anchors(),
        &anchors,
        "a renewal copies the anchors into its generation"
    );
    let texts = manifests(&root.0, &session_id)?;
    assert!(
        texts[1..]
            .iter()
            .all(|text| text.contains(r#""time_anchors":[{"source":"operator""#)),
        "every generation after the opening one names the anchors"
    );
    Ok(())
}

/// A session without anchors writes no `time_anchors` member, so an earlier
/// release reads it as it always did.
#[tokio::test]
async fn a_session_without_anchors_is_recorded_as_before() -> TestResult {
    let root = OwnedRoot::new()?;
    let (store, session_id, _) = open_with(&root.0, &TimeAnchors::none()).await?;
    assert!(store.session_status(&session_id)?.time_anchors().is_empty());
    for text in manifests(&root.0, &session_id)? {
        assert!(!text.contains("time_anchors"), "{text}");
    }
    Ok(())
}
