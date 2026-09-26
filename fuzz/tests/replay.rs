//! Stable replay of every fuzz target over its committed seed corpus.
//!
//! libFuzzer needs nightly and runs on a schedule; these tests run the same
//! target bodies over every seed on the repository's stable toolchain, and keep
//! the seeds honest: each is either a byte-for-byte copy of an existing test
//! fixture or an inline test document, or is re-derived here from one.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fmt::Write as _,
    fs,
    future::Future,
    num::{NonZeroU16, NonZeroU32},
    path::{Path, PathBuf},
    pin::pin,
    task::{Context, Poll, Waker},
};

use serde_json::Value;
use vsift_application::{
    AsrRevisionRequest, AsrTranscription, ExtendVisualIndexRequest, VisualIndexScope,
    VisualSampler, VisualSamplingError, build_asr_revision, extend_visual_index,
    whole_file_source_segment,
};
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, CursorToken, MediaTime, SearchMatch,
    SearchQuery, SessionId, Sha256Hex, SourceId, TimeRange, VISUAL_BLOCKS, VISUAL_FRAME_BYTES,
    VisualHash, VisualIndexProfile, VisualSample, VisualWindow, merge_chunks, plan_chunks,
    validate_chunk_output,
};
use vsift_fuzz::{Target, VISUAL_FUZZ_SESSION, visual_samples_input};
use vsift_infrastructure::{
    SourceContainer, VisualSamplingWindow, WhisperOutputLimits, decode_transcript_record,
    decode_visual_index_record, encode_transcript_record, encode_visual_index_record,
    parse_ffprobe_metadata, parse_supplied_transcript, parse_visual_samples,
    parse_whisper_full_json,
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
    /// A visual-samples input whose diagnostics carry the recorded sample
    /// times of this fixture (see [`the_visual_seeds_derive_from_the_recorded_samples`]).
    RecordedShowinfo(&'static str),
    /// The visual-index record encoded from this fixture's recorded samples.
    RecordedVisualIndex(&'static str),
    /// A `search_query` input: a query that appears verbatim in the first
    /// file, a line feed, and segment text that appears verbatim in the second.
    SearchPair {
        query: &'static str,
        query_in: &'static str,
        text: &'static str,
        text_in: &'static str,
    },
}

struct Seed {
    target: Target,
    file: &'static str,
    origin: Origin,
}

const TRANSCRIPT_DATA: &str = "crates/vsift-infrastructure/tests/data/transcripts";
const WHISPER_FIXTURES: &str = "crates/vsift-infrastructure/tests/fixtures/whisper-1.9.2";
const VISUAL_SAMPLES: &str = "crates/vsift-infrastructure/tests/data/visual_samples";

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
        Target::TranscriptRecord,
        "bundle-transcript-record.asr.json",
        Origin::Copy("schemas/v1/examples"),
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
    seed(
        Target::VisualSamples,
        "F01-showinfo.bin",
        Origin::RecordedShowinfo("F01"),
    ),
    seed(
        Target::VisualSamples,
        "F09-showinfo.bin",
        Origin::RecordedShowinfo("F09"),
    ),
    seed(
        Target::VisualIndexRecord,
        "F06-index.json",
        Origin::RecordedVisualIndex("F06"),
    ),
    seed(
        Target::VisualIndexRecord,
        "F10-index.json",
        Origin::RecordedVisualIndex("F10"),
    ),
    seed(
        Target::SearchQuery,
        "f10-r-17.txt",
        Origin::SearchPair {
            query: "R-17",
            query_in: "fixtures/corpus/manifest.json",
            text: "Dialog R-17 is displayed now.",
            text_in: "fixtures/corpus/transcripts/F10.srt",
        },
    ),
    seed(
        Target::SearchQuery,
        "f10-dialog-r-17.txt",
        Origin::SearchPair {
            query: "dialog r 17",
            query_in: "crates/vsift-contract/tests/search_contract.rs",
            text: "Dialog R-17 is displayed now.",
            text_in: "fixtures/corpus/transcripts/F10.srt",
        },
    ),
    seed(
        Target::SearchQuery,
        "f01-build-2048.txt",
        Origin::SearchPair {
            query: "build 2,048",
            query_in: "crates/vsift-contract/tests/search_contract.rs",
            text: "The service status is healthy and the build is 2048.",
            text_in: "crates/vsift-infrastructure/tests/fixtures/whisper-1.9.2/F01.base.json",
        },
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
        (Target::VisualSamples, "F01-showinfo.bin"),
        (Target::VisualSamples, "F09-showinfo.bin"),
        (Target::VisualIndexRecord, "F06-index.json"),
        (Target::VisualIndexRecord, "F10-index.json"),
        (Target::SearchQuery, "f10-r-17.txt"),
        (Target::SearchQuery, "f10-dialog-r-17.txt"),
        (Target::SearchQuery, "f01-build-2048.txt"),
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
            Target::VisualSamples => match data.as_slice() {
                [count, _, stderr @ ..] => parse_visual_samples(
                    &vec![0_u8; usize::from(*count) * VISUAL_FRAME_BYTES],
                    stderr,
                    &VisualSamplingWindow::new(
                        0,
                        0,
                        MediaTime::from_micros(0),
                        MediaTime::from_micros(60_000_000),
                    )?,
                )
                .is_ok_and(|frames| frames.len() == usize::from(*count)),
                _ => false,
            },
            Target::VisualIndexRecord => {
                decode_visual_index_record(&data, &SessionId::parse(VISUAL_FUZZ_SESSION)?).is_ok()
            }
            Target::SearchQuery => {
                let (query, text) = std::str::from_utf8(&data)?
                    .split_once('\n')
                    .ok_or("no line feed")?;
                // Each seed matches: two as a phrase, one as all terms.
                SearchQuery::parse(query)
                    .ok()
                    .and_then(|query| query.classify(text))
                    .is_some_and(|tier| SearchMatch::ALL.contains(&tier))
            }
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
            Origin::EncodedF01LocalAsr
            | Origin::RecordedShowinfo(_)
            | Origin::RecordedVisualIndex(_) => true,
            Origin::SearchPair {
                query,
                query_in,
                text,
                text_in,
            } => {
                data == format!("{query}\n{text}").as_bytes()
                    && fs::read_to_string(repository(query_in))?.contains(query)
                    && fs::read_to_string(repository(text_in))?.contains(text)
            }
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

fn recorded_samples(fixture: &str) -> Result<Vec<VisualSample>, Box<dyn Error>> {
    let value: Value = serde_json::from_slice(&fs::read(
        repository(VISUAL_SAMPLES).join(format!("{fixture}.json")),
    )?)?;
    let mut samples = Vec::new();
    for window in value["windows"].as_array().ok_or("no windows")? {
        for sample in window["samples"].as_array().ok_or("no samples")? {
            let text = sample["blocks"].as_str().ok_or("no blocks")?;
            let mut blocks = [0_u8; VISUAL_BLOCKS];
            for (position, block) in blocks.iter_mut().enumerate() {
                *block = u8::from_str_radix(
                    text.get(position * 2..position * 2 + 2)
                        .ok_or("short blocks")?,
                    16,
                )?;
            }
            samples.push(VisualSample::from_parts(
                MediaTime::from_micros(sample["time_us"].as_u64().ok_or("no time")?),
                blocks,
                VisualHash::parse_hex(sample["hash"].as_str().ok_or("no hash")?)?,
            ));
        }
    }
    Ok(samples)
}

/// `showinfo` diagnostics for the recorded sample times, in `FFmpeg` 9.0's
/// line format, with the fixture's stream time base (Matroska's 1/1000 for
/// F09, the MP4 fixtures' 1/10240 otherwise).
fn showinfo_seed(fixture: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let samples = recorded_samples(fixture)?;
    let denominator: u64 = if fixture == "F09" { 1_000 } else { 10_240 };
    let mut text = String::new();
    writeln!(
        text,
        "[Parsed_showinfo_3 @ 0000000000000000] config in time_base: 1/{denominator}, frame_rate: 20/1"
    )?;
    writeln!(
        text,
        "[Parsed_showinfo_3 @ 0000000000000000] config out time_base: 0/0, frame_rate: 0/0"
    )?;
    for (number, sample) in samples.iter().enumerate() {
        let micros = sample.time().as_micros();
        let pts = micros * denominator / 1_000_000;
        writeln!(
            text,
            "[Parsed_showinfo_3 @ 0000000000000000] n:{number:>4} pts:{pts:>7} pts_time:{}.{:06} fmt:gray s:128x72 i:P iskey:0 type:P",
            micros / 1_000_000,
            micros % 1_000_000
        )?;
    }
    Ok(visual_samples_input(
        u8::try_from(samples.len())?,
        0x40,
        text.as_bytes(),
    ))
}

/// Replays recorded samples; every window of a short fixture is window 0.
struct Recorded(Vec<VisualSample>);

impl VisualSampler for Recorded {
    fn window_samples(
        &self,
        _window: VisualWindow,
    ) -> impl Future<Output = Result<Vec<VisualSample>, VisualSamplingError>> + Send {
        std::future::ready(Ok(self.0.clone()))
    }
}

/// Completes a future that never waits, such as an extension over a
/// sampler whose answers are ready, without an async runtime.
fn ready<F: Future>(future: F) -> Result<F::Output, Box<dyn Error>> {
    let mut context = Context::from_waker(Waker::noop());
    match pin!(future).poll(&mut context) {
        Poll::Ready(output) => Ok(output),
        Poll::Pending => Err("the future waited".into()),
    }
}

fn visual_index_seed(fixture: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let samples = recorded_samples(fixture)?;
    let value: Value = serde_json::from_slice(&fs::read(
        repository(VISUAL_SAMPLES).join(format!("{fixture}.json")),
    )?)?;
    let duration = MediaTime::from_micros(value["duration_us"].as_u64().ok_or("no duration")?);
    let session = SessionId::parse(VISUAL_FUZZ_SESSION)?;
    let source = SourceId::from_sha256(value["source_sha256"].as_str().ok_or("no source")?)?;
    let extension = ready(extend_visual_index(
        ExtendVisualIndexRequest {
            scope: VisualIndexScope {
                session_id: &session,
                source_id: &source,
                stream_index: 0,
                duration,
                profile: VisualIndexProfile::R0,
            },
            previous: None,
            range: TimeRange::new(MediaTime::from_micros(0), duration)?,
        },
        &Recorded(samples),
    ))??;
    let index = extension.revision.ok_or("no revision")?;
    Ok(encode_visual_index_record(&session, &index)?)
}

/// The visual seeds are re-derived from the recorded samples of
/// `crates/vsift-infrastructure/tests/data/visual_samples/`. After a
/// re-recording or an encoder change, run this test with
/// `VSIFT_REGENERATE_FUZZ_SEEDS=1` to rewrite them, and review the diff.
#[test]
fn the_visual_seeds_derive_from_the_recorded_samples() -> TestResult {
    let regenerate = env::var("VSIFT_REGENERATE_FUZZ_SEEDS").is_ok_and(|value| value == "1");
    for seed in SEEDS {
        let derived = match seed.origin {
            Origin::RecordedShowinfo(fixture) => showinfo_seed(fixture)?,
            Origin::RecordedVisualIndex(fixture) => visual_index_seed(fixture)?,
            Origin::Copy(_)
            | Origin::InlineIn(_)
            | Origin::EncodedF01LocalAsr
            | Origin::SearchPair { .. } => continue,
        };
        let path = seed_directory(seed.target).join(seed.file);
        if regenerate {
            fs::create_dir_all(seed_directory(seed.target))?;
            fs::write(&path, &derived)?;
        }
        assert!(
            fs::read(&path)? == derived,
            "{} no longer derives from the recorded samples",
            seed.file
        );
    }
    Ok(())
}
