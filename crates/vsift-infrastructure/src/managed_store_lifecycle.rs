//! Inspection and guarded maintenance of the private managed store (P13 PR
//! 6, ADR 0023 §3 steps 5 and 6).
//!
//! [`ManagedArtifactStore::inspect`] reads the store without changing it or
//! creating it: every selection pointer, every published version and whether
//! it verifies, every stage and whether a sweep could remove it, and anything
//! no `VSift` operation creates. Verifying a version hashes its files while its
//! shared use lock is held, exactly as lookup does, so a version being
//! removed is never reported as verified.
//!
//! Under the root's install guard, [`ManagedArtifactStore::deselect_component`]
//! removes one component's pointer and [`ManagedArtifactStore::sweep_stale_stages`]
//! removes abandoned stages. A stage is created only by an installation,
//! which holds the same guard for its whole run, so while a sweep holds it no
//! stage can be live. A stage is removed only when every entry is positively
//! identified: the stage marker, the verified artifact, and the payload,
//! runtime and smoke folders holding only single-link regular files. Nothing
//! is followed: a link or junction anywhere keeps the whole stage, and the
//! marker is removed last so an interrupted sweep leaves a stage the next
//! sweep still recognises. Selection, removal and publication live in
//! `managed_artifact_store`.
//!
//! [`ManagedStoreInspector`] and [`GuardedManagedStore`] are the adapters of
//! the application's [`ManagedStoreReader`] and [`ManagedStoreMaintenance`]
//! ports.

use std::{fs, path::Path};

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use vsift_application::{
    MANAGED_COMPONENTS, ManagedInventory, ManagedSelection, ManagedStoreFault,
    ManagedStoreMaintenance, ManagedStoreReader, ManagedVersionFault, ManagedVersionRecord,
    ManagedVersionState, PreviousSelection, SelectionFailure, SelectionRecord,
    StageRetentionReason, StageSweep, VersionRemovalStatus,
};
use vsift_domain::ManagedComponent;

use crate::{
    ManagedArtifactError, ManagedArtifactStore, ManagedInstallGuard, ManagedRuntimeIdentity,
    ManagedRuntimePublicationError, ManagedVersionRemovalOutcome,
    file_lock::HeldFileLock,
    managed_artifact_store::{
        ARTIFACT, CURRENT, CURRENT_IDENTITY, ContentCheck, DIRECTORY_MARKER, INSTALL_LOCK,
        MAX_VERSION_METADATA_BYTES, PAYLOAD, ROOT_MARKER, RUNTIME, SMOKE, STAGE_IDENTITY,
        STAGE_MARKER, VERSION_MANIFEST, VERSION_REMOVING, VERSIONS, VERSIONS_IDENTITY,
        canonical_managed_key, check_marker, open_managed_directory, open_version_use_lock,
        parse_version_manifest, read_current_pointer, read_private_regular_file,
        remove_known_regular_file_if_present, sha256_hex, validate_install_guard,
        validate_owned_regular_file, validate_private_regular_metadata, validate_version_contents,
    },
    private_user_root::{validate_private_root, validate_same_held_directory},
};

/// The prefix of a stage directory's name; 32 lowercase hexadecimal digits
/// follow it.
const STAGE_PREFIX: &str = "stage-";

/// The folders a stage may hold, each flat.
const STAGE_FOLDERS: [&str; 3] = [PAYLOAD, RUNTIME, SMOKE];

/// The managed component a store key names.
fn managed_component(key: &str) -> Option<ManagedComponent> {
    MANAGED_COMPONENTS
        .into_iter()
        .find(|component| component.identifier() == key)
}

