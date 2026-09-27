//! Why a campaign step could not run.
//!
//! These are failures of the harness itself (a missing file, a command that
//! failed, a malformed log). What the campaign measures, lost
//! acknowledgements and damaged sessions, is reported as findings, never as
//! an error.

use std::{error::Error, fmt, io, path::PathBuf};

use vsift_application::{EvidenceError, JobKeyError};

use crate::logwrites::LogError;

/// A failure of the campaign harness.
#[derive(Debug)]
pub enum CampaignError {
    /// A file or device could not be read or written.
    Io {
        /// What the harness was doing.
        context: &'static str,
        /// The path involved.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
    /// An external command could not be started or failed.
    Command {
        /// The program.
        program: &'static str,
        /// Its exit status, when it ran.
        status: Option<i32>,
    },
    /// A stand-in's fixed value was rejected.
    StandIn,
    /// The clock is before the Unix epoch.
    Clock,
    /// An identifier could not be built.
    Identifier,
    /// A stand-in evidence extraction failed.
    Evidence(EvidenceError),
    /// A job key could not be derived.
    JobKey(JobKeyError),
    /// A dm-log-writes log is malformed.
    Log(LogError),
    /// A durable ingest was answered with a weaker guarantee.
    NotDurable,
    /// The acknowledgement log is damaged at these 1-based lines.
    MalformedAcks(Vec<usize>),
    /// An acknowledgement mark is missing from the write log.
    MissingMark(u64),
}

impl fmt::Display for CampaignError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                context,
                path,
                source,
            } => write!(formatter, "{context} ({}): {source}", path.display()),
            Self::Command { program, status } => match status {
                Some(code) => write!(formatter, "{program} exited with status {code}"),
                None => write!(formatter, "{program} could not be run or was killed"),
            },
            Self::StandIn => formatter.write_str("a stand-in's fixed value was rejected"),
            Self::Clock => formatter.write_str("the clock is before the Unix epoch"),
            Self::Identifier => formatter.write_str("an identifier could not be built"),
            Self::Evidence(error) => write!(formatter, "evidence stand-in: {error:?}"),
            Self::JobKey(error) => write!(formatter, "job key: {error}"),
            Self::Log(error) => write!(formatter, "write log: {error}"),
            Self::NotDurable => {
                formatter.write_str("a durable ingest was answered with a weaker guarantee")
            }
            Self::MalformedAcks(lines) => {
                write!(
                    formatter,
                    "the acknowledgement log is damaged at lines {lines:?}"
                )
            }
            Self::MissingMark(seq) => {
                write!(
                    formatter,
                    "acknowledgement {seq} has no mark in the write log"
                )
            }
        }
    }
}

impl Error for CampaignError {}

impl CampaignError {
    /// An I/O failure at `path` while doing `context`.
    pub fn io(context: &'static str, path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io {
            context,
            path,
            source,
        }
    }
}

impl From<LogError> for CampaignError {
    fn from(value: LogError) -> Self {
        Self::Log(value)
    }
}
