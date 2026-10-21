//! The optional online-video components in the setup contract (#409, ADR
//! 0028 section 4): a desktop plan offers them with one plain sentence, the
//! command that adds them and the instruction to ask the operator; an
//! included one's actions follow the core's; a failure of the optional part
//! leaves `setup install` a success that says the core is ready; and `setup
//! check` reports online videos apart from local readiness. Every result
//! validates against its published schema. Set
//! `VSIFT_REGENERATE_CONTRACT_EXAMPLES=1` to rewrite the examples, and review
//! the diff.

use std::{env, fs, io, path::PathBuf};

use serde_json::Value;
use vsift_application::{
    AcceptedManagedArtifact, AcceptedManagedCatalogue, ComponentInstallFailure,
    ComponentInstallOutcome, ComponentInstallReport, DownloadFailureReason, InstallFailureReason,
    InstallStep, LocalAsrCheckOutcome, LocalAsrModelStatus, LocalAsrNotRunReason,
    LocalAsrSetupStatus, ManagedInstallReport, ManagedPlanAvailability, ManagedPlanObservation,
    ManagedSetupPlan, OnlineVideoCapability, OptionalComponent, OptionalComponentSet,
    ReviewedArchiveLimits, ReviewedArchiveSelection, ReviewedBanner, ReviewedCompatibilityPolicy,
    ReviewedManagedFile, RuntimeDiagnosis, SetupProfile, SetupSelectionState, StageDisposal,
    plan_managed_setup_including,
};
use vsift_contract::{
    CommandName, DependencyLookup, InstallSource, OPERATOR_DECISION_INSTRUCTION, OperationResponse,
    SavedSetupPlan, SetupCheckResponse, SetupInstallResponse, SetupPlanResponse,
    optional_install_failure_summary,
};
use vsift_domain::{
    ArtifactIntegrity, DependencyState, DependencyStatus, FailureCode, ManagedArtifactFormat,
    ManagedComponent, ManagedTarget, RuntimeDependency, RuntimeReadiness,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const DENO: ManagedComponent = ManagedComponent::JsRuntimeDeno;
const YT_DLP: ManagedComponent = ManagedComponent::YtDlp;
const INCLUDED: OptionalComponentSet = OptionalComponentSet::EMPTY.with(OptionalComponent::YtDlp);

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative_path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative_path),
    )?)?)
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema = load(schema_path)?;
    jsonschema::validator_for(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// Compares `value` with an example, or rewrites the example first when
/// regenerating.
fn matches_example(value: &Value, relative_path: &str) -> TestResult {
    if env::var("VSIFT_REGENERATE_CONTRACT_EXAMPLES").is_ok_and(|setting| setting == "1") {
        fs::write(
            schema_root().join(relative_path),
            format!("{}\n", serde_json::to_string_pretty(value)?),
        )?;
    }
    assert_eq!(value, &load(relative_path)?, "{relative_path}");
    Ok(())
}

fn integrity(bytes: u64, sha256: &str) -> Result<ArtifactIntegrity, Box<dyn std::error::Error>> {
    Ok(ArtifactIntegrity::from_sha256_hex(bytes, sha256)?)
}

fn file(
    name: &str,
    bytes: u64,
    sha256: &str,
) -> Result<ReviewedManagedFile, Box<dyn std::error::Error>> {
    Ok(ReviewedManagedFile {
        name: name.to_owned(),
        integrity: integrity(bytes, sha256)?,
        executable: true,
    })
}

/// A catalogue whose optional part is the reviewed pins (ADR 0028) and
/// whose core is three small fixtures.
fn catalogue() -> Result<AcceptedManagedCatalogue, Box<dyn std::error::Error>> {
    let core = [
        ManagedComponent::MediaTools,
        ManagedComponent::WhisperCli,
        ManagedComponent::WhisperModel,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, component)| {
        let digest = char::from(b'a' + u8::try_from(index).unwrap_or(0))
            .to_string()
            .repeat(64);
        Ok(AcceptedManagedArtifact {
            component,
            version: format!("fixture-{}", component.identifier().replace('_', "-")),
            publisher: String::from("fixture publisher"),
            source_url: format!("https://publisher.invalid/{}", component.identifier()),
            integrity: integrity(10, &digest)?,
            format: ManagedArtifactFormat::RawFile,
            archive_limits: None,
            selected_files: Vec::new(),
            archive_links: Vec::new(),
            runtime_copies: Vec::new(),
            licence: String::from("MIT"),
            notice_url: String::from("https://publisher.invalid/notice"),
            source_code_url: String::from("https://publisher.invalid/source"),
            trust_limit: String::from("fixture evidence only"),
            files: vec![file(&format!("file-{index}"), 10, &digest)?],
        })
    })
    .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    let deno_sha = "ce6a052beb97c2b92de67077e3f3924ba7c9661ede0d8dc2f66a116a3c841f21";
    let deno = AcceptedManagedArtifact {
        component: DENO,
        version: String::from("deno-2.9.7-x86_64-linux-gnu"),
        publisher: String::from("Deno Land deno"),
        source_url: String::from(
            "https://github.com/denoland/deno/releases/download/v2.9.7/deno-x86_64-unknown-linux-gnu.zip",
        ),
        integrity: integrity(
            41_596_794,
            "c6527f24f4b16031d3ae4fa9f658d5f11534c8d84ce7dc8502420280919c3490",
        )?,
        format: ManagedArtifactFormat::Zip,
        archive_limits: Some(ReviewedArchiveLimits {
            max_stream_bytes: 95_830_104,
            entries: 1,
            expanded_bytes: 95_830_104,
        }),
        selected_files: vec![ReviewedArchiveSelection {
            archive_path: String::from("deno"),
            runtime_name: String::from("deno"),
            integrity: integrity(95_830_104, deno_sha)?,
        }],
        archive_links: Vec::new(),
        runtime_copies: Vec::new(),
        licence: String::from("MIT"),
        notice_url: String::from("https://github.com/denoland/deno/blob/v2.9.7/LICENSE.md"),
        source_code_url: String::from("https://github.com/denoland/deno/tree/v2.9.7"),
        trust_limit: String::from("Publisher release zip with no publisher signature."),
        files: vec![file("deno", 95_830_104, deno_sha)?],
    };
    let yt_dlp_sha = "58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a";
    let yt_dlp = AcceptedManagedArtifact {
        component: YT_DLP,
        version: String::from("yt-dlp-2026.08.19-linux-x86_64"),
        publisher: String::from("yt-dlp"),
        source_url: String::from(
            "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_linux",
        ),
        integrity: integrity(40_446_224, yt_dlp_sha)?,
        format: ManagedArtifactFormat::RawFile,
        archive_limits: None,
        selected_files: Vec::new(),
        archive_links: Vec::new(),
        runtime_copies: Vec::new(),
        licence: String::from(
            "GPL-3.0-or-later (the PyInstaller executable is a combined work; yt-dlp's own source is under the Unlicense)",
        ),
        notice_url: String::from("https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/README.md"),
        source_code_url: String::from("https://github.com/yt-dlp/yt-dlp/tree/2026.08.19"),
        trust_limit: String::from("Publisher one-file build whose SHA-256 is in the signed sums."),
        files: vec![file("yt-dlp", 40_446_224, yt_dlp_sha)?],
    };
    Ok(AcceptedManagedCatalogue {
        revision: String::from("ubuntu-24.04-x86_64-2026-10-21-r4"),
        target: ManagedTarget::Ubuntu2404X86_64,
        stop_new_plans_at: 2_000_000_000,
        stop_new_plans_date: String::from("2033-05-18T03:33:20Z"),
        artifacts: core,
        optional_artifacts: vec![deno, yt_dlp],
        compatibility: ReviewedCompatibilityPolicy {
            fixture: integrity(10, &"f".repeat(64))?,
            expected_ffmpeg_version: String::from("ffmpeg version fixture"),
            expected_ffprobe_version: String::from("ffprobe version fixture"),
            stream_limit_bytes: 64 * 1024,
            audio_file_limit_bytes: 256 * 1024,
            transcript_file_limit_bytes: 64 * 1024,
            media_deadline_seconds: 60,
            inference_deadline_seconds: 180,
            audio_sample_rate_hz: 16_000,
            audio_channels: 1,
            optional_banners: vec![
                ReviewedBanner {
                    component: DENO,
                    first_line_prefix: String::from("deno 2.9.7 "),
                },
                ReviewedBanner {
                    component: YT_DLP,
                    first_line_prefix: String::from("2026.08.19"),
                },
            ],
        },
        superseded: Vec::new(),
    })
}

