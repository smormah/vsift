//! P10 PR 1 (ADR 0020): incremental chain validation (#164), the durable
//! publication order, fsyncgate handling and process kills at every fault
//! point (S-07, S-08).

use std::{
    cell::RefCell,
    error::Error,
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use vsift_application::{SessionStorageError, StorageCapabilities};
use vsift_domain::{
    DurabilityRequirement, EvidenceMediaKind, OperationId, PublicationGuarantee, SessionId,
    StorageGeneration,
};

use super::{
    ARTIFACTS_DIRECTORY, CHAIN_CHECKPOINT_FILE, CURRENT_FILE, ChainCheck, EvidenceMediaFile,
    FilesystemSessionStore, GENERATIONS_DIRECTORY, LifecycleUpdate, SESSIONS_DIRECTORY,
    StoredDurability,
    chain::read_committed_manifest,
    commit::{CommitHooks, DirRole, FsOp},
    evidence::EvidenceFiles,
    initialization::{InitialGeneration, initialize},
    publication::{LifetimeHold, publish_generation, publish_generation_with_update},
    sha256_hex,
    tests::{Fixture, TestResult, publication_request},
};
use crate::fault_point::{FAULT_EXIT_CODE, FAULT_MARKER, FAULT_POINT_VARIABLE, FaultPoint};

pub(super) type Built<T> = Result<T, Box<dyn Error>>;

pub(super) const SESSION: &str = "ses_0123456789abcdef";
const INITIALIZE: &str = "op_0123456789abcdef";
const ACTIVATE: &str = "op_2222222222222222";
const FIRST_EVIDENCE: &str = "op_3333333333333333";
const CRASHED_EVIDENCE: &str = "op_7777777777777777";
pub(super) const SOURCE_BYTES: &[u8] = b"stand-in source copy";
const CHILD_ROOT: &str = "VSIFT_TEST_COMMIT_ROOT";

pub(super) fn session_id() -> Built<SessionId> {
    Ok(SessionId::parse(SESSION)?)
}

pub(super) fn operation(text: &str) -> Built<OperationId> {
    Ok(OperationId::parse(text)?)
}

pub(super) fn now() -> Built<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

pub(super) fn session_path(fixture: &Fixture) -> PathBuf {
    fixture.path.join(SESSIONS_DIRECTORY).join(SESSION)
}

pub(super) fn initialize_as(
    store: &FilesystemSessionStore,
    durability: StoredDurability,
    hooks: &CommitHooks<'_>,
) -> Built<StorageGeneration> {
    Ok(initialize(
        &store.root,
        &InitialGeneration {
            session_id: &session_id()?,
            operation_id: &operation(INITIALIZE)?,
            durability,
        },
        hooks,
    )?)
}

/// Generation 1: activation over a source copy as staging leaves it.
pub(super) fn activate(
    fixture: &Fixture,
    store: &FilesystemSessionStore,
    hooks: &CommitHooks<'_>,
) -> Built<StorageGeneration> {
    let name = format!("source-{ACTIVATE}.media");
    fs::write(
        session_path(fixture).join(ARTIFACTS_DIRECTORY).join(&name),
        SOURCE_BYTES,
    )?;
    Ok(publish_generation_with_update(
        &store.root,
        store.admission_capacity,
        &publication_request(ACTIVATE, 0, DurabilityRequirement::Ephemeral)?,
        LifecycleUpdate::Activate {
            source_id: format!("src_sha256_{}", sha256_hex(SOURCE_BYTES)),
            source_name: name,
            source_bytes: u64::try_from(SOURCE_BYTES.len())?,
            now: now()?,
            artifacts: Vec::new(),
        },
        LifetimeHold::Shared,
        hooks,
        None,
    )?)
}

/// A frame image whose bytes are unique to `marker`; the store checks only
/// its size and digest.
fn image(marker: &str) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(marker.as_bytes());
    bytes
}

/// Commits one evidence generation of `images` and a record for `marker`.
fn commit_evidence(
    store: &FilesystemSessionStore,
    operation_text: &str,
    expected: u64,
    images: &[Vec<u8>],
    marker: &str,
    hooks: &CommitHooks<'_>,
) -> Result<StorageGeneration, Box<dyn Error>> {
    let media: Vec<EvidenceMediaFile<'_>> = images
        .iter()
        .map(|bytes| EvidenceMediaFile {
            kind: EvidenceMediaKind::FramePng,
            bytes,
        })
        .collect();
    let record = format!("{{\"marker\":\"{marker}\"}}");
    Ok(store.commit_evidence(
        &session_id()?,
        &operation(operation_text)?,
        StorageGeneration::from_value(expected),
        &EvidenceFiles {
            media: &media,
            record: record.as_bytes(),
            verified_identity: None,
            now_unix_seconds: now()?,
        },
        hooks,
    )?)
}

