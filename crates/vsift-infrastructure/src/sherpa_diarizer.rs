//! sherpa-onnx speaker-diarization adapter (ADR 0031).
//!
//! The source's selected audio stream is decoded by the restricted `FFmpeg` adapter's
//! speech decode, thirty seconds at a time, into one 16 kHz mono 16-bit PCM WAV file in
//! a VSift-owned work folder; the engine never sees the source file. One supervised
//! `sherpa-onnx-offline-speaker-diarization` run with a closed argument list then hears
//! the file: pyannote segmentation 3.0, a speaker-embedding model, and either the
//! user's speaker count or the reviewed distance threshold. The WAV is removed whatever
//! the outcome.
//!
//! Standard output is the only output read. It is bounded, never stored or logged (its
//! first line echoes the configuration, model paths included), and reduced to the
//! engine's `start -- end speaker_NN` turn lines; every other line is skipped. Standard
//! error carries progress lines and is bounded and ignored. The engine computes its
//! voice embeddings in its own process memory and writes none of them anywhere; the
//! adapter receives times and cluster numbers only.
//!
//! The flags were read from the engine's source at the reviewed tag `v1.13.8`
//! (`sherpa-onnx/csrc/sherpa-onnx-offline-speaker-diarization.cc`,
//! `offline-speaker-diarization.cc`, `fast-clustering-config.cc`,
//! `offline-speaker-diarization-result.cc`) and checked on the official builds of three
//! systems by the evaluation workflow `P16 speakers evaluation`.

use std::{
    error::Error,
    ffi::OsString,
    fmt, io,
    io::{Seek, SeekFrom, Write},
    num::{NonZeroU16, NonZeroUsize},
    path::{Path, PathBuf},
    time::Duration,
};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, OpenOptions};
use vsift_application::{DiarizationRequest, DiarizationRunError, SpeakerDiarizer};
use vsift_domain::{
    DiarizationEngine, DiarizationProvenance, DiarizedTurn, EngineSpeaker, MAX_DIARIZED_TURNS,
    MediaDescription, MediaSelection, MediaTime, SPEECH_SAMPLE_RATE, Sha256Hex, SpeakerCount,
    TimeRange,
};

use crate::{
    BoundSource, FfmpegMedia, HostIsolation, MediaError, ProcessCancellation, ProcessError,
    ProcessRequest, ProcessRequestError, ProcessSupervisor, ProcessWorkingDirectory,
    SupervisorPolicy, TerminationReason, TrustedExecutable, ffmpeg_media::MAX_SPEECH_PCM_MICROS,
    whisper_build::hash_file_bounded, whisper_build::hex, whisper_cli::ended_abnormally,
};

/// Retained standard output: the configuration line and at most
/// [`MAX_DIARIZED_TURNS`] turn lines of about 40 bytes.
pub const DIARIZER_STDOUT_LIMIT: usize = 2 * 1024 * 1024;
/// Retained standard error: one progress line per segmentation window.
pub const DIARIZER_STDERR_LIMIT: usize = 4 * 1024 * 1024;
/// Largest engine executable or model file the adapter will hash for provenance.
pub const MAX_DIARIZER_FILE_BYTES: u64 = 1024 * 1024 * 1024;
/// The distance threshold of the automatic count (the maintainer's decision of
/// 2026-10-21: it found the right count on the evaluation's conversations and gave
/// RM06's best confusion with `TitaNet` small).
pub const AUTOMATIC_CLUSTER_THRESHOLD: &str = "0.9";
/// The shortest deadline of one run, for a short range on a slow machine.
pub const MIN_DIARIZATION_DEADLINE: Duration = Duration::from_secs(600);
/// The work file the adapter writes and the engine reads.
const WAV_NAME: &str = "diarization.wav";
const WAV_HEADER_BYTES: u64 = 44;