fn every_tool_available() -> RuntimeDiagnosis {
    RuntimeDiagnosis {
        readiness: RuntimeReadiness::Ready,
        dependencies: RuntimeDependency::ALL
            .into_iter()
            .map(|dependency| DependencyStatus {
                dependency,
                state: DependencyState::Available {
                    version: String::from("fixture 1"),
                },
            })
            .collect(),
    }
}

fn plan(
    profile: SetupProfile,
    include: OptionalComponentSet,
) -> Result<ManagedSetupPlan, Box<dyn std::error::Error>> {
    Ok(plan_managed_setup_including(
        profile,
        every_tool_available(),
        ManagedTarget::Ubuntu2404X86_64,
        SetupSelectionState {
            model: Some(String::from("configured model")),
            ..SetupSelectionState::default()
        },
        include,
        1_800_000_000,
        Some(catalogue()?),
    ))
}

fn present(plan: &ManagedSetupPlan) -> Result<Value, Box<dyn std::error::Error>> {
    let data = SetupPlanResponse::new(plan, &ManagedPlanObservation::without_managed_tier(plan));
    Ok(serde_json::to_value(OperationResponse::complete(
        CommandName::SetupPlan.identifier(),
        &data,
    )?)?)
}

/// #409 point 1 and 2: the default desktop plan installs nothing optional,
/// offers yt-dlp with its sentence, size, licences, the terms notice and the
/// exact command, and tells its reader to ask the operator.
#[test]
fn the_default_desktop_plan_offers_yt_dlp_and_asks_for_the_operator() -> TestResult {
    let plan = plan(SetupProfile::Desktop, OptionalComponentSet::EMPTY)?;
    let response = present(&plan)?;
    validate("setup-plan.schema.json", &response)?;
    let data = &response["data"];
    assert_eq!(data["managed_install"], "not_required");
    assert_eq!(data["actions"], Value::Array(Vec::new()));
    assert_eq!(data["install_needed"], false);
    assert_eq!(
        data["operator_decision_required"],
        OPERATOR_DECISION_INSTRUCTION
    );
    let offer = &data["optional_components"][0];
    assert_eq!(offer["component"], "yt_dlp");
    assert_eq!(offer["status"], "not_included");
    assert_eq!(
        offer["installs"],
        serde_json::json!(["js_runtime_deno", "yt_dlp"])
    );
    assert_eq!(offer["download_bytes"], 82_043_018);
    assert_eq!(
        offer["add_command"],
        serde_json::json!({
            "executable": "vsift",
            "arguments": ["setup", "plan", "--profile", "desktop", "--include", "yt_dlp", "--json"]
        })
    );
    let summary = offer["summary"].as_str().ok_or("no summary")?;
    for needed in ["about 83 MB", "GPL-3.0-or-later", "MIT", "YouTube", "terms"] {
        assert!(summary.contains(needed), "{needed}: {summary}");
    }
    assert_eq!(offer["actions"], Value::Array(Vec::new()));
    matches_example(&response, "examples/setup-plan.optional-offered.json")
}

