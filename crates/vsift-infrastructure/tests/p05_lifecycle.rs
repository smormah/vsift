//! Cross-process-capable lifecycle boundary checks on the real filesystem adapter.

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use vsift_application::{
    InitializeSessionStorage, InitializeSessionStorageRequest, SessionStorageError,
};
use vsift_domain::{
    DurabilityRequirement, OperationId, SessionArtifactKind, SessionId, SessionLifetime,
    SessionPhase,
};
use vsift_infrastructure::{
    BundleSourcePolicy, CleanOutcome, FilesystemSessionStore, SourceSnapshot,
};

type TestResult = Result<(), Box<dyn Error>>;
const CHILD_ROOT: &str = "VSIFT_P05_CHILD_ROOT";
const CHILD_READY: &str = "VSIFT_P05_CHILD_READY";
const CHILD_MODE: &str = "VSIFT_P05_CHILD_MODE";

struct ManagedChild(Child);

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_holder(root: &Path, ready: &Path, mode: &str) -> Result<ManagedChild, Box<dyn Error>> {
    let child = Command::new(env::current_exe()?)
        .args([
            "--exact",
            "lifecycle_child_holds_lock",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_ROOT, root)
        .env(CHILD_READY, ready)
        .env(CHILD_MODE, mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(ManagedChild(child))
}

fn await_ready(child: &mut ManagedChild, ready: &Path) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if ready.exists() {
            return Ok(());
        }
        if let Some(status) = child.0.try_wait()? {
            return Err(format!("lifecycle holder exited before ready: {status}").into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err("lifecycle holder did not become ready".into())
}

#[test]
#[ignore = "internal child fixture launched by watchdog-bounded parent tests"]
fn lifecycle_child_holds_lock() -> TestResult {
    let root = PathBuf::from(env::var_os(CHILD_ROOT).ok_or("missing child root")?);
    let ready = PathBuf::from(env::var_os(CHILD_READY).ok_or("missing child ready path")?);
    let mode = env::var(CHILD_MODE)?;
    let store = FilesystemSessionStore::open_existing(root)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    if mode == "read" {
        let _hold = store.acquire_read(&session_id)?;
        fs::write(ready, b"ready")?;
        thread::sleep(Duration::from_secs(30));
    } else if mode == "register" {
        let _registration = store.register_session(
            &session_id,
            &OperationId::parse("op_9999999999999999")?,
            1_000,
        )?;
        fs::write(ready, b"ready")?;
        thread::sleep(Duration::from_secs(30));
    } else {
        return Err("unknown child mode".into());
    }
    Ok(())
}

struct OwnedRoot(PathBuf);

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "vsift-p05-test-{}-{stamp}-{sequence}",
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
            .is_some_and(|name| name.starts_with("vsift-p05-test-"))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the cross-process lifecycle and preservation assertions in one journey"
)]
async fn open_renew_close_are_committed_and_active_source_blocks_close() -> TestResult {
    let root = OwnedRoot::new()?;
    let source = root.0.join("source.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomtest")?;
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
    let store = FilesystemSessionStore::open_existing(&workspace)?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        &source,
    )?;
    let initial_generation = store.activate_source(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        vsift_domain::StorageGeneration::INITIAL,
        1_000,
    )?;
    let generation = store.publish_artifact(
        &session_id,
        &OperationId::parse("op_6666666666666666")?,
        initial_generation,
        SessionArtifactKind::FramePng,
        b"png-artifact",
        1_001,
    )?;
    drop(registration);
    let reopened = FilesystemSessionStore::open_existing(&workspace)?;
    let status = reopened.session_status(&session_id)?;
    assert_eq!(status.phase(), SessionPhase::Open);
    assert_eq!(status.source_id(), snapshot.id());
    assert_eq!(status.artifact_count(), 1);
    assert_eq!(status.artifact_bytes(), 12);
    assert_eq!(
        status.lifetime().expires_at_unix_seconds(),
        1_000 + SessionLifetime::IDLE_SECONDS
    );
    let evidence_only = root.0.join("evidence-only");
    let retained = reopened.retain_bundle(
        &session_id,
        &evidence_only,
        BundleSourcePolicy::EvidenceOnly,
    )?;
    assert_eq!(retained.source_policy(), BundleSourcePolicy::EvidenceOnly);
    assert_eq!(retained.artifact_count(), 1);
    assert!(!evidence_only.join("source.media").exists());
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&evidence_only)?,
        retained
    );
    let portable = root.0.join("portable");
    let retained =
        reopened.retain_bundle(&session_id, &portable, BundleSourcePolicy::IncludeSource)?;
    assert_eq!(retained.source_policy(), BundleSourcePolicy::IncludeSource);
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&portable)?,
        retained
    );
    let moved = root.0.join("moved");
    fs::rename(&portable, &moved)?;
    assert_eq!(FilesystemSessionStore::validate_bundle(&moved)?, retained);
    let existing = reopened.retain_bundle(&session_id, &moved, BundleSourcePolicy::IncludeSource);
    assert!(matches!(existing, Err(SessionStorageError::StateConflict)));
    assert_eq!(
        reopened.close_session(
            &session_id,
            &OperationId::parse("op_3333333333333333")?,
            generation
        ),
        Err(SessionStorageError::Busy)
    );
    assert_eq!(
        reopened.clean_session(&session_id, 1_000 + SessionLifetime::MAX_SECONDS, false),
        Err(SessionStorageError::Busy)
    );
    drop(snapshot);
    let renewed = reopened.renew_session(
        &session_id,
        &OperationId::parse("op_4444444444444444")?,
        generation,
        2_000,
    )?;
    assert_eq!(
        reopened
            .session_status(&session_id)?
            .lifetime()
            .expires_at_unix_seconds(),
        2_000 + SessionLifetime::IDLE_SECONDS
    );
    reopened.close_session(
        &session_id,
        &OperationId::parse("op_5555555555555555")?,
        renewed,
    )?;
    assert_eq!(
        reopened.session_status(&session_id)?.phase(),
        SessionPhase::Closed
    );
    assert_eq!(
        reopened.clean_session(&session_id, 2_000, true)?,
        CleanOutcome::Eligible
    );
    assert_eq!(
        reopened.clean_session(&session_id, 2_000, false)?,
        CleanOutcome::Removed
    );
    assert!(matches!(
        reopened.session_status(&session_id),
        Err(SessionStorageError::IntegrityFailure)
    ));
    assert_eq!(FilesystemSessionStore::validate_bundle(&moved)?, retained);
    assert_eq!(fs::read(&source)?, b"\0\0\0\x18ftypisomtest");
    fs::remove_file(moved.join("source.media"))?;
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&moved),
        Err(SessionStorageError::IntegrityFailure)
    );
    fs::write(moved.join("source.media"), b"\0\0\0\x18ftypisomtest")?;
    assert_eq!(FilesystemSessionStore::validate_bundle(&moved)?, retained);
    fs::write(moved.join("source.media"), b"changed")?;
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&moved),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

