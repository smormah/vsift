//! The skill folders below a home or project folder, on the disk (ADR 0038
//! section 3 and the planner's condition 1).
//!
//! Every step reads with `symlink_metadata` and never follows a symbolic link
//! or a Windows junction: a link where the skill folder belongs is reported as
//! one and only ever removed as a link, and a link on the way to it (a linked
//! `.claude` folder, say) stops the step. Writing is staged: the skill is
//! written into a fresh folder beside its place, read back and compared with
//! the skill's bytes, and only then renamed into place. Replacing is a swap,
//! never delete-then-copy: the old folder is renamed aside under a name this
//! module made, the staged one renamed into place, and only then the old one
//! removed; if the second rename fails the old one is renamed back.

use std::{
    collections::BTreeSet,
    env, fs, io,
    path::{Path, PathBuf},
};

use vsift_application::{SkillFolderError, SkillFolders};
use vsift_domain::{
    SKILL_MAIN_FILE, SkillBundle, SkillFolderState, SkillLayout, names_the_vsift_skill,
};

/// The most entries a skill folder may hold before it is simply `different`:
/// a bound on what inspecting a folder reads.
const MAX_INSPECTED_ENTRIES: usize = 512;

/// The deepest a skill folder is read before it is simply `different`.
const MAX_INSPECTED_DEPTH: usize = 4;

/// The prefix of the folders this module makes beside a skill folder; only
/// such a folder is ever removed after a failed or finished step.
const WORK_PREFIX: &str = ".vsift-skill-";

/// Why the base folder for skill folders cannot be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillBaseError {
    /// No home folder is known (`USERPROFILE` on Windows, `HOME` elsewhere),
    /// or it is not an absolute path.
    HomeUnknown,
    /// The project folder does not exist or is not a folder.
    ProjectNotFolder,
}

/// The skill folders below one base folder, holding the skill to write.
#[derive(Clone, Debug)]
pub struct FilesystemSkillFolders {
    base: PathBuf,
    bundle: SkillBundle,
}

