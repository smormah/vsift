//! Speaker labels: "who said what" by voice, as provisional labels per transcript segment.
//!
//! A diarization engine (an external adapter) hears the source's audio and answers with
//! turns: a time range and the engine's own cluster number. This module turns those
//! turns into `VSift`'s speaker labels for one immutable transcript revision:
//!
//! - **Provisional labels.** Clusters are renumbered `S1`, `S2`, ... by when each is first
//!   heard. A label says "the same voice", never who the person is
//!   ([`SpeakerTag`]); the only identity is a name the user verified ([`VerifiedName`]).
//! - **Segment precision.** `VSift` has segment times, not word times, so a segment gets the
//!   speaker who holds most of the speech the engine heard inside it, when one clearly
//!   does ([`SpeakerLabellingProfile`]); otherwise it has no confident speaker. A segment
//!   in which two people speak is never split.
//! - **Derived on every read.** A [`SpeakerRecord`] holds the turns and how they were
//!   made; the per-segment labels are re-derived from them and the revision whenever
//!   the record is read, so a stored record cannot claim a label its turns do not give.
//! - **No voiceprints.** Nothing here holds audio or an embedding: an engine keeps those
//!   in its own memory and returns times and cluster numbers only (ADR 0031).

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    num::{NonZeroU8, NonZeroU16, NonZeroU32},
};

use crate::{
    MediaTime, Sha256Hex, SourceId, TimeRange, TranscriptRevision, TranscriptRevisionId,
    TranscriptSegmentId, identity::SpeakerRecordId,
};

#[cfg(test)]
mod tests;

/// The most turns one diarization may report.
///
/// sherpa-onnx with `TitaNet` small gave 50 turns for RM06's ten minutes of a two-person
/// call (run 39527538316): about 300 an hour. The bound leaves a wide margin for rapid
/// exchanges within the duration cap and keeps a stored record small.
pub const MAX_DIARIZED_TURNS: usize = 20_000;

/// The most distinct clusters one diarization may report.
///
/// An automatic count over a long, noisy recording can find dozens of short clusters
/// (40 on RM09's 28 minutes at the threshold 0.5); more than this is an engine
/// misbehaving, refused rather than shown as hundreds of "speakers".
pub const MAX_DIARIZED_SPEAKERS: usize = 64;

/// The largest speaker count a user may give.
pub const MAX_GIVEN_SPEAKERS: u8 = 16;

/// The longest name a user may record for a speaker label, in UTF-8 bytes.
pub const MAX_VERIFIED_NAME_BYTES: usize = 128;

/// The most names one speaker record may hold (one per label at most).
pub const MAX_VERIFIED_NAMES: usize = MAX_DIARIZED_SPEAKERS;

const BASIS_POINTS: u64 = 10_000;

/// The engine's own number for one cluster of turns; meaningful only within one run.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EngineSpeaker(u16);

impl EngineSpeaker {
    /// Wraps an engine's cluster number.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// The engine's cluster number.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }
}

/// One turn an engine reported: a time on the source timeline and its cluster.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiarizedTurn {
    range: TimeRange,
    speaker: EngineSpeaker,
}

impl DiarizedTurn {
    /// Records one engine turn.
    #[must_use]
    pub const fn new(range: TimeRange, speaker: EngineSpeaker) -> Self {
        Self { range, speaker }
    }

    /// The turn's time.
    #[must_use]
    pub const fn range(self) -> TimeRange {
        self.range
    }

    /// The engine's cluster.
    #[must_use]
    pub const fn speaker(self) -> EngineSpeaker {
        self.speaker
    }
}

/// `VSift`'s provisional label for one voice in one speaker record: `S1`, `S2`, ...
///
/// Numbered by when the voice is first heard, so the same turns always give the same
/// labels. It is not an identity: two records of the same recording may number the same
/// person differently, and two similar voices may share one label (L-207).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SpeakerTag(NonZeroU16);

impl SpeakerTag {
    /// The label numbered `value` (1 for `S1`).
    #[must_use]
    pub const fn new(value: NonZeroU16) -> Self {
        Self(value)
    }

