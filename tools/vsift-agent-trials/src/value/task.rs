//! The value measurement's task files (`tools/vsift-agent-trials/value/tasks/`).
//!
//! A task is one recording and one or two questions (two for T7, whose
//! second question is asked in a fresh conversation). Its schema is its own,
//! apart from the skill and cold scenarios, so nothing here changes how those
//! are checked. Truth comes from the corpus manifest for the synthetic tasks
//! (T1 to T3), by event identifier as the scenarios do, or inline from the
//! labels of the real recordings (T4 to T7, written by M2 through a reviewed
//! change). A task whose truth is still `pending_m2` may be piloted but is
//! refused for a counted run.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use super::claims::TruthItem;
use crate::{
    error::TrialError,
    truth::{CorpusTruth, KeyFact, fact_words, time_alternatives},
};

/// The four arms of phase 1 (method section 3).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    /// Raw, cold: `FFmpeg`, `FFprobe` and whisper-cli, a neutral prompt.
    R0,
    /// Raw, guided: R0 plus the frozen raw method guide as a skill.
    R1,
    /// `VSift`, cold: `vsift` on `PATH`, no skill.
    V0,
    /// `VSift` with its skill, as shipped.
    V1,
}

impl Arm {
    /// Every arm, in the order a batch interleaves them.
    pub const ALL: [Self; 4] = [Self::R0, Self::R1, Self::V0, Self::V1];

    /// Whether `vsift` is on the client's `PATH`.
    #[must_use]
    pub const fn has_vsift(self) -> bool {
        matches!(self, Self::V0 | Self::V1)
    }

    /// The arm's name as written in records and inputs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::R0 => "r0",
            Self::R1 => "r1",
            Self::V0 => "v0",
            Self::V1 => "v1",
        }
    }

    /// Reads an arm's name.
    ///
    /// # Errors
    ///
    /// [`TrialError::Invalid`] for any other word.
    pub fn parse(word: &str) -> Result<Self, TrialError> {
        Self::ALL
            .into_iter()
            .find(|arm| arm.name() == word.to_ascii_lowercase())
            .ok_or_else(|| TrialError::Invalid(format!("unknown arm {word:?}")))
    }
}

/// What a task asks about (method section 5).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// T1: a short clip, speech only.
    ShortSpeech,
    /// T2: a short screen recording with a visual change.
    ShortVisualChange,
    /// T3: lead/lag, variable frame rate, audio offset.
    TimingTrap,
    /// T4: a long recording with silence first and noise.
    LongSpeech,
    /// T5: a screen recording with small text.
    ScreenSmallText,
    /// T6: a talk with slides and a demonstration.
    TalkWithDemo,
    /// T7: a second question in a fresh context, the first run's work kept.
    SecondQuestion,
}

/// Where the recording comes from.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Media {
    /// A synthetic corpus fixture, from `fixtures/corpus/generated/`.
    Corpus {
        /// The fixture's identifier (`F04`).
        fixture: String,
        /// Its file name in `generated/`.
        file: String,
    },
    /// An item of the project's test-media release, fetched by the hosted
    /// workflow and handed to `prepare` by path, checked by its digest.
    TestMediaRelease {
        /// The item (`RM10`).
        item: String,
        /// Its file name in the release.
        file: String,
        /// Its SHA-256.
        sha256: String,
    },
}

impl Media {
    /// The file's extension, which the workspace copy keeps.
    #[must_use]
    pub fn extension(&self) -> String {
        let file = match self {
            Self::Corpus { file, .. } | Self::TestMediaRelease { file, .. } => file,
        };
        Path::new(file)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("mp4")
            .to_owned()
    }
}

/// The transcript every arm is given, if the task gives one.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transcript {
    /// The corpus fixture's script at its speech placement, as the skill
    /// scenarios' `from_script`.
    FromScript,
    /// A `SubRip` file beside the media (the captions of a real item), handed
    /// to `prepare` by path and checked by its digest.
    File {
        /// Its file name.
        file: String,
        /// Its SHA-256.
        sha256: String,
    },
}

/// One fact a report should state, with other accepted spellings.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FactSpec {
    /// The fact as written in the truth.
    pub term: String,
    /// Other wordings that state the same fact.
    #[serde(default)]
    pub alternatives: Vec<String>,
}

