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
//! - that a chunk with no audible signal, or with less than 100 ms of audio,
//!   is recorded as a gap rather than given to the recognizer;
//! - that a chunk whose recognised output cannot be used is a gap too, never a
//!   failed run, unless most of the run's answered chunks are
//!   ([`unusable_chunks_end_the_run`]);
//! - how the overlapping chunks are merged back into one ordered transcript
//!   without losing or doubling speech at a seam (T-03).
//!
//! Provider bytes and processes belong to infrastructure adapters; they hand
//! this module bounded, typed values and let it decide.

use std::{
    cmp::Reverse,
    collections::BTreeSet,
    error::Error,
    fmt,
    num::{NonZeroU16, NonZeroU32},
};

use crate::{
    Confidence, ConfidenceOrigin, CueText, LanguageTag, MAX_UNPLACED_STRETCHES, MediaTime,
    ProviderEndTrim, SourceSegment, SourceSegmentId, TimeRange, TranscriptRevisionError,
    TranscriptWarningKind, TranscriptWarnings,
    spans::{Span, SpanSet, join_nearest, merged, overlaps_any, span, subtract, to_ranges},
};

/// Sample rate of the speech PCM contract: mono signed 16-bit at 16 kHz.
pub const SPEECH_SAMPLE_RATE: u32 = 16_000;
/// Longest chunk window a plan may request; the PCM source bounds one chunk to it.
pub const MAX_CHUNK_WINDOW_MICROS: u64 = 30_000_000;
/// Most chunks one run may plan: four hours of R0 chunks, with headroom.
pub const MAX_PLANNED_CHUNKS: usize = 1_024;
/// How far past the speech a verified segment's end may lie in the local-ASR
/// verification (`setup check`), which compares a fixture's segment with the
/// recorded speech span.
///
/// For a chunk's provider end it is only part of the bound that
/// [`validate_chunk_output`] applies: the padded window of
/// [`MAX_CHUNK_WINDOW_MICROS`], or this much past audio that fills it, whichever
/// is later. Until P14 PR 7 (#274) a second past the decoded audio was the whole
/// bound, and a range cut mid-speech failed because whisper.cpp's segment ends
/// are predicted timestamp tokens, quantised coarsely on a small model, and are
/// not limited by the audio's length. A 5 s cut ended its last segment at 7 s.
pub const PROVIDER_END_TOLERANCE_MICROS: u64 = 1_000_000;
/// Most provider segments accepted for one chunk.
pub const MAX_PROVIDER_SEGMENTS: usize = 256;
/// Most provider tokens accepted for one segment.
pub const MAX_PROVIDER_TOKENS: usize = 512;
/// A segment this close to an inner window edge was probably cut by the edge.
const SEAM_EDGE_MICROS: u64 = 500_000;
/// The longest stretch of the union of all the validated segments of a run
/// that may be covered by no kept segment, and still be treated as the two
/// recognisers' timing noise and not as speech lost at a seam (#359): half a
/// second, the edge margin.
///
/// What it decides. [`merge_chunks`] keeps one of two segments that two chunks
/// recognised for the same speech, and drops the other. Wherever a stretch of
/// the union longer than this is covered by no kept segment, the guard restores
/// dropped segments over it, and a stretch no longer than this is left. So the
/// transcript has no hole longer than this in speech that any chunk wrote words
/// for, except where the repeat rule dropped a segment whose words a kept
/// segment has (rule 3), or the speech was never written by any chunk. The
/// guarantee is on the union and not on each dropped segment because a bound per
/// segment stacks (two dropped copies, each leaving up to this much at ends that
/// face each other, would leave twice this). The same length is the shortest
/// stretch of rejected words that [`merge_chunks`] reports as
/// [`MergedTranscript::unplaced`]: it would be inconsistent to report that much
/// of words nobody placed while leaving as much of a kept copy without a word,
/// and so no second figure has to be defended. That is a convention for the two
/// to agree, not a measurement of how long a stray word lasts.
///
/// Why this value. It is the edge margin because of one configuration, which is
/// an upper bound for it and not a proof that it is right for real speech. Take
/// the seam of #359: the dropped segment is cut by the later window's left edge,
/// and the segment that stands for it, in the earlier chunk, is not itself cut,
/// so it ends at least `SEAM_EDGE_MICROS` before its window's end `W`. What the
/// dropped segment carries beyond that end is left for at most the tolerance, so
/// it ends no later than `W - SEAM_EDGE_MICROS + tolerance`. With a tolerance of
/// at most the margin that is at or before `W`: what is left lies in audio the
/// earlier chunk heard too, and a segment that runs past `W`, into audio only the
/// later chunk heard, never loses the part beyond `W`. A tolerance of even one
/// microsecond more would let it, so for this configuration the margin is the
/// largest value that is safe. The right edge is the mirror image. The overlap
/// (5 s in the R0 plan) does not enter this bound. A smaller value would be as
/// safe at the window's edge and would restore more copies that differ by
/// timestamp noise; whether half a second is the right amount of that noise for
/// real speech has not been measured. A test finds the largest tolerance for
/// which two chunk layouts lose nothing beyond a window's edge by binary search,
/// so it checks the arithmetic of the code against this derivation (a change to
/// the margin moves it), not the choice.
///
/// What it costs. Up to this much at the end or start of a dropped copy can be
/// words the kept segment's recogniser did not write down (about one word at a
/// normal speaking rate), with no sign. The release before lost that and
/// everything after the neighbour's end with it.
pub const SEAM_COVERAGE_TOLERANCE_MICROS: u64 = SEAM_EDGE_MICROS;
/// Fewest shared words that identify seam text as a duplicate rather than a
/// genuine repeat.
const MIN_SEAM_DUPLICATE_WORDS: usize = 2;
/// 20 ms of 16 kHz audio.
const SILENCE_FRAME_SAMPLES: usize = 320;
/// Full-scale amplitude of signed 16-bit PCM, squared.
const FULL_SCALE_SQUARED: u128 = 32_768 * 32_768;
/// -50 dBFS as a power ratio denominator: 10^(50/10).
const SILENCE_POWER_RATIO: u128 = 100_000;
/// The least decoded speech audio a recognizer is given: 1,600 mono 16 kHz
/// samples, which is 100 ms. A chunk that decodes to less is recorded as a gap
/// and never reaches a recognizer ([`is_below_recognition_floor`]), and a chunk
/// whose window is shorter than that is not even decoded
/// ([`MIN_RECOGNITION_MICROS`], [`PlannedChunk::is_below_recognition_floor`]).
///
/// Why there is a floor. Nothing upstream sets a least length: a requested
/// range may be one microsecond, the last chunk of a plan is whatever is left
/// of its range, and a decoder returns only the samples an audio track has in a
/// window. So a recognizer could be handed a handful of samples, which no
/// recognizer can turn into words and which a recognizer need not survive: the
/// reviewed whisper.cpp v1.9.2 reads up to 200 samples past the buffer when it
/// is given fewer than 201 (its spectrogram mirrors samples 1 to 200 before it
/// checks the length; fixed upstream in v1.9.3), and by a reading of its
/// source its command line fails outright for 40 or fewer, which fails the
/// whole run (#322, known limit L-137). A user's own recognizer build is not
/// under review at all, so the bound belongs here, in front of every
/// recognizer, and not in one adapter.
///
/// Why 1,600. 100 ms is the figure below which whisper.cpp itself decodes
/// nothing: `whisper_full_with_state` returns without a segment when the
/// spectrogram has fewer than ten 10 ms frames ("input is too short ... <
/// 100 ms"). So a chunk under the floor is one that recognition would have
/// answered with nothing, were it safe to ask. The floor is about eight times
/// the 201 samples the unfixed read needs, and far below any spoken word. It
/// is a bound on what is sent, not a model of the recognizer: whisper.cpp's
/// framing reaches ten frames only at 1,640 samples, so audio a little over
/// the floor is sent and still answered with nothing, which is harmless.
pub const MIN_RECOGNITION_SAMPLES: usize = 1_600;
/// [`MIN_RECOGNITION_SAMPLES`] as source time: 100,000 microseconds. A planned
/// chunk whose window is shorter is recorded as a gap without being decoded
/// ([`PlannedChunk::is_below_recognition_floor`]).
///
/// Why the window is judged before the decode, and not only the samples
/// after it. A window under the floor asks for less audio than a recognizer
/// is ever given, so decoding it can only produce a gap; and a decoder must
/// not be trusted with a length that small. A window shorter than half a
/// sample made the media tool return a whole block of audio instead, about
/// four seconds of a 16 kHz track (it takes a duration that rounds to no
/// samples for no limit at all and then hands back one block of its filter),
/// so a range of a few microseconds was recognised as seconds of speech
/// (#332). Judging the request makes the answer the same for every decoder.
pub const MIN_RECOGNITION_MICROS: u64 = 100_000;
/// A chunk is unusable when more than one in this many of its text segments is
/// rejected ([`validate_chunk_output`]): a quarter.
const REJECTED_SEGMENT_SHARE_DENOMINATOR: u32 = 4;
/// The text-segment count from which the quarter rule alone judges a chunk
/// (#353). Below it a chunk tolerates **at most one** rejected segment, which is
/// dropped and counted while the others are kept; two or more rejected make it
/// unusable, as does every segment rejected, at any size.
///
/// Why 4, derived. The rule fails a chunk when more than a quarter of its text
/// segments is rejected. A single rejected segment is a quarter or less of the
/// chunk only from four segments on (1 of 4 is exactly a quarter; 1 of 3 is a
/// third), so four is the smallest count at which the quarter rule alone
/// tolerates one rejection. Below it the quarter rule alone tolerated none: it
/// failed a chunk for one rejection beside any number of kept segments, which
/// is the defect. The rule now tolerates one there by decision, so the rule is
/// the same for every count at or above four, and for one, two and three
/// segments it is "one rejection is not enough". Both sides agree at four
/// (the quarter rule tolerates one, the allowance tolerates one), so the rule
/// is monotonic: a chunk that is usable stays usable when a valid segment is
/// added, and an unusable one stays unusable when a rejection is added. A test
/// finds the constant by search and checks that property for every count up to
/// sixteen.
///
/// From four segments on the rule is the one it was, unchanged, so a chunk of
/// dense speech is judged as before and every chunk that passed before passes
/// the same way. A real recording with long pauses has chunks of two or three
/// segments, and one segment whose times do not fit its audio (empty,
/// backwards, at or after the audio's end) is routine there.
///
/// A chunk whose every text segment is rejected is unusable whatever its size:
/// nothing the recogniser said about it can be placed in its audio.
pub const MIN_SEGMENTS_FOR_REJECTION_RATIO: u32 = 4;
/// A run fails when more than one in this many of the chunks the recogniser
/// answered is unusable ([`unusable_chunks_end_the_run`]): a half, so "most".
pub const UNUSABLE_CHUNK_SHARE_DENOMINATOR: u32 = 2;
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
    /// The pinned multilingual whisper `base` model (ADR 0005), the default.
    Base,
    /// The pinned 5-bit (`q5_1`) quantization of the same multilingual `base`
    /// model: about 40% of its size, for machines where the default does not
    /// fit (maintainer decision D6).
    BaseQ5_1,
    /// A model file whose bytes match no reviewed profile.
    Unreviewed,
}

