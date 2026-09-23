//! Engine construction: explicit configuration and injected ports.

use std::{
    env,
    path::{Path, PathBuf},
};

use vsift_application::{Clock, IdentifierSource};
use vsift_domain::{OperationId, SessionId};
use vsift_infrastructure::{
    FilesystemSessionStore, RandomIdentifierSource, SessionRootProvisioning, SystemClock,
    UserDependencyConfigStore, open_session_root, platform_session_root,
};

use crate::error::{EngineError, SessionRootError};

/// Where the engine keeps disposable investigation sessions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionRootLocation {
    /// The platform's per-user cache (for example `%LOCALAPPDATA%\VSift-sessions`
    /// on Windows or `$XDG_CACHE_HOME/vsift-sessions` on Linux).
    PlatformDefault,
    /// A host-selected root. It must be absolute; a relative path is rejected
    /// when a session operation first needs the root.
    Explicit(PathBuf),
}

/// Where the engine reads and writes the user's dependency selections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserConfigurationLocation {
    /// The platform's per-user configuration directory, never a project file.
    PlatformDefault,
    /// A host-selected absolute configuration directory.
    Explicit(PathBuf),
}

/// Process isolation the host has established around the engine.
///
/// Hosts report what they provide; the engine never assumes a stronger
/// boundary than the one reported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostIsolation {
    /// Only per-process containment; no inherited strict worker boundary.
    ProcessOnly,
    /// A Linux worker host has established and verified the strict R0 boundary.
    #[cfg(target_os = "linux")]
    StrictLinux,
}

impl HostIsolation {
    pub(crate) const fn into_infrastructure(self) -> vsift_infrastructure::HostIsolation {
        match self {
            Self::ProcessOnly => vsift_infrastructure::HostIsolation::ProcessOnly,
            #[cfg(target_os = "linux")]
            Self::StrictLinux => vsift_infrastructure::HostIsolation::StrictLinux,
        }
    }
}

/// Immutable host configuration for one engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineConfig {
    /// Where disposable sessions live.
    pub session_root: SessionRootLocation,
    /// Where the user's dependency selections live.
    pub user_configuration: UserConfigurationLocation,
    /// Isolation the host has established around provider processes.
    pub host_isolation: HostIsolation,
}

/// Time and identity ports the engine uses for every decision that would
/// otherwise read ambient process state.
pub struct EnginePorts {
    clock: Box<dyn Clock>,
    identifiers: Box<dyn IdentifierSource>,
}

impl EnginePorts {
    /// Uses the injected clock and identifier source.
    ///
    /// Tests and hosts with their own time or identity authority use this;
    /// see [`EnginePorts::system`] for the production defaults.
    #[must_use]
    pub fn new(clock: impl Clock + 'static, identifiers: impl IdentifierSource + 'static) -> Self {
        Self {
            clock: Box::new(clock),
            identifiers: Box::new(identifiers),
        }
    }

    /// Uses the operating-system clock and unguessable random identifiers.
    #[must_use]
    pub fn system() -> Self {
        Self::new(SystemClock, RandomIdentifierSource)
    }
}

/// The embeddable `VSift` engine.
///
/// One engine serves any number of sequential or concurrent operations; it
/// holds configuration and ports only, and opens storage per operation so no
/// lock or handle outlives the call that needed it.
pub struct Engine {
    config: EngineConfig,
    ports: EnginePorts,
}

impl Engine {
    /// Creates an engine from explicit configuration and ports.
    ///
    /// Construction performs no I/O. Locations are resolved and validated when
    /// an operation first needs them, so a host can build one engine up front
    /// and still receive each location's failure from the operation that used it.
    #[must_use]
    pub fn new(config: EngineConfig, ports: EnginePorts) -> Self {
        Self { config, ports }
    }

    /// Returns the configuration this engine was built with.
    #[must_use]
    pub const fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub(crate) fn now_unix_seconds(&self) -> Result<u64, EngineError> {
        Ok(self.ports.clock.now_unix_seconds()?)
    }

    pub(crate) fn new_session_id(&self) -> Result<SessionId, EngineError> {
        Ok(self.ports.identifiers.session_id()?)
    }

    pub(crate) fn new_operation_id(&self) -> Result<OperationId, EngineError> {
        Ok(self.ports.identifiers.operation_id()?)
    }

    pub(crate) fn session_root_path(&self) -> Result<PathBuf, EngineError> {
        match &self.config.session_root {
            SessionRootLocation::Explicit(path) if path.is_absolute() => Ok(path.clone()),
            SessionRootLocation::Explicit(_) => Err(SessionRootError::NotAbsolute.into()),
            SessionRootLocation::PlatformDefault => platform_session_root()
                .ok_or_else(|| SessionRootError::PlatformDefaultUnavailable.into()),
        }
    }

    pub(crate) fn open_session_store(
        root: &Path,
        provisioning: SessionRootProvisioning,
    ) -> Result<Option<FilesystemSessionStore>, EngineError> {
        open_session_root(root, provisioning)
            .map_err(|error| EngineError::SessionRoot(SessionRootError::from(error)))
    }

    pub(crate) fn user_configuration(&self) -> Result<UserDependencyConfigStore, EngineError> {
        let store = match &self.config.user_configuration {
            UserConfigurationLocation::PlatformDefault => {
                UserDependencyConfigStore::default_location()
            }
            UserConfigurationLocation::Explicit(path) => {
                UserDependencyConfigStore::at(path.clone())
            }
        };
        Ok(store?)
    }
}

/// Resolves a caller-selected path against the process working directory.
///
/// A command-line host passes paths exactly as the user typed them, so a
/// relative path means "relative to where the command ran". Library hosts
/// should pass absolute paths, which are returned unchanged.
pub(crate) fn absolute_selection(path: &Path) -> Result<PathBuf, EngineError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        env::current_dir()
            .map(|working_directory| working_directory.join(path))
            .map_err(|_| EngineError::WorkingDirectoryUnavailable)
    }
}
