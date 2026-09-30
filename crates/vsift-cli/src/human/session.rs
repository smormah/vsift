//! `ingest`, the `session` commands and `bundle validate`.

use vsift_contract::CommandName;

use super::{
    push_outcome,
    text::{RenderedText, TerminalText, TooLarge},
    view::{Bundle, CleanPage, Envelope, Opened, SessionPage, SessionStatus, Workspace},
};

/// `ingest`: the newly opened session and any imported transcript.
pub(super) fn ingest(envelope: &Envelope<Opened>) -> Result<RenderedText, TooLarge> {
    let data = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("Opened session ")
        .push_value(&data.session_id)
        .end_line();
    push_source(&mut text, &data.source_id, data.source_bytes);
    text.push_fixed("Generation: ")
        .push_unsigned(data.generation)
        .push_fixed(" (")
        .push_value(&data.publication)
        .push_fixed(")")
        .end_line();
    match &data.transcript {
        Some(revision) => {
            text.push_fixed("Transcript: revision ")
                .push_unsigned(u64::from(revision.revision))
                .push_fixed(", ")
                .push_value(&revision.revision_id)
                .push_fixed(" (")
                .push_value(&revision.alignment.origin);
            if let Some(offset) = revision.alignment.offset_us {
                text.push_fixed(", offset ")
                    .push_signed(offset)
                    .push_fixed(" us");
            }
            text.push_fixed(", segments: ")
                .push_unsigned(revision.segment_count)
                .push_fixed(")")
                .end_line();
            text.push_fixed("Read it: vsift transcript get ")
                .push_value(&data.session_id)
                .end_line();
        }
        None => {
            text.push_fixed("Transcript: none imported").end_line();
        }
    }
    push_outcome(&mut text, envelope);
    text.finish()
}

/// `session status`, `session renew` and `session close`: the committed
/// status, and for `status` the session's newest jobs.
pub(super) fn status(
    command: CommandName,
    envelope: &Envelope<SessionStatus>,
) -> Result<RenderedText, TooLarge> {
    let status = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed(match command {
        CommandName::SessionRenew => "Renewed session ",
        CommandName::SessionClose => "Closed session ",
        _ => "Session ",
    })
    .push_value(&status.session_id)
    .end_line();
    push_status_lines(&mut text, status);
    if let Some(jobs) = &status.jobs {
        if jobs.is_empty() {
            text.push_fixed("Jobs: none").end_line();
        } else {
            text.push_fixed(if status.jobs_truncated {
                "Jobs (newest first; older ones are not listed):"
            } else {
                "Jobs (newest first):"
            })
            .end_line();
        }
        for job in jobs {
            text.push_fixed("  ")
                .push_value(&job.job_id)
                .push_fixed("  ")
                .push_value(&job.kind)
                .push_fixed("  ")
                .push_value(&job.state)
                .push_fixed(if job.resumable {
                    "  resumable ("
                } else {
                    "  not resumable ("
                })
                .push_value(&job.resumable_reason)
                .push_fixed(if job.live_owner {
                    "), running in a live process"
                } else {
                    ")"
                })
                .end_line();
        }
    }
    push_outcome(&mut text, envelope);
    text.finish()
}

/// `session list`: one line per session of the page.
pub(super) fn list(envelope: &Envelope<SessionPage>) -> Result<RenderedText, TooLarge> {
    let page = &envelope.data;
    let mut text = TerminalText::result();
    if page.items.is_empty() {
        text.push_fixed("No sessions.").end_line();
    } else {
        text.push_fixed("Sessions:").end_line();
    }
    for item in &page.items {
        text.push_fixed("  ")
            .push_value(&item.session_id)
            .push_fixed("  ")
            .push_value(&item.state);
        if let Some(status) = &item.status {
            text.push_fixed("  expires ").push_value(&status.expires_at);
        }
        if let Some(code) = &item.error_code {
            text.push_fixed("  ").push_value(code);
        }
        text.end_line();
    }
    push_next_cursor(&mut text, page.next_cursor);
    push_outcome(&mut text, envelope);
    text.finish()
}

