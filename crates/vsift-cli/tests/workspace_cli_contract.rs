//! Public CLI contract for worker workspaces (P11 PR 2, ADR 0021 section 3,
//! maintainer decisions D1 and D2) and the command line's durable mode
//! (ADR 0020 D-3): `session init-workspace`, then `ingest --session-root
//! <workspace>`.
//!
//! Everything runs on every platform. A durable workspace is qualified only
//! on Ubuntu 24.04 with local ext4, so the durable tests ask the
//! infrastructure's own profile check which outcome this host must give
//! (`os_crash_durable`, or `MISSING_CAPABILITY` and nothing created) and
//! assert exactly that one; they never skip.

use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_contract::{
    DURABILITY_UNAVAILABLE_REMEDIATION, WORKSPACE_POLICY_MISMATCH_REMEDIATION,
    WORKSPACE_ROOT_REMEDIATION,
};
use vsift_infrastructure::directory_offers_os_crash_durability;

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-workspace-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SOURCE_BYTES: &[u8] = b"\0\0\0\x18ftypisomworkspace-source";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }

    fn workspace(&self) -> PathBuf {
        self.path("worker workspace")
    }

    fn source(&self) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.path("original media.mp4");
        fs::write(&path, SOURCE_BYTES)?;
        Ok(path)
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_str(&fs::read_to_string(
            schema_root().join(name),
        )?)?)
    }
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema: Value =
        serde_json::from_str(&fs::read_to_string(schema_root().join(schema_path))?)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// Runs `vsift` with an isolated per-user base, at `root` when given.
fn vsift(
    owned: &OwnedRoot,
    root: Option<&Path>,
    arguments: &[&str],
) -> Result<Output, Box<dyn Error>> {
    let base = owned.path("user");
    let mut command = Command::cargo_bin("vsift")?;
    command
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("XDG_CACHE_HOME", &base)
        .env("HOME", &base);
    if let Some(root) = root {
        command.arg("--session-root").arg(root);
    }
    Ok(command.args(arguments).arg("--json").output()?)
}

/// The one schema-valid JSON result of a run.
fn result(output: &Output) -> Result<Value, Box<dyn Error>> {
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    Ok(value)
}

fn init(owned: &OwnedRoot, root: &Path, policy: &[&str]) -> Result<Output, Box<dyn Error>> {
    let mut arguments = vec!["session", "init-workspace"];
    arguments.extend_from_slice(policy);
    vsift(owned, Some(root), &arguments)
}

fn assert_failure(output: &Output, code: &str, remediation: Option<&str>) -> TestResult {
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let value = result(output)?;
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], code, "{value}");
    if let Some(remediation) = remediation {
        assert_eq!(value["error"]["remediation"][0]["summary"], remediation);
    }
    Ok(())
}

const EPHEMERAL: [&str; 6] = [
    "--durability",
    "ephemeral",
    "--admission-slots",
    "3",
    "--retention-hours",
    "2",
];

#[test]
fn a_workspace_is_created_once_and_its_policy_is_immutable() -> TestResult {
    let owned = OwnedRoot::new()?;
    let workspace = owned.workspace();

    let created = init(&owned, &workspace, &EPHEMERAL)?;
    assert!(created.status.success(), "{created:?}");
    let created = result(&created)?;
    assert_eq!(created["command"], "session.init-workspace");
    assert_eq!(created["status"], "complete");
    validate("workspace-data.schema.json", &created["data"])?;
    assert_eq!(created["data"]["profile"], "durable_workspace");
    assert_eq!(created["data"]["durability"], "ephemeral");
    assert_eq!(created["data"]["publication"], "process_crash_consistent");
    assert_eq!(created["data"]["admission_capacity"], 3);
    assert_eq!(created["data"]["session_retention_seconds"], 7_200);
    assert_eq!(created["data"]["outcome"], "created");
    assert!(
        !created.to_string().contains("worker workspace"),
        "{created}"
    );

    // The same policy again is idempotent.
    let again = init(&owned, &workspace, &EPHEMERAL)?;
    assert!(again.status.success(), "{again:?}");
    let again = result(&again)?;
    assert_eq!(again["data"]["outcome"], "already_initialized");
    assert_eq!(again["data"]["admission_capacity"], 3);

    // Any other policy, raised or lowered, is refused and changes nothing.
    for other in [
        [
            "--durability",
            "ephemeral",
            "--admission-slots",
            "4",
            "--retention-hours",
            "2",
        ],
        [
            "--durability",
            "ephemeral",
            "--admission-slots",
            "3",
            "--retention-hours",
            "3",
        ],
        [
            "--durability",
            "ephemeral",
            "--admission-slots",
            "2",
            "--retention-hours",
            "2",
        ],
    ] {
        assert_failure(
            &init(&owned, &workspace, &other)?,
            "INVALID_ARGUMENT",
            Some(WORKSPACE_POLICY_MISMATCH_REMEDIATION),
        )?;
    }
    // The default retention is 168 hours, so omitting it is another policy.
    assert_failure(
        &init(
            &owned,
            &workspace,
            &["--durability", "ephemeral", "--admission-slots", "3"],
        )?,
        "INVALID_ARGUMENT",
        Some(WORKSPACE_POLICY_MISMATCH_REMEDIATION),
    )?;
    let unchanged = result(&init(&owned, &workspace, &EPHEMERAL)?)?;
    assert_eq!(unchanged["data"]["outcome"], "already_initialized");
    Ok(())
}

