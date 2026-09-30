//! Offline import of a reviewed artifact from a user's folder (D-07).
//!
//! `setup install --artifact-dir <folder>` looks in the folder for the file
//! name the reviewed catalogue gives the artifact (the last segment of its
//! reviewed URL) and applies the same verification as a download: the exact
//! reviewed size and SHA-256, streamed into a fresh private stage that is
//! discarded on any failure. The user supplies only the folder; no URL,
//! checksum or file name is ever read from the user, so the trust anchor
//! stays the compiled catalogue.

use std::{fmt, io, path::Path};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use tokio::io::AsyncReadExt;
use vsift_application::ProgressSink;
use vsift_domain::{ArtifactIntegrity, ProgressStage, ProgressUpdate};

use crate::{
    ManagedArtifactError, ManagedArtifactStore, ProcessCancellation, StagedManagedArtifact,
    publisher_artifact_transfer::PROGRESS_STEP_BYTES,
};

/// Bytes read from the user's file per step.
const IMPORT_CHUNK_BYTES: usize = 1024 * 1024;

/// Why an offline artifact could not be staged.
#[derive(Debug)]
pub enum ArtifactImportError {
    /// The folder or the catalogue's file name in it does not exist.
    Missing,
    /// The name is a link, a folder or anything but a regular file.
    NotRegularFile,
    /// The file's size differs from the reviewed size.
    SizeMismatch,
    /// The file could not be read.
    Unreadable,
    /// The caller cancelled before completion.
    Cancelled,
    /// Owned staging or the complete size/SHA-256 check failed.
    Staging(ManagedArtifactError),
}

impl fmt::Display for ArtifactImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "the reviewed artifact file is not in the artifact folder",
            Self::NotRegularFile => "the reviewed artifact name is not a regular file",
            Self::SizeMismatch => "the artifact file's size differs from the reviewed size",
            Self::Unreadable => "the artifact file could not be read",
            Self::Cancelled => "the artifact import was cancelled",
            Self::Staging(_) => "the artifact bytes could not be verified in private staging",
        })
    }
}

impl std::error::Error for ArtifactImportError {}

/// Imports `file_name` from `directory` into a private, unactivated stage,
/// verifying the reviewed `integrity` exactly as a download does.
///
/// Links are not followed: the name must be a regular file in the folder.
/// `progress` receives a `fetching_artifact` observation about every
/// mebibyte.
///
/// # Errors
///
/// A typed failure; the stage is discarded unless its ownership cannot be
/// proved.
pub async fn import_reviewed_artifact(
    store: &ManagedArtifactStore,
    directory: &Path,
    file_name: &str,
    integrity: ArtifactIntegrity,
    cancellation: &ProcessCancellation,
    progress: &dyn ProgressSink,
) -> Result<StagedManagedArtifact, ArtifactImportError> {
    if cancellation.is_cancelled() {
        return Err(ArtifactImportError::Cancelled);
    }
    if !directory.is_absolute() || !flat_file_name(file_name) {
        return Err(ArtifactImportError::Missing);
    }
    let folder =
        Dir::open_ambient_dir(directory, cap_std::ambient_authority()).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                ArtifactImportError::Missing
            } else {
                ArtifactImportError::Unreadable
            }
        })?;
    match folder.symlink_metadata(file_name) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(ArtifactImportError::NotRegularFile),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ArtifactImportError::Missing);
        }
        Err(_) => return Err(ArtifactImportError::Unreadable),
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = folder
        .open_with(file_name, &options)
        .map_err(|_| ArtifactImportError::NotRegularFile)?;
    let metadata = file
        .metadata()
        .map_err(|_| ArtifactImportError::Unreadable)?;
    if !metadata.is_file() {
        return Err(ArtifactImportError::NotRegularFile);
    }
    if metadata.len() != integrity.bytes() {
        return Err(ArtifactImportError::SizeMismatch);
    }
    let mut source = tokio::fs::File::from_std(file.into_std());
    let mut staging = store
        .begin_stream(integrity)
        .map_err(ArtifactImportError::Staging)?;
    let mut buffer = vec![0_u8; IMPORT_CHUNK_BYTES];
    let mut received = 0_u64;
    let mut reported = 0_u64;
    report(progress, 0, integrity.bytes());
    loop {
        let read = tokio::select! {
            () = cancellation.wait_cancelled() => Err(ArtifactImportError::Cancelled),
            result = source.read(&mut buffer) => result.map_err(|_| ArtifactImportError::Unreadable),
        };
        let read = match read {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) => {
                staging.abort().map_err(ArtifactImportError::Staging)?;
                return Err(error);
            }
        };
        let Some(chunk) = buffer.get(..read) else {
            staging.abort().map_err(ArtifactImportError::Staging)?;
            return Err(ArtifactImportError::Unreadable);
        };
        if let Err(error) = staging.append(chunk).await {
            staging.abort().map_err(ArtifactImportError::Staging)?;
            return Err(ArtifactImportError::Staging(error));
        }
        received = received.saturating_add(read as u64);
        if received.saturating_sub(reported) >= PROGRESS_STEP_BYTES {
            reported = received;
            report(progress, received, integrity.bytes());
        }
    }
    if let Err(error) = staging.finish().await {
        staging.abort().map_err(ArtifactImportError::Staging)?;
        return Err(ArtifactImportError::Staging(error));
    }
    if cancellation.is_cancelled() {
        staging.abort().map_err(ArtifactImportError::Staging)?;
        return Err(ArtifactImportError::Cancelled);
    }
    report(progress, received, integrity.bytes());
    Ok(staging.complete())
}

fn report(progress: &dyn ProgressSink, completed: u64, total: u64) {
    progress.report(ProgressUpdate {
        stage: ProgressStage::FetchingArtifact,
        completed: completed.min(total),
        total: Some(total),
    });
}

fn flat_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}