#[test]
fn registration_scan_and_abandoned_cleanup_respect_the_live_lock() -> TestResult {
    let root = OwnedRoot::new()?;
    let workspace = root.0.join("workspace");
    let store = FilesystemSessionStore::provision_default(&workspace)?;
    let session_id = SessionId::parse("ses_aaaaaaaaaaaaaaaa")?;
    let registration = store.register_session(
        &session_id,
        &OperationId::parse("op_bbbbbbbbbbbbbbbb")?,
        1_000,
    )?;
    let mut found = false;
    for bucket in 0..=255 {
        let page = store
            .scan_session_bucket(bucket)
            .map_err(|error| format!("scan {bucket}: {error:?}"))?;
        assert!(page.session_ids().len() <= 256);
        found |= page.session_ids().contains(&session_id);
    }
    assert!(found);
    assert_eq!(
        store.clean_session(&session_id, 1_000 + SessionLifetime::MAX_SECONDS, false),
        Err(SessionStorageError::Busy)
    );
    drop(registration);
    assert_eq!(
        store
            .clean_session(
                &session_id,
                1_000 + SessionLifetime::IDLE_SECONDS - 1,
                false
            )
            .map_err(|error| format!("early clean: {error:?}"))?,
        CleanOutcome::Ineligible
    );
    assert_eq!(
        store
            .clean_session(&session_id, 1_000 + SessionLifetime::IDLE_SECONDS, true)
            .map_err(|error| format!("dry clean: {error:?}"))?,
        CleanOutcome::Eligible
    );
    assert_eq!(
        store
            .clean_session(&session_id, 1_000 + SessionLifetime::IDLE_SECONDS, false)
            .map_err(|error| format!("live clean: {error:?}"))?,
        CleanOutcome::Removed
    );
    assert_eq!(fs::read_dir(root.0.join("workspace/sessions"))?.count(), 0);
    Ok(())
}