/// One truth item of a real recording.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ItemSpec {
    /// Its identifier (`RM10-Q1-a`).
    pub id: String,
    /// The window's start, microseconds.
    pub start_us: u64,
    /// The window's end, microseconds.
    pub end_us: u64,
    /// How far either side a claim may lie (the labels' agreement, 1 s).
    pub tolerance_us: u64,
    /// The facts to state.
    pub facts: Vec<FactSpec>,
    /// Terms that contradict the truth.
    #[serde(default)]
    pub contradicts: Vec<String>,
}

/// A corpus event a phase asks about.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EventRef {
    /// The manifest event (`F04-E03`).
    pub id: String,
    /// Terms that contradict the truth.
    #[serde(default)]
    pub contradicts: Vec<String>,
}

/// Where a phase's truth comes from.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PhaseTruth {
    /// Corpus events, read from the manifest (windows exact, tolerance 0).
    Events {
        /// The events.
        events: Vec<EventRef>,
    },
    /// Inline items from the agreed labels.
    Items {
        /// The items.
        items: Vec<ItemSpec>,
    },
    /// Not yet written (M2): a pilot may run, a counted run may not.
    PendingM2,
}

/// One question of a task.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TaskPhase {
    /// The question, with `{{video}}` and (when a transcript is given)
    /// `{{transcript}}`.
    pub question: String,
    /// Its truth.
    pub truth: PhaseTruth,
}

/// The same limits for every arm (planner's ruling, question 4).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TaskBudget {
    /// Wall-clock limit of one phase, seconds.
    pub timeout_s: u64,
    /// Claude Code's `--max-turns`.
    pub max_turns: u32,
}

/// One task file.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// Its identifier (`T2-f04-order-1017`).
    pub id: String,
    /// The method's task number (`T2`).
    pub task: String,
    /// What it asks about.
    pub kind: TaskKind,
    /// The recording.
    pub media: Media,
    /// The transcript every arm is given.
    pub transcript: Option<Transcript>,
    /// Its questions, one per phase.
    pub phases: Vec<TaskPhase>,
    /// Its limits.
    pub budget: TaskBudget,
}

/// The fixed report requirement of method section 6, the same in every arm.
pub const REPORT_REQUIREMENT: &str = "In your answer, give for each statement about when something happens the time, and the file, frame or identifier it rests on.";

/// What phase 2 of a two-phase task tells every arm (planner's ruling,
/// question 10).
pub const EARLIER_ANSWER: &str =
    "An earlier assistant worked on this recording in this folder; its answer is in answer-1.md.";

/// The file the harness writes phase 1's answer to before phase 2.
pub const EARLIER_ANSWER_FILE: &str = "answer-1.md";

/// Words a question may not hold: a tool, the skill's vocabulary or an
/// instruction that would differ between arms.
const FORBIDDEN_IN_QUESTION: [&str; 11] = [
    "vsift",
    "ffmpeg",
    "ffprobe",
    "whisper",
    "skill",
    "handoff",
    "session",
    "guide",
    "budget",
    "evidence id",
    "--",
];

impl Task {
    /// Reads a task file.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when it cannot be read or parsed.
    pub fn load(path: &Path) -> Result<Self, TrialError> {
        let text = fs::read_to_string(path).map_err(|error| TrialError::io_at(path, error))?;
        serde_json::from_str(&text).map_err(|error| TrialError::json("value task", error))
    }

    /// Whether any phase still waits for M2's truth.
    #[must_use]
    pub fn truth_pending(&self) -> bool {
        self.phases
            .iter()
            .any(|phase| phase.truth == PhaseTruth::PendingM2)
    }