impl AsrModelProfile {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::BaseQ5_1 => "base_q5_1",
            Self::Unreviewed => "unreviewed",
        }
    }

    /// The reviewed pinned profile this is, or `None` for an unreviewed model.
    #[must_use]
    pub const fn reviewed(self) -> Option<ReviewedAsrModel> {
        match self {
            Self::Base => Some(ReviewedAsrModel::Base),
            Self::BaseQ5_1 => Some(ReviewedAsrModel::BaseQ5_1),
            Self::Unreviewed => None,
        }
    }
}

/// A reviewed pinned model profile: the only models a local-ASR run uses
/// (maintainer decision D5).
///
/// It is [`AsrModelProfile`] without `Unreviewed`, so a value that claims a
/// model is pinned can never name an unreviewed one. Which profile runs is
/// decided by the identity (size and SHA-256) of the registered model file,
/// never by a configuration field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewedAsrModel {
    /// The multilingual whisper `base` model, the default.
    Base,
    /// The `q5_1` quantization of the multilingual `base` model.
    BaseQ5_1,
}

impl ReviewedAsrModel {
    /// Every reviewed profile, default first.
    pub const ALL: [Self; 2] = [Self::Base, Self::BaseQ5_1];

    /// The model profile recorded in provenance.
    #[must_use]
    pub const fn profile(self) -> AsrModelProfile {
        match self {
            Self::Base => AsrModelProfile::Base,
            Self::BaseQ5_1 => AsrModelProfile::BaseQ5_1,
        }
    }