/// `--include yt_dlp`: the runtime's and yt-dlp's exact actions are part of
/// the plan after the core's, the plan still asks for the operator (who must
/// have said yes), and the saved plan names what it includes.
#[test]
fn an_included_yt_dlp_carries_its_actions_and_the_saved_plan_names_it() -> TestResult {
    let included = plan(SetupProfile::Desktop, INCLUDED)?;
    let response = present(&included)?;
    validate("setup-plan.schema.json", &response)?;
    let data = &response["data"];
    assert_eq!(data["managed_install"], "catalogue_accepted");
    assert_eq!(data["install_needed"], true);
    let optional = &data["optional_components"][0];
    assert_eq!(optional["status"], "included");
    assert_eq!(optional["add_command"], Value::Null);
    let actions = optional["actions"].as_array().ok_or("no actions")?;
    let ids: Vec<&str> = actions
        .iter()
        .filter_map(|action| action["id"].as_str())
        .collect();
    assert_eq!(ids, ["install-js_runtime_deno", "install-yt_dlp"]);
    assert_eq!(actions[0]["format"], "zip");
    assert!(data["operator_decision_required"].is_string());

    let saved: SavedSetupPlan = serde_json::from_value(response.clone())?;
    saved
        .validate_envelope()
        .map_err(|code| code.identifier())?;
    assert_eq!(saved.included_optional(), Ok(INCLUDED));
    let presented = SetupPlanResponse::new(
        &included,
        &ManagedPlanObservation::without_managed_tier(&included),
    );
    assert_eq!(saved.require_same_plan(&presented), Ok(()));

    // Flipping the saved status is a different plan, whatever digest is
    // given with it.
    let mut flipped = response.clone();
    flipped["data"]["optional_components"][0]["status"] = Value::from("not_included");
    let flipped: SavedSetupPlan = serde_json::from_value(flipped)?;
    assert_eq!(flipped.included_optional(), Ok(OptionalComponentSet::EMPTY));
    assert_eq!(
        flipped.require_same_plan(&presented),
        Err(FailureCode::InvalidArgument)
    );
    let mut unknown = response;
    unknown["data"]["optional_components"][0]["component"] = Value::from("aria2c");
    let unknown: SavedSetupPlan = serde_json::from_value(unknown)?;
    assert_eq!(
        unknown.included_optional(),
        Err(FailureCode::InvalidArgument)
    );
    Ok(())
}

