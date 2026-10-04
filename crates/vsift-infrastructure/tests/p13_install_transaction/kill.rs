//! P13 PR 7: kill tests of the managed store (ADR 0023 §3 step 7, decision
//! H9; D-03, D-05, D-08).
//!
//! A child process (this test binary, re-run on one ignored entry) runs one
//! managed command against a prepared managed root: `setup install` exactly
//! as the engine composes it (the stale-stage sweep, the transaction, the
//! bounded cleanup, all under one install guard), `setup rollback --version`,
//! `setup remove --version`, `setup remove <component>` or `setup remove
//! --stale-stages`, each through the same application use cases the engine
//! calls. The child stops at one arrival of one managed fault point
//! (`VSIFT_FAULT_POINT=<name>:<n>`, the `fault-injection` feature), or is
//! killed by the operating system (`SIGKILL`, `TerminateProcess`) while a
//! download stalls and at spread moments of a whole install.
//!
//! After every stop the parent requires the store to be consistent:
//!
//! - it can always be inspected;
//! - each component's selection is the one before the command or the one
//!   after it, and a selected version always opens, which re-hashes every
//!   file against its manifest (no half-selected state);
//! - `setup repair`'s diagnosis matches what an independent walk of the
//!   folder finds (abandoned stages, half-written pointers, interrupted
//!   removals), names an existing command for every finding and never
//!   needs the user to delete anything by hand;
//! - the same command, run again, completes; and the store is then healthy.
//!
//! Every arrival of every point a command reaches is tried, and each
//! scenario fails unless every point it names was reached at least once.
//! Together the scenarios reach every managed point.

use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use vsift_application::{
    ManagedInventory, ManagedRemovalTarget, ManagedStoreMaintenance as _, RepairFindingKind,
    RepairFix, RepairPlan, RepairStatus, SelectionStatus, StageSweep, clean_up_versions,
    diagnose_managed_store, remove_managed, roll_back_component,
};
use vsift_domain::ManagedVersionKey;
use vsift_infrastructure::{
    FAULT_EXIT_CODE, FAULT_MARKER, FAULT_POINT_VARIABLE, FaultPoint, GuardedManagedStore,
    ManagedInstallGuard, ManagedInstallerConfig, ManagedRuntimeIdentity, ReviewedManagedInstaller,
    ReviewedRuntimeLayout,
};

use super::{
    ActionAuthority, FixedVerifiers, Fixture, HostIsolation, MEDIA, MEDIA_FILE, MODEL,
    ManagedArtifactSource, ManagedArtifactStore, ManagedComponent, NoProgress, ProcessCancellation,
    Reply, StoreCompanions, TestResult, TestRoot, TestServer, WHISPER, install_managed_components,
    integrity_of, policy, statuses,
};

/// The ignored entry the child runs.
const CHILD_ENTRY: &str = "kill::managed_kill_child";
/// The managed root the child works on.
const ROOT_VARIABLE: &str = "VSIFT_P13_KILL_ROOT";
/// The local publisher the child downloads from.
const SERVER_VARIABLE: &str = "VSIFT_P13_KILL_SERVER";
/// The command the child runs.
const COMMAND_VARIABLE: &str = "VSIFT_P13_KILL_COMMAND";
/// More arrivals than any command reaches; a point still reached at this
/// arrival fails the test, so no arrival goes untried.
const MAX_ARRIVALS: u32 = 64;
/// Set to try every arrival of every point on any operating system.
const EVERY_ARRIVAL_VARIABLE: &str = "VSIFT_P13_KILL_EVERY_ARRIVAL";

/// Whether every arrival of a point is tried, or only the first.
///
/// Managed installation is qualified on Ubuntu 24.04 only (ADR 0023
/// decision E), so Linux tries every arrival: every marker, stage, version
/// and file a command writes or removes. Elsewhere the lifecycle commands
/// work on a store that is normally absent, and each point's first arrival
/// keeps the same code paths honest on that file system; a smoke there
/// costs seconds, so the exhaustive matrix would take many minutes.
fn every_arrival() -> bool {
    cfg!(target_os = "linux") || std::env::var_os(EVERY_ARRIVAL_VARIABLE).is_some()
}
/// How long a real kill waits for the moment it is aimed at.
const KILL_WAIT: Duration = Duration::from_secs(60);

const MEDIA_1: &str = "fixture-media-1";
const MEDIA_2: &str = "fixture-media-2";
const MEDIA_3: &str = "fixture-media-3";
const WHISPER_1: &str = "fixture-whisper-1";
const MODEL_1: &str = "fixture-model-1";

/// One managed command, as the child runs it and as the parent reruns it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManagedCommand {
    /// `setup install` of the fixture plan whose media tools are version
    /// `fixture-media-<n>`.
    Install(u8),
    /// `setup rollback ffmpeg_ffprobe --version fixture-media-1`.
    RollbackMedia,
    /// `setup remove ffmpeg_ffprobe --version fixture-media-1`.
    RemoveMediaVersion,
    /// `setup remove ffmpeg_ffprobe`.
    RemoveMedia,
    /// `setup remove --stale-stages`.
    SweepStages,
}

impl ManagedCommand {
    fn name(self) -> String {
        match self {
            Self::Install(version) => format!("install-{version}"),
            Self::RollbackMedia => String::from("rollback-media"),
            Self::RemoveMediaVersion => String::from("remove-media-version"),
            Self::RemoveMedia => String::from("remove-media"),
            Self::SweepStages => String::from("sweep-stages"),
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "rollback-media" => Some(Self::RollbackMedia),
            "remove-media-version" => Some(Self::RemoveMediaVersion),
            "remove-media" => Some(Self::RemoveMedia),
            "sweep-stages" => Some(Self::SweepStages),
            other => other
                .strip_prefix("install-")
                .and_then(|version| version.parse().ok())
                .map(Self::Install),
        }
    }
}