    /// The label's number.
    #[must_use]
    pub const fn number(self) -> u16 {
        self.0.get()
    }

    /// Parses `S2` (or `s2`); no leading zero, at most [`MAX_DIARIZED_SPEAKERS`].
    ///
    /// # Errors
    ///
    /// Returns [`SpeakerTagError`] for any other text.
    pub fn parse(text: &str) -> Result<Self, SpeakerTagError> {
        let digits = text
            .strip_prefix('S')
            .or_else(|| text.strip_prefix('s'))
            .ok_or(SpeakerTagError)?;
        if digits.is_empty()
            || digits.starts_with('0')
            || digits.len() > 2
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(SpeakerTagError);
        }
        let number: u16 = digits.parse().map_err(|_| SpeakerTagError)?;
        if usize::from(number) > MAX_DIARIZED_SPEAKERS {
            return Err(SpeakerTagError);
        }
        NonZeroU16::new(number).map(Self).ok_or(SpeakerTagError)
    }
}

impl fmt::Display for SpeakerTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "S{}", self.0)
    }
}

/// A speaker label was not `S` followed by a number from 1 to 64.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeakerTagError;

impl fmt::Display for SpeakerTagError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a speaker label is S followed by a number from 1 to 64, such as S2")
    }
}

impl Error for SpeakerTagError {}

/// How many speakers the engine is asked to find.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeakerCount {
    /// The engine decides, by its distance threshold (the maintainer's decision of
    /// 2026-10-21: 0.9 for sherpa-onnx with `TitaNet` small).
    Automatic,
    /// The user gave the count; it is passed to the engine as given.
    Given(NonZeroU8),
}

impl SpeakerCount {
    /// A count the user gave.
    ///
    /// # Errors
    ///
    /// Returns [`SpeakerCountError`] for 0 or more than [`MAX_GIVEN_SPEAKERS`].
    pub fn given(count: u8) -> Result<Self, SpeakerCountError> {
        if count > MAX_GIVEN_SPEAKERS {
            return Err(SpeakerCountError);
        }
        NonZeroU8::new(count)
            .map(Self::Given)
            .ok_or(SpeakerCountError)
    }

    /// The given count, if any.
    #[must_use]
    pub const fn given_count(self) -> Option<u8> {
        match self {
            Self::Automatic => None,
            Self::Given(count) => Some(count.get()),
        }
    }
}

/// A given speaker count was 0 or above 16.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeakerCountError;

impl fmt::Display for SpeakerCountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a speaker count is a whole number from 1 to {MAX_GIVEN_SPEAKERS}"
        )
    }
}

impl Error for SpeakerCountError {}

/// The reviewed rule that gives a transcript segment its speaker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeakerLabellingProfile {
    /// `speaker-labels-v1`: the speech the engine heard inside the segment covers at
    /// least half of it, and one label holds at least 70% of that speech. Measured in
    /// the engine evaluation (`docs/planning/speaker-labels-evaluation.md`): on RM06 it
    /// labelled 98% of segments with sherpa-onnx and `TitaNet` small, 1.9% of all wrongly.
    V1,
}

impl SpeakerLabellingProfile {
    /// The stable identifier in records and JSON.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::V1 => "speaker-labels-v1",
        }
    }

    /// Parses a stored identifier.
    #[must_use]
    pub fn from_identifier(value: &str) -> Option<Self> {
        (value == Self::V1.identifier()).then_some(Self::V1)
    }

    const fn min_speech_basis_points(self) -> u64 {
        match self {
            Self::V1 => 5_000,
        }
    }

    const fn min_dominant_basis_points(self) -> u64 {
        match self {
            Self::V1 => 7_000,
        }
    }
}

/// Why a segment has no confident speaker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoConfidentSpeaker {
    /// The engine heard speech in less than half of the segment.
    LittleSpeech,
    /// No one label holds 70% of the speech heard in it (often two people).
    Mixed,
}

