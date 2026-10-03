//! The hold-out separation and the freeze: the files that must not move
//! between the first counted trial of a batch and its last.

mod common;

use std::{error::Error, fs, path::Path};

use common::{Scratch, repository};
use serde_json::{Value, json};
use vsift_agent_trials::{
    freeze::{self, COMPONENTS},
    holdout::{self, INDEX_PATH, TranscriptPath},
    skill::{copy_directory, file_digest},
};

type TestResult = Result<(), Box<dyn Error>>;

fn harness(root: &Path) -> std::path::PathBuf {
    root.join("tools").join("vsift-agent-trials")
}

/// The repository's three scenario folders, copied into a scratch
/// repository so that a test can change one.
fn copy_scenarios(scratch: &Scratch) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let root = scratch.path().join("repository");
    for folder in ["scenarios", "cold", "holdout"] {
        copy_directory(
            &harness(&repository()).join(folder),
            &harness(&root).join(folder),
        )?;
    }
    Ok(root)
}

fn index(root: &Path) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        root.join(INDEX_PATH),
    )?)?)
}

fn rewrite_digest(root: &Path, id: &str) -> TestResult {
    let mut value = index(root)?;
    let file = harness(root).join("holdout").join(format!("{id}.json"));
    for entry in value["entries"].as_array_mut().into_iter().flatten() {
        if entry["id"] == id {
            entry["sha256"] = json!(file_digest(&file)?);
        }
    }
    fs::write(root.join(INDEX_PATH), serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

#[test]
fn the_repositorys_hold_outs_are_separate_frozen_and_cover_both_paths() -> TestResult {
    assert_eq!(holdout::check(&repository())?, Vec::<String>::new());
    let loaded = holdout::HoldoutIndex::load(&repository())?.ok_or("no index")?;
    assert!(loaded.contains("H-01-f10-supplied-sidecar"));
    assert!(loaded.contains("H-02-f01-local-asr"));
    assert!(!loaded.contains("A-09-f05-supplied"));
    let paths: Vec<TranscriptPath> = loaded.entries.iter().map(|entry| entry.path).collect();
    assert!(paths.contains(&TranscriptPath::SuppliedTranscript));
    assert!(paths.contains(&TranscriptPath::LocalAsr));
    Ok(())
}

#[test]
fn editing_a_hold_out_without_its_index_entry_is_caught() -> TestResult {
    let scratch = Scratch::new("holdout-edit")?;
    let root = copy_scenarios(&scratch)?;
    assert_eq!(holdout::check(&root)?, Vec::<String>::new());
    let file = harness(&root)
        .join("holdout")
        .join("H-02-f01-local-asr.json");
    let text = fs::read_to_string(&file)?;
    fs::write(&file, text.replace("service readout", "service dashboard"))?;
    let problems = holdout::check(&root)?;
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("changed since it was frozen")),
        "{problems:?}"
    );
    // Changing it in the open, with its index entry, is allowed.
    rewrite_digest(&root, "H-02-f01-local-asr")?;
    assert_eq!(holdout::check(&root)?, Vec::<String>::new());
    Ok(())
}

#[test]
fn a_hold_out_that_names_an_event_a_tuning_scenario_names_is_caught() -> TestResult {
    let scratch = Scratch::new("holdout-overlap")?;
    let root = copy_scenarios(&scratch)?;
    // Point the supplied-transcript hold-out at the event A-03 and A-09 use.
    let file = harness(&root)
        .join("holdout")
        .join("H-01-f10-supplied-sidecar.json");
    let mut value: Value = serde_json::from_str(&fs::read_to_string(&file)?)?;
    value["fixture"] = json!({"id": "F05", "file": "F05-speech.mp4", "workspace_name": "walkthrough.mp4", "build": null});
    value["transcript"] = json!({"source": {"kind": "from_script"}, "workspace_name": "walkthrough.srt", "offset_us": 0});
    value["truth_events"] = json!(["F05-E03"]);
    fs::write(&file, serde_json::to_string_pretty(&value)?)?;
    rewrite_digest(&root, "H-01-f10-supplied-sidecar")?;
    let problems = holdout::check(&root)?;
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("F05-E03 is named by a tuning or cold scenario")),
        "{problems:?}"
    );

    // The same is true of a cold scenario's event.
    let scratch = Scratch::new("holdout-overlap-cold")?;
    let root = copy_scenarios(&scratch)?;
    let file = harness(&root)
        .join("holdout")
        .join("H-02-f01-local-asr.json");
    let text = fs::read_to_string(&file)?
        .replace("F01-E01", "F03-E02")
        .replace("F01-speech.mp4", "F03-speech.mp4")
        .replace("\"id\": \"F01\"", "\"id\": \"F03\"");
    fs::write(&file, text)?;
    rewrite_digest(&root, "H-02-f01-local-asr")?;
    let problems = holdout::check(&root)?;
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("F03-E02 is named by a tuning or cold scenario")),
        "{problems:?}"
    );
    Ok(())
}

