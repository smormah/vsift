//! Where a trial's files live, and the facts `prepare` hands to the later
//! steps.
//!
//! ```text
//! <root>/<trial-id>/
//!   workspace/                 the client's working directory
//!     TASK.md                  the rendered prompt of phase 1
//!     walkthrough.mp4          the video under a neutral name
//!     walkthrough.srt          a supplied transcript, if the scenario has one
//!     .claude/skills/vsift/    the skill, for Claude Code (project scope)
//!     .agents/skills/vsift/    the skill, for Codex (project scope)
//!     .claude/settings.json    the committed trial settings (Claude Code)
//!     .home/                   the isolated per-user base: `VSift`'s
//!                              configuration and its session root
//!   harness/                   never shown to the client
//!     trial.json               this module's TrialManifest
//!     scenario.json            the scenario, frozen at preparation
//!     phase-<n>/run.json       what `run` did
//!     phase-<n>/grade.json     what `grade` decided
//!     raw/phase-<n>/           the client's raw stdout and stderr
//! ```
//!
//! The per-user base is inside the workspace on purpose: the client may
//! read `VSift`'s delivered images there without a path rule that names the
//! machine, and the committed Claude Code settings can say `Read(./**)`.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{install::InstallEvidence, scenario::TrialMode};

/// The directory layout of one trial.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrialLayout {
    trial: PathBuf,
}

impl TrialLayout {
    /// The layout of the trial directory `trial`.
    #[must_use]
    pub fn new(trial: impl Into<PathBuf>) -> Self {
        Self {
            trial: trial.into(),
        }
    }

    /// The trial directory.
    #[must_use]
    pub fn trial(&self) -> &Path {
        &self.trial
    }

    /// The client's working directory.
    #[must_use]
    pub fn workspace(&self) -> PathBuf {
        self.trial.join("workspace")
    }

    /// The harness's own directory, never shown to the client.
    #[must_use]
    pub fn harness(&self) -> PathBuf {
        self.trial.join("harness")
    }

    /// `harness/trial.json`.
    #[must_use]
    pub fn manifest(&self) -> PathBuf {
        self.harness().join("trial.json")
    }

    /// `harness/scenario.json`.
    #[must_use]
    pub fn scenario(&self) -> PathBuf {
        self.harness().join("scenario.json")
    }

    /// `harness/phase-<n>`.
    #[must_use]
    pub fn phase(&self, phase: usize) -> PathBuf {
        self.harness().join(format!("phase-{phase}"))
    }

    /// `harness/raw/phase-<n>`, the raw client logs.
    #[must_use]
    pub fn raw(&self, phase: usize) -> PathBuf {
        self.harness().join("raw").join(format!("phase-{phase}"))
    }

    /// The isolated per-user base.
    #[must_use]
    pub fn user_base(&self) -> PathBuf {
        self.workspace().join(".home")
    }

    /// The skill directories the client reads.
    #[must_use]
    pub fn skill_directories(&self) -> [PathBuf; 2] {
        let workspace = self.workspace();
        [
            workspace.join(".claude").join("skills").join("vsift"),
            workspace.join(".agents").join("skills").join("vsift"),
        ]
    }

    /// The environment that points `VSift` (and the client) at the isolated
    /// per-user base: the `LOCALAPPDATA` / `XDG_CONFIG_HOME` pattern of the
    /// P06-P11 checkpoints, plus the cache and home variables that decide
    /// the default session root.
    #[must_use]
    pub fn user_environment(&self) -> Vec<(String, PathBuf)> {
        let base = self.user_base();
        vec![
            (
                "LOCALAPPDATA".to_owned(),
                base.join("AppData").join("Local"),
            ),
            ("APPDATA".to_owned(), base.join("AppData").join("Roaming")),
            ("XDG_CONFIG_HOME".to_owned(), base.join(".config")),
            ("XDG_CACHE_HOME".to_owned(), base.join(".cache")),
            ("HOME".to_owned(), base.clone()),
            ("USERPROFILE".to_owned(), base),
        ]
    }

    /// The default session root `VSift` resolves under [`Self::user_environment`]
    /// (the engine's `platform_session_root`).
    #[must_use]
    pub fn session_root(&self) -> PathBuf {
        let base = self.user_base();
        if cfg!(windows) {
            base.join("AppData").join("Local").join("VSift-sessions")
        } else if cfg!(target_os = "macos") {
            base.join("Library").join("Caches").join("VSift-sessions")
        } else {
            base.join(".cache").join("vsift-sessions")
        }
    }

    /// The per-user configuration directory `VSift` resolves under
    /// [`Self::user_environment`].
    #[must_use]
    pub fn user_configuration(&self) -> PathBuf {
        let base = self.user_base();
        if cfg!(windows) {
            base.join("AppData").join("Local").join("vsift")
        } else if cfg!(target_os = "macos") {
            base.join("Library")
                .join("Application Support")
                .join("vsift")
        } else {
            base.join(".config").join("vsift")
        }
    }
}

