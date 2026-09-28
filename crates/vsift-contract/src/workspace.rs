//! The result of `session init-workspace`: `workspace-data` v1 (P11, ADR 0021
//! and maintainer decisions D1 and D2).
//!
//! A worker workspace is created explicitly, once, with an operator policy
//! that is immutable afterwards: its durability, its admission capacity and
//! how long its sessions live. The result states that policy and whether the
//! call created the workspace or found it already initialised with the same
//! policy; it never names the workspace's path.

use std::{error::Error, fmt, num::NonZeroU16};

use serde::Serialize;
use vsift_domain::PublicationGuarantee;

use crate::RequestDurability;

/// Default life of a session in a durable workspace: 168 hours (D2).
pub const DEFAULT_SESSION_RETENTION_SECONDS: u64 = 168 * 3_600;
/// Longest life of a session in a durable workspace: 720 hours (D2).
pub const MAX_SESSION_RETENTION_SECONDS: u64 = 720 * 3_600;
/// Shortest life of a session in a durable workspace: one hour.
pub const MIN_SESSION_RETENTION_SECONDS: u64 = 3_600;
/// Largest admission capacity a workspace policy may set, in weight units.
pub const MAX_ADMISSION_CAPACITY: u16 = 64;

/// Remediation when a workspace is initialised over a root that exists with
/// another policy, or over an ordinary desktop root.
pub const WORKSPACE_POLICY_MISMATCH_REMEDIATION: &str = "The session root already exists with another policy, or is an ordinary desktop root. Nothing was changed: a workspace's policy is fixed when it is created. Repeat session init-workspace with the workspace's recorded policy, or choose a new --session-root.";

/// Remediation when a durable workspace, or a durable session, is requested
/// where OS-crash durability is not qualified.
pub const DURABILITY_UNAVAILABLE_REMEDIATION: &str = "Durable publication is qualified only on Ubuntu 24.04 with a local ext4 filesystem, and this root is not on one. Nothing was created. Use --durability ephemeral here, or create the workspace on a qualified host.";

/// Remediation when a workspace is initialised without an explicit root, or
/// at the per-user session cache.
pub const WORKSPACE_ROOT_REMEDIATION: &str = "A worker workspace needs an explicit absolute --session-root that is not the per-user session cache. Nothing was created. Choose a directory for the workspace whose parent exists.";

/// Remediation when a durable session is required in an ephemeral workspace.
pub const WORKSPACE_NOT_DURABLE_REMEDIATION: &str = "The workspace's policy is ephemeral, so it cannot open a durable session; a request never changes a workspace's policy. Nothing was created. Use a workspace initialised with --durability durable.";

/// Remediation when work needs more admission weight than the whole root.
pub const ADMISSION_CAPACITY_REMEDIATION: &str = "This work needs more admission capacity than the session root has in total, so it could never start here. Nothing was run. Use a root with a larger capacity: a workspace created with more --admission-slots.";

/// Remediation when admission stayed busy for the whole wait.
pub const ADMISSION_BUSY_REMEDIATION: &str = "The session root's admission capacity stayed in use by other work for the whole wait. Nothing was run. Retry after retry_after_ms.";

/// Remediation when a worker request runs outside a worker workspace (P11 PR 3).
pub const WORKER_WORKSPACE_REQUIRED_REMEDIATION: &str = "Worker requests run only in a worker workspace. Nothing was run. Create one with session init-workspace and pass it as --session-root.";

/// Remediation when a retain step has no usable bundle root (P11 PR 3).
pub const BUNDLE_ROOT_REQUIRED_REMEDIATION: &str = "A retain step writes its bundle below --bundle-root, which must be an absolute, existing directory. Nothing was run.";

/// Remediation when the input root cannot be used (P11 PR 3).
pub const INPUT_ROOT_REMEDIATION: &str = "--input-root must be an absolute, existing, local directory; the request names its files relative to it. Nothing was run.";

/// Remediation when a path the request names does not exist (P11 PR 3).
pub const INPUT_NOT_FOUND_REMEDIATION: &str = "A file the request names does not exist inside --input-root. Nothing was run. Name an existing file relative to the input root.";

/// Remediation when a path the request names is not a single regular file (P11 PR 3).
pub const INPUT_NOT_REGULAR_FILE_REMEDIATION: &str = "A path the request names inside --input-root is a directory, a special file or a file with more than one hard link, and is refused. Name a single regular file.";

/// Remediation when a file the request names cannot be read (P11 PR 3).
pub const INPUT_UNREADABLE_REMEDIATION: &str = "A file the request names inside --input-root could not be opened or read. Nothing was changed. Check its permissions and deliver the request again.";

/// Remediation when a request's session is not in the workspace (P11 PR 3).
pub const REQUEST_SESSION_REMEDIATION: &str = "The request's session_id is not a session of this workspace. Nothing was run. Name a session the workspace holds, or ingest a source instead.";

/// Remediation when a retain step's bundle directory is taken (P11 PR 3).
pub const BUNDLE_MISMATCH_REMEDIATION: &str = "The bundle directory the retain step names already exists and is not this request's bundle. Nothing was retained. Choose another bundle_name, or remove the directory yourself if it is an incomplete bundle you do not need.";

/// Remediation when the same request runs in another process (P11 PR 3).
pub const REQUEST_BUSY_REMEDIATION: &str = "The same operation_id is running in another process. Nothing was changed. Retry after retry_after_ms: the retry continues the request or returns its result.";