/// Whether `name` is the name `create_stage` gives a stage.
fn is_stage_name(name: &str) -> bool {
    name.strip_prefix(STAGE_PREFIX).is_some_and(|random| {
        random.len() == 32
            && random
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

const fn store_fault(error: &ManagedArtifactError) -> ManagedStoreFault {
    match error {
        ManagedArtifactError::UnsafeStorage | ManagedArtifactError::Transfer(_) => {
            ManagedStoreFault::Unsafe
        }
        ManagedArtifactError::Unavailable
        | ManagedArtifactError::Busy
        | ManagedArtifactError::Io => ManagedStoreFault::Io,
    }
}

const fn publication_fault(error: &ManagedRuntimePublicationError) -> ManagedStoreFault {
    match error {
        ManagedRuntimePublicationError::Storage(error) => store_fault(error),
        ManagedRuntimePublicationError::InvalidIdentity
        | ManagedRuntimePublicationError::VersionConflict => ManagedStoreFault::Unsafe,
    }
}

impl ManagedArtifactStore {
    /// Everything the store holds, read without changing or creating
    /// anything; `None` when this user has no managed root.
    ///
    /// # Errors
    ///
    /// The root, its version folder or its selection folder cannot be
    /// proved private and `VSift`'s own (a link, another owner, open
    /// permissions, a changed marker), or cannot be read.
    pub fn inspect(&self) -> Result<Option<ManagedInventory>, ManagedStoreFault> {
        let Some(root) = self
            .open_existing_root()
            .map_err(|error| store_fault(&error))?
        else {
            return Ok(None);
        };
        let mut inventory = ManagedInventory::empty();
        for entry in root.entries().map_err(|_| ManagedStoreFault::Io)? {
            let entry = entry.map_err(|_| ManagedStoreFault::Io)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                inventory.unexpected_entries += 1;
                continue;
            };
            match name {
                ROOT_MARKER | INSTALL_LOCK => {}
                VERSIONS => {
                    let versions = open_managed_directory(
                        &root,
                        self.root_path(),
                        VERSIONS,
                        VERSIONS_IDENTITY,
                    )
                    .map_err(|error| store_fault(&error))?;
                    self.inspect_versions(&versions, &mut inventory)?;
                }
                CURRENT => {
                    let current =
                        open_managed_directory(&root, self.root_path(), CURRENT, CURRENT_IDENTITY)
                            .map_err(|error| store_fault(&error))?;
                    inspect_selections(&current, &mut inventory)?;
                }
                stage if is_stage_name(stage) => {
                    match sweep_stage(&root, self.root_path(), stage, SweepAction::Inspect) {
                        Ok(()) => inventory.stale_stages += 1,
                        Err(reason) => inventory.retained_stages.push(reason),
                    }
                }
                _ => inventory.unexpected_entries += 1,
            }
        }
        for component in &mut inventory.components {
            component
                .versions
                .sort_by(|left, right| left.version.cmp(&right.version));
        }
        Ok(Some(inventory))
    }

    fn inspect_versions(
        &self,
        versions: &Dir,
        inventory: &mut ManagedInventory,
    ) -> Result<(), ManagedStoreFault> {
        for entry in versions.entries().map_err(|_| ManagedStoreFault::Io)? {
            let entry = entry.map_err(|_| ManagedStoreFault::Io)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                inventory.unexpected_entries += 1;
                continue;
            };
            if name == DIRECTORY_MARKER {
                continue;
            }
            let Some((component, identity)) = name.split_once("--").and_then(|(key, version)| {
                let identity = ManagedRuntimeIdentity::new(key, version).ok()?;
                Some((managed_component(key)?, identity))
            }) else {
                inventory.unexpected_entries += 1;
                continue;
            };
            let state = version_state(self.root_path(), versions, name, &identity);
            if let Some(installed) = inventory
                .components
                .iter_mut()
                .find(|installed| installed.component == component)
            {
                installed.versions.push(ManagedVersionRecord {
                    version: identity.version().to_owned(),
                    state,
                });
            }
        }
        Ok(())
    }

    /// Removes `component`'s selection pointer, and a half-written one, so
    /// commands stop using its managed version; `true` when a pointer was
    /// removed. The pointer is removed whatever it says: an unreadable one
    /// is `VSift`'s own file in `VSift`'s own folder.
    ///
    /// # Errors
    ///
    /// Requires the install guard of this root; the selection folder or the
    /// pointer is not a private, `VSift`-owned folder or single-link regular
    /// file, or removal failed. The selection is then unchanged.
    pub fn deselect_component(
        &self,
        guard: &ManagedInstallGuard,
        component: &str,
    ) -> Result<bool, ManagedRuntimePublicationError> {
        if !canonical_managed_key(component) {
            return Err(ManagedRuntimePublicationError::InvalidIdentity);
        }
        let Some(root) = self
            .open_existing_root()
            .map_err(ManagedRuntimePublicationError::Storage)?
        else {
            return Ok(false);
        };
        validate_install_guard(guard, &root, self.root_path())?;
        if !root
            .try_exists(CURRENT)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?
        {
            return Ok(false);
        }
        let current = open_managed_directory(&root, self.root_path(), CURRENT, CURRENT_IDENTITY)
            .map_err(ManagedRuntimePublicationError::Storage)?;
        remove_known_regular_file_if_present(&current, &format!("{component}.pending"))
            .map_err(ManagedRuntimePublicationError::Storage)?;
        let name = format!("{component}.current");
        match current.symlink_metadata(&name) {
            Ok(metadata) => {
                validate_private_regular_metadata(&metadata)
                    .map_err(ManagedRuntimePublicationError::Storage)?;
                current.remove_file(&name).map_err(|_| {
                    ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io)
                })?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(ManagedRuntimePublicationError::Storage(
                ManagedArtifactError::UnsafeStorage,
            )),
        }
    }

    /// Removes every abandoned stage whose every entry is positively `VSift`'s
    /// own, and every half-written selection pointer, and reports the stages
    /// it kept and why.
    ///
    /// The guard proves no installation is running, and an installation is
    /// the only thing that creates a stage, so every stage found here is
    /// abandoned: left by a killed run, or kept by a failure whose cleanup
    /// could not prove it (PR 3 and PR 4 keep such stages rather than risk
    /// deleting what they did not create).
    ///
    /// # Errors
    ///
    /// Requires the install guard of this root; the root or its selection
    /// folder cannot be proved `VSift`'s own, or cannot be read.
    pub fn sweep_stale_stages(
        &self,
        guard: &ManagedInstallGuard,
    ) -> Result<StageSweep, ManagedRuntimePublicationError> {
        let mut sweep = StageSweep::default();
        let Some(root) = self
            .open_existing_root()
            .map_err(ManagedRuntimePublicationError::Storage)?
        else {
            return Ok(sweep);
        };
        validate_install_guard(guard, &root, self.root_path())?;
        let mut stages = Vec::new();
        for entry in root
            .entries()
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?
        {
            let entry = entry
                .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?;
            if let Some(name) = entry.file_name().to_str()
                && is_stage_name(name)
            {
                stages.push(name.to_owned());
            }
        }
        stages.sort();
        for stage in &stages {
            let outcome = sweep_stage(&root, self.root_path(), stage, SweepAction::Inspect)
                .and_then(|()| sweep_stage(&root, self.root_path(), stage, SweepAction::Remove));
            match outcome {
                Ok(()) => sweep.removed += 1,
                Err(reason) => sweep.retained.push(reason),
            }
        }
        if root
            .try_exists(CURRENT)
            .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?
        {
            let current =
                open_managed_directory(&root, self.root_path(), CURRENT, CURRENT_IDENTITY)
                    .map_err(ManagedRuntimePublicationError::Storage)?;
            let mut pending = Vec::new();
            for entry in current
                .entries()
                .map_err(|_| ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io))?
            {
                let entry = entry.map_err(|_| {
                    ManagedRuntimePublicationError::Storage(ManagedArtifactError::Io)
                })?;
                if let Some(name) = entry.file_name().to_str()
                    && name
                        .strip_suffix(".pending")
                        .is_some_and(|key| managed_component(key).is_some())
                {
                    pending.push(name.to_owned());
                }
            }
            for name in pending {
                // A pending pointer that is not a private regular file is
                // left for the user, and repair keeps reporting it.
                if remove_known_regular_file_if_present(&current, &name).is_ok() {
                    sweep.interrupted_selections_removed += 1;
                }
            }
        }
        Ok(sweep)
    }
}

