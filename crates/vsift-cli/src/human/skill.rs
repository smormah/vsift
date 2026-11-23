//! Human text of `skill install`, `skill status` and `skill show` (ADR 0038).
//!
//! Every value comes from `VSift`'s own results (fixed identifiers, folder
//! paths relative to their base, the shipped skill's text); each is still
//! written through [`TerminalText::push_value`], which applies the control and
//! hidden-character rules to anything a result carries.

use serde::Deserialize;

use super::{
    push_outcome,
    text::{RenderedText, TerminalText, TooLarge},
    view::Envelope,
};

/// One folder of an install.
#[derive(Debug, Deserialize)]
struct InstallFolder {
    path: String,
    clients: Vec<String>,
    outcome: String,
    replaced: Option<String>,
    refusal: Option<String>,
}

/// The `data` of a `skill.install` result.
#[derive(Debug, Deserialize)]
pub(crate) struct SkillInstall {
    scope: String,
    skill_digest: String,
    folders: Vec<InstallFolder>,
}

/// One folder of a status.
#[derive(Debug, Deserialize)]
struct StatusFolder {
    path: String,
    clients: Vec<String>,
    state: String,
}

/// The `data` of a `skill.status` result.
#[derive(Debug, Deserialize)]
pub(crate) struct SkillStatus {
    scope: String,
    skill_digest: String,
    files: Vec<String>,
    folders: Vec<StatusFolder>,
}

/// The `data` of a `skill.show` result.
#[derive(Debug, Deserialize)]
pub(crate) struct SkillShow {
    text: String,
}

fn push_header(text: &mut TerminalText, scope: &str, digest: &str) {
    text.push_fixed("Skill ")
        .push_value(digest.get(..12).unwrap_or(digest))
        .push_fixed("..., folders for ");
    if scope == "project" {
        text.push_fixed("this project");
    } else {
        text.push_fixed("you (below your home folder)");
    }
    text.end_line();
}

fn push_clients(text: &mut TerminalText, clients: &[String]) {
    text.push_fixed(" (");
    for (index, client) in clients.iter().enumerate() {
        if index > 0 {
            text.push_fixed(", ");
        }
        text.push_value(client);
    }
    text.push_fixed(")");
}

pub(super) fn install(envelope: &Envelope<SkillInstall>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    push_header(&mut text, &data.scope, &data.skill_digest);
    for folder in &data.folders {
        text.push_fixed("  ").push_value(&folder.path);
        push_clients(&mut text, &folder.clients);
        text.push_fixed(": ").push_value(&folder.outcome);
        if let Some(replaced) = &folder.replaced {
            text.push_fixed(", replaced what was there (")
                .push_value(replaced)
                .push_fixed(")");
        }
        if let Some(refusal) = &folder.refusal {
            text.push_fixed(", left as it was (")
                .push_value(refusal)
                .push_fixed(")");
        }
        text.end_line();
    }
    text.push_fixed("Start a new session of the client so that it finds the skill.")
        .end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}

pub(super) fn status(envelope: &Envelope<SkillStatus>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    push_header(&mut text, &data.scope, &data.skill_digest);
    for folder in &data.folders {
        text.push_fixed("  ").push_value(&folder.path);
        push_clients(&mut text, &folder.clients);
        text.push_fixed(": ").push_value(&folder.state).end_line();
    }
    text.blank_line();
    text.push_fixed("Files (vsift skill show <file>):")
        .end_line();
    for file in &data.files {
        text.push_fixed("  ").push_value(file).end_line();
    }
    push_outcome(&mut text, envelope);
    text.finish()
}

pub(super) fn show(envelope: &Envelope<SkillShow>) -> Result<RenderedText, TooLarge> {
    let mut text = TerminalText::result();
    for line in envelope.data.text.lines() {
        text.push_value(line).end_line();
    }
    push_outcome(&mut text, envelope);
    text.finish()
}
