//! Setup operations: dependency checks, the read-only managed plan and its
//! acceptance, and bring-your-own executable and model registration.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use vsift_application::{
    AcceptedManagedCatalogue, DependencyProbe, DiagnoseRuntime, ManagedSetupPlan, RuntimeDiagnosis,
    SetupProfile, SetupSelectionState, plan_managed_setup,
};
use vsift_contract::{DependencyLookup, SavedSetupPlan, SetupPlanResponse};
use vsift_domain::{ManagedTarget, RuntimeDependency};
use vsift_infrastructure::{
    ExplicitProbePaths, ProcessDependencyProbe, accepted_ubuntu_catalogue, detect_managed_target,
};

use crate::{engine::Engine, error::EngineError};

/// Executable paths selected for each runtime dependency.
///
/// An absent entry means "not selected here": the engine falls back to the
/// user's configured selection and then to a filtered `PATH` search.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExecutableSelections {
    /// Absolute path to an `FFmpeg` executable.
    pub ffmpeg: Option<PathBuf>,
    /// Absolute path to an `FFprobe` executable.
    pub ffprobe: Option<PathBuf>,
    /// Absolute path to a whisper.cpp CLI executable.
    pub whisper: Option<PathBuf>,
}

impl ExecutableSelections {
    /// Returns the selection for one dependency.
    #[must_use]
    pub fn for_dependency(&self, dependency: RuntimeDependency) -> Option<&PathBuf> {
        match dependency {
            RuntimeDependency::Ffmpeg => self.ffmpeg.as_ref(),
            RuntimeDependency::Ffprobe => self.ffprobe.as_ref(),
            RuntimeDependency::Whisper => self.whisper.as_ref(),
        }
    }

    fn from_configured(configured: ExplicitProbePaths) -> Self {
        Self {
            ffmpeg: configured.ffmpeg,
            ffprobe: configured.ffprobe,
            whisper: configured.whisper,
        }
    }

    fn into_probe_paths(self) -> ExplicitProbePaths {
        ExplicitProbePaths {
            ffmpeg: self.ffmpeg,
            ffprobe: self.ffprobe,
            whisper: self.whisper,
        }
    }
}

/// A read-only dependency check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupCheckRequest {
    /// Total deadline shared by every dependency probe in the check.
    pub probe_timeout: Duration,
    /// Selections for this call only; they take precedence over configured ones.
    pub per_call: ExecutableSelections,
}

/// Result of a dependency check: what responded, and where each probed
/// executable came from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupCheckReport {
    diagnosis: RuntimeDiagnosis,
    /// Whether a per-call path was supplied, indexed by `dependency_index`.
    per_call: [bool; 3],
    /// Whether any path (per-call or configured) was selected.
    selected: [bool; 3],
}

impl SetupCheckReport {
    /// Probe results and aggregate readiness.
    #[must_use]
    pub const fn diagnosis(&self) -> &RuntimeDiagnosis {
        &self.diagnosis
    }

    /// Where the executable probed for `dependency` came from: a per-call path
    /// wins over a configured user path, which wins over the filtered `PATH`.
    #[must_use]
    pub const fn lookup(&self, dependency: RuntimeDependency) -> DependencyLookup {
        let index = dependency_index(dependency);
        if self.per_call[index] {
            DependencyLookup::ExplicitPath
        } else if self.selected[index] {
            DependencyLookup::ConfiguredUserPath
        } else {
            DependencyLookup::FilteredPath
        }
    }
}

const fn dependency_index(dependency: RuntimeDependency) -> usize {
    match dependency {
        RuntimeDependency::Ffmpeg => 0,
        RuntimeDependency::Ffprobe => 1,
        RuntimeDependency::Whisper => 2,
    }
}

fn presence(selections: &ExecutableSelections) -> [bool; 3] {
    RuntimeDependency::ALL.map(|dependency| selections.for_dependency(dependency).is_some())
}

/// A request for the current read-only managed setup plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SetupPlanRequest {
    /// Profile whose dependencies are planned.
    pub profile: SetupProfile,
    /// Total deadline shared by every dependency probe in the plan.
    pub probe_timeout: Duration,
}

/// The current plan: its authority, bound by digest, and its exact public
/// presentation.
///
/// Both halves come from the same observation, so acceptance compares the
/// plan the user reviewed with the plan that would run now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluatedSetupPlan {
    authority: ManagedSetupPlan,
    presentation: SetupPlanResponse,
}

