//! Typed engine failures and their stable public failure codes.
//!
//! Every engine operation fails with one [`EngineError`]. Variants keep the
//! typed cause (a storage error, a configuration error, a rejected plan) so a
//! host can react precisely; [`EngineError::failure_code`] gives the one stable
//! public code every host reports for it. Infrastructure error types are mirrored
//! rather than re-exported, so hosts never depend on adapter internals.

use std::{error::Error, fmt};

use vsift_application::{
    AsrFailure, AsrFailureReason, AsrStage, ClockError, IdentifierGenerationError,
    LocalAsrVerificationFailure, MediaToolFailure, MediaToolPreflightFailure, OpenSessionError,
    PlanAcceptanceError, SessionStorageError, SourceProbeError, TranscriptBuildError,
    TranscriptQueryError,
};
use vsift_contract::PrivateFolder;
use vsift_domain::{FailureCode, RuntimeDependency, TranscriptImportError};
use vsift_infrastructure::{
    ExecutableResolutionError, SessionRootError as InfrastructureSessionRootError,
    SessionStoreOpenError, UserDependencyConfigError,
};

/// Why an engine operation did not complete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineError {
    /// The disposable-session root could not be located, opened or provisioned.
    SessionRoot(SessionRootError),
    /// A session-store operation failed.
    Storage(SessionStorageError),
    /// A new session could not be opened from the selected source.
    OpenSession(OpenSessionError),
    /// A relative path could not be resolved because the process working
    /// directory is unavailable.
    WorkingDirectoryUnavailable,
    /// A supplied transcript file could not be opened or read.
    TranscriptSource(TranscriptSourceError),
    /// A supplied transcript, or its offset, was rejected by the import policy.
    TranscriptRejected(TranscriptImportError),
    /// `FFmpeg` or `FFprobe`, needed to probe the source for alignment, was
    /// neither configured nor found on the filtered `PATH`.
    MediaToolUnavailable(RuntimeDependency),
    /// The session has no transcript revision.
    TranscriptUnavailable,
    /// A requested source range is empty or reversed.
    InvalidTimeRange,
    /// A requested page size is outside 1 to 100.
    InvalidPageLimit,
    /// A transcript page request, usually its cursor, was rejected.
    TranscriptQuery(TranscriptQueryError),
    /// Cleanup was requested without restricting it to expired sessions.
    UnrestrictedCleanRejected,
    /// The session index reported a continuation that it then did not provide.
    SessionIndexInconsistent,
    /// The per-user dependency configuration could not be read or changed.
    UserConfiguration(UserConfigurationError),
    /// A dependency needed by the operation has no configured selection.
    DependencyNotSelected(RuntimeDependency),
    /// No local ASR model file is configured.
    ModelNotSelected,
    /// A selected executable failed validation.
    Executable(ExecutableRejection),
    /// The built-in reviewed catalogue or compatibility policy failed its own
    /// integrity checks.
    ReviewedPolicyInvalid,
    /// The wire contract rejected a saved plan, with the public code it assigned.
    SavedPlanRejected(FailureCode),
    /// The supplied digest does not accept the current plan.
    PlanAcceptance(PlanAcceptanceError),
    /// The injected clock could not report the current time.
    Clock(ClockError),
    /// The injected identifier source could not issue a fresh identifier.
    Identifier(IdentifierGenerationError),
    /// The automatic preflight could not verify the selected `FFmpeg` and
    /// `FFprobe` before the first media stage, so nothing was written.
    MediaToolVerificationFailed(MediaToolPreflightFailure),
    /// `FFmpeg`, `FFprobe` or whisper.cpp, needed for local speech
    /// recognition, was neither configured nor found on the filtered `PATH`.
    LocalAsrToolUnavailable(RuntimeDependency),
    /// The configured local ASR model is not an absolute, readable regular file.
    LocalAsrModelUnavailable,
    /// The configured model is not a reviewed pinned profile, so it is never
    /// run (maintainer decision D5).
    LocalAsrModelNotPinned,
    /// The automatic local-ASR preflight did not pass, so nothing was written.
    LocalAsrVerificationFailed(LocalAsrVerificationFailure),
    /// Local speech recognition failed at a typed stage; nothing was committed.
    LocalAsrFailed(AsrFailure),
    /// The source has no audio stream the media adapter decodes.
    NoAudioStream,
    /// The requested range extends past the end of the source.
    RangeOutsideSource,
    /// The session's source copy could not be probed.
    SourceProbe(SourceProbeError),
    /// The session holds no transcript revision with the requested identity.
    TranscriptRevisionNotFound,
    /// A completed recognition run could not be assembled into a revision;
    /// this is an internal fault.
    TranscriptAssembly(TranscriptBuildError),
}

