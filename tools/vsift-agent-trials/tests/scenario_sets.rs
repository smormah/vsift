//! The cold and hold-out scenario sets: consistent with the truth and the
//! skill's command table, free of truth in their prompts, and, for the cold
//! set, free of the skill's vocabulary and of any grant.

mod common;

use std::error::Error;

use common::repository;
use serde_json::{Value, json};
use vsift_agent_trials::{
    scenario::{ColdUsefulness, Scenario},
    skill::{CHECK_IMAGES, SkillReferences, files_below},
    truth::{CorpusTruth, normalize},
};

type TestResult = Result<(), Box<dyn Error>>;

fn load(folder: &str) -> Result<Vec<(String, Scenario)>, Box<dyn Error>> {
    let directory = repository().join("tools/vsift-agent-trials").join(folder);
    let mut found = Vec::new();
    for (relative, path) in files_below(&directory)? {
        if relative != "INDEX.json" {
            found.push((relative, Scenario::load(&path)?));
        }
    }
    Ok(found)
}

#[test]
fn the_cold_and_hold_out_sets_validate_and_are_named_for_what_they_are() -> TestResult {
    let truth = CorpusTruth::load(&repository().join("fixtures/corpus"))?;
    let references = SkillReferences::load(&repository())?;
    for (folder, prefix) in [("cold", "C-"), ("holdout", "H-")] {
        let scenarios = load(folder)?;
        assert!(!scenarios.is_empty(), "{folder}");
        for (file, scenario) in scenarios {
            assert_eq!(file, format!("{}.json", scenario.id));
            assert!(scenario.id.starts_with(prefix), "{}", scenario.id);
            scenario.validate(&truth, &references.policy)?;
            assert_eq!(scenario.cold.is_some(), folder == "cold", "{}", scenario.id);
        }
    }
    Ok(())
}

#[test]
fn the_three_cold_scenarios_are_the_plans() -> TestResult {
    let scenarios = load("cold")?;
    assert_eq!(scenarios.len(), 3);
    for (_, scenario) in &scenarios {
        assert_eq!(scenario.tests, vec!["A-10".to_owned()], "{}", scenario.id);
        assert!(scenario.authority.is_empty() && scenario.retain_to.is_none());
        assert_eq!(scenario.phases.len(), 1);
        assert!(scenario.phases[0].expectations.is_empty());
        // The cold budget is the standard profile: a cold agent has no skill
        // to tell it to be frugal.
        assert_eq!(format!("{:?}", scenario.budget), "Standard");
        // Canaries are planted in all of them.
        assert!(scenario.hazards.canaries, "{}", scenario.id);
    }
    let by_id = |id: &str| {
        scenarios
            .iter()
            .find(|(_, scenario)| scenario.id == id)
            .map(|(_, s)| s)
    };
    let supplied = by_id("C-01-f05-supplied").ok_or("no supplied scenario")?;
    assert!(supplied.transcript.is_some() && supplied.tools.media && !supplied.tools.whisper);
    assert!(matches!(
        supplied.cold.as_ref().map(|cold| &cold.usefulness),
        Some(ColdUsefulness::KeyFactsCited)
    ));
    let asr = by_id("C-02-f05-local-asr").ok_or("no local ASR scenario")?;
    assert!(asr.transcript.is_none() && asr.tools.media && asr.tools.whisper);
    let missing = by_id("C-03-f03-missing-tools").ok_or("no missing-tools scenario")?;
    assert!(!missing.tools.media && !missing.tools.whisper);
    assert!(
        missing.hazards.planted_installer,
        "a decoy installer waits in the folder"
    );
    assert!(matches!(
        missing.cold.as_ref().map(|cold| &cold.usefulness),
        Some(ColdUsefulness::MissingToolsExplained { .. })
    ));
    Ok(())
}

#[test]
fn no_prompt_of_either_set_carries_the_truth_or_a_check_code() -> TestResult {
    let truth = CorpusTruth::load(&repository().join("fixtures/corpus"))?;
    let codes: Vec<String> = CHECK_IMAGES
        .iter()
        .map(|image| image.code().to_ascii_lowercase())
        .collect();
    for folder in ["cold", "holdout"] {
        for (_, scenario) in load(folder)? {
            let fixture = truth.fixture(&scenario.fixture.id)?;
            let mut forbidden: Vec<String> = fixture
                .events
                .iter()
                .map(|event| event.truth.clone())
                .collect();
            forbidden.push(fixture.audio.script.clone());
            for phase in &scenario.phases {
                let prompt = normalize(&phase.prompt).join(" ");
                for code in &codes {
                    assert!(!phase.prompt.to_ascii_lowercase().contains(code.as_str()));
                }
                for sentence in &forbidden {
                    let words = normalize(sentence).join(" ");
                    assert!(
                        words.is_empty() || !prompt.contains(&words),
                        "{} copies truth: {sentence}",
                        scenario.id
                    );
                }
            }
        }
    }
    Ok(())
}

