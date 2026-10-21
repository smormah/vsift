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
//!
//! A plan that includes optional components (ADR 0032) adds three members:
//! the core's status, the optional components' managed components in their
//! own list, and `optional_groups`, one summary per optional component. They
//! are absent otherwise, so the result of a plan without them is unchanged.
//! A failure of an optional component never makes the result a failure: the
//! core is ready, the result is `partial` with a warning, and its summary
//! says what failed and that local investigation is not affected.

use serde::Serialize;
use vsift_application::{
    ComponentInstallFailure, ComponentInstallOutcome, ComponentInstallReport,
    DownloadFailureReason, InstallFailureReason, InstallStep, ManagedInstallReport,
    OptionalComponent, OptionalGroupOutcome, StageDisposal, StageSweep, VersionRemovalReport,
};

use crate::optional_component::optional_component_text;
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
    /// `ready` or `failed`, only when the plan includes an optional
    /// component.
    #[serde(skip_serializing_if = "Option::is_none")]
    core: Option<&'static str>,
    /// The optional components, after the core, only when the plan
    /// includes one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    optional_components: Vec<InstallComponentResponse>,
    /// One summary per optional component the plan includes, only when it
    /// includes one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    optional_groups: Vec<OptionalGroupInstallResponse>,
}

/// One entry of a `setup install` result's `optional_groups`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionalGroupInstallResponse {
    name: &'static str,
    status: &'static str,
    component: Option<&'static str>,
    reason: Option<&'static str>,
    failure_code: Option<&'static str>,
    next_step: String,
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
        let optional = !report.optional.is_empty();
        let next_step = match report.failure_code() {
            None if optional && !report.optional_installed() => {
                "VSift is ready for local videos: every core component is installed and selected. An optional component was not installed (optional_groups says which and why); local investigation is not affected."
            }
            None if report.components.is_empty() && !optional => {
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
            core: optional.then_some(if report.failure_code().is_none() {
                "ready"
            } else {
                "failed"
            }),
            optional_components: report.optional.iter().map(component_response).collect(),
            optional_groups: report
                .optional_groups()
                .iter()
                .map(|outcome| optional_group_response(outcome, report.failure_code().is_none()))
                .collect(),
        }
    }

    /// The warning a successful result carries when an optional component
    /// was not installed, so the impression is never that `VSift` is broken;
    /// `None` otherwise. Such a result is `partial` (ADR 0032).
    #[must_use]
    pub fn optional_warning(&self) -> Option<&'static str> {
        (self.core == Some("ready")
            && self
                .optional_groups
                .iter()
                .any(|group| group.status == "not_installed"))
        .then_some(OPTIONAL_COMPONENT_NOT_INSTALLED_WARNING)
    }

    /// Each optional component's name and advice, when the plan included
    /// one, in plan order.
    #[must_use]
    pub fn optional_group_steps(&self) -> Vec<(&str, &str, &str)> {
        self.optional_groups
            .iter()
            .map(|group| (group.name, group.status, group.next_step.as_str()))
            .collect()
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

/// The envelope warning of a successful install in which an optional
/// component was not installed (ADR 0032).
pub const OPTIONAL_COMPONENT_NOT_INSTALLED_WARNING: &str = "optional_component_not_installed: VSift is ready for local videos, but an optional component was not installed; data.optional_groups says which and why.";

fn optional_group_response(
    outcome: &OptionalGroupOutcome,
    core_ready: bool,
) -> OptionalGroupInstallResponse {
    let text = optional_component_text(outcome.group);
    let name = outcome.group.identifier();
    if outcome.installed {
        return OptionalGroupInstallResponse {
            name,
            status: "installed",
            component: None,
            reason: None,
            failure_code: None,
            next_step: format!(
                "The optional {} component is installed and selected.",
                text.name_in_prose
            ),
        };
    }
    match outcome.failure {
        Some((component, failure)) => OptionalGroupInstallResponse {
            name,
            status: "not_installed",
            component: Some(component.identifier()),
            reason: Some(failure.reason.identifier()),
            failure_code: failure
                .reason
                .failure_code()
                .map(vsift_domain::FailureCode::identifier),
            next_step: optional_install_failure_summary(outcome.group, component, failure),
        },
        // The core failed, so the optional part was never attempted.
        None => OptionalGroupInstallResponse {
            name,
            status: "not_installed",
            component: None,
            reason: Some(InstallFailureReason::Blocked.identifier()),
            failure_code: None,
            next_step: if core_ready {
                format!(
                    "The optional {} component was not installed; run the same setup install again.",
                    text.name_in_prose
                )
            } else {
                format!(
                    "The optional {} component was not installed because a core component failed first; fix that failure and run the same setup install again.",
                    text.name_in_prose
                )
            },
        },
    }
}

/// Fixed-prose advice for an optional component that was not installed
/// (ADR 0032): what failed, that only that optional component failed, what
/// to try. Typed identifiers only, never a path, URL, server text or proxy
/// credentials.
#[must_use]
pub fn optional_install_failure_summary(
    group: OptionalComponent,
    component: ManagedComponent,
    failure: ComponentInstallFailure,
) -> String {
    let what = optional_component_text(group).name_in_prose;
    let component = component.identifier();
    let reason = failure.reason.identifier();
    let unaffected =
        format!("VSift is ready for local videos; only the optional {what} component failed.");
    let retry =
        "run the same setup install again: it keeps what is installed and retries what is not.";
    let (cause, what_to_try) = match failure.reason {
        InstallFailureReason::Download(DownloadFailureReason::ProxyAuth) => (
            "the network's proxy asked for credentials",
            "Ask whoever runs the network to allow the download, then",
        ),
        InstallFailureReason::Download(
            DownloadFailureReason::Offline | DownloadFailureReason::Tls,
        ) => (
            "the download was blocked or could not connect",
            "Check the network, TLS inspection or proxy (or ask whoever runs it), then",
        ),
        InstallFailureReason::Download(_) => (
            "the download did not complete",
            "Check the network or proxy, then",
        ),
        InstallFailureReason::Smoke(_) => (
            "it did not pass its compatibility check on this machine",
            "If a retry fails the same way, report it; meanwhile",
        ),
        InstallFailureReason::ArtifactMissing | InstallFailureReason::ArtifactNotRegularFile => (
            "the artifact folder has no regular file for it",
            "Add the file under the name its source URL in the plan ends with, then",
        ),
        InstallFailureReason::DigestMismatch
        | InstallFailureReason::SizeMismatch
        | InstallFailureReason::ReviewMismatch => (
            "its bytes differ from the reviewed file, so none of them was run",
            "If a retry fails the same way, report it; otherwise",
        ),
        InstallFailureReason::Storage => (
            "VSift could not write its private managed folder",
            "Free disk space or fix the folder's permissions, then",
        ),
        InstallFailureReason::Cancelled => ("the installation was cancelled", "To continue,"),
        InstallFailureReason::Blocked => ("the component before it failed", "Then"),
    };
    let offline = match failure.reason {
        InstallFailureReason::Download(_) => {
            " Without network access, put the plan's files in one folder and pass it with --artifact-dir."
        }
        _ => "",
    };
    format!(
        "The optional {what} component was not installed: {component} failed ({reason}) because {cause}. {unaffected} {what_to_try} {retry}{offline}"
    )
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
