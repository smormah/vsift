use std::num::NonZeroU32;

use vsift_application::{SessionStorageError, screen_text_index_id, screen_text_line_id};
use vsift_domain::{
    Confidence, ConfidenceOrigin, DEFAULT_READ_PERIOD_MICROS, FrameDimensions, FrameRef, MediaTime,
    ProviderLabel, ReadTarget, ScreenText, ScreenTextFailure, ScreenTextFrame, ScreenTextIndex,
    ScreenTextIndexParts, ScreenTextLine, ScreenTextOutcome, ScreenTextProfile, ScreenTextProvider,
    ScreenTextScope, SessionId, Sha256Hex, SourceId, TextRegion, TimeBase, VisualCandidateId,
};

use super::{decode_screen_text_index_record, encode_screen_text_index_record};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

pub(crate) fn sample_index(session: &SessionId) -> Built<ScreenTextIndex> {
    let digest = |fill: char| Sha256Hex::parse(fill.to_string().repeat(64));
    let scope = ScreenTextScope {
        session_id: session.clone(),
        source_id: SourceId::parse(format!("src_sha256_{}", "b".repeat(64)))?,
        stream_index: 0,
        profile: ScreenTextProfile::PpOcrV5LatinV1,
        provider: ScreenTextProvider {
            runner_sha256: digest('1')?,
            runner_version: ProviderLabel::parse("0.1.0")?,
            runtime_version: ProviderLabel::parse("1.31.0")?,
            detector_sha256: digest('2')?,
            recognizer_sha256: digest('3')?,
        },
    };
    let dimensions = FrameDimensions::new(1280, 720)?;
    let frame = |pts: i64| -> Built<FrameRef> {
        Ok(FrameRef {
            stream_index: 0,
            time_base: TimeBase::new(1, 30)?,
            pts,
            time: MediaTime::from_micros(u64::try_from(pts)? * 1_000_000 / 30),
            dimensions,
        })
    };
    let read = ScreenTextFrame {
        frame: frame(30)?,
        targets: vec![
            ReadTarget {
                time: MediaTime::from_micros(1_000_000),
                candidate: None,
            },
            ReadTarget {
                time: MediaTime::from_micros(1_000_000),
                candidate: Some(VisualCandidateId::parse(format!("vcd_{}", "c".repeat(32)))?),
            },
        ],
        outcome: ScreenTextOutcome::Read {
            lines: vec![
                ScreenTextLine {
                    id: screen_text_line_id(&scope, 30, 1)?,
                    region: TextRegion::new(40, 28, 300, 14, dimensions)?,
                    text: ScreenText::parse("Página principal")?,
                    confidence: Confidence::provider_score(
                        9_912,
                        ConfidenceOrigin::ProviderUncalibrated,
                    )?,
                },
                ScreenTextLine {
                    id: screen_text_line_id(&scope, 30, 2)?,
                    region: TextRegion::new(40, 60, 200, 14, dimensions)?,
                    text: ScreenText::parse("Total \"4,731,145\"")?,
                    confidence: Confidence::provider_score(
                        8_001,
                        ConfidenceOrigin::ProviderUncalibrated,
                    )?,
                },
            ],
            truncated: true,
        },
    };
    let failed = ScreenTextFrame {
        frame: frame(300)?,
        targets: vec![ReadTarget {
            time: MediaTime::from_micros(10_000_000),
            candidate: None,
        }],
        outcome: ScreenTextOutcome::Failed(ScreenTextFailure::ImageRejected),
    };
    let revision = NonZeroU32::new(2).ok_or("revision")?;
    Ok(ScreenTextIndex::new(ScreenTextIndexParts {
        id: screen_text_index_id(&scope, revision)?,
        revision,
        scope,
        period_micros: DEFAULT_READ_PERIOD_MICROS,
        frames: vec![read, failed],
        unresolved: vec![ReadTarget {
            time: MediaTime::from_micros(60_000_000),
            candidate: None,
        }],
    })?)
}

