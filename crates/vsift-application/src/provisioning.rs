//! Deterministic setup planning over a reviewed managed-artifact catalogue.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};
use vsift_domain::{
    ArtifactIntegrity, DependencyStatus, ManagedArtifactFormat, ManagedComponent, ManagedTarget,
    RuntimeDependency, RuntimeReadiness,
};

use crate::RuntimeDiagnosis;

/// User-selected setup profile bound into plan acceptance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupProfile {
    /// Interactive local investigation.
    Desktop,
    /// Noninteractive bounded worker execution.
    Worker,
}

/// Explicit selections observed alongside the dependency probes.
///
/// These identities are bound into the digest but omitted from public output.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SetupSelectionState {
    /// Explicit `FFmpeg` path, if configured.
    pub ffmpeg: Option<String>,
    /// Explicit `FFprobe` path, if configured.
    pub ffprobe: Option<String>,
    /// Explicit whisper.cpp CLI path, if configured.
    pub whisper: Option<String>,
    /// Explicit model path, if configured.
    pub model: Option<String>,
}

impl SetupSelectionState {
    fn for_dependency(&self, dependency: RuntimeDependency) -> Option<&str> {
        match dependency {
            RuntimeDependency::Ffmpeg => self.ffmpeg.as_deref(),
            RuntimeDependency::Ffprobe => self.ffprobe.as_deref(),
            RuntimeDependency::Whisper => self.whisper.as_deref(),
        }
    }
}

impl SetupProfile {
    /// Stable identifier used by public plans.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Worker => "worker",
        }
    }
}

/// One exact regular file or regular-file alias accepted into a runtime layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedManagedFile {
    /// Flat name under the immutable version root.
    pub name: String,
    /// Expected bytes after extraction or alias copying.
    pub integrity: ArtifactIntegrity,
    /// Whether owner-executable mode is required on Unix.
    pub executable: bool,
}

/// Whole-archive limits and exact inventory observed during artifact review.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewedArchiveLimits {
    /// Maximum decompressed tar stream bytes.
    pub max_stream_bytes: u64,
    /// Exact expected number of archive entries.
    pub entries: usize,
    /// Exact sum of declared expanded entry sizes.
    pub expanded_bytes: u64,
}

/// One archive file selected into the private runtime payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedArchiveSelection {
    /// Exact path from the archive root.
    pub archive_path: String,
    /// Flat selected payload name.
    pub runtime_name: String,
    /// Exact selected-file integrity.
    pub integrity: ArtifactIntegrity,
}

/// One reviewed archive link header; links are never materialized as links.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedArchiveLink {
    /// Exact archive path.
    pub archive_path: String,
    /// Exact relative link target in the archive header.
    pub target: String,
}

/// One regular alias copy made from a selected runtime file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedRuntimeCopy {
    /// Resulting flat runtime name.
    pub name: String,
    /// Name of the selected regular source file.
    pub source_selected: String,
}

/// One exact publisher artifact accepted by review for a target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedManagedArtifact {
    /// Capability component supplied by the artifact.
    pub component: ManagedComponent,
    /// Immutable managed-version key.
    pub version: String,
    /// Publisher or upstream project disclosed to the user.
    pub publisher: String,
    /// Exact immutable direct-origin URL.
    pub source_url: String,
    /// Whole downloaded artifact integrity.
    pub integrity: ArtifactIntegrity,
    /// Reviewed archive representation.
    pub format: ManagedArtifactFormat,
    /// Exact archive bounds, absent for a raw model file.
    pub archive_limits: Option<ReviewedArchiveLimits>,
    /// Selected archive file mapping and integrity.
    pub selected_files: Vec<ReviewedArchiveSelection>,
    /// Reviewed archive link headers.
    pub archive_links: Vec<ReviewedArchiveLink>,
    /// Regular-file alias copies made in the runtime layout.
    pub runtime_copies: Vec<ReviewedRuntimeCopy>,
    /// SPDX-style licence disclosure for the selected artifact.
    pub licence: String,
    /// Exact licence or notice reference.
    pub notice_url: String,
    /// Source/build reference disclosed for review.
    pub source_code_url: String,
    /// Explicit limit of compatibility, provenance or notice evidence.
    pub trust_limit: String,
    /// Exact files installed beneath this component's immutable version.
    pub files: Vec<ReviewedManagedFile>,
}