#[test]
fn a_desktop_root_never_becomes_a_workspace() -> TestResult {
    let owned = OwnedRoot::new()?;
    let desktop = owned.path("desktop sessions");
    let source = owned.source()?;
    let opened = vsift(
        &owned,
        Some(&desktop),
        &["ingest", source.to_str().ok_or("path")?],
    )?;
    assert!(opened.status.success(), "{opened:?}");
    assert_eq!(result(&opened)?["lifecycle"]["mode"], "ephemeral");
    assert_failure(
        &init(&owned, &desktop, &EPHEMERAL)?,
        "INVALID_ARGUMENT",
        Some(WORKSPACE_POLICY_MISMATCH_REMEDIATION),
    )
}

#[test]
fn a_workspace_needs_an_explicit_absolute_root_and_a_bounded_policy() -> TestResult {
    let owned = OwnedRoot::new()?;
    // No --session-root: the per-user cache is never a workspace.
    let mut arguments = vec!["session", "init-workspace"];
    arguments.extend_from_slice(&EPHEMERAL);
    assert_failure(
        &vsift(&owned, None, &arguments)?,
        "INVALID_ARGUMENT",
        Some(WORKSPACE_ROOT_REMEDIATION),
    )?;
    // The per-user cache named explicitly is refused too.
    let cache = owned.path("user").join(if cfg!(windows) {
        "VSift-sessions"
    } else if cfg!(target_os = "macos") {
        "Library/Caches/VSift-sessions"
    } else {
        "vsift-sessions"
    });
    assert_failure(
        &init(&owned, &cache, &EPHEMERAL)?,
        "INVALID_ARGUMENT",
        Some(WORKSPACE_ROOT_REMEDIATION),
    )?;
    assert!(!cache.exists());
    assert_failure(
        &init(&owned, Path::new("relative-workspace"), &EPHEMERAL)?,
        "INVALID_ARGUMENT",
        None,
    )?;
    let workspace = owned.workspace();
    for bad in [
        [
            "--durability",
            "ephemeral",
            "--admission-slots",
            "0",
            "--retention-hours",
            "2",
        ],
        [
            "--durability",
            "ephemeral",
            "--admission-slots",
            "65",
            "--retention-hours",
            "2",
        ],
        [
            "--durability",
            "ephemeral",
            "--admission-slots",
            "3",
            "--retention-hours",
            "0",
        ],
        [
            "--durability",
            "ephemeral",
            "--admission-slots",
            "3",
            "--retention-hours",
            "721",
        ],
        [
            "--durability",
            "forever",
            "--admission-slots",
            "3",
            "--retention-hours",
            "2",
        ],
    ] {
        assert_failure(&init(&owned, &workspace, &bad)?, "INVALID_ARGUMENT", None)?;
    }
    assert!(!workspace.exists());
    Ok(())
}

