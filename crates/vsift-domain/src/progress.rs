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
    /// How far the stage has come in media time, for a stage that works
    /// through media in order (`recognising_speech`, ADR 0033).
    pub media: Option<MediaProgress>,
}

/// An estimate needs at least this many chunks recognised in this attempt
/// (ADR 0033, D8): one chunk says little about the speed.
pub const ESTIMATE_MIN_FRESH_CHUNKS: u32 = 2;

/// An estimate needs at least this much wall time spent recognising in this
/// attempt: 20 s, so a recogniser's start-up does not set it.
pub const ESTIMATE_MIN_FRESH_MICROS: u64 = 20_000_000;

/// How far a recognition has come in media time (ADR 0033 section 5, D8).
///
/// Every figure is in microseconds. `done_until_us` is the media time every
/// planned chunk before has finished up to, so it only grows. The speed and
/// the estimate count only the chunks recognised in this attempt ("fresh"):
/// chunks taken from checkpoints on a resume cost almost nothing, and an
/// admission wait is not part of any chunk's time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaProgress {
    /// Where the recognised range starts.
    pub from_us: u64,
    /// The media time the finished chunks reach.
    pub done_until_us: u64,
    /// Where the range ends, when that is known (a live source has no end).
    pub to_us: Option<u64>,
    /// Wall time since this attempt began recognising.
    pub elapsed_us: u64,
    /// Media the fresh chunks advanced the run by.
    pub fresh_media_us: u64,
    /// Wall time the fresh chunks took.
    pub fresh_elapsed_us: u64,
    /// How many chunks this attempt recognised.
    pub fresh_chunks: u32,
}

impl MediaProgress {
    /// Media done so far.
    #[must_use]
    pub const fn done_us(&self) -> u64 {
        self.done_until_us.saturating_sub(self.from_us)
    }

    /// The media in all, when the end is known.
    #[must_use]
    pub const fn total_us(&self) -> Option<u64> {
        match self.to_us {
            Some(to) => Some(to.saturating_sub(self.from_us)),
            None => None,
        }
    }

    /// The whole percent done, when the end is known: rounded down, so 100
    /// only when everything is done.
    #[must_use]
    pub fn percent(&self) -> Option<u8> {
        let total = self.total_us().filter(|total| *total > 0)?;
        let percent = u128::from(self.done_us().min(total)) * 100 / u128::from(total);
        u8::try_from(percent).ok()
    }

    /// Media seconds recognised a wall second, in thousandths, over the fresh
    /// chunks; `None` before any.
    #[must_use]
    pub fn speed_millis(&self) -> Option<u64> {
        if self.fresh_chunks == 0 || self.fresh_elapsed_us == 0 {
            return None;
        }
        let speed = u128::from(self.fresh_media_us) * 1000 / u128::from(self.fresh_elapsed_us);
        Some(u64::try_from(speed).unwrap_or(u64::MAX))
    }

    /// The wall time left, estimated as the media still to do over the
    /// cumulative measured speed (D8). `None` while the end is unknown, and
    /// until at least [`ESTIMATE_MIN_FRESH_CHUNKS`] chunks and
    /// [`ESTIMATE_MIN_FRESH_MICROS`] of fresh work have run. It is optimistic
    /// over silence, which is recognised faster than speech.
    #[must_use]
    pub fn estimate_remaining_us(&self) -> Option<u64> {
        let to = self.to_us?;
        if self.fresh_chunks < ESTIMATE_MIN_FRESH_CHUNKS
            || self.fresh_elapsed_us < ESTIMATE_MIN_FRESH_MICROS
            || self.fresh_media_us == 0
        {
            return None;
        }
        let left = to.saturating_sub(self.done_until_us.max(self.from_us));
        let estimate =
            u128::from(left) * u128::from(self.fresh_elapsed_us) / u128::from(self.fresh_media_us);
        Some(u64::try_from(estimate).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::{MediaProgress, ProgressStage, ProgressUnit};

    fn progress(done_s: u64, fresh_s: u64, fresh_wall_s: u64, chunks: u32) -> MediaProgress {
        MediaProgress {
            from_us: 10_000_000,
            done_until_us: 10_000_000 + done_s * 1_000_000,
            to_us: Some(10_000_000 + 600_000_000),
            elapsed_us: fresh_wall_s * 1_000_000,
            fresh_media_us: fresh_s * 1_000_000,
            fresh_elapsed_us: fresh_wall_s * 1_000_000,
            fresh_chunks: chunks,
        }
    }

    /// D8: the estimate is the media left over the measured speed, and only
    /// once two fresh chunks and 20 s have run.
    #[test]
    fn the_estimate_waits_for_two_fresh_chunks_and_twenty_seconds() {
        // 100 s of media in 50 s: twice real time, so 500 s left take 250 s.
        let steady = progress(100, 100, 50, 4);
        assert_eq!(steady.done_us(), 100_000_000);
        assert_eq!(steady.total_us(), Some(600_000_000));
        assert_eq!(steady.percent(), Some(16));
        assert_eq!(steady.speed_millis(), Some(2000));
        assert_eq!(steady.estimate_remaining_us(), Some(250_000_000));
        // One fresh chunk, or under 20 s, is not enough.
        assert_eq!(progress(100, 25, 50, 1).estimate_remaining_us(), None);
        assert_eq!(progress(100, 50, 19, 2).estimate_remaining_us(), None);
        assert_eq!(
            progress(100, 50, 20, 2).estimate_remaining_us(),
            Some(200_000_000)
        );
        // Reused chunks advance the position, not the speed.
        let resumed = progress(300, 50, 25, 2);
        assert_eq!(resumed.speed_millis(), Some(2000));
        assert_eq!(resumed.estimate_remaining_us(), Some(150_000_000));
        // No end: no percent and no estimate; the speed stays.
        let live = MediaProgress {
            to_us: None,
            ..steady
        };
        assert_eq!(live.percent(), None);
        assert_eq!(live.estimate_remaining_us(), None);
        assert_eq!(live.speed_millis(), Some(2000));
        // Nothing fresh yet: no speed.
        assert_eq!(progress(0, 0, 0, 0).speed_millis(), None);
        assert_eq!(progress(600, 600, 300, 24).percent(), Some(100));
    }

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
