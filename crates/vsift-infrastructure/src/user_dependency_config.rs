//! Private per-user selection of externally managed executables.

use std::{
    env,
    error::Error,
    fmt, fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
#[cfg(unix)]
use cap_std::fs::OpenOptionsExt;
use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Serialize};
use vsift_domain::RuntimeDependency;

use crate::{
    ExplicitProbePaths, TrustedExecutable,
    file_lock::HeldFileLock,
    private_user_root::{PrivateRootError, open_private_root},
};

const CONFIG_FILE: &str = "dependencies-v1.json";
const LOCK_FILE: &str = "dependencies.lock";
const MAX_CONFIG_BYTES: u64 = 4096;

/// A typed failure to read or change explicitly selected user tools.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserDependencyConfigError {
    /// The per-user configuration base is unavailable or relative.
    Unavailable,
    /// A configured executable is not an absolute regular file.
    InvalidExecutable,
    /// A selected model is not an absolute, nonempty regular file.
    InvalidModel,
    /// The configuration directory or file is not private and regular.
    UnsafeStorage,
    /// The stored schema, paths or keys are invalid.
    InvalidRecord,
    /// A concurrent writer holds the configuration lock.
    Busy,
    /// The operating system could not complete a storage operation.
    Io,
}

impl fmt::Display for UserDependencyConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "per-user configuration location is unavailable",
            Self::InvalidExecutable => "selected executable is not an absolute regular file",
            Self::InvalidModel => "selected model is not an absolute nonempty regular file",
            Self::UnsafeStorage => "per-user configuration storage is not private",
            Self::InvalidRecord => "per-user dependency configuration is invalid",
            Self::Busy => "per-user dependency configuration is busy",
            Self::Io => "per-user dependency configuration I/O failed",
        })
    }
}

impl Error for UserDependencyConfigError {}

/// User-owned configuration; it never authorizes a managed download or install.
#[derive(Clone, Debug)]
pub struct UserDependencyConfigStore {
    root_path: PathBuf,
}

impl UserDependencyConfigStore {
    /// Resolves the platform's per-user configuration location without loading project files.
    ///
    /// # Errors
    ///
    /// Fails if the host has no absolute per-user configuration base.
    pub fn default_location() -> Result<Self, UserDependencyConfigError> {
        #[cfg(windows)]
        let base = env::var_os("LOCALAPPDATA").map(PathBuf::from);
        #[cfg(target_os = "macos")]
        let base = env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support"));
        #[cfg(all(unix, not(target_os = "macos")))]
        let base = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
        let base = base.ok_or(UserDependencyConfigError::Unavailable)?;
        Self::at(base.join("vsift"))
    }

    /// Selects an explicit configuration root for a trusted host or test.
    ///
    /// # Errors
    ///
    /// Rejects relative, parent-traversing and root paths.
    pub fn at(root_path: PathBuf) -> Result<Self, UserDependencyConfigError> {
        if !root_path.is_absolute()
            || root_path.file_name().is_none()
            || root_path
                .components()
                .any(|part| part == Component::ParentDir)
        {
            return Err(UserDependencyConfigError::Unavailable);
        }
        #[cfg(windows)]
        if matches!(
            root_path.components().next(),
            Some(Component::Prefix(prefix))
                if !matches!(
                    prefix.kind(),
                    std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
                )
        ) {
            return Err(UserDependencyConfigError::Unavailable);
        }
        Ok(Self { root_path })
    }

    /// Reads persisted selections without creating a directory or executing a provider.
    ///
    /// # Errors
    ///
    /// Fails closed on an unsafe root, malformed record or storage failure.
    pub fn read(&self) -> Result<ExplicitProbePaths, UserDependencyConfigError> {
        let Some(root) = self.open_root(false)? else {
            return Ok(ExplicitProbePaths::default());
        };
        read_record(&root).map(StoredSelections::into_paths)
    }

    /// Reads the optional user-selected ASR model path without opening model bytes.
    ///
    /// # Errors
    ///
    /// Fails closed on an unsafe root, malformed record or storage failure.
    pub fn read_model(&self) -> Result<Option<PathBuf>, UserDependencyConfigError> {
        let Some(root) = self.open_root(false)? else {
            return Ok(None);
        };
        read_record(&root).map(|record| record.model)
    }

