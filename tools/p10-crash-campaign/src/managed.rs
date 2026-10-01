//! The managed-store power-loss workload and verifier (P13 PR 7, ADR 0023
//! decision H9).
//!
//! The workload drives the private managed store directly, through the same
//! store and application use cases `setup install`, `setup rollback` and
//! `setup remove` use, with one-file stand-in versions instead of the
//! reviewed tools (a power loss is about the store's files, not about what
//! they run): it installs a new version and runs the bounded cleanup,
//! reinstalls the selected one, rolls back, removes a version or a whole
//! component, abandons a stage as a killed install would, and sweeps. Every
//! success writes a `MACK` line and a dm-log-writes mark.
//!
//! The claim it tests: a command that reported success survives a power
//! loss (every folder a command changes is flushed before it returns,
//! P13 PR 7), and whatever a power loss catches half done is detected and
//! repaired (decision H9). So at every replayed flush the verifier requires:
//!
//! - every acknowledged selection is still selected, or replaced by the
//!   selection the one command in flight at the point reported (see
//!   [`in_flight`]), and no acknowledged removal has come back;
//! - the store can be inspected;
//! - a component's lookup either refuses its selection or returns a version
//!   whose file holds exactly the bytes published under that name (checked
//!   here against the stand-in's own bytes, read without the store's code);
//! - `setup repair`'s plan names an existing command for every finding and
//!   never asks the user to delete anything by hand, and applying its
//!   commands, repeatedly as a user following repair would, leaves a
//!   healthy store;
//! - reinstalling each component's last acknowledged selection then
//!   completes and is selected.
//!
//! What a power loss undid (an acknowledged selection that reverted, a
//! removed version that reappeared) is a loss, which fails the positive run.
//! The negative control skips every folder flush and the flush of each
//! runtime file: it must lose acknowledgements, or the campaign cannot see
//! what it claims to prevent, and it must still show no damage. A selection
//! whose bytes are torn and that lookup refused is counted as well.

use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};
use vsift_application::{
    MANAGED_COMPONENTS, ManagedRemovalTarget, RepairFix, RepairStatus, clean_up_versions,
    diagnose_managed_store, remove_managed, roll_back_component,
};
use vsift_domain::{ArtifactIntegrity, ManagedVersionKey};
use vsift_infrastructure::{
    GuardedManagedStore, ManagedArtifactStore, ManagedInstallGuard, ManagedRuntimeIdentity,
    ReviewedRuntimeLayout,
};

use crate::{
    error::CampaignError,
    protocol::{mark_for, start_mark_for},
    rng::SplitMix64,
    verify::sha256_hex,
    workload::{Channel, mark, unix_nanos},
};

/// Which store a campaign step works on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub enum CampaignStore {
    /// The durable session store (P10, P11).
    #[default]
    Session,
    /// The private managed store (P13 PR 7).
    Managed,
}

/// The one file every stand-in version holds.
pub const STAND_IN_FILE: &str = "stand-in.bin";
/// Smallest stand-in file: large enough to span data blocks.
const STAND_IN_MIN_BYTES: usize = 64 * 1024;
/// Stand-in files are up to this many bytes larger.
const STAND_IN_SPREAD_BYTES: usize = 192 * 1024;
/// A user follows `setup repair` at most this many times.
const REPAIR_PASSES: usize = 4;

/// One managed command of the workload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedOperation {
    /// A new version, published and selected, then the bounded cleanup.
    Install,
    /// The selected version installed again (the idempotent publication).
    Reinstall,
    /// `setup rollback`: the recorded previous version selected again.
    Rollback,
    /// `setup remove --version` of an unselected version.
    RemoveVersion,
    /// `setup remove <component>`.
    RemoveComponent,
    /// A stage left behind, as a killed install leaves one.
    AbandonStage,
    /// `setup remove --stale-stages`.
    Sweep,
}

impl ManagedOperation {
    const ALL: [Self; 7] = [
        Self::Install,
        Self::Reinstall,
        Self::Rollback,
        Self::RemoveVersion,
        Self::RemoveComponent,
        Self::AbandonStage,
        Self::Sweep,
    ];

    /// The operation's name on the wire.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Reinstall => "reinstall",
            Self::Rollback => "rollback",
            Self::RemoveVersion => "remove-version",
            Self::RemoveComponent => "remove-component",
            Self::AbandonStage => "abandon-stage",
            Self::Sweep => "sweep",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|operation| operation.name() == text)
    }
}

