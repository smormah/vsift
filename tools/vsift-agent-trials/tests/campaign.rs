//! The campaign plan: the run counts of the plan's section 7, the order,
//! the counting rules and the retry limits.

mod common;

use std::{collections::BTreeSet, error::Error, path::Path};

use common::{Scratch, repository};
use vsift_agent_trials::{
    campaign::{
        CampaignState, ClientName, MAX_FAILED_ATTEMPTS, MAX_RESERVE_PER_STATE, Outcome, RunKind,
        Status, Tier, plan,
    },
    scenario::{Scenario, TrialMode},
};

type TestResult = Result<(), Box<dyn Error>>;

fn count(runs: &[vsift_agent_trials::campaign::PlannedRun], kind: RunKind) -> usize {
    runs.iter().filter(|run| run.kind == kind).count()
}

#[test]
fn the_batches_add_up_to_the_plans_72_runs_and_a_reserve_of_12() -> TestResult {
    let mut total = 0;
    let mut pilots = 0;
    let mut counted_with_skill = 0;
    let mut cold = 0;
    for batch in 1..=3 {
        for client in [ClientName::Claude, ClientName::Codex] {
            let runs = plan(batch, client)?;
            total += runs.len();
            pilots += count(&runs, RunKind::Pilot);
            counted_with_skill += runs
                .iter()
                .filter(|run| run.kind == RunKind::Counted && run.mode == TrialMode::Skill)
                .count();
            cold += runs
                .iter()
                .filter(|run| run.kind == RunKind::Counted && run.mode == TrialMode::Cold)
                .count();
        }
    }
    assert_eq!(pilots, 8, "pilots");
    assert_eq!(counted_with_skill, 34, "the counted set with the skill");
    assert_eq!(cold, 30, "the cold runs: 12 baseline and 18 final");
    assert_eq!(total, 72);
    assert_eq!(total + 12, 84, "with the reserve, the plan's 84 runs");
    Ok(())
}

#[test]
fn each_batch_has_the_shape_the_plan_describes() -> TestResult {
    // Batch 1: 4 pilots and 6 cold baseline runs per client, compact tier.
    for client in [ClientName::Claude, ClientName::Codex] {
        let runs = plan(1, client)?;
        assert_eq!(runs.len(), 10);
        assert_eq!(count(&runs, RunKind::Pilot), 4);
        assert!(runs.iter().all(|run| run.tier == Tier::Compact));
        let pilots: BTreeSet<&str> = runs
            .iter()
            .filter(|run| run.kind == RunKind::Pilot)
            .map(|run| run.scenario.as_str())
            .collect();
        assert_eq!(
            pilots,
            BTreeSet::from([
                "A-08-f05-local-asr",
                "A-09-f05-supplied",
                "C-01-f05-supplied",
                "C-02-f05-local-asr"
            ]),
            "a dry run per mode, on both transcript paths"
        );
        for scenario in [
            "C-01-f05-supplied",
            "C-02-f05-local-asr",
            "C-03-f03-missing-tools",
        ] {
            let baseline = runs
                .iter()
                .filter(|run| run.kind == RunKind::Counted && run.scenario == scenario)
                .count();
            assert_eq!(baseline, 2, "{scenario}");
        }
    }

    // Batch 2: review tier 12, compact tier 5, per client.
    for client in [ClientName::Claude, ClientName::Codex] {
        let runs = plan(2, client)?;
        let review: Vec<_> = runs.iter().filter(|run| run.tier == Tier::Review).collect();
        let compact: Vec<_> = runs
            .iter()
            .filter(|run| run.tier == Tier::Compact)
            .collect();
        assert_eq!((review.len(), compact.len()), (12, 5));
        let of = |scenario: &str| review.iter().filter(|run| run.scenario == scenario).count();
        assert_eq!(of("A-08-f05-local-asr"), 3);
        assert_eq!(of("A-09-f05-supplied"), 3);
        assert_eq!(of("H-01-f10-supplied-sidecar"), 1);
        assert_eq!(of("H-02-f01-local-asr"), 1);
        assert_eq!(of("A-01-f01-do-not-install"), 1);
        assert_eq!(of("A-09-f05-blurred"), 3);
        let compact_of = |scenario: &str| {
            compact
                .iter()
                .filter(|run| run.scenario == scenario)
                .count()
        };
        assert_eq!(compact_of("A-08-f05-local-asr"), 2);
        assert_eq!(compact_of("A-09-f05-supplied"), 2);
        assert_eq!(compact_of("SEC-T02-f12-webvtt"), 1);
        assert!(
            runs.iter()
                .all(|run| run.mode == TrialMode::Skill && run.kind == RunKind::Counted)
        );
    }

    // Batch 3: compact 6 and review 3 cold runs per client.
    for client in [ClientName::Claude, ClientName::Codex] {
        let runs = plan(3, client)?;
        assert_eq!(
            runs.iter().filter(|run| run.tier == Tier::Compact).count(),
            6
        );
        assert_eq!(
            runs.iter().filter(|run| run.tier == Tier::Review).count(),
            3
        );
        assert!(runs.iter().all(|run| run.mode == TrialMode::Cold));
    }
    assert!(plan(0, ClientName::Claude).is_err() && plan(4, ClientName::Codex).is_err());
    Ok(())
}