impl EvaluatedSetupPlan {
    /// The reviewable plan as the v1 contract presents it.
    #[must_use]
    pub const fn presentation(&self) -> &SetupPlanResponse {
        &self.presentation
    }

    /// Consumes the plan, returning its public presentation.
    #[must_use]
    pub fn into_presentation(self) -> SetupPlanResponse {
        self.presentation
    }

    /// Requires the saved public plan, the current observations and the
    /// supplied digest to describe exactly the same authority.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::SavedPlanRejected`] when the saved plan differs
    /// from the current one, and [`EngineError::PlanAcceptance`] when the digest
    /// does not accept the current plan.
    pub fn validate_acceptance(
        &self,
        saved: &SavedSetupPlan,
        supplied_digest: &str,
    ) -> Result<(), EngineError> {
        saved
            .require_same_plan(&self.presentation)
            .map_err(EngineError::SavedPlanRejected)?;
        self.authority
            .validate_acceptance(supplied_digest)
            .map_err(EngineError::PlanAcceptance)
    }
}

impl Engine {
    /// Probes `FFmpeg`, `FFprobe` and whisper.cpp without changing the machine.
    ///
    /// Only executable responses are checked; provider compatibility and the
    /// local ASR model are not.
    ///
    /// # Errors
    ///
    /// Fails when the per-user configuration cannot be read. Missing or
    /// unhealthy dependencies are results, not errors.
    pub async fn check_setup(
        &self,
        request: SetupCheckRequest,
    ) -> Result<SetupCheckReport, EngineError> {
        let configured = ExecutableSelections::from_configured(self.user_configuration()?.read()?);
        let per_call = request.per_call;
        let selections = ExecutableSelections {
            ffmpeg: per_call.ffmpeg.clone().or(configured.ffmpeg),
            ffprobe: per_call.ffprobe.clone().or(configured.ffprobe),
            whisper: per_call.whisper.clone().or(configured.whisper),
        };
        let selected = presence(&selections);
        let probe = ProcessDependencyProbe::with_explicit_paths(
            request.probe_timeout,
            selections.into_probe_paths(),
        );
        Ok(SetupCheckReport {
            diagnosis: DiagnoseRuntime::new(probe).execute().await,
            per_call: presence(&per_call),
            selected,
        })
    }

    /// Builds the current read-only managed setup plan from fresh observations.
    ///
    /// # Errors
    ///
    /// Fails when the configuration, clock or built-in reviewed catalogue
    /// cannot be read.
    pub async fn plan_setup(
        &self,
        request: SetupPlanRequest,
    ) -> Result<EvaluatedSetupPlan, EngineError> {
        let store = self.user_configuration()?;
        let configured = store.read()?;
        let configured_model = store.read_model()?;
        let now_unix_seconds = self.now_unix_seconds()?;
        let catalogue =
            accepted_ubuntu_catalogue().map_err(|_| EngineError::ReviewedPolicyInvalid)?;
        let selections = SetupSelectionState {
            ffmpeg: configured
                .ffmpeg
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            ffprobe: configured
                .ffprobe
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            whisper: configured
                .whisper
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            model: configured_model
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        };
        let probe = ProcessDependencyProbe::with_explicit_paths(request.probe_timeout, configured);
        Ok(evaluate_plan(
            probe,
            request.profile,
            detect_managed_target(),
            selections,
            now_unix_seconds,
            Some(catalogue),
        )
        .await)
    }

    /// Saves one user-managed executable selection without running it.
    ///
    /// # Errors
    ///
    /// Fails when the path is not an absolute regular file or the per-user
    /// configuration cannot be written.
    pub fn configure_executable(
        &self,
        dependency: RuntimeDependency,
        executable: &Path,
    ) -> Result<(), EngineError> {
        self.user_configuration()?
            .configure(dependency, executable)
            .map_err(EngineError::from)
    }

    /// Saves one user-managed local ASR model path without reading its bytes.
    ///
    /// # Errors
    ///
    /// Fails when the path is not an absolute nonempty regular file or the
    /// per-user configuration cannot be written.
    pub fn configure_model(&self, file: &Path) -> Result<(), EngineError> {
        self.user_configuration()?
            .configure_model(file)
            .map_err(EngineError::from)
    }
}

