//! Reading the committed generation and validating its manifest chain.
//!
//! Every generation names the SHA-256 of the one before it, so the pointer's
//! digest of the head fixes the whole history. Until P10 every read hashed the
//! chain down to generation 0, which made a warm read cost grow by about
//! 3.7 ms per generation (#164). Reads now stop at a verified anchor:
//!
//! - the session's `chain-verified.json`, which only the writer advances,
//!   under the writer lock, after a successful publication, and never ahead
//!   of the head; or
//! - the last head this store instance verified, so several reads in one
//!   command verify the chain once.
//!
//! **Guarantee.** Every read verifies the head and every generation committed
//! since the last full verification, in the same way as before: the pointer's
//! digest, each link's digest, the session and the generation number. The
//! generation at the anchor must hash to both the link from the generation
//! above it and the anchor's own digest. Generations below the anchor were
//! verified when it was set and are not re-read. Retained exports and cleanup
//! still walk the whole chain, and committed artifacts are still re-hashed
//! against the manifest whenever they are read (INV-02).
//!
//! A missing checkpoint, or one ahead of the head (possible only if an
//! unsynchronised ephemeral root lost its newest generations in an OS crash),
//! means a full walk. A checkpoint that does not parse or whose digest
//! disagrees with its generation is an integrity failure; a newer checkpoint
//! schema is an unsupported version.

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use vsift_application::SessionStorageError;
use vsift_domain::SessionId;

use super::{
    CHAIN_CHECKPOINT_FILE, CURRENT_FILE, ChainCheck, ChainCheckpoint, CommitPointer,
    CommittedManifest, GENERATIONS_DIRECTORY, GenerationManifest, MAX_GENERATIONS_PER_SESSION,
    VerifiedHead, is_canonical_sha256, map_storage_io, open_regular_file, read_bounded, sha256_hex,
    stored::{parse_versioned_json, read_versioned_json_file},
};

/// Reads the committed head of a session and validates its chain as `check`
/// asks.
pub(super) fn read_committed_manifest(
    session: &Dir,
    session_id: &SessionId,
    check: ChainCheck<'_>,
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
    let (anchor, cache) = match check {
        ChainCheck::Full => (None, None),
        ChainCheck::Incremental(cache) => {
            let checkpoint = read_chain_checkpoint(session)?;
            let cached = cache
                .and_then(|cache| cache.get(session_id))
                .map(|head| Anchor {
                    generation: head.generation,
                    manifest_sha256: head.manifest_sha256,
                });
            (
                newest_usable(checkpoint, cached, manifest.generation),
                cache,
            )
        }
    };
    validate_manifest_chain(
        &generations,
        session_id,
        &manifest,
        &pointer.manifest_sha256,
        anchor.as_ref(),
    )?;
    if let Some(cache) = cache {
        cache.set(VerifiedHead {
            session_id: session_id.clone(),
            generation: manifest.generation,
            manifest_sha256: pointer.manifest_sha256.clone(),
        });
    }
    Ok(CommittedManifest {
        manifest,
        digest: pointer.manifest_sha256,
    })
}

/// A generation already verified with its whole chain, and its digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Anchor {
    pub(super) generation: u64,
    pub(super) manifest_sha256: String,
}

/// The session's chain checkpoint, if it has one.
fn read_chain_checkpoint(session: &Dir) -> Result<Option<Anchor>, SessionStorageError> {
    if !session
        .try_exists(CHAIN_CHECKPOINT_FILE)
        .map_err(map_storage_io)?
    {
        return Ok(None);
    }
    let checkpoint = read_versioned_json_file::<ChainCheckpoint>(session, CHAIN_CHECKPOINT_FILE)?;
    if !is_canonical_sha256(&checkpoint.manifest_sha256)
        || checkpoint.generation >= MAX_GENERATIONS_PER_SESSION
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(Some(Anchor {
        generation: checkpoint.generation,
        manifest_sha256: checkpoint.manifest_sha256,
    }))
}

/// The newest anchor at or below the head; an anchor above it is ignored.
fn newest_usable(first: Option<Anchor>, second: Option<Anchor>, head: u64) -> Option<Anchor> {
    [first, second]
        .into_iter()
        .flatten()
        .filter(|anchor| anchor.generation <= head)
        .max_by_key(|anchor| anchor.generation)
}

/// Walks the chain down from `current` to generation 0, or to `anchor`.
pub(super) fn validate_manifest_chain(
    generations: &Dir,
    session_id: &SessionId,
    current: &GenerationManifest,
    current_sha256: &str,
    anchor: Option<&Anchor>,
) -> Result<(), SessionStorageError> {
    if current.generation >= MAX_GENERATIONS_PER_SESSION {
        return Err(SessionStorageError::CapacityExhausted);
    }
    if let Some(anchor) = anchor.filter(|anchor| anchor.generation == current.generation) {
        return if anchor.manifest_sha256 == current_sha256 {
            Ok(())
        } else {
            Err(SessionStorageError::IntegrityFailure)
        };
    }
    let mut generation = current.generation;
    let mut expected_digest = current.previous_manifest_sha256.clone();
    while generation > 0 {
        generation -= 1;
        let expected = expected_digest.ok_or(SessionStorageError::IntegrityFailure)?;
        let file = open_regular_file(generations, &format!("{generation}.json"), false)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let bytes = read_bounded(file).map_err(|_| SessionStorageError::IntegrityFailure)?;
        let digest = sha256_hex(&bytes);
        if digest != expected {
            return Err(SessionStorageError::IntegrityFailure);
        }
        if let Some(anchor) = anchor.filter(|anchor| anchor.generation == generation) {
            return if anchor.manifest_sha256 == digest {
                Ok(())
            } else {
                Err(SessionStorageError::IntegrityFailure)
            };
        }
        let manifest: GenerationManifest = parse_versioned_json(&bytes)?;
        if manifest.session_id != session_id.as_str()
            || manifest.generation != generation
            || manifest.durability != current.durability
        {
            return Err(SessionStorageError::IntegrityFailure);
        }
        expected_digest = manifest.previous_manifest_sha256;
    }
    if expected_digest.is_some() {
        return Err(SessionStorageError::IntegrityFailure);
    }
    Ok(())
}
