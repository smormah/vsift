//! Named points in the session store's commit path and in its jobs where a
//! test or a fault campaign can make the process stop (P10, ADR 0020).
//!
//! Each commit point ([`FaultPoint::COMMIT`]) marks one boundary of a commit:
//! a content-addressed artifact installed, the artifact directory
//! synchronised, a generation manifest written, flushed, renamed and its
//! directory synchronised, the commit pointer likewise, and the chain
//! checkpoint written. The points are reached in both publication modes; in
//! ephemeral mode, where no directory is synchronised, a directory-sync point
//! marks the place the synchronisation would be.
//!
//! Each job point ([`FaultPoint::JOB`]) marks one boundary of a recoverable
//! job: its record and index entry created, a chunk recognised, its
//! checkpoint written, flushed and renamed, the record saying `committing`
//! and `succeeded`, and its checkpoints being deleted.
//!
//! Each request point ([`FaultPoint::REQUEST`], P11) marks one write of a
//! worker request's record: an attempt accepted, a step finished, the
//! request ended.
//!
//! Each registration point ([`FaultPoint::REGISTRATION`], issue #197) marks
//! one boundary of a session's registration in the index: its marker staged
//! but not yet written, and the marker renamed into its bucket.
//!
//! Stopping the process is compiled only into this crate's unit tests and
//! into builds with the `fault-injection` feature, which must never be
//! enabled in a release build (the crate refuses to compile it without debug
//! assertions, and the governance check refuses it outside development
//! dependencies). There, `VSIFT_FAULT_POINT=<name>[:<n>]` makes the process
//! exit at once with [`FAULT_EXIT_CODE`], without unwinding or running
//! destructors, the `n`-th time (default the first) one commit reaches the
//! named point. For the files already written that is the same as a kill: the
//! kernel holds whatever the process wrote, and nothing after the point runs.
//! The count is kept by the commit itself, not in process-wide state.

use std::fmt;

/// Exit status of a process stopped at a fault point.
#[cfg(any(test, feature = "fault-injection"))]
pub const FAULT_EXIT_CODE: i32 = 91;

/// Environment variable that selects a fault point.
#[cfg(any(test, feature = "fault-injection"))]
pub const FAULT_POINT_VARIABLE: &str = "VSIFT_FAULT_POINT";

/// Line written to standard error just before a process stops at a point.
#[cfg(any(test, feature = "fault-injection"))]
pub const FAULT_MARKER: &str = "VSIFT_FAULT_POINT_REACHED";

/// One boundary of the session store's commit path.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FaultPoint {
    /// One content-addressed artifact was created, written and flushed.
    ArtifactInstall,
    /// The artifact directory was synchronised after a batch of installs.
    ArtifactDirectorySync,
    /// The staged generation manifest was written.
    ManifestWrite,
    /// The staged generation manifest was flushed.
    ManifestFlush,
    /// The manifest was renamed into `generations/`.
    ManifestRename,
    /// `generations/` was synchronised.
    ManifestDirectorySync,
    /// The staged commit pointer was written.
    PointerWrite,
    /// The staged commit pointer was flushed.
    PointerFlush,
    /// The pointer was renamed to `current.json`.
    PointerRename,
    /// The session directory was synchronised: the commit is acknowledged.
    PointerDirectorySync,
    /// The staged chain checkpoint was written, before it replaces the old one.
    ChainCheckpointWrite,
    /// A new job's record was written, before its index entry.
    JobCreate,
    /// The job's root index entry was written.
    JobIndex,
    /// A chunk was recognised (or found silent or empty), before its
    /// checkpoint is written.
    ChunkRecognised,
    /// The staged chunk checkpoint was written.
    CheckpointWrite,
    /// The staged chunk checkpoint was flushed.
    CheckpointFlush,
    /// The chunk checkpoint was renamed into place.
    CheckpointRename,
    /// The job record says `committing`, before the generation is published.
    JobCommitting,
    /// The job record says `succeeded`, before its checkpoints are deleted.
    JobSucceeded,
    /// The first of the job's checkpoints was deleted.
    CheckpointDeletion,
    /// A worker request's record says an attempt accepted it (P11): its
    /// session id is recorded and no step of this attempt has run.
    RequestAccept,
    /// A worker request's record lists one more finished step.
    RequestStep,
    /// A worker request's record holds its result: the request has ended.
    RequestComplete,
    /// A session's staged index marker was created, before its bytes are
    /// written: the moment a marker created in place was empty (#197).
    RegistrationMarkerCreate,
    /// A session's index marker was renamed into its bucket, before the
    /// registration holds it.
    RegistrationMarkerRename,
}