impl EngineError {
    /// Returns the stable public failure code for this error.
    ///
    /// The mapping is the single authority for every host, so the same failure
    /// is reported with the same code by the CLI and by any later host.
    #[must_use]
    pub const fn failure_code(&self) -> FailureCode {
        match self {
            Self::SessionRoot(error) => error.failure_code(),
            Self::Storage(error) => storage_failure_code(*error),
            Self::OpenSession(error) => match error {
                OpenSessionError::InvalidSource => FailureCode::InvalidSource,
                OpenSessionError::SourceIo => FailureCode::StorageIo,
                OpenSessionError::InvalidClock => FailureCode::InvalidArgument,
                OpenSessionError::Storage(storage) => storage_failure_code(*storage),
                OpenSessionError::SourceProbe(probe) => probe_failure_code(*probe),
                OpenSessionError::TranscriptRejected(rejected) => {
                    transcript_failure_code(*rejected)
                }
                OpenSessionError::TranscriptInvalid(_) => FailureCode::Internal,
            },
            Self::TranscriptRejected(rejected) => transcript_failure_code(*rejected),
            Self::TranscriptSource(
                TranscriptSourceError::InvalidPath | TranscriptSourceError::NotRegularFile,
            ) => FailureCode::InvalidSource,
            Self::UserConfiguration(error) => error.failure_code(),
            Self::SavedPlanRejected(code) => *code,
            Self::WorkingDirectoryUnavailable
            | Self::TranscriptSource(TranscriptSourceError::Io)
            | Self::Executable(ExecutableRejection::Uninspectable) => FailureCode::StorageIo,
            Self::UnrestrictedCleanRejected
            | Self::PlanAcceptance(_)
            | Self::TranscriptUnavailable
            | Self::InvalidTimeRange
            | Self::InvalidPageLimit
            | Self::TranscriptQuery(_)
            | Self::NoAudioStream
            | Self::RangeOutsideSource
            | Self::TranscriptRevisionNotFound
            | Self::Executable(
                ExecutableRejection::NotAbsolute
                | ExecutableRejection::NotRegularFile
                | ExecutableRejection::Invalid,
            ) => FailureCode::InvalidArgument,
            Self::DependencyNotSelected(_)
            | Self::MediaToolUnavailable(_)
            | Self::ModelNotSelected
            | Self::LocalAsrToolUnavailable(_)
            | Self::LocalAsrModelUnavailable
            | Self::LocalAsrModelNotPinned
            | Self::Executable(ExecutableRejection::NotFound) => FailureCode::MissingCapability,
            Self::SessionIndexInconsistent
            | Self::ReviewedPolicyInvalid
            | Self::Clock(_)
            | Self::Identifier(_)
            | Self::TranscriptAssembly(_) => FailureCode::Internal,
            Self::MediaToolVerificationFailed(failure) => {
                media_tool_verification_failure_code(failure.failure)
            }
            Self::LocalAsrVerificationFailed(failure) => {
                local_asr_verification_failure_code(*failure)
            }
            Self::LocalAsrFailed(failure) => asr_failure_code(*failure),
            Self::SourceProbe(error) => probe_failure_code(*error),
        }
    }
}

/// Public code for a failed local speech-recognition run (ADR 0017).
///
/// A recognizer or model that cannot be used, or that produced unusable
/// output, means no working capability; exhausted bounds (including a
/// recognizer that ended abnormally, usually for lack of memory) are resource
/// limits; audio that cannot be decoded is the source's fault. `Io` is storage
/// when the session's source copy or decoded audio could not be read, and a
/// missing capability when a recognizer process could not run.
pub(crate) const fn asr_failure_code(failure: AsrFailure) -> FailureCode {
    match failure.reason {
        AsrFailureReason::InvalidRange => FailureCode::InvalidArgument,
        AsrFailureReason::TooManyChunks
        | AsrFailureReason::ResourceLimit
        | AsrFailureReason::AbnormalTermination => FailureCode::ResourceLimit,
        AsrFailureReason::ModelChanged
        | AsrFailureReason::ModelUnavailable
        | AsrFailureReason::UnpinnedModel
        | AsrFailureReason::ProviderFailed
        | AsrFailureReason::UnparseableOutput
        | AsrFailureReason::MalformedOutput(_) => FailureCode::MissingCapability,
        AsrFailureReason::Cancelled => FailureCode::Cancelled,
        AsrFailureReason::Deadline => FailureCode::DeadlineExceeded,
        AsrFailureReason::Busy => FailureCode::Busy,
        AsrFailureReason::AudioUnavailable => FailureCode::InvalidSource,
        AsrFailureReason::Workspace => FailureCode::StorageIo,
        AsrFailureReason::Io => match failure.stage {
            AsrStage::AudioExtraction => FailureCode::StorageIo,
            AsrStage::Planning
            | AsrStage::RecognizerIdentity
            | AsrStage::Recognition
            | AsrStage::OutputValidation
            | AsrStage::Assembly => FailureCode::MissingCapability,
        },
        AsrFailureReason::InvalidRun(_) => FailureCode::Internal,
    }
}

