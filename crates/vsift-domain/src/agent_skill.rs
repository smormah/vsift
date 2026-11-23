//! Where AI coding clients look for the agent skill, and what installing it
//! into such a folder may do (ADR 0038 section 3).
//!
//! The catalogue is the product's statement of where each named client reads
//! skills from, confirmed from that client's own documentation on 2026-10-28
//! (known limit L-241). Two folders reach every client named: `.claude/skills/`
//! for Claude Code and `.agents/skills/` for the others. Everything here is
//! pure: resolving a home or project folder and touching the disk belong to
//! infrastructure.

use std::fmt;

/// The skill's folder name inside a client's skill folder.
pub const SKILL_FOLDER_NAME: &str = "vsift";

/// An AI coding client whose skill folder `VSift` knows.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AgentClient {
    /// Claude Code: `.claude/skills/`.
    ClaudeCode,
    /// `OpenAI` Codex: `.agents/skills/`.
    Codex,
    /// GitHub Copilot in VS Code and the Copilot CLI: `.agents/skills/`.
    Copilot,
    /// Cursor: `.agents/skills/`.
    Cursor,
    /// Gemini CLI: `.agents/skills/`.
    GeminiCli,
    /// `OpenCode`: `.agents/skills/`.
    OpenCode,
    /// Goose: `.agents/skills/`.
    Goose,
    /// Amp: `.agents/skills/`.
    Amp,
}

impl AgentClient {
    /// Every client, in the order the documentation lists them.
    pub const ALL: [Self; 8] = [
        Self::ClaudeCode,
        Self::Codex,
        Self::Copilot,
        Self::Cursor,
        Self::GeminiCli,
        Self::OpenCode,
        Self::Goose,
        Self::Amp,
    ];

    /// The stable name `--client` takes and results print.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::Cursor => "cursor",
            Self::GeminiCli => "gemini-cli",
            Self::OpenCode => "opencode",
            Self::Goose => "goose",
            Self::Amp => "amp",
        }
    }

    /// The skill folder layout this client reads.
    #[must_use]
    pub const fn layout(self) -> SkillLayout {
        match self {
            Self::ClaudeCode => SkillLayout::Claude,
            Self::Codex
            | Self::Copilot
            | Self::Cursor
            | Self::GeminiCli
            | Self::OpenCode
            | Self::Goose
            | Self::Amp => SkillLayout::Agents,
        }
    }
}

impl fmt::Display for AgentClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.identifier())
    }
}

/// One of the two skill folder layouts the named clients read.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SkillLayout {
    /// `.claude/skills/vsift/`, read by Claude Code.
    Claude,
    /// `.agents/skills/vsift/`, the folder shared by the other clients.
    Agents,
}

impl SkillLayout {
    /// Both layouts.
    pub const ALL: [Self; 2] = [Self::Claude, Self::Agents];

    /// The stable name results print.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Agents => "agents",
        }
    }

    /// The skill folder's path below the home or project folder, one
    /// component at a time.
    #[must_use]
    pub const fn components(self) -> [&'static str; 3] {
        match self {
            Self::Claude => [".claude", "skills", SKILL_FOLDER_NAME],
            Self::Agents => [".agents", "skills", SKILL_FOLDER_NAME],
        }
    }

    /// The folder as the documentation writes it, with `/` separators.
    #[must_use]
    pub const fn display_path(self) -> &'static str {
        match self {
            Self::Claude => ".claude/skills/vsift",
            Self::Agents => ".agents/skills/vsift",
        }
    }

    /// The distinct layouts `clients` read, in [`SkillLayout::ALL`] order, so
    /// clients that share a folder have it written once.
    #[must_use]
    pub fn of_clients(clients: &[AgentClient]) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|layout| clients.iter().any(|client| client.layout() == *layout))
            .collect()
    }
}

/// Whose skill folder: the person's own, or one project's.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillScope {
    /// Below the person's home folder, for every project.
    User,
    /// Below one project folder the person names.
    Project,
}

