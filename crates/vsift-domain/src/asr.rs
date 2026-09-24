//! Local speech recognition: chunk planning, provider-output validation,
//! silence and seam merging.
//!
//! A local ASR run decodes bounded, overlapping PCM chunks of one source
//! segment and hands each to a speech recognizer. This module owns every rule
//! that decides what the recognizer's output may become as evidence:
//!
//! - how a range is cut into chunks, deterministically (`plan_chunks`);
//! - which provider segments are credible for their chunk (T-05/T-06): times
//!   are checked against the chunk's decoded audio, never clamped silently;
//! - that a chunk with no audible signal is recorded as a silent gap rather
//!   than transcribed;
//! - how the overlapping chunks are merged back into one ordered transcript
//!   without losing or doubling speech at a seam (T-03).
//!
//! Provider bytes and processes belong to infrastructure adapters; they hand
//! this module bounded, typed values and let it decide.

use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    num::{NonZeroU16, NonZeroU32},
};

use crate::{
    Confidence, ConfidenceOrigin, CueText, LanguageTag, MediaTime, ProviderEndTrim, SourceSegment,
    SourceSegmentId, TimeRange, TranscriptRevisionError, TranscriptWarningKind, TranscriptWarnings,
};

/// Sample rate of the speech PCM contract: mono signed 16-bit at 16 kHz.
pub const SPEECH_SAMPLE_RATE: u32 = 16_000;
/// Longest chunk window a plan may request; the PCM source bounds one chunk to it.
pub const MAX_CHUNK_WINDOW_MICROS: u64 = 30_000_000;
/// Most chunks one run may plan: four hours of R0 chunks, with headroom.
pub const MAX_PLANNED_CHUNKS: usize = 1_024;
/// How far past a chunk's decoded audio a provider end may lie and still be
/// trimmed to the audio end rather than rejected.
///
/// whisper.cpp rounds segment ends to its 10 ms token grid and may report the
/// end of its padded 30 s window; a second covers that without accepting text
/// placed after the audio that produced it.
pub const PROVIDER_END_TOLERANCE_MICROS: u64 = 1_000_000;
/// Most provider segments accepted for one chunk.
pub const MAX_PROVIDER_SEGMENTS: usize = 256;
/// Most provider tokens accepted for one segment.
pub const MAX_PROVIDER_TOKENS: usize = 512;
/// A segment this close to an inner window edge was probably cut by the edge.
const SEAM_EDGE_MICROS: u64 = 500_000;
/// Fewest shared words that identify seam text as a duplicate rather than a
/// genuine repeat.
const MIN_SEAM_DUPLICATE_WORDS: usize = 2;
/// 20 ms of 16 kHz audio.
const SILENCE_FRAME_SAMPLES: usize = 320;
/// Full-scale amplitude of signed 16-bit PCM, squared.
const FULL_SCALE_SQUARED: u128 = 32_768 * 32_768;
/// -50 dBFS as a power ratio denominator: 10^(50/10).
const SILENCE_POWER_RATIO: u128 = 100_000;
const MICROS_PER_SECOND: u64 = 1_000_000;
const SHA256_HEX_LENGTH: usize = 64;

/// A lowercase hexadecimal SHA-256 digest recorded as provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sha256Hex(String);

impl Sha256Hex {
    /// Accepts exactly 64 lowercase hexadecimal digits.
    ///
    /// # Errors
    ///
    /// Returns [`DigestError`] for any other text.
    pub fn parse(value: impl Into<String>) -> Result<Self, DigestError> {
        let value = value.into();
        if value.len() != SHA256_HEX_LENGTH
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DigestError);
        }
        Ok(Self(value))
    }

    /// The canonical digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A digest was not 64 lowercase hexadecimal digits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DigestError;

impl fmt::Display for DigestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("digest is not a canonical SHA-256")
    }
}

impl Error for DigestError {}

/// The local speech-recognition engine family that produced a run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsrProvider {
    /// The whisper.cpp command-line interface.
    WhisperCpp,
}

impl AsrProvider {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::WhisperCpp => "whisper_cpp",
        }
    }
}

/// The exact provider build that ran: its family and executable digest.
///
/// The digest, not a version string the executable prints, is the identity:
/// whisper.cpp prints no stable banner, and a digest cannot carry a path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsrProviderBuild {
    provider: AsrProvider,
    executable_sha256: Sha256Hex,
}

impl AsrProviderBuild {
    /// Records a provider build.
    #[must_use]
    pub const fn new(provider: AsrProvider, executable_sha256: Sha256Hex) -> Self {
        Self {
            provider,
            executable_sha256,
        }
    }

    /// Engine family.
    #[must_use]
    pub const fn provider(&self) -> AsrProvider {
        self.provider
    }

    /// SHA-256 of the executable bytes.
    #[must_use]
    pub const fn executable_sha256(&self) -> &Sha256Hex {
        &self.executable_sha256
    }
}

