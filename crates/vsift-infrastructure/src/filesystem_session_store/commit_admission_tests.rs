//! Fable review C-2 and C-3 (ADR 0033, note of 2026-11-11): a run's part
//! commits are admitted under the run's own permit, and a commit waits a
//! bounded moment for another commit of its session to let the writer lock go.

use std::cell::{Cell, RefCell};

use cap_fs_ext::DirExt;

use vsift_application::{CommitAdmission, SessionStorageError};
use vsift_domain::{EvidenceMediaKind, SessionArtifactKind, StorageGeneration};

use super::{
    COORDINATION_DIRECTORY, EvidenceMediaFile, FilesystemSessionStore, StoredDurability,
    commit::CommitHooks,
    evidence::{ArtifactCommit, EvidenceFiles},
    open_session_lock,
    p10_tests::{Built, now, open_session, operation, session_id},
    root::acquire_admission,
    tests::{Fixture, TestResult},
};
use crate::file_lock::HeldFileLock;

/// The generation [`open_session`] leaves the session at.
const OPENED: u64 = 2;

/// Commits one stand-in artifact at `expected` as `admission` says, under
/// `hooks`.
fn commit_artifact(
    store: &FilesystemSessionStore,
    operation_text: &str,
    expected: u64,
    admission: CommitAdmission<'_, super::FilesystemAdmissionPermit>,
    hooks: &CommitHooks<'_>,
) -> Result<StorageGeneration, SessionStorageError> {
    let Ok(session) = session_id() else {
        return Err(SessionStorageError::Io);
    };
    let Ok(operation) = operation(operation_text) else {
        return Err(SessionStorageError::Io);
    };
    let Ok(now) = now() else {
        return Err(SessionStorageError::Io);
    };
    store.publish_artifact_hooked(
        &session,
        &operation,
        StorageGeneration::from_value(expected),
        SessionArtifactKind::VisualIndexRecord,
        format!("{{\"operation\":\"{operation_text}\"}}").as_bytes(),
        now,
        ArtifactCommit::Add,
        admission,
        hooks,
    )
}

/// Takes the session's writer lock as another commit would hold it.
fn hold_writer(store: &FilesystemSessionStore) -> Built<HeldFileLock> {
    let coordination = store.root.open_dir_nofollow(COORDINATION_DIRECTORY)?;
    Ok(
        HeldFileLock::try_exclusive(open_session_lock(&coordination, &session_id()?, "writer")?)
            .map_err(|_| "the writer lock is held")?,
    )
}

/// C-3: a run that holds the root's whole capacity commits its part under its
/// own permit, which takes no unit; the same commit asking for a unit of its
/// own is refused by the run's reservation, as every part was before the fix.
#[test]
fn a_commit_under_the_runs_own_permit_takes_no_unit() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let run = store.try_admit_without_waiting(4)?;
    assert_eq!(
        commit_artifact(
            &store,
            "op_1111111111111111",
            OPENED,
            CommitAdmission::Ordinary,
            &CommitHooks::new(),
        ),
        Err(SessionStorageError::Busy)
    );
    let committed = commit_artifact(
        &store,
        "op_1111111111111111",
        OPENED,
        CommitAdmission::UnderPermit(&run),
        &CommitHooks::new(),
    )?;
    assert_eq!(committed.value(), OPENED + 1);
    Ok(())
}

/// The permit covers a commit only on the root it was taken on: a permit of
/// another root, or a commit's own unit (whose root is not recorded), falls
/// back to an ordinary admission and is refused while the root is full,
/// never let past it.
#[test]
fn a_permit_of_another_root_or_of_no_known_root_covers_nothing() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let other_fixture = Fixture::new()?;
    let other = FilesystemSessionStore::open_existing(&other_fixture.path)?;
    let elsewhere = other.try_admit_without_waiting(1)?;
    let run = store.try_admit_without_waiting(3)?;
    let own_unit = acquire_admission(&store.root, store.admission_capacity, 1)?;
    for admission in [
        CommitAdmission::UnderPermit(&elsewhere),
        CommitAdmission::UnderPermit(&own_unit),
    ] {
        assert_eq!(
            commit_artifact(
                &store,
                "op_1111111111111111",
                OPENED,
                admission,
                &CommitHooks::new(),
            ),
            Err(SessionStorageError::Busy)
        );
    }
    drop((run, own_unit));
    Ok(())
}

/// C-2: while a watched run holds three of the four units and its part
/// commit is under way (here, waiting for the writer lock another commit of
/// the session holds), the fourth unit is free for a frame; and the part
/// commit, once the writer lock is let go, commits instead of failing.
#[test]
fn a_part_commit_leaves_the_free_unit_and_waits_for_the_writer_lock() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let run = store.try_admit_without_waiting(3)?;
    let writer = RefCell::new(Some(hold_writer(&store)?));
    let frame_admitted = Cell::new(None);
    let busy = || {
        if writer.borrow().is_some() {
            frame_admitted.set(Some(store.try_admit(1).is_ok()));
            writer.borrow_mut().take();
        }
    };
    let committed = commit_artifact(
        &store,
        "op_1111111111111111",
        OPENED,
        CommitAdmission::UnderPermit(&run),
        &CommitHooks::new().on_writer_busy(&busy),
    )?;
    assert_eq!(committed.value(), OPENED + 1);
    assert_eq!(frame_admitted.get(), Some(true));
    Ok(())
}

/// C-2, the frame's side: an evidence commit that meets the writer lock held
/// by another commit of its session waits for it and commits, where it was
/// refused `BUSY` at once before.
#[test]
fn an_evidence_commit_waits_for_the_writer_lock_and_commits() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let writer = RefCell::new(Some(hold_writer(&store)?));
    let looks = Cell::new(0_u32);
    let busy = || {
        looks.set(looks.get() + 1);
        writer.borrow_mut().take();
    };
    let image = b"\x89PNG\r\n\x1a\nwaited".to_vec();
    let media = [EvidenceMediaFile {
        kind: EvidenceMediaKind::FramePng,
        bytes: &image,
    }];
    let committed = store.commit_evidence(
        &session_id()?,
        &operation("op_1111111111111111")?,
        StorageGeneration::from_value(OPENED),
        &EvidenceFiles {
            media: &media,
            record: b"{\"marker\":\"waited\"}",
            verified_identity: None,
            now_unix_seconds: now()?,
        },
        &CommitHooks::new().on_writer_busy(&busy),
    )?;
    assert_eq!(committed.value(), OPENED + 1);
    assert_eq!(looks.get(), 1);
    Ok(())
}

/// The wait is bounded: a writer lock that is not let go within 2 s leaves
/// the commit refused `BUSY`, as before, with nothing committed.
#[test]
fn a_writer_lock_held_past_the_wait_still_answers_busy() -> TestResult {
    let fixture = Fixture::new()?;
    let store = open_session(&fixture, StoredDurability::Ephemeral)?;
    let _writer = hold_writer(&store)?;
    assert_eq!(
        commit_artifact(
            &store,
            "op_1111111111111111",
            OPENED,
            CommitAdmission::Ordinary,
            &CommitHooks::new(),
        ),
        Err(SessionStorageError::Busy)
    );
    assert_eq!(
        store.session_status(&session_id()?)?.generation().value(),
        OPENED
    );
    Ok(())
}
