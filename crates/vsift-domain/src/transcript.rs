//! Transcript evidence on the normalized source timeline.
//!
//! A transcript revision is an immutable, identified set of timestamped
//! segments for one source segment (ADR 0016 decision 4). This module owns the
//! business rules that decide which text becomes citable evidence:
//!
//! - cue order and overlap policy for imported sidecars;
//! - the one implementation of offset alignment (C-10);
//! - the rule that nothing outside the probed source timeline is imported, and
//!   nothing is clamped or shifted to make it fit (T-02);
//! - the rule that imported and unscored text never carries a manufactured
//!   confidence or speaker identity;
//! - the rule that every segment's source time is re-derivable from its own
//!   origin: an imported cue's timing plus the offset, or a local-ASR chunk's
//!   decoded start plus the provider's chunk-relative times.
//!
//! Byte parsing belongs to infrastructure adapters; they hand this module
//! validated values and let it decide. The local-ASR rules for chunks,
//! provider output and seams live in [`crate::asr`].

use std::{collections::HashSet, error::Error, fmt, num::NonZeroU32};

use crate::{
    AsrChunkOutcome, AsrRun, ChunkTime, Confidence, ConfidenceOrigin, MediaTime, PageLimit,
    SourceId, SourceSegmentId, SpeakerLabel, TimeRange, TranscriptRevisionId, TranscriptSegmentId,
};

/// Largest supplied transcript file accepted for import.
pub const MAX_SUPPLIED_TRANSCRIPT_BYTES: u64 = 8 * 1024 * 1024;
/// Largest number of timed cues, including skipped empty cues, in one import.
pub const MAX_TRANSCRIPT_CUES: usize = 20_000;
/// Largest cue payload, before or after markup removal, in UTF-8 bytes.
pub const MAX_CUE_TEXT_BYTES: usize = 4_096;
/// Largest magnitude of an explicit transcript offset: twenty-four hours.
pub const MAX_TRANSCRIPT_OFFSET_MICROS: u64 = 86_400_000_000;
const MAX_LANGUAGE_TAG_BYTES: usize = 35;
const SHA256_HEX_LENGTH: usize = 64;

/// Sidecar syntax a supplied transcript was written in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptFormat {
    /// `SubRip` (`.srt`) cues.
    Srt,
    /// W3C `WebVTT` (`.vtt`) cues.
    WebVtt,
}

impl TranscriptFormat {
    /// Stable machine-readable format identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::WebVtt => "webvtt",
        }
    }

    /// Alignment origin of cues imported from this format.
    #[must_use]
    pub const fn alignment_origin(self) -> AlignmentOrigin {
        match self {
            Self::Srt => AlignmentOrigin::ImportedSrt,
            Self::WebVtt => AlignmentOrigin::ImportedWebVtt,
        }
    }
}

/// How a transcript's timestamps were placed on the source timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignmentOrigin {
    /// Timestamps written in a supplied `SubRip` file, plus the explicit offset.
    ImportedSrt,
    /// Timestamps written in a supplied `WebVTT` file, plus the explicit offset.
    ImportedWebVtt,
    /// Timestamps reported by a local speech recognizer for one decoded audio
    /// chunk, plus that chunk's observed first decoded timestamp.
    LocalAsr,
}

impl AlignmentOrigin {
    /// Stable machine-readable origin identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::ImportedSrt => "imported_srt",
            Self::ImportedWebVtt => "imported_webvtt",
            Self::LocalAsr => "local_asr",
        }
    }

    /// Whether this origin can carry provider speaker labels.
    ///
    /// Only `WebVTT` has voice syntax; `SubRip` speaker prefixes are ordinary
    /// text and are never guessed into labels.
    #[must_use]
    pub const fn carries_speaker_labels(self) -> bool {
        matches!(self, Self::ImportedWebVtt)
    }
}

/// Explicit signed shift from a supplied file's timeline to the source timeline.
///
/// Supplied files often start their clock at a different point than the video.
/// The caller states the difference; `VSift` never guesses it, and records it
/// with every imported segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TranscriptOffset(i64);

impl TranscriptOffset {
    /// No shift: file time equals source time.
    pub const ZERO: Self = Self(0);

    /// Validates a signed offset in microseconds.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptRejection::OffsetOutOfRange`] beyond twenty-four hours.
    pub const fn from_micros(value: i64) -> Result<Self, TranscriptRejection> {
        if value.unsigned_abs() > MAX_TRANSCRIPT_OFFSET_MICROS {
            return Err(TranscriptRejection::OffsetOutOfRange);
        }
        Ok(Self(value))
    }

    /// Returns the signed offset in microseconds.
    #[must_use]
    pub const fn as_micros(self) -> i64 {
        self.0
    }

    /// The single conversion from file time to signed source time.
    ///
    /// Widening to `i128` makes the sum exact for every `u64` file time and
    /// every accepted offset, so no overflow branch can silently clamp.
    #[must_use]
    pub fn shift(self, file_micros: u64) -> i128 {
        i128::from(file_micros) + i128::from(self.0)
    }
}

/// A positive `[start, end)` interval on a supplied file's own timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CueTiming {
    start: u64,
    end: u64,
}

impl CueTiming {
    /// Creates cue timing written in the sidecar.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptRejection::NonPositiveDuration`] unless `end > start`.
    pub const fn new(start_micros: u64, end_micros: u64) -> Result<Self, TranscriptRejection> {
        if end_micros <= start_micros {
            return Err(TranscriptRejection::NonPositiveDuration);
        }
        Ok(Self {
            start: start_micros,
            end: end_micros,
        })
    }

    /// Inclusive start on the file timeline, in microseconds.
    #[must_use]
    pub const fn start_micros(self) -> u64 {
        self.start
    }

    /// Exclusive end on the file timeline, in microseconds.
    #[must_use]
    pub const fn end_micros(self) -> u64 {
        self.end
    }
}

/// Whether cue text differed from its original payload after markup removal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CueMarkup {
    /// The payload contained no recognised markup; text is the original.
    None,
    /// Recognised tags or character references were removed or decoded.
    Removed,
}

impl CueMarkup {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Removed => "removed",
        }
    }
}

/// Plain cue text plus, when it differs, the payload exactly as supplied.
///
/// Lines are joined with `\n`. Both forms are bounded and free of control
/// characters other than the line separator (and tab in the original), so
/// neither can forge records or drive a terminal once presented.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CueText {
    text: String,
    original: Option<String>,
}

impl CueText {
    /// Validates plain text and the original payload it was derived from.
    ///
    /// # Errors
    ///
    /// Rejects empty text, oversized forms and control characters.
    pub fn new(text: String, original: String) -> Result<Self, TranscriptRejection> {
        if text.trim().is_empty() {
            return Err(TranscriptRejection::UntimedText);
        }
        if text.len() > MAX_CUE_TEXT_BYTES || original.len() > MAX_CUE_TEXT_BYTES {
            return Err(TranscriptRejection::CueTextTooLong);
        }
        if text
            .chars()
            .any(|character| character.is_control() && character != '\n')
            || original
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
        {
            return Err(TranscriptRejection::ControlCharacter);
        }
        let original = (original != text).then_some(original);
        Ok(Self { text, original })
    }

    /// Plain text with recognised markup removed.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The payload as supplied, present only when it differs from [`Self::text`].
    #[must_use]
    pub fn original(&self) -> Option<&str> {
        self.original.as_deref()
    }

    /// Whether markup was removed.
    #[must_use]
    pub const fn markup(&self) -> CueMarkup {
        if self.original.is_some() {
            CueMarkup::Removed
        } else {
            CueMarkup::None
        }
    }
}

/// Where a cue was written in the supplied file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CueSource {
    ordinal: NonZeroU32,
    line: NonZeroU32,
}

