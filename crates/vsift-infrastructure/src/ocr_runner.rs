//! The OCR runner adapter (ADR 0030): `vsift-ocr-runner` under the process supervisor.
//!
//! `VSift` runs the runner as it runs whisper.cpp: an explicit executable and
//! argument list, never a shell; in the runner's own folder; with a deadline,
//! bounded output and no inherited environment. Frames are handed over as PNG
//! files in a private, VSift-owned folder, created new and removed after the run.
//!
//! The runner's answer is untrusted provider output (SEC-28). It is a stream of
//! JSON lines in protocol 1 ([`parse_runner_output`]), read with every count and
//! length bounded; anything else is [`RunnerOutputError`] and no reading of the
//! batch is kept. Text checks (empty, control characters, length) and region
//! checks against the frame are the application's.
//!
//! **One engine build per index, proven at every read** (RV-2 finding 3). The
//! build is learnt once, from `describe` and the executable's own digest. Each
//! `read` then hashes the executable again before it starts the runner, and the
//! runner's greeting names the runtime and models it hashed itself before
//! loading them; a batch by any other build is refused as
//! [`ScreenTextReadError::BuildChanged`] and nothing of it is kept.
//!
//! **The call's bound holds** (RV-2 finding 4). A runner given a call deadline
//! ([`OcrRunner::with_call_deadline`]) runs each process for at most the time
//! the call has left, and starts none once it has none.

use std::{
    error::Error,
    ffi::OsString,
    fmt, io,
    io::Write,
    num::{NonZeroU16, NonZeroUsize},
    time::{Duration, Instant},
};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use serde::Deserialize;
use vsift_application::{
    EngineLine, EngineReading, ScreenTextImage, ScreenTextReadError, ScreenTextReader,
};
use vsift_domain::{
    MAX_SCREEN_TEXT_LINES_PER_FRAME, ProviderLabel, ScreenTextProfile, ScreenTextProvider,
    Sha256Hex,
};

use crate::{
    FilesystemSessionStore, HostIsolation, ProcessCancellation, ProcessError, ProcessRequest,
    ProcessSupervisor, ProcessWorkingDirectory, SupervisorPolicy, TerminationReason,
    TrustedExecutable,
    whisper_build::{hash_file_bounded, hex},
};

/// The runner protocol this adapter reads.
pub const RUNNER_PROTOCOL: u32 = 1;
/// Most images handed to one runner run.
pub const MAX_RUNNER_IMAGES: usize = 8;
/// Largest runner output read from one run: 256 lines of 8 images with long text.
pub const MAX_RUNNER_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
/// Largest runner executable hashed for its identity.
pub const MAX_RUNNER_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
/// Most bytes of one line's text as the runner writes it (the domain bounds characters).
const MAX_LINE_TEXT_BYTES: usize = 4 * 512;
/// Largest region side the runner may report, before the frame check.
const MAX_REGION_SIDE: u32 = 65_536;
const DESCRIBE_DEADLINE: Duration = Duration::from_secs(30);
const READ_BASE_DEADLINE: Duration = Duration::from_secs(30);
const READ_DEADLINE_PER_IMAGE: Duration = Duration::from_secs(20);
const STDERR_LIMIT: usize = 64 * 1024;

/// Why the runner's output was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunnerOutputError {
    /// More bytes than [`MAX_RUNNER_OUTPUT_BYTES`].
    TooLarge,
    /// Not UTF-8, not one JSON object a line, or a record of an unknown shape.
    Malformed,
    /// Another protocol version, or no greeting first.
    Protocol,
    /// Records out of order, counts that disagree, or an image left without its end.
    Inconsistent,
    /// More lines or longer text than the bounds allow.
    OverBound,
}

impl fmt::Display for RunnerOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "the OCR runner wrote more than its output bound",
            Self::Malformed => "the OCR runner's output is not its JSON-lines protocol",
            Self::Protocol => "the OCR runner speaks another protocol",
            Self::Inconsistent => "the OCR runner's records disagree",
            Self::OverBound => "the OCR runner reported more lines or longer text than allowed",
        })
    }
}

