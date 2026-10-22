//! The root's queue of queued `prepare` jobs, and the capacity queued work
//! gives back (ADR 0033 section 6).
//!
//! **The queue.** `job start --mode queued` enters its job in `job-queue/`, one
//! file a job named by a stamp of when it was entered and its id, so that a
//! listing in name order is first in, first out and survives a restart. The
//! stamp is the second times 10^9, raised past the newest entry's while
//! `coordination/job-queue.lock` is held, so two jobs entered in one second
//! keep the order they arrived in (RV-1 finding 8). Only one queued
//! job of a root runs at a time: its runner holds `coordination/queue-turn.lock`
//! for the whole job. An entry whose job has ended is removed when the job
//! ends, and skipped (and removed) by whoever finds it later.
//!
//! **Fairness.** Queued work takes the root's whole capacity, marked by an
//! exclusive lock on `coordination/admission-queued.lock` for as long as it
//! holds it. An ordinary caller that finds the capacity taken while that
//! marker is held does not answer `BUSY` at once: it holds a shared lock on
//! `coordination/admission-demand.lock` and waits up to
//! [`vsift_domain::QUEUED_YIELD_WAIT`] for the slots, and the queued work,
//! which looks at the demand lock before every chunk, gives its permit back
//! and waits until nobody demands capacity any more. Every lock is an
//! operating-system lock, so a process that dies releases what it held.

use std::fs;

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use vsift_application::SessionStorageError;
use vsift_domain::{JobId, SessionId};

use super::{
    COORDINATION_DIRECTORY, FilesystemAdmissionPermit, FilesystemSessionStore,
    job_records::StoredJobIndex,
    jobs::{create_directory, create_or_open, replace_file},
    map_committed_io, map_storage_io,
    stored::read_versioned_json_file,
};
use crate::file_lock::HeldFileLock;

/// The root directory of the queue.
const JOB_QUEUE_DIRECTORY: &str = "job-queue";
/// Held by the runner whose queued job runs.
const QUEUE_TURN_LOCK: &str = "queue-turn.lock";
/// Held while an entry is entered, so that no two entries share a stamp.
const QUEUE_ENTRY_LOCK: &str = "job-queue.lock";
/// Stamps a second: an entry's stamp is its second times this, raised past
/// the newest entry's stamp.
const STAMPS_PER_SECOND: u64 = 1_000_000_000;
/// How often, and how many times, entering a job looks for the entry lock,
/// which is only ever held for the few file operations of one entry.
const QUEUE_ENTRY_LOCK_POLL: std::time::Duration = std::time::Duration::from_millis(10);
const QUEUE_ENTRY_LOCK_TRIES: u32 = 500;
/// Held exclusively by queued work while it holds capacity.
pub(super) const ADMISSION_QUEUED_LOCK: &str = "admission-queued.lock";
/// Held shared by ordinary callers that wait for capacity queued work holds.
pub(super) const ADMISSION_DEMAND_LOCK: &str = "admission-demand.lock";
/// Most jobs one root's queue holds.
pub const MAX_QUEUED_JOBS: usize = 256;

/// One entry of the queue, in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedEntry {
    /// The job's session.
    pub session_id: SessionId,
    /// The job.
    pub job_id: JobId,
    /// The second it was entered, from its stamp.
    pub entered_at_unix_seconds: u64,
}

/// The turn of the queued job that runs: held for the whole job, released when
/// dropped.
#[derive(Debug)]
pub struct QueueTurn {
    _lock: HeldFileLock,
}

fn entry_name(stamp: u64, job_id: &JobId) -> String {
    format!("{stamp:020}-{}.json", job_id.as_str())
}

/// The stamp and the job an entry's name holds, if it is an entry.
fn entry_parts(name: &str) -> Option<(u64, JobId)> {
    let (stamp, rest) = name.split_once('-')?;
    if stamp.len() != 20 || !stamp.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((
        stamp.parse().ok()?,
        JobId::parse(rest.strip_suffix(".json")?).ok()?,
    ))
}