/// Reads every selection pointer and half-written pointer.
fn inspect_selections(
    current: &Dir,
    inventory: &mut ManagedInventory,
) -> Result<(), ManagedStoreFault> {
    for entry in current.entries().map_err(|_| ManagedStoreFault::Io)? {
        let entry = entry.map_err(|_| ManagedStoreFault::Io)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            inventory.unexpected_entries += 1;
            continue;
        };
        if name == DIRECTORY_MARKER {
            continue;
        }
        if name
            .strip_suffix(".pending")
            .is_some_and(|key| managed_component(key).is_some())
        {
            inventory.interrupted_selections += 1;
            continue;
        }
        let Some((key, component)) = name
            .strip_suffix(".current")
            .and_then(|key| Some((key, managed_component(key)?)))
        else {
            inventory.unexpected_entries += 1;
            continue;
        };
        let selection = match read_current_pointer(current, name) {
            Ok(pointer) if pointer.identity.component() == key => {
                ManagedSelection::Recorded(SelectionRecord {
                    version: pointer.identity.version().to_owned(),
                    manifest_sha256: pointer.manifest_sha256,
                    previous: pointer.previous.map(|previous| PreviousSelection {
                        version: previous.identity.version().to_owned(),
                        manifest_sha256: previous.manifest_sha256,
                    }),
                })
            }
            Ok(_) | Err(_) => ManagedSelection::Unreadable,
        };
        if let Some(installed) = inventory
            .components
            .iter_mut()
            .find(|installed| installed.component == component)
        {
            installed.selection = selection;
        }
    }
    Ok(())
}

