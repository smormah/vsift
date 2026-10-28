//! Deterministic setup planning over a reviewed managed-artifact catalogue.

use std::{fmt::Write as _, future::Future};

use sha2::{Digest, Sha256};
use vsift_domain::{
    ArtifactIntegrity, DependencyStatus, ManagedArtifactFormat, ManagedComponent, ManagedTarget,
    RuntimeDependency, RuntimeReadiness, SharedLibraryName,
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

/// An optional component a user can add to a plan with `setup plan
/// --include <name>` (ADR 0032): a named group of managed components, never
/// planned unless included and never needed for local investigation. Each
/// group installs as a part of its own after the core, so a failure of one
/// never fails the core or another group. Each feature story adds its group
/// (ADR 0032: `ocr`, `speakers`, `online_links`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptionalComponent {
    /// Online links (ADR 0028 section 4): yt-dlp, which fetches a video from
    /// a public platform page, with the managed JavaScript runtime it runs a
    /// page's player code with. Offered on Ubuntu 24.04 x86-64 only.
    OnlineLinks,
}

impl OptionalComponent {
    /// Every optional component, in plan order.
    pub const ALL: [Self; 1] = [Self::OnlineLinks];

    /// The name `setup plan --include` takes and the plan shows.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::OnlineLinks => "online_links",
        }
    }

    /// The optional component `identifier` names.
    #[must_use]
    pub fn from_identifier(identifier: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|component| component.identifier() == identifier)
    }

    /// The managed components it installs, in install order: the runtime
    /// only yt-dlp needs comes with it and before it.
    #[must_use]
    pub const fn components(self) -> &'static [ManagedComponent] {
        match self {
            Self::OnlineLinks => &[ManagedComponent::JsRuntimeDeno, ManagedComponent::YtDlp],
        }
    }

    /// The optional component `component` belongs to; `None` for a core
    /// component.
    #[must_use]
    pub fn of(component: ManagedComponent) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|group| group.components().contains(&component))
    }

    const fn bit(self) -> u8 {
        match self {
            Self::OnlineLinks => 1,
        }
    }
}

/// The optional components a user included in a plan.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OptionalComponentSet(u8);

impl OptionalComponentSet {
    /// No optional component: the default plan.
    pub const EMPTY: Self = Self(0);

    /// This set with `component` added.
    #[must_use]
    pub const fn with(self, component: OptionalComponent) -> Self {
        Self(self.0 | component.bit())
    }

    /// Whether `component` is included.
    #[must_use]
    pub const fn contains(self, component: OptionalComponent) -> bool {
        self.0 & component.bit() != 0
    }

    /// Whether nothing is included.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The included components, in plan order.
    pub fn iter(self) -> impl Iterator<Item = OptionalComponent> {
        OptionalComponent::ALL
            .into_iter()
            .filter(move |component| self.contains(*component))
    }
}

/// A core part of the setup, as a person names it (ADR 0036 section 4):
/// each managed core component belongs to one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CorePart {
    /// The media tools, `FFmpeg` with `FFprobe`.
    Video,
    /// The whisper.cpp program and its model.
    Speech,
}

impl CorePart {
    /// Both parts, video first: the order a plan installs them.
    pub const ALL: [Self; 2] = [Self::Video, Self::Speech];

    /// The part a core component belongs to; `None` for an optional one.
    #[must_use]
    pub const fn of(component: ManagedComponent) -> Option<Self> {
        match component {
            ManagedComponent::MediaTools => Some(Self::Video),
            ManagedComponent::WhisperCli | ManagedComponent::WhisperModel => Some(Self::Speech),
            ManagedComponent::JsRuntimeDeno | ManagedComponent::YtDlp => None,
        }
    }

    /// Stable machine-readable identifier: `video` or `speech`.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Speech => "speech",
        }
    }

    const fn bit(self) -> u8 {
        match self {
            Self::Video => 1,
            Self::Speech => 2,
        }
    }
}

/// The core parts a plan may install (ADR 0036 section 6): both, unless an
/// unattended setup names fewer. A plan of both is the plan `setup plan`
/// builds, with the same intent and digest as before parts existed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CorePartSet(u8);

impl CorePartSet {
    /// Both parts.
    pub const ALL: Self = Self(3);

    /// No part.
    pub const EMPTY: Self = Self(0);

    /// This set with `part` added.
    #[must_use]
    pub const fn with(self, part: CorePart) -> Self {
        Self(self.0 | part.bit())
    }

    /// Whether `part` is in the set.
    #[must_use]
    pub const fn contains(self, part: CorePart) -> bool {
        self.0 & part.bit() != 0
    }

    /// The parts, video first.
    pub fn iter(self) -> impl Iterator<Item = CorePart> {
        CorePart::ALL
            .into_iter()
            .filter(move |part| self.contains(*part))
    }
}

impl Default for CorePartSet {
    fn default() -> Self {
        Self::ALL
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

/// Exact bounded compatibility contract reviewed for one managed target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedCompatibilityPolicy {
    /// Synthetic media fixture that every provider set must process.
    pub fixture: ArtifactIntegrity,
    /// Expected first-line prefix from the selected `FFmpeg` build.
    pub expected_ffmpeg_version: String,
    /// Expected first-line prefix from the selected `FFprobe` build.
    pub expected_ffprobe_version: String,
    /// Maximum captured bytes on each provider output stream.
    pub stream_limit_bytes: usize,
    /// Maximum generated PCM/WAV bytes admitted by the smoke workspace.
    pub audio_file_limit_bytes: u64,
    /// Maximum generated transcript bytes admitted by the smoke workspace.
    pub transcript_file_limit_bytes: u64,
    /// Per-process deadline for media inspection and extraction.
    pub media_deadline_seconds: u64,
    /// Deadline for the model-backed inference process.
    pub inference_deadline_seconds: u64,
    /// Required PCM sample rate passed between `FFmpeg` and whisper.cpp.
    pub audio_sample_rate_hz: u32,
    /// Required PCM channel count passed between `FFmpeg` and whisper.cpp.
    pub audio_channels: u8,
    /// The reviewed version banner of each optional component's executable,
    /// checked by its smoke (ADR 0028 section 4); one per optional component
    /// the catalogue offers.
    pub optional_banners: Vec<ReviewedBanner>,
}

/// The first line an optional component's executable must print when asked
/// for its version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedBanner {
    /// The component whose executable prints it.
    pub component: ManagedComponent,
    /// The reviewed prefix of the first line of its version output.
    pub first_line_prefix: String,
}

