//! whisper.cpp CLI adapter for local speech recognition.
//!
//! One chunk is written as a 16 kHz mono WAV file in a VSift-owned work
//! directory and recognized by one supervised `whisper-cli` run with a closed
//! argument list. The run's `-ojf` JSON file is the only output read: stdout
//! and stderr are bounded and ignored, because they carry absolute paths and
//! loader logs rather than results. The JSON is untrusted. It is size-checked
//! before it is read, opened without following links, and reduced to the few
//! fields recognition needs (detected language, segment offsets and text, and
//! token probabilities); the model path and system information it also
//! contains are never parsed into a value, stored or logged.
//!
//! The flags were checked against `whisper-cli --help` of the reviewed v1.9.2
//! build; the relevant help lines are recorded in
//! `crates/vsift-infrastructure/tests/fixtures/whisper-1.9.2/help-flags.txt`.
//! Translation (`-tr`) is never requested: evidence stays in the spoken language.

use std::{
    error::Error,
    ffi::OsString,
    fmt, io,
    io::{Read, Write},
    num::{NonZeroU16, NonZeroUsize},
    path::{Path, PathBuf},
    time::Duration,
};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use serde::Deserialize;
use vsift_application::{
    AsrCancellation, RecognizerIdentity, SpeechPcm, SpeechRecognitionError, SpeechRecognizer,
};
use vsift_domain::{
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild, ChunkTime,
    CueText, LanguageTag, PlannedChunk, ProviderChunkOutput, ProviderSegment, ProviderToken,
    ProviderTokenKind, SPEECH_SAMPLE_RATE, Sha256Hex, TranscriptRejection,
};

use crate::{
    HostIsolation, ProcessCancellation, ProcessError, ProcessRequest, ProcessRequestError,
    ProcessSupervisor, ProcessWorkingDirectory, SupervisorPolicy, TerminationReason,
    TrustedExecutable, identify_whisper_build, pinned_whisper_model,
    whisper_build::hash_file_bounded, whisper_build::hex,
};

/// Retained standard output of one run; whisper prints its transcript there,
/// which the adapter ignores in favour of the JSON file.
pub const WHISPER_STDOUT_LIMIT: usize = 256 * 1024;
/// Retained standard error of one run; loader and progress logs, ignored.
pub const WHISPER_STDERR_LIMIT: usize = 64 * 1024;
/// Deadline for recognizing one chunk of at most thirty seconds on a CPU.
pub const WHISPER_CHUNK_DEADLINE: Duration = Duration::from_secs(120);
/// Largest whisper model file the adapter will hash for provenance.
pub const MAX_WHISPER_MODEL_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Bounds on one chunk's `-ojf` JSON output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WhisperOutputLimits {
    /// Largest output file, checked before it is read.
    pub max_bytes: u64,
    /// Most segments.
    pub max_segments: usize,
    /// Most tokens in one segment.
    pub max_tokens: usize,
}

impl WhisperOutputLimits {
    /// R0 bounds: 4 MiB, 256 segments, 512 tokens per segment. Thirty seconds
    /// of speech is a few dozen segments and well under 100 KiB of JSON.
    pub const R0: Self = Self {
        max_bytes: 4 * 1024 * 1024,
        max_segments: vsift_domain::MAX_PROVIDER_SEGMENTS,
        max_tokens: vsift_domain::MAX_PROVIDER_TOKENS,
    };
}

/// Why a whisper `-ojf` JSON document was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WhisperOutputError {
    /// The document exceeds [`WhisperOutputLimits::max_bytes`].
    TooLarge,
    /// The document is not valid UTF-8.
    InvalidUtf8,
    /// The document is not the documented JSON shape.
    Unparseable,
    /// More segments than [`WhisperOutputLimits::max_segments`].
    TooManySegments,
    /// A segment has more tokens than [`WhisperOutputLimits::max_tokens`].
    TooManyTokens,
    /// Segment text contains a control character or exceeds the cue-text bound.
    InvalidText(TranscriptRejection),
}

