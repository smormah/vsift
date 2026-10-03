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
    InstallStep, ManagedInstallReport, StageDisposal, StageSweep, VersionRemovalReport,
};
use vsift_domain::{ManagedComponent, SharedLibraryName};

use crate::lifecycle::InstallCleanupResponse;

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
    cleanup: InstallCleanupResponse,
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
    /// The shared library a program could not start without, read from its
    /// own error output and validated as a plain library file name (#256).
    /// Absent unless a failure named one, so earlier readers of the object
    /// see nothing new.
    #[serde(skip_serializing_if = "Option::is_none")]
    missing_shared_library: Option<String>,
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
            cleanup: InstallCleanupResponse::default(),
            next_step,
        }
    }

    /// Adds what the stale-stage sweep before the transaction and the
    /// bounded version cleanup after it did (P13 PR 6).
    #[must_use]
    pub fn with_cleanup(mut self, stages: &StageSweep, versions: &[VersionRemovalReport]) -> Self {
        self.cleanup = InstallCleanupResponse::new(stages, versions);
        self
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
        missing_shared_library: failure.and_then(|failure| match failure.reason {
            InstallFailureReason::Smoke(smoke) => smoke
                .reason
                .missing_shared_library()
                .map(|name| name.as_str().to_owned()),
            _ => None,
        }),
        stage,
        retention_reason,
    }
}

/// Remediation when another `setup install` holds the managed root.
pub const MANAGED_INSTALL_BUSY_REMEDIATION: &str = "Another setup install, setup rollback or setup remove is running for this user and holds the managed folder; nothing was changed. Wait for it to finish, then run the same command again.";

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
        InstallFailureReason::Smoke(smoke) => match smoke.reason.missing_shared_library() {
            Some(library) => missing_library_summary(component, smoke.check.identifier(), library),
            None => format!(
                "The reviewed {component} did not pass its compatibility check on this machine (the {} check, {reason}), so it was not installed. {manual}",
                smoke.check.identifier()
            ),
        },
        InstallFailureReason::Cancelled => format!(
            "The installation was cancelled at {component}; the components already installed stay installed and nothing partial was kept. Run the same setup install again to continue."
        ),
        InstallFailureReason::Blocked => {
            format!("{component} was not installed because an earlier component failed. {manual}")
        }
    }
}

/// Fixed-prose remediation for a reviewed tool that could not start because
/// a shared library is not installed (#256).
///
/// The library's file name is the only text taken from the program's own
/// error output, and only after [`SharedLibraryName`] validated it as a plain
/// file name. The package is named only where it is known: the OpenMP runtime
/// `libgomp.so.1` is Ubuntu's and Debian's `libgomp1`, which a minimal
/// container image lacks. For any other library the text says what a person
/// can rely on and no more: install the package that provides it. The
/// managed installation is qualified on Ubuntu 24.04 only.
fn missing_library_summary(component: &str, check: &str, library: SharedLibraryName) -> String {
    let fix = if library.as_str() == "libgomp.so.1" {
        "On Ubuntu and Debian install it with `sudo apt-get install libgomp1` (a minimal container image does not include it)".to_owned()
    } else {
        format!(
            "Install the package that provides {library} with your system's package manager (the managed installation is qualified on Ubuntu 24.04 only)"
        )
    };
    format!(
        "The reviewed {component} could not start on this machine: it needs the shared library {library}, which is not installed here (the {check} check; the program's own error output says so), so it was not installed. {fix}, then run the same setup install again: it continues from this component. Or install the tool yourself and register it with setup configure (the model with setup configure-model), then run setup check."
    )
}
