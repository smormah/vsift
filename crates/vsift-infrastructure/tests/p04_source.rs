//! Source-binding regressions independent of installed media providers.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use vsift_application::{InitializeSessionStorage, InitializeSessionStorageRequest};
use vsift_domain::{DurabilityRequirement, OperationId, SessionId};
use vsift_infrastructure::{
    FilesystemSessionStore, ProcessCancellation, SourceError, SourceSnapshot,
};

type TestResult = Result<(), Box<dyn Error>>;

struct OwnedRoot(PathBuf);

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

impl OwnedRoot {
    fn create() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "vsift-p04-source-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("vsift-p04-source-"))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

async fn workspace(
    root: &OwnedRoot,
) -> Result<(FilesystemSessionStore, SessionId), Box<dyn Error>> {
    let workspace = root.0.join("workspace");
    let store = FilesystemSessionStore::provision_default(&workspace)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    Ok((
        FilesystemSessionStore::open_existing(workspace)?,
        session_id,
    ))
}

#[tokio::test]
async fn staged_source_survives_original_change_and_preserves_original_bytes() -> TestResult {
    let root = OwnedRoot::create()?;
    let (store, session_id) = workspace(&root).await?;
    let corpus =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/generated/F01.mp4");
    let original = fs::read(corpus)?;
    let source_name = if cfg!(windows) {
        "-quoted ' source.mp4"
    } else {
        "-quoted ' source\nline.mp4"
    };
    let source = root.0.join(source_name);
    fs::write(&source, &original)?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_abcdef0123456789")?,
        &source,
    )?;
    snapshot.verify()?;
    assert!(!snapshot.original_changed(&source)?);
    assert_eq!(fs::read(&source)?, original);
    fs::write(&source, b"changed")?;
    assert!(snapshot.original_changed(&source)?);
    snapshot.verify()?;
    Ok(())
}

#[tokio::test]
async fn playlist_and_external_reference_sources_fail_before_provider_execution() -> TestResult {
    let root = OwnedRoot::create()?;
    let (store, session_id) = workspace(&root).await?;
    for variant in ["F11-external.m3u8", "F11-network.m3u8", "F11-empty.media"] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/corpus/generated")
            .join(variant);
        let result = SourceSnapshot::stage(
            &store,
            &session_id,
            &OperationId::parse("op_abcdef0123456789")?,
            &path,
        );
        assert!(matches!(result, Err(SourceError::UnsupportedContainer)));
    }
    assert!(matches!(
        SourceSnapshot::stage(
            &store,
            &session_id,
            &OperationId::parse("op_abcdef0123456789")?,
            std::path::Path::new("relative.mp4")
        ),
        Err(SourceError::InvalidPath)
    ));
    Ok(())
}

/// P10 PR 3: a cancelled copy stops between blocks, removes its partial
/// private file and never touches the original; an uncancelled one of the
/// same source still stages.
#[tokio::test]
async fn a_cancelled_copy_stops_and_leaves_no_partial_file() -> TestResult {
    let root = OwnedRoot::create()?;
    let (store, session_id) = workspace(&root).await?;
    let corpus =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/generated/F01.mp4");
    let original = fs::read(&corpus)?;
    let source = root.0.join("source.mp4");
    fs::write(&source, &original)?;
    let cancellation = ProcessCancellation::new();
    cancellation.cancel();
    let result = SourceSnapshot::stage_cancellable(
        &store,
        &session_id,
        &OperationId::parse("op_abcdef0123456789")?,
        &source,
        &cancellation,
    );
    assert!(matches!(result, Err(SourceError::Cancelled)));
    let artifacts = root
        .0
        .join("workspace/sessions")
        .join(session_id.as_str())
        .join("artifacts");
    let left: Vec<_> = fs::read_dir(&artifacts)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<_, _>>()?;
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(fs::read(&source)?, original);

    let staged = SourceSnapshot::stage_cancellable(
        &store,
        &session_id,
        &OperationId::parse("op_abcdef0123456780")?,
        &source,
        &ProcessCancellation::new(),
    )?;
    staged.verify()?;
    Ok(())
}

/// Makes `link` a symbolic link to `target`; `false` when the platform will not
/// allow it (an unprivileged Windows account), in which case a test that needs
/// one has nothing to check there.
fn make_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    made.is_ok()
}

/// #265: a link named as the source is refused as a source problem, never
/// followed and never a storage failure, whether it points at a file or at
/// nothing; the file behind it is not read, and nothing is staged.
#[tokio::test]
async fn a_link_is_refused_as_a_source_and_never_followed() -> TestResult {
    let root = OwnedRoot::create()?;
    let (store, session_id) = workspace(&root).await?;
    let target = root.0.join("target.mp4");
    fs::write(&target, b"\0\0\0\x18ftypisomtarget")?;
    let link = root.0.join("link.mp4");
    let dangling = root.0.join("dangling.mp4");
    if !make_link(&target, &link) || !make_link(&root.0.join("missing.mp4"), &dangling) {
        return Ok(());
    }

    for (index, source) in [&link, &dangling].into_iter().enumerate() {
        let result = SourceSnapshot::stage(
            &store,
            &session_id,
            &OperationId::parse(format!("op_abcdef012345678{index}"))?,
            source,
        );
        assert!(
            matches!(result, Err(SourceError::SymbolicLink)),
            "{source:?}: {:?}",
            result.err()
        );
    }
    let artifacts = root
        .0
        .join("workspace/sessions")
        .join(session_id.as_str())
        .join("artifacts");
    assert_eq!(fs::read_dir(artifacts)?.count(), 0, "nothing was staged");

    // The file itself is still accepted: only the link is refused.
    let staged = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_abcdef0123456789")?,
        &target,
    )?;
    staged.verify()?;
    Ok(())
}