impl fmt::Display for WhisperOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => formatter.write_str("whisper output exceeds its size bound"),
            Self::InvalidUtf8 => formatter.write_str("whisper output is not valid UTF-8"),
            Self::Unparseable => formatter.write_str("whisper output is not the expected JSON"),
            Self::TooManySegments => formatter.write_str("whisper output has too many segments"),
            Self::TooManyTokens => formatter.write_str("whisper segment has too many tokens"),
            Self::InvalidText(rejection) => {
                write!(formatter, "whisper segment text was rejected ({rejection})")
            }
        }
    }
}

impl Error for WhisperOutputError {}

#[derive(Deserialize)]
struct RawOutput {
    result: Option<RawResult>,
    transcription: Vec<RawSegment>,
}

#[derive(Deserialize)]
struct RawResult {
    language: Option<String>,
}

#[derive(Deserialize)]
struct RawSegment {
    offsets: RawOffsets,
    text: String,
    #[serde(default)]
    tokens: Vec<RawToken>,
}

#[derive(Deserialize)]
struct RawOffsets {
    from: u64,
    to: u64,
}

#[derive(Deserialize)]
struct RawToken {
    text: String,
    p: f64,
}

/// Parses one chunk's whisper `-ojf` JSON under `limits`.
///
/// Only `result.language`, `transcription[].offsets.{from,to}` (milliseconds
/// relative to the chunk's first sample), `transcription[].text` and
/// `transcription[].tokens[].{text,p}` are read; every other field, including
/// the model path and system information, is skipped without being retained.
/// Token text is used only to tell special tokens from text tokens. Segment
/// text is trimmed and must pass the cue-text rules; whitespace-only text
/// becomes `None`. Probabilities are passed through for the domain to judge.
///
/// # Errors
///
/// Returns the first [`WhisperOutputError`]; parsing never partially succeeds.
pub fn parse_whisper_full_json(
    bytes: &[u8],
    limits: WhisperOutputLimits,
) -> Result<ProviderChunkOutput, WhisperOutputError> {
    if u64::try_from(bytes.len()).map_or(true, |length| length > limits.max_bytes) {
        return Err(WhisperOutputError::TooLarge);
    }
    std::str::from_utf8(bytes).map_err(|_| WhisperOutputError::InvalidUtf8)?;
    let raw: RawOutput =
        serde_json::from_slice(bytes).map_err(|_| WhisperOutputError::Unparseable)?;
    if raw.transcription.len() > limits.max_segments {
        return Err(WhisperOutputError::TooManySegments);
    }
    let mut segments = Vec::with_capacity(raw.transcription.len());
    for segment in raw.transcription {
        if segment.tokens.len() > limits.max_tokens {
            return Err(WhisperOutputError::TooManyTokens);
        }
        let trimmed = segment.text.trim();
        let text = if trimmed.is_empty() {
            None
        } else {
            Some(
                CueText::new(trimmed.to_owned(), trimmed.to_owned())
                    .map_err(WhisperOutputError::InvalidText)?,
            )
        };
        segments.push(ProviderSegment {
            start: ChunkTime::from_millis(segment.offsets.from)
                .ok_or(WhisperOutputError::Unparseable)?,
            end: ChunkTime::from_millis(segment.offsets.to)
                .ok_or(WhisperOutputError::Unparseable)?,
            text,
            tokens: segment
                .tokens
                .into_iter()
                .map(|token| ProviderToken {
                    kind: ProviderTokenKind::classify(&token.text),
                    probability: token.p,
                })
                .collect(),
        });
    }
    Ok(ProviderChunkOutput {
        language: raw
            .result
            .and_then(|result| result.language)
            .and_then(|tag| LanguageTag::parse(tag).ok()),
        segments,
    })
}