#[test]
fn the_models_are_the_named_ones_and_the_ids_are_unique_and_plain() -> TestResult {
    let mut seen = BTreeSet::new();
    for batch in 1..=3 {
        for client in [ClientName::Claude, ClientName::Codex] {
            for run in plan(batch, client)? {
                assert!(seen.insert(run.run_id.clone()), "{}", run.run_id);
                assert!(
                    run.run_id
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                    "{}",
                    run.run_id
                );
                let expected = match (client, run.tier) {
                    (ClientName::Claude, Tier::Review) => "claude-opus-5-5",
                    (ClientName::Claude, Tier::Compact) => "claude-sonnet-5-5",
                    (ClientName::Codex, Tier::Review) => "gpt-6-astra",
                    (ClientName::Codex, Tier::Compact) => "gpt-6-sol",
                };
                assert_eq!(run.model, expected);
            }
        }
    }
    Ok(())
}

fn scenario_file(directory: &str, id: &str) -> std::path::PathBuf {
    repository()
        .join("tools/vsift-agent-trials")
        .join(directory)
        .join(format!("{id}.json"))
}

#[test]
fn every_planned_scenario_exists_in_the_folder_its_mode_says() -> TestResult {
    for batch in 1..=3 {
        for run in plan(batch, ClientName::Claude)? {
            let found: Vec<&str> = ["scenarios", "cold", "holdout"]
                .into_iter()
                .filter(|directory| Path::new(&scenario_file(directory, &run.scenario)).is_file())
                .collect();
            assert_eq!(found.len(), 1, "{}: {found:?}", run.scenario);
            let loaded = Scenario::load(&scenario_file(found[0], &run.scenario))?;
            assert_eq!(loaded.mode(), run.mode, "{}", run.scenario);
            assert_eq!(found[0] == "cold", run.mode == TrialMode::Cold);
        }
    }
    Ok(())
}

