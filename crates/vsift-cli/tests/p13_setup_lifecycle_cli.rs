//! P13 PR 6: `setup list`, `setup rollback`, `setup remove` and `setup
//! repair` through the compiled binary, headless and offline, on every
//! platform.
//!
//! Every invocation runs with an empty `PATH`, a fresh per-user base, no
//! terminal and a deadline. Stand-in versions are published into the base's
//! managed root through the store, as `setup install` publishes them; the
//! commands never run them. On Ubuntu 24.04 x86-64 one more test proves
//! that `setup install` sweeps an abandoned stage once its plan is accepted.

use std::{
    env,
    error::Error,
    fmt::Write as _,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use vsift_domain::{ArtifactIntegrity, ManagedComponent, ManagedTarget};
use vsift_infrastructure::{
    ArchiveInventoryBounds, ManagedArtifactStore, ManagedRuntimeIdentity, ReviewedArchiveFile,
    ReviewedPayloadArchive, ReviewedRuntimeLayout, StagedManagedCandidate, detect_managed_target,
};

type TestResult = Result<(), Box<dyn Error>>;
/// One file of a snapshot: its path and bytes.
type SnapshotFile = (PathBuf, Vec<u8>);

const CLI_DEADLINE: Duration = Duration::from_secs(120);
const OLD: &str = "whisper.cpp-v1.9.1-ubuntu-x64";
const NEW: &str = "whisper.cpp-v1.9.2-ubuntu-x64";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A fresh per-user base for one test, removed on drop.
struct Base(PathBuf);

impl Base {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!(
            "vsift-p13-lifecycle-cli-{}-{stamp}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    /// Where the binary puts its managed root under this base.
    fn managed_root(&self) -> PathBuf {
        #[cfg(windows)]
        let base = self.0.join("local");
        #[cfg(target_os = "macos")]
        let base = self.0.join("Library/Application Support");
        #[cfg(all(unix, not(target_os = "macos")))]
        let base = self.0.join("data");
        base.join("vsift/managed-v1")
    }

    fn store(&self) -> Result<ManagedArtifactStore, Box<dyn Error>> {
        Ok(ManagedArtifactStore::at(self.managed_root())?)
    }

    fn vsift(&self) -> Result<Command, Box<dyn Error>> {
        let mut command = Command::cargo_bin("vsift")?;
        command
            .env("PATH", "")
            .env("HOME", &self.0)
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env("XDG_CACHE_HOME", self.0.join("cache"))
            .env("LOCALAPPDATA", self.0.join("local"))
            .env("APPDATA", self.0.join("roaming"))
            .timeout(CLI_DEADLINE)
            .write_stdin("");
        Ok(command)
    }

    fn run(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        Ok(self.vsift()?.args(arguments).output()?)
    }

    fn version_path(&self, version: &str) -> PathBuf {
        self.managed_root()
            .join("versions-v1")
            .join(format!("whisper_cli--{version}"))
    }
}

impl Drop for Base {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn qualified() -> bool {
    detect_managed_target() == ManagedTarget::Ubuntu2404X86_64
}

fn json(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn schema(name: &str) -> Result<Value, Box<dyn Error>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v1")
        .join(name);
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn validate(name: &str, instance: &Value) -> TestResult {
    jsonschema::validator_for(&schema(name)?)?
        .validate(instance)
        .map_err(|error| std::io::Error::other(format!("{name}: {error}")))?;
    Ok(())
}

/// The result, validated as an envelope and its data against `data_schema`.
fn result(output: &Output, data_schema: &str) -> Result<Value, Box<dyn Error>> {
    let value = json(output)?;
    validate("operation-response.schema.json", &value)?;
    if !value["data"].is_null() {
        validate(data_schema, &value["data"])?;
    }
    Ok(value)
}

fn integrity_of(bytes: &[u8]) -> Result<ArtifactIntegrity, Box<dyn Error>> {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(hex, "{byte:02x}")?;
    }
    Ok(ArtifactIntegrity::from_sha256_hex(
        u64::try_from(bytes.len())?,
        &hex,
    )?)
}

/// Publishes and selects a stand-in `version` of the whisper.cpp CLI.
fn publish(store: &ManagedArtifactStore, version: &str) -> TestResult {
    let contents = format!("stand-in whisper-cli {version}").into_bytes();
    let mut archive = Builder::new(Vec::new());
    let mut header = Header::new_gnu();
    header.set_path("root/whisper-cli")?;
    header.set_size(u64::try_from(contents.len())?);
    header.set_mode(0o755);
    header.set_cksum();
    archive.append(&header, Cursor::new(&contents))?;
    let bytes = archive.into_inner()?;
    let staged = store.import_verified(&bytes[..], integrity_of(&bytes)?)?;
    let candidate = StagedManagedCandidate::prepare_archive(
        staged,
        ManagedComponent::WhisperCli,
        &[],
        ReviewedPayloadArchive::Tar {
            max_tar_bytes: u64::try_from(bytes.len())?,
        },
        ArchiveInventoryBounds::new(1, u64::try_from(contents.len())?)?,
        &[],
        &[ReviewedArchiveFile {
            path: "root/whisper-cli",
            integrity: integrity_of(&contents)?,
        }],
        ReviewedRuntimeLayout {
            max_bytes: u64::try_from(contents.len())?,
            aliases: &[],
            executables: &["whisper-cli"],
        },
    )?;
    let guard = store.try_install_guard()?;
    let identity = ManagedRuntimeIdentity::new("whisper_cli", version)?;
    let (published, _) = candidate.publish_and_select(&guard, &identity);
    published?;
    Ok(())
}

/// An abandoned download: a stage whose run ended before it was discarded.
fn abandon_stage(store: &ManagedArtifactStore) -> TestResult {
    let bytes = b"abandoned artifact".to_vec();
    drop(store.import_verified(&bytes[..], integrity_of(&bytes)?)?);
    Ok(())
}

fn stages(root: &Path) -> Result<usize, Box<dyn Error>> {
    Ok(fs::read_dir(root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("stage-"))
        .count())
}

/// Every file under `path` with its bytes, to prove a command changed nothing.
fn snapshot(path: &Path) -> Result<Vec<SnapshotFile>, Box<dyn Error>> {
    let mut files = Vec::new();
    let mut pending = vec![path.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                files.push((entry.path(), fs::read(entry.path())?));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// With nothing installed every command answers, typed, on every platform,
/// and none creates the managed folder.
#[test]
fn with_nothing_installed_every_command_answers_and_creates_nothing() -> TestResult {
    let base = Base::new()?;
    let list = base.run(&["setup", "list", "--json"])?;
    assert_eq!(list.status.code(), Some(0));
    let list = result(&list, "setup-list.schema.json")?;
    assert_eq!(list["data"]["managed_folder"], "absent");
    assert_eq!(
        list["data"]["managed_install"],
        if qualified() {
            "catalogue_accepted"
        } else {
            "unavailable_target"
        }
    );
    let repair = base.run(&["setup", "repair", "--json"])?;
    assert_eq!(repair.status.code(), Some(0));
    assert_eq!(
        result(&repair, "setup-repair.schema.json")?["data"]["status"],
        "nothing_installed"
    );
    let rollback = base.run(&["setup", "rollback", "whisper_cli", "--json"])?;
    assert_eq!(rollback.status.code(), Some(2));
    let rollback = result(&rollback, "setup-rollback.schema.json")?;
    assert_eq!(rollback["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(
        rollback["error"]["remediation"][0]["command"]["arguments"],
        serde_json::json!(["setup", "list"])
    );
    for arguments in [
        &["setup", "remove", "whisper_model", "--json"][..],
        &["setup", "remove", "--stale-stages", "--json"][..],
    ] {
        let output = base.run(arguments)?;
        assert_eq!(output.status.code(), Some(0), "{arguments:?}");
        result(&output, "setup-remove.schema.json")?;
    }
    for arguments in [&["setup", "list"][..], &["setup", "repair"][..]] {
        let human = base.run(arguments)?;
        assert_eq!(human.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&human.stdout).starts_with("VSift setup"));
    }
    assert!(
        !base.managed_root().exists(),
        "a lifecycle command created the managed folder"
    );
    Ok(())
}

/// List, roll back and forward, refuse to remove the selected version,
/// remove the other, then the component; every result in every output mode
/// validates.
#[test]
fn list_rollback_and_remove_through_the_binary() -> TestResult {
    let base = Base::new()?;
    let store = base.store()?;
    publish(&store, OLD)?;
    publish(&store, NEW)?;

    let list = result(
        &base.run(&["setup", "list", "--json"])?,
        "setup-list.schema.json",
    )?;
    let cli = &list["data"]["components"][1];
    assert_eq!(cli["component"], "whisper_cli");
    assert_eq!(cli["selection"], "verified");
    assert_eq!(cli["selected_version"], NEW);
    assert_eq!(cli["previous_version"], OLD);

    let back = base.run(&["setup", "rollback", "whisper_cli", "--json"])?;
    assert_eq!(back.status.code(), Some(0));
    let back = result(&back, "setup-rollback.schema.json")?;
    assert_eq!(back["data"]["status"], "rolled_back");
    assert_eq!(back["data"]["selected_version"], OLD);
    assert_eq!(back["data"]["replaced_version"], NEW);

    let events = base.run(&["setup", "rollback", "whisper_cli", "--events", "jsonl"])?;
    assert_eq!(events.status.code(), Some(0));
    let stdout = String::from_utf8(events.stdout)?;
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1);
    let terminal: Value = serde_json::from_str(lines[0])?;
    validate("terminal-event.schema.json", &terminal)?;
    assert_eq!(terminal["result"]["data"]["selected_version"], NEW);

    let human = base.run(&["setup", "rollback", "whisper_cli", "--version", OLD])?;
    assert_eq!(human.status.code(), Some(0));
    let text = String::from_utf8(human.stdout)?;
    assert!(text.contains("whisper_cli: rolled_back, selected whisper.cpp-v1.9.1-ubuntu-x64"));

    let selected = base.run(&["setup", "remove", "whisper_cli", "--version", OLD, "--json"])?;
    assert_eq!(selected.status.code(), Some(2));
    assert!(base.version_path(OLD).is_dir());
    let removed = base.run(&["setup", "remove", "whisper_cli", "--version", NEW, "--json"])?;
    assert_eq!(removed.status.code(), Some(0));
    let removed = result(&removed, "setup-remove.schema.json")?;
    assert_eq!(removed["data"]["versions"][0]["status"], "removed");
    assert!(!base.version_path(NEW).exists());

    let component = base.run(&["setup", "remove", "whisper_cli"])?;
    assert_eq!(component.status.code(), Some(0));
    let text = String::from_utf8(component.stdout)?;
    assert!(text.contains("Selection removed"), "{text}");
    assert!(!base.version_path(OLD).exists());
    Ok(())
}

/// A held install guard makes a change `BUSY` at once and never waits, while
/// list and repair still read; a version a job holds is kept, reported with
/// the failure, and removed by the rerun once the job lets go.
#[test]
fn busy_guards_and_versions_in_use_are_respected() -> TestResult {
    let base = Base::new()?;
    let store = base.store()?;
    publish(&store, OLD)?;
    publish(&store, NEW)?;

    let guard = store.try_install_guard()?;
    for arguments in [
        &["setup", "rollback", "whisper_cli", "--json"][..],
        &["setup", "remove", "whisper_cli", "--json"][..],
        &["setup", "remove", "--stale-stages", "--json"][..],
    ] {
        let output = base.run(arguments)?;
        assert_eq!(output.status.code(), Some(4), "{arguments:?}");
        let value = json(&output)?;
        assert_eq!(value["error"]["code"], "BUSY");
        assert_eq!(value["error"]["retry_after_ms"], 30_000);
    }
    assert_eq!(
        base.run(&["setup", "list", "--json"])?.status.code(),
        Some(0)
    );
    assert_eq!(
        base.run(&["setup", "repair", "--json"])?.status.code(),
        Some(0)
    );
    drop(guard);

    // A job holds the selected version, as lookup hands it to one.
    let held = store
        .open_selected_runtime("whisper_cli")?
        .ok_or("nothing selected")?;
    let output = base.run(&["setup", "remove", "whisper_cli", "--json"])?;
    assert_eq!(output.status.code(), Some(4));
    let value = result(&output, "setup-remove.schema.json")?;
    assert_eq!(value["error"]["code"], "BUSY");
    assert_eq!(value["data"]["deselected"], true);
    let statuses: Vec<&str> = value["data"]["versions"]
        .as_array()
        .ok_or("no versions")?
        .iter()
        .filter_map(|version| version["status"].as_str())
        .collect();
    assert_eq!(statuses, ["removed", "in_use"]);
    assert!(base.version_path(NEW).is_dir());

    let human = base.run(&["setup", "remove", "whisper_cli"])?;
    assert_eq!(human.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&human.stdout).contains("[in_use] whisper_cli"));
    assert!(String::from_utf8_lossy(&human.stderr).contains("(BUSY)"));
    drop(held);

    let rerun = base.run(&["setup", "remove", "whisper_cli", "--json"])?;
    assert_eq!(rerun.status.code(), Some(0));
    assert!(!base.version_path(NEW).exists());
    Ok(())
}

/// Repair diagnoses a corrupted selection and an abandoned stage, names the
/// commands that fix them, and changes nothing; applying them fixes it.
#[test]
fn repair_changes_nothing_and_its_plan_fixes_the_store() -> TestResult {
    let base = Base::new()?;
    let store = base.store()?;
    publish(&store, OLD)?;
    publish(&store, NEW)?;
    fs::write(base.version_path(NEW).join("whisper-cli"), b"bit rot")?;
    abandon_stage(&store)?;
    let before = snapshot(&base.managed_root())?;

    let repair = base.run(&["setup", "repair", "--json"])?;
    assert_eq!(repair.status.code(), Some(0));
    let repair = result(&repair, "setup-repair.schema.json")?;
    assert_eq!(
        snapshot(&base.managed_root())?,
        before,
        "repair changed the store"
    );
    assert_eq!(repair["data"]["status"], "needs_repair");
    let commands: Vec<Vec<String>> = repair["data"]["findings"]
        .as_array()
        .ok_or("no findings")?
        .iter()
        .filter_map(|finding| {
            finding["command"]["arguments"].as_array().map(|arguments| {
                arguments
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
        })
        .collect();
    assert_eq!(
        commands,
        [
            vec!["setup", "rollback", "whisper_cli"],
            vec!["setup", "remove", "--stale-stages"]
        ]
    );
    let list = result(
        &base.run(&["setup", "list", "--json"])?,
        "setup-list.schema.json",
    )?;
    assert_eq!(list["data"]["components"][1]["selection"], "unverified");
    assert_eq!(list["data"]["stale_stages"], 1);

    for command in &commands {
        let mut arguments: Vec<&str> = command.iter().map(String::as_str).collect();
        arguments.push("--json");
        assert_eq!(base.run(&arguments)?.status.code(), Some(0), "{command:?}");
    }
    assert_eq!(stages(&base.managed_root())?, 0);
    let after = result(
        &base.run(&["setup", "repair", "--json"])?,
        "setup-repair.schema.json",
    )?;
    let fixes: Vec<&str> = after["data"]["findings"]
        .as_array()
        .ok_or("no findings")?
        .iter()
        .filter_map(|finding| finding["fix"].as_str())
        .collect();
    assert_eq!(fixes, ["remove_version"]);
    Ok(())
}

/// A version that is not a canonical key never reaches the store or the
/// output: the parser rejects it and the result does not echo it.
#[test]
fn a_hostile_version_is_rejected_by_the_parser_and_never_echoed() -> TestResult {
    let base = Base::new()?;
    for arguments in [
        &[
            "setup",
            "rollback",
            "whisper_cli",
            "--version",
            "../../etc",
            "--json",
        ][..],
        &[
            "setup",
            "remove",
            "whisper_cli",
            "--version",
            "A\u{202e}B",
            "--json",
        ][..],
        &["setup", "remove", "--stale-stages", "whisper_cli", "--json"][..],
        &["setup", "remove", "--json"][..],
    ] {
        let output = base.run(arguments)?;
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        let value = json(&output)?;
        validate("operation-response.schema.json", &value)?;
        assert_eq!(value["command"], "parse");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("etc") && !stdout.contains('\u{202e}'),
            "{stdout}"
        );
    }
    assert!(!base.managed_root().exists());
    Ok(())
}

/// Ubuntu 24.04 x86-64: once the plan is accepted, `setup install` sweeps
/// the stages earlier runs abandoned before it stages anything, and reports
/// it; the transaction itself then stops on an empty artifact folder.
#[test]
fn setup_install_sweeps_abandoned_stages_once_the_plan_is_accepted() -> TestResult {
    if !qualified() {
        println!("skipped: managed installation runs on Ubuntu 24.04 x86-64 only");
        return Ok(());
    }
    let base = Base::new()?;
    let plan = base.run(&["setup", "plan", "--profile", "desktop", "--json"])?;
    assert!(plan.status.success(), "setup plan failed");
    let digest = json(&plan)?["data"]["plan_digest"]
        .as_str()
        .ok_or("no digest")?
        .to_owned();
    let plan_path = base.0.join("plan.json");
    fs::write(&plan_path, &plan.stdout)?;
    let artifacts = base.0.join("artifacts");
    fs::create_dir(&artifacts)?;
    let store = base.store()?;
    abandon_stage(&store)?;
    abandon_stage(&store)?;
    assert_eq!(stages(&base.managed_root())?, 2);

    let output = base
        .vsift()?
        .args(["setup", "install", "--plan"])
        .arg(&plan_path)
        .args(["--accept-plan", &digest, "--artifact-dir"])
        .arg(&artifacts)
        .arg("--json")
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let value = result(&output, "setup-install.schema.json")?;
    assert_eq!(value["data"]["components"][0]["reason"], "artifact_missing");
    assert_eq!(value["data"]["cleanup"]["stale_stages_removed"], 2);
    assert_eq!(value["data"]["cleanup"]["stale_stages_retained"], 0);
    assert_eq!(stages(&base.managed_root())?, 0);
    Ok(())
}