#[test]
fn a_hold_out_listed_nowhere_a_missing_path_and_a_wrong_path_are_caught() -> TestResult {
    // Unlisted file.
    let scratch = Scratch::new("holdout-unlisted")?;
    let root = copy_scenarios(&scratch)?;
    fs::copy(
        harness(&root)
            .join("holdout")
            .join("H-02-f01-local-asr.json"),
        harness(&root).join("holdout").join("H-09-extra.json"),
    )?;
    let problems = holdout::check(&root)?;
    assert!(
        problems.iter().any(|problem| problem
            .contains("H-09-extra.json is in the hold-out folder but not in the index")),
        "{problems:?}"
    );

    // One path left uncovered.
    let scratch = Scratch::new("holdout-path")?;
    let root = copy_scenarios(&scratch)?;
    let mut value = index(&root)?;
    value["entries"] = json!([value["entries"][0].clone()]);
    fs::write(root.join(INDEX_PATH), serde_json::to_string_pretty(&value)?)?;
    let problems = holdout::check(&root)?;
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("no hold-out covers")),
        "{problems:?}"
    );

    // An entry that says the wrong path.
    let scratch = Scratch::new("holdout-wrong-path")?;
    let root = copy_scenarios(&scratch)?;
    let mut value = index(&root)?;
    value["entries"][1]["path"] = json!("supplied_transcript");
    fs::write(root.join(INDEX_PATH), serde_json::to_string_pretty(&value)?)?;
    let problems = holdout::check(&root)?;
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("the index says")),
        "{problems:?}"
    );

    // No index at all.
    let scratch = Scratch::new("holdout-none")?;
    let problems = holdout::check(scratch.path())?;
    assert_eq!(problems.len(), 1);
    Ok(())
}

/// A repository with just the files a freeze reads.
fn minimal_repository(scratch: &Scratch) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let root = scratch.path().join("repository");
    let files = [
        ("skills/vsift/SKILL.md", "skill"),
        ("skills/vsift/references/commands.md", "commands"),
        ("tools/vsift-agent-trials/src/lib.rs", "grader"),
        ("tools/vsift-agent-trials/src/grade.rs", "grader"),
        ("tools/vsift-agent-trials/Cargo.toml", "manifest"),
        ("tools/vsift-agent-trials/scenarios/A-01.json", "{}"),
        ("tools/vsift-agent-trials/cold/C-01.json", "{}"),
        ("tools/vsift-agent-trials/holdout/H-01.json", "{}"),
        ("tools/vsift-agent-trials/claude-trial-settings.json", "{}"),
        (
            "tools/vsift-agent-trials/claude-cold-trial-settings.json",
            "{}",
        ),
        (
            "tools/vsift-agent-trials/claude-cold-trial-settings.realistic.json",
            "{}",
        ),
        ("fixtures/corpus/manifest.json", "{}"),
        ("fixtures/corpus/generated/speech-provenance.json", "{}"),
    ];
    for (relative, text) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("no parent")?)?;
        fs::write(path, text)?;
    }
    Ok(root)
}

