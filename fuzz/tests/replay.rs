//! Stable replay of every fuzz target over its committed seed corpus.
//!
//! libFuzzer needs nightly and runs on a schedule; these tests run the same
//! target bodies over every seed on the repository's stable toolchain, and keep
//! the seeds honest: each is either a byte-for-byte copy of an existing test
//! fixture or an inline test document, or is re-derived here from one.

use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    num::{NonZeroU16, NonZeroU32},
    path::{Path, PathBuf},
};

use vsift_application::{
    AsrRevisionRequest, AsrTranscription, build_asr_revision, whole_file_source_segment,
};
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, CursorToken, MediaTime, SessionId, Sha256Hex,
    SourceId, TimeRange, merge_chunks, plan_chunks, validate_chunk_output,
};
use vsift_fuzz::Target;
use vsift_infrastructure::{
    SourceContainer, WhisperOutputLimits, decode_transcript_record, encode_transcript_record,
    parse_ffprobe_metadata, parse_supplied_transcript, parse_whisper_full_json,
};

type TestResult = Result<(), Box<dyn Error>>;

/// Where a committed seed came from.
enum Origin {
    /// A byte-for-byte copy of this repository file.
    Copy(&'static str),
    /// An inline test document that appears verbatim in this repository file.
    InlineIn(&'static str),
    /// The version-2 record `encode_transcript_record` writes for the local-ASR
    /// revision built from the recorded F01 whisper output (see
    /// [`the_local_asr_record_seed_is_the_encoded_f01_revision`]).
    EncodedF01LocalAsr,
}

struct Seed {
    target: Target,
    file: &'static str,
    origin: Origin,
}

const TRANSCRIPT_DATA: &str = "crates/vsift-infrastructure/tests/data/transcripts";
const WHISPER_FIXTURES: &str = "crates/vsift-infrastructure/tests/fixtures/whisper-1.9.2";

const SEEDS: &[Seed] = &[
    seed(
        Target::TranscriptSrt,
        "F10.srt",
        Origin::Copy("fixtures/corpus/transcripts/F10.srt"),
    ),
    seed(
        Target::TranscriptSrt,
        "invalid-timestamp.srt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptSrt,
        "markup.srt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptSrt,
        "out-of-order.srt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptSrt,
        "reversed-timing.srt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptSrt,
        "untimed-text.srt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptWebVtt,
        "F10.vtt",
        Origin::Copy("fixtures/corpus/transcripts/F10.vtt"),
    ),
    seed(
        Target::TranscriptWebVtt,
        "markup.vtt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptWebVtt,
        "missing-arrow.vtt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptWebVtt,
        "overlapping.vtt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::WhisperFullJson,
        "F01.base.json",
        Origin::Copy(WHISPER_FIXTURES),
    ),
    seed(
        Target::WhisperFullJson,
        "F05.base.json",
        Origin::Copy(WHISPER_FIXTURES),
    ),
    seed(
        Target::WhisperFullJson,
        "F08.base.json",
        Origin::Copy(WHISPER_FIXTURES),
    ),
    seed(
        Target::WhisperFullJson,
        "F09.base.json",
        Origin::Copy(WHISPER_FIXTURES),
    ),
    seed(
        Target::TranscriptRecord,
        "bundle-transcript-record.json",
        Origin::Copy("schemas/v1/examples"),
    ),
    seed(
        Target::TranscriptRecord,
        "F01-local-asr-v2.json",
        Origin::EncodedF01LocalAsr,
    ),
    seed(
        Target::FfprobeMetadata,
        "F11-excessive-streams.json",
        Origin::Copy("fixtures/corpus/generated"),
    ),
    seed(
        Target::FfprobeMetadata,
        "F11-oversized-dimensions.json",
        Origin::Copy("fixtures/corpus/generated"),
    ),
    seed(
        Target::FfprobeMetadata,
        "two-streams-rotated.json",
        Origin::InlineIn("crates/vsift-infrastructure/src/ffmpeg_media.rs"),
    ),
    seed(
        Target::FfprobeMetadata,
        "audio-only-unknown-codec.json",
        Origin::InlineIn("crates/vsift-infrastructure/src/ffmpeg_media.rs"),
    ),
    seed(
        Target::TranscriptCursor,
        "transcript-get-next-cursor.txt",
        Origin::InlineIn("schemas/v1/examples/transcript-get.json"),
    ),
];

const fn seed(target: Target, file: &'static str, origin: Origin) -> Seed {
    Seed {
        target,
        file,
        origin,
    }
}

fn fuzz_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repository(relative: &str) -> PathBuf {
    fuzz_root().join("..").join(relative)
}

fn seed_directory(target: Target) -> PathBuf {
    fuzz_root().join("seeds").join(target.name())
}

fn seed_files(target: Target) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(seed_directory(target))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(format!("{} holds a non-file entry", target.name()).into());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "non-UTF-8 seed name")?;
        names.insert(name);
    }
    Ok(names)
}

/// Every target runs over every committed seed, and over empty input, without
/// a violation (a panic inside a parser fails the test too).
#[test]
fn every_seed_replays_without_a_violation() -> TestResult {
    for target in Target::ALL {
        target.check(&[])?;
        let names = seed_files(target)?;
        assert!(!names.is_empty(), "{} has no seeds", target.name());
        for name in names {
            let data = fs::read(seed_directory(target).join(&name))?;
            if let Err(violation) = target.check(&data) {
                return Err(format!("{}/{name}: {violation}", target.name()).into());
            }
        }
    }
    Ok(())
}