impl Error for RunnerOutputError {}

#[derive(Debug, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
enum Record {
    Runner {
        protocol: u32,
        runner_version: String,
        runtime_version: String,
        runtime_sha256: String,
        detector_sha256: String,
        recognizer_sha256: String,
    },
    Line {
        image: usize,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        text: String,
        text_cut: bool,
        confidence: f64,
    },
    Image {
        image: usize,
        width: u32,
        height: u32,
        lines: usize,
        truncated: bool,
    },
    Error {
        image: Option<usize>,
        code: String,
    },
}

/// The build a `read` run says it is: the runner's version, and the runtime and
/// models it hashed before loading them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerGreeting {
    /// The runner's version.
    pub runner_version: ProviderLabel,
    /// The ONNX Runtime library's version label, as `describe` names it.
    pub runtime_version: ProviderLabel,
    /// SHA-256 of that library.
    pub runtime_sha256: Sha256Hex,
    /// SHA-256 of the detection model.
    pub detector_sha256: Sha256Hex,
    /// SHA-256 of the recognition model.
    pub recognizer_sha256: Sha256Hex,
}

impl RunnerGreeting {
    /// Whether the run is by the build `describe` reported: the same runner
    /// version, the same runtime (named by its digest) and the same two models.
    /// The executable's own digest is `VSift`'s to check, which it does before the run.
    #[must_use]
    pub fn is_build(&self, build: &ScreenTextProvider) -> bool {
        self.runner_version == build.runner_version
            && self.runtime_version == build.runtime_version
            && self.detector_sha256 == build.detector_sha256
            && self.recognizer_sha256 == build.recognizer_sha256
    }
}

/// One `read` run's output, parsed: who answered, and one reading per image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerRun {
    /// The greeting.
    pub greeting: RunnerGreeting,
    /// The readings, in the order the images were given.
    pub readings: Vec<EngineReading>,
}

/// Parses one run's output for images of `dimensions` (width, height), in order:
/// the greeting, then each image's lines and its `image` record, or its `error`
/// record. A line the runner cut at its length bound marks its image's reading
/// as truncated.
///
/// Published for fuzzing (ADR 0016 decision 6).
///
/// # Errors
///
/// [`RunnerOutputError`] for anything but exactly that, within the bounds, or an
/// `image` record whose size is not the image's.
pub fn parse_runner_output(
    stdout: &[u8],
    dimensions: &[(u32, u32)],
) -> Result<RunnerRun, RunnerOutputError> {
    if stdout.len() > MAX_RUNNER_OUTPUT_BYTES {
        return Err(RunnerOutputError::TooLarge);
    }
    let text = std::str::from_utf8(stdout).map_err(|_| RunnerOutputError::Malformed)?;
    let mut records = text.split('\n').filter(|line| !line.is_empty());
    let greeting = match records.next().map(serde_json::from_str::<Record>) {
        Some(Ok(Record::Runner {
            protocol: RUNNER_PROTOCOL,
            runner_version,
            runtime_version,
            runtime_sha256,
            detector_sha256,
            recognizer_sha256,
        })) => greeting(
            runner_version,
            runtime_version,
            runtime_sha256,
            detector_sha256,
            recognizer_sha256,
        )
        .ok_or(RunnerOutputError::Malformed)?,
        Some(Ok(_)) | None => return Err(RunnerOutputError::Protocol),
        Some(Err(_)) => return Err(RunnerOutputError::Malformed),
    };
    let mut readings = Readings {
        dimensions,
        done: Vec::with_capacity(dimensions.len()),
        lines: Vec::new(),
        cut: false,
    };
    for record in records {
        let record: Record =
            serde_json::from_str(record).map_err(|_| RunnerOutputError::Malformed)?;
        readings.accept(record)?;
    }
    Ok(RunnerRun {
        greeting,
        readings: readings.finish()?,
    })
}

