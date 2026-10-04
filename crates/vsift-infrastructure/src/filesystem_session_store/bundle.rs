//! Explicitly retained bundles: export and validation as data.

use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::Path,
};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use vsift_application::SessionStorageError;
use vsift_domain::{EvidenceRecord, EvidenceSubject, PublicationGuarantee, SessionId, SourceId};

use super::map_committed_io;
use super::{
    ARTIFACTS_DIRECTORY, BundleManifest, BundleSourcePolicy, BundleStatus, ChainCheck,
    FilesystemSessionStore, MAX_SESSION_ARTIFACT_BYTES, MAX_SESSION_ARTIFACTS, SESSIONS_DIRECTORY,
    StoredArtifact, StoredArtifactKind,
    chain::read_committed_manifest,
    copy_and_hash_bounded, create_private_child_directory, create_regular_file, hash_bounded,
    map_open_error, map_storage_io, open_regular_file,
    publication::evidence_artifact_count,
    read_bounded_manifest,
    reads::{read_evidence_artifact, read_transcript_artifact, read_visual_index_artifact},
    root::{
        map_restrict_error, validate_platform_root_permissions, validate_root_selection,
        validate_same_object,
    },
    sha256_hex,
    stored::{parse_versioned_json, validate_artifact_record},
};
use crate::private_user_root::restrict_new_directory;