/// A complete target catalogue accepted in reviewed application source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedManagedCatalogue {
    /// Revision bound into every plan digest.
    pub revision: String,
    /// Exact compatible host profile.
    pub target: ManagedTarget,
    /// UTC epoch second after which no new plan may be issued.
    pub stop_new_plans_at: u64,
    /// Human-readable expiry date disclosed in plans.
    pub stop_new_plans_date: String,
    /// Complete component set in stable component order.
    pub artifacts: Vec<AcceptedManagedArtifact>,
}

/// Why managed actions are or are not available in a setup plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedPlanAvailability {
    /// This target has an accepted, unexpired catalogue; install is still reserved.
    Qualified,
    /// No changes are required by the current read-only observations.
    NotRequired,
    /// No accepted catalogue exists for this exact host target.
    TargetUnavailable,
    /// The accepted entry has reached its stop-new-plans boundary.
    CatalogueExpired,
    /// Reviewed source is incomplete or inconsistent and cannot authorize actions.
    CatalogueInvalid,
}

impl ManagedPlanAvailability {
    /// Stable identifier used by the public setup contract.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Qualified => "catalogue_accepted_install_pending",
            Self::NotRequired => "not_required",
            Self::TargetUnavailable => "unavailable_target",
            Self::CatalogueExpired => "unavailable_catalogue_expired",
            Self::CatalogueInvalid => "unavailable_catalogue_invalid",
        }
    }
}

/// Planned disposition for one executable dependency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupDependencyDisposition {
    /// A user-managed executable responded but still needs compatibility preflight.
    ExistingProbeOnly,
    /// An accepted managed action supplies this missing dependency.
    ManagedInstall,
    /// The user must supply or install this dependency externally.
    ManualSelection,
}

impl SetupDependencyDisposition {
    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::ExistingProbeOnly => "existing_executable_probe_only",
            Self::ManagedInstall => "managed_install",
            Self::ManualSelection => "manual_selection_required",
        }
    }
}

/// Planned disposition of local ASR model weights.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupModelDisposition {
    /// A configured user-managed file exists but has not yet passed model preflight.
    ConfiguredProbeOnly,
    /// The reviewed managed model artifact is required.
    ManagedInstall,
    /// The user must configure model weights externally.
    ManualSelection,
}

impl SetupModelDisposition {
    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::ConfiguredProbeOnly => "configured_model_probe_only",
            Self::ManagedInstall => "managed_install",
            Self::ManualSelection => "manual_selection_required",
        }
    }
}

/// One exact managed action bound into a plan digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedSetupAction {
    /// Stable ordinal action identifier.
    pub id: String,
    /// Reviewed artifact applied by this action.
    pub artifact: AcceptedManagedArtifact,
}

/// Read-only setup plan whose digest binds catalogue, target, observations and actions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedSetupPlan {
    /// Selected profile.
    pub profile: SetupProfile,
    /// Executable-probe aggregate.
    pub readiness: RuntimeReadiness,
    /// Exact host target assessment.
    pub target: ManagedTarget,
    /// Managed availability decision.
    pub availability: ManagedPlanAvailability,
    /// Catalogue revision, when one is applicable.
    pub catalogue_revision: Option<String>,
    /// Stop-new-plans date, when one is applicable.
    pub stop_new_plans_date: Option<String>,
    /// Per-dependency state and disposition in stable order.
    pub dependencies: Vec<(DependencyStatus, SetupDependencyDisposition)>,
    /// Model presence disposition.
    pub model: SetupModelDisposition,
    /// Exact actions in stable component order.
    pub actions: Vec<ManagedSetupAction>,
    /// Canonical SHA-256 acceptance digest, absent when managed setup is unavailable.
    pub digest: Option<String>,
    selection_state: SetupSelectionState,
}