fn media_version(version: u8) -> String {
    format!("fixture-media-{version}")
}

/// What one install did: the sweep before it, its report and the cleanup
/// after it.
struct InstallDone {
    sweep: StageSweep,
    statuses: Vec<&'static str>,
}

/// `setup install` as `Engine::install_setup` composes it once the plan is
/// accepted: the guard, the stale-stage sweep, the transaction and the
/// bounded cleanup.
async fn install(store: &ManagedArtifactStore, base: &str, version: u8) -> TestResult<InstallDone> {
    let fixture = Fixture::new(&media_version(version))?;
    let actions = fixture.actions()?;
    let guard = store.try_install_guard()?;
    let sweep = GuardedManagedStore::new(store, &guard)
        .sweep_stages()
        .map_err(|fault| format!("the sweep failed: {fault:?}"))?;
    let installer = ReviewedManagedInstaller::new(
        ManagedInstallerConfig {
            store: store.clone(),
            source: ManagedArtifactSource::Publisher,
            authority: ActionAuthority::Loopback {
                base_url: base.to_owned(),
                proxy: None,
            },
            policy: policy()?,
            host_isolation: HostIsolation::ProcessOnly,
            verifiers: FixedVerifiers::passing(),
            companions: StoreCompanions {
                store: store.clone(),
            },
            cancellation: ProcessCancellation::new(),
        },
        &guard,
        &NoProgress,
    );
    let report = install_managed_components(&installer, &actions, &NoProgress).await;
    drop(installer);
    clean_up_versions(&GuardedManagedStore::new(store, &guard))
        .map_err(|refusal| format!("the cleanup failed: {refusal:?}"))?;
    Ok(InstallDone {
        sweep,
        statuses: statuses(&report),
    })
}

/// Runs `command` to the end; an install must activate or find current
/// every component.
async fn run_command(
    store: &ManagedArtifactStore,
    base: &str,
    command: ManagedCommand,
) -> TestResult {
    let media = ManagedComponent::MediaTools;
    match command {
        ManagedCommand::Install(version) => {
            let done = install(store, base, version).await?;
            if done
                .statuses
                .iter()
                .any(|status| !matches!(*status, "activated" | "already_current"))
            {
                return Err(format!("the install did not complete: {:?}", done.statuses).into());
            }
        }
        ManagedCommand::RollbackMedia => {
            let guard = store.try_install_guard()?;
            roll_back_component(
                &GuardedManagedStore::new(store, &guard),
                media,
                Some(&ManagedVersionKey::parse(MEDIA_1)?),
            )
            .map_err(|refusal| format!("the rollback was refused: {refusal:?}"))?;
        }
        ManagedCommand::RemoveMediaVersion
        | ManagedCommand::RemoveMedia
        | ManagedCommand::SweepStages => {
            let target = match command {
                ManagedCommand::RemoveMediaVersion => ManagedRemovalTarget::Version {
                    component: media,
                    version: ManagedVersionKey::parse(MEDIA_1)?,
                },
                ManagedCommand::RemoveMedia => ManagedRemovalTarget::Component(media),
                _ => ManagedRemovalTarget::StaleStages,
            };
            let guard = store.try_install_guard()?;
            let report = remove_managed(&GuardedManagedStore::new(store, &guard), &target)
                .map_err(|refusal| format!("the removal was refused: {refusal:?}"))?;
            if let Some(code) = report.failure_code() {
                return Err(format!("the removal failed: {code:?} {report:?}").into());
            }
        }
    }
    Ok(())
}

/// The child: runs the command `VSIFT_P13_KILL_COMMAND` names on the root
/// `VSIFT_P13_KILL_ROOT` names, stopping where `VSIFT_FAULT_POINT` says.
#[tokio::test]
#[ignore = "internal child entry launched by the managed-store kill tests"]
async fn managed_kill_child() -> TestResult {
    let root = std::env::var_os(ROOT_VARIABLE).ok_or("no managed root")?;
    let base = std::env::var(SERVER_VARIABLE)?;
    let command = std::env::var(COMMAND_VARIABLE)?;
    let command = ManagedCommand::parse(&command).ok_or("unknown command")?;
    let store = ManagedArtifactStore::at(std::path::PathBuf::from(root))?;
    run_command(&store, &base, command).await
}

/// How a child ended.
#[derive(Debug, Eq, PartialEq)]
enum ChildEnd {
    /// It stopped at the fault point.
    Stopped,
    /// The command completed.
    Completed,
}

