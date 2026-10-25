//! The words and the `setup check` row of each optional component (ADR 0032).
//!
//! An optional component is a named group of managed components that a user
//! adds with `setup plan --include <name>`. Every one of them is presented
//! the same way: one plain sentence to relay to the operator, what it adds,
//! a terms notice when one applies, what to do when it is offered and left
//! out, when it is not offered on this host, and when it failed. This module
//! holds the fixed prose of each one in a single place, so a feature story
//! adds its component by adding its [`OptionalComponentText`] and nothing
//! else in the presentation.

use serde::Serialize;
use vsift_application::{OptionalCapability, OptionalComponent, SetupProfile};

/// The fixed prose of one optional component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionalComponentText {
    /// What it adds, in one sentence of the plan.
    pub purpose: &'static str,
    /// What it is, for the plan's one-sentence summary: the components and
    /// their licences, without the download size, which is added.
    pub subject: &'static str,
    /// The notice every offer of it carries, when one applies.
    pub terms_notice: Option<&'static str>,
    /// The systems a managed version of it is offered on, and what works
    /// without it, for a host where it is not offered.
    pub unavailable: &'static str,
    /// What `setup check` says when it is installed.
    pub installed: &'static str,
    /// What `setup check` and `setup install` call it in a sentence.
    pub name_in_prose: &'static str,
}

/// The notice every offer of online links carries.
pub const SITE_TERMS_NOTICE: &str = "Downloading from a site is subject to that site's terms, which are yours to follow. VSift does not get round sign-in, DRM, age, region or bot checks.";

/// The fixed prose of `component`.
#[must_use]
pub const fn optional_component_text(component: OptionalComponent) -> OptionalComponentText {
    match component {
        OptionalComponent::OnlineLinks => OptionalComponentText {
            purpose: "Fetches a video from a public web page (YouTube and other sites) when you give VSift a link with ingest --url, using yt-dlp and the JavaScript runtime it needs. Direct HTTPS video links work without it.",
            subject: "yt-dlp (GPL-3.0-or-later, with the MIT-licensed Deno runtime) fetches videos from YouTube and other sites when you give VSift a link. Downloading is subject to each site's terms.",
            terms_notice: Some(SITE_TERMS_NOTICE),
            unavailable: "A managed yt-dlp is offered on Ubuntu 24.04 x86-64 only. Direct HTTPS video links (ingest --url) work without it, as does every local file.",
            installed: "Online links can be fetched with ingest --url: yt-dlp and its JavaScript runtime are installed. Sites change and can break extraction; a newer reviewed yt-dlp comes with a VSift release, and managed_updates names it when one is known.",
            name_in_prose: "online links",
        },
    }
}

/// The `vsift` command that plans `component`, as an executable and an
/// argument array run without a shell.
#[must_use]
pub fn include_command_arguments(
    component: OptionalComponent,
    profile: SetupProfile,
) -> Vec<&'static str> {
    vec![
        "setup",
        "plan",
        "--profile",
        profile.identifier(),
        "--include",
        component.identifier(),
        "--json",
    ]
}

/// One entry of `setup check`'s `optional_components` (since `0.3.0`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct OptionalCheckResponse {
    name: &'static str,
    status: &'static str,
    components: Vec<OptionalCheckComponentResponse>,
    next_step: String,
    add_command: Option<SuggestedVsiftCommand>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct OptionalCheckComponentResponse {
    component: &'static str,
    version: String,
}

/// A `vsift` command as an executable and an argument array.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SuggestedVsiftCommand {
    executable: &'static str,
    arguments: Vec<&'static str>,
}

/// The `next_step` of one `setup check` optional component, for the
/// readable output too.
#[must_use]
pub fn optional_check_step(
    component: OptionalComponent,
    capability: &OptionalCapability,
    profile: SetupProfile,
) -> String {
    OptionalCheckResponse::new(component, capability, profile).next_step
}

impl OptionalCheckResponse {
    pub(crate) fn new(
        component: OptionalComponent,
        capability: &OptionalCapability,
        profile: SetupProfile,
    ) -> Self {
        let text = optional_component_text(component);
        let name = component.identifier();
        let (components, next_step, add_command) = match capability {
            OptionalCapability::Installed(components) => (
                components
                    .iter()
                    .map(|(component, version)| OptionalCheckComponentResponse {
                        component: component.identifier(),
                        version: version.clone(),
                    })
                    .collect(),
                String::from(text.installed),
                None,
            ),
            OptionalCapability::NotInstalled if profile == SetupProfile::Worker => (
                Vec::new(),
                format!(
                    "The optional {} component is not installed. The worker profile does not offer optional components; an operator adds it with setup plan --profile worker --include {name} only for a host that needs it.",
                    text.name_in_prose
                ),
                None,
            ),
            OptionalCapability::NotInstalled => (
                Vec::new(),
                format!(
                    "The optional {} component is not installed; local investigation does not need it. Ask the operator first; to add it, run vsift setup plan --profile {} --include {name} --json and accept that plan.",
                    text.name_in_prose,
                    profile.identifier()
                ),
                Some(SuggestedVsiftCommand {
                    executable: "vsift",
                    arguments: include_command_arguments(component, profile),
                }),
            ),
            OptionalCapability::Unavailable => (Vec::new(), String::from(text.unavailable), None),
        };
        Self {
            name,
            status: capability.identifier(),
            components,
            next_step,
            add_command,
        }
    }
}
