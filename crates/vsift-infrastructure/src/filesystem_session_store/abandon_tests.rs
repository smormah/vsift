//! A failed open removes the registration it made, at once (#277).
//!
//! `abandon_unpublished_open` is `session clean` under a narrower rule, so
//! these tests pin the rule: it removes only a registration that names the
//! operation that made it and only a session that was never published, and it
//! never touches anything else.

use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use cap_std::fs::Dir;
use vsift_application::SessionStorageError;
use vsift_domain::{OperationId, SessionId};

use super::{
    CleanOutcome, FilesystemSessionStore, SESSION_INDEX_DIRECTORY, SESSIONS_DIRECTORY,
    index::read_marker_if_present,
    p10_tests, session_bucket,
    tests::{Fixture, TestResult},
};

/// The operation `initialize_as` registers its session under.
const OWN_OPERATION: &str = "op_0123456789abcdef";
const FOREIGN_OPERATION: &str = "op_fedcba9876543210";

fn own() -> Result<OperationId, Box<dyn std::error::Error>> {
    p10_tests::operation(OWN_OPERATION)
}

fn registered(
    fixture: &Fixture,
) -> Result<(FilesystemSessionStore, SessionId), Box<dyn std::error::Error>> {
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let session = p10_tests::session_id()?;
    drop(store.register_session(&session, &own()?, p10_tests::now()?)?);
    Ok((store, session))
}

fn registrations(
    store: &FilesystemSessionStore,
    session: &SessionId,
) -> Result<usize, Box<dyn std::error::Error>> {
    let bucket = u16::from_str_radix(&session_bucket(session), 16)?;
    Ok(store.scan_session_bucket(bucket)?.session_ids().len())
}

fn folder_exists(fixture: &Fixture, session: &SessionId) -> bool {
    fixture
        .path
        .join(SESSIONS_DIRECTORY)
        .join(session.as_str())
        .exists()
}

/// A registration with no session folder (a failure before the folder was
/// made) is removed at once by the operation that made it.
#[test]
fn a_registration_alone_is_removed_at_once_by_its_own_operation() -> TestResult {
    let fixture = Fixture::new()?;
    let (store, session) = registered(&fixture)?;
    assert_eq!(registrations(&store, &session)?, 1);

    assert_eq!(
        store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?)?,
        CleanOutcome::Removed
    );
    assert_eq!(registrations(&store, &session)?, 0);
    Ok(())
}

/// An initialized session that was never published (a failed copy, a refused
/// source) is removed with its registration and its folder.
#[test]
fn an_initialized_open_that_never_published_is_removed_with_its_folder() -> TestResult {
    let fixture = Fixture::new()?;
    let (store, session) = registered(&fixture)?;
    p10_tests::initialize_as(
        &store,
        super::StoredDurability::Ephemeral,
        &super::commit::CommitHooks::new(),
    )?;
    assert!(folder_exists(&fixture, &session));

    assert_eq!(
        store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?)?,
        CleanOutcome::Removed
    );
    assert!(!folder_exists(&fixture, &session));
    assert_eq!(registrations(&store, &session)?, 0);
    Ok(())
}

/// A registration another operation made is not this open's to remove: the
/// marker names the operation, and a different one is ineligible, with
/// everything left exactly as it was.
#[test]
fn another_operations_registration_is_never_removed() -> TestResult {
    let fixture = Fixture::new()?;
    let (store, session) = registered(&fixture)?;
    p10_tests::initialize_as(
        &store,
        super::StoredDurability::Ephemeral,
        &super::commit::CommitHooks::new(),
    )?;
    let foreign = p10_tests::operation(FOREIGN_OPERATION)?;

    assert_eq!(
        store.abandon_unpublished_open(&session, &foreign, p10_tests::now()?)?,
        CleanOutcome::Ineligible
    );
    assert!(folder_exists(&fixture, &session));
    assert_eq!(registrations(&store, &session)?, 1);
    Ok(())
}

