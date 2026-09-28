//! Strict Linux worker attestation (P11, ADR 0021 section 8; SEC-T01
//! groundwork, known limit L-004).
//!
//! A worker host may claim strict isolation only when the kernel says the
//! process runs inside host-enforced limits. [`attest_strict_linux_host`]
//! reads, each with a byte bound:
//!
//! - `/proc/self/cgroup`: the process must be in a cgroup v2 (unified)
//!   hierarchy only, at a path of at most [`MAX_CGROUP_DEPTH`] components;
//! - `cpu.max`, `memory.max` and `pids.max` of that cgroup and of each of its
//!   ancestors up to `/sys/fs/cgroup`: each resource must be finite at some
//!   level, because a limit on an ancestor bounds every descendant;
//! - `/proc/self/mountinfo`: the mount the process sees at `/` must be
//!   read-only in its own mount options;
//! - `/proc/self/net/dev`: the process's network namespace must hold no
//!   interface but loopback (`lo`).
//!
//! The limits are the host's (a container runtime or a supervisor's cgroup);
//! `VSift` never sets or claims to enforce them itself, and reports them as
//! coming from the host cgroup. The parsers and the decision table are
//! platform-neutral and tested on fixture files everywhere; only the reading
//! of the real files is Linux-only, and the real attestation is exercised in
//! a container by P11's SEC-T01 job. Every parser is public so the fuzz
//! harness reaches it through the same surface.

use std::{error::Error, fmt};

use crate::durable_profile::RootMountAccess;

/// Largest `/proc/self/cgroup` read: 64 KiB, far above any real membership list.
pub const MAX_PROC_CGROUP_BYTES: usize = 64 * 1024;
/// Largest cgroup interface file read (`cpu.max`, `memory.max`, `pids.max`).
pub const MAX_CGROUP_FILE_BYTES: usize = 4 * 1024;
/// Largest `/proc/self/net/dev` read: 64 KiB, several hundred interfaces.
pub const MAX_NET_DEV_BYTES: usize = 64 * 1024;
/// Deepest cgroup path accepted, in components.
pub const MAX_CGROUP_DEPTH: usize = 32;
/// Longest cgroup path component accepted, in bytes (a Linux file name).
const MAX_CGROUP_COMPONENT_BYTES: usize = 255;

/// Why a kernel file was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttestationParseError {
    /// Larger than its bound.
    TooLarge,
    /// Not UTF-8 text.
    NotUtf8,
    /// Not the documented shape.
    Malformed,
}

impl fmt::Display for AttestationParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "the kernel file is too large",
            Self::NotUtf8 => "the kernel file is not UTF-8",
            Self::Malformed => "the kernel file is malformed",
        })
    }
}

impl Error for AttestationParseError {}

/// Where `/proc/self/cgroup` places the process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CgroupMembership {
    /// Only the unified (v2) hierarchy, at this path's components below the
    /// cgroup root (none for the root itself).
    Unified(Vec<String>),
    /// A v1 or hybrid hierarchy: no single cgroup v2 path bounds the process.
    NotUnified,
}

/// Parses `/proc/self/cgroup`.
///
/// A cgroup v2 only host lists exactly one line, `0::<path>`. Any other
/// line (a v1 controller hierarchy) makes the membership
/// [`CgroupMembership::NotUnified`]. The path must be absolute, and a
/// component that is empty, `.` or `..` (a process outside its cgroup
/// namespace's root) is refused, as is a path deeper than
/// [`MAX_CGROUP_DEPTH`].
///
/// # Errors
///
/// A file over [`MAX_PROC_CGROUP_BYTES`], not UTF-8, or not the documented
/// shape.
pub fn parse_proc_cgroup(bytes: &[u8]) -> Result<CgroupMembership, AttestationParseError> {
    let text = bounded_text(bytes, MAX_PROC_CGROUP_BYTES)?;
    let mut unified = None;
    let mut other = false;
    for line in text.lines() {
        let mut fields = line.splitn(3, ':');
        let (Some(id), Some(controllers), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Err(AttestationParseError::Malformed);
        };
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(AttestationParseError::Malformed);
        }
        if id == "0" && controllers.is_empty() {
            if unified.is_some() {
                return Err(AttestationParseError::Malformed);
            }
            unified = Some(cgroup_components(path)?);
        } else {
            other = true;
        }
    }
    match unified {
        Some(components) if !other => Ok(CgroupMembership::Unified(components)),
        Some(_) => Ok(CgroupMembership::NotUnified),
        None if other => Ok(CgroupMembership::NotUnified),
        None => Err(AttestationParseError::Malformed),
    }
}