impl fmt::Display for ManagedOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// An acknowledged managed command:
/// `MACK <seq> <unix_ns> <operation> <component> <selected|-> <removed|->`.
///
/// `selected` is what the component selects once the command returned;
/// `removed` the version a removal or the cleanup removed (the first one).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedAck {
    /// The workload's sequence number.
    pub seq: u64,
    /// When the acknowledgement was written, in Unix nanoseconds.
    pub unix_ns: u128,
    /// The command.
    pub operation: ManagedOperation,
    /// The component's store key.
    pub component: String,
    /// The version selected afterwards.
    pub selected: Option<String>,
    /// A version the command removed.
    pub removed: Option<String>,
}

impl ManagedAck {
    /// The acknowledgement as one protocol line.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "MACK {} {} {} {} {} {}",
            self.seq,
            self.unix_ns,
            self.operation,
            self.component,
            self.selected.as_deref().unwrap_or("-"),
            self.removed.as_deref().unwrap_or("-")
        )
    }
}

/// Every managed acknowledgement of a log, and the lines that start like
/// one but do not parse (1-based).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ManagedEvents {
    /// Acknowledged commands, in order.
    pub acks: Vec<ManagedAck>,
    /// Malformed protocol lines.
    pub malformed: Vec<usize>,
}

/// Parses the `MACK` lines of `text`; every other line is ignored.
#[must_use]
pub fn parse_managed_events(text: &str) -> ManagedEvents {
    let mut events = ManagedEvents::default();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        let mut fields = line.split(' ');
        if fields.next() != Some("MACK") {
            continue;
        }
        let parsed = (|| {
            let seq = fields.next()?.parse().ok()?;
            let unix_ns = fields.next()?.parse().ok()?;
            let operation = ManagedOperation::parse(fields.next()?)?;
            let component = fields.next()?.to_owned();
            let optional = |field: &str| (field != "-").then(|| field.to_owned());
            let selected = optional(fields.next()?);
            let removed = optional(fields.next()?);
            if fields.next().is_some() {
                return None;
            }
            Some(ManagedAck {
                seq,
                unix_ns,
                operation,
                component,
                selected,
                removed,
            })
        })();
        match parsed {
            Some(ack) => events.acks.push(ack),
            None => events.malformed.push(index + 1),
        }
    }
    events
}

/// The stand-in bytes of `version` of `component`: 64 to 256 KiB derived
/// from the names alone, so the verifier knows them without the store.
#[must_use]
pub fn stand_in_bytes(component: &str, version: &str) -> Vec<u8> {
    let seed = Sha256::digest(format!("vsift-managed-stand-in/{component}/{version}"));
    let spread = usize::from(u16::from_le_bytes([seed[0], seed[1]])) * STAND_IN_SPREAD_BYTES
        / usize::from(u16::MAX);
    let length = STAND_IN_MIN_BYTES + spread;
    let mut bytes = Vec::with_capacity(length + 32);
    let mut counter = 0_u64;
    while bytes.len() < length {
        let mut block = Sha256::new();
        block.update(seed);
        block.update(counter.to_le_bytes());
        bytes.extend_from_slice(&block.finalize());
        counter += 1;
    }
    bytes.truncate(length);
    bytes
}

/// Why one command stopped without an acknowledgement.
enum Stop {
    /// The store refused or failed; the command is reported, not acknowledged.
    Failed(String),
    /// The harness itself failed.
    Harness(CampaignError),
}

impl From<CampaignError> for Stop {
    fn from(value: CampaignError) -> Self {
        Self::Harness(value)
    }
}

fn failed(what: impl fmt::Debug) -> Stop {
    Stop::Failed(format!("{what:?}").replace(' ', "_"))
}

/// Publishes and selects the stand-in `version` of `component` under
/// `guard`: import, raw payload, runtime copy, publication and selection,
/// then the stage's removal, as an install does after its smoke.
///
/// # Errors
///
/// The store refused or failed at some step.
pub fn publish_stand_in(
    store: &ManagedArtifactStore,
    guard: &ManagedInstallGuard,
    component: &str,
    version: &str,
) -> Result<(), String> {
    let bytes = stand_in_bytes(component, version);
    let integrity = ArtifactIntegrity::from_sha256_hex(bytes.len() as u64, &sha256_hex(&bytes))
        .map_err(|error| format!("{error:?}"))?;
    let identity =
        ManagedRuntimeIdentity::new(component, version).map_err(|error| format!("{error:?}"))?;
    let artifact = store
        .import_verified(&bytes[..], integrity)
        .map_err(|error| format!("import: {error:?}"))?;
    let payload = artifact
        .stage_reviewed_raw_file(STAND_IN_FILE, integrity)
        .map_err(|error| format!("payload: {error:?}"))?;
    let mut runtime = payload
        .prepare_reviewed_runtime(ReviewedRuntimeLayout {
            max_bytes: integrity.bytes(),
            aliases: &[],
            executables: &[],
        })
        .map_err(|error| format!("runtime: {error:?}"))?;
    let published = runtime
        .publish_and_select(guard, &identity)
        .map_err(|error| format!("publication: {error:?}"));
    drop(published?);
    runtime
        .discard()
        .map_err(|error| format!("discard: {error:?}"))?;
    payload
        .discard()
        .map_err(|error| format!("discard: {error:?}"))?;
    artifact
        .discard()
        .map_err(|error| format!("discard: {error:?}"))
}

