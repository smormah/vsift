//! The line protocol between the workload, the host harness and the verifier.
//!
//! The workload writes one line per event, unbuffered, to its
//! acknowledgement channel (a file, or a guest's serial port drained before
//! the next operation starts):
//!
//! ```text
//! START <seq> <unix_ns> <kind>
//! ACK <seq> <unix_ns> <kind> <session> <generation> <manifest_sha256> <artifacts> <revision> [<request>]
//! FAIL <seq> <unix_ns> <kind> <FAILURE_CODE>
//! ```
//!
//! `<artifacts>` is a comma-separated list of the SHA-256 digests the
//! operation committed (the source copy, a frame and its evidence record),
//! or `-`; `<revision>` is the transcript revision a retranscription
//! committed, or `-`; `<request>`, only on a worker request's
//! acknowledgement (P11 PR 3), is `<operation_id>:<result_sha256>`, the
//! request's key and the digest of the result its record holds. An `ACK`
//! is written only after the operation returned
//! success, so it is the acknowledgement the campaign holds the store to.
//! Every other line (kernel messages, the verifier's report) is ignored.

use std::fmt;

use vsift_domain::{OperationId, SessionId, TranscriptRevisionId};

/// One kind of operation the workload runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum OperationKind {
    /// A durable session opened through the engine (`Engine::ingest`).
    Ingest,
    /// A frame and its evidence record committed in one generation.
    Evidence,
    /// A checkpointed retranscription job committing a revision.
    Retranscribe,
    /// A lifecycle generation that extends the session.
    Renew,
    /// A worker request (P11 PR 3): an ingest through
    /// `Engine::run_work_request`, acknowledged once its result is recorded.
    Request,
}

impl OperationKind {
    /// Every kind, in a fixed order.
    pub const ALL: [Self; 5] = [
        Self::Ingest,
        Self::Evidence,
        Self::Retranscribe,
        Self::Renew,
        Self::Request,
    ];

    /// The kind's name on the wire.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ingest => "ingest",
            Self::Evidence => "evidence",
            Self::Retranscribe => "retranscribe",
            Self::Renew => "renew",
            Self::Request => "request",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == text)
    }
}

impl fmt::Display for OperationKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// An acknowledged operation: what the store must still hold after any
/// crash that happened after the acknowledgement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ack {
    /// The workload's sequence number, unique across a campaign.
    pub seq: u64,
    /// When the acknowledgement was written, in Unix nanoseconds.
    pub unix_ns: u128,
    /// The operation.
    pub kind: OperationKind,
    /// The session it committed to.
    pub session: SessionId,
    /// The generation it committed.
    pub generation: u64,
    /// SHA-256 of that generation's manifest.
    pub manifest_sha256: String,
    /// SHA-256 digests of the artifacts it committed.
    pub artifacts: Vec<String>,
    /// The transcript revision it committed, for a retranscription.
    pub revision: Option<TranscriptRevisionId>,
    /// The worker request's key and recorded result, for a request.
    pub request: Option<RequestAck>,
}

/// What a worker request's acknowledgement holds the store to: its record
/// under `operation_id` holds the result whose SHA-256 is `result_sha256`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestAck {
    /// The request's operation id.
    pub operation_id: OperationId,
    /// SHA-256 of its recorded result.
    pub result_sha256: String,
}

impl Ack {
    /// The acknowledgement as one protocol line, without the line feed.
    #[must_use]
    pub fn line(&self) -> String {
        let artifacts = if self.artifacts.is_empty() {
            "-".to_owned()
        } else {
            self.artifacts.join(",")
        };
        let revision = self
            .revision
            .as_ref()
            .map_or("-", TranscriptRevisionId::as_str);
        let request = self.request.as_ref().map_or_else(String::new, |request| {
            format!(
                " {}:{}",
                request.operation_id.as_str(),
                request.result_sha256
            )
        });
        format!(
            "ACK {} {} {} {} {} {} {artifacts} {revision}{request}",
            self.seq,
            self.unix_ns,
            self.kind,
            self.session.as_str(),
            self.generation,
            self.manifest_sha256
        )
    }

    /// The dm-log-writes mark the workload records after this
    /// acknowledgement.
    #[must_use]
    pub fn mark(&self) -> String {
        mark_for(self.seq)
    }
}

