use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use vsift_application::SkillFolders;
use vsift_domain::{SkillBundle, SkillEntryName, SkillFile, SkillFolderState, SkillLayout};

use super::{FilesystemSkillFolders, SkillBaseError, WORK_PREFIX};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const OWNED_PREFIX: &str = "vsift-skill-folders-test-";
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A temporary base folder, removed when dropped (only by its own prefix).
struct Base(PathBuf);

impl Base {
    fn new() -> Built<Self> {
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Base {
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

const MAIN: &[u8] = b"---\nname: vsift\ndescription: test\n---\n# Test skill\n";

fn bundle() -> Built<SkillBundle> {
    Ok(SkillBundle::new(vec![
        SkillFile::new(SkillEntryName::parse("SKILL.md")?, MAIN.to_vec()),
        SkillFile::new(
            SkillEntryName::parse("references/commands.md")?,
            b"commands\n".to_vec(),
        ),
        SkillFile::new(
            SkillEntryName::parse("assets/image-check.png")?,
            vec![0x89, b'P', b'N', b'G', 0, 1, 2],
        ),
    ])?)
}

fn folders(base: &Base) -> Built<FilesystemSkillFolders> {
    Ok(FilesystemSkillFolders::for_project(&base.0, bundle()?)
        .map_err(|error| format!("{error:?}"))?)
}

/// No work folder of this module is left beside the skill folders.
fn no_work_folders_left(folder: &Path) -> Built<()> {
    let Some(parent) = folder.parent() else {
        return Ok(());
    };
    for entry in fs::read_dir(parent)? {
        let name = entry?.file_name();
        assert!(
            !name.to_string_lossy().starts_with(WORK_PREFIX),
            "{name:?} was left"
        );
    }
    Ok(())
}

#[test]
fn a_skill_is_written_where_nothing_was_and_reads_back_current() -> TestResult {
    let base = Base::new()?;
    let folders = folders(&base)?;
    for layout in SkillLayout::ALL {
        assert_eq!(folders.inspect(layout)?, SkillFolderState::Absent);
        folders.create(layout)?;
        assert_eq!(folders.inspect(layout)?, SkillFolderState::Current);
        let folder = folders.folder(layout);
        assert_eq!(fs::read(folder.join("SKILL.md"))?, MAIN);
        assert_eq!(
            fs::read(folder.join("references").join("commands.md"))?,
            b"commands\n"
        );
        no_work_folders_left(&folder)?;
    }
    assert!(base.0.join(".claude/skills/vsift/SKILL.md").is_file());
    assert!(base.0.join(".agents/skills/vsift/SKILL.md").is_file());
    Ok(())
}

#[test]
fn an_edited_or_extended_skill_is_different_and_a_foreign_folder_is_not_vsift() -> TestResult {
    let base = Base::new()?;
    let folders = folders(&base)?;
    let layout = SkillLayout::Agents;
    folders.create(layout)?;
    let folder = folders.folder(layout);

    fs::write(folder.join("references").join("commands.md"), b"edited\n")?;
    assert_eq!(folders.inspect(layout)?, SkillFolderState::Different);
    fs::write(folder.join("references").join("commands.md"), b"commands\n")?;
    fs::write(folder.join("extra.md"), b"x")?;
    assert_eq!(folders.inspect(layout)?, SkillFolderState::Different);
    fs::remove_file(folder.join("extra.md"))?;
    assert_eq!(folders.inspect(layout)?, SkillFolderState::Current);

    fs::write(folder.join("SKILL.md"), b"---\nname: other\n---\n")?;
    assert_eq!(folders.inspect(layout)?, SkillFolderState::NotVsift);
    fs::remove_dir_all(&folder)?;
    fs::write(&folder, b"a file")?;
    assert_eq!(folders.inspect(layout)?, SkillFolderState::NotVsift);
    Ok(())
}

#[test]
fn a_swap_replaces_a_different_skill_and_leaves_nothing_beside_it() -> TestResult {
    let base = Base::new()?;
    let folders = folders(&base)?;
    let layout = SkillLayout::Claude;
    folders.create(layout)?;
    let folder = folders.folder(layout);
    fs::write(folder.join("references").join("commands.md"), b"older\n")?;
    fs::write(folder.join("references").join("removed.md"), b"gone\n")?;
    assert_eq!(folders.inspect(layout)?, SkillFolderState::Different);

    folders.swap(layout)?;
    assert_eq!(folders.inspect(layout)?, SkillFolderState::Current);
    assert!(!folder.join("references").join("removed.md").exists());
    no_work_folders_left(&folder)?;
    Ok(())
}

#[test]
fn a_create_never_writes_over_a_folder_that_appeared() -> TestResult {
    let base = Base::new()?;
    let folders = folders(&base)?;
    let layout = SkillLayout::Agents;
    let folder = folders.folder(layout);
    fs::create_dir_all(&folder)?;
    fs::write(folder.join("mine.txt"), b"keep")?;
    assert!(folders.create(layout).is_err());
    assert_eq!(fs::read(folder.join("mine.txt"))?, b"keep");
    no_work_folders_left(&folder)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_link_is_reported_and_replaced_only_as_a_link() -> TestResult {
    use std::os::unix::fs::symlink;
    let base = Base::new()?;
    let folders = folders(&base)?;
    let layout = SkillLayout::Claude;
    let elsewhere = base.0.join("elsewhere");
    fs::create_dir(&elsewhere)?;
    fs::write(elsewhere.join("SKILL.md"), MAIN)?;
    let folder = folders.folder(layout);
    fs::create_dir_all(folder.parent().ok_or("parent")?)?;
    symlink(&elsewhere, &folder)?;
    assert_eq!(folders.inspect(layout)?, SkillFolderState::Link);

    folders.swap(layout)?;
    assert_eq!(folders.inspect(layout)?, SkillFolderState::Current);
    assert_eq!(
        fs::read(elsewhere.join("SKILL.md"))?,
        MAIN,
        "the link's target was touched"
    );
    assert_eq!(fs::read_dir(&elsewhere)?.count(), 1);
    no_work_folders_left(&folder)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_linked_parent_is_never_followed() -> TestResult {
    use std::os::unix::fs::symlink;
    let base = Base::new()?;
    let folders = folders(&base)?;
    let elsewhere = base.0.join("elsewhere");
    fs::create_dir(&elsewhere)?;
    symlink(&elsewhere, base.0.join(".agents"))?;
    assert_eq!(
        folders.inspect(SkillLayout::Agents)?,
        SkillFolderState::ParentNotFolder
    );
    assert!(folders.create(SkillLayout::Agents).is_err());
    assert_eq!(fs::read_dir(&elsewhere)?.count(), 0);
    Ok(())
}

#[test]
fn a_parent_that_is_a_file_stops_the_step() -> TestResult {
    let base = Base::new()?;
    let folders = folders(&base)?;
    fs::write(base.0.join(".claude"), b"a file")?;
    assert_eq!(
        folders.inspect(SkillLayout::Claude)?,
        SkillFolderState::ParentNotFolder
    );
    assert!(folders.create(SkillLayout::Claude).is_err());
    Ok(())
}

#[test]
fn a_project_folder_must_exist_and_be_a_folder() -> TestResult {
    let base = Base::new()?;
    let missing = base.0.join("missing");
    assert_eq!(
        FilesystemSkillFolders::for_project(&missing, bundle()?).err(),
        Some(SkillBaseError::ProjectNotFolder)
    );
    let file = base.0.join("file");
    fs::write(&file, b"x")?;
    assert_eq!(
        FilesystemSkillFolders::for_project(&file, bundle()?).err(),
        Some(SkillBaseError::ProjectNotFolder)
    );
    Ok(())
}
