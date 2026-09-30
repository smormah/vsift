//! `setup install` data and remediation (P13, ADR 0023 §3 step 3).
//!
//! The data lists every component of the accepted plan in plan order with
//! what the transaction did to it: `activated`, `already_current` or
//! `failed`, the step and typed reason of a failure, and what cleanup did
//! with its private stage. A successful result carries it as `data`; a
//! failed one carries the same object as failure data beside the error, so
//! a caller always sees which components are installed. Every value is a
//! closed identifier; nothing here carries a path, a URL a server chose,
//! provider output or credentials.

use serde::Serialize;
use vsift_application::{
    ComponentInstallFailure, ComponentInstallOutcome, ComponentInstallReport, InstallFailureReason,
    InstallStep, ManagedInstallReport, StageDisposal,
};
use vsift_domain::ManagedComponent;

/// Where `setup install` read the reviewed artifacts from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallSource {
    /// Each artifact's reviewed publisher, over HTTPS.
    Publisher,
    /// The user's `--artifact-dir`, by the file names of the catalogue.
    ArtifactDirectory,
}

impl InstallSource {
    /// Stable machine-readable identifier.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Publisher => "publisher",
            Self::ArtifactDirectory => "artifact_directory",
        }
    }
}

/// Data of a `setup install` result, successful or failed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetupInstallResponse {
    catalogue_revision: Option<String>,
    source: &'static str,
    components: Vec<InstallComponentResponse>,
    next_step: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct InstallComponentResponse {
    component: &'static str,
    version: String,
    status: &'static str,
    step: Option<&'static str>,
    reason: Option<&'static str>,
    failure_code: Option<&'static str>,
    smoke_check: Option<&'static str>,
    stage: Option<&'static str>,
    retention_reason: Option<&'static str>,
}

impl SetupInstallResponse {
    /// Presents one transaction's report.
    #[must_use]
    pub fn new(
        catalogue_revision: Option<String>,
        source: InstallSource,
        report: &ManagedInstallReport,
    ) -> Self {
        let next_step = match report.failure_code() {
            None if report.components.is_empty() => {
                "Nothing needed installing. Run setup check to see what each command will use."
            }
            None => {
                "Every component of the plan is installed and selected. Run setup check to confirm what each command will use."
            }
            Some(_) => {
                "Components reported activated or already_current are installed. Fix the failure the error names, then run the same setup install again: it continues from the first component not yet installed."
            }
        };
        Self {
            catalogue_revision,
            source: source.identifier(),
            components: report.components.iter().map(component_response).collect(),
            next_step,
        }
    }
}

fn component_response(report: &ComponentInstallReport) -> InstallComponentResponse {
    let failure = match report.outcome {
        ComponentInstallOutcome::Failed(failure) => Some(failure),
        ComponentInstallOutcome::Activated | ComponentInstallOutcome::AlreadyCurrent => None,
    };
    let (stage, retention_reason) = match report.stage {
        None => (None, None),
        Some(StageDisposal::Discarded) => (Some("discarded"), None),
        Some(StageDisposal::Retained(reason)) => (Some("retained"), Some(reason.identifier())),
    };
    InstallComponentResponse {
        component: report.component.identifier(),
        version: report.version.clone(),
        status: report.outcome.identifier(),
        step: failure.and_then(|failure| failure.step.map(InstallStep::identifier)),
        reason: failure.map(|failure| failure.reason.identifier()),
        failure_code: failure
            .and_then(|failure| failure.reason.failure_code())
            .map(vsift_domain::FailureCode::identifier),
        smoke_check: failure.and_then(|failure| match failure.reason {
            InstallFailureReason::Smoke(smoke) => Some(smoke.check.identifier()),
            _ => None,
        }),
        stage,
        retention_reason,
    }
}

/// Remediation when another `setup install` holds the managed root.
pub const MANAGED_INSTALL_BUSY_REMEDIATION: &str = "Another setup install is running for this user and holds the managed folder; nothing was changed. Wait for it to finish, then run setup check, or run the same setup install again.";