/// The deadline of one run: ten minutes, plus twice the audio's length.
///
/// The engine ran at about a tenth of real time with two threads on a 4-vCPU hosted
/// runner (the evaluation); twice the length leaves room for a machine twenty times
/// slower.
#[must_use]
pub fn diarization_deadline(range: TimeRange) -> Duration {
    MIN_DIARIZATION_DEADLINE + Duration::from_micros(range.duration_micros().saturating_mul(2))
}

/// Why sherpa-onnx's output was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiarizerOutputError {
    /// Output is not valid UTF-8.
    InvalidUtf8,
    /// A turn line is malformed, or a time is out of order or out of range.
    Unparseable,
    /// More turns than [`MAX_DIARIZED_TURNS`].
    TooManyTurns,
}

impl fmt::Display for DiarizerOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidUtf8 => "diarization output is not valid UTF-8",
            Self::Unparseable => "diarization output has a malformed turn",
            Self::TooManyTurns => "diarization output has too many turns",
        })
    }
}

impl Error for DiarizerOutputError {}

/// Parses the engine's standard output into turns on the source timeline.
///
/// A turn line is `<start> -- <end> speaker_<n>`, optionally followed by
/// ` confidence=<value>` (ignored), with times in seconds and exactly three decimals,
/// relative to the WAV's first sample, which lies at `offset` on the source timeline.
/// Lines that do not start with a digit (the configuration echo, `Started`) are
/// skipped; a line that starts with a digit and is not a turn is refused, so a changed
/// output format fails closed rather than losing turns. A turn of zero length is
/// skipped (the engine writes none, but its format allows it).
///
/// # Errors
///
/// Returns the first [`DiarizerOutputError`]; parsing never partially succeeds.
pub fn parse_diarizer_output(
    stdout: &[u8],
    offset: MediaTime,
) -> Result<Vec<DiarizedTurn>, DiarizerOutputError> {
    let text = std::str::from_utf8(stdout).map_err(|_| DiarizerOutputError::InvalidUtf8)?;
    let mut turns = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if !line.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            continue;
        }
        let Some(turn) = parse_turn(line, offset)? else {
            continue;
        };
        if turns.len() == MAX_DIARIZED_TURNS {
            return Err(DiarizerOutputError::TooManyTurns);
        }
        turns.push(turn);
    }
    Ok(turns)
}