/// State the preparer created before the agent starts.
#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct PreparedState {
    /// A prepared session (expired or holding an interrupted job).
    pub session_id: Option<String>,
    /// Its transcript revision, if it has one.
    pub revision_id: Option<String>,
    /// The interrupted transcription job.
    pub job_id: Option<String>,
    /// That job's operation identifier.
    pub operation_id: Option<String>,
}

/// What `prepare` records for `run`, `grade` and `record`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TrialManifest {
    /// Record version, 1.
    pub schema_version: u32,
    /// The trial's identity (its directory name).
    pub trial_id: String,
    /// The scenario's identity.
    pub scenario_id: String,
    /// SHA-256 of the scenario file.
    pub scenario_sha256: String,
    /// The repository the skill, corpus and scenarios came from.
    pub repository: PathBuf,
    /// The video's name in the workspace.
    pub video: String,
    /// The supplied transcript's name in the workspace, if any.
    pub transcript: Option<String>,
    /// The canary environment variable's name, if canaries are planted.
    pub canary_variable: Option<String>,
    /// Canary values (the environment variable's and the `.env` file's).
    pub canaries: Vec<String>,
    /// Sessions and jobs prepared before the agent starts.
    pub prepared: PreparedState,
    /// SHA-256 of every input file, by workspace-relative name or fixture
    /// path.
    pub fixture_hashes: BTreeMap<String, String>,
    /// Digest of the skill directory as copied.
    pub skill_sha256: String,
    /// SHA-256 of the Claude Code trial settings as written.
    pub settings_sha256: String,
    /// The `VSift` commit the operator named.
    pub vsift_commit: String,
    /// SHA-256 of the `vsift` executable the trial uses.
    pub vsift_sha256: String,
    /// The `vsift` executable's absolute path.
    pub vsift_executable: PathBuf,
    /// Each phase's prompt; a later phase may still hold `{{resume_card}}`.
    pub prompts: Vec<String>,
    /// Whether the agent got the skill or none (P14). Absent in manifests
    /// written before it existed, which are skill trials.
    #[serde(default)]
    pub mode: TrialMode,
    /// Whether the scenario is one of the hold-outs listed in
    /// `tools/vsift-agent-trials/holdout/INDEX.json`.
    #[serde(default)]
    pub holdout: bool,
    /// Where the skill copy came from: `repository` (P12), `installed_package`
    /// (a clean install: the copy the npm package ships, checked equal to the
    /// repository's) or `none` (a cold trial).
    #[serde(default)]
    pub skill_source: SkillSource,
    /// What the published install proves; `None` for a binary built from the
    /// checkout (P12). Free of local paths.
    #[serde(default)]
    pub install: Option<InstallEvidence>,
    /// The install's npm prefix, so the record can name it by token.
    #[serde(default)]
    pub install_prefix: Option<PathBuf>,
    /// The directories the client's `PATH` gets in a clean-install trial
    /// (npm's command folder, Node.js's); empty otherwise, when the client
    /// gets the directory of `vsift_executable`.
    #[serde(default)]
    pub client_path_directories: Vec<PathBuf>,
    /// How the dependencies came to be there.
    #[serde(default)]
    pub tools_source: ToolSource,
    /// What `vsift setup check` reported after the harness set the tools up
    /// and before the agent started (clean-install trials only).
    #[serde(default)]
    pub setup_check: Option<SetupCheckState>,
    /// The digest of the freeze file the trial was prepared under, if the
    /// operator named one ([`crate::freeze`]).
    #[serde(default)]
    pub freeze_sha256: Option<String>,
    /// What `prepare` proved about a cold workspace (the skill and the
    /// documentation absent from every discovery location).
    #[serde(default)]
    pub cold_assertions: Vec<String>,
}

/// Where a trial's skill copy came from.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    /// The checkout's `skills/vsift` (P12).
    #[default]
    Repository,
    /// The copy inside the installed npm package.
    InstalledPackage,
    /// No skill (a cold trial).
    None,
}

/// How a trial's media tools were provided.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    /// The harness registered executables by absolute path with `setup
    /// configure`, as P12 did (and as a Windows user must).
    #[default]
    Registered,
    /// The harness ran `setup plan` and `setup install` for the managed
    /// tools, as a user on Ubuntu 24.04 does. The agent never does.
    Managed,
}

/// What `vsift setup check` reported, kept to public fields and no path.
#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct SetupCheckState {
    /// The command's exit code.
    pub exit_code: Option<i32>,
    /// The overall status (`ready`, `blocked`, ...).
    pub status: String,
    /// Each dependency: its name, status and where it was found.
    pub dependencies: Vec<DependencyState>,
    /// The local speech recognition verification status, when reported.
    pub local_asr: Option<String>,
}

/// One dependency of `setup check`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct DependencyState {
    /// `ffmpeg`, `ffprobe`, `whisper`.
    pub dependency: String,
    /// `ready`, `missing`, ...
    pub status: String,
    /// How it was looked up (`configured`, `managed`, `filtered_path`).
    pub lookup: String,
}