fn session() -> Built<SessionId> {
    Ok(SessionId::parse("ses_0123456789abcdef")?)
}

#[test]
fn a_record_round_trips_exactly() -> TestResult {
    let session = session()?;
    let index = sample_index(&session)?;
    let bytes = encode_screen_text_index_record(&index)?;
    assert_eq!(decode_screen_text_index_record(&bytes, &session)?, index);
    let text = String::from_utf8(bytes)?;
    assert!(text.contains("\"format\":\"vsift.screen_text_index_record\""));
    assert!(!text.contains("png"), "the record holds no pixels");
    Ok(())
}

#[test]
fn a_record_of_another_session_or_with_an_edited_identity_is_refused() -> TestResult {
    let session = session()?;
    let bytes = encode_screen_text_index_record(&sample_index(&session)?)?;
    let other = SessionId::parse("ses_fedcba9876543210")?;
    assert_eq!(
        decode_screen_text_index_record(&bytes, &other),
        Err(SessionStorageError::IntegrityFailure)
    );
    let text = String::from_utf8(bytes)?;
    let mut value: serde_json::Value = serde_json::from_str(&text)?;
    if let Some(line) = value.pointer_mut("/frames/0/lines/1") {
        line["id"] = serde_json::Value::from("stl_0000000000000000");
    }
    let edited = serde_json::to_vec(&value)?;
    assert_eq!(
        decode_screen_text_index_record(&edited, &session),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

#[test]
fn a_record_that_breaks_a_domain_rule_or_holds_an_unknown_field_is_refused() -> TestResult {
    let session = session()?;
    let text = String::from_utf8(encode_screen_text_index_record(&sample_index(&session)?)?)?;
    let base: serde_json::Value = serde_json::from_str(&text)?;
    let edits: [(&str, serde_json::Value); 6] = [
        ("/frames/0/lines/0/x", serde_json::Value::from(1_270)),
        (
            "/frames/0/lines/0/text",
            serde_json::Value::from("bad\u{7}text"),
        ),
        (
            "/frames/0/lines/0/confidence_bp",
            serde_json::Value::from(10_001),
        ),
        ("/frames/1/pts", serde_json::Value::from(1)),
        ("/period_us", serde_json::Value::from(1)),
        ("/profile", serde_json::Value::from("tesseract")),
    ];
    for (pointer, replacement) in edits {
        let mut value = base.clone();
        if let Some(slot) = value.pointer_mut(pointer) {
            *slot = replacement;
        }
        assert_eq!(
            decode_screen_text_index_record(&serde_json::to_vec(&value)?, &session),
            Err(SessionStorageError::IntegrityFailure),
            "{pointer}"
        );
    }
    let mut extra = base.clone();
    extra["extra"] = serde_json::Value::from(true);
    assert_eq!(
        decode_screen_text_index_record(&serde_json::to_vec(&extra)?, &session),
        Err(SessionStorageError::IntegrityFailure)
    );
    let mut failed_with_lines = base.clone();
    if let Some(frame) = failed_with_lines.pointer_mut("/frames/1") {
        frame["lines"] = serde_json::json!([]);
    }
    assert_eq!(
        decode_screen_text_index_record(&serde_json::to_vec(&failed_with_lines)?, &session),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

#[test]
fn a_newer_record_version_is_unsupported_not_damaged() -> TestResult {
    let session = session()?;
    let text = String::from_utf8(encode_screen_text_index_record(&sample_index(&session)?)?)?;
    let mut value: serde_json::Value = serde_json::from_str(&text)?;
    value["schema_version"] = serde_json::Value::from(2);
    assert_eq!(
        decode_screen_text_index_record(&serde_json::to_vec(&value)?, &session),
        Err(SessionStorageError::UnsupportedVersion)
    );
    Ok(())
}