impl ReviewedCompatibilityPolicy {
    /// The reviewed banner prefix of `component`'s executable, for an
    /// optional component.
    #[must_use]
    pub fn optional_banner(&self, component: ManagedComponent) -> Option<&str> {
        self.optional_banners
            .iter()
            .find(|banner| banner.component == component)
            .map(|banner| banner.first_line_prefix.as_str())
    }
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
    /// Complete core component set in stable component order.
    pub artifacts: Vec<AcceptedManagedArtifact>,
    /// The optional components' artifacts, in install order
    /// ([`ManagedComponent::JsRuntimeDeno`], then [`ManagedComponent::YtDlp`]),
    /// or none on a catalogue that offers none. Never planned unless the user
    /// includes them.
    pub optional_artifacts: Vec<AcceptedManagedArtifact>,
    /// Compatibility checks that must pass before publication.
    pub compatibility: ReviewedCompatibilityPolicy,
    /// Earlier reviewed versions this catalogue's pins replace, so that a
    /// machine still running one can be told (never part of a plan digest).
    pub superseded: Vec<SupersededManagedVersion>,
}

impl AcceptedManagedCatalogue {
    /// The update to report when `selected_version` of `component` is what
    /// the managed store selects now.
    ///
    /// Only a version this catalogue lists as superseded yields one: any
    /// other version that differs from the pin (one a later release installed,
    /// say, before `VSift` itself was downgraded) is not called older, because
    /// nothing here says it is.
    #[must_use]
    pub fn update_for(
        &self,
        component: ManagedComponent,
        selected_version: &str,
    ) -> Option<ManagedUpdate> {
        let superseded = self
            .superseded
            .iter()
            .find(|entry| entry.component == component && entry.version == selected_version)?;
        let current = self
            .artifacts
            .iter()
            .chain(&self.optional_artifacts)
            .find(|artifact| artifact.component == component)?;
        (current.version != selected_version).then(|| ManagedUpdate {
            component,
            selected_version: superseded.version.clone(),
            reviewed_version: current.version.clone(),
            reason: superseded.reason,
        })
    }
}

/// Why the current pin of a component replaced an earlier reviewed version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupersessionReason {
    /// The current pin carries fixes for security defects the earlier
    /// version has (advisories, records or memory-safety fixes upstream).
    SecurityFixes,
    /// Any other reason: a refresh, a retention move or a non-security fix.
    Maintenance,
}

impl SupersessionReason {
    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::SecurityFixes => "security_fixes",
            Self::Maintenance => "maintenance",
        }
    }

    /// Every reason, in identifier order of the public contract.
    pub const ALL: [Self; 2] = [Self::SecurityFixes, Self::Maintenance];
}

/// An earlier reviewed version of one component and why it was replaced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupersededManagedVersion {
    /// The component the version belongs to.
    pub component: ManagedComponent,
    /// Its immutable managed-version key, as an earlier catalogue pinned it.
    pub version: String,
    /// Why the current pin replaced it.
    pub reason: SupersessionReason,
}

/// A managed version in use that the current catalogue supersedes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedUpdate {
    /// The component.
    pub component: ManagedComponent,
    /// The version the managed store selects now.
    pub selected_version: String,
    /// The version the current catalogue pins.
    pub reviewed_version: String,
    /// Why the pin moved.
    pub reason: SupersessionReason,
}

/// Why managed actions are or are not available in a setup plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedPlanAvailability {
    /// This target has an accepted, unexpired catalogue whose actions
    /// `setup install` applies once the plan is accepted.
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
            Self::Qualified => "catalogue_accepted",
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
    /// An accepted managed action supplies this dependency, which the probe
    /// showed to be missing or failing.
    ManagedInstall,
    /// The plan proposes nothing for this dependency and the next move is the
    /// user's: supply or install it outside managed setup, or, when its probe
    /// timed out and so showed nothing, check it again and plan again.
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
    /// The optional components this plan offers or includes, in plan order
    /// (#409). An included one's actions follow the core's; the core never
    /// depends on them.
    pub optional: Vec<OptionalComponentPlan>,
    /// Canonical SHA-256 acceptance digest, absent when managed setup is unavailable.
    pub digest: Option<String>,
    selection_state: SetupSelectionState,
    compatibility: Option<ReviewedCompatibilityPolicy>,
}

/// What `setup check` found about one optional component (ADR 0032):
/// whether its managed components are installed. It never changes the
/// readiness of the local-video profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptionalCapability {
    /// Every managed component of it is installed, selected and verified,
    /// at these versions, in install order.
    Installed(Vec<(ManagedComponent, String)>),
    /// It is offered on this host and not every part of it is installed.
    NotInstalled,
    /// No managed version of it is offered on this host.
    Unavailable,
}

impl OptionalCapability {
    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(&self) -> &'static str {
        match self {
            Self::Installed(_) => "installed",
            Self::NotInstalled => "not_installed",
            Self::Unavailable => "unavailable",
        }
    }
}

/// What a plan does about one optional component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptionalComponentDisposition {
    /// The user included it: its actions are in the plan and its digest.
    Included,
    /// Offered and left out. Whoever runs setup must tell the operator about
    /// it and ask before accepting the plan (#409).
    NotIncluded,
    /// No managed version can be planned on this host: it has no accepted,
    /// unexpired catalogue.
    Unavailable,
}

impl OptionalComponentDisposition {
    /// Stable public identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Included => "included",
            Self::NotIncluded => "not_included",
            Self::Unavailable => "unavailable",
        }
    }
}

/// One optional component of a plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionalComponentPlan {
    /// The optional component.
    pub component: OptionalComponent,
    /// Whether it is included, offered or unavailable here.
    pub disposition: OptionalComponentDisposition,
    /// The reviewed artifacts it would install, in install order: what an
    /// offer states (size, licence); empty when unavailable.
    pub artifacts: Vec<AcceptedManagedArtifact>,
    /// The actions that install it, only when included.
    pub actions: Vec<ManagedSetupAction>,
}

impl OptionalComponentPlan {
    /// The bytes its artifacts download, together.
    #[must_use]
    pub fn download_bytes(&self) -> u64 {
        self.artifacts
            .iter()
            .map(|artifact| artifact.integrity.bytes())
            .fold(0, u64::saturating_add)
    }
}

/// Which model a command would use now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservedModel {
    /// The user's configured model file.
    Configured,
    /// The managed model `setup install` selected.
    Managed,
    /// No model at all.
    Missing,
}

/// What the machine shows now, beside a plan's intent (P13).
///
/// A plan's digest binds its intent: the components, versions, sources,
/// sizes and digests the catalogue fixes, and the observations of the tools
/// outside `VSift`'s managed store that decide which components are needed.
/// Installing a managed component changes none of that, so an accepted plan
/// stays acceptable while `setup install` applies it and a rerun continues.
/// This observation is what commands would use now, managed tier included:
/// it is shown with the plan and never digested.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedPlanObservation {
    /// Aggregate readiness as commands resolve the tools now.
    pub readiness: RuntimeReadiness,
    /// Each dependency as commands resolve it now, in stable order.
    pub dependencies: Vec<DependencyStatus>,
    /// The model a command would use.
    pub model: ObservedModel,
    /// The plan's components whose reviewed version is selected now.
    pub current: Vec<ManagedComponent>,
}