/// Encodes mono 16 kHz signed 16-bit samples as a canonical 44-byte-header WAV.
///
/// # Errors
///
/// Returns [`WhisperError::AudioTooLong`] when the data size does not fit a
/// WAV header; speech chunks are at most thirty seconds, far below it.
pub fn encode_speech_wav(samples: &[i16]) -> Result<Vec<u8>, WhisperError> {
    let data_bytes = samples
        .len()
        .checked_mul(2)
        .and_then(|bytes| u32::try_from(bytes).ok())
        .ok_or(WhisperError::AudioTooLong)?;
    let riff_bytes = data_bytes
        .checked_add(36)
        .ok_or(WhisperError::AudioTooLong)?;
    let mut wav = Vec::with_capacity(samples.len() * 2 + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_bytes.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&SPEECH_SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SPEECH_SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(wav)
}

/// Why one whisper run could not produce usable output.
#[derive(Debug)]
pub enum WhisperError {
    /// The model path is not an absolute, existing regular file.
    InvalidModel,
    /// The chunk's audio does not fit a WAV file.
    AudioTooLong,
    /// A chunk file from an earlier run is still in the work directory.
    StaleChunkFile,
    /// The work directory or a chunk file could not be used.
    Workspace(io::Error),
    /// The provider request could not be validated.
    Request(ProcessRequestError),
    /// The provider process could not be supervised.
    Process(ProcessError),
    /// The run exceeded its deadline.
    Deadline,
    /// The run was cancelled.
    Cancelled,
    /// The provider wrote more than its output bound.
    OutputLimit,
    /// The provider exited unsuccessfully or wrote no JSON output.
    ProviderFailed,
    /// The JSON output was rejected.
    Output(WhisperOutputError),
}

impl fmt::Display for WhisperError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidModel => "whisper model file is invalid",
            Self::AudioTooLong => "speech chunk is too long for a WAV file",
            Self::StaleChunkFile => "a stale whisper chunk file is in the work directory",
            Self::Workspace(_) => "whisper work directory could not be used",
            Self::Request(_) => "whisper request is invalid",
            Self::Process(_) => "whisper process could not run",
            Self::Deadline => "whisper exceeded its chunk deadline",
            Self::Cancelled => "whisper was cancelled",
            Self::OutputLimit => "whisper exceeded its output bound",
            Self::ProviderFailed => "whisper did not complete",
            Self::Output(_) => "whisper output was rejected",
        })
    }
}

impl Error for WhisperError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::Request(error) => Some(error),
            Self::Process(error) => Some(error),
            Self::Output(error) => Some(error),
            _ => None,
        }
    }
}

impl From<&WhisperError> for SpeechRecognitionError {
    fn from(error: &WhisperError) -> Self {
        match error {
            WhisperError::InvalidModel => Self::ModelUnavailable,
            WhisperError::Deadline => Self::Deadline,
            WhisperError::Cancelled => Self::Cancelled,
            WhisperError::OutputLimit
            | WhisperError::AudioTooLong
            | WhisperError::Output(
                WhisperOutputError::TooLarge
                | WhisperOutputError::TooManySegments
                | WhisperOutputError::TooManyTokens,
            ) => Self::ResourceLimit,
            WhisperError::ProviderFailed => Self::ProviderFailed,
            WhisperError::Output(
                WhisperOutputError::InvalidUtf8
                | WhisperOutputError::Unparseable
                | WhisperOutputError::InvalidText(_),
            ) => Self::UnparseableOutput,
            WhisperError::StaleChunkFile
            | WhisperError::Workspace(_)
            | WhisperError::Request(_)
            | WhisperError::Process(_) => Self::Io,
        }
    }
}

/// A whisper.cpp CLI with a fixed model and decoding profile.
#[derive(Clone, Debug)]
pub struct WhisperCli {
    executable: TrustedExecutable,
    model: PathBuf,
    threads: NonZeroU16,
    host_isolation: HostIsolation,
}

impl WhisperCli {
    /// Selects a trusted executable and a model file for the R0 decoding profile.
    ///
    /// # Errors
    ///
    /// Returns [`WhisperError::InvalidModel`] unless `model` is an absolute
    /// path to an existing regular file; it is canonicalized so the provider
    /// is always given one exact file.
    pub fn new(
        executable: TrustedExecutable,
        model: &Path,
        threads: NonZeroU16,
        host_isolation: HostIsolation,
    ) -> Result<Self, WhisperError> {
        if !model.is_absolute() {
            return Err(WhisperError::InvalidModel);
        }
        let model = std::fs::canonicalize(model).map_err(|_| WhisperError::InvalidModel)?;
        if !std::fs::metadata(&model).is_ok_and(|metadata| metadata.is_file()) {
            return Err(WhisperError::InvalidModel);
        }
        Ok(Self {
            executable,
            model,
            threads,
            host_isolation,
        })
    }

