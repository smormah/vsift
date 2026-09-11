//! Bounded P03 API experiments, not a production storage adapter or durability proof.

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::error::Error;
use std::fs::{self, File, TryLockError};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type TestResult = Result<(), Box<dyn Error>>;
const WATCHDOG: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(10);
const OLD: &[u8] = b"synthetic generation 0";
const NEW: &[u8] = b"synthetic generation 1";

// Flat fixtures deliberately avoid recursive cleanup and never accept media paths.
struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "vsift-p03-probe-{}-{stamp}-{label}",
            std::process::id()
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.recursive(false).create(&path)?;
        Ok(Self { path })
    }

    fn dir(&self) -> io::Result<Dir> {
        Dir::open_ambient_dir(&self.path, cap_std::ambient_authority())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Exact fixture-owned entries only. Unexpected content is left for inspection.
        for name in ["current", "next", "anchor", "ready", "source", "alias"] {
            let _ = fs::remove_file(self.path.join(name));
        }
        #[cfg(unix)]
        let _ = fs::remove_file(self.path.join("link"));
        let _ = fs::remove_dir(self.path.join("child"));
        let _ = fs::remove_dir(self.path.join("moved"));
        let _ = fs::remove_dir(&self.path);
    }
}

fn create(dir: &Dir, name: &str, bytes: &[u8]) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    options.follow(FollowSymlinks::No);
    let mut file = dir.open_with(name, &options)?.into_std();
    file.write_all(bytes)?;
    Ok(file)
}

struct ProbeChild(Child);

impl ProbeChild {
    fn start(fixture: &Fixture, phase: &str) -> io::Result<Self> {
        Command::new(std::env::current_exe()?)
            .args(["--ignored", "--exact", "storage_probe_child", "--nocapture"])
            .env("VSIFT_STORAGE_PROBE_ROOT", &fixture.path)
            .env("VSIFT_STORAGE_PROBE_PHASE", phase)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(Self)
    }

    fn ready(&mut self, fixture: &Fixture) -> io::Result<()> {
        let started = Instant::now();
        while started.elapsed() < WATCHDOG {
            if fixture.path.join("ready").try_exists()? {
                return Ok(());
            }
            if self.0.try_wait()?.is_some() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            thread::sleep(POLL);
        }
        Err(io::ErrorKind::TimedOut.into())
    }

    fn kill_and_reap(&mut self) -> io::Result<()> {
        self.0.kill()?;
        self.0.wait()?;
        Ok(())
    }
}

impl Drop for ProbeChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "internal child fixture; launched by bounded parent tests"]
fn storage_probe_child() -> TestResult {
    let root = std::env::var_os("VSIFT_STORAGE_PROBE_ROOT")
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let phase = std::env::var("VSIFT_STORAGE_PROBE_PHASE")?;
    let dir = Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
    let anchor = dir
        .open_with("anchor", OpenOptions::new().read(true).write(true))?
        .into_std();
    anchor.try_lock()?;
    match phase.as_str() {
        "locked" => {}
        "written" | "flushed" | "published" => {
            let next = create(&dir, "next", NEW)?;
            if phase != "written" {
                next.sync_all()?;
            }
            drop(next);
            if phase == "published" {
                dir.rename("next", &dir, "current")?;
            }
        }
        _ => return Err(io::Error::from(io::ErrorKind::InvalidInput).into()),
    }
    drop(create(&dir, "ready", b"ready")?);
    // Parent kills us at the selected boundary. Self-limit if the parent fails.
    let started = Instant::now();
    while started.elapsed() < WATCHDOG * 2 {
        thread::sleep(POLL);
    }
    Err(io::Error::from(io::ErrorKind::TimedOut).into())
}