/// The managed workload's settings.
#[derive(Clone, Debug)]
pub struct ManagedWorkloadConfig {
    /// The managed root, on the filesystem under test.
    pub root: PathBuf,
    /// Where protocol lines go.
    pub ack_out: PathBuf,
    /// The dm-log-writes device to mark after each acknowledgement.
    pub mark_device: Option<String>,
    /// The `dmsetup` executable.
    pub dmsetup: PathBuf,
    /// Sequence number of the first command.
    pub first_seq: u64,
    /// Stop after this many commands.
    pub max_ops: Option<u64>,
    /// Seed of the command mix.
    pub seed: u64,
}

/// Runs managed commands until the budget is spent, acknowledging each
/// success.
///
/// # Errors
///
/// A [`CampaignError`] when the harness itself fails; a store that refuses
/// a command is reported as an `MFAIL` line.
pub fn run_managed_workload(config: &ManagedWorkloadConfig) -> Result<u64, CampaignError> {
    let mut channel = Channel::open(&config.ack_out, false)?;
    let store =
        ManagedArtifactStore::at(config.root.clone()).map_err(|_| CampaignError::StandIn)?;
    let mut rng = SplitMix64::new(config.seed);
    let mut next_version: BTreeMap<&'static str, u64> = BTreeMap::new();
    channel.line(&format!(
        "WORKLOAD-START store=managed seq={}",
        config.first_seq
    ))?;
    let mut done = 0_u64;
    while config.max_ops.is_none_or(|max| done < max) {
        let seq = config.first_seq + done;
        done += 1;
        let component_index =
            usize::try_from(rng.below(MANAGED_COMPONENTS.len() as u64)).unwrap_or(0);
        let component = MANAGED_COMPONENTS
            .get(component_index)
            .copied()
            .ok_or(CampaignError::StandIn)?;
        let key = component.identifier();
        let operation = match rng.below(100) {
            0..40 => ManagedOperation::Install,
            40..45 => ManagedOperation::Reinstall,
            45..60 => ManagedOperation::Rollback,
            60..70 => ManagedOperation::RemoveVersion,
            70..74 => ManagedOperation::RemoveComponent,
            74..86 => ManagedOperation::AbandonStage,
            _ => ManagedOperation::Sweep,
        };
        channel.line(&format!("MSTART {seq} {} {operation} {key}", unix_nanos()?))?;
        // Logged before the command's first write: the replay needs it to
        // know which command may be in flight at a point (see `in_flight`).
        if let Some(device) = &config.mark_device {
            mark(&config.dmsetup, device, &start_mark_for(seq))?;
        }
        let version = next_version.entry(key).or_insert(0);
        let outcome = run_operation(&store, component, operation, version);
        match outcome {
            Ok((selected, removed)) => {
                let ack = ManagedAck {
                    seq,
                    unix_ns: unix_nanos()?,
                    operation,
                    component: key.to_owned(),
                    selected,
                    removed,
                };
                channel.line(&ack.line())?;
                if let Some(device) = &config.mark_device {
                    mark(&config.dmsetup, device, &mark_for(seq))?;
                }
            }
            Err(Stop::Failed(reason)) => {
                channel.line(&format!(
                    "MFAIL {seq} {} {operation} {key} {reason}",
                    unix_nanos()?
                ))?;
            }
            Err(Stop::Harness(error)) => return Err(error),
        }
    }
    channel.line(&format!("WORKLOAD-END operations={done}"))?;
    Ok(done)
}