/// The job an entry's name holds, if it is an entry.
fn entry_job(name: &str) -> Option<JobId> {
    entry_parts(name).map(|(_, job)| job)
}

/// The stamp of an entry entered at `now_unix_seconds` after the entries
/// `names`: the second times [`STAMPS_PER_SECOND`], or one past the newest
/// stamp when that is later, so the order of entering is kept within a second
/// and when the clock steps back.
fn next_stamp(names: &[String], now_unix_seconds: u64) -> u64 {
    let newest = names
        .iter()
        .filter_map(|name| entry_parts(name))
        .map(|(stamp, _)| stamp.saturating_add(1))
        .max()
        .unwrap_or(0);
    now_unix_seconds
        .saturating_mul(STAMPS_PER_SECOND)
        .max(newest)
}

impl FilesystemSessionStore {
    fn queue_directory(&self, create: bool) -> Result<Option<Dir>, SessionStorageError> {
        self.revalidate_root()?;
        if !self
            .root
            .try_exists(JOB_QUEUE_DIRECTORY)
            .map_err(map_storage_io)?
        {
            if !create {
                return Ok(None);
            }
            create_directory(&self.root, JOB_QUEUE_DIRECTORY)?;
        }
        self.root
            .open_dir_nofollow(JOB_QUEUE_DIRECTORY)
            .map(Some)
            .map_err(map_committed_io)
    }

    /// Enters `job_id` of `session_id` at the end of the queue, unless it is
    /// in it already.
    ///
    /// # Errors
    ///
    /// [`SessionStorageError::CapacityExhausted`] for a full queue, and
    /// storage failures.
    pub fn enqueue_job(
        &self,
        session_id: &SessionId,
        job_id: &JobId,
        now_unix_seconds: u64,
    ) -> Result<(), SessionStorageError> {
        let queue = self.queue_directory(true)?.ok_or(SessionStorageError::Io)?;
        let _entering = self.take_entry_lock()?;
        let names = entry_names(&queue)?;
        if names
            .iter()
            .any(|name| entry_job(name).as_ref() == Some(job_id))
        {
            return Ok(());
        }
        if names.len() >= MAX_QUEUED_JOBS {
            return Err(SessionStorageError::CapacityExhausted);
        }
        let bytes = serde_json::to_vec(&StoredJobIndex::new(session_id))
            .map_err(|_| SessionStorageError::Io)?;
        let name = entry_name(next_stamp(&names, now_unix_seconds), job_id);
        replace_file(
            &queue,
            &name,
            &format!(".{}.tmp", job_id.as_str()),
            &bytes,
            super::StoredDurability::Ephemeral,
            None,
        )
    }

    /// Removes `job_id` from the queue, best effort.
    pub fn dequeue_job(&self, job_id: &JobId) {
        let Ok(Some(queue)) = self.queue_directory(false) else {
            return;
        };
        let Ok(names) = entry_names(&queue) else {
            return;
        };
        for name in names {
            if entry_job(&name).as_ref() == Some(job_id) {
                let _ = queue.remove_file(&name);
            }
        }
    }

    /// The queue, first in first: every entry that reads as one. An entry
    /// that does not (a stray or damaged file) is left out.
    ///
    /// # Errors
    ///
    /// Storage failures.
    pub fn job_queue(&self) -> Result<Vec<QueuedEntry>, SessionStorageError> {
        let Some(queue) = self.queue_directory(false)? else {
            return Ok(Vec::new());
        };
        let mut names = entry_names(&queue)?;
        names.sort();
        Ok(names
            .into_iter()
            .filter_map(|name| {
                let (stamp, job_id) = entry_parts(&name)?;
                let session_id = read_versioned_json_file::<StoredJobIndex>(&queue, &name)
                    .ok()?
                    .session()?;
                Some(QueuedEntry {
                    session_id,
                    job_id,
                    entered_at_unix_seconds: stamp / STAMPS_PER_SECOND,
                })
            })
            .collect())
    }