    /// The closed argument list for one chunk (decoding profile `r0-v1`).
    ///
    /// `-ojf` writes `<output_base>.json` with token probabilities; `-np`
    /// suppresses progress output; `-l auto` detects the spoken language and
    /// never translates; `-p 1 -bs 5 -bo 5` is one processor with beam search
    /// 5 and best-of 5; `-sns` suppresses non-speech tokens; `-ng` keeps
    /// recognition on the CPU so results do not depend on a GPU driver.
    #[must_use]
    pub fn argv(&self, chunk_wav: &Path, output_base: &Path) -> Vec<OsString> {
        let mut arguments: Vec<OsString> = Vec::with_capacity(20);
        arguments.push("-m".into());
        arguments.push(self.model.clone().into_os_string());
        arguments.push("-f".into());
        arguments.push(chunk_wav.as_os_str().to_owned());
        arguments.push("-of".into());
        arguments.push(output_base.as_os_str().to_owned());
        for flag in ["-ojf", "-np", "-l", "auto", "-t"] {
            arguments.push(flag.into());
        }
        arguments.push(self.threads.get().to_string().into());
        for flag in ["-p", "1", "-bs", "5", "-bo", "5", "-sns", "-ng"] {
            arguments.push(flag.into());
        }
        arguments
    }

    /// Recognizes one chunk's samples in `workspace` and returns the parsed output.
    ///
    /// The chunk's WAV and JSON files are created fresh (an existing file of
    /// either name fails the run rather than being reused) and removed again
    /// whatever the outcome.
    ///
    /// # Errors
    ///
    /// Returns a typed [`WhisperError`] for workspace, process or output failures.
    pub async fn transcribe_chunk(
        &self,
        workspace: &ProcessWorkingDirectory,
        index: u32,
        samples: &[i16],
        cancellation: ProcessCancellation,
    ) -> Result<ProviderChunkOutput, WhisperError> {
        let stem = format!("chunk-{index:05}");
        let wav_name = format!("{stem}.wav");
        let json_name = format!("{stem}.json");
        let directory = Dir::open_ambient_dir(workspace.path(), cap_std::ambient_authority())
            .map_err(WhisperError::Workspace)?;
        if directory.symlink_metadata(&json_name).is_ok() {
            return Err(WhisperError::StaleChunkFile);
        }
        write_new_file(&directory, &wav_name, &encode_speech_wav(samples)?)?;
        let result = self
            .run_and_read(workspace, &directory, &stem, &json_name, cancellation)
            .await;
        // Only the two names this call created are removed; a failed removal
        // leaves them for the next run to refuse as stale rather than reuse.
        let _ = directory.remove_file(&wav_name);
        let _ = directory.remove_file(&json_name);
        result
    }

    async fn run_and_read(
        &self,
        workspace: &ProcessWorkingDirectory,
        directory: &Dir,
        stem: &str,
        json_name: &str,
        cancellation: ProcessCancellation,
    ) -> Result<ProviderChunkOutput, WhisperError> {
        let wav = workspace.path().join(format!("{stem}.wav"));
        let base = workspace.path().join(stem);
        let request = ProcessRequest::new(
            self.executable.clone(),
            workspace.clone(),
            WHISPER_CHUNK_DEADLINE,
        )
        .map_err(WhisperError::Request)?
        .with_arguments(self.argv(&wav, &base));
        let stdout = NonZeroUsize::new(WHISPER_STDOUT_LIMIT).unwrap_or(NonZeroUsize::MIN);
        let stderr = NonZeroUsize::new(WHISPER_STDERR_LIMIT).unwrap_or(NonZeroUsize::MIN);
        let supervisor = ProcessSupervisor::new(
            SupervisorPolicy::default().with_stream_limits(stdout, stderr),
            self.host_isolation,
        );
        let outcome = supervisor
            .run(request, cancellation)
            .await
            .map_err(|error| match error {
                ProcessError::CancelledBeforeSpawn => WhisperError::Cancelled,
                other => WhisperError::Process(other),
            })?;
        match outcome.termination {
            TerminationReason::Exited if outcome.status.success() => {}
            TerminationReason::Exited => return Err(WhisperError::ProviderFailed),
            TerminationReason::Deadline => return Err(WhisperError::Deadline),
            TerminationReason::Cancelled => return Err(WhisperError::Cancelled),
            TerminationReason::OutputLimit(_) => return Err(WhisperError::OutputLimit),
        }
        let bytes = read_bounded_output(directory, json_name, WhisperOutputLimits::R0.max_bytes)?;
        parse_whisper_full_json(&bytes, WhisperOutputLimits::R0).map_err(WhisperError::Output)
    }