/// Runs one command; returns the selection afterwards and a removed version.
fn run_operation(
    store: &ManagedArtifactStore,
    component: vsift_domain::ManagedComponent,
    operation: ManagedOperation,
    next_version: &mut u64,
) -> Result<(Option<String>, Option<String>), Stop> {
    let key = component.identifier();
    let guard = store.try_install_guard().map_err(failed)?;
    let maintainer = GuardedManagedStore::new(store, &guard);
    let inventory = vsift_application::ManagedStoreReader::inventory(&maintainer)
        .map_err(failed)?
        .unwrap_or_else(vsift_application::ManagedInventory::empty);
    let installed = inventory.component(component).cloned();
    let selected_now = installed
        .as_ref()
        .and_then(|installed| installed.selected_version().map(str::to_owned));
    let mut removed = None;
    match operation {
        ManagedOperation::Install | ManagedOperation::Reinstall => {
            let version =
                if let (ManagedOperation::Reinstall, Some(version)) = (operation, &selected_now) {
                    version.clone()
                } else {
                    *next_version += 1;
                    format!("v{next_version:05}")
                };
            publish_stand_in(store, &guard, key, &version).map_err(Stop::Failed)?;
            let cleaned = clean_up_versions(&maintainer).map_err(failed)?;
            removed = cleaned
                .into_iter()
                .find(|report| report.component == component)
                .map(|report| report.version);
        }
        ManagedOperation::Rollback => {
            roll_back_component(&maintainer, component, None).map_err(failed)?;
        }
        ManagedOperation::RemoveVersion => {
            let target = installed.as_ref().and_then(|installed| {
                installed
                    .versions
                    .iter()
                    .find(|record| Some(&record.version) != selected_now.as_ref())
                    .map(|record| record.version.clone())
            });
            let Some(target) = target else {
                return Err(Stop::Failed(String::from("no_unselected_version")));
            };
            let report = remove_managed(
                &maintainer,
                &ManagedRemovalTarget::Version {
                    component,
                    version: ManagedVersionKey::parse(&target).map_err(failed)?,
                },
            )
            .map_err(failed)?;
            if let Some(code) = report.failure_code() {
                return Err(failed(code));
            }
            removed = Some(target);
        }
        ManagedOperation::RemoveComponent => {
            let report = remove_managed(&maintainer, &ManagedRemovalTarget::Component(component))
                .map_err(failed)?;
            if let Some(code) = report.failure_code() {
                return Err(failed(code));
            }
            removed = report.versions.first().map(|report| report.version.clone());
        }
        ManagedOperation::AbandonStage => {
            let bytes = stand_in_bytes(key, "abandoned");
            let integrity =
                ArtifactIntegrity::from_sha256_hex(bytes.len() as u64, &sha256_hex(&bytes))
                    .map_err(failed)?;
            // Dropped without its discard, as a killed install leaves it.
            drop(
                store
                    .import_verified(&bytes[..], integrity)
                    .map_err(failed)?,
            );
        }
        ManagedOperation::Sweep => {
            let report =
                remove_managed(&maintainer, &ManagedRemovalTarget::StaleStages).map_err(failed)?;
            if let Some(code) = report.failure_code() {
                return Err(failed(code));
            }
        }
    }
    let selected = store
        .open_selected_runtime(key)
        .map_err(failed)?
        .map(|runtime| runtime.identity().version().to_owned());
    Ok((selected, removed))
}

// ---------------------------------------------------------------------------
// The verifier.

/// What one replay point of the managed store found.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ManagedFindings {
    /// Acknowledgements a power loss undid, by sequence number.
    pub lost: Vec<(u64, String)>,
    /// Violations of the claim: the point fails.
    pub damage: Vec<String>,
    /// Selections whose file was torn and that lookup refused (fail closed).
    pub torn_refused: usize,
    /// Selections with whole bytes that lookup refused anyway.
    pub refused_whole: usize,
    /// Acknowledgements checked.
    pub acks: usize,
    /// Components found holding the selection of the command in flight at
    /// the point rather than their last acknowledged one (allowed: a later
    /// command superseded it, nothing was undone).
    pub in_flight_states: usize,
    /// Repair passes applied.
    pub repair_passes: usize,
}

impl ManagedFindings {
    /// Whether nothing violated the claim.
    #[must_use]
    pub const fn clean(&self) -> bool {
        self.damage.is_empty()
    }

    /// The report lines: one per finding, then a summary.
    #[must_use]
    pub fn lines(&self, prefix: &str) -> Vec<String> {
        let mut lines: Vec<String> = self
            .lost
            .iter()
            .map(|(seq, loss)| format!("{prefix}-LOST seq={seq} {loss}"))
            .chain(
                self.damage
                    .iter()
                    .map(|damage| format!("{prefix}-DAMAGE {damage}")),
            )
            .collect();
        lines.push(format!(
            "{prefix} managed acks={} lost={} damaged={} torn_refused={} refused_whole={} \
             in_flight_states={} repair_passes={}",
            self.acks,
            self.lost.len(),
            self.damage.len(),
            self.torn_refused,
            self.refused_whole,
            self.in_flight_states,
            self.repair_passes
        ));
        lines
    }
}