impl FilesystemSkillFolders {
    /// The person's own skill folders, below their home folder.
    ///
    /// # Errors
    ///
    /// [`SkillBaseError::HomeUnknown`].
    pub fn for_user(bundle: SkillBundle) -> Result<Self, SkillBaseError> {
        let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let home = env::var_os(variable)
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())
            .ok_or(SkillBaseError::HomeUnknown)?;
        Ok(Self { base: home, bundle })
    }

    /// One project's skill folders: `project` is canonicalised and must
    /// already be a folder.
    ///
    /// # Errors
    ///
    /// [`SkillBaseError::ProjectNotFolder`].
    pub fn for_project(project: &Path, bundle: SkillBundle) -> Result<Self, SkillBaseError> {
        let base = fs::canonicalize(project).map_err(|_| SkillBaseError::ProjectNotFolder)?;
        if !fs::metadata(&base).is_ok_and(|metadata| metadata.is_dir()) {
            return Err(SkillBaseError::ProjectNotFolder);
        }
        Ok(Self { base, bundle })
    }

    /// The skill folder of `layout`.
    #[must_use]
    pub fn folder(&self, layout: SkillLayout) -> PathBuf {
        layout
            .components()
            .iter()
            .fold(self.base.clone(), |path, component| path.join(component))
    }

    /// The folders on the way to the skill folder: `.claude` (or `.agents`)
    /// and its `skills`.
    fn parents(&self, layout: SkillLayout) -> [PathBuf; 2] {
        let [top, skills, _] = layout.components();
        let first = self.base.join(top);
        let second = first.join(skills);
        [first, second]
    }

    /// Writes the skill into a fresh folder beside `target`, reads it back
    /// and compares it with the skill's bytes.
    fn stage(&self, target: &Path) -> Result<PathBuf, SkillFolderError> {
        let parent = target.parent().ok_or(SkillFolderError::Io)?;
        let stage = parent.join(work_name("stage")?);
        fs::create_dir(&stage).map_err(|_| SkillFolderError::Io)?;
        let written = self.write_files(&stage).and_then(|()| self.verify(&stage));
        if let Err(error) = written {
            remove_work_folder(&stage);
            return Err(error);
        }
        Ok(stage)
    }

    fn write_files(&self, stage: &Path) -> Result<(), SkillFolderError> {
        for file in self.bundle.files() {
            let path = entry_path(stage, file.name().components());
            if let Some(folder) = path.parent()
                && folder != stage
            {
                match fs::create_dir(folder) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(_) => return Err(SkillFolderError::Io),
                }
            }
            write_new(&path, file.bytes()).map_err(|_| SkillFolderError::Io)?;
        }
        Ok(())
    }

    fn verify(&self, stage: &Path) -> Result<(), SkillFolderError> {
        match self.compare(stage) {
            Ok(true) => Ok(()),
            Ok(false) => Err(SkillFolderError::VerificationFailed),
            Err(_) => Err(SkillFolderError::Io),
        }
    }

    /// Whether the real folder `folder` holds exactly the skill's files with
    /// exactly its bytes, read without following a link.
    fn compare(&self, folder: &Path) -> io::Result<bool> {
        let Some(found) = list_files(folder)? else {
            return Ok(false);
        };
        let expected: BTreeSet<String> = self
            .bundle
            .files()
            .iter()
            .map(|file| file.name().as_str().to_owned())
            .collect();
        if found != expected {
            return Ok(false);
        }
        for file in self.bundle.files() {
            let bytes = fs::read(entry_path(folder, file.name().components()))?;
            if bytes != file.bytes() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Makes the folders on the way to `layout`'s folder that do not exist.
    fn make_parents(&self, layout: SkillLayout) -> Result<(), SkillFolderError> {
        for parent in self.parents(layout) {
            match fs::symlink_metadata(&parent) {
                Ok(metadata) if is_plain_folder(&metadata) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&parent).map_err(|_| SkillFolderError::Io)?;
                }
                Ok(_) | Err(_) => return Err(SkillFolderError::Io),
            }
        }
        Ok(())
    }
}

impl SkillFolders for FilesystemSkillFolders {
    fn inspect(&self, layout: SkillLayout) -> Result<SkillFolderState, SkillFolderError> {
        for parent in self.parents(layout) {
            match fs::symlink_metadata(&parent) {
                Ok(metadata) if is_plain_folder(&metadata) => {}
                Ok(_) => return Ok(SkillFolderState::ParentNotFolder),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Ok(SkillFolderState::Absent);
                }
                Err(_) => return Err(SkillFolderError::Io),
            }
        }
        let folder = self.folder(layout);
        let metadata = match fs::symlink_metadata(&folder) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(SkillFolderState::Absent);
            }
            Err(_) => return Err(SkillFolderError::Io),
        };
        if is_link(&metadata) {
            return Ok(SkillFolderState::Link);
        }
        if !metadata.is_dir() {
            return Ok(SkillFolderState::NotVsift);
        }
        let main = folder.join(SKILL_MAIN_FILE);
        let is_vsift = fs::symlink_metadata(&main)
            .is_ok_and(|metadata| metadata.is_file() && !is_link(&metadata))
            && fs::read(&main).is_ok_and(|bytes| names_the_vsift_skill(&bytes));
        if !is_vsift {
            return Ok(SkillFolderState::NotVsift);
        }
        match self.compare(&folder) {
            Ok(true) => Ok(SkillFolderState::Current),
            Ok(false) => Ok(SkillFolderState::Different),
            Err(_) => Err(SkillFolderError::Io),
        }
    }

    fn create(&self, layout: SkillLayout) -> Result<(), SkillFolderError> {
        self.make_parents(layout)?;
        let target = self.folder(layout);
        let stage = self.stage(&target)?;
        // A rename never replaces a folder that appeared meanwhile on
        // Windows, and on Unix replaces only an empty one; either way the
        // stage is removed and the step fails.
        if fs::rename(&stage, &target).is_err() {
            remove_work_folder(&stage);
            return Err(SkillFolderError::Io);
        }
        Ok(())
    }

    fn swap(&self, layout: SkillLayout) -> Result<(), SkillFolderError> {
        let target = self.folder(layout);
        let stage = self.stage(&target)?;
        let parent = target.parent().ok_or(SkillFolderError::Io)?;
        let aside = parent.join(work_name("old")?);
        if fs::rename(&target, &aside).is_err() {
            remove_work_folder(&stage);
            return Err(SkillFolderError::Io);
        }
        if fs::rename(&stage, &target).is_err() {
            // Put the old folder (or link) back before anything else. Should
            // that fail too, it is kept under its work name, never removed.
            let _ = fs::rename(&aside, &target);
            remove_work_folder(&stage);
            return Err(SkillFolderError::Io);
        }
        remove_aside(&aside);
        Ok(())
    }
}