impl CueSource {
    /// Records a 1-based timed-cue ordinal and the 1-based line of its timing.
    #[must_use]
    pub const fn new(ordinal: NonZeroU32, line: NonZeroU32) -> Self {
        Self { ordinal, line }
    }

    /// 1-based position among every timed cue in the file, skipped ones included.
    #[must_use]
    pub const fn ordinal(self) -> u32 {
        self.ordinal.get()
    }

    /// 1-based line number of the cue's timing line.
    #[must_use]
    pub const fn line(self) -> u32 {
        self.line.get()
    }
}

/// One syntactically valid timed cue read from a supplied file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportedCue {
    /// Where the cue was written.
    pub source: CueSource,
    /// Timing as written, before the offset.
    pub timing: CueTiming,
    /// Validated text.
    pub text: CueText,
    /// Provider voice label, only from formats that have voice syntax.
    pub speaker: Option<SpeakerLabel>,
}

/// A bounded BCP 47-shaped language tag declared by a transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguageTag(String);

impl LanguageTag {
    /// Accepts letters, digits and single inner hyphens, starting with a letter.
    ///
    /// # Errors
    ///
    /// Returns [`LanguageTagError`] for any other shape.
    pub fn parse(value: impl Into<String>) -> Result<Self, LanguageTagError> {
        let value = value.into();
        let bytes = value.as_bytes();
        let shaped = !bytes.is_empty()
            && bytes.len() <= MAX_LANGUAGE_TAG_BYTES
            && bytes[0].is_ascii_alphabetic()
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
            && !value.ends_with('-')
            && !value.contains("--");
        if !shaped {
            return Err(LanguageTagError);
        }
        Ok(Self(value))
    }

    /// Returns the tag as declared.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A declared language tag was not a bounded BCP 47-shaped value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LanguageTagError;

impl fmt::Display for LanguageTagError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("language tag is not a bounded BCP 47-shaped value")
    }
}

impl Error for LanguageTagError {}

/// A condition that did not stop an import but changed what was imported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptWarningKind {
    /// Timed cues with no text were not imported.
    EmptyCuesSkipped,
    /// Recognised markup was removed from cue text; the original is kept.
    MarkupRemoved,
    /// A voice label was invalid or ambiguous and was not attached.
    SpeakerLabelDiscarded,
    /// Cues overlap in time; each keeps its own timing.
    OverlappingCues,
    /// Cues lie entirely outside the source timeline and were not imported.
    CuesOutsideSource,
    /// Cues cross the start or end of the source and were not imported.
    CuesCrossingSourceBoundary,
    /// Local ASR: provider segments with an empty or reversed range, or a range
    /// outside their chunk's decoded audio or the source, were not used.
    ProviderSegmentsRejected,
    /// Local ASR: provider segments ending less than one second past their
    /// chunk's decoded audio were cut at the audio end; raw times are kept.
    ProviderEndTrimmed,
    /// Local ASR: whole-segment non-speech markers such as `[BLANK_AUDIO]`, or
    /// segments with no text, were removed.
    NonSpeechMarkersRemoved,
    /// Local ASR: text repeated by both chunks of an overlap was kept once.
    SeamDuplicatesRemoved,
    /// Local ASR: chunks with no audible signal were not transcribed and are
    /// recorded as silent gaps.
    SilentChunksSkipped,
}

impl TranscriptWarningKind {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::EmptyCuesSkipped => "empty_cues_skipped",
            Self::MarkupRemoved => "markup_removed",
            Self::SpeakerLabelDiscarded => "speaker_label_discarded",
            Self::OverlappingCues => "overlapping_cues",
            Self::CuesOutsideSource => "cues_outside_source",
            Self::CuesCrossingSourceBoundary => "cues_crossing_source_boundary",
            Self::ProviderSegmentsRejected => "provider_segments_rejected",
            Self::ProviderEndTrimmed => "provider_end_trimmed",
            Self::NonSpeechMarkersRemoved => "non_speech_markers_removed",
            Self::SeamDuplicatesRemoved => "seam_duplicates_removed",
            Self::SilentChunksSkipped => "silent_chunks_skipped",
        }
    }

    /// Whether cues or provider segments of this kind were left out of the revision.
    #[must_use]
    pub const fn excludes_cues(self) -> bool {
        matches!(
            self,
            Self::EmptyCuesSkipped
                | Self::CuesOutsideSource
                | Self::CuesCrossingSourceBoundary
                | Self::ProviderSegmentsRejected
                | Self::NonSpeechMarkersRemoved
                | Self::SeamDuplicatesRemoved
                | Self::SilentChunksSkipped
        )
    }

    /// Whether this kind can only arise from local ASR.
    ///
    /// Imported revisions never carry these kinds, which is what keeps their
    /// stored record and public presentation unchanged.
    #[must_use]
    pub const fn is_local_asr(self) -> bool {
        matches!(
            self,
            Self::ProviderSegmentsRejected
                | Self::ProviderEndTrimmed
                | Self::NonSpeechMarkersRemoved
                | Self::SeamDuplicatesRemoved
                | Self::SilentChunksSkipped
        )
    }
}

/// How often one warning occurred and where it first occurred.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TranscriptWarning {
    kind: TranscriptWarningKind,
    count: u32,
    first_cue: u32,
}

impl TranscriptWarning {
    /// Reconstructs a stored warning.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptRevisionError::InvalidWarning`] for a zero count or cue.
    pub const fn new(
        kind: TranscriptWarningKind,
        count: u32,
        first_cue: u32,
    ) -> Result<Self, TranscriptRevisionError> {
        if count == 0 || first_cue == 0 {
            return Err(TranscriptRevisionError::InvalidWarning);
        }
        Ok(Self {
            kind,
            count,
            first_cue,
        })
    }

    /// What happened.
    #[must_use]
    pub const fn kind(self) -> TranscriptWarningKind {
        self.kind
    }

    /// How many cues it affected.
    #[must_use]
    pub const fn count(self) -> u32 {
        self.count
    }

    /// Ordinal of the first affected cue; for local-ASR kinds, the 1-based
    /// ordinal of the first affected chunk.
    #[must_use]
    pub const fn first_cue(self) -> u32 {
        self.first_cue
    }
}

/// Ordered, de-duplicated warnings: one entry per kind, in first-seen order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TranscriptWarnings(Vec<TranscriptWarning>);

impl TranscriptWarnings {
    /// Counts one occurrence of `kind` at cue `ordinal`.
    pub fn record(&mut self, kind: TranscriptWarningKind, ordinal: CueSource) {
        self.add(kind, 1, ordinal.ordinal);
    }

    /// Counts `count` occurrences of `kind`, first seen at 1-based position
    /// `first` (a cue ordinal, or a chunk ordinal for local-ASR kinds).
    ///
    /// A zero count records nothing, so callers can add tallies unconditionally.
    pub fn add(&mut self, kind: TranscriptWarningKind, count: u32, first: NonZeroU32) {
        if count == 0 {
            return;
        }
        if let Some(existing) = self.0.iter_mut().find(|warning| warning.kind == kind) {
            existing.count = existing.count.saturating_add(count);
        } else {
            self.0.push(TranscriptWarning {
                kind,
                count,
                first_cue: first.get(),
            });
        }
    }

    /// Adds every warning of `other`, keeping this set's first-seen order.
    pub fn extend(&mut self, other: &Self) {
        for warning in &other.0 {
            if let Some(first) = NonZeroU32::new(warning.first_cue) {
                self.add(warning.kind, warning.count, first);
            }
        }
    }