    /// Takes the root's queue turn, if no other runner holds it.
    ///
    /// # Errors
    ///
    /// Storage failures.
    pub fn try_take_queue_turn(&self) -> Result<Option<QueueTurn>, SessionStorageError> {
        let coordination = self.coordination()?;
        match HeldFileLock::try_exclusive(
            create_or_open(&coordination, QUEUE_TURN_LOCK)?.into_std(),
        ) {
            Ok(lock) => Ok(Some(QueueTurn { _lock: lock })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(map_storage_io(error)),
        }
    }

    /// Takes `weight` units for queued work, marked as such (ADR 0033 section
    /// 6). Another queued holder (a second queue on one root cannot run) makes
    /// it busy.
    ///
    /// # Errors
    ///
    /// [`SessionStorageError::Busy`] when the units are taken, and storage
    /// failures.
    pub fn try_admit_queued(
        &self,
        weight: u16,
    ) -> Result<FilesystemAdmissionPermit, SessionStorageError> {
        let permit = self.try_admit_without_waiting(weight)?;
        let coordination = self.coordination()?;
        let marker = match HeldFileLock::try_exclusive(
            create_or_open(&coordination, ADMISSION_QUEUED_LOCK)?.into_std(),
        ) {
            Ok(marker) => marker,
            Err(fs::TryLockError::WouldBlock) => return Err(SessionStorageError::Busy),
            Err(fs::TryLockError::Error(error)) => return Err(map_storage_io(error)),
        };
        Ok(permit.marked_queued(marker))
    }

    /// Whether an ordinary caller waits for capacity queued work holds.
    #[must_use]
    pub fn capacity_demanded(&self) -> bool {
        let Ok(coordination) = self.coordination() else {
            return false;
        };
        let Ok(file) = create_or_open(&coordination, ADMISSION_DEMAND_LOCK) else {
            return false;
        };
        match HeldFileLock::try_exclusive(file.into_std()) {
            Ok(probe) => {
                let _ = probe.release();
                false
            }
            Err(fs::TryLockError::WouldBlock) => true,
            Err(fs::TryLockError::Error(_)) => false,
        }
    }

    /// When queued work holds the root's capacity, demands it until the
    /// returned demand is dropped; `None` otherwise, or when the demand
    /// cannot be recorded.
    #[must_use]
    pub fn demand_queued_capacity(&self) -> Option<CapacityDemand> {
        let coordination = self.coordination().ok()?;
        if !queued_work_holds(&coordination) {
            return None;
        }
        demand_capacity(&coordination).map(|lock| CapacityDemand { _lock: lock })
    }

    /// Whether queued work holds the root's capacity. A read failure counts as
    /// it does not.
    #[must_use]
    pub fn queued_capacity_held(&self) -> bool {
        self.coordination()
            .is_ok_and(|coordination| queued_work_holds(&coordination))
    }

    /// Takes the lock under which one entry is entered, waiting briefly for
    /// another entry being entered.
    fn take_entry_lock(&self) -> Result<HeldFileLock, SessionStorageError> {
        let coordination = self.coordination()?;
        for _ in 0..QUEUE_ENTRY_LOCK_TRIES {
            match HeldFileLock::try_exclusive(
                create_or_open(&coordination, QUEUE_ENTRY_LOCK)?.into_std(),
            ) {
                Ok(lock) => return Ok(lock),
                Err(fs::TryLockError::WouldBlock) => std::thread::sleep(QUEUE_ENTRY_LOCK_POLL),
                Err(fs::TryLockError::Error(error)) => return Err(map_storage_io(error)),
            }
        }
        Err(SessionStorageError::Busy)
    }

    fn coordination(&self) -> Result<Dir, SessionStorageError> {
        self.revalidate_root()?;
        self.root
            .open_dir_nofollow(COORDINATION_DIRECTORY)
            .map_err(map_storage_io)
    }
}

/// Whether queued work holds capacity of the root whose coordination
/// directory is `coordination`. Probing takes a shared lock for an instant.
pub(super) fn queued_work_holds(coordination: &Dir) -> bool {
    let Ok(file) = create_or_open(coordination, ADMISSION_QUEUED_LOCK) else {
        return false;
    };
    match HeldFileLock::try_shared(file.into_std()) {
        Ok(probe) => {
            let _ = probe.release();
            false
        }
        Err(fs::TryLockError::WouldBlock) => true,
        Err(fs::TryLockError::Error(_)) => false,
    }
}

/// An ordinary caller's demand for the capacity queued work holds: queued
/// work gives it back at its next chunk boundary while any demand is held.
/// Released when dropped.
#[derive(Debug)]
pub struct CapacityDemand {
    _lock: HeldFileLock,
}

/// Holds the demand for capacity while an ordinary caller waits for queued
/// work to give it back.
pub(super) fn demand_capacity(coordination: &Dir) -> Option<HeldFileLock> {
    create_or_open(coordination, ADMISSION_DEMAND_LOCK)
        .ok()
        .and_then(|file| HeldFileLock::try_shared(file.into_std()).ok())
}

fn entry_names(queue: &Dir) -> Result<Vec<String>, SessionStorageError> {
    let mut names = Vec::new();
    for entry in queue.entries().map_err(map_storage_io)? {
        let entry = entry.map_err(map_storage_io)?;
        if let Ok(name) = entry.file_name().into_string()
            && !name.starts_with('.')
        {
            names.push(name);
        }
        if names.len() > MAX_QUEUED_JOBS {
            break;
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use vsift_domain::{JobId, SessionId};

    use super::super::{
        FilesystemSessionStore,
        tests::{Fixture, TestResult},
    };

    fn job(digit: char) -> Result<JobId, Box<dyn std::error::Error>> {
        Ok(JobId::parse(format!(
            "job_{}",
            digit.to_string().repeat(32)
        ))?)
    }

    /// ADR 0033 section 6: the queue is first in, first out across sessions,
    /// keeps a job's place when it is entered again, survives a new store (a
    /// restart), and loses a job once it is taken out; one turn at a time.
    #[test]
    fn the_queue_is_first_in_first_out_and_survives_a_restart() -> TestResult {
        let fixture = Fixture::new()?;
        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        let one = SessionId::parse("ses_1111111111111111")?;
        let two = SessionId::parse("ses_2222222222222222")?;
        assert_eq!(store.job_queue()?, []);
        store.enqueue_job(&one, &job('a')?, 100)?;
        store.enqueue_job(&two, &job('b')?, 200)?;
        store.enqueue_job(&one, &job('c')?, 300)?;
        // Entered again later, a job keeps its place.
        store.enqueue_job(&one, &job('a')?, 400)?;
        let order =
            |store: &FilesystemSessionStore| -> Result<Vec<JobId>, Box<dyn std::error::Error>> {
                Ok(store
                    .job_queue()?
                    .into_iter()
                    .map(|entry| entry.job_id)
                    .collect())
            };
        assert_eq!(order(&store)?, [job('a')?, job('b')?, job('c')?]);
        assert_eq!(
            store
                .job_queue()?
                .first()
                .map(|entry| entry.session_id.clone()),
            Some(one)
        );
        drop(store);
        let restarted = FilesystemSessionStore::open_existing(&fixture.path)?;
        assert_eq!(order(&restarted)?, [job('a')?, job('b')?, job('c')?]);
        restarted.dequeue_job(&job('a')?);
        assert_eq!(order(&restarted)?, [job('b')?, job('c')?]);

        let turn = restarted.try_take_queue_turn()?.ok_or("no turn")?;
        let other = FilesystemSessionStore::open_existing(&fixture.path)?;
        assert!(other.try_take_queue_turn()?.is_none(), "two turns at once");
        drop(turn);
        assert!(other.try_take_queue_turn()?.is_some());
        Ok(())
    }

    /// RV-1 finding 8: jobs entered in one second keep the order they arrived
    /// in, not the order of their ids, and the stamp gives each its second.
    #[test]
    fn entries_of_one_second_keep_their_order() -> TestResult {
        let fixture = Fixture::new()?;
        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        let session = SessionId::parse("ses_1111111111111111")?;
        for digit in ['c', 'a', 'b'] {
            store.enqueue_job(&session, &job(digit)?, 500)?;
        }
        store.enqueue_job(&session, &job('d')?, 501)?;
        let queue = store.job_queue()?;
        assert_eq!(
            queue
                .iter()
                .map(|entry| entry.job_id.clone())
                .collect::<Vec<_>>(),
            [job('c')?, job('a')?, job('b')?, job('d')?]
        );
        assert_eq!(
            queue
                .iter()
                .map(|entry| entry.entered_at_unix_seconds)
                .collect::<Vec<_>>(),
            [500, 500, 500, 501]
        );
        Ok(())
    }

    /// ADR 0033 section 6: an ordinary caller that finds the capacity held by
    /// queued work waits for it, demanding it, instead of being refused; it is
    /// admitted as soon as the queued work gives it back. Without queued work
    /// it is refused at once, as before.
    #[test]
    fn an_ordinary_caller_waits_for_queued_work_to_yield() -> TestResult {
        let fixture = Fixture::new()?;
        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        // Ordinary work holding everything: refused at once.
        let ordinary = store.try_admit(4)?;
        let started = Instant::now();
        assert!(store.try_admit(1).is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(ordinary);

        let queued = store.try_admit_queued(4)?;
        assert!(!store.capacity_demanded());
        let path = fixture.path.clone();
        let (sender, receiver) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let result = FilesystemSessionStore::open_existing(&path)
                .map_err(|_| "open")
                .and_then(|store| store.try_admit(1).map(|_| ()).map_err(|_| "refused"));
            let _ = sender.send(result);
        });
        // The waiter demands the capacity, and waits.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !store.capacity_demanded() {
            if Instant::now() > deadline {
                return Err("the waiter never demanded capacity".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            receiver.try_recv().is_err(),
            "admitted while queued work held"
        );
        // The queued work yields: the waiter is admitted.
        drop(queued);
        let admitted = receiver.recv_timeout(Duration::from_secs(10))?;
        assert_eq!(admitted, Ok(()));
        waiter.join().map_err(|_| "waiter panicked")?;
        assert!(!store.capacity_demanded());
        Ok(())
    }

    /// ADR 0033 section 6: a recognition's own admission never waits inside
    /// the store, so its run can wait where it says so; it demands the
    /// capacity only while queued work holds it, and the demand ends when it
    /// is dropped.
    #[test]
    fn a_recognition_demands_queued_capacity_without_waiting_in_the_store() -> TestResult {
        use std::num::NonZeroU16;

        use vsift_application::RevisionStore;

        let fixture = Fixture::new()?;
        let store = FilesystemSessionStore::open_existing(&fixture.path)?;
        let one = NonZeroU16::MIN;
        assert!(RevisionStore::demand_capacity(&store).is_none());
        assert!(!RevisionStore::queued_work_holds(&store));

        let queued = store.try_admit_queued(4)?;
        assert!(RevisionStore::queued_work_holds(&store));
        let started = Instant::now();
        assert!(RevisionStore::admit(&store, one).is_err());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the admission waited in the store"
        );
        let demand = RevisionStore::demand_capacity(&store).ok_or("no demand")?;
        assert!(RevisionStore::others_waiting(&store));
        drop(demand);
        assert!(!RevisionStore::others_waiting(&store));
        drop(queued);
        assert!(!RevisionStore::queued_work_holds(&store));
        assert!(RevisionStore::admit(&store, one).is_ok());
        Ok(())
    }
}
