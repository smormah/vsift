//! Prepares one value-measurement trial: a task, an arm and a run index
//! (design sections 2 and 3).
//!
//! Every arm gets the same recording, transcript, model, tools, settings and
//! budget. Only what is added differs: `vsift` on `PATH` and registered (V0,
//! V1), and a skill folder (the raw guide for R1, `VSift`'s skill for V1).
//! The tools sit in a folder of their own beside the workspace, which `run`
//! puts on the client's `PATH`; the model is placed inside the workspace, so
//! no arm needs to read outside it. The trial folder is named `run-<hex>`
//! for every arm, so no path tells the agent which arm or task it is in.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::task::{Arm, Media, MODEL_IN_WORKSPACE, Task, TaskBudget, Transcript};
use crate::{
    cold::assert_cold_workspace,
    error::{TrialError, write_json},
    install::InstallProof,
    layout::{SkillSource, ToolSource, TrialLayout, TrialManifest},
    prepare::srt_time,
    roots::RootPolicy,
    scenario::TrialMode,
    skill::{copy_directory, directory_digest, file_digest, random_hex, skill_directory},
    truth::CorpusTruth,
    vsift_cli::{VsiftCli, arguments},
};

/// The Claude Code settings every arm gets (planner's ruling, question 3).
pub const VALUE_SETTINGS: &str = "tools/vsift-agent-trials/claude-value-trial-settings.json";

/// The public copy of the method, frozen with the measurement.
pub const METHOD_NOTE: &str = "docs/planning/value-measurement/method.md";

/// The programs every arm may run besides the tools (planner's ruling,
/// question 3); held equal to the settings file by a test.
pub const VALUE_HELPERS: [&str; 18] = [
    "ls", "cat", "head", "tail", "pwd", "cd", "wc", "echo", "sort", "grep", "awk", "cut", "tr",
    "uniq", "mkdir", "jq", "sed", "find",
];

/// The media tools every arm has on `PATH`.
pub const VALUE_TOOLS: [&str; 3] = ["ffmpeg", "ffprobe", "whisper-cli"];

/// What `value prepare` needs.
#[derive(Clone, Debug)]
pub struct ValuePrepareRequest {
    /// The neutral trial root.
    pub root: PathBuf,
    /// The repository (corpus, settings, skill, method note).
    pub repository: PathBuf,
    /// The task file.
    pub task: PathBuf,
    /// The arm.
    pub arm: Arm,
    /// Which of the cell's runs this is, from 1 (run pairs are by index).
    pub run_index: u32,
    /// A pilot: a task whose truth is pending may run, and the record says
    /// it never counts.
    pub pilot: bool,
    /// The `vsift` executable (absolute), for V arms built from a commit.
    pub vsift: Option<PathBuf>,
    /// A published install, for V arms (the counted runs).
    pub install: Option<InstallProof>,
    /// The `VSift` commit under test.
    pub vsift_commit: String,
    /// `FFmpeg` (absolute).
    pub ffmpeg: PathBuf,
    /// `FFprobe` (absolute).
    pub ffprobe: PathBuf,
    /// whisper.cpp's CLI (absolute).
    pub whisper: PathBuf,
    /// The ggml model (absolute).
    pub model: PathBuf,
    /// The recording, for a task from the test-media release.
    pub media: Option<PathBuf>,
    /// The transcript file, for a task that supplies one by file.
    pub transcript: Option<PathBuf>,
    /// The frozen raw guide's folder (its digest is in every arm's record).
    pub guide: PathBuf,
    /// The digest of the value freeze the trial runs under.
    pub freeze_sha256: Option<String>,
    /// What the root must avoid.
    pub root_policy: RootPolicy,
    /// Where the cold-workspace check stops looking up (tests only).
    pub cold_scan_stop: Option<PathBuf>,
}

/// What a value trial adds to the trial manifest.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ValueManifest {
    /// The arm.
    pub arm: Arm,
    /// The task's identifier.
    pub task_id: String,
    /// The method's task number (`T2`).
    pub task: String,
    /// The task file's SHA-256.
    pub task_sha256: String,
    /// The run index within its cell.
    pub run_index: u32,
    /// Whether this is a pilot (never counted).
    pub pilot: bool,
    /// The raw guide's folder digest.
    pub guide_sha256: String,
    /// The raw guide's skill name.
    pub guide_name: String,
    /// The public method note's SHA-256.
    pub method_sha256: String,
    /// The limits every arm runs under.
    pub budget: TaskBudget,
    /// The folder of tools `run` puts on `PATH`.
    pub tools_directory: PathBuf,
}