/// Whether one published version verifies, and if not why. Never changes
/// anything; the bytes are hashed while the version's shared use lock is
/// held, as lookup does.
fn version_state(
    root_path: &Path,
    versions: &Dir,
    name: &str,
    identity: &ManagedRuntimeIdentity,
) -> ManagedVersionState {
    use ManagedVersionFault::{
        ChangedContent, InvalidManifest, MissingManifest, UnexpectedEntry, Unreadable,
    };
    let unverified = ManagedVersionState::Unverified;
    match versions.symlink_metadata(name) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return unverified(UnexpectedEntry),
        Err(_) => return unverified(Unreadable),
    }
    let Ok(directory) = versions.open_dir_nofollow(name) else {
        return unverified(UnexpectedEntry);
    };
    if validate_private_root(&root_path.join(VERSIONS).join(name), &directory).is_err() {
        return unverified(UnexpectedEntry);
    }
    let Ok(removing) = directory.try_exists(VERSION_REMOVING) else {
        return unverified(Unreadable);
    };
    match directory.try_exists(VERSION_MANIFEST) {
        Ok(true) => {}
        Ok(false) if removing => return ManagedVersionState::RemovalInterrupted,
        Ok(false) => return unverified(MissingManifest),
        Err(_) => return unverified(Unreadable),
    }
    let Ok(manifest) =
        read_private_regular_file(&directory, VERSION_MANIFEST, MAX_VERSION_METADATA_BYTES)
    else {
        return unverified(InvalidManifest);
    };
    let Ok((observed, files)) = parse_version_manifest(&manifest) else {
        return unverified(InvalidManifest);
    };
    if observed != *identity {
        return unverified(InvalidManifest);
    }
    if removing {
        return ManagedVersionState::RemovalInterrupted;
    }
    if validate_version_contents(&directory, &files, false, ContentCheck::Ownership).is_err() {
        return unverified(UnexpectedEntry);
    }
    if validate_version_contents(&directory, &files, false, ContentCheck::Layout).is_err() {
        return unverified(ChangedContent);
    }
    let Ok(lock) = open_version_use_lock(&directory) else {
        return unverified(UnexpectedEntry);
    };
    let _held = match HeldFileLock::try_shared(lock) {
        Ok(held) => held,
        // Only a removal holds the use lock exclusively.
        Err(fs::TryLockError::WouldBlock) => return ManagedVersionState::RemovalInterrupted,
        Err(fs::TryLockError::Error(_)) => return unverified(Unreadable),
    };
    match validate_version_contents(&directory, &files, false, ContentCheck::LayoutAndBytes) {
        Ok(()) => ManagedVersionState::Verified {
            manifest_sha256: sha256_hex(&manifest),
        },
        Err(ManagedArtifactError::Transfer(_) | ManagedArtifactError::UnsafeStorage) => {
            unverified(ChangedContent)
        }
        Err(
            ManagedArtifactError::Io
            | ManagedArtifactError::Busy
            | ManagedArtifactError::Unavailable,
        ) => unverified(Unreadable),
    }
}

/// Whether a stage sweep only checks a stage or also removes it.
#[derive(Clone, Copy, Eq, PartialEq)]
enum SweepAction {
    /// Prove every entry `VSift`'s own; change nothing.
    Inspect,
    /// Remove every entry, rechecking each, the marker last.
    Remove,
}

