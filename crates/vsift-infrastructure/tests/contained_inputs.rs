//! S-01, S-02 and SEC-05 for worker inputs (P11 PR 2, ADR 0021 section 9):
//! a request's source and transcript paths are opened inside the operator's
//! input root through a held directory, one component at a time and
//! following no link, and every spelling, link or alias that could reach a
//! file outside the root is refused. The sentinel outside the root stays
//! byte-identical and is never read.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use vsift_application::{
    InitializeSessionStorage, InitializeSessionStorageRequest, NeverCancelled,
};
use vsift_domain::{DurabilityRequirement, OperationId, SessionId};
use vsift_infrastructure::{
    ContainedPathError, FilesystemSessionStore, InputRoot, InputRootError, SourceSnapshot,
    read_supplied_transcript_contained,
};

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-contained-inputs-";
const SOURCE: &[u8] = b"\0\0\0\x18ftypisomcontained-source";
const SECRET: &[u8] = b"\0\0\0\x18ftypisomoutside-the-input-root";
const TRANSCRIPT: &[u8] = b"1\n00:00:00,000 --> 00:00:01,000\nhello\n";
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A temporary directory holding the input root `inputs/` and, beside it,
/// the sentinel `outside/secret.mp4`.
struct Layout(PathBuf);

impl Layout {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(path.join("inputs/videos"))?;
        fs::create_dir_all(path.join("inputs/subs"))?;
        fs::create_dir_all(path.join("outside"))?;
        fs::write(path.join("inputs/videos/talk.mp4"), SOURCE)?;
        fs::write(path.join("inputs/subs/talk.srt"), TRANSCRIPT)?;
        fs::write(path.join("outside/secret.mp4"), SECRET)?;
        Ok(Self(path))
    }

    fn inputs(&self) -> PathBuf {
        self.0.join("inputs")
    }

    fn secret(&self) -> PathBuf {
        self.0.join("outside/secret.mp4")
    }

    fn assert_sentinel_untouched(&self) -> TestResult {
        assert_eq!(fs::read(self.secret())?, SECRET);
        Ok(())
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

async fn session(layout: &Layout) -> Result<(FilesystemSessionStore, SessionId), Box<dyn Error>> {
    let root = layout.0.join("sessions");
    let store = FilesystemSessionStore::provision_default(&root)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    Ok((FilesystemSessionStore::open_existing(root)?, session_id))
}

#[tokio::test]
async fn a_contained_source_and_transcript_are_read_through_the_root() -> TestResult {
    let layout = Layout::new()?;
    let root = InputRoot::open(&layout.inputs())?;
    let (store, session_id) = session(&layout).await?;
    let source = root.open_file("videos/talk.mp4")?;
    assert_eq!(source.len(), u64::try_from(SOURCE.len())?);
    let snapshot = SourceSnapshot::stage_contained(
        &store,
        &session_id,
        &OperationId::parse("op_abcdef0123456789")?,
        source,
        &NeverCancelled,
    )?;
    snapshot.verify()?;
    assert_eq!(snapshot.bytes(), u64::try_from(SOURCE.len())?);
    let transcript = read_supplied_transcript_contained(root.open_file("subs/talk.srt")?)?;
    assert_eq!(transcript.sidecar.bytes(), u64::try_from(TRANSCRIPT.len())?);
    // The original is never changed.
    assert_eq!(fs::read(layout.inputs().join("videos/talk.mp4"))?, SOURCE);
    layout.assert_sentinel_untouched()
}

/// S-01: every traversal form, absolute spelling, drive, stream, device
/// name, separator or control character is refused by the grammar before
/// anything is opened.
#[test]
fn every_escape_spelling_is_refused_before_anything_is_opened() -> TestResult {
    let layout = Layout::new()?;
    let root = InputRoot::open(&layout.inputs())?;
    let long = "a/".repeat(33) + "b";
    for spelling in [
        "",
        "../outside/secret.mp4",
        "videos/../../outside/secret.mp4",
        "./videos/talk.mp4",
        "videos/./talk.mp4",
        "videos//talk.mp4",
        "/outside/secret.mp4",
        "C:/outside/secret.mp4",
        "C:outside",
        "videos\\talk.mp4",
        "..\\outside\\secret.mp4",
        "videos/talk.mp4:stream",
        "videos/talk.mp4::$DATA",
        "CON",
        "videos/nul.txt",
        "videos/COM1.mp4",
        "videos/LPT\u{b9}.mp4",
        "videos/talk.mp4.",
        "videos/talk.mp4 ",
        "videos/ta\u{1}lk.mp4",
        "videos/talk*.mp4",
        long.as_str(),
    ] {
        assert!(
            matches!(root.open_file(spelling), Err(ContainedPathError::Invalid)),
            "{spelling:?}"
        );
    }
    assert!(matches!(
        root.open_file("videos/missing.mp4"),
        Err(ContainedPathError::NotFound)
    ));
    assert!(matches!(
        root.open_file("videos"),
        Err(ContainedPathError::NotRegularFile)
    ));
    assert!(matches!(
        InputRoot::open(Path::new("relative-inputs")),
        Err(InputRootError::NotAbsolute)
    ));
    assert!(matches!(
        InputRoot::open(&layout.0.join("missing")),
        Err(InputRootError::Unavailable)
    ));
    layout.assert_sentinel_untouched()
}

/// S-02: a hard link to a file outside the root is refused (it has two
/// links), so the outside bytes are never read.
#[test]
fn a_hard_link_out_of_the_root_is_refused() -> TestResult {
    let layout = Layout::new()?;
    fs::hard_link(layout.secret(), layout.inputs().join("videos/linked.mp4"))?;
    let root = InputRoot::open(&layout.inputs())?;
    assert!(matches!(
        root.open_file("videos/linked.mp4"),
        Err(ContainedPathError::NotRegularFile)
    ));
    layout.assert_sentinel_untouched()
}

/// S-02: a symbolic link anywhere on the path is refused, whether it names
/// a file or a directory outside the root.
#[cfg(unix)]
#[test]
fn a_symbolic_link_out_of_the_root_is_refused() -> TestResult {
    let layout = Layout::new()?;
    std::os::unix::fs::symlink(layout.secret(), layout.inputs().join("videos/secret.mp4"))?;
    std::os::unix::fs::symlink(layout.0.join("outside"), layout.inputs().join("escape"))?;
    std::os::unix::fs::symlink(
        layout.inputs().join("videos"),
        layout.inputs().join("inside"),
    )?;
    let root = InputRoot::open(&layout.inputs())?;
    for path in ["videos/secret.mp4", "escape/secret.mp4", "inside/talk.mp4"] {
        assert!(
            matches!(root.open_file(path), Err(ContainedPathError::Link)),
            "{path}"
        );
    }
    layout.assert_sentinel_untouched()
}

/// S-02 on Windows: a symbolic link out of the root is refused. Creating one
/// needs the symbolic-link privilege (Developer Mode or elevation); where the
/// OS refuses to create it, that documented refusal is the outcome asserted,
/// so the test never passes silently without either.
#[cfg(windows)]
#[test]
fn a_symbolic_link_out_of_the_root_is_refused() -> TestResult {
    let layout = Layout::new()?;
    let file_link = layout.inputs().join("videos/secret.mp4");
    let directory_link = layout.inputs().join("escape");
    let created = std::os::windows::fs::symlink_file(layout.secret(), &file_link).and_then(|()| {
        std::os::windows::fs::symlink_dir(layout.0.join("outside"), &directory_link)
    });
    match created {
        Ok(()) => {
            let root = InputRoot::open(&layout.inputs())?;
            for path in ["videos/secret.mp4", "escape/secret.mp4"] {
                assert!(
                    matches!(root.open_file(path), Err(ContainedPathError::Link)),
                    "{path}"
                );
            }
        }
        // ERROR_PRIVILEGE_NOT_HELD: this account may not create links.
        Err(error) => assert_eq!(error.raw_os_error(), Some(1_314), "{error}"),
    }
    layout.assert_sentinel_untouched()
}