/// Public code for a failed local-ASR preflight: the same as a run for a
/// failed transcription, and an unusable capability for a transcript that
/// missed the fixture's words.
const fn local_asr_verification_failure_code(failure: LocalAsrVerificationFailure) -> FailureCode {
    match failure {
        LocalAsrVerificationFailure::FixtureIntegrity => FailureCode::Internal,
        LocalAsrVerificationFailure::Workspace => FailureCode::StorageIo,
        LocalAsrVerificationFailure::FixtureMedia
        | LocalAsrVerificationFailure::UnexpectedTranscript => FailureCode::MissingCapability,
        LocalAsrVerificationFailure::Transcription(failure) => asr_failure_code(failure),
    }
}

impl EngineError {
    /// The typed transcript rejection behind this error, if any.
    ///
    /// Hosts use it to tell the caller how to correct a supplied transcript
    /// or its offset; the rejection may come from parsing or from alignment.
    #[must_use]
    pub const fn transcript_rejection(&self) -> Option<TranscriptImportError> {
        match self {
            Self::TranscriptRejected(rejection)
            | Self::OpenSession(OpenSessionError::TranscriptRejected(rejection)) => {
                Some(*rejection)
            }
            _ => None,
        }
    }

    /// The existing `VSift` folder that was refused because other accounts
    /// can access it, if that is why the operation failed.
    ///
    /// Hosts use it to explain which folder to fix; the folder was not used
    /// or changed. Only the observed permission finding qualifies: links,
    /// unreadable permissions and I/O failures are not reported here.
    #[must_use]
    pub const fn non_private_folder(&self) -> Option<PrivateFolder> {
        match self {
            Self::UserConfiguration(UserConfigurationError::StorageNotPrivate) => {
                Some(PrivateFolder::UserConfiguration)
            }
            Self::SessionRoot(SessionRootError::NotPrivate) => Some(PrivateFolder::SessionRoot),
            _ => None,
        }
    }

    /// The media tool that transcript alignment needed but could not find.
    #[must_use]
    pub const fn missing_media_tool(&self) -> Option<RuntimeDependency> {
        match self {
            Self::MediaToolUnavailable(dependency) => Some(*dependency),
            _ => None,
        }
    }

    /// The check and reason that stopped the automatic media-tool preflight.
    ///
    /// Hosts use it to tell the caller which check failed and how to select
    /// working tools; the operation wrote nothing.
    #[must_use]
    pub const fn media_tool_verification_failure(&self) -> Option<MediaToolPreflightFailure> {
        match self {
            Self::MediaToolVerificationFailed(failure) => Some(*failure),
            _ => None,
        }
    }
}

