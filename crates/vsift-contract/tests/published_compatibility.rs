//! Compatibility of the current v1 contract with the published 0.1.0
//! (P14 PR 2, evidence item RQ-04; ADR 0024).
//!
//! `schemas/v1/frozen/v0.1.0/examples/` holds every file of
//! `schemas/v1/examples/` exactly as the tag `v0.1.0` published it. v1 is
//! additive since that release, so each frozen document must still validate
//! against the **current** v1 schemas: a schema that became stricter, a field
//! that was removed or renamed, an enum value that disappeared or a new
//! required member all fail here, whatever a pull request changed alongside.
//! Nothing else compares a build with the published release: the live
//! `examples/` are free to grow with the contract, which is why they cannot
//! stand in for the published ones.
//!
//! The frozen copy is checked into the tree so this test runs in every
//! pull-request check, on every operating system, with no history and no
//! network. A second test proves the copy is the tag's: it asks Git for every
//! file of the tag and compares bytes. A checkout without the tag (CI's
//! default shallow clone) reports that it could not check and passes, unless
//! `VSIFT_REQUIRE_RELEASE_TAG` names the tag, which the `P14 published
//! artifacts` workflow sets after fetching the history: there a missing tag is
//! a failure.
//!
//! What this does not show: that a *session* made by 0.1.0 opens in a newer
//! build. The stored records of a session are decoded by the current readers in
//! `vsift-infrastructure`'s `published_v0_1_0_records` test; the whole session
//! across an upgrade is the upgrade jobs' check on real binaries.

use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The release the frozen examples come from.
const RELEASE_TAG: &str = "v0.1.0";
/// The identifier base of the published schemas.
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
/// The frozen examples, relative to `schemas/v1`.
const FROZEN: &str = "frozen/v0.1.0/examples";
/// Where the release's examples are in the tag, for Git.
const TAG_EXAMPLES: &str = "schemas/v1/examples";

