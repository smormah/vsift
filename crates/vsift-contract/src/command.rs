//! Public v1 operation identifiers carried in the envelope's `command` field.

/// One public v1 operation identifier.
///
/// The identifier is the host's command path joined with `.` (`setup.check`,
/// `setup.configure-model`, `ingest`); each segment is a lowercase kebab-case
/// word. It is public API: consumers dispatch on it, and the published schemas
/// constrain it with a pattern that every listed identifier must satisfy.
///
/// Reserved operations are listed too, because a host still answers them with a
/// schema-valid `COMMAND_NOT_IMPLEMENTED` failure that carries their identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandName {
    /// Arguments could not be parsed into any operation.
    Parse,
    /// `setup check`.
    SetupCheck,
    /// `setup plan`.
    SetupPlan,
    /// `setup install` (reserved).
    SetupInstall,
    /// `setup repair` (reserved).
    SetupRepair,
    /// `setup list` (reserved).
    SetupList,
    /// `setup remove` (reserved).
    SetupRemove,
    /// `setup rollback` (reserved).
    SetupRollback,
    /// `setup configure`.
    SetupConfigure,
    /// `setup configure-model`.
    SetupConfigureModel,
    /// `ingest`.
    Ingest,
    /// `session list`.
    SessionList,
    /// `session status`.
    SessionStatus,
    /// `session close`.
    SessionClose,
    /// `session renew`.
    SessionRenew,
    /// `session retain`.
    SessionRetain,
    /// `session clean`.
    SessionClean,
    /// `transcript get`.
    TranscriptGet,
    /// `transcript retranscribe`.
    TranscriptRetranscribe,
    /// `search`.
    Search,
    /// `candidates`.
    Candidates,
    /// `frame get`.
    FrameGet,
    /// `frame neighbours`.
    FrameNeighbours,
    /// `frame burst`.
    FrameBurst,
    /// `audio` (reserved).
    Audio,
    /// `crop` (reserved).
    Crop,
    /// `bundle validate`.
    BundleValidate,
    /// `job run` (reserved).
    JobRun,
    /// `job batch` (reserved).
    JobBatch,
    /// `job status` (reserved).
    JobStatus,
    /// `job resume` (reserved).
    JobResume,
    /// `job cancel` (reserved).
    JobCancel,
}

impl CommandName {
    /// Every public v1 operation identifier, in declaration order.
    ///
    /// Contract tests iterate this list to prove each identifier satisfies the
    /// published schemas. An exhaustive private `ordinal` match and a
    /// compile-time assertion keep it in step with the variants.
    pub const ALL: [Self; 32] = [
        Self::Parse,
        Self::SetupCheck,
        Self::SetupPlan,
        Self::SetupInstall,
        Self::SetupRepair,
        Self::SetupList,
        Self::SetupRemove,
        Self::SetupRollback,
        Self::SetupConfigure,
        Self::SetupConfigureModel,
        Self::Ingest,
        Self::SessionList,
        Self::SessionStatus,
        Self::SessionClose,
        Self::SessionRenew,
        Self::SessionRetain,
        Self::SessionClean,
        Self::TranscriptGet,
        Self::TranscriptRetranscribe,
        Self::Search,
        Self::Candidates,
        Self::FrameGet,
        Self::FrameNeighbours,
        Self::FrameBurst,
        Self::Audio,
        Self::Crop,
        Self::BundleValidate,
        Self::JobRun,
        Self::JobBatch,
        Self::JobStatus,
        Self::JobResume,
        Self::JobCancel,
    ];

    /// Returns the stable identifier written to the envelope's `command` field.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::SetupCheck => "setup.check",
            Self::SetupPlan => "setup.plan",
            Self::SetupInstall => "setup.install",
            Self::SetupRepair => "setup.repair",
            Self::SetupList => "setup.list",
            Self::SetupRemove => "setup.remove",
            Self::SetupRollback => "setup.rollback",
            Self::SetupConfigure => "setup.configure",
            Self::SetupConfigureModel => "setup.configure-model",
            Self::Ingest => "ingest",
            Self::SessionList => "session.list",
            Self::SessionStatus => "session.status",
            Self::SessionClose => "session.close",
            Self::SessionRenew => "session.renew",
            Self::SessionRetain => "session.retain",
            Self::SessionClean => "session.clean",
            Self::TranscriptGet => "transcript.get",
            Self::TranscriptRetranscribe => "transcript.retranscribe",
            Self::Search => "search",
            Self::Candidates => "candidates",
            Self::FrameGet => "frame.get",
            Self::FrameNeighbours => "frame.neighbours",
            Self::FrameBurst => "frame.burst",
            Self::Audio => "audio",
            Self::Crop => "crop",
            Self::BundleValidate => "bundle.validate",
            Self::JobRun => "job.run",
            Self::JobBatch => "job.batch",
            Self::JobStatus => "job.status",
            Self::JobResume => "job.resume",
            Self::JobCancel => "job.cancel",
        }
    }

    /// Returns the variant's position in [`CommandName::ALL`].
    ///
    /// This exhaustive match is the completeness guard for `ALL`: a new variant
    /// does not compile until it is given a position here, and the constant
    /// assertion after this `impl` block requires `ALL` to hold each position
    /// exactly once, in order. Give a new variant the next position and append
    /// it to `ALL`; a host's own tests should also compare `ALL` with the
    /// commands it parses.
    const fn ordinal(self) -> usize {
        match self {
            Self::Parse => 0,
            Self::SetupCheck => 1,
            Self::SetupPlan => 2,
            Self::SetupInstall => 3,
            Self::SetupRepair => 4,
            Self::SetupList => 5,
            Self::SetupRemove => 6,
            Self::SetupRollback => 7,
            Self::SetupConfigure => 8,
            Self::SetupConfigureModel => 9,
            Self::Ingest => 10,
            Self::SessionList => 11,
            Self::SessionStatus => 12,
            Self::SessionClose => 13,
            Self::SessionRenew => 14,
            Self::SessionRetain => 15,
            Self::SessionClean => 16,
            Self::TranscriptGet => 17,
            Self::TranscriptRetranscribe => 18,
            Self::Search => 19,
            Self::Candidates => 20,
            Self::FrameGet => 21,
            Self::FrameNeighbours => 22,
            Self::FrameBurst => 23,
            Self::Audio => 24,
            Self::Crop => 25,
            Self::BundleValidate => 26,
            Self::JobRun => 27,
            Self::JobBatch => 28,
            Self::JobStatus => 29,
            Self::JobResume => 30,
            Self::JobCancel => 31,
        }
    }
}

// Compile-time guard for `CommandName::ALL` (see `ordinal`): every listed name
// sits at its own position, so the list has no duplicate, gap or reordering.
const _: () = {
    let mut index = 0;
    while index < CommandName::ALL.len() {
        assert!(CommandName::ALL[index].ordinal() == index);
        index += 1;
    }
};

#[cfg(test)]
mod tests {
    use super::CommandName;

    #[test]
    fn identifiers_are_distinct() {
        for (index, name) in CommandName::ALL.into_iter().enumerate() {
            assert!(
                CommandName::ALL[index + 1..]
                    .iter()
                    .all(|other| other.identifier() != name.identifier()),
                "{} is listed twice",
                name.identifier()
            );
        }
    }
}
