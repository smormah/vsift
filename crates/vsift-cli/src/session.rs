//! Presentation of engine session and bundle results through the v1 contract.
//!
//! The engine performs every session operation. This module maps its typed
//! results into `vsift-contract` data, formats RFC 3339 timestamps and decides
//! the command-line response shape.

use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use vsift::{
    BundleSummary, CleanDecision, CleanEntry, CleanMode, CleanPage, CleanRequest, CleanScope,
    Engine, FailureCode, IngestRequest, SessionListEntry, SessionPage, SessionSnapshot,
    SourceRetention, SuppliedTranscriptRequest, TranscriptExcerpt, TranscriptQuery,
};
use vsift_contract::{
    BundleData, BundleSourceInclusion, CleanData, CleanItem, CleanItemOutcome, CommandName,
    LifecycleResponse, ListedSession, OpenData, OperationResponse, PageData, SessionState,
    StatusData, TranscriptEvidenceStream, TranscriptPageData, transcript_warning_messages,
};

use crate::{
    CommandFailure,
    command::{IngestArguments, SessionCommand, TranscriptGetArguments},
};

type Response = OperationResponse<serde_json::Value>;

fn rfc3339(seconds: u64) -> Result<String, FailureCode> {
    let seconds = i64::try_from(seconds).map_err(|_| FailureCode::InvalidArgument)?;
    OffsetDateTime::from_unix_timestamp(seconds)
        .map_err(|_| FailureCode::InvalidArgument)?
        .format(&Rfc3339)
        .map_err(|_| FailureCode::Internal)
}

fn status_data(snapshot: &SessionSnapshot) -> Result<StatusData, FailureCode> {
    Ok(StatusData {
        session_id: snapshot.session_id().as_str().to_owned(),
        state: SessionState::observed(
            snapshot.phase(),
            snapshot.lifetime(),
            snapshot.observed_at_unix_seconds(),
        ),
        source_id: snapshot.source_id().as_str().to_owned(),
        source_bytes: snapshot.source_bytes(),
        artifact_count: snapshot.artifact_count(),
        artifact_bytes: snapshot.artifact_bytes(),
        generation: snapshot.generation().value(),
        expires_at: rfc3339(snapshot.lifetime().expires_at_unix_seconds())?,
    })
}

fn response<T: Serialize>(command: CommandName, data: &T) -> Result<Response, FailureCode> {
    OperationResponse::complete(command.identifier(), data).map_err(|_| FailureCode::Internal)
}

fn partial_response<T: Serialize>(
    command: CommandName,
    data: &T,
    warning: &'static str,
) -> Result<Response, FailureCode> {
    OperationResponse::partial(command.identifier(), data, warning)
        .map_err(|_| FailureCode::Internal)
}

fn status_response(
    command: CommandName,
    snapshot: &SessionSnapshot,
) -> Result<Response, FailureCode> {
    let data = status_data(snapshot)?;
    Ok(response(command, &data)?
        .with_lifecycle(LifecycleResponse::ephemeral(data.expires_at.clone())))
}

fn bundle_data(bundle: &BundleSummary) -> BundleData {
    let source = match bundle.source_retention() {
        SourceRetention::EvidenceOnly => BundleSourceInclusion::EvidenceOnly,
        SourceRetention::IncludeSource => BundleSourceInclusion::SourceIncluded,
    };
    BundleData::new(
        bundle.session_id(),
        bundle.source_id(),
        bundle.source_bytes(),
        source,
        bundle.artifact_count(),
        bundle.artifact_bytes(),
    )
}

fn list_response(page: SessionPage) -> Result<Response, FailureCode> {
    let partial = page.is_partial();
    let next_cursor = page.next_cursor();
    let items =
        page.into_entries()
            .into_iter()
            .map(|entry| match entry {
                SessionListEntry::Indexed(snapshot) => Ok(ListedSession::indexed(
                    snapshot.session_id(),
                    status_data(&snapshot)?,
                )),
                SessionListEntry::Initializing(session_id) => {
                    Ok(ListedSession::initializing(&session_id))
                }
                SessionListEntry::Unavailable { session_id, error } => Ok(
                    ListedSession::unavailable(&session_id, error.failure_code()),
                ),
            })
            .collect::<Result<Vec<_>, FailureCode>>()?;
    let data = PageData { items, next_cursor };
    if partial {
        partial_response(
            CommandName::SessionList,
            &data,
            "Some session records could not be read.",
        )
    } else {
        response(CommandName::SessionList, &data)
    }
}

fn clean_response(page: CleanPage) -> Result<Response, FailureCode> {
    let partial = page.is_partial();
    let next_cursor = page.next_cursor();
    let dry_run = page.mode() == CleanMode::DryRun;
    let items = page
        .into_entries()
        .into_iter()
        .map(|entry| match entry {
            CleanEntry::Examined {
                session_id,
                decision,
            } => CleanItem::examined(
                &session_id,
                match decision {
                    CleanDecision::Ineligible => CleanItemOutcome::Ineligible,
                    CleanDecision::Eligible => CleanItemOutcome::Eligible,
                    CleanDecision::Removed => CleanItemOutcome::Removed,
                },
            ),
            CleanEntry::Skipped { session_id, error } => {
                CleanItem::skipped(&session_id, error.failure_code())
            }
        })
        .collect();
    let data = CleanData {
        items,
        next_cursor,
        dry_run,
    };
    if partial {
        partial_response(
            CommandName::SessionClean,
            &data,
            "Some sessions were not cleaned.",
        )
    } else {
        response(CommandName::SessionClean, &data)
    }
}

