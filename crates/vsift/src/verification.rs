//! Functional verification of selected tools, the automatic media-tool
//! preflight, and identification of a registered speech model.
//!
//! [`Engine::verify_media_tools`] and [`Engine::identify_model`] are exposed for
//! hosts; no CLI command calls them. The preflight runs inside operations that
//! use `FFmpeg`/`FFprobe` on user media, before their first media stage.

use std::path::PathBuf;

use vsift_application::{
    MediaToolCheck, MediaToolFailure, MediaToolPreflightFailure, MediaToolPreflightOutcome,
    MediaToolVerification, MediaToolVerifier, ModelVerification, preflight_media_tools,
};
use vsift_domain::RuntimeDependency;
use vsift_infrastructure::{
    FixtureMediaToolVerifier, MediaProviderConformance, MediaToolVerificationAuthority,
    ProcessCancellation, TrustedExecutable, identify_whisper_model_file, media_tool_fingerprint,
    reviewed_compatibility_policy,
};

use crate::{
    engine::Engine,
    error::{EngineError, ExecutableRejection},
};

/// A clonable cancellation signal for a long-running engine operation.
///
/// Every clone observes the same signal; cancelling any clone stops the
/// operation at its next provider boundary.
#[derive(Clone, Debug)]
pub struct Cancellation(pub(crate) ProcessCancellation);

impl Cancellation {
    /// Creates an uncancelled signal.
    #[must_use]
    pub fn new() -> Self {
        Self(ProcessCancellation::new())
    }

    /// Requests cancellation. Repeated requests are idempotent.
    pub fn cancel(&self) {
        self.0.cancel();
    }

    /// Reports whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

impl Default for Cancellation {
    fn default() -> Self {
        Self::new()
    }
}

/// Which `FFmpeg` and `FFprobe` to verify.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaToolSelection {
    /// The executables saved in the user's dependency configuration.
    Configured,
    /// Explicit absolute executable paths.
    Explicit {
        /// `FFmpeg` executable.
        ffmpeg: PathBuf,
        /// `FFprobe` executable.
        ffprobe: PathBuf,
    },
}

/// A request to prove the selected media tools work on this machine.
#[derive(Clone, Debug)]
pub struct MediaToolVerificationRequest {
    /// Executables to verify.
    pub tools: MediaToolSelection,
    /// Existing host-controlled directory in which one uniquely named
    /// workspace is created, used and removed. Nothing else in it is touched.
    pub workspace_parent: PathBuf,
    /// Signal that stops verification early.
    pub cancellation: Cancellation,
}

/// Which model file to identify.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelSelection {
    /// The model saved in the user's dependency configuration.
    Configured,
    /// An explicit model file.
    Explicit(PathBuf),
}

impl Engine {
    /// Runs the embedded reviewed fixture through the real probe, frame and
    /// audio steps with the selected tools and checks each result.
    ///
    /// A failed check is a result ([`MediaToolVerification::Failed`]), not an
    /// error.
    ///
    /// # Errors
    ///
    /// Fails before running anything when the configuration cannot be read, a
    /// configured tool is not selected, an executable is rejected, or the
    /// built-in compatibility policy is invalid.
    pub async fn verify_media_tools(
        &self,
        request: MediaToolVerificationRequest,
    ) -> Result<MediaToolVerification, EngineError> {
        let (ffmpeg, ffprobe) = match request.tools {
            MediaToolSelection::Explicit { ffmpeg, ffprobe } => (ffmpeg, ffprobe),
            MediaToolSelection::Configured => {
                let configured = self.user_configuration()?.read()?;
                (
                    configured.ffmpeg.ok_or(EngineError::DependencyNotSelected(
                        RuntimeDependency::Ffmpeg,
                    ))?,
                    configured
                        .ffprobe
                        .ok_or(EngineError::DependencyNotSelected(
                            RuntimeDependency::Ffprobe,
                        ))?,
                )
            }
        };
        let ffmpeg = trusted(&ffmpeg)?;
        let ffprobe = trusted(&ffprobe)?;
        let policy =
            reviewed_compatibility_policy().map_err(|_| EngineError::ReviewedPolicyInvalid)?;
        let verifier = FixtureMediaToolVerifier::new(
            MediaProviderConformance::r0(ffmpeg, ffprobe),
            self.config().host_isolation.into_infrastructure(),
            request.workspace_parent,
            policy,
            request.cancellation.0,
        );
        Ok(verifier.verify().await)
    }