/// The components of an absolute cgroup path.
fn cgroup_components(path: &str) -> Result<Vec<String>, AttestationParseError> {
    let relative = path
        .strip_prefix('/')
        .ok_or(AttestationParseError::Malformed)?;
    if relative.is_empty() {
        return Ok(Vec::new());
    }
    let components: Vec<String> = relative.split('/').map(str::to_owned).collect();
    if components.len() > MAX_CGROUP_DEPTH
        || components.iter().any(|component| {
            component.is_empty()
                || component == "."
                || component == ".."
                || component.len() > MAX_CGROUP_COMPONENT_BYTES
                || component.contains('\0')
        })
    {
        return Err(AttestationParseError::Malformed);
    }
    Ok(components)
}

/// One cgroup resource limit as its interface file states it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CgroupLimit {
    /// `max`: no limit at this level.
    Unlimited,
    /// A finite limit: bytes for `memory.max`, processes for `pids.max`, and
    /// for `cpu.max` the quota in microseconds per period.
    Finite(u64),
}

/// Parses `memory.max` or `pids.max`: `max` or a decimal count, then a line
/// feed.
///
/// # Errors
///
/// A file over [`MAX_CGROUP_FILE_BYTES`], not UTF-8, or not that shape.
pub fn parse_cgroup_limit(bytes: &[u8]) -> Result<CgroupLimit, AttestationParseError> {
    let text = bounded_text(bytes, MAX_CGROUP_FILE_BYTES)?;
    parse_limit_value(single_line(text)?)
}

/// Parses `cpu.max`: `<quota> <period>`, where the quota is `max` or a
/// decimal count of microseconds and the period a positive decimal count.
///
/// # Errors
///
/// A file over [`MAX_CGROUP_FILE_BYTES`], not UTF-8, or not that shape.
pub fn parse_cpu_max(bytes: &[u8]) -> Result<CgroupLimit, AttestationParseError> {
    let text = bounded_text(bytes, MAX_CGROUP_FILE_BYTES)?;
    let (quota, period) = single_line(text)?
        .split_once(' ')
        .ok_or(AttestationParseError::Malformed)?;
    match parse_limit_value(period)? {
        CgroupLimit::Finite(period) if period > 0 => parse_limit_value(quota),
        CgroupLimit::Finite(_) | CgroupLimit::Unlimited => Err(AttestationParseError::Malformed),
    }
}

fn parse_limit_value(value: &str) -> Result<CgroupLimit, AttestationParseError> {
    if value == "max" {
        return Ok(CgroupLimit::Unlimited);
    }
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AttestationParseError::Malformed);
    }
    value
        .parse()
        .map(CgroupLimit::Finite)
        .map_err(|_| AttestationParseError::Malformed)
}

/// The one line of a cgroup interface file, without its line feed.
fn single_line(text: &str) -> Result<&str, AttestationParseError> {
    let line = text
        .strip_suffix('\n')
        .ok_or(AttestationParseError::Malformed)?;
    if line.contains('\n') {
        return Err(AttestationParseError::Malformed);
    }
    Ok(line)
}

/// The network interfaces a process's namespace holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkInterfaces {
    /// Only the loopback interface `lo`.
    LoopbackOnly,
    /// At least one interface other than `lo`.
    Other,
    /// No interface at all (not even `lo`).
    None,
}

