//! The agent skill as this binary ships it, and the results of `skill
//! install`, `skill status` and `skill show` (ADR 0038 section 3).
//!
//! The skill is embedded at build time, one entry per file of
//! `skills/vsift/`, so `skill install` writes exactly the skill this release
//! was built and tested with, from npm, an archive or a source build alike. A
//! test holds the list equal to the folder, so a file added to the skill
//! cannot be forgotten here. No result names an absolute path: a folder is
//! named by its scope (`user` or `project`) and its path below that base.

use serde::Serialize;
use sha2::{Digest, Sha256};
use vsift_domain::{
    AgentClient, SkillBundle, SkillBundleError, SkillEntryName, SkillFile, SkillFolderState,
    SkillInstallRefusal, SkillLayout, SkillScope,
};

/// Every file of `skills/vsift/`, by its name in the skill, as embedded.
pub const SKILL_FILES: [(&str, &[u8]); 21] = [
    ("SKILL.md", include_bytes!("../../../skills/vsift/SKILL.md")),
    (
        "agents/openai.yaml",
        include_bytes!("../../../skills/vsift/agents/openai.yaml"),
    ),
    (
        "assets/image-check.png",
        include_bytes!("../../../skills/vsift/assets/image-check.png"),
    ),
    (
        "examples/bug-report.handoff.json",
        include_bytes!("../../../skills/vsift/examples/bug-report.handoff.json"),
    ),
    (
        "examples/findings.handoff.json",
        include_bytes!("../../../skills/vsift/examples/findings.handoff.json"),
    ),
    (
        "examples/meeting-notes.handoff.json",
        include_bytes!("../../../skills/vsift/examples/meeting-notes.handoff.json"),
    ),
    (
        "examples/no-transcript-no-images.handoff.json",
        include_bytes!("../../../skills/vsift/examples/no-transcript-no-images.handoff.json"),
    ),
    (
        "examples/supplied-transcript.handoff.json",
        include_bytes!("../../../skills/vsift/examples/supplied-transcript.handoff.json"),
    ),
    (
        "handoff.schema.json",
        include_bytes!("../../../skills/vsift/handoff.schema.json"),
    ),
    (
        "references/budgets.md",
        include_bytes!("../../../skills/vsift/references/budgets.md"),
    ),
    (
        "references/commands.md",
        include_bytes!("../../../skills/vsift/references/commands.md"),
    ),
    (
        "references/handoff.md",
        include_bytes!("../../../skills/vsift/references/handoff.md"),
    ),
    (
        "references/lifecycle.md",
        include_bytes!("../../../skills/vsift/references/lifecycle.md"),
    ),
    (
        "references/resume.md",
        include_bytes!("../../../skills/vsift/references/resume.md"),
    ),
    (
        "references/safety.md",
        include_bytes!("../../../skills/vsift/references/safety.md"),
    ),
    (
        "workflows/bug-report.md",
        include_bytes!("../../../skills/vsift/workflows/bug-report.md"),
    ),
    (
        "workflows/demo-backlog.md",
        include_bytes!("../../../skills/vsift/workflows/demo-backlog.md"),
    ),
    (
        "workflows/findings.md",
        include_bytes!("../../../skills/vsift/workflows/findings.md"),
    ),
    (
        "workflows/logs-by-clock.md",
        include_bytes!("../../../skills/vsift/workflows/logs-by-clock.md"),
    ),
    (
        "workflows/meeting-notes.md",
        include_bytes!("../../../skills/vsift/workflows/meeting-notes.md"),
    ),
    (
        "workflows/workshop-spec.md",
        include_bytes!("../../../skills/vsift/workflows/workshop-spec.md"),
    ),
];

/// The remediation of a `skill install` that left a folder as it was.
pub const SKILL_INSTALL_REFUSED_REMEDIATION: &str = "A skill folder already holds something else, so it was left as it was (data.folders says which and why). To put this release's skill in place of a different vsift skill or a link, run the same command again with --replace; anything that is not a vsift skill is never replaced: move it yourself first.";

/// The remediation of a `skill install` or `skill status` with no home
/// folder.
pub const SKILL_HOME_UNKNOWN_REMEDIATION: &str = "No home folder is known (USERPROFILE on Windows, HOME elsewhere, as an absolute path), so the skill folders for one user cannot be found; set it, or name a project folder with --project.";