impl FaultPoint {
    /// Every commit point, in commit order.
    #[cfg(any(test, feature = "fault-injection"))]
    pub const COMMIT: [Self; 11] = [
        Self::ArtifactInstall,
        Self::ArtifactDirectorySync,
        Self::ManifestWrite,
        Self::ManifestFlush,
        Self::ManifestRename,
        Self::ManifestDirectorySync,
        Self::PointerWrite,
        Self::PointerFlush,
        Self::PointerRename,
        Self::PointerDirectorySync,
        Self::ChainCheckpointWrite,
    ];

    /// Every job point, in the order a job reaches them.
    #[cfg(any(test, feature = "fault-injection"))]
    pub const JOB: [Self; 9] = [
        Self::JobCreate,
        Self::JobIndex,
        Self::ChunkRecognised,
        Self::CheckpointWrite,
        Self::CheckpointFlush,
        Self::CheckpointRename,
        Self::JobCommitting,
        Self::JobSucceeded,
        Self::CheckpointDeletion,
    ];

    /// Every worker request point, in the order a request reaches them.
    #[cfg(any(test, feature = "fault-injection"))]
    pub const REQUEST: [Self; 3] = [
        Self::RequestAccept,
        Self::RequestStep,
        Self::RequestComplete,
    ];

    /// Every session registration point, in the order a registration
    /// reaches them.
    #[cfg(any(test, feature = "fault-injection"))]
    pub const REGISTRATION: [Self; 2] = [
        Self::RegistrationMarkerCreate,
        Self::RegistrationMarkerRename,
    ];

    /// Every point: the commit points, the job points, the request points,
    /// then the registration points.
    #[cfg(any(test, feature = "fault-injection"))]
    pub const ALL: [Self; 25] = [
        Self::ArtifactInstall,
        Self::ArtifactDirectorySync,
        Self::ManifestWrite,
        Self::ManifestFlush,
        Self::ManifestRename,
        Self::ManifestDirectorySync,
        Self::PointerWrite,
        Self::PointerFlush,
        Self::PointerRename,
        Self::PointerDirectorySync,
        Self::ChainCheckpointWrite,
        Self::JobCreate,
        Self::JobIndex,
        Self::ChunkRecognised,
        Self::CheckpointWrite,
        Self::CheckpointFlush,
        Self::CheckpointRename,
        Self::JobCommitting,
        Self::JobSucceeded,
        Self::CheckpointDeletion,
        Self::RequestAccept,
        Self::RequestStep,
        Self::RequestComplete,
        Self::RegistrationMarkerCreate,
        Self::RegistrationMarkerRename,
    ];

    /// The point's stable name, as `VSIFT_FAULT_POINT` spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ArtifactInstall => "artifact-install",
            Self::ArtifactDirectorySync => "artifact-directory-sync",
            Self::ManifestWrite => "manifest-write",
            Self::ManifestFlush => "manifest-flush",
            Self::ManifestRename => "manifest-rename",
            Self::ManifestDirectorySync => "manifest-directory-sync",
            Self::PointerWrite => "pointer-write",
            Self::PointerFlush => "pointer-flush",
            Self::PointerRename => "pointer-rename",
            Self::PointerDirectorySync => "pointer-directory-sync",
            Self::ChainCheckpointWrite => "chain-checkpoint-write",
            Self::JobCreate => "job-create",
            Self::JobIndex => "job-index",
            Self::ChunkRecognised => "chunk-recognised",
            Self::CheckpointWrite => "checkpoint-write",
            Self::CheckpointFlush => "checkpoint-flush",
            Self::CheckpointRename => "checkpoint-rename",
            Self::JobCommitting => "job-committing",
            Self::JobSucceeded => "job-succeeded",
            Self::CheckpointDeletion => "checkpoint-deletion",
            Self::RequestAccept => "request-accept",
            Self::RequestStep => "request-step",
            Self::RequestComplete => "request-complete",
            Self::RegistrationMarkerCreate => "registration-marker-create",
            Self::RegistrationMarkerRename => "registration-marker-rename",
        }
    }

    /// Parses a point's stable name.
    #[cfg(any(test, feature = "fault-injection"))]
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|point| point.name() == name)
    }
}

