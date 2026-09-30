//! Reading one session's identities for `handoff check --session` (P13 PR 5).

use vsift_application::SessionStorageError;
use vsift_contract::{HandoffEvidenceKind, HandoffSessionGap, HandoffSessionRecords};
use vsift_domain::{EvidenceSubject, SessionId, SessionPhase};

use crate::{engine::Engine, error::EngineError};

/// What a session offers a handoff check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HandoffSessionLookup {
    /// The session is open: every transcript segment of every committed
    /// revision, and every committed evidence item with its kind.
    Open(HandoffSessionRecords),
    /// The session's records cannot be read now.
    Unavailable(HandoffSessionGap),
}

impl Engine {
    /// Reads, without renewing or writing anything, the identities one
    /// session holds, so a handoff's citations can be resolved against them.
    ///
    /// A missing session root or session, a closed session and an expired
    /// one are [`HandoffSessionLookup::Unavailable`], not failures: the
    /// check of the draft itself still runs. Only the identities are read:
    /// no evidence file is opened.
    ///
    /// # Errors
    ///
    /// Fails when the root or clock cannot be read, or a committed record
    /// fails its integrity checks.
    pub fn handoff_session_records(
        &self,
        session_id: &SessionId,
    ) -> Result<HandoffSessionLookup, EngineError> {
        let (store, now) = self.existing_store()?;
        let Some(store) = store else {
            return Ok(HandoffSessionLookup::Unavailable(
                HandoffSessionGap::SessionNotFound,
            ));
        };
        let Some(status) = store.indexed_session_status(session_id)? else {
            return Ok(HandoffSessionLookup::Unavailable(
                HandoffSessionGap::SessionNotFound,
            ));
        };
        if status.phase() == SessionPhase::Closed {
            return Ok(HandoffSessionLookup::Unavailable(
                HandoffSessionGap::SessionClosed,
            ));
        }
        if status.lifetime().expired(now) {
            return Ok(HandoffSessionLookup::Unavailable(
                HandoffSessionGap::SessionExpired,
            ));
        }
        let mut records = HandoffSessionRecords::new();
        let read = store
            .visit_transcript_revisions(session_id, now, |revision| {
                for segment in revision.segments() {
                    records.add_segment(segment.id().as_str());
                }
            })
            .and_then(|()| store.read_evidence_records(session_id, now));
        let inventory = match read {
            Ok(inventory) => inventory,
            // The session closed or expired between the status and the
            // reads; report it as the gap the status now shows.
            Err(SessionStorageError::StateConflict) => {
                return Ok(HandoffSessionLookup::Unavailable(
                    match store.session_status(session_id) {
                        Ok(status) if status.phase() == SessionPhase::Closed => {
                            HandoffSessionGap::SessionClosed
                        }
                        _ => HandoffSessionGap::SessionExpired,
                    },
                ));
            }
            Err(error) => return Err(error.into()),
        };
        for record in inventory.records() {
            for item in record.items() {
                let kind = match item.subject() {
                    EvidenceSubject::Frame(_) => HandoffEvidenceKind::Frame,
                    EvidenceSubject::Crop { .. } => HandoffEvidenceKind::Crop,
                    EvidenceSubject::Audio { .. } => HandoffEvidenceKind::Audio,
                };
                records.add_evidence(item.id().as_str(), kind);
            }
        }
        Ok(HandoffSessionLookup::Open(records))
    }
}