/// The remediation of a `--project` that is not an existing folder.
pub const SKILL_PROJECT_REMEDIATION: &str =
    "--project needs an existing folder: the project the skill is for.";

/// The remediation of a `skill show` entry the skill does not hold as text.
pub const SKILL_SHOW_ENTRY_REMEDIATION: &str = "skill show takes the name of one text file of the skill, as skill status lists them (for example SKILL.md or references/commands.md), never a path.";

/// The skill this binary ships, as a domain bundle.
///
/// # Errors
///
/// [`SkillBundleError`] only if the embedded list is not a skill, which a
/// test keeps from shipping.
pub fn shipped_skill() -> Result<SkillBundle, SkillBundleError> {
    let mut files = Vec::with_capacity(SKILL_FILES.len());
    for (name, bytes) in SKILL_FILES {
        let name = SkillEntryName::parse(name).map_err(|_| SkillBundleError::MissingSkillFile)?;
        files.push(SkillFile::new(name, bytes.to_vec()));
    }
    SkillBundle::new(files)
}

/// The skill's digest: SHA-256, in lower-case hex, over each file in name
/// order as its name, a NUL, its length in decimal, a NUL and its bytes. Two
/// folders with the same digest hold the same skill byte for byte.
#[must_use]
pub fn skill_digest(bundle: &SkillBundle) -> String {
    let mut hasher = Sha256::new();
    for file in bundle.files() {
        hasher.update(file.name().as_str().as_bytes());
        hasher.update([0]);
        hasher.update(file.bytes().len().to_string().as_bytes());
        hasher.update([0]);
        hasher.update(file.bytes());
    }
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            use std::fmt::Write;
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// One skill folder, as results name it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillFolderData {
    layout: &'static str,
    path: &'static str,
    clients: Vec<&'static str>,
}

impl SkillFolderData {
    /// The folder of `layout`, with the named clients that read it.
    #[must_use]
    pub fn new(layout: SkillLayout) -> Self {
        Self {
            layout: layout.identifier(),
            path: layout.display_path(),
            clients: AgentClient::ALL
                .into_iter()
                .filter(|client| client.layout() == layout)
                .map(AgentClient::identifier)
                .collect(),
        }
    }
}

/// What `skill install` did at one folder.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillInstallFolderData {
    #[serde(flatten)]
    folder: SkillFolderData,
    outcome: &'static str,
    replaced: Option<&'static str>,
    refusal: Option<&'static str>,
}

/// One folder's outcome, as the application reported it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillInstallFolderOutcome {
    /// Written where nothing was.
    Installed,
    /// Already this release's skill; nothing written.
    Unchanged,
    /// Swapped in for what was there, which is named.
    Replaced(SkillFolderState),
    /// Left as it was, for the reason named.
    Refused(SkillInstallRefusal),
}

impl SkillInstallFolderData {
    /// The published form of one folder's outcome.
    #[must_use]
    pub fn new(layout: SkillLayout, outcome: SkillInstallFolderOutcome) -> Self {
        let (name, replaced, refusal) = match outcome {
            SkillInstallFolderOutcome::Installed => ("installed", None, None),
            SkillInstallFolderOutcome::Unchanged => ("unchanged", None, None),
            SkillInstallFolderOutcome::Replaced(state) => {
                ("replaced", Some(state.identifier()), None)
            }
            SkillInstallFolderOutcome::Refused(reason) => {
                ("refused", None, Some(reason.identifier()))
            }
        };
        Self {
            folder: SkillFolderData::new(layout),
            outcome: name,
            replaced,
            refusal,
        }
    }

    /// Whether the folder was left as it was for a reason.
    #[must_use]
    pub const fn refused(&self) -> bool {
        self.refusal.is_some()
    }
}

/// The `data` of a `skill.install` result
/// (`schemas/v1/skill-install-data.schema.json`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillInstallData {
    scope: &'static str,
    skill_digest: String,
    folders: Vec<SkillInstallFolderData>,
}

