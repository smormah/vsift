//! How one commit touches the filesystem: the session's publication mode, the
//! fault points it passes and, in unit tests, a trace of its operations.
//!
//! Ephemeral sessions keep the P03 protocol: every staged file is flushed
//! before it is renamed into place, so a process crash leaves either the old or
//! the new generation. Durable sessions (ADR 0020) add the ordering that makes
//! an acknowledged generation survive an OS crash on a qualified profile:
//!
//! 1. each content-addressed file is created new, written and flushed, then the
//!    artifact directory is synchronised once;
//! 2. the manifest is staged in `attempts/`, written, flushed, renamed into
//!    `generations/`, and `generations/` is synchronised;
//! 3. the pointer is staged, written, flushed, renamed to `current.json`, and
//!    the session directory is synchronised;
//! 4. only then is the generation acknowledged; the chain checkpoint follows.
//!
//! After a failed flush Linux may drop the dirty pages yet report a later flush
//! of the same file as successful ("fsyncgate"). A durable commit therefore
//! never re-flushes a file an earlier attempt left behind: a leftover staged
//! file is deleted and written again, and an existing content-addressed file is
//! accepted only when the committed head already lists it. Every flush, sync
//! or rename error is a storage failure and nothing is acknowledged.

use std::marker::PhantomData;

use cap_std::fs::Dir;
use vsift_application::SessionStorageError;

use super::{StoredDurability, map_storage_io};
use crate::fault_point::{FaultPlan, FaultPoint};

/// A directory a commit writes in, for the operation trace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "only the unit tests' trace reads the roles")
)]
pub(super) enum DirRole {
    /// `sessions/`.
    Sessions,
    /// `session-index/`.
    SessionIndex,
    /// The session's `session-index/<bucket>/`.
    IndexBucket,
    /// `sessions/<session>/`, or the initialization attempt that becomes it.
    Session,
    /// `sessions/<session>/artifacts/`.
    Artifacts,
    /// `sessions/<session>/generations/`.
    Generations,
    /// `sessions/<session>/attempts/`.
    Attempts,
}

/// One filesystem operation of a commit, as the unit tests' trace records it.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "only the unit tests' trace reads the operations")
)]
pub(super) enum FsOp {
    /// A file was created new (never over an existing one).
    Create(DirRole, String),
    /// The created file's bytes were written.
    Write(DirRole, String),
    /// The file was flushed (`sync_all`).
    SyncFile(DirRole, String),
    /// A leftover file was deleted.
    Remove(DirRole, String),
    /// An existing file was accepted as the one to install.
    Accept(DirRole, String),
    /// A file was renamed into place.
    Rename(DirRole, String, DirRole, String),
    /// A directory was synchronised.
    SyncDirectory(DirRole),
    /// A directory was renamed into place.
    RenameDirectory(DirRole, String, DirRole, String),
    /// The generation is committed and may be acknowledged.
    Committed,
}

/// What a caller sets for the commits of one operation: the fault points they
/// may stop or fail at and, in unit tests, a trace.
pub(super) struct CommitHooks<'a> {
    plan: FaultPlan,
    /// A point at which the commit fails with a storage error instead of
    /// continuing (in-process fault injection in the unit tests).
    failing_at: Option<FaultPoint>,
    #[cfg(test)]
    trace: Option<&'a std::cell::RefCell<Vec<FsOp>>>,
    /// The crash campaign's negative control: skip the session directory
    /// synchronisation after the pointer rename.
    #[cfg(feature = "durability-campaign")]
    negative_control: bool,
    lifetime: PhantomData<&'a ()>,
}

impl CommitHooks<'_> {
    /// The hooks of an ordinary operation: only the fault point
    /// `VSIFT_FAULT_POINT` selects, in builds that can stop at one.
    pub(super) fn new() -> Self {
        Self {
            plan: FaultPlan::from_environment(),
            failing_at: None,
            #[cfg(test)]
            trace: None,
            #[cfg(feature = "durability-campaign")]
            negative_control: std::env::var_os(NEGATIVE_CONTROL_VARIABLE)
                .is_some_and(|value| value == "1"),
            lifetime: PhantomData,
        }
    }

    /// The commit of a session with `durability` under these hooks.
    pub(super) const fn commit(&self, durability: StoredDurability) -> Commit<'_> {
        Commit {
            durability,
            hooks: self,
        }
    }
}

#[cfg(test)]
impl<'a> CommitHooks<'a> {
    /// Hooks that fail the commit with a storage error at `point`.
    pub(super) fn failing_at(point: FaultPoint) -> Self {
        Self {
            failing_at: Some(point),
            ..Self::new()
        }
    }

    /// These hooks, also failing the commit with a storage error at `point`.
    pub(super) const fn failing(mut self, point: FaultPoint) -> Self {
        self.failing_at = Some(point);
        self
    }

