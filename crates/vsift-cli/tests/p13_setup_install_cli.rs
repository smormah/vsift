//! P13 PR 4: `setup install` through the compiled binary, headless, with no
//! network (D-02, D-07, D-09, D-10).
//!
//! Every invocation runs with an empty `PATH`, a fresh per-user base, no
//! terminal (standard input is empty) and a deadline, so a prompt or a hang
//! fails the test. On a host other than Ubuntu 24.04 x86-64 managed
//! installation is unavailable: the tests prove it creates nothing and
//! gives the manual path. On Ubuntu 24.04 (the Linux CI runner) they drive
//! the real compiled catalogue up to its first download, which a local
//! proxy that demands authentication stops before any byte leaves the
//! machine, and prove the busy guard, a denied managed folder, an offline
//! folder without the artifacts and that proxy credentials never appear in
//! any output. Stand-in versions published under the catalogue's identities
//! prove that `setup plan` shows installed components and that the accepted
//! plan survives a partial and a complete install.

use std::{
    env,
    error::Error,
    fmt::Write as _,
    fs,
    io::{BufRead, BufReader, Cursor, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use vsift_domain::{ArtifactIntegrity, ManagedComponent, ManagedTarget};
use vsift_infrastructure::{
    ArchiveInventoryBounds, ManagedArtifactStore, ManagedRuntimeIdentity, ManagedRuntimeRole,
    ReviewedArchiveFile, ReviewedPayloadArchive, ReviewedRuntimeLayout, StagedManagedCandidate,
    detect_managed_target, managed_executable_name,
};

type TestResult = Result<(), Box<dyn Error>>;

/// Upper bound for one invocation: a prompt or a hang is killed and fails.
const CLI_DEADLINE: Duration = Duration::from_secs(120);
const SENTINEL: &str = "vsift-sentinel-proxy-credential-5d2e";
const PROXY_VARIABLES: [&str; 8] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A fresh per-user base for one test, removed on drop.
struct Base(PathBuf);

impl Base {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!(
            "vsift-p13-install-cli-{}-{stamp}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    /// Where the binary puts its managed root under this base.
    fn managed_root(&self) -> PathBuf {
        #[cfg(windows)]
        let base = self.0.join("local");
        #[cfg(target_os = "macos")]
        let base = self.0.join("Library/Application Support");
        #[cfg(all(unix, not(target_os = "macos")))]
        let base = self.data();
        base.join("vsift/managed-v1")
    }

    /// The binary with this base, an empty `PATH`, no proxy and no stdin.
    fn vsift(&self) -> Result<Command, Box<dyn Error>> {
        let mut command = Command::cargo_bin("vsift")?;
        command
            .env("PATH", "")
            .env("HOME", &self.0)
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("XDG_DATA_HOME", self.data())
            .env("XDG_CACHE_HOME", self.0.join("cache"))
            .env("LOCALAPPDATA", self.0.join("local"))
            .env("APPDATA", self.0.join("roaming"))
            .timeout(CLI_DEADLINE)
            .write_stdin("");
        for variable in PROXY_VARIABLES {
            command.env_remove(variable);
        }
        Ok(command)
    }

    /// Saves `setup plan --profile desktop --json` and returns its path
    /// and digest.
    fn saved_plan(&self) -> Result<(PathBuf, Option<String>), Box<dyn Error>> {
        let output = self
            .vsift()?
            .args(["setup", "plan", "--profile", "desktop", "--json"])
            .output()?;
        assert!(output.status.success(), "setup plan failed");
        let plan: Value = serde_json::from_slice(&output.stdout)?;
        let path = self.0.join("plan.json");
        fs::write(&path, &output.stdout)?;
        Ok((
            path,
            plan["data"]["plan_digest"].as_str().map(str::to_owned),
        ))
    }
}

impl Drop for Base {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = fs::set_permissions(self.data(), fs::Permissions::from_mode(0o700));
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn qualified() -> bool {
    detect_managed_target() == ManagedTarget::Ubuntu2404X86_64
}

fn path_text(path: &Path) -> Result<&str, Box<dyn Error>> {
    path.to_str().ok_or_else(|| "non-UTF-8 test path".into())
}

fn json(output: &std::process::Output) -> Result<Value, Box<dyn Error>> {
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

fn summary(value: &Value) -> &str {
    value["error"]["remediation"][0]["summary"]
        .as_str()
        .unwrap_or_default()
}

/// D-02, D-10: a saved plan of an unknown version, an edited plan, a wrong
/// digest and a relative artifact folder are refused with a remediation
/// and change nothing; off the managed target there is nothing to accept
/// and the remediation is the manual path.
#[test]
fn refused_acceptance_changes_nothing() -> TestResult {
    let base = Base::new()?;
    let (plan, digest) = base.saved_plan()?;
    let digest = digest.unwrap_or_else(|| "0".repeat(64));
    let plan_text = path_text(&plan)?.to_owned();

    let wrong = base
        .vsift()?
        .args(["setup", "install", "--plan", &plan_text, "--accept-plan"])
        .arg("0".repeat(64))
        .arg("--json")
        .output()?;
    assert_eq!(wrong.status.code(), Some(2));
    let value = json(&wrong)?;
    validate("operation-response.schema.json", &value)?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    if qualified() {
        assert!(summary(&value).contains("Run setup plan --json again"));
    } else {
        assert!(summary(&value).contains("setup configure"));
        assert!(summary(&value).contains("Ubuntu 24.04"));
    }

    let mut edited: Value = serde_json::from_str(&fs::read_to_string(&plan)?)?;
    edited["schema_version"] = Value::from("2");
    let unknown_version = base.0.join("unknown-version.json");
    fs::write(&unknown_version, serde_json::to_vec(&edited)?)?;
    let mut changed: Value = serde_json::from_str(&fs::read_to_string(&plan)?)?;
    changed["data"]["catalogue_revision"] = Value::from("ubuntu-24.04-x86_64-2099-01-01-r9");
    let changed_revision = base.0.join("changed-revision.json");
    fs::write(&changed_revision, serde_json::to_vec(&changed)?)?;
    for edited in [&unknown_version, &changed_revision] {
        let output = base
            .vsift()?
            .args(["setup", "install", "--plan", path_text(edited)?])
            .args(["--accept-plan", &digest, "--json"])
            .output()?;
        assert_eq!(output.status.code(), Some(2), "{}", edited.display());
        assert_eq!(json(&output)?["error"]["code"], "INVALID_ARGUMENT");
    }

    let relative = base
        .vsift()?
        .args(["setup", "install", "--plan", &plan_text])
        .args([
            "--accept-plan",
            &digest,
            "--artifact-dir",
            "artifacts",
            "--json",
        ])
        .output()?;
    let value = json(&relative)?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert!(summary(&value).contains("--artifact-dir must be the absolute path"));

    // The install guard is taken before the plan is revalidated, so on the
    // managed target the refusals may leave the empty marked root; nothing
    // is ever staged or published. Elsewhere nothing is created at all.
    let root = base.managed_root();
    if qualified() {
        assert!(!root.join("versions-v1").exists());
        assert!(!root.join("current-v1").exists());
    } else {
        assert!(!root.exists(), "a refusal created the root");
    }
    Ok(())
}

/// A local HTTP proxy that answers every request with `407` and counts
/// the requests that carried credentials.
fn start_proxy() -> Result<u16, Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || {
                let Ok(clone) = stream.try_clone() else {
                    return;
                };
                let mut reader = BufReader::new(clone);
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                    line.clear();
                }
                let mut stream = stream;
                let _ = stream.write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"vsift-test\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            });
        }
    });
    Ok(port)
}

/// D-07, D-10: on the managed target, a proxy that demands authentication
/// stops the first download with `DOWNLOAD_FAILED` (`proxy_auth`), headless
/// in every output mode; the proxy credentials from the environment appear
/// in no output, and the `--events jsonl` stream is schema-valid progress
/// then the terminal failure with every component.
#[test]
fn a_proxy_demanding_authentication_fails_the_download_without_leaking_credentials() -> TestResult {
    if !qualified() {
        println!("skipped: managed installation runs on Ubuntu 24.04 x86-64 only");
        return Ok(());
    }
    let base = Base::new()?;
    let (plan, digest) = base.saved_plan()?;
    let digest = digest.ok_or("the qualified plan has no digest")?;
    let proxy = format!("http://vsift-user:{SENTINEL}@127.0.0.1:{}", start_proxy()?);
    for mode in [&["--json"][..], &["--events", "jsonl"][..], &[][..]] {
        let mut command = base.vsift()?;
        for variable in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
            command.env(variable, &proxy);
        }
        let output = command
            .args(["setup", "install", "--plan", path_text(&plan)?])
            .args(["--accept-plan", &digest])
            .args(mode)
            .output()?;
        assert_eq!(output.status.code(), Some(7), "{mode:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stdout.contains(SENTINEL) && !stderr.contains(SENTINEL));
        assert!(!stdout.contains("vsift-user") && !stderr.contains("vsift-user"));
        let terminal = match mode {
            ["--json"] => json(&output)?,
            ["--events", "jsonl"] => {
                let lines: Vec<Value> = stdout
                    .lines()
                    .map(serde_json::from_str)
                    .collect::<Result<_, _>>()?;
                let (last, progress) = lines.split_last().ok_or("no events")?;
                for event in progress {
                    validate("progress-event.schema.json", event)?;
                    assert_eq!(event["command"], "setup.install");
                }
                validate("terminal-event.schema.json", last)?;
                last["result"].clone()
            }
            _ => {
                assert!(stdout.contains("[failed] ffmpeg_ffprobe"), "{stdout}");
                assert!(stderr.contains("DOWNLOAD_FAILED"), "{stderr}");
                continue;
            }
        };
        validate("operation-response.schema.json", &terminal)?;
        validate("setup-install.schema.json", &terminal["data"])?;
        assert_eq!(terminal["error"]["code"], "DOWNLOAD_FAILED");
        let components = terminal["data"]["components"]
            .as_array()
            .ok_or("no components")?;
        assert_eq!(components.len(), 3);
        assert_eq!(components[0]["reason"], "proxy_auth");
        assert_eq!(components[0]["step"], "download");
        assert_eq!(components[1]["reason"], "blocked");
        assert!(summary(&terminal).contains("--artifact-dir"));
    }
    let root = base.managed_root();
    assert!(!root.join("versions-v1").exists());
    for entry in fs::read_dir(&root)? {
        let name = entry?.file_name();
        assert!(
            !name.to_string_lossy().starts_with("stage-"),
            "a stage was left behind"
        );
    }
    Ok(())
}

/// D-10: a second installation never waits for the first: it answers
/// `BUSY` with a retry hint and changes nothing.
#[test]
fn a_held_install_guard_is_busy_at_once() -> TestResult {
    if !qualified() {
        println!("skipped: managed installation runs on Ubuntu 24.04 x86-64 only");
        return Ok(());
    }
    let base = Base::new()?;
    let (plan, digest) = base.saved_plan()?;
    let digest = digest.ok_or("the qualified plan has no digest")?;
    let store = ManagedArtifactStore::at(base.managed_root())?;
    let guard = store.try_install_guard()?;
    let output = base
        .vsift()?
        .args(["setup", "install", "--plan", path_text(&plan)?])
        .args(["--accept-plan", &digest, "--json"])
        .output()?;
    drop(guard);
    assert_eq!(output.status.code(), Some(4));
    let value = json(&output)?;
    assert_eq!(value["error"]["code"], "BUSY");
    assert_eq!(value["error"]["retry_after_ms"], 30_000);
    assert!(
        summary(&value)
            .contains("Another setup install, setup rollback or setup remove is running")
    );
    Ok(())
}

/// D-09, D-10: a managed folder that cannot be written gives `STORAGE_IO`
/// and the manual path, once, with nothing retried or elevated.
#[cfg(unix)]
#[test]
fn a_denied_managed_folder_gives_the_manual_path() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    if !qualified() {
        println!("skipped: managed installation runs on Ubuntu 24.04 x86-64 only");
        return Ok(());
    }
    let base = Base::new()?;
    let (plan, digest) = base.saved_plan()?;
    let digest = digest.ok_or("the qualified plan has no digest")?;
    fs::create_dir_all(base.data())?;
    fs::set_permissions(base.data(), fs::Permissions::from_mode(0o500))?;
    let output = base
        .vsift()?
        .args(["setup", "install", "--plan", path_text(&plan)?])
        .args(["--accept-plan", &digest, "--json"])
        .output()?;
    assert_eq!(output.status.code(), Some(7));
    let value = json(&output)?;
    assert_eq!(value["error"]["code"], "STORAGE_IO");
    assert!(summary(&value).contains("setup configure ffmpeg|ffprobe|whisper"));
    assert!(!base.managed_root().exists());
    Ok(())
}