    /// Rebuilds stored warnings, rejecting a repeated kind.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptRevisionError::InvalidWarning`] for a repeated kind.
    pub fn from_stored(
        warnings: impl IntoIterator<Item = TranscriptWarning>,
    ) -> Result<Self, TranscriptRevisionError> {
        let mut collected: Vec<TranscriptWarning> = Vec::new();
        for warning in warnings {
            if collected
                .iter()
                .any(|existing| existing.kind == warning.kind)
            {
                return Err(TranscriptRevisionError::InvalidWarning);
            }
            collected.push(warning);
        }
        Ok(Self(collected))
    }

    /// Warnings in first-seen order.
    #[must_use]
    pub fn as_slice(&self) -> &[TranscriptWarning] {
        &self.0
    }
}

/// A complete, ordered, syntactically valid supplied transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedTranscript {
    format: TranscriptFormat,
    language: Option<LanguageTag>,
    cues: Vec<ImportedCue>,
    warnings: TranscriptWarnings,
}

impl ParsedTranscript {
    /// Applies the order and overlap policy to cues produced by a parser.
    ///
    /// Cues must be in non-decreasing start order, as `WebVTT` requires; a cue
    /// starting before its predecessor rejects the file rather than being
    /// re-sorted, because a re-ordered file is more likely damaged than
    /// intentional. Overlapping cues are legitimate (several voices) and are
    /// kept with a warning.
    ///
    /// # Errors
    ///
    /// Rejects an import with no text cues, too many cues, or out-of-order cues.
    pub fn new(
        format: TranscriptFormat,
        language: Option<LanguageTag>,
        cues: Vec<ImportedCue>,
        parse_warnings: TranscriptWarnings,
    ) -> Result<Self, TranscriptImportError> {
        if cues.is_empty() {
            return Err(TranscriptImportError::new(TranscriptRejection::NoCues));
        }
        if cues.len() > MAX_TRANSCRIPT_CUES {
            return Err(TranscriptImportError::new(TranscriptRejection::TooManyCues));
        }
        let mut warnings = parse_warnings;
        let mut previous_start = 0_u64;
        let mut latest_end = 0_u64;
        for cue in &cues {
            if cue.timing.start_micros() < previous_start {
                return Err(TranscriptImportError::at_line(
                    TranscriptRejection::OutOfOrder,
                    cue.source.line(),
                ));
            }
            if cue.timing.start_micros() < latest_end {
                warnings.record(TranscriptWarningKind::OverlappingCues, cue.source);
            }
            previous_start = cue.timing.start_micros();
            latest_end = latest_end.max(cue.timing.end_micros());
        }
        Ok(Self {
            format,
            language,
            cues,
            warnings,
        })
    }

    /// Sidecar syntax.
    #[must_use]
    pub const fn format(&self) -> TranscriptFormat {
        self.format
    }

    /// Declared language, if any.
    #[must_use]
    pub const fn language(&self) -> Option<&LanguageTag> {
        self.language.as_ref()
    }

    /// Imported cues in file order.
    #[must_use]
    pub fn cues(&self) -> &[ImportedCue] {
        &self.cues
    }

    /// Parse and order warnings.
    #[must_use]
    pub const fn warnings(&self) -> &TranscriptWarnings {
        &self.warnings
    }
}

/// Lifecycle of one source segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceSegmentState {
    /// Complete and immutable; a finite file is one closed segment.
    Closed,
}

impl SourceSegmentState {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Closed => "closed",
        }
    }
}

/// One ordered, identified segment of a source stream (ADR 0016 decision 4).
///
/// A finite file is a single closed segment covering its probed duration.
/// Live capture will add further segments without changing this contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSegment {
    id: SourceSegmentId,
    index: u32,
    range: TimeRange,
    state: SourceSegmentState,
}

impl SourceSegment {
    /// Describes a finite file as its only, closed segment `[0, duration)`.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptRevisionError::EmptySourceSegment`] for a zero duration.
    pub fn whole_file(
        id: SourceSegmentId,
        duration: MediaTime,
    ) -> Result<Self, TranscriptRevisionError> {
        let range = TimeRange::new(MediaTime::from_micros(0), duration)
            .map_err(|_| TranscriptRevisionError::EmptySourceSegment)?;
        Ok(Self {
            id,
            index: 0,
            range,
            state: SourceSegmentState::Closed,
        })
    }

    /// Segment identity.
    #[must_use]
    pub const fn id(&self) -> &SourceSegmentId {
        &self.id
    }

    /// Zero-based position in the source stream.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Covered source time.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// Lifecycle state.
    #[must_use]
    pub const fn state(&self) -> SourceSegmentState {
        self.state
    }
}

/// An imported cue placed on the source timeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignedCue {
    /// The cue as supplied.
    pub cue: ImportedCue,
    /// Its source-timeline range after the offset.
    pub range: TimeRange,
}

/// Places supplied cues on one source segment's timeline (T-02 policy).
///
/// Only cues that lie wholly inside the segment after the explicit offset are
/// imported. A cue entirely outside it, or crossing its start or end, is left
/// out and counted in a warning; it is never clamped, trimmed or shifted,
/// because any of those would cite text at a time it was not written for.
///
/// # Errors
///
/// Returns [`TranscriptRejection::NoCuesWithinSource`] when nothing remains,
/// which usually means the offset or the source is wrong.
pub fn align_imported_cues(
    parsed: &ParsedTranscript,
    offset: TranscriptOffset,
    source: &SourceSegment,
) -> Result<(Vec<AlignedCue>, TranscriptWarnings), TranscriptImportError> {
    let mut warnings = parsed.warnings.clone();
    let segment_start = i128::from(source.range.start().as_micros());
    let segment_end = i128::from(source.range.end().as_micros());
    let mut aligned = Vec::with_capacity(parsed.cues.len());
    for cue in &parsed.cues {
        let start = offset.shift(cue.timing.start_micros());
        let end = offset.shift(cue.timing.end_micros());
        if end <= segment_start || start >= segment_end {
            warnings.record(TranscriptWarningKind::CuesOutsideSource, cue.source);
            continue;
        }
        if start < segment_start || end > segment_end {
            warnings.record(
                TranscriptWarningKind::CuesCrossingSourceBoundary,
                cue.source,
            );
            continue;
        }
        let range = source_range(start, end).ok_or_else(|| {
            TranscriptImportError::at_line(TranscriptRejection::InvalidTimestamp, cue.source.line())
        })?;
        aligned.push(AlignedCue {
            cue: cue.clone(),
            range,
        });
    }
    if aligned.is_empty() {
        return Err(TranscriptImportError::new(
            TranscriptRejection::NoCuesWithinSource,
        ));
    }
    Ok((aligned, warnings))
}

fn source_range(start: i128, end: i128) -> Option<TimeRange> {
    let start = MediaTime::from_micros(u64::try_from(start).ok()?);
    let end = MediaTime::from_micros(u64::try_from(end).ok()?);
    TimeRange::new(start, end).ok()
}

/// Identity of the supplied file a revision was imported from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidecarIdentity {
    sha256: String,
    bytes: u64,
}

impl SidecarIdentity {
    /// Records the exact bytes that were parsed.
    ///
    /// # Errors
    ///
    /// Returns [`TranscriptRevisionError::InvalidSidecar`] for a non-canonical
    /// digest or an empty or oversized file.
    pub fn new(sha256: impl Into<String>, bytes: u64) -> Result<Self, TranscriptRevisionError> {
        let sha256 = sha256.into();
        if sha256.len() != SHA256_HEX_LENGTH
            || !sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || bytes == 0
            || bytes > MAX_SUPPLIED_TRANSCRIPT_BYTES
        {
            return Err(TranscriptRevisionError::InvalidSidecar);
        }
        Ok(Self { sha256, bytes })
    }

    /// Lowercase SHA-256 of the supplied bytes.
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Size of the supplied file.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// Whether a local-ASR segment's end was cut at its chunk's decoded audio end.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderEndTrim {
    /// The source range ends exactly at the provider's reported end.
    Unchanged,
    /// The provider reported an end less than one second past the decoded
    /// audio; the source range ends at the audio end and the raw provider end
    /// is kept beside it.
    TrimmedToAudioEnd,
}

