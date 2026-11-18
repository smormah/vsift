//! Versioned storage record of one speaker record (ADR 0031).
//!
//! Each speaker record is one immutable, content-addressed session artifact of kind
//! `speaker_record`, and travels unchanged into retained bundles. It names the
//! transcript revision it labels and holds the engine's turns (times and cluster
//! numbers), what was asked of the engine, the labelling profile, the engine's and
//! models' SHA-256 and the names a user verified. It never holds audio, an embedding
//! or any other voice-derived vector: the engine keeps those in its own memory.
//!
//! Decoding is strict, like the transcript and visual-index records: unknown fields,
//! versions and values are rejected, the identity is re-derived from the session,
//! source, revision and number, and every value passes the domain's constructors.
//! The per-segment labels are not stored at all: the engine derives them from the
//! turns and the revision on every read ([`vsift_domain::SpeakerRecord::derive`]).

use std::{collections::BTreeMap, num::NonZeroU32};

use serde::{Deserialize, Serialize};
use vsift_application::{SessionStorageError, speaker_record_id};
use vsift_domain::{
    DiarizationEngine, DiarizationProvenance, DiarizedTurn, EngineSpeaker, MAX_DIARIZED_TURNS,
    MAX_VERIFIED_NAMES, MediaTime, SessionId, Sha256Hex, SourceId, SpeakerCount,
    SpeakerLabellingProfile, SpeakerRecordId, SpeakerRecordParts, SpeakerTag, TimeRange,
    TranscriptRevisionId, VerifiedName,
};

/// Largest encoded speaker record a session will store or read.
///
/// A turn is about 60 bytes of JSON; [`MAX_DIARIZED_TURNS`] turns are about 1.2 MB,
/// so 2 MiB holds the most a record may claim without allowing an unbounded one.
pub const MAX_SPEAKER_RECORD_BYTES: usize = 2 * 1024 * 1024;
/// Most speaker records one session holds. Each diarization and each recorded or
/// cleared name commits one; the limit only stops a runaway caller.
pub const MAX_SPEAKER_RECORDS: usize = 128;
const RECORD_VERSION: u16 = 1;
const RECORD_FORMAT: &str = "vsift.speaker_record";

/// Encodes a speaker record of `session_id` as its bounded storage record.
///
/// # Errors
///
/// Returns [`SessionStorageError::CapacityExhausted`] when the record would exceed
/// [`MAX_SPEAKER_RECORD_BYTES`].
pub fn encode_speaker_record(
    session_id: &SessionId,
    parts: &SpeakerRecordParts,
) -> Result<Vec<u8>, SessionStorageError> {
    let bytes = serde_json::to_vec(&StoredSpeakerRecord::from_parts(session_id, parts))
        .map_err(|_| SessionStorageError::Io)?;
    if bytes.len() > MAX_SPEAKER_RECORD_BYTES {
        return Err(SessionStorageError::CapacityExhausted);
    }
    Ok(bytes)
}

