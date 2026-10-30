//! Finding retained bundles and reading what a search needs of them (ADR 0035).
//!
//! Finding looks at folder listings and the presence of `bundle.json`, and
//! never reads a manifest: which bundles are opened is decided before any is.
//! Opening runs `bundle validate`'s folder and manifest checks. Reading takes
//! only a bundle's newest transcript, screen-text and visual records, each
//! checked by size and digest against the manifest and decoded by the session
//! readers. The source, frames, audio and evidence records are not read or
//! hashed, because `bundle validate` hashes up to 20 GiB and a search reads
//! every bundle on every page.

use std::{
    fs,
    path::{Path, PathBuf},
};

use cap_std::fs::Dir;
use vsift_application::{
    BundleIdentity, BundleSearchError, BundleUnreadable, FoundBundle, FoundBundles,
    MAX_BUNDLE_SEARCH_ENTRIES, MAX_BUNDLE_SEARCH_PATHS, OpenedBundle, SessionStorageError,
};
use vsift_domain::{ScreenTextIndex, SessionId, SourceId, TranscriptRevision, VisualIndex};

use super::{
    FilesystemSessionStore, StoredArtifact, StoredArtifactKind,
    bundle::{ReadManifest, open_selected_bundle, read_bundle_manifest},
    reads::{
        read_screen_text_index_artifact, read_transcript_artifact, read_visual_index_artifact,
    },
    sha256_hex,
    stored::validate_artifact_record,
};

/// A bundle's manifest file, whose presence is what makes a folder a bundle.
const BUNDLE_MANIFEST: &str = "bundle.json";

/// A bundle opened for a search: its folder and what its manifest declares.
#[derive(Debug)]
pub struct SearchableBundle {
    directory: Dir,
    identity: BundleIdentity,
    source_id: SourceId,
    session_id: SessionId,
    transcript: Option<StoredArtifact>,
    screen_text: Option<StoredArtifact>,
    visual: Option<StoredArtifact>,
}

impl OpenedBundle for SearchableBundle {
    fn identity(&self) -> &BundleIdentity {
        &self.identity
    }

    fn record_bytes(&self) -> u64 {
        [&self.transcript, &self.screen_text, &self.visual]
            .into_iter()
            .flatten()
            .map(|artifact| artifact.bytes)
            .fold(0, u64::saturating_add)
    }

    fn read_transcript(&self) -> Result<Option<TranscriptRevision>, BundleUnreadable> {
        self.transcript
            .as_ref()
            .map(|artifact| read_transcript_artifact(&self.directory, artifact, &self.source_id))
            .transpose()
            .map_err(|error| BundleUnreadable::of(&error))
    }

    fn read_screen_text(&self) -> Result<Option<ScreenTextIndex>, BundleUnreadable> {
        self.screen_text
            .as_ref()
            .map(|artifact| {
                read_screen_text_index_artifact(
                    &self.directory,
                    artifact,
                    &self.session_id,
                    &self.source_id,
                )
            })
            .transpose()
            .map_err(|error| BundleUnreadable::of(&error))
    }

    fn read_visual(&self) -> Result<Option<VisualIndex>, BundleUnreadable> {
        self.visual
            .as_ref()
            .map(|artifact| {
                read_visual_index_artifact(
                    &self.directory,
                    artifact,
                    &self.session_id,
                    &self.source_id,
                )
            })
            .transpose()
            .map_err(|error| BundleUnreadable::of(&error))
    }
}

/// A bundle found, with the path it is ordered by.
struct Located {
    name: String,
    /// The real path: two names for one folder are one bundle, and it breaks
    /// the tie between two folders with the same name.
    canonical: PathBuf,
    path: PathBuf,
}

