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

/// #264: a named pipe that nothing writes to is refused as not a regular
/// file. Opening it for reading blocks until a writer appears, so the open
/// used to wait for ever (`vsift ingest <pipe>` had to be killed); the file
/// type is now found on a handle that was opened without waiting.
#[cfg(unix)]
#[tokio::test]
async fn a_named_pipe_with_no_writer_is_refused_and_not_waited_for() -> TestResult {
    use std::{sync::mpsc, time::Duration};

    let root = OwnedRoot::create()?;
    let (store, session_id) = workspace(&root).await?;
    let pipe = root.0.join("pipe.mp4");
    // The system's own `mkfifo` (rustix has no `mknodat` on macOS).
    let made = std::process::Command::new("mkfifo").arg(&pipe).status()?;
    assert!(made.success(), "mkfifo failed: {made:?}");
    let operation = OperationId::parse("op_abcdef0123456789")?;
    // Staged on its own thread: a build that waits would otherwise hang the
    // whole test run instead of failing this test.
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = SourceSnapshot::stage(&store, &session_id, &operation, &pipe);
        let _ = sender.send(matches!(result, Err(SourceError::NotRegularFile)));
    });
    let refused = receiver
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| "staging a named pipe with no writer did not return within 30 s")?;
    assert!(refused, "the pipe was not refused as not a regular file");
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

/// Makes `link` a symbolic link to `target`.
///
/// `Ok(false)` is an **explicit skip** and only ever happens on Windows, for an
/// account that does not hold the privilege to create links (error 1314), and
/// never when `CI` is set: the test prints why it checked nothing, and a hosted
/// run, where links can always be made, fails instead of skipping. Every other
/// failure is an error.
fn make_link(target: &std::path::Path, link: &std::path::Path) -> Result<bool, Box<dyn Error>> {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    match made {
        Ok(()) => Ok(true),
        Err(error)
            if cfg!(windows)
                && error.raw_os_error() == Some(1314)
                && env::var_os("CI").is_none() =>
        {
            eprintln!(
                "SKIPPED: this account may not create symbolic links ({error}); the test checks nothing here"
            );
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

/// #265: a link named as the source is refused as a link and never followed,
/// whether it points at a file or at nothing; the file behind it is not read,
/// and nothing is staged.
#[tokio::test]
async fn a_link_is_refused_as_a_source_and_never_followed() -> TestResult {
    let root = OwnedRoot::create()?;
    let (store, session_id) = workspace(&root).await?;
    let target = root.0.join("target.mp4");
    fs::write(&target, b"\0\0\0\x18ftypisomtarget")?;
    let link = root.0.join("link.mp4");
    let dangling = root.0.join("dangling.mp4");
    if !make_link(&target, &link)? || !make_link(&root.0.join("missing.mp4"), &dangling)? {
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

/// #264 and #265, what a source can be: a device file is opened and found not to
/// be a regular file (`NotRegularFile`, `INVALID_SOURCE`); a UNIX socket cannot
/// be opened at all, so it is an I/O failure (`STORAGE_IO`, an answer that
/// describes it only loosely, known limit L-127). This pins both, so a change in
/// either is a decision and not an accident.
#[cfg(unix)]
#[tokio::test]
async fn a_device_is_not_a_regular_file_and_a_socket_cannot_be_opened() -> TestResult {
    let root = OwnedRoot::create()?;
    let (store, session_id) = workspace(&root).await?;
    let device = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_abcdef0123456789")?,
        std::path::Path::new("/dev/null"),
    );
    assert!(
        matches!(device, Err(SourceError::NotRegularFile)),
        "{:?}",
        device.err()
    );

    // A short path: a socket's address is limited to about a hundred bytes.
    let socket_path = PathBuf::from(format!(
        "/tmp/vsift-p04-{}-{}.sock",
        std::process::id(),
        NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
    ));
    let listener = std::os::unix::net::UnixListener::bind(&socket_path)?;
    let socket = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_abcdef0123456780")?,
        &socket_path,
    );
    drop(listener);
    let _ = fs::remove_file(&socket_path);
    assert!(
        matches!(socket, Err(SourceError::Io(_))),
        "{:?}",
        socket.err()
    );
    Ok(())
}