fn parse_turn(line: &str, offset: MediaTime) -> Result<Option<DiarizedTurn>, DiarizerOutputError> {
    let mut fields = line.split(' ');
    let start = fields
        .next()
        .and_then(parse_millis)
        .ok_or(DiarizerOutputError::Unparseable)?;
    if fields.next() != Some("--") {
        return Err(DiarizerOutputError::Unparseable);
    }
    let end = fields
        .next()
        .and_then(parse_millis)
        .ok_or(DiarizerOutputError::Unparseable)?;
    let speaker = fields
        .next()
        .and_then(|field| field.strip_prefix("speaker_"))
        .filter(|digits| {
            (2..=4).contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
        .and_then(|digits| digits.parse::<u16>().ok())
        .ok_or(DiarizerOutputError::Unparseable)?;
    match fields.next() {
        None => {}
        Some(confidence) if confidence.starts_with("confidence=") && fields.next().is_none() => {}
        Some(_) => return Err(DiarizerOutputError::Unparseable),
    }
    if end < start {
        return Err(DiarizerOutputError::Unparseable);
    }
    if end == start {
        return Ok(None);
    }
    let base = offset.as_micros();
    let range = TimeRange::new(
        MediaTime::from_micros(base.saturating_add(start.saturating_mul(1_000))),
        MediaTime::from_micros(base.saturating_add(end.saturating_mul(1_000))),
    )
    .map_err(|_| DiarizerOutputError::Unparseable)?;
    Ok(Some(DiarizedTurn::new(range, EngineSpeaker::new(speaker))))
}

/// `12.345` as 12,345 milliseconds: digits, a point, exactly three digits.
fn parse_millis(field: &str) -> Option<u64> {
    let (seconds, millis) = field.split_once('.')?;
    if seconds.is_empty()
        || seconds.len() > 7
        || millis.len() != 3
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || !millis.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let seconds: u64 = seconds.parse().ok()?;
    let millis: u64 = millis.parse().ok()?;
    seconds.checked_mul(1_000)?.checked_add(millis)
}

/// Why one diarization could not produce turns.
#[derive(Debug)]
pub enum DiarizerError {
    /// A model path is not an absolute, existing regular file.
    InvalidModel,
    /// The work folder or the WAV file could not be used.
    Workspace(io::Error),
    /// A WAV file from an earlier run is still in the work folder.
    StaleWorkFile,
    /// The range's audio does not fit a WAV file.
    AudioTooLong,
    /// The audio could not be decoded.
    Decode(MediaError),
    /// The selected stream has no audio anywhere in the range.
    NoAudio,
    /// The provider request could not be validated.
    Request(ProcessRequestError),
    /// The provider process could not be supervised.
    Process(ProcessError),
    /// The run exceeded its deadline.
    Deadline,
    /// The run was cancelled.
    Cancelled,
    /// The engine wrote more than its output bound.
    OutputLimit,
    /// The engine exited unsuccessfully.
    EngineFailed,
    /// The engine ended by a signal or an operating-system crash status.
    AbnormalTermination,
    /// The output was refused.
    Output(DiarizerOutputError),
}

impl fmt::Display for DiarizerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidModel => "a diarization model file is invalid",
            Self::Workspace(_) => "the diarization work folder could not be used",
            Self::StaleWorkFile => "a stale diarization work file is in the work folder",
            Self::AudioTooLong => "the audio is too long for a WAV file",
            Self::Decode(_) => "the audio could not be decoded",
            Self::NoAudio => "the range holds no decodable audio",
            Self::Request(_) => "the diarization request is invalid",
            Self::Process(_) => "the diarization engine could not run",
            Self::Deadline => "the diarization engine exceeded its deadline",
            Self::Cancelled => "diarization was cancelled",
            Self::OutputLimit => "the diarization engine exceeded its output bound",
            Self::EngineFailed => "the diarization engine did not complete",
            Self::AbnormalTermination => "the diarization engine terminated abnormally",
            Self::Output(_) => "the diarization output was refused",
        })
    }
}

impl Error for DiarizerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::Decode(error) => Some(error),
            Self::Request(error) => Some(error),
            Self::Process(error) => Some(error),
            Self::Output(error) => Some(error),
            _ => None,
        }
    }
}

impl From<&DiarizerError> for DiarizationRunError {
    fn from(error: &DiarizerError) -> Self {
        match error {
            DiarizerError::Workspace(_) | DiarizerError::StaleWorkFile => Self::Workspace,
            DiarizerError::AudioTooLong
            | DiarizerError::OutputLimit
            | DiarizerError::Output(DiarizerOutputError::TooManyTurns) => Self::ResourceLimit,
            DiarizerError::Decode(MediaError::Deadline) | DiarizerError::Deadline => Self::Deadline,
            DiarizerError::Decode(MediaError::Cancelled) | DiarizerError::Cancelled => {
                Self::Cancelled
            }
            DiarizerError::Decode(MediaError::CapacityUnavailable) => Self::Busy,
            DiarizerError::Decode(MediaError::StreamUnavailable | MediaError::UnsupportedCodec)
            | DiarizerError::NoAudio => Self::NoAudio,
            DiarizerError::Decode(_) => Self::Source,
            DiarizerError::InvalidModel | DiarizerError::Request(_) | DiarizerError::Process(_) => {
                Self::EngineUnavailable
            }
            DiarizerError::EngineFailed => Self::EngineFailed,
            DiarizerError::AbnormalTermination => Self::AbnormalTermination,
            DiarizerError::Output(_) => Self::UnparseableOutput,
        }
    }
}