    /// Atomically records one canonical user-selected executable path.
    ///
    /// This validates the file's present identity but does not claim provider compatibility.
    /// A later check resolves and probes the path again because BYO files are user-managed.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, storage, integrity or contention failure.
    pub fn configure(
        &self,
        dependency: RuntimeDependency,
        executable: &Path,
    ) -> Result<(), UserDependencyConfigError> {
        let selected = TrustedExecutable::explicit(executable)
            .map_err(|_| UserDependencyConfigError::InvalidExecutable)?;
        if selected.path().to_str().is_none() {
            return Err(UserDependencyConfigError::InvalidExecutable);
        }
        self.update_record(|record| record.set(dependency, selected.path().to_path_buf()))
    }

    /// Records a canonical path to a user-selected model file without loading or running it.
    ///
    /// Registration only establishes file presence; it does not prove model format,
    /// compatibility, accuracy or safety. A later preflight must revalidate the file.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, storage, integrity or contention failure.
    pub fn configure_model(&self, file: &Path) -> Result<(), UserDependencyConfigError> {
        if !file.is_absolute() {
            return Err(UserDependencyConfigError::InvalidModel);
        }
        let selected =
            fs::canonicalize(file).map_err(|_| UserDependencyConfigError::InvalidModel)?;
        let metadata =
            fs::metadata(&selected).map_err(|_| UserDependencyConfigError::InvalidModel)?;
        if !metadata.is_file() || metadata.len() == 0 || selected.to_str().is_none() {
            return Err(UserDependencyConfigError::InvalidModel);
        }
        self.update_record(|record| record.model = Some(selected))
    }