fn greeting(
    runner_version: String,
    runtime_version: String,
    runtime_sha256: String,
    detector_sha256: String,
    recognizer_sha256: String,
) -> Option<RunnerGreeting> {
    Some(RunnerGreeting {
        runner_version: ProviderLabel::parse(runner_version).ok()?,
        runtime_version: ProviderLabel::parse(runtime_version).ok()?,
        runtime_sha256: Sha256Hex::parse(runtime_sha256).ok()?,
        detector_sha256: Sha256Hex::parse(detector_sha256).ok()?,
        recognizer_sha256: Sha256Hex::parse(recognizer_sha256).ok()?,
    })
}

/// The readings of one run as its records arrive, image by image.
struct Readings<'a> {
    dimensions: &'a [(u32, u32)],
    done: Vec<EngineReading>,
    lines: Vec<EngineLine>,
    cut: bool,
}

impl Readings<'_> {
    fn accept(&mut self, record: Record) -> Result<(), RunnerOutputError> {
        let current = self.done.len();
        match record {
            Record::Line {
                image,
                x,
                y,
                width,
                height,
                text,
                text_cut,
                confidence,
            } => {
                if image != current {
                    return Err(RunnerOutputError::Inconsistent);
                }
                let outside = [x, y, width, height]
                    .iter()
                    .any(|value| *value > MAX_REGION_SIDE);
                if self.lines.len() >= MAX_SCREEN_TEXT_LINES_PER_FRAME
                    || text.len() > MAX_LINE_TEXT_BYTES
                    || outside
                    || !(0.0..=1.0).contains(&confidence)
                {
                    return Err(RunnerOutputError::OverBound);
                }
                self.cut |= text_cut;
                self.lines.push(EngineLine {
                    x,
                    y,
                    width,
                    height,
                    text,
                    confidence_basis_points: basis_points(confidence),
                });
            }
            Record::Image {
                image,
                width,
                height,
                lines: count,
                truncated,
            } => {
                if image != current
                    || count != self.lines.len()
                    || self.dimensions.get(image) != Some(&(width, height))
                {
                    return Err(RunnerOutputError::Inconsistent);
                }
                self.done.push(EngineReading::Lines {
                    lines: std::mem::take(&mut self.lines),
                    truncated: truncated || self.cut,
                });
                self.cut = false;
            }
            Record::Error {
                image: Some(image),
                code,
            } => {
                if image != current || !self.lines.is_empty() {
                    return Err(RunnerOutputError::Inconsistent);
                }
                self.done.push(match code.as_str() {
                    "image_invalid" => EngineReading::Rejected,
                    "inference_failed" => EngineReading::Failed,
                    _ => return Err(RunnerOutputError::Malformed),
                });
            }
            Record::Error { image: None, .. } | Record::Runner { .. } => {
                return Err(RunnerOutputError::Inconsistent);
            }
        }
        if self.done.len() > self.dimensions.len() {
            return Err(RunnerOutputError::Inconsistent);
        }
        Ok(())
    }

    fn finish(self) -> Result<Vec<EngineReading>, RunnerOutputError> {
        if self.done.len() != self.dimensions.len() || !self.lines.is_empty() {
            return Err(RunnerOutputError::Inconsistent);
        }
        Ok(self.done)
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is checked to lie in 0..=1, so the product lies in 0..=10,000"
)]
fn basis_points(confidence: f64) -> u16 {
    (confidence * 10_000.0).round().clamp(0.0, 10_000.0) as u16
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Description {
    record: String,
    protocol: u32,
    runner_version: String,
    runtime_version: String,
    runtime_sha256: String,
    profile: String,
    detector_sha256: String,
    recognizer_sha256: String,
}

/// What `vsift-ocr-runner describe` says about itself, checked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerDescription {
    /// The runner's version.
    pub runner_version: ProviderLabel,
    /// The ONNX Runtime version it loaded.
    pub runtime_version: ProviderLabel,
    /// SHA-256 of the ONNX Runtime library, taken before it was loaded.
    pub runtime_sha256: Sha256Hex,
    /// The pipeline it runs.
    pub profile: ScreenTextProfile,
    /// SHA-256 of the detection model it loaded.
    pub detector_sha256: Sha256Hex,
    /// SHA-256 of the recognition model it loaded.
    pub recognizer_sha256: Sha256Hex,
}