#[test]
fn a_run_counts_once_and_invalid_attempts_retry_until_three() -> TestResult {
    let mut state = CampaignState::new(2, ClientName::Claude, "0.2.0-rc.1")?;
    let first = state.next()?.ok_or("nothing to run")?.run.run_id.clone();

    // A usage limit never blocks: the run waits and stays next.
    for _ in 0..10 {
        let status = state.mark(&first, None, Outcome::UsageLimited, None)?;
        assert_eq!(status, Status::Pending);
    }
    assert_eq!(
        state.next()?.map(|run| run.run.run_id.clone()),
        Some(first.clone())
    );

    // An invalid trial and a harness error do not count and retry.
    assert_eq!(
        state.mark(&first, Some("t1".into()), Outcome::Invalid, None)?,
        Status::Pending
    );
    assert_eq!(
        state.mark(&first, None, Outcome::HarnessError, None)?,
        Status::Pending
    );
    // The third such attempt blocks the run: counts are never reduced silently.
    assert_eq!(
        state.mark(&first, Some("t3".into()), Outcome::Invalid, None)?,
        Status::Blocked
    );
    assert_eq!(MAX_FAILED_ATTEMPTS, 3);
    assert_eq!(state.next().err().as_deref(), Some(first.as_str()));

    // A counted trial, pass or fail, is the run's result and is final.
    let mut state = CampaignState::new(2, ClientName::Codex, "0.2.0-rc.1")?;
    let first = state.next()?.ok_or("nothing")?.run.run_id.clone();
    assert_eq!(
        state.mark(&first, Some("t1".into()), Outcome::Counted, None)?,
        Status::Counted
    );
    assert!(
        state
            .mark(&first, Some("t2".into()), Outcome::Counted, None)
            .is_err()
    );
    let second = state.next()?.ok_or("nothing")?.run.run_id.clone();
    assert_ne!(first, second, "the next run moves on");
    assert!(
        state
            .mark("no-such-run", None, Outcome::Counted, None)
            .is_err()
    );
    Ok(())
}

#[test]
fn a_whole_batch_runs_to_nothing_left_and_resumes_from_its_file() -> TestResult {
    let scratch = Scratch::new("campaign")?;
    let file = scratch.path().join("state.json");
    let state = CampaignState::new(3, ClientName::Claude, "0.2.0-rc.1")?;
    state.save(&file)?;
    let total = state.runs.len();
    let mut done = 0;
    loop {
        // Every step reloads the file, as the scripts do: a stop and a restart
        // lose nothing.
        let mut loaded = CampaignState::load(&file)?;
        let Some(next) = loaded.next()?.map(|state| state.run.run_id.clone()) else {
            break;
        };
        loaded.mark(&next, Some(format!("trial-{done}")), Outcome::Counted, None)?;
        loaded.save(&file)?;
        done += 1;
    }
    assert_eq!(done, total);
    assert!(
        CampaignState::load(&file)?
            .describe()
            .contains(&format!("{total} counted, 0 pending"))
    );
    // Nothing the state file holds is a path, a prompt or a name.
    let text = std::fs::read_to_string(&file)?;
    assert!(!text.contains('\\') && !text.contains(":/"), "{text}");
    Ok(())
}

#[test]
fn the_reserve_is_added_by_hand_and_capped() -> TestResult {
    let mut state = CampaignState::new(2, ClientName::Codex, "0.2.0-rc.1")?;
    for index in 0..MAX_RESERVE_PER_STATE {
        let id = state.add_reserve("A-09-f05-supplied", Tier::Compact)?;
        assert!(id.contains("reserve"), "{id}");
        assert_eq!(
            state
                .runs
                .iter()
                .filter(|run| run.run.kind == RunKind::Reserve)
                .count(),
            index + 1
        );
    }
    assert!(
        state
            .add_reserve("A-09-f05-supplied", Tier::Compact)
            .is_err()
    );
    // A reserve run is last, so it never jumps the plan's order.
    assert_eq!(
        state.runs.last().map(|run| run.run.kind),
        Some(RunKind::Reserve)
    );
    Ok(())
}

#[test]
fn a_version_must_be_exact() {
    for bad in ["next", "latest", "^0.1.0", ""] {
        assert!(
            CampaignState::new(1, ClientName::Claude, bad).is_err(),
            "{bad:?}"
        );
    }
    assert!(CampaignState::new(1, ClientName::Claude, "0.1.0").is_ok());
}
