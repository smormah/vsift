//! The runner's command line and its two commands, over an inference [`Backend`].
//!
//! `VSift` runs the runner with a closed argument list (ADR 0030 decision 8):
//!
//! - `describe`: one `describe` record naming the runner's version, the ONNX
//!   Runtime version it loaded, its profile and its two models' SHA-256, so
//!   `VSift` can tell one engine build from another;
//! - `read --threads <1..8> --max-lines <1..256> <png>...` (one to eight images):
//!   the `runner` record, then for each image its lines and its `image` record,
//!   or its `error` record, in the order given ([`crate::output`]).
//!
//! The runtime library and the models sit beside the executable
//! ([`Layout`]), so the runner reads no environment and no configuration. A
//! failure of the whole run is one `error` record without an image and a
//! nonzero exit status ([`RunnerFailure::exit_status`]); an image that cannot be
//! read is an `error` record of its own and the run goes on.

use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read, Write},
    num::NonZeroU16,
    path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    RunnerFailure,
    detection::DetectionSettings,
    output::{ErrorRecord, Hello, PROTOCOL, reading_records, to_line},
    pipeline::{Inference, read_frame},
    png::{MAX_FILE_BYTES, decode_rgb8},
    recognition::Dictionary,
};

/// The runner's own version, as `describe` and `read` report it.
pub const RUNNER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The profile the runner reads with: `PP-OCRv5` mobile with the Latin recognizer.
pub const PROFILE: &str = "ppocrv5-latin-v1";
/// Most images one `read` takes; `VSift` hands it at most eight.
pub const MAX_IMAGES: usize = 8;
/// Most threads one run uses.
pub const MAX_THREADS: u16 = 8;
/// Most lines one image may report.
pub const MAX_LINES: usize = 256;
/// Largest model file the runner hashes or loads.
pub const MAX_MODEL_BYTES: u64 = 64 * 1024 * 1024;
/// The folder beside the executable that holds the two models.
pub const MODELS_FOLDER: &str = "models";
/// The detector's file name in [`MODELS_FOLDER`], as `RapidOCR`'s list names it.
pub const DETECTOR_FILE: &str = "ch_PP-OCRv5_det_mobile.onnx";
/// The Latin recognizer's file name in [`MODELS_FOLDER`].
pub const RECOGNIZER_FILE: &str = "latin_PP-OCRv5_rec_mobile.onnx";
/// The folder beside the executable that holds the ONNX Runtime library.
pub const RUNTIME_FOLDER: &str = "onnxruntime";

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Where the runner's files are: everything beside its own executable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Layout {
    /// The folder of the ONNX Runtime shared library.
    pub runtime: PathBuf,
    /// The detection model.
    pub detector: PathBuf,
    /// The recognition model.
    pub recognizer: PathBuf,
}

impl Layout {
    /// The layout of a runner installed in `folder`.
    #[must_use]
    pub fn beside(folder: &Path) -> Self {
        let models = folder.join(MODELS_FOLDER);
        Self {
            runtime: folder.join(RUNTIME_FOLDER),
            detector: models.join(DETECTOR_FILE),
            recognizer: models.join(RECOGNIZER_FILE),
        }
    }
}

/// The inference runtime the runner reads with.
pub trait Backend {
    /// A loaded pair of models, ready to run.
    type Session: Inference;

    /// The runtime's own version, once it is loaded from `layout.runtime`.
    ///
    /// # Errors
    ///
    /// [`RunnerFailure::RuntimeUnavailable`] when the runtime cannot be loaded.
    fn runtime_version(&self, layout: &Layout) -> Result<String, RunnerFailure>;

    /// Loads the two models with `threads` inference threads, and the
    /// recognizer's dictionary from the model's own metadata.
    ///
    /// # Errors
    ///
    /// [`RunnerFailure::RuntimeUnavailable`] or [`RunnerFailure::ModelUnavailable`].
    fn open(
        &self,
        layout: &Layout,
        threads: NonZeroU16,
    ) -> Result<(Self::Session, Dictionary), RunnerFailure>;
}

/// A parsed command line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    /// `describe`.
    Describe,
    /// `read --threads <n> --max-lines <n> <png>...`.
    Read {
        /// Inference threads.
        threads: NonZeroU16,
        /// Most lines an image reports.
        max_lines: usize,
        /// The images, in order.
        images: Vec<PathBuf>,
    },
}

impl Command {
    /// Parses the arguments after the program name.
    ///
    /// # Errors
    ///
    /// [`RunnerFailure::Usage`] for anything but the two forms above, within their bounds.
    pub fn parse(arguments: &[OsString]) -> Result<Self, RunnerFailure> {
        let text = |index: usize| arguments.get(index).and_then(|value| value.to_str());
        match text(0) {
            Some("describe") if arguments.len() == 1 => Ok(Self::Describe),
            Some("read") => {
                if text(1) != Some("--threads") || text(3) != Some("--max-lines") {
                    return Err(RunnerFailure::Usage);
                }
                let threads = text(2)
                    .and_then(|value| value.parse::<u16>().ok())
                    .filter(|threads| *threads <= MAX_THREADS)
                    .and_then(NonZeroU16::new)
                    .ok_or(RunnerFailure::Usage)?;
                let max_lines = text(4)
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|lines| (1..=MAX_LINES).contains(lines))
                    .ok_or(RunnerFailure::Usage)?;
                let images: Vec<PathBuf> = arguments.iter().skip(5).map(PathBuf::from).collect();
                if images.is_empty() || images.len() > MAX_IMAGES {
                    return Err(RunnerFailure::Usage);
                }
                Ok(Self::Read {
                    threads,
                    max_lines,
                    images,
                })
            }
            _ => Err(RunnerFailure::Usage),
        }
    }
}

