//! Reading the committed generation and validating its manifest chain.

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use vsift_application::SessionStorageError;
use vsift_domain::SessionId;

use super::{
    CURRENT_FILE, CommitPointer, CommittedManifest, GENERATIONS_DIRECTORY, GenerationManifest,
    MAX_GENERATIONS_PER_SESSION, is_canonical_sha256, open_regular_file, read_bounded, sha256_hex,
    stored::{parse_versioned_json, read_versioned_json_file},
};

pub(super) fn read_committed_manifest(
    session: &Dir,
    session_id: &SessionId,
) -> Result<CommittedManifest, SessionStorageError> {
    let pointer = read_versioned_json_file::<CommitPointer>(session, CURRENT_FILE)?;
    if !is_canonical_sha256(&pointer.manifest_sha256) {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let generations = session
        .open_dir_nofollow(GENERATIONS_DIRECTORY)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let name = format!("{}.json", pointer.generation);
    let file = open_regular_file(&generations, &name, false)
        .map_err(|_| SessionStorageError::IntegrityFailure)?;
    let bytes = read_bounded(file).map_err(|_| SessionStorageError::IntegrityFailure)?;
    if sha256_hex(&bytes) != pointer.manifest_sha256 {
        return Err(SessionStorageError::IntegrityFailure);
    }
    let manifest: GenerationManifest = parse_versioned_json(&bytes)?;
    if manifest.generation != pointer.generation || manifest.session_id != session_id.as_str() {
        return Err(SessionStorageError::IntegrityFailure);
    }
    if manifest.generation == 0 {
        if manifest.previous_manifest_sha256.is_some() {
            return Err(SessionStorageError::IntegrityFailure);
        }
    } else if !manifest
        .previous_manifest_sha256
        .as_deref()
        .is_some_and(is_canonical_sha256)
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    validate_manifest_chain(&generations, session_id, &manifest)?;
    Ok(CommittedManifest {
        manifest,
        digest: pointer.manifest_sha256,
    })
}

pub(super) fn validate_manifest_chain(
    generations: &Dir,
    session_id: &SessionId,
    current: &GenerationManifest,
) -> Result<(), SessionStorageError> {
    if current.generation >= MAX_GENERATIONS_PER_SESSION {
        return Err(SessionStorageError::CapacityExhausted);
    }
    let mut generation = current.generation;
    let mut expected_digest = current.previous_manifest_sha256.clone();
    while generation > 0 {
        generation -= 1;
        let expected = expected_digest.ok_or(SessionStorageError::IntegrityFailure)?;
        let file = open_regular_file(generations, &format!("{generation}.json"), false)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let bytes = read_bounded(file).map_err(|_| SessionStorageError::IntegrityFailure)?;
        if sha256_hex(&bytes) != expected {
            return Err(SessionStorageError::IntegrityFailure);
        }
        let manifest: GenerationManifest = parse_versioned_json(&bytes)?;
        if manifest.session_id != session_id.as_str() || manifest.generation != generation {
            return Err(SessionStorageError::IntegrityFailure);
        }
        expected_digest = manifest.previous_manifest_sha256;
    }
    if expected_digest.is_some() {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(())
}