impl NoConfidentSpeaker {
    /// The stable identifier in JSON.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::LittleSpeech => "little_speech",
            Self::Mixed => "mixed",
        }
    }
}

/// What the labelling rule says of one segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentSpeaker {
    /// One label clearly holds the segment.
    Labelled {
        /// The label.
        tag: SpeakerTag,
        /// The label's share of the speech heard in the segment, in basis points.
        dominant_basis_points: u16,
        /// The share of the segment the engine heard speech in, in basis points.
        speech_basis_points: u16,
    },
    /// No label clearly holds it.
    NoConfidentSpeaker(NoConfidentSpeaker),
}

impl SegmentSpeaker {
    /// The label, when the segment has one.
    #[must_use]
    pub const fn tag(self) -> Option<SpeakerTag> {
        match self {
            Self::Labelled { tag, .. } => Some(tag),
            Self::NoConfidentSpeaker(_) => None,
        }
    }
}

/// One transcript segment's speaker under a record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentAttribution {
    segment: TranscriptSegmentId,
    range: TimeRange,
    speaker: SegmentSpeaker,
}

impl SegmentAttribution {
    /// The segment.
    #[must_use]
    pub const fn segment(&self) -> &TranscriptSegmentId {
        &self.segment
    }

    /// The segment's time.
    #[must_use]
    pub const fn range(&self) -> TimeRange {
        self.range
    }

    /// What the rule says of it.
    #[must_use]
    pub const fn speaker(&self) -> SegmentSpeaker {
        self.speaker
    }
}

/// A turn with its provisional label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LabelledTurn {
    range: TimeRange,
    tag: SpeakerTag,
}

impl LabelledTurn {
    /// The turn's time.
    #[must_use]
    pub const fn range(self) -> TimeRange {
        self.range
    }

    /// The turn's label.
    #[must_use]
    pub const fn tag(self) -> SpeakerTag {
        self.tag
    }
}

/// Labels the turns `S1`, `S2`, ... by when each cluster is first heard.
///
/// The turns are put in start order (a tie by end, then by cluster) first, so the same
/// set of turns always gives the same labels, whatever order the engine wrote them in.
///
/// # Errors
///
/// Returns [`DiarizationError`] when there are more turns or clusters than reviewed.
pub fn label_turns(turns: &[DiarizedTurn]) -> Result<Vec<LabelledTurn>, DiarizationError> {
    if turns.len() > MAX_DIARIZED_TURNS {
        return Err(DiarizationError::TooManyTurns);
    }
    let mut ordered = turns.to_vec();
    ordered.sort_by_key(|turn| (turn.range.start(), turn.range.end(), turn.speaker));
    let mut tags: BTreeMap<EngineSpeaker, SpeakerTag> = BTreeMap::new();
    let mut labelled = Vec::with_capacity(ordered.len());
    for turn in ordered {
        let next = tags.len() + 1;
        let tag = if let Some(tag) = tags.get(&turn.speaker) {
            *tag
        } else {
            if next > MAX_DIARIZED_SPEAKERS {
                return Err(DiarizationError::TooManySpeakers);
            }
            let number = u16::try_from(next)
                .ok()
                .and_then(NonZeroU16::new)
                .ok_or(DiarizationError::TooManySpeakers)?;
            let tag = SpeakerTag::new(number);
            tags.insert(turn.speaker, tag);
            tag
        };
        labelled.push(LabelledTurn {
            range: turn.range,
            tag,
        });
    }
    Ok(labelled)
}