/// Public code for a failed media-tool preflight.
///
/// A tool that cannot be started, rejects the reviewed fixture, floods its
/// output or returns wrong results means no working media capability is
/// available. A workspace that cannot be prepared is local storage, a deadline
/// is a deadline, and a corrupt embedded fixture is a defect in this build.
const fn media_tool_verification_failure_code(failure: MediaToolFailure) -> FailureCode {
    match failure {
        MediaToolFailure::ProcessFailure
        | MediaToolFailure::ProviderRejected
        | MediaToolFailure::OutputLimit
        | MediaToolFailure::UnexpectedResult => FailureCode::MissingCapability,
        MediaToolFailure::Workspace => FailureCode::StorageIo,
        MediaToolFailure::Deadline => FailureCode::DeadlineExceeded,
        MediaToolFailure::Cancelled => FailureCode::Cancelled,
        MediaToolFailure::FixtureIntegrity => FailureCode::Internal,
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionRoot(error) => error.fmt(formatter),
            Self::Storage(error) => error.fmt(formatter),
            Self::OpenSession(error) => error.fmt(formatter),
            Self::WorkingDirectoryUnavailable => {
                formatter.write_str("working directory is unavailable")
            }
            Self::TranscriptSource(error) => error.fmt(formatter),
            Self::TranscriptRejected(error) => error.fmt(formatter),
            Self::MediaToolUnavailable(dependency) => write!(
                formatter,
                "{} is needed to align a supplied transcript but was not found",
                dependency.display_name()
            ),
            Self::TranscriptUnavailable => formatter.write_str("session has no transcript"),
            Self::InvalidTimeRange => {
                formatter.write_str("time range must have a positive duration")
            }
            Self::InvalidPageLimit => formatter.write_str("page limit must be 1 to 100"),
            Self::TranscriptQuery(error) => error.fmt(formatter),
            Self::UnrestrictedCleanRejected => {
                formatter.write_str("cleanup must be restricted to expired sessions")
            }
            Self::SessionIndexInconsistent => formatter.write_str("session index is inconsistent"),
            Self::UserConfiguration(error) => error.fmt(formatter),
            Self::DependencyNotSelected(dependency) => write!(
                formatter,
                "no {} executable is selected",
                dependency.display_name()
            ),
            Self::ModelNotSelected => formatter.write_str("no local ASR model is selected"),
            Self::Executable(error) => error.fmt(formatter),
            Self::ReviewedPolicyInvalid => {
                formatter.write_str("built-in reviewed policy failed its integrity check")
            }
            Self::SavedPlanRejected(code) => {
                write!(formatter, "saved plan was rejected ({})", code.identifier())
            }
            Self::PlanAcceptance(PlanAcceptanceError::ManagedUnavailable) => {
                formatter.write_str("no managed plan is available to accept")
            }
            Self::PlanAcceptance(PlanAcceptanceError::DigestMismatch) => {
                formatter.write_str("plan digest does not match the current plan")
            }
            Self::Clock(error) => error.fmt(formatter),
            Self::Identifier(error) => error.fmt(formatter),
            Self::MediaToolVerificationFailed(failure) => write!(
                formatter,
                "selected FFmpeg and FFprobe failed verification at the {} check ({})",
                failure.check.identifier(),
                failure.failure.identifier()
            ),
            Self::LocalAsrToolUnavailable(dependency) => write!(
                formatter,
                "{} is needed for local speech recognition but was not found",
                dependency.display_name()
            ),
            Self::LocalAsrModelUnavailable => {
                formatter.write_str("the configured local ASR model cannot be read")
            }
            Self::LocalAsrModelNotPinned => {
                formatter.write_str("the configured local ASR model is not a reviewed pinned model")
            }
            Self::LocalAsrVerificationFailed(failure) => match failure {
                LocalAsrVerificationFailure::Transcription(asr) => {
                    write!(
                        formatter,
                        "local speech recognition failed verification: {asr}"
                    )
                }
                other => write!(
                    formatter,
                    "local speech recognition failed verification ({})",
                    other.identifier()
                ),
            },
            Self::LocalAsrFailed(failure) => failure.fmt(formatter),
            Self::NoAudioStream => formatter.write_str("the source has no decodable audio stream"),
            Self::RangeOutsideSource => {
                formatter.write_str("the requested range extends past the end of the source")
            }
            Self::SourceProbe(error) => error.fmt(formatter),
            Self::TranscriptRevisionNotFound => {
                formatter.write_str("the session has no transcript revision with that identity")
            }
            Self::TranscriptAssembly(error) => error.fmt(formatter),
        }
    }
}