    /// Checks the task against the corpus truth and its own rules.
    ///
    /// # Errors
    ///
    /// [`TrialError::Invalid`] naming the first problem.
    pub fn validate(&self, corpus: &CorpusTruth, counted: bool) -> Result<(), TrialError> {
        let refuse = |why: String| Err(TrialError::Invalid(format!("{}: {why}", self.id)));
        if self.phases.is_empty() || self.phases.len() > 2 {
            return refuse("a task has one or two phases".to_owned());
        }
        if (self.phases.len() == 2) != (self.kind == TaskKind::SecondQuestion) {
            return refuse("only a second-question task has two phases".to_owned());
        }
        if counted && self.truth_pending() {
            return refuse("its truth is pending (M2); it may only be piloted".to_owned());
        }
        if let Media::Corpus { fixture, .. } = &self.media {
            corpus.fixture(fixture)?;
        }
        if matches!(self.transcript, Some(Transcript::FromScript))
            && !matches!(self.media, Media::Corpus { .. })
        {
            return refuse("a script transcript needs a corpus fixture".to_owned());
        }
        for (index, phase) in self.phases.iter().enumerate() {
            let lower = phase.question.to_lowercase();
            if let Some(word) = FORBIDDEN_IN_QUESTION
                .iter()
                .find(|word| lower.contains(*word))
            {
                return refuse(format!(
                    "phase {} names {word:?}, which would differ between arms",
                    index + 1
                ));
            }
            if !phase.question.contains("{{video}}") {
                return refuse(format!("phase {} does not name {{{{video}}}}", index + 1));
            }
            let leftover = phase
                .question
                .replace("{{video}}", "")
                .replace("{{transcript}}", "");
            if leftover.contains("{{")
                || (self.transcript.is_none() && phase.question.contains("{{transcript}}"))
            {
                return refuse(format!("phase {} has an unknown placeholder", index + 1));
            }
            if phase.truth != PhaseTruth::PendingM2 && self.truth_items(index, corpus)?.is_empty() {
                return refuse(format!("phase {} has no truth item", index + 1));
            }
        }
        Ok(())
    }

    /// The truth items of a phase (from 0), with their key facts.
    ///
    /// # Errors
    ///
    /// [`TrialError`] when an event is unknown or a fact has no words.
    pub fn truth_items(
        &self,
        phase: usize,
        corpus: &CorpusTruth,
    ) -> Result<Vec<TruthItem>, TrialError> {
        let Some(spec) = self.phases.get(phase) else {
            return Err(TrialError::Invalid(format!(
                "{} has no phase {}",
                self.id,
                phase + 1
            )));
        };
        match &spec.truth {
            PhaseTruth::PendingM2 => Ok(Vec::new()),
            PhaseTruth::Events { events } => events
                .iter()
                .map(|reference| {
                    let (_, event) = corpus.event(&reference.id)?;
                    Ok(TruthItem {
                        id: event.id.clone(),
                        start_us: event.start_us,
                        end_us: event.end_us,
                        tolerance_us: 0,
                        facts: corpus.key_facts(&event.id)?,
                        contradicts: words_of(&reference.contradicts),
                    })
                })
                .collect(),
            PhaseTruth::Items { items } => items
                .iter()
                .map(|item| {
                    if item.end_us < item.start_us {
                        return Err(TrialError::Invalid(format!(
                            "{}: item {} ends before it starts",
                            self.id, item.id
                        )));
                    }
                    let facts = item
                        .facts
                        .iter()
                        .map(|fact| {
                            let words = fact_words(&fact.term);
                            if words.is_empty() {
                                return Err(TrialError::Invalid(format!(
                                    "{}: a fact of {} has no words",
                                    self.id, item.id
                                )));
                            }
                            let mut alternatives = time_alternatives(&fact.term);
                            alternatives
                                .extend(fact.alternatives.iter().map(|text| fact_words(text)));
                            Ok(KeyFact {
                                event: item.id.clone(),
                                term: fact.term.clone(),
                                words,
                                alternatives,
                            })
                        })
                        .collect::<Result<Vec<_>, TrialError>>()?;
                    Ok(TruthItem {
                        id: item.id.clone(),
                        start_us: item.start_us,
                        end_us: item.end_us,
                        tolerance_us: item.tolerance_us,
                        facts,
                        contradicts: words_of(&item.contradicts),
                    })
                })
                .collect(),
        }
    }

    /// The workspace name of the recording: neutral, so no path tells the
    /// agent which task it is in.
    #[must_use]
    pub fn video_name(&self) -> String {
        format!("recording.{}", self.media.extension())
    }

    /// The workspace name of the transcript, when there is one.
    #[must_use]
    pub fn transcript_name(&self) -> Option<String> {
        self.transcript.as_ref().map(|_| "recording.srt".to_owned())
    }