/// sherpa-onnx's diarization tool with a fixed segmentation and embedding model.
#[derive(Clone, Debug)]
pub struct SherpaOnnxDiarizer {
    executable: TrustedExecutable,
    segmentation_model: PathBuf,
    embedding_model: PathBuf,
    threads: NonZeroU16,
    host_isolation: HostIsolation,
}

impl SherpaOnnxDiarizer {
    /// Selects a trusted executable and the two model files.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizerError::InvalidModel`] unless each model is an absolute path to
    /// an existing regular file; each is canonicalized so the engine is always given
    /// one exact file.
    pub fn new(
        executable: TrustedExecutable,
        segmentation_model: &Path,
        embedding_model: &Path,
        threads: NonZeroU16,
        host_isolation: HostIsolation,
    ) -> Result<Self, DiarizerError> {
        Ok(Self {
            executable,
            segmentation_model: canonical_model(segmentation_model)?,
            embedding_model: canonical_model(embedding_model)?,
            threads,
            host_isolation,
        })
    }

    /// The closed argument list for one run over `wav`.
    ///
    /// The two thread counts are the engine's segmentation and embedding threads; a
    /// given count sets `--clustering.num-clusters`, the automatic count sets the
    /// reviewed `--clustering.cluster-threshold` instead (the engine reads the count
    /// first when both are set, so only one is ever passed).
    #[must_use]
    pub fn argv(&self, wav: &Path, count: SpeakerCount) -> Vec<OsString> {
        let mut segmentation = OsString::from("--segmentation.pyannote-model=");
        segmentation.push(self.segmentation_model.as_os_str());
        let mut embedding = OsString::from("--embedding.model=");
        embedding.push(self.embedding_model.as_os_str());
        let threads = self.threads.get();
        let clustering = match count {
            SpeakerCount::Given(count) => format!("--clustering.num-clusters={count}"),
            SpeakerCount::Automatic => {
                format!("--clustering.cluster-threshold={AUTOMATIC_CLUSTER_THRESHOLD}")
            }
        };
        vec![
            segmentation,
            embedding,
            format!("--segmentation.num-threads={threads}").into(),
            format!("--embedding.num-threads={threads}").into(),
            clustering.into(),
            wav.as_os_str().to_owned(),
        ]
    }

    /// Identifies the engine and the two models as they are now, from their bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizationRunError::EngineUnavailable`] when a file cannot be read
    /// within [`MAX_DIARIZER_FILE_BYTES`].
    pub async fn provenance(&self) -> Result<DiarizationProvenance, DiarizationRunError> {
        let digest = |path: PathBuf| async move {
            let (_, sha256) = hash_file_bounded(&path, MAX_DIARIZER_FILE_BYTES)
                .await
                .map_err(|_| DiarizationRunError::EngineUnavailable)?;
            Sha256Hex::parse(hex(&sha256)).map_err(|_| DiarizationRunError::EngineUnavailable)
        };
        Ok(DiarizationProvenance::new(
            DiarizationEngine::SherpaOnnx,
            digest(self.executable.path().to_path_buf()).await?,
            digest(self.segmentation_model.clone()).await?,
            digest(self.embedding_model.clone()).await?,
        ))
    }

