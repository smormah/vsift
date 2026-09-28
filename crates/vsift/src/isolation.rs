//! The isolation a host asks for, and the strict Linux worker attestation
//! behind it (P11, ADR 0021 section 8; ADR 0010's strict worker profile).
//!
//! A host never simply claims strict isolation: it asks for
//! [`IsolationProfile::StrictLinux`], and [`attest_host_isolation`] returns
//! [`HostIsolation::StrictLinux`] only when the kernel attests a cgroup v2
//! with finite CPU, memory and PID limits, a read-only root and no network
//! but loopback. Otherwise the host gets `ISOLATION_UNAVAILABLE` with every
//! missing control named, before any work runs. The limits are the host's;
//! `VSift` reports them, it does not set or enforce them.

use std::fmt;

use vsift_infrastructure::{AttestationGap, attest_strict_linux_host};

use crate::{engine::HostIsolation, error::EngineError};

/// The isolation a host asks the engine to run under.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IsolationProfile {
    /// Per-process containment only: the desktop default, which claims no
    /// strict boundary.
    ProcessOnly,
    /// The strict Linux worker boundary, accepted only when attested.
    StrictLinux,
}

/// One strict worker control the host could not attest.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum IsolationGap {
    /// The host is not Linux.
    NotLinux,
    /// The process is not in a cgroup v2 (unified) hierarchy only.
    NoCgroupV2,
    /// No finite CPU limit bounds the process's cgroup.
    CpuUnlimited,
    /// No finite memory limit bounds the process's cgroup.
    MemoryUnlimited,
    /// No finite process-count limit bounds the process's cgroup.
    PidsUnlimited,
    /// The root filesystem is writable.
    RootWritable,
    /// A network interface other than loopback is present.
    NetworkReachable,
}

impl IsolationGap {
    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::NotLinux => "not_linux",
            Self::NoCgroupV2 => "no_cgroup_v2",
            Self::CpuUnlimited => "cpu_unlimited",
            Self::MemoryUnlimited => "memory_unlimited",
            Self::PidsUnlimited => "pids_unlimited",
            Self::RootWritable => "root_writable",
            Self::NetworkReachable => "network_reachable",
        }
    }

    const fn from_attestation(gap: AttestationGap) -> Self {
        match gap {
            AttestationGap::NotLinux => Self::NotLinux,
            AttestationGap::NoCgroupV2 => Self::NoCgroupV2,
            AttestationGap::CpuUnlimited => Self::CpuUnlimited,
            AttestationGap::MemoryUnlimited => Self::MemoryUnlimited,
            AttestationGap::PidsUnlimited => Self::PidsUnlimited,
            AttestationGap::RootWritable => Self::RootWritable,
            AttestationGap::NetworkReachable => Self::NetworkReachable,
        }
    }
}

/// Every strict worker control a host could not attest, in report order;
/// never empty.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IsolationGaps(Vec<IsolationGap>);

impl IsolationGaps {
    fn from_attestation(gaps: &[AttestationGap]) -> Self {
        let mut mirrored: Vec<IsolationGap> = gaps
            .iter()
            .copied()
            .map(IsolationGap::from_attestation)
            .collect();
        if mirrored.is_empty() {
            // A refused attestation always names a gap; keep the type's
            // promise even if an adapter ever returned none.
            mirrored.push(IsolationGap::NoCgroupV2);
        }
        Self(mirrored)
    }

    /// The missing controls.
    #[must_use]
    pub fn gaps(&self) -> &[IsolationGap] {
        &self.0
    }
}

impl fmt::Display for IsolationGaps {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for gap in &self.0 {
            if !first {
                formatter.write_str(", ")?;
            }
            formatter.write_str(gap.identifier())?;
            first = false;
        }
        Ok(())
    }
}

/// The isolation the engine may run under for `profile`: process-only as
/// asked, or strict Linux once the kernel attests every control.
///
/// A host calls this before it builds its engine and before it reads any
/// request, so a host that cannot provide the boundary stops before any
/// work.
///
/// # Errors
///
/// [`EngineError::IsolationUnavailable`] naming every missing control.
pub fn attest_host_isolation(profile: IsolationProfile) -> Result<HostIsolation, EngineError> {
    match profile {
        IsolationProfile::ProcessOnly => Ok(HostIsolation::ProcessOnly),
        IsolationProfile::StrictLinux => strict_linux(),
    }
}

#[cfg(target_os = "linux")]
fn strict_linux() -> Result<HostIsolation, EngineError> {
    attest_strict_linux_host()
        .map(|_| HostIsolation::StrictLinux)
        .map_err(|gaps| EngineError::IsolationUnavailable(IsolationGaps::from_attestation(&gaps)))
}

/// Off Linux the attestation always names `not_linux`; it still runs, so
/// every gap is reported the same way on every platform.
#[cfg(not(target_os = "linux"))]
fn strict_linux() -> Result<HostIsolation, EngineError> {
    let gaps = attest_strict_linux_host()
        .err()
        .unwrap_or_else(|| vec![AttestationGap::NotLinux]);
    Err(EngineError::IsolationUnavailable(
        IsolationGaps::from_attestation(&gaps),
    ))
}

#[cfg(test)]
mod tests {
    use vsift_domain::FailureCode;

    use super::{IsolationGap, IsolationProfile, attest_host_isolation};
    use crate::{engine::HostIsolation, error::EngineError};

    #[test]
    fn process_only_is_never_attested_and_strict_fails_closed_off_a_strict_host()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            attest_host_isolation(IsolationProfile::ProcessOnly),
            Ok(HostIsolation::ProcessOnly)
        );
        match attest_host_isolation(IsolationProfile::StrictLinux) {
            Ok(isolation) => {
                // Only inside a strict Linux container.
                assert_eq!(std::env::consts::OS, "linux");
                assert_ne!(isolation, HostIsolation::ProcessOnly);
            }
            Err(EngineError::IsolationUnavailable(gaps)) => {
                assert_eq!(
                    EngineError::IsolationUnavailable(gaps.clone()).failure_code(),
                    FailureCode::IsolationUnavailable
                );
                assert!(!gaps.gaps().is_empty());
                assert_eq!(
                    gaps.gaps().contains(&IsolationGap::NotLinux),
                    std::env::consts::OS != "linux"
                );
            }
            Err(other) => return Err(format!("unexpected failure: {other}").into()),
        }
        Ok(())
    }
}