/// Where one segment's text and timing came from.
///
/// Each variant carries what is needed to re-derive the segment's source range,
/// so a stored segment can be checked against the rule that produced it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentOrigin {
    /// A cue written in a supplied sidecar.
    ImportedCue {
        /// Where the cue was written in the supplied file.
        cue: CueSource,
        /// Timing as written, before the offset.
        timing: CueTiming,
    },
    /// A segment reported by a local speech recognizer for one audio chunk.
    Asr {
        /// Zero-based index of the chunk in the revision's ASR run.
        chunk: u32,
        /// Provider start, relative to the chunk's first decoded sample.
        provider_start: ChunkTime,
        /// Provider end as reported, relative to the chunk's first decoded sample.
        provider_end: ChunkTime,
        /// Whether the end was cut at the chunk's decoded audio end.
        trimmed: ProviderEndTrim,
    },
}

/// Provenance of a whole revision: a supplied file, or one local ASR run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TranscriptProvenance {
    /// Cues imported from a supplied sidecar.
    Imported {
        /// Sidecar syntax.
        format: TranscriptFormat,
        /// Identity of the exact supplied bytes.
        sidecar: SidecarIdentity,
        /// Explicit offset applied to every cue.
        offset: TranscriptOffset,
    },
    /// Segments recognised locally from the source's own audio.
    LocalAsr(AsrRun),
}

impl TranscriptProvenance {
    /// How the revision's timestamps were placed on the source timeline.
    #[must_use]
    pub const fn alignment_origin(&self) -> AlignmentOrigin {
        match self {
            Self::Imported { format, .. } => format.alignment_origin(),
            Self::LocalAsr(_) => AlignmentOrigin::LocalAsr,
        }
    }
}

/// One citable, timestamped transcript segment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptSegment {
    id: TranscriptSegmentId,
    ordinal: NonZeroU32,
    range: TimeRange,
    text: CueText,
    speaker: Option<SpeakerLabel>,
    confidence: Confidence,
    origin: SegmentOrigin,
}

/// Every field of a [`TranscriptSegment`], validated together by its revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptSegmentParts {
    /// Segment identity.
    pub id: TranscriptSegmentId,
    /// 1-based position in the revision.
    pub ordinal: NonZeroU32,
    /// Source-timeline range.
    pub range: TimeRange,
    /// Text.
    pub text: CueText,
    /// Provider speaker label.
    pub speaker: Option<SpeakerLabel>,
    /// Provider confidence; unknown for imported text.
    pub confidence: Confidence,
    /// Where the text and timing came from.
    pub origin: SegmentOrigin,
}

impl TranscriptSegment {
    /// Assembles a segment; [`TranscriptRevision::new`] validates it in context.
    #[must_use]
    pub fn new(parts: TranscriptSegmentParts) -> Self {
        Self {
            id: parts.id,
            ordinal: parts.ordinal,
            range: parts.range,
            text: parts.text,
            speaker: parts.speaker,
            confidence: parts.confidence,
            origin: parts.origin,
        }
    }

    /// Segment identity.
    #[must_use]
    pub const fn id(&self) -> &TranscriptSegmentId {
        &self.id
    }

    /// 1-based position in its revision, in start order.
    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal.get()
    }

    /// Source-timeline range, half-open.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// Text.
    #[must_use]
    pub const fn text(&self) -> &CueText {
        &self.text
    }

    /// Provider speaker label; never a verified identity.
    #[must_use]
    pub const fn speaker(&self) -> Option<&SpeakerLabel> {
        self.speaker.as_ref()
    }

    /// Provider confidence with its origin.
    #[must_use]
    pub const fn confidence(&self) -> Confidence {
        self.confidence
    }

    /// Where the text and timing came from.
    #[must_use]
    pub const fn origin(&self) -> SegmentOrigin {
        self.origin
    }

    fn intersects(&self, window: TimeRange) -> bool {
        self.range.start() < window.end() && self.range.end() > window.start()
    }
}

/// Every field of a [`TranscriptRevision`], validated together.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptRevisionParts {
    /// Revision identity.
    pub id: TranscriptRevisionId,
    /// 1-based revision number within the session.
    pub number: NonZeroU32,
    /// Source the revision describes.
    pub source_id: SourceId,
    /// Source segment the revision covers.
    pub source_segment: SourceSegment,
    /// Where the revision's segments came from.
    pub provenance: TranscriptProvenance,
    /// The revision this one replaces, if any; imports never supersede.
    pub supersedes: Option<TranscriptRevisionId>,
    /// The source range whose text this revision replaced in the superseded
    /// revision; present only with `supersedes`.
    pub replaced_range: Option<TimeRange>,
    /// Declared or detected language, if known.
    pub language: Option<LanguageTag>,
    /// Segments in start order.
    pub segments: Vec<TranscriptSegment>,
    /// Import or recognition warnings.
    pub warnings: TranscriptWarnings,
}

/// One immutable transcript revision for one source segment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptRevision {
    id: TranscriptRevisionId,
    number: NonZeroU32,
    source_id: SourceId,
    source_segment: SourceSegment,
    provenance: TranscriptProvenance,
    supersedes: Option<TranscriptRevisionId>,
    replaced_range: Option<TimeRange>,
    language: Option<LanguageTag>,
    segments: Vec<TranscriptSegment>,
    warnings: TranscriptWarnings,
}

impl TranscriptRevision {
    /// Validates every cross-segment invariant of a revision.
    ///
    /// Each segment is checked against its own origin: an imported cue's range
    /// must be its written timing plus the revision offset, and a local-ASR
    /// segment's range must be its chunk's decoded start plus the provider's
    /// times (or the audio end when trimmed). Stored revisions are rebuilt
    /// through this constructor too, so a modified record cannot bypass the
    /// rules that produced it.
    ///
    /// # Errors
    ///
    /// Returns the first violated [`TranscriptRevisionError`].
    pub fn new(parts: TranscriptRevisionParts) -> Result<Self, TranscriptRevisionError> {
        if parts.segments.is_empty() {
            return Err(TranscriptRevisionError::Empty);
        }
        if parts.segments.len() > MAX_TRANSCRIPT_CUES {
            return Err(TranscriptRevisionError::TooManySegments);
        }
        let bounds = parts.source_segment.range();
        validate_supersession(&parts, bounds)?;
        if let TranscriptProvenance::LocalAsr(run) = &parts.provenance {
            run.validate_within(&parts.source_segment)?;
        }
        let origin = parts.provenance.alignment_origin();
        let mut identities = HashSet::with_capacity(parts.segments.len());
        let mut previous_start = bounds.start();
        for (index, segment) in parts.segments.iter().enumerate() {
            let expected =
                u32::try_from(index + 1).map_err(|_| TranscriptRevisionError::TooManySegments)?;
            if segment.ordinal() != expected {
                return Err(TranscriptRevisionError::OrdinalGap);
            }
            if segment.range.start() < previous_start {
                return Err(TranscriptRevisionError::OutOfOrder);
            }
            previous_start = segment.range.start();
            if segment.range.start() < bounds.start() || segment.range.end() > bounds.end() {
                return Err(TranscriptRevisionError::OutsideSourceSegment);
            }
            validate_segment_origin(segment, &parts.provenance)?;
            if segment.speaker.is_some() && !origin.carries_speaker_labels() {
                return Err(TranscriptRevisionError::UnsupportedSpeaker);
            }
            if !identities.insert(segment.id.clone()) {
                return Err(TranscriptRevisionError::DuplicateSegment);
            }
        }
        Ok(Self {
            id: parts.id,
            number: parts.number,
            source_id: parts.source_id,
            source_segment: parts.source_segment,
            provenance: parts.provenance,
            supersedes: parts.supersedes,
            replaced_range: parts.replaced_range,
            language: parts.language,
            segments: parts.segments,
            warnings: parts.warnings,
        })
    }

