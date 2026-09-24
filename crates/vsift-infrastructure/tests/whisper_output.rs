//! whisper.cpp v1.9.2 `-ojf` output parsing, WAV encoding and argument policy.
//!
//! The fixtures under `tests/fixtures/whisper-1.9.2/` are real outputs of the
//! reviewed Windows build (`whisper-cli.exe`, SHA-256 `95e3c0b0...`) with the
//! pinned multilingual base model, for the committed F01, F05, F08 and F09
//! speech clips decoded to 16 kHz mono. Only `params.model` and `systeminfo`
//! were replaced, because they held a local path and host details; the parser
//! never reads either. Mutated variants are derived from them here.

use std::{
    env,
    error::Error,
    fs,
    num::{NonZeroU16, NonZeroU32},
    path::{Path, PathBuf},
};

use proptest::prelude::{prop_assert, proptest};
use vsift_application::{
    AsrRevisionRequest, AsrTranscription, SessionStorageError, build_asr_revision,
    whole_file_source_segment,
};
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, ConfidenceOrigin, MediaTime, PlannedChunk,
    ProviderOutputError, ProviderTokenKind, SessionId, Sha256Hex, SourceId, SourceSegmentId,
    TimeRange, TranscriptRejection, merge_chunks, plan_chunks, validate_chunk_output,
};
use vsift_infrastructure::{
    HostIsolation, TrustedExecutable, WhisperCli, WhisperOutputError, WhisperOutputLimits,
    decode_transcript_record, encode_speech_wav, encode_transcript_record, parse_whisper_full_json,
};

type TestResult = Result<(), Box<dyn Error>>;

fn fixture(id: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/whisper-1.9.2")
            .join(format!("{id}.base.json")),
    )?)
}

fn value(id: &str) -> Result<serde_json::Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&fixture(id)?)?)
}

fn range(start: u64, end: u64) -> Result<TimeRange, Box<dyn Error>> {
    Ok(TimeRange::new(
        MediaTime::from_micros(start),
        MediaTime::from_micros(end),
    )?)
}

fn chunk(window_end: u64) -> Result<PlannedChunk, Box<dyn Error>> {
    Ok(PlannedChunk::new(
        SourceSegmentId::parse("sgm_0123456789abcdef")?,
        0,
        range(0, window_end)?,
    ))
}

fn texts(output: &vsift_domain::ProviderChunkOutput) -> Vec<String> {
    output
        .segments
        .iter()
        .filter_map(|segment| segment.text.as_ref().map(|text| text.text().to_owned()))
        .collect()
}

/// The recorded outputs parse and pass the domain's chunk rules.
#[test]
fn recorded_outputs_parse_and_validate() -> TestResult {
    let clips = [
        ("F01", 6_000_000, 1),
        ("F05", 20_000_000, 2),
        ("F08", 18_000_000, 2),
        ("F09", 11_250_000, 1),
    ];
    for (id, decoded, segments) in clips {
        let output = parse_whisper_full_json(&fixture(id)?, WhisperOutputLimits::R0)?;
        assert_eq!(output.segments.len(), segments, "{id}");
        assert_eq!(
            output
                .language
                .as_ref()
                .map(vsift_domain::LanguageTag::as_str),
            Some("en"),
            "{id}"
        );
        for segment in &output.segments {
            assert!(
                segment
                    .tokens
                    .iter()
                    .any(|token| token.kind == ProviderTokenKind::Special)
            );
            assert!(
                segment
                    .tokens
                    .iter()
                    .any(|token| token.kind == ProviderTokenKind::Text)
            );
        }
        let planned = chunk(decoded.max(1))?;
        let validated =
            validate_chunk_output(&planned, range(0, decoded)?, range(0, 60_000_000)?, output)?;
        assert_eq!(validated.segments().len(), segments, "{id}");
        for segment in validated.segments() {
            assert_eq!(
                segment.confidence().origin(),
                ConfidenceOrigin::ProviderUncalibrated
            );
        }
    }
    let f01 = parse_whisper_full_json(&fixture("F01")?, WhisperOutputLimits::R0)?;
    let text = texts(&f01).join(" ");
    for expected in ["service status", "healthy", "2048"] {
        assert!(text.contains(expected), "F01 text lacks {expected}");
    }
    // Text is trimmed of whisper's leading space; offsets are milliseconds.
    assert_eq!(
        texts(&f01),
        ["The service status is healthy and the build is 2048."]
    );
    assert_eq!(f01.segments[0].end.as_micros(), 5_260_000);
    Ok(())
}

