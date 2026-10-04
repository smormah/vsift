//! A failed open removes the registration it made, at once (#277).
//!
//! `abandon_unpublished_open` is `session clean` under a narrower rule, so
//! these tests pin the rule: it removes only a registration that names the
//! operation that made it and only a session that was never published, and it
//! never touches anything else.

use std::fs;

use vsift_domain::{OperationId, SessionId};

use super::{
    CleanOutcome, FilesystemSessionStore, SESSIONS_DIRECTORY, p10_tests, session_bucket,
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