impl ManagedPlanObservation {
    /// The observation of a machine with no managed tier: what the plan's
    /// own probes saw.
    #[must_use]
    pub fn without_managed_tier(plan: &ManagedSetupPlan) -> Self {
        Self {
            readiness: plan.readiness,
            dependencies: plan
                .dependencies
                .iter()
                .map(|(status, _)| status.clone())
                .collect(),
            model: if plan.selection_state.model.is_some() {
                ObservedModel::Configured
            } else {
                ObservedModel::Missing
            },
            current: Vec::new(),
        }
    }

    /// Whether `component`'s reviewed version is selected now.
    #[must_use]
    pub fn is_current(&self, component: ManagedComponent) -> bool {
        self.current.contains(&component)
    }
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
    /// Whoever runs setup must ask the operator before accepting this plan:
    /// it offers or includes an optional component (#409). The instruction
    /// is in the plan itself, so an agent with no skill relays it too.
    #[must_use]
    pub fn operator_decision_required(&self) -> bool {
        self.optional.iter().any(|optional| {
            matches!(
                optional.disposition,
                OptionalComponentDisposition::Included | OptionalComponentDisposition::NotIncluded
            )
        })
    }

    /// Every optional action the plan includes, in install order.
    pub fn optional_actions(&self) -> impl Iterator<Item = &ManagedSetupAction> {
        self.optional
            .iter()
            .flat_map(|optional| optional.actions.iter())
    }

    /// The optional components the plan includes.
    #[must_use]
    pub fn included(&self) -> OptionalComponentSet {
        self.optional
            .iter()
            .filter(|optional| optional.disposition == OptionalComponentDisposition::Included)
            .fold(OptionalComponentSet::EMPTY, |set, optional| {
                set.with(optional.component)
            })
    }

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

    /// What this plan's actions do about the managed build that supplies
    /// `dependency`.
    ///
    /// A dependency's own disposition does not answer this. `FFmpeg` and
    /// `FFprobe` come in one build, so a plan that installs it because one of
    /// them is missing also installs the other, whatever the other's probe
    /// showed; and a managed version is used ahead of a tool found on `PATH`.
    /// What a plan says about a tool it decided nothing for must be true of
    /// that too (issue #391).
    #[must_use]
    pub fn supply_of(&self, dependency: RuntimeDependency) -> PlannedSupply {
        let supplier = supplying_component(dependency);
        if self
            .actions
            .iter()
            .any(|action| action.artifact.component == supplier)
        {
            PlannedSupply::Installed
        } else {
            PlannedSupply::NotInstalled
        }
    }
}

/// What a plan's actions do about the managed build that supplies one
/// dependency, whatever that dependency's own disposition is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlannedSupply {
    /// An action of the plan installs the build, so once the plan is applied
    /// the managed tool is there and is used ahead of one found on `PATH`.
    Installed,
    /// No action of the plan installs it.
    NotInstalled,
}

/// The managed component whose build supplies `dependency`.
const fn supplying_component(dependency: RuntimeDependency) -> ManagedComponent {
    match dependency {
        RuntimeDependency::Ffmpeg | RuntimeDependency::Ffprobe => ManagedComponent::MediaTools,
        RuntimeDependency::Whisper => ManagedComponent::WhisperCli,
    }
}

/// The step of a compatibility smoke that stopped it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilitySmokeCheck {
    /// The staged files, their executable format and their reviewed modes,
    /// checked before any process runs.
    Layout,
    /// Each staged executable's reviewed build banner or startup response.
    Banner,
    /// The reviewed media fixture through the media tools.
    MediaFixture,
    /// The reviewed speech fixture through the recognizer and model.
    SpeechFixture,
    /// The staged files rechecked, and the private smoke directory removed,
    /// after every process ended.
    Recheck,
}

impl CompatibilitySmokeCheck {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Layout => "layout",
            Self::Banner => "banner",
            Self::MediaFixture => "media_fixture",
            Self::SpeechFixture => "speech_fixture",
            Self::Recheck => "recheck",
        }
    }
}

/// Why a compatibility smoke did not pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilitySmokeFailureReason {
    /// A required executable or model file is not in the staged runtime and
    /// no already selected provider supplies it.
    MissingExecutable,
    /// An executable is built for another machine or operating system.
    WrongArchitecture,
    /// A file that must run is not reviewed as executable, is not in any
    /// executable format, or the operating system refused to run it.
    NotExecutable,
    /// A build banner differs from the reviewed prefix.
    BannerMismatch,
    /// A process wrote, or a check generated, more than a reviewed bound.
    OutputOverBound,
    /// A process or check exceeded its reviewed deadline.
    DeadlineExceeded,
    /// The staged runtime or the smoke directory holds an entry the review
    /// does not name.
    UnexpectedExtraFile,
    /// A staged file's bytes, links or permissions differ from the review.
    ChangedContent,
    /// A provider could not start, crashed, exited unsuccessfully or rejected
    /// the fixture.
    ProviderFailed,
    /// A provider could not start because a shared library it needs is not
    /// installed on this machine, and its own error output named the library
    /// in the operating system loader's fixed words (#256). It is
    /// [`Self::ProviderFailed`] for every public purpose (the same
    /// [`Self::identifier`], the same failure code); the name is carried only
    /// so the remediation can say what to install.
    MissingSharedLibrary(SharedLibraryName),
    /// A provider ran, but its results differ from the fixture's recorded
    /// truth.
    FixtureMismatch,
    /// `VSift`'s own smoke directory or embedded fixture could not be
    /// prepared; the candidate is not at fault.
    Preparation,
    /// The request named no candidate, or the same component twice.
    InvalidRequest,
    /// The caller cancelled the smoke.
    Cancelled,
}

impl CompatibilitySmokeFailureReason {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::MissingExecutable => "missing_executable",
            Self::WrongArchitecture => "wrong_architecture",
            Self::NotExecutable => "not_executable",
            Self::BannerMismatch => "banner_mismatch",
            Self::OutputOverBound => "output_over_bound",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::UnexpectedExtraFile => "unexpected_extra_file",
            Self::ChangedContent => "changed_content",
            Self::ProviderFailed | Self::MissingSharedLibrary(_) => "provider_failed",
            Self::FixtureMismatch => "fixture_mismatch",
            Self::Preparation => "preparation",
            Self::InvalidRequest => "invalid_request",
            Self::Cancelled => "cancelled",
        }
    }

    /// The shared library a provider could not start without, when its own
    /// error output named one.
    #[must_use]
    pub const fn missing_shared_library(self) -> Option<SharedLibraryName> {
        match self {
            Self::MissingSharedLibrary(name) => Some(name),
            _ => None,
        }
    }
}