/// A saved plan from before `0.3.0` has neither member; it still decodes,
/// and includes nothing.
#[test]
fn a_saved_plan_without_the_optional_members_still_decodes() -> TestResult {
    let mut earlier = present(&plan(SetupProfile::Desktop, OptionalComponentSet::EMPTY)?)?;
    let data = earlier["data"].as_object_mut().ok_or("no data")?;
    data.remove("optional_components");
    data.remove("operator_decision_required");
    let saved: SavedSetupPlan = serde_json::from_value(earlier)?;
    assert_eq!(saved.included_optional(), Ok(OptionalComponentSet::EMPTY));
    Ok(())
}

/// The worker plan neither lists nor asks unless the component is included.
#[test]
fn the_worker_plan_lists_only_what_it_includes() -> TestResult {
    let worker = present(&plan(SetupProfile::Worker, OptionalComponentSet::EMPTY)?)?;
    validate("setup-plan.schema.json", &worker)?;
    assert_eq!(
        worker["data"]["optional_components"],
        Value::Array(Vec::new())
    );
    assert_eq!(worker["data"]["operator_decision_required"], Value::Null);
    let included = present(&plan(SetupProfile::Worker, INCLUDED)?)?;
    validate("setup-plan.schema.json", &included)?;
    assert_eq!(
        included["data"]["optional_components"][0]["status"],
        "included"
    );
    Ok(())
}

fn report(
    component: ManagedComponent,
    version: &str,
    outcome: ComponentInstallOutcome,
) -> ComponentInstallReport {
    ComponentInstallReport {
        component,
        version: version.to_owned(),
        outcome,
        stage: match outcome {
            ComponentInstallOutcome::Activated => Some(StageDisposal::Discarded),
            ComponentInstallOutcome::AlreadyCurrent | ComponentInstallOutcome::Failed(_) => None,
        },
    }
}

fn core_current() -> Vec<ComponentInstallReport> {
    [
        (ManagedComponent::MediaTools, "fixture-ffmpeg-ffprobe"),
        (ManagedComponent::WhisperCli, "fixture-whisper-cli"),
        (ManagedComponent::WhisperModel, "fixture-whisper-model"),
    ]
    .into_iter()
    .map(|(component, version)| report(component, version, ComponentInstallOutcome::AlreadyCurrent))
    .collect()
}

fn blocked() -> ComponentInstallOutcome {
    ComponentInstallOutcome::Failed(ComponentInstallFailure {
        step: None,
        reason: InstallFailureReason::Blocked,
    })
}