impl SkillScope {
    /// The stable name results print.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

/// What is at a skill folder's place, read without following any link.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillFolderState {
    /// Nothing is there.
    Absent,
    /// A folder holding exactly this release's skill, byte for byte.
    Current,
    /// A folder whose `SKILL.md` names the skill `vsift`, with other bytes:
    /// another version, or edited.
    Different,
    /// A symbolic link or junction; never followed.
    Link,
    /// A file, or a folder that is not a `vsift` skill.
    NotVsift,
    /// A folder on the way to it (`.claude`, `.agents` or their `skills`) is
    /// a link or junction, or not a folder; nothing below it is read.
    ParentNotFolder,
}

impl SkillFolderState {
    /// The stable name `skill status` prints.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Current => "current",
            Self::Different => "different",
            Self::Link => "link",
            Self::NotVsift => "not_vsift",
            Self::ParentNotFolder => "parent_not_folder",
        }
    }
}

/// What an install does at one folder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillInstallStep {
    /// Write the skill into the empty place.
    Create,
    /// The bytes are already there: write nothing.
    KeepCurrent,
    /// Swap the skill in for what is there (`--replace`): a different
    /// `vsift` skill, or a link, which is removed as a link.
    Swap,
    /// Change nothing, and say why.
    Refuse(SkillInstallRefusal),
}

/// Why an install leaves a folder as it was.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillInstallRefusal {
    /// A different `vsift` skill is there and `--replace` was not given.
    DifferentWithoutReplace,
    /// A link is there and `--replace` was not given.
    LinkWithoutReplace,
    /// Something that is not a `vsift` skill is there; it is never replaced.
    NotVsift,
    /// A folder on the way is a link or not a folder; it is never followed.
    ParentNotFolder,
}

impl SkillInstallRefusal {
    /// The stable name results print.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::DifferentWithoutReplace => "different_without_replace",
            Self::LinkWithoutReplace => "link_without_replace",
            Self::NotVsift => "not_vsift",
            Self::ParentNotFolder => "parent_not_folder",
        }
    }
}

/// Whether an install may replace a different `vsift` skill or a link.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplacePolicy {
    /// Never replace: refuse instead.
    Keep,
    /// The person asked for `--replace`.
    Replace,
}

/// What an install does at a folder in `state` under `policy`. Never
/// overwrites without the person's `--replace`, and never touches anything
/// that is not a `vsift` skill or a link.
#[must_use]
pub const fn install_step(state: SkillFolderState, policy: ReplacePolicy) -> SkillInstallStep {
    match (state, policy) {
        (SkillFolderState::Absent, _) => SkillInstallStep::Create,
        (SkillFolderState::Current, _) => SkillInstallStep::KeepCurrent,
        (SkillFolderState::Different | SkillFolderState::Link, ReplacePolicy::Replace) => {
            SkillInstallStep::Swap
        }
        (SkillFolderState::Different, ReplacePolicy::Keep) => {
            SkillInstallStep::Refuse(SkillInstallRefusal::DifferentWithoutReplace)
        }
        (SkillFolderState::Link, ReplacePolicy::Keep) => {
            SkillInstallStep::Refuse(SkillInstallRefusal::LinkWithoutReplace)
        }
        (SkillFolderState::NotVsift, _) => SkillInstallStep::Refuse(SkillInstallRefusal::NotVsift),
        (SkillFolderState::ParentNotFolder, _) => {
            SkillInstallStep::Refuse(SkillInstallRefusal::ParentNotFolder)
        }
    }
}

/// Why a skill file name is not a manifest entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkillEntryNameError;

impl fmt::Display for SkillEntryNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a skill entry name is lower-case components joined by '/'")
    }
}

impl std::error::Error for SkillEntryNameError {}

/// The name of one file of the skill, relative to its folder, such as
/// `references/commands.md`.
///
/// One to three components joined by `/`, each of ASCII letters, digits,
/// `.`, `-` and `_`, not starting with a dot. So a name can never climb out
/// of the skill folder, be absolute or name a hidden file.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SkillEntryName(String);

