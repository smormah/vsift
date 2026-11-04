//! Presentation of `changes` results through the v1 contract (ADR 0034).
//!
//! The engine analyses, pages, takes the frames and compares them; this
//! module maps the parsed command to its engine request, the engine's typed
//! page to the `vsift-contract` result or evidence stream, and an engine
//! failure to its code with the remediation a caller of `changes` needs.

use vsift::{
    Cancellation, CandidatesRange, Engine, EngineError, EvidenceMediaError, FailureCode,
    ScreenChangesRequest, ScreenChangesResults, SessionStorageError, SourceProbeError,
    VisualExtensionStop, VisualIndexBuildError, VisualSamplingError,
};
use vsift_contract::{
    CHANGES_CURSOR_REMEDIATION, CHANGES_CURSOR_WITHOUT_INDEX_REMEDIATION, DeliveredEvidenceFile,
    EVIDENCE_BUDGET_REMEDIATION, EVIDENCE_PATH_REMEDIATION, EVIDENCE_TOOLS_REMEDIATION,
    EvidencePresentationError, LifecycleResponse, ListedScreenChange, MEDIA_BUSY_REMEDIATION,
    OperationResponse, ScreenChangeResumePresentation, ScreenChangesEvidenceStream,
    ScreenChangesPresentation, UNDECODABLE_EVIDENCE_REMEDIATION, screen_changes_response,
};

use crate::{CommandFailure, command::ChangesArguments, session::session_lifecycle};

/// Whether the failure is another request holding what `changes` needs
/// (#342): the causes `candidates` meets while analysing and those the
/// evidence commands meet while taking a frame. Nothing is committed in any
/// of them, so the caller waits and runs the same command again.
const fn is_contention(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::VisualAnalysisStopped(VisualExtensionStop::Busy { .. })
            | EngineError::VisualIndexBuild(VisualIndexBuildError::Sampling(
                VisualSamplingError::Busy
            ))
            | EngineError::EvidenceMedia(EvidenceMediaError::Busy)
            | EngineError::SourceProbe(SourceProbeError::Busy)
            | EngineError::Storage(SessionStorageError::Busy)
    )
}

/// Maps a `changes` failure to its code and remediation.
///
/// The causes that only `changes` meets, or meets differently, are matched
/// first: a rejected cursor and a cursor for a session without an index
/// name `changes` rather than `candidates`; a session with no room for even
/// the first change's frames, and a media tool missing when a frame is
/// taken, get `frame get`'s answers. Every other
/// failure keeps the engine's generic mapping, which already answers a
/// missing media tool, a source without video and a running transcription.
fn changes_failure(error: EngineError) -> CommandFailure {
    if is_contention(&error) {
        return CommandFailure::contention(error.failure_code(), MEDIA_BUSY_REMEDIATION.to_owned());
    }
    let remediation = match &error {
        EngineError::ScreenChangeQuery(_) => Some(CHANGES_CURSOR_REMEDIATION),
        EngineError::CandidateCursorWithoutIndex => Some(CHANGES_CURSOR_WITHOUT_INDEX_REMEDIATION),
        EngineError::EvidenceBudgetExhausted => Some(EVIDENCE_BUDGET_REMEDIATION),
        EngineError::MediaToolUnavailable(_) => Some(EVIDENCE_TOOLS_REMEDIATION),
        EngineError::EvidenceMedia(
            EvidenceMediaError::Undecodable | EvidenceMediaError::NoDecodedAudio,
        ) => Some(UNDECODABLE_EVIDENCE_REMEDIATION),
        _ => None,
    };
    match remediation {
        Some(summary) => CommandFailure::with_remediation(error.failure_code(), summary.to_owned()),
        None => CommandFailure::from(error),
    }
}

/// A presentation failure: a path that JSON cannot name is the output's
/// fault, anything else an internal one.
fn presentation_failure(error: &EvidencePresentationError) -> CommandFailure {
    match error {
        EvidencePresentationError::NonUtf8Path => CommandFailure::with_remediation(
            FailureCode::StorageIo,
            EVIDENCE_PATH_REMEDIATION.to_owned(),
        ),
        EvidencePresentationError::OperationMismatch
        | EvidencePresentationError::FileMismatch
        | EvidencePresentationError::Serialization(_) => {
            CommandFailure::from(FailureCode::Internal)
        }
    }
}

async fn run(
    engine: &Engine,
    arguments: ChangesArguments,
    cancellation: &Cancellation,
) -> Result<ScreenChangesResults, CommandFailure> {
    engine
        .screen_changes(ScreenChangesRequest {
            session: arguments.session,
            range: CandidatesRange {
                from_micros: arguments.from,
                to_micros: arguments.to,
            },
            limit: arguments.limit,
            cursor: arguments.cursor,
            cancellation: cancellation.clone(),
        })
        .await
        .map_err(changes_failure)
}

fn lifecycle(results: &ScreenChangesResults) -> Result<LifecycleResponse, CommandFailure> {
    Ok(session_lifecycle(results.session().lifetime())?)
}

/// The page's changes and files in the contract's borrowed form.
fn listed(results: &ScreenChangesResults) -> Vec<ListedScreenChange<'_>> {
    results
        .changes()
        .iter()
        .map(|item| ListedScreenChange {
            id: &item.id,
            compared: &item.compared,
        })
        .collect()
}

