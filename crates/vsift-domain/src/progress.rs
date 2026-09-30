//! How far a long operation has come (P11, ADR 0021).
//!
//! Progress is advisory: a host may coalesce or drop updates to keep its
//! output bounded, and the operation's terminal result stays the only
//! authority on what happened. It closes the "is it stalled?" half of
//! known limit L-025: a supervisor can see a long recognition advance chunk
//! by chunk instead of waiting on silence.

/// The stage a progress update measures; each counts in one [`ProgressUnit`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressStage {
    /// Copying the source into a session, in bytes.
    CopyingSource,
    /// Recognising speech, in chunks of the chunk plan.
    RecognisingSpeech,
    /// Analysing video for visual candidates, in windows.
    AnalysingVideo,
    /// Running a worker request, in steps.
    RunningRequest,
    /// Downloading or importing one reviewed managed artifact, in bytes
    /// (`setup install`, P13).
    FetchingArtifact,
    /// Installing the managed components of an accepted plan, in
    /// components finished (`setup install`, P13).
    InstallingComponents,
}

impl ProgressStage {
    /// Every stage, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::CopyingSource,
        Self::RecognisingSpeech,
        Self::AnalysingVideo,
        Self::RunningRequest,
        Self::FetchingArtifact,
        Self::InstallingComponents,
    ];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::CopyingSource => "copying_source",
            Self::RecognisingSpeech => "recognising_speech",
            Self::AnalysingVideo => "analysing_video",
            Self::RunningRequest => "running_request",
            Self::FetchingArtifact => "fetching_artifact",
            Self::InstallingComponents => "installing_components",
        }
    }

    /// The unit the stage counts in: one home, so a stage and its unit
    /// cannot disagree.
    #[must_use]
    pub const fn unit(self) -> ProgressUnit {
        match self {
            Self::CopyingSource | Self::FetchingArtifact => ProgressUnit::Bytes,
            Self::RecognisingSpeech => ProgressUnit::Chunks,
            Self::AnalysingVideo => ProgressUnit::Windows,
            Self::RunningRequest => ProgressUnit::Steps,
            Self::InstallingComponents => ProgressUnit::Components,
        }
    }

    const fn ordinal(self) -> usize {
        match self {
            Self::CopyingSource => 0,
            Self::RecognisingSpeech => 1,
            Self::AnalysingVideo => 2,
            Self::RunningRequest => 3,
            Self::FetchingArtifact => 4,
            Self::InstallingComponents => 5,
        }
    }
}

const _: () = {
    let mut index = 0;
    while index < ProgressStage::ALL.len() {
        assert!(ProgressStage::ALL[index].ordinal() == index);
        index += 1;
    }
};

/// What a progress count counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressUnit {
    /// Bytes.
    Bytes,
    /// Chunks of a chunk plan.
    Chunks,
    /// Visual-analysis windows.
    Windows,
    /// Steps of a request.
    Steps,
    /// Managed components of an installation plan.
    Components,
}

impl ProgressUnit {
    /// Every unit, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Bytes,
        Self::Chunks,
        Self::Windows,
        Self::Steps,
        Self::Components,
    ];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Bytes => "bytes",
            Self::Chunks => "chunks",
            Self::Windows => "windows",
            Self::Steps => "steps",
            Self::Components => "components",
        }
    }
}

/// One progress observation: `completed` of `total` units of a stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgressUpdate {
    /// The stage.
    pub stage: ProgressStage,
    /// Units completed so far (reused work included).
    pub completed: u64,
    /// Units in all, when known.
    pub total: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{ProgressStage, ProgressUnit};

    #[test]
    fn every_stage_counts_in_one_published_unit() {
        for stage in ProgressStage::ALL {
            assert!(ProgressUnit::ALL.contains(&stage.unit()));
            assert!(
                stage
                    .identifier()
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            );
        }
    }
}