/// Diagnoses current dependencies and builds plan authority plus its presentation.
async fn evaluate_plan<P: DependencyProbe>(
    probe: P,
    profile: SetupProfile,
    target: ManagedTarget,
    selections: SetupSelectionState,
    now_unix_seconds: u64,
    catalogue: Option<AcceptedManagedCatalogue>,
) -> EvaluatedSetupPlan {
    let plan = plan_managed_setup(
        profile,
        DiagnoseRuntime::new(probe).execute().await,
        target,
        selections,
        now_unix_seconds,
        catalogue,
    );
    let presentation = SetupPlanResponse::new(&plan);
    EvaluatedSetupPlan {
        authority: plan,
        presentation,
    }
}

#[cfg(test)]
mod tests {
    use std::future::ready;

    use vsift_application::{DependencyProbe, SetupProfile, SetupSelectionState};
    use vsift_contract::{OperationResponse, SavedSetupPlan};
    use vsift_domain::{
        DependencyState, DependencyStatus, FailureCode, ManagedTarget, RuntimeDependency,
    };
    use vsift_infrastructure::accepted_ubuntu_catalogue;

    use super::{EvaluatedSetupPlan, evaluate_plan};
    use crate::error::EngineError;

    struct FixedProbe {
        state: DependencyState,
    }

    struct MissingOneProbe {
        missing: RuntimeDependency,
    }

    impl DependencyProbe for FixedProbe {
        fn probe(
            &self,
            dependency: RuntimeDependency,
        ) -> impl Future<Output = DependencyStatus> + Send {
            ready(DependencyStatus {
                dependency,
                state: self.state.clone(),
            })
        }
    }

    impl DependencyProbe for MissingOneProbe {
        fn probe(
            &self,
            dependency: RuntimeDependency,
        ) -> impl Future<Output = DependencyStatus> + Send {
            let state = if dependency == self.missing {
                DependencyState::Missing
            } else {
                DependencyState::Available {
                    version: String::from("fixture 1"),
                }
            };
            ready(DependencyStatus { dependency, state })
        }
    }

    fn plan_json(
        plan: &EvaluatedSetupPlan,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let response = OperationResponse::complete("setup.plan", plan.presentation())?;
        Ok(serde_json::to_value(response)?)
    }

    fn setup_plan_schema() -> Result<serde_json::Value, serde_json::Error> {
        serde_json::from_str(include_str!("../../../schemas/v1/setup-plan.schema.json"))
    }

    #[tokio::test]
    async fn accepted_ubuntu_plan_is_reviewable_and_matches_public_schema()
    -> Result<(), Box<dyn std::error::Error>> {
        let plan = evaluate_plan(
            FixedProbe {
                state: DependencyState::Missing,
            },
            SetupProfile::Desktop,
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_800_000_000,
            Some(accepted_ubuntu_catalogue()?),
        )
        .await;
        let value = plan_json(&plan)?;
        jsonschema::validator_for(&setup_plan_schema()?)?
            .validate(&value)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let data = &value["data"];
        assert_eq!(
            data["managed_install"],
            "catalogue_accepted_install_pending"
        );
        assert_eq!(data["target"], "ubuntu_24_04_x86_64");
        assert_eq!(data["actions"].as_array().map(Vec::len), Some(3));
        assert_eq!(
            data["actions"][0]["files"].as_array().map(Vec::len),
            Some(3)
        );
        assert_eq!(
            data["actions"][1]["files"].as_array().map(Vec::len),
            Some(12)
        );
        assert_eq!(
            data["actions"][1]["archive_links"].as_array().map(Vec::len),
            Some(8)
        );
        assert_eq!(
            data["actions"][1]["runtime_copies"]
                .as_array()
                .map(Vec::len),
            Some(6)
        );
        assert!(
            data["actions"][2]["source_url"]
                .as_str()
                .is_some_and(|url| url.contains("/resolve/80da2d8b"))
        );
        assert_eq!(data["plan_digest"].as_str().map(str::len), Some(64));
        Ok(())
    }

    #[tokio::test]
    async fn accepted_plan_is_strict_and_revalidated_against_current_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = evaluate_plan(
            FixedProbe {
                state: DependencyState::Missing,
            },
            SetupProfile::Desktop,
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_800_000_000,
            Some(accepted_ubuntu_catalogue()?),
        )
        .await;
        let digest = original
            .authority
            .digest
            .clone()
            .ok_or_else(|| std::io::Error::other("qualified plan omitted its digest"))?;
        let bytes = serde_json::to_vec(&plan_json(&original)?)?;
        let saved: SavedSetupPlan = serde_json::from_slice(&bytes)?;