fn create(path: &Path) -> Result<(), TrialError> {
    fs::create_dir_all(path).map_err(|error| TrialError::io_at(path, error))
}

/// The guide's skill name, from its `SKILL.md` front matter.
///
/// # Errors
///
/// [`TrialError::Invalid`] when the guide has no `SKILL.md` or no `name`, or
/// the name mentions `VSift`.
pub fn guide_name(guide: &Path) -> Result<String, TrialError> {
    let file = guide.join("SKILL.md");
    let text = fs::read_to_string(&file).map_err(|error| TrialError::io_at(&file, error))?;
    let name = text
        .lines()
        .skip_while(|line| line.trim() != "---")
        .skip(1)
        .take_while(|line| line.trim() != "---")
        .find_map(|line| line.strip_prefix("name:"))
        .map(|name| name.trim().trim_matches('"').to_owned())
        .filter(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
        .ok_or_else(|| {
            TrialError::Invalid("the guide's SKILL.md has no usable `name:` in its front matter".to_owned())
        })?;
    if name.to_ascii_lowercase().contains("vsift") {
        return Err(TrialError::Invalid("the guide's name may not mention VSift".to_owned()));
    }
    Ok(name)
}

/// Places a file at `target`: a link where the platform allows one, else a
/// copy. A link keeps a 150 MB model from being copied for every run.
fn place(source: &Path, target: &Path) -> Result<(), TrialError> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target).map_err(|error| TrialError::io_at(target, error))
    }
    #[cfg(not(unix))]
    {
        fs::copy(source, target)
            .map(|_| ())
            .map_err(|error| TrialError::io_at(target, error))
    }
}

fn checked_copy(source: &Path, sha256: &str, target: &Path) -> Result<String, TrialError> {
    let actual = file_digest(source)?;
    if !actual.eq_ignore_ascii_case(sha256) {
        return Err(TrialError::Refused(format!(
            "{} is not the file the task names (SHA-256 differs)",
            source.display()
        )));
    }
    fs::copy(source, target).map_err(|error| TrialError::io_at(target, error))?;
    Ok(actual)
}

fn absolute_file(path: &Path, what: &str) -> Result<(), TrialError> {
    if path.is_absolute() && path.is_file() {
        Ok(())
    } else {
        Err(TrialError::Refused(format!("--{what} must be an absolute path to a file")))
    }
}

