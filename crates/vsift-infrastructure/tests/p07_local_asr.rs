//! Opt-in local-ASR adapter check over the committed P07 speech clips.
//!
//! Runs F01, F05, F08 and F09 through the real chain: staged snapshot bound
//! for the run (hashed once, identity-checked per call, hashed again at the
//! end), `FFprobe` description, `FfmpegMedia::speech_pcm` chunks, whisper.cpp with the pinned
//! decoding profile, `-ojf` parsing, domain validation and seam merge, then
//! builds a local-ASR revision and round-trips its version-2 record. It needs
//! `FFmpeg` and `FFprobe` on `PATH` and two environment variables:
//!
//! ```console
//! VSIFT_TEST_WHISPER_CLI=<absolute whisper-cli path>
//! VSIFT_TEST_WHISPER_MODEL=<absolute ggml model path>
//! cargo test -p vsift-infrastructure --locked --test p07_local_asr -- --ignored --nocapture
//! ```
//!
//! Timing assertions are deliberately loose: the base model reports coarse
//! segment times and may format numbers differently from the script.

use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs,
    num::{NonZeroU16, NonZeroU32},
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use vsift_application::{
    AsrRevisionRequest, InitializeSessionStorage, InitializeSessionStorageRequest,
    SpeechRecognizer, TranscribeRangeRequest, build_asr_revision, transcribe_range,
    whole_file_source_segment,
};
use vsift_domain::{
    AsrChunkOutcome, ChunkPlan, DurabilityRequirement, MediaSelection, OperationId, SessionId,
};
use vsift_infrastructure::{
    BoundSource, ExecutableResolver, FfmpegMedia, FfmpegSpeechAudio, FilesystemSessionStore,
    HostIsolation, MediaProviderConformance, ProcessCancellation, ProcessWorkingDirectory,
    SourceSnapshot, TrustedExecutable, WhisperCli, WhisperSpeechRecognizer,
    decode_transcript_record, encode_transcript_record,
};

type TestResult = Result<(), Box<dyn Error>>;

struct OwnedRoot(PathBuf);

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("vsift-p07-asr-"))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn required_path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| format!("set {name} to an absolute path").into())
}

/// The generator's exact speech span for a clip, from the committed provenance.
fn speech_span(fixture: &str) -> Result<(u64, u64), Box<dyn Error>> {
    let provenance: serde_json::Value = serde_json::from_slice(&fs::read(repository(
        "fixtures/corpus/generated/speech-provenance.json",
    ))?)?;
    let variant = provenance["assembly"]["variants"]
        .as_array()
        .and_then(|variants| {
            variants
                .iter()
                .find(|variant| variant["fixture"] == fixture)
        })
        .ok_or("fixture missing from speech provenance")?;
    Ok((
        variant["speech_start_us"].as_u64().ok_or("no start")?,
        variant["speech_end_us"].as_u64().ok_or("no end")?,
    ))
}

