//! Verified reads of committed transcript, visual-index and evidence records and the source copy.

use std::{io::Read, path::PathBuf};

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use vsift_application::SessionStorageError;
use vsift_domain::{
    EvidenceRecord, SessionId, SessionPhase, SourceId, TranscriptRevision, TranscriptRevisionId,
    VisualIndex,
};

use super::map_committed_io;
use super::{
    ARTIFACTS_DIRECTORY, CommittedSource, FilesystemSessionStore, SESSIONS_DIRECTORY,
    SessionReadHold, SessionStatus, StoredArtifact, StoredArtifactKind,
    chain::read_committed_manifest, map_storage_io, open_regular_file, sha256_hex,
};
use crate::VerifiedSourceIdentity;

impl FilesystemSessionStore {
    /// Reads the latest committed transcript revision of an open session.
    ///
    /// The record is read under a shared lifetime hold, its size and SHA-256
    /// are checked against the committed manifest, and it is rebuilt through
    /// the domain constructors. The committed status is returned beside it so
    /// callers can present lifecycle and scope cursors to it.
    ///
    /// # Errors
    ///
    /// Closed or expired sessions conflict; a changed, oversized or invalid
    /// record is an integrity failure.
    pub fn read_transcript(
        &self,
        session_id: &SessionId,
        now_unix_seconds: u64,
    ) -> Result<Option<(TranscriptRevision, SessionStatus)>, SessionStorageError> {
        self.read_transcript_where(session_id, now_unix_seconds, |_| true)
    }

    /// Reads one committed transcript revision of an open session by identity.
    ///
    /// Every revision stays readable after a newer one supersedes it, so a
    /// citation of an older revision can always be resolved (ADR 0017). Records
    /// are read newest first, each verified and decoded as by
    /// [`Self::read_transcript`], until one has `revision`'s identity.
    ///
    /// # Errors
    ///
    /// As [`Self::read_transcript`]; a revision the session does not hold is
    /// `Ok(None)`.
    pub fn read_transcript_revision(
        &self,
        session_id: &SessionId,
        revision: &TranscriptRevisionId,
        now_unix_seconds: u64,
    ) -> Result<Option<(TranscriptRevision, SessionStatus)>, SessionStorageError> {
        self.read_transcript_where(session_id, now_unix_seconds, |candidate| {
            candidate.id() == revision
        })
    }

    /// Reads the newest committed transcript revision of an open session, if
    /// it has one, and the committed status, both from one manifest.
    ///
    /// A caller that needs both, such as a commit that must follow the
    /// session, never pairs a head without a revision with a generation that
    /// already holds one: separate reads could see a commit in between.
    ///
    /// # Errors
    ///
    /// As [`Self::read_transcript`].
    pub fn read_transcript_head(
        &self,
        session_id: &SessionId,
        now_unix_seconds: u64,
    ) -> Result<(Option<TranscriptRevision>, SessionStatus), SessionStorageError> {
        let mut status = None;
        let newest = self.read_transcript_scan(
            session_id,
            now_unix_seconds,
            |_| true,
            |observed| status = Some(observed.clone()),
        )?;
        match (newest, status) {
            (Some((revision, status)), _) => Ok((Some(revision), status)),
            (None, Some(status)) => Ok((None, status)),
            (None, None) => Err(SessionStorageError::StateConflict),
        }
    }

    pub(super) fn read_transcript_where(
        &self,
        session_id: &SessionId,
        now_unix_seconds: u64,
        wanted: impl FnMut(&TranscriptRevision) -> bool,
    ) -> Result<Option<(TranscriptRevision, SessionStatus)>, SessionStorageError> {
        self.read_transcript_scan(session_id, now_unix_seconds, wanted, |_| {})
    }