/// Opens a disposable session, importing a supplied transcript when one is given.
pub(crate) async fn ingest(
    engine: &Engine,
    arguments: IngestArguments,
) -> Result<Response, CommandFailure> {
    let transcript = arguments.transcript.map(|path| SuppliedTranscriptRequest {
        path,
        offset_micros: arguments.transcript_offset.unwrap_or(0),
    });
    let outcome = engine
        .ingest(IngestRequest {
            source: arguments.source,
            transcript,
        })
        .await?;
    let expires_at = rfc3339(outcome.session.lifetime.expires_at_unix_seconds())?;
    let mut data = OpenData::new(&outcome.session, expires_at.clone());
    let mut warnings = Vec::new();
    if let Some(revision) = &outcome.transcript {
        data = data.with_transcript(revision);
        warnings = transcript_warning_messages(revision);
    }
    Ok(response(CommandName::Ingest, &data)?
        .with_lifecycle(LifecycleResponse::ephemeral(expires_at))
        .with_warnings(&warnings))
}

fn read_transcript(
    engine: &Engine,
    arguments: TranscriptGetArguments,
) -> Result<TranscriptExcerpt, CommandFailure> {
    Ok(engine.transcript(TranscriptQuery {
        session: arguments.session,
        from_micros: arguments.from,
        to_micros: arguments.to,
        limit: arguments.limit,
        cursor: arguments.cursor,
    })?)
}

/// Reads one bounded page of a session's transcript as one result.
pub(crate) fn transcript_get(
    engine: &Engine,
    arguments: TranscriptGetArguments,
) -> Result<Response, CommandFailure> {
    let excerpt = read_transcript(engine, arguments)?;
    let data = TranscriptPageData::new(
        excerpt.session().session_id(),
        excerpt.revision(),
        excerpt.range(),
        excerpt.segments(),
        excerpt.next_cursor(),
    );
    let expires_at = rfc3339(excerpt.session().lifetime().expires_at_unix_seconds())?;
    Ok(response(CommandName::TranscriptGet, &data)?
        .with_lifecycle(LifecycleResponse::ephemeral(expires_at)))
}

/// Reads one bounded page of a session's transcript as an evidence stream:
/// one evidence event per segment, then the terminal event.
pub(crate) fn transcript_stream(
    engine: &Engine,
    arguments: TranscriptGetArguments,
) -> Result<TranscriptEvidenceStream, CommandFailure> {
    let excerpt = read_transcript(engine, arguments)?;
    let expires_at = rfc3339(excerpt.session().lifetime().expires_at_unix_seconds())?;
    TranscriptEvidenceStream::new(
        excerpt.session().session_id(),
        excerpt.revision(),
        excerpt.range(),
        excerpt.segments(),
        excerpt.next_cursor(),
        LifecycleResponse::ephemeral(expires_at),
    )
    .map_err(|_| CommandFailure::from(FailureCode::Internal))
}

/// Executes one visible P05 session operation.
pub(crate) fn execute_session(
    engine: &Engine,
    command: SessionCommand,
) -> Result<Response, FailureCode> {
    match command {
        SessionCommand::List(arguments) => list_response(engine.list_sessions(arguments.cursor)?),
        SessionCommand::Status(arguments) => status_response(
            CommandName::SessionStatus,
            &engine.session_status(&arguments.session)?,
        ),
        SessionCommand::Renew(arguments) => status_response(
            CommandName::SessionRenew,
            &engine.renew_session(&arguments.session)?,
        ),
        SessionCommand::Close(arguments) => status_response(
            CommandName::SessionClose,
            &engine.close_session(&arguments.session)?,
        ),
        SessionCommand::Retain(arguments) => {
            let retention = if arguments.include_source {
                SourceRetention::IncludeSource
            } else {
                SourceRetention::EvidenceOnly
            };
            let bundle = engine.retain_session(&arguments.session, &arguments.output, retention)?;
            Ok(response(CommandName::SessionRetain, &bundle_data(&bundle))?
                .with_lifecycle(LifecycleResponse::retained()))
        }
        SessionCommand::Clean(arguments) => {
            let page = engine.clean_sessions(CleanRequest {
                scope: if arguments.expired {
                    CleanScope::Expired
                } else {
                    CleanScope::Unrestricted
                },
                mode: if arguments.dry_run {
                    CleanMode::DryRun
                } else {
                    CleanMode::Remove
                },
                cursor: arguments.cursor,
            })?;
            clean_response(page)
        }
    }
}

/// Validates one user-selected bundle independently of any disposable root.
pub(crate) fn validate_bundle(
    engine: &Engine,
    directory: &std::path::Path,
) -> Result<Response, FailureCode> {
    let bundle = engine.validate_bundle(directory)?;
    Ok(
        response(CommandName::BundleValidate, &bundle_data(&bundle))?
            .with_lifecycle(LifecycleResponse::retained()),
    )
}