/// Parses the runner's `describe` answer: one JSON object.
///
/// # Errors
///
/// [`RunnerOutputError`] for anything else, another protocol, or an unknown profile.
pub fn parse_runner_description(stdout: &[u8]) -> Result<RunnerDescription, RunnerOutputError> {
    if stdout.len() > 4096 {
        return Err(RunnerOutputError::TooLarge);
    }
    let text = std::str::from_utf8(stdout).map_err(|_| RunnerOutputError::Malformed)?;
    let description: Description =
        serde_json::from_str(text.trim_end()).map_err(|_| RunnerOutputError::Malformed)?;
    if description.record != "describe" || description.protocol != RUNNER_PROTOCOL {
        return Err(RunnerOutputError::Protocol);
    }
    Ok(RunnerDescription {
        runner_version: ProviderLabel::parse(description.runner_version)
            .map_err(|_| RunnerOutputError::Malformed)?,
        runtime_version: ProviderLabel::parse(description.runtime_version)
            .map_err(|_| RunnerOutputError::Malformed)?,
        runtime_sha256: Sha256Hex::parse(description.runtime_sha256)
            .map_err(|_| RunnerOutputError::Malformed)?,
        profile: ScreenTextProfile::from_identifier(&description.profile)
            .ok_or(RunnerOutputError::Protocol)?,
        detector_sha256: Sha256Hex::parse(description.detector_sha256)
            .map_err(|_| RunnerOutputError::Malformed)?,
        recognizer_sha256: Sha256Hex::parse(description.recognizer_sha256)
            .map_err(|_| RunnerOutputError::Malformed)?,
    })
}

/// The runner executable and how `VSift` runs it.
#[derive(Clone, Debug)]
pub struct OcrRunner {
    executable: TrustedExecutable,
    threads: NonZeroU16,
    host_isolation: HostIsolation,
    call_deadline: Option<Instant>,
}

impl OcrRunner {
    /// A runner that reads with `threads` inference threads.
    #[must_use]
    pub const fn new(
        executable: TrustedExecutable,
        threads: NonZeroU16,
        host_isolation: HostIsolation,
    ) -> Self {
        Self {
            executable,
            threads,
            host_isolation,
            call_deadline: None,
        }
    }

    /// The same runner bounded by the call it serves: each run gets at most the
    /// time left until `deadline` (its own deadline otherwise), and none starts
    /// once the deadline has passed ([`ScreenTextReadError::Deadline`]).
    #[must_use]
    pub const fn with_call_deadline(mut self, deadline: Instant) -> Self {
        self.call_deadline = Some(deadline);
        self
    }

    /// The executable.
    #[must_use]
    pub const fn executable(&self) -> &TrustedExecutable {
        &self.executable
    }

    /// Inference threads, and so the admission weight of a run.
    #[must_use]
    pub const fn threads(&self) -> NonZeroU16 {
        self.threads
    }

    /// The engine build: the executable's own SHA-256 and what `describe` reports.
    ///
    /// # Errors
    ///
    /// [`ScreenTextReadError::Unavailable`] when the executable cannot be read or
    /// started, [`ScreenTextReadError::ModelUnavailable`] when it cannot load its
    /// runtime or models, and the run's own failures.
    pub async fn identity(
        &self,
        cancellation: ProcessCancellation,
    ) -> Result<ScreenTextProvider, ScreenTextReadError> {
        let (_, digest) = hash_file_bounded(self.executable.path(), MAX_RUNNER_EXECUTABLE_BYTES)
            .await
            .map_err(|_| ScreenTextReadError::Unavailable)?;
        let stdout = self
            .run(
                vec![OsString::from("describe")],
                DESCRIBE_DEADLINE,
                cancellation,
            )
            .await?;
        let description = parse_runner_description(&stdout)
            .map_err(|_| ScreenTextReadError::UnparseableOutput)?;
        Ok(ScreenTextProvider {
            runner_sha256: Sha256Hex::parse(hex(&digest))
                .map_err(|_| ScreenTextReadError::Unavailable)?,
            runner_version: description.runner_version,
            runtime_version: description.runtime_version,
            detector_sha256: description.detector_sha256,
            recognizer_sha256: description.recognizer_sha256,
        })
    }