    /// The first revision, newest first, that `wanted` accepts; `observed`
    /// sees the status of the manifest read, whatever is found.
    fn read_transcript_scan(
        &self,
        session_id: &SessionId,
        now_unix_seconds: u64,
        mut wanted: impl FnMut(&TranscriptRevision) -> bool,
        observed: impl FnOnce(&SessionStatus),
    ) -> Result<Option<(TranscriptRevision, SessionStatus)>, SessionStorageError> {
        let _hold = self.acquire_read(session_id)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(map_committed_io)?;
        let committed = read_committed_manifest(&session, session_id, self.chain_check())?;
        let record = committed
            .manifest
            .lifecycle
            .ok_or(SessionStorageError::StateConflict)?;
        let status = record.to_status(session_id.clone(), committed.manifest.generation)?;
        if status.phase() != SessionPhase::Open || status.lifetime().expired(now_unix_seconds) {
            return Err(SessionStorageError::StateConflict);
        }
        observed(&status);
        let mut records = record
            .artifacts
            .iter()
            .rev()
            .filter(|artifact| artifact.kind == StoredArtifactKind::TranscriptRecord)
            .peekable();
        if records.peek().is_none() {
            return Ok(None);
        }
        let artifacts = session
            .open_dir_nofollow(ARTIFACTS_DIRECTORY)
            .map_err(map_committed_io)?;
        for artifact in records {
            let revision = read_transcript_artifact(&artifacts, artifact, status.source_id())?;
            if wanted(&revision) {
                return Ok(Some((revision, status)));
            }
        }
        Ok(None)
    }

    /// Reads the newest committed visual-index revision of an open session.
    ///
    /// Each revision holds every window of the one before it, so only the
    /// newest record is read. It is read under a shared lifetime hold, its
    /// size and SHA-256 are checked against the committed manifest, and it is
    /// decoded strictly (window grid, coverage and change rules, identities)
    /// and must describe the session's source.
    ///
    /// # Errors
    ///
    /// Closed or expired sessions conflict; a changed, oversized or invalid
    /// record is an integrity failure, and a newer record version is
    /// unsupported.
    pub fn read_visual_index(
        &self,
        session_id: &SessionId,
        now_unix_seconds: u64,
    ) -> Result<Option<(VisualIndex, SessionStatus)>, SessionStorageError> {
        let _hold = self.acquire_read(session_id)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(map_committed_io)?;
        let committed = read_committed_manifest(&session, session_id, self.chain_check())?;
        let record = committed
            .manifest
            .lifecycle
            .ok_or(SessionStorageError::StateConflict)?;
        let status = record.to_status(session_id.clone(), committed.manifest.generation)?;
        if status.phase() != SessionPhase::Open || status.lifetime().expired(now_unix_seconds) {
            return Err(SessionStorageError::StateConflict);
        }
        let Some(newest) = record
            .artifacts
            .iter()
            .rev()
            .find(|artifact| artifact.kind == StoredArtifactKind::VisualIndexRecord)
        else {
            return Ok(None);
        };
        let artifacts = session
            .open_dir_nofollow(ARTIFACTS_DIRECTORY)
            .map_err(map_committed_io)?;
        let index = read_visual_index_artifact(&artifacts, newest, session_id, status.source_id())?;
        Ok(Some((index, status)))
    }

    /// Opens the committed private source copy of an open session under a
    /// shared lifetime hold, with the identity its lifecycle record gives it.
    pub(crate) fn committed_source(
        &self,
        session_id: &SessionId,
        now_unix_seconds: u64,
    ) -> Result<CommittedSource, SessionStorageError> {
        let hold = self.acquire_read(session_id)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(map_committed_io)?;
        let committed = read_committed_manifest(&session, session_id, self.chain_check())?;
        let record = committed
            .manifest
            .lifecycle
            .ok_or(SessionStorageError::StateConflict)?;
        let status = record.to_status(session_id.clone(), committed.manifest.generation)?;
        if status.phase() != SessionPhase::Open || status.lifetime().expired(now_unix_seconds) {
            return Err(SessionStorageError::StateConflict);
        }
        let directory = session
            .open_dir_nofollow(ARTIFACTS_DIRECTORY)
            .map_err(map_committed_io)?;
        let directory_path = self
            .root_path
            .join(SESSIONS_DIRECTORY)
            .join(session_id.as_str())
            .join(ARTIFACTS_DIRECTORY);
        let verified_identity = match record.verified_source_identity.as_deref() {
            None => None,
            Some(text) => Some(
                VerifiedSourceIdentity::parse(text).ok_or(SessionStorageError::IntegrityFailure)?,
            ),
        };
        Ok(CommittedSource {
            directory,
            directory_path,
            hold,
            source_id: status.source_id().clone(),
            file_name: record.source_name,
            bytes: status.source_bytes(),
            verified_identity,
        })
    }
}

