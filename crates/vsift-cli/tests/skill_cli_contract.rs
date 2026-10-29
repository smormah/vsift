//! Public CLI contract of `vsift skill install`, `skill status` and `skill
//! show` (ADR 0038 section 3), through the binary.
//!
//! Every run gets a temporary home folder (`HOME` and `USERPROFILE`), so no
//! test reads or writes the real one. No result names an absolute path.

use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
};

use assert_cmd::Command;
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-skill-cli-";
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A temporary folder holding the fake home and any project, removed when
/// dropped (only by its own prefix).
struct Root(PathBuf);

impl Root {
    fn new() -> Built<Self> {
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        fs::create_dir(path.join("home"))?;
        Ok(Self(path))
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
}

impl Drop for Root {
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

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema: Value = serde_json::from_str(&fs::read_to_string(
        repository("schemas/v1").join(schema_path),
    )?)?;
    jsonschema::validator_for(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

fn vsift(root: &Root, arguments: &[&str]) -> Built<Output> {
    let base = root.0.join("user");
    Ok(Command::cargo_bin("vsift")?
        .env("HOME", root.home())
        .env("USERPROFILE", root.home())
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .args(arguments)
        .output()?)
}

/// A `--json` result, validated against the envelope and, when it has data,
/// against `schema`. It never names the temporary folder.
fn json(root: &Root, output: &Output, command: &str, schema: &str) -> Built<Value> {
    assert!(output.stderr.is_empty(), "a JSON command wrote to stderr");
    let text = String::from_utf8(output.stdout.clone())?;
    let root_name = root
        .0
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("name")?;
    assert!(!text.contains(root_name), "a result named an absolute path");
    let value: Value = serde_json::from_str(&text)?;
    validate("operation-response.schema.json", &value)?;
    assert_eq!(value["command"], command);
    if value["data"].is_object() {
        validate(schema, &value["data"])?;
    }
    Ok(value)
}

fn install(root: &Root, extra: &[&str]) -> Built<(Option<i32>, Value)> {
    let mut arguments = vec![
        "skill",
        "install",
        "--client",
        "claude-code",
        "--client",
        "codex",
        "--client",
        "cursor",
    ];
    arguments.extend_from_slice(extra);
    arguments.push("--json");
    let output = vsift(root, &arguments)?;
    let value = json(
        root,
        &output,
        "skill.install",
        "skill-install-data.schema.json",
    )?;
    Ok((output.status.code(), value))
}

fn outcomes(value: &Value) -> Vec<String> {
    value["data"]["folders"]
        .as_array()
        .map(|folders| {
            folders
                .iter()
                .map(|folder| folder["outcome"].as_str().unwrap_or("").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn same_as_the_repository_skill(folder: &Path) -> Built<bool> {
    let shipped = fs::read(repository("skills/vsift/references/commands.md"))?;
    Ok(
        fs::read(folder.join("SKILL.md"))? == fs::read(repository("skills/vsift/SKILL.md"))?
            && fs::read(folder.join("references").join("commands.md"))? == shipped,
    )
}

#[test]
fn the_skill_goes_once_into_each_folder_and_again_is_unchanged() -> TestResult {
    let root = Root::new()?;
    let (code, value) = install(&root, &[])?;
    assert_eq!(code, Some(0), "{value}");
    assert_eq!(value["status"], "complete");
    assert_eq!(value["data"]["scope"], "user");
    assert_eq!(outcomes(&value), ["installed", "installed"]);
    assert_eq!(value["data"]["folders"][1]["path"], ".agents/skills/vsift");
    let claude = root.home().join(".claude").join("skills").join("vsift");
    let agents = root.home().join(".agents").join("skills").join("vsift");
    assert!(same_as_the_repository_skill(&claude)?);
    assert!(same_as_the_repository_skill(&agents)?);

    let (code, value) = install(&root, &[])?;
    assert_eq!(code, Some(0));
    assert_eq!(outcomes(&value), ["unchanged", "unchanged"]);

    let status = vsift(&root, &["skill", "status", "--json"])?;
    let status = json(
        &root,
        &status,
        "skill.status",
        "skill-status-data.schema.json",
    )?;
    assert_eq!(status["data"]["folders"][0]["state"], "current");
    assert_eq!(status["data"]["folders"][1]["state"], "current");
    Ok(())
}

#[test]
fn a_different_skill_is_replaced_only_with_replace() -> TestResult {
    let root = Root::new()?;
    install(&root, &[])?;
    let claude = root.home().join(".claude").join("skills").join("vsift");
    fs::write(claude.join("references").join("commands.md"), b"edited\n")?;

    let (code, value) = install(&root, &[])?;
    assert_eq!(code, Some(2), "{value}");
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        value["error"]["remediation"][0]["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("--replace"))
    );
    assert_eq!(outcomes(&value), ["refused", "unchanged"]);
    assert_eq!(
        value["data"]["folders"][0]["refusal"],
        "different_without_replace"
    );
    assert_eq!(
        fs::read(claude.join("references").join("commands.md"))?,
        b"edited\n",
        "a refused folder was changed"
    );

    let (code, value) = install(&root, &["--replace"])?;
    assert_eq!(code, Some(0), "{value}");
    assert_eq!(outcomes(&value), ["replaced", "unchanged"]);
    assert_eq!(value["data"]["folders"][0]["replaced"], "different");
    assert!(same_as_the_repository_skill(&claude)?);
    let leftovers = fs::read_dir(claude.parent().ok_or("parent")?)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".vsift-skill-")
        })
        .count();
    assert_eq!(leftovers, 0);
    Ok(())
}

#[test]
fn a_folder_that_is_not_a_vsift_skill_is_never_replaced() -> TestResult {
    let root = Root::new()?;
    let agents = root.home().join(".agents").join("skills").join("vsift");
    fs::create_dir_all(&agents)?;
    fs::write(agents.join("notes.txt"), b"mine")?;
    let (code, value) = install(&root, &["--replace"])?;
    assert_eq!(code, Some(2));
    assert_eq!(outcomes(&value), ["installed", "refused"]);
    assert_eq!(value["data"]["folders"][1]["refusal"], "not_vsift");
    assert_eq!(fs::read(agents.join("notes.txt"))?, b"mine");
    Ok(())
}

#[test]
fn a_project_must_be_an_existing_folder_and_gets_its_own_folders() -> TestResult {
    let root = Root::new()?;
    let project = root.0.join("project");
    for missing in [project.clone(), root.0.join("file")] {
        let text = missing.to_str().ok_or("path")?;
        let output = vsift(
            &root,
            &[
                "skill",
                "install",
                "--client",
                "codex",
                "--project",
                text,
                "--json",
            ],
        )?;
        assert_eq!(output.status.code(), Some(2));
        let value: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
        fs::write(root.0.join("file"), b"x")?;
    }
    fs::create_dir(&project)?;
    let output = vsift(
        &root,
        &[
            "skill",
            "install",
            "--client",
            "goose",
            "--project",
            project.to_str().ok_or("path")?,
            "--json",
        ],
    )?;
    let value = json(
        &root,
        &output,
        "skill.install",
        "skill-install-data.schema.json",
    )?;
    assert_eq!(value["data"]["scope"], "project");
    assert!(project.join(".agents/skills/vsift/SKILL.md").is_file());
    assert!(
        !root.home().join(".agents").exists(),
        "the home was written"
    );
    Ok(())
}

#[test]
fn show_prints_a_manifest_entry_and_refuses_any_path() -> TestResult {
    let root = Root::new()?;
    let output = vsift(
        &root,
        &["skill", "show", "references/commands.md", "--json"],
    )?;
    let value = json(&root, &output, "skill.show", "skill-show-data.schema.json")?;
    assert_eq!(
        value["data"]["text"].as_str().map(str::as_bytes),
        Some(fs::read(repository("skills/vsift/references/commands.md"))?.as_slice())
    );
    let absolute = repository("skills/vsift/SKILL.md");
    let absolute = absolute.to_str().ok_or("path")?;
    for refused in [
        "../SKILL.md",
        "references/../SKILL.md",
        "/SKILL.md",
        absolute,
        "assets/image-check.png",
        "references",
    ] {
        let output = vsift(&root, &["skill", "show", refused, "--json"])?;
        assert_eq!(output.status.code(), Some(2), "{refused}");
        let value: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT", "{refused}");
    }
    Ok(())
}

#[test]
fn human_output_names_folders_relative_to_their_base() -> TestResult {
    let root = Root::new()?;
    let output = vsift(&root, &["skill", "install", "--client", "claude-code"])?;
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout)?;
    assert!(
        text.contains(".claude/skills/vsift (claude-code): installed"),
        "{text}"
    );
    let output = vsift(&root, &["skill", "status"])?;
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains(".agents/skills/vsift"), "{text}");
    assert!(text.contains("references/commands.md"), "{text}");
    Ok(())
}
