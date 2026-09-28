//! X-07 (P11, ADR 0021 section 5a): weighted admission never lets the work
//! admitted at once weigh more than the root's capacity, across processes,
//! on the real OS locks.
//!
//! Several child processes (this test binary re-run on an ignored entry)
//! race to reserve the weights the engine uses (a copy or an evidence
//! extraction 1, a visual window 2, a recognition its threads up to the
//! root's capacity) on roots of capacity 2, 4 and 8. Each child adds its
//! weight to a shared ledger after its reservation is granted and removes it
//! before releasing, under the ledger file's own exclusive lock, so the
//! ledger only ever counts reservations that are held at that moment; a
//! ledger above the capacity is an oversubscription. Every reservation
//! attempt must be granted or refused as busy (the two documented outcomes
//! of contention); anything else fails the run. A weight above the capacity
//! is refused as a capacity error before anything is held.

use std::{
    env,
    error::Error,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use vsift_application::SessionStorageError;
use vsift_infrastructure::FilesystemSessionStore;

type TestResult = Result<(), Box<dyn Error>>;

const CHILD_ROOT: &str = "VSIFT_X07_ADMISSION_ROOT";
const CHILD_LEDGER: &str = "VSIFT_X07_ADMISSION_LEDGER";
const CHILD_SEED: &str = "VSIFT_X07_ADMISSION_SEED";
const CHILDREN: usize = 5;
/// How long each child keeps reserving and releasing.
const CHILD_RUN: Duration = Duration::from_millis(1_500);
const OWNED_PREFIX: &str = "vsift-x07-admission-";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct OwnedDirectory(PathBuf);

impl OwnedDirectory {
    fn new() -> Result<Self, Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for OwnedDirectory {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// The weights the engine reserves, as far as a root of `capacity` admits
/// them: 1 (copy, evidence), 2 (a visual window) and every recognizer
/// thread count up to the capacity and the cap of eight.
fn weights(capacity: u16) -> Vec<u16> {
    (1..=capacity.min(8)).collect()
}

/// A small deterministic generator, so each child's schedule differs
/// without a random-number dependency.
struct Schedule(u64);

impl Schedule {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

/// The ledger: the total weight held right now, as eight little-endian bytes,
/// and the most it has been.
struct Ledger(File);

impl Ledger {
    fn open(path: &Path) -> Result<Self, Box<dyn Error>> {
        Ok(Self(OpenOptions::new().read(true).write(true).open(path)?))
    }

    /// Adds `delta` (negative to release) under the ledger's exclusive lock
    /// and returns the new total.
    fn change(&mut self, delta: i64) -> Result<i64, Box<dyn Error>> {
        self.0.lock()?;
        let result = self.apply(delta);
        self.0.unlock()?;
        result
    }

    fn apply(&mut self, delta: i64) -> Result<i64, Box<dyn Error>> {
        let mut bytes = [0_u8; 16];
        self.0.seek(SeekFrom::Start(0))?;
        self.0.read_exact(&mut bytes)?;
        let (current, peak) = bytes.split_at(8);
        let current = i64::from_le_bytes(current.try_into()?);
        let peak = i64::from_le_bytes(peak.try_into()?);
        let total = current + delta;
        let peak = peak.max(total);
        self.0.seek(SeekFrom::Start(0))?;
        self.0.write_all(&total.to_le_bytes())?;
        self.0.write_all(&peak.to_le_bytes())?;
        self.0.sync_data()?;
        Ok(total)
    }
}

#[test]
#[ignore = "internal child entry launched by weighted_admission_never_exceeds_root_capacity"]
fn weighted_admission_child() -> TestResult {
    let root = PathBuf::from(env::var_os(CHILD_ROOT).ok_or("missing root")?);
    let ledger_path = PathBuf::from(env::var_os(CHILD_LEDGER).ok_or("missing ledger")?);
    let mut schedule = Schedule(env::var(CHILD_SEED)?.parse()?);
    let store = FilesystemSessionStore::open_existing(root)?;
    let capacity = store.admission_capacity();
    let weights = weights(capacity);
    let mut ledger = Ledger::open(&ledger_path)?;
    let started = Instant::now();
    let mut granted = 0_u32;
    while started.elapsed() < CHILD_RUN {
        let index = usize::try_from(schedule.next())? % weights.len();
        let weight = weights[index];
        match store.try_admit(weight) {
            Ok(permit) => {
                granted += 1;
                let total = ledger.change(i64::from(weight))?;
                if total > i64::from(capacity) {
                    return Err(format!("{total} units held on a root of {capacity}").into());
                }
                thread::sleep(Duration::from_micros(schedule.next() % 2_000));
                ledger.change(-i64::from(weight))?;
                drop(permit);
            }
            Err(SessionStorageError::Busy) => {
                thread::sleep(Duration::from_micros(schedule.next() % 500));
            }
            Err(other) => return Err(format!("weight {weight}: unexpected {other:?}").into()),
        }
    }
    if granted == 0 {
        return Err("no reservation was ever granted".into());
    }
    Ok(())
}

fn spawn_child(root: &Path, ledger: &Path, seed: u64) -> Result<Child, Box<dyn Error>> {
    Ok(Command::new(env::current_exe()?)
        .args(["--exact", "weighted_admission_child", "--ignored"])
        .env(CHILD_ROOT, root)
        .env(CHILD_LEDGER, ledger)
        .env(CHILD_SEED, seed.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?)
}

/// X-07: on real OS locks, across processes, the reservations held at once
/// never weigh more than the root's capacity, at capacities 2, 4 and 8.
#[test]
fn weighted_admission_never_exceeds_root_capacity() -> TestResult {
    for capacity in [2_u16, 4, 8] {
        let owned = OwnedDirectory::new()?;
        let root = owned.0.join("root");
        FilesystemSessionStore::provision(&root, capacity)?;
        let ledger = owned.0.join("ledger");
        fs::write(&ledger, [0_u8; 16])?;
        let children = (0..CHILDREN)
            .map(|index| {
                spawn_child(
                    &root,
                    &ledger,
                    u64::from(capacity) * 1_000 + u64::try_from(index).unwrap_or(0),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut failures = Vec::new();
        for child in children {
            let output = child.wait_with_output()?;
            if !output.status.success() {
                failures.push(String::from_utf8_lossy(&output.stdout).into_owned());
            }
        }
        assert!(failures.is_empty(), "capacity {capacity}: {failures:#?}");
        let bytes = fs::read(&ledger)?;
        let (current, peak) = bytes.split_at(8);
        assert_eq!(i64::from_le_bytes(current.try_into()?), 0);
        let peak = i64::from_le_bytes(peak.try_into()?);
        assert!(
            (1..=i64::from(capacity)).contains(&peak),
            "capacity {capacity}: peak {peak}"
        );
    }
    Ok(())
}

/// A reservation heavier than the whole root is refused as a capacity
/// error before anything is held, and a full-weight one is granted alone.
#[test]
fn a_request_heavier_than_the_root_is_refused_before_anything_is_held() -> TestResult {
    let owned = OwnedDirectory::new()?;
    let root = owned.0.join("root");
    let store = FilesystemSessionStore::provision(&root, 3)?;
    assert!(matches!(
        store.try_admit(4),
        Err(SessionStorageError::CapacityExhausted)
    ));
    let whole = store.try_admit(3)?;
    assert!(matches!(store.try_admit(1), Err(SessionStorageError::Busy)));
    drop(whole);
    let _two = store.try_admit(2)?;
    let _one = store.try_admit(1)?;
    Ok(())
}
