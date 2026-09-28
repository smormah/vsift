//! Worker workspaces: explicitly initialised session roots with an operator
//! policy (P11, ADR 0021 section 3 and maintainer decisions D1, D2).
//!
//! An ordinary session root is created on first use with desktop rules. A
//! worker workspace is created only by [`Engine::init_workspace`], at an
//! explicit absolute root that is not the per-user cache, with a policy
//! recorded once in its marker: how its sessions publish (durable only where
//! OS-crash durability is qualified), how much work the root admits at once,
//! and how long its sessions live. Nothing becomes a workspace by accident,
//! and nothing changes a workspace's policy afterwards.

use std::path::{Path, PathBuf};

use vsift_domain::{DurabilityRequirement, PublicationGuarantee, WorkspacePolicy};
use vsift_infrastructure::{
    FilesystemSessionStore, SessionRootProvisioning, SessionStoreOpenError, platform_session_root,
};

use crate::{
    engine::{Engine, SessionRootLocation},
    error::{EngineError, SessionRootError},
};

/// A request to create a worker workspace at the engine's session root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceInitRequest {
    /// The operator's immutable policy.
    pub policy: WorkspacePolicy,
}

/// Whether [`Engine::init_workspace`] created the workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceInitOutcome {
    /// The workspace was created with the requested policy.
    Created,
    /// The workspace already existed with exactly the requested policy;
    /// nothing was changed.
    AlreadyInitialized,
}

/// An initialised worker workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceInit {
    policy: WorkspacePolicy,
    publication: PublicationGuarantee,
    outcome: WorkspaceInitOutcome,
}

impl WorkspaceInit {
    /// The workspace's policy.
    #[must_use]
    pub const fn policy(&self) -> WorkspacePolicy {
        self.policy
    }

    /// The guarantee every session of the workspace publishes with.
    #[must_use]
    pub const fn publication(&self) -> PublicationGuarantee {
        self.publication
    }

    /// Whether the call created the workspace.
    #[must_use]
    pub const fn outcome(&self) -> WorkspaceInitOutcome {
        self.outcome
    }
}

impl Engine {
    /// Creates a worker workspace at the engine's session root, or confirms
    /// that one exists with exactly `request.policy`.
    ///
    /// The root must be an explicit absolute path that is not the platform's
    /// per-user session cache, whose parent already exists. A durable policy
    /// is accepted only where OS-crash durability is qualified (Ubuntu 24.04
    /// on local ext4): anywhere else the call fails before anything is
    /// created. An existing root is never converted: a desktop root or a
    /// workspace with another policy is refused.
    ///
    /// # Errors
    ///
    /// [`SessionRootError::WorkspaceRootNotExplicit`] for the platform cache
    /// or no explicit root, [`SessionRootError::NotAbsolute`] for a relative
    /// one, [`SessionRootError::DurabilityUnavailable`] for a durable policy
    /// on an unqualified host, [`SessionRootError::WorkspacePolicyMismatch`]
    /// for an existing root with another policy, and the root's own open or
    /// provisioning failures.
    pub fn init_workspace(
        &self,
        request: WorkspaceInitRequest,
    ) -> Result<WorkspaceInit, EngineError> {
        let root = self.workspace_root()?;
        let policy = request.policy;
        if root.exists() {
            return self.confirm_workspace(&root, policy);
        }
        let parent = root.parent().ok_or(SessionRootError::WithoutParent)?;
        if !parent.is_dir() {
            return Err(SessionRootError::ParentUnavailable.into());
        }
        match FilesystemSessionStore::provision_workspace(&root, policy) {
            Ok(store) => Ok(WorkspaceInit {
                policy,
                publication: publication_of(&store, policy)?,
                outcome: WorkspaceInitOutcome::Created,
            }),
            // Another initialisation won the race: its policy decides.
            Err(SessionStoreOpenError::RootAlreadyExists) => self.confirm_workspace(&root, policy),
            Err(error) => Err(SessionRootError::from(error).into()),
        }
    }

    /// The explicit, absolute root a workspace may live at.
    fn workspace_root(&self) -> Result<PathBuf, EngineError> {
        let SessionRootLocation::Explicit(root) = &self.config().session_root else {
            return Err(SessionRootError::WorkspaceRootNotExplicit.into());
        };
        if !root.is_absolute() {
            return Err(SessionRootError::NotAbsolute.into());
        }
        if platform_session_root().is_some_and(|cache| same_location(&cache, root)) {
            return Err(SessionRootError::WorkspaceRootNotExplicit.into());
        }
        Ok(root.clone())
    }

    /// Answers an initialisation that found the root present.
    fn confirm_workspace(
        &self,
        root: &Path,
        policy: WorkspacePolicy,
    ) -> Result<WorkspaceInit, EngineError> {
        let store = self
            .open_session_store(root, SessionRootProvisioning::ExistingOnly)?
            .ok_or(EngineError::SessionRoot(SessionRootError::Missing))?;
        if store.workspace_policy() != Some(policy) {
            return Err(SessionRootError::WorkspacePolicyMismatch.into());
        }
        Ok(WorkspaceInit {
            policy,
            publication: publication_of(&store, policy)?,
            outcome: WorkspaceInitOutcome::AlreadyInitialized,
        })
    }
}

/// The guarantee a workspace's sessions get on this host. A durable
/// workspace found on a host that no longer qualifies (a disk moved to
/// another machine) fails closed rather than reporting less than its policy.
fn publication_of(
    store: &FilesystemSessionStore,
    policy: WorkspacePolicy,
) -> Result<PublicationGuarantee, EngineError> {
    match policy.durability() {
        DurabilityRequirement::Ephemeral => Ok(PublicationGuarantee::ProcessCrashConsistent),
        DurabilityRequirement::Durable if store.offers_os_crash_durability() => {
            Ok(PublicationGuarantee::OsCrashDurable)
        }
        DurabilityRequirement::Durable => Err(SessionRootError::DurabilityUnavailable.into()),
    }
}

/// Whether two paths name the same location: equal as written, or, when
/// both exist, equal once canonicalised.
fn same_location(first: &Path, second: &Path) -> bool {
    first == second
        || matches!(
            (std::fs::canonicalize(first), std::fs::canonicalize(second)),
            (Ok(first), Ok(second)) if first == second
        )
}