/// Decodes and revalidates a stored speaker record of `session_id`.
///
/// # Errors
///
/// Returns [`SessionStorageError::UnsupportedVersion`] for a newer record and
/// [`SessionStorageError::IntegrityFailure`] for anything malformed, oversized, of
/// another session, with an identity its content does not derive, or with a value the
/// domain refuses.
pub fn decode_speaker_record(
    bytes: &[u8],
    session_id: &SessionId,
) -> Result<SpeakerRecordParts, SessionStorageError> {
    #[derive(Deserialize)]
    struct VersionProbe {
        schema_version: u16,
    }
    if bytes.len() > MAX_SPEAKER_RECORD_BYTES {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let probe: VersionProbe =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    match probe.schema_version {
        RECORD_VERSION => {}
        newer if newer > RECORD_VERSION => return Err(SessionStorageError::UnsupportedVersion),
        _ => return Err(SessionStorageError::IntegrityFailure),
    }
    let stored: StoredSpeakerRecord =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    stored
        .into_parts(session_id)
        .ok_or(SessionStorageError::IntegrityFailure)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSpeakerRecord {
    schema_version: u16,
    format: String,
    record_id: String,
    number: u32,
    session_id: String,
    source_id: String,
    revision_id: String,
    diarized_start_us: u64,
    diarized_end_us: u64,
    /// `null` for the engine's automatic count.
    given_speaker_count: Option<u8>,
    profile: String,
    engine: StoredEngine,
    turns: Vec<StoredTurn>,
    names: Vec<StoredName>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredEngine {
    name: String,
    executable_sha256: String,
    segmentation_model_sha256: String,
    embedding_model_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredTurn {
    start_us: u64,
    end_us: u64,
    engine_speaker: u16,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredName {
    label: String,
    name: String,
}

impl StoredSpeakerRecord {
    fn from_parts(session_id: &SessionId, parts: &SpeakerRecordParts) -> Self {
        Self {
            schema_version: RECORD_VERSION,
            format: RECORD_FORMAT.to_owned(),
            record_id: parts.id.as_str().to_owned(),
            number: parts.number.get(),
            session_id: session_id.as_str().to_owned(),
            source_id: parts.source_id.as_str().to_owned(),
            revision_id: parts.revision_id.as_str().to_owned(),
            diarized_start_us: parts.diarized_range.start().as_micros(),
            diarized_end_us: parts.diarized_range.end().as_micros(),
            given_speaker_count: parts.count.given_count(),
            profile: parts.profile.identifier().to_owned(),
            engine: StoredEngine {
                name: parts.provenance.engine().identifier().to_owned(),
                executable_sha256: parts.provenance.executable_sha256().as_str().to_owned(),
                segmentation_model_sha256: parts
                    .provenance
                    .segmentation_model_sha256()
                    .as_str()
                    .to_owned(),
                embedding_model_sha256: parts
                    .provenance
                    .embedding_model_sha256()
                    .as_str()
                    .to_owned(),
            },
            turns: parts
                .turns
                .iter()
                .map(|turn| StoredTurn {
                    start_us: turn.range().start().as_micros(),
                    end_us: turn.range().end().as_micros(),
                    engine_speaker: turn.speaker().value(),
                })
                .collect(),
            names: parts
                .names
                .iter()
                .map(|(tag, name)| StoredName {
                    label: tag.to_string(),
                    name: name.as_str().to_owned(),
                })
                .collect(),
        }
    }

    fn into_parts(self, session_id: &SessionId) -> Option<SpeakerRecordParts> {
        if self.format != RECORD_FORMAT || self.session_id != session_id.as_str() {
            return None;
        }
        if self.turns.len() > MAX_DIARIZED_TURNS || self.names.len() > MAX_VERIFIED_NAMES {
            return None;
        }
        let number = NonZeroU32::new(self.number)?;
        let source_id = SourceId::parse(self.source_id).ok()?;
        let revision_id = TranscriptRevisionId::parse(self.revision_id).ok()?;
        let id = SpeakerRecordId::parse(self.record_id).ok()?;
        if speaker_record_id(session_id, &source_id, &revision_id, number).ok()? != id {
            return None;
        }
        let diarized_range = TimeRange::new(
            MediaTime::from_micros(self.diarized_start_us),
            MediaTime::from_micros(self.diarized_end_us),
        )
        .ok()?;
        let count = match self.given_speaker_count {
            None => SpeakerCount::Automatic,
            Some(count) => SpeakerCount::given(count).ok()?,
        };
        let profile = SpeakerLabellingProfile::from_identifier(&self.profile)?;
        let provenance = DiarizationProvenance::new(
            DiarizationEngine::from_identifier(&self.engine.name)?,
            Sha256Hex::parse(self.engine.executable_sha256).ok()?,
            Sha256Hex::parse(self.engine.segmentation_model_sha256).ok()?,
            Sha256Hex::parse(self.engine.embedding_model_sha256).ok()?,
        );
        let mut turns = Vec::with_capacity(self.turns.len());
        for turn in self.turns {
            let range = TimeRange::new(
                MediaTime::from_micros(turn.start_us),
                MediaTime::from_micros(turn.end_us),
            )
            .ok()?;
            turns.push(DiarizedTurn::new(
                range,
                EngineSpeaker::new(turn.engine_speaker),
            ));
        }
        let mut names = BTreeMap::new();
        for entry in self.names {
            let tag = SpeakerTag::parse(&entry.label).ok()?;
            let name = VerifiedName::parse(&entry.name).ok()?;
            // A name must be stored exactly as validated, once per label.
            if name.as_str() != entry.name || names.insert(tag, name).is_some() {
                return None;
            }
        }
        Some(SpeakerRecordParts {
            id,
            number,
            source_id,
            revision_id,
            diarized_range,
            count,
            profile,
            provenance,
            turns,
            names,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, num::NonZeroU32};

    use vsift_application::{SessionStorageError, speaker_record_id};
    use vsift_domain::{
        DiarizationEngine, DiarizationProvenance, DiarizedTurn, EngineSpeaker, MediaTime,
        SessionId, Sha256Hex, SourceId, SpeakerCount, SpeakerLabellingProfile, SpeakerRecordParts,
        SpeakerTag, TimeRange, TranscriptRevisionId, VerifiedName,
    };

    use super::{MAX_SPEAKER_RECORD_BYTES, decode_speaker_record, encode_speaker_record};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn parts(session: &SessionId) -> Result<SpeakerRecordParts, Box<dyn std::error::Error>> {
        let source_id = SourceId::from_sha256(DIGEST)?;
        let revision_id = TranscriptRevisionId::parse("trv_0123456789abcdef")?;
        let number = NonZeroU32::MIN;
        let mut names = BTreeMap::new();
        names.insert(
            SpeakerTag::parse("S1")?,
            VerifiedName::parse("Alice Example")?,
        );
        Ok(SpeakerRecordParts {
            id: speaker_record_id(session, &source_id, &revision_id, number)?,
            number,
            source_id,
            revision_id,
            diarized_range: TimeRange::new(
                MediaTime::from_micros(0),
                MediaTime::from_micros(10_000_000),
            )?,
            count: SpeakerCount::given(2)?,
            profile: SpeakerLabellingProfile::V1,
            provenance: DiarizationProvenance::new(
                DiarizationEngine::SherpaOnnx,
                Sha256Hex::parse(DIGEST)?,
                Sha256Hex::parse(DIGEST)?,
                Sha256Hex::parse(DIGEST)?,
            ),
            turns: vec![DiarizedTurn::new(
                TimeRange::new(
                    MediaTime::from_micros(318_000),
                    MediaTime::from_micros(6_865_000),
                )?,
                EngineSpeaker::new(0),
            )],
            names,
        })
    }

    #[test]
    fn a_record_round_trips_and_holds_no_voice_data() -> TestResult {
        let session = SessionId::parse("ses_0123456789abcdef")?;
        let original = parts(&session)?;
        let bytes = encode_speaker_record(&session, &original)?;
        assert_eq!(decode_speaker_record(&bytes, &session)?, original);
        let text = std::str::from_utf8(&bytes)?;
        for field in ["embedding\"", "audio", "samples", "vector"] {
            assert!(!text.contains(field), "{field}");
        }
        Ok(())
    }

    #[test]
    fn a_forged_identity_another_session_or_a_newer_version_is_refused() -> TestResult {
        let session = SessionId::parse("ses_0123456789abcdef")?;
        let bytes = encode_speaker_record(&session, &parts(&session)?)?;
        let other = SessionId::parse("ses_fedcba9876543210")?;
        assert_eq!(
            decode_speaker_record(&bytes, &other),
            Err(SessionStorageError::IntegrityFailure)
        );
        let text = std::str::from_utf8(&bytes)?;
        let forged = text.replace("\"number\":1", "\"number\":2");
        assert_eq!(
            decode_speaker_record(forged.as_bytes(), &session),
            Err(SessionStorageError::IntegrityFailure)
        );
        let newer = text.replace("\"schema_version\":1", "\"schema_version\":2");
        assert_eq!(
            decode_speaker_record(newer.as_bytes(), &session),
            Err(SessionStorageError::UnsupportedVersion)
        );
        let unknown = text.replacen('{', "{\"embedding\":[0.1],", 1);
        assert_eq!(
            decode_speaker_record(unknown.as_bytes(), &session),
            Err(SessionStorageError::IntegrityFailure)
        );
        let control = text.replace("Alice Example", "Alice\\u001b[31m");
        assert_eq!(
            decode_speaker_record(control.as_bytes(), &session),
            Err(SessionStorageError::IntegrityFailure)
        );
        let oversized = vec![b' '; MAX_SPEAKER_RECORD_BYTES + 1];
        assert_eq!(
            decode_speaker_record(&oversized, &session),
            Err(SessionStorageError::IntegrityFailure)
        );
        Ok(())
    }
}