/// Which model profile a model file is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsrModelProfile {
    /// The pinned multilingual whisper `base` model (ADR 0005).
    Base,
    /// A model file whose bytes match no reviewed profile.
    Unreviewed,
}

impl AsrModelProfile {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Unreviewed => "unreviewed",
        }
    }
}

/// The exact model a run used: its profile and file digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsrModel {
    profile: AsrModelProfile,
    sha256: Sha256Hex,
}

impl AsrModel {
    /// Records a model identity.
    #[must_use]
    pub const fn new(profile: AsrModelProfile, sha256: Sha256Hex) -> Self {
        Self { profile, sha256 }
    }

    /// Model profile.
    #[must_use]
    pub const fn profile(&self) -> AsrModelProfile {
        self.profile
    }

    /// SHA-256 of the model file.
    #[must_use]
    pub const fn sha256(&self) -> &Sha256Hex {
        &self.sha256
    }
}

/// The fixed decoding settings a run used, versioned so a change is visible
/// in provenance and never silently mixed with older output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsrDecodingProfile {
    /// R0: beam search 5, best-of 5, automatic language detection, non-speech
    /// tokens suppressed, no translation, CPU only, one processor.
    R0V1,
}

impl AsrDecodingProfile {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::R0V1 => "r0-v1",
        }
    }
}

/// A time relative to a chunk's first decoded sample, in microseconds.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ChunkTime(u64);

impl ChunkTime {
    /// Creates a chunk-relative time from microseconds.
    #[must_use]
    pub const fn from_micros(value: u64) -> Self {
        Self(value)
    }

    /// Converts a provider's millisecond offset, if representable.
    #[must_use]
    pub const fn from_millis(value: u64) -> Option<Self> {
        match value.checked_mul(1_000) {
            Some(micros) => Some(Self(micros)),
            None => None,
        }
    }

    /// Returns the time in microseconds.
    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }
}

/// How a range is cut into overlapping chunks.
///
/// The overlap gives the recognizer context on both sides of every seam, so a
/// sentence crossing a window edge is heard whole by at least one chunk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkPlan {
    window_us: u64,
    overlap_us: u64,
}

impl ChunkPlan {
    /// R0: 30 s windows (whisper's native context) overlapping by 5 s.
    pub const R0: Self = Self {
        window_us: 30_000_000,
        overlap_us: 5_000_000,
    };

    /// Validates a plan.
    ///
    /// # Errors
    ///
    /// Rejects a zero or over-long window, and an overlap of half the window
    /// or more, which would leave a chunk with no core of its own.
    pub const fn new(window_us: u64, overlap_us: u64) -> Result<Self, ChunkPlanError> {
        if window_us == 0 || window_us > MAX_CHUNK_WINDOW_MICROS {
            return Err(ChunkPlanError::InvalidWindow);
        }
        if overlap_us.saturating_mul(2) >= window_us {
            return Err(ChunkPlanError::InvalidOverlap);
        }
        Ok(Self {
            window_us,
            overlap_us,
        })
    }

    /// Window length in microseconds.
    #[must_use]
    pub const fn window_us(self) -> u64 {
        self.window_us
    }

    /// Overlap between consecutive windows in microseconds.
    #[must_use]
    pub const fn overlap_us(self) -> u64 {
        self.overlap_us
    }

    const fn stride_us(self) -> u64 {
        self.window_us - self.overlap_us
    }
}

/// Why a chunk plan could not be made.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkPlanError {
    /// The window is zero or longer than [`MAX_CHUNK_WINDOW_MICROS`].
    InvalidWindow,
    /// The overlap is half the window or more.
    InvalidOverlap,
    /// The range needs more than [`MAX_PLANNED_CHUNKS`] chunks.
    TooManyChunks,
}

impl fmt::Display for ChunkPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidWindow => "chunk window is zero or too long",
            Self::InvalidOverlap => "chunk overlap is half the window or more",
            Self::TooManyChunks => "range needs too many chunks",
        })
    }
}

impl Error for ChunkPlanError {}

/// One planned chunk, identified by its source segment, index and window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedChunk {
    source_segment: SourceSegmentId,
    index: u32,
    window: TimeRange,
}

impl PlannedChunk {
    /// Records a planned chunk; [`AsrRun::new`] checks it against its plan.
    #[must_use]
    pub const fn new(source_segment: SourceSegmentId, index: u32, window: TimeRange) -> Self {
        Self {
            source_segment,
            index,
            window,
        }
    }

    /// Source segment the chunk belongs to.
    #[must_use]
    pub const fn source_segment(&self) -> &SourceSegmentId {
        &self.source_segment
    }

    /// Zero-based position in the run.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// 1-based position, used as the first-affected ordinal of warnings.
    #[must_use]
    pub const fn ordinal(&self) -> NonZeroU32 {
        match NonZeroU32::new(self.index.saturating_add(1)) {
            Some(ordinal) => ordinal,
            None => NonZeroU32::MAX,
        }
    }