/// The first step that failed and why.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibilitySmokeFailure {
    /// Step that did not pass; later steps did not run.
    pub check: CompatibilitySmokeCheck,
    /// Why it did not pass.
    pub reason: CompatibilitySmokeFailureReason,
}

/// Outcome of running the reviewed compatibility smoke over staged components.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilitySmokeVerdict {
    /// Every check passed and the staged files are unchanged afterwards.
    Passed,
    /// A check failed.
    Failed(CompatibilitySmokeFailure),
}

/// Why a staged component was left in place instead of being removed.
///
/// Cleanup removes only content it can positively identify as `VSift`'s own
/// staging (AGENTS.md); anything it cannot prove is kept for explicit repair
/// and reported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageRetentionReason {
    /// The stage's ownership marker, private owner or held identity could not
    /// be proved, so nothing in it was touched.
    OwnershipUnproved,
    /// The stage holds an entry the review does not name, or a linked or
    /// changed one, so that entry and its directory were kept.
    UnexpectedContent,
    /// Storage failed while removing positively identified content.
    StorageFailure,
}

impl StageRetentionReason {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::OwnershipUnproved => "ownership_unproved",
            Self::UnexpectedContent => "unexpected_content",
            Self::StorageFailure => "storage_failure",
        }
    }
}

/// What cleanup did with one staged component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageDisposal {
    /// Every positively identified file and directory of the stage was removed.
    Discarded,
    /// The stage, or part of it, was kept and is reported.
    Retained(StageRetentionReason),
}

/// One reviewed component staged privately and not activated.
///
/// Infrastructure owns the files; the application only sees which component
/// it is and can discard it. Nothing on this port activates a component, so
/// a candidate that fails its smoke is consumed by cleanup and can never be
/// published.
pub trait StagedManagedComponent: Send + Sync {
    /// The reviewed component this stage holds.
    fn component(&self) -> ManagedComponent;

    /// Removes the stage if its ownership and every entry can be proved
    /// `VSift`'s own; otherwise keeps what it cannot prove and reports why.
    fn discard(self) -> StageDisposal
    where
        Self: Sized;
}

/// Port that runs the reviewed compatibility smoke over staged components
/// before any of them is activated.
///
/// Implementations run the staged executables explicitly, never through a
/// shell, under the digest-bound [`ReviewedCompatibilityPolicy`], and never
/// publish, select or remove anything.
pub trait CompatibilitySmoke<C: StagedManagedComponent>: Send + Sync {
    /// Checks `candidates` together: a component that needs another (the
    /// recognizer needs media tools and a model) uses the staged one when
    /// present.
    fn smoke(&self, candidates: &[C]) -> impl Future<Output = CompatibilitySmokeVerdict> + Send;
}

/// The result of smoking staged components before activation.
#[derive(Debug, Eq, PartialEq)]
pub enum SmokeStageOutcome<C> {
    /// The smoke passed. The candidates come back unchanged and still
    /// unactivated; publication is a separate, guarded step.
    Passed(Vec<C>),
    /// The smoke failed, and every candidate was handed to cleanup.
    Failed {
        /// First failing step and its reason.
        failure: CompatibilitySmokeFailure,
        /// What cleanup did with each candidate, in request order.
        stages: Vec<(ManagedComponent, StageDisposal)>,
    },
}

/// Runs the compatibility smoke and discards every candidate when it fails.
///
/// No candidate is published here, whatever the verdict. A failed smoke
/// cannot say which of several candidates is at fault, and none may be
/// published without a passing smoke, so all of them are discarded; a stage
/// whose ownership or content cannot be proved is retained and reported
/// rather than deleted.
pub async fn smoke_before_activation<C, S>(smoke: &S, candidates: Vec<C>) -> SmokeStageOutcome<C>
where
    C: StagedManagedComponent,
    S: CompatibilitySmoke<C>,
{
    let verdict = if candidates.is_empty() || has_duplicate_component(&candidates) {
        CompatibilitySmokeVerdict::Failed(CompatibilitySmokeFailure {
            check: CompatibilitySmokeCheck::Layout,
            reason: CompatibilitySmokeFailureReason::InvalidRequest,
        })
    } else {
        smoke.smoke(&candidates).await
    };
    match verdict {
        CompatibilitySmokeVerdict::Passed => SmokeStageOutcome::Passed(candidates),
        CompatibilitySmokeVerdict::Failed(failure) => SmokeStageOutcome::Failed {
            failure,
            stages: candidates
                .into_iter()
                .map(|candidate| (candidate.component(), candidate.discard()))
                .collect(),
        },
    }
}

fn has_duplicate_component<C: StagedManagedComponent>(candidates: &[C]) -> bool {
    candidates.iter().enumerate().any(|(index, candidate)| {
        candidates
            .iter()
            .skip(index + 1)
            .any(|other| other.component() == candidate.component())
    })
}

/// Builds a deterministic, non-mutating plan from current observations,
/// with no optional component included.
#[must_use]
pub fn plan_managed_setup(
    profile: SetupProfile,
    diagnosis: RuntimeDiagnosis,
    target: ManagedTarget,
    selection_state: SetupSelectionState,
    now_unix_seconds: u64,
    catalogue: Option<AcceptedManagedCatalogue>,
) -> ManagedSetupPlan {
    plan_managed_setup_including(
        profile,
        diagnosis,
        target,
        selection_state,
        OptionalComponentSet::EMPTY,
        now_unix_seconds,
        catalogue,
    )
}

/// Builds a deterministic, non-mutating plan from current observations,
/// with the optional components in `include` (#409).
///
/// An included component is planned after the core, always: whether it is
/// installed already is the transaction's `already_current`, so the plan's
/// intent does not depend on the managed store. A desktop plan lists every
/// optional component it leaves out, so the operator is told it exists; a
/// worker plan lists only what was included.
#[must_use]
pub fn plan_managed_setup_including(
    profile: SetupProfile,
    diagnosis: RuntimeDiagnosis,
    target: ManagedTarget,
    selection_state: SetupSelectionState,
    include: OptionalComponentSet,
    now_unix_seconds: u64,
    catalogue: Option<AcceptedManagedCatalogue>,
) -> ManagedSetupPlan {
    plan_managed_setup_of_parts(
        profile,
        diagnosis,
        target,
        selection_state,
        CorePartSet::ALL,
        include,
        now_unix_seconds,
        catalogue,
    )
}

