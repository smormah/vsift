//! The harness's typed failures.

use std::{error::Error, fmt, io, path::Path};

/// Why a harness step stopped.
///
/// A harness failure is never a trial result: a trial that ran and did
/// something wrong is a failed [`crate::grade::Grade`], while a
/// `TrialError` means the harness could not prepare, run, grade or record
/// the trial at all.
#[derive(Debug)]
pub enum TrialError {
    /// A file or directory could not be read, written or created.
    Io {
        /// What the harness was doing.
        context: String,
        /// The operating system's error.
        source: io::Error,
    },
    /// A JSON document could not be parsed or serialized.
    Json {
        /// Which document.
        context: String,
        /// The parser's error.
        source: serde_json::Error,
    },
    /// The operator's request is refused (for example a trial root inside the
    /// user's profile), so nothing was done.
    Refused(String),
    /// A scenario, skill reference, corpus record or event stream is not what
    /// the harness requires.
    Invalid(String),
    /// A child process could not be started, waited on or stopped, or did
    /// not produce what the harness needed.
    Process(String),
}

impl TrialError {
    /// An I/O failure while doing `context`.
    #[must_use]
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// An I/O failure on `path`.
    #[must_use]
    pub fn io_at(path: &Path, source: io::Error) -> Self {
        Self::io(path.display().to_string(), source)
    }

    /// A JSON failure in `context`.
    #[must_use]
    pub fn json(context: impl Into<String>, source: serde_json::Error) -> Self {
        Self::Json {
            context: context.into(),
            source,
        }
    }
}

impl fmt::Display for TrialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
            Self::Json { context, source } => write!(formatter, "{context}: {source}"),
            Self::Refused(reason) => write!(formatter, "refused: {reason}"),
            Self::Invalid(reason) => write!(formatter, "invalid: {reason}"),
            Self::Process(reason) => write!(formatter, "process: {reason}"),
        }
    }
}

impl Error for TrialError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::Refused(_) | Self::Invalid(_) | Self::Process(_) => None,
        }
    }
}

/// Reads a UTF-8 text file.
///
/// # Errors
///
/// [`TrialError::Io`] naming the file.
pub fn read_text(path: &Path) -> Result<String, TrialError> {
    std::fs::read_to_string(path).map_err(|error| TrialError::io_at(path, error))
}

/// Reads and parses a JSON file.
///
/// # Errors
///
/// [`TrialError::Io`] or [`TrialError::Json`] naming the file.
pub fn read_json(path: &Path) -> Result<serde_json::Value, TrialError> {
    serde_json::from_str(&read_text(path)?)
        .map_err(|error| TrialError::json(path.display().to_string(), error))
}

/// Writes `value` as pretty JSON, creating the parent directory.
///
/// # Errors
///
/// [`TrialError::Io`] or [`TrialError::Json`] naming the file.
pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), TrialError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| TrialError::io_at(parent, error))?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| TrialError::json(path.display().to_string(), error))?;
    std::fs::write(path, format!("{text}\n")).map_err(|error| TrialError::io_at(path, error))
}
