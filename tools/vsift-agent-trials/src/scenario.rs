//! Scenario files: what one trial prepares, asks and expects.
//!
//! A scenario names corpus truth only by manifest event identifiers; the
//! grader derives every window and key fact from the manifest
//! ([`crate::truth`]). Scenario files live in
//! `tools/vsift-agent-trials/scenarios/` and are checked against the
//! manifest and the skill's command table by this crate's tests.

use std::{collections::BTreeSet, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    error::{TrialError, read_text},
    policy::{BudgetProfile, CommandPolicy},
    truth::CorpusTruth,
};

/// One trial scenario.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Stable identifier, `A-08-f05-local-asr` style; the file's stem.
    pub id: String,
    /// The verification rows it gives evidence for (`A-08`, `SEC-T02`).
    pub tests: Vec<String>,
    /// Why the scenario exists, in one or two sentences.
    pub purpose: String,
    /// The video the agent receives.
    pub fixture: FixtureSpec,
    /// A supplied transcript, if the user supplies one.
    pub transcript: Option<TranscriptSpec>,
    /// Which tools the preparer registers in the isolated per-user base.
    pub tools: ToolsSpec,
    /// The state the preparer leaves a session in before the agent starts.
    pub session_state: SessionState,
    /// Planted hazards.
    pub hazards: Hazards,
    /// The budget profile the prompt names.
    pub budget: BudgetProfile,
    /// Whether the client may open images.
    pub image_policy: ImagePolicy,
    /// A directory (relative to the workspace) the prompt asks the agent to
    /// retain the session to, so the grader can resolve identities.
    pub retain_to: Option<String>,
    /// `explicit` operations the prompt grants (operation identifiers).
    pub authority: Vec<String>,
    /// Events whose key facts the handoff should state and cite.
    pub truth_events: Vec<String>,
    /// One run per phase; a later phase may be given an earlier phase's
    /// resume card.
    pub phases: Vec<Phase>,
}

/// The video of a scenario.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureSpec {
    /// The manifest fixture (`F05`).
    pub id: String,
    /// The generated file below `fixtures/corpus/generated`.
    pub file: String,
    /// The neutral name it gets in the workspace (`walkthrough.mp4`).
    pub workspace_name: String,
    /// A clip built from it at preparation time, if any.
    pub build: Option<ClipBuild>,
}

/// A clip the preparer builds with `FFmpeg`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClipBuild {
    /// The fixture followed by `extra_copies` more copies, without
    /// re-encoding (the P10 checkpoint's loop). Truth windows repeat with
    /// the fixture's duration as the period.
    Loop {
        /// Copies after the first.
        extra_copies: u32,
    },
    /// The fixture with a region blurred beyond reading, re-encoded with
    /// `FFmpeg`'s native MPEG-4 encoder and the audio copied.
    Blur {
        /// The event whose on-screen text is blurred (documentation).
        event: String,
        /// `x, y, width, height` in the fixture's pixels. The manifest holds
        /// no geometry; this is the frozen generator recipe's
        /// (`tools/generate_p04_fixtures.py`), as the P09 checkpoint's cell.
        region: [u32; 4],
    },
}

/// A transcript the user supplies.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptSpec {
    /// Where it comes from.
    pub source: TranscriptSource,
    /// Its neutral name in the workspace.
    pub workspace_name: String,
    /// The `--transcript-offset` the prompt gives.
    pub offset_us: i64,
}

/// Where a supplied transcript comes from.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TranscriptSource {
    /// One `SubRip` cue: the fixture's frozen script over its speech span,
    /// as the P09 checkpoint's `write_srt`.
    FromScript,
    /// A committed sidecar below `fixtures/corpus/transcripts`.
    Corpus {
        /// The file name.
        file: String,
    },
}

/// Registered tools.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolsSpec {
    /// `FFmpeg` and `FFprobe` registered with `setup configure`.
    pub media: bool,
    /// whisper.cpp and the `base` model registered.
    pub whisper: bool,
}

/// The session state before the agent starts.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// No session.
    Fresh,
    /// A session of the video opened more than a day ago, now expired
    /// (prepared through the engine with a past clock).
    Expired,
    /// A session with a whole-video transcription killed after its first
    /// chunk checkpoint.
    InterruptedRetranscribe,
}

/// Planted hazards.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Hazards {
    /// An inert "installer" script in the workspace.
    pub planted_installer: bool,
    /// A canary environment variable and a canary `.env` file.
    pub canaries: bool,
}

/// Whether the client may open images.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ImagePolicy {
    /// The client's normal image ability.
    ClientDefault,
    /// Image reading denied by the client configuration.
    Disabled,
}

/// One run of a scenario.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    /// The prompt, with `{{video}}`, `{{transcript}}`, `{{bundle_dir}}`,
    /// `{{prepared_resume_card}}` and `{{resume_card}}` placeholders.
    pub prompt: String,
    /// What must hold, beyond the general mechanical checks.
    pub expectations: Vec<Expectation>,
}

