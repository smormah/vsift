//! Public CLI contract of `vsift handoff finish` (0.3.0, M5b; ADR 0023 note
//! of 2026-11-09; the planner's ruling of 2026-11-09).
//!
//! The draft arrives as for `handoff check`. The command checks it with the
//! session's records always resolved, then, only when the draft passes and
//! states the requested write's lifecycle action, makes that one write
//! through the same engine call as `session retain` or `session close`:
//! - without a write it behaves as `handoff check --session` (a missing,
//!   closed or expired session is a gap, not a failure);
//! - a stop at the draft or the lifecycle is `complete`, exit 0, `finished`
//!   false, and writes nothing (no bundle directory exists afterwards);
//! - a write that fails, a session that is missing, closed or expired
//!   included, answers exactly as the standalone write does.

use std::{
    env,
    error::Error,
    fs, io,
    num::NonZeroU32,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::{Value, json};
use vsift::{
    DurabilityRequirement, MediaTime, OperationId, SessionId, StorageGeneration, TranscriptOffset,
};
use vsift_application::{
    ForegroundSessionPort, ImportedRevisionRequest, InitializeSessionStorage,
    InitializeSessionStorageRequest, build_imported_revision,
};
use vsift_infrastructure::{FilesystemSessionStore, SourceSnapshot, read_supplied_transcript};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-handoff-finish-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const SECOND: u64 = 1_000_000;
/// Three days: a session seeded this long ago is past its 24 idle hours.
const EXPIRED_AGO: u64 = 3 * 24 * 60 * 60;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{OWNED_PREFIX}{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self, child: &str) -> PathBuf {
        self.0.join(child)
    }

    fn sessions(&self) -> PathBuf {
        self.path("private sessions")
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Resolves sibling `$ref`s by their published identifier to the local copy.
struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
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
            repository("schemas/v1").join(name),
        )?)?)
    }
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema: Value = serde_json::from_str(&fs::read_to_string(
        repository("schemas/v1").join(schema_path),
    )?)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// Runs `vsift` with an isolated per-user base, the root's session store
/// and `input` on standard input.
fn vsift(root: &OwnedRoot, arguments: &[&str], input: &[u8]) -> Built<Output> {
    let base = root.path("user");
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .write_stdin(input.to_vec())
        .output()?)
}

/// Any `--json` result, validated against the envelope schema.
fn envelope(output: &Output) -> Built<Value> {
    assert!(output.stderr.is_empty(), "a JSON command wrote to stderr");
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    Ok(value)
}

/// A `handoff finish --json` result; its data is validated when it
/// completed.
fn finish_json(output: &Output) -> Built<Value> {
    let value = envelope(output)?;
    assert_eq!(value["command"], "handoff.finish");
    if value["error"].is_null() {
        assert_eq!(exit(output), Some(0));
        validate("handoff-finish-data.schema.json", &value["data"])?;
    }
    Ok(value)
}

fn exit(output: &Output) -> Option<i32> {
    output.status.code()
}

fn handoff(segment: &str, action: &str) -> Value {
    json!({
        "handoff_version": "1", "status": "complete",
        "question": "What does the speaker name?",
        "capabilities": {"image_access": "unavailable"},
        "claims": [{"id": "c1", "section": "actual", "kind": "observed", "support": "supported",
            "certainty": "medium", "statement": "The speaker names dialog R-17.", "citations": ["e1"]}],
        "citations": [{"id": "e1", "type": "transcript_segment", "segment_id": segment}],
        "gaps": [{"kind": "image_access", "reason": "image_access_unavailable", "note": null}],
        "untrusted_instructions": [],
        "lifecycle": {"action": action}
    })
}

fn report(handoff: &Value) -> String {
    format!(
        "## Problem\n\nThe speaker names the dialog [c1: e1].\n\n```vsift-handoff\n{handoff}\n```\n"
    )
}

