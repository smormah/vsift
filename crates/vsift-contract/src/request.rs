//! The versioned worker request: `job-request` v1 (P11, ADR 0021).
//!
//! A supervisor hands `VSift` one request per `job run` or one per line of a
//! `job batch` file. A request names an operation id (the idempotency key),
//! the durability it needs, an optional deadline, its target (a local file
//! to ingest, relative to the operator's input root, or an existing session)
//! and at most eight steps. It can never name an executable, an environment,
//! shell text, a callback URL, an absolute path or a policy override
//! (architecture and contracts section 10): the operator, not the request,
//! owns those.
//!
//! Decoding is strict and bounded: at most 64 KiB and 16 levels of nesting,
//! checked on the raw bytes first; every member is required unless stated,
//! unknown members are refused, and every value is validated into a typed
//! [`WorkRequest`]. A refusal is a typed [`RequestRejection`] with fixed-prose
//! remediation, never an echo of the input.
//!
//! **Digest.** [`WorkRequest::digest`] is SHA-256 over the domain prefix
//! `vsift.job-request.v1` and the canonical serialization of the decoded
//! request, so whitespace, member order and an omitted or `null`
//! `deadline_ms` do not change it. The operation id is left out: it is the
//! key the digest is bound to, and the same id with another digest is
//! `IDEMPOTENCY_CONFLICT`.

mod path;
#[cfg(test)]
mod tests;

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest as _, Sha256};
use vsift_domain::{FailureCode, MediaTime, OperationId, SessionId, TimeRange, TranscriptOffset};

pub use path::{
    InputPathError, MAX_INPUT_PATH_BYTES, MAX_INPUT_PATH_COMPONENTS, RelativeInputPath,
};

use crate::{
    BundleSourceInclusion,
    input::{JsonLimits, StrictJsonError, decode_strict_json},
};

/// The byte and nesting budget of one worker request, also of one line of a
/// batch file.
pub const WORK_REQUEST_LIMITS: JsonLimits = JsonLimits {
    max_bytes: 65_536,
    max_nesting: 16,
};
/// Most steps one request lists.
pub const MAX_REQUEST_STEPS: usize = 8;
/// Longest request deadline: one day.
pub const MAX_REQUEST_DEADLINE_MS: u64 = 86_400_000;
/// Most lines one `job batch` file may hold.
pub const MAX_BATCH_LINES: usize = 1_000;
/// Longest retained-bundle name.
pub const MAX_BUNDLE_NAME_BYTES: usize = 64;

/// Domain prefix of the request digest.
const REQUEST_DIGEST_DOMAIN: &str = "vsift.job-request.v1";
/// The only schema major this build reads.
const REQUEST_SCHEMA_VERSION: &str = "1";

/// The durability a request needs from the session it creates or uses.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestDurability {
    /// Acknowledged results survive an OS crash or power loss: only in a
    /// durable workspace on the qualified profile.
    Durable,
    /// Results survive a process crash; the session is disposable.
    Ephemeral,
}

impl RequestDurability {
    /// Every durability, in declaration order.
    pub const ALL: [Self; 2] = [Self::Durable, Self::Ephemeral];

    /// The stable identifier written to `durability`.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Durable => "durable",
            Self::Ephemeral => "ephemeral",
        }
    }
}

/// A request's own deadline, 1 ms to one day; the host's maximum still applies.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RequestDeadline(u64);

impl RequestDeadline {
    /// Validates a deadline in milliseconds.
    ///
    /// # Errors
    ///
    /// [`RequestRejection::InvalidDeadline`] for 0 or more than one day.
    pub const fn from_millis(value: u64) -> Result<Self, RequestRejection> {
        if value == 0 || value > MAX_REQUEST_DEADLINE_MS {
            return Err(RequestRejection::InvalidDeadline);
        }
        Ok(Self(value))
    }

    /// The deadline in milliseconds.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }
}

/// A retained bundle's name: `[a-z0-9][a-z0-9_-]{0,63}`, a single directory
/// name below the operator's `--bundle-root`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BundleName(String);

impl BundleName {
    /// Validates a bundle name.
    ///
    /// # Errors
    ///
    /// [`RequestRejection::InvalidBundleName`] outside the grammar.
    pub fn parse(value: &str) -> Result<Self, RequestRejection> {
        let bytes = value.as_bytes();
        let valid = bytes.len() <= MAX_BUNDLE_NAME_BYTES
            && bytes
                .first()
                .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
            && bytes.iter().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
            });
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(RequestRejection::InvalidBundleName)
        }
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A supplied transcript to import with the source, as in `ingest
/// --transcript --transcript-offset`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SuppliedTranscriptInput {
    path: RelativeInputPath,
    offset: TranscriptOffset,
}