/// The dm-log-writes mark of acknowledgement `seq`.
#[must_use]
pub fn mark_for(seq: u64) -> String {
    format!("ack-{seq}")
}

/// The sequence number a mark names, if it is an acknowledgement mark.
#[must_use]
pub fn seq_of_mark(mark: &str) -> Option<u64> {
    mark.strip_prefix("ack-")?.parse().ok()
}

/// The dm-log-writes mark the managed workload logs just before command
/// `seq` touches the store (P13 PR 7 addendum, 2026-10-01).
///
/// A replay point after this mark may hold some of that command's writes
/// although its acknowledgement mark comes later: the replay needs the start
/// to tell a command in flight at the point from one that had not begun.
#[must_use]
pub fn start_mark_for(seq: u64) -> String {
    format!("start-{seq}")
}

/// The sequence number a mark names, if it is a command's start mark.
#[must_use]
pub fn seq_of_start_mark(mark: &str) -> Option<u64> {
    mark.strip_prefix("start-")?.parse().ok()
}

/// A failed operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Failure {
    /// The workload's sequence number.
    pub seq: u64,
    /// When it failed, in Unix nanoseconds.
    pub unix_ns: u128,
    /// The operation.
    pub kind: OperationKind,
    /// The public failure code the engine's mapping gives it.
    pub code: String,
}

/// An operation that started.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Start {
    /// The workload's sequence number.
    pub seq: u64,
    /// When it started, in Unix nanoseconds.
    pub unix_ns: u128,
    /// The operation.
    pub kind: OperationKind,
}

/// Every protocol event of one log, in order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Events {
    /// Started operations.
    pub starts: Vec<Start>,
    /// Acknowledged operations.
    pub acks: Vec<Ack>,
    /// Failed operations.
    pub failures: Vec<Failure>,
    /// Lines that start like a protocol line but do not parse (1-based).
    pub malformed: Vec<usize>,
}

/// Parses every protocol line of `text`; other lines are ignored.
///
/// A line that begins with a protocol keyword but does not parse is
/// recorded as malformed rather than skipped, so a damaged acknowledgement
/// log is noticed.
#[must_use]
pub fn parse_events(text: &str) -> Events {
    let mut events = Events::default();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        let mut fields = line.split(' ');
        match fields.next() {
            Some("ACK") => match parse_ack(&mut fields) {
                Some(ack) => events.acks.push(ack),
                None => events.malformed.push(index + 1),
            },
            Some("FAIL") => match parse_failure(&mut fields) {
                Some(failure) => events.failures.push(failure),
                None => events.malformed.push(index + 1),
            },
            Some("START") => match parse_start(&mut fields) {
                Some(start) => events.starts.push(start),
                None => events.malformed.push(index + 1),
            },
            _ => {}
        }
    }
    events
}

fn parse_ack<'a>(fields: &mut impl Iterator<Item = &'a str>) -> Option<Ack> {
    let seq = fields.next()?.parse().ok()?;
    let unix_ns = fields.next()?.parse().ok()?;
    let kind = OperationKind::parse(fields.next()?)?;
    let session = SessionId::parse(fields.next()?).ok()?;
    let generation = fields.next()?.parse().ok()?;
    let manifest_sha256 = hex_digest(fields.next()?)?;
    let artifacts = match fields.next()? {
        "-" => Vec::new(),
        list => list
            .split(',')
            .map(hex_digest)
            .collect::<Option<Vec<_>>>()?,
    };
    let revision = match fields.next()? {
        "-" => None,
        text => Some(TranscriptRevisionId::parse(text).ok()?),
    };
    let request = match fields.next() {
        Some(text) => {
            let (operation, digest) = text.split_once(':')?;
            Some(RequestAck {
                operation_id: OperationId::parse(operation).ok()?,
                result_sha256: hex_digest(digest)?,
            })
        }
        None => None,
    };
    // Exactly a request's acknowledgement names a request.
    if fields.next().is_some() || request.is_some() != (kind == OperationKind::Request) {
        return None;
    }
    Some(Ack {
        seq,
        unix_ns,
        kind,
        session,
        generation,
        manifest_sha256,
        artifacts,
        revision,
        request,
    })
}