/// D-07: an offline folder without the plan's artifacts stops at the first
/// component with `artifact_missing` and no request.
#[test]
fn an_offline_folder_without_the_artifacts_is_refused() -> TestResult {
    if !qualified() {
        println!("skipped: managed installation runs on Ubuntu 24.04 x86-64 only");
        return Ok(());
    }
    let base = Base::new()?;
    let (plan, digest) = base.saved_plan()?;
    let digest = digest.ok_or("the qualified plan has no digest")?;
    let folder = base.0.join("artifacts");
    fs::create_dir(&folder)?;
    let output = base
        .vsift()?
        .args(["setup", "install", "--plan", path_text(&plan)?])
        .args([
            "--accept-plan",
            &digest,
            "--artifact-dir",
            path_text(&folder)?,
        ])
        .arg("--json")
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let value = json(&output)?;
    validate("setup-install.schema.json", &value["data"])?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(value["data"]["source"], "artifact_directory");
    assert_eq!(value["data"]["components"][0]["reason"], "artifact_missing");
    assert_eq!(value["data"]["components"][0]["step"], "import");
    Ok(())
}

fn integrity_of(bytes: &[u8]) -> Result<ArtifactIntegrity, Box<dyn Error>> {
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(hex, "{byte:02x}")?;
    }
    Ok(ArtifactIntegrity::from_sha256_hex(
        u64::try_from(bytes.len())?,
        &hex,
    )?)
}