/// Proves (and with [`SweepAction::Remove`] removes) one stage.
///
/// A stage is `VSift`'s when it is a private real directory (not a link or
/// junction) whose marker is intact, and whose entries are only the marker,
/// the artifact (a single-link regular file) and the payload, runtime and
/// smoke folders (private real directories holding only single-link regular
/// files). A stage with no marker is `VSift`'s only when it is empty or holds
/// just the start of its marker: what a creation killed before its marker was
/// complete, or a removal killed after its marker was removed, leaves.
fn sweep_stage(
    root: &Dir,
    root_path: &Path,
    name: &str,
    action: SweepAction,
) -> Result<(), StageRetentionReason> {
    let unproved = StageRetentionReason::OwnershipUnproved;
    let unexpected = StageRetentionReason::UnexpectedContent;
    let storage = StageRetentionReason::StorageFailure;
    match root.symlink_metadata(name) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        _ => return Err(unproved),
    }
    let stage = root.open_dir_nofollow(name).map_err(|_| unproved)?;
    let stage_path = root_path.join(name);
    validate_private_root(&stage_path, &stage).map_err(|_| unproved)?;
    if check_marker(&stage, STAGE_MARKER, STAGE_IDENTITY).is_err() {
        return sweep_unmarked_stage(root, name, stage, action);
    }
    let mut folders = Vec::new();
    for entry in stage.entries().map_err(|_| storage)? {
        let entry = entry.map_err(|_| storage)?;
        let entry_name = entry.file_name();
        let entry_name = entry_name.to_str().ok_or(unexpected)?;
        let metadata = stage.symlink_metadata(entry_name).map_err(|_| unexpected)?;
        match entry_name {
            STAGE_MARKER | ARTIFACT => {
                validate_owned_regular_file(&metadata).map_err(|_| unexpected)?;
            }
            folder if STAGE_FOLDERS.contains(&folder) => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(unexpected);
                }
                folders.push(folder.to_owned());
            }
            _ => return Err(unexpected),
        }
    }
    for folder in &folders {
        sweep_stage_folder(&stage, &stage_path, folder, action)?;
    }
    if action == SweepAction::Inspect {
        return Ok(());
    }
    match stage.symlink_metadata(ARTIFACT) {
        Ok(metadata) => {
            validate_owned_regular_file(&metadata).map_err(|_| unexpected)?;
            stage.remove_file(ARTIFACT).map_err(|_| storage)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(storage),
    }
    // The marker last: until it goes, the stage is still recognisably ours.
    stage.remove_file(STAGE_MARKER).map_err(|_| storage)?;
    drop(stage);
    root.remove_dir(name).map_err(|_| storage)
}

/// One flat folder of a stage: every entry a single-link regular file.
fn sweep_stage_folder(
    stage: &Dir,
    stage_path: &Path,
    folder: &str,
    action: SweepAction,
) -> Result<(), StageRetentionReason> {
    let unexpected = StageRetentionReason::UnexpectedContent;
    let storage = StageRetentionReason::StorageFailure;
    let held = stage.open_dir_nofollow(folder).map_err(|_| unexpected)?;
    validate_private_root(&stage_path.join(folder), &held).map_err(|_| unexpected)?;
    let mut names = Vec::new();
    for entry in held.entries().map_err(|_| storage)? {
        let entry = entry.map_err(|_| storage)?;
        let entry_name = entry.file_name();
        let entry_name = entry_name.to_str().ok_or(unexpected)?.to_owned();
        let metadata = held.symlink_metadata(&entry_name).map_err(|_| unexpected)?;
        validate_owned_regular_file(&metadata).map_err(|_| unexpected)?;
        names.push(entry_name);
    }
    if action == SweepAction::Inspect {
        return Ok(());
    }
    for entry_name in &names {
        let metadata = held.symlink_metadata(entry_name).map_err(|_| unexpected)?;
        validate_owned_regular_file(&metadata).map_err(|_| unexpected)?;
        held.remove_file(entry_name).map_err(|_| storage)?;
    }
    let at_name = stage.open_dir_nofollow(folder).map_err(|_| unexpected)?;
    validate_same_held_directory(&at_name, &held).map_err(|_| unexpected)?;
    drop(at_name);
    drop(held);
    stage.remove_dir(folder).map_err(|_| unexpected)
}