    /// Hooks that record every operation in `trace`. Where the platform
    /// cannot synchronise a directory through a capability handle, the
    /// recorder stands in for the call, so the order is checked everywhere.
    pub(super) fn traced(trace: &'a std::cell::RefCell<Vec<FsOp>>) -> Self {
        Self {
            trace: Some(trace),
            ..Self::new()
        }
    }
}

/// One session's commit under a set of hooks.
#[derive(Clone, Copy)]
pub(super) struct Commit<'h> {
    durability: StoredDurability,
    hooks: &'h CommitHooks<'h>,
}

impl Commit<'_> {
    /// Whether the session uses the durable protocol.
    pub(super) const fn durable(self) -> bool {
        matches!(self.durability, StoredDurability::Durable)
    }

    /// The session's publication mode.
    pub(super) const fn durability(self) -> StoredDurability {
        self.durability
    }

    /// Passes `point`: stops the process if the fault plan selects it, and
    /// fails if the unit tests inject a failure there.
    pub(super) fn reach(self, point: FaultPoint) -> Result<(), SessionStorageError> {
        self.hooks.plan.reach(point);
        if self.hooks.failing_at == Some(point) {
            Err(SessionStorageError::Io)
        } else {
            Ok(())
        }
    }

    /// Records an operation in the unit tests' trace.
    #[cfg_attr(
        not(test),
        allow(clippy::unused_self, reason = "only the unit tests keep a trace")
    )]
    pub(super) fn record(self, operation: impl FnOnce() -> FsOp) {
        #[cfg(test)]
        if let Some(trace) = self.hooks.trace {
            trace.borrow_mut().push(operation());
        }
        #[cfg(not(test))]
        let _ = operation;
    }

    /// Synchronises `directory` when the session is durable; ephemeral
    /// commits make no OS-crash promise and skip it.
    pub(super) fn sync_directory(
        self,
        directory: &Dir,
        role: DirRole,
    ) -> Result<(), SessionStorageError> {
        if !self.durable() {
            return Ok(());
        }
        self.record(|| FsOp::SyncDirectory(role));
        #[cfg(all(test, not(unix)))]
        if self.hooks.trace.is_some() {
            return Ok(());
        }
        sync_directory(directory).map_err(map_storage_io)
    }

    /// Synchronises the session directory after the pointer rename: the
    /// commit point of a durable session.
    ///
    /// Under the crash campaign's negative control the synchronisation is
    /// left out (see [`Self::negative_control`]).
    pub(super) fn sync_pointer_directory(self, session: &Dir) -> Result<(), SessionStorageError> {
        if self.negative_control() {
            return Ok(());
        }
        self.sync_directory(session, DirRole::Session)
    }

    /// Whether this durable commit runs the crash campaign's negative
    /// control: a `durability-campaign` build with
    /// `VSIFT_CAMPAIGN_NEGATIVE_CONTROL=1`, never any other build.
    ///
    /// The control leaves out every synchronisation after the pointer
    /// rename: the session directory's and the flush of the chain
    /// checkpoint. Both must go, because on ext4 any file flush commits the
    /// whole running journal transaction, so the checkpoint's flush alone
    /// would make the pointer rename durable before the acknowledgement and
    /// hide the missing directory synchronisation (the campaign's first
    /// negative-control run found exactly that). With both gone an
    /// acknowledged generation is lost by a power loss before the next
    /// commit, which the campaign must detect (ADR 0020).
    #[cfg_attr(
        not(feature = "durability-campaign"),
        allow(
            clippy::unused_self,
            reason = "only a campaign build can select the control"
        )
    )]
    pub(super) const fn negative_control(self) -> bool {
        #[cfg(feature = "durability-campaign")]
        {
            self.hooks.negative_control && self.durable()
        }
        #[cfg(not(feature = "durability-campaign"))]
        {
            false
        }
    }
}

/// Environment variable that selects the crash campaign's negative control
/// in a `durability-campaign` build: `1` removes every synchronisation after
/// the pointer rename (see `Commit::negative_control`).
#[cfg(feature = "durability-campaign")]
pub(crate) const NEGATIVE_CONTROL_VARIABLE: &str = "VSIFT_CAMPAIGN_NEGATIVE_CONTROL";

/// Synchronises a directory's entries.
///
/// A capability directory handle on Linux can be an `O_PATH` descriptor, which
/// cannot be flushed (`EBADF`, the P03/P05 FS-01 finding), so `.` is reopened
/// relative to it with read access and that handle is flushed.
#[cfg(unix)]
pub(super) fn sync_directory(directory: &Dir) -> std::io::Result<()> {
    directory.open(".")?.sync_all()
}

/// Durable publication is qualified only on Linux (ADR 0010); elsewhere a
/// directory cannot be synchronised through a capability handle, and a durable
/// commit fails rather than acknowledge.
#[cfg(not(unix))]
pub(super) fn sync_directory(_directory: &Dir) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}