/// Parses `/proc/<pid>/net/dev`: two header lines, then one line per
/// interface, `<name>:` followed by sixteen decimal counters.
///
/// # Errors
///
/// A file over [`MAX_NET_DEV_BYTES`], not UTF-8, or not that shape.
pub fn parse_net_dev(bytes: &[u8]) -> Result<NetworkInterfaces, AttestationParseError> {
    let text = bounded_text(bytes, MAX_NET_DEV_BYTES)?;
    let mut lines = text.lines();
    let (Some(first), Some(second)) = (lines.next(), lines.next()) else {
        return Err(AttestationParseError::Malformed);
    };
    if !first.starts_with("Inter-|") || !second.trim_start().starts_with("face |") {
        return Err(AttestationParseError::Malformed);
    }
    let mut loopback = false;
    let mut other = false;
    for line in lines {
        let (name, counters) = line
            .split_once(':')
            .ok_or(AttestationParseError::Malformed)?;
        let name = name.trim_start();
        let counters: Vec<&str> = counters.split_whitespace().collect();
        if name.is_empty()
            || name.contains(char::is_whitespace)
            || counters.len() != 16
            || counters
                .iter()
                .any(|counter| counter.is_empty() || !counter.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(AttestationParseError::Malformed);
        }
        if name == "lo" {
            loopback = true;
        } else {
            other = true;
        }
    }
    Ok(match (loopback, other) {
        (_, true) => NetworkInterfaces::Other,
        (true, false) => NetworkInterfaces::LoopbackOnly,
        (false, false) => NetworkInterfaces::None,
    })
}

fn bounded_text(bytes: &[u8], limit: usize) -> Result<&str, AttestationParseError> {
    if bytes.len() > limit {
        return Err(AttestationParseError::TooLarge);
    }
    std::str::from_utf8(bytes).map_err(|_| AttestationParseError::NotUtf8)
}

/// Why a host is not a strict Linux worker. Every gap found is reported, in
/// this order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AttestationGap {
    /// The process is not running on Linux.
    NotLinux,
    /// The process is not in a cgroup v2 (unified) hierarchy only, or its
    /// membership could not be read.
    NoCgroupV2,
    /// No level of the cgroup path sets a finite `cpu.max`.
    CpuUnlimited,
    /// No level of the cgroup path sets a finite `memory.max`.
    MemoryUnlimited,
    /// No level of the cgroup path sets a finite `pids.max`.
    PidsUnlimited,
    /// The root filesystem is writable, or its mount could not be read.
    RootWritable,
    /// A network interface other than loopback is present, or the list
    /// could not be read.
    NetworkReachable,
}

impl AttestationGap {
    /// Every gap, in report order.
    pub const ALL: [Self; 7] = [
        Self::NotLinux,
        Self::NoCgroupV2,
        Self::CpuUnlimited,
        Self::MemoryUnlimited,
        Self::PidsUnlimited,
        Self::RootWritable,
        Self::NetworkReachable,
    ];

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
}

/// What was read from the kernel, each value `None` when it could not be
/// read or parsed. `cpu`, `memory` and `pids` list the limit at each level
/// of the cgroup path, leaf first; a level whose file does not exist (the
/// root cgroup has none) is [`CgroupLimit::Unlimited`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HostObservation {
    /// Whether the process runs on Linux at all.
    pub linux: bool,
    /// `/proc/self/cgroup`.
    pub cgroup: Option<CgroupMembership>,
    /// `cpu.max` at each level.
    pub cpu: Option<Vec<CgroupLimit>>,
    /// `memory.max` at each level.
    pub memory: Option<Vec<CgroupLimit>>,
    /// `pids.max` at each level.
    pub pids: Option<Vec<CgroupLimit>>,
    /// The mount at `/`.
    pub root: Option<RootMountAccess>,
    /// The namespace's interfaces.
    pub network: Option<NetworkInterfaces>,
}

/// A host whose kernel attested every strict worker control. Only
/// [`decide_strict_linux`] makes one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StrictLinuxAttestation {
    cpu_quota_micros: u64,
    memory_bytes: u64,
    pids: u64,
}

