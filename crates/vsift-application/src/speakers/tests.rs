use std::{
    future::Future,
    num::{NonZeroU8, NonZeroU32},
    sync::Mutex,
};

use vsift_domain::{
    Confidence, CueSource, CueText, CueTiming, DiarizationEngine, DiarizationError,
    DiarizationProvenance, DiarizedTurn, EngineSpeaker, MediaTime, SegmentOrigin, SessionId,
    Sha256Hex, SidecarIdentity, SourceId, SourceSegment, SourceSegmentId, SpeakerCount, SpeakerTag,
    TimeRange, TranscriptFormat, TranscriptOffset, TranscriptProvenance, TranscriptRevision,
    TranscriptRevisionId, TranscriptRevisionParts, TranscriptSegment, TranscriptSegmentId,
    TranscriptSegmentParts, TranscriptWarnings, VerifiedName,
};

use super::{
    DiarizationRequest, DiarizationRunError, LabelSpeakersRequest, MAX_DIARIZATION_MICROS,
    SpeakerDiarizer, SpeakerLabelError, diarization_range, label_speakers, name_speaker,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SECOND: u64 = 1_000_000;
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OTHER_DIGEST: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn span(start: u64, end: u64) -> Result<TimeRange, Box<dyn std::error::Error>> {
    Ok(TimeRange::new(
        MediaTime::from_micros(start),
        MediaTime::from_micros(end),
    )?)
}

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

fn provenance(engine_digest: &str) -> Result<DiarizationProvenance, Box<dyn std::error::Error>> {
    Ok(DiarizationProvenance::new(
        DiarizationEngine::SherpaOnnx,
        Sha256Hex::parse(engine_digest)?,
        Sha256Hex::parse(DIGEST)?,
        Sha256Hex::parse(DIGEST)?,
    ))
}

/// A diarizer that answers fixed turns and records what it was asked.
struct FakeDiarizer {
    turns: Result<Vec<DiarizedTurn>, DiarizationRunError>,
    identities: Mutex<Vec<DiarizationProvenance>>,
    requests: Mutex<Vec<DiarizationRequest>>,
}

impl FakeDiarizer {
    fn answering(turns: Vec<DiarizedTurn>) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            turns: Ok(turns),
            identities: Mutex::new(vec![provenance(DIGEST)?, provenance(DIGEST)?]),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<DiarizationRequest> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }
}

impl SpeakerDiarizer for FakeDiarizer {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<DiarizationProvenance, DiarizationRunError>> + Send {
        let next = self
            .identities
            .lock()
            .ok()
            .and_then(|mut identities| (!identities.is_empty()).then(|| identities.remove(0)));
        async move { next.ok_or(DiarizationRunError::EngineUnavailable) }
    }

    fn diarize(
        &self,
        request: DiarizationRequest,
    ) -> impl Future<Output = Result<Vec<DiarizedTurn>, DiarizationRunError>> + Send {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request);
        }
        let answer = self.turns.clone();
        async move { answer }
    }
}

fn turn(start: u64, end: u64, speaker: u16) -> Result<DiarizedTurn, Box<dyn std::error::Error>> {
    Ok(DiarizedTurn::new(
        span(start, end)?,
        EngineSpeaker::new(speaker),
    ))
}

fn session() -> Result<SessionId, Box<dyn std::error::Error>> {
    Ok(SessionId::parse("ses_0123456789abcdef")?)
}

#[tokio::test]
async fn the_whole_source_is_diarized_and_every_segment_labelled() -> TestResult {
    let revision = revision(&[(0, 4), (4, 8)])?;
    let diarizer = FakeDiarizer::answering(vec![
        turn(0, 4 * SECOND, 1)?,
        turn(4 * SECOND, 8 * SECOND, 0)?,
    ])?;
    let session = session()?;
    let record = label_speakers(
        LabelSpeakersRequest {
            session_id: &session,
            revision: &revision,
            source_duration: MediaTime::from_micros(20 * SECOND),
            range: None,
            count: SpeakerCount::Given(NonZeroU8::MIN.saturating_add(1)),
            number: NonZeroU32::MIN,
        },
        &diarizer,
    )
    .await?;
    assert_eq!(
        diarizer.requests(),
        [DiarizationRequest {
            range: span(0, 20 * SECOND)?,
            count: SpeakerCount::Given(NonZeroU8::MIN.saturating_add(1)),
        }]
    );
    let labels: Vec<String> = record
        .attributions()
        .iter()
        .filter_map(|attribution| attribution.speaker().tag())
        .map(|tag| tag.to_string())
        .collect();
    assert_eq!(labels, ["S1", "S2"]);
    assert!(record.id().as_str().starts_with("spk_"));
    assert_eq!(record.parts().revision_id, *revision.id());
    Ok(())
}