    /// Requested source-time window.
    #[must_use]
    pub const fn window(&self) -> TimeRange {
        self.window
    }
}

/// Cuts `range` of one source segment into overlapping chunks.
///
/// Consecutive windows start one stride (window minus overlap) apart; the last
/// window ends exactly at the range end and may be shorter. The same inputs
/// always give the same chunks, so a resumed or repeated run addresses the
/// same audio.
///
/// # Errors
///
/// Returns [`ChunkPlanError::TooManyChunks`] beyond [`MAX_PLANNED_CHUNKS`].
pub fn plan_chunks(
    source_segment: &SourceSegmentId,
    range: TimeRange,
    plan: ChunkPlan,
) -> Result<Vec<PlannedChunk>, ChunkPlanError> {
    let mut chunks = Vec::new();
    let mut start = range.start().as_micros();
    let end = range.end().as_micros();
    loop {
        if chunks.len() == MAX_PLANNED_CHUNKS {
            return Err(ChunkPlanError::TooManyChunks);
        }
        let window_end = start.saturating_add(plan.window_us).min(end);
        let window = TimeRange::new(
            MediaTime::from_micros(start),
            MediaTime::from_micros(window_end),
        )
        .map_err(|_| ChunkPlanError::InvalidWindow)?;
        let index = u32::try_from(chunks.len()).map_err(|_| ChunkPlanError::TooManyChunks)?;
        chunks.push(PlannedChunk::new(source_segment.clone(), index, window));
        if window_end == end {
            return Ok(chunks);
        }
        start = start.saturating_add(plan.stride_us());
    }
}

/// Returns the source range of `samples` decoded mono 16 kHz samples starting
/// at `start`, or `None` when there are none.
#[must_use]
pub fn decoded_audio_range(start: MediaTime, samples: usize) -> Option<TimeRange> {
    let samples = u64::try_from(samples).ok()?;
    let duration = samples.checked_mul(MICROS_PER_SECOND)? / u64::from(SPEECH_SAMPLE_RATE);
    let end = start.as_micros().checked_add(duration)?;
    TimeRange::new(start, MediaTime::from_micros(end)).ok()
}

/// What happened to one planned chunk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsrChunkOutcome {
    /// Audio was decoded and transcribed; `audio` is its decoded source range.
    Transcribed {
        /// Observed first decoded sample to last decoded sample.
        audio: TimeRange,
    },
    /// Audio was decoded but every 20 ms frame was below -50 dBFS, so the
    /// recognizer was not run; the window is a silent gap.
    Silent {
        /// Observed first decoded sample to last decoded sample.
        audio: TimeRange,
    },
    /// No audio sample was decoded in the window; it is a gap with no audio.
    NoAudio,
}

/// One chunk of a run and its outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsrChunkRecord {
    chunk: PlannedChunk,
    outcome: AsrChunkOutcome,
}

impl AsrChunkRecord {
    /// Records a chunk outcome.
    #[must_use]
    pub const fn new(chunk: PlannedChunk, outcome: AsrChunkOutcome) -> Self {
        Self { chunk, outcome }
    }

    /// The planned chunk.
    #[must_use]
    pub const fn chunk(&self) -> &PlannedChunk {
        &self.chunk
    }

    /// What happened to it.
    #[must_use]
    pub const fn outcome(&self) -> AsrChunkOutcome {
        self.outcome
    }
}

/// Every field of an [`AsrRun`], validated together.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsrRunParts {
    /// Provider build that ran.
    pub provider: AsrProviderBuild,
    /// Model that ran.
    pub model: AsrModel,
    /// Decoding settings.
    pub decoding: AsrDecodingProfile,
    /// How the range was chunked.
    pub plan: ChunkPlan,
    /// Recognizer threads.
    pub threads: NonZeroU16,
    /// Original index of the audio stream that was decoded.
    pub audio_stream: u32,
    /// Every planned chunk in order, with its outcome.
    pub chunks: Vec<AsrChunkRecord>,
}

/// Provenance of one local ASR run: what ran, how, and on which audio.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsrRun {
    provider: AsrProviderBuild,
    model: AsrModel,
    decoding: AsrDecodingProfile,
    plan: ChunkPlan,
    threads: NonZeroU16,
    audio_stream: u32,
    chunks: Vec<AsrChunkRecord>,
}

impl AsrRun {
    /// Validates that the chunk records are exactly the plan of their range.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptRevisionError::InvalidAsrRun`] when chunks are
    /// missing, repeated, reordered, span several source segments, or are not
    /// what [`plan_chunks`] gives for the range they cover.
    pub fn new(parts: AsrRunParts) -> Result<Self, TranscriptRevisionError> {
        let invalid = TranscriptRevisionError::InvalidAsrRun;
        let (Some(first), Some(last)) = (parts.chunks.first(), parts.chunks.last()) else {
            return Err(invalid);
        };
        let covered = TimeRange::new(first.chunk.window.start(), last.chunk.window.end())
            .map_err(|_| invalid)?;
        let expected =
            plan_chunks(&first.chunk.source_segment, covered, parts.plan).map_err(|_| invalid)?;
        if expected.len() != parts.chunks.len()
            || expected
                .iter()
                .zip(&parts.chunks)
                .any(|(planned, record)| *planned != record.chunk)
        {
            return Err(invalid);
        }
        Ok(Self {
            provider: parts.provider,
            model: parts.model,
            decoding: parts.decoding,
            plan: parts.plan,
            threads: parts.threads,
            audio_stream: parts.audio_stream,
            chunks: parts.chunks,
        })
    }