fn delivered(results: &ScreenChangesResults) -> Vec<DeliveredEvidenceFile<'_>> {
    results
        .files()
        .iter()
        .map(|file| DeliveredEvidenceFile {
            evidence_id: file.evidence_id(),
            kind: file.kind(),
            path: file.path(),
        })
        .collect()
}

fn presentation<'a>(
    results: &'a ScreenChangesResults,
    changes: &'a [ListedScreenChange<'a>],
    files: &'a [DeliveredEvidenceFile<'a>],
) -> ScreenChangesPresentation<'a> {
    ScreenChangesPresentation {
        session_id: results.session().session_id(),
        index: results.index(),
        range: results.range(),
        searched: results.searched(),
        changes,
        frames: results.frames(),
        files,
        next_cursor: results.next_cursor(),
        resume: results
            .resume()
            .map(|resume| ScreenChangeResumePresentation {
                from: resume.from,
                reason: resume.reason.identifier(),
            }),
        analysed: results.analysed(),
        gaps: results.gaps(),
        stop: results.stop(),
        time_anchors: results.session().time_anchors(),
    }
}

/// Pages a session's screen changes, analysing missing windows first, and
/// presents one page as one result.
pub(crate) async fn changes(
    engine: &Engine,
    arguments: ChangesArguments,
    cancellation: &Cancellation,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let results = run(engine, arguments, cancellation).await?;
    let changes = listed(&results);
    let files = delivered(&results);
    screen_changes_response(
        &presentation(&results, &changes, &files),
        lifecycle(&results)?,
    )
    .map_err(|error| presentation_failure(&error))
}

/// Pages a session's screen changes and presents one page as an evidence
/// stream: the frames, then the changes, then the terminal event.
pub(crate) async fn changes_stream(
    engine: &Engine,
    arguments: ChangesArguments,
    cancellation: &Cancellation,
) -> Result<ScreenChangesEvidenceStream, CommandFailure> {
    let results = run(engine, arguments, cancellation).await?;
    let changes = listed(&results);
    let files = delivered(&results);
    ScreenChangesEvidenceStream::new(
        &presentation(&results, &changes, &files),
        lifecycle(&results)?,
    )
    .map_err(|error| presentation_failure(&error))
}

#[cfg(test)]
mod tests {
    use vsift::{
        CursorError, EngineError, EvidenceMediaError, FailureCode, ScreenChangeQueryError,
        SessionStorageError, VisualExtensionStop,
    };
    use vsift_contract::{
        CANDIDATE_CURSOR_REMEDIATION, CHANGES_CURSOR_REMEDIATION,
        CHANGES_CURSOR_WITHOUT_INDEX_REMEDIATION, EVIDENCE_BUDGET_REMEDIATION,
        MEDIA_BUSY_REMEDIATION,
    };

    use super::changes_failure;

    /// A rejected cursor and a cursor for a session without an index are
    /// `INVALID_ARGUMENT` with remediations that name `changes`, not
    /// `candidates`; `candidates` keeps its own words.
    #[test]
    fn cursor_failures_name_changes() {
        let rejected = changes_failure(EngineError::ScreenChangeQuery(
            ScreenChangeQueryError::Cursor(CursorError::Malformed),
        ));
        assert_eq!(rejected.code, FailureCode::InvalidArgument);
        assert_eq!(rejected.summary(), Some(CHANGES_CURSOR_REMEDIATION));
        let without = changes_failure(EngineError::CandidateCursorWithoutIndex);
        assert_eq!(without.code, FailureCode::InvalidArgument);
        assert_eq!(
            without.summary(),
            Some(CHANGES_CURSOR_WITHOUT_INDEX_REMEDIATION)
        );
        assert_ne!(
            CHANGES_CURSOR_WITHOUT_INDEX_REMEDIATION,
            CANDIDATE_CURSOR_REMEDIATION
        );
        for summary in [
            CHANGES_CURSOR_REMEDIATION,
            CHANGES_CURSOR_WITHOUT_INDEX_REMEDIATION,
        ] {
            assert!(summary.contains("changes without --cursor"));
            assert!(summary.len() <= 1024);
        }
    }

    /// No room for even the first change's frames is `RESOURCE_LIMIT` with
    /// `frame get`'s remediation; contention is `BUSY` with the retry hint;
    /// a deadline keeps its code.
    #[test]
    fn room_contention_and_deadlines_keep_their_codes() {
        let full = changes_failure(EngineError::EvidenceBudgetExhausted);
        assert_eq!(full.code, FailureCode::ResourceLimit);
        assert_eq!(full.summary(), Some(EVIDENCE_BUDGET_REMEDIATION));
        for error in [
            EngineError::Storage(SessionStorageError::Busy),
            EngineError::EvidenceMedia(EvidenceMediaError::Busy),
            EngineError::VisualAnalysisStopped(VisualExtensionStop::Busy { ordinal: 0 }),
        ] {
            let busy = changes_failure(error.clone());
            assert_eq!(busy.code, FailureCode::Busy, "{error}");
            assert_eq!(busy.summary(), Some(MEDIA_BUSY_REMEDIATION), "{error}");
            assert_eq!(busy.retry_after_ms, Some(2_000), "{error}");
        }
        let late = changes_failure(EngineError::EvidenceMedia(EvidenceMediaError::Deadline));
        assert_eq!(late.code, FailureCode::DeadlineExceeded);
        assert_eq!(late.retry_after_ms, None);
    }
}