    /// Revision identity.
    #[must_use]
    pub const fn id(&self) -> &TranscriptRevisionId {
        &self.id
    }

    /// 1-based revision number within the session.
    #[must_use]
    pub const fn number(&self) -> u32 {
        self.number.get()
    }

    /// Source the revision describes.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Source segment the revision covers.
    #[must_use]
    pub const fn source_segment(&self) -> &SourceSegment {
        &self.source_segment
    }

    /// How timestamps were placed.
    #[must_use]
    pub const fn origin(&self) -> AlignmentOrigin {
        self.provenance.alignment_origin()
    }

    /// Where the revision's segments came from.
    #[must_use]
    pub const fn provenance(&self) -> &TranscriptProvenance {
        &self.provenance
    }

    /// The revision this one replaces, if any.
    #[must_use]
    pub const fn supersedes(&self) -> Option<&TranscriptRevisionId> {
        self.supersedes.as_ref()
    }

    /// The source range whose text this revision replaced, if any.
    #[must_use]
    pub const fn replaced_range(&self) -> Option<TimeRange> {
        self.replaced_range
    }

    /// Declared or detected language, if known.
    #[must_use]
    pub const fn language(&self) -> Option<&LanguageTag> {
        self.language.as_ref()
    }

    /// Segments in start order.
    #[must_use]
    pub fn segments(&self) -> &[TranscriptSegment] {
        &self.segments
    }

    /// Import warnings.
    #[must_use]
    pub const fn warnings(&self) -> &TranscriptWarnings {
        &self.warnings
    }

    /// Returns at most `limit` segments intersecting `window`, after segment
    /// ordinal `after` when continuing a previous page.
    ///
    /// A segment intersects `[from, to)` when it starts before `to` and ends
    /// after `from`, so a long segment that began earlier is still returned.
    /// Ordering is by ordinal (start order), which is fixed for the revision,
    /// so pages never skip or repeat a segment.
    #[must_use]
    pub fn page(
        &self,
        window: TimeRange,
        after: Option<u32>,
        limit: PageLimit,
    ) -> TranscriptSlice<'_> {
        let skip = after.map_or(0, |ordinal| usize::try_from(ordinal).unwrap_or(usize::MAX));
        let mut segments = Vec::new();
        let mut has_more = false;
        for segment in self.segments.iter().skip(skip) {
            if segment.range.start() >= window.end() {
                break;
            }
            if !segment.intersects(window) {
                continue;
            }
            if segments.len() == usize::from(limit.get()) {
                has_more = true;
                break;
            }
            segments.push(segment);
        }
        TranscriptSlice { segments, has_more }
    }
}

/// A superseding revision names what it replaces; imports replace nothing.
fn validate_supersession(
    parts: &TranscriptRevisionParts,
    bounds: TimeRange,
) -> Result<(), TranscriptRevisionError> {
    let invalid = match (&parts.supersedes, parts.replaced_range, &parts.provenance) {
        (Some(_), _, TranscriptProvenance::Imported { .. }) | (None, Some(_), _) => true,
        (Some(previous), replaced, TranscriptProvenance::LocalAsr(_)) => {
            *previous == parts.id
                || replaced.is_some_and(|range| {
                    range.start() < bounds.start() || range.end() > bounds.end()
                })
        }
        (None, None, _) => false,
    };
    if invalid {
        return Err(TranscriptRevisionError::InvalidSupersession);
    }
    Ok(())
}

/// Re-derives a segment's source range from its own origin.
fn validate_segment_origin(
    segment: &TranscriptSegment,
    provenance: &TranscriptProvenance,
) -> Result<(), TranscriptRevisionError> {
    match (segment.origin, provenance) {
        (
            SegmentOrigin::ImportedCue { timing, .. },
            TranscriptProvenance::Imported { offset, .. },
        ) => {
            if offset.shift(timing.start_micros()) != i128::from(segment.range.start().as_micros())
                || offset.shift(timing.end_micros()) != i128::from(segment.range.end().as_micros())
            {
                return Err(TranscriptRevisionError::AlignmentMismatch);
            }
            // A supplied file states no score, so any number would be invented.
            if segment.confidence != Confidence::unknown() {
                return Err(TranscriptRevisionError::ManufacturedConfidence);
            }
            Ok(())
        }
        (
            SegmentOrigin::Asr {
                chunk,
                provider_start,
                provider_end,
                trimmed,
            },
            TranscriptProvenance::LocalAsr(run),
        ) => {
            let record = run
                .chunks()
                .get(usize::try_from(chunk).map_err(|_| TranscriptRevisionError::OriginMismatch)?)
                .ok_or(TranscriptRevisionError::OriginMismatch)?;
            let AsrChunkOutcome::Transcribed { audio } = record.outcome() else {
                return Err(TranscriptRevisionError::OriginMismatch);
            };
            let expected =
                crate::asr::asr_source_range(audio, provider_start, provider_end, trimmed)
                    .ok_or(TranscriptRevisionError::AlignmentMismatch)?;
            if expected != segment.range {
                return Err(TranscriptRevisionError::AlignmentMismatch);
            }
            // whisper.cpp token probabilities are not calibrated; claiming
            // calibration would overstate what the provider measured.
            if segment.confidence.origin() == ConfidenceOrigin::ProviderCalibrated {
                return Err(TranscriptRevisionError::ManufacturedConfidence);
            }
            Ok(())
        }
        (SegmentOrigin::ImportedCue { .. }, TranscriptProvenance::LocalAsr(_))
        | (SegmentOrigin::Asr { .. }, TranscriptProvenance::Imported { .. }) => {
            Err(TranscriptRevisionError::OriginMismatch)
        }
    }
}

/// One bounded page of a revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptSlice<'a> {
    /// Segments on the page, in ordinal order.
    pub segments: Vec<&'a TranscriptSegment>,
    /// Whether a further intersecting segment exists after the last one.
    pub has_more: bool,
}

/// Why a supplied transcript was rejected before anything was imported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptRejection {
    /// The file exceeds the supplied-transcript byte limit.
    TooLarge,
    /// The file is UTF-16 or UTF-32; only UTF-8 is accepted.
    UnsupportedEncoding,
    /// The file is not valid UTF-8.
    InvalidUtf8,
    /// A line contains a control character other than tab.
    ControlCharacter,
    /// A line exceeds the line-length limit.
    LineTooLong,
    /// The file has more timed cues than the cue limit.
    TooManyCues,
    /// A cue's text exceeds the cue-text limit.
    CueTextTooLong,
    /// A `WebVTT` signature line is malformed.
    InvalidHeader,
    /// A `SubRip` block does not start with a positive cue number.
    InvalidCueNumber,
    /// A timing line is not `start --> end` in the format's syntax.
    InvalidTimingLine,
    /// A timestamp component is malformed or out of range.
    InvalidTimestamp,
    /// A cue ends at or before its start.
    NonPositiveDuration,
    /// A cue starts before the cue preceding it.
    OutOfOrder,
    /// Text appears without timing, so it cannot support a citation.
    UntimedText,
    /// The file contains no timed cue with text.
    NoCues,
    /// The explicit offset exceeds twenty-four hours.
    OffsetOutOfRange,
    /// No cue lies wholly inside the source timeline after the offset.
    NoCuesWithinSource,
}