    /// Provider build that ran.
    #[must_use]
    pub const fn provider(&self) -> &AsrProviderBuild {
        &self.provider
    }

    /// Model that ran.
    #[must_use]
    pub const fn model(&self) -> &AsrModel {
        &self.model
    }

    /// Decoding settings.
    #[must_use]
    pub const fn decoding(&self) -> AsrDecodingProfile {
        self.decoding
    }

    /// How the range was chunked.
    #[must_use]
    pub const fn plan(&self) -> ChunkPlan {
        self.plan
    }

    /// Recognizer threads.
    #[must_use]
    pub const fn threads(&self) -> NonZeroU16 {
        self.threads
    }

    /// Original index of the decoded audio stream.
    #[must_use]
    pub const fn audio_stream(&self) -> u32 {
        self.audio_stream
    }

    /// Every chunk in order, with its outcome.
    #[must_use]
    pub fn chunks(&self) -> &[AsrChunkRecord] {
        &self.chunks
    }

    /// The source range the run's chunks cover.
    #[must_use]
    pub fn covered_range(&self) -> Option<TimeRange> {
        let first = self.chunks.first()?;
        let last = self.chunks.last()?;
        TimeRange::new(first.chunk.window.start(), last.chunk.window.end()).ok()
    }

    /// Checks that the run belongs to, and lies inside, `source`.
    pub(crate) fn validate_within(
        &self,
        source: &SourceSegment,
    ) -> Result<(), TranscriptRevisionError> {
        let covered = self
            .covered_range()
            .ok_or(TranscriptRevisionError::InvalidAsrRun)?;
        if self
            .chunks
            .iter()
            .any(|record| record.chunk.source_segment != *source.id())
            || covered.start() < source.range().start()
            || covered.end() > source.range().end()
        {
            return Err(TranscriptRevisionError::InvalidAsrRun);
        }
        Ok(())
    }
}

/// Whether a provider token is text or a control token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderTokenKind {
    /// A text token; its probability counts toward segment confidence.
    Text,
    /// A timestamp or control token such as `[_BEG_]`, `[_TT_263]` or `<|en|>`.
    Special,
}

impl ProviderTokenKind {
    /// Classifies a whisper token by its text: special tokens start with `[_` or `<|`.
    #[must_use]
    pub fn classify(text: &str) -> Self {
        if text.starts_with("[_") || text.starts_with("<|") {
            Self::Special
        } else {
            Self::Text
        }
    }
}

/// One provider token: only its kind and probability are kept.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProviderToken {
    /// Text or control token.
    pub kind: ProviderTokenKind,
    /// Provider probability as reported; validated to be finite and in `[0, 1]`.
    pub probability: f64,
}

/// One provider segment as reported, before validation.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderSegment {
    /// Reported start, relative to the chunk's first decoded sample.
    pub start: ChunkTime,
    /// Reported end, relative to the chunk's first decoded sample.
    pub end: ChunkTime,
    /// Validated text, or `None` when the provider reported only whitespace.
    pub text: Option<CueText>,
    /// Tokens in provider order.
    pub tokens: Vec<ProviderToken>,
}

/// A provider's bounded, parsed output for one chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderChunkOutput {
    /// Language the provider detected, if it reported a well-formed one.
    pub language: Option<LanguageTag>,
    /// Segments in provider order.
    pub segments: Vec<ProviderSegment>,
}

/// Why a chunk's provider output could not be used at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderOutputError {
    /// More than [`MAX_PROVIDER_SEGMENTS`] segments.
    TooManySegments,
    /// A segment has more than [`MAX_PROVIDER_TOKENS`] tokens.
    TooManyTokens,
    /// Segments are not in non-decreasing start order.
    OutOfOrderSegments,
    /// A token probability is not a finite number in `[0, 1]`.
    InvalidTokenProbability,
    /// More than a quarter of the chunk's text segments, or all of them, had
    /// ranges that could not be placed in its decoded audio.
    TooManyRejectedSegments,
}

impl ProviderOutputError {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::TooManySegments => "too_many_segments",
            Self::TooManyTokens => "too_many_tokens",
            Self::OutOfOrderSegments => "out_of_order_segments",
            Self::InvalidTokenProbability => "invalid_token_probability",
            Self::TooManyRejectedSegments => "too_many_rejected_segments",
        }
    }
}

