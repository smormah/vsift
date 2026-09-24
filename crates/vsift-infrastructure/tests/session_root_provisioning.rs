//! First-use provisioning of a session root by concurrent processes and threads
//! (issue #131), and the ownership checks that adoption must still pass.

use std::{
    env,
    error::Error,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use vsift_infrastructure::{
    FilesystemSessionStore, SessionRootError, SessionRootProvisioning, SessionStoreOpenError,
    open_session_root,
};

type TestResult = Result<(), Box<dyn Error>>;

const CHILD_ROOT: &str = "VSIFT_ROOT_RACE_ROOT";
const CHILD_GATE: &str = "VSIFT_ROOT_RACE_GATE";
const CHILD_MODE: &str = "VSIFT_ROOT_RACE_MODE";
const CREATORS: usize = 6;
const READERS: usize = 2;
/// Well under the five-second provisioning wait: a rejection that arrives
/// sooner than this was not spent waiting for a creator.
const PROMPT: Duration = Duration::from_secs(4);
const MARKER: &[u8] =
    br#"{"schema_version":1,"application":"vsift","layout_version":1,"admission_capacity":4}"#;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

/// A private scratch directory; the session root is created two levels below
/// it, so racing openers also race to create the root's parent.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "vsift-root-race-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn root(&self) -> PathBuf {
        self.0.join("cache").join("vsift-sessions")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("vsift-root-race-"))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

struct ManagedChild(Child);

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Describes an open outcome by kind only, never by path.
fn outcome(result: &Result<Option<FilesystemSessionStore>, SessionRootError>) -> String {
    match result {
        Ok(Some(_)) => String::from("opened"),
        Ok(None) => String::from("absent"),
        Err(error) => format!("error: {error}"),
    }
}

/// Owner-only on Unix, so the ownership check rather than the mode check is
/// what rejects the directory.
#[cfg(unix)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(windows)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

fn sorted_names(directory: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut names = fs::read_dir(directory)?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| "non-UTF-8 entry name".into())
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    names.sort();
    Ok(names)
}

/// Exactly one complete, valid root exists and nothing else was left behind.
fn assert_single_valid_root(scratch: &Scratch) -> TestResult {
    let root = scratch.root();
    assert_eq!(sorted_names(&scratch.0)?, ["cache"]);
    assert_eq!(sorted_names(&scratch.0.join("cache"))?, ["vsift-sessions"]);
    assert_eq!(
        sorted_names(&root)?,
        ["coordination", "ownership.json", "sessions"]
    );
    assert_eq!(
        sorted_names(&root.join("coordination"))?,
        [
            "admission-000.lock",
            "admission-001.lock",
            "admission-002.lock",
            "admission-003.lock",
            "session-initialize.lock",
        ]
    );
    assert!(sorted_names(&root.join("sessions"))?.is_empty());
    assert_eq!(fs::read(root.join("ownership.json"))?, MARKER);
    let store = FilesystemSessionStore::open_existing(&root)?;
    assert_eq!(store.admission_capacity(), 4);
    Ok(())
}