/// Prepares a value trial and returns its layout.
///
/// # Errors
///
/// [`TrialError`] when the root is refused, the task is invalid, a file is
/// missing or not the one the task names, or a preparation step fails.
#[allow(
    clippy::too_many_lines,
    reason = "One function reads top to bottom as the order of a trial's preparation"
)]
pub fn prepare(request: &ValuePrepareRequest) -> Result<TrialLayout, TrialError> {
    request.root_policy.check(&request.root)?;
    for (path, what) in [
        (&request.ffmpeg, "ffmpeg"),
        (&request.ffprobe, "ffprobe"),
        (&request.whisper, "whisper"),
        (&request.model, "model"),
    ] {
        absolute_file(path, what)?;
    }
    let corpus = CorpusTruth::load(&request.repository.join("fixtures").join("corpus"))?;
    let task = Task::load(&request.task)?;
    task.validate(&corpus, !request.pilot)?;
    let guide_name = guide_name(&request.guide)?;
    let binary = if request.arm.has_vsift() {
        let binary = request.install.as_ref().map_or_else(
            || request.vsift.clone(),
            |proof| Some(proof.native_executable.clone()),
        );
        let binary = binary.ok_or_else(|| {
            TrialError::Refused("a V arm needs --vsift or --install-proof".to_owned())
        })?;
        absolute_file(&binary, "vsift")?;
        Some(binary)
    } else {
        None
    };

    let trial_id = format!("run-{}", random_hex(4)?);
    let layout = TrialLayout::new(request.root.join(&trial_id));
    if layout.trial().exists() {
        return Err(TrialError::Refused("the trial directory already exists".to_owned()));
    }
    let workspace = layout.workspace();
    create(&workspace)?;
    create(&layout.harness())?;
    for (_, directory) in layout.user_environment() {
        create(&directory)?;
    }
    let mut hashes = BTreeMap::new();

    // The recording, under a neutral name.
    let video = workspace.join(task.video_name());
    match &task.media {
        Media::Corpus { file, .. } => {
            let relative = format!("fixtures/corpus/generated/{file}");
            let source = request.repository.join(&relative);
            hashes.insert(relative, file_digest(&source)?);
            fs::copy(&source, &video).map_err(|error| TrialError::io_at(&video, error))?;
        }
        Media::TestMediaRelease { item, sha256, .. } => {
            let source = request.media.as_ref().ok_or_else(|| {
                TrialError::Refused(format!("{} needs --media (the {item} file)", task.id))
            })?;
            absolute_file(source, "media")?;
            hashes.insert(item.clone(), checked_copy(source, sha256, &video)?);
        }
    }
    // The transcript every arm is given.
    if let (Some(spec), Some(name)) = (&task.transcript, task.transcript_name()) {
        let target = workspace.join(&name);
        match spec {
            Transcript::FromScript => {
                let Media::Corpus { fixture, .. } = &task.media else {
                    return Err(TrialError::Invalid("a script transcript needs a corpus fixture".to_owned()));
                };
                let fixture = corpus.fixture(fixture)?;
                let span = corpus.speech_span(&fixture.id).ok_or_else(|| {
                    TrialError::Invalid(format!("{} has no speech placement", fixture.id))
                })?;
                fs::write(
                    &target,
                    format!(
                        "1\n{} --> {}\n{}\n",
                        srt_time(span.start_us),
                        srt_time(span.end_us),
                        fixture.audio.script
                    ),
                )
                .map_err(|error| TrialError::io_at(&target, error))?;
            }
            Transcript::File { sha256, .. } => {
                let source = request.transcript.as_ref().ok_or_else(|| {
                    TrialError::Refused(format!("{} needs --transcript", task.id))
                })?;
                absolute_file(source, "transcript")?;
                checked_copy(source, sha256, &target)?;
            }
        }
        hashes.insert(name, file_digest(&target)?);
    }
    // The model, inside the workspace for every arm.
    let model = workspace.join(MODEL_IN_WORKSPACE);
    if let Some(parent) = model.parent() {
        create(parent)?;
    }
    place(&request.model, &model)?;
    hashes.insert(MODEL_IN_WORKSPACE.to_owned(), file_digest(&request.model)?);
    // The tools, in a folder of their own outside the workspace.
    let tools_directory = layout.trial().join("tools");
    create(&tools_directory)?;
    for (name, source) in [
        ("ffmpeg", &request.ffmpeg),
        ("ffprobe", &request.ffprobe),
        ("whisper-cli", &request.whisper),
    ] {
        let file_name = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
        place(source, &tools_directory.join(file_name))?;
    }

    // Nothing but the arm's own skill may be in reach: prove the workspace
    // is cold before the arm's skill goes in.
    let mut cold_assertions =
        assert_cold_workspace(&workspace, request.cold_scan_stop.as_deref(), &[])?;
    let repository_skill = skill_directory(&request.repository);
    let skills = workspace.join(".claude").join("skills");
    let (skill_source, skill_sha256) = match request.arm {
        Arm::R0 | Arm::V0 => (SkillSource::None, "none".to_owned()),
        Arm::R1 => {
            copy_directory(&request.guide, &skills.join(&guide_name))?;
            cold_assertions.push(format!("the only skill is the raw guide `{guide_name}`"));
            (SkillSource::Repository, directory_digest(&request.guide)?)
        }
        Arm::V1 => {
            let source = request
                .install
                .as_ref()
                .map_or_else(|| repository_skill.clone(), |proof| proof.package_root.join("skills").join("vsift"));
            if directory_digest(&source)? != directory_digest(&repository_skill)? {
                return Err(TrialError::Invalid(
                    "the skill inside the installed package differs from the checkout's".to_owned(),
                ));
            }
            copy_directory(&source, &skills.join("vsift"))?;
            cold_assertions.push("the only skill is vsift".to_owned());
            let kind = if request.install.is_some() {
                SkillSource::InstalledPackage
            } else {
                SkillSource::Repository
            };
            (kind, directory_digest(&source)?)
        }
    };

    // The same settings for every arm.
    let settings_text = fs::read_to_string(request.repository.join(VALUE_SETTINGS))
        .map_err(|error| TrialError::io_at(&request.repository.join(VALUE_SETTINGS), error))?;
    let settings_folder = workspace.join(".claude");
    create(&settings_folder)?;
    fs::write(settings_folder.join("settings.json"), &settings_text)
        .map_err(|error| TrialError::io_at(&settings_folder.join("settings.json"), error))?;

    // V arms: VSift finds the same tools by registration, never by search.
    let vsift_sha256 = match &binary {
        Some(binary) => {
            let cli = VsiftCli::new(binary, &layout)?;
            for (dependency, path) in [("ffmpeg", &request.ffmpeg), ("ffprobe", &request.ffprobe), ("whisper", &request.whisper)] {
                cli.ok(&arguments(&[&"setup", &"configure", &dependency, &"--executable", path, &"--json"]))?;
            }
            cli.ok(&arguments(&[&"setup", &"configure-model", &"--file", &request.model, &"--json"]))?;
            file_digest(binary)?
        }
        None => "none".to_owned(),
    };

    let prompts = (0..task.phases.len())
        .map(|phase| task.prompt(phase, request.arm, &guide_name))
        .collect::<Result<Vec<_>, _>>()?;
    fs::copy(&request.task, layout.harness().join("task.json"))
        .map_err(|error| TrialError::io_at(&layout.harness().join("task.json"), error))?;
    let value = ValueManifest {
        arm: request.arm,
        task_id: task.id.clone(),
        task: task.task.clone(),
        task_sha256: file_digest(&request.task)?,
        run_index: request.run_index,
        pilot: request.pilot,
        guide_sha256: directory_digest(&request.guide)?,
        guide_name,
        method_sha256: file_digest(&request.repository.join(METHOD_NOTE))?,
        budget: task.budget,
        tools_directory,
    };
    let manifest = TrialManifest {
        schema_version: 1,
        trial_id,
        scenario_id: task.id.clone(),
        scenario_sha256: value.task_sha256.clone(),
        repository: request.repository.clone(),
        video: task.video_name(),
        transcript: task.transcript_name(),
        canary_variable: None,
        canaries: Vec::new(),
        prepared: crate::layout::PreparedState::default(),
        fixture_hashes: hashes,
        skill_sha256,
        settings_sha256: crate::skill::sha256_hex(settings_text.as_bytes()),
        vsift_commit: request.vsift_commit.clone(),
        vsift_sha256,
        vsift_executable: binary.unwrap_or_default(),
        prompts,
        mode: match request.arm {
            Arm::R0 | Arm::V0 => TrialMode::Cold,
            Arm::R1 | Arm::V1 => TrialMode::Skill,
        },
        holdout: false,
        skill_source,
        install: request.install.as_ref().map(|proof| proof.evidence.clone()),
        install_prefix: request.install.as_ref().map(|proof| proof.prefix.clone()),
        client_path_directories: request
            .install
            .as_ref()
            .map(|proof| proof.client_path_directories.clone())
            .unwrap_or_default(),
        tools_source: ToolSource::Registered,
        setup_check: None,
        freeze_sha256: request.freeze_sha256.clone(),
        cold_assertions,
        cold_variant: None,
        value: Some(value),
    };
    write_json(&layout.manifest(), &manifest)?;
    Ok(layout)
}