#[tokio::test]
async fn cleaner_never_steals_live_cross_process_read_or_registration() -> TestResult {
    let temp = OwnedRoot::new()?;
    let workspace = temp.0.join("workspace");
    let store = FilesystemSessionStore::provision_default(&workspace)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    let store = FilesystemSessionStore::open_existing(&workspace)?;
    let ready = temp.0.join("read-ready");
    let mut reader = spawn_holder(&workspace, &ready, "read")?;
    await_ready(&mut reader, &ready)?;
    assert_eq!(
        store.clean_session(&session_id, u64::MAX, false),
        Err(SessionStorageError::Busy)
    );
    drop(reader);
    assert_eq!(
        store.clean_session(&session_id, u64::MAX, false)?,
        CleanOutcome::Removed
    );

    let ready = temp.0.join("registration-ready");
    let mut opener = spawn_holder(&workspace, &ready, "register")?;
    await_ready(&mut opener, &ready)?;
    assert_eq!(
        store.clean_session(&session_id, u64::MAX, false),
        Err(SessionStorageError::Busy)
    );
    drop(opener);
    assert_eq!(
        store.clean_session(&session_id, u64::MAX, false)?,
        CleanOutcome::Removed
    );
    Ok(())
}

#[test]
fn bounded_index_refuses_a_257th_marker_in_one_bucket() -> TestResult {
    let temp = OwnedRoot::new()?;
    let store = FilesystemSessionStore::provision_default(temp.0.join("workspace"))?;
    let mut candidates = Vec::new();
    let mut selected_bucket = None;
    for counter in 0_u64..100_000 {
        let session_id = SessionId::parse(format!("ses_{counter:016x}"))?;
        let bucket = Sha256::digest(session_id.as_str().as_bytes())[0];
        if selected_bucket.is_none() {
            selected_bucket = Some(bucket);
        }
        if Some(bucket) == selected_bucket {
            candidates.push(session_id);
            if candidates.len() == 257 {
                break;
            }
        }
    }
    assert_eq!(candidates.len(), 257);
    for session_id in candidates.iter().take(256) {
        let registration = store.register_session(
            session_id,
            &OperationId::parse("op_aaaaaaaaaaaaaaaa")?,
            1_000,
        )?;
        drop(registration);
    }
    let bucket = u16::from(selected_bucket.ok_or("missing selected bucket")?);
    assert_eq!(store.scan_session_bucket(bucket)?.session_ids().len(), 256);
    assert!(matches!(
        store.register_session(
            &candidates[256],
            &OperationId::parse("op_bbbbbbbbbbbbbbbb")?,
            1_000,
        ),
        Err(SessionStorageError::CapacityExhausted)
    ));
    Ok(())
}

#[tokio::test]
async fn corrupt_root_ownership_prevents_cleanup_without_source_deletion() -> TestResult {
    let temp = OwnedRoot::new()?;
    let workspace = temp.0.join("workspace");
    let store = FilesystemSessionStore::provision_default(&workspace)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    let source = temp.0.join("original.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomoriginal")?;
    let store = FilesystemSessionStore::open_existing(&workspace)?;
    fs::write(workspace.join("ownership.json"), b"corrupt")?;
    assert!(matches!(
        store.clean_session(&session_id, u64::MAX, false),
        Err(SessionStorageError::IntegrityFailure)
    ));
    assert!(
        workspace
            .join("sessions")
            .join(session_id.as_str())
            .exists()
    );
    assert_eq!(fs::read(&source)?, b"\0\0\0\x18ftypisomoriginal");
    Ok(())
}