/// Remediation when an operation id is bound to another request (P11 PR 3).
pub const REQUEST_CONFLICT_REMEDIATION: &str = "This operation_id was used for a different request (another request digest), whose record is kept. Nothing was changed. Use a new operation_id for a new request.";

/// Remediation when a request's deadline ends it (P11 PR 3).
pub const REQUEST_DEADLINE_REMEDIATION: &str = "The request's deadline passed, or too little of it was left to start the next step. Finished steps are kept. Deliver the same request again, with a longer deadline_ms if needed: it continues from its first unfinished step.";

/// Remediation when a shutdown stops a request (P11 PR 3).
pub const REQUEST_STOPPED_REMEDIATION: &str = "A shutdown stopped the request before it finished. Finished steps are kept. Deliver the same request again: it continues from its first unfinished step.";

/// Remediation when the job request file cannot be read (P11 PR 3).
pub const REQUEST_FILE_REMEDIATION: &str = "Name a readable job request file with --request; it holds one job-request v1 object of at most 64 KiB.";

/// Remediation when strict worker isolation cannot be attested.
pub const ISOLATION_UNAVAILABLE_REMEDIATION: &str = "Strict worker isolation needs a Linux cgroup v2 with finite CPU, memory and process limits, a read-only root filesystem and no network interface but loopback, and this host does not attest them all. Nothing was run. Run the worker in such a container, or without strict isolation.";

/// Whether `session init-workspace` created the workspace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceInitOutcome {
    /// The workspace was created with the requested policy.
    Created,
    /// The workspace already existed with exactly the requested policy.
    AlreadyInitialized,
}

impl WorkspaceInitOutcome {
    /// Every outcome, in declaration order.
    pub const ALL: [Self; 2] = [Self::Created, Self::AlreadyInitialized];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::AlreadyInitialized => "already_initialized",
        }
    }
}

/// A workspace policy outside its bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspacePolicyError {
    /// An admission capacity above [`MAX_ADMISSION_CAPACITY`].
    AdmissionCapacity,
    /// A session life outside one hour to 720 hours.
    SessionRetention,
}

impl fmt::Display for WorkspacePolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AdmissionCapacity => "the admission capacity is out of range",
            Self::SessionRetention => "the session retention is out of range",
        })
    }
}

impl Error for WorkspacePolicyError {}

/// `workspace-data` v1.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkspaceData {
    profile: &'static str,
    durability: &'static str,
    publication: &'static str,
    admission_capacity: u16,
    session_retention_seconds: u64,
    outcome: &'static str,
}

impl WorkspaceData {
    /// Presents a workspace's policy. The publication guarantee is the one
    /// its durability gives on this host: a durable workspace exists only
    /// where OS-crash durability is qualified.
    ///
    /// # Errors
    ///
    /// [`WorkspacePolicyError`] for a policy outside its bounds.
    pub fn new(
        durability: RequestDurability,
        admission_capacity: NonZeroU16,
        session_retention_seconds: u64,
        outcome: WorkspaceInitOutcome,
    ) -> Result<Self, WorkspacePolicyError> {
        if admission_capacity.get() > MAX_ADMISSION_CAPACITY {
            return Err(WorkspacePolicyError::AdmissionCapacity);
        }
        if !(MIN_SESSION_RETENTION_SECONDS..=MAX_SESSION_RETENTION_SECONDS)
            .contains(&session_retention_seconds)
        {
            return Err(WorkspacePolicyError::SessionRetention);
        }
        let publication = match durability {
            RequestDurability::Durable => PublicationGuarantee::OsCrashDurable,
            RequestDurability::Ephemeral => PublicationGuarantee::ProcessCrashConsistent,
        };
        Ok(Self {
            profile: "durable_workspace",
            durability: durability.identifier(),
            publication: publication.identifier(),
            admission_capacity: admission_capacity.get(),
            session_retention_seconds,
            outcome: outcome.identifier(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::{
        DEFAULT_SESSION_RETENTION_SECONDS, MAX_ADMISSION_CAPACITY, MAX_SESSION_RETENTION_SECONDS,
        WorkspaceData, WorkspaceInitOutcome, WorkspacePolicyError,
    };
    use crate::RequestDurability;

    #[test]
    fn the_policy_is_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let capacity = NonZeroU16::new(8).ok_or("zero")?;
        let data = WorkspaceData::new(
            RequestDurability::Durable,
            capacity,
            DEFAULT_SESSION_RETENTION_SECONDS,
            WorkspaceInitOutcome::Created,
        )?;
        let value = serde_json::to_value(&data)?;
        assert_eq!(value["publication"], "os_crash_durable");
        assert_eq!(value["session_retention_seconds"], 604_800);
        for retention in [0, 3_599, MAX_SESSION_RETENTION_SECONDS + 1] {
            assert_eq!(
                WorkspaceData::new(
                    RequestDurability::Ephemeral,
                    capacity,
                    retention,
                    WorkspaceInitOutcome::Created
                ),
                Err(WorkspacePolicyError::SessionRetention)
            );
        }
        assert_eq!(
            WorkspaceData::new(
                RequestDurability::Ephemeral,
                NonZeroU16::new(MAX_ADMISSION_CAPACITY + 1).ok_or("zero")?,
                MAX_SESSION_RETENTION_SECONDS,
                WorkspaceInitOutcome::AlreadyInitialized
            ),
            Err(WorkspacePolicyError::AdmissionCapacity)
        );
        Ok(())
    }
}
