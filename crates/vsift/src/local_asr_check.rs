//! The local-ASR part of `setup check` (maintainer decision D4, P07
//! increment 3c).
//!
//! `setup check` reports whether the registered model is a reviewed pinned
//! profile and whether local speech recognition passed its functional
//! verification (ADR 0017). A still-valid recorded pass is reported without
//! running anything. Otherwise the check runs the same media-tool and
//! local-ASR preflights a retranscription runs, with the same tools, model and
//! fingerprints, under its own time budget, separate from the executable
//! probe timeout. It writes only the per-user verification record, and only
//! for a pass. Nothing here fails the check: an unusable setup is a typed
//! result, and only configuration or built-in policy problems are errors.

use std::time::Duration;

use vsift_application::{
    AsrFailure, AsrStage, LocalAsrCheckFailure, LocalAsrCheckOutcome, LocalAsrModelStatus,
    LocalAsrNotRunReason, LocalAsrSetupStatus, LocalAsrVerificationFailure,
    LocalAsrVerificationSource, MediaToolPreflightOutcome, RecognizerIdentity,
    SpeechRecognitionError,
};
use vsift_domain::RuntimeDependency;
use vsift_infrastructure::{
    ExecutableResolver, MediaProviderConformance, ProcessCancellation, identify_whisper_model_file,
    run_within_budget,
};

use crate::{
    asr::{SelectedRecognizer, resolve_whisper},
    engine::Engine,
    error::EngineError,
    setup::ExecutableSelections,
    transcripts::resolve_tool,
};

/// Time `setup check` allows the local-ASR verification when no pass is
/// recorded (maintainer decision D4). The reviewed fixture takes a few seconds
/// on the reference machine once the model is cached by the operating system.
pub const DEFAULT_LOCAL_ASR_CHECK_BUDGET: Duration = Duration::from_secs(60);

impl Engine {
    /// Reports the registered model and the local-ASR verification for the
    /// executables `setup check` selected (per call, configured, then `PATH`).
    ///
    /// With a host-supplied recognizer the model is the one it reports and no
    /// whisper.cpp is resolved, exactly as a retranscription would.
    pub(crate) async fn check_local_asr(
        &self,
        selections: &ExecutableSelections,
        budget: Duration,
    ) -> Result<LocalAsrSetupStatus, EngineError> {
        if let Some(host) = self.host_asr() {
            let identity = host.identity().await;
            let model = match &identity {
                Ok(identity) => identity
                    .model
                    .profile()
                    .reviewed()
                    .map_or(LocalAsrModelStatus::Unrecognised, |profile| {
                        LocalAsrModelStatus::KnownPinned(profile)
                    }),
                Err(_) => LocalAsrModelStatus::Unreadable,
            };
            let verification = match (media_tools(selections)?, identity) {
                (None, _) => {
                    LocalAsrCheckOutcome::NotRun(LocalAsrNotRunReason::MediaToolsUnavailable)
                }
                (Some(_), Err(_)) => {
                    LocalAsrCheckOutcome::NotRun(LocalAsrNotRunReason::ModelNotPinned)
                }
                (Some(tools), Ok(identity)) => {
                    if model.profile().is_none() {
                        LocalAsrCheckOutcome::NotRun(LocalAsrNotRunReason::ModelNotPinned)
                    } else {
                        self.verify_local_asr_within(
                            &tools,
                            &SelectedRecognizer::Host(host),
                            &identity,
                            budget,
                        )
                        .await?
                    }
                }
            };
            return Ok(LocalAsrSetupStatus {
                model,
                verification,
            });
        }

        let registered = self.user_configuration()?.read_model()?;
        let model = match &registered {
            None => LocalAsrModelStatus::NotSelected,
            Some(path) => identify_whisper_model_file(path)
                .map_err(|_| EngineError::ReviewedPolicyInvalid)?
                .into(),
        };
        let verification = self
            .whisper_local_asr_outcome(selections, registered.as_deref(), model, budget)
            .await?;
        Ok(LocalAsrSetupStatus {
            model,
            verification,
        })
    }