impl StrictLinuxAttestation {
    /// The tightest CPU quota on the cgroup path, in microseconds per period.
    #[must_use]
    pub const fn cpu_quota_micros(&self) -> u64 {
        self.cpu_quota_micros
    }

    /// The tightest memory limit on the cgroup path, in bytes.
    #[must_use]
    pub const fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }

    /// The tightest process limit on the cgroup path.
    #[must_use]
    pub const fn pids(&self) -> u64 {
        self.pids
    }
}

/// The decision table: a host is a strict Linux worker only when it is
/// Linux, in a cgroup v2 hierarchy with a finite CPU, memory and PID limit
/// somewhere on its path, on a read-only root, with loopback as its only
/// network interface. Anything unread is a gap, never a pass.
///
/// # Errors
///
/// Every gap found, in [`AttestationGap::ALL`] order; never empty.
pub fn decide_strict_linux(
    observation: &HostObservation,
) -> Result<StrictLinuxAttestation, Vec<AttestationGap>> {
    let mut gaps = Vec::new();
    if !observation.linux {
        gaps.push(AttestationGap::NotLinux);
    }
    if !matches!(observation.cgroup, Some(CgroupMembership::Unified(_))) {
        gaps.push(AttestationGap::NoCgroupV2);
    }
    let cpu = tightest(observation.cpu.as_deref());
    let memory = tightest(observation.memory.as_deref());
    let pids = tightest(observation.pids.as_deref());
    if cpu.is_none() {
        gaps.push(AttestationGap::CpuUnlimited);
    }
    if memory.is_none() {
        gaps.push(AttestationGap::MemoryUnlimited);
    }
    if pids.is_none() {
        gaps.push(AttestationGap::PidsUnlimited);
    }
    if observation.root != Some(RootMountAccess::ReadOnly) {
        gaps.push(AttestationGap::RootWritable);
    }
    if observation.network != Some(NetworkInterfaces::LoopbackOnly) {
        gaps.push(AttestationGap::NetworkReachable);
    }
    match (cpu, memory, pids) {
        (Some(cpu_quota_micros), Some(memory_bytes), Some(pids)) if gaps.is_empty() => {
            Ok(StrictLinuxAttestation {
                cpu_quota_micros,
                memory_bytes,
                pids,
            })
        }
        _ => Err(gaps),
    }
}

/// The smallest finite limit on the path, if any level sets one.
fn tightest(levels: Option<&[CgroupLimit]>) -> Option<u64> {
    levels?
        .iter()
        .filter_map(|limit| match limit {
            CgroupLimit::Finite(value) => Some(*value),
            CgroupLimit::Unlimited => None,
        })
        .min()
}

/// Attests the running host as a strict Linux worker, reading the kernel's
/// view of this process (see the module documentation).
///
/// # Errors
///
/// Every [`AttestationGap`] found; on any other target only
/// [`AttestationGap::NotLinux`] and the gaps an empty observation has.
pub fn attest_strict_linux_host() -> Result<StrictLinuxAttestation, Vec<AttestationGap>> {
    decide_strict_linux(&observe_host())
}

/// Reads the kernel files, each bounded.
#[cfg(target_os = "linux")]
fn observe_host() -> HostObservation {
    let cgroup = read_bounded("/proc/self/cgroup", MAX_PROC_CGROUP_BYTES)
        .and_then(|bytes| parse_proc_cgroup(&bytes).ok());
    let levels = match &cgroup {
        Some(CgroupMembership::Unified(components)) => Some(cgroup_levels(components)),
        Some(CgroupMembership::NotUnified) | None => None,
    };
    let limits = |file: &str, parse: fn(&[u8]) -> Result<CgroupLimit, AttestationParseError>| {
        levels.as_ref().and_then(|levels| {
            levels
                .iter()
                .map(|level| read_level_limit(&level.join(file), parse))
                .collect::<Option<Vec<_>>>()
        })
    };
    HostObservation {
        linux: true,
        cpu: limits("cpu.max", parse_cpu_max),
        memory: limits("memory.max", parse_cgroup_limit),
        pids: limits("pids.max", parse_cgroup_limit),
        cgroup,
        root: read_bounded("/proc/self/mountinfo", crate::MAX_MOUNTINFO_BYTES)
            .and_then(|bytes| crate::classify_root_mount(&bytes).ok()),
        network: read_bounded("/proc/self/net/dev", MAX_NET_DEV_BYTES)
            .and_then(|bytes| parse_net_dev(&bytes).ok()),
    }
}