    /// Reads `images` (at most [`MAX_RUNNER_IMAGES`]) with `build`, the build
    /// [`Self::identity`] reported, handing them over in `workspace`.
    ///
    /// # Errors
    ///
    /// [`ScreenTextReadError::BuildChanged`] when the executable or the build
    /// the run's greeting names is not `build`; [`ScreenTextReadError::Workspace`]
    /// when the hand-over files cannot be written; and the run's failures. No
    /// reading is returned for a failed or refused run.
    pub async fn read(
        &self,
        workspace: &ProcessWorkingDirectory,
        build: &ScreenTextProvider,
        images: &[ScreenTextImage<'_>],
        cancellation: ProcessCancellation,
    ) -> Result<Vec<EngineReading>, ScreenTextReadError> {
        if images.is_empty() || images.len() > MAX_RUNNER_IMAGES {
            return Err(ScreenTextReadError::ProviderFailed);
        }
        let (_, digest) = hash_file_bounded(self.executable.path(), MAX_RUNNER_EXECUTABLE_BYTES)
            .await
            .map_err(|_| ScreenTextReadError::Unavailable)?;
        if hex(&digest) != build.runner_sha256.as_str() {
            return Err(ScreenTextReadError::BuildChanged);
        }
        let directory = Dir::open_ambient_dir(workspace.path(), cap_std::ambient_authority())
            .map_err(|_| ScreenTextReadError::Workspace)?;
        let names: Vec<String> = (0..images.len())
            .map(|index| format!("ocr-frame-{index}.png"))
            .collect();
        let mut written = Vec::with_capacity(names.len());
        for (name, image) in names.iter().zip(images) {
            if write_new_file(&directory, name, image.png).is_err() {
                break;
            }
            written.push(name);
        }
        let result = if written.len() == names.len() {
            self.read_written(workspace, build, &names, images, cancellation)
                .await
        } else {
            Err(ScreenTextReadError::Workspace)
        };
        // Only the files this call created are removed; one left behind is refused
        // as stale by the next run rather than read again.
        for name in written {
            let _ = directory.remove_file(name);
        }
        result
    }

    async fn read_written(
        &self,
        workspace: &ProcessWorkingDirectory,
        build: &ScreenTextProvider,
        names: &[String],
        images: &[ScreenTextImage<'_>],
        cancellation: ProcessCancellation,
    ) -> Result<Vec<EngineReading>, ScreenTextReadError> {
        let mut arguments: Vec<OsString> = vec![
            OsString::from("read"),
            OsString::from("--threads"),
            OsString::from(self.threads.to_string()),
            OsString::from("--max-lines"),
            OsString::from(MAX_SCREEN_TEXT_LINES_PER_FRAME.to_string()),
        ];
        arguments.extend(
            names
                .iter()
                .map(|name| workspace.path().join(name).into_os_string()),
        );
        let deadline = READ_BASE_DEADLINE
            + READ_DEADLINE_PER_IMAGE * u32::try_from(images.len()).unwrap_or(u32::MAX);
        let stdout = self.run(arguments, deadline, cancellation).await?;
        let sizes: Vec<(u32, u32)> = images
            .iter()
            .map(|image| (image.dimensions.width(), image.dimensions.height()))
            .collect();
        let run = parse_runner_output(&stdout, &sizes).map_err(|error| match error {
            RunnerOutputError::TooLarge | RunnerOutputError::OverBound => {
                ScreenTextReadError::ResourceLimit
            }
            RunnerOutputError::Malformed
            | RunnerOutputError::Protocol
            | RunnerOutputError::Inconsistent => ScreenTextReadError::UnparseableOutput,
        })?;
        if !run.greeting.is_build(build) {
            return Err(ScreenTextReadError::BuildChanged);
        }
        Ok(run.readings)
    }

    async fn run(
        &self,
        arguments: Vec<OsString>,
        deadline: Duration,
        cancellation: ProcessCancellation,
    ) -> Result<Vec<u8>, ScreenTextReadError> {
        let deadline = match self.call_deadline {
            None => deadline,
            Some(call) => {
                let left = call.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(ScreenTextReadError::Deadline);
                }
                deadline.min(left)
            }
        };
        let cwd = ProcessWorkingDirectory::of_executable(&self.executable)
            .map_err(|_| ScreenTextReadError::Unavailable)?;
        let request = ProcessRequest::new(self.executable.clone(), cwd, deadline)
            .map_err(|_| ScreenTextReadError::Unavailable)?
            .with_arguments(arguments);
        let stdout = NonZeroUsize::new(MAX_RUNNER_OUTPUT_BYTES).unwrap_or(NonZeroUsize::MIN);
        let stderr = NonZeroUsize::new(STDERR_LIMIT).unwrap_or(NonZeroUsize::MIN);
        let supervisor = ProcessSupervisor::new(
            SupervisorPolicy::default().with_stream_limits(stdout, stderr),
            self.host_isolation,
        );
        let outcome = supervisor
            .run(request, cancellation)
            .await
            .map_err(|error| match error {
                ProcessError::CancelledBeforeSpawn | ProcessError::CancelledDuringStart => {
                    ScreenTextReadError::Cancelled
                }
                _ => ScreenTextReadError::Unavailable,
            })?;
        match outcome.termination {
            TerminationReason::Exited if outcome.status.success() => Ok(outcome.stdout.bytes),
            TerminationReason::Exited if crate::whisper_cli::ended_abnormally(outcome.status) => {
                Err(ScreenTextReadError::AbnormalTermination)
            }
            TerminationReason::Exited => Err(match outcome.status.code() {
                Some(3 | 4) => ScreenTextReadError::ModelUnavailable,
                _ => ScreenTextReadError::ProviderFailed,
            }),
            TerminationReason::Deadline => Err(ScreenTextReadError::Deadline),
            TerminationReason::Cancelled => Err(ScreenTextReadError::Cancelled),
            TerminationReason::OutputLimit(_) => Err(ScreenTextReadError::ResourceLimit),
        }
    }
}

fn write_new_file(directory: &Dir, name: &str, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = directory.open_with(name, &options)?;
    file.write_all(bytes)?;
    file.flush()
}

/// The runner as the application's [`ScreenTextReader`] port: each run admitted
/// for the runner's threads, its images handed over in a session's private folder.
pub struct OcrScreenTextReader<'a> {
    runner: OcrRunner,
    workspace: ProcessWorkingDirectory,
    store: &'a FilesystemSessionStore,
    cancellation: ProcessCancellation,
}