/// Writes phase `phase - 1`'s final answer to the workspace before phase
/// `phase` starts, the same way for every arm (planner's ruling, question
/// 10): a fresh client finds the earlier answer and the earlier work, and
/// no resume card that only `VSift` could read.
///
/// # Errors
///
/// [`TrialError`] when the earlier phase's run or log cannot be read or
/// gave no answer.
pub fn write_earlier_answer(layout: &TrialLayout, phase: usize) -> Result<(), TrialError> {
    let Some(earlier) = phase.checked_sub(1).filter(|earlier| *earlier >= 1) else {
        return Err(TrialError::Invalid(format!(
            "phase {phase} has no earlier phase to carry an answer from"
        )));
    };
    let record = crate::run::read_run(layout, earlier)?;
    let (stdout, _) = crate::run::raw_output(&record)?;
    let trace = crate::trace::parse(record.client, &stdout)?;
    let answer = trace.final_message.filter(|text| !text.trim().is_empty()).ok_or_else(|| {
        TrialError::Invalid(format!("phase {earlier} gave no answer to carry into phase {phase}"))
    })?;
    let file = layout.workspace().join(super::task::EARLIER_ANSWER_FILE);
    fs::write(&file, format!("{}\n", answer.trim_end()))
        .map_err(|error| TrialError::io_at(&file, error))
}

