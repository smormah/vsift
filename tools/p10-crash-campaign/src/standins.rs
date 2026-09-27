//! Deterministic stand-ins for the media and speech providers.
//!
//! The campaign qualifies the store's commit path, not `FFmpeg` or
//! whisper.cpp, so evidence frames come from a synthetic 20 fps, 6 s video
//! and retranscriptions from a recognizer that returns one segment per
//! chunk. They mirror the stand-ins of the storage and CLI contract tests.

use std::{future::Future, num::NonZeroU16, time::Duration};

use vsift_application::{
    AsrCancellation, CommitGuard, EvidenceControl, EvidenceMediaError, EvidenceStop,
    ExtractedFrame, FrameExtractor, RecognizerIdentity, RetryTimer, SessionStorageError,
    SpeechAudioError, SpeechAudioSource, SpeechPcm, SpeechRecognitionError, SpeechRecognizer,
    VideoStreamFacts,
};
use vsift_domain::{
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild, ChunkTime,
    CropRect, CueText, FrameDimensions, FrameListing, Jitter, LanguageTag, ListedFrame,
    ListingTail, MediaTime, PlannedChunk, ProviderChunkOutput, ProviderSegment, ProviderToken,
    ProviderTokenKind, Sha256Hex, TimeBase, TimeRange,
};

use crate::error::CampaignError;

/// Every stand-in's fixed provider digest.
pub const STAND_IN_DIGEST: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// Frames of the synthetic video: 6 s at 20 fps.
pub const VIDEO_FRAMES: i64 = 120;
const FRAME_MICROS: u64 = 50_000;
/// The synthetic source's speech: three R0 chunks (0-30, 25-55 and 50-60 s).
pub const SPEECH_MICROS: u64 = 60_000_000;

/// The start of an 8-bit RGB PNG of `width` x `height`, then `marker`.
fn fake_png(width: u32, height: u32, marker: &str) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13_u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    png.extend_from_slice(marker.as_bytes());
    png
}

fn time_of(pts: i64) -> MediaTime {
    MediaTime::from_micros(u64::try_from(pts).unwrap_or(0) * FRAME_MICROS)
}

/// A 20 fps, 6 s, 64x36 stream whose frame `n` has timestamp `n` in a 1/20
/// time base; each frame's image names the frame and the session's source,
/// so two sessions hold different files.
pub struct StandInVideo {
    facts: VideoStreamFacts,
    salt: String,
}

impl StandInVideo {
    /// The video of the source whose identity ends in `salt`.
    ///
    /// # Errors
    ///
    /// Never for the fixed facts; a rejected value is reported rather than
    /// assumed.
    pub fn new(salt: &str) -> Result<Self, CampaignError> {
        Ok(Self {
            facts: VideoStreamFacts {
                stream_index: 0,
                time_base: TimeBase::new(1, 20).map_err(|_| CampaignError::StandIn)?,
                displayed: FrameDimensions::new(64, 36).map_err(|_| CampaignError::StandIn)?,
                duration: MediaTime::from_micros(6_000_000),
            },
            salt: salt.to_owned(),
        })
    }

    fn listing(&self, requested: TimeRange) -> Result<FrameListing, EvidenceMediaError> {
        let duration = self.facts.duration;
        let (end, tail) = if requested.end() >= duration {
            (duration, ListingTail::EndOfStream)
        } else {
            (requested.end(), ListingTail::MoreMayFollow)
        };
        let covered =
            TimeRange::new(requested.start(), end).map_err(|_| EvidenceMediaError::Invalid)?;
        let frames = (0..VIDEO_FRAMES)
            .map(|pts| ListedFrame {
                pts,
                time: time_of(pts),
            })
            .filter(|frame| frame.time >= covered.start() && frame.time < covered.end())
            .collect();
        FrameListing::new(covered, frames, tail).map_err(|_| EvidenceMediaError::Invalid)
    }

    fn frame(&self, pts: i64) -> ExtractedFrame {
        ExtractedFrame {
            pts,
            time: time_of(pts),
            png: fake_png(64, 36, &format!("frame {pts} of {}", self.salt)),
        }
    }
}

impl FrameExtractor for StandInVideo {
    fn stream(&self) -> VideoStreamFacts {
        self.facts
    }

    fn max_frames_per_run(&self) -> usize {
        8
    }