fn executable_name(role: ManagedRuntimeRole) -> Result<String, Box<dyn Error>> {
    managed_executable_name(role).ok_or_else(|| "no executable name".into())
}

/// Publishes and selects a stand-in of `component` at `version` in the
/// binary's managed root: executables that exit 0 (so the plan's probes
/// pass) or a model file, never the reviewed bytes, with nothing fetched.
fn publish_stand_in(base: &Base, component: ManagedComponent, version: &str) -> TestResult {
    let (roles, executables) = match component {
        ManagedComponent::MediaTools => (
            vec![
                (
                    ManagedRuntimeRole::Ffmpeg,
                    executable_name(ManagedRuntimeRole::Ffmpeg)?,
                ),
                (
                    ManagedRuntimeRole::Ffprobe,
                    executable_name(ManagedRuntimeRole::Ffprobe)?,
                ),
            ],
            true,
        ),
        ManagedComponent::WhisperCli => (
            vec![(
                ManagedRuntimeRole::WhisperCli,
                executable_name(ManagedRuntimeRole::WhisperCli)?,
            )],
            true,
        ),
        ManagedComponent::WhisperModel => (
            vec![(ManagedRuntimeRole::SpeechModel, String::from("model.bin"))],
            false,
        ),
    };
    let contents: &[u8] = if executables {
        b"#!/bin/sh\nexit 0\n"
    } else {
        b"stand-in model weights"
    };
    let mut archive = Builder::new(Vec::new());
    let mut total = 0_u64;
    let mut selected = Vec::new();
    for (_, name) in &roles {
        let path = format!("root/{name}");
        let mut header = Header::new_gnu();
        header.set_path(&path)?;
        header.set_size(u64::try_from(contents.len())?);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append(&header, Cursor::new(contents))?;
        total += u64::try_from(contents.len())?;
        selected.push((path, integrity_of(contents)?));
    }
    let bytes = archive.into_inner()?;
    let store = ManagedArtifactStore::at(base.managed_root())?;
    let staged = store.import_verified(&bytes[..], integrity_of(&bytes)?)?;
    let reviewed: Vec<ReviewedArchiveFile<'_>> = selected
        .iter()
        .map(|(path, integrity)| ReviewedArchiveFile {
            path,
            integrity: *integrity,
        })
        .collect();
    let role_names: Vec<(ManagedRuntimeRole, &str)> = roles
        .iter()
        .map(|(role, name)| (*role, name.as_str()))
        .collect();
    let executable_names: Vec<&str> = if executables {
        roles.iter().map(|(_, name)| name.as_str()).collect()
    } else {
        Vec::new()
    };
    let candidate = StagedManagedCandidate::prepare_archive(
        staged,
        component,
        &role_names,
        ReviewedPayloadArchive::Tar {
            max_tar_bytes: u64::try_from(bytes.len())?,
        },
        ArchiveInventoryBounds::new(roles.len(), total)?,
        &[],
        &reviewed,
        ReviewedRuntimeLayout {
            max_bytes: total,
            aliases: &[],
            executables: &executable_names,
        },
    )?;
    let guard = store.try_install_guard()?;
    let identity = ManagedRuntimeIdentity::new(component.identifier(), version)?;
    let (published, _) = candidate.publish_and_select(&guard, &identity);
    published?;
    Ok(())
}