#[test]
#[ignore = "internal child entry launched by the concurrent-provisioning parent test"]
fn provisioning_race_child() -> TestResult {
    let root = PathBuf::from(env::var_os(CHILD_ROOT).ok_or("missing child root")?);
    let gate = PathBuf::from(env::var_os(CHILD_GATE).ok_or("missing child gate")?);
    let provisioning = match env::var(CHILD_MODE)?.as_str() {
        "create" => SessionRootProvisioning::CreateIfMissing,
        "existing" => SessionRootProvisioning::ExistingOnly,
        _ => return Err("unknown child mode".into()),
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    while !gate.exists() {
        if Instant::now() >= deadline {
            return Err("start gate never opened".into());
        }
        thread::sleep(Duration::from_millis(1));
    }
    let result = open_session_root(&root, provisioning);
    let described = outcome(&result);
    println!("outcome={described}");
    match (provisioning, result) {
        (_, Ok(Some(_))) | (SessionRootProvisioning::ExistingOnly, Ok(None)) => Ok(()),
        _ => Err(described.into()),
    }
}

fn spawn_child(root: &Path, gate: &Path, mode: &str) -> Result<ManagedChild, Box<dyn Error>> {
    let child = Command::new(env::current_exe()?)
        .args([
            "--exact",
            "provisioning_race_child",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_ROOT, root)
        .env(CHILD_GATE, gate)
        .env(CHILD_MODE, mode)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(ManagedChild(child))
}

/// Waits for a child within a watchdog bound and returns its outcome line.
fn finish_child(child: &mut ManagedChild, deadline: Instant) -> Result<String, Box<dyn Error>> {
    loop {
        if let Some(status) = child.0.try_wait()? {
            let mut output = String::new();
            if let Some(stdout) = child.0.stdout.as_mut() {
                stdout.read_to_string(&mut output)?;
            }
            let line = output
                .lines()
                .find_map(|line| line.strip_prefix("outcome="))
                .unwrap_or("no outcome reported")
                .to_owned();
            if status.success() {
                return Ok(line);
            }
            return Err(format!("child failed ({status}): {line}").into());
        }
        if Instant::now() >= deadline {
            return Err("child exceeded the watchdog bound".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn concurrent_processes_create_one_root_and_every_one_adopts_it() -> TestResult {
    let scratch = Scratch::new()?;
    let root = scratch.root();
    let gate = scratch.0.join("start-gate");
    let mut children = Vec::with_capacity(CREATORS + READERS);
    for index in 0..CREATORS + READERS {
        let mode = if index < CREATORS {
            "create"
        } else {
            "existing"
        };
        children.push(spawn_child(&root, &gate, mode)?);
    }
    // Every child is already running and polling; the gate releases them together.
    fs::write(&gate, b"go")?;

    let deadline = Instant::now() + Duration::from_secs(120);
    let mut opened = 0_usize;
    for child in &mut children {
        if finish_child(child, deadline)? == "opened" {
            opened += 1;
        }
    }
    fs::remove_file(&gate)?;

    assert!(opened >= CREATORS, "every creator opened the root");
    assert_single_valid_root(&scratch)
}

#[test]
fn concurrent_threads_create_one_root_and_every_one_adopts_it() -> TestResult {
    const THREADS: usize = 8;
    let scratch = Scratch::new()?;
    let root = Arc::new(scratch.root());
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles = (0..THREADS)
        .map(|index| {
            let root = Arc::clone(&root);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let provisioning = if index % 4 == 3 {
                    SessionRootProvisioning::ExistingOnly
                } else {
                    SessionRootProvisioning::CreateIfMissing
                };
                barrier.wait();
                let result = open_session_root(&root, provisioning);
                (provisioning, outcome(&result), result.is_ok())
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        let (provisioning, described, succeeded) =
            handle.join().map_err(|_| "provisioning thread panicked")?;
        assert!(succeeded, "open failed: {described}");
        if provisioning == SessionRootProvisioning::CreateIfMissing {
            assert_eq!(described, "opened");
        }
    }
    assert_single_valid_root(&scratch)
}

fn assert_rejected_promptly(root: &Path, expected: SessionStoreOpenError) {
    for provisioning in [
        SessionRootProvisioning::ExistingOnly,
        SessionRootProvisioning::CreateIfMissing,
    ] {
        let started = Instant::now();
        let result = open_session_root(root, provisioning);
        let elapsed = started.elapsed();
        let described = outcome(&result);
        assert!(
            matches!(result, Err(SessionRootError::Store(error)) if error == expected),
            "unexpected outcome: {described}"
        );
        assert!(elapsed < PROMPT, "rejection waited for a creator");
    }
}

#[test]
fn an_unmarked_directory_with_content_is_rejected_and_left_untouched() -> TestResult {
    let scratch = Scratch::new()?;
    let root = scratch.root();
    fs::create_dir(scratch.0.join("cache"))?;
    create_private_directory(&root)?;
    fs::write(root.join("notes.txt"), b"not a vsift root")?;

    assert_rejected_promptly(&root, SessionStoreOpenError::InvalidOwnership);

    assert_eq!(sorted_names(&root)?, ["notes.txt"]);
    assert_eq!(fs::read(root.join("notes.txt"))?, b"not a vsift root");
    Ok(())
}

#[test]
fn a_foreign_ownership_marker_is_rejected() -> TestResult {
    let scratch = Scratch::new()?;
    let root = scratch.root();
    fs::create_dir(scratch.0.join("cache"))?;
    drop(FilesystemSessionStore::provision_default(&root)?);
    let foreign =
        br#"{"schema_version":1,"application":"other","layout_version":1,"admission_capacity":4}"#;
    fs::remove_file(root.join("ownership.json"))?;
    fs::write(root.join("ownership.json"), foreign)?;

    assert_rejected_promptly(&root, SessionStoreOpenError::InvalidOwnership);

    assert_eq!(fs::read(root.join("ownership.json"))?, foreign);
    Ok(())
}

#[test]
fn a_hard_linked_ownership_marker_is_rejected() -> TestResult {
    let scratch = Scratch::new()?;
    let root = scratch.root();
    fs::create_dir(scratch.0.join("cache"))?;
    drop(FilesystemSessionStore::provision_default(&root)?);
    let outside = scratch.0.join("outside-marker.json");
    fs::write(&outside, MARKER)?;
    fs::remove_file(root.join("ownership.json"))?;
    fs::hard_link(&outside, root.join("ownership.json"))?;

    assert_rejected_promptly(&root, SessionStoreOpenError::InvalidOwnership);

    assert_eq!(fs::read(&outside)?, MARKER);
    Ok(())
}
