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

/// The code word printed in `skills/vsift/assets/image-check.png`, in two
/// parts so a plain search of the repository for the joined word does not
/// find it (the same convention as the CLI's `skill_contract` guard, which
/// keeps the skill's own text free of it). The grader compares a
/// `verified` handoff's `image_check_code` with it.
const IMAGE_CODE_PARTS: [&str; 2] = ["OK", "API 6281"];

/// The check image's code word.
#[must_use]
pub fn image_code() -> String {
    IMAGE_CODE_PARTS.concat()
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