/// Explicit plan acceptance failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanAcceptanceError {
    /// The current target has no accepted plan to authorize.
    ManagedUnavailable,
    /// The supplied digest does not match the complete current plan.
    DigestMismatch,
}

impl ManagedSetupPlan {
    /// Verifies explicit authority for this exact plan.
    ///
    /// The caller must rebuild the plan from current state immediately before this
    /// check. Any target, catalogue, dependency, model or action change produces a
    /// different digest and therefore requires fresh user acceptance.
    ///
    /// # Errors
    ///
    /// Refuses unavailable plans and mismatched acceptance digests.
    pub fn validate_acceptance(&self, supplied: &str) -> Result<(), PlanAcceptanceError> {
        let expected = self
            .digest
            .as_deref()
            .ok_or(PlanAcceptanceError::ManagedUnavailable)?;
        if supplied == expected {
            Ok(())
        } else {
            Err(PlanAcceptanceError::DigestMismatch)
        }
    }
}

/// Builds a deterministic, non-mutating plan from current observations.
#[must_use]
pub fn plan_managed_setup(
    profile: SetupProfile,
    diagnosis: RuntimeDiagnosis,
    target: ManagedTarget,
    selection_state: SetupSelectionState,
    now_unix_seconds: u64,
    catalogue: Option<AcceptedManagedCatalogue>,
) -> ManagedSetupPlan {
    let applicable = catalogue.filter(|entry| entry.target == target);
    let (availability, catalogue) = match applicable {
        Some(entry) if !catalogue_is_complete(&entry) => {
            (ManagedPlanAvailability::CatalogueInvalid, Some(entry))
        }
        Some(entry) if now_unix_seconds >= entry.stop_new_plans_at => {
            (ManagedPlanAvailability::CatalogueExpired, Some(entry))
        }
        Some(entry) => (ManagedPlanAvailability::Qualified, Some(entry)),
        None => (ManagedPlanAvailability::TargetUnavailable, None),
    };

    let qualified = availability == ManagedPlanAvailability::Qualified;
    let media_missing = diagnosis.dependencies.iter().any(|status| {
        matches!(
            status.dependency,
            RuntimeDependency::Ffmpeg | RuntimeDependency::Ffprobe
        ) && !status.state.is_available()
    });
    let whisper_missing = diagnosis.dependencies.iter().any(|status| {
        status.dependency == RuntimeDependency::Whisper && !status.state.is_available()
    });

    let dependencies = diagnosis
        .dependencies
        .into_iter()
        .map(|status| {
            let needs_media = matches!(
                status.dependency,
                RuntimeDependency::Ffmpeg | RuntimeDependency::Ffprobe
            ) && media_missing;
            let needs_whisper = status.dependency == RuntimeDependency::Whisper && whisper_missing;
            let disposition = if status.state.is_available() {
                SetupDependencyDisposition::ExistingProbeOnly
            } else if qualified && (needs_media || needs_whisper) {
                SetupDependencyDisposition::ManagedInstall
            } else {
                SetupDependencyDisposition::ManualSelection
            };
            (status, disposition)
        })
        .collect::<Vec<_>>();

    let model = if selection_state.model.is_some() {
        SetupModelDisposition::ConfiguredProbeOnly
    } else if qualified {
        SetupModelDisposition::ManagedInstall
    } else {
        SetupModelDisposition::ManualSelection
    };

    let mut actions = Vec::new();
    if let Some(entry) = catalogue.as_ref().filter(|_| qualified) {
        for artifact in &entry.artifacts {
            let required = match artifact.component {
                ManagedComponent::MediaTools => media_missing,
                ManagedComponent::WhisperCli => whisper_missing,
                ManagedComponent::WhisperModel => selection_state.model.is_none(),
            };
            if required {
                actions.push(ManagedSetupAction {
                    id: format!("install-{}", artifact.component.identifier()),
                    artifact: artifact.clone(),
                });
            }
        }
    }

    let availability = if qualified && actions.is_empty() {
        ManagedPlanAvailability::NotRequired
    } else {
        availability
    };
    let mut plan = ManagedSetupPlan {
        profile,
        readiness: diagnosis.readiness,
        target,
        availability,
        catalogue_revision: catalogue.as_ref().map(|entry| entry.revision.clone()),
        stop_new_plans_date: catalogue
            .as_ref()
            .map(|entry| entry.stop_new_plans_date.clone()),
        dependencies,
        model,
        actions,
        digest: None,
        selection_state,
    };
    if matches!(
        availability,
        ManagedPlanAvailability::Qualified | ManagedPlanAvailability::NotRequired
    ) {
        plan.digest = Some(plan_digest(&plan));
    }
    plan
}