impl FilesystemSessionStore {
    /// Explicitly exports one committed session to a new private data-only directory.
    ///
    /// The destination is created exclusively and is never selected for automatic
    /// cleanup. The manifest is written last: interrupted output remains visibly
    /// incomplete and cannot validate as a retained bundle.
    ///
    /// # Errors
    ///
    /// Existing destinations conflict; invalid source bytes, metadata, permissions,
    /// or output paths fail without changing the original media.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the export validation and commit ordering visible together"
    )]
    pub fn retain_bundle(
        &self,
        session_id: &SessionId,
        output_path: &Path,
        source_policy: BundleSourcePolicy,
    ) -> Result<BundleStatus, SessionStorageError> {
        validate_root_selection(output_path).map_err(map_open_error)?;
        let _hold = self.acquire_read(session_id)?;
        let sessions = self
            .root
            .open_dir_nofollow(SESSIONS_DIRECTORY)
            .map_err(map_storage_io)?;
        let session = sessions
            .open_dir_nofollow(session_id.as_str())
            .map_err(map_committed_io)?;
        let committed = read_committed_manifest(&session, session_id, ChainCheck::Full)?;
        let record = committed
            .manifest
            .lifecycle
            .ok_or(SessionStorageError::StateConflict)?;
        let status = record.to_status(session_id.clone(), committed.manifest.generation)?;

        let parent_path = output_path
            .parent()
            .ok_or(SessionStorageError::AccessDenied)?;
        let name = output_path
            .file_name()
            .ok_or(SessionStorageError::AccessDenied)?;
        let canonical_parent = fs::canonicalize(parent_path).map_err(map_storage_io)?;
        let parent = Dir::open_ambient_dir(&canonical_parent, cap_std::ambient_authority())
            .map_err(map_storage_io)?;
        create_private_child_directory(&parent, Path::new(name)).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                SessionStorageError::StateConflict
            } else {
                map_storage_io(error)
            }
        })?;
        let bundle = parent
            .open_dir_nofollow(Path::new(name))
            .map_err(map_storage_io)?;
        // The export is private whatever the chosen parent would pass on.
        if let Err(error) = restrict_new_directory(&canonical_parent.join(name)) {
            drop(bundle);
            let _ = parent.remove_dir(Path::new(name));
            return Err(map_open_error(map_restrict_error(error)));
        }
        if let Err(error) = validate_platform_root_permissions(output_path, &bundle) {
            drop(bundle);
            let _ = parent.remove_dir(Path::new(name));
            return Err(map_open_error(error));
        }
        let bundle = parent
            .open_dir_nofollow(Path::new(name))
            .map_err(map_storage_io)?;
        let manifest = BundleManifest {
            schema_version: 1,
            format: "vsift.bundle".to_owned(),
            session_id: session_id.as_str().to_owned(),
            source_id: status.source_id().as_str().to_owned(),
            source_bytes: status.source_bytes(),
            source_included: source_policy == BundleSourcePolicy::IncludeSource,
            publication: PublicationGuarantee::ProcessCrashConsistent
                .identifier()
                .to_owned(),
            artifacts: record.artifacts.clone(),
        };
        let artifacts = session
            .open_dir_nofollow(ARTIFACTS_DIRECTORY)
            .map_err(map_committed_io)?;
        let source =
            open_regular_file(&artifacts, &record.source_name, false).map_err(map_committed_io)?;
        let source_digest = hash_bounded(source, status.source_bytes())?;
        if source_digest
            != status
                .source_id()
                .as_str()
                .trim_start_matches("src_sha256_")
        {
            return Err(SessionStorageError::IntegrityFailure);
        }
        for artifact in &record.artifacts {
            validate_artifact_record(artifact)?;
            let input =
                open_regular_file(&artifacts, &artifact.name, false).map_err(map_committed_io)?;
            let mut target_options = OpenOptions::new();
            target_options
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No);
            let mut target = bundle
                .open_with(&artifact.name, &target_options)
                .map_err(map_storage_io)?;
            let observed = copy_and_hash_bounded(input, &mut target, artifact.bytes)?;
            if observed != artifact.sha256 {
                return Err(SessionStorageError::IntegrityFailure);
            }
            target.sync_all().map_err(map_storage_io)?;
        }
        if source_policy == BundleSourcePolicy::IncludeSource {
            let source = open_regular_file(&artifacts, &record.source_name, false)
                .map_err(map_committed_io)?;
            let mut target_options = OpenOptions::new();
            target_options
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No);
            let mut target = bundle
                .open_with("source.media", &target_options)
                .map_err(map_storage_io)?;
            let observed = copy_and_hash_bounded(source, &mut target, status.source_bytes())?;
            if observed
                != status
                    .source_id()
                    .as_str()
                    .trim_start_matches("src_sha256_")
            {
                return Err(SessionStorageError::IntegrityFailure);
            }
            target.sync_all().map_err(map_storage_io)?;
        }
        let bytes = serde_json::to_vec(&manifest).map_err(|_| SessionStorageError::Io)?;
        create_regular_file(&bundle, "bundle.json", &bytes).map_err(map_storage_io)?;
        let observed_parent = fs::canonicalize(parent_path).map_err(map_storage_io)?;
        if observed_parent != canonical_parent {
            return Err(SessionStorageError::AccessDenied);
        }
        Self::validate_bundle(output_path)
    }

    /// Validates a selected retained directory as bounded data without executing it.
    ///
    /// # Errors
    ///
    /// Rejects future versions, unexpected entries, links, size changes, and hash
    /// mismatches. Evidence-only bundles disclose the matching source requirement.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep every bundle check and its order visible together"
    )]
    pub fn validate_bundle(path: &Path) -> Result<BundleStatus, SessionStorageError> {
        validate_root_selection(path).map_err(map_open_error)?;
        let metadata = fs::symlink_metadata(path).map_err(map_storage_io)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(SessionStorageError::IntegrityFailure);
        }
        let parent_path = path.parent().ok_or(SessionStorageError::AccessDenied)?;
        let name = path.file_name().ok_or(SessionStorageError::AccessDenied)?;
        let parent = Dir::open_ambient_dir(parent_path, cap_std::ambient_authority())
            .map_err(map_storage_io)?;
        let bundle = parent
            .open_dir_nofollow(Path::new(name))
            .map_err(map_committed_io)?;
        validate_same_object(&metadata, &bundle).map_err(map_open_error)?;
        validate_platform_root_permissions(path, &bundle).map_err(map_open_error)?;
        let manifest_bytes = open_regular_file(&bundle, "bundle.json", false)
            .and_then(read_bounded_manifest)
            .map_err(map_committed_io)?;
        let manifest: BundleManifest = parse_versioned_json(&manifest_bytes)?;
        if manifest.format != "vsift.bundle"
            || manifest.publication != PublicationGuarantee::ProcessCrashConsistent.identifier()
            || manifest.source_bytes == 0
            || manifest.source_bytes > crate::MAX_SOURCE_BYTES
        {
            return Err(SessionStorageError::IntegrityFailure);
        }
        let session_id = SessionId::parse(&manifest.session_id)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        let source_id = SourceId::parse(&manifest.source_id)
            .map_err(|_| SessionStorageError::IntegrityFailure)?;
        if manifest.artifacts.len() > MAX_SESSION_ARTIFACTS {
            return Err(SessionStorageError::CapacityExhausted);
        }
        let expected_entries = 1 + manifest.artifacts.len() + usize::from(manifest.source_included);
        let mut entries = 0_usize;
        for entry in bundle.entries().map_err(map_storage_io)? {
            let entry = entry.map_err(map_storage_io)?;
            let name = entry.file_name();
            if name != "bundle.json"
                && !(manifest.source_included && name == "source.media")
                && !manifest
                    .artifacts
                    .iter()
                    .any(|artifact| name.to_str() == Some(artifact.name.as_str()))
            {
                return Err(SessionStorageError::IntegrityFailure);
            }
            entries = entries
                .checked_add(1)
                .ok_or(SessionStorageError::CapacityExhausted)?;
            if entries > expected_entries {
                return Err(SessionStorageError::IntegrityFailure);
            }
        }
        if entries != expected_entries {
            return Err(SessionStorageError::IntegrityFailure);
        }
        if manifest.source_included {
            let source =
                open_regular_file(&bundle, "source.media", false).map_err(map_committed_io)?;
            let digest = hash_bounded(source, manifest.source_bytes)?;
            if digest != source_id.as_str().trim_start_matches("src_sha256_") {
                return Err(SessionStorageError::IntegrityFailure);
            }
        }
        let mut artifact_bytes = 0_u64;
        let mut evidence = Vec::new();
        for artifact in &manifest.artifacts {
            validate_artifact_record(artifact)?;
            if artifact.kind == StoredArtifactKind::EvidenceRecord {
                // As for the other records, a matching digest proves only that
                // the file is the one the manifest names; decoding re-derives
                // the request key and every item identity (ADR 0013 note of
                // 2026-09-26), and the items are checked against the media
                // below.
                evidence.push(read_evidence_artifact(
                    &bundle,
                    artifact,
                    &session_id,
                    &source_id,
                )?);
            } else if artifact.kind == StoredArtifactKind::TranscriptRecord {
                // A matching digest only proves the file is the one the manifest
                // names; both could have been rewritten together. Decoding
                // applies the same strict record and import rules as a session
                // read, so a bundle validates only if its transcript could be
                // cited.
                read_transcript_artifact(&bundle, artifact, &source_id)?;
            } else if artifact.kind == StoredArtifactKind::VisualIndexRecord {
                // The same reasoning as for transcripts: every visual-index
                // record is decoded with the window grid, coverage, policy
                // and identity rules a session read applies (ADR 0013 note
                // of 2026-09-26).
                read_visual_index_artifact(&bundle, artifact, &session_id, &source_id)?;
            } else {
                let file =
                    open_regular_file(&bundle, &artifact.name, false).map_err(map_committed_io)?;
                if hash_bounded(file, artifact.bytes)? != artifact.sha256 {
                    return Err(SessionStorageError::IntegrityFailure);
                }
            }
            artifact_bytes = artifact_bytes
                .checked_add(artifact.bytes)
                .ok_or(SessionStorageError::CapacityExhausted)?;
        }
        if artifact_bytes > MAX_SESSION_ARTIFACT_BYTES {
            return Err(SessionStorageError::CapacityExhausted);
        }
        if evidence_artifact_count(&manifest.artifacts) > crate::MAX_EVIDENCE_ARTIFACTS {
            return Err(SessionStorageError::CapacityExhausted);
        }
        validate_bundled_evidence(&bundle, &manifest.artifacts, &evidence)?;
        Ok(BundleStatus {
            session_id,
            source_id,
            source_bytes: manifest.source_bytes,
            source_policy: if manifest.source_included {
                BundleSourcePolicy::IncludeSource
            } else {
                BundleSourcePolicy::EvidenceOnly
            },
            artifact_count: manifest.artifacts.len(),
            artifact_bytes,
            manifest_sha256: sha256_hex(&manifest_bytes),
        })
    }
}