/// Remediation when the private managed folder cannot be created or
/// written: the manual path, which never needs it (D-09, D-10).
pub const MANAGED_STORAGE_REMEDIATION: &str = "VSift could not create or write its private managed folder, so nothing was installed. Free disk space or fix the folder's permissions and run the same setup install again; or install FFmpeg, FFprobe and the whisper.cpp CLI yourself, register each with setup configure ffmpeg|ffprobe|whisper --executable <absolute-path> and the model with setup configure-model --file <absolute-path>, then run setup check.";

/// Remediation when this host has no managed installation to accept.
pub const MANAGED_UNAVAILABLE_REMEDIATION: &str = "Managed installation is not available on this host (it is qualified on Ubuntu 24.04 x86-64 only), so there is no plan to accept. Install FFmpeg, FFprobe and the whisper.cpp CLI yourself, register each with setup configure ffmpeg|ffprobe|whisper --executable <absolute-path> and the model with setup configure-model --file <absolute-path>, then run setup check.";

/// Remediation when the saved plan or its digest no longer describes the
/// current plan.
pub const STALE_PLAN_REMEDIATION: &str = "The saved plan, or the digest given with it, does not describe the plan for this machine as it is now; nothing was changed. Run setup plan --json again, review it, save it, and pass its plan_digest to setup install --accept-plan.";

/// Remediation when `--artifact-dir` is not an absolute folder path.
pub const ARTIFACT_DIRECTORY_REMEDIATION: &str = "--artifact-dir must be the absolute path of a folder that holds the plan's artifact files under the names their source URLs end with; nothing was changed.";

/// Fixed-prose remediation for the component failure that stopped a
/// transaction: typed identifiers only, never a path, URL or server text.
#[must_use]
pub fn setup_install_failure_summary(
    component: ManagedComponent,
    failure: ComponentInstallFailure,
) -> String {
    let component = component.identifier();
    let reason = failure.reason.identifier();
    let manual = "Or install the tool yourself and register it with setup configure (the model with setup configure-model), then run setup check.";
    match failure.reason {
        InstallFailureReason::Download(_) => format!(
            "The download of {component} did not complete ({reason}); nothing of it was installed. Check the network, TLS inspection or proxy, then run the same setup install again: it restarts that download from the first byte and keeps the components already installed. Without network access, save the plan's artifact files in one folder and pass it with --artifact-dir. {manual}"
        ),
        InstallFailureReason::ArtifactMissing | InstallFailureReason::ArtifactNotRegularFile => {
            format!(
                "The artifact folder has no regular file for {component} ({reason}): it must hold the reviewed file under the name its source URL in the plan ends with. Add it, then run the same setup install again. {manual}"
            )
        }
        InstallFailureReason::DigestMismatch
        | InstallFailureReason::SizeMismatch
        | InstallFailureReason::ReviewMismatch => format!(
            "The bytes of {component} differ from the reviewed artifact ({reason}), so none of them was extracted, run or installed. Run the same setup install again; if it fails the same way, report it. {manual}"
        ),
        InstallFailureReason::Storage => format!(
            "VSift could not write its private managed folder while installing {component} ({reason}); the version selected before is unchanged. Free disk space or fix the folder's permissions, then run the same setup install again. {manual}"
        ),
        InstallFailureReason::Smoke(smoke) => format!(
            "The reviewed {component} did not pass its compatibility check on this machine (the {} check, {reason}), so it was not installed. {manual}",
            smoke.check.identifier()
        ),
        InstallFailureReason::Cancelled => format!(
            "The installation was cancelled at {component}; the components already installed stay installed and nothing partial was kept. Run the same setup install again to continue."
        ),
        InstallFailureReason::Blocked => {
            format!("{component} was not installed because an earlier component failed. {manual}")
        }
    }
}