/// The client's `PATH` for a value trial: `vsift`'s folder (V arms only),
/// the tools folder, the operator's extra folders and the system folders.
/// A media tool anywhere but the tools folder, or `vsift` anywhere in an R
/// arm, is refused, so every arm runs the pinned tools and R arms cannot
/// reach `VSift`.
///
/// # Errors
///
/// [`TrialError::Refused`] as above.
pub fn client_path(
    manifest: &TrialManifest,
    value: &ValueManifest,
    extra: &[PathBuf],
    system: &[PathBuf],
) -> Result<OsString, TrialError> {
    let mut directories: Vec<PathBuf> = Vec::new();
    if value.arm.has_vsift() {
        if manifest.client_path_directories.is_empty() {
            directories.extend(manifest.vsift_executable.parent().map(Path::to_path_buf));
        } else {
            directories.extend(manifest.client_path_directories.iter().cloned());
        }
    }
    let others: Vec<PathBuf> = extra.iter().chain(system).cloned().collect();
    for directory in directories.iter().chain(&others) {
        for tool in VALUE_TOOLS {
            if [tool.to_owned(), format!("{tool}.exe")]
                .iter()
                .any(|name| directory.join(name).is_file())
            {
                return Err(TrialError::Refused(format!(
                    "{tool} is reachable on the client's PATH outside the trial's tools folder"
                )));
            }
        }
    }
    if !value.arm.has_vsift() {
        for directory in &others {
            if ["vsift", "vsift.exe", "vsift.cmd", "vsift.ps1"]
                .iter()
                .any(|name| directory.join(name).exists())
            {
                return Err(TrialError::Refused(
                    "vsift is reachable on an R arm's PATH".to_owned(),
                ));
            }
        }
    }
    directories.push(value.tools_directory.clone());
    directories.extend(others);
    std::env::join_paths(directories)
        .map_err(|error| TrialError::Refused(format!("a PATH directory is not usable: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_name_comes_from_its_front_matter() {
        let folder = std::env::temp_dir().join(format!("value-guide-{}", std::process::id()));
        let _ = fs::create_dir_all(&folder);
        let write = |text: &str| fs::write(folder.join("SKILL.md"), text);
        assert!(write("---\nname: recording-evidence\ndescription: x\n---\nBody\n").is_ok());
        assert_eq!(guide_name(&folder).ok(), Some("recording-evidence".to_owned()));
        assert!(write("---\nname: vsift-raw\n---\n").is_ok());
        assert!(guide_name(&folder).is_err());
        assert!(write("No front matter\n").is_ok());
        assert!(guide_name(&folder).is_err());
        let _ = fs::remove_dir_all(&folder);
    }

    /// The settings file and [`VALUE_HELPERS`] agree, every arm's rules are
    /// the same file, and nothing that writes outside a tool or reaches the
    /// network is allowed.
    #[test]
    fn the_value_settings_allow_exactly_the_tools_and_helpers() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("claude-value-trial-settings.json");
        let settings = crate::error::read_json(&path);
        assert!(settings.is_ok(), "{settings:?}");
        let settings = settings.unwrap_or_default();
        let allowed: Vec<String> = settings["permissions"]["allow"]
            .as_array()
            .map(|rules| rules.iter().filter_map(|rule| rule.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        let mut expected: Vec<String> = ["vsift"]
            .iter()
            .chain(VALUE_TOOLS.iter())
            .chain(VALUE_HELPERS.iter())
            .map(|name| format!("Bash({name}:*)"))
            .collect();
        expected.push("Read(./**)".to_owned());
        let mut sorted = allowed.clone();
        sorted.sort();
        expected.sort();
        assert_eq!(sorted, expected);
        let denied = settings["permissions"]["deny"].to_string();
        for tool in ["WebFetch", "WebSearch", "Write", "Edit", "Task"] {
            assert!(denied.contains(tool), "{tool} is not denied");
        }
    }
}