impl SuppliedTranscriptInput {
    /// The sidecar, relative to the input root.
    #[must_use]
    pub const fn path(&self) -> &RelativeInputPath {
        &self.path
    }

    /// The signed offset from file time to source time.
    #[must_use]
    pub const fn offset(&self) -> TranscriptOffset {
        self.offset
    }
}

/// What a request works on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkTarget {
    /// Copy a local file (relative to the input root) into a new session,
    /// importing a supplied transcript with it when one is named.
    Ingest {
        /// The source media.
        source: RelativeInputPath,
        /// The supplied transcript, if any.
        transcript: Option<SuppliedTranscriptInput>,
    },
    /// An existing session of the workspace.
    Session(SessionId),
}

/// One step of a request, run in order after its target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkStep {
    /// Local speech recognition of the whole source or one range.
    Retranscribe {
        /// The range; `None` for the whole source.
        range: Option<TimeRange>,
    },
    /// Visual candidates over the whole source or one range, analysing
    /// until no window of it is left unanalysed.
    Candidates {
        /// The range; `None` for the whole source.
        range: Option<TimeRange>,
    },
    /// Retain the session as a bundle under the operator's bundle root.
    Retain {
        /// The bundle's directory name.
        bundle: BundleName,
        /// Whether the bundle carries a copy of the source.
        source: BundleSourceInclusion,
    },
    /// Close the session.
    Close,
}

impl WorkStep {
    /// The step's kind.
    #[must_use]
    pub const fn kind(&self) -> WorkStepKind {
        match self {
            Self::Retranscribe { .. } => WorkStepKind::Retranscribe,
            Self::Candidates { .. } => WorkStepKind::Candidates,
            Self::Retain { .. } => WorkStepKind::Retain,
            Self::Close => WorkStepKind::Close,
        }
    }
}

/// The kind of one step of a request or of its result. `ingest` is the
/// result of an `ingest` target; the others are requested steps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkStepKind {
    /// Copying the source (and importing a supplied transcript).
    Ingest,
    /// Local speech recognition.
    Retranscribe,
    /// Visual candidates.
    Candidates,
    /// Retaining a bundle.
    Retain,
    /// Closing the session.
    Close,
}

impl WorkStepKind {
    /// Every kind, in declaration order; the `job-result` schema's `kind`
    /// enum is exactly these.
    pub const ALL: [Self; 5] = [
        Self::Ingest,
        Self::Retranscribe,
        Self::Candidates,
        Self::Retain,
        Self::Close,
    ];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Ingest => "ingest",
            Self::Retranscribe => "retranscribe",
            Self::Candidates => "candidates",
            Self::Retain => "retain",
            Self::Close => "close",
        }
    }

    const fn ordinal(self) -> usize {
        match self {
            Self::Ingest => 0,
            Self::Retranscribe => 1,
            Self::Candidates => 2,
            Self::Retain => 3,
            Self::Close => 4,
        }
    }
}

const _: () = {
    let mut index = 0;
    while index < WorkStepKind::ALL.len() {
        assert!(WorkStepKind::ALL[index].ordinal() == index);
        index += 1;
    }
};

/// The digest of a decoded request: 64 lowercase hexadecimal digits.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WorkRequestDigest(String);

impl WorkRequestDigest {
    /// Reads a stored digest back.
    ///
    /// # Errors
    ///
    /// [`vsift_domain::DigestError`] for anything but 64 lowercase
    /// hexadecimal digits.
    pub fn parse(value: &str) -> Result<Self, vsift_domain::DigestError> {
        vsift_domain::Sha256Hex::parse(value).map(|digest| Self(digest.as_str().to_owned()))
    }

    /// The digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A decoded, validated `job-request` v1.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkRequest {
    operation_id: OperationId,
    durability: RequestDurability,
    deadline: Option<RequestDeadline>,
    target: WorkTarget,
    steps: Vec<WorkStep>,
    digest: WorkRequestDigest,
}

impl WorkRequest {
    /// The caller's idempotency key.
    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// The durability the request needs.
    #[must_use]
    pub const fn durability(&self) -> RequestDurability {
        self.durability
    }