#[test]
fn malicious_bundle_paths_and_future_versions_fail_data_only_validation() -> TestResult {
    let temp = OwnedRoot::new()?;
    let bundle = temp.0.join("bundle");
    fs::create_dir(&bundle)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bundle, fs::Permissions::from_mode(0o700))?;
    }
    let path = bundle.join("bundle.json");
    let base = serde_json::json!({
        "schema_version": 1,
        "format": "vsift.bundle",
        "session_id": "ses_0123456789abcdef",
        "source_id": format!("src_sha256_{}", "a".repeat(64)),
        "source_bytes": 1,
        "source_included": false,
        "publication": "process_crash_consistent",
        "artifacts": [{
            "kind": "frame_png",
            "name": "../outside",
            "sha256": "a".repeat(64),
            "bytes": 1
        }]
    });
    fs::write(&path, serde_json::to_vec(&base)?)?;
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&bundle),
        Err(SessionStorageError::IntegrityFailure)
    );
    let mut future = base;
    future["schema_version"] = serde_json::json!(2);
    fs::write(&path, serde_json::to_vec(&future)?)?;
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&bundle),
        Err(SessionStorageError::UnsupportedVersion)
    );
    Ok(())
}

#[tokio::test]
async fn concurrent_renew_and_close_commit_exactly_one_transition() -> TestResult {
    let temp = OwnedRoot::new()?;
    let workspace = temp.0.join("workspace");
    let source = temp.0.join("source.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomsource")?;
    let store = FilesystemSessionStore::provision_default(&workspace)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    let store = Arc::new(FilesystemSessionStore::open_existing(&workspace)?);
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        &source,
    )?;
    let generation = store.activate_source(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        vsift_domain::StorageGeneration::INITIAL,
        1_000,
    )?;
    drop(snapshot);
    let barrier = Arc::new(Barrier::new(3));
    let renew_store = Arc::clone(&store);
    let renew_id = session_id.clone();
    let renew_barrier = Arc::clone(&barrier);
    let renew = thread::spawn(move || {
        renew_barrier.wait();
        renew_store.renew_session(
            &renew_id,
            &OperationId::parse("op_3333333333333333").map_err(|_| SessionStorageError::Io)?,
            generation,
            2_000,
        )
    });
    let close_store = Arc::clone(&store);
    let close_id = session_id.clone();
    let close_barrier = Arc::clone(&barrier);
    let close = thread::spawn(move || {
        close_barrier.wait();
        close_store.close_session(
            &close_id,
            &OperationId::parse("op_4444444444444444").map_err(|_| SessionStorageError::Io)?,
            generation,
        )
    });
    barrier.wait();
    let renew_result = renew.join().map_err(|_| "renew thread panicked")?;
    let close_result = close.join().map_err(|_| "close thread panicked")?;
    assert_eq!(
        usize::from(renew_result.is_ok()) + usize::from(close_result.is_ok()),
        1
    );
    assert!(matches!(
        renew_result,
        Ok(_) | Err(SessionStorageError::Busy | SessionStorageError::StateConflict)
    ));
    assert!(matches!(
        close_result,
        Ok(_) | Err(SessionStorageError::Busy | SessionStorageError::StateConflict)
    ));
    assert_eq!(
        store.session_status(&session_id)?.generation().value(),
        generation.value() + 1
    );
    Ok(())
}

#[tokio::test]
async fn cleanup_refuses_external_hard_link_and_preserves_its_source() -> TestResult {
    let temp = OwnedRoot::new()?;
    let workspace = temp.0.join("workspace");
    let external = temp.0.join("original.mp4");
    fs::write(&external, b"original must survive")?;
    let store = FilesystemSessionStore::provision_default(&workspace)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    let link = workspace
        .join("sessions")
        .join(session_id.as_str())
        .join("artifacts")
        .join("external-link");
    fs::hard_link(&external, &link)?;
    let store = FilesystemSessionStore::open_existing(&workspace)?;
    assert_eq!(
        store.clean_session(&session_id, u64::MAX, false),
        Err(SessionStorageError::IntegrityFailure)
    );
    assert_eq!(fs::read(&external)?, b"original must survive");
    assert!(
        workspace
            .join("sessions")
            .join(session_id.as_str())
            .exists()
    );
    Ok(())
}
