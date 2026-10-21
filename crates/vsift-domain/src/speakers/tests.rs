use std::{collections::BTreeMap, num::NonZeroU32};

use super::{
    DiarizationEngine, DiarizationError, DiarizationProvenance, DiarizedTurn, EngineSpeaker,
    MAX_DIARIZED_SPEAKERS, MAX_DIARIZED_TURNS, NoConfidentSpeaker, SegmentSpeaker, SpeakerCount,
    SpeakerLabellingProfile, SpeakerRecord, SpeakerRecordParts, SpeakerTag, VerifiedName,
    VerifiedNameError, label_turns, segment_speaker,
};
use crate::{
    Confidence, CueSource, CueText, CueTiming, MediaTime, SegmentOrigin, Sha256Hex,
    SidecarIdentity, SourceId, SourceSegment, SourceSegmentId, SpeakerRecordId, TimeRange,
    TranscriptFormat, TranscriptOffset, TranscriptProvenance, TranscriptRevision,
    TranscriptRevisionId, TranscriptRevisionParts, TranscriptSegment, TranscriptSegmentId,
    TranscriptSegmentParts, TranscriptWarnings,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SECOND: u64 = 1_000_000;
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PROFILE: SpeakerLabellingProfile = SpeakerLabellingProfile::V1;

fn span(start: u64, end: u64) -> Result<TimeRange, Box<dyn std::error::Error>> {
    Ok(TimeRange::new(
        MediaTime::from_micros(start),
        MediaTime::from_micros(end),
    )?)
}

fn turn(start: u64, end: u64, speaker: u16) -> Result<DiarizedTurn, Box<dyn std::error::Error>> {
    Ok(DiarizedTurn::new(
        span(start, end)?,
        EngineSpeaker::new(speaker),
    ))
}

fn tag(number: u16) -> Result<SpeakerTag, Box<dyn std::error::Error>> {
    Ok(SpeakerTag::parse(&format!("S{number}"))?)
}

/// A twenty-second imported revision with one segment per `(start, end)`, in seconds.
fn revision(ranges: &[(u64, u64)]) -> Result<TranscriptRevision, Box<dyn std::error::Error>> {
    let mut segments = Vec::new();
    for (index, (start, end)) in ranges.iter().enumerate() {
        let ordinal = u32::try_from(index + 1)?;
        let number = NonZeroU32::new(ordinal).ok_or("zero ordinal")?;
        segments.push(TranscriptSegment::new(TranscriptSegmentParts {
            id: TranscriptSegmentId::parse(format!("tsg_{ordinal:016x}"))?,
            ordinal: number,
            range: span(start * SECOND, end * SECOND)?,
            text: CueText::new(format!("segment {ordinal}"), format!("segment {ordinal}"))?,
            speaker: None,
            confidence: Confidence::unknown(),
            origin: SegmentOrigin::ImportedCue {
                cue: CueSource::new(number, number),
                timing: CueTiming::new(start * SECOND, end * SECOND)?,
            },
        }));
    }
    Ok(TranscriptRevision::new(TranscriptRevisionParts {
        id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
        number: NonZeroU32::MIN,
        source_id: SourceId::from_sha256(DIGEST)?,
        source_segment: SourceSegment::whole_file(
            SourceSegmentId::parse("sgm_0123456789abcdef")?,
            MediaTime::from_micros(20 * SECOND),
        )?,
        provenance: TranscriptProvenance::Imported {
            format: TranscriptFormat::Srt,
            sidecar: SidecarIdentity::new(DIGEST, 10)?,
            offset: TranscriptOffset::ZERO,
        },
        supersedes: None,
        replaced_range: None,
        inherited: Vec::new(),
        carried_untranscribed: Vec::new(),
        unplaced: Vec::new(),
        language: None,
        segments,
        warnings: TranscriptWarnings::default(),
    })?)
}

fn parts(
    revision: &TranscriptRevision,
    turns: Vec<DiarizedTurn>,
) -> Result<SpeakerRecordParts, Box<dyn std::error::Error>> {
    Ok(SpeakerRecordParts {
        id: SpeakerRecordId::parse("spk_0123456789abcdef")?,
        number: NonZeroU32::MIN,
        source_id: revision.source_id().clone(),
        revision_id: revision.id().clone(),
        diarized_range: span(0, 20 * SECOND)?,
        count: SpeakerCount::Automatic,
        profile: PROFILE,
        provenance: DiarizationProvenance::new(
            DiarizationEngine::SherpaOnnx,
            Sha256Hex::parse(DIGEST)?,
            Sha256Hex::parse(DIGEST)?,
            Sha256Hex::parse(DIGEST)?,
        ),
        turns,
        names: BTreeMap::new(),
    })
}

#[test]
fn labels_are_numbered_by_when_each_voice_is_first_heard() -> TestResult {
    // The engine calls the first voice 7 and writes its turns out of order.
    let turns = [
        turn(9 * SECOND, 12 * SECOND, 7)?,
        turn(4 * SECOND, 8 * SECOND, 3)?,
        turn(SECOND, 3 * SECOND, 7)?,
    ];
    let labelled = label_turns(&turns)?;
    let tags: Vec<String> = labelled.iter().map(|turn| turn.tag().to_string()).collect();
    assert_eq!(tags, ["S1", "S2", "S1"]);
    assert_eq!(labelled[0].range(), span(SECOND, 3 * SECOND)?);
    Ok(())
}

#[test]
fn a_segment_one_voice_clearly_holds_gets_that_voice() -> TestResult {
    let labelled = label_turns(&[turn(0, 4 * SECOND, 0)?, turn(4 * SECOND, 4_500_000, 1)?])?;
    // 4 s of S1 and 0.5 s of S2 in a 5 s segment: S1 holds 89% of the speech.
    let speaker = segment_speaker(&labelled, span(0, 5 * SECOND)?, PROFILE);
    assert_eq!(
        speaker,
        SegmentSpeaker::Labelled {
            tag: tag(1)?,
            dominant_basis_points: 8_888,
            speech_basis_points: 9_000,
        }
    );
    Ok(())
}

#[test]
fn a_segment_two_voices_share_has_no_confident_speaker() -> TestResult {
    let labelled = label_turns(&[turn(0, 2 * SECOND, 0)?, turn(2 * SECOND, 4 * SECOND, 1)?])?;
    assert_eq!(
        segment_speaker(&labelled, span(0, 4 * SECOND)?, PROFILE),
        SegmentSpeaker::NoConfidentSpeaker(NoConfidentSpeaker::Mixed)
    );
    Ok(())
}

#[test]
fn a_segment_with_little_speech_heard_has_no_confident_speaker() -> TestResult {
    let labelled = label_turns(&[turn(0, SECOND, 0)?])?;
    assert_eq!(
        segment_speaker(&labelled, span(0, 4 * SECOND)?, PROFILE),
        SegmentSpeaker::NoConfidentSpeaker(NoConfidentSpeaker::LittleSpeech)
    );
    assert_eq!(
        segment_speaker(&labelled, span(5 * SECOND, 6 * SECOND)?, PROFILE),
        SegmentSpeaker::NoConfidentSpeaker(NoConfidentSpeaker::LittleSpeech)
    );
    // Exactly half heard is enough.
    assert_eq!(
        segment_speaker(&labelled, span(0, 2 * SECOND)?, PROFILE).tag(),
        Some(tag(1)?)
    );
    Ok(())
}

#[test]
fn exactly_seventy_percent_is_enough_and_a_tie_goes_to_the_lower_label() -> TestResult {
    let labelled = label_turns(&[turn(0, 7 * SECOND, 0)?, turn(7 * SECOND, 10 * SECOND, 1)?])?;
    assert_eq!(
        segment_speaker(&labelled, span(0, 10 * SECOND)?, PROFILE).tag(),
        Some(tag(1)?)
    );
    let overlapping = label_turns(&[turn(0, 4 * SECOND, 0)?, turn(0, 4 * SECOND, 1)?])?;
    assert_eq!(
        segment_speaker(&overlapping, span(0, 4 * SECOND)?, PROFILE),
        SegmentSpeaker::NoConfidentSpeaker(NoConfidentSpeaker::Mixed)
    );
    Ok(())
}

#[test]
fn a_record_labels_every_segment_and_says_what_it_covers() -> TestResult {
    let revision = revision(&[(0, 4), (4, 8), (8, 12), (13, 15)])?;
    let record = SpeakerRecord::derive(
        parts(
            &revision,
            vec![
                turn(0, 4 * SECOND, 5)?,
                turn(4 * SECOND, 8 * SECOND, 2)?,
                turn(8 * SECOND, 10 * SECOND, 5)?,
                turn(10 * SECOND, 12 * SECOND, 2)?,
            ],
        )?,
        &revision,
    )?;
    let labels: Vec<Option<String>> = record
        .attributions()
        .iter()
        .map(|attribution| attribution.speaker().tag().map(|tag| tag.to_string()))
        .collect();
    assert_eq!(
        labels,
        [Some("S1".to_owned()), Some("S2".to_owned()), None, None]
    );
    let coverage = record.coverage();
    assert_eq!(coverage.labelled_segments(), 2);
    assert_eq!(coverage.unlabelled_segments(), 2);
    assert_eq!(coverage.labelled(), [span(0, 8 * SECOND)?]);
    assert_eq!(
        coverage.no_confident_speaker(),
        [
            span(8 * SECOND, 12 * SECOND)?,
            span(13 * SECOND, 15 * SECOND)?
        ]
    );
    assert!(!coverage.is_complete());
    let speakers = record.speakers();
    assert_eq!(speakers.len(), 2);
    assert_eq!(speakers[0].speech_micros(), 6 * SECOND);
    assert_eq!(speakers[0].labelled_segments(), 1);
    Ok(())
}

#[test]
fn segments_outside_the_diarized_audio_are_not_diarized() -> TestResult {
    let revision = revision(&[(0, 4), (12, 16)])?;
    let mut capped = parts(&revision, vec![turn(0, 4 * SECOND, 0)?])?;
    capped.diarized_range = span(0, 10 * SECOND)?;
    let record = SpeakerRecord::derive(capped, &revision)?;
    let coverage = record.coverage();
    assert_eq!(coverage.labelled_segments(), 1);
    assert_eq!(coverage.not_diarized_segments(), 1);
    assert_eq!(coverage.not_diarized(), [span(12 * SECOND, 16 * SECOND)?]);
    Ok(())
}

#[test]
fn a_record_is_refused_for_another_revision_or_a_turn_outside_its_audio() -> TestResult {
    let revision = revision(&[(0, 4)])?;
    let mut other = parts(&revision, vec![turn(0, SECOND, 0)?])?;
    other.revision_id = TranscriptRevisionId::parse("trv_fedcba9876543210")?;
    assert_eq!(
        SpeakerRecord::derive(other, &revision),
        Err(DiarizationError::OtherRevision)
    );
    let outside = parts(&revision, vec![turn(19 * SECOND, 21 * SECOND, 0)?])?;
    assert_eq!(
        SpeakerRecord::derive(outside, &revision),
        Err(DiarizationError::TurnOutsideAudio)
    );
    Ok(())
}

#[test]
fn too_many_turns_or_voices_are_refused() -> TestResult {
    let one = turn(0, SECOND, 0)?;
    assert_eq!(
        label_turns(&vec![one; MAX_DIARIZED_TURNS + 1]),
        Err(DiarizationError::TooManyTurns)
    );
    let mut many = Vec::new();
    for speaker in 0..=u16::try_from(MAX_DIARIZED_SPEAKERS)? {
        let start = u64::from(speaker) * SECOND / 10;
        many.push(turn(start, start + 1_000, speaker)?);
    }
    assert_eq!(label_turns(&many), Err(DiarizationError::TooManySpeakers));
    Ok(())
}

#[test]
fn a_name_is_recorded_only_for_a_label_the_record_has() -> TestResult {
    let revision = revision(&[(0, 4)])?;
    let record =
        SpeakerRecord::derive(parts(&revision, vec![turn(0, 4 * SECOND, 0)?])?, &revision)?;
    let named = SpeakerRecord::derive(
        record.with_name(tag(1)?, Some(VerifiedName::parse("  Alice Example ")?))?,
        &revision,
    )?;
    assert_eq!(
        named.name(tag(1)?).map(VerifiedName::as_str),
        Some("Alice Example")
    );
    assert_eq!(
        record.with_name(tag(2)?, Some(VerifiedName::parse("Bob")?)),
        Err(DiarizationError::NameForUnknownLabel)
    );
    let cleared = SpeakerRecord::derive(named.with_name(tag(1)?, None)?, &revision)?;
    assert_eq!(cleared.name(tag(1)?), None);
    let mut forged = parts(&revision, vec![turn(0, 4 * SECOND, 0)?])?;
    forged
        .names
        .insert(tag(3)?, VerifiedName::parse("Mallory")?);
    assert_eq!(
        SpeakerRecord::derive(forged, &revision),
        Err(DiarizationError::NameForUnknownLabel)
    );
    Ok(())
}

#[test]
fn speaker_labels_names_and_counts_are_bounded() -> TestResult {
    assert_eq!(tag(12)?.to_string(), "S12");
    assert_eq!(SpeakerTag::parse("s3")?.number(), 3);
    for refused in ["S0", "S01", "S65", "S", "T1", "S1 ", "S-1", "S100"] {
        assert!(SpeakerTag::parse(refused).is_err(), "{refused}");
    }
    assert_eq!(
        VerifiedName::parse(" "),
        Err(VerifiedNameError::InvalidLength)
    );
    assert_eq!(
        VerifiedName::parse(&"x".repeat(129)),
        Err(VerifiedNameError::InvalidLength)
    );
    assert_eq!(
        VerifiedName::parse("Alice\u{1b}[31m"),
        Err(VerifiedNameError::ControlCharacter)
    );
    assert!(SpeakerCount::given(0).is_err());
    assert!(SpeakerCount::given(17).is_err());
    assert_eq!(SpeakerCount::given(2)?.given_count(), Some(2));
    assert_eq!(SpeakerCount::Automatic.given_count(), None);
    assert_eq!(
        SpeakerLabellingProfile::from_identifier("speaker-labels-v1"),
        Some(PROFILE)
    );
    Ok(())
}