/// `base` joined with each component of a skill entry name.
fn entry_path<'name>(base: &Path, components: impl Iterator<Item = &'name str>) -> PathBuf {
    components.fold(base.to_path_buf(), |path, component| path.join(component))
}

/// A fresh name for a work folder: the prefix, `kind` and 16 random hex
/// digits.
fn work_name(kind: &str) -> Result<String, SkillFolderError> {
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random).map_err(|_| SkillFolderError::Io)?;
    let hex = random.iter().fold(String::new(), |mut hex, byte| {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    Ok(format!("{WORK_PREFIX}{kind}-{hex}"))
}

/// Creates `path` exclusively and writes `bytes` to it.
fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Every file below the real folder `folder`, as `/`-joined names relative
/// to it, or `None` when it holds a link, anything that is neither a file
/// nor a folder, or more than the inspection bounds.
fn list_files(folder: &Path) -> io::Result<Option<BTreeSet<String>>> {
    let mut found = BTreeSet::new();
    let mut pending = vec![(folder.to_path_buf(), String::new(), 0_usize)];
    let mut entries = 0_usize;
    while let Some((path, prefix, depth)) = pending.pop() {
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            entries += 1;
            if entries > MAX_INSPECTED_ENTRIES {
                return Ok(None);
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                return Ok(None);
            };
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if is_link(&metadata) {
                return Ok(None);
            } else if metadata.is_dir() {
                if depth + 1 >= MAX_INSPECTED_DEPTH {
                    return Ok(None);
                }
                pending.push((entry.path(), relative, depth + 1));
            } else if metadata.is_file() {
                found.insert(relative);
            } else {
                return Ok(None);
            }
        }
    }
    Ok(Some(found))
}

/// A folder that is not a link or junction.
fn is_plain_folder(metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && !is_link(metadata)
}

/// A symbolic link, or on Windows any reparse point (a junction included).
fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Removes a work folder this module made, identified by its name; best
/// effort, since the step's outcome is already decided.
fn remove_work_folder(path: &Path) {
    let ours = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(WORK_PREFIX));
    if ours && fs::symlink_metadata(path).is_ok_and(|metadata| is_plain_folder(&metadata)) {
        let _ = fs::remove_dir_all(path);
    }
}

/// Removes what was moved aside by a swap: a link only as a link, a folder
/// with everything in it (`remove_dir_all` does not follow links). Best
/// effort: the skill is in place, and a leftover keeps its `VSift` name.
fn remove_aside(aside: &Path) {
    let Ok(metadata) = fs::symlink_metadata(aside) else {
        return;
    };
    if is_link(&metadata) {
        // A directory link or junction is removed with `remove_dir` on
        // Windows and `remove_file` elsewhere; neither touches its target.
        let _ = fs::remove_dir(aside).or_else(|_| fs::remove_file(aside));
    } else {
        remove_work_folder(aside);
    }
}

#[cfg(test)]
mod tests;
