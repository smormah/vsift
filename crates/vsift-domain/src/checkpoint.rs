//! Chunk checkpoints of a local speech-recognition job (P10, ADR 0020
//! section 4).
//!
//! A checkpoint records what one planned chunk produced so an interrupted
//! retranscription can continue without decoding and recognising the chunk
//! again. It holds the *raw* provider output, not the validated segments: a
//! resumed run passes the stored output through exactly the same validation
//! and merge rules as a fresh one, so the result is the same revision, and a
//! checkpoint can never smuggle in a segment the rules would have rejected.
//!
//! A checkpoint is a private, uncommitted stage file. It is never evidence,
//! never listed in a manifest and never exported in a bundle.

use std::fmt;

use crate::{PlannedChunk, ProviderChunkOutput, Sha256Hex, TimeRange};

/// Digest of everything that decides what a recognition run hears and how
/// it is recognised: session, source, audio stream, replaced range, chunk
/// plan, decoding profile, recognizer identity and the local-ASR
/// verification fingerprint.
///
/// It deliberately leaves out the revision the run will supersede, so
/// finished chunks stay usable when only the base revision changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecognitionKey(Sha256Hex);

impl RecognitionKey {
    /// Wraps a canonical digest derived by the application.
    #[must_use]
    pub const fn new(digest: Sha256Hex) -> Self {
        Self(digest)
    }

    /// The lowercase hexadecimal digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for RecognitionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What one chunk produced.
#[derive(Clone, Debug, PartialEq)]
pub enum CheckpointOutcome {
    /// No audio sample was decoded in the window.
    NoAudio,
    /// Audio was decoded and found silent, so the recognizer was not run.
    Silent {
        /// Observed decoded source range.
        audio: TimeRange,
    },
    /// Audio was decoded and recognised.
    Recognised {
        /// Observed decoded source range.
        audio: TimeRange,
        /// The recognizer's raw, unvalidated output.
        output: ProviderChunkOutput,
    },
}

impl CheckpointOutcome {
    /// Stable lowercase identifier of the outcome kind.
    #[must_use]
    pub const fn identifier(&self) -> &'static str {
        match self {
            Self::NoAudio => "no_audio",
            Self::Silent { .. } => "silent",
            Self::Recognised { .. } => "recognised",
        }
    }
}

/// One chunk's stored result, bound to the run it belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkCheckpoint {
    key: RecognitionKey,
    index: u32,
    window: TimeRange,
    outcome: CheckpointOutcome,
}

impl ChunkCheckpoint {
    /// Records `outcome` for `chunk` of the run identified by `key`.
    #[must_use]
    pub fn new(key: RecognitionKey, chunk: &PlannedChunk, outcome: CheckpointOutcome) -> Self {
        Self {
            key,
            index: chunk.index(),
            window: chunk.window(),
            outcome,
        }
    }

    /// Rebuilds a checkpoint read from storage; [`Self::belongs_to`] decides
    /// whether it may be used.
    #[must_use]
    pub const fn from_parts(
        key: RecognitionKey,
        index: u32,
        window: TimeRange,
        outcome: CheckpointOutcome,
    ) -> Self {
        Self {
            key,
            index,
            window,
            outcome,
        }
    }

    /// The run the checkpoint belongs to.
    #[must_use]
    pub const fn key(&self) -> &RecognitionKey {
        &self.key
    }

    /// Zero-based chunk index.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// The planned window the chunk decoded.
    #[must_use]
    pub const fn window(&self) -> TimeRange {
        self.window
    }

    /// What the chunk produced.
    #[must_use]
    pub const fn outcome(&self) -> &CheckpointOutcome {
        &self.outcome
    }

    /// Consumes the checkpoint, returning its outcome.
    #[must_use]
    pub fn into_outcome(self) -> CheckpointOutcome {
        self.outcome
    }

    /// Whether this checkpoint was written for exactly `chunk` of the run
    /// `key` identifies: the same key, index and planned window. Anything
    /// else is a checkpoint of another run, or a damaged or forged one.
    #[must_use]
    pub fn belongs_to(&self, key: &RecognitionKey, chunk: &PlannedChunk) -> bool {
        self.key == *key && self.index == chunk.index() && self.window == chunk.window()
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckpointOutcome, ChunkCheckpoint, RecognitionKey};
    use crate::{MediaTime, PlannedChunk, Sha256Hex, SourceSegmentId, TimeRange};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn key(digit: char) -> Result<RecognitionKey, Box<dyn std::error::Error>> {
        Ok(RecognitionKey::new(Sha256Hex::parse(
            digit.to_string().repeat(64),
        )?))
    }

    fn window(start: u64, end: u64) -> Result<TimeRange, Box<dyn std::error::Error>> {
        Ok(TimeRange::new(
            MediaTime::from_micros(start),
            MediaTime::from_micros(end),
        )?)
    }

    #[test]
    fn a_checkpoint_belongs_only_to_its_own_run_index_and_window() -> TestResult {
        let segment = SourceSegmentId::parse("sgm_0123456789abcdef")?;
        let chunk = PlannedChunk::new(segment.clone(), 1, window(25, 55)?);
        let checkpoint = ChunkCheckpoint::new(key('a')?, &chunk, CheckpointOutcome::NoAudio);
        assert!(checkpoint.belongs_to(&key('a')?, &chunk));
        assert!(!checkpoint.belongs_to(&key('b')?, &chunk));
        assert!(!checkpoint.belongs_to(
            &key('a')?,
            &PlannedChunk::new(segment.clone(), 2, window(25, 55)?)
        ));
        assert!(
            !checkpoint.belongs_to(&key('a')?, &PlannedChunk::new(segment, 1, window(25, 54)?))
        );
        assert_eq!(checkpoint.outcome().identifier(), "no_audio");
        let silent = CheckpointOutcome::Silent {
            audio: window(25, 55)?,
        };
        assert_eq!(silent.identifier(), "silent");
        Ok(())
    }
}