/// The cgroup directory of every level of the path, leaf first.
#[cfg(target_os = "linux")]
fn cgroup_levels(components: &[String]) -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new("/sys/fs/cgroup");
    (0..=components.len())
        .rev()
        .map(|depth| {
            components
                .iter()
                .take(depth)
                .fold(root.to_path_buf(), |path, component| path.join(component))
        })
        .collect()
}

/// One level's limit: a missing file (the root cgroup has none) is no limit
/// at that level; anything else unreadable or malformed fails the whole
/// resource.
#[cfg(target_os = "linux")]
fn read_level_limit(
    path: &std::path::Path,
    parse: fn(&[u8]) -> Result<CgroupLimit, AttestationParseError>,
) -> Option<CgroupLimit> {
    use std::io::Read as _;
    match std::fs::File::open(path) {
        Ok(file) => {
            let mut bytes = Vec::new();
            let limit = u64::try_from(MAX_CGROUP_FILE_BYTES).unwrap_or(u64::MAX);
            file.take(limit.saturating_add(1))
                .read_to_end(&mut bytes)
                .ok()?;
            parse(&bytes).ok()
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(CgroupLimit::Unlimited),
        Err(_) => None,
    }
}

/// Reads at most `limit` bytes and one more of `path`, so an oversized file
/// is refused by its parser rather than truncated.
#[cfg(target_os = "linux")]
fn read_bounded(path: &str, limit: usize) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    let limit = u64::try_from(limit).unwrap_or(u64::MAX);
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

/// No other target can be a strict Linux worker.
#[cfg(not(target_os = "linux"))]
fn observe_host() -> HostObservation {
    HostObservation::default()
}

#[cfg(test)]
mod tests {
    use super::{
        AttestationGap, AttestationParseError, CgroupLimit, CgroupMembership, HostObservation,
        MAX_CGROUP_DEPTH, MAX_CGROUP_FILE_BYTES, MAX_NET_DEV_BYTES, MAX_PROC_CGROUP_BYTES,
        NetworkInterfaces, decide_strict_linux, parse_cgroup_limit, parse_cpu_max, parse_net_dev,
        parse_proc_cgroup,
    };
    use crate::durable_profile::RootMountAccess;

    /// `/proc/self/net/dev` of a container started with `--network none`.
    const LOOPBACK_ONLY: &str = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo:       0       0    0    0    0     0          0         0        0       0    0    0    0     0       0          0
";

    /// `/proc/self/cgroup` of a systemd service on a cgroup v2 host (the
    /// `host_attestation` fuzz seeds quote these files verbatim).
    const UNIFIED: &str = "0::/system.slice/worker.service
";
    /// `cpu.max` of a two-CPU container.
    const CPU_MAX: &str = "200000 100000
";
    /// `memory.max` of a 512 MiB container.
    const MEMORY_MAX: &str = "536870912
";

    #[test]
    fn the_fuzz_seed_files_parse_as_their_kernel_files() {
        assert!(matches!(
            parse_proc_cgroup(UNIFIED.as_bytes()),
            Ok(CgroupMembership::Unified(_))
        ));
        assert_eq!(
            parse_cpu_max(CPU_MAX.as_bytes()),
            Ok(CgroupLimit::Finite(200_000))
        );
        assert_eq!(
            parse_cgroup_limit(MEMORY_MAX.as_bytes()),
            Ok(CgroupLimit::Finite(536_870_912))
        );
    }

    #[test]
    fn a_cgroup_v2_membership_is_one_unified_line() {
        assert_eq!(
            parse_proc_cgroup(b"0::/system.slice/worker.service\n"),
            Ok(CgroupMembership::Unified(vec![
                "system.slice".to_owned(),
                "worker.service".to_owned()
            ]))
        );
        assert_eq!(
            parse_proc_cgroup(b"0::/\n"),
            Ok(CgroupMembership::Unified(Vec::new()))
        );
        for hybrid in [
            "12:memory:/docker/abc\n0::/docker/abc\n",
            "1:name=systemd:/user.slice\n",
        ] {
            assert_eq!(
                parse_proc_cgroup(hybrid.as_bytes()),
                Ok(CgroupMembership::NotUnified),
                "{hybrid:?}"
            );
        }
        let deep = format!("0::/{}\n", vec!["a"; MAX_CGROUP_DEPTH + 1].join("/"));
        for malformed in [
            "",
            "0::relative\n",
            "0::/../../escaped\n",
            "0::/a//b\n",
            "0::/a/./b\n",
            "x::/\n",
            "0::/\n0::/again\n",
            "no colons\n",
            deep.as_str(),
        ] {
            assert_eq!(
                parse_proc_cgroup(malformed.as_bytes()),
                Err(AttestationParseError::Malformed),
                "{malformed:?}"
            );
        }
        assert_eq!(
            parse_proc_cgroup(&vec![b'0'; MAX_PROC_CGROUP_BYTES + 1]),
            Err(AttestationParseError::TooLarge)
        );
        assert_eq!(
            parse_proc_cgroup(b"0::/\xff\n"),
            Err(AttestationParseError::NotUtf8)
        );
    }

    #[test]
    fn cgroup_limits_are_max_or_a_count() {
        assert_eq!(parse_cgroup_limit(b"max\n"), Ok(CgroupLimit::Unlimited));
        assert_eq!(
            parse_cgroup_limit(b"536870912\n"),
            Ok(CgroupLimit::Finite(536_870_912))
        );
        assert_eq!(parse_cpu_max(b"max 100000\n"), Ok(CgroupLimit::Unlimited));
        assert_eq!(
            parse_cpu_max(b"200000 100000\n"),
            Ok(CgroupLimit::Finite(200_000))
        );
        for malformed in [
            &b"max"[..],
            b"-1\n",
            b"1 2\n",
            b"\n",
            b"12a\n",
            b"99999999999999999999999\n",
            b"max\nmax\n",
        ] {
            assert_eq!(
                parse_cgroup_limit(malformed),
                Err(AttestationParseError::Malformed),
                "{malformed:?}"
            );
        }
        for malformed in [
            &b"max\n"[..],
            b"100000 0\n",
            b"100000 max\n",
            b"100000  100000\n",
            b"max 100000",
        ] {
            assert_eq!(
                parse_cpu_max(malformed),
                Err(AttestationParseError::Malformed),
                "{malformed:?}"
            );
        }
        assert_eq!(
            parse_cgroup_limit(&vec![b'1'; MAX_CGROUP_FILE_BYTES + 1]),
            Err(AttestationParseError::TooLarge)
        );
    }

    #[test]
    fn only_loopback_counts_as_no_network() {
        assert_eq!(
            parse_net_dev(LOOPBACK_ONLY.as_bytes()),
            Ok(NetworkInterfaces::LoopbackOnly)
        );
        let bridged = format!("{LOOPBACK_ONLY}  eth0: 1296 16 0 0 0 0 0 0 1076 14 0 0 0 0 0 0\n");
        assert_eq!(
            parse_net_dev(bridged.as_bytes()),
            Ok(NetworkInterfaces::Other)
        );
        let headers = LOOPBACK_ONLY
            .lines()
            .take(2)
            .flat_map(|line| [line, "\n"])
            .collect::<String>();
        assert_eq!(
            parse_net_dev(headers.as_bytes()),
            Ok(NetworkInterfaces::None)
        );
        for malformed in [
            String::new(),
            "Inter-|\n".to_owned(),
            format!("{headers}    lo: 1 2 3\n"),
            format!("{headers}    lo 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16\n"),
            format!("{headers}    : 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16\n"),
            format!("{headers}    lo: 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 x\n"),
            "garbage\nmore\n".to_owned(),
        ] {
            assert_eq!(
                parse_net_dev(malformed.as_bytes()),
                Err(AttestationParseError::Malformed),
                "{malformed:?}"
            );
        }
        assert_eq!(
            parse_net_dev(&vec![b' '; MAX_NET_DEV_BYTES + 1]),
            Err(AttestationParseError::TooLarge)
        );
    }

    /// One way to take a control away from a strict host.
    type Breakage = fn(&mut HostObservation);

    fn strict() -> HostObservation {
        HostObservation {
            linux: true,
            cgroup: Some(CgroupMembership::Unified(vec!["worker".to_owned()])),
            cpu: Some(vec![CgroupLimit::Finite(100_000), CgroupLimit::Unlimited]),
            memory: Some(vec![CgroupLimit::Unlimited, CgroupLimit::Finite(1 << 30)]),
            pids: Some(vec![CgroupLimit::Finite(256), CgroupLimit::Finite(64)]),
            root: Some(RootMountAccess::ReadOnly),
            network: Some(NetworkInterfaces::LoopbackOnly),
        }
    }

    /// The decision table: every control must be attested, a limit on any
    /// level counts (the tightest is reported), and each missing control is
    /// its own gap; nothing unread ever passes.
    #[test]
    fn the_strict_decision_needs_every_control() -> Result<(), Box<dyn std::error::Error>> {
        let attested = decide_strict_linux(&strict()).map_err(|gaps| format!("{gaps:?}"))?;
        assert_eq!(attested.cpu_quota_micros(), 100_000);
        assert_eq!(attested.memory_bytes(), 1 << 30);
        assert_eq!(attested.pids(), 64);

        let cases: [(Breakage, AttestationGap); 11] = [
            (|host| host.linux = false, AttestationGap::NotLinux),
            (|host| host.cgroup = None, AttestationGap::NoCgroupV2),
            (
                |host| host.cgroup = Some(CgroupMembership::NotUnified),
                AttestationGap::NoCgroupV2,
            ),
            (
                |host| host.cpu = Some(vec![CgroupLimit::Unlimited]),
                AttestationGap::CpuUnlimited,
            ),
            (|host| host.cpu = None, AttestationGap::CpuUnlimited),
            (
                |host| host.memory = Some(Vec::new()),
                AttestationGap::MemoryUnlimited,
            ),
            (|host| host.pids = None, AttestationGap::PidsUnlimited),
            (
                |host| host.root = Some(RootMountAccess::Writable),
                AttestationGap::RootWritable,
            ),
            (
                |host| host.root = Some(RootMountAccess::NotListed),
                AttestationGap::RootWritable,
            ),
            (
                |host| host.network = Some(NetworkInterfaces::Other),
                AttestationGap::NetworkReachable,
            ),
            (|host| host.network = None, AttestationGap::NetworkReachable),
        ];
        for (break_it, gap) in cases {
            let mut host = strict();
            break_it(&mut host);
            assert_eq!(decide_strict_linux(&host), Err(vec![gap]), "{gap:?}");
        }
        assert_eq!(
            decide_strict_linux(&HostObservation::default()),
            Err(AttestationGap::ALL.to_vec())
        );
        Ok(())
    }

    /// Off Linux the real attestation reports `not_linux` and fails
    /// everything else closed; on Linux the host here (a developer machine
    /// or a hosted runner, never a strict container) must not pass either,
    /// unless the test runs inside one.
    #[test]
    fn this_host_is_attested_or_every_gap_is_named() {
        match super::attest_strict_linux_host() {
            Ok(attested) => {
                assert_eq!(std::env::consts::OS, "linux");
                assert!(attested.pids() > 0 && attested.memory_bytes() > 0);
            }
            Err(gaps) => {
                assert!(!gaps.is_empty());
                assert_eq!(
                    gaps.contains(&AttestationGap::NotLinux),
                    std::env::consts::OS != "linux"
                );
                let mut sorted = gaps.clone();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(sorted, gaps, "gaps are unique and in report order");
            }
        }
    }
}
