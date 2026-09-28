//! Every committed scenario is consistent with the corpus truth and the
//! skill, covers the verification rows it must, and never copies truth.

use std::{
    collections::BTreeSet,
    error::Error,
    path::{Path, PathBuf},
};

use vsift_agent_trials::{
    scenario::Scenario,
    skill::{SkillReferences, files_below, image_code},
    truth::{CorpusTruth, normalize},
};

type TestResult = Result<(), Box<dyn Error>>;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn scenarios() -> Result<Vec<(String, Scenario)>, Box<dyn Error>> {
    let directory = repository().join("tools/vsift-agent-trials/scenarios");
    let mut found = Vec::new();
    for (relative, path) in files_below(&directory)? {
        found.push((relative, Scenario::load(&path)?));
    }
    Ok(found)
}

#[test]
fn every_scenario_validates_against_the_truth_and_the_skill() -> TestResult {
    let truth = CorpusTruth::load(&repository().join("fixtures/corpus"))?;
    let references = SkillReferences::load(&repository())?;
    let mut covered = BTreeSet::new();
    for (file, scenario) in scenarios()? {
        assert_eq!(
            file,
            format!("{}.json", scenario.id),
            "file name and id differ"
        );
        scenario.validate(&truth, &references.policy)?;
        covered.extend(scenario.tests.iter().cloned());
    }
    let required: BTreeSet<String> = [
        "A-01", "A-02", "A-03", "A-04", "A-05", "A-06", "A-07", "A-08", "A-09", "SEC-T02",
    ]
    .iter()
    .map(|id| (*id).to_owned())
    .collect();
    assert!(
        required.is_subset(&covered),
        "missing {:?}",
        required.difference(&covered)
    );
    Ok(())
}

#[test]
fn prompts_never_carry_truth_or_the_image_code() -> TestResult {
    let truth = CorpusTruth::load(&repository().join("fixtures/corpus"))?;
    let code = image_code().to_ascii_lowercase();
    for (_, scenario) in scenarios()? {
        let fixture = truth.fixture(&scenario.fixture.id)?;
        let mut forbidden: Vec<String> = fixture
            .events
            .iter()
            .map(|event| event.truth.clone())
            .collect();
        forbidden.push(fixture.audio.script.clone());
        for phase in &scenario.phases {
            let prompt = normalize(&phase.prompt).join(" ");
            assert!(
                !phase.prompt.to_ascii_lowercase().contains(&code),
                "{}",
                scenario.id
            );
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
    Ok(())
}