/// A published session is never an abandoned open, whatever operation asks.
#[test]
fn a_published_session_is_never_removed() -> TestResult {
    let fixture = Fixture::new()?;
    let (store, session) = registered(&fixture)?;
    let hooks = super::commit::CommitHooks::new();
    p10_tests::initialize_as(&store, super::StoredDurability::Ephemeral, &hooks)?;
    p10_tests::activate(&fixture, &store, &hooks)?;

    assert_eq!(
        store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?)?,
        CleanOutcome::Ineligible
    );
    assert!(folder_exists(&fixture, &session));
    assert_eq!(registrations(&store, &session)?, 1);
    assert!(
        store.acquire_read(&session).is_ok(),
        "the published session still reads"
    );
    Ok(())
}

/// A session id that was never registered is not removed, and nothing is made
/// for it.
#[test]
fn an_unregistered_session_is_ineligible_and_nothing_is_made() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let session = p10_tests::session_id()?;

    assert_eq!(
        store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?)?,
        CleanOutcome::Ineligible
    );
    assert!(!folder_exists(&fixture, &session));
    let entries = fs::read_dir(fixture.path.join(SESSIONS_DIRECTORY))?.count();
    assert_eq!(entries, 0);
    Ok(())
}

/// A removal that meets a live opener (it still holds its marker) is `Busy` and
/// removes nothing; once the opener lets go, the same call removes it. This is
/// what the engine's bounded retry relies on.
#[test]
fn a_registration_a_live_opener_still_holds_is_busy_and_then_removable() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let session = p10_tests::session_id()?;
    let held = store.register_session(&session, &own()?, p10_tests::now()?)?;

    assert_eq!(
        store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?),
        Err(SessionStorageError::Busy)
    );
    assert_eq!(registrations(&store, &session)?, 1);

    drop(held);
    assert_eq!(
        store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?)?,
        CleanOutcome::Removed
    );
    assert_eq!(registrations(&store, &session)?, 0);
    Ok(())
}

/// The engine asks before it tries a removal, so a failure that came before the
/// registration was made does not retry a removal of nothing.
#[test]
fn is_registered_tells_a_marker_from_none_without_taking_a_lock() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let session = p10_tests::session_id()?;
    assert_eq!(store.is_registered(&session), Ok(false));

    let held = store.register_session(&session, &own()?, p10_tests::now()?)?;
    // Held by a live opener, and still readable: no lock is taken.
    assert_eq!(store.is_registered(&session), Ok(true));
    drop(held);

    store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?)?;
    assert_eq!(store.is_registered(&session), Ok(false));
    Ok(())
}

fn bucket_of(fixture: &Fixture, session: &SessionId) -> Result<Dir, Box<dyn std::error::Error>> {
    let root = Dir::open_ambient_dir(&fixture.path, cap_std::ambient_authority())?;
    let index = root.open_dir(SESSION_INDEX_DIRECTORY)?;
    Ok(index.open_dir(session_bucket(session))?)
}