        let unchanged = evaluate_plan(
            FixedProbe {
                state: DependencyState::Missing,
            },
            SetupProfile::Desktop,
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_800_000_000,
            Some(accepted_ubuntu_catalogue()?),
        )
        .await;
        assert_eq!(unchanged.validate_acceptance(&saved, &digest), Ok(()));
        let wrong_digest = unchanged.validate_acceptance(&saved, &"0".repeat(64));
        assert!(matches!(wrong_digest, Err(EngineError::PlanAcceptance(_))));
        assert_eq!(
            wrong_digest.map_err(|error| error.failure_code()),
            Err(FailureCode::InvalidArgument)
        );

        let changed = evaluate_plan(
            MissingOneProbe {
                missing: RuntimeDependency::Whisper,
            },
            SetupProfile::Desktop,
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_800_000_000,
            Some(accepted_ubuntu_catalogue()?),
        )
        .await;
        assert_eq!(
            changed
                .validate_acceptance(&saved, &digest)
                .map_err(|error| error.failure_code()),
            Err(FailureCode::InvalidArgument)
        );

        let mut unknown: serde_json::Value = serde_json::from_slice(&bytes)?;
        unknown["data"]["unreviewed"] = serde_json::json!(true);
        assert!(serde_json::from_value::<SavedSetupPlan>(unknown).is_err());
        Ok(())
    }

    /// D-07: every target without a usable reviewed artifact gives typed manual
    /// guidance for each missing tool and the model, and nothing to accept.
    #[tokio::test]
    async fn unavailable_managed_targets_give_typed_manual_guidance()
    -> Result<(), Box<dyn std::error::Error>> {
        // 2030-11, after the reviewed catalogue's 2028-08-01 stop-new-plans boundary.
        const AFTER_CATALOGUE_EXPIRY: u64 = 1_920_000_000;
        let schema = setup_plan_schema()?;
        let validator = jsonschema::validator_for(&schema)?;
        for (target, now, catalogue, expected) in [
            (
                ManagedTarget::WindowsX86_64,
                1_800_000_000,
                Some(accepted_ubuntu_catalogue()?),
                "unavailable_target",
            ),
            (
                ManagedTarget::MacOsArm64,
                1_800_000_000,
                Some(accepted_ubuntu_catalogue()?),
                "unavailable_target",
            ),
            (
                ManagedTarget::Unsupported,
                1_800_000_000,
                Some(accepted_ubuntu_catalogue()?),
                "unavailable_target",
            ),
            (
                ManagedTarget::Ubuntu2404X86_64,
                1_800_000_000,
                None,
                "unavailable_target",
            ),
            (
                ManagedTarget::Ubuntu2404X86_64,
                AFTER_CATALOGUE_EXPIRY,
                Some(accepted_ubuntu_catalogue()?),
                "unavailable_catalogue_expired",
            ),
        ] {
            let plan = evaluate_plan(
                FixedProbe {
                    state: DependencyState::Missing,
                },
                SetupProfile::Worker,
                target,
                SetupSelectionState::default(),
                now,
                catalogue,
            )
            .await;
            let value = plan_json(&plan)?;
            validator
                .validate(&value)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let data = &value["data"];
            assert_eq!(data["managed_install"], expected, "{target:?}");
            assert_eq!(data["actions"], serde_json::json!([]), "{target:?}");
            assert!(data["plan_digest"].is_null(), "{target:?}");
            let dependencies = data["dependencies"]
                .as_array()
                .ok_or("dependencies missing")?;
            assert_eq!(dependencies.len(), 3);
            for dependency in dependencies {
                assert_eq!(dependency["disposition"], "manual_selection_required");
                assert_eq!(dependency["required_authority"], "user");
                assert!(
                    dependency["next_step"]
                        .as_str()
                        .is_some_and(|step| step.contains("setup configure")),
                    "{dependency}"
                );
            }
            let model = &data["local_asr_model"];
            assert_eq!(model["disposition"], "manual_selection_required");
            assert_eq!(model["required_authority"], "user");
            assert!(
                model["next_step"]
                    .as_str()
                    .is_some_and(|step| step.contains("setup configure-model")
                        && step.contains("supplied transcript")),
                "{model}"
            );
        }
        Ok(())
    }
}