/// How one frozen example is held to the current schemas.
enum Check {
    /// The document is an instance of this schema.
    Whole(&'static str),
    /// An operation response; when given, its `data` is also an instance of
    /// this data schema.
    Operation(Option<&'static str>),
    /// A JSON Lines stream of `--events jsonl` events.
    Events,
    /// A JSON Lines file of worker requests.
    Requests,
}

use Check::{Events, Operation, Requests, Whole};

/// Every file of the 0.1.0 examples and what it must satisfy. A file the
/// release published that is missing here, or listed here and not published,
/// fails `the_table_lists_exactly_the_frozen_files`.
const EXAMPLES: [(&str, Check); 52] = [
    ("audio.json", Operation(Some("audio-data.schema.json"))),
    (
        "bundle-evidence-record.json",
        Whole("bundle-evidence-record.schema.json"),
    ),
    (
        "bundle-transcript-record.asr.json",
        Whole("bundle-transcript-record.schema.json"),
    ),
    (
        "bundle-transcript-record.json",
        Whole("bundle-transcript-record.schema.json"),
    ),
    (
        "bundle-visual-index-record.json",
        Whole("bundle-visual-index-record.schema.json"),
    ),
    ("candidates.events.jsonl", Events),
    (
        "candidates.json",
        Operation(Some("candidates-data.schema.json")),
    ),
    (
        "candidates.partial.json",
        Operation(Some("candidates-data.schema.json")),
    ),
    ("config.json", Whole("config.schema.json")),
    ("crop.json", Operation(Some("frame-data.schema.json"))),
    (
        "frame-burst.partial.json",
        Operation(Some("frame-data.schema.json")),
    ),
    ("frame-get.events.jsonl", Events),
    ("frame-get.json", Operation(Some("frame-data.schema.json"))),
    (
        "frame-neighbours.json",
        Operation(Some("frame-data.schema.json")),
    ),
    (
        "handoff-check.json",
        Operation(Some("handoff-check-data.schema.json")),
    ),
    (
        "ingest.transcript.json",
        Operation(Some("ingest-data.schema.json")),
    ),
    ("job-batch.events.jsonl", Events),
    (
        "job-batch.json",
        Operation(Some("job-batch-data.schema.json")),
    ),
    ("job-batch.requests.jsonl", Requests),
    ("job-cancel.json", Operation(Some("job-data.schema.json"))),
    ("job-request.json", Whole("job-request.schema.json")),
    (
        "job-resume.json",
        Operation(Some("job-resume-data.schema.json")),
    ),
    ("job-run.json", Operation(Some("job-result.schema.json"))),
    (
        "job-run.partial.json",
        Operation(Some("job-result.schema.json")),
    ),
    (
        "job-run.replayed.json",
        Operation(Some("job-result.schema.json")),
    ),
    ("job-status.json", Operation(Some("job-data.schema.json"))),
    ("media-tool-verification-failed.json", Operation(None)),
    ("operation-error.json", Operation(None)),
    ("parse-failure.json", Operation(None)),
    ("retranscribe-cancelled.json", Operation(None)),
    ("search.events.jsonl", Events),
    ("search.json", Operation(Some("search-data.schema.json"))),
    (
        "setup-check.blocked.json",
        Whole("setup-check-response.schema.json"),
    ),
    (
        "setup-check.local-asr.json",
        Whole("setup-check-response.schema.json"),
    ),
    (
        "setup-install.failed.json",
        Operation(Some("setup-install.schema.json")),
    ),
    (
        "setup-install.json",
        Operation(Some("setup-install.schema.json")),
    ),
    ("setup-list.json", Operation(Some("setup-list.schema.json"))),
    (
        "setup-plan.unavailable.json",
        Whole("setup-plan.schema.json"),
    ),
    (
        "setup-plan.unqualified.json",
        Whole("setup-plan-unqualified.schema.json"),
    ),
    (
        "setup-remove.failed.json",
        Operation(Some("setup-remove.schema.json")),
    ),
    (
        "setup-remove.json",
        Operation(Some("setup-remove.schema.json")),
    ),
    (
        "setup-repair.json",
        Operation(Some("setup-repair.schema.json")),
    ),
    (
        "setup-rollback.json",
        Operation(Some("setup-rollback.schema.json")),
    ),
    ("storage-not-private.json", Operation(None)),
    ("terminal-event.json", Whole("terminal-event.schema.json")),
    (
        "transcript-get.asr.json",
        Operation(Some("transcript-get-data.schema.json")),
    ),
    ("transcript-get.events.jsonl", Events),
    (
        "transcript-get.json",
        Operation(Some("transcript-get-data.schema.json")),
    ),
    ("transcript-rejected.json", Operation(None)),
    ("transcript-retranscribe.events.jsonl", Events),
    (
        "transcript-retranscribe.json",
        Operation(Some("transcript-retranscribe-data.schema.json")),
    ),
    (
        "workspace-init.json",
        Operation(Some("workspace-data.schema.json")),
    ),
];

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn frozen_root() -> PathBuf {
    schema_root().join(FROZEN)
}

/// Resolves a schema's sibling `$ref`s by their published identifier to the
/// **current** local copy: the point is to hold old documents to today's
/// schemas.
struct CurrentSchemas;

impl Retrieve for CurrentSchemas {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_str(&fs::read_to_string(
            schema_root().join(name),
        )?)?)
    }
}

fn validate(schema_name: &str, instance: &Value, what: &str) -> TestResult {
    let schema: Value =
        serde_json::from_str(&fs::read_to_string(schema_root().join(schema_name))?)?;
    jsonschema::options()
        .with_retriever(CurrentSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{what} against {schema_name}: {error}")))?;
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

/// The current schema an `--events jsonl` event of this kind is an instance of.
fn event_schema(kind: &str) -> Result<&'static str, io::Error> {
    match kind {
        "evidence" => Ok("evidence-event.schema.json"),
        "terminal" => Ok("terminal-event.schema.json"),
        "progress" => Ok("progress-event.schema.json"),
        "lifecycle" => Ok("lifecycle-event.schema.json"),
        "result" => Ok("result-event.schema.json"),
        other => Err(io::Error::other(format!("unknown event kind {other:?}"))),
    }
}

fn check_example(name: &str, check: &Check) -> TestResult {
    let path = frozen_root().join(name);
    match check {
        Whole(schema) => {
            let document = read_json(&path)?;
            validate(schema, &document, name)?;
            if *schema == "terminal-event.schema.json" {
                validate("operation-response.schema.json", &document["result"], name)?;
            }
        }
        Operation(data_schema) => {
            let document = read_json(&path)?;
            validate("operation-response.schema.json", &document, name)?;
            if let Some(data_schema) = data_schema {
                validate(data_schema, &document["data"], name)?;
            }
        }
        Events | Requests => {
            let text = fs::read_to_string(&path)?;
            let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
            if lines.is_empty() {
                return Err(io::Error::other(format!("{name} holds no line")).into());
            }
            for (index, line) in lines.iter().enumerate() {
                let event: Value = serde_json::from_str(line)?;
                let what = format!("{name} line {}", index + 1);
                if matches!(check, Requests) {
                    validate("job-request.schema.json", &event, &what)?;
                    continue;
                }
                let kind = event["event"]
                    .as_str()
                    .ok_or_else(|| io::Error::other(format!("{what} has no event kind")))?;
                validate(event_schema(kind)?, &event, &what)?;
                if kind == "terminal" {
                    validate("operation-response.schema.json", &event["result"], &what)?;
                }
            }
        }
    }
    Ok(())
}

fn listed_files(directory: &Path) -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(
                io::Error::other(format!("{} is not a file", entry.path().display())).into(),
            );
        }
        names.insert(entry.file_name().to_string_lossy().into_owned());
    }
    Ok(names)
}