/// [`plan_managed_setup_including`] limited to the core `parts` (ADR 0036
/// section 6): a part left out is offered nothing, so its tools are
/// planned as for a host with no catalogue for them. A plan of
/// [`CorePartSet::ALL`] is exactly the plan `setup plan` builds.
#[allow(
    clippy::too_many_arguments,
    reason = "the inputs of one plan, each a distinct observation or choice"
)]
#[must_use]
pub fn plan_managed_setup_of_parts(
    profile: SetupProfile,
    diagnosis: RuntimeDiagnosis,
    target: ManagedTarget,
    selection_state: SetupSelectionState,
    parts: CorePartSet,
    include: OptionalComponentSet,
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
    // Only a probe that showed a tool unusable (missing, or found and failing)
    // is a reason to install one. A probe that timed out shows nothing: the
    // tool was found and may be perfectly healthy, as a large executable is on
    // its first start after a restart (issue #391). Planning a download for it
    // would replace a working tool on the strength of one slow start, so a
    // timeout plans nothing, and its own disposition below says the user has to
    // look again.
    let media_missing = diagnosis.dependencies.iter().any(|status| {
        matches!(
            status.dependency,
            RuntimeDependency::Ffmpeg | RuntimeDependency::Ffprobe
        ) && status.state.is_unusable()
    });
    let whisper_missing = diagnosis.dependencies.iter().any(|status| {
        status.dependency == RuntimeDependency::Whisper && status.state.is_unusable()
    });
    let CoreOffer {
        media: media_offered,
        whisper: whisper_offered,
        model: model_offered,
    } = CoreOffer::of(catalogue.as_ref().filter(|_| qualified), &diagnosis);
    let media_offered = media_offered && parts.contains(CorePart::Video);
    let whisper_offered = whisper_offered && parts.contains(CorePart::Speech);
    let model_offered = model_offered && parts.contains(CorePart::Speech);

    let dependencies = diagnosis
        .dependencies
        .into_iter()
        .map(|status| {
            let needs_media = matches!(
                status.dependency,
                RuntimeDependency::Ffmpeg | RuntimeDependency::Ffprobe
            ) && media_missing
                && media_offered;
            let needs_whisper = status.dependency == RuntimeDependency::Whisper
                && whisper_missing
                && whisper_offered;
            let disposition = if status.state.is_available() {
                SetupDependencyDisposition::ExistingProbeOnly
            } else if qualified && status.state.is_unusable() && (needs_media || needs_whisper) {
                SetupDependencyDisposition::ManagedInstall
            } else {
                SetupDependencyDisposition::ManualSelection
            };
            (status, disposition)
        })
        .collect::<Vec<_>>();

    let model = if selection_state.model.is_some() {
        SetupModelDisposition::ConfiguredProbeOnly
    } else if model_offered {
        SetupModelDisposition::ManagedInstall
    } else {
        SetupModelDisposition::ManualSelection
    };

    let wanted = |component: ManagedComponent| match component {
        ManagedComponent::MediaTools => media_missing && media_offered,
        ManagedComponent::WhisperCli => whisper_missing && whisper_offered,
        ManagedComponent::WhisperModel => selection_state.model.is_none() && model_offered,
        // The core list holds no optional component (the catalogue check
        // refuses one); optional ones are planned below.
        ManagedComponent::JsRuntimeDeno | ManagedComponent::YtDlp => false,
    };
    let actions = core_actions(catalogue.as_ref().filter(|_| qualified), wanted);

    let optional =
        plan_optional_components(profile, include, catalogue.as_ref().filter(|_| qualified));
    let optional_required = optional.iter().any(|optional| !optional.actions.is_empty());
    let availability = if qualified && actions.is_empty() && !optional_required {
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
        optional,
        digest: None,
        compatibility: catalogue.as_ref().map(|entry| entry.compatibility.clone()),
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

/// The actions that install the core components `wanted` accepts, in the
/// catalogue's order.
fn core_actions(
    catalogue: Option<&AcceptedManagedCatalogue>,
    wanted: impl Fn(ManagedComponent) -> bool,
) -> Vec<ManagedSetupAction> {
    catalogue
        .map(|entry| {
            entry
                .artifacts
                .iter()
                .filter(|artifact| wanted(artifact.component))
                .map(|artifact| ManagedSetupAction {
                    id: format!("install-{}", artifact.component.identifier()),
                    artifact: artifact.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Which core components a plan may install on this host.
struct CoreOffer {
    media: bool,
    whisper: bool,
    model: bool,
}

impl CoreOffer {
    /// What `catalogue` (`None` when managed installation is not qualified)
    /// offers. A target's catalogue may offer only some core components
    /// (ADR 0032: one whose reviewed build is not pinned yet stays the
    /// user's to supply), and the model is smoked with a whisper.cpp, the
    /// managed one or one that already works here.
    fn of(catalogue: Option<&AcceptedManagedCatalogue>, diagnosis: &RuntimeDiagnosis) -> Self {
        let offers = |component: ManagedComponent| {
            catalogue.is_some_and(|entry| {
                entry
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.component == component)
            })
        };
        let whisper_available = diagnosis.dependencies.iter().any(|status| {
            status.dependency == RuntimeDependency::Whisper && status.state.is_available()
        });
        let whisper = offers(ManagedComponent::WhisperCli);
        Self {
            media: offers(ManagedComponent::MediaTools),
            whisper,
            model: offers(ManagedComponent::WhisperModel) && (whisper || whisper_available),
        }
    }
}

/// The optional part of a plan: an entry per optional component the plan
/// includes, and, for the desktop profile, per one it leaves out.
fn plan_optional_components(
    profile: SetupProfile,
    include: OptionalComponentSet,
    qualified: Option<&AcceptedManagedCatalogue>,
) -> Vec<OptionalComponentPlan> {
    OptionalComponent::ALL
        .into_iter()
        .filter(|component| profile == SetupProfile::Desktop || include.contains(*component))
        .map(|component| {
            let artifacts: Vec<AcceptedManagedArtifact> = qualified
                .map(|entry| {
                    component
                        .components()
                        .iter()
                        .filter_map(|wanted| {
                            entry
                                .optional_artifacts
                                .iter()
                                .find(|artifact| artifact.component == *wanted)
                                .cloned()
                        })
                        .collect()
                })
                .unwrap_or_default();
            let available = artifacts.len() == component.components().len();
            let disposition = if !available {
                OptionalComponentDisposition::Unavailable
            } else if include.contains(component) {
                OptionalComponentDisposition::Included
            } else {
                OptionalComponentDisposition::NotIncluded
            };
            let actions = if disposition == OptionalComponentDisposition::Included {
                artifacts
                    .iter()
                    .map(|artifact| ManagedSetupAction {
                        id: format!("install-{}", artifact.component.identifier()),
                        artifact: artifact.clone(),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            OptionalComponentPlan {
                component,
                disposition,
                artifacts: if available { artifacts } else { Vec::new() },
                actions,
            }
        })
        .collect()
}

fn catalogue_is_complete(entry: &AcceptedManagedCatalogue) -> bool {
    // Each optional artifact belongs to an optional component, appears once
    // and is complete, with its reviewed banner. A group the catalogue does
    // not complete is unavailable in the plan, not an invalid catalogue.
    let optional_complete = entry
        .optional_artifacts
        .iter()
        .enumerate()
        .all(|(index, artifact)| {
            OptionalComponent::of(artifact.component).is_some()
                && entry.optional_artifacts[..index]
                    .iter()
                    .all(|earlier| earlier.component != artifact.component)
                && artifact_is_complete(artifact)
                && entry
                    .compatibility
                    .optional_banner(artifact.component)
                    .is_some_and(|banner| !banner.is_empty())
        });
    // The core components a target's catalogue offers, in plan order, each
    // once: all three, or fewer while a target's reviewed build of one is
    // not pinned yet (ADR 0032).
    let order = [
        ManagedComponent::MediaTools,
        ManagedComponent::WhisperCli,
        ManagedComponent::WhisperModel,
    ];
    let ranks: Vec<usize> = entry
        .artifacts
        .iter()
        .filter_map(|artifact| order.iter().position(|core| *core == artifact.component))
        .collect();
    !entry.artifacts.is_empty()
        && ranks.len() == entry.artifacts.len()
        && ranks.windows(2).all(|pair| pair[0] < pair[1])
        && compatibility_is_complete(&entry.compatibility)
        && entry.artifacts.iter().all(artifact_is_complete)
        && optional_complete
}

fn artifact_is_complete(artifact: &AcceptedManagedArtifact) -> bool {
    !artifact.files.is_empty()
        && !artifact.version.is_empty()
        && !artifact.source_url.is_empty()
        && !artifact.trust_limit.is_empty()
        && match artifact.format {
            ManagedArtifactFormat::TarXz
            | ManagedArtifactFormat::TarGz
            | ManagedArtifactFormat::TarBz2 => {
                artifact.archive_limits.is_some() && !artifact.selected_files.is_empty()
            }
            // A zip's links are refused by its reader, so none is reviewed.
            ManagedArtifactFormat::Zip => {
                artifact.archive_limits.is_some()
                    && !artifact.selected_files.is_empty()
                    && artifact.archive_links.is_empty()
            }
            ManagedArtifactFormat::RawFile => {
                artifact.archive_limits.is_none()
                    && artifact.selected_files.is_empty()
                    && artifact.archive_links.is_empty()
                    && artifact.runtime_copies.is_empty()
            }
        }
}

fn compatibility_is_complete(policy: &ReviewedCompatibilityPolicy) -> bool {
    !policy.expected_ffmpeg_version.is_empty()
        && !policy.expected_ffprobe_version.is_empty()
        && policy.stream_limit_bytes > 0
        && policy.stream_limit_bytes <= 64 * 1024
        && policy.audio_file_limit_bytes > 44
        && policy.audio_file_limit_bytes <= 1024 * 1024
        && policy.transcript_file_limit_bytes > 0
        && policy.transcript_file_limit_bytes <= 1024 * 1024
        && policy.media_deadline_seconds > 0
        && policy.media_deadline_seconds <= 60
        && policy.inference_deadline_seconds > 0
        && policy.inference_deadline_seconds <= 300
        && policy.audio_sample_rate_hz == 16_000
        && policy.audio_channels == 1
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
            // Which kind of timeout it was, and under which deadline, is not
            // bound: the plan's text for a timed-out probe is the same for
            // both kinds, and a deadline is not an observation of the tool.
            vsift_domain::DependencyState::Missing | vsift_domain::DependencyState::TimedOut(_) => {
            }
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
    if let Some(policy) = &plan.compatibility {
        digest_field(&mut digest, &policy.fixture.bytes().to_string());
        digest_field(&mut digest, &policy.fixture.sha256_hex());
        digest_field(&mut digest, &policy.expected_ffmpeg_version);
        digest_field(&mut digest, &policy.expected_ffprobe_version);
        digest_field(&mut digest, &policy.stream_limit_bytes.to_string());
        digest_field(&mut digest, &policy.audio_file_limit_bytes.to_string());
        digest_field(&mut digest, &policy.transcript_file_limit_bytes.to_string());
        digest_field(&mut digest, &policy.media_deadline_seconds.to_string());
        digest_field(&mut digest, &policy.inference_deadline_seconds.to_string());
        digest_field(&mut digest, &policy.audio_sample_rate_hz.to_string());
        digest_field(&mut digest, &policy.audio_channels.to_string());
    }
    for action in &plan.actions {
        digest_field(&mut digest, &action.id);
        digest_artifact(&mut digest, &action.artifact);
    }
    // The optional part is bound only when something is included, so a plan
    // without it is digested exactly as before #409. What a plan offers and
    // leaves out is fixed by the catalogue revision, which is bound above.
    for optional in plan
        .optional
        .iter()
        .filter(|optional| optional.disposition == OptionalComponentDisposition::Included)
    {
        digest_field(&mut digest, "optional");
        digest_field(&mut digest, optional.component.identifier());
        for action in &optional.actions {
            digest_field(&mut digest, &action.id);
            digest_artifact(&mut digest, &action.artifact);
            if let Some(policy) = &plan.compatibility {
                digest_field(
                    &mut digest,
                    policy
                        .optional_banner(action.artifact.component)
                        .unwrap_or(""),
                );
            }
        }
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
    use std::time::Duration;

    use vsift_domain::{
        ArtifactIntegrity, DependencyState, DependencyStatus, ManagedArtifactFormat,
        ManagedComponent, ManagedTarget, ProbeTimeout, ProbeTimeoutKind, RuntimeDependency,
        RuntimeReadiness,
    };

    use super::{
        AcceptedManagedArtifact, AcceptedManagedCatalogue, ManagedPlanAvailability, ManagedUpdate,
        PlanAcceptanceError, PlannedSupply, ReviewedCompatibilityPolicy, ReviewedManagedFile,
        SetupDependencyDisposition, SetupProfile, SetupSelectionState, SupersededManagedVersion,
        SupersessionReason, plan_managed_setup,
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
            compatibility: ReviewedCompatibilityPolicy {
                fixture: integrity('f')?,
                expected_ffmpeg_version: String::from("ffmpeg version fixture"),
                expected_ffprobe_version: String::from("ffprobe version fixture"),
                stream_limit_bytes: 64 * 1024,
                audio_file_limit_bytes: 256 * 1024,
                transcript_file_limit_bytes: 64 * 1024,
                media_deadline_seconds: 60,
                inference_deadline_seconds: 180,
                audio_sample_rate_hz: 16_000,
                audio_channels: 1,
                optional_banners: Vec::new(),
            },
            optional_artifacts: Vec::new(),
            superseded: Vec::new(),
        })
    }

    /// Only a version the catalogue lists as superseded, for its own
    /// component, is reported, with the pin that replaces it and the reason;
    /// the pin itself and an unknown version are not.
    #[test]
    fn only_a_listed_superseded_version_is_an_update() -> Result<(), Box<dyn std::error::Error>> {
        let mut catalogue = catalogue()?;
        catalogue.superseded = vec![
            SupersededManagedVersion {
                component: ManagedComponent::WhisperCli,
                version: String::from("old-1"),
                reason: SupersessionReason::SecurityFixes,
            },
            SupersededManagedVersion {
                component: ManagedComponent::MediaTools,
                version: String::from("old-0"),
                reason: SupersessionReason::Maintenance,
            },
        ];
        assert_eq!(
            catalogue.update_for(ManagedComponent::WhisperCli, "old-1"),
            Some(ManagedUpdate {
                component: ManagedComponent::WhisperCli,
                selected_version: String::from("old-1"),
                reviewed_version: String::from("version-1"),
                reason: SupersessionReason::SecurityFixes,
            })
        );
        assert_eq!(
            catalogue
                .update_for(ManagedComponent::MediaTools, "old-0")
                .map(|update| update.reason),
            Some(SupersessionReason::Maintenance)
        );
        for (component, version) in [
            (ManagedComponent::WhisperCli, "version-1"),
            (ManagedComponent::WhisperCli, "newer-than-the-pin"),
            (ManagedComponent::MediaTools, "old-1"),
            (ManagedComponent::WhisperModel, "old-1"),
        ] {
            assert_eq!(catalogue.update_for(component, version), None, "{version}");
        }
        assert_eq!(
            SupersessionReason::ALL.map(SupersessionReason::identifier),
            ["security_fixes", "maintenance"]
        );
        Ok(())
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
        assert_eq!(
            available.actions,
            [] as [crate::provisioning::ManagedSetupAction; 0]
        );
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
        assert_eq!(
            unsupported.actions,
            [] as [crate::provisioning::ManagedSetupAction; 0]
        );
        assert_eq!(
            expired.actions,
            [] as [crate::provisioning::ManagedSetupAction; 0]
        );
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
        let mut changed_compatibility = catalogue()?;
        changed_compatibility
            .compatibility
            .inference_deadline_seconds = 181;
        let changed_policy = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&[RuntimeDependency::Ffmpeg]),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_000,
            Some(changed_compatibility),
        );

        assert_eq!(
            selected.validate_acceptance(prior),
            Err(PlanAcceptanceError::DigestMismatch)
        );
        assert_eq!(
            changed.validate_acceptance(prior),
            Err(PlanAcceptanceError::DigestMismatch)
        );
        assert_eq!(
            changed_policy.validate_acceptance(prior),
            Err(PlanAcceptanceError::DigestMismatch)
        );
        Ok(())
    }

    #[test]
    fn an_inconsistent_core_catalogue_cannot_produce_actions_or_digest()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut reversed = catalogue()?;
        reversed.artifacts.reverse();
        let mut twice = catalogue()?;
        let first = twice.artifacts[0].clone();
        twice.artifacts.push(first);
        let mut empty = catalogue()?;
        empty.artifacts.clear();
        for invalid in [reversed, twice, empty] {
            let plan = plan_managed_setup(
                SetupProfile::Desktop,
                diagnosis(&RuntimeDependency::ALL),
                ManagedTarget::Ubuntu2404X86_64,
                SetupSelectionState::default(),
                1_000,
                Some(invalid),
            );
            assert_eq!(plan.availability, ManagedPlanAvailability::CatalogueInvalid);
            assert_eq!(
                plan.actions,
                [] as [crate::provisioning::ManagedSetupAction; 0]
            );
            assert!(plan.digest.is_none());
        }
        Ok(())
    }

    /// ADR 0032: a target's catalogue may offer only some core components
    /// while the reviewed build of another is not pinned yet. The plan
    /// installs what it offers, leaves the rest to the user, and plans the
    /// model only when a whisper.cpp can smoke it.
    #[test]
    fn a_partial_core_catalogue_plans_what_it_offers_and_leaves_the_rest()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut partial = catalogue()?;
        partial
            .artifacts
            .retain(|artifact| artifact.component != ManagedComponent::WhisperCli);
        let plan = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&RuntimeDependency::ALL),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_000,
            Some(partial.clone()),
        );
        assert_eq!(plan.availability, ManagedPlanAvailability::Qualified);
        let planned: Vec<ManagedComponent> = plan
            .actions
            .iter()
            .map(|action| action.artifact.component)
            .collect();
        // Nothing can smoke the model: no managed or working whisper.cpp.
        assert_eq!(planned, [ManagedComponent::MediaTools]);
        for (status, disposition) in &plan.dependencies {
            let expected = if status.dependency == RuntimeDependency::Whisper {
                SetupDependencyDisposition::ManualSelection
            } else {
                SetupDependencyDisposition::ManagedInstall
            };
            assert_eq!(*disposition, expected, "{:?}", status.dependency);
        }
        assert_eq!(plan.model, super::SetupModelDisposition::ManualSelection);

        // With the user's own whisper.cpp working, the model is planned.
        let plan = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&[RuntimeDependency::Ffmpeg, RuntimeDependency::Ffprobe]),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_000,
            Some(partial),
        );
        let planned: Vec<ManagedComponent> = plan
            .actions
            .iter()
            .map(|action| action.artifact.component)
            .collect();
        assert_eq!(
            planned,
            [ManagedComponent::MediaTools, ManagedComponent::WhisperModel]
        );
        assert_eq!(plan.model, super::SetupModelDisposition::ManagedInstall);
        Ok(())
    }

    #[test]
    fn invalid_compatibility_policy_cannot_produce_actions_or_digest()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut invalid = catalogue()?;
        invalid.compatibility.stream_limit_bytes = 0;
        let plan = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis(&RuntimeDependency::ALL),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_000,
            Some(invalid),
        );
        assert_eq!(plan.availability, ManagedPlanAvailability::CatalogueInvalid);
        assert_eq!(
            plan.actions,
            [] as [crate::provisioning::ManagedSetupAction; 0]
        );
        assert!(plan.digest.is_none());
        Ok(())
    }

    fn timed_out(kind: ProbeTimeoutKind) -> DependencyState {
        DependencyState::TimedOut(ProbeTimeout {
            kind,
            shared_deadline: Duration::from_secs(30),
        })
    }

    /// A blocked diagnosis with these states, in the stable dependency order.
    fn diagnosis_of(states: [DependencyState; 3]) -> RuntimeDiagnosis {
        RuntimeDiagnosis {
            readiness: RuntimeReadiness::Blocked,
            dependencies: RuntimeDependency::ALL
                .into_iter()
                .zip(states)
                .map(|(dependency, state)| DependencyStatus { dependency, state })
                .collect(),
        }
    }

    fn planned_components(plan: &super::ManagedSetupPlan) -> Vec<ManagedComponent> {
        plan.actions
            .iter()
            .map(|action| action.artifact.component)
            .collect()
    }

    fn with_model() -> SetupSelectionState {
        SetupSelectionState {
            model: Some(String::from("/fixture/model")),
            ..SetupSelectionState::default()
        }
    }

    /// Issue #391: where managed installation is available, a tool whose probe
    /// only timed out was planned for a download exactly like a missing one. A
    /// healthy tool that was slow to start once is not a reason to install
    /// another: a timeout plans nothing for its tool, whichever way the
    /// timeout happened, and leaves the next move with the user.
    #[test]
    fn a_probe_that_timed_out_plans_no_install_for_its_tool()
    -> Result<(), Box<dyn std::error::Error>> {
        // The reported shape: the first tool does not answer in time and the
        // two after it are never started.
        let plan = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis_of([
                timed_out(ProbeTimeoutKind::NoAnswer),
                timed_out(ProbeTimeoutKind::NotStarted),
                timed_out(ProbeTimeoutKind::NotStarted),
            ]),
            ManagedTarget::Ubuntu2404X86_64,
            with_model(),
            1_000,
            Some(catalogue()?),
        );
        assert!(plan.actions.is_empty(), "{:?}", planned_components(&plan));
        assert_eq!(plan.readiness, RuntimeReadiness::Blocked);
        for (status, disposition) in &plan.dependencies {
            assert_eq!(
                *disposition,
                SetupDependencyDisposition::ManualSelection,
                "{:?}",
                status.dependency
            );
            assert_eq!(
                plan.supply_of(status.dependency),
                PlannedSupply::NotInstalled,
                "{:?}",
                status.dependency
            );
        }

        // Without a configured model the model is still planned, as it always
        // was whatever the tools do, and nothing else is.
        let without_model = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis_of([
                timed_out(ProbeTimeoutKind::NoAnswer),
                timed_out(ProbeTimeoutKind::NotStarted),
                timed_out(ProbeTimeoutKind::NotStarted),
            ]),
            ManagedTarget::Ubuntu2404X86_64,
            SetupSelectionState::default(),
            1_000,
            Some(catalogue()?),
        );
        assert_eq!(
            planned_components(&without_model),
            [ManagedComponent::WhisperModel]
        );
        Ok(())
    }

    /// A timeout decides nothing in either direction: a tool that is really
    /// missing is still planned, with the tool it is paired with, while the
    /// timed-out one keeps its own disposition; and a tool that was found and
    /// failed is planned as before.
    #[test]
    fn a_timeout_beside_a_missing_or_failing_tool_changes_only_its_own_disposition()
    -> Result<(), Box<dyn std::error::Error>> {
        let available = || DependencyState::Available {
            version: String::from("fixture"),
        };
        let beside_missing = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis_of([
                DependencyState::Missing,
                timed_out(ProbeTimeoutKind::NoAnswer),
                timed_out(ProbeTimeoutKind::NotStarted),
            ]),
            ManagedTarget::Ubuntu2404X86_64,
            with_model(),
            1_000,
            Some(catalogue()?),
        );
        assert_eq!(
            planned_components(&beside_missing),
            [ManagedComponent::MediaTools]
        );
        assert_eq!(
            beside_missing
                .dependencies
                .iter()
                .map(|(_, disposition)| *disposition)
                .collect::<Vec<_>>(),
            [
                SetupDependencyDisposition::ManagedInstall,
                SetupDependencyDisposition::ManualSelection,
                SetupDependencyDisposition::ManualSelection,
            ]
        );
        // The pair's build is installed because one of the two is missing, so
        // the plan does install what supplies the timed-out one, whatever its
        // own disposition says; nothing is installed for the third tool.
        assert_eq!(
            RuntimeDependency::ALL.map(|dependency| beside_missing.supply_of(dependency)),
            [
                PlannedSupply::Installed,
                PlannedSupply::Installed,
                PlannedSupply::NotInstalled,
            ]
        );

        let failing = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis_of([
                DependencyState::Unhealthy {
                    message: String::from("process exited with exit code: 3"),
                },
                available(),
                DependencyState::Unhealthy {
                    message: String::from("process exited with exit code: 3"),
                },
            ]),
            ManagedTarget::Ubuntu2404X86_64,
            with_model(),
            1_000,
            Some(catalogue()?),
        );
        assert_eq!(
            planned_components(&failing),
            [ManagedComponent::MediaTools, ManagedComponent::WhisperCli]
        );
        assert_eq!(
            failing.dependencies[0].1,
            SetupDependencyDisposition::ManagedInstall
        );
        assert_eq!(
            failing.dependencies[2].1,
            SetupDependencyDisposition::ManagedInstall
        );
        Ok(())
    }

    /// The digest binds that a probe timed out, so a plan made while a tool
    /// did not answer is not the plan of the same machine once it does; it
    /// does not bind which kind of timeout it was or under which deadline.
    #[test]
    fn the_digest_binds_a_timeout_but_not_its_kind_or_deadline()
    -> Result<(), Box<dyn std::error::Error>> {
        let plan = |ffprobe: DependencyState| -> Result<_, Box<dyn std::error::Error>> {
            Ok(plan_managed_setup(
                SetupProfile::Desktop,
                diagnosis_of([
                    DependencyState::Available {
                        version: String::from("fixture"),
                    },
                    ffprobe,
                    DependencyState::Missing,
                ]),
                ManagedTarget::Ubuntu2404X86_64,
                with_model(),
                1_000,
                Some(catalogue()?),
            ))
        };
        let no_answer = plan(timed_out(ProbeTimeoutKind::NoAnswer))?;
        let not_started = plan(DependencyState::TimedOut(ProbeTimeout {
            kind: ProbeTimeoutKind::NotStarted,
            shared_deadline: Duration::from_secs(5),
        }))?;
        let answered = plan(DependencyState::Available {
            version: String::from("fixture"),
        })?;
        assert!(no_answer.digest.is_some());
        assert_eq!(no_answer.digest, not_started.digest);
        assert_ne!(no_answer.digest, answered.digest);
        Ok(())
    }

    /// Where nothing can be installed a timed-out tool was already left to the
    /// user; that is unchanged.
    #[test]
    fn a_timeout_on_a_target_without_managed_installation_stays_with_the_user()
    -> Result<(), Box<dyn std::error::Error>> {
        let plan = plan_managed_setup(
            SetupProfile::Desktop,
            diagnosis_of([
                timed_out(ProbeTimeoutKind::NoAnswer),
                timed_out(ProbeTimeoutKind::NotStarted),
                DependencyState::Missing,
            ]),
            ManagedTarget::WindowsX86_64,
            SetupSelectionState::default(),
            1_000,
            Some(catalogue()?),
        );
        assert_eq!(
            plan.availability,
            ManagedPlanAvailability::TargetUnavailable
        );
        assert_eq!(
            plan.actions,
            [] as [crate::provisioning::ManagedSetupAction; 0]
        );
        for (_, disposition) in &plan.dependencies {
            assert_eq!(*disposition, SetupDependencyDisposition::ManualSelection);
        }
        Ok(())
    }
}