    /// The prompt of a phase (from 0) for an arm. Only the tool sentence and
    /// the skill sentence differ between arms; the question, the report
    /// requirement and the earlier-answer sentence are the same words.
    ///
    /// # Errors
    ///
    /// [`TrialError::Invalid`] when the phase does not exist.
    pub fn prompt(&self, phase: usize, arm: Arm, guide_name: &str) -> Result<String, TrialError> {
        let Some(spec) = self.phases.get(phase) else {
            return Err(TrialError::Invalid(format!(
                "{} has no phase {}",
                self.id,
                phase + 1
            )));
        };
        let mut question = spec.question.replace("{{video}}", &self.video_name());
        if let Some(name) = self.transcript_name() {
            question = question.replace("{{transcript}}", &name);
        }
        let mut lines = vec![tool_sentence(arm)];
        match arm {
            Arm::R1 => lines.push(format!("Use the {guide_name} skill.")),
            Arm::V1 => lines.push("Use the vsift skill.".to_owned()),
            Arm::R0 | Arm::V0 => {}
        }
        let mut text = lines.join(" ");
        text.push_str("\n\n");
        if phase > 0 {
            text.push_str(EARLIER_ANSWER);
            text.push(' ');
        }
        text.push_str(&question);
        text.push_str("\n\n");
        text.push_str(REPORT_REQUIREMENT);
        Ok(text)
    }
}

/// The tool sentence of an arm (design table, section 2).
#[must_use]
pub fn tool_sentence(arm: Arm) -> String {
    let raw = format!(
        "FFmpeg, FFprobe and whisper-cli are installed; a whisper model is at {MODEL_IN_WORKSPACE}."
    );
    if arm.has_vsift() {
        format!("{raw} A command-line tool named `vsift` is installed.")
    } else {
        raw
    }
}

/// Where every arm finds the whisper model: inside the workspace, so that
/// no arm needs to read outside it.
pub const MODEL_IN_WORKSPACE: &str = "models/ggml-base.bin";

fn words_of(terms: &[String]) -> Vec<Vec<String>> {
    terms
        .iter()
        .map(|term| fact_words(term))
        .filter(|words| !words.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(question: &str) -> Task {
        Task {
            id: "T9-test".to_owned(),
            task: "T9".to_owned(),
            kind: TaskKind::ScreenSmallText,
            media: Media::TestMediaRelease {
                item: "RM10".to_owned(),
                file: "RM10.mp4".to_owned(),
                sha256: "00".to_owned(),
            },
            transcript: None,
            phases: vec![TaskPhase {
                question: question.to_owned(),
                truth: PhaseTruth::PendingM2,
            }],
            budget: TaskBudget {
                timeout_s: 900,
                max_turns: 60,
            },
        }
    }

    #[test]
    fn prompts_differ_between_arms_only_in_the_tool_and_skill_sentences() {
        let task = task("What does {{video}} show at the start?");
        let prompts: Vec<String> = Arm::ALL
            .iter()
            .filter_map(|arm| task.prompt(0, *arm, "recording-evidence").ok())
            .collect();
        assert_eq!(prompts.len(), 4);
        for prompt in &prompts {
            assert!(
                prompt.contains("What does recording.mp4 show at the start?"),
                "{prompt}"
            );
            assert!(prompt.ends_with(REPORT_REQUIREMENT), "{prompt}");
            assert!(prompt.contains(MODEL_IN_WORKSPACE), "{prompt}");
        }
        assert!(!prompts[0].contains("vsift") && !prompts[1].contains("vsift"));
        assert!(prompts[1].contains("Use the recording-evidence skill."));
        assert!(prompts[2].contains("`vsift` is installed") && !prompts[2].contains("skill"));
        assert!(prompts[3].contains("Use the vsift skill."));
    }

    #[test]
    fn a_question_that_names_a_tool_is_refused() {
        for question in [
            "Use ffmpeg on {{video}}.",
            "What does the vsift session of {{video}} hold?",
        ] {
            let corpus = CorpusTruth::default();
            assert!(
                task(question).validate(&corpus, false).is_err(),
                "{question}"
            );
        }
    }

    #[test]
    fn a_pending_task_may_be_piloted_but_not_counted() {
        let corpus = CorpusTruth::default();
        let task = task("What does {{video}} show?");
        assert!(task.validate(&corpus, false).is_ok());
        assert!(task.validate(&corpus, true).is_err());
    }

    #[test]
    fn arm_names_round_trip() {
        for arm in Arm::ALL {
            assert_eq!(Arm::parse(arm.name()).ok(), Some(arm));
        }
        assert!(Arm::parse("v2").is_err());
    }
}
