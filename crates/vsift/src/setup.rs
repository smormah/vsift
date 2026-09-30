//! Setup operations: dependency checks, the read-only managed plan and its
//! acceptance, and bring-your-own executable and model registration.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use vsift_application::{
    DiagnoseRuntime, LocalAsrSetupStatus, ManagedPlanAvailability, ManagedPlanObservation,
    ManagedSetupPlan, ObservedModel, RuntimeDiagnosis, SetupProfile, SetupSelectionState,
    plan_managed_setup,
};
use vsift_contract::{DependencyLookup, SavedSetupPlan, SetupPlanResponse};
use vsift_domain::RuntimeDependency;
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

    fn select(&mut self, dependency: RuntimeDependency, path: PathBuf) {
        match dependency {
            RuntimeDependency::Ffmpeg => self.ffmpeg = Some(path),
            RuntimeDependency::Ffprobe => self.ffprobe = Some(path),
            RuntimeDependency::Whisper => self.whisper = Some(path),
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
    /// Time allowed for the local-ASR verification when no pass is recorded,
    /// separate from `probe_timeout`; hosts normally pass
    /// [`crate::DEFAULT_LOCAL_ASR_CHECK_BUDGET`].
    pub local_asr_budget: Duration,
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
    /// Whether the managed version was used, when nothing was selected.
    managed: [bool; 3],
    local_asr: LocalAsrSetupStatus,
    managed_install: ManagedPlanAvailability,
}

impl SetupCheckReport {
    /// The registered model and the local-ASR functional verification
    /// (maintainer decision D4).
    #[must_use]
    pub const fn local_asr(&self) -> &LocalAsrSetupStatus {
        &self.local_asr
    }

    /// Probe results and aggregate readiness.
    #[must_use]
    pub const fn diagnosis(&self) -> &RuntimeDiagnosis {
        &self.diagnosis
    }

    /// Where the executable probed for `dependency` came from: a per-call path
    /// wins over a configured user path, which wins over the managed
    /// version, which wins over the filtered `PATH`.
    #[must_use]
    pub const fn lookup(&self, dependency: RuntimeDependency) -> DependencyLookup {
        let index = dependency_index(dependency);
        if self.per_call[index] {
            DependencyLookup::ExplicitPath
        } else if self.selected[index] {
            DependencyLookup::ConfiguredUserPath
        } else if self.managed[index] {
            DependencyLookup::ManagedVersion
        } else {
            DependencyLookup::FilteredPath
        }
    }

    /// Whether managed installation can supply a missing dependency on this
    /// host: the availability `setup plan` reports for this target and the
    /// built-in catalogue (the setup-check remediation's `managed_install`).
    #[must_use]
    pub const fn managed_install(&self) -> ManagedPlanAvailability {
        self.managed_install
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
    /// A plan's authority with its presentation beside `observation`.
    fn new(authority: ManagedSetupPlan, observation: &ManagedPlanObservation) -> Self {
        let presentation = SetupPlanResponse::new(&authority, observation);
        Self {
            authority,
            presentation,
        }
    }

    /// The plan's authority: its actions, digest and catalogue revision.
    pub(crate) const fn authority(&self) -> &ManagedSetupPlan {
        &self.authority
    }

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
    /// Probes `FFmpeg`, `FFprobe` and whisper.cpp, identifies the registered
    /// model and reports the local-ASR functional verification.
    ///
    /// Probes only show that executables respond. The local-ASR report says
    /// whether the model is a reviewed pinned profile and whether the selected
    /// tools and model passed the verification: a recorded pass is reported
    /// as is; otherwise, when everything it needs is present, the
    /// verification runs within `local_asr_budget` and a pass is recorded. It
    /// changes nothing else on the machine.
    ///
    /// # Errors
    ///
    /// Fails when the per-user configuration, the clock or the built-in
    /// reviewed policy cannot be read. Missing or unhealthy dependencies and a
    /// failed verification are results, not errors.
    pub async fn check_setup(
        &self,
        request: SetupCheckRequest,
    ) -> Result<SetupCheckReport, EngineError> {
        let store = self.user_configuration()?;
        let configured = ExecutableSelections::from_configured(store.read()?);
        let per_call = request.per_call;
        let mut selections = ExecutableSelections {
            ffmpeg: per_call.ffmpeg.clone().or(configured.ffmpeg),
            ffprobe: per_call.ffprobe.clone().or(configured.ffprobe),
            whisper: per_call.whisper.clone().or(configured.whisper),
        };
        let selected = presence(&selections);
        // The managed tier fills what nothing selected; each managed
        // executable is held (its version cannot be removed) for the check.
        let mut held = Vec::new();
        let mut managed = [false; 3];
        let mut lookup = self.managed_lookup();
        for dependency in RuntimeDependency::ALL {
            if selections.for_dependency(dependency).is_none()
                && let Some(executable) = lookup.executable(dependency)
            {
                selections.select(dependency, executable.path().to_path_buf());
                managed[dependency_index(dependency)] = true;
                held.push(executable);
            }
        }
        let probe = ProcessDependencyProbe::with_explicit_paths(
            request.probe_timeout,
            selections.clone().into_probe_paths(),
        );
        let diagnosis = DiagnoseRuntime::new(probe).execute().await;
        let managed_model = match store.read_model()? {
            Some(configured) => Some((configured, None)),
            None => lookup.model().map(|model| (model.path, Some(model.hold))),
        };
        let local_asr = self
            .check_local_asr(
                &selections,
                managed_model.as_ref().map(|(path, _)| path.as_path()),
                request.local_asr_budget,
            )
            .await?;
        drop((held, managed_model));
        Ok(SetupCheckReport {
            diagnosis,
            per_call: presence(&per_call),
            selected,
            managed,
            local_asr,
            managed_install: self.managed_install_availability()?,
        })
    }

    /// Builds the current read-only managed setup plan from fresh observations.
    ///
    /// The plan's intent (and digest) comes from the tools outside the
    /// managed store, which `setup install` never changes; beside it, the
    /// plan shows what commands would use now, managed versions included:
    /// each action's `state`, the dependencies' statuses and readiness.
    ///
    /// # Errors
    ///
    /// Fails when the configuration, clock or built-in reviewed catalogue
    /// cannot be read.
    pub async fn plan_setup(
        &self,
        request: SetupPlanRequest,
    ) -> Result<EvaluatedSetupPlan, EngineError> {
        let (plan, configured) = self.plan_intent(request).await?;
        let observation = self
            .observe_plan(&plan, configured, request.probe_timeout)
            .await;
        Ok(EvaluatedSetupPlan::new(plan, &observation))
    }

    /// The current plan's intent alone, for acceptance, which compares the
    /// intent only: nothing managed is opened or probed.
    pub(crate) async fn plan_setup_intent(
        &self,
        request: SetupPlanRequest,
    ) -> Result<EvaluatedSetupPlan, EngineError> {
        let (plan, _) = self.plan_intent(request).await?;
        let observation = ManagedPlanObservation::without_managed_tier(&plan);
        Ok(EvaluatedSetupPlan::new(plan, &observation))
    }

    /// What commands would use now for `plan`, the managed tier included.
    async fn observe_plan(
        &self,
        plan: &ManagedSetupPlan,
        configured: ExplicitProbePaths,
        probe_timeout: Duration,
    ) -> ManagedPlanObservation {
        let mut observation = ManagedPlanObservation::without_managed_tier(plan);
        let mut lookup = self.managed_lookup();
        let mut paths = configured;
        let mut held = Vec::new();
        for dependency in RuntimeDependency::ALL {
            if paths.for_dependency(dependency).is_none()
                && let Some(executable) = lookup.executable(dependency)
            {
                let path = Some(executable.path().to_path_buf());
                match dependency {
                    RuntimeDependency::Ffmpeg => paths.ffmpeg = path,
                    RuntimeDependency::Ffprobe => paths.ffprobe = path,
                    RuntimeDependency::Whisper => paths.whisper = path,
                }
                held.push(executable);
            }
        }
        if !held.is_empty() {
            let probe = ProcessDependencyProbe::with_explicit_paths(probe_timeout, paths);
            let diagnosis = DiagnoseRuntime::new(probe).execute().await;
            observation.readiness = diagnosis.readiness;
            observation.dependencies = diagnosis.dependencies;
        }
        if observation.model == ObservedModel::Missing && lookup.model().is_some() {
            observation.model = ObservedModel::Managed;
        }
        observation.current = plan
            .actions
            .iter()
            .filter(|action| {
                lookup
                    .selected_version(action.artifact.component)
                    .as_deref()
                    == Some(action.artifact.version.as_str())
            })
            .map(|action| action.artifact.component)
            .collect();
        drop(held);
        observation
    }

    /// The plan's intent from the configuration, the clock, the catalogue
    /// and the tools outside the managed store, with the configured paths.
    async fn plan_intent(
        &self,
        request: SetupPlanRequest,
    ) -> Result<(ManagedSetupPlan, ExplicitProbePaths), EngineError> {
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
        let probe =
            ProcessDependencyProbe::with_explicit_paths(request.probe_timeout, configured.clone());
        let plan = plan_managed_setup(
            request.profile,
            DiagnoseRuntime::new(probe).execute().await,
            detect_managed_target(),
            selections,
            now_unix_seconds,
            Some(catalogue),
        );
        Ok((plan, configured))
    }

    /// Whether managed installation is qualified on this host today: the
    /// built-in catalogue's target, expiry and completeness, before any
    /// observation of the tools.
    pub(crate) fn managed_install_availability(
        &self,
    ) -> Result<ManagedPlanAvailability, EngineError> {
        let catalogue =
            accepted_ubuntu_catalogue().map_err(|_| EngineError::ReviewedPolicyInvalid)?;
        let target = detect_managed_target();
        Ok(if catalogue.target != target {
            ManagedPlanAvailability::TargetUnavailable
        } else if self.now_unix_seconds()? >= catalogue.stop_new_plans_at {
            ManagedPlanAvailability::CatalogueExpired
        } else {
            ManagedPlanAvailability::Qualified
        })
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

/// Diagnoses current dependencies and builds plan authority plus its
/// presentation on a machine with no managed tier (the unit tests' plans).
#[cfg(test)]
async fn evaluate_plan<P: vsift_application::DependencyProbe>(
    probe: P,
    profile: SetupProfile,
    target: vsift_domain::ManagedTarget,
    selections: SetupSelectionState,
    now_unix_seconds: u64,
    catalogue: Option<vsift_application::AcceptedManagedCatalogue>,
) -> EvaluatedSetupPlan {
    let plan = plan_managed_setup(
        profile,
        DiagnoseRuntime::new(probe).execute().await,
        target,
        selections,
        now_unix_seconds,
        catalogue,
    );
    let observation = ManagedPlanObservation::without_managed_tier(&plan);
    EvaluatedSetupPlan::new(plan, &observation)
}

#[cfg(test)]
mod tests {
    use std::future::ready;

    use vsift_application::{
        DependencyProbe, ManagedPlanObservation, ObservedModel, SetupProfile, SetupSelectionState,
    };
    use vsift_contract::{OperationResponse, SavedSetupPlan};
    use vsift_domain::{
        DependencyState, DependencyStatus, FailureCode, ManagedComponent, ManagedTarget,
        RuntimeDependency, RuntimeReadiness,
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
        assert_eq!(data["managed_install"], "catalogue_accepted");
        assert_eq!(data["target"], "ubuntu_24_04_x86_64");
        assert_eq!(data["actions"].as_array().map(Vec::len), Some(3));
        assert_eq!(
            data["actions"][0]["files"].as_array().map(Vec::len),
            Some(3)
        );
        assert_eq!(
            data["actions"][1]["files"].as_array().map(Vec::len),
            Some(25)
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

    /// The plan an Ubuntu machine with none of the tools makes.
    async fn missing_tools_plan() -> Result<EvaluatedSetupPlan, Box<dyn std::error::Error>> {
        Ok(evaluate_plan(
            FixedProbe {
                state: DependencyState::Missing,
            },
            SetupProfile::Desktop,
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_800_000_000,
            Some(accepted_ubuntu_catalogue()?),
        )
        .await)
    }

    fn available(dependency: RuntimeDependency) -> DependencyStatus {
        DependencyStatus {
            dependency,
            state: DependencyState::Available {
                version: String::from("managed fixture 1"),
            },
        }
    }

    /// P13 PR 4 (a) and (b): installing managed components changes the plan's
    /// observed state, never its intent, so the accepted plan stays acceptable
    /// after a partial install and the plan shows what is installed.
    #[tokio::test]
    async fn installed_managed_components_are_observed_without_changing_acceptance()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = missing_tools_plan().await?;
        let digest = original
            .authority
            .digest
            .clone()
            .ok_or_else(|| std::io::Error::other("qualified plan omitted its digest"))?;
        let original_value = plan_json(&original)?;
        assert_eq!(original_value["data"]["install_needed"], true);
        assert_eq!(original_value["data"]["readiness"], "blocked");
        let saved: SavedSetupPlan = serde_json::from_value(original_value)?;
        let schema = setup_plan_schema()?;
        let validator = jsonschema::validator_for(&schema)?;

        // (b) After a partial install (media tools only) the same accepted
        // plan and digest are still accepted, so a rerun continues.
        let partial_authority = missing_tools_plan().await?.authority;
        let partial = EvaluatedSetupPlan::new(
            partial_authority.clone(),
            &ManagedPlanObservation {
                readiness: RuntimeReadiness::Degraded,
                dependencies: vec![
                    available(RuntimeDependency::Ffmpeg),
                    available(RuntimeDependency::Ffprobe),
                    DependencyStatus {
                        dependency: RuntimeDependency::Whisper,
                        state: DependencyState::Missing,
                    },
                ],
                model: ObservedModel::Missing,
                current: vec![ManagedComponent::MediaTools],
            },
        );
        assert_eq!(partial.validate_acceptance(&saved, &digest), Ok(()));
        let value = plan_json(&partial)?;
        validator
            .validate(&value)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let data = &value["data"];
        assert_eq!(data["plan_digest"].as_str(), Some(digest.as_str()));
        assert_eq!(data["install_needed"], true);
        assert_eq!(data["readiness"], "degraded");
        assert_eq!(data["actions"][0]["component"], "ffmpeg_ffprobe");
        assert_eq!(data["actions"][0]["state"], "current");
        assert_eq!(data["actions"][1]["state"], "pending");
        assert_eq!(data["actions"][2]["state"], "pending");
        assert_eq!(data["dependencies"][0]["status"], "available");
        assert_eq!(data["dependencies"][2]["status"], "missing");
        assert_eq!(data["local_asr_model"]["status"], "missing");

        // (a) After the complete install the plan reports every managed
        // component current and readiness no longer blocked on them.
        let complete = EvaluatedSetupPlan::new(
            partial_authority,
            &ManagedPlanObservation {
                readiness: RuntimeReadiness::Ready,
                dependencies: RuntimeDependency::ALL.into_iter().map(available).collect(),
                model: ObservedModel::Managed,
                current: vec![
                    ManagedComponent::MediaTools,
                    ManagedComponent::WhisperCli,
                    ManagedComponent::WhisperModel,
                ],
            },
        );
        assert_eq!(complete.validate_acceptance(&saved, &digest), Ok(()));
        let value = plan_json(&complete)?;
        validator
            .validate(&value)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let data = &value["data"];
        assert_eq!(data["readiness"], "ready");
        assert_eq!(data["install_needed"], false);
        assert_eq!(data["local_asr_model"]["status"], "managed_current");
        for action in data["actions"].as_array().ok_or("actions missing")? {
            assert_eq!(action["state"], "current", "{action}");
        }
        for dependency in data["dependencies"].as_array().ok_or("deps missing")? {
            assert_eq!(dependency["status"], "available", "{dependency}");
        }
        Ok(())
    }

    /// P13 PR 4: observed state is ignored by acceptance, but any change to
    /// the plan's intent in the saved file is still refused.
    #[tokio::test]
    async fn acceptance_still_refuses_a_changed_intent_beside_observed_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = missing_tools_plan().await?;
        let digest = original
            .authority
            .digest
            .clone()
            .ok_or_else(|| std::io::Error::other("qualified plan omitted its digest"))?;
        let value = plan_json(&original)?;

        // A saved plan whose observed members differ is still the same plan.
        let mut observed = value.clone();
        observed["data"]["readiness"] = serde_json::json!("ready");
        observed["data"]["install_needed"] = serde_json::json!(false);
        observed["data"]["actions"][0]["state"] = serde_json::json!("current");
        let saved: SavedSetupPlan = serde_json::from_value(observed)?;
        assert_eq!(original.validate_acceptance(&saved, &digest), Ok(()));

        // A saved plan whose reviewed artifact differs is refused.
        let mut tampered = value;
        tampered["data"]["actions"][0]["version"] = serde_json::json!("n0.0.0-unreviewed");
        let saved: SavedSetupPlan = serde_json::from_value(tampered)?;
        assert_eq!(
            original
                .validate_acceptance(&saved, &digest)
                .map_err(|error| error.failure_code()),
            Err(FailureCode::InvalidArgument)
        );
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