impl TranscriptRejection {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::TooLarge => "too_large",
            Self::UnsupportedEncoding => "unsupported_encoding",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::ControlCharacter => "control_character",
            Self::LineTooLong => "line_too_long",
            Self::TooManyCues => "too_many_cues",
            Self::CueTextTooLong => "cue_text_too_long",
            Self::InvalidHeader => "invalid_header",
            Self::InvalidCueNumber => "invalid_cue_number",
            Self::InvalidTimingLine => "invalid_timing_line",
            Self::InvalidTimestamp => "invalid_timestamp",
            Self::NonPositiveDuration => "non_positive_duration",
            Self::OutOfOrder => "out_of_order",
            Self::UntimedText => "untimed_text",
            Self::NoCues => "no_cues",
            Self::OffsetOutOfRange => "offset_out_of_range",
            Self::NoCuesWithinSource => "no_cues_within_source",
        }
    }

    /// Whether a size or count budget, rather than malformed data, stopped the import.
    #[must_use]
    pub const fn is_resource_limit(self) -> bool {
        matches!(
            self,
            Self::TooLarge | Self::LineTooLong | Self::TooManyCues | Self::CueTextTooLong
        )
    }

    /// Whether the caller's alignment request, rather than the file, is at fault.
    #[must_use]
    pub const fn is_alignment(self) -> bool {
        matches!(self, Self::OffsetOutOfRange | Self::NoCuesWithinSource)
    }
}

impl fmt::Display for TranscriptRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.identifier())
    }
}

impl Error for TranscriptRejection {}

/// A rejected import and, when known, the 1-based line that caused it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TranscriptImportError {
    rejection: TranscriptRejection,
    line: Option<u32>,
}

impl TranscriptImportError {
    /// A rejection that concerns the file or request as a whole.
    #[must_use]
    pub const fn new(rejection: TranscriptRejection) -> Self {
        Self {
            rejection,
            line: None,
        }
    }

    /// A rejection located at one line.
    #[must_use]
    pub const fn at_line(rejection: TranscriptRejection, line: u32) -> Self {
        Self {
            rejection,
            line: Some(line),
        }
    }

    /// Why the import was rejected.
    #[must_use]
    pub const fn rejection(self) -> TranscriptRejection {
        self.rejection
    }

    /// 1-based line, when the rejection has one.
    #[must_use]
    pub const fn line(self) -> Option<u32> {
        self.line
    }
}

impl From<TranscriptRejection> for TranscriptImportError {
    fn from(value: TranscriptRejection) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for TranscriptImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(
                formatter,
                "supplied transcript rejected ({}) at line {line}",
                self.rejection
            ),
            None => write!(
                formatter,
                "supplied transcript rejected ({})",
                self.rejection
            ),
        }
    }
}

impl Error for TranscriptImportError {}

/// Why a transcript revision violated its invariants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptRevisionError {
    /// A revision must contain at least one segment.
    Empty,
    /// The revision exceeds the segment limit.
    TooManySegments,
    /// Segment ordinals are not `1..=n` in order.
    OrdinalGap,
    /// Segments are not in start order.
    OutOfOrder,
    /// A segment lies outside its source segment.
    OutsideSourceSegment,
    /// A segment's range is not the one its origin determines: cue timing plus
    /// the revision offset, or chunk decoded start plus provider times.
    AlignmentMismatch,
    /// An imported segment carries a numeric confidence it was never given, or
    /// a local-ASR segment claims a calibrated one.
    ManufacturedConfidence,
    /// A speaker label is attached to an origin without voice syntax.
    UnsupportedSpeaker,
    /// Two segments share an identity.
    DuplicateSegment,
    /// A source segment has no duration.
    EmptySourceSegment,
    /// The supplied-file identity is malformed.
    InvalidSidecar,
    /// A warning has a zero count or cue, or repeats a kind.
    InvalidWarning,
    /// A derived revision, segment or source-segment identity is not canonical.
    InvalidIdentity,
    /// A segment's origin does not belong to the revision's provenance, or
    /// names a chunk that was not transcribed.
    OriginMismatch,
    /// Superseding metadata is inconsistent: a replaced range without a
    /// superseded revision, a revision superseding itself, a range outside
    /// the source segment, or an import that claims to supersede.
    InvalidSupersession,
    /// A local-ASR run's chunk records do not follow its chunk plan.
    InvalidAsrRun,
}

impl fmt::Display for TranscriptRevisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "transcript revision has no segments",
            Self::TooManySegments => "transcript revision exceeds the segment limit",
            Self::OrdinalGap => "transcript segment ordinals are not contiguous",
            Self::OutOfOrder => "transcript segments are not in start order",
            Self::OutsideSourceSegment => "transcript segment lies outside its source segment",
            Self::AlignmentMismatch => "transcript segment does not match its origin's timing",
            Self::ManufacturedConfidence => {
                "transcript segment claims a confidence its origin cannot give"
            }
            Self::UnsupportedSpeaker => "transcript origin cannot carry speaker labels",
            Self::DuplicateSegment => "transcript segment identity is repeated",
            Self::EmptySourceSegment => "source segment has no duration",
            Self::InvalidSidecar => "supplied transcript identity is invalid",
            Self::InvalidWarning => "transcript warning is invalid",
            Self::InvalidIdentity => "transcript identity is not canonical",
            Self::OriginMismatch => "transcript segment origin does not match its revision",
            Self::InvalidSupersession => "transcript supersession metadata is inconsistent",
            Self::InvalidAsrRun => "local ASR run does not follow its chunk plan",
        })
    }
}