    /// Runs the engine over the WAV named `wav` in `workspace` and parses its turns,
    /// placed at `offset` on the source timeline.
    ///
    /// # Errors
    ///
    /// Returns a typed [`DiarizerError`] for process and output failures.
    pub async fn run(
        &self,
        wav: &Path,
        offset: MediaTime,
        count: SpeakerCount,
        deadline: Duration,
        cancellation: ProcessCancellation,
    ) -> Result<Vec<DiarizedTurn>, DiarizerError> {
        // The engine runs in its own folder and is given every file by absolute path
        // (#469, `ProcessWorkingDirectory::of_executable`).
        let cwd = ProcessWorkingDirectory::of_executable(&self.executable)
            .map_err(DiarizerError::Request)?;
        let request = ProcessRequest::new(self.executable.clone(), cwd, deadline)
            .map_err(DiarizerError::Request)?
            .with_arguments(self.argv(wav, count));
        let stdout = NonZeroUsize::new(DIARIZER_STDOUT_LIMIT).unwrap_or(NonZeroUsize::MIN);
        let stderr = NonZeroUsize::new(DIARIZER_STDERR_LIMIT).unwrap_or(NonZeroUsize::MIN);
        let supervisor = ProcessSupervisor::new(
            SupervisorPolicy::default().with_stream_limits(stdout, stderr),
            self.host_isolation,
        );
        let outcome = supervisor
            .run(request, cancellation)
            .await
            .map_err(|error| match error {
                ProcessError::CancelledBeforeSpawn | ProcessError::CancelledDuringStart => {
                    DiarizerError::Cancelled
                }
                other => DiarizerError::Process(other),
            })?;
        match outcome.termination {
            TerminationReason::Exited if outcome.status.success() => {}
            TerminationReason::Exited if ended_abnormally(outcome.status) => {
                return Err(DiarizerError::AbnormalTermination);
            }
            TerminationReason::Exited => return Err(DiarizerError::EngineFailed),
            TerminationReason::Deadline => return Err(DiarizerError::Deadline),
            TerminationReason::Cancelled => return Err(DiarizerError::Cancelled),
            TerminationReason::OutputLimit(_) => return Err(DiarizerError::OutputLimit),
        }
        if !outcome.stdout.complete {
            return Err(DiarizerError::OutputLimit);
        }
        parse_diarizer_output(&outcome.stdout.bytes, offset).map_err(DiarizerError::Output)
    }

    /// The selected executable.
    #[must_use]
    pub const fn executable(&self) -> &TrustedExecutable {
        &self.executable
    }
}

fn canonical_model(path: &Path) -> Result<PathBuf, DiarizerError> {
    if !path.is_absolute() {
        return Err(DiarizerError::InvalidModel);
    }
    let canonical = std::fs::canonicalize(path).map_err(|_| DiarizerError::InvalidModel)?;
    if !std::fs::metadata(&canonical).is_ok_and(|metadata| metadata.is_file()) {
        return Err(DiarizerError::InvalidModel);
    }
    Ok(canonical)
}

/// A 16 kHz mono 16-bit PCM WAV of a known length, written sample by sample.
///
/// The header is written first with the final size; samples are placed by their source
/// time, so a decode that starts late is padded with silence and one that starts early
/// (a codec's pre-roll) is trimmed, and the file always holds exactly the range.
pub struct DiarizationWav {
    file: File,
    range: TimeRange,
    total_samples: u64,
    written: u64,
}

impl DiarizationWav {
    /// Creates `name` in `directory` (a new file, never following a link) for `range`.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizerError::StaleWorkFile`] when the name exists,
    /// [`DiarizerError::AudioTooLong`] when the range does not fit a WAV file and
    /// [`DiarizerError::Workspace`] for other I/O failures.
    pub fn create(directory: &Dir, name: &str, range: TimeRange) -> Result<Self, DiarizerError> {
        let total_samples = samples_in(range.duration_micros());
        let data_bytes = total_samples
            .checked_mul(2)
            .and_then(|bytes| u32::try_from(bytes).ok())
            .ok_or(DiarizerError::AudioTooLong)?;
        let riff_bytes = data_bytes
            .checked_add(36)
            .ok_or(DiarizerError::AudioTooLong)?;
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let mut file = directory
            .open_with(name, &options)
            .map_err(|error| match error.kind() {
                io::ErrorKind::AlreadyExists => DiarizerError::StaleWorkFile,
                _ => DiarizerError::Workspace(error),
            })?;
        let mut header = Vec::with_capacity(44);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&riff_bytes.to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        header.extend_from_slice(&16_u32.to_le_bytes());
        header.extend_from_slice(&1_u16.to_le_bytes());
        header.extend_from_slice(&1_u16.to_le_bytes());
        header.extend_from_slice(&SPEECH_SAMPLE_RATE.to_le_bytes());
        header.extend_from_slice(&(SPEECH_SAMPLE_RATE * 2).to_le_bytes());
        header.extend_from_slice(&2_u16.to_le_bytes());
        header.extend_from_slice(&16_u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&data_bytes.to_le_bytes());
        file.write_all(&header).map_err(DiarizerError::Workspace)?;
        Ok(Self {
            file,
            range,
            total_samples,
            written: 0,
        })
    }