/// The recorded F08 output, which includes Spanish, is valid UTF-8 end to end.
#[test]
fn the_spanish_bearing_output_is_valid_utf8() -> TestResult {
    let bytes = fixture("F08")?;
    assert!(std::str::from_utf8(&bytes).is_ok());
    let output = parse_whisper_full_json(&bytes, WhisperOutputLimits::R0)?;
    assert!(texts(&output)[1].starts_with("El identificador es"));
    // A multi-byte token split across tokens, as older whisper.cpp builds
    // wrote it, makes the whole document invalid UTF-8 and is rejected.
    let text = String::from_utf8(bytes)?;
    let mut broken = text
        .replacen("identific", "identific\u{e9}", 1)
        .into_bytes();
    let position = broken
        .windows(2)
        .position(|pair| pair == [0xc3, 0xa9])
        .ok_or("no accent")?;
    broken.remove(position + 1);
    assert_eq!(
        parse_whisper_full_json(&broken, WhisperOutputLimits::R0),
        Err(WhisperOutputError::InvalidUtf8)
    );
    Ok(())
}

/// Mutated outputs fail with typed errors, at the parser or in the domain.
#[test]
fn mutated_outputs_are_rejected() -> TestResult {
    let bytes = fixture("F05")?;
    assert_eq!(
        parse_whisper_full_json(&bytes[..bytes.len() / 2], WhisperOutputLimits::R0),
        Err(WhisperOutputError::Unparseable)
    );
    let small = WhisperOutputLimits {
        max_bytes: 1_024,
        ..WhisperOutputLimits::R0
    };
    assert_eq!(
        parse_whisper_full_json(&bytes, small),
        Err(WhisperOutputError::TooLarge)
    );

    let mut many = value("F09")?;
    let segment = many["transcription"][0].clone();
    many["transcription"] = serde_json::Value::Array(vec![segment; 257]);
    assert_eq!(
        parse_whisper_full_json(&serde_json::to_vec(&many)?, WhisperOutputLimits::R0),
        Err(WhisperOutputError::TooManySegments)
    );

    let mut tokens = value("F09")?;
    let token = tokens["transcription"][0]["tokens"][1].clone();
    tokens["transcription"][0]["tokens"] = serde_json::Value::Array(vec![token; 513]);
    assert_eq!(
        parse_whisper_full_json(&serde_json::to_vec(&tokens)?, WhisperOutputLimits::R0),
        Err(WhisperOutputError::TooManyTokens)
    );

    let mut control = value("F09")?;
    control["transcription"][0]["text"] = serde_json::json!(" Marker \u{1b}[31m beta");
    assert_eq!(
        parse_whisper_full_json(&serde_json::to_vec(&control)?, WhisperOutputLimits::R0),
        Err(WhisperOutputError::InvalidText(
            TranscriptRejection::ControlCharacter
        ))
    );

    let mut negative = value("F09")?;
    negative["transcription"][0]["offsets"]["from"] = serde_json::json!(-5);
    assert_eq!(
        parse_whisper_full_json(&serde_json::to_vec(&negative)?, WhisperOutputLimits::R0),
        Err(WhisperOutputError::Unparseable)
    );

    // Structurally valid but wrong for the audio: the domain decides.
    let planned = chunk(11_250_000)?;
    let audio = range(750_000, 12_000_000)?;
    let source = range(0, 12_000_000)?;
    let mut reversed = value("F09")?;
    reversed["transcription"][0]["offsets"] = serde_json::json!({"from": 4980, "to": 1000});
    let parsed = parse_whisper_full_json(&serde_json::to_vec(&reversed)?, WhisperOutputLimits::R0)?;
    assert_eq!(
        validate_chunk_output(&planned, audio, source, parsed),
        Err(ProviderOutputError::TooManyRejectedSegments)
    );
    let mut probability = value("F09")?;
    probability["transcription"][0]["tokens"][2]["p"] = serde_json::json!(1.5);
    let parsed =
        parse_whisper_full_json(&serde_json::to_vec(&probability)?, WhisperOutputLimits::R0)?;
    assert_eq!(
        validate_chunk_output(&planned, audio, source, parsed),
        Err(ProviderOutputError::InvalidTokenProbability)
    );
    Ok(())
}

