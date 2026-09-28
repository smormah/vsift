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