/// Checks a bundle's evidence records against its media (ADR 0013 note of
/// 2026-09-26): every item's file is in the manifest with the item's kind,
/// size and digest; an image's PNG header states the item's dimensions and
/// a clip's WAV header the 16 kHz mono 16-bit format and its data size;
/// every crop parent and neighbours anchor is an item of the bundle; and
/// items with one identity agree on everything but their source check.
pub(super) fn validate_bundled_evidence(
    bundle: &Dir,
    artifacts: &[StoredArtifact],
    records: &[EvidenceRecord],
) -> Result<(), SessionStorageError> {
    let mut items: Vec<&vsift_domain::EvidenceItem> = Vec::new();
    for record in records {
        for item in record.items() {
            match items.iter().find(|known| known.id() == item.id()) {
                Some(known) if !known.same_content(item) => {
                    return Err(SessionStorageError::IntegrityFailure);
                }
                Some(_) => {}
                None => items.push(item),
            }
        }
    }
    let mut checked: BTreeSet<&str> = BTreeSet::new();
    for item in &items {
        let media = item.media();
        let kind = StoredArtifactKind::from_media(media.kind());
        let artifact = artifacts
            .iter()
            .find(|artifact| {
                artifact.kind == kind
                    && artifact.sha256 == media.sha256().as_str()
                    && artifact.bytes == media.bytes()
            })
            .ok_or(SessionStorageError::IntegrityFailure)?;
        if let EvidenceSubject::Crop { region, .. } = item.subject()
            && !items.iter().any(|parent| parent.id() == &region.parent)
        {
            return Err(SessionStorageError::IntegrityFailure);
        }
        if !checked.insert(artifact.name.as_str()) {
            continue;
        }
        let file = open_regular_file(bundle, &artifact.name, false).map_err(map_committed_io)?;
        let mut header = Vec::new();
        file.take(u64::try_from(crate::WAV_HEADER_BYTES).unwrap_or(u64::MAX))
            .read_to_end(&mut header)
            .map_err(map_storage_io)?;
        let matches = match item.subject().image_dimensions() {
            Some(dimensions) => png_header_states(&header, dimensions),
            None => wav_header_states(&header, media.bytes()),
        };
        if !matches {
            return Err(SessionStorageError::IntegrityFailure);
        }
    }
    for record in records {
        if let vsift_domain::EvidenceRequest::Neighbours { anchor, .. } = record.request()
            && !items.iter().any(|item| item.id() == anchor)
        {
            return Err(SessionStorageError::IntegrityFailure);
        }
    }
    Ok(())
}

