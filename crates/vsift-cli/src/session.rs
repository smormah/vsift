//! Public presentation and composition for disposable desktop sessions.

use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use vsift_application::{OpenSession, OpenSessionError, OpenSessionRequest, SessionStorageError};
use vsift_domain::{DurabilityRequirement, FailureCode, OperationId, SessionId, SessionPhase};
use vsift_infrastructure::{
    BundleSourcePolicy, BundleStatus, CleanOutcome, FilesystemSessionStore, SessionStatus,
    SessionStoreOpenError,
};

use crate::{
    command::{IngestArguments, SessionCommand},
    output::{LifecycleResponse, OperationResponse},
};

type Response = OperationResponse<serde_json::Value>;
const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Serialize)]
struct OpenData {
    session_id: String,
    source_id: String,
    source_bytes: u64,
    generation: u64,
    publication: &'static str,
    expires_at: String,
}

#[derive(Serialize)]
struct StatusData {
    session_id: String,
    state: &'static str,
    source_id: String,
    source_bytes: u64,
    artifact_count: usize,
    artifact_bytes: u64,
    generation: u64,
    expires_at: String,
}

#[derive(Serialize)]
struct PageData {
    items: Vec<ListedSession>,
    next_cursor: Option<u16>,
}

#[derive(Serialize)]
struct ListedSession {
    session_id: String,
    state: &'static str,
    status: Option<StatusData>,
    error_code: Option<&'static str>,
}

#[derive(Serialize)]
struct CleanItem {
    session_id: String,
    outcome: &'static str,
    error_code: Option<&'static str>,
}

#[derive(Serialize)]
struct CleanData {
    items: Vec<CleanItem>,
    next_cursor: Option<u16>,
    dry_run: bool,
}

#[derive(Serialize)]
struct BundleData {
    session_id: String,
    source_id: String,
    source_bytes: u64,
    source_included: bool,
    artifact_count: usize,
    artifact_bytes: u64,
    reextraction_requires_matching_original: bool,
    publication: &'static str,
}

fn now_seconds() -> Result<u64, FailureCode> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| FailureCode::Internal)
}

fn rfc3339(seconds: u64) -> Result<String, FailureCode> {
    let seconds = i64::try_from(seconds).map_err(|_| FailureCode::InvalidArgument)?;
    OffsetDateTime::from_unix_timestamp(seconds)
        .map_err(|_| FailureCode::InvalidArgument)?
        .format(&Rfc3339)
        .map_err(|_| FailureCode::Internal)
}

fn new_id(prefix: &str) -> Result<String, FailureCode> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| FailureCode::Internal)?;
    let mut result = String::with_capacity(prefix.len() + 32);
    result.push_str(prefix);
    for byte in random {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(result)
}

fn session_id() -> Result<SessionId, FailureCode> {
    SessionId::parse(&new_id("ses_")?).map_err(|_| FailureCode::Internal)
}

fn operation_id() -> Result<OperationId, FailureCode> {
    OperationId::parse(&new_id("op_")?).map_err(|_| FailureCode::Internal)
}

fn root_path(explicit: Option<&Path>) -> Result<PathBuf, FailureCode> {
    if let Some(path) = explicit {
        return if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            Err(FailureCode::InvalidArgument)
        };
    }
    #[cfg(windows)]
    {
        let base = env::var_os("LOCALAPPDATA").ok_or(FailureCode::MissingCapability)?;
        Ok(PathBuf::from(base).join("VSift-sessions"))
    }
    #[cfg(target_os = "macos")]
    {
        let home = env::var_os("HOME").ok_or(FailureCode::MissingCapability)?;
        Ok(PathBuf::from(home).join("Library/Caches/VSift-sessions"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let base = env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .ok_or(FailureCode::MissingCapability)?;
        Ok(base.join("vsift-sessions"))
    }
}

fn absolute_selection(path: &Path) -> Result<PathBuf, FailureCode> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|_| FailureCode::StorageIo)
    }
}