impl Error for EngineError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SessionRoot(error) => Some(error),
            Self::Storage(error) => Some(error),
            Self::OpenSession(error) => Some(error),
            Self::UserConfiguration(error) => Some(error),
            Self::Executable(error) => Some(error),
            Self::Clock(error) => Some(error),
            Self::Identifier(error) => Some(error),
            Self::TranscriptSource(error) => Some(error),
            Self::TranscriptRejected(error) => Some(error),
            Self::TranscriptQuery(error) => Some(error),
            Self::LocalAsrFailed(error) => Some(error),
            Self::SourceProbe(error) => Some(error),
            Self::TranscriptAssembly(error) => Some(error),
            Self::LocalAsrToolUnavailable(_)
            | Self::LocalAsrModelUnavailable
            | Self::LocalAsrModelNotPinned
            | Self::LocalAsrVerificationFailed(_)
            | Self::NoAudioStream
            | Self::RangeOutsideSource
            | Self::TranscriptRevisionNotFound
            | Self::WorkingDirectoryUnavailable
            | Self::MediaToolUnavailable(_)
            | Self::MediaToolVerificationFailed(_)
            | Self::TranscriptUnavailable
            | Self::InvalidTimeRange
            | Self::InvalidPageLimit
            | Self::UnrestrictedCleanRejected
            | Self::SessionIndexInconsistent
            | Self::DependencyNotSelected(_)
            | Self::ModelNotSelected
            | Self::ReviewedPolicyInvalid
            | Self::SavedPlanRejected(_)
            | Self::PlanAcceptance(_) => None,
        }
    }
}

impl From<EngineError> for FailureCode {
    /// Presents an engine error as its stable public code at a host boundary.
    fn from(value: EngineError) -> Self {
        value.failure_code()
    }
}

impl From<SessionStorageError> for EngineError {
    fn from(value: SessionStorageError) -> Self {
        Self::Storage(value)
    }
}

impl From<SessionRootError> for EngineError {
    fn from(value: SessionRootError) -> Self {
        Self::SessionRoot(value)
    }
}

impl From<UserDependencyConfigError> for EngineError {
    fn from(value: UserDependencyConfigError) -> Self {
        Self::UserConfiguration(UserConfigurationError::from(value))
    }
}

impl From<ClockError> for EngineError {
    fn from(value: ClockError) -> Self {
        Self::Clock(value)
    }
}

impl From<IdentifierGenerationError> for EngineError {
    fn from(value: IdentifierGenerationError) -> Self {
        Self::Identifier(value)
    }
}

const fn storage_failure_code(error: SessionStorageError) -> FailureCode {
    match error {
        SessionStorageError::UnsupportedGuarantee { .. } => FailureCode::MissingCapability,
        SessionStorageError::Busy => FailureCode::Busy,
        SessionStorageError::StateConflict => FailureCode::InvalidArgument,
        SessionStorageError::IntegrityFailure => FailureCode::IntegrityFailure,
        SessionStorageError::UnsupportedVersion => FailureCode::UnsupportedSchema,
        SessionStorageError::AccessDenied | SessionStorageError::Io => FailureCode::StorageIo,
        SessionStorageError::CapacityExhausted => FailureCode::ResourceLimit,
    }
}

/// Public code for a rejected transcript import.
///
/// Budget violations are resource limits, a wrong offset or source is the
/// caller's alignment request, and everything else is malformed input.
const fn transcript_failure_code(error: TranscriptImportError) -> FailureCode {
    let rejection = error.rejection();
    if rejection.is_resource_limit() {
        FailureCode::ResourceLimit
    } else if rejection.is_alignment() {
        FailureCode::InvalidArgument
    } else {
        FailureCode::InvalidSource
    }
}

/// Public code for a failed source-duration probe.
const fn probe_failure_code(error: SourceProbeError) -> FailureCode {
    match error {
        SourceProbeError::InvalidSource => FailureCode::InvalidSource,
        SourceProbeError::Busy => FailureCode::Busy,
        SourceProbeError::Deadline => FailureCode::DeadlineExceeded,
        SourceProbeError::Cancelled => FailureCode::Cancelled,
        SourceProbeError::ResourceLimit => FailureCode::ResourceLimit,
        SourceProbeError::Io => FailureCode::MissingCapability,
    }
}

/// Why a supplied transcript file could not be opened or read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptSourceError {
    /// The path is not an absolute, local, directly named file.
    InvalidPath,
    /// The path names something other than a regular file.
    NotRegularFile,
    /// The file could not be read.
    Io,
}

impl fmt::Display for TranscriptSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPath => "supplied transcript path is invalid",
            Self::NotRegularFile => "supplied transcript is not a regular file",
            Self::Io => "supplied transcript could not be read",
        })
    }
}

impl Error for TranscriptSourceError {}

