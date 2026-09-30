//! The managed tier of dependency lookup (P13, ADR 0007 and ADR 0023 §3
//! step 4).
//!
//! Every command that runs a tool resolves it in one order: a path given for
//! this call (only `setup check` takes one), the path the user configured,
//! the version `setup install` selected in the private managed root, then a
//! search of the filtered `PATH`. The model follows the same order without
//! the `PATH` tier.
//!
//! A managed version is opened through the store: its selection pointer must
//! name the exact manifest SHA-256, and every file's size and SHA-256 must
//! match that manifest before anything runs, so a managed tool is identified
//! by digest rather than by path (known limit L-006). A version that cannot
//! be opened that way is never run; lookup falls through to `PATH` as though
//! nothing were installed. The opened version is held by the executable
//! resolved from it (see [`ManagedRuntimeHold`]), so its shared use lock
//! stays held for as long as a job keeps the executable.
//!
//! Opening a version hashes all of its files, so one operation resolves
//! through one [`ManagedLookup`], which opens each component at most once
//! (`FFmpeg` and `FFprobe` share the media-tools version).

use std::path::PathBuf;

use vsift_domain::{ManagedComponent, RuntimeDependency};
use vsift_infrastructure::{
    ManagedArtifactStore, ManagedRuntimeHold, ManagedRuntimeRole, TrustedExecutable,
    managed_executable_name,
};

use crate::engine::{Engine, ManagedRootLocation};

/// The managed component that supplies `dependency`, and its role in it.
const fn managed_source(dependency: RuntimeDependency) -> (ManagedComponent, ManagedRuntimeRole) {
    match dependency {
        RuntimeDependency::Ffmpeg => (ManagedComponent::MediaTools, ManagedRuntimeRole::Ffmpeg),
        RuntimeDependency::Ffprobe => (ManagedComponent::MediaTools, ManagedRuntimeRole::Ffprobe),
        RuntimeDependency::Whisper => {
            (ManagedComponent::WhisperCli, ManagedRuntimeRole::WhisperCli)
        }
    }
}

/// A managed model: its file and the hold that keeps its version in use.
#[derive(Clone, Debug)]
pub(crate) struct ManagedModel {
    /// The model file inside the held version.
    pub(crate) path: PathBuf,
    /// Keeps the version in use while the model may be read.
    pub(crate) hold: ManagedRuntimeHold,
}

/// The managed tier for one operation: each component's selected version is
/// opened, verified and held at most once, on first use.
pub(crate) struct ManagedLookup<'engine> {
    engine: &'engine Engine,
    opened: [Opened; 3],
}

/// What one lookup found for one component.
#[derive(Clone)]
enum Opened {
    /// Not asked for yet.
    NotYet,
    /// No verified version is selected.
    Absent,
    /// The selected version, verified and held.
    Held(ManagedRuntimeHold),
}

const fn slot(component: ManagedComponent) -> usize {
    match component {
        ManagedComponent::MediaTools => 0,
        ManagedComponent::WhisperCli => 1,
        ManagedComponent::WhisperModel => 2,
    }
}

impl ManagedLookup<'_> {
    fn hold(&mut self, component: ManagedComponent) -> Option<ManagedRuntimeHold> {
        let opened = &mut self.opened[slot(component)];
        if matches!(opened, Opened::NotYet) {
            *opened = self
                .engine
                .open_managed(component)
                .map_or(Opened::Absent, Opened::Held);
        }
        match opened {
            Opened::Held(hold) => Some(hold.clone()),
            Opened::NotYet | Opened::Absent => None,
        }
    }

    /// The managed executable for `dependency`, carrying its version's hold.
    pub(crate) fn executable(
        &mut self,
        dependency: RuntimeDependency,
    ) -> Option<TrustedExecutable> {
        let (component, role) = managed_source(dependency);
        let hold = self.hold(component)?;
        TrustedExecutable::managed_in(&hold, &managed_executable_name(role)?).ok()
    }

    /// The version selected for `component` now, when it is verified.
    pub(crate) fn selected_version(&mut self, component: ManagedComponent) -> Option<String> {
        self.hold(component)
            .map(|hold| hold.runtime().identity().version().to_owned())
    }

    /// The managed model file, with its version's hold.
    pub(crate) fn model(&mut self) -> Option<ManagedModel> {
        let hold = self.hold(ManagedComponent::WhisperModel)?;
        let path = {
            let runtime = hold.runtime();
            let names = runtime.reviewed_names();
            let [name] = names.as_slice() else {
                return None;
            };
            runtime.file_path(name)?
        };
        Some(ManagedModel { path, hold })
    }
}

impl Engine {
    /// The managed root this engine uses, or `None` when the platform has
    /// no per-user location for it (nothing is managed there).
    pub(crate) fn managed_store(&self) -> Option<ManagedArtifactStore> {
        match &self.config().managed_root {
            ManagedRootLocation::PlatformDefault => ManagedArtifactStore::default_location().ok(),
            ManagedRootLocation::Explicit(path) => ManagedArtifactStore::at(path.clone()).ok(),
        }
    }

    /// A lookup of the managed tier for one operation.
    pub(crate) const fn managed_lookup(&self) -> ManagedLookup<'_> {
        ManagedLookup {
            engine: self,
            opened: [Opened::NotYet, Opened::NotYet, Opened::NotYet],
        }
    }

    /// The selected version of `component`, verified and held, or `None`
    /// when none is selected or it cannot be verified.
    fn open_managed(&self, component: ManagedComponent) -> Option<ManagedRuntimeHold> {
        self.managed_store()?
            .open_selected_runtime(component.identifier())
            .ok()
            .flatten()
            .map(ManagedRuntimeHold::new)
    }
}