fn parse_failure<'a>(fields: &mut impl Iterator<Item = &'a str>) -> Option<Failure> {
    let seq = fields.next()?.parse().ok()?;
    let unix_ns = fields.next()?.parse().ok()?;
    let kind = OperationKind::parse(fields.next()?)?;
    let code = fields.next()?;
    if code.is_empty()
        || !code
            .chars()
            .all(|character| character.is_ascii_uppercase() || character == '_')
        || fields.next().is_some()
    {
        return None;
    }
    Some(Failure {
        seq,
        unix_ns,
        kind,
        code: code.to_owned(),
    })
}

fn parse_start<'a>(fields: &mut impl Iterator<Item = &'a str>) -> Option<Start> {
    let seq = fields.next()?.parse().ok()?;
    let unix_ns = fields.next()?.parse().ok()?;
    let kind = OperationKind::parse(fields.next()?)?;
    if fields.next().is_some() {
        return None;
    }
    Some(Start { seq, unix_ns, kind })
}

/// A lowercase 64-digit SHA-256 in hex.
fn hex_digest(text: &str) -> Option<String> {
    (text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        Ack, OperationKind, mark_for, parse_events, seq_of_mark, seq_of_start_mark, start_mark_for,
    };
    use vsift_domain::SessionId;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn ack() -> Result<Ack, Box<dyn std::error::Error>> {
        Ok(Ack {
            seq: 7,
            unix_ns: 1_700_000_000_000_000_000,
            kind: OperationKind::Evidence,
            session: SessionId::parse("ses_0123456789abcdef0123456789abcdef")?,
            generation: 3,
            manifest_sha256: DIGEST.to_owned(),
            artifacts: vec![DIGEST.to_owned(), DIGEST.replace('0', "f")],
            revision: None,
            request: None,
        })
    }

    /// P11 PR 3: a request's acknowledgement names its key and result, and
    /// only a request's may.
    #[test]
    fn a_request_acknowledgement_names_its_record() -> Result<(), Box<dyn std::error::Error>> {
        let mut ack = ack()?;
        ack.kind = OperationKind::Request;
        ack.request = Some(super::RequestAck {
            operation_id: vsift_domain::OperationId::parse("op_0123456789abcdef0123456789abcdef")?,
            result_sha256: DIGEST.to_owned(),
        });
        let events = parse_events(&ack.line());
        assert_eq!(events.acks, vec![ack.clone()]);
        let without = ack
            .line()
            .rsplit_once(' ')
            .map(|(head, _)| head.to_owned())
            .ok_or("no field")?;
        let foreign = ack.line().replacen(" request ", " renew ", 1);
        let events = parse_events(&format!("{without}\n{foreign}\n"));
        assert!(events.acks.is_empty());
        assert_eq!(events.malformed, vec![1, 2]);
        Ok(())
    }

    #[test]
    fn acknowledgements_round_trip_and_other_lines_are_ignored()
    -> Result<(), Box<dyn std::error::Error>> {
        let ack = ack()?;
        let text = format!(
            "[    1.234] kernel noise\nSTART 7 1 evidence\n{}\nFAIL 8 2 renew STORAGE_IO\nVERIFY OK\n",
            ack.line()
        );
        let events = parse_events(&text);
        assert_eq!(events.acks, vec![ack]);
        assert_eq!(events.starts.len(), 1);
        assert_eq!(events.failures.len(), 1);
        assert_eq!(events.failures[0].code, "STORAGE_IO");
        assert!(events.malformed.is_empty());
        Ok(())
    }

    #[test]
    fn a_damaged_protocol_line_is_reported() -> Result<(), Box<dyn std::error::Error>> {
        let line = ack()?.line();
        let truncated = &line[..line.len() - 10];
        let events = parse_events(&format!(
            "{truncated}\nACK x\nFAIL 1 2 renew storage_io\nSTART 1 2 nothing\n{line} extra\n"
        ));
        assert!(events.acks.is_empty());
        assert_eq!(events.malformed, vec![1, 2, 3, 4, 5]);
        Ok(())
    }

    #[test]
    fn marks_name_their_acknowledgement() {
        assert_eq!(mark_for(42), "ack-42");
        assert_eq!(seq_of_mark("ack-42"), Some(42));
        assert_eq!(seq_of_mark("ack-"), None);
        assert_eq!(seq_of_mark("mkfs"), None);
        assert_eq!(start_mark_for(42), "start-42");
        assert_eq!(seq_of_start_mark("start-42"), Some(42));
        assert_eq!(seq_of_start_mark("ack-42"), None);
        assert_eq!(seq_of_mark("start-42"), None);
    }
}