/// #409: a proxy that asks for credentials stops the runtime's download.
/// The core is ready, the result is a success with a warning, and
/// `online_video` names the step's typed reason and says local
/// investigation is not affected.
#[test]
fn an_optional_failure_is_a_success_that_says_the_core_is_ready() -> TestResult {
    let failure = ComponentInstallFailure::at(
        InstallStep::Download,
        InstallFailureReason::Download(DownloadFailureReason::ProxyAuth),
    );
    let data = SetupInstallResponse::new(
        Some(String::from("ubuntu-24.04-x86_64-2026-10-21-r4")),
        InstallSource::Publisher,
        &ManagedInstallReport {
            components: core_current(),
            optional: vec![
                report(
                    DENO,
                    "deno-2.9.7-x86_64-linux-gnu",
                    ComponentInstallOutcome::Failed(failure),
                ),
                report(YT_DLP, "yt-dlp-2026.08.19-linux-x86_64", blocked()),
            ],
        },
    );
    let warning = data.optional_warning().ok_or("no warning")?;
    let response = serde_json::to_value(
        OperationResponse::complete(CommandName::SetupInstall.identifier(), &data)?
            .with_warnings(&[warning]),
    )?;
    validate("operation-response.schema.json", &response)?;
    validate("setup-install.schema.json", &response["data"])?;
    assert_eq!(response["status"], "complete");
    assert_eq!(response["error"], Value::Null);
    let data = &response["data"];
    assert_eq!(data["core"], "ready");
    assert_eq!(data["online_video"]["status"], "not_installed");
    assert_eq!(data["online_video"]["component"], "js_runtime_deno");
    assert_eq!(data["online_video"]["reason"], "proxy_auth");
    assert_eq!(data["online_video"]["failure_code"], "DOWNLOAD_FAILED");
    let advice = data["online_video"]["next_step"]
        .as_str()
        .ok_or("no advice")?;
    assert!(advice.contains("ready for local videos") && advice.contains("--artifact-dir"));
    assert!(
        data["next_step"]
            .as_str()
            .is_some_and(|step| step.starts_with("VSift is ready for local videos"))
    );
    matches_example(
        &response,
        "examples/setup-install.optional-not-installed.json",
    )
}

/// Installed, the optional part says so; a core failure is a failure as
/// before, with the optional part blocked and `core` failed.
#[test]
fn the_optional_part_reports_installed_or_blocked() -> TestResult {
    let installed = SetupInstallResponse::new(
        None,
        InstallSource::Publisher,
        &ManagedInstallReport {
            components: core_current(),
            optional: vec![
                report(
                    DENO,
                    "deno-2.9.7-x86_64-linux-gnu",
                    ComponentInstallOutcome::Activated,
                ),
                report(
                    YT_DLP,
                    "yt-dlp-2026.08.19-linux-x86_64",
                    ComponentInstallOutcome::Activated,
                ),
            ],
        },
    );
    assert_eq!(installed.optional_warning(), None);
    let value = serde_json::to_value(&installed)?;
    validate("setup-install.schema.json", &value)?;
    assert_eq!(value["online_video"]["status"], "installed");
    assert_eq!(value["core"], "ready");

    let mut core = core_current();
    core[0].outcome = ComponentInstallOutcome::Failed(ComponentInstallFailure::at(
        InstallStep::Download,
        InstallFailureReason::Download(DownloadFailureReason::Offline),
    ));
    let failed = SetupInstallResponse::new(
        None,
        InstallSource::Publisher,
        &ManagedInstallReport {
            components: core,
            optional: vec![
                report(DENO, "deno-2.9.7-x86_64-linux-gnu", blocked()),
                report(YT_DLP, "yt-dlp-2026.08.19-linux-x86_64", blocked()),
            ],
        },
    );
    assert_eq!(failed.optional_warning(), None);
    let value = serde_json::to_value(&failed)?;
    validate("setup-install.schema.json", &value)?;
    assert_eq!(value["core"], "failed");
    assert_eq!(value["online_video"]["reason"], "blocked");

    // A plan without optional components keeps its earlier shape exactly.
    let core_only = serde_json::to_value(SetupInstallResponse::new(
        None,
        InstallSource::Publisher,
        &ManagedInstallReport {
            components: core_current(),
            optional: Vec::new(),
        },
    ))?;
    for member in ["core", "optional_components", "online_video"] {
        assert!(core_only.get(member).is_none(), "{member}");
    }
    Ok(())
}