/// The well-formed seeds are accepted, so the targets' invariants are exercised
/// from the first run rather than only once the fuzzer finds a valid input.
#[test]
fn well_formed_seeds_are_accepted() -> TestResult {
    let accepted = [
        (Target::TranscriptSrt, "F10.srt"),
        (Target::TranscriptSrt, "markup.srt"),
        (Target::TranscriptWebVtt, "F10.vtt"),
        (Target::TranscriptWebVtt, "markup.vtt"),
        (Target::TranscriptWebVtt, "overlapping.vtt"),
        (Target::WhisperFullJson, "F01.base.json"),
        (Target::WhisperFullJson, "F05.base.json"),
        (Target::WhisperFullJson, "F08.base.json"),
        (Target::WhisperFullJson, "F09.base.json"),
        (Target::TranscriptRecord, "bundle-transcript-record.json"),
        (Target::TranscriptRecord, "F01-local-asr-v2.json"),
        (Target::FfprobeMetadata, "two-streams-rotated.json"),
        (Target::FfprobeMetadata, "audio-only-unknown-codec.json"),
        (Target::TranscriptCursor, "transcript-get-next-cursor.txt"),
    ];
    for (target, file) in accepted {
        let data = fs::read(seed_directory(target).join(file))?;
        let is_accepted = match target {
            Target::TranscriptSrt | Target::TranscriptWebVtt => {
                parse_supplied_transcript(&data).is_ok()
            }
            Target::WhisperFullJson => {
                parse_whisper_full_json(&data, WhisperOutputLimits::R0).is_ok()
            }
            Target::TranscriptRecord => decode_transcript_record(&data).is_ok(),
            Target::FfprobeMetadata => {
                parse_ffprobe_metadata(&data, SourceContainer::IsoMedia).is_ok()
            }
            Target::TranscriptCursor => CursorToken::parse(std::str::from_utf8(&data)?).is_ok(),
        };
        assert!(is_accepted, "{}/{file} is rejected", target.name());
    }
    Ok(())
}

/// The seed directories hold exactly the listed seeds, and each listed copy or
/// inline document still matches the fixture it was taken from.
#[test]
fn seeds_are_listed_and_match_their_fixtures() -> TestResult {
    for target in Target::ALL {
        let listed: BTreeSet<String> = SEEDS
            .iter()
            .filter(|seed| seed.target == target)
            .map(|seed| seed.file.to_owned())
            .collect();
        assert!(
            listed == seed_files(target)?,
            "{} seeds differ from the list",
            target.name()
        );
    }
    for seed in SEEDS {
        let data = fs::read(seed_directory(seed.target).join(seed.file))?;
        let matches = match seed.origin {
            Origin::Copy(path) => {
                let path = repository(path);
                let source = if path.is_dir() {
                    path.join(seed.file)
                } else {
                    path
                };
                fs::read(source)? == data
            }
            Origin::InlineIn(path) => {
                fs::read_to_string(repository(path))?.contains(std::str::from_utf8(&data)?)
            }
            Origin::EncodedF01LocalAsr => true,
        };
        assert!(matches, "{} no longer matches its origin", seed.file);
    }
    Ok(())
}

/// The version-2 record seed is what the encoder writes for the local-ASR
/// revision built from the recorded F01 output, as in the infrastructure
/// crate's own round-trip test, so the fuzzer starts from a valid record.
#[test]
fn the_local_asr_record_seed_is_the_encoded_f01_revision() -> TestResult {
    let source_id =
        SourceId::from_sha256("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(6_000_000))?;
    let planned = plan_chunks(source.id(), source.range(), ChunkPlan::R0)?;
    let first = planned.first().ok_or("no planned chunk")?;
    let audio = TimeRange::new(MediaTime::from_micros(0), MediaTime::from_micros(6_000_000))?;
    let output = parse_whisper_full_json(
        &fs::read(repository(WHISPER_FIXTURES).join("F01.base.json"))?,
        WhisperOutputLimits::R0,
    )?;
    let validated = validate_chunk_output(first, audio, source.range(), output)?;
    let (segments, language, warnings) = validated.into_parts();
    let merged = merge_chunks(&[segments]);
    let digest =
        Sha256Hex::parse("95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d")?;
    let run = AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, digest.clone()),
        model: AsrModel::new(AsrModelProfile::Base, digest),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero threads")?,
        audio_stream: 1,
        chunks: vec![AsrChunkRecord::new(
            first.clone(),
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
        splice: None,
    })?;
    let encoded = encode_transcript_record(&revision)?;
    let committed =
        fs::read(seed_directory(Target::TranscriptRecord).join("F01-local-asr-v2.json"))?;
    assert!(
        committed == encoded,
        "regenerate the seed: the encoder no longer writes the committed bytes"
    );
    Ok(())
}

/// Each target has a libFuzzer entry point of the same name.
#[test]
fn every_target_has_a_libfuzzer_entry_point() -> TestResult {
    let manifest = fs::read_to_string(fuzz_root().join("Cargo.toml"))?;
    for target in Target::ALL {
        let entry = Path::new("fuzz_targets").join(format!("{}.rs", target.name()));
        assert!(
            fuzz_root().join(&entry).is_file(),
            "{} is missing",
            entry.display()
        );
        assert!(
            manifest.contains(&format!("name = \"{}\"", target.name())),
            "{} has no [[bin]]",
            target.name()
        );
    }
    Ok(())
}