/// The scan's reader: a marker that is gone is `None` (it was removed after the
/// scan listed it, which a removal does without the scan's lock), a marker
/// that is present reads, and one that is damaged is still an integrity
/// failure.
#[test]
fn a_marker_that_vanished_is_not_an_error_and_a_damaged_one_still_is() -> TestResult {
    let fixture = Fixture::new()?;
    let (store, session) = registered(&fixture)?;
    let bucket = bucket_of(&fixture, &session)?;

    let present = read_marker_if_present(&bucket, session.as_str())?;
    assert_eq!(
        present.map(|marker| marker.operation_id),
        Some(OWN_OPERATION.to_owned())
    );

    store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?)?;
    assert!(read_marker_if_present(&bucket, session.as_str())?.is_none());

    let damaged = SessionId::parse("ses_00000000000000000000000000000abc")?;
    let damaged_bucket = fixture
        .path
        .join(SESSION_INDEX_DIRECTORY)
        .join(session_bucket(&damaged));
    fs::create_dir_all(&damaged_bucket)?;
    fs::write(damaged_bucket.join(damaged.as_str()), b"not json")?;
    let bucket = bucket_of(&fixture, &damaged)?;
    assert_eq!(
        read_marker_if_present(&bucket, damaged.as_str()).map(|_| ()),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

/// On Windows a lock is mandatory: a marker a remover holds exclusively (as
/// `session clean` and a failed open's removal do while they examine and remove
/// it) cannot be read, and a scan used to fail with an integrity failure over
/// it. The marker is not listed that time. On Unix the lock is advisory, so
/// the marker still reads.
#[test]
fn a_marker_a_remover_holds_exclusively_is_skipped_on_windows_and_read_on_unix() -> TestResult {
    let fixture = Fixture::new()?;
    let (_store, session) = registered(&fixture)?;
    let bucket = bucket_of(&fixture, &session)?;
    let remover = crate::file_lock::HeldFileLock::try_exclusive(
        super::open_regular_file(&bucket, session.as_str(), true)?.into_std(),
    )
    .map_err(|_| "the marker could not be locked")?;

    let read = read_marker_if_present(&bucket, session.as_str())?;

    assert_eq!(read.is_some(), cfg!(unix));
    drop(remover);
    assert!(read_marker_if_present(&bucket, session.as_str())?.is_some());
    Ok(())
}

/// A scan never fails because registrations are removed around it. Removing a
/// failed open's registration is routine since #277, and it is done without the
/// initialization lock a scan holds, so a marker the scan listed can be gone,
/// locked or being deleted when it reads it. **Every session of this test is in
/// one bucket and the scanner scans only that bucket, in a tight loop**, so a
/// scan is always in flight over the marker that is being registered or removed
/// (a scan of all 256 buckets rarely overlaps one). On Windows this found, in
/// turn, a vanished marker, a locked one (os error 33) and one deleted while a
/// handle was open (os error 5), each of which read as an integrity failure.
#[test]
fn a_scan_never_fails_while_registrations_in_its_bucket_are_removed() -> TestResult {
    const ROUNDS: usize = 150;
    let fixture = Fixture::new()?;
    let scanning = FilesystemSessionStore::open_existing(&fixture.path)?;
    let working = FilesystemSessionStore::open_existing(&fixture.path)?;
    let target = session_bucket(&SessionId::parse(format!("ses_{:032x}", 1))?);
    let mut sessions = Vec::new();
    let mut candidate = 1_u128;
    while sessions.len() < ROUNDS {
        let id = SessionId::parse(format!("ses_{candidate:032x}"))?;
        if session_bucket(&id) == target {
            sessions.push(id);
        }
        candidate += 1;
    }
    let bucket = u16::from_str_radix(&target, 16)?;
    let stop = Arc::new(AtomicBool::new(false));
    let scanner = {
        let stop = Arc::clone(&stop);
        thread::spawn(move || -> Result<u32, SessionStorageError> {
            let mut scans = 0_u32;
            while !stop.load(Ordering::Acquire) {
                match scanning.scan_session_bucket(bucket) {
                    Ok(_) => scans += 1,
                    Err(SessionStorageError::Busy) => thread::yield_now(),
                    Err(error) => return Err(error),
                }
            }
            Ok(scans)
        })
    };

    let mut failure: Option<String> = None;
    for (round, session) in sessions.iter().enumerate() {
        let operation = own()?;
        let now = p10_tests::now()?;
        let registration = loop {
            match working.register_session(session, &operation, now) {
                Err(SessionStorageError::Busy) => thread::yield_now(),
                other => break other,
            }
        };
        let removed = match registration {
            Ok(held) => {
                drop(held);
                loop {
                    match working.abandon_unpublished_open(session, &operation, now) {
                        Err(SessionStorageError::Busy | SessionStorageError::AccessDenied) => {
                            thread::yield_now();
                        }
                        other => break other,
                    }
                }
            }
            Err(error) => Err(error),
        };
        if removed != Ok(CleanOutcome::Removed) {
            failure = Some(format!("round {round}: {removed:?}"));
            break;
        }
    }
    stop.store(true, Ordering::Release);
    let scans = scanner
        .join()
        .map_err(|_| "the scanning thread panicked")?
        .map_err(|error| format!("a scan failed while registrations were removed: {error:?}"))?;

    assert_eq!(failure, None);
    assert!(scans > 0, "the scanner never completed a scan");
    Ok(())
}

/// A removal that was cut short (a process killed, or on Windows a scanner or a
/// child holding a file open) leaves the session folder renamed into quarantine
/// and may already have deleted its manifest pointer. Reading that manifest used
/// to fail every later removal with an integrity failure, so such a registration
/// stayed `initializing` for ever. The next removal now finishes it, whichever
/// way it is asked.
fn quarantine_without_its_manifest(
    fixture: &Fixture,
    session: &SessionId,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let sessions = fixture.path.join(SESSIONS_DIRECTORY);
    let quarantine = sessions.join(format!(".quarantine-{}", session.as_str()));
    fs::rename(sessions.join(session.as_str()), &quarantine)?;
    fs::remove_file(quarantine.join(super::CURRENT_FILE))?;
    Ok(quarantine)
}

#[test]
fn a_removal_cut_short_after_it_deleted_the_manifest_is_finished_by_the_next_abandon() -> TestResult
{
    let fixture = Fixture::new()?;
    let (store, session) = registered(&fixture)?;
    p10_tests::initialize_as(
        &store,
        super::StoredDurability::Ephemeral,
        &super::commit::CommitHooks::new(),
    )?;
    let quarantine = quarantine_without_its_manifest(&fixture, &session)?;

    assert_eq!(
        store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?)?,
        CleanOutcome::Removed
    );

    assert!(!quarantine.exists());
    assert_eq!(registrations(&store, &session)?, 0);
    Ok(())
}