    /// Places little-endian samples that start at `start` on the source timeline.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizerError::Workspace`] when the file cannot be written.
    pub fn place(&mut self, start: MediaTime, pcm_s16le: &[u8]) -> Result<(), DiarizerError> {
        let range_start = self.range.start().as_micros();
        // Where the first sample lands, or how many samples precede the range.
        let (at, mut skip) = if start.as_micros() >= range_start {
            (samples_in(start.as_micros() - range_start), 0)
        } else {
            (0, samples_in(range_start - start.as_micros()))
        };
        if at > self.written {
            self.pad_to(at)?;
        } else {
            skip += self.written - at;
        }
        let available = u64::try_from(pcm_s16le.len() / 2).unwrap_or(u64::MAX);
        let take = available
            .saturating_sub(skip)
            .min(self.total_samples.saturating_sub(self.written));
        if take == 0 {
            return Ok(());
        }
        let from = usize::try_from(skip.saturating_mul(2)).unwrap_or(usize::MAX);
        let to = usize::try_from((skip + take).saturating_mul(2)).unwrap_or(usize::MAX);
        let bytes = pcm_s16le.get(from..to).unwrap_or_default();
        self.file
            .write_all(bytes)
            .map_err(DiarizerError::Workspace)?;
        self.written += u64::try_from(bytes.len() / 2).unwrap_or(0);
        Ok(())
    }

    /// Pads the rest with silence, checks the file's length and closes it.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizerError::Workspace`] when the file cannot be written or does not
    /// hold exactly the header and the range's samples.
    pub fn finish(mut self) -> Result<(), DiarizerError> {
        self.pad_to(self.total_samples)?;
        self.file.flush().map_err(DiarizerError::Workspace)?;
        let length = self
            .file
            .seek(SeekFrom::End(0))
            .map_err(DiarizerError::Workspace)?;
        if length != WAV_HEADER_BYTES + self.total_samples * 2 {
            return Err(DiarizerError::Workspace(io::Error::other(
                "the diarization WAV has an unexpected length",
            )));
        }
        Ok(())
    }

    fn pad_to(&mut self, samples: u64) -> Result<(), DiarizerError> {
        static SILENCE: [u8; 64 * 1024] = [0; 64 * 1024];
        let target = samples.min(self.total_samples);
        while self.written < target {
            let chunk = (target - self.written).min(SILENCE.len() as u64 / 2);
            let bytes = usize::try_from(chunk * 2).unwrap_or(0);
            self.file
                .write_all(SILENCE.get(..bytes).unwrap_or_default())
                .map_err(DiarizerError::Workspace)?;
            self.written += chunk;
        }
        Ok(())
    }
}

fn samples_in(micros: u64) -> u64 {
    micros.saturating_mul(u64::from(SPEECH_SAMPLE_RATE)) / 1_000_000
}

/// The [`SpeakerDiarizer`] port over the restricted `FFmpeg` adapter and sherpa-onnx.
pub struct FfmpegSherpaDiarizer<'a> {
    media: &'a FfmpegMedia<'a>,
    source: &'a BoundSource,
    description: &'a MediaDescription,
    selection: MediaSelection,
    workspace: &'a ProcessWorkingDirectory,
    engine: &'a SherpaOnnxDiarizer,
    cancellation: ProcessCancellation,
}