impl fmt::Display for FaultPoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// The point one commit (or one job owner) stops at, read from the
/// environment when it starts, and how often it has reached it.
///
/// The count is atomic so a job owner, which the recognition run shares
/// between tasks, can keep one plan for the whole run.
#[derive(Debug, Default)]
pub(crate) struct FaultPlan {
    #[cfg(any(test, feature = "fault-injection"))]
    selected: Option<(FaultPoint, u32)>,
    #[cfg(any(test, feature = "fault-injection"))]
    reached: std::sync::atomic::AtomicU32,
}

impl FaultPlan {
    /// The plan `VSIFT_FAULT_POINT` selects; always empty in a build that
    /// cannot stop at fault points.
    pub(crate) fn from_environment() -> Self {
        #[cfg(any(test, feature = "fault-injection"))]
        {
            Self {
                selected: std::env::var(FAULT_POINT_VARIABLE)
                    .ok()
                    .as_deref()
                    .and_then(parse_selection),
                reached: std::sync::atomic::AtomicU32::new(0),
            }
        }
        #[cfg(not(any(test, feature = "fault-injection")))]
        {
            Self::default()
        }
    }

    /// Stops the process if this is the selected arrival at `point`.
    #[cfg_attr(
        not(any(test, feature = "fault-injection")),
        allow(clippy::unused_self, reason = "a build that cannot stop keeps no plan")
    )]
    pub(crate) fn reach(&self, point: FaultPoint) {
        #[cfg(any(test, feature = "fault-injection"))]
        if let Some((selected, arrival)) = self.selected
            && selected == point
        {
            let reached = self
                .reached
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                .saturating_add(1);
            if reached == arrival {
                use std::io::Write as _;
                let _ = writeln!(std::io::stderr(), "{FAULT_MARKER}={point}");
                std::process::exit(FAULT_EXIT_CODE);
            }
        }
        #[cfg(not(any(test, feature = "fault-injection")))]
        let _ = point;
    }
}

/// Parses `<name>[:<n>]`, `n` at least 1.
#[cfg(any(test, feature = "fault-injection"))]
fn parse_selection(text: &str) -> Option<(FaultPoint, u32)> {
    let (name, arrival) = match text.split_once(':') {
        Some((name, count)) => (name, count.parse::<u32>().ok().filter(|n| *n >= 1)?),
        None => (text, 1),
    };
    Some((FaultPoint::parse(name)?, arrival))
}

#[cfg(test)]
mod tests {
    use super::{FaultPoint, parse_selection};

    #[test]
    fn the_commit_job_request_and_registration_points_together_are_every_point() {
        let joined: Vec<FaultPoint> = FaultPoint::COMMIT
            .into_iter()
            .chain(FaultPoint::JOB)
            .chain(FaultPoint::REQUEST)
            .chain(FaultPoint::REGISTRATION)
            .collect();
        assert_eq!(joined, FaultPoint::ALL.to_vec());
    }

    #[test]
    fn every_point_has_a_distinct_name_that_parses_back() {
        for (index, point) in FaultPoint::ALL.into_iter().enumerate() {
            assert_eq!(FaultPoint::parse(point.name()), Some(point));
            assert!(
                FaultPoint::ALL
                    .iter()
                    .skip(index + 1)
                    .all(|other| other.name() != point.name())
            );
        }
    }

    #[test]
    fn selections_name_a_point_and_an_optional_positive_arrival() {
        assert_eq!(
            parse_selection("manifest-write"),
            Some((FaultPoint::ManifestWrite, 1))
        );
        assert_eq!(
            parse_selection("artifact-install:2"),
            Some((FaultPoint::ArtifactInstall, 2))
        );
        for rejected in [
            "",
            "manifest",
            "artifact-install:0",
            "artifact-install:",
            "x:1",
        ] {
            assert_eq!(parse_selection(rejected), None, "{rejected}");
        }
    }
}