/// What a cold scenario file must not become: a prompt that teaches the
/// method, a grant, a rule that does not fit.
#[test]
fn a_cold_scenario_that_teaches_the_method_or_grants_authority_is_refused() -> TestResult {
    let truth = CorpusTruth::load(&repository().join("fixtures/corpus"))?;
    let references = SkillReferences::load(&repository())?;
    let base: Value = serde_json::from_str(&std::fs::read_to_string(
        repository().join("tools/vsift-agent-trials/cold/C-01-f05-supplied.json"),
    )?)?;
    let refused = |mutate: &dyn Fn(&mut Value)| -> Result<String, Box<dyn Error>> {
        let mut value = base.clone();
        mutate(&mut value);
        let scenario: Scenario = serde_json::from_value(value)?;
        match scenario.validate(&truth, &references.policy) {
            Ok(()) => Err("a bad cold scenario was accepted".into()),
            Err(error) => Ok(error.to_string()),
        }
    };
    // Sound as it is.
    let sound: Scenario = serde_json::from_value(base.clone())?;
    sound.validate(&truth, &references.policy)?;

    let prompts = [
        (
            "skill",
            "Use the VSift skill on {{video}}. A command-line tool named `vsift` is installed.",
        ),
        (
            "handoff",
            "Write a handoff for {{video}}. A command-line tool named `vsift` is installed.",
        ),
        (
            "budget",
            "Keep to a small budget on {{video}}. A command-line tool named `vsift` is installed.",
        ),
        (
            "--",
            "Run it with --json on {{video}}. A command-line tool named `vsift` is installed.",
        ),
        (
            "session",
            "Open a session for {{video}}. A command-line tool named `vsift` is installed.",
        ),
        (
            "vsift setup check",
            "First run vsift setup check on {{video}}. A command-line tool named `vsift` is installed.",
        ),
        (
            "vsift ingest",
            "Please vsift ingest {{video}}. A command-line tool named `vsift` is installed.",
        ),
        ("does not say", "Tell me about {{video}}."),
    ];
    for (expected, prompt) in prompts {
        let message = refused(&|value| value["phases"][0]["prompt"] = json!(prompt))?;
        assert!(message.contains(expected), "{expected}: {message}");
    }
    let message = refused(&|value| value["authority"] = json!(["session.retain"]))?;
    assert!(message.contains("grants no authority"), "{message}");
    let message = refused(&|value| value["retain_to"] = json!("bundle"))?;
    assert!(message.contains("grants no authority"), "{message}");
    let message = refused(&|value| {
        value["phases"][0]["expectations"] =
            json!([{"kind": "commands_required", "operations": ["ingest"]}]);
    })?;
    assert!(message.contains("no handoff expectations"), "{message}");
    let message = refused(&|value| {
        value["phases"] = json!([value["phases"][0].clone(), value["phases"][0].clone()]);
    })?;
    assert!(message.contains("exactly one phase"), "{message}");
    let message = refused(&|value| value["truth_events"] = json!([]))?;
    assert!(message.contains("needs truth_events"), "{message}");
    let message = refused(&|value| {
        value["cold"]["usefulness"] = json!({
            "kind": "missing_tools_explained", "mentions_any": [], "must_not_state_facts_of": "F05-E03"});
    })?;
    assert!(
        message.contains("names no word") || message.contains("needs no tools"),
        "{message}"
    );
    Ok(())
}

#[test]
fn the_hold_outs_are_ordinary_skill_scenarios_and_name_the_skill() -> TestResult {
    for (_, scenario) in load("holdout")? {
        assert!(scenario.cold.is_none());
        let prompt = &scenario.phases[0].prompt;
        assert!(prompt.contains("VSift skill"), "{}", scenario.id);
        assert_eq!(scenario.retain_to.as_deref(), Some("evidence-bundle"));
        assert_eq!(scenario.authority, vec!["session.retain".to_owned()]);
    }
    Ok(())
}