fn open_store(root: &Path, create: bool) -> Result<Option<FilesystemSessionStore>, FailureCode> {
    if !root.is_absolute() {
        return Err(FailureCode::InvalidArgument);
    }
    if root.exists() {
        return FilesystemSessionStore::open_existing(root)
            .map(Some)
            .map_err(map_open_error);
    }
    if !create {
        return Ok(None);
    }
    let parent = root.parent().ok_or(FailureCode::InvalidArgument)?;
    if !parent.exists() {
        let grandparent = parent.parent().ok_or(FailureCode::InvalidArgument)?;
        if !grandparent.is_dir() {
            return Err(FailureCode::StorageIo);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(parent) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(FailureCode::StorageIo),
            }
        }
        #[cfg(windows)]
        {
            match fs::create_dir(parent) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(FailureCode::StorageIo),
            }
        }
    }
    match FilesystemSessionStore::provision_default(root) {
        Ok(store) => Ok(Some(store)),
        Err(SessionStoreOpenError::RootAlreadyExists) => {
            FilesystemSessionStore::open_existing(root)
                .map(Some)
                .map_err(map_open_error)
        }
        Err(error) => Err(map_open_error(error)),
    }
}

fn map_open_error(error: SessionStoreOpenError) -> FailureCode {
    match error {
        SessionStoreOpenError::RootMustBeAbsolute
        | SessionStoreOpenError::RootNotDirectory
        | SessionStoreOpenError::RootNotPrivate
        | SessionStoreOpenError::RootAlreadyExists
        | SessionStoreOpenError::InvalidAdmissionCapacity => FailureCode::InvalidArgument,
        SessionStoreOpenError::InvalidOwnership | SessionStoreOpenError::InvalidLayout => {
            FailureCode::IntegrityFailure
        }
        SessionStoreOpenError::RootUnavailable => FailureCode::StorageIo,
    }
}

