//! A process killed while it registers a session (issue #197).
//!
//! A registration used to create its index marker in place and then write
//! it. A process killed between the two left an empty marker, and every later
//! scan of that bucket (so every session listing) and the registration's own
//! cleanup failed as an integrity failure, for good. A worker batch met it
//! when one request's process was killed while the other request registered
//! its session. These tests stop a child process at each registration fault
//! point and check what the next process finds.

use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

use vsift_application::SessionStorageError;
use vsift_domain::{OperationId, SessionId, SessionLifetime};

use super::{
    CleanOutcome, FilesystemSessionStore, SESSION_INDEX_DIRECTORY, session_bucket,
    tests::{Fixture, TestResult},
};
use crate::fault_point::{FAULT_EXIT_CODE, FAULT_MARKER, FAULT_POINT_VARIABLE, FaultPoint};

const CHILD_ROOT: &str = "VSIFT_REGISTRATION_CRASH_ROOT";
const KILLED_SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const REGISTERED_AT: u64 = 1_000;

fn opener() -> Result<OperationId, SessionStorageError> {
    OperationId::parse("op_0123456789abcdef").map_err(|_| SessionStorageError::IntegrityFailure)
}

/// The bucket number of a session, as a scan cursor.
fn bucket_of(session: &SessionId) -> Result<u16, std::num::ParseIntError> {
    u16::from_str_radix(&session_bucket(session), 16)
}

/// A session id other than [`KILLED_SESSION`] in the same index bucket.
fn neighbour_of(session: &SessionId) -> Result<SessionId, Box<dyn std::error::Error>> {
    let bucket = session_bucket(session);
    for candidate in 0_u32..100_000 {
        let neighbour = SessionId::parse(format!("ses_{candidate:032x}"))?;
        if session_bucket(&neighbour) == bucket && neighbour != *session {
            return Ok(neighbour);
        }
    }
    Err("no neighbour in the bucket".into())
}

#[test]
#[ignore = "internal child entry launched by the registration kill test"]
fn registration_crash_child() -> TestResult {
    let root = PathBuf::from(std::env::var_os(CHILD_ROOT).ok_or("missing crash root")?);
    let store = FilesystemSessionStore::open_existing(root)?;
    drop(store.register_session(
        &SessionId::parse(KILLED_SESSION)?,
        &opener()?,
        REGISTERED_AT,
    )?);
    Err("the registration fault point was not reached".into())
}

/// A registration killed at each of its fault points leaves an index that
/// scans: before the marker is renamed into its bucket there is no marker
/// (only a staged file the next registration replaces), after it the whole
/// marker, which cleanup removes once it is idle. A neighbour in the same
/// bucket then registers and is listed.
#[test]
fn a_kill_while_registering_leaves_the_index_readable() -> TestResult {
    let killed = SessionId::parse(KILLED_SESSION)?;
    let neighbour = neighbour_of(&killed)?;
    for point in FaultPoint::REGISTRATION {
        let fixture = Fixture::new()?;
        let output = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "filesystem_session_store::index_tests::registration_crash_child",
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
            "{point}: {stderr}"
        );
        assert!(stderr.contains(FAULT_MARKER), "{point}: {stderr}");

        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        let bucket = bucket_of(&killed)?;
        let published = point == FaultPoint::RegistrationMarkerRename;
        let listed = store
            .scan_session_bucket(bucket)
            .map_err(|error| format!("{point}: scan after the kill: {error:?}"))?;
        let expected = if published {
            vec![killed.clone()]
        } else {
            Vec::new()
        };
        assert_eq!(listed.session_ids(), expected.as_slice(), "{point}");

        drop(store.register_session(&neighbour, &opener()?, REGISTERED_AT)?);
        let mut expected = expected;
        expected.push(neighbour.clone());
        expected.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        let listed = store
            .scan_session_bucket(bucket)
            .map_err(|error| format!("{point}: scan after a new registration: {error:?}"))?;
        assert_eq!(listed.session_ids(), expected.as_slice(), "{point}");
        let index = fixture.path.join(SESSION_INDEX_DIRECTORY);
        assert_eq!(
            std::fs::read_dir(&index)?.count(),
            1,
            "{point}: only the bucket remains in the index"
        );

        if published {
            let idle = REGISTERED_AT + SessionLifetime::IDLE_SECONDS;
            assert_eq!(
                store.clean_session(&killed, idle, false)?,
                CleanOutcome::Removed,
                "{point}"
            );
            let listed = store.scan_session_bucket(bucket)?;
            assert_eq!(
                listed.session_ids(),
                [neighbour.clone()].as_slice(),
                "{point}"
            );
        }
    }
    Ok(())
}