/// A durable workspace exists only where OS-crash durability is qualified:
/// on such a host it is created and reports `os_crash_durable`; anywhere
/// else the command fails with `MISSING_CAPABILITY` and creates nothing.
#[test]
fn a_durable_workspace_is_created_only_where_durability_is_qualified() -> TestResult {
    let owned = OwnedRoot::new()?;
    let workspace = owned.workspace();
    let qualified = directory_offers_os_crash_durability(&owned.0);
    let durable = [
        "--durability",
        "durable",
        "--admission-slots",
        "8",
        "--retention-hours",
        "168",
    ];
    let output = init(&owned, &workspace, &durable)?;
    if qualified {
        assert!(output.status.success(), "{output:?}");
        let value = result(&output)?;
        validate("workspace-data.schema.json", &value["data"])?;
        assert_eq!(value["data"]["publication"], "os_crash_durable");
        assert_eq!(value["data"]["session_retention_seconds"], 604_800);
    } else {
        assert_failure(
            &output,
            "MISSING_CAPABILITY",
            Some(DURABILITY_UNAVAILABLE_REMEDIATION),
        )?;
        assert!(!workspace.exists(), "nothing may be created");
    }
    Ok(())
}

/// ADR 0020 D-3: `ingest --session-root <workspace>` opens a session with the
/// workspace's durability and lifetime. A durable workspace (where it can
/// exist) gives `os_crash_durable`; an ephemeral one gives a process-crash
/// consistent session that still lives the workspace's retention and reports
/// `durable_worker`.
#[test]
fn ingest_in_a_workspace_inherits_its_durability_and_retention() -> TestResult {
    let owned = OwnedRoot::new()?;
    let source = owned.source()?;
    let source = source.to_str().ok_or("path")?;

    let workspace = owned.workspace();
    assert!(init(&owned, &workspace, &EPHEMERAL)?.status.success());
    let before = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let opened = vsift(&owned, Some(&workspace), &["ingest", source])?;
    assert!(opened.status.success(), "{opened:?}");
    let opened = result(&opened)?;
    validate("ingest-data.schema.json", &opened["data"])?;
    assert_eq!(opened["data"]["publication"], "process_crash_consistent");
    assert_eq!(opened["lifecycle"]["mode"], "durable_worker");
    let id = opened["data"]["session_id"].as_str().ok_or("session id")?;

    let status = result(&vsift(
        &owned,
        Some(&workspace),
        &["session", "status", id],
    )?)?;
    assert_eq!(status["lifecycle"]["mode"], "durable_worker");
    // RFC 3339 UTC timestamps of one shape order as text.
    let expires = status["data"]["expires_at"]
        .as_str()
        .ok_or("expiry")?
        .to_owned();
    assert!(
        (rfc3339(before + 7_200)?..=rfc3339(before + 7_200 + 60)?).contains(&expires),
        "{expires} is not two hours after {before}"
    );
    assert_eq!(status["lifecycle"]["expires_at"], expires.as_str());
    let renewed = result(&vsift(&owned, Some(&workspace), &["session", "renew", id])?)?;
    assert_eq!(renewed["lifecycle"]["mode"], "durable_worker");
    assert!(renewed["data"]["expires_at"].as_str().ok_or("expiry")? >= expires.as_str());
    let cleaned = result(&vsift(
        &owned,
        Some(&workspace),
        &["session", "clean", "--expired", "--dry-run"],
    )?)?;
    assert_eq!(cleaned["data"]["items"][0]["outcome"], "ineligible");

    let durable_root = owned.path("durable workspace");
    let durable = init(
        &owned,
        &durable_root,
        &["--durability", "durable", "--admission-slots", "4"],
    )?;
    if directory_offers_os_crash_durability(&owned.0) {
        assert!(durable.status.success(), "{durable:?}");
        let opened = result(&vsift(&owned, Some(&durable_root), &["ingest", source])?)?;
        assert_eq!(opened["data"]["publication"], "os_crash_durable");
        assert_eq!(opened["lifecycle"]["mode"], "durable_worker");
    } else {
        assert_failure(&durable, "MISSING_CAPABILITY", None)?;
        assert!(!durable_root.exists());
    }
    Ok(())
}

/// Unix seconds as the CLI formats them.
fn rfc3339(seconds: u64) -> Result<String, Box<dyn Error>> {
    Ok(
        time::OffsetDateTime::from_unix_timestamp(i64::try_from(seconds)?)?
            .format(&time::format_description::well_known::Rfc3339)?,
    )
}