    /// Stable machine-readable identifier, the same as its profile's.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        self.profile().identifier()
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

    /// Whether the window is shorter than [`MIN_RECOGNITION_MICROS`]
    /// (100 ms): too short to hold the least audio a recognizer is given.
    ///
    /// Such a chunk is recorded as a gap with no audio
    /// ([`AsrChunkOutcome::NoAudio`]) and is neither decoded nor recognised.
    /// With the R0 plan only a requested range under 100 ms gives one: the
    /// last window of a longer range is at least the overlap long.
    #[must_use]
    pub const fn is_below_recognition_floor(&self) -> bool {
        self.window.duration_micros() < MIN_RECOGNITION_MICROS
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

/// Whether the length of `range`, counted in samples of mono 16 kHz PCM (the
/// form every audio decode is asked for, a speech chunk and an evidence clip
/// alike) and rounded to the nearest, is no sample at all: under half a sample
/// (31.25 microseconds), so 31 microseconds or less.
///
/// Such a range is never a decode request. The media tool cuts its output by
/// that rounded count, and a count of none it takes for no limit at all: a
/// range of 31 microseconds or less was answered with one whole block of the
/// tool's filter, up to 65,536 samples of the source (about four seconds at
/// 16 kHz, or what is left of the audio when that is less), not with its
/// range (#332). The rule refuses exactly those lengths and no other: a range of 32
/// to 62 microseconds is shorter than a sample too, but it rounds to one, the
/// tool cuts it to one, and a request that was answered correctly keeps its
/// answer.
#[must_use]
pub fn rounds_to_no_pcm_sample(range: TimeRange) -> bool {
    range
        .duration_micros()
        .saturating_mul(2 * u64::from(SPEECH_SAMPLE_RATE))
        < MICROS_PER_SECOND
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
    /// Audio was decoded but held nothing a recognizer could use, so the
    /// recognizer was not run and the window is a gap: every 20 ms frame was
    /// below -50 dBFS ([`is_silent_pcm`]), or there was less than 100 ms of it
    /// ([`is_below_recognition_floor`]), whatever its level.
    ///
    /// The second case shares this outcome and its stored name (`silent`)
    /// rather than getting one of its own: a new outcome in a stored revision
    /// is one that the releases before it read as damage, and a consumer acts
    /// on both in the same way (nothing was transcribed here, and the decoded
    /// range says how much audio there was). `audio` tells the two apart for a
    /// reader who needs to: a range under 100 ms is the short case.
    Silent {
        /// Observed first decoded sample to last decoded sample.
        audio: TimeRange,
    },
    /// No audio sample was decoded in the window; it is a gap with no audio.
    /// Either the stream holds none there, or the window is shorter than
    /// 100 ms and was not decoded at all
    /// ([`PlannedChunk::is_below_recognition_floor`]): nothing was observed,
    /// so there is no decoded range to record, and this outcome, which has
    /// none, is the one that says so.
    NoAudio,
    /// Audio was decoded and given to the recognizer, and what it answered
    /// could not be used for this chunk as a whole ([`validate_chunk_output`]
    /// refused it): every text segment was rejected, two or more were among
    /// fewer than [`MIN_SEGMENTS_FOR_REJECTION_RATIO`], more than a quarter
    /// were among at least that many, or the output broke a structural rule.
    /// The window has no transcript from this chunk (#353).
    ///
    /// This is a gap that is not a quiet one, and it is never recorded as
    /// [`Self::Silent`] or [`Self::NoAudio`]: those say that nothing was there
    /// to recognise, and here there may have been speech that the recognizer
    /// could not place. So a search reports the window as untranscribed, where
    /// a quiet one is reported as no speech, and a reader is not told a
    /// spoken passage was silence. `audio` is the decoded range, as for the
    /// other outcomes that decoded audio. A run in which most chunks that the
    /// recognizer answered are unusable fails instead
    /// ([`unusable_chunks_end_the_run`]), so a revision holds this outcome
    /// only for a recognizer that described most of its audio.
    Unusable {
        /// Observed first decoded sample to last decoded sample.
        audio: TimeRange,
    },
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

    /// The parts of the run's range that no chunk of it transcribed or found
    /// quiet, because the chunks that cover them were unusable
    /// ([`AsrChunkOutcome::Unusable`]) and no neighbouring chunk, which overlaps
    /// them by five seconds, covers them: in start order, merged, empty when the
    /// run had no unusable chunk (#353).
    ///
    /// These are the ranges the run did not re-transcribe. What becomes of the
    /// text the superseded revision had in and around them is
    /// [`EarlierTextRule`]'s.
    #[must_use]
    pub fn unusable_gaps(&self) -> Vec<TimeRange> {
        let mut covered = Vec::new();
        let mut unusable = Vec::new();
        for record in &self.chunks {
            let window = span(record.chunk.window);
            match record.outcome {
                AsrChunkOutcome::Unusable { .. } => unusable.push(window),
                AsrChunkOutcome::Transcribed { .. }
                | AsrChunkOutcome::Silent { .. }
                | AsrChunkOutcome::NoAudio => covered.push(window),
            }
        }
        to_ranges(&subtract(&merged(unusable), &merged(covered)))
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

/// What a retranscription keeps of the earlier text inside the range it
/// replaced, when it could not read all of that range (#353).
///
/// **The rule, in one sentence:** a segment of the superseded revision that
/// lies in the replaced range is kept, whole and with its original provenance,
/// when it reaches into a part of the range the run could not read (a gap,
/// [`AsrRun::unusable_gaps`]) and none of the text the run itself wrote
/// overlaps it; every other segment in the range is replaced.
///
/// Why this and not another. A run replaces earlier text because it read the
/// audio and wrote what it heard. A segment that lies wholly in what the run
/// read is replaced by that, or by silence. A segment that lies wholly in a gap
/// was not read at all, so there is nothing to replace it with and deleting it
/// would only lose text. A segment that crosses the edge of a gap, which sits
/// at the edge of a neighbouring chunk's window, was read only in part, and
/// even a microsecond past the edge makes it so:
///
/// - If the run's own text overlaps it, the run has heard the same words and
///   its text replaces the segment, as it replaces any other (a sentence that
///   crosses a window edge is heard again by the neighbour that read its end).
/// - If nothing the run wrote overlaps it, the part of the audio the run did
///   read and found quiet, or without text, is no evidence against the whole
///   segment: the read part of a window beside an unread one is its edge, which
///   the chunk overlap exists to make a neighbour answer for. The segment is
///   kept whole. A run does not replace what it did not read in full.
///
/// The rule is the same for every consumer: the application builds the
/// revision with it, the revision applies it when it is validated (so a stored
/// record cannot keep or drop what the rule would not), and coverage counts
/// what it keeps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EarlierTextRule {
    replaced: Span,
    gaps: Vec<Span>,
    written: Vec<Span>,
}

impl EarlierTextRule {
    /// The rule for a run over `replaced` whose own text occupies `written`.
    pub fn new(
        replaced: TimeRange,
        run: &AsrRun,
        written: impl IntoIterator<Item = TimeRange>,
    ) -> Self {
        Self {
            replaced: span(replaced),
            gaps: run.unusable_gaps().into_iter().map(span).collect(),
            written: merged(written.into_iter().map(span).collect()),
        }
    }

    /// Whether a segment of the superseded revision at `earlier` is carried
    /// into the new revision: it lies outside the replaced range, or lies in
    /// it and the rule keeps it. A segment that crosses the replaced range's
    /// own edge is never carried (a range is widened to whole segments, so
    /// none does).
    #[must_use]
    pub fn carries(&self, earlier: TimeRange) -> bool {
        let candidate = span(earlier);
        if !overlaps_any(&[self.replaced], candidate) {
            return true;
        }
        self.replaced.0 <= candidate.0
            && candidate.1 <= self.replaced.1
            && overlaps_any(&self.gaps, candidate)
            && !overlaps_any(&self.written, candidate)
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
///
/// Such a chunk is an unusable chunk ([`AsrChunkOutcome::Unusable`]), not a
/// failed run; whether enough chunks are unusable to fail the run is decided by
/// [`unusable_chunks_end_the_run`].
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
    /// Every text segment of the chunk, or two or more of fewer than
    /// [`MIN_SEGMENTS_FOR_REJECTION_RATIO`], or more than a quarter of at least
    /// that many, had a range that could not be placed in its decoded audio.
    TooManyRejectedSegments,
}

impl ProviderOutputError {
    /// Every reason, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::TooManySegments,
        Self::TooManyTokens,
        Self::OutOfOrderSegments,
        Self::InvalidTokenProbability,
        Self::TooManyRejectedSegments,
    ];

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

    /// Parses a stable identifier, as a stored checkpoint holds it.
    #[must_use]
    pub fn parse(identifier: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reason| reason.identifier() == identifier)
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
    unplaced: Vec<TimeRange>,
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

    /// The stretches of this chunk's audio in which words the recogniser wrote
    /// down may lie, which were rejected and so are in no segment (#359): see
    /// [`ChunkSegments::unplaced`]. Empty when nothing of the kind was rejected.
    #[must_use]
    pub fn unplaced(&self) -> &[TimeRange] {
        &self.unplaced
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
                unplaced: self.unplaced,
            },
            self.language,
            self.warnings,
        )
    }
}

/// The latest chunk-relative time a provider end may name and still be cut to
/// the audio end, for audio of `decoded` microseconds.
///
/// A recogniser's end timestamps are predicted tokens that run on past a short
/// audio (#274), but they stay inside the padded window it works in, so the
/// bound is the window ([`MAX_CHUNK_WINDOW_MICROS`], 30 s): an end beyond it is
/// not a timestamp of this audio at all and the segment is rejected. The bound
/// is never below [`PROVIDER_END_TOLERANCE_MICROS`] past the audio, which is
/// what 0.1.0 applied to every chunk, so a stored revision that 0.1.0 accepted
/// (a full window with an end a few hundred milliseconds past it) is still
/// valid here.
const fn provider_end_limit(decoded: u64) -> u64 {
    let tolerated = decoded.saturating_add(PROVIDER_END_TOLERANCE_MICROS);
    if tolerated > MAX_CHUNK_WINDOW_MICROS {
        tolerated
    } else {
        MAX_CHUNK_WINDOW_MICROS
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
        // The segment starts inside the audio (checked above), so its end is
        // cut at the audio end however far past it the provider put it, up to
        // the bound of `provider_end_limit`.
        ProviderEndTrim::TrimmedToAudioEnd
            if end > decoded && end <= provider_end_limit(decoded) =>
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
/// audio end, or would leave `source`, is rejected and counted. The chunk
/// fails when **every** text segment is rejected, or, with at least
/// [`MIN_SEGMENTS_FOR_REJECTION_RATIO`] text segments, when more than a quarter
/// are, because the provider evidently did not describe this audio. With fewer
/// segments than that a share means nothing (one rejection beside two kept
/// segments is a third), so **one** rejected segment is dropped and counted and
/// the rest are kept, and two or more fail the chunk (#353).
///
/// A segment that starts inside the audio and **ends past it, as far as the
/// padded 30 s window the recogniser works in**, is cut to the audio end and
/// counted, keeping the raw provider end: a recogniser's end timestamps are
/// predicted, not measured, and a range cut mid-speech makes it run on
/// (whisper.cpp ended a 5 s cut's last segment at 7 s, #274), so an end past
/// the audio says nothing about the audio's content, and the cut end is the
/// audio's end, not evidence that speech continued there. An end beyond that
/// window (or, for audio that fills it, more than a second past the audio, the
/// bound 0.1.0 applied) is not a time of this audio and rejects the segment.
/// Whole-segment non-speech markers and empty segments are removed and
/// counted; an empty *text* is a marker, not a rejection, and never counts
/// towards the share. Confidence is the mean probability of text tokens,
/// `provider_uncalibrated`.
///
/// # Errors
///
/// Returns the [`ProviderOutputError`] that makes the chunk unusable. The run
/// decides what an unusable chunk costs it ([`unusable_chunks_end_the_run`]).
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
    let mut unplaced = Vec::new();
    // The start of each segment in provider order, markers and rejected ones
    // included: where one rejected segment's words may end is where the next
    // thing the recogniser said begins.
    let starts: Vec<ChunkTime> = output
        .segments
        .iter()
        .map(|segment| segment.start)
        .collect();
    for (position, segment) in output.segments.into_iter().enumerate() {
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
            unplaced.extend(unplaced_stretch(
                audio,
                source,
                &text,
                (segment.start, segment.end),
                starts.get(position + 1).copied(),
            ));
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
    if rejections_make_chunk_unusable(considered, rejected) {
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
        unplaced,
        warnings,
    })
}

/// The stretch of `audio` in which the words of a rejected segment may lie, or
/// `None` when no stretch can be said (#359).
///
/// A rejected segment names a time that cannot be placed in its audio, so it
/// cannot say how long its words took. What can be said is where they start, when
/// the segment gives a start inside the audio, and where they end by: the
/// recogniser wrote its segments in order, so words that start at the segment's
/// start end before the next thing it wrote begins. The stretch runs from the
/// segment's own start (never before it, whatever its end says) to the start of
/// the next segment in provider order, whatever that is (a marker, a rejected
/// segment, a kept one). It is an upper bound, not a measurement: the words took
/// part of it. [`merge_chunks`] takes what the kept segments cover out of it and
/// reports the rest if it is longer than [`SEAM_COVERAGE_TOLERANCE_MICROS`].
///
/// Nothing is said, and the segment stays only counted, when
///
/// - its text has no letter or digit: punctuation alone is not a word, so
///   nothing a reader could look for is lost;
/// - its range is empty (`start` equals `end`): an empty range says nothing
///   about where its words lie, and recognisers write text with an empty range
///   in silence (a "Thank you." at 0 s before speech that begins at 20 s would
///   otherwise report twenty seconds as untranscribed, and turn a search of an
///   ordinary recording `partial`). A reversed range (`end` before `start`) is a
///   fault of a segment that did state a start, and keeps its stretch, from that
///   start;
/// - it starts at or after the audio's end: words the recogniser places after
///   the audio ended are not about the audio, as an end past it is not evidence
///   that speech continued there ([`TranscriptWarningKind::ProviderEndTrimmed`]),
///   and the recogniser works in a window padded to 30 s, so a time after the
///   audio's end is a time in the padding;
/// - no segment starts after it inside the audio: nothing bounds its words, and
///   a stretch to the audio's end would list a silent tail as untranscribed.
///   That is a limit, not a claim that nothing was lost. The words of a chunk's
///   last segment are usually also in the next chunk's first seconds, but not
///   for the last chunk of a run, nor for one whose neighbour is silent or
///   unusable.
///
/// The bound has a cliff. A next segment 0.1 s before the audio's end gives a
/// stretch of nearly all that is left of the chunk; one at or after the audio's
/// end gives none. Both follow from "up to the next thing the recogniser wrote".
fn unplaced_stretch(
    audio: TimeRange,
    source: TimeRange,
    text: &CueText,
    (start, end): (ChunkTime, ChunkTime),
    next: Option<ChunkTime>,
) -> Option<TimeRange> {
    if start == end || !text.text().chars().any(char::is_alphanumeric) {
        return None;
    }
    let decoded = audio.duration_micros();
    let from = start.as_micros();
    let until = next?.as_micros();
    if from >= decoded || until >= decoded || from >= until {
        return None;
    }
    let first = audio.start().as_micros().checked_add(from)?;
    let last = audio.start().as_micros().checked_add(until)?;
    TimeRange::new(
        MediaTime::from_micros(first.max(source.start().as_micros())),
        MediaTime::from_micros(last.min(source.end().as_micros())),
    )
    .ok()
}

/// Whether `rejected` of `considered` text segments make a chunk unusable
/// ([`validate_chunk_output`]): all of them; or, below
/// [`MIN_SEGMENTS_FOR_REJECTION_RATIO`] segments, more than one; or, from that
/// count on, more than a quarter.
///
/// It is monotonic in both arguments, which is what makes it safe to reason
/// about: another valid segment never turns a usable chunk unusable, and
/// another rejection never turns an unusable one usable. (A rule that judged
/// the share below the minimum, or only the count, would not be: with two
/// rejections, `[rejected, ok, rejected]` would be kept and `[rejected, ok,
/// rejected, ok]` would fail.) A test checks it for every pair up to sixteen
/// segments.
const fn rejections_make_chunk_unusable(considered: u32, rejected: u32) -> bool {
    if considered == 0 {
        return false;
    }
    if rejected == considered {
        return true;
    }
    if considered < MIN_SEGMENTS_FOR_REJECTION_RATIO {
        return rejected > 1;
    }
    rejected.saturating_mul(REJECTED_SEGMENT_SHARE_DENOMINATOR) > considered
}

/// Whether `unusable` of the `answered` chunks of a run end it: the run fails
/// when more than half of the chunks the recognizer answered are unusable
/// ([`UNUSABLE_CHUNK_SHARE_DENOMINATOR`]), because it then evidently did not
/// describe this audio (T-05). Otherwise the unusable chunks are recorded as
/// gaps ([`AsrChunkOutcome::Unusable`]) and the run goes on (#353).
///
/// `answered` counts every chunk that was given to the recognizer and answered,
/// usable or not. A chunk that was never given to it (silent, too short, no
/// audio) says nothing about the recognizer and is not in it.
///
/// How the threshold was chosen:
///
/// - **More than half**, because the failure the rule is for (a recognizer that
///   answers with garbage, such as one run on audio it cannot read) fails most
///   chunks, while a recording with pauses and typing fails few: the real
///   recording that found #353 failed 1 chunk of 83. One chunk that cannot be
///   placed must not cost the 82 that can.
/// - **A strict majority, not a half**, so that a run of two chunks of which one
///   is unusable is a partial run, not a failure: the other chunk was answered
///   by the same recognizer, build and model on audio of the same recording,
///   which shows the recognizer works. The one chunk that is unproven is
///   reported as a gap. A run whose every answered chunk is unusable fails
///   whatever its size, so a single chunk the recognizer cannot describe (a
///   short range) still fails, as it always did.
/// - **Consecutive failures are not a second rule.** A recording with a long
///   stretch of typing or music can have several unusable chunks in a row, and
///   chunks overlap by five seconds, so adjacent chunks are not independent
///   samples. Only the share of the answered chunks separates a broken
///   recognizer from such a stretch.
///
/// This is a maintainer decision to confirm (ADR 0017, note of 2026-10-10).
#[must_use]
pub const fn unusable_chunks_end_the_run(unusable: u32, answered: u32) -> bool {
    unusable.saturating_mul(UNUSABLE_CHUNK_SHARE_DENOMINATOR) > answered
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

/// Whether decoded speech PCM is too short to be given to a recognizer: fewer
/// than [`MIN_RECOGNITION_SAMPLES`] samples (100 ms), whatever they hold.
///
/// Such a chunk is recorded as a gap, exactly as a silent one is
/// ([`AsrChunkOutcome::Silent`], with its decoded range), and the recognizer
/// is not run. The constant says why. Loud or quiet makes no difference: the
/// rule is about how much audio there is, and it is decided before the silence
/// test so that it never depends on the signal.
#[must_use]
pub const fn is_below_recognition_floor(samples: &[i16]) -> bool {
    samples.len() < MIN_RECOGNITION_SAMPLES
}

/// One chunk's accepted segments, the input of [`merge_chunks`].
///
/// A chunk that was not recognised (silent, too short or with no audio), or
/// whose recognised output was unusable, takes part with no segments: it still
/// owns its core, so nothing from a neighbour is attributed into its time, and
/// a neighbour's copy of speech that falls in it is kept, because the owner
/// heard nothing there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkSegments {
    /// The planned chunk.
    pub chunk: PlannedChunk,
    /// Accepted segments in provider order.
    pub segments: Vec<AsrSegmentDraft>,
    /// Where the recogniser wrote words that were rejected and so are in no
    /// segment (#359): for each rejected segment with a word in its text, a
    /// range that is not empty, a start inside the chunk's audio, and another
    /// segment after it there, the stretch from its start to the next one's. An
    /// upper bound on where the words were said, not a measurement.
    /// [`merge_chunks`] takes the parts of it that kept segments cover out and
    /// reports what is left, if it is longer than the tolerance
    /// ([`SEAM_COVERAGE_TOLERANCE_MICROS`]), as [`MergedTranscript::unplaced`].
    pub unplaced: Vec<TimeRange>,
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
    /// The parts of the run where the recogniser wrote words that no segment
    /// holds, in start order and merged (#359): the stretches of
    /// [`ChunkSegments::unplaced`] that no kept segment covers, where they are
    /// longer than [`SEAM_COVERAGE_TOLERANCE_MICROS`]. Each lies inside the
    /// audio of the chunks and outside every segment above, and at most
    /// [`crate::MAX_UNPLACED_STRETCHES`] are listed (the nearest are joined
    /// beyond that). Empty for a run whose rejected segments (if any) left
    /// nothing the kept segments do not cover.
    pub unplaced: Vec<TimeRange>,
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
///    segment spanning the cut segment's midpoint that is not itself cut at
///    the facing edge; that neighbour segment is kept instead, wherever its
///    own midpoint lies. A neighbour segment that only touches the cut one
///    (the end of the previous sentence) is not a copy of it. Without such a
///    neighbour segment the cut segment is kept.
///
///    **The guard on rules 1 and 2 (#359).** Before rule 3, segments that rules
///    1 and 2 dropped are restored until no stretch longer than
///    [`SEAM_COVERAGE_TOLERANCE_MICROS`] of the union of every validated segment
///    of every chunk is covered by no kept segment (the guard's own rustdoc says
///    how, and why the guarantee is on the union). Rules 1 and 2 decide by one
///    point of the segment (its midpoint) and by whether a neighbour has a
///    segment there, so they could drop a segment that runs past its neighbour's
///    end, or starts before it, into audio that no other chunk heard, and lose
///    the words there with no sign; the guard looks at the time itself, at either
///    edge and along a chain of chunks. **It judges by time and not by words**, so
///    it can restore a copy whose words the kept neighbour also has, and then the
///    same words stand twice where nothing was lost; a repeat can be seen, a hole
///    cannot, and the trade is made on purpose. A run in which the union has no
///    such hole is merged exactly as rules 1 to 3 alone merge it (a test holds
///    that against a frozen copy of them), but a run in which no words were
///    lost may still be changed, because words and time are not the same.
/// 3. At each seam, the last kept segment of the earlier chunk and the first
///    kept segment of the later one are compared when their times overlap. If
///    the earlier one's normalised words end with at least two words the later
///    one starts with, those words are trimmed from the later copy, which is
///    dropped when nothing remains; a later copy wholly contained in the earlier
///    one is dropped. Repeats at times that do not overlap are genuine and kept.
///    **Only that one pair is compared**, so a restored segment that is not the
///    first of its chunk to be kept, or whose neighbour is not the last of the
///    earlier chunk to be kept, is never trimmed, and a repeat that differs by a
///    word is never found. This rule decides about words, not time, and its
///    containment can drop a segment that carries new time (a later copy whose
///    few words all occur in the earlier one, while it runs on past it): the
///    guarantee of the guard is therefore on time before this rule, and after it
///    holds except where this rule has dropped a copy by its words.
///
/// Last, [`MergedTranscript::unplaced`] is what the chunks' rejected segments
/// leave uncovered: [`ChunkSegments::unplaced`] less the kept segments, where
/// what is left is longer than the tolerance.
///
/// `chunks` must be in plan order.
#[must_use]
pub fn merge_chunks(chunks: &[ChunkSegments]) -> MergedTranscript {
    let mut kept = keep_by_rules_one_and_two(chunks);
    restore_lost_speech(chunks, &mut kept);
    assemble(chunks, kept)
}

/// Rules 1 and 2 of [`merge_chunks`]: the `(chunk, position)` of every segment
/// they keep.
fn keep_by_rules_one_and_two(chunks: &[ChunkSegments]) -> BTreeSet<(usize, usize)> {
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
    kept
}

/// Orders the kept segments, applies rule 3 and finds what is left unplaced.
fn assemble(chunks: &[ChunkSegments], kept: BTreeSet<(usize, usize)>) -> MergedTranscript {
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
    let unplaced = uncovered_unplaced_stretches(chunks, &ordered);
    MergedTranscript {
        segments: ordered
            .into_iter()
            .map(|(index, segment)| MergedSegment {
                chunk: u32::try_from(index).unwrap_or(u32::MAX),
                segment,
            })
            .collect(),
        warnings,
        unplaced,
    }
}

/// The guard on rules 1 and 2 of [`merge_chunks`] (#359): puts segments back
/// until no stretch longer than [`SEAM_COVERAGE_TOLERANCE_MICROS`] of **the union
/// of all the validated segments of all the chunks** is covered by no kept
/// segment.
///
/// The guarantee is on the union and not on each dropped segment because a bound
/// per segment stacks: two dropped copies that each leave half the tolerance
/// uncovered, at ends that face each other, leave a hole of the whole tolerance
/// twice over, in speech that both chunks wrote words for. A guarantee on the
/// union cannot: a stretch of it that nothing kept covers is a hole of the
/// transcript, however many dropped segments lie over it. A guarantee on the
/// union also implies the one on each segment, since a segment lies inside the
/// union.
///
/// How, deterministically. The holes (the stretches of the union that nothing
/// kept covers) are taken left to right. For a hole longer than the tolerance,
/// the dropped segments that overlap it are taken the most overlapping first
/// (then by start, end, chunk and position), each restored if it overlaps a
/// remaining piece of the hole that is still longer than the tolerance, until no
/// such piece is left. A hole is therefore closed by as few segments as that
/// order needs, whole, and a segment that an earlier restoration made needless
/// is not restored. Nothing that rules 1 and 2 kept is ever removed. The hole's
/// dropped segments cover it entirely (it lies in the union and nothing kept
/// covers it), so the loop always ends with every piece within the tolerance.
fn restore_lost_speech(chunks: &[ChunkSegments], kept: &mut BTreeSet<(usize, usize)>) {
    restore_lost_speech_within(chunks, kept, SEAM_COVERAGE_TOLERANCE_MICROS);
}

/// [`restore_lost_speech`] for a given tolerance, so that a test can find by
/// search the largest one with the arithmetic that
/// [`SEAM_COVERAGE_TOLERANCE_MICROS`] explains.
fn restore_lost_speech_within(
    chunks: &[ChunkSegments],
    kept: &mut BTreeSet<(usize, usize)>,
    tolerance: u64,
) {
    let mut covered = SpanSet::default();
    let mut every: Vec<Span> = Vec::new();
    let mut dropped: Vec<(Span, usize, usize)> = Vec::new();
    for (index, chunk) in chunks.iter().enumerate() {
        for (position, segment) in chunk.segments.iter().enumerate() {
            let placed = span(segment.range);
            every.push(placed);
            if kept.contains(&(index, position)) {
                covered.insert(placed);
            } else {
                dropped.push((placed, index, position));
            }
        }
    }
    dropped.sort_unstable();
    let longest = dropped
        .iter()
        .map(|&((start, end), _, _)| end - start)
        .max()
        .unwrap_or(0);
    for stretch in merged(every) {
        for hole in covered.gaps_within(stretch) {
            if hole.1 - hole.0 > tolerance {
                close_hole(&dropped, longest, hole, tolerance, kept, &mut covered);
            }
        }
    }
}

/// Restores dropped segments over `hole`, as [`restore_lost_speech`] says.
/// `dropped` is sorted by start and `longest` is the longest of them, so that
/// the ones that can overlap the hole are found without looking at the rest.
fn close_hole(
    dropped: &[(Span, usize, usize)],
    longest: u64,
    (from, until): Span,
    tolerance: u64,
    kept: &mut BTreeSet<(usize, usize)>,
    covered: &mut SpanSet,
) {
    let first =
        dropped.partition_point(|&((start, _), _, _)| start.saturating_add(longest) <= from);
    let after = dropped.partition_point(|&((start, _), _, _)| start < until);
    let mut candidates: Vec<(u64, Span, usize, usize)> = dropped
        .get(first..after)
        .unwrap_or_default()
        .iter()
        .filter(|&&((_, end), index, position)| end > from && !kept.contains(&(index, position)))
        .map(|&((start, end), index, position)| {
            let overlap = end.min(until) - start.max(from);
            (overlap, (start, end), index, position)
        })
        .collect();
    candidates.sort_unstable_by_key(|&(overlap, placed, index, position)| {
        (Reverse(overlap), placed, index, position)
    });
    for (_, placed, index, position) in candidates {
        let wide: Vec<Span> = covered
            .gaps_within((from, until))
            .into_iter()
            .filter(|&(start, end)| end - start > tolerance)
            .collect();
        if wide.is_empty() {
            break;
        }
        if wide
            .iter()
            .any(|&(start, end)| placed.0 < end && start < placed.1)
        {
            kept.insert((index, position));
            covered.insert(placed);
        }
    }
}

/// [`MergedTranscript::unplaced`]: what the chunks' rejected segments left
/// uncovered by the segments that were kept, where it is longer than
/// [`SEAM_COVERAGE_TOLERANCE_MICROS`].
fn uncovered_unplaced_stretches(
    chunks: &[ChunkSegments],
    kept: &[(usize, AsrSegmentDraft)],
) -> Vec<TimeRange> {
    let mut covered = SpanSet::default();
    for (_, segment) in kept {
        covered.insert(span(segment.range));
    }
    let candidates = merged(
        chunks
            .iter()
            .flat_map(|chunk| chunk.unplaced.iter().copied().map(span))
            .collect(),
    );
    let stretches: Vec<Span> = candidates
        .into_iter()
        .flat_map(|candidate| covered.gaps_within(candidate))
        .filter(|&(start, end)| end - start > SEAM_COVERAGE_TOLERANCE_MICROS)
        .collect();
    to_ranges(&join_nearest(stretches, MAX_UNPLACED_STRETCHES, None))
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

/// Segments of chunk `neighbour` spanning the midpoint of `segment` and not
/// cut at the neighbour's edge that faces it.
///
/// Spanning the midpoint, not merely overlapping, is what makes a neighbour
/// segment a copy of the same speech: a segment that ends just inside the
/// cut one is the sentence before it, and treating it as a copy would drop
/// the cut sentence from both chunks (found with the `base_q5_1` profile in
/// P07 increment 3c).
fn covering(
    chunks: &[ChunkSegments],
    neighbour: usize,
    segment: &AsrSegmentDraft,
    facing: Side,
) -> Vec<(usize, usize)> {
    let middle = segment.midpoint();
    chunks[neighbour]
        .segments
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate.range.start().as_micros() <= middle
                && candidate.range.end().as_micros() > middle
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

#[cfg(test)]
mod previous_merge;