impl FilesystemSessionStore {
    /// Finds the bundles `paths` hold, without reading any of them.
    ///
    /// A path is a bundle (a folder with a `bundle.json`) or a folder of
    /// bundles: its immediate subfolders with one, one level and never
    /// recursive. A link is never followed. The bundles come back by folder
    /// name and then whole path, the same folder once, so the order depends on
    /// nothing but what is there.
    ///
    /// # Errors
    ///
    /// [`BundleSearchError::TooManyPaths`] over [`MAX_BUNDLE_SEARCH_PATHS`];
    /// [`BundleSearchError::TooManyEntries`] when the paths hold more than
    /// [`MAX_BUNDLE_SEARCH_ENTRIES`] entries in all; and
    /// [`BundleSearchError::NoBundles`] for a path that holds no bundle, because
    /// a wrong folder must never read as "nobody said it".
    pub fn find_bundles(paths: &[PathBuf]) -> Result<FoundBundles<PathBuf>, BundleSearchError> {
        if paths.len() > MAX_BUNDLE_SEARCH_PATHS {
            return Err(BundleSearchError::TooManyPaths);
        }
        let mut located = Vec::new();
        let mut entries = 0_usize;
        let mut skipped_entries = 0_usize;
        for path in paths {
            let metadata = fs::symlink_metadata(path).map_err(|_| BundleSearchError::NoBundles)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(BundleSearchError::NoBundles);
            }
            if holds_manifest(path) {
                located.push(locate(path)?);
                continue;
            }
            let before = located.len();
            let listing = fs::read_dir(path).map_err(|_| BundleSearchError::NoBundles)?;
            for entry in listing {
                entries = entries.saturating_add(1);
                if entries > MAX_BUNDLE_SEARCH_ENTRIES {
                    return Err(BundleSearchError::TooManyEntries);
                }
                let Ok(entry) = entry else {
                    continue;
                };
                // `DirEntry::file_type` does not follow a link.
                if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let child = entry.path();
                if holds_manifest(&child) {
                    located.push(locate(&child)?);
                } else {
                    skipped_entries = skipped_entries.saturating_add(1);
                }
            }
            if located.len() == before {
                return Err(BundleSearchError::NoBundles);
            }
        }
        located.sort_by(|left, right| {
            (&left.name, &left.canonical).cmp(&(&right.name, &right.canonical))
        });
        located.dedup_by(|later, earlier| later.canonical == earlier.canonical);
        Ok(FoundBundles {
            bundles: located
                .into_iter()
                .map(|bundle| FoundBundle {
                    name: bundle.name,
                    location: bundle.path,
                })
                .collect(),
            skipped_entries,
        })
    }

    /// Opens one bundle for a search: `bundle validate`'s folder and manifest
    /// checks, and which records are its newest.
    ///
    /// # Errors
    ///
    /// The reason it cannot be searched. A bundle that holds a transcript part
    /// is damage: a bundle is retained only when its revisions are sealed.
    pub fn open_bundle_for_search(path: &Path) -> Result<SearchableBundle, BundleUnreadable> {
        let opened = open_for_search(path).map_err(|error| BundleUnreadable::of(&error))?;
        Ok(opened)
    }
}

fn open_for_search(path: &Path) -> Result<SearchableBundle, SessionStorageError> {
    let directory = open_selected_bundle(path)?;
    let ReadManifest {
        manifest,
        bytes,
        session_id,
        source_id,
    } = read_bundle_manifest(&directory)?;
    if manifest
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == StoredArtifactKind::TranscriptPart)
    {
        return Err(SessionStorageError::IntegrityFailure);
    }
    // `retain` copies the session's list in order, so the last of a kind is the
    // newest, the one a read of the session finds.
    let newest = |kind: StoredArtifactKind| -> Result<Option<StoredArtifact>, SessionStorageError> {
        let artifact = manifest
            .artifacts
            .iter()
            .rev()
            .find(|artifact| artifact.kind == kind)
            .cloned();
        if let Some(artifact) = &artifact {
            validate_artifact_record(artifact)?;
        }
        Ok(artifact)
    };
    let transcript = newest(StoredArtifactKind::TranscriptRecord)?;
    let screen_text = newest(StoredArtifactKind::ScreenTextIndexRecord)?;
    let visual = newest(StoredArtifactKind::VisualIndexRecord)?;
    Ok(SearchableBundle {
        directory,
        identity: BundleIdentity {
            session_id: session_id.clone(),
            source_id: source_id.clone(),
            source_included: manifest.source_included,
            manifest_sha256: sha256_hex(&bytes),
        },
        source_id,
        session_id,
        transcript,
        screen_text,
        visual,
    })
}

/// Whether `folder` holds a `bundle.json` that is a file and no link.
fn holds_manifest(folder: &Path) -> bool {
    fs::symlink_metadata(folder.join(BUNDLE_MANIFEST)).is_ok_and(|entry| entry.is_file())
}

fn locate(path: &Path) -> Result<Located, BundleSearchError> {
    let canonical = fs::canonicalize(path).map_err(|_| BundleSearchError::NoBundles)?;
    let name = canonical
        .file_name()
        .ok_or(BundleSearchError::NoBundles)?
        .to_string_lossy()
        .into_owned();
    Ok(Located {
        name,
        path: canonical.clone(),
        canonical,
    })
}