/// The version a component's pointer names, read without the store's code.
fn raw_pointer_version(root: &Path, component: &str) -> Option<String> {
    let text =
        fs::read_to_string(root.join("current-v1").join(format!("{component}.current"))).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("version="))
        .map(str::to_owned)
}

/// Whether the stand-in file of a version holds exactly its bytes, read
/// without the store's code.
fn raw_version_whole(root: &Path, component: &str, version: &str) -> bool {
    fs::read(
        root.join("versions-v1")
            .join(format!("{component}--{version}"))
            .join(STAND_IN_FILE),
    )
    .is_ok_and(|bytes| bytes == stand_in_bytes(component, version))
}

/// The acknowledged command that may be in flight at a replay point, given
/// the acknowledgements in workload order, how many of them must hold there
/// (`required`) and the last command whose start mark the point's prefix
/// holds (`started`).
///
/// Why (run 36793177930, P13 PR 7 addendum of 2026-10-01): a command's
/// acknowledgement binds only from the point before its mark, but the
/// command's own flushes come earlier, so between its selection's flush and
/// its mark the device durably holds the selection the command is about to
/// report while the previous acknowledgement of that component is still the
/// last one required. A managed selection is overwritten, not appended, so
/// that is a newer state, not an undone one. The workload is sequential, so
/// only the first acknowledged command after the required ones can be in
/// flight (every later one started after its mark), and only once it has
/// started: before its start mark the prefix holds nothing of it, and the
/// required acknowledgements must hold exactly. Failed commands in between
/// are not admitted: their outcome was never reported.
#[must_use]
pub fn in_flight(
    acks: &[ManagedAck],
    required: usize,
    started: Option<u64>,
) -> Option<&ManagedAck> {
    let next = acks.get(required)?;
    started
        .is_some_and(|started| started >= next.seq)
        .then_some(next)
}

/// Holds the managed root at one replay point to the claim, then repairs
/// it and reinstalls each component's last acknowledged selection. It
/// writes to the store: run it on a copy.
///
/// `acks` are the acknowledgements that must hold; `in_flight` the one
/// acknowledged command that may be part-way through at the point
/// ([`in_flight`]): a component may hold the selection that command
/// reported instead of its last acknowledged one. Anything else is a loss.
#[must_use]
pub fn verify_managed(
    root: &Path,
    acks: &[ManagedAck],
    in_flight: Option<&ManagedAck>,
) -> ManagedFindings {
    let mut findings = ManagedFindings {
        acks: acks.len(),
        ..ManagedFindings::default()
    };
    let Ok(store) = ManagedArtifactStore::at(root.to_path_buf()) else {
        findings.damage.push(String::from("root-unusable"));
        return findings;
    };
    if let Err(fault) = store.inspect() {
        findings
            .damage
            .push(format!("store-uninspectable {fault:?}"));
        return findings;
    }
    let mut last: BTreeMap<&str, &ManagedAck> = BTreeMap::new();
    for ack in acks {
        last.insert(ack.component.as_str(), ack);
    }
    for component in MANAGED_COMPONENTS {
        let key = component.identifier();
        let raw = raw_pointer_version(root, key);
        let whole = raw
            .as_deref()
            .is_some_and(|version| raw_version_whole(root, key, version));
        match store.open_selected_runtime(key) {
            Ok(Some(runtime)) => {
                let version = runtime.identity().version();
                if raw.as_deref() != Some(version) || !whole {
                    findings.damage.push(format!(
                        "torn-selection-used component={key} version={version}"
                    ));
                }
            }
            Ok(None) => {
                if raw.is_some() {
                    findings
                        .damage
                        .push(format!("pointer-ignored component={key}"));
                }
            }
            Err(_) if raw.is_some() && !whole => findings.torn_refused += 1,
            Err(_) => findings.refused_whole += 1,
        }
        let Some(ack) = last.get(key) else {
            continue;
        };
        if ack.selected == raw {
            continue;
        }
        if in_flight.is_some_and(|command| command.component == key && command.selected == raw) {
            findings.in_flight_states += 1;
        } else {
            findings.lost.push((
                ack.seq,
                format!(
                    "selection component={key} acknowledged={} found={}",
                    ack.selected.as_deref().unwrap_or("-"),
                    raw.as_deref().unwrap_or("-")
                ),
            ));
        }
    }
    for ack in acks {
        if let Some(removed) = &ack.removed
            && root
                .join("versions-v1")
                .join(format!("{}--{removed}", ack.component))
                .exists()
            && !acks.iter().any(|later| {
                later.seq > ack.seq
                    && later.component == ack.component
                    && later.selected.as_ref() == Some(removed)
            })
        {
            findings.lost.push((
                ack.seq,
                format!(
                    "removal component={} version={removed} reappeared",
                    ack.component
                ),
            ));
        }
    }
    repair_and_reinstall(&store, &last, &mut findings);
    findings
}