    /// The verification outcome for whisper.cpp with the registered model.
    async fn whisper_local_asr_outcome(
        &self,
        selections: &ExecutableSelections,
        registered: Option<&std::path::Path>,
        model: LocalAsrModelStatus,
        budget: Duration,
    ) -> Result<LocalAsrCheckOutcome, EngineError> {
        let not_run = |reason| Ok(LocalAsrCheckOutcome::NotRun(reason));
        let Some(tools) = media_tools(selections)? else {
            return not_run(LocalAsrNotRunReason::MediaToolsUnavailable);
        };
        let executable = match resolve_whisper(selections.whisper.clone()) {
            Ok(executable) => executable,
            Err(EngineError::LocalAsrToolUnavailable(_) | EngineError::Executable(_)) => {
                return not_run(LocalAsrNotRunReason::WhisperUnavailable);
            }
            Err(other) => return Err(other),
        };
        let Some(path) = registered else {
            return not_run(LocalAsrNotRunReason::ModelNotSelected);
        };
        if model.profile().is_none() {
            return not_run(LocalAsrNotRunReason::ModelNotPinned);
        }
        let Ok(cli) = self.whisper_recognizer(executable, path) else {
            return not_run(LocalAsrNotRunReason::ModelNotPinned);
        };
        let identity = match cli.recognizer_identity().await {
            Ok(identity) => identity,
            Err(SpeechRecognitionError::ModelUnavailable) => {
                return not_run(LocalAsrNotRunReason::ModelNotPinned);
            }
            Err(other) => {
                return Ok(LocalAsrCheckOutcome::Failed(
                    LocalAsrCheckFailure::Verification(LocalAsrVerificationFailure::Transcription(
                        AsrFailure {
                            stage: AsrStage::RecognizerIdentity,
                            reason: other.into(),
                        },
                    )),
                ));
            }
        };
        // The file may have changed since it was identified above.
        if identity.model.profile().reviewed().is_none() {
            return not_run(LocalAsrNotRunReason::ModelNotPinned);
        }
        self.verify_local_asr_within(&tools, &SelectedRecognizer::Whisper(cli), &identity, budget)
            .await
    }

    /// A recorded pass, or the media-tool and local-ASR preflights run now
    /// within `budget`.
    async fn verify_local_asr_within(
        &self,
        tools: &MediaProviderConformance,
        recognizer: &SelectedRecognizer<'_>,
        identity: &RecognizerIdentity,
        budget: Duration,
    ) -> Result<LocalAsrCheckOutcome, EngineError> {
        let preflight = match self.prepare_local_asr_preflight(tools, recognizer, identity) {
            Ok(preflight) => preflight,
            Err(EngineError::MediaToolVerificationFailed(_)) => {
                return Ok(LocalAsrCheckOutcome::NotRun(
                    LocalAsrNotRunReason::MediaToolsUnavailable,
                ));
            }
            Err(other) => return Err(other),
        };
        if preflight.recorded() {
            return Ok(LocalAsrCheckOutcome::Verified(
                LocalAsrVerificationSource::Recorded,
            ));
        }
        let cancellation = ProcessCancellation::new();
        let work = async {
            match self
                .ensure_media_tools_verified_with(tools, &cancellation)
                .await
            {
                Ok(_) => {}
                Err(EngineError::MediaToolVerificationFailed(_)) => {
                    return Ok(LocalAsrCheckOutcome::NotRun(
                        LocalAsrNotRunReason::MediaToolsUnavailable,
                    ));
                }
                Err(other) => return Err(other),
            }
            Ok(
                match self
                    .run_local_asr_preflight(&preflight, tools, recognizer, identity, &cancellation)
                    .await
                {
                    Ok(MediaToolPreflightOutcome::AlreadyVerified) => {
                        LocalAsrCheckOutcome::Verified(LocalAsrVerificationSource::Recorded)
                    }
                    Ok(MediaToolPreflightOutcome::VerifiedNow(_)) => {
                        LocalAsrCheckOutcome::Verified(LocalAsrVerificationSource::RanNow)
                    }
                    Err(failure) => {
                        LocalAsrCheckOutcome::Failed(LocalAsrCheckFailure::Verification(failure))
                    }
                },
            )
        };
        run_within_budget(budget, &cancellation, work)
            .await
            .unwrap_or(Ok(LocalAsrCheckOutcome::Failed(
                LocalAsrCheckFailure::BudgetExceeded,
            )))
    }
}

/// `FFmpeg` and `FFprobe` as `setup check` selected them, or `None` when
/// either is missing or its selection is rejected.
fn media_tools(
    selections: &ExecutableSelections,
) -> Result<Option<MediaProviderConformance>, EngineError> {
    let resolver = ExecutableResolver::from_current_path();
    let resolve = |dependency| match resolve_tool(
        &resolver,
        selections.for_dependency(dependency).cloned(),
        dependency,
    ) {
        Ok(executable) => Ok(Some(executable)),
        Err(EngineError::MediaToolUnavailable(_) | EngineError::Executable(_)) => Ok(None),
        Err(other) => Err(other),
    };
    Ok(
        match (
            resolve(RuntimeDependency::Ffmpeg)?,
            resolve(RuntimeDependency::Ffprobe)?,
        ) {
            (Some(ffmpeg), Some(ffprobe)) => Some(MediaProviderConformance::r0(ffmpeg, ffprobe)),
            _ => None,
        },
    )
}