impl SkillInstallData {
    /// Every folder's outcome in `scope`, for the skill of `skill_digest`.
    #[must_use]
    pub const fn new(
        scope: SkillScope,
        skill_digest: String,
        folders: Vec<SkillInstallFolderData>,
    ) -> Self {
        Self {
            scope: scope.identifier(),
            skill_digest,
            folders,
        }
    }

    /// Whether any folder was left as it was; the result is then a failure
    /// that keeps this data.
    #[must_use]
    pub fn any_refused(&self) -> bool {
        self.folders.iter().any(SkillInstallFolderData::refused)
    }
}

/// One folder's state in a `skill.status` result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillStatusFolderData {
    #[serde(flatten)]
    folder: SkillFolderData,
    state: &'static str,
}

/// The `data` of a `skill.status` result
/// (`schemas/v1/skill-status-data.schema.json`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillStatusData {
    scope: &'static str,
    skill_digest: String,
    files: Vec<String>,
    folders: Vec<SkillStatusFolderData>,
}

impl SkillStatusData {
    /// The state of every folder in `scope`, beside this release's skill.
    #[must_use]
    pub fn new(
        scope: SkillScope,
        bundle: &SkillBundle,
        states: &[(SkillLayout, SkillFolderState)],
    ) -> Self {
        Self {
            scope: scope.identifier(),
            skill_digest: skill_digest(bundle),
            files: bundle
                .files()
                .iter()
                .map(|file| file.name().as_str().to_owned())
                .collect(),
            folders: states
                .iter()
                .map(|&(layout, state)| SkillStatusFolderData {
                    folder: SkillFolderData::new(layout),
                    state: state.identifier(),
                })
                .collect(),
        }
    }
}

/// The `data` of a `skill.show` result
/// (`schemas/v1/skill-show-data.schema.json`): one text file of the skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillShowData {
    entry: String,
    text: String,
}

impl SkillShowData {
    /// The text of the skill's file named `entry`: only a name in the
    /// embedded manifest, never a path, and only a file that is UTF-8 text.
    #[must_use]
    pub fn of(bundle: &SkillBundle, entry: &str) -> Option<Self> {
        let file = bundle.file(entry)?;
        let text = std::str::from_utf8(file.bytes()).ok()?;
        Some(Self {
            entry: file.name().as_str().to_owned(),
            text: text.to_owned(),
        })
    }

    /// The file's text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fs, path::Path};

    use super::{SKILL_FILES, SkillShowData, shipped_skill, skill_digest};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn walk(root: &Path, folder: &Path, found: &mut BTreeSet<String>) -> TestResult {
        for entry in fs::read_dir(folder)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, found)?;
            } else {
                let relative = path.strip_prefix(root)?;
                let name: Vec<String> = relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().into_owned())
                    .collect();
                found.insert(name.join("/"));
            }
        }
        Ok(())
    }

    #[test]
    fn the_embedded_skill_is_the_skill_folder_byte_for_byte() -> TestResult {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/vsift");
        let mut on_disk = BTreeSet::new();
        walk(&root, &root, &mut on_disk)?;
        let embedded: BTreeSet<String> = SKILL_FILES
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect();
        assert_eq!(
            embedded, on_disk,
            "a skill file is missing from SKILL_FILES"
        );
        for (name, bytes) in SKILL_FILES {
            assert_eq!(fs::read(root.join(name))?, bytes, "{name}");
        }
        let bundle = shipped_skill()?;
        assert_eq!(bundle.files().len(), SKILL_FILES.len());
        assert_eq!(skill_digest(&bundle).len(), 64);
        Ok(())
    }

    #[test]
    fn show_takes_only_a_manifest_name_of_a_text_file() -> TestResult {
        let bundle = shipped_skill()?;
        let shown = SkillShowData::of(&bundle, "references/commands.md").ok_or("commands")?;
        assert!(shown.text().contains("vsift handoff check"));
        for refused in [
            "assets/image-check.png",
            "../SKILL.md",
            "./SKILL.md",
            "references/../SKILL.md",
            "/SKILL.md",
            "C:/SKILL.md",
            r"references\commands.md",
            "skill.md",
            "references",
            "",
        ] {
            assert!(SkillShowData::of(&bundle, refused).is_none(), "{refused}");
        }
        Ok(())
    }
}
