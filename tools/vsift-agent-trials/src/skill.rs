//! The skill's references as the harness reads them, and the skill copy a
//! trial workspace receives.

use std::{
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{
    error::{TrialError, read_json, read_text},
    handoff::HandoffSchema,
    policy::{Budgets, CommandPolicy},
};

/// One check image the skill has shipped as `assets/image-check.png`.
///
/// A trial is graded against the image its own workspace received, never
/// against the repository's current one: re-grading an older trial after
/// the image changed must still compare its report with the code that trial
/// was shown. The image is identified by the SHA-256 of its bytes.
#[derive(Clone, Copy, Debug)]
pub struct CheckImage {
    /// SHA-256 of the PNG, lower-case hexadecimal.
    pub sha256: &'static str,
    /// The code printed in it, in two parts so a plain search of the
    /// repository for the joined code does not find it (the convention of
    /// the CLI's `skill_contract` guard, which keeps the skill's own text
    /// free of every part).
    code_parts: [&'static str; 2],
    /// When the image was replaced, or `None` for the one the skill ships.
    pub retired: Option<&'static str>,
}

impl CheckImage {
    /// The code printed in the image.
    #[must_use]
    pub fn code(&self) -> String {
        self.code_parts.concat()
    }
}

/// Every check image the skill has shipped, oldest first; the last one is
/// the image in `skills/vsift/assets/` (a test holds the file to it).
///
/// The first image (P12 PR 1) printed its code small in a 360x96 frame; one
/// compact model misread it in 5 runs, the same letter missing each time
/// (P12 PR 3i). Its
/// replacement uses no easily confused glyph (I, l, 1, O, 0, S, 5, B, 8),
/// larger type and wide spacing; `docs/agents/skill.md` records how it was
/// drawn.
pub const CHECK_IMAGES: [CheckImage; 2] = [
    CheckImage {
        sha256: "a94b7b820a57ceb6f17eccaaf4e94ad47af6078db08a72c490c9bbf5741459f0",
        code_parts: ["OK", "API 6281"],
        retired: Some("2026-09-30, P12 PR 3i"),
    },
    CheckImage {
        sha256: "cfc5c888aae502a2bb5d47bae6b66ec7e5832fab3aeaa7af255e948c1e1962a1",
        code_parts: ["HKR", "X 4739"],
        retired: None,
    },
];

/// The check image the skill ships now.
#[must_use]
pub fn current_check_image() -> CheckImage {
    CHECK_IMAGES[CHECK_IMAGES.len() - 1]
}

/// The check image with these bytes, if the skill ever shipped it.
#[must_use]
pub fn check_image_for(bytes: &[u8]) -> Option<CheckImage> {
    let digest = sha256_hex(bytes);
    CHECK_IMAGES
        .iter()
        .find(|image| image.sha256 == digest)
        .copied()
}

/// The code of the check image a trial workspace received: the first skill
/// copy's `assets/image-check.png` whose bytes are a known check image.
/// `None` when no copy is readable or none is known, which fails the image
/// check rather than guessing.
#[must_use]
pub fn workspace_image_code(skill_directories: &[PathBuf]) -> Option<String> {
    skill_directories.iter().find_map(|directory| {
        fs::read(directory.join("assets").join("image-check.png"))
            .ok()
            .and_then(|bytes| check_image_for(&bytes))
            .map(|image| image.code())
    })
}

/// The skill directory of a repository.
#[must_use]
pub fn skill_directory(repository: &Path) -> PathBuf {
    repository.join("skills").join("vsift")
}

/// The command policy, budgets and handoff schema of a repository's skill.
pub struct SkillReferences {
    /// From `references/commands.md`.
    pub policy: CommandPolicy,
    /// From `references/budgets.md`.
    pub budgets: Budgets,
    /// `handoff.schema.json`, compiled.
    pub schema: HandoffSchema,
}

impl SkillReferences {
    /// Reads the references below `repository/skills/vsift`.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when a reference is missing or does not parse.
    pub fn load(repository: &Path) -> Result<Self, TrialError> {
        let skill = skill_directory(repository);
        let references = skill.join("references");
        let budgets = Budgets::from_budgets_md(&read_text(&references.join("budgets.md"))?)?;
        Ok(Self {
            policy: CommandPolicy::from_commands_md(&read_text(&references.join("commands.md"))?)?,
            budgets,
            schema: HandoffSchema::new(&read_json(&skill.join("handoff.schema.json"))?, budgets)?,
        })
    }
}

/// Every regular file below `directory`, as sorted relative paths with
/// forward slashes.
///
/// # Errors
///
/// [`TrialError::Io`] when a directory cannot be listed.
pub fn files_below(directory: &Path) -> Result<Vec<(String, PathBuf)>, TrialError> {
    let mut pending = vec![directory.to_path_buf()];
    let mut files = Vec::new();
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).map_err(|error| TrialError::io_at(&current, error))? {
            let entry = entry.map_err(|error| TrialError::io_at(&current, error))?;
            let kind = entry
                .file_type()
                .map_err(|error| TrialError::io_at(&entry.path(), error))?;
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                let relative = path
                    .strip_prefix(directory)
                    .map_err(|_| TrialError::Invalid("a file outside its directory".to_owned()))?
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push((relative, path));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// SHA-256 over every file's relative path and bytes, in path order: the
/// skill's identity in a trial record.
///
/// # Errors
///
/// [`TrialError::Io`] when a file cannot be read.
pub fn directory_digest(directory: &Path) -> Result<String, TrialError> {
    let mut hasher = Sha256::new();
    for (relative, path) in files_below(directory)? {
        let bytes = fs::read(&path).map_err(|error| TrialError::io_at(&path, error))?;
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hex(&hasher.finalize()))
}

/// SHA-256 of a file.
///
/// # Errors
///
/// [`TrialError::Io`] when it cannot be read.
pub fn file_digest(path: &Path) -> Result<String, TrialError> {
    let bytes = fs::read(path).map_err(|error| TrialError::io_at(path, error))?;
    Ok(sha256_hex(&bytes))
}

/// SHA-256 of bytes, lower-case hexadecimal.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Lower-case hexadecimal.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// Copies `source` recursively into the new directory `destination`.
///
/// # Errors
///
/// [`TrialError::Io`] when a file cannot be copied.
pub fn copy_directory(source: &Path, destination: &Path) -> Result<(), TrialError> {
    for (relative, path) in files_below(source)? {
        let target = destination.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| TrialError::io_at(parent, error))?;
        }
        fs::copy(&path, &target).map_err(|error| TrialError::io_at(&target, error))?;
    }
    Ok(())
}

/// Random lower-case hexadecimal of `bytes` random bytes.
///
/// # Errors
///
/// [`TrialError::Process`] when the operating system's random source
/// fails.
pub fn random_hex(bytes: usize) -> Result<String, TrialError> {
    let mut buffer = vec![0_u8; bytes];
    getrandom::fill(&mut buffer)
        .map_err(|_| TrialError::Process("the random source failed".to_owned()))?;
    Ok(hex(&buffer))
}