/// Applies the labelling rule to one segment's time.
#[must_use]
pub fn segment_speaker(
    turns: &[LabelledTurn],
    range: TimeRange,
    profile: SpeakerLabellingProfile,
) -> SegmentSpeaker {
    let length = range.duration_micros();
    let mut shares: BTreeMap<SpeakerTag, u64> = BTreeMap::new();
    for turn in turns {
        let start = turn.range.start().max(range.start()).as_micros();
        let end = turn.range.end().min(range.end()).as_micros();
        if end > start {
            *shares.entry(turn.tag).or_insert(0) += end - start;
        }
    }
    let speech: u64 = shares.values().sum();
    let covered = speech.min(length);
    if speech == 0
        || covered.saturating_mul(BASIS_POINTS)
            < profile.min_speech_basis_points().saturating_mul(length)
    {
        return SegmentSpeaker::NoConfidentSpeaker(NoConfidentSpeaker::LittleSpeech);
    }
    // The largest share; a tie goes to the lower label, so the rule is deterministic.
    let mut dominant: Option<(SpeakerTag, u64)> = None;
    for (tag, share) in &shares {
        if dominant.is_none_or(|(_, best)| *share > best) {
            dominant = Some((*tag, *share));
        }
    }
    let Some((tag, share)) = dominant else {
        return SegmentSpeaker::NoConfidentSpeaker(NoConfidentSpeaker::LittleSpeech);
    };
    if share.saturating_mul(BASIS_POINTS)
        < profile.min_dominant_basis_points().saturating_mul(speech)
    {
        return SegmentSpeaker::NoConfidentSpeaker(NoConfidentSpeaker::Mixed);
    }
    SegmentSpeaker::Labelled {
        tag,
        dominant_basis_points: basis_points(share, speech),
        speech_basis_points: basis_points(covered, length),
    }
}

fn basis_points(part: u64, whole: u64) -> u16 {
    if whole == 0 {
        return 0;
    }
    let value = part.saturating_mul(BASIS_POINTS) / whole;
    u16::try_from(value.min(BASIS_POINTS)).unwrap_or(u16::MAX)
}

/// A name the user recorded for a speaker label: their own statement, kept as given.
///
/// `VSift` never infers a name from a voice or from text on the screen; this is the only
/// identity a speaker label can carry (the maintainer's decision of 2026-10-21).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VerifiedName(String);

impl VerifiedName {
    /// Validates a name: trimmed, 1 to 128 bytes, no control characters.
    ///
    /// # Errors
    ///
    /// Returns [`VerifiedNameError`] when it is empty, too long or holds a control
    /// character.
    pub fn parse(value: &str) -> Result<Self, VerifiedNameError> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_VERIFIED_NAME_BYTES {
            return Err(VerifiedNameError::InvalidLength);
        }
        if trimmed.chars().any(char::is_control) {
            return Err(VerifiedNameError::ControlCharacter);
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The name as recorded.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a name was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifiedNameError {
    /// Empty after trimming, or longer than 128 bytes.
    InvalidLength,
    /// Holds a control character (a line break, a terminal escape).
    ControlCharacter,
}

impl fmt::Display for VerifiedNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength => write!(
                formatter,
                "a speaker name is 1 to {MAX_VERIFIED_NAME_BYTES} bytes after trimming"
            ),
            Self::ControlCharacter => {
                formatter.write_str("a speaker name holds a control character")
            }
        }
    }
}

impl Error for VerifiedNameError {}

/// The diarization engine and models that made a record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiarizationProvenance {
    engine: DiarizationEngine,
    executable_sha256: Sha256Hex,
    segmentation_model_sha256: Sha256Hex,
    embedding_model_sha256: Sha256Hex,
}

impl DiarizationProvenance {
    /// Records what made a record.
    #[must_use]
    pub const fn new(
        engine: DiarizationEngine,
        executable_sha256: Sha256Hex,
        segmentation_model_sha256: Sha256Hex,
        embedding_model_sha256: Sha256Hex,
    ) -> Self {
        Self {
            engine,
            executable_sha256,
            segmentation_model_sha256,
            embedding_model_sha256,
        }
    }

    /// The engine family.
    #[must_use]
    pub const fn engine(&self) -> DiarizationEngine {
        self.engine
    }

    /// The SHA-256 of the engine executable that ran.
    #[must_use]
    pub const fn executable_sha256(&self) -> &Sha256Hex {
        &self.executable_sha256
    }

    /// The SHA-256 of the segmentation model.
    #[must_use]
    pub const fn segmentation_model_sha256(&self) -> &Sha256Hex {
        &self.segmentation_model_sha256
    }