impl fmt::Display for ProviderOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManySegments => "provider output has too many segments",
            Self::TooManyTokens => "provider segment has too many tokens",
            Self::OutOfOrderSegments => "provider segments are out of order",
            Self::InvalidTokenProbability => "provider token probability is invalid",
            Self::TooManyRejectedSegments => "too many provider segments were rejected",
        })
    }
}

impl Error for ProviderOutputError {}

/// One accepted provider segment, placed on the source timeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsrSegmentDraft {
    range: TimeRange,
    text: CueText,
    confidence: Confidence,
    provider_start: ChunkTime,
    provider_end: ChunkTime,
    trimmed: ProviderEndTrim,
}

impl AsrSegmentDraft {
    /// Source-timeline range.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// Text.
    #[must_use]
    pub const fn text(&self) -> &CueText {
        &self.text
    }

    /// Mean text-token probability, `provider_uncalibrated`, or unknown.
    #[must_use]
    pub const fn confidence(&self) -> Confidence {
        self.confidence
    }

    /// Provider start as reported.
    #[must_use]
    pub const fn provider_start(&self) -> ChunkTime {
        self.provider_start
    }

    /// Provider end as reported.
    #[must_use]
    pub const fn provider_end(&self) -> ChunkTime {
        self.provider_end
    }

    /// Whether the end was cut at the decoded audio end.
    #[must_use]
    pub const fn trimmed(&self) -> ProviderEndTrim {
        self.trimmed
    }

    fn midpoint(&self) -> u64 {
        u64::midpoint(self.range.start().as_micros(), self.range.end().as_micros())
    }
}

/// A chunk whose provider output passed validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedChunk {
    chunk: PlannedChunk,
    audio: TimeRange,
    language: Option<LanguageTag>,
    segments: Vec<AsrSegmentDraft>,
    warnings: TranscriptWarnings,
}

impl ValidatedChunk {
    /// The planned chunk.
    #[must_use]
    pub const fn chunk(&self) -> &PlannedChunk {
        &self.chunk
    }

    /// Decoded audio range.
    #[must_use]
    pub const fn audio(&self) -> TimeRange {
        self.audio
    }

    /// Language the provider detected.
    #[must_use]
    pub const fn language(&self) -> Option<&LanguageTag> {
        self.language.as_ref()
    }

    /// Accepted segments in provider order.
    #[must_use]
    pub fn segments(&self) -> &[AsrSegmentDraft] {
        &self.segments
    }

    /// Counted rejections, trims and removed markers.
    #[must_use]
    pub const fn warnings(&self) -> &TranscriptWarnings {
        &self.warnings
    }

    /// Splits the chunk into its merge input and its warnings.
    #[must_use]
    pub fn into_parts(self) -> (ChunkSegments, Option<LanguageTag>, TranscriptWarnings) {
        (
            ChunkSegments {
                chunk: self.chunk,
                segments: self.segments,
            },
            self.language,
            self.warnings,
        )
    }
}

/// Places one provider segment's chunk-relative times on the source timeline.
///
/// This is the single conversion for local ASR, used both when output is
/// validated and when a stored revision is rebuilt. It returns `None` for any
/// range the trim policy does not allow.
pub(crate) fn asr_source_range(
    audio: TimeRange,
    provider_start: ChunkTime,
    provider_end: ChunkTime,
    trimmed: ProviderEndTrim,
) -> Option<TimeRange> {
    let decoded = audio.duration_micros();
    let (start, end) = (provider_start.as_micros(), provider_end.as_micros());
    if end <= start || start >= decoded {
        return None;
    }
    let source_start = audio.start().as_micros().checked_add(start)?;
    let source_end = match trimmed {
        ProviderEndTrim::Unchanged if end <= decoded => {
            audio.start().as_micros().checked_add(end)?
        }
        ProviderEndTrim::TrimmedToAudioEnd
            if end > decoded && end - decoded <= PROVIDER_END_TOLERANCE_MICROS =>
        {
            audio.end().as_micros()
        }
        ProviderEndTrim::Unchanged | ProviderEndTrim::TrimmedToAudioEnd => return None,
    };
    TimeRange::new(
        MediaTime::from_micros(source_start),
        MediaTime::from_micros(source_end),
    )
    .ok()
}

