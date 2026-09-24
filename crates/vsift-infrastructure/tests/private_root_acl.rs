//! Every private root `VSift` creates stays private under a parent whose DACL
//! passes entries for other principals to new children.
//!
//! This reproduces the maintainer's Windows 11 profile, where
//! `%LOCALAPPDATA%` passes a sandbox group and two `AppContainer` capability
//! SIDs to every new child, so `setup configure` failed with `STORAGE_IO`. The
//! fixture parent grants `BUILTIN\Users` inheritable read access with the
//! system `icacls.exe`; results are read back from `icacls /save` as SDDL,
//! which is independent of the display language and of the code under test.

#![cfg(windows)]

use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use vsift_infrastructure::{
    ManagedArtifactStore, SessionRootError, SessionRootProvisioning, SessionStoreOpenError,
    UserDependencyConfigError, UserDependencyConfigStore, open_session_root,
};

type TestResult = Result<(), Box<dyn Error>>;

const PREFIX: &str = "vsift-private-root-acl-";

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

/// A temporary parent that passes `BUILTIN\Users` read access to new children.
struct HostileParent(PathBuf);

impl HostileParent {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            env::temp_dir().join(format!("{PREFIX}{}-{stamp}-{sequence}", std::process::id()));
        fs::create_dir(&path)?;
        let parent = Self(path);
        icacls(&[
            parent.0.as_os_str(),
            "/grant".as_ref(),
            "*S-1-5-32-545:(OI)(CI)(RX)".as_ref(),
            "/Q".as_ref(),
        ])?;
        Ok(parent)
    }

    fn join(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
}