    /// The SHA-256 of the speaker-embedding model.
    #[must_use]
    pub const fn embedding_model_sha256(&self) -> &Sha256Hex {
        &self.embedding_model_sha256
    }
}

/// A diarization engine family `VSift` has an adapter for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiarizationEngine {
    /// sherpa-onnx's `sherpa-onnx-offline-speaker-diarization` (ADR 0031).
    SherpaOnnx,
}

impl DiarizationEngine {
    /// The stable identifier in records and JSON.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::SherpaOnnx => "sherpa-onnx",
        }
    }

    /// Parses a stored identifier.
    #[must_use]
    pub fn from_identifier(value: &str) -> Option<Self> {
        (value == Self::SherpaOnnx.identifier()).then_some(Self::SherpaOnnx)
    }
}

/// Every field of a [`SpeakerRecord`], validated together by [`SpeakerRecord::derive`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpeakerRecordParts {
    /// The record's identity.
    pub id: SpeakerRecordId,
    /// 1-based number among the session's speaker records.
    pub number: NonZeroU32,
    /// The source the turns were heard in.
    pub source_id: SourceId,
    /// The transcript revision whose segments the record labels.
    pub revision_id: TranscriptRevisionId,
    /// The source time the engine heard (the whole source, or less under the cap).
    pub diarized_range: TimeRange,
    /// What was asked of the engine.
    pub count: SpeakerCount,
    /// The labelling rule.
    pub profile: SpeakerLabellingProfile,
    /// What made the turns.
    pub provenance: DiarizationProvenance,
    /// The engine's turns, as it reported them.
    pub turns: Vec<DiarizedTurn>,
    /// Names the user verified, by label; empty until a user records one.
    pub names: BTreeMap<SpeakerTag, VerifiedName>,
}

/// Speaker labels for one transcript revision, with every per-segment label derived.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpeakerRecord {
    parts: SpeakerRecordParts,
    turns: Vec<LabelledTurn>,
    attributions: Vec<SegmentAttribution>,
}

impl SpeakerRecord {
    /// Validates the parts against the revision they label and derives every label.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizationError`] when the revision is another one, a turn lies
    /// outside the diarized range, there are too many turns or clusters, or a name is
    /// recorded for a label the turns do not have.
    pub fn derive(
        parts: SpeakerRecordParts,
        revision: &TranscriptRevision,
    ) -> Result<Self, DiarizationError> {
        if revision.id() != &parts.revision_id || revision.source_id() != &parts.source_id {
            return Err(DiarizationError::OtherRevision);
        }
        if parts
            .turns
            .iter()
            .any(|turn| !within(turn.range, parts.diarized_range))
        {
            return Err(DiarizationError::TurnOutsideAudio);
        }
        let turns = label_turns(&parts.turns)?;
        let tags: BTreeSet<SpeakerTag> = turns.iter().map(|turn| turn.tag).collect();
        if parts.names.len() > MAX_VERIFIED_NAMES
            || parts.names.keys().any(|tag| !tags.contains(tag))
        {
            return Err(DiarizationError::NameForUnknownLabel);
        }
        let attributions = revision
            .segments()
            .iter()
            .map(|segment| SegmentAttribution {
                segment: segment.id().clone(),
                range: segment.range(),
                speaker: segment_speaker(&turns, segment.range(), parts.profile),
            })
            .collect();
        Ok(Self {
            parts,
            turns,
            attributions,
        })
    }

    /// Every stored field.
    #[must_use]
    pub const fn parts(&self) -> &SpeakerRecordParts {
        &self.parts
    }

    /// The record's identity.
    #[must_use]
    pub const fn id(&self) -> &SpeakerRecordId {
        &self.parts.id
    }

    /// The labelled turns, in start order.
    #[must_use]
    pub fn turns(&self) -> &[LabelledTurn] {
        &self.turns
    }

    /// Every segment of the revision, in order, with what the rule says of it.
    #[must_use]
    pub fn attributions(&self) -> &[SegmentAttribution] {
        &self.attributions
    }