    fn list_frames(
        &self,
        range: TimeRange,
    ) -> impl Future<Output = Result<FrameListing, EvidenceMediaError>> + Send {
        std::future::ready(self.listing(range))
    }

    fn frames(
        &self,
        pts: &[i64],
    ) -> impl Future<Output = Result<Vec<ExtractedFrame>, EvidenceMediaError>> + Send {
        std::future::ready(Ok(pts.iter().map(|value| self.frame(*value)).collect()))
    }

    fn crop(
        &self,
        pts: i64,
        rect: CropRect,
    ) -> impl Future<Output = Result<ExtractedFrame, EvidenceMediaError>> + Send {
        let mut frame = self.frame(pts);
        frame.png = fake_png(
            rect.width(),
            rect.height(),
            &format!("crop {pts} {} {} of {}", rect.x(), rect.y(), self.salt),
        );
        std::future::ready(Ok(frame))
    }
}

/// An evidence call that never stops early.
pub struct NeverStop;

impl EvidenceControl for NeverStop {
    fn stop(&self) -> Option<EvidenceStop> {
        None
    }
}

impl AsrCancellation for NeverStop {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// The stand-in recognizer's identity: a pinned whisper.cpp profile with the
/// fixed digest.
///
/// # Errors
///
/// Never for the fixed digest.
pub fn recognizer_identity() -> Result<RecognizerIdentity, SpeechRecognitionError> {
    let digest = Sha256Hex::parse(STAND_IN_DIGEST).map_err(|_| SpeechRecognitionError::Io)?;
    Ok(RecognizerIdentity {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, digest.clone()),
        model: AsrModel::new(AsrModelProfile::Base, digest),
        decoding: AsrDecodingProfile::R0V1,
        threads: NonZeroU16::MIN,
    })
}

/// One segment per chunk, inside every window.
fn words(chunk: &PlannedChunk) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    let text = format!("words of chunk {}", chunk.index());
    let text = CueText::new(text.clone(), text).map_err(|_| SpeechRecognitionError::Io)?;
    let third = chunk.window().duration_micros() / 3_000;
    let start = if chunk.index() == 0 { 10_000 } else { 6_000 }.min(third);
    let length = 2_000.min(third);
    Ok(ProviderChunkOutput {
        language: LanguageTag::parse("en").ok(),
        segments: vec![ProviderSegment {
            start: ChunkTime::from_millis(start).ok_or(SpeechRecognitionError::Io)?,
            end: ChunkTime::from_millis(start + length).ok_or(SpeechRecognitionError::Io)?,
            text: Some(text),
            tokens: vec![ProviderToken {
                kind: ProviderTokenKind::Text,
                probability: 0.5,
            }],
        }],
    })
}

/// A deterministic recognizer that takes `delay` per chunk, so a crash can
/// land while a job is running and checkpointing.
pub struct StandInRecognizer {
    /// Time one chunk takes.
    pub delay: Duration,
}

impl SpeechRecognizer for StandInRecognizer {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send {
        std::future::ready(recognizer_identity())
    }

    fn recognize(
        &self,
        chunk: &PlannedChunk,
        _pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        let delay = self.delay;
        let output = words(chunk);
        async move {
            tokio::time::sleep(delay).await;
            output
        }
    }
}

/// Speech in every chunk.
pub struct StandInAudio;

impl SpeechAudioSource for StandInAudio {
    fn speech_pcm(
        &self,
        chunk: &PlannedChunk,
    ) -> impl Future<Output = Result<SpeechPcm, SpeechAudioError>> + Send {
        let samples = usize::try_from(chunk.window().duration_micros() / 1_000 * 16).unwrap_or(0);
        std::future::ready(Ok(SpeechPcm {
            actual_start: chunk.window().start(),
            samples: vec![3_000; samples],
        }))
    }
}

/// Retries at once: the workload is the only writer, so contention is not
/// what the campaign measures.
pub struct NoBackoff;

impl RetryTimer for NoBackoff {
    fn jitter(&self) -> Jitter {
        Jitter::NONE
    }

    fn sleep(&self, _delay: Duration) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }
}

/// The stand-in source cannot change between recognition and commit.
pub struct SourceUnchanged;

impl CommitGuard for SourceUnchanged {
    fn verify(&mut self) -> Result<(), SessionStorageError> {
        Ok(())
    }
}