/// Every optional failure's advice is fixed prose that names the component
/// and the reason, says the core is ready, and stays within the schema.
#[test]
fn every_optional_failure_has_bounded_fixed_advice() {
    let mut reasons: Vec<InstallFailureReason> = DownloadFailureReason::ALL
        .into_iter()
        .map(InstallFailureReason::Download)
        .collect();
    reasons.extend([
        InstallFailureReason::ArtifactMissing,
        InstallFailureReason::ArtifactNotRegularFile,
        InstallFailureReason::DigestMismatch,
        InstallFailureReason::SizeMismatch,
        InstallFailureReason::ReviewMismatch,
        InstallFailureReason::Storage,
        InstallFailureReason::Cancelled,
    ]);
    for reason in reasons {
        for component in [DENO, YT_DLP] {
            let advice = optional_install_failure_summary(
                component,
                ComponentInstallFailure::at(InstallStep::Download, reason),
            );
            assert!(advice.len() <= 640, "{advice}");
            assert!(advice.contains(component.identifier()), "{advice}");
            assert!(advice.contains(reason.identifier()), "{advice}");
            assert!(advice.contains("ready for local videos"), "{advice}");
            assert_eq!(
                advice.contains("--artifact-dir"),
                matches!(reason, InstallFailureReason::Download(_)),
                "{advice}"
            );
        }
    }
}

const NOTHING_REGISTERED: LocalAsrSetupStatus = LocalAsrSetupStatus {
    model: LocalAsrModelStatus::NotSelected,
    verification: LocalAsrCheckOutcome::NotRun(LocalAsrNotRunReason::ModelNotSelected),
};

/// `setup check` reports online videos apart from local readiness: not
/// installed with the add command, installed with the versions, or
/// unavailable off the managed target; the worker profile suggests nothing.
#[test]
fn setup_check_reports_online_videos_apart_from_local_readiness() -> TestResult {
    let check = |capability: &OnlineVideoCapability,
                 profile: SetupProfile,
                 availability: ManagedPlanAvailability|
     -> Result<Value, Box<dyn std::error::Error>> {
        let value = serde_json::to_value(
            SetupCheckResponse::new(
                &every_tool_available(),
                profile,
                |_| DependencyLookup::ManagedVersion,
                availability,
                &NOTHING_REGISTERED,
            )
            .with_online_video(capability, profile),
        )?;
        validate("setup-check-response.schema.json", &value)?;
        Ok(value)
    };
    let installed = check(
        &OnlineVideoCapability::Installed(vec![
            (DENO, String::from("deno-2.9.7-x86_64-linux-gnu")),
            (YT_DLP, String::from("yt-dlp-2026.08.19-linux-x86_64")),
        ]),
        SetupProfile::Desktop,
        ManagedPlanAvailability::Qualified,
    )?;
    assert_eq!(installed["status"], "ready");
    assert_eq!(installed["online_video"]["status"], "installed");
    assert_eq!(installed["online_video"]["add_command"], Value::Null);
    matches_example(&installed, "examples/setup-check.online-video.json")?;

    let missing = check(
        &OnlineVideoCapability::NotInstalled,
        SetupProfile::Desktop,
        ManagedPlanAvailability::Qualified,
    )?;
    assert_eq!(missing["status"], "ready");
    assert_eq!(missing["online_video"]["status"], "not_installed");
    assert_eq!(
        missing["online_video"]["add_command"]["arguments"][5],
        "yt_dlp"
    );
    let worker = check(
        &OnlineVideoCapability::NotInstalled,
        SetupProfile::Worker,
        ManagedPlanAvailability::Qualified,
    )?;
    assert_eq!(worker["online_video"]["add_command"], Value::Null);
    let elsewhere = check(
        &OnlineVideoCapability::Unavailable,
        SetupProfile::Desktop,
        ManagedPlanAvailability::TargetUnavailable,
    )?;
    assert_eq!(elsewhere["online_video"]["status"], "unavailable");
    Ok(())
}
