//! `vsift handoff check` (P13 PR 5, ADR 0023 decisions G and H),
//! `vsift handoff skeleton` (ADR 0038) and `vsift handoff finish` (0.3.0,
//! M5b; ADR 0023 note of 2026-11-09).
//!
//! The draft arrives on standard input (the skill's quoted heredoc or
//! single-quoted here-string) or from `--file`. It is read bounded to
//! 128 KiB (64 KiB until `0.2.2`, #366) and must be UTF-8; anything larger, or not text, is the caller's
//! input error. Everything else about the draft is an answer, not a
//! failure: the result is `complete` with `data.valid`, and the process
//! exits 0 (decision H1). The check itself lives in `vsift_contract`, which
//! the trial grader uses too.

use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

use vsift::{Engine, FailureCode, HandoffSessionLookup, SessionId, SourceRetention};
use vsift_contract::{
    CommandName, HANDOFF_DRAFT_NOT_UTF8_REMEDIATION, HANDOFF_DRAFT_TOO_LARGE_REMEDIATION,
    HANDOFF_FILE_REMEDIATION, HANDOFF_SKELETON_QUESTION_REMEDIATION, HandoffCheck,
    HandoffCheckData, HandoffChecker, HandoffFinishData, HandoffFinishDecision,
    HandoffFinishRequest, HandoffSkeletonError, LifecycleResponse, MAX_HANDOFF_REPORT_BYTES,
    OperationResponse,
};

use crate::{
    CommandFailure,
    command::{HandoffCheckArguments, HandoffFinishArguments, HandoffSkeletonArguments},
    complete,
    session::{bundle_data, session_lifecycle, status_data},
};

/// Checks the draft named by `arguments`, reading `standard_input` when no
/// file is named.
pub(crate) fn check(
    engine: &Engine,
    arguments: &HandoffCheckArguments,
    standard_input: impl Read,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let report = read_draft(arguments.file.as_deref(), standard_input)?;
    let checker = HandoffChecker::new().map_err(|_| FailureCode::Internal)?;
    let mut check = checker.check_report(&report);
    if let Some(session) = &arguments.session {
        resolve(engine, session, &mut check)?;
    }
    Ok(complete(
        CommandName::HandoffCheck,
        &HandoffCheckData::new(check),
    )?)
}

/// Finishes the draft `arguments` name (0.3.0, M5b): the check of
/// [`check`] with the session's records always resolved, then, only when
/// [`HandoffFinishRequest::decide`] allows it, the one write asked for,
/// through the same engine call as `session retain` or `session close`.
///
/// A stop before the write is an answer (`complete`, `finished` false). A
/// write that fails answers with that write's own failure, whatever the
/// check found, exactly as the standalone command would: a session that is
/// closed, expired or missing included (the planner's ruling of
/// 2026-11-09). Without a write, a session that cannot be read is a gap in
/// `data.check.session`, as for `handoff check --session`.
pub(crate) fn finish(
    engine: &Engine,
    arguments: &HandoffFinishArguments,
    standard_input: impl Read,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let write = FinishWrite::of(arguments)?;
    let report = read_draft(arguments.file.as_deref(), standard_input)?;
    let checker = HandoffChecker::new().map_err(|_| FailureCode::Internal)?;
    let mut check = checker.check_report(&report);
    resolve(engine, &arguments.session, &mut check)?;
    let request = write.request();
    if let HandoffFinishDecision::Stop(stop) = request.decide(&check) {
        return Ok(complete(
            CommandName::HandoffFinish,
            &HandoffFinishData::stopped(check, request, stop),
        )?);
    }
    match write {
        FinishWrite::Nothing => Ok(complete(
            CommandName::HandoffFinish,
            &HandoffFinishData::checked(check),
        )?),
        FinishWrite::Retain { output, retention } => {
            let bundle = engine.retain_session(&arguments.session, output, retention)?;
            let data = HandoffFinishData::retained(check, bundle_data(&bundle));
            Ok(complete(CommandName::HandoffFinish, &data)?
                .with_lifecycle(LifecycleResponse::retained()))
        }
        FinishWrite::Close => {
            let snapshot = engine.close_session(&arguments.session)?;
            let data = HandoffFinishData::closed(check, status_data(&snapshot)?);
            Ok(complete(CommandName::HandoffFinish, &data)?
                .with_lifecycle(session_lifecycle(snapshot.lifetime())?))
        }
    }
}

/// The one write a `handoff finish` command line asks for.
enum FinishWrite<'a> {
    /// Neither `--retain` nor `--close`.
    Nothing,
    /// `--retain <new-directory>`, with `--include-source` or without.
    Retain {
        output: &'a Path,
        retention: SourceRetention,
    },
    /// `--close`.
    Close,
}