    /// Canonical path of the selected model.
    #[must_use]
    pub fn model(&self) -> &Path {
        &self.model
    }

    /// The selected executable.
    #[must_use]
    pub const fn executable(&self) -> &TrustedExecutable {
        &self.executable
    }

    /// Recognizer threads.
    #[must_use]
    pub const fn threads(&self) -> NonZeroU16 {
        self.threads
    }
}

fn write_new_file(directory: &Dir, name: &str, bytes: &[u8]) -> Result<(), WhisperError> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = directory
        .open_with(name, &options)
        .map_err(|error| match error.kind() {
            io::ErrorKind::AlreadyExists => WhisperError::StaleChunkFile,
            _ => WhisperError::Workspace(error),
        })?;
    file.write_all(bytes).map_err(WhisperError::Workspace)?;
    file.flush().map_err(WhisperError::Workspace)
}

/// Reads a provider output file after checking its size, never following a link.
fn read_bounded_output(
    directory: &Dir,
    name: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, WhisperError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = directory
        .open_with(name, &options)
        .map_err(|_| WhisperError::ProviderFailed)?;
    let metadata = file.metadata().map_err(WhisperError::Workspace)?;
    if !metadata.is_file() {
        return Err(WhisperError::ProviderFailed);
    }
    if metadata.len() > max_bytes {
        return Err(WhisperError::Output(WhisperOutputError::TooLarge));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(WhisperError::Workspace)?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > max_bytes) {
        return Err(WhisperError::Output(WhisperOutputError::TooLarge));
    }
    Ok(bytes)
}

/// The [`SpeechRecognizer`] port over one whisper.cpp CLI and work directory.
#[derive(Clone, Debug)]
pub struct WhisperSpeechRecognizer {
    cli: WhisperCli,
    workspace: ProcessWorkingDirectory,
    cancellation: ProcessCancellation,
}

impl WhisperSpeechRecognizer {
    /// Recognizes chunks with `cli`, writing chunk files only in `workspace`.
    ///
    /// The workspace must be a private, VSift-owned directory: chunk audio is
    /// user media.
    #[must_use]
    pub const fn new(
        cli: WhisperCli,
        workspace: ProcessWorkingDirectory,
        cancellation: ProcessCancellation,
    ) -> Self {
        Self {
            cli,
            workspace,
            cancellation,
        }
    }
}

impl SpeechRecognizer for WhisperSpeechRecognizer {
    async fn identity(&self) -> Result<RecognizerIdentity, SpeechRecognitionError> {
        let build = identify_whisper_build(self.cli.executable())
            .await
            .map_err(|_| SpeechRecognitionError::Io)?;
        let (bytes, sha256) = hash_file_bounded(self.cli.model(), MAX_WHISPER_MODEL_BYTES)
            .await
            .map_err(|_| SpeechRecognitionError::ModelUnavailable)?;
        let profile = match pinned_whisper_model() {
            Ok(pinned) if pinned.bytes() == bytes && pinned.sha256() == sha256 => {
                AsrModelProfile::Base
            }
            _ => AsrModelProfile::Unreviewed,
        };
        Ok(RecognizerIdentity {
            provider: AsrProviderBuild::new(
                AsrProvider::WhisperCpp,
                Sha256Hex::parse(build.sha256_hex()).map_err(|_| SpeechRecognitionError::Io)?,
            ),
            model: AsrModel::new(
                profile,
                Sha256Hex::parse(hex(&sha256)).map_err(|_| SpeechRecognitionError::Io)?,
            ),
            decoding: AsrDecodingProfile::R0V1,
            threads: self.cli.threads(),
        })
    }

    async fn recognize(
        &self,
        chunk: &PlannedChunk,
        pcm: &SpeechPcm,
    ) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
        self.cli
            .transcribe_chunk(
                &self.workspace,
                chunk.index(),
                &pcm.samples,
                self.cancellation.clone(),
            )
            .await
            .map_err(|error| SpeechRecognitionError::from(&error))
    }
}

impl AsrCancellation for ProcessCancellation {
    fn is_cancelled(&self) -> bool {
        Self::is_cancelled(self)
    }
}