/// Applies `setup repair`'s commands until the store is healthy, then
/// reinstalls each component's last acknowledged selection.
fn repair_and_reinstall(
    store: &ManagedArtifactStore,
    last: &BTreeMap<&str, &ManagedAck>,
    findings: &mut ManagedFindings,
) {
    // The guard creates the root when a power loss took it with everything
    // else: the reinstall below starts from nothing, as a user's would.
    let guard = match store.try_install_guard() {
        Ok(guard) => guard,
        Err(error) => {
            findings.damage.push(format!("guard-unavailable {error:?}"));
            return;
        }
    };
    let maintainer = GuardedManagedStore::new(store, &guard);
    let mut healthy = false;
    for _ in 0..REPAIR_PASSES {
        let inventory = match store.inspect() {
            Ok(inventory) => inventory,
            Err(fault) => {
                findings
                    .damage
                    .push(format!("store-uninspectable {fault:?}"));
                return;
            }
        };
        let plan = diagnose_managed_store(inventory.as_ref());
        if matches!(
            plan.status,
            RepairStatus::Healthy | RepairStatus::NothingInstalled
        ) {
            healthy = true;
            break;
        }
        findings.repair_passes += 1;
        for finding in &plan.findings {
            let applied = match &finding.fix {
                RepairFix::Manual => {
                    findings
                        .damage
                        .push(format!("manual-repair kind={}", finding.kind.identifier()));
                    return;
                }
                RepairFix::RollBack(component) => {
                    roll_back_component(&maintainer, *component, None).map(|_| ())
                }
                RepairFix::RollBackTo(component, version) => ManagedVersionKey::parse(version)
                    .map_err(|_| vsift_application::ManagedLifecycleRefusal::VersionNotInstalled)
                    .and_then(|version| {
                        roll_back_component(&maintainer, *component, Some(&version)).map(|_| ())
                    }),
                RepairFix::RemoveVersion(component, version) => ManagedVersionKey::parse(version)
                    .map_err(|_| vsift_application::ManagedLifecycleRefusal::VersionNotInstalled)
                    .and_then(|version| {
                        remove_managed(
                            &maintainer,
                            &ManagedRemovalTarget::Version {
                                component: *component,
                                version,
                            },
                        )
                        .map(|_| ())
                    }),
                RepairFix::RemoveComponentAndReinstall(component) => {
                    remove_managed(&maintainer, &ManagedRemovalTarget::Component(*component))
                        .map(|_| ())
                }
                RepairFix::RemoveStaleStages => {
                    remove_managed(&maintainer, &ManagedRemovalTarget::StaleStages).map(|_| ())
                }
            };
            if let Err(refusal) = applied {
                findings.damage.push(format!(
                    "repair-refused kind={} {refusal:?}",
                    finding.kind.identifier()
                ));
                return;
            }
        }
    }
    if !healthy {
        findings.damage.push(String::from("repair-not-converged"));
        return;
    }
    for (component, ack) in last {
        let Some(version) = &ack.selected else {
            continue;
        };
        if let Err(error) = publish_stand_in(store, &guard, component, version) {
            findings
                .damage
                .push(format!("reinstall-failed component={component} {error}"));
            continue;
        }
        match store.open_selected_runtime(component) {
            Ok(Some(runtime)) if runtime.identity().version() == version => {}
            other => findings.damage.push(format!(
                "reinstall-not-selected component={component} {:?}",
                other.map(|runtime| runtime.map(|runtime| runtime.identity().version().to_owned()))
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use vsift_application::{ManagedRemovalTarget, remove_managed};
    use vsift_infrastructure::{GuardedManagedStore, ManagedArtifactStore};

    use super::{
        ManagedAck, ManagedOperation, ManagedWorkloadConfig, STAND_IN_MIN_BYTES, in_flight,
        parse_managed_events, publish_stand_in, run_managed_workload, stand_in_bytes,
        verify_managed,
    };
    use crate::workload::{MAX_ACK_BYTES, read_text};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Result<Self, Box<dyn std::error::Error>> {
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let path = env::temp_dir().join(format!(
                "vsift-campaign-managed-{}-{stamp}",
                std::process::id()
            ));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn acknowledgements_round_trip_and_malformed_lines_are_counted() {
        let ack = ManagedAck {
            seq: 7,
            unix_ns: 12,
            operation: ManagedOperation::RemoveVersion,
            component: String::from("whisper_cli"),
            selected: Some(String::from("v00002")),
            removed: None,
        };
        let text = format!("{}\nMACK 8 1 install\nnoise\n", ack.line());
        let events = parse_managed_events(&text);
        assert_eq!(events.acks, [ack]);
        assert_eq!(events.malformed, [2]);
    }

    #[test]
    fn stand_in_bytes_are_deterministic_and_distinct() {
        let first = stand_in_bytes("whisper_cli", "v00001");
        assert_eq!(first, stand_in_bytes("whisper_cli", "v00001"));
        assert_ne!(first, stand_in_bytes("whisper_cli", "v00002"));
        assert!(first.len() >= STAND_IN_MIN_BYTES);
    }

    /// Without a power loss the workload's store passes the verifier with
    /// nothing lost, and a store whose selected file was torn is refused by
    /// lookup, repaired and reinstalled: the verifier sees a torn selection
    /// and does not call it damage.
    #[test]
    fn the_workload_passes_the_verifier_and_a_torn_selection_is_refused() -> TestResult {
        let scratch = Scratch::new()?;
        let root = scratch.0.join("managed");
        let acks_path = scratch.0.join("acks.log");
        run_managed_workload(&ManagedWorkloadConfig {
            root: root.clone(),
            ack_out: acks_path.clone(),
            mark_device: None,
            dmsetup: PathBuf::from("dmsetup"),
            first_seq: 1,
            max_ops: Some(40),
            seed: 20_260_930,
        })?;
        let events = parse_managed_events(&read_text(&acks_path, MAX_ACK_BYTES)?);
        assert!(events.malformed.is_empty());
        assert!(events.acks.len() > 10, "{}", events.acks.len());
        let findings = verify_managed(&root, &events.acks, None);
        assert!(findings.clean(), "{:?}", findings.lines("TEST"));
        assert!(findings.lost.is_empty(), "{:?}", findings.lines("TEST"));

        // Tear the file of a selected version, as a power loss can when its
        // bytes were never flushed.
        let (component, version) = vsift_application::MANAGED_COMPONENTS
            .iter()
            .find_map(|component| {
                let key = component.identifier();
                super::raw_pointer_version(&root, key).map(|version| (key, version))
            })
            .ok_or("nothing is selected")?;
        let file = root
            .join("versions-v1")
            .join(format!("{component}--{version}"))
            .join(super::STAND_IN_FILE);
        fs::write(&file, b"")?;
        let findings = verify_managed(&root, &events.acks, None);
        assert!(findings.clean(), "{:?}", findings.lines("TEST"));
        assert_eq!(findings.torn_refused, 1, "{:?}", findings.lines("TEST"));
        assert!(findings.repair_passes >= 1);
        Ok(())
    }

    fn ack(
        seq: u64,
        operation: ManagedOperation,
        component: &str,
        selected: Option<&str>,
    ) -> ManagedAck {
        ManagedAck {
            seq,
            unix_ns: u128::from(seq),
            operation,
            component: component.to_owned(),
            selected: selected.map(str::to_owned),
            removed: None,
        }
    }

    /// What happens to the component after its installs.
    #[derive(Clone, Copy)]
    enum Then {
        /// Nothing: the last install stays selected.
        Keep,
        /// `setup remove <component>`.
        RemoveComponent,
    }

    /// A fresh managed root at `scratch/name` where `component` installed
    /// `versions` in order, then `then`.
    fn store_with(
        scratch: &Scratch,
        name: &str,
        component: vsift_domain::ManagedComponent,
        versions: &[&str],
        then: Then,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let root = scratch.0.join(name);
        let store = ManagedArtifactStore::at(root.clone()).map_err(|error| format!("{error:?}"))?;
        let guard = store
            .try_install_guard()
            .map_err(|error| format!("{error:?}"))?;
        for version in versions {
            publish_stand_in(&store, &guard, component.identifier(), version)?;
        }
        if let Then::RemoveComponent = then {
            let maintainer = GuardedManagedStore::new(&store, &guard);
            remove_managed(&maintainer, &ManagedRemovalTarget::Component(component))
                .map_err(|error| format!("{error:?}"))?;
        }
        Ok(root)
    }

    /// Only the first acknowledged command beyond the required ones can be
    /// in flight, and only once its start mark is in the prefix.
    #[test]
    fn only_the_next_acknowledged_command_is_in_flight_once_it_started() {
        let acks = [
            ack(1, ManagedOperation::Install, "whisper_cli", Some("v00001")),
            ack(4, ManagedOperation::Install, "whisper_cli", Some("v00002")),
            ack(5, ManagedOperation::Rollback, "whisper_cli", Some("v00001")),
        ];
        assert_eq!(in_flight(&acks, 1, None), None);
        // Failed commands 2 and 3 started; command 4 had not.
        assert_eq!(in_flight(&acks, 1, Some(3)), None);
        assert_eq!(in_flight(&acks, 1, Some(4)), acks.get(1));
        // Command 5 cannot be in flight while 4's acknowledgement does not
        // bind: it started after 4's mark.
        assert_eq!(in_flight(&acks, 2, Some(4)), None);
        assert_eq!(in_flight(&acks, 2, Some(5)), acks.get(2));
        assert_eq!(in_flight(&acks, 3, Some(5)), None);
    }

    /// Regression for run 36793177930 (53 "lost" acknowledgements, none
    /// undone): at a point between a command's selection flush and its
    /// acknowledgement mark, the component durably holds the selection that
    /// command reports, newer than its last required acknowledgement. That
    /// is not a loss. A selection that went back, or one no command in
    /// flight reported, still is.
    #[test]
    fn a_selection_the_command_in_flight_made_is_not_a_loss_but_an_undone_one_is() -> TestResult {
        let scratch = Scratch::new()?;
        let component = vsift_application::MANAGED_COMPONENTS
            .first()
            .copied()
            .ok_or("no managed component")?;
        let other = vsift_application::MANAGED_COMPONENTS
            .get(1)
            .copied()
            .ok_or("one managed component")?;
        let key = component.identifier();
        let first = ack(1, ManagedOperation::Install, key, Some("v00001"));
        let second = ack(2, ManagedOperation::Install, key, Some("v00002"));
        let removal = ack(3, ManagedOperation::RemoveComponent, key, None);

        // The store selects v00002: command 2 committed, its mark not yet.
        // The old rule (no command in flight) calls it lost, as the run did.
        let root = store_with(
            &scratch,
            "strict",
            component,
            &["v00001", "v00002"],
            Then::Keep,
        )?;
        let findings = verify_managed(&root, std::slice::from_ref(&first), None);
        assert_eq!(findings.lost.len(), 1, "{:?}", findings.lines("TEST"));

        // With command 2 in flight it is the newer state, and nothing else
        // about the point changes.
        let root = store_with(
            &scratch,
            "ahead",
            component,
            &["v00001", "v00002"],
            Then::Keep,
        )?;
        let findings = verify_managed(&root, std::slice::from_ref(&first), Some(&second));
        assert!(findings.lost.is_empty(), "{:?}", findings.lines("TEST"));
        assert!(findings.clean(), "{:?}", findings.lines("TEST"));
        assert_eq!(findings.in_flight_states, 1);

        // A component removal in flight: nothing selected is its outcome.
        let root = store_with(
            &scratch,
            "removed",
            component,
            &["v00001", "v00002"],
            Then::RemoveComponent,
        )?;
        let required = [first.clone(), second.clone()];
        let findings = verify_managed(&root, &required, Some(&removal));
        assert!(findings.lost.is_empty(), "{:?}", findings.lines("TEST"));
        assert_eq!(findings.in_flight_states, 1);
        let root = store_with(
            &scratch,
            "removed-strict",
            component,
            &["v00001", "v00002"],
            Then::RemoveComponent,
        )?;
        let findings = verify_managed(&root, &required, None);
        assert_eq!(findings.lost.len(), 1, "{:?}", findings.lines("TEST"));

        // The command in flight reports another version, or concerns another
        // component: the selection is still a loss.
        let root = store_with(
            &scratch,
            "elsewhere",
            component,
            &["v00001", "v00002"],
            Then::Keep,
        )?;
        let wrong_version = ack(2, ManagedOperation::Install, key, Some("v00003"));
        let findings = verify_managed(&root, std::slice::from_ref(&first), Some(&wrong_version));
        assert_eq!(findings.lost.len(), 1, "{:?}", findings.lines("TEST"));
        let root = store_with(
            &scratch,
            "other",
            component,
            &["v00001", "v00002"],
            Then::Keep,
        )?;
        let other_component = ack(
            2,
            ManagedOperation::Install,
            other.identifier(),
            Some("v00002"),
        );
        let findings = verify_managed(&root, std::slice::from_ref(&first), Some(&other_component));
        assert_eq!(findings.lost.len(), 1, "{:?}", findings.lines("TEST"));

        // An acknowledged install undone (the store went back to v00001)
        // while a third install is in flight: still a loss.
        let root = store_with(&scratch, "undone", component, &["v00001"], Then::Keep)?;
        let third = ack(3, ManagedOperation::Install, key, Some("v00003"));
        let findings = verify_managed(&root, &required, Some(&third));
        assert_eq!(findings.lost.len(), 1, "{:?}", findings.lines("TEST"));
        assert_eq!(findings.in_flight_states, 0);
        Ok(())
    }
}