impl Drop for HostileParent {
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

/// Runs the system `icacls.exe` with explicit arguments and no shell.
fn icacls(arguments: &[&std::ffi::OsStr]) -> TestResult {
    let system_root = env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
    let status = Command::new(PathBuf::from(system_root).join("System32/icacls.exe"))
        .args(arguments)
        .output()?
        .status;
    if !status.success() {
        return Err("icacls failed".into());
    }
    Ok(())
}

/// The directory's DACL in SDDL form, for example `D:PAI(A;OICI;FA;;;SY)...`.
fn dacl_sddl(directory: &Path, scratch: &HostileParent) -> Result<String, Box<dyn Error>> {
    let saved = scratch.join("saved-acl.txt");
    icacls(&[
        directory.as_os_str(),
        "/save".as_ref(),
        saved.as_os_str(),
        "/Q".as_ref(),
    ])?;
    let bytes = fs::read(&saved)?;
    fs::remove_file(&saved)?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let text = String::from_utf16(&units)?;
    let sddl = text
        .lines()
        .find(|line| line.starts_with("D:"))
        .ok_or("icacls saved no DACL")?;
    Ok(sddl.to_owned())
}

/// Protected (`D:P`), with no inherited entry and no entry for `BUILTIN\Users`.
fn assert_private_and_protected(directory: &Path, scratch: &HostileParent) -> TestResult {
    let sddl = dacl_sddl(directory, scratch)?;
    assert!(sddl.starts_with("D:P"), "the DACL is not protected");
    assert!(!sddl.contains("ID;"), "an inherited entry remained");
    assert!(!sddl.contains(";BU)"), "BUILTIN\\Users kept access");
    Ok(())
}

/// Not protected and still granting `BUILTIN\Users` access, as inherited.
fn assert_inherits_the_broad_entry(directory: &Path, scratch: &HostileParent) -> TestResult {
    let sddl = dacl_sddl(directory, scratch)?;
    assert!(
        !sddl.starts_with("D:P"),
        "the fixture directory was protected"
    );
    assert!(
        sddl.contains(";BU)"),
        "the fixture did not inherit the entry"
    );
    Ok(())
}

#[test]
fn the_fixture_parent_passes_the_broad_entry_to_a_plain_child() -> TestResult {
    let parent = HostileParent::new()?;
    let plain = parent.join("plain");
    fs::create_dir(&plain)?;
    assert_inherits_the_broad_entry(&plain, &parent)
}

#[test]
fn dependency_configuration_and_verification_state_are_created_private() -> TestResult {
    let parent = HostileParent::new()?;
    let root = parent.join("vsift");
    let model = parent.join("model.bin");
    fs::write(&model, b"fixture only")?;
    let config = UserDependencyConfigStore::at(root.clone())?;
    config.configure_model(&model)?;
    assert_private_and_protected(&root, &parent)?;
    let state = config.media_tool_verification_state()?;
    assert!(state.is_recording(), "the verification state was refused");
    let state_sddl = dacl_sddl(&root.join("media-tool-verification"), &parent)?;
    assert!(
        !state_sddl.contains(";BU)"),
        "BUILTIN\\Users reached the state"
    );
    assert!(config.read_model()?.is_some());
    Ok(())
}

#[test]
fn managed_root_and_its_created_parent_are_private_and_shared_with_configuration() -> TestResult {
    let parent = HostileParent::new()?;
    let shared = parent.join("vsift");
    let root = shared.join("managed-v1");
    let guard = ManagedArtifactStore::at(root.clone())?.try_install_guard()?;
    drop(guard);
    assert_private_and_protected(&shared, &parent)?;
    assert_private_and_protected(&root, &parent)?;
    // On Windows and macOS the configuration root is the managed root's
    // parent; created by the managed store, it must still pass validation.
    assert!(
        UserDependencyConfigStore::at(shared)?
            .read()?
            .ffmpeg
            .is_none()
    );
    Ok(())
}

#[test]
fn session_roots_and_a_created_parent_are_private() -> TestResult {
    let parent = HostileParent::new()?;
    let default_like = parent.join("VSift-sessions");
    let created = open_session_root(&default_like, SessionRootProvisioning::CreateIfMissing)?;
    assert!(created.is_some());
    drop(created);
    assert_private_and_protected(&default_like, &parent)?;
    assert!(open_session_root(&default_like, SessionRootProvisioning::ExistingOnly)?.is_some());

    let nested = parent.join("cache").join("sessions");
    assert!(open_session_root(&nested, SessionRootProvisioning::CreateIfMissing)?.is_some());
    assert_private_and_protected(&parent.join("cache"), &parent)?;
    assert_private_and_protected(&nested, &parent)?;
    Ok(())
}

#[test]
fn existing_non_private_roots_are_refused_and_left_untouched() -> TestResult {
    let parent = HostileParent::new()?;
    // Each folder holds content, as one left by an older VSift would, so no
    // concurrent creator can still be finishing it and it is judged at once.
    let config_root = parent.join("vsift");
    fs::create_dir(&config_root)?;
    fs::write(config_root.join("notes.txt"), b"user content")?;
    let before = dacl_sddl(&config_root, &parent)?;
    let config = UserDependencyConfigStore::at(config_root.clone())?;
    assert!(matches!(
        config.read(),
        Err(UserDependencyConfigError::StorageNotPrivate)
    ));
    let model = parent.join("model.bin");
    fs::write(&model, b"fixture only")?;
    assert!(matches!(
        config.configure_model(&model),
        Err(UserDependencyConfigError::StorageNotPrivate)
    ));
    assert_eq!(dacl_sddl(&config_root, &parent)?, before);
    assert_eq!(fs::read_dir(&config_root)?.count(), 1);

    let session_root = parent.join("VSift-sessions");
    fs::create_dir(&session_root)?;
    fs::write(session_root.join("notes.txt"), b"user content")?;
    let before = dacl_sddl(&session_root, &parent)?;
    assert!(matches!(
        open_session_root(&session_root, SessionRootProvisioning::CreateIfMissing),
        Err(SessionRootError::Store(
            SessionStoreOpenError::RootNotPrivate
        ))
    ));
    assert_eq!(dacl_sddl(&session_root, &parent)?, before);
    assert_eq!(fs::read_dir(&session_root)?.count(), 1);
    Ok(())
}