/// Validates one chunk's provider output against its decoded audio (T-05/T-06).
///
/// Structural faults fail the whole chunk: out-of-order segments, a token
/// probability that is not a finite number in `[0, 1]`, or oversized output.
/// A segment whose range is empty or reversed, starts at or after the decoded
/// audio end, ends more than a second past it, or would leave `source`, is
/// rejected and counted; if more than a quarter of the text segments (or all
/// of them) are rejected, the chunk fails, because the provider evidently did
/// not describe this audio. An end within a second past the audio is cut to
/// the audio end and counted, keeping the raw provider end. Whole-segment
/// non-speech markers and empty segments are removed and counted. Confidence
/// is the mean probability of text tokens, `provider_uncalibrated`.
///
/// # Errors
///
/// Returns the [`ProviderOutputError`] that makes the chunk unusable.
pub fn validate_chunk_output(
    chunk: &PlannedChunk,
    audio: TimeRange,
    source: TimeRange,
    output: ProviderChunkOutput,
) -> Result<ValidatedChunk, ProviderOutputError> {
    if output.segments.len() > MAX_PROVIDER_SEGMENTS {
        return Err(ProviderOutputError::TooManySegments);
    }
    let mut previous_start = None;
    for segment in &output.segments {
        if segment.tokens.len() > MAX_PROVIDER_TOKENS {
            return Err(ProviderOutputError::TooManyTokens);
        }
        if previous_start.is_some_and(|previous| segment.start < previous) {
            return Err(ProviderOutputError::OutOfOrderSegments);
        }
        previous_start = Some(segment.start);
        if segment.tokens.iter().any(|token| {
            !token.probability.is_finite() || !(0.0..=1.0).contains(&token.probability)
        }) {
            return Err(ProviderOutputError::InvalidTokenProbability);
        }
    }
    let decoded = audio.duration_micros();
    let (mut considered, mut rejected, mut trimmed, mut markers) = (0_u32, 0_u32, 0_u32, 0_u32);
    let mut segments = Vec::with_capacity(output.segments.len());
    for segment in output.segments {
        let Some(text) = segment
            .text
            .filter(|text| !is_non_speech_marker(text.text()))
        else {
            markers = markers.saturating_add(1);
            continue;
        };
        considered = considered.saturating_add(1);
        let trim = if segment.end.as_micros() > decoded {
            ProviderEndTrim::TrimmedToAudioEnd
        } else {
            ProviderEndTrim::Unchanged
        };
        let Some(range) = asr_source_range(audio, segment.start, segment.end, trim)
            .filter(|range| range.start() >= source.start() && range.end() <= source.end())
        else {
            rejected = rejected.saturating_add(1);
            continue;
        };
        if trim == ProviderEndTrim::TrimmedToAudioEnd {
            trimmed = trimmed.saturating_add(1);
        }
        segments.push(AsrSegmentDraft {
            range,
            text,
            confidence: mean_text_confidence(&segment.tokens),
            provider_start: segment.start,
            provider_end: segment.end,
            trimmed: trim,
        });
    }
    if considered > 0 && (rejected == considered || rejected.saturating_mul(4) > considered) {
        return Err(ProviderOutputError::TooManyRejectedSegments);
    }
    let mut warnings = TranscriptWarnings::default();
    let ordinal = chunk.ordinal();
    warnings.add(
        TranscriptWarningKind::ProviderSegmentsRejected,
        rejected,
        ordinal,
    );
    warnings.add(TranscriptWarningKind::ProviderEndTrimmed, trimmed, ordinal);
    warnings.add(
        TranscriptWarningKind::NonSpeechMarkersRemoved,
        markers,
        ordinal,
    );
    Ok(ValidatedChunk {
        chunk: chunk.clone(),
        audio,
        language: output.language,
        segments,
        warnings,
    })
}

/// Whether text is only bracketed non-speech markers, such as `[BLANK_AUDIO]`,
/// `(music)` or `[Music] [Applause]`.
fn is_non_speech_marker(text: &str) -> bool {
    let mut rest = text.trim();
    if rest.is_empty() {
        return true;
    }
    while !rest.is_empty() {
        let close = match rest.chars().next() {
            Some('[') => ']',
            Some('(') => ')',
            _ => return false,
        };
        let Some(end) = rest.find(close) else {
            return false;
        };
        rest = rest
            .get(end + close.len_utf8()..)
            .unwrap_or_default()
            .trim_start();
    }
    true
}