    /// The request's own deadline, if it set one.
    #[must_use]
    pub const fn deadline(&self) -> Option<RequestDeadline> {
        self.deadline
    }

    /// What the request works on.
    #[must_use]
    pub const fn target(&self) -> &WorkTarget {
        &self.target
    }

    /// The steps, in order.
    #[must_use]
    pub fn steps(&self) -> &[WorkStep] {
        &self.steps
    }

    /// The request digest (see the module documentation).
    #[must_use]
    pub const fn digest(&self) -> &WorkRequestDigest {
        &self.digest
    }
}

/// One line of a `job batch` file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchLine {
    /// A request.
    Request(Box<WorkRequest>),
    /// A line of only whitespace: skipped, but it still counts as a line.
    Blank,
}

/// Why a request (or a batch line) was refused. Every rejection has a fixed
/// identifier, a public failure code and fixed-prose remediation; none
/// echoes the input.
///
/// The first group is decided by [`decode_work_request`] alone; the last
/// three by the host that admits the request, against its workspace and
/// input root (P11 PRs 2-4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestRejection {
    /// More than 64 KiB.
    TooLarge,
    /// Nested deeper than 16 levels.
    TooDeep,
    /// Not JSON, a missing or unknown member, or a value of the wrong type.
    Malformed,
    /// A `schema_version` other than `"1"`.
    UnsupportedSchemaVersion,
    /// An `operation_id` outside the `op_` grammar.
    InvalidOperationId,
    /// A `session_id` outside the `ses_` grammar.
    InvalidSessionId,
    /// A path outside the relative input path grammar.
    InvalidPath,
    /// A transcript offset beyond twenty-four hours.
    InvalidTranscriptOffset,
    /// A deadline of 0 or more than one day.
    InvalidDeadline,
    /// A range whose end is not after its start.
    InvalidRange,
    /// A bundle name outside `[a-z0-9][a-z0-9_-]{0,63}`.
    InvalidBundleName,
    /// More than eight steps.
    TooManySteps,
    /// Steps in an order the request model refuses (see
    /// [`validate_steps`]).
    StepOrder,
    /// A session target with no step: nothing to do.
    NoWork,
    /// A path that resolves outside the operator's input root.
    PathOutsideInputRoot,
    /// An operation id another line of the same batch already used.
    DuplicateOperationId,
    /// A durable request in a workspace that is not durable.
    WorkspaceNotDurable,
}

impl RequestRejection {
    /// Every rejection, in declaration order.
    pub const ALL: [Self; 17] = [
        Self::TooLarge,
        Self::TooDeep,
        Self::Malformed,
        Self::UnsupportedSchemaVersion,
        Self::InvalidOperationId,
        Self::InvalidSessionId,
        Self::InvalidPath,
        Self::InvalidTranscriptOffset,
        Self::InvalidDeadline,
        Self::InvalidRange,
        Self::InvalidBundleName,
        Self::TooManySteps,
        Self::StepOrder,
        Self::NoWork,
        Self::PathOutsideInputRoot,
        Self::DuplicateOperationId,
        Self::WorkspaceNotDurable,
    ];