    /// Identifies a model file against the reviewed pinned whisper.cpp
    /// models, reporting which profile it is.
    ///
    /// Only identity (exact size and SHA-256) is checked. This reads and hashes
    /// the whole file when its size matches, so hosts with a responsive thread
    /// should call it off that thread.
    ///
    /// # Errors
    ///
    /// Fails when the configuration cannot be read, no model is configured, or
    /// the built-in pin is invalid. An unreadable or unrecognised file is a
    /// result, not an error.
    pub fn identify_model(
        &self,
        selection: ModelSelection,
    ) -> Result<ModelVerification, EngineError> {
        let path = match selection {
            ModelSelection::Explicit(path) => path,
            ModelSelection::Configured => self
                .user_configuration()?
                .read_model()?
                .ok_or(EngineError::ModelNotSelected)?,
        };
        identify_whisper_model_file(&path).map_err(|_| EngineError::ReviewedPolicyInvalid)
    }

    /// Ensures the resolved media tools are verified before an operation's
    /// first media stage touches user media or the session root.
    ///
    /// A still-valid recorded pass for the same tool identity skips
    /// verification; otherwise the reviewed fixture (or a host-supplied
    /// verifier) runs in a private workspace under the per-user state
    /// directory. Recording problems never fail the operation; they only mean
    /// the next preflight verifies again. Each preflight first removes stale
    /// workspaces left there by killed verifications.
    pub(crate) async fn ensure_media_tools_verified(
        &self,
        tools: &MediaProviderConformance,
    ) -> Result<MediaToolPreflightOutcome, EngineError> {
        let policy =
            reviewed_compatibility_policy().map_err(|_| EngineError::ReviewedPolicyInvalid)?;
        let now = self.now_unix_seconds()?;
        let isolation = self.config().host_isolation.into_infrastructure();
        // Without a private place to run, verification cannot run at all.
        let state = self
            .user_configuration()?
            .media_tool_verification_state()
            .map_err(|_| {
                EngineError::MediaToolVerificationFailed(MediaToolPreflightFailure {
                    check: MediaToolCheck::Preparation,
                    failure: MediaToolFailure::Workspace,
                })
            })?;
        // Reclaims workspaces of verifications that were killed before they
        // could remove them (issue #132). It never waits and never fails the
        // operation; a skipped sweep is simply retried by the next preflight.
        let _ = state.remove_stale_workspaces(now);
        let outcome = if let Some(verifier) = self.host_media_tool_verifier() {
            let fingerprint = media_tool_fingerprint(
                tools,
                isolation,
                &policy,
                MediaToolVerificationAuthority::HostSupplied,
            );
            preflight_media_tools(&verifier, &state, fingerprint.as_ref(), now).await
        } else {
            let fingerprint = media_tool_fingerprint(
                tools,
                isolation,
                &policy,
                MediaToolVerificationAuthority::ReviewedFixture,
            );
            let verifier = FixtureMediaToolVerifier::new(
                tools.clone(),
                isolation,
                state.workspace_parent().to_path_buf(),
                policy,
                ProcessCancellation::new(),
            );
            preflight_media_tools(&verifier, &state, fingerprint.as_ref(), now).await
        };
        outcome.map_err(EngineError::MediaToolVerificationFailed)
    }
}

fn trusted(path: &std::path::Path) -> Result<TrustedExecutable, EngineError> {
    TrustedExecutable::explicit(path)
        .map_err(|error| EngineError::Executable(ExecutableRejection::from(&error)))
}
