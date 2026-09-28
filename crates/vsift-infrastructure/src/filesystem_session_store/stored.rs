//! Validation and versioned decoding of stored metadata.

use cap_std::fs::Dir;
use serde::Deserialize;
use vsift_application::SessionStorageError;
use vsift_domain::{
    OperationId, SessionId, SessionLifetime, SessionLifetimePolicy, SessionPhase, SourceId,
    StorageGeneration, WorkspaceRetention,
};

use super::map_committed_io;
use super::{
    MAX_SESSION_ARTIFACT_BYTES, MAX_SESSION_ARTIFACTS, MetadataVersion, STORAGE_SCHEMA_VERSION,
    SessionStatus, StoredArtifact, StoredLifecycle, StoredSessionPhase, is_canonical_sha256,
    open_replaced_file, read_bounded,
};

pub(super) fn validate_artifact_record(
    artifact: &StoredArtifact,
) -> Result<(), SessionStorageError> {
    let max = artifact.kind.max_bytes();
    if !is_canonical_sha256(&artifact.sha256)
        || artifact.bytes == 0
        || artifact.bytes
            > u64::try_from(max).map_err(|_| SessionStorageError::CapacityExhausted)?
        || artifact.name != format!("artifact-{}.{}", artifact.sha256, artifact.kind.extension())
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(())
}

pub(super) fn validate_source_record(
    source_id: &str,
    source_name: &str,
    source_bytes: u64,
) -> Result<SourceId, SessionStorageError> {
    let source_id =
        SourceId::parse(source_id).map_err(|_| SessionStorageError::IntegrityFailure)?;
    let operation = source_name
        .strip_prefix("source-")
        .and_then(|value| value.strip_suffix(".media"))
        .ok_or(SessionStorageError::IntegrityFailure)?;
    OperationId::parse(operation).map_err(|_| SessionStorageError::IntegrityFailure)?;
    if source_bytes == 0 || source_bytes > crate::MAX_SOURCE_BYTES {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(source_id)
}

impl StoredLifecycle {
    /// The recorded lifetime, validated under the rules it was opened with:
    /// a workspace session's recorded retention, else the desktop policy.
    pub(super) fn validated_lifetime(&self) -> Result<SessionLifetime, SessionStorageError> {
        let policy = match self.workspace_retention_seconds {
            None => SessionLifetimePolicy::Desktop,
            Some(seconds) => SessionLifetimePolicy::Workspace(
                WorkspaceRetention::from_seconds(seconds)
                    .map_err(|_| SessionStorageError::IntegrityFailure)?,
            ),
        };
        SessionLifetime::from_record_under(
            policy,
            self.opened_at_unix_seconds,
            self.expires_at_unix_seconds,
        )
        .map_err(|_| SessionStorageError::IntegrityFailure)
    }

    pub(super) fn to_status(
        &self,
        session_id: SessionId,
        generation: u64,
    ) -> Result<SessionStatus, SessionStorageError> {
        let lifetime = self.validated_lifetime()?;
        let source_id =
            validate_source_record(&self.source_id, &self.source_name, self.source_bytes)?;
        let phase = match self.phase {
            StoredSessionPhase::Open => SessionPhase::Open,
            StoredSessionPhase::Closed => SessionPhase::Closed,
        };
        if self.artifacts.len() > MAX_SESSION_ARTIFACTS {
            return Err(SessionStorageError::CapacityExhausted);
        }
        let mut artifact_bytes = 0_u64;
        for artifact in &self.artifacts {
            validate_artifact_record(artifact)?;
            artifact_bytes = artifact_bytes
                .checked_add(artifact.bytes)
                .ok_or(SessionStorageError::CapacityExhausted)?;
        }
        if artifact_bytes > MAX_SESSION_ARTIFACT_BYTES {
            return Err(SessionStorageError::CapacityExhausted);
        }
        if self
            .verified_source_identity
            .as_deref()
            .is_some_and(|identity| !is_canonical_sha256(identity))
        {
            return Err(SessionStorageError::IntegrityFailure);
        }
        Ok(SessionStatus {
            session_id,
            source_id,
            source_bytes: self.source_bytes,
            phase,
            lifetime,
            generation: StorageGeneration::from_value(generation),
            artifact_count: self.artifacts.len(),
            artifact_bytes,
        })
    }
}

/// Reads and strictly decodes one small metadata file.
///
/// Several of these files (the commit pointer, the chain checkpoint, job
/// bindings and index entries) are replaced by rename while other processes
/// read them, so the file is opened with [`open_replaced_file`]: meeting a
/// replacement is retried, never reported as an integrity failure.
pub(super) fn read_versioned_json_file<T>(
    directory: &Dir,
    name: &str,
) -> Result<T, SessionStorageError>
where
    T: for<'de> Deserialize<'de> + MetadataVersion,
{
    let file = open_replaced_file(directory, name, false).map_err(map_committed_io)?;
    let bytes = read_bounded(file).map_err(map_committed_io)?;
    parse_versioned_json(&bytes)
}

pub(super) fn parse_versioned_json<T>(bytes: &[u8]) -> Result<T, SessionStorageError>
where
    T: for<'de> Deserialize<'de> + MetadataVersion,
{
    #[derive(Deserialize)]
    struct VersionProbe {
        schema_version: u16,
    }
    let probe: VersionProbe =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    if probe.schema_version > STORAGE_SCHEMA_VERSION {
        return Err(SessionStorageError::UnsupportedVersion);
    }
    let value: T =
        serde_json::from_slice(bytes).map_err(|_| SessionStorageError::IntegrityFailure)?;
    if value.schema_version() != STORAGE_SCHEMA_VERSION {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(value)
}
