//! `vsift handoff check` (P13 PR 5, ADR 0023 decisions G and H).
//!
//! The draft arrives on standard input (the skill's quoted heredoc or
//! single-quoted here-string) or from `--file`. It is read bounded to
//! 64 KiB and must be UTF-8; anything larger, or not text, is the caller's
//! input error. Everything else about the draft is an answer, not a
//! failure: the result is `complete` with `data.valid`, and the process
//! exits 0 (decision H1). The check itself lives in `vsift_contract`, which
//! the trial grader uses too.

use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

use vsift::{Engine, FailureCode, HandoffSessionLookup};
use vsift_contract::{
    CommandName, HANDOFF_DRAFT_NOT_UTF8_REMEDIATION, HANDOFF_DRAFT_TOO_LARGE_REMEDIATION,
    HANDOFF_FILE_REMEDIATION, HandoffCheckData, HandoffChecker, MAX_HANDOFF_REPORT_BYTES,
    OperationResponse,
};

use crate::{CommandFailure, command::HandoffCheckArguments, complete};

/// Checks the draft named by `arguments`, reading `standard_input` when no
/// file is named.
pub(crate) fn check(
    engine: &Engine,
    arguments: &HandoffCheckArguments,
    standard_input: impl Read,
) -> Result<OperationResponse<serde_json::Value>, CommandFailure> {
    let draft = match &arguments.file {
        Some(path) => read_file(path)?,
        None => read_bounded(standard_input).map_err(|_| FailureCode::StorageIo)?,
    };
    let report = decode(draft)?;
    let checker = HandoffChecker::new().map_err(|_| FailureCode::Internal)?;
    let mut check = checker.check_report(&report);
    if let Some(session) = &arguments.session {
        match engine.handoff_session_records(session)? {
            HandoffSessionLookup::Open(records) => {
                check.resolve_in_session(session.as_str(), &records);
            }
            HandoffSessionLookup::Unavailable(gap) => {
                check.session_unavailable(session.as_str(), gap);
            }
        }
    }
    Ok(complete(
        CommandName::HandoffCheck,
        &HandoffCheckData::new(check),
    )?)
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