/// An open session at generation 2: initialized, activated and holding one
/// evidence commit.
pub(super) fn open_session(
    fixture: &Fixture,
    durability: StoredDurability,
) -> Built<FilesystemSessionStore> {
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let hooks = CommitHooks::new();
    initialize_as(&store, durability, &hooks)?;
    activate(fixture, &store, &hooks)?;
    commit_evidence(
        &store,
        FIRST_EVIDENCE,
        1,
        &[image("first")],
        "first",
        &hooks,
    )?;
    Ok(store)
}

fn publish_keep(store: &FilesystemSessionStore, operation_text: &str, expected: u64) -> Built<()> {
    publish_generation(
        &store.root,
        store.admission_capacity,
        &publication_request(operation_text, expected, DurabilityRequirement::Ephemeral)?,
        &CommitHooks::new(),
    )?;
    Ok(())
}

/// An ephemeral session published to `generations` with plain generations.
fn chain_of(fixture: &Fixture, generations: u64) -> Built<FilesystemSessionStore> {
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    initialize_as(&store, StoredDurability::Ephemeral, &CommitHooks::new())?;
    for expected in 0..generations {
        publish_keep(&store, &format!("op_{:016x}", expected + 0x100), expected)?;
    }
    Ok(store)
}

fn read_error(fixture: &Fixture) -> Built<Option<SessionStorageError>> {
    // A new instance: nothing it verified earlier stands in for the checkpoint.
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    Ok(store.acquire_read(&session_id()?).err())
}

fn checkpoint_path(fixture: &Fixture) -> PathBuf {
    session_path(fixture).join(CHAIN_CHECKPOINT_FILE)
}

fn generation_digest(fixture: &Fixture, generation: u64) -> Built<String> {
    Ok(sha256_hex(&fs::read(
        session_path(fixture)
            .join(GENERATIONS_DIRECTORY)
            .join(format!("{generation}.json")),
    )?))
}

fn write_checkpoint(fixture: &Fixture, text: &str) -> Built<()> {
    fs::write(checkpoint_path(fixture), text)?;
    Ok(())
}

fn flip_first_byte(path: &PathBuf) -> Built<()> {
    let mut bytes = fs::read(path)?;
    if let Some(first) = bytes.first_mut() {
        *first ^= 1;
    }
    fs::write(path, bytes)?;
    Ok(())
}

#[test]
fn publication_advances_the_checkpoint_to_the_head_and_readers_never_write_it() -> TestResult {
    let fixture = Fixture::new()?;
    let store = chain_of(&fixture, 3)?;
    let checkpoint: serde_json::Value =
        serde_json::from_slice(&fs::read(checkpoint_path(&fixture))?)?;
    assert_eq!(checkpoint["schema_version"], 1);
    assert_eq!(checkpoint["generation"], 3);
    assert_eq!(
        checkpoint["manifest_sha256"],
        generation_digest(&fixture, 3)?
    );

    fs::remove_file(checkpoint_path(&fixture))?;
    let hold = store.acquire_read(&session_id()?)?;
    assert_eq!(hold.generation(), StorageGeneration::from_value(3));
    drop(hold);
    assert!(
        !checkpoint_path(&fixture).exists(),
        "a reader wrote the checkpoint"
    );
    Ok(())
}