impl SkillEntryName {
    /// Validates `name`.
    ///
    /// # Errors
    ///
    /// [`SkillEntryNameError`] for anything but the form above.
    pub fn parse(name: &str) -> Result<Self, SkillEntryNameError> {
        let components: Vec<&str> = name.split('/').collect();
        let valid_component = |component: &str| {
            !component.is_empty()
                && !component.starts_with('.')
                && component.len() <= 64
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        };
        if components.len() > 3
            || !components
                .iter()
                .all(|component| valid_component(component))
        {
            return Err(SkillEntryNameError);
        }
        Ok(Self(name.to_owned()))
    }

    /// The name as written, with `/` separators.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name's components.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

impl fmt::Display for SkillEntryName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One file of the skill: its name and its bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillFile {
    name: SkillEntryName,
    bytes: Vec<u8>,
}

impl SkillFile {
    /// A file of the skill.
    #[must_use]
    pub const fn new(name: SkillEntryName, bytes: Vec<u8>) -> Self {
        Self { name, bytes }
    }

    /// Its name.
    #[must_use]
    pub const fn name(&self) -> &SkillEntryName {
        &self.name
    }

    /// Its bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Why a set of files is not a skill bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillBundleError {
    /// No `SKILL.md`.
    MissingSkillFile,
    /// The same name twice.
    DuplicateEntry(SkillEntryName),
    /// One name is a folder another name lies inside, such as `a` and `a/b`.
    EntryIsFolder(SkillEntryName),
}

impl fmt::Display for SkillBundleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSkillFile => formatter.write_str("the skill has no SKILL.md"),
            Self::DuplicateEntry(name) => write!(formatter, "the skill lists {name} twice"),
            Self::EntryIsFolder(name) => {
                write!(formatter, "the skill's {name} is both a file and a folder")
            }
        }
    }
}

impl std::error::Error for SkillBundleError {}

/// The whole skill as this release ships it: every file, in name order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillBundle {
    files: Vec<SkillFile>,
}

/// The skill's main file, which names it.
pub const SKILL_MAIN_FILE: &str = "SKILL.md";

impl SkillBundle {
    /// Validates `files` as one skill folder.
    ///
    /// # Errors
    ///
    /// [`SkillBundleError`] for a missing `SKILL.md`, a name given twice, or a
    /// name that is also another's folder.
    pub fn new(mut files: Vec<SkillFile>) -> Result<Self, SkillBundleError> {
        files.sort_by(|left, right| left.name.cmp(&right.name));
        for pair in files.windows(2) {
            if let [left, right] = pair
                && left.name == right.name
            {
                return Err(SkillBundleError::DuplicateEntry(left.name.clone()));
            }
        }
        for file in &files {
            let prefix = format!("{}/", file.name.as_str());
            if files
                .iter()
                .any(|other| other.name.as_str().starts_with(&prefix))
            {
                return Err(SkillBundleError::EntryIsFolder(file.name.clone()));
            }
        }
        if !files
            .iter()
            .any(|file| file.name.as_str() == SKILL_MAIN_FILE)
        {
            return Err(SkillBundleError::MissingSkillFile);
        }
        Ok(Self { files })
    }

    /// Every file, in name order.
    #[must_use]
    pub fn files(&self) -> &[SkillFile] {
        &self.files
    }

    /// The file named `name`, if the bundle has one.
    #[must_use]
    pub fn file(&self, name: &str) -> Option<&SkillFile> {
        self.files.iter().find(|file| file.name.as_str() == name)
    }
}

/// Whether `skill_md` is the main file of a skill named `vsift`: its front
/// matter's `name` member is exactly `vsift`. Only such a folder is ever
/// replaced.
#[must_use]
pub fn names_the_vsift_skill(skill_md: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(skill_md) else {
        return false;
    };
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return false;
    }
    lines
        .take_while(|line| line.trim_end() != "---")
        .any(|line| line.trim_end() == "name: vsift")
}

#[cfg(test)]
mod tests;