    /// The attribution of one segment, if the revision holds it.
    #[must_use]
    pub fn attribution(&self, segment: &TranscriptSegmentId) -> Option<&SegmentAttribution> {
        self.attributions
            .iter()
            .find(|attribution| &attribution.segment == segment)
    }

    /// The verified name of a label, if the user recorded one.
    #[must_use]
    pub fn name(&self, tag: SpeakerTag) -> Option<&VerifiedName> {
        self.parts.names.get(&tag)
    }

    /// The shortfall when the user gave a speaker count and the engine kept fewer
    /// voices apart; `None` for an automatic count or when it kept at least as many.
    ///
    /// A given count is passed to the engine, but the engine can still put two voices
    /// under one label (on RM11, told three, it kept two), so a count the user knows is
    /// not a guarantee and an answer must say when it was not met.
    #[must_use]
    pub fn count_shortfall(&self) -> Option<SpeakerCountShortfall> {
        let requested = self.parts.count.given_count()?;
        let found: BTreeSet<SpeakerTag> = self.turns.iter().map(|turn| turn.tag).collect();
        let found = u8::try_from(found.len()).ok()?;
        (found < requested).then_some(SpeakerCountShortfall { requested, found })
    }

    /// Every label the turns have, in order, with its totals.
    #[must_use]
    pub fn speakers(&self) -> Vec<SpeakerSummary> {
        let mut summaries: BTreeMap<SpeakerTag, SpeakerSummary> = BTreeMap::new();
        for turn in &self.turns {
            let summary = summaries.entry(turn.tag).or_insert(SpeakerSummary {
                tag: turn.tag,
                speech_micros: 0,
                labelled_segments: 0,
                first_heard: turn.range.start(),
            });
            summary.speech_micros += turn.range.duration_micros();
        }
        for attribution in &self.attributions {
            if let Some(tag) = attribution.speaker.tag()
                && let Some(summary) = summaries.get_mut(&tag)
            {
                summary.labelled_segments += 1;
            }
        }
        summaries.into_values().collect()
    }

    /// The record with `tag` named `name`, or the name removed when `name` is `None`.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizationError::NameForUnknownLabel`] when the record has no such
    /// label.
    pub fn with_name(
        &self,
        tag: SpeakerTag,
        name: Option<VerifiedName>,
    ) -> Result<SpeakerRecordParts, DiarizationError> {
        if !self.turns.iter().any(|turn| turn.tag == tag) {
            return Err(DiarizationError::NameForUnknownLabel);
        }
        let mut parts = self.parts.clone();
        match name {
            Some(name) => {
                parts.names.insert(tag, name);
            }
            None => {
                parts.names.remove(&tag);
            }
        }
        Ok(parts)
    }

    /// How the record covers the revision's segments.
    #[must_use]
    pub fn coverage(&self) -> SpeakerCoverage {
        let mut coverage = SpeakerCoverage::default();
        for attribution in &self.attributions {
            let range = attribution.range;
            if !overlaps(range, self.parts.diarized_range) {
                coverage.not_diarized_segments += 1;
                push_merged(&mut coverage.not_diarized, range);
                continue;
            }
            match attribution.speaker {
                SegmentSpeaker::Labelled { .. } => {
                    coverage.labelled_segments += 1;
                    push_merged(&mut coverage.labelled, range);
                }
                SegmentSpeaker::NoConfidentSpeaker(_) => {
                    coverage.unlabelled_segments += 1;
                    push_merged(&mut coverage.no_confident_speaker, range);
                }
            }
        }
        coverage
    }
}

fn within(inner: TimeRange, outer: TimeRange) -> bool {
    inner.start() >= outer.start() && inner.end() <= outer.end()
}

fn overlaps(left: TimeRange, right: TimeRange) -> bool {
    left.start() < right.end() && left.end() > right.start()
}