/// #164: a read stops at the checkpoint; without one it walks the whole chain.
#[test]
fn reads_stop_at_the_checkpoint_and_walk_everything_without_one() -> TestResult {
    let fixture = Fixture::new()?;
    chain_of(&fixture, 4)?;
    let first = session_path(&fixture)
        .join(GENERATIONS_DIRECTORY)
        .join("1.json");
    flip_first_byte(&first)?;
    // Generation 1 lies below the verified head, so reads do not see it...
    assert_eq!(read_error(&fixture)?, None);
    // ...but a full walk does (retained exports and cleanup).
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let session = store
        .root
        .open_dir(PathBuf::from(SESSIONS_DIRECTORY).join(SESSION))?;
    assert_eq!(
        read_committed_manifest(&session, &session_id()?, ChainCheck::Full).err(),
        Some(SessionStorageError::IntegrityFailure)
    );
    // Without a checkpoint every read walks the whole chain again.
    fs::remove_file(checkpoint_path(&fixture))?;
    assert_eq!(
        read_error(&fixture)?,
        Some(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

/// S-08 for the checkpoint: truncated, forged, malformed and future-version
/// checkpoints fail closed; one ahead of the head means a full walk.
#[test]
fn damaged_forged_or_future_checkpoints_fail_closed() -> TestResult {
    let fixture = Fixture::new()?;
    chain_of(&fixture, 4)?;
    let head = generation_digest(&fixture, 4)?;
    let two = generation_digest(&fixture, 2)?;
    let other = "a".repeat(64);
    for (text, expected) in [
        (
            format!(r#"{{"schema_version":1,"generation":4,"manifest_sha256":"{head}""#),
            SessionStorageError::IntegrityFailure,
        ),
        (
            format!(r#"{{"schema_version":1,"generation":4,"manifest_sha256":"{other}"}}"#),
            SessionStorageError::IntegrityFailure,
        ),
        (
            format!(r#"{{"schema_version":1,"generation":2,"manifest_sha256":"{other}"}}"#),
            SessionStorageError::IntegrityFailure,
        ),
        (
            format!(
                r#"{{"schema_version":1,"generation":2,"manifest_sha256":"{}"}}"#,
                two.to_uppercase()
            ),
            SessionStorageError::IntegrityFailure,
        ),
        (
            format!(r#"{{"schema_version":1,"generation":2,"manifest_sha256":"{two}","extra":1}}"#),
            SessionStorageError::IntegrityFailure,
        ),
        (
            format!(r#"{{"schema_version":2,"generation":2,"manifest_sha256":"{two}"}}"#),
            SessionStorageError::UnsupportedVersion,
        ),
        (String::new(), SessionStorageError::IntegrityFailure),
    ] {
        write_checkpoint(&fixture, &text)?;
        assert_eq!(read_error(&fixture)?, Some(expected), "{text}");
    }

    // A valid checkpoint below the head stops the walk there.
    write_checkpoint(
        &fixture,
        &format!(r#"{{"schema_version":1,"generation":2,"manifest_sha256":"{two}"}}"#),
    )?;
    assert_eq!(read_error(&fixture)?, None);

    // One ahead of the head is ignored: the whole chain is walked, and a
    // damaged generation 0 is found.
    write_checkpoint(
        &fixture,
        &format!(r#"{{"schema_version":1,"generation":9,"manifest_sha256":"{other}"}}"#),
    )?;
    assert_eq!(read_error(&fixture)?, None);
    flip_first_byte(
        &session_path(&fixture)
            .join(GENERATIONS_DIRECTORY)
            .join("0.json"),
    )?;
    assert_eq!(
        read_error(&fixture)?,
        Some(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

/// A generation changed at or above the checkpoint is found at its link or at
/// the checkpoint itself.
#[test]
fn a_tampered_generation_is_detected_down_to_its_checkpoint() -> TestResult {
    let fixture = Fixture::new()?;
    chain_of(&fixture, 5)?;
    let two = generation_digest(&fixture, 2)?;
    write_checkpoint(
        &fixture,
        &format!(r#"{{"schema_version":1,"generation":2,"manifest_sha256":"{two}"}}"#),
    )?;
    let generations = session_path(&fixture).join(GENERATIONS_DIRECTORY);
    for generation in [2, 3] {
        let path = generations.join(format!("{generation}.json"));
        let original = fs::read(&path)?;
        flip_first_byte(&path)?;
        assert_eq!(
            read_error(&fixture)?,
            Some(SessionStorageError::IntegrityFailure),
            "generation {generation}"
        );
        fs::write(&path, original)?;
    }
    assert_eq!(read_error(&fixture)?, None);
    Ok(())
}

/// The verified head is per store instance: another instance, or another
/// process, starts from the checkpoint.
#[test]
fn the_verified_head_cache_belongs_to_one_store_instance() -> TestResult {
    let fixture = Fixture::new()?;
    let store = chain_of(&fixture, 3)?;
    drop(store.acquire_read(&session_id()?)?);
    fs::remove_file(checkpoint_path(&fixture))?;
    flip_first_byte(
        &session_path(&fixture)
            .join(GENERATIONS_DIRECTORY)
            .join("0.json"),
    )?;
    // This instance verified the head already...
    drop(store.acquire_read(&session_id()?)?);
    // ...a new one walks the chain in full and fails.
    assert_eq!(
        read_error(&fixture)?,
        Some(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

#[test]
fn ephemeral_manifests_omit_the_durability_field_and_durable_ones_record_it() -> TestResult {
    let fixture = Fixture::new()?;
    chain_of(&fixture, 1)?;
    for generation in [0, 1] {
        let text = fs::read_to_string(
            session_path(&fixture)
                .join(GENERATIONS_DIRECTORY)
                .join(format!("{generation}.json")),
        )?;
        assert!(!text.contains("durability"), "{text}");
    }
    let durable = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&durable.path)?;
    let trace = RefCell::new(Vec::new());
    initialize_as(
        &store,
        StoredDurability::Durable,
        &CommitHooks::traced(&trace),
    )?;
    let text = fs::read_to_string(
        session_path(&durable)
            .join(GENERATIONS_DIRECTORY)
            .join("0.json"),
    )?;
    assert!(text.contains(r#""durability":"durable""#), "{text}");
    Ok(())
}

/// A session's mode never changes: a durable request cannot upgrade an
/// ephemeral session, and reinitializing with another mode conflicts.
#[test]
fn an_ephemeral_session_cannot_be_published_durably() -> TestResult {
    let fixture = Fixture::new()?;
    let store = chain_of(&fixture, 0)?;
    assert_eq!(
        publish_generation(
            &store.root,
            store.admission_capacity,
            &publication_request("op_4444444444444444", 0, DurabilityRequirement::Durable)?,
            &CommitHooks::new(),
        ),
        Err(SessionStorageError::StateConflict)
    );
    assert_eq!(
        initialize_as(&store, StoredDurability::Durable, &CommitHooks::new())
            .err()
            .and_then(|error| error.downcast_ref::<SessionStorageError>().copied()),
        Some(SessionStorageError::StateConflict)
    );
    Ok(())
}

/// No profile is qualified yet (ADR 0010): durable requests keep failing
/// closed with an unsupported guarantee.
#[test]
fn every_root_still_offers_only_process_crash_consistency() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    assert_eq!(
        store.capabilities,
        StorageCapabilities::new(PublicationGuarantee::ProcessCrashConsistent)
    );
    Ok(())
}

fn artifact_name(bytes: &[u8], extension: &str) -> String {
    format!("artifact-{}.{extension}", sha256_hex(bytes))
}

fn staged(operation_text: &str, generation: u64, suffix: &str) -> String {
    format!("{operation_text}.{generation}.{suffix}.tmp")
}

fn created(role: DirRole, name: &str) -> [FsOp; 3] {
    [
        FsOp::Create(role, name.to_owned()),
        FsOp::Write(role, name.to_owned()),
        FsOp::SyncFile(role, name.to_owned()),
    ]
}

/// The exact durable order of an evidence commit (ADR 0020).
fn expected_evidence_order(image: &[u8], record: &[u8], generation: u64) -> Vec<FsOp> {
    let manifest = staged(FIRST_EVIDENCE, generation, "manifest");
    let pointer = staged(FIRST_EVIDENCE, generation, "pointer");
    let chain = staged(FIRST_EVIDENCE, generation, "chain");
    let mut order = Vec::new();
    order.extend(created(DirRole::Artifacts, &artifact_name(image, "png")));
    order.extend(created(DirRole::Artifacts, &artifact_name(record, "json")));
    order.push(FsOp::SyncDirectory(DirRole::Artifacts));
    order.extend(created(DirRole::Attempts, &manifest));
    order.push(FsOp::Rename(
        DirRole::Attempts,
        manifest,
        DirRole::Generations,
        format!("{generation}.json"),
    ));
    order.push(FsOp::SyncDirectory(DirRole::Generations));
    order.extend(created(DirRole::Attempts, &pointer));
    order.push(FsOp::Rename(
        DirRole::Attempts,
        pointer,
        DirRole::Session,
        CURRENT_FILE.to_owned(),
    ));
    order.push(FsOp::SyncDirectory(DirRole::Session));
    order.push(FsOp::Committed);
    order.extend(created(DirRole::Attempts, &chain));
    order.push(FsOp::Rename(
        DirRole::Attempts,
        chain,
        DirRole::Session,
        CHAIN_CHECKPOINT_FILE.to_owned(),
    ));
    order
}

fn without_directory_syncs(order: &[FsOp]) -> Vec<FsOp> {
    order
        .iter()
        .filter(|operation| !matches!(operation, FsOp::SyncDirectory(_)))
        .cloned()
        .collect()
}

/// The syscall order of a durable session's initialization and evidence
/// commit, on every platform (where a directory cannot be synchronised
/// through a capability handle, the recorder stands in for the call). An
/// ephemeral session performs the same steps without the directory syncs.
#[test]
fn a_durable_commit_follows_the_documented_order() -> TestResult {
    let image_bytes = image("ordered");
    let record = br#"{"marker":"ordered"}"#;
    for durability in [StoredDurability::Durable, StoredDurability::Ephemeral] {
        let fixture = Fixture::new()?;
        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        drop(store.register_session(&session_id()?, &operation(INITIALIZE)?, now()?)?);

        let trace = RefCell::new(Vec::new());
        initialize_as(&store, durability, &CommitHooks::traced(&trace))?;
        let attempt = format!(".initialize-{SESSION}-{INITIALIZE}");
        let mut expected = Vec::new();
        expected.extend(created(DirRole::Generations, "0.json"));
        expected.push(FsOp::SyncDirectory(DirRole::Generations));
        expected.extend(created(DirRole::Session, CURRENT_FILE));
        expected.push(FsOp::SyncDirectory(DirRole::Session));
        expected.push(FsOp::RenameDirectory(
            DirRole::Sessions,
            attempt,
            DirRole::Sessions,
            SESSION.to_owned(),
        ));
        expected.extend([
            FsOp::SyncDirectory(DirRole::Sessions),
            FsOp::SyncDirectory(DirRole::IndexBucket),
            FsOp::SyncDirectory(DirRole::SessionIndex),
            FsOp::Committed,
        ]);
        if durability == StoredDurability::Ephemeral {
            expected = without_directory_syncs(&expected);
        }
        assert_eq!(trace.take(), expected, "{durability:?} initialization");

        activate(&fixture, &store, &CommitHooks::traced(&trace))?;
        let activation = trace.take();
        assert_eq!(
            activation.first() == Some(&FsOp::SyncDirectory(DirRole::Artifacts)),
            durability == StoredDurability::Durable,
            "{durability:?} activation makes the source copy reachable first"
        );

        commit_evidence(
            &store,
            FIRST_EVIDENCE,
            1,
            std::slice::from_ref(&image_bytes),
            "ordered",
            &CommitHooks::traced(&trace),
        )?;
        let mut expected = expected_evidence_order(&image_bytes, record, 2);
        if durability == StoredDurability::Ephemeral {
            expected = without_directory_syncs(&expected);
        }
        assert_eq!(trace.take(), expected, "{durability:?} evidence commit");
        assert_eq!(
            store.session_status(&session_id()?)?.generation(),
            StorageGeneration::from_value(2)
        );
    }
    Ok(())
}

/// fsyncgate: after a failed attempt, a durable retry never trusts what that
/// attempt flushed. It re-stages its artifacts, stages the manifest again
/// over the one already renamed, and deletes and rewrites the leftover
/// pointer; an ephemeral retry keeps them.
#[test]
fn a_durable_retry_rewrites_what_a_failed_attempt_left() -> TestResult {
    let image_bytes = image("retried");
    for durability in [StoredDurability::Durable, StoredDurability::Ephemeral] {
        let fixture = Fixture::new()?;
        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        let trace = RefCell::new(Vec::new());
        initialize_as(&store, durability, &CommitHooks::traced(&trace))?;
        activate(&fixture, &store, &CommitHooks::traced(&trace))?;
        assert!(
            commit_evidence(
                &store,
                FIRST_EVIDENCE,
                1,
                std::slice::from_ref(&image_bytes),
                "retried",
                &CommitHooks::traced(&trace).failing(FaultPoint::PointerFlush),
            )
            .is_err()
        );
        trace.take();
        commit_evidence(
            &store,
            FIRST_EVIDENCE,
            1,
            std::slice::from_ref(&image_bytes),
            "retried",
            &CommitHooks::traced(&trace),
        )?;
        let retry = trace.take();
        let image_name = artifact_name(&image_bytes, "png");
        let pointer = staged(FIRST_EVIDENCE, 2, "pointer");
        let durable = durability == StoredDurability::Durable;
        assert_eq!(
            retry.contains(&FsOp::Accept(DirRole::Artifacts, image_name.clone())),
            !durable,
            "{durability:?}"
        );
        assert_eq!(
            retry.contains(&FsOp::Rename(
                DirRole::Attempts,
                format!("{FIRST_EVIDENCE}.{}.artifact.tmp", sha256_hex(&image_bytes)),
                DirRole::Artifacts,
                image_name,
            )),
            durable,
            "{durability:?}"
        );
        assert_eq!(
            retry.contains(&FsOp::Accept(DirRole::Generations, "2.json".to_owned())),
            !durable,
            "{durability:?}"
        );
        assert_eq!(
            retry.contains(&FsOp::Remove(DirRole::Attempts, pointer.clone())),
            durable,
            "{durability:?}"
        );
        assert_eq!(
            retry.contains(&FsOp::Accept(DirRole::Attempts, pointer)),
            !durable,
            "{durability:?}"
        );
        assert_eq!(
            store.session_status(&session_id()?)?.generation(),
            StorageGeneration::from_value(2)
        );
    }
    Ok(())
}

/// A durable commit accepts an existing file only when the committed head
/// lists it.
#[test]
fn a_durable_commit_accepts_a_file_the_head_already_lists() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let trace = RefCell::new(Vec::new());
    initialize_as(
        &store,
        StoredDurability::Durable,
        &CommitHooks::traced(&trace),
    )?;
    activate(&fixture, &store, &CommitHooks::traced(&trace))?;
    let shared = image("shared");
    commit_evidence(
        &store,
        FIRST_EVIDENCE,
        1,
        std::slice::from_ref(&shared),
        "one",
        &CommitHooks::traced(&trace),
    )?;
    trace.take();
    commit_evidence(
        &store,
        "op_4444444444444444",
        2,
        std::slice::from_ref(&shared),
        "two",
        &CommitHooks::traced(&trace),
    )?;
    assert!(trace.take().contains(&FsOp::Accept(
        DirRole::Artifacts,
        artifact_name(&shared, "png")
    )));
    Ok(())
}

/// Internal child of [`every_fault_point_is_reached_and_a_kill_there_recovers`]:
/// commits an evidence generation over the prepared session, stopping at the
/// point `VSIFT_FAULT_POINT` names.
#[test]
#[ignore = "internal child entry launched by the fault-point kill test"]
fn commit_crash_child() -> TestResult {
    let root = PathBuf::from(std::env::var_os(CHILD_ROOT).ok_or("missing crash root")?);
    let store = FilesystemSessionStore::open_existing(root)?;
    let images = [image("crash one"), image("crash two")];
    commit_evidence(
        &store,
        CRASHED_EVIDENCE,
        2,
        &images,
        "crash",
        &CommitHooks::new(),
    )?;
    Err("the fault point was not reached".into())
}

/// Hashes every artifact the committed head lists against its manifest
/// entry, and walks the whole chain.
pub(super) fn assert_committed_state_is_whole(store: &FilesystemSessionStore) -> TestResult {
    let session = store
        .root
        .open_dir(PathBuf::from(SESSIONS_DIRECTORY).join(SESSION))?;
    let committed = read_committed_manifest(&session, &session_id()?, ChainCheck::Full)?;
    let lifecycle = committed.manifest.lifecycle.ok_or("no lifecycle")?;
    for artifact in &lifecycle.artifacts {
        let bytes = session.read(PathBuf::from(ARTIFACTS_DIRECTORY).join(&artifact.name))?;
        assert_eq!(sha256_hex(&bytes), artifact.sha256, "{}", artifact.name);
    }
    Ok(())
}

/// S-07 and the fault-point registry: a process killed at every fault point
/// of an evidence commit leaves a session that reopens at its last
/// acknowledged generation or the new one, with every listed file whole, and
/// the same operation then completes. Every point in [`FaultPoint::COMMIT`] must
/// be reached, so a point no commit passes fails this test.
#[test]
fn every_fault_point_is_reached_and_a_kill_there_recovers() -> TestResult {
    let mut modes = vec![StoredDurability::Ephemeral];
    // Durable commits synchronise directories for real, which a capability
    // handle supports only on Unix.
    if cfg!(unix) {
        modes.push(StoredDurability::Durable);
    }
    for durability in modes {
        for point in FaultPoint::COMMIT {
            let fixture = Fixture::new()?;
            let store = open_session(&fixture, durability)?;
            let before = store.session_status(&session_id()?)?;
            assert_eq!(before.generation(), StorageGeneration::from_value(2));
            let output = Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "filesystem_session_store::p10_tests::commit_crash_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env(CHILD_ROOT, &fixture.path)
                .env(FAULT_POINT_VARIABLE, point.name())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()?;
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                output.status.code(),
                Some(FAULT_EXIT_CODE),
                "{durability:?} {point}: {stderr}"
            );
            assert!(
                stderr.contains(&format!("{FAULT_MARKER}={point}")),
                "{durability:?} {point}: {stderr}"
            );

            let reopened = FilesystemSessionStore::open_existing(&fixture.path)?;
            let status = reopened.session_status(&session_id()?)?;
            let committed = status.generation() == StorageGeneration::from_value(3);
            assert!(
                committed || status.generation() == before.generation(),
                "{durability:?} {point}: generation {}",
                status.generation()
            );
            let added = if committed { 3 } else { 0 };
            assert_eq!(
                status.artifact_count(),
                before.artifact_count() + added,
                "{durability:?} {point}"
            );
            assert_committed_state_is_whole(&reopened)?;

            let images = [image("crash one"), image("crash two")];
            let retried = commit_evidence(
                &reopened,
                CRASHED_EVIDENCE,
                2,
                &images,
                "crash",
                &CommitHooks::new(),
            )?;
            assert_eq!(
                retried,
                StorageGeneration::from_value(3),
                "{durability:?} {point}"
            );
            let after = reopened.session_status(&session_id()?)?;
            assert_eq!(after.artifact_count(), before.artifact_count() + 3);
            assert_committed_state_is_whole(&reopened)?;
        }
    }
    Ok(())
}

/// ADR 0020 D-2: a session holds 384 evidence artifacts, and its manifest,
/// larger than any other metadata file may be, is written and read back by
/// a new store instance; the next evidence artifact does not fit.
#[test]
fn the_raised_evidence_cap_holds_and_its_manifest_reads_back() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    // Generation 2 holds one image and one record; this call adds the rest
    // and its own record.
    let images: Vec<Vec<u8>> = (0..crate::MAX_EVIDENCE_ARTIFACTS - 3)
        .map(|number| image(&format!("filler {number}")))
        .collect();
    commit_evidence(
        &store,
        CRASHED_EVIDENCE,
        2,
        &images,
        "fill",
        &CommitHooks::new(),
    )?;
    let manifest = session_path(&fixture)
        .join(GENERATIONS_DIRECTORY)
        .join("3.json");
    let size = fs::metadata(manifest)?.len();
    assert!(
        size > super::MAX_METADATA_BYTES && size <= super::MAX_MANIFEST_BYTES,
        "{size}"
    );
    let reopened = FilesystemSessionStore::open_existing(&fixture.path)?;
    let status = reopened.session_status(&session_id()?)?;
    assert_eq!(status.artifact_count(), crate::MAX_EVIDENCE_ARTIFACTS);
    let session = reopened
        .root
        .open_dir(PathBuf::from(SESSIONS_DIRECTORY).join(SESSION))?;
    read_committed_manifest(&session, &session_id()?, ChainCheck::Full)?;
    let one_more = [image("one too many")];
    let media = [EvidenceMediaFile {
        kind: EvidenceMediaKind::FramePng,
        bytes: &one_more[0],
    }];
    assert_eq!(
        reopened
            .commit_evidence(
                &session_id()?,
                &operation("op_8888888888888888")?,
                StorageGeneration::from_value(3),
                &EvidenceFiles {
                    media: &media,
                    record: b"{\"marker\":\"more\"}",
                    verified_identity: None,
                    now_unix_seconds: now()?,
                },
                &CommitHooks::new(),
            )
            .err(),
        Some(SessionStorageError::CapacityExhausted)
    );
    Ok(())
}

/// ADR 0020 D-2: 512 artifacts fit (a 512-entry manifest stays within the
/// manifest bound even with the largest sizes); the 513th does not.
#[test]
fn a_session_holds_at_most_512_artifacts_in_a_bounded_manifest() -> TestResult {
    let lifetime = vsift_domain::SessionLifetime::open(now()?)?;
    let lifecycle = |count: usize| super::StoredLifecycle {
        phase: super::StoredSessionPhase::Open,
        opened_at_unix_seconds: lifetime.opened_at_unix_seconds(),
        expires_at_unix_seconds: lifetime.expires_at_unix_seconds(),
        source_id: format!("src_sha256_{}", sha256_hex(SOURCE_BYTES)),
        source_name: format!("source-{ACTIVATE}.media"),
        source_bytes: u64::try_from(SOURCE_BYTES.len()).unwrap_or(1),
        artifacts: (0..count).map(record_artifact).collect(),
        verified_source_identity: None,
    };
    let add = |count: usize| {
        super::publication::update_lifecycle(
            Some(lifecycle(count)),
            LifecycleUpdate::AddArtifact {
                artifact: record_artifact(count),
                now: lifetime.opened_at_unix_seconds(),
            },
        )
    };
    let full = add(super::MAX_SESSION_ARTIFACTS - 1)?.ok_or("no lifecycle")?;
    assert_eq!(full.artifacts.len(), super::MAX_SESSION_ARTIFACTS);
    assert_eq!(
        add(super::MAX_SESSION_ARTIFACTS).err(),
        Some(SessionStorageError::CapacityExhausted)
    );
    let manifest = super::GenerationManifest {
        schema_version: super::STORAGE_SCHEMA_VERSION,
        session_id: SESSION.to_owned(),
        operation_id: FIRST_EVIDENCE.to_owned(),
        generation: super::MAX_GENERATIONS_PER_SESSION - 1,
        previous_manifest_sha256: Some("f".repeat(64)),
        durability: StoredDurability::Durable,
        lifecycle: Some(full),
    };
    let size = serde_json::to_vec(&manifest)?.len();
    assert!(u64::try_from(size)? <= super::MAX_MANIFEST_BYTES, "{size}");
    Ok(())
}

/// A transcript-record entry with a distinct digest and an eight-digit size
/// (512 of them stay within the session's 10 GiB).
fn record_artifact(number: usize) -> super::StoredArtifact {
    let digest = sha256_hex(number.to_string().as_bytes());
    super::StoredArtifact {
        kind: super::StoredArtifactKind::TranscriptRecord,
        name: format!("artifact-{digest}.json"),
        sha256: digest,
        bytes: 20_000_000,
    }
}

/// L-048 (resolved in P10 PR 2): a publication that ended between its
/// manifest and pointer renames leaves an unreferenced manifest; another
/// operation then publishes that generation over it instead of being
/// refused, in both publication modes.
#[test]
fn another_operation_publishes_over_an_abandoned_manifest() -> TestResult {
    let mut modes = vec![StoredDurability::Ephemeral];
    if cfg!(unix) {
        modes.push(StoredDurability::Durable);
    }
    for durability in modes {
        let fixture = Fixture::new()?;
        let store = open_session(&fixture, durability)?;
        let abandoned = commit_evidence(
            &store,
            CRASHED_EVIDENCE,
            2,
            &[image("abandoned")],
            "abandoned",
            &CommitHooks::failing_at(FaultPoint::ManifestRename),
        );
        assert!(abandoned.is_err(), "{durability:?}");
        assert!(
            session_path(&fixture)
                .join(GENERATIONS_DIRECTORY)
                .join("3.json")
                .exists()
        );
        let published = commit_evidence(
            &store,
            "op_9999999999999999",
            2,
            &[image("replacement")],
            "replacement",
            &CommitHooks::new(),
        )?;
        assert_eq!(
            published,
            StorageGeneration::from_value(3),
            "{durability:?}"
        );
        let reopened = FilesystemSessionStore::open_existing(&fixture.path)?;
        assert_committed_state_is_whole(&reopened)?;
        assert_eq!(
            reopened.session_status(&session_id()?)?.generation(),
            StorageGeneration::from_value(3)
        );
    }
    Ok(())
}

/// Opens the session's commit pointer the way a reader does, without the
/// single-link check, so a test can act between the open and the check.
fn raw_pointer(session: &cap_std::fs::Dir) -> std::io::Result<cap_std::fs::File> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    session.open_with(CURRENT_FILE, &options)
}

/// Replaces the commit pointer with the same bytes, staged and renamed as
/// a correct writer does.
fn reinstall_pointer(session: &cap_std::fs::Dir) -> std::io::Result<()> {
    let bytes = session.read(CURRENT_FILE)?;
    session.write("pointer.test.tmp", bytes)?;
    session.rename("pointer.test.tmp", session, CURRENT_FILE)
}

/// Regression (PR #179 X-04 stress): a reader that opens a metadata file a
/// writer is replacing by rename meets two states a correct writer causes:
/// the file it opened has no link left by the time it reads its metadata
/// (every platform), and the name is briefly absent (Windows). Before the
/// fix the first was reported as a damaged file and both became
/// `INTEGRITY_FAILURE`; now both are retried and the reader gets the
/// committed pointer. The interleavings are driven on the real filesystem
/// in a fixed order.
#[test]
fn a_reader_that_meets_a_rename_retries_instead_of_reporting_damage() -> TestResult {
    let fixture = Fixture::new()?;
    let store = chain_of(&fixture, 2)?;
    let session = store
        .root
        .open_dir(PathBuf::from(SESSIONS_DIRECTORY).join(SESSION))?;
    let mut step = 0;
    let mut met = Vec::new();
    let opened = super::open_replaced_file_with(|| {
        step += 1;
        match step {
            // Opened, then renamed over before its metadata is read.
            1 => {
                let old = raw_pointer(&session)?;
                reinstall_pointer(&session)?;
                let checked = super::checked_regular_file(old);
                met.push(checked.as_ref().err().map(std::io::Error::kind));
                checked
            }
            // The name is absent for a moment during the rename.
            2 => {
                session.rename(CURRENT_FILE, &session, "current.moving")?;
                let absent = super::open_regular_file(&session, CURRENT_FILE, false);
                met.push(absent.as_ref().err().map(std::io::Error::kind));
                session.rename("current.moving", &session, CURRENT_FILE)?;
                absent
            }
            _ => super::open_regular_file(&session, CURRENT_FILE, false),
        }
    })?;
    assert_eq!(
        met,
        vec![
            Some(std::io::ErrorKind::NotFound),
            Some(std::io::ErrorKind::NotFound)
        ]
    );
    assert_eq!(step, 3);
    let pointer: super::CommitPointer =
        super::stored::parse_versioned_json(&super::read_bounded(opened)?)?;
    assert_eq!(pointer.generation, 2);
    // The real read path reads the same committed head.
    let head = read_committed_manifest(&session, &session_id()?, ChainCheck::Full)?;
    assert_eq!(head.manifest.generation, 2);
    Ok(())
}

/// The retry never hides damage: a hard-linked pointer is refused at once,
/// and a pointer that stays missing is still an integrity failure.
#[test]
fn a_linked_or_missing_pointer_is_still_damage() -> TestResult {
    let fixture = Fixture::new()?;
    chain_of(&fixture, 1)?;
    let pointer = session_path(&fixture).join(CURRENT_FILE);
    let linked = session_path(&fixture).join("linked.json");
    fs::hard_link(&pointer, &linked)?;
    let session = FilesystemSessionStore::open_existing(&fixture.path)?
        .root
        .open_dir(PathBuf::from(SESSIONS_DIRECTORY).join(SESSION))?;
    let started = std::time::Instant::now();
    let refused = super::open_replaced_file(&session, CURRENT_FILE, false);
    assert_eq!(
        refused.err().map(|error| error.kind()),
        Some(std::io::ErrorKind::InvalidData)
    );
    assert!(started.elapsed() < super::REPLACED_FILE_RETRY);
    assert_eq!(
        read_error(&fixture)?,
        Some(SessionStorageError::IntegrityFailure)
    );
    fs::remove_file(&linked)?;
    assert_eq!(read_error(&fixture)?, None);
    fs::remove_file(&pointer)?;
    assert_eq!(
        read_error(&fixture)?,
        Some(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

/// Regression (PR #179): readers running through the real read path while
/// another store instance publishes generation after generation (each
/// replacing the pointer and the chain checkpoint by rename) always read a
/// committed head and never report damage.
#[test]
fn readers_never_report_damage_while_generations_are_published() -> TestResult {
    const PUBLICATIONS: u64 = 300;
    let fixture = Fixture::new()?;
    chain_of(&fixture, 1)?;
    let root = fixture.path.clone();
    let writer = std::thread::spawn(move || -> Result<(), String> {
        let store =
            FilesystemSessionStore::open_existing(&root).map_err(|error| error.to_string())?;
        for expected in 1..=PUBLICATIONS {
            publish_keep(&store, &format!("op_{:016x}", expected + 0x9000), expected)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    });
    let session = FilesystemSessionStore::open_existing(&fixture.path)?
        .root
        .open_dir(PathBuf::from(SESSIONS_DIRECTORY).join(SESSION))?;
    let (mut reads, mut failures) = (0_u64, Vec::new());
    let mut last = 0;
    while !writer.is_finished() {
        match read_committed_manifest(&session, &session_id()?, ChainCheck::Incremental(None)) {
            Ok(head) => {
                assert!(head.manifest.generation >= last, "the head went back");
                last = head.manifest.generation;
            }
            Err(error) => failures.push(error),
        }
        reads += 1;
    }
    writer.join().map_err(|_| "writer panicked")??;
    assert!(reads > 0);
    assert_eq!(
        failures,
        Vec::new(),
        "{} of {reads} reads failed",
        failures.len()
    );
    let head = read_committed_manifest(&session, &session_id()?, ChainCheck::Full)?;
    assert_eq!(head.manifest.generation, PUBLICATIONS + 1);
    Ok(())
}