impl Error for TranscriptRevisionError {}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use proptest::prelude::{prop_assert, prop_assert_eq, proptest};

    use super::{
        CueMarkup, CueSource, CueText, CueTiming, ImportedCue, LanguageTag,
        MAX_TRANSCRIPT_OFFSET_MICROS, ParsedTranscript, SegmentOrigin, SidecarIdentity,
        SourceSegment, TranscriptFormat, TranscriptImportError, TranscriptOffset,
        TranscriptProvenance, TranscriptRejection, TranscriptRevision, TranscriptRevisionError,
        TranscriptRevisionParts, TranscriptSegment, TranscriptSegmentParts, TranscriptWarningKind,
        TranscriptWarnings, align_imported_cues,
    };
    use crate::{
        Confidence, ConfidenceOrigin, MediaTime, PageLimit, SourceId, SourceSegmentId,
        SpeakerLabel, TimeRange, TranscriptRevisionId, TranscriptSegmentId,
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn position(ordinal: u32, line: u32) -> Result<CueSource, Box<dyn std::error::Error>> {
        Ok(CueSource::new(
            NonZeroU32::new(ordinal).ok_or("zero ordinal")?,
            NonZeroU32::new(line).ok_or("zero line")?,
        ))
    }

    fn cue(ordinal: u32, start: u64, end: u64) -> Result<ImportedCue, Box<dyn std::error::Error>> {
        Ok(ImportedCue {
            source: position(ordinal, ordinal * 4)?,
            timing: CueTiming::new(start, end)?,
            text: CueText::new(format!("cue {ordinal}"), format!("cue {ordinal}"))?,
            speaker: None,
        })
    }

    fn parsed(cues: Vec<ImportedCue>) -> Result<ParsedTranscript, TranscriptImportError> {
        ParsedTranscript::new(
            TranscriptFormat::Srt,
            None,
            cues,
            TranscriptWarnings::default(),
        )
    }

    fn twelve_second_source() -> Result<SourceSegment, Box<dyn std::error::Error>> {
        Ok(SourceSegment::whole_file(
            SourceSegmentId::parse("sgm_0123456789abcdef")?,
            MediaTime::from_micros(12_000_000),
        )?)
    }

    fn revision(
        offset: TranscriptOffset,
        ranges: &[(u64, u64)],
    ) -> Result<TranscriptRevision, Box<dyn std::error::Error>> {
        let mut segments = Vec::new();
        for (index, (start, end)) in ranges.iter().enumerate() {
            let ordinal = u32::try_from(index + 1)?;
            let shift = u64::try_from(offset.as_micros())?;
            segments.push(TranscriptSegment::new(TranscriptSegmentParts {
                id: TranscriptSegmentId::parse(format!("tsg_{ordinal:016x}"))?,
                ordinal: NonZeroU32::new(ordinal).ok_or("zero")?,
                range: TimeRange::new(
                    MediaTime::from_micros(*start),
                    MediaTime::from_micros(*end),
                )?,
                text: CueText::new(format!("segment {ordinal}"), format!("segment {ordinal}"))?,
                speaker: None,
                confidence: Confidence::unknown(),
                origin: SegmentOrigin::ImportedCue {
                    cue: position(ordinal, ordinal)?,
                    timing: CueTiming::new(start - shift, end - shift)?,
                },
            }));
        }
        Ok(TranscriptRevision::new(TranscriptRevisionParts {
            id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
            number: NonZeroU32::MIN,
            source_id: SourceId::from_sha256(DIGEST)?,
            source_segment: twelve_second_source()?,
            provenance: imported(TranscriptFormat::Srt, offset)?,
            supersedes: None,
            replaced_range: None,
            language: None,
            segments,
            warnings: TranscriptWarnings::default(),
        })?)
    }

    fn imported(
        format: TranscriptFormat,
        offset: TranscriptOffset,
    ) -> Result<TranscriptProvenance, Box<dyn std::error::Error>> {
        Ok(TranscriptProvenance::Imported {
            format,
            sidecar: SidecarIdentity::new(DIGEST, 10)?,
            offset,
        })
    }

    fn parts_of(revision: &TranscriptRevision) -> TranscriptRevisionParts {
        TranscriptRevisionParts {
            id: revision.id().clone(),
            number: NonZeroU32::MIN,
            source_id: revision.source_id().clone(),
            source_segment: revision.source_segment().clone(),
            provenance: revision.provenance().clone(),
            supersedes: None,
            replaced_range: None,
            language: None,
            segments: revision.segments().to_vec(),
            warnings: TranscriptWarnings::default(),
        }
    }

    fn with_segment(
        original: &TranscriptSegment,
        speaker: Option<SpeakerLabel>,
        confidence: Confidence,
        range: TimeRange,
    ) -> TranscriptSegment {
        TranscriptSegment::new(TranscriptSegmentParts {
            id: original.id().clone(),
            ordinal: NonZeroU32::MIN,
            range,
            text: original.text().clone(),
            speaker,
            confidence,
            origin: original.origin(),
        })
    }

    #[test]
    fn cue_timing_requires_a_positive_duration() {
        assert_eq!(
            CueTiming::new(5, 5),
            Err(TranscriptRejection::NonPositiveDuration)
        );
        assert_eq!(
            CueTiming::new(6, 5),
            Err(TranscriptRejection::NonPositiveDuration)
        );
        assert!(CueTiming::new(5, 6).is_ok());
    }

    #[test]
    fn offsets_are_bounded_and_shift_exactly() -> TestResult {
        let limit = i64::try_from(MAX_TRANSCRIPT_OFFSET_MICROS)?;
        assert!(TranscriptOffset::from_micros(limit).is_ok());
        assert!(TranscriptOffset::from_micros(-limit).is_ok());
        assert_eq!(
            TranscriptOffset::from_micros(limit + 1),
            Err(TranscriptRejection::OffsetOutOfRange)
        );
        assert_eq!(
            TranscriptOffset::from_micros(i64::MIN),
            Err(TranscriptRejection::OffsetOutOfRange)
        );
        let offset = TranscriptOffset::from_micros(-500_000)?;
        assert_eq!(offset.shift(4_500_000), 4_000_000);
        assert_eq!(offset.shift(0), -500_000);
        assert_eq!(
            TranscriptOffset::from_micros(limit)?.shift(u64::MAX),
            i128::from(u64::MAX) + i128::from(limit)
        );
        Ok(())
    }

    #[test]
    fn cue_text_keeps_the_original_only_when_markup_changed_it() -> TestResult {
        let plain = CueText::new("Dialog R-17".to_owned(), "Dialog R-17".to_owned())?;
        assert_eq!(plain.markup(), CueMarkup::None);
        assert_eq!(plain.original(), None);

        let marked = CueText::new("Dialog R-17".to_owned(), "<i>Dialog</i> R-17".to_owned())?;
        assert_eq!(marked.markup(), CueMarkup::Removed);
        assert_eq!(marked.original(), Some("<i>Dialog</i> R-17"));

        assert_eq!(
            CueText::new(" \n ".to_owned(), " ".to_owned()),
            Err(TranscriptRejection::UntimedText)
        );
        assert_eq!(
            CueText::new("a\u{1b}[31m".to_owned(), "a".to_owned()),
            Err(TranscriptRejection::ControlCharacter)
        );
        assert_eq!(
            CueText::new("a\u{9b}".to_owned(), "a".to_owned()),
            Err(TranscriptRejection::ControlCharacter)
        );
        assert_eq!(
            CueText::new("a".repeat(4_097), "a".to_owned()),
            Err(TranscriptRejection::CueTextTooLong)
        );
        Ok(())
    }

    #[test]
    fn language_tags_are_bounded_and_shaped() {
        assert!(LanguageTag::parse("en").is_ok());
        assert!(LanguageTag::parse("en-GB").is_ok());
        for invalid in [
            "",
            "1en",
            "en--gb",
            "en-",
            "en gb",
            "e\u{301}",
            &"a".repeat(36),
        ] {
            assert!(LanguageTag::parse(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn out_of_order_cues_are_rejected_and_overlaps_are_warned() -> TestResult {
        let rejected = parsed(vec![cue(1, 2_000, 3_000)?, cue(2, 1_000, 4_000)?]);
        assert_eq!(
            rejected,
            Err(TranscriptImportError::at_line(
                TranscriptRejection::OutOfOrder,
                8
            ))
        );

        let overlapping = parsed(vec![
            cue(1, 0, 5_000)?,
            cue(2, 1_000, 2_000)?,
            cue(3, 4_000, 6_000)?,
            cue(4, 6_000, 7_000)?,
        ])?;
        let warnings = overlapping.warnings().as_slice();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind(), TranscriptWarningKind::OverlappingCues);
        assert_eq!(warnings[0].count(), 2);
        assert_eq!(warnings[0].first_cue(), 2);

        assert_eq!(
            parsed(Vec::new()),
            Err(TranscriptImportError::new(TranscriptRejection::NoCues))
        );
        Ok(())
    }

    /// T-02: a positive offset places cues; nothing is clamped at either end.
    #[test]
    fn alignment_applies_the_offset_and_never_clamps() -> TestResult {
        let source = twelve_second_source()?;
        let transcript = parsed(vec![
            cue(1, 0, 400_000)?,
            cue(2, 300_000, 1_000_000)?,
            cue(3, 4_500_000, 8_500_000)?,
            cue(4, 11_000_000, 12_000_000)?,
            cue(5, 11_600_000, 13_000_000)?,
        ])?;

        let (aligned, warnings) = align_imported_cues(
            &transcript,
            TranscriptOffset::from_micros(-500_000)?,
            &source,
        )?;

        let ranges: Vec<(u64, u64)> = aligned
            .iter()
            .map(|cue| (cue.range.start().as_micros(), cue.range.end().as_micros()))
            .collect();
        // Cue 1 ends before zero, cue 2 crosses zero and cue 5 crosses the end:
        // all three are left out rather than clamped.
        assert_eq!(ranges, [(4_000_000, 8_000_000), (10_500_000, 11_500_000)]);
        let kinds: Vec<_> = warnings
            .as_slice()
            .iter()
            .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
            .collect();
        assert_eq!(
            kinds,
            [
                (TranscriptWarningKind::OverlappingCues, 2, 2),
                (TranscriptWarningKind::CuesOutsideSource, 1, 1),
                (TranscriptWarningKind::CuesCrossingSourceBoundary, 2, 2),
            ]
        );
        Ok(())
    }

    /// T-02: a transcript for a different recording aligns nothing and is rejected.
    #[test]
    fn a_transcript_that_misses_the_source_entirely_is_rejected() -> TestResult {
        let source = twelve_second_source()?;
        let transcript = parsed(vec![cue(1, 20_000_000, 21_000_000)?])?;

        assert_eq!(
            align_imported_cues(&transcript, TranscriptOffset::ZERO, &source),
            Err(TranscriptImportError::new(
                TranscriptRejection::NoCuesWithinSource
            ))
        );
        assert_eq!(
            align_imported_cues(
                &parsed(vec![cue(1, 0, 1_000_000)?])?,
                TranscriptOffset::from_micros(-1_000_000)?,
                &source
            ),
            Err(TranscriptImportError::new(
                TranscriptRejection::NoCuesWithinSource
            ))
        );
        Ok(())
    }

    /// C-10: imported segments never claim confidence, and SRT never claims a speaker.
    #[test]
    fn revisions_refuse_manufactured_certainty_and_identity() -> TestResult {
        let valid = revision(TranscriptOffset::ZERO, &[(0, 1_000), (500, 2_000)])?;
        assert_eq!(valid.segments()[0].confidence(), Confidence::unknown());
        assert_eq!(
            valid.segments()[0].confidence().origin(),
            ConfidenceOrigin::Unavailable
        );

        let mut parts = parts_of(&valid);
        let original = parts.segments[0].clone();
        for confidence in [
            Confidence::provider_score(10_000, ConfidenceOrigin::ProviderCalibrated)?,
            Confidence::provider_score(9_000, ConfidenceOrigin::ProviderUncalibrated)?,
        ] {
            parts.segments[0] = with_segment(&original, None, confidence, original.range());
            assert_eq!(
                TranscriptRevision::new(parts.clone()),
                Err(TranscriptRevisionError::ManufacturedConfidence)
            );
        }

        parts.segments[0] = with_segment(
            &original,
            Some(SpeakerLabel::parse("Narrator")?),
            Confidence::unknown(),
            original.range(),
        );
        assert_eq!(
            TranscriptRevision::new(parts.clone()),
            Err(TranscriptRevisionError::UnsupportedSpeaker)
        );
        parts.provenance = imported(TranscriptFormat::WebVtt, TranscriptOffset::ZERO)?;
        assert!(TranscriptRevision::new(parts).is_ok());
        Ok(())
    }

    /// C-10: a stored segment whose range is not its cue timing plus the offset is rejected.
    #[test]
    fn revisions_verify_the_single_offset_conversion() -> TestResult {
        let offset = TranscriptOffset::from_micros(500_000)?;
        let valid = revision(offset, &[(1_000_000, 2_000_000)])?;
        let segment = &valid.segments()[0];
        let SegmentOrigin::ImportedCue { timing, .. } = segment.origin() else {
            return Err("imported segment has another origin".into());
        };
        assert_eq!(timing.start_micros(), 500_000);

        let mut parts = parts_of(&valid);
        parts.segments = vec![with_segment(
            segment,
            None,
            Confidence::unknown(),
            TimeRange::new(
                MediaTime::from_micros(1_000_001),
                MediaTime::from_micros(2_000_000),
            )?,
        )];
        assert_eq!(
            TranscriptRevision::new(parts),
            Err(TranscriptRevisionError::AlignmentMismatch)
        );
        Ok(())
    }

    /// Imports never supersede, and a replaced range needs a superseded revision.
    #[test]
    fn imports_carry_no_supersession() -> TestResult {
        let valid = revision(TranscriptOffset::ZERO, &[(0, 1_000)])?;
        assert_eq!(valid.supersedes(), None);
        assert_eq!(valid.replaced_range(), None);
        let mut parts = parts_of(&valid);
        parts.supersedes = Some(TranscriptRevisionId::parse("trv_fedcba9876543210")?);
        assert_eq!(
            TranscriptRevision::new(parts.clone()),
            Err(TranscriptRevisionError::InvalidSupersession)
        );
        parts.supersedes = None;
        parts.replaced_range = Some(TimeRange::new(
            MediaTime::from_micros(0),
            MediaTime::from_micros(1_000),
        )?);
        assert_eq!(
            TranscriptRevision::new(parts),
            Err(TranscriptRevisionError::InvalidSupersession)
        );
        Ok(())
    }

    #[test]
    fn pages_return_intersecting_segments_without_gaps_or_repeats() -> TestResult {
        let revision = revision(
            TranscriptOffset::ZERO,
            &[
                (0, 10_000_000),
                (1_000_000, 2_000_000),
                (3_000_000, 4_000_000),
                (5_000_000, 6_000_000),
                (8_000_000, 9_000_000),
            ],
        )?;
        let window = TimeRange::new(
            MediaTime::from_micros(3_500_000),
            MediaTime::from_micros(8_000_000),
        )?;

        let first = revision.page(window, None, PageLimit::new(2)?);
        let ordinals: Vec<u32> = first
            .segments
            .iter()
            .map(|segment| segment.ordinal())
            .collect();
        assert_eq!(ordinals, [1, 3]);
        assert!(first.has_more);

        let second = revision.page(window, Some(3), PageLimit::new(2)?);
        let ordinals: Vec<u32> = second
            .segments
            .iter()
            .map(|segment| segment.ordinal())
            .collect();
        assert_eq!(ordinals, [4]);
        assert!(!second.has_more);

        let exact = revision.page(window, None, PageLimit::new(3)?);
        assert_eq!(exact.segments.len(), 3);
        assert!(!exact.has_more);
        Ok(())
    }

    proptest! {
        /// Aligned cues always satisfy the source bounds and the offset.
        #[test]
        fn aligned_cues_stay_inside_the_source(
            starts in proptest::collection::vec(0_u64..20_000_000, 1..40),
            length in 1_u64..3_000_000,
            offset in -15_000_000_i64..15_000_000,
        ) {
            let mut sorted = starts;
            sorted.sort_unstable();
            let mut cues = Vec::new();
            for (index, start) in sorted.iter().enumerate() {
                let ordinal = u32::try_from(index + 1).unwrap_or(u32::MAX);
                let built = cue(ordinal, *start, start + length);
                prop_assert!(built.is_ok());
                if let Ok(built) = built {
                    cues.push(built);
                }
            }
            let transcript = parsed(cues);
            prop_assert!(transcript.is_ok());
            let (Ok(transcript), Ok(source), Ok(offset)) = (
                transcript,
                twelve_second_source(),
                TranscriptOffset::from_micros(offset),
            ) else {
                return Ok(());
            };
            match align_imported_cues(&transcript, offset, &source) {
                Ok((aligned, warnings)) => {
                    let excluded: u32 = warnings
                        .as_slice()
                        .iter()
                        .filter(|warning| warning.kind().excludes_cues())
                        .map(|warning| warning.count())
                        .sum();
                    prop_assert_eq!(
                        aligned.len() + usize::try_from(excluded).unwrap_or(0),
                        transcript.cues().len()
                    );
                    for cue in aligned {
                        prop_assert!(cue.range.end().as_micros() <= 12_000_000);
                        prop_assert!(cue.range.start() < cue.range.end());
                        prop_assert_eq!(
                            i128::from(cue.range.start().as_micros()),
                            offset.shift(cue.cue.timing.start_micros())
                        );
                    }
                }
                Err(error) => prop_assert_eq!(
                    error.rejection(),
                    TranscriptRejection::NoCuesWithinSource
                ),
            }
        }
    }
}