/// Why the disposable-session root could not be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionRootError {
    /// An explicitly selected or platform-derived root is not absolute.
    NotAbsolute,
    /// The platform variable naming the per-user cache is not set.
    PlatformDefaultUnavailable,
    /// The root, or the parent it would be created in, has no parent directory.
    WithoutParent,
    /// The directory that must contain the root's parent is missing, or the
    /// parent could not be created.
    ParentUnavailable,
    /// No session root exists yet, so there is no session to act on.
    Missing,
    /// The root is not a directory.
    NotDirectory,
    /// The root's permissions allow access outside the owning user.
    NotPrivate,
    /// Provisioning found the root already present.
    AlreadyExists,
    /// The root's recorded admission capacity is outside the supported bound.
    InvalidAdmissionCapacity,
    /// The root's ownership marker is absent or not a `VSift` marker.
    InvalidOwnership,
    /// A required contained directory or lock anchor is invalid.
    InvalidLayout,
    /// The root exists but cannot be opened safely.
    Unavailable,
    /// Another process was still creating the root when the bounded wait for
    /// it ended; a retry is expected to succeed.
    ProvisioningInProgress,
}

impl SessionRootError {
    /// Returns the stable public failure code.
    #[must_use]
    pub const fn failure_code(self) -> FailureCode {
        match self {
            Self::NotAbsolute
            | Self::WithoutParent
            | Self::NotDirectory
            | Self::AlreadyExists
            | Self::InvalidAdmissionCapacity => FailureCode::InvalidArgument,
            Self::PlatformDefaultUnavailable => FailureCode::MissingCapability,
            // A root other accounts can access is storage that cannot be used,
            // whether it was selected or is the platform default, reported like
            // a non-private per-user configuration folder.
            Self::ParentUnavailable | Self::Missing | Self::Unavailable | Self::NotPrivate => {
                FailureCode::StorageIo
            }
            Self::InvalidOwnership | Self::InvalidLayout => FailureCode::IntegrityFailure,
            Self::ProvisioningInProgress => FailureCode::Busy,
        }
    }
}

impl fmt::Display for SessionRootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotAbsolute => "session root must be absolute",
            Self::PlatformDefaultUnavailable => "per-user session cache location is unavailable",
            Self::WithoutParent => "session root has no parent directory",
            Self::ParentUnavailable => "session root parent directory is unavailable",
            Self::Missing => "session root does not exist",
            Self::NotDirectory => "session root is not a directory",
            Self::NotPrivate => "session root permissions are not private",
            Self::AlreadyExists => "session root already exists",
            Self::InvalidAdmissionCapacity => "session root admission capacity is invalid",
            Self::InvalidOwnership => "session root ownership marker is invalid",
            Self::InvalidLayout => "session root layout is invalid",
            Self::Unavailable => "session root is unavailable",
            Self::ProvisioningInProgress => {
                "session root is still being created by another process"
            }
        })
    }
}

impl Error for SessionRootError {}

impl From<SessionStoreOpenError> for SessionRootError {
    fn from(value: SessionStoreOpenError) -> Self {
        match value {
            SessionStoreOpenError::RootMustBeAbsolute => Self::NotAbsolute,
            SessionStoreOpenError::RootUnavailable => Self::Unavailable,
            SessionStoreOpenError::RootNotDirectory => Self::NotDirectory,
            SessionStoreOpenError::RootNotPrivate => Self::NotPrivate,
            SessionStoreOpenError::InvalidOwnership => Self::InvalidOwnership,
            SessionStoreOpenError::InvalidLayout => Self::InvalidLayout,
            SessionStoreOpenError::RootAlreadyExists => Self::AlreadyExists,
            SessionStoreOpenError::InvalidAdmissionCapacity => Self::InvalidAdmissionCapacity,
        }
    }
}

impl From<InfrastructureSessionRootError> for SessionRootError {
    fn from(value: InfrastructureSessionRootError) -> Self {
        match value {
            InfrastructureSessionRootError::RootMustBeAbsolute => Self::NotAbsolute,
            InfrastructureSessionRootError::RootWithoutParent => Self::WithoutParent,
            InfrastructureSessionRootError::ParentUnavailable => Self::ParentUnavailable,
            InfrastructureSessionRootError::ProvisioningInProgress => Self::ProvisioningInProgress,
            InfrastructureSessionRootError::Store(error) => Self::from(error),
        }
    }
}

/// Why the per-user dependency configuration could not be read or changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserConfigurationError {
    /// The per-user configuration location is unavailable or relative.
    Unavailable,
    /// A selected executable is not an absolute regular file.
    InvalidExecutable,
    /// A selected model is not an absolute, nonempty regular file.
    InvalidModel,
    /// The configuration directory or file is a link, not regular, or cannot
    /// be inspected or written safely.
    UnsafeStorage,
    /// The existing configuration directory is accessible to other accounts.
    /// It was left unchanged.
    StorageNotPrivate,
    /// The stored record is malformed.
    InvalidRecord,
    /// A concurrent writer holds the configuration lock.
    Busy,
    /// The operating system could not complete a storage operation.
    Io,
}

