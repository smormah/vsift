//! Every committed scenario is consistent with the corpus truth and the
//! skill, covers the verification rows it must, and never copies truth.

use std::{
    collections::BTreeSet,
    error::Error,
    path::{Path, PathBuf},
};

use vsift_agent_trials::{
    scenario::Scenario,
    skill::{
        CHECK_IMAGES, SkillReferences, check_image_for, current_check_image, files_below,
        workspace_image_code,
    },
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
    let codes: Vec<String> = CHECK_IMAGES
        .iter()
        .map(|image| image.code().to_ascii_lowercase())
        .collect();
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
            for code in &codes {
                assert!(
                    !phase.prompt.to_ascii_lowercase().contains(code.as_str()),
                    "{}",
                    scenario.id
                );
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
    Ok(())
}

/// The skill's check image is the table's current one, so the grader's
/// truth and the pixels cannot drift apart, and every image the skill ever
/// shipped stays known, so an older trial can still be graded again.
#[test]
fn the_shipped_check_image_is_the_current_known_one() -> TestResult {
    let bytes = std::fs::read(repository().join("skills/vsift/assets/image-check.png"))?;
    let shipped =
        check_image_for(&bytes).ok_or("the skill's check image is not in CHECK_IMAGES")?;
    assert_eq!(shipped.sha256, current_check_image().sha256);
    assert!(current_check_image().retired.is_none());
    let retired = CHECK_IMAGES.len() - 1;
    assert!(
        CHECK_IMAGES
            .iter()
            .take(retired)
            .all(|image| image.retired.is_some()),
        "only the last image is current"
    );
    let mut digests: Vec<&str> = CHECK_IMAGES.iter().map(|image| image.sha256).collect();
    digests.sort_unstable();
    digests.dedup();
    assert_eq!(digests.len(), CHECK_IMAGES.len());

    // A workspace's skill copy names its own image; an unknown one names none.
    let workspace = std::env::temp_dir().join(format!(
        "vsift-check-image-{}",
        vsift_agent_trials::skill::random_hex(8)?
    ));
    let claude = workspace.join(".claude/skills/vsift");
    let codex = workspace.join(".agents/skills/vsift");
    std::fs::create_dir_all(codex.join("assets"))?;
    std::fs::write(codex.join("assets/image-check.png"), &bytes)?;
    let directories = [claude.clone(), codex.clone()];
    assert_eq!(
        workspace_image_code(&directories),
        Some(current_check_image().code())
    );
    std::fs::write(codex.join("assets/image-check.png"), b"not the image")?;
    assert_eq!(workspace_image_code(&directories), None);
    std::fs::remove_dir_all(&workspace)?;
    Ok(())
}

/// Issue #219: `-stream_loop` with stream copy starts each copy of F02
/// (12 s) 12.064 s after the one before, so the A-02 clip of 41 copies
/// lasts 494.559875 s, not 492 s. The period is derived from `VSift`'s
/// measurement of the clip; the nominal period drifts 2.56 s by the last
/// copy and would place a frame showing 12 in the window of 0.
#[test]
fn a_looped_clip_repeats_its_truth_with_the_measured_period() -> TestResult {
    use vsift_agent_trials::scenario::PeriodBasis;
    let truth = CorpusTruth::load(&repository().join("fixtures/corpus"))?;
    let looped = Scenario::load(
        &repository().join("tools/vsift-agent-trials/scenarios/A-02-f02-compact-resume.json"),
    )?;
    let measured = looped.timeline(&truth, Some(494_559_875))?;
    assert_eq!(measured.basis, PeriodBasis::Measured);
    assert_eq!(measured.period_us, 12_063_996);
    assert_eq!(measured.total_us, 494_559_875);
    // GPT-6-Sol's A-02 run 2 phase 2 cited this frame for "depth 12": in
    // the clip it lies 7.45 s into the 41st copy, inside F02-E02 (4-8 s).
    assert!((4_000_000..8_000_000).contains(&measured.phase(490_009_863)));
    // The second copy's first frame (12.063964 s) lies a rounding error
    // before the derived start, and its 4 s frame (16.063964 s) 32 µs
    // before the window: both land at their own fixture time.
    assert!(measured.phase(12_063_964) < 50_000);
    assert!((4_000_000..8_000_000).contains(&measured.phase(16_063_964)));
    // The frames just before and at the next change stay in their windows.
    assert!(measured.phase(12_063_964 + 3_950_000) < 4_000_000);
    assert!(measured.phase(12_063_964 + 8_000_000) >= 8_000_000);

    let nominal = looped.timeline(&truth, None)?;
    assert_eq!(nominal.basis, PeriodBasis::Nominal);
    assert_eq!(
        (nominal.period_us, nominal.total_us),
        (12_000_000, 492_000_000)
    );
    assert!((8_000_000..12_000_000).contains(&nominal.phase(490_009_863)));

    // A measurement more than 2% from the nominal period is not trusted.
    let off = looped.timeline(&truth, Some(600_000_000))?;
    assert_eq!(off.basis, PeriodBasis::Nominal);

    let single = Scenario::load(
        &repository().join("tools/vsift-agent-trials/scenarios/A-03-f05-supplied.json"),
    )?;
    let timeline = single.timeline(&truth, Some(20_100_000))?;
    assert_eq!(timeline.basis, PeriodBasis::NotLooped);
    assert_eq!(timeline.period_us, timeline.fixture_us);
    Ok(())
}