impl<'a> FfmpegSherpaDiarizer<'a> {
    /// Diarizes `source`'s selected audio with `engine`, in `workspace`.
    #[must_use]
    pub const fn new(
        media: &'a FfmpegMedia<'a>,
        source: &'a BoundSource,
        description: &'a MediaDescription,
        selection: MediaSelection,
        workspace: &'a ProcessWorkingDirectory,
        engine: &'a SherpaOnnxDiarizer,
        cancellation: ProcessCancellation,
    ) -> Self {
        Self {
            media,
            source,
            description,
            selection,
            workspace,
            engine,
            cancellation,
        }
    }

    async fn write_wav(&self, directory: &Dir, range: TimeRange) -> Result<(), DiarizerError> {
        let mut wav = DiarizationWav::create(directory, WAV_NAME, range)?;
        let mut heard_audio = false;
        let mut start = range.start().as_micros();
        let end = range.end().as_micros();
        while start < end {
            let stop = start.saturating_add(MAX_SPEECH_PCM_MICROS).min(end);
            let window =
                TimeRange::new(MediaTime::from_micros(start), MediaTime::from_micros(stop))
                    .map_err(|_| DiarizerError::Decode(MediaError::InvalidAudioRange))?;
            match self
                .media
                .speech_pcm(
                    self.source,
                    self.description,
                    self.selection,
                    window,
                    self.cancellation.clone(),
                )
                .await
            {
                Ok(audio) => {
                    heard_audio = true;
                    wav.place(audio.actual_start, &audio.pcm_s16le)?;
                }
                // A window with no decoded audio stays silent in the file.
                Err(MediaError::NoDecodedAudio) => {}
                Err(error) => return Err(DiarizerError::Decode(error)),
            }
            start = stop;
        }
        if !heard_audio {
            return Err(DiarizerError::NoAudio);
        }
        wav.finish()
    }

    async fn diarize_range(
        &self,
        request: DiarizationRequest,
    ) -> Result<Vec<DiarizedTurn>, DiarizerError> {
        let directory = Dir::open_ambient_dir(self.workspace.path(), cap_std::ambient_authority())
            .map_err(DiarizerError::Workspace)?;
        let written = self.write_wav(&directory, request.range).await;
        let result = match written {
            Ok(()) => {
                self.engine
                    .run(
                        &self.workspace.path().join(WAV_NAME),
                        request.range.start(),
                        request.count,
                        diarization_deadline(request.range),
                        self.cancellation.clone(),
                    )
                    .await
            }
            Err(error) => Err(error),
        };
        // Only the name this call created is removed; a failed removal leaves it for
        // the next run to refuse as stale rather than reuse.
        let _ = directory.remove_file(WAV_NAME);
        result
    }
}

impl SpeakerDiarizer for FfmpegSherpaDiarizer<'_> {
    async fn identity(&self) -> Result<DiarizationProvenance, DiarizationRunError> {
        self.engine.provenance().await
    }

    async fn diarize(
        &self,
        request: DiarizationRequest,
    ) -> Result<Vec<DiarizedTurn>, DiarizationRunError> {
        self.diarize_range(request)
            .await
            .map_err(|error| DiarizationRunError::from(&error))
    }
}

#[cfg(test)]
mod tests {
    use cap_std::fs::Dir;
    use vsift_domain::{MediaTime, TimeRange};

    use super::{DiarizationWav, DiarizerOutputError, parse_diarizer_output};
    use crate::test_folder::TestFolder;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const OUTPUT: &str = "OfflineSpeakerDiarizationConfig(segmentation=OfflineSpeakerSegmentationModelConfig(pyannote=OfflineSpeakerSegmentationPyannoteModelConfig(model=\"/x/model.onnx\"), num_threads=2, debug=False, provider=\"cpu\"), embedding=SpeakerEmbeddingExtractorConfig(model=\"/x/titanet.onnx\", num_threads=2, debug=False, provider=\"cpu\"), clustering=FastClusteringConfig(num_clusters=-1, threshold=0.9, compute_confidence=False), min_duration_on=0.3, min_duration_off=0.5)\nStarted\n0.318 -- 6.865 speaker_00\n7.017 -- 10.747 speaker_01\r\n11.000 -- 11.000 speaker_01\n12.500 -- 14.000 speaker_00 confidence=0.812\n";