impl UserConfigurationError {
    /// Returns the stable public failure code.
    #[must_use]
    pub const fn failure_code(self) -> FailureCode {
        match self {
            Self::Unavailable => FailureCode::MissingCapability,
            Self::InvalidExecutable | Self::InvalidModel => FailureCode::InvalidArgument,
            // A folder other accounts can access is storage that cannot be
            // used, not a malformed record or a missing capability; it is not
            // retryable until the user fixes it, which the remediation explains.
            Self::UnsafeStorage | Self::StorageNotPrivate | Self::Io => FailureCode::StorageIo,
            Self::InvalidRecord => FailureCode::IntegrityFailure,
            Self::Busy => FailureCode::Busy,
        }
    }
}

impl fmt::Display for UserConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "per-user configuration location is unavailable",
            Self::InvalidExecutable => "selected executable is not an absolute regular file",
            Self::InvalidModel => "selected model is not an absolute nonempty regular file",
            Self::UnsafeStorage => "per-user configuration storage is not private",
            Self::StorageNotPrivate => {
                "per-user configuration folder is accessible to other accounts"
            }
            Self::InvalidRecord => "per-user dependency configuration is invalid",
            Self::Busy => "per-user dependency configuration is busy",
            Self::Io => "per-user dependency configuration I/O failed",
        })
    }
}

impl Error for UserConfigurationError {}

impl From<UserDependencyConfigError> for UserConfigurationError {
    fn from(value: UserDependencyConfigError) -> Self {
        match value {
            UserDependencyConfigError::Unavailable => Self::Unavailable,
            UserDependencyConfigError::InvalidExecutable => Self::InvalidExecutable,
            UserDependencyConfigError::InvalidModel => Self::InvalidModel,
            UserDependencyConfigError::UnsafeStorage => Self::UnsafeStorage,
            UserDependencyConfigError::StorageNotPrivate => Self::StorageNotPrivate,
            UserDependencyConfigError::InvalidRecord => Self::InvalidRecord,
            UserDependencyConfigError::Busy => Self::Busy,
            UserDependencyConfigError::Io => Self::Io,
        }
    }
}

/// Why a selected executable was not accepted for verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutableRejection {
    /// The path is relative.
    NotAbsolute,
    /// Nothing exists at the path.
    NotFound,
    /// The path does not name a regular file.
    NotRegularFile,
    /// The path's metadata could not be inspected safely.
    Uninspectable,
    /// The selection is otherwise invalid.
    Invalid,
}

impl fmt::Display for ExecutableRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotAbsolute => "executable path must be absolute",
            Self::NotFound => "executable was not found",
            Self::NotRegularFile => "executable is not a regular file",
            Self::Uninspectable => "executable could not be inspected",
            Self::Invalid => "executable selection is invalid",
        })
    }
}

impl Error for ExecutableRejection {}

impl From<&ExecutableResolutionError> for ExecutableRejection {
    fn from(value: &ExecutableResolutionError) -> Self {
        match value {
            ExecutableResolutionError::PathNotAbsolute => Self::NotAbsolute,
            ExecutableResolutionError::NotFound => Self::NotFound,
            ExecutableResolutionError::NotRegularFile => Self::NotRegularFile,
            ExecutableResolutionError::Inspection(_) => Self::Uninspectable,
            ExecutableResolutionError::SearchDirectoryNotAbsolute
            | ExecutableResolutionError::InvalidProviderName => Self::Invalid,
        }
    }
}

#[cfg(test)]
mod tests {
    use vsift_application::{OpenSessionError, SessionStorageError};
    use vsift_domain::FailureCode;
    use vsift_infrastructure::{SessionStoreOpenError, UserDependencyConfigError};

    use super::{EngineError, SessionRootError};