#[test]
fn the_table_lists_exactly_the_frozen_files() -> TestResult {
    let table: BTreeSet<String> = EXAMPLES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    assert_eq!(table.len(), EXAMPLES.len(), "a file is listed twice");
    assert_eq!(table, listed_files(&frozen_root())?);
    Ok(())
}

#[test]
fn every_published_0_1_0_example_validates_against_the_current_v1_schemas() -> TestResult {
    for (name, check) in &EXAMPLES {
        check_example(name, check)?;
    }
    Ok(())
}

#[test]
fn every_error_code_of_the_published_examples_is_still_published() -> TestResult {
    let schema = read_json(&schema_root().join("operation-response.schema.json"))?;
    let current: BTreeSet<&str> = schema
        .pointer("/$defs/error/properties/code/enum")
        .and_then(Value::as_array)
        .ok_or("operation-response.schema.json has no error code enum")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for (name, check) in &EXAMPLES {
        if !matches!(check, Operation(_)) {
            continue;
        }
        let document = read_json(&frozen_root().join(name))?;
        if let Some(code) = document["error"]["code"].as_str() {
            assert!(
                current.contains(code),
                "{name} reports {code}, which the current envelope no longer lists"
            );
        }
    }
    Ok(())
}

fn git(arguments: &[&str]) -> io::Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("--literal-pathspecs")
        .arg("-C")
        .arg(repository_root())
        .args(arguments)
        .output()?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

/// The files of `schemas/v1/examples` in the release tag, as Git lists them, or
/// why the tag cannot be read here.
fn release_tag_files() -> Result<Vec<String>, io::Error> {
    let listing = git(&[
        "ls-tree",
        "-r",
        "--name-only",
        "-z",
        RELEASE_TAG,
        "--",
        TAG_EXAMPLES,
    ])?;
    Ok(String::from_utf8(listing)
        .map_err(|_| io::Error::other("git printed a name that is not UTF-8"))?
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect())
}

#[test]
fn the_frozen_examples_are_the_bytes_the_release_tag_holds() -> TestResult {
    let required =
        std::env::var("VSIFT_REQUIRE_RELEASE_TAG").is_ok_and(|value| value == RELEASE_TAG);
    let files = match release_tag_files() {
        Ok(files) => files,
        Err(error) if !required => {
            eprintln!(
                "the tag {RELEASE_TAG} cannot be read here ({error}): the frozen copy was not \
                 compared with it; set VSIFT_REQUIRE_RELEASE_TAG={RELEASE_TAG} in a full checkout \
                 to require the comparison"
            );
            return Ok(());
        }
        Err(error) => {
            return Err(io::Error::other(format!(
                "VSIFT_REQUIRE_RELEASE_TAG names {RELEASE_TAG}, but Git cannot read it: {error}"
            ))
            .into());
        }
    };
    let in_tag: BTreeSet<String> = files
        .iter()
        .filter_map(|file| file.rsplit('/').next())
        .map(str::to_owned)
        .collect();
    assert_eq!(
        in_tag,
        listed_files(&frozen_root())?,
        "the frozen folder and the tag hold different files"
    );
    for file in &files {
        let name = file.rsplit('/').next().unwrap_or(file);
        let published = git(&["show", &format!("{RELEASE_TAG}:{file}")])?;
        let frozen = fs::read(frozen_root().join(name))?;
        assert!(
            published == frozen,
            "{name} is not byte-identical to the file in {RELEASE_TAG}"
        );
    }
    Ok(())
}