fn push_merged(ranges: &mut Vec<TimeRange>, range: TimeRange) {
    if let Some(last) = ranges.last_mut()
        && range.start() <= last.end()
    {
        if range.end() > last.end()
            && let Ok(joined) = TimeRange::new(last.start(), range.end())
        {
            *last = joined;
        }
        return;
    }
    ranges.push(range);
}

/// A given speaker count the engine did not meet: it kept `found` voices apart where
/// the user asked for `requested`, so some voices may share a label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeakerCountShortfall {
    requested: u8,
    found: u8,
}

impl SpeakerCountShortfall {
    /// The count the user gave.
    #[must_use]
    pub const fn requested(self) -> u8 {
        self.requested
    }

    /// The labels the engine's turns have (fewer than requested).
    #[must_use]
    pub const fn found(self) -> u8 {
        self.found
    }
}

/// One label's totals in a record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeakerSummary {
    tag: SpeakerTag,
    speech_micros: u64,
    labelled_segments: u32,
    first_heard: MediaTime,
}

impl SpeakerSummary {
    /// The label.
    #[must_use]
    pub const fn tag(self) -> SpeakerTag {
        self.tag
    }

    /// The engine's speech time for the label.
    #[must_use]
    pub const fn speech_micros(self) -> u64 {
        self.speech_micros
    }

    /// The segments the rule gave the label.
    #[must_use]
    pub const fn labelled_segments(self) -> u32 {
        self.labelled_segments
    }

    /// When the label is first heard.
    #[must_use]
    pub const fn first_heard(self) -> MediaTime {
        self.first_heard
    }
}

/// How a record covers its revision: segments with a label, without a confident one,
/// and outside the audio the engine heard; each with its merged time ranges.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SpeakerCoverage {
    labelled_segments: u32,
    unlabelled_segments: u32,
    not_diarized_segments: u32,
    labelled: Vec<TimeRange>,
    no_confident_speaker: Vec<TimeRange>,
    not_diarized: Vec<TimeRange>,
}

impl SpeakerCoverage {
    /// Segments the rule gave a label.
    #[must_use]
    pub const fn labelled_segments(&self) -> u32 {
        self.labelled_segments
    }

    /// Segments with no confident speaker.
    #[must_use]
    pub const fn unlabelled_segments(&self) -> u32 {
        self.unlabelled_segments
    }

    /// Segments outside the audio the engine heard.
    #[must_use]
    pub const fn not_diarized_segments(&self) -> u32 {
        self.not_diarized_segments
    }

    /// The merged ranges of labelled segments.
    #[must_use]
    pub fn labelled(&self) -> &[TimeRange] {
        &self.labelled
    }

    /// The merged ranges of segments with no confident speaker.
    #[must_use]
    pub fn no_confident_speaker(&self) -> &[TimeRange] {
        &self.no_confident_speaker
    }

    /// The merged ranges of segments the engine did not hear.
    #[must_use]
    pub fn not_diarized(&self) -> &[TimeRange] {
        &self.not_diarized
    }

    /// Whether every segment has a label.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.unlabelled_segments == 0 && self.not_diarized_segments == 0
    }
}

/// Why diarization output or a speaker record was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiarizationError {
    /// More than [`MAX_DIARIZED_TURNS`] turns.
    TooManyTurns,
    /// More than [`MAX_DIARIZED_SPEAKERS`] clusters.
    TooManySpeakers,
    /// A turn lies outside the audio the engine was given.
    TurnOutsideAudio,
    /// The record names another revision or source.
    OtherRevision,
    /// A name is recorded for a label the record does not have.
    NameForUnknownLabel,
}

impl fmt::Display for DiarizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::TooManyTurns => "the diarization reported more turns than reviewed",
            Self::TooManySpeakers => "the diarization reported more speakers than reviewed",
            Self::TurnOutsideAudio => "a diarized turn lies outside the audio that was diarized",
            Self::OtherRevision => "the speaker record belongs to another transcript revision",
            Self::NameForUnknownLabel => "the speaker record has no such label",
        };
        formatter.write_str(message)
    }
}

impl Error for DiarizationError {}