    /// The stable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::TooLarge => "request_too_large",
            Self::TooDeep => "request_too_deep",
            Self::Malformed => "malformed_request",
            Self::UnsupportedSchemaVersion => "unsupported_schema_version",
            Self::InvalidOperationId => "invalid_operation_id",
            Self::InvalidSessionId => "invalid_session_id",
            Self::InvalidPath => "invalid_path",
            Self::InvalidTranscriptOffset => "invalid_transcript_offset",
            Self::InvalidDeadline => "invalid_deadline",
            Self::InvalidRange => "invalid_range",
            Self::InvalidBundleName => "invalid_bundle_name",
            Self::TooManySteps => "too_many_steps",
            Self::StepOrder => "step_order",
            Self::NoWork => "no_work",
            Self::PathOutsideInputRoot => "path_outside_input_root",
            Self::DuplicateOperationId => "duplicate_operation_id",
            Self::WorkspaceNotDurable => "workspace_not_durable",
        }
    }

    /// The public failure code: an unknown major is `UNSUPPORTED_SCHEMA`,
    /// every other rejection `INVALID_ARGUMENT`.
    #[must_use]
    pub const fn failure_code(self) -> FailureCode {
        match self {
            Self::UnsupportedSchemaVersion => FailureCode::UnsupportedSchema,
            _ => FailureCode::InvalidArgument,
        }
    }

    /// Fixed-prose remediation.
    #[must_use]
    pub const fn remediation(self) -> &'static str {
        match self {
            Self::TooLarge => "Keep one job request, or one batch line, within 64 KiB.",
            Self::TooDeep => "Keep a job request within 16 levels of nested objects and arrays.",
            Self::Malformed => {
                "Send one JSON object with exactly the job-request v1 members (see schemas/v1/job-request.schema.json); unknown members are refused."
            }
            Self::UnsupportedSchemaVersion => {
                "Send schema_version \"1\"; this build reads job-request v1 only."
            }
            Self::InvalidOperationId => {
                "Use an operation_id of op_ followed by 16 to 64 lowercase letters or digits."
            }
            Self::InvalidSessionId => {
                "Use a session_id of ses_ followed by 16 to 64 lowercase letters or digits."
            }
            Self::InvalidPath => {
                "Name files relative to the input root with / between names: no leading /, no empty, . or .. names, no \\ or :, no control characters, no name ending in a dot or space and no device name such as CON."
            }
            Self::InvalidTranscriptOffset => {
                "Keep the transcript offset_us within plus or minus 24 hours."
            }
            Self::InvalidDeadline => {
                "Give deadline_ms from 1 to 86400000, or omit it to use the host's limit."
            }
            Self::InvalidRange => "Give a range whose to_us is greater than its from_us.",
            Self::InvalidBundleName => {
                "Name the bundle with 1 to 64 lowercase letters, digits, _ or -, starting with a letter or digit."
            }
            Self::TooManySteps => "List at most 8 steps in one job request.",
            Self::StepOrder => {
                "Put retain after every retranscribe and candidates step and close last; list retain and close at most once each."
            }
            Self::NoWork => "List at least one step for a request on an existing session.",
            Self::PathOutsideInputRoot => {
                "Name a file inside the operator's input root; links out of it are refused."
            }
            Self::DuplicateOperationId => {
                "Give every request of one batch its own operation_id; repeat a request in a later batch to replay it."
            }
            Self::WorkspaceNotDurable => {
                "Send durability ephemeral, or run the request in a workspace initialised as durable."
            }
        }
    }

    const fn ordinal(self) -> usize {
        match self {
            Self::TooLarge => 0,
            Self::TooDeep => 1,
            Self::Malformed => 2,
            Self::UnsupportedSchemaVersion => 3,
            Self::InvalidOperationId => 4,
            Self::InvalidSessionId => 5,
            Self::InvalidPath => 6,
            Self::InvalidTranscriptOffset => 7,
            Self::InvalidDeadline => 8,
            Self::InvalidRange => 9,
            Self::InvalidBundleName => 10,
            Self::TooManySteps => 11,
            Self::StepOrder => 12,
            Self::NoWork => 13,
            Self::PathOutsideInputRoot => 14,
            Self::DuplicateOperationId => 15,
            Self::WorkspaceNotDurable => 16,
        }
    }
}

const _: () = {
    let mut index = 0;
    while index < RequestRejection::ALL.len() {
        assert!(RequestRejection::ALL[index].ordinal() == index);
        index += 1;
    }
};

impl fmt::Display for RequestRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.identifier())
    }
}

impl std::error::Error for RequestRejection {}

impl From<StrictJsonError> for RequestRejection {
    fn from(error: StrictJsonError) -> Self {
        match error {
            StrictJsonError::TooLarge => Self::TooLarge,
            StrictJsonError::TooDeep => Self::TooDeep,
            StrictJsonError::Malformed(_) => Self::Malformed,
        }
    }
}

/// Decodes and validates one `job-request` v1 document.
///
/// # Errors
///
/// The first [`RequestRejection`] that applies, in this order: size, depth,
/// schema major, strict shape, then each value and the step order.
pub fn decode_work_request(bytes: &[u8]) -> Result<WorkRequest, RequestRejection> {
    // The major is read first, leniently, so a newer request is reported as
    // such rather than as a document with unknown members.
    let probe: VersionProbe = decode_strict_json(bytes, WORK_REQUEST_LIMITS)?;
    if let Some(serde_json::Value::String(version)) = &probe.schema_version
        && version != REQUEST_SCHEMA_VERSION
    {
        return Err(RequestRejection::UnsupportedSchemaVersion);
    }
    let wire: WireRequest =
        serde_json::from_slice(bytes).map_err(|_| RequestRejection::Malformed)?;
    if wire.schema_version != REQUEST_SCHEMA_VERSION {
        return Err(RequestRejection::UnsupportedSchemaVersion);
    }
    validate(wire)
}