#[test]
fn a_range_longer_than_the_cap_is_refused_with_the_cap() -> TestResult {
    let long = MediaTime::from_micros(MAX_DIARIZATION_MICROS + SECOND);
    assert_eq!(
        diarization_range(None, long),
        Err(SpeakerLabelError::RangeTooLong {
            cap_micros: MAX_DIARIZATION_MICROS,
            requested_micros: MAX_DIARIZATION_MICROS + SECOND,
        })
    );
    // A range of the long source within the cap is accepted.
    assert_eq!(
        diarization_range(Some(span(SECOND, MAX_DIARIZATION_MICROS + SECOND)?), long),
        Ok(span(SECOND, MAX_DIARIZATION_MICROS + SECOND)?)
    );
    assert_eq!(
        diarization_range(
            Some(span(0, 30 * SECOND)?),
            MediaTime::from_micros(20 * SECOND)
        ),
        Err(SpeakerLabelError::RangeOutsideSource)
    );
    assert_eq!(
        diarization_range(
            Some(span(0, SECOND / 2)?),
            MediaTime::from_micros(20 * SECOND)
        ),
        Err(SpeakerLabelError::RangeTooShort)
    );
    Ok(())
}

#[tokio::test]
async fn a_swapped_engine_or_model_keeps_no_turns() -> TestResult {
    let revision = revision(&[(0, 4)])?;
    let diarizer = FakeDiarizer {
        turns: Ok(vec![turn(0, 4 * SECOND, 0)?]),
        identities: Mutex::new(vec![provenance(DIGEST)?, provenance(OTHER_DIGEST)?]),
        requests: Mutex::new(Vec::new()),
    };
    let session = session()?;
    let outcome = label_speakers(
        LabelSpeakersRequest {
            session_id: &session,
            revision: &revision,
            source_duration: MediaTime::from_micros(20 * SECOND),
            range: None,
            count: SpeakerCount::Automatic,
            number: NonZeroU32::MIN,
        },
        &diarizer,
    )
    .await;
    assert_eq!(outcome, Err(SpeakerLabelError::EngineChanged));
    Ok(())
}

#[tokio::test]
async fn engine_failures_and_turns_outside_the_audio_are_typed() -> TestResult {
    let revision = revision(&[(0, 4)])?;
    let session = session()?;
    let request = |range| LabelSpeakersRequest {
        session_id: &session,
        revision: &revision,
        source_duration: MediaTime::from_micros(20 * SECOND),
        range,
        count: SpeakerCount::Automatic,
        number: NonZeroU32::MIN,
    };
    let failing = FakeDiarizer {
        turns: Err(DiarizationRunError::AbnormalTermination),
        identities: Mutex::new(vec![provenance(DIGEST)?, provenance(DIGEST)?]),
        requests: Mutex::new(Vec::new()),
    };
    assert_eq!(
        label_speakers(request(None), &failing).await,
        Err(SpeakerLabelError::Diarization(
            DiarizationRunError::AbnormalTermination
        ))
    );
    // A turn 5 ms past the range is the engine's rounding and is trimmed; one 2 s past
    // it is refused.
    let rounding = FakeDiarizer::answering(vec![turn(0, 10 * SECOND + 5_000, 0)?])?;
    let record = label_speakers(request(Some(span(0, 10 * SECOND)?)), &rounding).await?;
    assert_eq!(record.turns()[0].range(), span(0, 10 * SECOND)?);
    let outside = FakeDiarizer::answering(vec![turn(0, 12 * SECOND, 0)?])?;
    assert_eq!(
        label_speakers(request(Some(span(0, 10 * SECOND)?)), &outside).await,
        Err(SpeakerLabelError::Record(
            DiarizationError::TurnOutsideAudio
        ))
    );
    Ok(())
}

#[tokio::test]
async fn naming_makes_the_next_record_of_the_same_turns() -> TestResult {
    let revision = revision(&[(0, 4), (4, 8)])?;
    let session = session()?;
    let diarizer = FakeDiarizer::answering(vec![
        turn(0, 4 * SECOND, 0)?,
        turn(4 * SECOND, 8 * SECOND, 1)?,
    ])?;
    let first = label_speakers(
        LabelSpeakersRequest {
            session_id: &session,
            revision: &revision,
            source_duration: MediaTime::from_micros(20 * SECOND),
            range: None,
            count: SpeakerCount::Automatic,
            number: NonZeroU32::MIN,
        },
        &diarizer,
    )
    .await?;
    let second_number = NonZeroU32::MIN.saturating_add(1);
    let tag = SpeakerTag::parse("S2")?;
    let named = name_speaker(
        &session,
        &first,
        &revision,
        tag,
        Some(VerifiedName::parse("Alice Example")?),
        second_number,
    )?;
    assert_ne!(named.id(), first.id());
    assert_eq!(named.parts().number, second_number);
    assert_eq!(named.turns(), first.turns());
    assert_eq!(
        named.name(tag).map(VerifiedName::as_str),
        Some("Alice Example")
    );
    assert_eq!(first.name(tag), None);
    assert_eq!(
        name_speaker(
            &session,
            &first,
            &revision,
            SpeakerTag::parse("S9")?,
            None,
            second_number
        ),
        Err(SpeakerLabelError::Record(
            DiarizationError::NameForUnknownLabel
        ))
    );
    Ok(())
}