#[test]
fn a_freeze_notices_every_component_that_moves() -> TestResult {
    let scratch = Scratch::new("freeze")?;
    let root = minimal_repository(&scratch)?;
    let file = scratch.path().join("freeze.json");
    let written = freeze::write(&root, "abc123", &file)?;
    assert_eq!(written.components.len(), COMPONENTS.len());
    assert_eq!(freeze::check(&root, &file, None)?, Vec::<String>::new());

    for (component, relative) in [
        ("skill", "skills/vsift/references/commands.md"),
        ("grader", "tools/vsift-agent-trials/src/grade.rs"),
        ("grader", "tools/vsift-agent-trials/Cargo.toml"),
        ("scenarios", "tools/vsift-agent-trials/scenarios/A-01.json"),
        ("cold", "tools/vsift-agent-trials/cold/C-01.json"),
        ("holdout", "tools/vsift-agent-trials/holdout/H-01.json"),
        (
            "settings",
            "tools/vsift-agent-trials/claude-trial-settings.json",
        ),
        (
            "settings",
            "tools/vsift-agent-trials/claude-cold-trial-settings.json",
        ),
        (
            "settings",
            "tools/vsift-agent-trials/claude-cold-trial-settings.realistic.json",
        ),
        ("truth", "fixtures/corpus/manifest.json"),
        ("truth", "fixtures/corpus/generated/speech-provenance.json"),
    ] {
        let path = root.join(relative);
        let original = fs::read_to_string(&path)?;
        fs::write(&path, format!("{original} changed"))?;
        let problems = freeze::check(&root, &file, None)?;
        assert_eq!(
            problems,
            vec![format!("{component} changed since the freeze")],
            "{relative}"
        );
        fs::write(&path, original)?;
        assert!(
            freeze::check(&root, &file, None)?.is_empty(),
            "{relative} restored"
        );
    }
    Ok(())
}

#[test]
fn between_the_cold_rounds_only_the_cold_parts_must_hold() -> TestResult {
    let scratch = Scratch::new("freeze-only")?;
    let root = minimal_repository(&scratch)?;
    let file = scratch.path().join("freeze.json");
    freeze::write(&root, "abc123", &file)?;
    // The skill and the hold-outs may move between the cold rounds; the
    // grader, the cold scenarios, the settings and the truth may not.
    fs::write(root.join("skills/vsift/SKILL.md"), "a new skill")?;
    fs::write(
        root.join("tools/vsift-agent-trials/holdout/H-01.json"),
        "{\"x\": 1}",
    )?;
    let only: Vec<String> = ["grader", "cold", "settings", "truth"]
        .map(str::to_owned)
        .to_vec();
    assert!(freeze::check(&root, &file, Some(&only))?.is_empty());
    fs::write(
        root.join("tools/vsift-agent-trials/src/grade.rs"),
        "a changed grader",
    )?;
    assert_eq!(
        freeze::check(&root, &file, Some(&only))?,
        vec!["grader changed since the freeze".to_owned()]
    );
    // An unknown component is refused, not ignored.
    let unknown = vec!["scenarios-ish".to_owned()];
    assert!(freeze::check(&root, &file, Some(&unknown)).is_err());
    Ok(())
}

#[test]
fn a_freeze_file_edited_by_hand_is_refused() -> TestResult {
    let scratch = Scratch::new("freeze-edit")?;
    let root = minimal_repository(&scratch)?;
    let file = scratch.path().join("freeze.json");
    freeze::write(&root, "abc123", &file)?;
    let mut value: Value = serde_json::from_str(&fs::read_to_string(&file)?)?;
    value["components"]["skill"] = json!("0".repeat(64));
    fs::write(&file, serde_json::to_string_pretty(&value)?)?;
    let problems = freeze::check(&root, &file, None)?;
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].contains("own digest does not match"),
        "{problems:?}"
    );
    Ok(())
}

#[test]
fn the_real_repository_has_a_freezable_state() -> TestResult {
    let scratch = Scratch::new("freeze-real")?;
    let file = scratch.path().join("freeze.json");
    let written = freeze::write(&repository(), "0000", &file)?;
    assert_eq!(written.freeze_sha256.len(), 64);
    assert!(freeze::check(&repository(), &file, None)?.is_empty());
    Ok(())
}