impl<'a> OcrScreenTextReader<'a> {
    /// A reader that hands frames over in `workspace`, a private VSift-owned folder.
    #[must_use]
    pub const fn new(
        runner: OcrRunner,
        workspace: ProcessWorkingDirectory,
        store: &'a FilesystemSessionStore,
        cancellation: ProcessCancellation,
    ) -> Self {
        Self {
            runner,
            workspace,
            store,
            cancellation,
        }
    }
}

impl ScreenTextReader for OcrScreenTextReader<'_> {
    async fn identity(&self) -> Result<ScreenTextProvider, ScreenTextReadError> {
        self.runner.identity(self.cancellation.clone()).await
    }

    fn max_images_per_run(&self) -> usize {
        MAX_RUNNER_IMAGES
    }

    async fn read(
        &self,
        build: &ScreenTextProvider,
        images: &[ScreenTextImage<'_>],
    ) -> Result<Vec<EngineReading>, ScreenTextReadError> {
        let _permit =
            self.store
                .try_admit(self.runner.threads().get())
                .map_err(|error| match error {
                    vsift_application::SessionStorageError::CapacityExhausted => {
                        ScreenTextReadError::ResourceLimit
                    }
                    _ => ScreenTextReadError::Busy,
                })?;
        self.runner
            .read(&self.workspace, build, images, self.cancellation.clone())
            .await
    }
}

#[cfg(test)]
mod tests;