fn child_command(
    root: &TestRoot,
    server: &TestServer,
    command: ManagedCommand,
) -> TestResult<Command> {
    let mut child = Command::new(std::env::current_exe()?);
    child
        .args([
            CHILD_ENTRY,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(ROOT_VARIABLE, root.root())
        .env(SERVER_VARIABLE, server.base())
        .env(COMMAND_VARIABLE, command.name())
        .env_remove(FAULT_POINT_VARIABLE)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    Ok(child)
}

/// Runs the child to the end, stopping at `fault` when given.
fn run_child(
    root: &TestRoot,
    server: &TestServer,
    command: ManagedCommand,
    fault: Option<(FaultPoint, u32)>,
) -> TestResult<ChildEnd> {
    let mut child = child_command(root, server, command)?;
    if let Some((point, arrival)) = fault {
        child.env(FAULT_POINT_VARIABLE, format!("{}:{arrival}", point.name()));
    }
    let output = child.output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    match output.status.code() {
        Some(code) if code == FAULT_EXIT_CODE => {
            let point = fault.map(|(point, _)| point.name()).unwrap_or_default();
            if stderr.contains(&format!("{FAULT_MARKER}={point}")) {
                Ok(ChildEnd::Stopped)
            } else {
                Err(format!("the child stopped without its marker: {stderr}").into())
            }
        }
        Some(0) => Ok(ChildEnd::Completed),
        other => Err(format!("{} failed ({other:?}): {stderr}", command.name()).into()),
    }
}

// ---------------------------------------------------------------------------
// What the folder holds, read without the store's code.

/// What an independent walk of the managed folder finds.
#[derive(Debug, Default, Eq, PartialEq)]
struct Facts {
    /// `stage-*` folders: abandoned stages.
    stages: usize,
    /// `*.pending` files in `current-v1`: half-written pointers.
    pending: usize,
    /// Version folders a removal left behind: holding its tombstone, or
    /// empty.
    interrupted_removals: BTreeSet<String>,
}

impl Facts {
    fn observe(root: &TestRoot) -> TestResult<Self> {
        let mut facts = Self::default();
        let base = root.root();
        if !base.exists() {
            return Ok(facts);
        }
        for entry in fs::read_dir(&base)? {
            if entry?.file_name().to_string_lossy().starts_with("stage-") {
                facts.stages += 1;
            }
        }
        let current = base.join("current-v1");
        if current.is_dir() {
            for entry in fs::read_dir(&current)? {
                if entry?.file_name().to_string_lossy().ends_with(".pending") {
                    facts.pending += 1;
                }
            }
        }
        let versions = base.join("versions-v1");
        if versions.is_dir() {
            for entry in fs::read_dir(&versions)? {
                let entry = entry?;
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let empty = fs::read_dir(&path)?.next().is_none();
                if empty || path.join("removing-v1").exists() {
                    facts
                        .interrupted_removals
                        .insert(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        Ok(facts)
    }
}

/// The selections a component may have after a stop: the one before the
/// command and the one after it.
struct Allowed<'a> {
    component: &'a str,
    versions: &'a [Option<&'a str>],
}

/// The consistency every stop must leave, and the repair plan it implies.
fn assert_consistent(
    root: &TestRoot,
    allowed: &[Allowed<'_>],
    context: &str,
) -> TestResult<RepairPlan> {
    let store = root.store()?;
    let inventory: Option<ManagedInventory> = store
        .inspect()
        .map_err(|fault| format!("{context}: the store cannot be inspected: {fault:?}"))?;
    for expectation in allowed {
        let selected = store
            .open_selected_runtime(expectation.component)
            .map_err(|error| {
                format!(
                    "{context}: {} does not open: {error:?}",
                    expectation.component
                )
            })?
            .map(|runtime| runtime.identity().version().to_owned());
        assert!(
            expectation.versions.contains(&selected.as_deref()),
            "{context}: {} selects {selected:?}, allowed {:?}",
            expectation.component,
            expectation.versions
        );
    }
    let facts = Facts::observe(root)?;
    let plan = diagnose_managed_store(inventory.as_ref());
    let Some(inventory) = inventory else {
        assert_eq!(
            facts,
            Facts::default(),
            "{context}: no store, but {facts:?}"
        );
        assert_eq!(plan.status, RepairStatus::NothingInstalled, "{context}");
        return Ok(plan);
    };
    assert_eq!(inventory.stale_stages, facts.stages, "{context}: stages");
    assert!(
        inventory.retained_stages.is_empty(),
        "{context}: {inventory:?}"
    );
    assert_eq!(
        inventory.interrupted_selections, facts.pending,
        "{context}: pointers"
    );
    assert_eq!(inventory.unexpected_entries, 0, "{context}: {inventory:?}");
    for component in &inventory.components {
        assert!(
            matches!(
                component.selection_status(),
                SelectionStatus::None | SelectionStatus::Verified
            ),
            "{context}: {component:?}"
        );
    }
    assert_findings_match(&plan, &facts, context)?;
    // The install guard is free: nothing a killed command held survives it.
    drop(store.try_install_guard()?);
    Ok(plan)
}

/// `setup repair`'s findings are exactly what the independent walk found,
/// each with the existing command that fixes it and none `manual`.
fn assert_findings_match(plan: &RepairPlan, facts: &Facts, context: &str) -> TestResult {
    let mut interrupted = BTreeSet::new();
    for finding in &plan.findings {
        assert_ne!(finding.fix, RepairFix::Manual, "{context}: {finding:?}");
        match finding.kind {
            RepairFindingKind::StaleStages => {
                assert_eq!(finding.count, Some(facts.stages), "{context}");
                assert_eq!(finding.fix, RepairFix::RemoveStaleStages, "{context}");
            }
            RepairFindingKind::InterruptedSelection => {
                assert_eq!(finding.count, Some(facts.pending), "{context}");
                assert_eq!(finding.fix, RepairFix::RemoveStaleStages, "{context}");
            }
            RepairFindingKind::RemovalInterrupted => {
                let component = finding.component.ok_or("no component")?;
                let version = finding.version.clone().ok_or("no version")?;
                assert_eq!(
                    finding.fix,
                    RepairFix::RemoveVersion(component, version.clone()),
                    "{context}"
                );
                interrupted.insert(format!("{}--{version}", component.identifier()));
            }
            other => {
                return Err(format!("{context}: unexpected finding {other:?}: {finding:?}").into());
            }
        }
    }
    assert_eq!(
        interrupted, facts.interrupted_removals,
        "{context}: removals"
    );
    assert_eq!(
        plan.findings
            .iter()
            .any(|f| f.kind == RepairFindingKind::StaleStages),
        facts.stages > 0,
        "{context}"
    );
    assert_eq!(
        plan.findings
            .iter()
            .any(|f| f.kind == RepairFindingKind::InterruptedSelection),
        facts.pending > 0,
        "{context}"
    );
    assert_eq!(
        plan.status,
        if plan.findings.is_empty() {
            RepairStatus::Healthy
        } else {
            RepairStatus::NeedsRepair
        },
        "{context}"
    );
    Ok(())
}

/// Applies every fix of `plan` with the command it names, as a user would.
fn apply_repair(root: &TestRoot, plan: &RepairPlan) -> TestResult {
    let store = root.store()?;
    let Some(guard) = store.try_existing_install_guard()? else {
        return Ok(());
    };
    let maintainer = GuardedManagedStore::new(&store, &guard);
    for finding in &plan.findings {
        let target = match &finding.fix {
            RepairFix::RemoveStaleStages => ManagedRemovalTarget::StaleStages,
            RepairFix::RemoveVersion(component, version) => ManagedRemovalTarget::Version {
                component: *component,
                version: ManagedVersionKey::parse(version)?,
            },
            other => return Err(format!("unexpected fix {other:?}").into()),
        };
        remove_managed(&maintainer, &target).map_err(|refusal| format!("{refusal:?}"))?;
    }
    Ok(())
}

/// Nothing needs repair, and each component selects what `expected` says.
fn assert_healthy(root: &TestRoot, expected: &[(&str, Option<&str>)], context: &str) -> TestResult {
    let store = root.store()?;
    let inventory = store.inspect().map_err(|fault| format!("{fault:?}"))?;
    let plan = diagnose_managed_store(inventory.as_ref());
    assert!(
        matches!(
            plan.status,
            RepairStatus::Healthy | RepairStatus::NothingInstalled
        ),
        "{context}: {plan:?}"
    );
    assert_eq!(Facts::observe(root)?, Facts::default(), "{context}");
    for (component, version) in expected {
        assert_eq!(
            root.selected(component)?.as_deref(),
            *version,
            "{context}: {component}"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Scenarios.

/// One command, the state it starts from and what it must end in.
struct Scenario {
    command: ManagedCommand,
    /// The points the command must reach.
    points: &'static [FaultPoint],
    /// Each component's selection before and after.
    allowed: &'static [(&'static str, &'static [Option<&'static str>])],
    /// Each component's selection once the command completed.
    after: &'static [(&'static str, Option<&'static str>)],
    /// Folders that must be gone once the command completed.
    gone: &'static [&'static str],
}

/// Publishes and selects a one-file stand-in version of `component`, as
/// an earlier install would have left it. The kill tests need published
/// versions to roll back, remove and clean up, not their smoke, so this
/// skips the transaction and keeps the preparation fast.
fn publish_stand_in(
    store: &ManagedArtifactStore,
    guard: &ManagedInstallGuard,
    component: &str,
    version: &str,
) -> TestResult {
    let bytes = format!("stand-in {component} {version}\n").into_bytes();
    let integrity = integrity_of(&bytes)?;
    let artifact = store.import_verified(&bytes[..], integrity)?;
    let payload = artifact.stage_reviewed_raw_file("stand-in.bin", integrity)?;
    let mut runtime = payload.prepare_reviewed_runtime(ReviewedRuntimeLayout {
        max_bytes: integrity.bytes(),
        aliases: &[],
        executables: &[],
    })?;
    let published = runtime
        .publish_and_select(guard, &ManagedRuntimeIdentity::new(component, version)?)
        .map_err(|error| format!("publishing {component} {version}: {error:?}"))?;
    drop(published);
    runtime.discard()?;
    payload.discard()?;
    artifact.discard()?;
    Ok(())
}

/// Media tools 1 then 2 selected (1 recorded as previous), the recognizer
/// and the model selected, each the version the fixture plan installs;
/// then, when `leftovers`, a stage an earlier run abandoned with part of
/// its payload, one killed at its creation, and a half-written selection
/// pointer.
fn prepare_two_versions(root: &TestRoot, leftovers: bool) -> TestResult {
    let store = root.store()?;
    let guard = store.try_install_guard()?;
    for (component, version) in [
        (MEDIA, MEDIA_1),
        (MEDIA, MEDIA_2),
        (WHISPER, WHISPER_1),
        (MODEL, MODEL_1),
    ] {
        publish_stand_in(&store, &guard, component, version)?;
    }
    drop(guard);
    if leftovers {
        // A stage an earlier run abandoned after staging part of its
        // payload: the marker, the artifact and a payload folder.
        let fixture = Fixture::new(MEDIA_1)?;
        let bytes = fixture.bytes(MEDIA_FILE)?;
        let staged = store.import_verified(&bytes[..], integrity_of(&bytes)?)?;
        drop(staged);
        let abandoned = root.stages()?.into_iter().next().ok_or("no stage")?;
        private_directory(&abandoned.join("payload.pending"))?;
        private_file(
            &abandoned.join("payload.pending").join("ffmpeg"),
            b"partial",
        )?;
        // A stage killed while its marker was written.
        let stage = root.root().join(format!("stage-{}", "e".repeat(32)));
        private_directory(&stage)?;
        private_file(&stage.join("stage-v1"), b"VSIFT-MAN")?;
        // A selection killed while its pending pointer was written.
        private_file(
            &root
                .root()
                .join("current-v1")
                .join(format!("{MEDIA}.pending")),
            b"VSIFT-MANAGED-POINTER-v2\ncompo",
        )?;
    }
    Ok(())
}

fn private_directory(path: &Path) -> TestResult {
    fs::create_dir(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn private_file(path: &Path, contents: &[u8]) -> TestResult {
    fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Kills `scenario`'s command at every arrival of every point it names, and
/// checks the store after each stop, after a rerun, and once repaired.
async fn kill_everywhere(scenario: &Scenario, prepare: Preparation) -> TestResult {
    let server = TestServer::start()?;
    let allowed: Vec<Allowed<'_>> = scenario
        .allowed
        .iter()
        .map(|(component, versions)| Allowed {
            component,
            versions,
        })
        .collect();
    let mut reached = BTreeSet::new();
    let started = Instant::now();
    let mut stops = 0_u32;
    let every_arrival = every_arrival();
    for point in scenario.points {
        for arrival in 1..=MAX_ARRIVALS {
            if arrival > 1 && !every_arrival {
                break;
            }
            let root = TestRoot::new()?;
            prepare.run(&root)?;
            if let ManagedCommand::Install(version) = scenario.command {
                Fixture::new(&media_version(version))?.serve_all(&server);
            }
            let context = format!("{} at {}:{arrival}", scenario.command.name(), point.name());
            match run_child(&root, &server, scenario.command, Some((*point, arrival)))? {
                ChildEnd::Completed => {
                    assert_healthy(&root, scenario.after, &context)?;
                    break;
                }
                ChildEnd::Stopped => {
                    stops += 1;
                    reached.insert(point.name());
                    assert_consistent(&root, &allowed, &context)?;
                    // The same command, run again, completes; whatever it
                    // does not clean up itself, repair names and fixes.
                    run_command(&root.store()?, &server.base(), scenario.command).await?;
                    for folder in scenario.gone {
                        assert!(!root.root().join(folder).exists(), "{context}: {folder}");
                    }
                    let after = assert_consistent(&root, &allowed, &context)?;
                    apply_repair(&root, &after)?;
                    assert_healthy(&root, scenario.after, &context)?;
                }
            }
            assert!(
                arrival < MAX_ARRIVALS,
                "{context}: more arrivals than tried"
            );
        }
    }
    let missing: Vec<&str> = scenario
        .points
        .iter()
        .map(|point| point.name())
        .filter(|name| !reached.contains(name))
        .collect();
    assert!(
        missing.is_empty(),
        "{} never reached {missing:?}",
        scenario.command.name()
    );
    println!(
        "{}: {stops} stops at {} points in {:.1} s",
        scenario.command.name(),
        reached.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// The state a scenario starts from.
#[derive(Clone, Copy)]
enum Preparation {
    /// No managed root at all.
    Nothing,
    /// [`prepare_two_versions`] without leftovers.
    TwoVersions,
    /// [`prepare_two_versions`] with leftovers.
    TwoVersionsAndLeftovers,
}

impl Preparation {
    fn run(self, root: &TestRoot) -> TestResult {
        match self {
            Self::Nothing => Ok(()),
            Self::TwoVersions => prepare_two_versions(root, false),
            Self::TwoVersionsAndLeftovers => prepare_two_versions(root, true),
        }
    }
}

const FRESH_INSTALL_POINTS: [FaultPoint; 15] = [
    FaultPoint::ManagedDirectoryCreated,
    FaultPoint::ManagedMarkerCreated,
    FaultPoint::ManagedArtifactPartial,
    FaultPoint::ManagedArtifactWritten,
    FaultPoint::ManagedPayloadStaged,
    FaultPoint::ManagedRuntimePrepared,
    FaultPoint::ManagedSmokeStarted,
    FaultPoint::ManagedSmokePassed,
    FaultPoint::ManagedVersionPublished,
    FaultPoint::ManagedPointerPrepared,
    FaultPoint::ManagedPointerReplaced,
    FaultPoint::ManagedStageFileRemoved,
    FaultPoint::ManagedStageFolderRemoved,
    FaultPoint::ManagedStageArtifactRemoved,
    FaultPoint::ManagedStageMarkerRemoved,
];

const REMOVAL_POINTS: [FaultPoint; 7] = [
    FaultPoint::ManagedMarkerCreated,
    FaultPoint::ManagedTombstoneWritten,
    FaultPoint::ManagedVersionFileRemoved,
    FaultPoint::ManagedUseLockRemoved,
    FaultPoint::ManagedManifestRemoved,
    FaultPoint::ManagedTombstoneRemoved,
    FaultPoint::ManagedVersionRemoved,
];

/// A first install: the root, its folders, three stages, three versions
/// and three pointers are created; each component's selection is none or
/// its version.
#[tokio::test]
async fn a_first_install_killed_anywhere_is_consistent_and_a_rerun_completes() -> TestResult {
    kill_everywhere(
        &Scenario {
            command: ManagedCommand::Install(1),
            points: &FRESH_INSTALL_POINTS,
            allowed: &[
                (MEDIA, &[None, Some(MEDIA_1)]),
                (WHISPER, &[None, Some(WHISPER_1)]),
                (MODEL, &[None, Some(MODEL_1)]),
            ],
            after: &[
                (MEDIA, Some(MEDIA_1)),
                (WHISPER, Some(WHISPER_1)),
                (MODEL, Some(MODEL_1)),
            ],
            gone: &[],
        },
        Preparation::Nothing,
    )
    .await
}

/// An update over two installed versions with leftovers: the sweep before
/// the transaction removes an abandoned stage, one killed at its creation
/// and a half-written pointer; the new version is staged, smoked, published
/// and selected; the cleanup after it removes the oldest version.
#[tokio::test]
async fn an_update_killed_anywhere_keeps_a_whole_selection_and_a_rerun_completes() -> TestResult {
    const POINTS: [FaultPoint; 19] = [
        FaultPoint::ManagedDirectoryCreated,
        FaultPoint::ManagedMarkerCreated,
        FaultPoint::ManagedStageFileRemoved,
        FaultPoint::ManagedStageFolderRemoved,
        FaultPoint::ManagedStageArtifactRemoved,
        FaultPoint::ManagedStageMarkerRemoved,
        FaultPoint::ManagedPointerRemoved,
        FaultPoint::ManagedArtifactPartial,
        FaultPoint::ManagedArtifactWritten,
        FaultPoint::ManagedSmokePassed,
        FaultPoint::ManagedVersionPublished,
        FaultPoint::ManagedPointerPrepared,
        FaultPoint::ManagedPointerReplaced,
        FaultPoint::ManagedTombstoneWritten,
        FaultPoint::ManagedVersionFileRemoved,
        FaultPoint::ManagedUseLockRemoved,
        FaultPoint::ManagedManifestRemoved,
        FaultPoint::ManagedTombstoneRemoved,
        FaultPoint::ManagedVersionRemoved,
    ];
    kill_everywhere(
        &Scenario {
            command: ManagedCommand::Install(3),
            points: &POINTS,
            allowed: &[
                (MEDIA, &[Some(MEDIA_2), Some(MEDIA_3)]),
                (WHISPER, &[Some(WHISPER_1)]),
                (MODEL, &[Some(MODEL_1)]),
            ],
            after: &[
                (MEDIA, Some(MEDIA_3)),
                (WHISPER, Some(WHISPER_1)),
                (MODEL, Some(MODEL_1)),
            ],
            gone: &["versions-v1/ffmpeg_ffprobe--fixture-media-1"],
        },
        Preparation::TwoVersionsAndLeftovers,
    )
    .await
}

/// `setup rollback --version`: the pending pointer, then its rename.
#[tokio::test]
async fn a_rollback_killed_anywhere_selects_one_whole_version() -> TestResult {
    kill_everywhere(
        &Scenario {
            command: ManagedCommand::RollbackMedia,
            points: &[
                FaultPoint::ManagedMarkerCreated,
                FaultPoint::ManagedPointerPrepared,
                FaultPoint::ManagedPointerReplaced,
            ],
            allowed: &[(MEDIA, &[Some(MEDIA_2), Some(MEDIA_1)])],
            after: &[(MEDIA, Some(MEDIA_1))],
            gone: &[],
        },
        Preparation::TwoVersions,
    )
    .await
}

/// `setup remove --version` of the unselected version: each boundary of
/// its removal.
#[tokio::test]
async fn a_version_removal_killed_anywhere_is_reported_and_finished() -> TestResult {
    kill_everywhere(
        &Scenario {
            command: ManagedCommand::RemoveMediaVersion,
            points: &REMOVAL_POINTS,
            allowed: &[(MEDIA, &[Some(MEDIA_2)])],
            after: &[(MEDIA, Some(MEDIA_2))],
            gone: &["versions-v1/ffmpeg_ffprobe--fixture-media-1"],
        },
        Preparation::TwoVersions,
    )
    .await
}

/// `setup remove <component>`: the deselection, then each version.
#[tokio::test]
async fn a_component_removal_killed_anywhere_is_reported_and_finished() -> TestResult {
    const POINTS: [FaultPoint; 8] = [
        FaultPoint::ManagedPointerRemoved,
        FaultPoint::ManagedMarkerCreated,
        FaultPoint::ManagedTombstoneWritten,
        FaultPoint::ManagedVersionFileRemoved,
        FaultPoint::ManagedUseLockRemoved,
        FaultPoint::ManagedManifestRemoved,
        FaultPoint::ManagedTombstoneRemoved,
        FaultPoint::ManagedVersionRemoved,
    ];
    kill_everywhere(
        &Scenario {
            command: ManagedCommand::RemoveMedia,
            points: &POINTS,
            allowed: &[
                (MEDIA, &[Some(MEDIA_2), None]),
                (WHISPER, &[Some(WHISPER_1)]),
            ],
            after: &[
                (MEDIA, None),
                (WHISPER, Some(WHISPER_1)),
                (MODEL, Some(MODEL_1)),
            ],
            gone: &[
                "versions-v1/ffmpeg_ffprobe--fixture-media-1",
                "versions-v1/ffmpeg_ffprobe--fixture-media-2",
            ],
        },
        Preparation::TwoVersions,
    )
    .await
}

/// `setup remove --stale-stages`: every file, folder, artifact and marker
/// of an abandoned stage, a stage killed at its creation and a half-written
/// pointer.
#[tokio::test]
async fn a_stage_sweep_killed_anywhere_is_finished_by_the_next() -> TestResult {
    kill_everywhere(
        &Scenario {
            command: ManagedCommand::SweepStages,
            points: &[
                FaultPoint::ManagedStageFileRemoved,
                FaultPoint::ManagedStageFolderRemoved,
                FaultPoint::ManagedStageArtifactRemoved,
                FaultPoint::ManagedStageMarkerRemoved,
                FaultPoint::ManagedPointerRemoved,
            ],
            allowed: &[(MEDIA, &[Some(MEDIA_2)])],
            after: &[(MEDIA, Some(MEDIA_2))],
            gone: &[],
        },
        Preparation::TwoVersionsAndLeftovers,
    )
    .await
}

/// The scenarios together reach every managed point.
#[test]
fn the_scenarios_cover_every_managed_point() {
    let named: BTreeSet<&str> = FRESH_INSTALL_POINTS
        .iter()
        .chain(REMOVAL_POINTS.iter())
        .chain(
            [
                FaultPoint::ManagedPointerRemoved,
                FaultPoint::ManagedStageFileRemoved,
            ]
            .iter(),
        )
        .map(|point| point.name())
        .collect();
    let every: BTreeSet<&str> = FaultPoint::MANAGED
        .iter()
        .map(|point| point.name())
        .collect();
    assert_eq!(named, every);
}

// ---------------------------------------------------------------------------
// Real kills by the operating system.

/// What the script below prints for each process whose command line names a
/// test's private folder: its id, `suspended` when every thread of it waits
/// suspended and `running` otherwise, and its command line, tab separated. The
/// folder arrives in the environment, never in the script text.
#[cfg(windows)]
const LIST_STRAYS: &str = "Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($env:VSIFT_STRAY_FOLDER) } | ForEach-Object { $threads = @((Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).Threads); $state = if ($threads.Count -gt 0 -and @($threads | Where-Object { $_.WaitReason -ne 'Suspended' }).Count -eq 0) { 'suspended' } else { 'running' }; '{0}{1}{2}{1}{3}' -f $_.ProcessId, [char]9, $state, $_.CommandLine }";

/// A process the killed host left behind under the test's private folder.
#[cfg(windows)]
#[derive(Debug)]
struct Stray {
    pid: u32,
    suspended: bool,
    command: String,
}

/// The processes whose command line names this test's private folder.
#[cfg(windows)]
fn strays_under(root: &TestRoot) -> TestResult<Vec<Stray>> {
    // The folder's own name is unique (process id, time and a counter); the
    // trailing separator keeps `...-6` from matching `...-60`.
    let name = root
        .parent
        .file_name()
        .ok_or("the test root has no name")?
        .to_string_lossy();
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", LIST_STRAYS])
        .env("VSIFT_STRAY_FOLDER", format!("{name}\\"))
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "listing the processes failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let mut strays = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.splitn(3, '\t');
        let (Some(pid), Some(state), Some(command)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        strays.push(Stray {
            pid: pid.trim().parse()?,
            suspended: state == "suspended",
            command: command.to_owned(),
        });
    }
    Ok(strays)
}

/// Ends the providers a killed host left suspended, and says so.
///
/// On Windows the supervisor creates a provider suspended and assigns it to
/// its kill-on-close job a moment later (`process-wrap`'s `JobObject`). A host
/// killed in that moment leaves the provider suspended for good: never
/// started, in no job, nothing ends it, and its mapped image keeps its stage
/// from being deleted, so no sweep or repair can remove that stage (known
/// limit L-129). The kill test cannot prevent the window, only clean up after
/// it, so it ends such a stray, **only a process whose command line names this
/// test's own private folder**, and prints what it ended. A provider that is
/// *not* suspended and outlives its host by more than the wait fails the test:
/// that would be a job that did not contain it.
#[cfg_attr(
    not(windows),
    allow(
        clippy::unnecessary_wraps,
        reason = "only Windows can leave a suspended provider; the other systems have nothing to end"
    )
)]
fn end_suspended_providers(root: &TestRoot, context: &str) -> TestResult {
    end_suspended_providers_within(root, context, Duration::from_secs(10))
}

/// [`end_suspended_providers`] with the wait for a running provider to go.
#[cfg_attr(
    not(windows),
    allow(
        clippy::unnecessary_wraps,
        reason = "only Windows can leave a suspended provider; the other systems have nothing to end"
    )
)]
fn end_suspended_providers_within(root: &TestRoot, context: &str, wait: Duration) -> TestResult {
    #[cfg(windows)]
    {
        let deadline = Instant::now() + wait;
        let mut reported = BTreeSet::new();
        loop {
            let strays = strays_under(root)?;
            if strays.is_empty() {
                return Ok(());
            }
            for stray in strays.iter().filter(|stray| stray.suspended) {
                if reported.insert(stray.pid) {
                    eprintln!(
                        "{context}: ending process {}, a provider the killed host left suspended (L-129): {}",
                        stray.pid, stray.command
                    );
                }
                // A refusal means the process is already going; the next
                // listing shows whether it is.
                let _ = Command::new("taskkill")
                    .args(["/F", "/PID", &stray.pid.to_string()])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "{context}: processes of the killed host were still there after {wait:?}: {strays:?}"
                )
                .into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (root, context, wait);
        Ok(())
    }
}

/// Starts a stand-in provider whose command line names `root`'s private
/// folder, as a provider in a stage does: `suspended` is how a killed host
/// leaves one (created suspended, never resumed); otherwise it runs a minute.
#[cfg(windows)]
fn start_stand_in_provider(root: &TestRoot, suspended: bool) -> TestResult<Child> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_SUSPENDED: u32 = 0x0000_0004;
    let name = root
        .parent
        .file_name()
        .ok_or("the test root has no name")?
        .to_string_lossy();
    let script = format!("Start-Sleep -Seconds 60 # {name}\\managed\\stage-0\\runtime.pending");
    let mut command = Command::new("powershell");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if suspended {
        command.creation_flags(CREATE_SUSPENDED);
    }
    Ok(command.spawn()?)
}

/// The reaper the kill test relies on really ends a provider created
/// suspended and never resumed (the state a killed host leaves), and a
/// provider that is not suspended is not ended but fails it.
#[cfg(windows)]
#[test]
fn a_provider_left_suspended_by_a_killed_host_is_ended_and_a_running_one_fails() -> TestResult {
    let root = TestRoot::new()?;

    let mut suspended = start_stand_in_provider(&root, true)?;
    let listed = strays_under(&root)?;
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(listed.iter().all(|stray| stray.suspended), "{listed:?}");
    end_suspended_providers(&root, "suspended stand-in")?;
    let status = suspended.wait()?;
    assert!(!status.success(), "the stray was not ended: {status:?}");
    assert!(strays_under(&root)?.is_empty());

    let mut running = start_stand_in_provider(&root, false)?;
    let outcome =
        end_suspended_providers_within(&root, "running stand-in", Duration::from_millis(1500));
    let still_running = running.try_wait()?.is_none();
    running.kill()?;
    running.wait()?;
    let message = outcome.err().ok_or("a running provider was not reported")?;
    assert!(
        message.to_string().contains("still there"),
        "unexpected report: {message}"
    );
    assert!(still_running, "a running provider must not be ended");
    Ok(())
}

/// Waits, with a deadline, until `ready` holds, while the child runs.
fn wait_for(child: &mut Child, ready: impl Fn() -> bool, what: &str) -> TestResult {
    let deadline = Instant::now() + KILL_WAIT;
    while !ready() {
        if child.try_wait()?.is_some() {
            return Err(format!("the child ended before {what}").into());
        }
        if Instant::now() > deadline {
            return Err(format!("{what} never happened").into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

/// The bytes of the first stage's artifact so far.
fn staged_artifact_bytes(root: &TestRoot) -> u64 {
    root.stages()
        .ok()
        .and_then(|stages| stages.into_iter().next())
        .and_then(|stage| fs::metadata(stage.join("artifact.pending")).ok())
        .map_or(0, |metadata| metadata.len())
}

/// D-03: a download killed by the operating system (`SIGKILL` on Unix,
/// `TerminateProcess` on Windows) while its body stalls leaves one
/// abandoned stage, nothing selected and a free guard; repair names the
/// sweep; the next accepted install sweeps the stage and completes.
#[tokio::test]
async fn a_download_killed_by_the_operating_system_leaves_a_stage_the_next_install_sweeps()
-> TestResult {
    let fixture = Fixture::new(MEDIA_1)?;
    let server = TestServer::start()?;
    fixture.serve_all(&server);
    let media = fixture.bytes(MEDIA_FILE)?;
    let half = media.len() / 2;
    server.route(
        &format!("/{MEDIA_FILE}"),
        vec![Reply::StallAfter {
            body: media.clone(),
            sent: half,
        }],
    );
    let root = TestRoot::new()?;
    let mut child = child_command(&root, &server, ManagedCommand::Install(1))?.spawn()?;
    let waited = wait_for(
        &mut child,
        || staged_artifact_bytes(&root) >= u64::try_from(half).unwrap_or(u64::MAX),
        "the stalled download",
    );
    child.kill()?;
    child.wait()?;
    waited?;

    let allowed = [
        Allowed {
            component: MEDIA,
            versions: &[None],
        },
        Allowed {
            component: WHISPER,
            versions: &[None],
        },
    ];
    let plan = assert_consistent(&root, &allowed, "killed download")?;
    assert_eq!(Facts::observe(&root)?.stages, 1);
    assert!(
        plan.findings
            .iter()
            .any(|finding| finding.kind == RepairFindingKind::StaleStages)
    );

    server.route(&format!("/{MEDIA_FILE}"), vec![Reply::Body(media)]);
    let done = install(&root.store()?, &server.base(), 1).await?;
    assert_eq!(done.sweep.removed, 1, "the abandoned stage was not swept");
    assert_eq!(done.statuses, ["activated"; 3]);
    assert_healthy(
        &root,
        &[
            (MEDIA, Some(MEDIA_1)),
            (WHISPER, Some(WHISPER_1)),
            (MODEL, Some(MODEL_1)),
        ],
        "after the rerun",
    )
}

/// A whole install killed by the operating system at spread moments, each
/// on a fresh root: every kill leaves a consistent store and a rerun
/// completes. The moments are fractions of one uninterrupted run, so they
/// fall in the download, stage, smoke, publication and cleanup of some
/// component on any machine.
#[tokio::test]
async fn installs_killed_by_the_operating_system_at_spread_moments_are_consistent() -> TestResult {
    const KILLS: u32 = 8;
    let fixture = Fixture::new(MEDIA_1)?;
    let server = TestServer::start()?;
    fixture.serve_all(&server);
    let timed = TestRoot::new()?;
    let started = Instant::now();
    let end = run_child(&timed, &server, ManagedCommand::Install(1), None)?;
    let whole = started.elapsed();
    assert_eq!(end, ChildEnd::Completed);
    let allowed = [
        Allowed {
            component: MEDIA,
            versions: &[None, Some(MEDIA_1)],
        },
        Allowed {
            component: WHISPER,
            versions: &[None, Some(WHISPER_1)],
        },
        Allowed {
            component: MODEL,
            versions: &[None, Some(MODEL_1)],
        },
    ];
    for kill in 1..=KILLS {
        let root = TestRoot::new()?;
        let mut child = child_command(&root, &server, ManagedCommand::Install(1))?.spawn()?;
        std::thread::sleep(whole * kill / (KILLS + 1));
        child.kill()?;
        child.wait()?;
        let context = format!("kill {kill} of {KILLS}");
        // On Windows a kill in the first moments of a provider's start leaves
        // that provider suspended and its stage undeletable (L-129); end it
        // and say so before anything tries to sweep the stage. A provider
        // runs from a stage, so a kill that left none needs no search.
        if !root.stages()?.is_empty() {
            end_suspended_providers(&root, &context)?;
        }
        assert_consistent(&root, &allowed, &context)?;
        run_command(&root.store()?, &server.base(), ManagedCommand::Install(1)).await?;
        // A kill during a smoke can leave its provider finishing for a moment
        // (on Unix it runs on, L-055; on Windows the job object ends it, its
        // image may still be mapped for a moment, and a provider caught before
        // it joined the job is ended above), so the rerun's sweep may keep
        // that stage once; repair then names it and its command removes it.
        let after = assert_consistent(&root, &allowed, &context)?;
        apply_repair(&root, &after)?;
        let healthy = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_healthy(
                &root,
                &[
                    (MEDIA, Some(MEDIA_1)),
                    (WHISPER, Some(WHISPER_1)),
                    (MODEL, Some(MODEL_1)),
                ],
                &context,
            )
        }));
        match healthy {
            Ok(result) => result?,
            Err(panic) => {
                // The store is not healthy after the repair: say what is still
                // there, so a failure that is not the leaked provider of L-129
                // can be told from it (the kill test's flake, #253, was
                // undiagnosable from its repair plan alone).
                eprintln!("{context}: {}", describe_leftovers(&root));
                std::panic::resume_unwind(panic);
            }
        }
    }
    Ok(())
}

/// What a store that is not healthy after a kill still holds: its stage
/// folders and, on Windows, the processes whose command line names its folder.
fn describe_leftovers(root: &TestRoot) -> String {
    let stages: Vec<String> = root
        .stages()
        .unwrap_or_default()
        .iter()
        .filter_map(|stage| {
            stage
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect();
    #[cfg(windows)]
    let processes = strays_under(root).map_or_else(
        |error| format!("(cannot list processes: {error})"),
        |strays| format!("{strays:?}"),
    );
    #[cfg(not(windows))]
    let processes = String::from("(not listed on this platform)");
    format!("stages left: {stages:?}; processes under the folder: {processes}")
}