#[test]
fn a_removal_cut_short_after_it_deleted_the_manifest_is_finished_by_session_clean() -> TestResult {
    let fixture = Fixture::new()?;
    let (store, session) = registered(&fixture)?;
    p10_tests::initialize_as(
        &store,
        super::StoredDurability::Ephemeral,
        &super::commit::CommitHooks::new(),
    )?;
    let quarantine = quarantine_without_its_manifest(&fixture, &session)?;

    // Long before the idle interval: a quarantine is a removal already decided.
    assert_eq!(
        store.clean_session(&session, p10_tests::now()?, false)?,
        CleanOutcome::Removed
    );

    assert!(!quarantine.exists());
    assert_eq!(registrations(&store, &session)?, 0);
    Ok(())
}

/// A registration that was removed after `session clean` listed it is gone, not
/// damage: the clean used to answer an integrity failure for it, which surfaced
/// as a skipped item and a partial page.
#[test]
fn a_registration_removed_after_it_was_listed_is_gone_not_damage() -> TestResult {
    let fixture = Fixture::new()?;
    let (store, session) = registered(&fixture)?;
    store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?)?;

    assert_eq!(
        store.clean_session(&session, p10_tests::now()?, false)?,
        CleanOutcome::Gone
    );
    assert_eq!(
        store.clean_session(&session, p10_tests::now()?, true)?,
        CleanOutcome::Gone
    );
    Ok(())
}

/// A marker another remover holds (exclusively locked, as a removal holds it
/// while it examines and removes it) is busy to a second remover, on every
/// platform: it is tried again, and then finds the marker gone. It used to read
/// as an integrity failure on Windows, where a lock stops a read too.
#[test]
fn a_marker_another_remover_holds_is_busy_to_a_second_remover() -> TestResult {
    let fixture = Fixture::new()?;
    let (store, session) = registered(&fixture)?;
    let bucket = bucket_of(&fixture, &session)?;
    let remover = crate::file_lock::HeldFileLock::try_exclusive(
        super::open_regular_file(&bucket, session.as_str(), true)?.into_std(),
    )
    .map_err(|_| "the marker could not be locked")?;

    assert_eq!(
        store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?),
        Err(SessionStorageError::Busy)
    );
    assert_eq!(
        store.clean_session(&session, p10_tests::now()?, true),
        Err(SessionStorageError::Busy)
    );

    drop(remover);
    assert_eq!(
        store.abandon_unpublished_open(&session, &own()?, p10_tests::now()?)?,
        CleanOutcome::Removed
    );
    Ok(())
}
