use std::future::Future;

use serde_json::Value;
use vsift_application::{
    ExtendVisualIndexRequest, SessionStorageError, VisualIndexScope, VisualSampler,
    VisualSamplingError, extend_visual_index,
};
use vsift_domain::{
    MediaTime, SessionId, SourceId, TimeRange, VISUAL_BLOCKS, VisualHash, VisualIndex,
    VisualIndexProfile, VisualSample, VisualWindow,
};

use super::{
    MAX_VISUAL_INDEX_RECORD_BYTES, decode_visual_index_record, encode_visual_index_record,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SECOND: u64 = 1_000_000;

/// Draws F10's shape every minute, with a scroll from 30 s to 33 s, and
/// rejects window 2.
struct Scripted;

impl Scripted {
    fn answer(window: VisualWindow) -> Result<Vec<VisualSample>, VisualSamplingError> {
        if window.ordinal() == 2 {
            return Err(VisualSamplingError::Undecodable);
        }
        let mut samples = Vec::new();
        let mut time = window.lead_in_start().as_micros();
        while time < window.range().end().as_micros() {
            let second = (time / SECOND) % 60;
            let level = match second {
                5..=8 => 90,
                30..=32 => u8::try_from(100 + ((time % (60 * SECOND)) - 30 * SECOND) / 50_000)
                    .unwrap_or(u8::MAX),
                33.. => 160,
                _ => 40,
            };
            samples.push(VisualSample::from_parts(
                MediaTime::from_micros(time),
                [level; VISUAL_BLOCKS],
                VisualHash::from_bits(u64::from(level)),
            ));
            time += SECOND / 2;
        }
        Ok(samples)
    }
}

impl VisualSampler for Scripted {
    fn window_samples(
        &self,
        window: VisualWindow,
    ) -> impl Future<Output = Result<Vec<VisualSample>, VisualSamplingError>> + Send {
        std::future::ready(Self::answer(window))
    }
}

async fn built(session: &SessionId) -> Result<VisualIndex, Box<dyn std::error::Error>> {
    let source = SourceId::from_sha256(DIGEST)?;
    let scope = VisualIndexScope {
        session_id: session,
        source_id: &source,
        stream_index: 0,
        duration: MediaTime::from_micros(200 * SECOND),
        profile: VisualIndexProfile::R0,
    };
    let range = TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(200 * SECOND),
    )?;
    let extension = extend_visual_index(
        ExtendVisualIndexRequest {
            scope,
            previous: None,
            range,
        },
        &Scripted,
    )
    .await?;
    Ok(extension.revision.ok_or("no revision")?)
}

fn session() -> Result<SessionId, Box<dyn std::error::Error>> {
    Ok(SessionId::parse("ses_0123456789abcdef")?)
}

fn edited(
    bytes: &[u8],
    edit: impl FnOnce(&mut Value),
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_slice(bytes)?;
    edit(&mut value);
    Ok(serde_json::to_vec(&value)?)
}

#[tokio::test]
async fn a_record_round_trips_through_strict_decoding() -> TestResult {
    let session = session()?;
    let index = built(&session).await?;
    assert_eq!(index.windows().len(), 4);
    let bytes = encode_visual_index_record(&session, &index)?;
    assert_eq!(decode_visual_index_record(&bytes, &session)?, index);
    let value: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(value["format"], "vsift.visual_index_record");
    assert_eq!(value["windows"][2]["state"], "undecodable");
    assert!(value["windows"][2].get("candidates").is_none());
    assert!(
        value["windows"][0]["candidates"]
            .as_array()
            .is_some_and(|candidates| candidates
                .iter()
                .any(|candidate| candidate["reason"] == "motion_start"))
    );
    Ok(())
}

#[tokio::test]
async fn decoding_rejects_every_claim_the_rules_would_not_produce() -> TestResult {
    let session = session()?;
    let bytes = encode_visual_index_record(&session, &built(&session).await?)?;
    let other = SessionId::parse("ses_fedcba9876543210")?;
    assert_eq!(
        decode_visual_index_record(&bytes, &other),
        Err(SessionStorageError::IntegrityFailure)
    );
    let replacements = [
        ("/format", Value::from("other")),
        ("/revision", Value::from(0)),
        ("/revision", Value::from(2)),
        ("/stream_index", Value::from(1)),
        (
            "/source_id",
            Value::from(format!("src_sha256_{}", "f".repeat(64))),
        ),
        ("/profile", Value::from("r1")),
        ("/duration_us", Value::from(201 * SECOND)),
        ("/windows/1/start_us", Value::from(61 * SECOND)),
        (
            "/windows/0/candidates/0/id",
            Value::from(format!("vcd_{}", "0".repeat(32))),
        ),
        (
            "/windows/0/candidates/0/span_end_us",
            Value::from(4 * SECOND),
        ),
        ("/windows/0/candidates/0/reason", Value::from("guess")),
        (
            "/windows/0/candidates/0/visual_hash",
            Value::from("ABCDEF0123456789"),
        ),
        (
            "/windows/0/candidates/1/change",
            serde_json::json!({"previous_sample_us": 4_500_000_u64, "changed_blocks": 1, "max_block_delta": 5}),
        ),
        ("/windows/0/sample_count", Value::from(119)),
        (
            "/windows/0/frameless_cells",
            serde_json::json!([{"start_us": 50_000_000_u64, "end_us": 60_000_000_u64}]),
        ),
        ("/schema_version", Value::from(0)),
    ];
    for (pointer, replacement) in replacements {
        let tampered = edited(&bytes, |value| {
            if let Some(field) = value.pointer_mut(pointer) {
                *field = replacement;
            }
        })?;
        assert_eq!(
            decode_visual_index_record(&tampered, &session),
            Err(SessionStorageError::IntegrityFailure),
            "{pointer}"
        );
    }
    let additions = [
        edited(&bytes, |value| value["extra"] = Value::Bool(true))?,
        edited(&bytes, |value| {
            value["windows"][2]["candidates"] = serde_json::json!([]);
        })?,
        edited(&bytes, |value| {
            if let Some(windows) = value["windows"].as_array_mut() {
                windows.swap(0, 1);
            }
        })?,
    ];
    for tampered in additions {
        assert_eq!(
            decode_visual_index_record(&tampered, &session),
            Err(SessionStorageError::IntegrityFailure)
        );
    }
    assert_eq!(
        decode_visual_index_record(
            &edited(&bytes, |value| value["schema_version"] = 2.into())?,
            &session
        ),
        Err(SessionStorageError::UnsupportedVersion)
    );
    assert_eq!(
        decode_visual_index_record(&vec![b' '; MAX_VISUAL_INDEX_RECORD_BYTES + 1], &session),
        Err(SessionStorageError::IntegrityFailure)
    );
    assert_eq!(
        decode_visual_index_record(b"not json", &session),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}