proptest! {
    /// Arbitrary bytes never panic the parser, and any accepted output is bounded.
    #[test]
    fn arbitrary_bytes_are_parsed_safely(bytes in proptest::collection::vec(0_u8..=255, 0..2_048)) {
        if let Ok(output) = parse_whisper_full_json(&bytes, WhisperOutputLimits::R0) {
            prop_assert!(output.segments.len() <= WhisperOutputLimits::R0.max_segments);
        }
    }

    /// Near-miss documents built from the real structure never panic either.
    #[test]
    fn near_miss_documents_are_parsed_safely(
        from in 0_u64..u64::MAX,
        to in 0_u64..u64::MAX,
        p in proptest::num::f64::ANY,
        text in ".{0,64}",
    ) {
        let document = serde_json::json!({
            "result": {"language": text.clone()},
            "transcription": [{
                "offsets": {"from": from, "to": to},
                "text": text,
                "tokens": [{"text": "[_BEG_]", "p": if p.is_finite() { p } else { 0.5 }}]
            }]
        });
        let bytes = serde_json::to_vec(&document).unwrap_or_default();
        if let Ok(output) = parse_whisper_full_json(&bytes, WhisperOutputLimits::R0) {
            prop_assert!(output.segments.len() == 1);
        }
    }
}

/// A local-ASR revision is stored as record version 2 and rebuilt exactly;
/// a changed provider time or chunk outcome fails closed.
#[test]
fn a_local_asr_revision_round_trips_through_record_version_2() -> TestResult {
    let source_id =
        SourceId::from_sha256("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(6_000_000))?;
    let planned = plan_chunks(source.id(), source.range(), ChunkPlan::R0)?;
    let audio = range(0, 6_000_000)?;
    let output = parse_whisper_full_json(&fixture("F01")?, WhisperOutputLimits::R0)?;
    let validated = validate_chunk_output(&planned[0], audio, source.range(), output)?;
    let (segments, language, warnings) = validated.into_parts();
    let merged = merge_chunks(&[segments]);
    let digest =
        Sha256Hex::parse("95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d")?;
    let run = AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, digest.clone()),
        model: AsrModel::new(AsrModelProfile::Base, digest),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero")?,
        audio_stream: 1,
        chunks: vec![AsrChunkRecord::new(
            planned[0].clone(),
            AsrChunkOutcome::Transcribed { audio },
        )],
    })?;
    let revision = build_asr_revision(AsrRevisionRequest {
        session_id: &SessionId::parse("ses_0123456789abcdef")?,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription: AsrTranscription {
            run,
            language,
            segments: merged.segments,
            warnings,
        },
        supersedes: None,
        replaced_range: None,
    })?;
    let encoded = encode_transcript_record(&revision)?;
    let record: serde_json::Value = serde_json::from_slice(&encoded)?;
    assert_eq!(record["schema_version"], 2);
    assert_eq!(record["origin"], "local_asr");
    assert_eq!(decode_transcript_record(&encoded)?, revision);
    // The stored record carries no path of any kind.
    let text = String::from_utf8(encoded)?;
    assert!(!text.contains(":\\") && !text.contains("ggml-base"));

    let mut shifted = record.clone();
    shifted["segments"][0]["provider_start_us"] = serde_json::json!(1);
    let mut silenced = record.clone();
    silenced["run"]["chunks"][0]["outcome"] = serde_json::json!("silent");
    let mut calibrated = record.clone();
    calibrated["segments"][0]["confidence_origin"] = serde_json::json!("provider_calibrated");
    let mut future = record;
    future["schema_version"] = serde_json::json!(3);
    for (label, changed, expected) in [
        ("shifted", shifted, SessionStorageError::IntegrityFailure),
        ("silenced", silenced, SessionStorageError::IntegrityFailure),
        (
            "unknown field",
            calibrated,
            SessionStorageError::IntegrityFailure,
        ),
        ("future", future, SessionStorageError::UnsupportedVersion),
    ] {
        assert_eq!(
            decode_transcript_record(&serde_json::to_vec(&changed)?),
            Err(expected),
            "{label}"
        );
    }
    Ok(())
}