    #[test]
    fn storage_failures_keep_their_public_codes() {
        for (error, code) in [
            (SessionStorageError::Busy, FailureCode::Busy),
            (
                SessionStorageError::StateConflict,
                FailureCode::InvalidArgument,
            ),
            (
                SessionStorageError::IntegrityFailure,
                FailureCode::IntegrityFailure,
            ),
            (
                SessionStorageError::UnsupportedVersion,
                FailureCode::UnsupportedSchema,
            ),
            (SessionStorageError::AccessDenied, FailureCode::StorageIo),
            (SessionStorageError::Io, FailureCode::StorageIo),
            (
                SessionStorageError::CapacityExhausted,
                FailureCode::ResourceLimit,
            ),
        ] {
            assert_eq!(EngineError::Storage(error).failure_code(), code);
            assert_eq!(
                EngineError::OpenSession(OpenSessionError::Storage(error)).failure_code(),
                code
            );
        }
        assert_eq!(
            EngineError::OpenSession(OpenSessionError::InvalidClock).failure_code(),
            FailureCode::InvalidArgument
        );
    }

    #[test]
    fn store_open_failures_map_through_the_root_error() {
        for (error, code) in [
            (
                SessionStoreOpenError::RootNotPrivate,
                FailureCode::StorageIo,
            ),
            (
                SessionStoreOpenError::InvalidLayout,
                FailureCode::IntegrityFailure,
            ),
            (
                SessionStoreOpenError::RootUnavailable,
                FailureCode::StorageIo,
            ),
        ] {
            assert_eq!(
                EngineError::SessionRoot(SessionRootError::from(error)).failure_code(),
                code
            );
        }
    }

    #[test]
    fn a_root_still_being_created_is_retryable_busy() {
        let error =
            SessionRootError::from(vsift_infrastructure::SessionRootError::ProvisioningInProgress);
        assert_eq!(error, SessionRootError::ProvisioningInProgress);
        assert_eq!(
            EngineError::SessionRoot(error).failure_code(),
            FailureCode::Busy
        );
    }

    #[test]
    fn media_tool_preflight_failures_map_by_reason() {
        use vsift_application::{MediaToolCheck, MediaToolFailure, MediaToolPreflightFailure};

        for (failure, code) in [
            (
                MediaToolFailure::ProcessFailure,
                FailureCode::MissingCapability,
            ),
            (
                MediaToolFailure::ProviderRejected,
                FailureCode::MissingCapability,
            ),
            (
                MediaToolFailure::OutputLimit,
                FailureCode::MissingCapability,
            ),
            (
                MediaToolFailure::UnexpectedResult,
                FailureCode::MissingCapability,
            ),
            (MediaToolFailure::Workspace, FailureCode::StorageIo),
            (MediaToolFailure::Deadline, FailureCode::DeadlineExceeded),
            (MediaToolFailure::Cancelled, FailureCode::Cancelled),
            (MediaToolFailure::FixtureIntegrity, FailureCode::Internal),
        ] {
            let preflight = MediaToolPreflightFailure {
                check: MediaToolCheck::Probe,
                failure,
            };
            let error = EngineError::MediaToolVerificationFailed(preflight);
            assert_eq!(error.failure_code(), code);
            assert_eq!(error.media_tool_verification_failure(), Some(preflight));
        }
    }

    #[test]
    fn configuration_failures_keep_their_public_codes() {
        for (error, code) in [
            (
                UserDependencyConfigError::Unavailable,
                FailureCode::MissingCapability,
            ),
            (
                UserDependencyConfigError::InvalidModel,
                FailureCode::InvalidArgument,
            ),
            (
                UserDependencyConfigError::InvalidRecord,
                FailureCode::IntegrityFailure,
            ),
            (UserDependencyConfigError::Busy, FailureCode::Busy),
            (UserDependencyConfigError::Io, FailureCode::StorageIo),
            (
                UserDependencyConfigError::StorageNotPrivate,
                FailureCode::StorageIo,
            ),
        ] {
            assert_eq!(EngineError::from(error).failure_code(), code);
        }
    }

    #[test]
    fn only_a_folder_other_accounts_can_access_is_reported_as_not_private() {
        use vsift_contract::PrivateFolder;

        assert_eq!(
            EngineError::from(UserDependencyConfigError::StorageNotPrivate).non_private_folder(),
            Some(PrivateFolder::UserConfiguration)
        );
        assert_eq!(
            EngineError::SessionRoot(SessionRootError::from(
                SessionStoreOpenError::RootNotPrivate
            ))
            .non_private_folder(),
            Some(PrivateFolder::SessionRoot)
        );
        for error in [
            EngineError::from(UserDependencyConfigError::UnsafeStorage),
            EngineError::from(UserDependencyConfigError::Io),
            EngineError::SessionRoot(SessionRootError::Unavailable),
            EngineError::Storage(SessionStorageError::AccessDenied),
        ] {
            assert_eq!(error.non_private_folder(), None);
        }
    }
}