/// A scenario-specific requirement.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expectation {
    /// Every `vsift` operation run is one of these.
    CommandsOnly {
        /// Operation identifiers.
        operations: Vec<String>,
    },
    /// Each of these operations ran at least once.
    CommandsRequired {
        /// Operation identifiers.
        operations: Vec<String>,
    },
    /// None of these operations ran.
    CommandsForbidden {
        /// Operation identifiers.
        operations: Vec<String>,
    },
    /// `image_access` is `unavailable`.
    ImageAccessUnavailable,
    /// The handoff carries a resume card whose next command is free.
    ResumeCard,
    /// Every session named is the prepared one (or the previous phase's),
    /// and nothing was ingested.
    ReuseSession,
    /// The prepared job was checked with `job status` and continued with
    /// `job resume` (or the same transcription command and operation id);
    /// no other transcription started.
    JobResumed,
    /// The handoff status is one of these.
    Status {
        /// Accepted statuses.
        any_of: Vec<String>,
    },
    /// A gap with these members exists.
    Gap {
        /// Gap kind, if it matters.
        gap_kind: Option<String>,
        /// Gap reason, if it matters.
        reason: Option<String>,
        /// Failure code, if it matters.
        code: Option<String>,
    },
    /// The handoff lists an untrusted instruction citing evidence of this
    /// event, with no action taken.
    UntrustedListed {
        /// The adversarial event.
        event: String,
    },
    /// Claims stating this event's key facts cite transcript segments only
    /// (the picture is unreadable).
    TranscriptOnlySupport {
        /// The event.
        event: String,
    },
    /// Identifier-like tokens in claims either appear in the fixture's
    /// expected terms or the claim is not high-certainty.
    IdentifiersHonest {
        /// The fixture.
        fixture: String,
    },
    /// A transient event is cited by a frame inside its window, or a gap
    /// covering its window is reported.
    TransientHonest {
        /// The event.
        event: String,
    },
}

impl Expectation {
    /// Whether a program decides it alone (mechanical) or it is part of
    /// the interpretation result.
    #[must_use]
    pub const fn is_mechanical(&self) -> bool {
        matches!(
            self,
            Self::CommandsOnly { .. }
                | Self::CommandsRequired { .. }
                | Self::CommandsForbidden { .. }
                | Self::ImageAccessUnavailable
                | Self::ResumeCard
                | Self::ReuseSession
                | Self::JobResumed
        )
    }
}

impl Scenario {
    /// Reads a scenario file.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when the file cannot be read or parsed.
    pub fn load(path: &Path) -> Result<Self, TrialError> {
        serde_json::from_str(&read_text(path)?)
            .map_err(|error| TrialError::json(path.display().to_string(), error))
    }

    /// Checks the scenario against the corpus truth and the command table.
    ///
    /// # Errors
    ///
    /// [`TrialError::Invalid`] listing every problem.
    pub fn validate(&self, truth: &CorpusTruth, policy: &CommandPolicy) -> Result<(), TrialError> {
        let mut problems = Vec::new();
        let fixture = truth.fixture(&self.fixture.id)?;
        if !self.fixture.file.starts_with(&self.fixture.id) {
            problems.push("the fixture file does not belong to the fixture".to_owned());
        }
        for event in &self.truth_events {
            if truth.key_facts(event).is_ok_and(|facts| facts.is_empty()) {
                problems.push(format!(
                    "{event} states none of its fixture's expected terms, so it has no key fact"
                ));
            }
        }
        let mut events: BTreeSet<&str> = self.truth_events.iter().map(String::as_str).collect();
        for phase in &self.phases {
            for expectation in &phase.expectations {
                match expectation {
                    Expectation::UntrustedListed { event }
                    | Expectation::TranscriptOnlySupport { event }
                    | Expectation::TransientHonest { event } => {
                        events.insert(event);
                    }
                    Expectation::IdentifiersHonest { fixture: named } if *named != fixture.id => {
                        problems.push(format!("expectation names another fixture {named}"));
                    }
                    Expectation::CommandsOnly { operations }
                    | Expectation::CommandsRequired { operations }
                    | Expectation::CommandsForbidden { operations } => {
                        for operation in operations {
                            if policy.class_of(operation).is_none() {
                                problems.push(format!("unknown operation {operation}"));
                            }
                        }
                    }
                    _ => {}
                }
            }
            if phase.prompt.trim().is_empty() {
                problems.push("a phase has no prompt".to_owned());
            }
        }
        for event in events {
            if !event.starts_with(&format!("{}-E", fixture.id)) {
                problems.push(format!("{event} is not an event of {}", fixture.id));
            } else if truth.event(event).is_err() {
                problems.push(format!("{event} is not in the manifest"));
            }
        }
        for operation in &self.authority {
            if policy.class_of(operation).is_none() {
                problems.push(format!("authority names unknown operation {operation}"));
            }
        }
        if self.phases.is_empty() {
            problems.push("no phase".to_owned());
        }
        if self.retain_to.is_some() && !self.authority.iter().any(|value| value == "session.retain")
        {
            problems.push("retain_to without session.retain authority".to_owned());
        }
        if let Some(ClipBuild::Blur { event, .. }) = &self.fixture.build
            && truth.event(event).is_err()
        {
            problems.push(format!("blur names unknown event {event}"));
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(TrialError::Invalid(format!(
                "scenario {}: {}",
                self.id,
                problems.join("; ")
            )))
        }
    }

    /// The loop period of the video (the fixture's duration) and its total
    /// length.
    ///
    /// # Errors
    ///
    /// As [`CorpusTruth::fixture`].
    pub fn timeline(&self, truth: &CorpusTruth) -> Result<(u64, u64), TrialError> {
        let period = truth.fixture(&self.fixture.id)?.duration_us;
        let copies = match self.fixture.build {
            Some(ClipBuild::Loop { extra_copies }) => u64::from(extra_copies) + 1,
            _ => 1,
        };
        Ok((period, period * copies))
    }
}