/// `session clean`: what cleanup decided for each examined session.
pub(super) fn clean(envelope: &Envelope<CleanPage>) -> Result<RenderedText, TooLarge> {
    let page = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed(if page.dry_run {
        "Session cleanup, dry run: nothing was removed."
    } else {
        "Session cleanup:"
    })
    .end_line();
    if page.items.is_empty() {
        text.push_fixed("  No sessions examined.").end_line();
    }
    for item in &page.items {
        text.push_fixed("  ")
            .push_value(&item.session_id)
            .push_fixed("  ")
            .push_value(&item.outcome);
        if let Some(code) = &item.error_code {
            text.push_fixed("  ").push_value(code);
        }
        text.end_line();
    }
    push_next_cursor(&mut text, page.next_cursor);
    push_outcome(&mut text, envelope);
    text.finish()
}

/// `session retain` and `bundle validate`: the retained bundle.
pub(super) fn bundle(
    command: CommandName,
    envelope: &Envelope<Bundle>,
) -> Result<RenderedText, TooLarge> {
    let bundle = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed(match command {
        CommandName::BundleValidate => "Valid bundle of session ",
        _ => "Retained session ",
    })
    .push_value(&bundle.session_id)
    .end_line();
    push_source(&mut text, &bundle.source_id, bundle.source_bytes);
    text.push_fixed(if bundle.source_included {
        "Source copy: included in the bundle"
    } else {
        "Source copy: not included; re-extraction needs the matching original video"
    })
    .end_line();
    text.push_fixed("Artifacts: ")
        .push_unsigned(bundle.artifact_count)
        .push_fixed(" (")
        .push_unsigned(bundle.artifact_bytes)
        .push_fixed(" bytes)")
        .end_line();
    text.push_fixed("Publication: ")
        .push_value(&bundle.publication)
        .end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}

/// `session init-workspace`: the worker workspace's policy.
pub(super) fn workspace(envelope: &Envelope<Workspace>) -> Result<RenderedText, TooLarge> {
    let workspace = &envelope.data;
    let mut text = TerminalText::result();
    text.push_fixed("Worker workspace: ")
        .push_value(&workspace.outcome)
        .end_line();
    text.push_fixed("Profile: ")
        .push_value(&workspace.profile)
        .end_line();
    text.push_fixed("Durability: ")
        .push_value(&workspace.durability)
        .push_fixed(" (")
        .push_value(&workspace.publication)
        .push_fixed(")")
        .end_line();
    text.push_fixed("Admission slots: ")
        .push_unsigned(workspace.admission_capacity)
        .end_line();
    text.push_fixed("Session retention: ")
        .push_unsigned(workspace.session_retention_seconds)
        .push_fixed(" s (")
        .push_unsigned(workspace.session_retention_seconds / 3_600)
        .push_fixed(" h)")
        .end_line();
    push_outcome(&mut text, envelope);
    text.finish()
}

/// Writes the lines of a committed status after its heading.
fn push_status_lines(text: &mut TerminalText, status: &SessionStatus) {
    text.push_fixed("State: ")
        .push_value(&status.state)
        .end_line();
    push_source(text, &status.source_id, status.source_bytes);
    text.push_fixed("Artifacts: ")
        .push_unsigned(status.artifact_count)
        .push_fixed(" (")
        .push_unsigned(status.artifact_bytes)
        .push_fixed(" bytes)")
        .end_line();
    // The expiry is the lifecycle's, written with the outcome.
    text.push_fixed("Generation: ")
        .push_unsigned(status.generation)
        .end_line();
}

/// Writes a source identity and its size.
fn push_source(text: &mut TerminalText, source_id: &str, source_bytes: u64) {
    text.push_fixed("Source: ")
        .push_value(source_id)
        .push_fixed(" (")
        .push_unsigned(source_bytes)
        .push_fixed(" bytes)")
        .end_line();
}

/// Says how to read the next page of a session list or cleanup.
fn push_next_cursor(text: &mut TerminalText, cursor: Option<u16>) {
    if let Some(cursor) = cursor {
        text.push_fixed("More on the next page: repeat the command with --cursor ")
            .push_unsigned(u64::from(cursor))
            .end_line();
    }
}