/// Mean probability of text tokens as `provider_uncalibrated` basis points,
/// or unknown when the segment has no text token.
fn mean_text_confidence(tokens: &[ProviderToken]) -> Confidence {
    let mut sum = 0.0_f64;
    let mut count = 0_u32;
    for token in tokens {
        if token.kind == ProviderTokenKind::Text {
            sum += token.probability;
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        return Confidence::unknown();
    }
    let scaled = (sum / f64::from(count) * 10_000.0).round();
    // Probabilities are validated to [0, 1], so the scaled mean is in
    // [0, 10000]; a binary search converts it without a lossy cast.
    let (mut low, mut high) = (0_u16, 10_000_u16);
    while low < high {
        let middle = low + (high - low) / 2;
        if f64::from(middle) < scaled {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    Confidence::provider_score(low, ConfidenceOrigin::ProviderUncalibrated)
        .unwrap_or(Confidence::unknown())
}

/// Whether decoded speech PCM has no audible signal.
///
/// Silent means every 20 ms frame (the last may be shorter) has a mean power
/// below -50 dBFS, relative to a full-scale square wave. The test is exact
/// integer arithmetic, so it is identical on every platform. No samples is silent.
#[must_use]
pub fn is_silent_pcm(samples: &[i16]) -> bool {
    samples.chunks(SILENCE_FRAME_SAMPLES).all(|frame| {
        let energy: u128 = frame
            .iter()
            .map(|sample| {
                let magnitude = u128::from(sample.unsigned_abs());
                magnitude * magnitude
            })
            .sum();
        let length = u128::try_from(frame.len()).unwrap_or(u128::MAX);
        energy.saturating_mul(SILENCE_POWER_RATIO) < FULL_SCALE_SQUARED.saturating_mul(length)
    })
}

/// One chunk's accepted segments, the input of [`merge_chunks`].
///
/// A silent chunk or one with no audio takes part with no segments: it still
/// owns its core, so nothing from a neighbour is attributed into its time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkSegments {
    /// The planned chunk.
    pub chunk: PlannedChunk,
    /// Accepted segments in provider order.
    pub segments: Vec<AsrSegmentDraft>,
}

/// One segment kept by [`merge_chunks`], with the chunk that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergedSegment {
    /// Zero-based index of the producing chunk.
    pub chunk: u32,
    /// The accepted segment, its text possibly trimmed at a seam.
    pub segment: AsrSegmentDraft,
}

/// The merged, ordered transcript of a run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergedTranscript {
    /// Segments in source start order.
    pub segments: Vec<MergedSegment>,
    /// Seam duplicates removed.
    pub warnings: TranscriptWarnings,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Side {
    Left,
    Right,
}

/// Merges overlapping chunks into one ordered transcript (T-03).
///
/// Deterministic rules, applied in order:
///
/// 1. Each chunk owns a core running from the midpoint of its overlap with the
///    previous chunk to the midpoint of its overlap with the next. A segment
///    is kept by the chunk whose core contains the segment's midpoint. A copy
///    from another chunk is kept only when the owning chunk has no segment at
///    that time at all, so a recognizer miss in one chunk never loses speech
///    the other chunk heard.
/// 2. A kept segment within 0.5 s of an inner window edge was probably cut by
///    that edge. It is dropped when the neighbour across that edge has a
///    segment overlapping it that is not itself cut at the facing edge; that
///    neighbour segment is kept instead, wherever its midpoint lies. Without
///    such a neighbour segment the cut segment is kept, so speech is never lost.
/// 3. At each seam, the last kept segment of the earlier chunk and the first
///    kept segment of the later one are compared when their times overlap. If
///    the earlier one's normalised words end with at least two words the later
///    one starts with, those words are trimmed from the later copy, which is
///    dropped when nothing remains; a later copy wholly contained in the earlier
///    one is dropped. Repeats at times that do not overlap are genuine and kept.
///
/// `chunks` must be in plan order.
#[must_use]
pub fn merge_chunks(chunks: &[ChunkSegments]) -> MergedTranscript {
    let cores = chunk_cores(chunks);
    let mut kept = BTreeSet::new();
    for (index, chunk) in chunks.iter().enumerate() {
        for (position, segment) in chunk.segments.iter().enumerate() {
            let middle = segment.midpoint();
            let owner = cores
                .iter()
                .position(|(start, end)| middle >= *start && middle < *end)
                .unwrap_or(index);
            if owner != index {
                // The owning chunk decides, unless it heard nothing at this
                // time; then this copy is the only one and is kept.
                if !overlapping(&chunks[owner].segments, segment) {
                    kept.insert((index, position));
                }
                continue;
            }
            let mut cut_sides = Vec::new();
            if cut_at(chunks, index, segment, Side::Left) {
                cut_sides.push((index - 1, Side::Right));
            }
            if cut_at(chunks, index, segment, Side::Right) {
                cut_sides.push((index + 1, Side::Left));
            }
            let mut replacements = Vec::new();
            let covered = !cut_sides.is_empty()
                && cut_sides.iter().all(|(neighbour, facing)| {
                    let found = covering(chunks, *neighbour, segment, *facing);
                    let any = !found.is_empty();
                    replacements.extend(found);
                    any
                });
            if covered {
                kept.extend(replacements);
            } else {
                kept.insert((index, position));
            }
        }
    }
    let mut ordered: Vec<(usize, AsrSegmentDraft)> = kept
        .into_iter()
        .filter_map(|(index, position)| {
            chunks
                .get(index)
                .and_then(|chunk| chunk.segments.get(position))
                .map(|segment| (index, segment.clone()))
        })
        .collect();
    ordered.sort_by_key(|(index, segment)| (segment.range.start(), segment.range.end(), *index));
    let warnings = remove_seam_duplicates(&mut ordered, chunks);
    MergedTranscript {
        segments: ordered
            .into_iter()
            .map(|(index, segment)| MergedSegment {
                chunk: u32::try_from(index).unwrap_or(u32::MAX),
                segment,
            })
            .collect(),
        warnings,
    }
}

fn overlapping(segments: &[AsrSegmentDraft], segment: &AsrSegmentDraft) -> bool {
    segments.iter().any(|candidate| {
        candidate.range.start() < segment.range.end()
            && candidate.range.end() > segment.range.start()
    })
}

/// Each chunk's `[start, end)` core: overlap midpoints to overlap midpoints.
fn chunk_cores(chunks: &[ChunkSegments]) -> Vec<(u64, u64)> {
    let boundaries: Vec<u64> = chunks
        .windows(2)
        .map(|pair| {
            u64::midpoint(
                pair[1].chunk.window.start().as_micros(),
                pair[0].chunk.window.end().as_micros(),
            )
        })
        .collect();
    (0..chunks.len())
        .map(|index| {
            let start = if index == 0 { 0 } else { boundaries[index - 1] };
            let end = boundaries.get(index).copied().unwrap_or(u64::MAX);
            (start, end)
        })
        .collect()
}

/// Whether `segment` of chunk `index` lies within the edge margin of an inner
/// window edge on `side`.
fn cut_at(chunks: &[ChunkSegments], index: usize, segment: &AsrSegmentDraft, side: Side) -> bool {
    let window = chunks[index].chunk.window;
    match side {
        Side::Left => {
            index > 0
                && segment.range.start().as_micros()
                    < window.start().as_micros().saturating_add(SEAM_EDGE_MICROS)
        }
        Side::Right => {
            index + 1 < chunks.len()
                && segment
                    .range
                    .end()
                    .as_micros()
                    .saturating_add(SEAM_EDGE_MICROS)
                    > window.end().as_micros()
        }
    }
}

/// Segments of chunk `neighbour` overlapping `segment` and not cut at the
/// neighbour's edge that faces it.
fn covering(
    chunks: &[ChunkSegments],
    neighbour: usize,
    segment: &AsrSegmentDraft,
    facing: Side,
) -> Vec<(usize, usize)> {
    chunks[neighbour]
        .segments
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate.range.start() < segment.range.end()
                && candidate.range.end() > segment.range.start()
                && !cut_at(chunks, neighbour, candidate, facing)
        })
        .map(|(position, _)| (neighbour, position))
        .collect()
}