/// The `describe` record.
#[derive(Debug, Serialize)]
struct Description<'a> {
    record: &'static str,
    protocol: u32,
    runner_version: &'static str,
    runtime_version: &'a str,
    profile: &'static str,
    detector_sha256: &'a str,
    recognizer_sha256: &'a str,
}

/// Runs the command line `arguments` and returns the exit status.
///
/// Records go to `stdout`, one a line; nothing else is ever written there. A
/// failure to write is [`RunnerFailure::OutputFailed`].
pub fn run(
    arguments: &[OsString],
    layout: &Layout,
    backend: &impl Backend,
    stdout: &mut impl Write,
) -> u8 {
    let outcome = match Command::parse(arguments) {
        Ok(Command::Describe) => describe(layout, backend, stdout),
        Ok(Command::Read {
            threads,
            max_lines,
            images,
        }) => read(layout, backend, stdout, threads, max_lines, &images),
        Err(failure) => Err(failure),
    };
    match outcome {
        Ok(()) => 0,
        Err(failure) => {
            if failure != RunnerFailure::OutputFailed {
                // Best effort: the status says what happened if the record cannot be written.
                let _ = write_record(stdout, &whole_run_error(failure));
            }
            failure.exit_status()
        }
    }
}

fn whole_run_error(failure: RunnerFailure) -> String {
    to_line(&ErrorRecord {
        record: "error",
        image: None,
        code: failure.code(),
    })
}

fn write_record(stdout: &mut impl Write, record: &str) -> Result<(), RunnerFailure> {
    stdout
        .write_all(record.as_bytes())
        .and_then(|()| stdout.write_all(b"\n"))
        .map_err(|_| RunnerFailure::OutputFailed)
}

fn describe(
    layout: &Layout,
    backend: &impl Backend,
    stdout: &mut impl Write,
) -> Result<(), RunnerFailure> {
    let detector = file_sha256(&layout.detector, MAX_MODEL_BYTES)
        .map_err(|_| RunnerFailure::ModelUnavailable)?;
    let recognizer = file_sha256(&layout.recognizer, MAX_MODEL_BYTES)
        .map_err(|_| RunnerFailure::ModelUnavailable)?;
    let runtime_version = backend.runtime_version(layout)?;
    write_record(
        stdout,
        &to_line(&Description {
            record: "describe",
            protocol: PROTOCOL,
            runner_version: RUNNER_VERSION,
            runtime_version: &runtime_version,
            profile: PROFILE,
            detector_sha256: &detector,
            recognizer_sha256: &recognizer,
        }),
    )?;
    stdout.flush().map_err(|_| RunnerFailure::OutputFailed)
}

fn read(
    layout: &Layout,
    backend: &impl Backend,
    stdout: &mut impl Write,
    threads: NonZeroU16,
    max_lines: usize,
    images: &[PathBuf],
) -> Result<(), RunnerFailure> {
    write_record(
        stdout,
        &to_line(&Hello {
            record: "runner",
            protocol: PROTOCOL,
            runner_version: RUNNER_VERSION,
        }),
    )?;
    let (mut session, dictionary) = backend.open(layout, threads)?;
    for (index, path) in images.iter().enumerate() {
        let frame = read_bounded(path, MAX_FILE_BYTES)
            .ok()
            .and_then(|bytes| decode_rgb8(&bytes).ok());
        let records = match frame {
            None => vec![image_error(index, RunnerFailure::ImageInvalid)],
            Some(frame) => match read_frame(
                &mut session,
                &frame,
                &dictionary,
                &DetectionSettings::PP_OCR_V5,
                max_lines,
            ) {
                Ok(reading) => reading_records(index, frame.width(), frame.height(), &reading),
                Err(_) => vec![image_error(index, RunnerFailure::InferenceFailed)],
            },
        };
        for record in records {
            write_record(stdout, &record)?;
        }
    }
    stdout.flush().map_err(|_| RunnerFailure::OutputFailed)
}

fn image_error(image: usize, failure: RunnerFailure) -> String {
    to_line(&ErrorRecord {
        record: "error",
        image: Some(image),
        code: failure.code(),
    })
}

/// Reads at most `limit` bytes of `path`; a longer file is refused.
fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(bytes)
}

/// The lower-case SHA-256 of a file of at most `limit` bytes, read in blocks.
fn file_sha256(path: &Path, limit: u64) -> io::Result<String> {
    let mut file = File::open(path)?.take(limit.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut block = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut block)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if total > limit {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        hasher.update(block.get(..read).unwrap_or_default());
    }
    let mut text = String::with_capacity(64);
    for byte in hasher.finalize() {
        text.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    Ok(text)
}

#[cfg(test)]
mod tests;