/// Decodes one line of a `job batch` file: the bytes between two line feeds,
/// without the line feed. One trailing carriage return is ignored.
///
/// # Errors
///
/// As [`decode_work_request`]; a line that still holds a line feed is
/// [`RequestRejection::Malformed`].
pub fn decode_batch_line(line: &[u8]) -> Result<BatchLine, RequestRejection> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.contains(&b'\n') {
        return Err(RequestRejection::Malformed);
    }
    if line.iter().all(u8::is_ascii_whitespace) {
        return Ok(BatchLine::Blank);
    }
    decode_work_request(line).map(|request| BatchLine::Request(Box::new(request)))
}

/// The step order a request must follow: at most [`MAX_REQUEST_STEPS`]
/// steps; `retain` at most once and followed by nothing but `close`, so the
/// bundle holds everything the request produced; `close` at most once and
/// last; and a session target needs at least one step.
///
/// # Errors
///
/// [`RequestRejection::TooManySteps`], [`RequestRejection::StepOrder`] or
/// [`RequestRejection::NoWork`].
pub fn validate_steps(target: &WorkTarget, steps: &[WorkStep]) -> Result<(), RequestRejection> {
    if steps.len() > MAX_REQUEST_STEPS {
        return Err(RequestRejection::TooManySteps);
    }
    if steps.is_empty() && matches!(target, WorkTarget::Session(_)) {
        return Err(RequestRejection::NoWork);
    }
    let mut retained = false;
    let mut closed = false;
    for step in steps {
        if closed {
            return Err(RequestRejection::StepOrder);
        }
        match step {
            WorkStep::Retranscribe { .. } | WorkStep::Candidates { .. } if retained => {
                return Err(RequestRejection::StepOrder);
            }
            WorkStep::Retranscribe { .. } | WorkStep::Candidates { .. } => {}
            WorkStep::Retain { .. } if retained => return Err(RequestRejection::StepOrder),
            WorkStep::Retain { .. } => retained = true,
            WorkStep::Close => closed = true,
        }
    }
    Ok(())
}

fn validate(wire: WireRequest) -> Result<WorkRequest, RequestRejection> {
    let operation_id =
        OperationId::parse(wire.operation_id).map_err(|_| RequestRejection::InvalidOperationId)?;
    let deadline = wire
        .deadline_ms
        .map(RequestDeadline::from_millis)
        .transpose()?;
    let target = match wire.target {
        WireTarget::Ingest(ingest) => WorkTarget::Ingest {
            source: input_path(&ingest.source)?,
            transcript: ingest
                .transcript
                .map(|transcript| {
                    Ok::<_, RequestRejection>(SuppliedTranscriptInput {
                        path: input_path(&transcript.path)?,
                        offset: TranscriptOffset::from_micros(transcript.offset_us)
                            .map_err(|_| RequestRejection::InvalidTranscriptOffset)?,
                    })
                })
                .transpose()?,
        },
        WireTarget::SessionId(session) => WorkTarget::Session(
            SessionId::parse(session).map_err(|_| RequestRejection::InvalidSessionId)?,
        ),
    };
    if wire.steps.len() > MAX_REQUEST_STEPS {
        return Err(RequestRejection::TooManySteps);
    }
    let steps = wire
        .steps
        .into_iter()
        .map(|step| {
            Ok(match step {
                WireStep::Retranscribe(step) => WorkStep::Retranscribe {
                    range: step.range.map(WireRange::range).transpose()?,
                },
                WireStep::Candidates(step) => WorkStep::Candidates {
                    range: step.range.map(WireRange::range).transpose()?,
                },
                WireStep::Retain(step) => WorkStep::Retain {
                    bundle: BundleName::parse(&step.bundle_name)?,
                    source: if step.include_source {
                        BundleSourceInclusion::SourceIncluded
                    } else {
                        BundleSourceInclusion::EvidenceOnly
                    },
                },
                WireStep::Close(WireEmpty {}) => WorkStep::Close,
            })
        })
        .collect::<Result<Vec<_>, RequestRejection>>()?;
    validate_steps(&target, &steps)?;
    let digest = request_digest(wire.durability, deadline, &target, &steps);
    Ok(WorkRequest {
        operation_id,
        durability: wire.durability,
        deadline,
        target,
        steps,
        digest,
    })
}

fn input_path(value: &str) -> Result<RelativeInputPath, RequestRejection> {
    RelativeInputPath::parse(value).map_err(|_| RequestRejection::InvalidPath)
}