impl FilesystemSessionStore {
    /// Opens the existing artifact directory under a verified session and lifetime hold.
    /// The returned capability is for internal media staging; P05 owns publication.
    pub(crate) fn source_artifact_directory(
        &self,
        session_id: &SessionId,
    ) -> Result<(Dir, PathBuf, SessionReadHold), SessionStorageError> {
        let hold = self.acquire_read(session_id)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(map_storage_io)?;
        let committed = read_committed_manifest(&session, session_id, self.chain_check())?;
        if let Some(record) = committed.manifest.lifecycle {
            let status = record.to_status(session_id.clone(), committed.manifest.generation)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| SessionStorageError::Io)?
                .as_secs();
            if status.phase() != SessionPhase::Open || status.lifetime().expired(now) {
                return Err(SessionStorageError::StateConflict);
            }
        }
        let artifacts = session
            .open_dir_nofollow(ARTIFACTS_DIRECTORY)
            .map_err(map_storage_io)?;
        let path = self
            .root_path
            .join(SESSIONS_DIRECTORY)
            .join(session_id.as_str())
            .join(ARTIFACTS_DIRECTORY);
        Ok((artifacts, path, hold))
    }
}

/// Reads, verifies and decodes one transcript record artifact.
///
/// The recorded size and digest are checked before any byte is interpreted;
/// the record is then decoded strictly and must describe `source_id`. The
/// artifact record's size was already bounded by its kind, so the read is too.
pub(super) fn read_transcript_artifact(
    directory: &Dir,
    artifact: &StoredArtifact,
    source_id: &SourceId,
) -> Result<TranscriptRevision, SessionStorageError> {
    let file = open_regular_file(directory, &artifact.name, false).map_err(map_committed_io)?;
    let mut bytes = Vec::new();
    file.take(artifact.bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(map_storage_io)?;
    if u64::try_from(bytes.len()).ok() != Some(artifact.bytes)
        || sha256_hex(&bytes) != artifact.sha256
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let revision = crate::decode_transcript_record(&bytes)?;
    if revision.source_id() != source_id {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(revision)
}

/// Reads, verifies and decodes one visual-index record artifact.
///
/// As for transcript records, the recorded size and digest are checked
/// before any byte is interpreted; the record is then decoded strictly, must
/// belong to `session_id` and must describe `source_id`.
pub(super) fn read_visual_index_artifact(
    directory: &Dir,
    artifact: &StoredArtifact,
    session_id: &SessionId,
    source_id: &SourceId,
) -> Result<VisualIndex, SessionStorageError> {
    let file = open_regular_file(directory, &artifact.name, false).map_err(map_committed_io)?;
    let mut bytes = Vec::new();
    file.take(artifact.bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(map_storage_io)?;
    if u64::try_from(bytes.len()).ok() != Some(artifact.bytes)
        || sha256_hex(&bytes) != artifact.sha256
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let index = crate::decode_visual_index_record(&bytes, session_id)?;
    if index.source_id() != source_id {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(index)
}

/// Reads, verifies and decodes one evidence record artifact.
///
/// As for the other records, the recorded size and digest are checked
/// before any byte is interpreted; the record is then decoded strictly,
/// must belong to `session_id` and must describe `source_id`.
pub(super) fn read_evidence_artifact(
    directory: &Dir,
    artifact: &StoredArtifact,
    session_id: &SessionId,
    source_id: &SourceId,
) -> Result<EvidenceRecord, SessionStorageError> {
    let file = open_regular_file(directory, &artifact.name, false).map_err(map_committed_io)?;
    let mut bytes = Vec::new();
    file.take(artifact.bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(map_storage_io)?;
    if u64::try_from(bytes.len()).ok() != Some(artifact.bytes)
        || sha256_hex(&bytes) != artifact.sha256
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let record = crate::decode_evidence_record(&bytes, session_id)?;
    if record.source_id() != source_id {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(record)
}