impl<'a> FinishWrite<'a> {
    /// The write of `arguments`. Clap already refuses `--retain` with
    /// `--close` and `--include-source` without `--retain`; they are refused
    /// here too, so the rule does not rest on the parser's attributes alone.
    fn of(arguments: &'a HandoffFinishArguments) -> Result<Self, CommandFailure> {
        match (&arguments.retain, arguments.close, arguments.include_source) {
            (Some(_), true, _) | (None, _, true) => {
                Err(CommandFailure::from(FailureCode::InvalidArgument))
            }
            (Some(output), false, include_source) => Ok(Self::Retain {
                output: output.as_path(),
                retention: if include_source {
                    SourceRetention::IncludeSource
                } else {
                    SourceRetention::EvidenceOnly
                },
            }),
            (None, true, false) => Ok(Self::Close),
            (None, false, false) => Ok(Self::Nothing),
        }
    }

    const fn request(&self) -> HandoffFinishRequest {
        match self {
            Self::Nothing => HandoffFinishRequest::Check,
            Self::Retain { .. } => HandoffFinishRequest::Retain,
            Self::Close => HandoffFinishRequest::Close,
        }
    }
}

/// The draft, from `file` or else `standard_input`, bounded and decoded.
fn read_draft(file: Option<&Path>, standard_input: impl Read) -> Result<String, CommandFailure> {
    let draft = match file {
        Some(path) => read_file(path)?,
        None => read_bounded(standard_input).map_err(|_| FailureCode::StorageIo)?,
    };
    decode(draft)
}

/// Resolves `check`'s cited identities in `session`'s records, read only;
/// a session that cannot be read is recorded as its gap, not a failure.
fn resolve(
    engine: &Engine,
    session: &SessionId,
    check: &mut HandoffCheck,
) -> Result<(), CommandFailure> {
    match engine.handoff_session_records(session)? {
        HandoffSessionLookup::Open(records) => {
            check.resolve_in_session(session.as_str(), &records);
        }
        HandoffSessionLookup::Unavailable(gap) => {
            check.session_unavailable(session.as_str(), gap);
        }
    }
    Ok(())
}

/// Writes the skeleton handoff for the session and question `arguments`
/// name: `VSift` fills in what its records hold, and the agent adds the rest.
pub(crate) fn skeleton(
    engine: &Engine,
    arguments: &HandoffSkeletonArguments,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let facts = engine.handoff_skeleton_facts(&arguments.session)?;
    let checker = HandoffChecker::new().map_err(|_| FailureCode::Internal)?;
    let data = checker
        .skeleton(&arguments.question, &facts)
        .map_err(|error| match error {
            HandoffSkeletonError::QuestionRefused => CommandFailure::with_remediation(
                FailureCode::InvalidArgument,
                HANDOFF_SKELETON_QUESTION_REMEDIATION.to_owned(),
            ),
            HandoffSkeletonError::NotValid => CommandFailure::from(FailureCode::Internal),
        })?;
    Ok(complete(CommandName::HandoffSkeleton, &data)?)
}

/// The draft's bytes, at most one more than the limit so an oversized draft
/// is recognised without reading all of it.
fn read_bounded(source: impl Read) -> io::Result<Vec<u8>> {
    let budget = u64::try_from(MAX_HANDOFF_REPORT_BYTES).map_or(u64::MAX, |bytes| bytes + 1);
    let mut bytes = Vec::with_capacity(4 * 1024);
    source.take(budget).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_file(path: &Path) -> Result<Vec<u8>, CommandFailure> {
    let refused = || {
        CommandFailure::with_remediation(
            FailureCode::InvalidArgument,
            HANDOFF_FILE_REMEDIATION.to_owned(),
        )
    };
    if !path.is_absolute() {
        return Err(refused());
    }
    let file = File::open(path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::IsADirectory | io::ErrorKind::PermissionDenied => {
            refused()
        }
        _ => CommandFailure::from(FailureCode::StorageIo),
    })?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(refused());
    }
    read_bounded(file).map_err(|_| CommandFailure::from(FailureCode::StorageIo))
}

/// The draft as text: at most [`MAX_HANDOFF_REPORT_BYTES`] of strict UTF-8.
fn decode(bytes: Vec<u8>) -> Result<String, CommandFailure> {
    if bytes.len() > MAX_HANDOFF_REPORT_BYTES {
        return Err(CommandFailure::with_remediation(
            FailureCode::InvalidArgument,
            HANDOFF_DRAFT_TOO_LARGE_REMEDIATION.to_owned(),
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        CommandFailure::with_remediation(
            FailureCode::InvalidArgument,
            HANDOFF_DRAFT_NOT_UTF8_REMEDIATION.to_owned(),
        )
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use vsift_contract::MAX_HANDOFF_REPORT_BYTES;

    use super::{decode, read_bounded};

    #[test]
    fn a_draft_is_bounded_utf8() -> Result<(), Box<dyn std::error::Error>> {
        let at_limit = vec![b'a'; MAX_HANDOFF_REPORT_BYTES];
        assert!(decode(read_bounded(Cursor::new(at_limit))?).is_ok());
        let over = vec![b'a'; MAX_HANDOFF_REPORT_BYTES * 4];
        let read = read_bounded(Cursor::new(over))?;
        assert_eq!(read.len(), MAX_HANDOFF_REPORT_BYTES + 1);
        assert!(decode(read).is_err());
        assert!(decode(vec![b'a', 0xff, b'b']).is_err());
        Ok(())
    }
}