fn catalogue_is_complete(entry: &AcceptedManagedCatalogue) -> bool {
    entry.artifacts.len() == 3
        && entry
            .artifacts
            .iter()
            .map(|artifact| artifact.component)
            .eq([
                ManagedComponent::MediaTools,
                ManagedComponent::WhisperCli,
                ManagedComponent::WhisperModel,
            ])
        && entry.artifacts.iter().all(|artifact| {
            !artifact.files.is_empty()
                && !artifact.version.is_empty()
                && !artifact.source_url.is_empty()
                && !artifact.trust_limit.is_empty()
                && match artifact.format {
                    ManagedArtifactFormat::TarXz | ManagedArtifactFormat::TarGz => {
                        artifact.archive_limits.is_some() && !artifact.selected_files.is_empty()
                    }
                    ManagedArtifactFormat::RawFile => {
                        artifact.archive_limits.is_none()
                            && artifact.selected_files.is_empty()
                            && artifact.archive_links.is_empty()
                            && artifact.runtime_copies.is_empty()
                    }
                }
        })
}

fn plan_digest(plan: &ManagedSetupPlan) -> String {
    let mut digest = Sha256::new();
    digest_field(&mut digest, "vsift.setup-plan.v1");
    digest_field(&mut digest, plan.profile.identifier());
    digest_field(&mut digest, plan.target.identifier());
    digest_field(&mut digest, plan.availability.identifier());
    digest_field(
        &mut digest,
        plan.catalogue_revision.as_deref().unwrap_or(""),
    );
    digest_field(
        &mut digest,
        plan.stop_new_plans_date.as_deref().unwrap_or(""),
    );
    for (status, disposition) in &plan.dependencies {
        digest_field(&mut digest, status.dependency.identifier());
        digest_field(&mut digest, status.state.identifier());
        match &status.state {
            vsift_domain::DependencyState::Available { version } => {
                digest_field(&mut digest, version);
            }
            vsift_domain::DependencyState::Unhealthy { message } => {
                digest_field(&mut digest, message);
            }
            vsift_domain::DependencyState::Missing | vsift_domain::DependencyState::TimedOut => {}
        }
        digest_field(
            &mut digest,
            plan.selection_state
                .for_dependency(status.dependency)
                .unwrap_or(""),
        );
        digest_field(&mut digest, disposition.identifier());
    }
    digest_field(&mut digest, plan.model.identifier());
    digest_field(
        &mut digest,
        plan.selection_state.model.as_deref().unwrap_or(""),
    );
    for action in &plan.actions {
        digest_field(&mut digest, &action.id);
        digest_artifact(&mut digest, &action.artifact);
    }
    let mut encoded = String::with_capacity(64);
    for byte in digest.finalize() {
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn digest_artifact(digest: &mut Sha256, artifact: &AcceptedManagedArtifact) {
    digest_field(digest, artifact.component.identifier());
    digest_field(digest, &artifact.version);
    digest_field(digest, &artifact.publisher);
    digest_field(digest, &artifact.source_url);
    digest_field(digest, &artifact.integrity.bytes().to_string());
    digest_field(digest, &artifact.integrity.sha256_hex());
    digest_field(digest, artifact.format.identifier());
    if let Some(limits) = artifact.archive_limits {
        digest_field(digest, &limits.max_stream_bytes.to_string());
        digest_field(digest, &limits.entries.to_string());
        digest_field(digest, &limits.expanded_bytes.to_string());
    }
    for selected in &artifact.selected_files {
        digest_field(digest, &selected.archive_path);
        digest_field(digest, &selected.runtime_name);
        digest_field(digest, &selected.integrity.bytes().to_string());
        digest_field(digest, &selected.integrity.sha256_hex());
    }
    for link in &artifact.archive_links {
        digest_field(digest, &link.archive_path);
        digest_field(digest, &link.target);
    }
    for copy in &artifact.runtime_copies {
        digest_field(digest, &copy.name);
        digest_field(digest, &copy.source_selected);
    }
    digest_field(digest, &artifact.licence);
    digest_field(digest, &artifact.notice_url);
    digest_field(digest, &artifact.source_code_url);
    digest_field(digest, &artifact.trust_limit);
    for file in &artifact.files {
        digest_field(digest, &file.name);
        digest_field(digest, &file.integrity.bytes().to_string());
        digest_field(digest, &file.integrity.sha256_hex());
        digest_field(
            digest,
            if file.executable {
                "executable"
            } else {
                "regular"
            },
        );
    }
}

fn digest_field(digest: &mut Sha256, value: &str) {
    digest.update(value.len().to_be_bytes());
    digest.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use vsift_domain::{
        ArtifactIntegrity, DependencyState, DependencyStatus, ManagedArtifactFormat,
        ManagedComponent, ManagedTarget, RuntimeDependency, RuntimeReadiness,
    };

    use super::{
        AcceptedManagedArtifact, AcceptedManagedCatalogue, ManagedPlanAvailability,
        PlanAcceptanceError, ReviewedManagedFile, SetupDependencyDisposition, SetupProfile,
        SetupSelectionState, plan_managed_setup,
    };
    use crate::RuntimeDiagnosis;

    fn integrity(byte: char) -> Result<ArtifactIntegrity, Box<dyn std::error::Error>> {
        Ok(ArtifactIntegrity::from_sha256_hex(
            10,
            &byte.to_string().repeat(64),
        )?)
    }

    fn catalogue() -> Result<AcceptedManagedCatalogue, Box<dyn std::error::Error>> {
        let artifacts = [
            ManagedComponent::MediaTools,
            ManagedComponent::WhisperCli,
            ManagedComponent::WhisperModel,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, component)| {
            let digest = char::from(b'a' + u8::try_from(index).unwrap_or(0));
            Ok(AcceptedManagedArtifact {
                component,
                version: format!("version-{index}"),
                publisher: String::from("publisher"),
                source_url: format!("https://publisher.invalid/{index}"),
                integrity: integrity(digest)?,
                format: ManagedArtifactFormat::RawFile,
                archive_limits: None,
                selected_files: Vec::new(),
                archive_links: Vec::new(),
                runtime_copies: Vec::new(),
                licence: String::from("MIT"),
                notice_url: String::from("https://publisher.invalid/notice"),
                source_code_url: String::from("https://publisher.invalid/source"),
                trust_limit: String::from("fixture evidence only"),
                files: vec![ReviewedManagedFile {
                    name: format!("file-{index}"),
                    integrity: integrity(digest)?,
                    executable: index != 2,
                }],
            })
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
        Ok(AcceptedManagedCatalogue {
            revision: String::from("fixture-r1"),
            target: ManagedTarget::Ubuntu2404X86_64,
            stop_new_plans_at: 2_000,
            stop_new_plans_date: String::from("fixture-date"),
            artifacts,
        })
    }

    fn diagnosis(missing: &[RuntimeDependency]) -> RuntimeDiagnosis {
        RuntimeDiagnosis {
            readiness: RuntimeReadiness::Blocked,
            dependencies: RuntimeDependency::ALL
                .into_iter()
                .map(|dependency| DependencyStatus {
                    dependency,
                    state: if missing.contains(&dependency) {
                        DependencyState::Missing
                    } else {
                        DependencyState::Available {
                            version: String::from("fixture"),
                        }
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn qualified_target_produces_stable_reviewable_actions_and_acceptance()
    -> Result<(), Box<dyn std::error::Error>> {
        let first = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&RuntimeDependency::ALL),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_000,
            Some(catalogue()?),
        );
        let second = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&RuntimeDependency::ALL),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_500,
            Some(catalogue()?),
        );

        assert_eq!(first.availability, ManagedPlanAvailability::Qualified);
        assert_eq!(first.actions.len(), 3);
        assert_eq!(first.digest, second.digest);
        let accepted = first.digest.as_deref().ok_or("digest missing")?;
        assert_eq!(first.validate_acceptance(accepted), Ok(()));
        assert_eq!(
            first.validate_acceptance(&"0".repeat(64)),
            Err(PlanAcceptanceError::DigestMismatch)
        );
        Ok(())
    }

    #[test]
    fn state_change_changes_digest_and_avoids_unneeded_download()
    -> Result<(), Box<dyn std::error::Error>> {
        let missing = plan_managed_setup(
            SetupProfile::Worker,
            diagnosis(&[RuntimeDependency::Ffmpeg]),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState {
                model: Some(String::from("/fixture/model")),
                ..SetupSelectionState::default()
            },
            1_000,
            Some(catalogue()?),
        );
        let available = plan_managed_setup(
            SetupProfile::Worker,
            diagnosis(&[]),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState {
                model: Some(String::from("/fixture/model")),
                ..SetupSelectionState::default()
            },
            1_000,
            Some(catalogue()?),
        );

        assert_eq!(missing.actions.len(), 1);
        assert!(available.actions.is_empty());
        assert_eq!(available.availability, ManagedPlanAvailability::NotRequired);
        assert_ne!(missing.digest, available.digest);
        assert_eq!(
            missing.dependencies[1].1,
            SetupDependencyDisposition::ExistingProbeOnly
        );
        Ok(())
    }

    #[test]
    fn unsupported_and_expired_targets_fail_closed_without_digest()
    -> Result<(), Box<dyn std::error::Error>> {
        let unsupported = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&RuntimeDependency::ALL),
            ManagedTarget::WindowsX86_64,
            SetupSelectionState::default(),
            1_000,
            Some(catalogue()?),
        );
        let expired = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&RuntimeDependency::ALL),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            2_000,
            Some(catalogue()?),
        );

        assert_eq!(
            unsupported.availability,
            ManagedPlanAvailability::TargetUnavailable
        );
        assert_eq!(
            expired.availability,
            ManagedPlanAvailability::CatalogueExpired
        );
        assert!(unsupported.digest.is_none());
        assert!(expired.digest.is_none());
        assert!(unsupported.actions.is_empty());
        assert!(expired.actions.is_empty());
        Ok(())
    }

    #[test]
    fn changed_selection_or_catalogue_requires_fresh_acceptance()
    -> Result<(), Box<dyn std::error::Error>> {
        let baseline = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&[RuntimeDependency::Ffmpeg]),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_000,
            Some(catalogue()?),
        );
        let prior = baseline.digest.as_deref().ok_or("digest missing")?;
        let selected = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&[RuntimeDependency::Ffmpeg]),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState {
                ffprobe: Some(String::from("/new/ffprobe")),
                ..SetupSelectionState::default()
            },
            1_000,
            Some(catalogue()?),
        );
        let mut changed_catalogue = catalogue()?;
        changed_catalogue.artifacts[0]
            .trust_limit
            .push_str(" changed");
        let changed = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&[RuntimeDependency::Ffmpeg]),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_000,
            Some(changed_catalogue),
        );

        assert_eq!(
            selected.validate_acceptance(prior),
            Err(PlanAcceptanceError::DigestMismatch)
        );
        assert_eq!(
            changed.validate_acceptance(prior),
            Err(PlanAcceptanceError::DigestMismatch)
        );
        Ok(())
    }

    #[test]
    fn incomplete_catalogue_cannot_produce_actions_or_digest()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut incomplete = catalogue()?;
        incomplete.artifacts.pop();
        let plan = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&RuntimeDependency::ALL),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_000,
            Some(incomplete),
        );
        assert_eq!(plan.availability, ManagedPlanAvailability::CatalogueInvalid);
        assert!(plan.actions.is_empty());
        assert!(plan.digest.is_none());
        Ok(())
    }
}