    #[test]
    fn turns_are_parsed_onto_the_source_timeline_and_the_echo_is_skipped() -> TestResult {
        let turns = parse_diarizer_output(OUTPUT.as_bytes(), MediaTime::from_micros(60_000_000))?;
        let read: Vec<(u64, u64, u16)> = turns
            .iter()
            .map(|turn| {
                (
                    turn.range().start().as_micros(),
                    turn.range().end().as_micros(),
                    turn.speaker().value(),
                )
            })
            .collect();
        assert_eq!(
            read,
            [
                (60_318_000, 66_865_000, 0),
                (67_017_000, 70_747_000, 1),
                (72_500_000, 74_000_000, 0),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_changed_format_fails_closed() {
        for line in [
            "1.5 -- 2.000 speaker_00",
            "1.500 - 2.000 speaker_00",
            "1.500 -- 2.000 spk_00",
            "1.500 -- 2.000 speaker_0",
            "2.500 -- 2.000 speaker_00",
            "1.500 -- 2.000 speaker_00 extra",
            "1.500 -- 2.000",
        ] {
            assert_eq!(
                parse_diarizer_output(line.as_bytes(), MediaTime::from_micros(0)),
                Err(DiarizerOutputError::Unparseable),
                "{line}"
            );
        }
        assert_eq!(
            parse_diarizer_output(&[0xff, 0xfe], MediaTime::from_micros(0)),
            Err(DiarizerOutputError::InvalidUtf8)
        );
    }

    #[test]
    fn too_many_turns_are_refused() {
        let line = "0.000 -- 1.000 speaker_00\n";
        let flood = line.repeat(vsift_domain::MAX_DIARIZED_TURNS + 1);
        assert_eq!(
            parse_diarizer_output(flood.as_bytes(), MediaTime::from_micros(0)),
            Err(DiarizerOutputError::TooManyTurns)
        );
    }

    fn pcm(samples: &[i16]) -> Vec<u8> {
        samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect()
    }

    #[test]
    fn the_wav_holds_exactly_the_range_with_each_decode_at_its_time() -> TestResult {
        let folder = TestFolder::new("vsift-diarization-wav")?;
        let directory = Dir::open_ambient_dir(&*folder, cap_std::ambient_authority())?;
        // One millisecond is 16 samples. The range starts at 10 ms and is 2 ms long.
        let range = TimeRange::new(
            MediaTime::from_micros(10_000),
            MediaTime::from_micros(12_000),
        )?;
        let mut wav = DiarizationWav::create(&directory, "a.wav", range)?;
        // A decode that starts 0.5 ms early: its first 8 samples are trimmed.
        wav.place(MediaTime::from_micros(9_500), &pcm(&[1; 16]))?;
        // A decode that starts after a gap: 4 silent samples are padded first; samples
        // past the range are dropped.
        wav.place(MediaTime::from_micros(11_250), &pcm(&[2; 40]))?;
        wav.finish()?;
        let bytes = std::fs::read(folder.join("a.wav"))?;
        assert_eq!(bytes.len(), 44 + 32 * 2);
        let samples: Vec<i16> = bytes[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect();
        let mut expected = vec![1_i16; 8];
        expected.extend([0; 12]);
        expected.extend([2; 12]);
        assert_eq!(samples, expected);
        assert_eq!(
            DiarizationWav::create(&directory, "a.wav", range)
                .err()
                .map(|error| error.to_string()),
            Some("a stale diarization work file is in the work folder".to_owned())
        );
        Ok(())
    }
}