/// One session whose transcript is the F10 import, committed through the
/// store `ago` seconds before now (no `FFprobe` needed), and the id of its
/// first segment.
async fn seed_session(root: &OwnedRoot, ago: u64) -> Built<String> {
    let source = root.path("stand-in.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomhandoff-finish-contract")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().saturating_sub(ago);
    let session_id = SessionId::parse(SESSION)?;
    let opener = OperationId::parse("op_0123456789abcdef")?;
    let store = FilesystemSessionStore::provision_default(root.sessions())?;
    let registration = store.register_session(&session_id, &opener, now)?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            opener,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    drop(registration);
    let store = FilesystemSessionStore::open_existing(root.sessions())?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        &source,
    )?;
    let supplied = read_supplied_transcript(&repository("fixtures/corpus/transcripts/F10.srt"))?;
    let revision = build_imported_revision(ImportedRevisionRequest {
        session_id: &session_id,
        source_id: snapshot.id(),
        source_duration: MediaTime::from_micros(12 * SECOND),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?;
    let segment = revision
        .segments()
        .first()
        .ok_or("no segment")?
        .id()
        .as_str()
        .to_owned();
    store.activate_with_transcript(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        now,
        &vsift_domain::TimeAnchors::none(),
        &revision,
    )?;
    Ok(segment)
}

fn path_text(path: &Path) -> Built<&str> {
    path.to_str().ok_or_else(|| "a non-UTF-8 temporary path".into())
}

/// The error code and exit of a failed result.
fn failure(output: &Output) -> Built<(Value, Option<i32>)> {
    let value = envelope(output)?;
    assert_eq!(value["status"], "failed", "{value}");
    Ok((value["error"]["code"].clone(), exit(output)))
}

/// Without a write, the command is `handoff check --session`: the same
/// check data for an open, missing, closed or expired session, and never a
/// failure.
#[tokio::test]
async fn without_a_write_it_answers_as_handoff_check_with_the_session() -> TestResult {
    let missing = OwnedRoot::new()?;
    let draft = report(&handoff("tsg_0123456789abcdef0123456789abcdef", "left_open"));
    let open = OwnedRoot::new()?;
    let segment = seed_session(&open, 0).await?;
    let open_draft = report(&handoff(&segment, "left_open"));
    let expired = OwnedRoot::new()?;
    let expired_segment = seed_session(&expired, EXPIRED_AGO).await?;
    let expired_draft = report(&handoff(&expired_segment, "expired"));
    let closed = OwnedRoot::new()?;
    let closed_segment = seed_session(&closed, 0).await?;
    let closed_draft = report(&handoff(&closed_segment, "closed"));
    envelope(&vsift(
        &closed,
        &["session", "close", SESSION, "--json"],
        b"",
    )?)?;

    for (root, draft, gap) in [
        (&missing, &draft, json!("session_not_found")),
        (&open, &open_draft, Value::Null),
        (&expired, &expired_draft, json!("session_expired")),
        (&closed, &closed_draft, json!("session_closed")),
    ] {
        let finish = finish_json(&vsift(
            root,
            &["handoff", "finish", SESSION, "--json"],
            draft.as_bytes(),
        )?)?;
        let check = envelope(&vsift(
            root,
            &["handoff", "check", "--session", SESSION, "--json"],
            draft.as_bytes(),
        )?)?;
        assert_eq!(finish["data"]["check"], check["data"], "{gap}");
        assert_eq!(finish["data"]["check"]["session"]["gap"], gap);
        assert_eq!(finish["data"]["finished"], true, "{finish}");
        assert_eq!(finish["data"]["requested"], "check");
        assert!(finish["lifecycle"].is_null(), "nothing was written");
    }
    Ok(())
}

/// A retention after a passing draft writes the validated bundle once; the
/// result says it was validated, and `bundle validate` agrees.
#[tokio::test]
async fn a_passing_draft_is_retained_and_validated_in_one_call() -> TestResult {
    let root = OwnedRoot::new()?;
    let segment = seed_session(&root, 0).await?;
    let bundle = root.path("evidence bundle");
    let value = finish_json(&vsift(
        &root,
        &[
            "handoff",
            "finish",
            SESSION,
            "--retain",
            path_text(&bundle)?,
            "--json",
        ],
        report(&handoff(&segment, "retained")).as_bytes(),
    )?)?;
    assert_eq!(value["data"]["finished"], true, "{value}");
    assert_eq!(value["data"]["requested"], "retain");
    assert_eq!(value["data"]["bundle_validated"], true);
    assert_eq!(value["data"]["retain"]["session_id"], SESSION);
    assert_eq!(value["data"]["retain"]["source_included"], false);
    assert_eq!(value["data"]["check"]["session"]["identities_checked"], 1);
    assert_eq!(value["lifecycle"]["mode"], "retained");
    let validated = envelope(&vsift(
        &root,
        &["bundle", "validate", path_text(&bundle)?, "--json"],
        b"",
    )?)?;
    assert_eq!(validated["status"], "complete");
    assert_eq!(validated["data"], value["data"]["retain"]);
    Ok(())
}

/// A draft that does not pass, or that does not state `retained`, writes no
/// bundle: `complete`, exit 0, `finished` false, and the human first line
/// says so.
#[tokio::test]
async fn a_stop_writes_nothing_and_says_so() -> TestResult {
    let root = OwnedRoot::new()?;
    let segment = seed_session(&root, 0).await?;
    let bundle = root.path("evidence bundle");
    let unknown = report(&handoff("tsg_ffffffffffffffffffffffffffffffff", "retained"));
    let left_open = report(&handoff(&segment, "left_open"));
    for (draft, stop) in [(&unknown, "draft"), (&left_open, "lifecycle")] {
        let value = finish_json(&vsift(
            &root,
            &[
                "handoff",
                "finish",
                SESSION,
                "--retain",
                path_text(&bundle)?,
                "--json",
            ],
            draft.as_bytes(),
        )?)?;
        assert_eq!(value["status"], "complete");
        assert_eq!(value["data"]["finished"], false);
        assert_eq!(value["data"]["stopped_at"], stop);
        assert!(value["data"]["retain"].is_null());
        assert!(!bundle.exists(), "a {stop} stop wrote the bundle directory");

        let human = vsift(
            &root,
            &["handoff", "finish", SESSION, "--retain", path_text(&bundle)?],
            draft.as_bytes(),
        )?;
        assert_eq!(exit(&human), Some(0));
        let text = String::from_utf8(human.stdout)?;
        assert!(text.starts_with("Not retained: "), "{text}");
        assert!(!bundle.exists());
    }
    assert_eq!(
        envelope(&vsift(
            &root,
            &["handoff", "finish", SESSION, "--close", "--json"],
            left_open.as_bytes(),
        )?)?["data"]["stopped_at"],
        "lifecycle"
    );
    let status = envelope(&vsift(
        &root,
        &["session", "status", SESSION, "--json"],
        b"",
    )?)?;
    assert_eq!(status["data"]["state"], "open", "a stop closed the session");
    Ok(())
}

/// After a passing draft, a retention to a folder that exists answers as
/// `session retain` does, and leaves the folder as it was.
#[tokio::test]
async fn a_failed_retention_answers_as_session_retain() -> TestResult {
    let root = OwnedRoot::new()?;
    let segment = seed_session(&root, 0).await?;
    let existing = root.path("already here");
    fs::create_dir(&existing)?;
    let finish = vsift(
        &root,
        &[
            "handoff",
            "finish",
            SESSION,
            "--retain",
            path_text(&existing)?,
            "--json",
        ],
        report(&handoff(&segment, "retained")).as_bytes(),
    )?;
    let standalone = vsift(
        &root,
        &[
            "session",
            "retain",
            SESSION,
            "--output",
            path_text(&existing)?,
            "--json",
        ],
        b"",
    )?;
    assert_eq!(failure(&finish)?, failure(&standalone)?);
    assert_eq!(failure(&finish)?.0, "INVALID_ARGUMENT");
    assert_eq!(fs::read_dir(&existing)?.count(), 0);
    Ok(())
}

/// With a write requested, a missing, closed or expired session answers as
/// the standalone write would, with its code and exit (the planner's ruling
/// of 2026-11-09, change 1).
#[tokio::test]
async fn a_write_on_an_unavailable_session_answers_as_the_standalone_write() -> TestResult {
    let missing = OwnedRoot::new()?;
    let expired = OwnedRoot::new()?;
    let expired_segment = seed_session(&expired, EXPIRED_AGO).await?;
    let closed = OwnedRoot::new()?;
    let closed_segment = seed_session(&closed, 0).await?;
    envelope(&vsift(
        &closed,
        &["session", "close", SESSION, "--json"],
        b"",
    )?)?;
    for (root, segment) in [
        (&missing, "tsg_0123456789abcdef0123456789abcdef"),
        (&expired, expired_segment.as_str()),
        (&closed, closed_segment.as_str()),
    ] {
        let bundle = root.path("evidence bundle");
        let finish = vsift(
            root,
            &[
                "handoff",
                "finish",
                SESSION,
                "--retain",
                path_text(&bundle)?,
                "--json",
            ],
            report(&handoff(segment, "retained")).as_bytes(),
        )?;
        let finish_bundle_written = bundle.exists();
        let finish_answer = if exit(&finish) == Some(0) {
            None
        } else {
            Some(failure(&finish)?)
        };
        if finish_bundle_written {
            fs::remove_dir_all(&bundle)?;
        }
        let standalone = vsift(
            root,
            &[
                "session",
                "retain",
                SESSION,
                "--output",
                path_text(&bundle)?,
                "--json",
            ],
            b"",
        )?;
        let standalone_answer = if exit(&standalone) == Some(0) {
            None
        } else {
            Some(failure(&standalone)?)
        };
        assert_eq!(finish_answer, standalone_answer, "retain");
        assert_eq!(finish_bundle_written, bundle.exists(), "retain");

        let finish = vsift(
            root,
            &["handoff", "finish", SESSION, "--close", "--json"],
            report(&handoff(segment, "closed")).as_bytes(),
        )?;
        let standalone = vsift(root, &["session", "close", SESSION, "--json"], b"")?;
        assert_eq!(exit(&finish), exit(&standalone), "close");
        if exit(&finish) != Some(0) {
            assert_eq!(failure(&finish)?, failure(&standalone)?, "close");
        }
    }
    Ok(())
}

/// A close after a passing draft closes the session once.
#[tokio::test]
async fn a_passing_draft_closes_the_session() -> TestResult {
    let root = OwnedRoot::new()?;
    let segment = seed_session(&root, 0).await?;
    let value = finish_json(&vsift(
        &root,
        &["handoff", "finish", SESSION, "--close", "--json"],
        report(&handoff(&segment, "closed")).as_bytes(),
    )?)?;
    assert_eq!(value["data"]["finished"], true, "{value}");
    assert_eq!(value["data"]["close"]["state"], "closed");
    let status = envelope(&vsift(
        &root,
        &["session", "status", SESSION, "--json"],
        b"",
    )?)?;
    assert_eq!(status["data"]["state"], "closed");
    Ok(())
}

/// Two writes, or `--include-source` without `--retain`, are refused
/// before the draft is read.
#[test]
fn conflicting_flags_are_invalid_arguments() -> TestResult {
    let root = OwnedRoot::new()?;
    let bundle = root.path("evidence bundle");
    for arguments in [
        vec![
            "handoff",
            "finish",
            SESSION,
            "--retain",
            path_text(&bundle)?,
            "--close",
            "--json",
        ],
        vec!["handoff", "finish", SESSION, "--include-source", "--json"],
    ] {
        let output = vsift(&root, &arguments, b"")?;
        assert_eq!(exit(&output), Some(2), "{arguments:?}");
        let value = envelope(&output)?;
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
        assert!(!bundle.exists());
    }
    Ok(())
}
