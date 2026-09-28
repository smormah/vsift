//! Worker workspace markers (P11, ADR 0021 D1, D2): the policy is recorded
//! once, read back whole, and never adopted when it changes underneath.

use std::{fs, num::NonZeroU16, path::PathBuf};

use vsift_application::SessionStorageError;
use vsift_domain::{
    DurabilityRequirement, SessionLifetimePolicy, WorkspacePolicy, WorkspaceRetention,
};

use super::{
    FREE_SPACE_RESERVE_BYTES, FilesystemSessionStore, FreeSpaceCheck, OWNERSHIP_FILE,
    SessionStoreOpenError,
    tests::{Fixture, TestResult},
};
use crate::durable_profile::directory_offers_os_crash_durability;

fn policy(
    durability: DurabilityRequirement,
    capacity: u16,
    hours: u64,
) -> Result<WorkspacePolicy, Box<dyn std::error::Error>> {
    Ok(WorkspacePolicy::new(
        durability,
        NonZeroU16::new(capacity).ok_or("zero")?,
        WorkspaceRetention::from_hours(hours)?,
    )?)
}

/// A fresh, not yet existing root path inside a fixture's directory.
fn new_root(fixture: &Fixture) -> PathBuf {
    fixture.path.join("workspace")
}

#[test]
fn a_workspace_records_its_policy_and_reopens_with_it() -> TestResult {
    let fixture = Fixture::new()?;
    let root = new_root(&fixture);
    let expected = policy(DurabilityRequirement::Ephemeral, 3, 2)?;
    let created = FilesystemSessionStore::provision_workspace(&root, expected)?;
    assert_eq!(created.workspace_policy(), Some(expected));
    assert_eq!(created.admission_capacity(), 3);
    assert_eq!(
        created.lifetime_policy(),
        SessionLifetimePolicy::Workspace(expected.retention())
    );
    let marker = fs::read_to_string(root.join(OWNERSHIP_FILE))?;
    assert!(
        marker.contains(
            r#""workspace":{"profile":"durable_workspace","durability":"ephemeral","session_retention_seconds":7200}"#
        ),
        "{marker}"
    );

    let reopened = FilesystemSessionStore::open_existing(&root)?;
    assert_eq!(reopened.workspace_policy(), Some(expected));
    assert_eq!(reopened.admission_capacity(), 3);
    // An ordinary desktop root has no policy.
    let desktop = FilesystemSessionStore::open_existing(&fixture.path)?;
    assert_eq!(desktop.workspace_policy(), None);
    assert_eq!(desktop.lifetime_policy(), SessionLifetimePolicy::Desktop);
    Ok(())
}

/// The policy is immutable: a marker changed underneath an open store is
/// damage, never a new policy, and an out-of-range one is refused whole.
#[test]
fn a_changed_workspace_policy_is_never_adopted() -> TestResult {
    let fixture = Fixture::new()?;
    let root = new_root(&fixture);
    let store = FilesystemSessionStore::provision_workspace(
        &root,
        policy(DurabilityRequirement::Ephemeral, 2, 2)?,
    )?;
    let marker_path = root.join(OWNERSHIP_FILE);
    let marker = fs::read_to_string(&marker_path)?;
    let raised = marker.replace(
        r#""session_retention_seconds":7200"#,
        r#""session_retention_seconds":10800"#,
    );
    assert_ne!(raised, marker);
    fs::remove_file(&marker_path)?;
    fs::write(&marker_path, &raised)?;
    assert!(matches!(
        store.try_admit(1),
        Err(SessionStorageError::IntegrityFailure)
    ));

    for (from, to) in [
        (
            r#""session_retention_seconds":10800"#,
            r#""session_retention_seconds":3599"#,
        ),
        (
            r#""session_retention_seconds":10800"#,
            r#""session_retention_seconds":2592001"#,
        ),
        (r#""profile":"durable_workspace""#, r#""profile":"other""#),
        (r#""durability":"ephemeral""#, r#""durability":"sometimes""#),
        ("}}", r#","extra":1}}"#),
    ] {
        let damaged = raised.replace(from, to);
        assert_ne!(damaged, raised, "{from}");
        fs::remove_file(&marker_path)?;
        fs::write(&marker_path, &damaged)?;
        assert_eq!(
            FilesystemSessionStore::open_existing(&root).err(),
            Some(SessionStoreOpenError::InvalidOwnership),
            "{damaged}"
        );
    }
    Ok(())
}

/// A durable workspace exists only where OS-crash durability is qualified;
/// anywhere else nothing is created.
#[test]
fn a_durable_workspace_fails_closed_off_the_qualified_profile() -> TestResult {
    let fixture = Fixture::new()?;
    let root = new_root(&fixture);
    let durable = policy(DurabilityRequirement::Durable, 4, 168)?;
    match FilesystemSessionStore::provision_workspace(&root, durable) {
        Ok(store) => {
            assert!(directory_offers_os_crash_durability(&fixture.path));
            assert!(store.offers_os_crash_durability());
            assert_eq!(store.workspace_policy(), Some(durable));
        }
        Err(SessionStoreOpenError::DurabilityUnavailable) => {
            assert!(!directory_offers_os_crash_durability(&fixture.path));
            assert!(!root.exists(), "nothing may be created");
        }
        Err(other) => return Err(format!("unexpected failure: {other}").into()),
    }
    Ok(())
}

/// The free-space reserve (P11 PR 2): on Unix a copy that would leave less
/// than the reserve free is refused as a capacity error before anything is
/// written; Windows reports that it did not check.
#[test]
fn the_free_space_reserve_is_checked_on_unix_only() -> TestResult {
    let fixture = Fixture::new()?;
    let store = FilesystemSessionStore::open_existing(&fixture.path)?;
    let impossible = u64::MAX - FREE_SPACE_RESERVE_BYTES;
    if cfg!(unix) {
        assert_eq!(store.ensure_free_space(0), Ok(FreeSpaceCheck::Enforced));
        assert_eq!(
            store.ensure_free_space(impossible),
            Err(SessionStorageError::CapacityExhausted)
        );
    } else {
        for incoming in [0, impossible] {
            assert_eq!(
                store.ensure_free_space(incoming),
                Ok(FreeSpaceCheck::NotEnforced)
            );
        }
    }
    Ok(())
}