/// Whether `header` begins a PNG whose first chunk is an `IHDR` for an
/// 8-bit RGB, non-interlaced image of `dimensions`.
pub(super) fn png_header_states(header: &[u8], dimensions: vsift_domain::FrameDimensions) -> bool {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    let mut expected = SIGNATURE.to_vec();
    expected.extend_from_slice(&13_u32.to_be_bytes());
    expected.extend_from_slice(b"IHDR");
    expected.extend_from_slice(&dimensions.width().to_be_bytes());
    expected.extend_from_slice(&dimensions.height().to_be_bytes());
    // Bit depth 8, colour type 2 (RGB), deflate, adaptive filtering, no
    // interlace.
    expected.extend_from_slice(&[8, 2, 0, 0, 0]);
    header.get(..expected.len()) == Some(expected.as_slice())
}

/// Whether `header` is the canonical 44-byte header of a 16 kHz mono
/// signed 16-bit WAV file of `bytes` bytes.
pub(super) fn wav_header_states(header: &[u8], bytes: u64) -> bool {
    let Ok(data) = u32::try_from(bytes.saturating_sub(44)) else {
        return false;
    };
    let Some(riff) = data.checked_add(36) else {
        return false;
    };
    let mut expected = b"RIFF".to_vec();
    expected.extend_from_slice(&riff.to_le_bytes());
    expected.extend_from_slice(b"WAVEfmt ");
    expected.extend_from_slice(&16_u32.to_le_bytes());
    expected.extend_from_slice(&1_u16.to_le_bytes());
    expected.extend_from_slice(&1_u16.to_le_bytes());
    expected.extend_from_slice(&crate::WAV_SAMPLE_RATE.to_le_bytes());
    expected.extend_from_slice(&(crate::WAV_SAMPLE_RATE * 2).to_le_bytes());
    expected.extend_from_slice(&2_u16.to_le_bytes());
    expected.extend_from_slice(&16_u16.to_le_bytes());
    expected.extend_from_slice(b"data");
    expected.extend_from_slice(&data.to_le_bytes());
    bytes > 44 && header == expected.as_slice()
}