fn plan_now(base: &Base) -> Result<Value, Box<dyn Error>> {
    let output = base
        .vsift()?
        .args(["setup", "plan", "--profile", "desktop", "--json"])
        .output()?;
    assert!(output.status.success(), "setup plan failed");
    let value = json(&output)?;
    validate("operation-response.schema.json", &value)?;
    validate("setup-plan.schema.json", &value)?;
    Ok(value)
}

fn action_states(plan: &Value) -> Vec<String> {
    plan["data"]["actions"]
        .as_array()
        .map(|actions| {
            actions
                .iter()
                .map(|action| action["state"].as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn action_version(plan: &Value, index: usize) -> Result<String, Box<dyn Error>> {
    plan["data"]["actions"][index]["version"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "action without a version".into())
}

/// P13 PR 4: the plan shows what is installed, and the plan the user
/// accepted before installing stays acceptable. With the media tools
/// installed the plan shows them current and the same accepted plan
/// continues (the next component's artifact is missing from an empty
/// offline folder, so nothing is requested); with every component installed
/// the same plan is all `already_current`, and `setup plan` reports every
/// component current, readiness `ready` and nothing to install, under the
/// same digest.
#[test]
fn the_plan_shows_installed_components_and_its_acceptance_survives_the_install() -> TestResult {
    if !qualified() {
        println!("skipped: managed installation runs on Ubuntu 24.04 x86-64 only");
        return Ok(());
    }
    let base = Base::new()?;
    let (plan, digest) = base.saved_plan()?;
    let digest = digest.ok_or("the qualified plan has no digest")?;
    let saved: Value = serde_json::from_str(&fs::read_to_string(&plan)?)?;
    assert_eq!(saved["data"]["install_needed"], true);
    assert_eq!(action_states(&saved), ["pending", "pending", "pending"]);
    let folder = base.0.join("artifacts");
    fs::create_dir(&folder)?;
    let install = |base: &Base| -> Result<std::process::Output, Box<dyn Error>> {
        Ok(base
            .vsift()?
            .args(["setup", "install", "--plan", path_text(&plan)?])
            .args([
                "--accept-plan",
                &digest,
                "--artifact-dir",
                path_text(&folder)?,
            ])
            .arg("--json")
            .output()?)
    };

    publish_stand_in(
        &base,
        ManagedComponent::MediaTools,
        &action_version(&saved, 0)?,
    )?;
    let partial = plan_now(&base)?;
    assert_eq!(
        partial["data"]["plan_digest"].as_str(),
        Some(digest.as_str())
    );
    assert_eq!(partial["data"]["install_needed"], true);
    assert_eq!(action_states(&partial), ["current", "pending", "pending"]);
    let continued = install(&base)?;
    assert_eq!(continued.status.code(), Some(2));
    let value = json(&continued)?;
    validate("setup-install.schema.json", &value["data"])?;
    assert_eq!(value["data"]["components"][0]["status"], "already_current");
    assert_eq!(value["data"]["components"][1]["reason"], "artifact_missing");

    publish_stand_in(
        &base,
        ManagedComponent::WhisperCli,
        &action_version(&saved, 1)?,
    )?;
    publish_stand_in(
        &base,
        ManagedComponent::WhisperModel,
        &action_version(&saved, 2)?,
    )?;
    let complete = install(&base)?;
    assert_eq!(complete.status.code(), Some(0));
    let value = json(&complete)?;
    validate("setup-install.schema.json", &value["data"])?;
    for component in value["data"]["components"]
        .as_array()
        .ok_or("no components")?
    {
        assert_eq!(component["status"], "already_current", "{component}");
    }

    let installed = plan_now(&base)?;
    let data = &installed["data"];
    assert_eq!(data["plan_digest"].as_str(), Some(digest.as_str()));
    assert_eq!(data["install_needed"], false);
    assert_eq!(data["readiness"], "ready");
    assert_eq!(data["local_asr_model"]["status"], "managed_current");
    assert_eq!(action_states(&installed), ["current", "current", "current"]);
    for dependency in data["dependencies"].as_array().ok_or("no dependencies")? {
        assert_eq!(dependency["status"], "available", "{dependency}");
    }
    let human = base
        .vsift()?
        .args(["setup", "plan", "--profile", "desktop"])
        .output()?;
    assert!(String::from_utf8_lossy(&human.stdout).contains("nothing to install"));
    Ok(())
}