/// A stage whose marker is missing or incomplete.
fn sweep_unmarked_stage(
    root: &Dir,
    name: &str,
    stage: Dir,
    action: SweepAction,
) -> Result<(), StageRetentionReason> {
    let unproved = StageRetentionReason::OwnershipUnproved;
    let storage = StageRetentionReason::StorageFailure;
    let mut entries = stage.entries().map_err(|_| storage)?;
    let marker_only = match entries.next() {
        None => false,
        Some(entry) => {
            let entry = entry.map_err(|_| storage)?;
            if entry.file_name() != STAGE_MARKER || entries.next().is_some() {
                return Err(unproved);
            }
            true
        }
    };
    drop(entries);
    if marker_only {
        let metadata = stage.symlink_metadata(STAGE_MARKER).map_err(|_| unproved)?;
        validate_owned_regular_file(&metadata).map_err(|_| unproved)?;
        let partial = read_private_regular_file(&stage, STAGE_MARKER, STAGE_IDENTITY.len() as u64)
            .map_or(metadata.len() == 0, |bytes| {
                bytes.len() < STAGE_IDENTITY.len() && STAGE_IDENTITY.starts_with(&bytes)
            });
        if !partial {
            return Err(unproved);
        }
    }
    if action == SweepAction::Inspect {
        return Ok(());
    }
    if marker_only {
        stage.remove_file(STAGE_MARKER).map_err(|_| storage)?;
    }
    drop(stage);
    root.remove_dir(name).map_err(|_| storage)
}

/// The application's read port over a managed root: inspection only, with
/// no guard, so it never waits and never changes anything.
#[derive(Clone, Debug)]
pub struct ManagedStoreInspector {
    store: ManagedArtifactStore,
}

impl ManagedStoreInspector {
    /// An inspector of `store`.
    #[must_use]
    pub const fn new(store: ManagedArtifactStore) -> Self {
        Self { store }
    }
}

impl ManagedStoreReader for ManagedStoreInspector {
    fn inventory(&self) -> Result<Option<ManagedInventory>, ManagedStoreFault> {
        self.store.inspect()
    }
}

/// The application's maintenance port over a managed root, for as long as
/// the root's install guard is held.
#[derive(Debug)]
pub struct GuardedManagedStore<'guard> {
    store: &'guard ManagedArtifactStore,
    guard: &'guard ManagedInstallGuard,
}

impl<'guard> GuardedManagedStore<'guard> {
    /// Maintenance of `store` under `guard`, which must be its install
    /// guard (every step checks).
    #[must_use]
    pub const fn new(
        store: &'guard ManagedArtifactStore,
        guard: &'guard ManagedInstallGuard,
    ) -> Self {
        Self { store, guard }
    }
}

impl ManagedStoreReader for GuardedManagedStore<'_> {
    fn inventory(&self) -> Result<Option<ManagedInventory>, ManagedStoreFault> {
        self.store.inspect()
    }
}

impl ManagedStoreMaintenance for GuardedManagedStore<'_> {
    fn select(
        &self,
        component: ManagedComponent,
        version: &str,
        expected_manifest_sha256: Option<&str>,
    ) -> Result<(), SelectionFailure> {
        let identity = ManagedRuntimeIdentity::new(component.identifier(), version)
            .map_err(|_| SelectionFailure::Unverified)?;
        match self
            .store
            .select_verified_runtime(self.guard, &identity, expected_manifest_sha256)
        {
            Ok(published) => {
                drop(published);
                Ok(())
            }
            Err(error) => Err(match publication_fault(&error) {
                ManagedStoreFault::Unsafe => SelectionFailure::Unverified,
                ManagedStoreFault::Io => SelectionFailure::Store(ManagedStoreFault::Io),
            }),
        }
    }

    fn deselect(&self, component: ManagedComponent) -> Result<bool, ManagedStoreFault> {
        self.store
            .deselect_component(self.guard, component.identifier())
            .map_err(|error| publication_fault(&error))
    }

    fn remove_version(&self, component: ManagedComponent, version: &str) -> VersionRemovalStatus {
        let Ok(identity) = ManagedRuntimeIdentity::new(component.identifier(), version) else {
            return VersionRemovalStatus::AlreadyAbsent;
        };
        match self.store.remove_published_runtime(self.guard, &identity) {
            Ok(ManagedVersionRemovalOutcome::Removed) => VersionRemovalStatus::Removed,
            Ok(ManagedVersionRemovalOutcome::Selected) => VersionRemovalStatus::Selected,
            Ok(ManagedVersionRemovalOutcome::InUse) => VersionRemovalStatus::InUse,
            Err(error) => match publication_fault(&error) {
                ManagedStoreFault::Unsafe => VersionRemovalStatus::UnexpectedContent,
                ManagedStoreFault::Io => VersionRemovalStatus::StorageFailure,
            },
        }
    }

    fn sweep_stages(&self) -> Result<StageSweep, ManagedStoreFault> {
        self.store
            .sweep_stale_stages(self.guard)
            .map_err(|error| publication_fault(&error))
    }
}

#[cfg(test)]
mod tests;