#[test]
fn s07_probe_file_and_directory_flush() -> TestResult {
    let fixture = Fixture::new("flush")?;
    let dir = fixture.dir()?;
    let file = create(&dir, "next", NEW)?;
    file.sync_all()?;
    drop(file);
    dir.rename("next", &dir, "current")?;
    let result = dir.into_std_file().sync_all();
    match &result {
        Ok(()) => println!("P03_DIRECTORY_SYNC=ok; OS-crash qualification still required"),
        Err(error) => println!(
            "P03_DIRECTORY_SYNC=failed kind={:?} os={:?}; durability gate NOT met",
            error.kind(),
            error.raw_os_error()
        ),
    }
    // This records a candidate's capability; neither return value qualifies durability.
    assert_eq!(fs::read(fixture.path.join("current"))?, NEW);
    Ok(())
}

#[cfg(windows)]
#[test]
fn s07_probe_windows_directory_write_access() -> TestResult {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let fixture = Fixture::new("directory-write")?;
    // Test whether requesting the access required by FlushFileBuffers resolves
    // the read-only directory-handle failure. This does not grant a volume handle.
    let result = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(&fixture.path)
        .and_then(|file| file.sync_all());
    match result {
        Ok(()) => println!("P03_WRITABLE_DIRECTORY_SYNC=ok; publication remains unqualified"),
        Err(error) => println!(
            "P03_WRITABLE_DIRECTORY_SYNC=failed kind={:?} os={:?}",
            error.kind(),
            error.raw_os_error()
        ),
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn s07_probe_relative_readable_directory_flush() -> TestResult {
    let fixture = Fixture::new("readable-directory")?;
    let dir = fixture.dir()?;
    // Linux capability directory handles can be O_PATH descriptors, which cannot
    // fsync. Reopen "." relative to that held directory with actual read access.
    dir.open(".")?.sync_all()?;
    println!("P03_RELATIVE_READABLE_DIRECTORY_SYNC=ok; publication remains unqualified");
    Ok(())
}

#[test]
fn s01_probe_relative_escape_denied() -> TestResult {
    let fixture = Fixture::new("escape")?;
    let outside = Fixture::new("sentinel")?;
    let outside_dir = outside.dir()?;
    drop(create(&outside_dir, "source", OLD)?);
    let dir = fixture.dir()?;
    let sibling = outside
        .path
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let traversal = PathBuf::from("..").join(sibling).join("source");
    assert!(dir.open(&traversal).is_err());
    assert!(dir.open(outside.path.join("source")).is_err());
    assert_eq!(outside_dir.read("source")?, OLD);
    Ok(())
}

#[test]
fn s02_probe_hard_links_require_additional_identity_policy() -> TestResult {
    let fixture = Fixture::new("hardlink")?;
    let outside = Fixture::new("hardlink-sentinel")?;
    let outside_dir = outside.dir()?;
    drop(create(&outside_dir, "source", OLD)?);
    fs::hard_link(outside.path.join("source"), fixture.path.join("alias"))?;
    let dir = fixture.dir()?;
    // No-follow alone does not exclude an external inode with an in-root hard link.
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut bytes = Vec::new();
    dir.open_with("alias", &options)?.read_to_end(&mut bytes)?;
    assert_eq!(bytes, OLD);
    assert!(create(&dir, "alias", NEW).is_err());
    assert_eq!(outside_dir.read("source")?, OLD);
    Ok(())
}

#[cfg(unix)]
#[test]
fn s02_probe_symlink_nofollow_and_parent_substitution() -> TestResult {
    let fixture = Fixture::new("symlink")?;
    let dir = fixture.dir()?;
    dir.create_dir("child")?;
    dir.symlink_dir("child", "link")?;
    assert!(dir.open_dir_nofollow("link").is_err());
    let held = dir.open_dir_nofollow("child")?;
    dir.rename("child", &dir, "moved")?;
    dir.create_dir("child")?;
    drop(create(&held, "next", NEW)?);
    assert!(!dir.open_dir("child")?.try_exists("next")?);
    assert_eq!(dir.open_dir("moved")?.read("next")?, NEW);
    held.remove_file("next")?;
    Ok(())
}

#[cfg(windows)]
#[test]
fn s02_probe_held_windows_directory_denies_substitution() -> TestResult {
    let fixture = Fixture::new("directory-hold")?;
    let dir = fixture.dir()?;
    dir.create_dir("child")?;
    let held = dir.open_dir_nofollow("child")?;
    assert!(dir.rename("child", &dir, "moved").is_err());
    drop(held);
    dir.rename("child", &dir, "moved")?;
    Ok(())
}

#[test]
fn s03_probe_open_handle_is_not_immutable_snapshot() -> TestResult {
    let fixture = Fixture::new("source")?;
    let dir = fixture.dir()?;
    drop(create(&dir, "source", OLD)?);
    let mut source = dir.open("source")?;
    // An owned synthetic file only: same-size in-place mutation preserves its handle.
    let mut other = dir.open_with("source", OpenOptions::new().write(true))?;
    other.write_all(NEW)?;
    drop(other);
    let mut bytes = Vec::new();
    source.read_to_end(&mut bytes)?;
    assert_eq!(bytes, NEW);
    Ok(())
}

#[test]
fn s12_x05_probe_os_lock_survives_idle_owner_then_releases_on_kill() -> TestResult {
    let fixture = Fixture::new("lock")?;
    let dir = fixture.dir()?;
    let anchor = create(&dir, "anchor", b"stable anchor")?;
    let mut child = ProbeChild::start(&fixture, "locked")?;
    child.ready(&fixture)?;
    for _ in 0..8 {
        assert!(matches!(anchor.try_lock(), Err(TryLockError::WouldBlock)));
        thread::sleep(POLL);
    }
    child.kill_and_reap()?;
    anchor.try_lock()?;
    anchor.unlock()?;
    assert_eq!(dir.read("anchor")?, b"stable anchor");
    Ok(())
}

#[test]
fn x04_probe_shared_holds_exclude_writer() -> TestResult {
    let fixture = Fixture::new("read-holds")?;
    let dir = fixture.dir()?;
    let first = create(&dir, "anchor", b"stable anchor")?;
    let second = dir
        .open_with("anchor", OpenOptions::new().read(true).write(true))?
        .into_std();
    let writer = dir
        .open_with("anchor", OpenOptions::new().read(true).write(true))?
        .into_std();
    first.try_lock_shared()?;
    second.try_lock_shared()?;
    assert!(matches!(writer.try_lock(), Err(TryLockError::WouldBlock)));
    first.unlock()?;
    assert!(matches!(writer.try_lock(), Err(TryLockError::WouldBlock)));
    second.unlock()?;
    writer.try_lock()?;
    writer.unlock()?;
    Ok(())
}

#[test]
fn x01_probe_kill_before_and_after_pointer_replacement() -> TestResult {
    for phase in ["written", "flushed", "published"] {
        let fixture = Fixture::new(phase)?;
        let dir = fixture.dir()?;
        drop(create(&dir, "anchor", b"stable anchor")?);
        drop(create(&dir, "current", OLD)?);
        let mut child = ProbeChild::start(&fixture, phase)?;
        child.ready(&fixture)?;
        child.kill_and_reap()?;
        // Reopen from the root after abrupt process death; never infer from staging.
        let recovered = fixture.dir()?.read("current")?;
        assert_eq!(recovered, if phase == "published" { NEW } else { OLD });
    }
    Ok(())
}

#[test]
fn x04_probe_reader_handle_across_atomic_replacement() -> TestResult {
    let fixture = Fixture::new("reader")?;
    let dir = fixture.dir()?;
    drop(create(&dir, "current", OLD)?);
    let mut reader = dir.open("current")?;
    drop(create(&dir, "next", NEW)?);
    dir.rename("next", &dir, "current")?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    assert_eq!(bytes, OLD);
    assert_eq!(dir.read("current")?, NEW);
    Ok(())
}