#[test]
fn speech_wav_has_a_canonical_header() -> TestResult {
    let wav = encode_speech_wav(&[0, 1, -1, i16::MAX])?;
    assert_eq!(wav.len(), 44 + 8);
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(u32::from_le_bytes(wav[4..8].try_into()?), 36 + 8);
    assert_eq!(&wav[8..16], b"WAVEfmt ");
    assert_eq!(u16::from_le_bytes(wav[20..22].try_into()?), 1);
    assert_eq!(u16::from_le_bytes(wav[22..24].try_into()?), 1);
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into()?), 16_000);
    assert_eq!(u32::from_le_bytes(wav[28..32].try_into()?), 32_000);
    assert_eq!(u16::from_le_bytes(wav[34..36].try_into()?), 16);
    assert_eq!(&wav[36..40], b"data");
    assert_eq!(u32::from_le_bytes(wav[40..44].try_into()?), 8);
    assert_eq!(&wav[44..], [0, 0, 1, 0, 0xff, 0xff, 0xff, 0x7f]);
    Ok(())
}

/// The argument list is closed: exact flags, one argument per value, no translation.
#[test]
fn whisper_arguments_are_the_reviewed_closed_list() -> TestResult {
    let executable = TrustedExecutable::explicit(env::current_exe()?)?;
    let model = env::temp_dir().join(format!("vsift-whisper-argv-{}.bin", std::process::id()));
    fs::write(&model, b"model")?;
    let cli = WhisperCli::new(
        executable,
        &model,
        NonZeroU16::new(4).ok_or("zero")?,
        HostIsolation::ProcessOnly,
    );
    let canonical_model = fs::canonicalize(&model)?;
    fs::remove_file(&model)?;
    let cli = cli?;
    let work = Path::new("/work");
    let argv: Vec<String> = cli
        .argv(&work.join("chunk-00003.wav"), &work.join("chunk-00003"))
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    let expected = [
        "-m".to_owned(),
        canonical_model.to_string_lossy().into_owned(),
        "-f".to_owned(),
        work.join("chunk-00003.wav").to_string_lossy().into_owned(),
        "-of".to_owned(),
        work.join("chunk-00003").to_string_lossy().into_owned(),
    ]
    .into_iter()
    .chain(
        [
            "-ojf", "-np", "-l", "auto", "-t", "4", "-p", "1", "-bs", "5", "-bo", "5", "-sns",
            "-ng",
        ]
        .map(str::to_owned),
    )
    .collect::<Vec<_>>();
    assert_eq!(argv, expected);
    assert!(!argv.iter().any(|argument| argument == "-tr"));
    assert!(
        WhisperCli::new(
            TrustedExecutable::explicit(env::current_exe()?)?,
            Path::new("relative.bin"),
            NonZeroU16::MIN,
            HostIsolation::ProcessOnly,
        )
        .is_err()
    );
    Ok(())
}