fn map_storage_error(error: SessionStorageError) -> FailureCode {
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

fn status_data(status: &SessionStatus, now: u64) -> Result<StatusData, FailureCode> {
    let state = match status.phase() {
        SessionPhase::Closed => "closed",
        SessionPhase::Open if status.lifetime().expired(now) => "expired",
        SessionPhase::Open => "open",
    };
    Ok(StatusData {
        session_id: status.session_id().as_str().to_owned(),
        state,
        source_id: status.source_id().as_str().to_owned(),
        source_bytes: status.source_bytes(),
        artifact_count: status.artifact_count(),
        artifact_bytes: status.artifact_bytes(),
        generation: status.generation().value(),
        expires_at: rfc3339(status.lifetime().expires_at_unix_seconds())?,
    })
}

fn response<T: Serialize>(command: &'static str, data: &T) -> Result<Response, FailureCode> {
    OperationResponse::complete(command, data).map_err(|_| FailureCode::Internal)
}

fn partial_response<T: Serialize>(
    command: &'static str,
    data: &T,
    warning: &'static str,
) -> Result<Response, FailureCode> {
    OperationResponse::partial(command, data, warning).map_err(|_| FailureCode::Internal)
}

fn bundle_data(bundle: &BundleStatus) -> BundleData {
    let source_included = bundle.source_policy() == BundleSourcePolicy::IncludeSource;
    BundleData {
        session_id: bundle.session_id().as_str().to_owned(),
        source_id: bundle.source_id().as_str().to_owned(),
        source_bytes: bundle.source_bytes(),
        source_included,
        artifact_count: bundle.artifact_count(),
        artifact_bytes: bundle.artifact_bytes(),
        reextraction_requires_matching_original: !source_included,
        publication: "process_crash_consistent",
    }
}

/// Executes the P05 portion of ingestion without starting P07 transcription.
pub(crate) async fn ingest(
    arguments: IngestArguments,
    explicit_root: Option<&Path>,
) -> Result<Response, FailureCode> {
    if arguments.transcript.is_some() {
        return Err(FailureCode::CommandNotImplemented);
    }
    let source = if arguments.source.is_absolute() {
        arguments.source
    } else {
        env::current_dir()
            .map_err(|_| FailureCode::StorageIo)?
            .join(arguments.source)
    };
    let root = root_path(explicit_root)?;
    let store = open_store(&root, true)?.ok_or(FailureCode::StorageIo)?;
    let now = now_seconds()?;
    let opened = OpenSession::new(store)
        .execute(OpenSessionRequest {
            source,
            session_id: session_id()?,
            initialize_operation_id: operation_id()?,
            stage_operation_id: operation_id()?,
            activate_operation_id: operation_id()?,
            durability: DurabilityRequirement::Ephemeral,
            now_unix_seconds: now,
        })
        .await
        .map_err(|error| match error {
            OpenSessionError::InvalidSource => FailureCode::InvalidSource,
            OpenSessionError::SourceIo => FailureCode::StorageIo,
            OpenSessionError::InvalidClock => FailureCode::InvalidArgument,
            OpenSessionError::Storage(storage) => map_storage_error(storage),
        })?;
    let expires_at = rfc3339(opened.lifetime.expires_at_unix_seconds())?;
    let data = OpenData {
        session_id: opened.session_id.as_str().to_owned(),
        source_id: opened.source_id.as_str().to_owned(),
        source_bytes: opened.source_bytes,
        generation: opened.generation.value(),
        publication: opened.publication.identifier(),
        expires_at: expires_at.clone(),
    };
    Ok(response("ingest", &data)?.with_lifecycle(LifecycleResponse::ephemeral(expires_at)))
}

/// Executes one visible P05 session operation.
#[allow(
    clippy::too_many_lines,
    reason = "Keep typed subcommand presentation in one exhaustive dispatch"
)]
pub(crate) fn execute_session(
    command: SessionCommand,
    explicit_root: Option<&Path>,
) -> Result<Response, FailureCode> {
    let root = root_path(explicit_root)?;
    let now = now_seconds()?;
    let store = open_store(&root, false)?;
    match command {
        SessionCommand::List(arguments) => {
            let Some(store) = store else {
                return response(
                    "session.list",
                    &PageData {
                        items: Vec::new(),
                        next_cursor: None,
                    },
                );
            };
            let mut bucket = arguments.cursor.unwrap_or(0);
            loop {
                let page = store
                    .scan_session_bucket(bucket)
                    .map_err(map_storage_error)?;
                if !page.session_ids().is_empty() || page.next_bucket().is_none() {
                    let mut items = Vec::new();
                    let mut partial = false;
                    for session_id in page.session_ids() {
                        let id = session_id.as_str().to_owned();
                        match store.indexed_session_status(session_id) {
                            Ok(Some(status)) => {
                                let data = status_data(&status, now)?;
                                items.push(ListedSession {
                                    session_id: id,
                                    state: data.state,
                                    status: Some(data),
                                    error_code: None,
                                });
                            }
                            Ok(None) => items.push(ListedSession {
                                session_id: id,
                                state: "initializing",
                                status: None,
                                error_code: None,
                            }),
                            Err(error) => {
                                partial = true;
                                items.push(ListedSession {
                                    session_id: id,
                                    state: "unavailable",
                                    status: None,
                                    error_code: Some(map_storage_error(error).identifier()),
                                });
                            }
                        }
                    }
                    let data = PageData {
                        items,
                        next_cursor: page.next_bucket(),
                    };
                    return if partial {
                        partial_response(
                            "session.list",
                            &data,
                            "Some session records could not be read.",
                        )
                    } else {
                        response("session.list", &data)
                    };
                }
                bucket = page.next_bucket().ok_or(FailureCode::Internal)?;
            }
        }
        SessionCommand::Status(arguments) => {
            let store = store.ok_or(FailureCode::StorageIo)?;
            let status = store
                .session_status(&arguments.session)
                .map_err(map_storage_error)?;
            let data = status_data(&status, now)?;
            Ok(response("session.status", &data)?
                .with_lifecycle(LifecycleResponse::ephemeral(data.expires_at.clone())))
        }
        SessionCommand::Renew(arguments) => {
            let store = store.ok_or(FailureCode::StorageIo)?;
            let current = store
                .session_status(&arguments.session)
                .map_err(map_storage_error)?;
            store
                .renew_session(
                    &arguments.session,
                    &operation_id()?,
                    current.generation(),
                    now,
                )
                .map_err(map_storage_error)?;
            let status = store
                .session_status(&arguments.session)
                .map_err(map_storage_error)?;
            let data = status_data(&status, now)?;
            Ok(response("session.renew", &data)?
                .with_lifecycle(LifecycleResponse::ephemeral(data.expires_at.clone())))
        }
        SessionCommand::Close(arguments) => {
            let store = store.ok_or(FailureCode::StorageIo)?;
            let current = store
                .session_status(&arguments.session)
                .map_err(map_storage_error)?;
            store
                .close_session(&arguments.session, &operation_id()?, current.generation())
                .map_err(map_storage_error)?;
            let status = store
                .session_status(&arguments.session)
                .map_err(map_storage_error)?;
            let data = status_data(&status, now)?;
            Ok(response("session.close", &data)?
                .with_lifecycle(LifecycleResponse::ephemeral(data.expires_at.clone())))
        }
        SessionCommand::Retain(arguments) => {
            let store = store.ok_or(FailureCode::StorageIo)?;
            let policy = if arguments.include_source {
                BundleSourcePolicy::IncludeSource
            } else {
                BundleSourcePolicy::EvidenceOnly
            };
            let output = absolute_selection(&arguments.output)?;
            let bundle = store
                .retain_bundle(&arguments.session, &output, policy)
                .map_err(map_storage_error)?;
            Ok(response("session.retain", &bundle_data(&bundle))?
                .with_lifecycle(LifecycleResponse::retained()))
        }
        SessionCommand::Clean(arguments) => {
            if !arguments.expired {
                return Err(FailureCode::InvalidArgument);
            }
            let Some(store) = store else {
                return response(
                    "session.clean",
                    &CleanData {
                        items: Vec::new(),
                        next_cursor: None,
                        dry_run: arguments.dry_run,
                    },
                );
            };
            let mut bucket = arguments.cursor.unwrap_or(0);
            let page = loop {
                let page = store
                    .scan_session_bucket(bucket)
                    .map_err(map_storage_error)?;
                if !page.session_ids().is_empty() || page.next_bucket().is_none() {
                    break page;
                }
                bucket = page.next_bucket().ok_or(FailureCode::Internal)?;
            };
            let mut items = Vec::new();
            let mut partial = false;
            for session_id in page.session_ids() {
                match store.clean_session(session_id, now, arguments.dry_run) {
                    Ok(outcome) => items.push(CleanItem {
                        session_id: session_id.as_str().to_owned(),
                        outcome: match outcome {
                            CleanOutcome::Ineligible => "ineligible",
                            CleanOutcome::Eligible => "eligible",
                            CleanOutcome::Removed => "removed",
                        },
                        error_code: None,
                    }),
                    Err(error) => {
                        partial = true;
                        items.push(CleanItem {
                            session_id: session_id.as_str().to_owned(),
                            outcome: "skipped",
                            error_code: Some(map_storage_error(error).identifier()),
                        });
                    }
                }
            }
            let data = CleanData {
                items,
                next_cursor: page.next_bucket(),
                dry_run: arguments.dry_run,
            };
            if partial {
                partial_response("session.clean", &data, "Some sessions were not cleaned.")
            } else {
                response("session.clean", &data)
            }
        }
    }
}

/// Validates one user-selected bundle independently of any disposable root.
pub(crate) fn validate_bundle(path: &Path) -> Result<Response, FailureCode> {
    let selected = absolute_selection(path)?;
    let bundle = FilesystemSessionStore::validate_bundle(&selected).map_err(map_storage_error)?;
    Ok(response("bundle.validate", &bundle_data(&bundle))?
        .with_lifecycle(LifecycleResponse::retained()))
}