fn request_digest(
    durability: RequestDurability,
    deadline: Option<RequestDeadline>,
    target: &WorkTarget,
    steps: &[WorkStep],
) -> WorkRequestDigest {
    let canonical = CanonicalRequest {
        durability,
        deadline_ms: deadline.map(RequestDeadline::as_millis),
        target: match target {
            WorkTarget::Ingest { source, transcript } => CanonicalTarget::Ingest {
                source: source.as_str(),
                transcript: transcript.as_ref().map(|transcript| CanonicalTranscript {
                    path: transcript.path.as_str(),
                    offset_us: transcript.offset.as_micros(),
                }),
            },
            WorkTarget::Session(session) => CanonicalTarget::SessionId(session.as_str()),
        },
        steps: steps.iter().map(CanonicalStep::of).collect(),
    };
    let mut material = Vec::from(REQUEST_DIGEST_DOMAIN.as_bytes());
    material.push(b'\n');
    // Serializing owned, validated values into a vector cannot fail; an
    // empty body would still give a well-formed (if useless) digest.
    material.extend(serde_json::to_vec(&canonical).unwrap_or_default());
    WorkRequestDigest(sha256_hex(&material))
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    text
}

/// A member that must be present but may be `null`: `serde` treats a missing
/// `Option` as `None` unless a field is deserialized through a function.
fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Deserialize)]
struct VersionProbe {
    schema_version: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    schema_version: String,
    operation_id: String,
    durability: RequestDurability,
    /// The one optional member: omitted and `null` both mean "no deadline
    /// of its own".
    #[serde(default)]
    deadline_ms: Option<u64>,
    target: WireTarget,
    steps: Vec<WireStep>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum WireTarget {
    Ingest(WireIngest),
    SessionId(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireIngest {
    source: String,
    #[serde(deserialize_with = "required_nullable")]
    transcript: Option<WireTranscript>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTranscript {
    path: String,
    offset_us: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum WireStep {
    Retranscribe(WireRangeStep),
    Candidates(WireRangeStep),
    Retain(WireRetain),
    Close(WireEmpty),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRangeStep {
    #[serde(deserialize_with = "required_nullable")]
    range: Option<WireRange>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRange {
    from_us: u64,
    to_us: u64,
}

impl WireRange {
    fn range(self) -> Result<TimeRange, RequestRejection> {
        TimeRange::new(
            MediaTime::from_micros(self.from_us),
            MediaTime::from_micros(self.to_us),
        )
        .map_err(|_| RequestRejection::InvalidRange)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRetain {
    bundle_name: String,
    include_source: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEmpty {}

/// The canonical form the digest is computed over: fixed member order, no
/// whitespace, every optional member written (as `null` when absent).
#[derive(Serialize)]
struct CanonicalRequest<'a> {
    durability: RequestDurability,
    deadline_ms: Option<u64>,
    target: CanonicalTarget<'a>,
    steps: Vec<CanonicalStep<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum CanonicalTarget<'a> {
    Ingest {
        source: &'a str,
        transcript: Option<CanonicalTranscript<'a>>,
    },
    SessionId(&'a str),
}

#[derive(Serialize)]
struct CanonicalTranscript<'a> {
    path: &'a str,
    offset_us: i64,
}

#[derive(Serialize)]
struct CanonicalRange {
    from_us: u64,
    to_us: u64,
}

impl CanonicalRange {
    const fn of(range: TimeRange) -> Self {
        Self {
            from_us: range.start().as_micros(),
            to_us: range.end().as_micros(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum CanonicalStep<'a> {
    Retranscribe {
        range: Option<CanonicalRange>,
    },
    Candidates {
        range: Option<CanonicalRange>,
    },
    Retain {
        bundle_name: &'a str,
        include_source: bool,
    },
    Close {},
}

impl<'a> CanonicalStep<'a> {
    fn of(step: &'a WorkStep) -> Self {
        match step {
            WorkStep::Retranscribe { range } => Self::Retranscribe {
                range: range.map(CanonicalRange::of),
            },
            WorkStep::Candidates { range } => Self::Candidates {
                range: range.map(CanonicalRange::of),
            },
            WorkStep::Retain { bundle, source } => Self::Retain {
                bundle_name: bundle.as_str(),
                include_source: *source == BundleSourceInclusion::SourceIncluded,
            },
            WorkStep::Close => Self::Close {},
        }
    }
}