fn remove_seam_duplicates(
    ordered: &mut Vec<(usize, AsrSegmentDraft)>,
    chunks: &[ChunkSegments],
) -> TranscriptWarnings {
    let mut warnings = TranscriptWarnings::default();
    for (seam, later_chunk) in chunks.iter().enumerate().skip(1) {
        let earlier = ordered.iter().rposition(|(index, _)| *index == seam - 1);
        let later = ordered.iter().position(|(index, _)| *index == seam);
        let (Some(earlier), Some(later)) = (earlier, later) else {
            continue;
        };
        let (first, second) = (&ordered[earlier].1, &ordered[later].1);
        if first.range.start() >= second.range.end() || second.range.start() >= first.range.end() {
            continue;
        }
        let earlier_words = normalized_words(first.text.text());
        let later_words = normalized_words(second.text.text());
        let shared = shared_seam_words(&earlier_words, &later_words);
        if shared == 0 {
            continue;
        }
        let remaining = (shared < later_words.len())
            .then(|| without_leading_words(second.text.text(), shared))
            .flatten();
        match remaining {
            Some(text) => ordered[later].1.text = text,
            None => {
                ordered.remove(later);
            }
        }
        warnings.add(
            TranscriptWarningKind::SeamDuplicatesRemoved,
            1,
            later_chunk.chunk.ordinal(),
        );
    }
    warnings
}

/// Number of leading words of `later` repeated at the end of `earlier`, or
/// all of `later` when it appears whole inside `earlier`; zero when fewer than
/// [`MIN_SEAM_DUPLICATE_WORDS`] words are shared.
fn shared_seam_words(earlier: &[String], later: &[String]) -> usize {
    if later.len() >= MIN_SEAM_DUPLICATE_WORDS
        && earlier.windows(later.len()).any(|window| window == later)
    {
        return later.len();
    }
    (MIN_SEAM_DUPLICATE_WORDS..=earlier.len().min(later.len()))
        .rev()
        .find(|length| earlier[earlier.len() - length..] == later[..*length])
        .unwrap_or(0)
}

/// Lowercase words with punctuation removed, for comparing seam text.
fn normalized_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// `text` without its first `count` normalised words, or `None` when no word remains.
fn without_leading_words(text: &str, count: usize) -> Option<CueText> {
    let mut skipped = 0;
    let mut rest = Vec::new();
    for word in text.split_whitespace() {
        let counts = word.chars().any(char::is_alphanumeric);
        if skipped < count {
            if counts {
                skipped += 1;
            }
            continue;
        }
        rest.push(word);
    }
    if !rest
        .iter()
        .any(|word| word.chars().any(char::is_alphanumeric))
    {
        return None;
    }
    let joined = rest.join(" ");
    CueText::new(joined.clone(), joined).ok()
}

#[cfg(test)]
mod tests;