    fn update_record(
        &self,
        change: impl FnOnce(&mut StoredSelections),
    ) -> Result<(), UserDependencyConfigError> {
        let root = self
            .open_root(true)?
            .ok_or(UserDependencyConfigError::Unavailable)?;
        let mut lock_options = OpenOptions::new();
        lock_options
            .read(true)
            .write(true)
            .create(true)
            .follow(FollowSymlinks::No);
        let lock = root
            .open_with(LOCK_FILE, &lock_options)
            .map_err(|_| UserDependencyConfigError::Io)?;
        let metadata = lock.metadata().map_err(|_| UserDependencyConfigError::Io)?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(UserDependencyConfigError::UnsafeStorage);
        }
        let _lock = HeldFileLock::try_exclusive(lock.into_std()).map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => UserDependencyConfigError::Busy,
            std::fs::TryLockError::Error(_) => UserDependencyConfigError::Io,
        })?;
        let mut record = read_record(&root)?;
        change(&mut record);
        let bytes =
            serde_json::to_vec(&record).map_err(|_| UserDependencyConfigError::InvalidRecord)?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(UserDependencyConfigError::InvalidRecord);
        }
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| UserDependencyConfigError::Io)?;
        let temporary = format!("dependencies-{}.pending", hex(&random));
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = root
            .open_with(&temporary, &options)
            .map_err(|_| UserDependencyConfigError::Io)?;
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            let _ = root.remove_file(&temporary);
            return Err(if error.kind() == std::io::ErrorKind::PermissionDenied {
                UserDependencyConfigError::UnsafeStorage
            } else {
                UserDependencyConfigError::Io
            });
        }
        drop(file);
        if root.rename(&temporary, &root, CONFIG_FILE).is_err() {
            let _ = root.remove_file(&temporary);
            return Err(UserDependencyConfigError::Io);
        }
        Ok(())
    }

    fn open_root(&self, create: bool) -> Result<Option<Dir>, UserDependencyConfigError> {
        open_private_root(&self.root_path, create).map_err(|error| match error {
            PrivateRootError::Unavailable => UserDependencyConfigError::Unavailable,
            PrivateRootError::UnsafeStorage => UserDependencyConfigError::UnsafeStorage,
            PrivateRootError::Busy => UserDependencyConfigError::Busy,
            PrivateRootError::Io => UserDependencyConfigError::Io,
        })
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSelections {
    schema_version: u8,
    ffmpeg: Option<PathBuf>,
    ffprobe: Option<PathBuf>,
    whisper: Option<PathBuf>,
    model: Option<PathBuf>,
}

impl StoredSelections {
    fn set(&mut self, dependency: RuntimeDependency, path: PathBuf) {
        match dependency {
            RuntimeDependency::Ffmpeg => self.ffmpeg = Some(path),
            RuntimeDependency::Ffprobe => self.ffprobe = Some(path),
            RuntimeDependency::Whisper => self.whisper = Some(path),
        }
    }

    fn into_paths(self) -> ExplicitProbePaths {
        ExplicitProbePaths {
            ffmpeg: self.ffmpeg,
            ffprobe: self.ffprobe,
            whisper: self.whisper,
        }
    }
}

fn read_record(root: &Dir) -> Result<StoredSelections, UserDependencyConfigError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let Ok(file) = root.open_with(CONFIG_FILE, &options) else {
        return match root.symlink_metadata(CONFIG_FILE) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(StoredSelections {
                schema_version: 1,
                ..StoredSelections::default()
            }),
            Ok(_) => Err(UserDependencyConfigError::UnsafeStorage),
            Err(_) => Err(UserDependencyConfigError::Io),
        };
    };
    let metadata = file.metadata().map_err(|_| UserDependencyConfigError::Io)?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.len() > MAX_CONFIG_BYTES {
        return Err(UserDependencyConfigError::UnsafeStorage);
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UserDependencyConfigError::Io)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(UserDependencyConfigError::InvalidRecord);
    }
    let record: StoredSelections =
        serde_json::from_slice(&bytes).map_err(|_| UserDependencyConfigError::InvalidRecord)?;
    if record.schema_version != 1
        || [
            record.ffmpeg.as_ref(),
            record.ffprobe.as_ref(),
            record.whisper.as_ref(),
            record.model.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|path| !path.is_absolute())
    {
        return Err(UserDependencyConfigError::InvalidRecord);
    }
    Ok(record)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    result
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use vsift_domain::RuntimeDependency;

    use super::{UserDependencyConfigError, UserDependencyConfigStore, hex};

    fn fixture_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| std::io::Error::other("random source failed"))?;
        let root = std::env::temp_dir().join(format!("vsift-user-config-test-{}", hex(&random)));
        fs::create_dir(&root)?;
        Ok(root)
    }

    #[test]
    fn records_and_replaces_canonical_paths_without_executing_them()
    -> Result<(), Box<dyn std::error::Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let config = UserDependencyConfigStore::at(parent.join("config"))?;
            assert!(config.read()?.ffmpeg.is_none());
            let first = parent.join("first");
            let second = parent.join("second");
            fs::write(&first, b"not executed")?;
            fs::write(&second, b"also not executed")?;
            config.configure(RuntimeDependency::Ffmpeg, &first)?;
            assert_eq!(config.read()?.ffmpeg, Some(fs::canonicalize(&first)?));
            config.configure(RuntimeDependency::Ffmpeg, &second)?;
            assert_eq!(config.read()?.ffmpeg, Some(fs::canonicalize(&second)?));
            Ok::<(), Box<dyn std::error::Error>>(())
        })();
        fs::remove_dir_all(&parent)?;
        result
    }

    #[test]
    fn model_registration_preserves_executables_and_replaces_only_the_model()
    -> Result<(), Box<dyn std::error::Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let config = UserDependencyConfigStore::at(parent.join("config"))?;
            let executable = parent.join("ffmpeg");
            let first = parent.join("first-model");
            let second = parent.join("second-model");
            fs::write(&executable, b"not executed")?;
            fs::write(&first, b"not parsed")?;
            fs::write(&second, b"also not parsed")?;
            config
                .configure(RuntimeDependency::Ffmpeg, &executable)
                .map_err(|error| std::io::Error::other(format!("configure ffmpeg: {error:?}")))?;
            config.configure_model(&first).map_err(|error| {
                std::io::Error::other(format!("configure first model: {error:?}"))
            })?;
            assert_eq!(config.read_model()?, Some(fs::canonicalize(&first)?));
            config.configure_model(&second).map_err(|error| {
                std::io::Error::other(format!("configure second model: {error:?}"))
            })?;
            assert_eq!(config.read_model()?, Some(fs::canonicalize(&second)?));
            assert_eq!(config.read()?.ffmpeg, Some(fs::canonicalize(&executable)?));
            Ok::<(), Box<dyn std::error::Error>>(())
        })();
        fs::remove_dir_all(&parent)?;
        result
    }

    #[test]
    fn concurrent_configuration_lock_reports_busy_without_changing_the_record()
    -> Result<(), Box<dyn std::error::Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let config = UserDependencyConfigStore::at(parent.join("config"))?;
            let executable = parent.join("ffmpeg");
            let model = parent.join("model");
            fs::write(&executable, b"not executed")?;
            fs::write(&model, b"not parsed")?;
            config.configure(RuntimeDependency::Ffmpeg, &executable)?;
            let held = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(parent.join("config/dependencies.lock"))?;
            held.try_lock()?;
            assert_eq!(
                config.configure_model(&model),
                Err(UserDependencyConfigError::Busy)
            );
            assert!(config.read_model()?.is_none());
            drop(held);
            config.configure_model(&model)?;
            assert_eq!(config.read_model()?, Some(fs::canonicalize(&model)?));
            Ok::<(), Box<dyn std::error::Error>>(())
        })();
        fs::remove_dir_all(&parent)?;
        result
    }

    #[test]
    fn invalid_model_does_not_create_or_change_configuration()
    -> Result<(), Box<dyn std::error::Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let config = UserDependencyConfigStore::at(parent.join("config"))?;
            assert_eq!(
                config.configure_model(PathBuf::from("relative-model").as_path()),
                Err(UserDependencyConfigError::InvalidModel)
            );
            assert!(!parent.join("config").exists());
            let empty = parent.join("empty-model");
            fs::write(&empty, b"")?;
            assert_eq!(
                config.configure_model(&empty),
                Err(UserDependencyConfigError::InvalidModel)
            );
            assert!(!parent.join("config").exists());
            Ok::<(), Box<dyn std::error::Error>>(())
        })();
        fs::remove_dir_all(&parent)?;
        result
    }

    #[test]
    fn prior_executable_only_record_remains_readable() -> Result<(), Box<dyn std::error::Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let config = UserDependencyConfigStore::at(parent.join("config"))?;
            let executable = parent.join("tool");
            fs::write(&executable, b"not executed")?;
            config.configure(RuntimeDependency::Whisper, &executable)?;
            let record_path = parent.join("config/dependencies-v1.json");
            let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&record_path)?)?;
            if let Some(fields) = record.as_object_mut() {
                fields.remove("model");
            }
            fs::write(&record_path, serde_json::to_vec(&record)?)?;
            assert_eq!(config.read_model()?, None);
            assert_eq!(config.read()?.whisper, Some(fs::canonicalize(&executable)?));
            Ok::<(), Box<dyn std::error::Error>>(())
        })();
        fs::remove_dir_all(&parent)?;
        result
    }

    #[test]
    fn corrupt_or_unrecognized_config_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let config = UserDependencyConfigStore::at(parent.join("config"))?;
            let executable = parent.join("tool");
            fs::write(&executable, b"not executed")?;
            config.configure(RuntimeDependency::Whisper, &executable)?;
            fs::write(
                parent.join("config/dependencies-v1.json"),
                br#"{"schema_version":1,"unexpected":true}"#,
            )?;
            assert!(matches!(
                config.read(),
                Err(UserDependencyConfigError::InvalidRecord)
            ));
            Ok::<(), Box<dyn std::error::Error>>(())
        })();
        fs::remove_dir_all(&parent)?;
        result
    }

    #[test]
    fn rejects_relative_selection_without_creating_config() -> Result<(), Box<dyn std::error::Error>>
    {
        let parent = fixture_root()?;
        let result = (|| {
            let config = UserDependencyConfigStore::at(parent.join("config"))?;
            assert_eq!(
                config.configure(
                    RuntimeDependency::Ffprobe,
                    PathBuf::from("relative").as_path()
                ),
                Err(UserDependencyConfigError::InvalidExecutable)
            );
            assert!(!parent.join("config").exists());
            Ok::<(), Box<dyn std::error::Error>>(())
        })();
        fs::remove_dir_all(&parent)?;
        result
    }

    #[test]
    fn rejects_hard_linked_record_without_following_an_external_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let parent = fixture_root()?;
        let result = (|| {
            let config = UserDependencyConfigStore::at(parent.join("config"))?;
            let executable = parent.join("tool");
            fs::write(&executable, b"not executed")?;
            config.configure(RuntimeDependency::Ffmpeg, &executable)?;
            let outside = parent.join("outside.json");
            fs::hard_link(parent.join("config/dependencies-v1.json"), &outside)?;
            assert!(matches!(
                config.read(),
                Err(UserDependencyConfigError::UnsafeStorage)
            ));
            Ok::<(), Box<dyn std::error::Error>>(())
        })();
        fs::remove_dir_all(&parent)?;
        result
    }

    #[cfg(unix)]
    #[test]
    fn refuses_configuration_root_with_group_access() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;

        let parent = fixture_root()?;
        let result = (|| {
            let config = UserDependencyConfigStore::at(parent.join("config"))?;
            let executable = parent.join("tool");
            fs::write(&executable, b"not executed")?;
            config.configure(RuntimeDependency::Ffmpeg, &executable)?;
            fs::set_permissions(parent.join("config"), fs::Permissions::from_mode(0o750))?;
            assert!(matches!(
                config.read(),
                Err(UserDependencyConfigError::UnsafeStorage)
            ));
            Ok::<(), Box<dyn std::error::Error>>(())
        })();
        fs::remove_dir_all(&parent)?;
        result
    }
}