#[tokio::test]
#[ignore = "requires VSIFT_TEST_WHISPER_CLI, VSIFT_TEST_WHISPER_MODEL and FFmpeg/FFprobe on PATH"]
#[allow(clippy::too_many_lines)] // One end-to-end journey per clip, reported as it runs.
async fn speech_clips_are_transcribed_through_the_real_adapters() -> TestResult {
    let whisper = TrustedExecutable::explicit(required_path("VSIFT_TEST_WHISPER_CLI")?)?;
    let model = required_path("VSIFT_TEST_WHISPER_MODEL")?;
    let resolver = ExecutableResolver::from_current_path();
    let ffmpeg = resolver.resolve(OsStr::new("ffmpeg"))?;
    let ffprobe = resolver.resolve(OsStr::new("ffprobe"))?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root =
        OwnedRoot(env::temp_dir().join(format!("vsift-p07-asr-{}-{stamp}", std::process::id())));
    let store_root = root.0.join("sessions");
    let workspace_path = root.0.join("asr-work");
    fs::create_dir_all(&workspace_path)?;
    let workspace = ProcessWorkingDirectory::new(&workspace_path)?;
    let store = FilesystemSessionStore::provision_default(&store_root)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    let store = FilesystemSessionStore::open_existing(&store_root)?;
    let media = FfmpegMedia::new(
        MediaProviderConformance::r0(ffmpeg, ffprobe),
        HostIsolation::ProcessOnly,
        &store,
    );
    let recognizer = WhisperSpeechRecognizer::new(
        WhisperCli::new(
            whisper,
            &model,
            NonZeroU16::new(4).ok_or("zero")?,
            HostIsolation::ProcessOnly,
        )?,
        workspace,
        ProcessCancellation::new(),
    );
    let expected = recognizer.identity().await?;
    println!(
        "recognizer provider={} model_profile={} decoding={}",
        expected.provider.executable_sha256().as_str(),
        expected.model.profile().identifier(),
        expected.decoding.identifier()
    );

    for (index, (fixture, file, required)) in [
        (
            "F01",
            "F01-speech.mp4",
            &["service status", "healthy", "2048"][..],
        ),
        ("F05", "F05-speech.mp4", &["submit", "error"][..]),
        ("F08", "F08-speech.mp4", &["731", "identificador"][..]),
        ("F09", "F09-speech.mkv", &["visible"][..]),
    ]
    .into_iter()
    .enumerate()
    {
        let started = Instant::now();
        let bound = BoundSource::bind(SourceSnapshot::stage(
            &store,
            &session_id,
            &OperationId::parse(format!("op_{:016x}", index + 1))?,
            &repository(&format!("fixtures/corpus/generated/{file}")),
        )?)?;
        let description = media.probe(&bound, ProcessCancellation::new()).await?;
        let selection = MediaSelection {
            video: Some(0),
            audio: Some(1),
        };
        let audio = FfmpegSpeechAudio::new(
            &media,
            &bound,
            &description,
            selection,
            ProcessCancellation::new(),
        );
        let source = whole_file_source_segment(bound.snapshot().id(), description.duration)?;
        let transcription = transcribe_range(
            TranscribeRangeRequest {
                source_segment: &source,
                range: source.range(),
                plan: ChunkPlan::R0,
                audio_stream: 1,
                expected: &expected,
            },
            &audio,
            &recognizer,
            &ProcessCancellation::new(),
        )
        .await?;
        drop(audio);
        let snapshot = bound.release_verified()?;
        let outcomes: Vec<String> = transcription
            .run
            .chunks()
            .iter()
            .map(|record| match record.outcome() {
                AsrChunkOutcome::Transcribed { audio } => format!(
                    "transcribed[{}..{}]",
                    audio.start().as_micros(),
                    audio.end().as_micros()
                ),
                AsrChunkOutcome::Silent { .. } => "silent".to_owned(),
                AsrChunkOutcome::NoAudio => "no_audio".to_owned(),
            })
            .collect();
        let text = transcription
            .segments
            .iter()
            .map(|merged| merged.segment.text().text().to_owned())
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{fixture}: {} ms, chunks {outcomes:?}, language {:?}, text {text:?}",
            started.elapsed().as_millis(),
            transcription
                .language
                .as_ref()
                .map(vsift_domain::LanguageTag::as_str),
        );
        for merged in &transcription.segments {
            println!(
                "  [{}..{}) chunk {} confidence {:?}",
                merged.segment.range().start().as_micros(),
                merged.segment.range().end().as_micros(),
                merged.chunk,
                merged.segment.confidence().basis_points()
            );
        }
        let lowered = text.to_lowercase();
        for phrase in required {
            if !lowered.contains(phrase) {
                return Err(format!("{fixture} transcript lacks {phrase:?}").into());
            }
        }
        let (speech_start, speech_end) = speech_span(fixture)?;
        if !transcription.segments.iter().all(|merged| {
            let range = merged.segment.range();
            range.start().as_micros() < speech_end + 1_000_000
                && range.end().as_micros() + 1_000_000 > speech_start
        }) {
            return Err(format!("{fixture} has a segment away from its speech").into());
        }
        let revision = build_asr_revision(AsrRevisionRequest {
            session_id: &session_id,
            source_id: snapshot.id(),
            source_segment: &source,
            number: NonZeroU32::MIN,
            transcription,
            splice: None,
        })?;
        let encoded = encode_transcript_record(&revision)?;
        if decode_transcript_record(&encoded)? != revision {
            return Err(format!("{fixture} record did not round-trip").into());
        }
        if fs::read_dir(&workspace_path)?.next().is_some() {
            return Err("chunk files were left in the work directory".into());
        }
    }
    Ok(())
}
