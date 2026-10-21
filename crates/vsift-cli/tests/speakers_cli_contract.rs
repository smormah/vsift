//! Public CLI contract of speaker labels (0.3.0, ADR 0031), through the binary.
//!
//! Runs everywhere: no provider runs. The session (the F10 imported transcript) and
//! its first speaker record are committed directly through the session store, as the
//! transcript stream contract seeds its session, so `transcript get`, `search
//! --speaker` and `transcript name-speaker` are tested without the diarization engine;
//! `transcript speakers` itself is tested for its refusal when the engine is absent
//! (the opt-in `p16_speakers_e2e` runs it with the real engine).

use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fs, io,
    num::NonZeroU32,
    path::PathBuf,
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift::{
    DurabilityRequirement, MediaTime, OperationId, SessionId, StorageGeneration, TranscriptOffset,
};
use vsift_application::{
    ForegroundSessionPort, ImportedRevisionRequest, InitializeSessionStorage,
    InitializeSessionStorageRequest, build_imported_revision, speaker_record_id,
};
use vsift_domain::{
    DiarizationEngine, DiarizationProvenance, DiarizedTurn, EngineSpeaker, SessionArtifactKind,
    Sha256Hex, SpeakerCount, SpeakerLabellingProfile, SpeakerRecordParts, TimeRange,
};
use vsift_infrastructure::{
    FilesystemSessionStore, SourceSnapshot, encode_speaker_record, read_supplied_transcript,
};

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-speakers-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const DIGEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Result<Self, Box<dyn Error>> {
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

/// `vsift` with an isolated per-user base, the root's session store and no `PATH`.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    let base = root.path("user");
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .env("PATH", "")
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .output()?)
}

fn json(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn turn(start: u64, end: u64, speaker: u16) -> Result<DiarizedTurn, Box<dyn Error>> {
    Ok(DiarizedTurn::new(
        TimeRange::new(MediaTime::from_micros(start), MediaTime::from_micros(end))?,
        EngineSpeaker::new(speaker),
    ))
}

/// Commits the F10 session and, unless `labelled` is false, its first speaker
/// record: S1 holds the first segment, S2 the second, and the third is half each.
async fn seed(root: &OwnedRoot, labelled: bool) -> Result<(), Box<dyn Error>> {
    seed_told(root, labelled, SpeakerCount::Automatic).await
}

/// [`seed`] with `count` recorded as what was asked of the engine (two voices either way).
async fn seed_told(
    root: &OwnedRoot,
    labelled: bool,
    count: SpeakerCount,
) -> Result<(), Box<dyn Error>> {
    let source = root.path("f10-stand-in.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomspeaker-labels")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
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
        source_duration: MediaTime::from_micros(12_000_000),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?;
    let generation = store.activate_with_transcript(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        now,
        &revision,
    )?;
    if !labelled {
        return Ok(());
    }
    let parts = SpeakerRecordParts {
        id: speaker_record_id(
            &session_id,
            revision.source_id(),
            revision.id(),
            NonZeroU32::MIN,
        )?,
        number: NonZeroU32::MIN,
        source_id: revision.source_id().clone(),
        revision_id: revision.id().clone(),
        diarized_range: TimeRange::new(
            MediaTime::from_micros(0),
            MediaTime::from_micros(12_000_000),
        )?,
        count,
        profile: SpeakerLabellingProfile::V1,
        provenance: DiarizationProvenance::new(
            DiarizationEngine::SherpaOnnx,
            Sha256Hex::parse(DIGEST)?,
            Sha256Hex::parse(DIGEST)?,
            Sha256Hex::parse(DIGEST)?,
        ),
        turns: vec![
            turn(400_000, 3_600_000, 4)?,
            turn(4_500_000, 8_400_000, 9)?,
            turn(9_000_000, 10_000_000, 4)?,
            turn(10_000_000, 11_000_000, 9)?,
        ],
        names: BTreeMap::new(),
    };
    store.publish_artifact(
        &session_id,
        &OperationId::parse("op_3333333333333333")?,
        generation,
        SessionArtifactKind::SpeakerRecord,
        &encode_speaker_record(&session_id, &parts)?,
        now,
    )?;
    Ok(())
}

#[tokio::test]
async fn a_given_count_the_engine_did_not_keep_is_warned_in_json_and_in_the_terminal() -> TestResult
{
    let root = OwnedRoot::new()?;
    // A stand-in turn set: told three, the engine kept two voices apart.
    seed_told(&root, true, SpeakerCount::given(3)?).await?;
    let page = json(&vsift(
        &root,
        &[
            "transcript",
            "get",
            SESSION,
            "--from",
            "0",
            "--to",
            "12000000",
            "--json",
        ],
    )?)?;
    validate("operation-response.schema.json", &page)?;
    validate("transcript-get-data.schema.json", &page["data"])?;
    let shortfall = &page["data"]["speakers"]["count_shortfall"];
    assert_eq!(shortfall["requested_speaker_count"], 3);
    assert_eq!(shortfall["found_speaker_count"], 2);
    let shown = vsift(
        &root,
        &[
            "transcript",
            "get",
            SESSION,
            "--from",
            "0",
            "--to",
            "12000000",
        ],
    )?;
    assert_eq!(shown.status.code(), Some(0));
    let shown = String::from_utf8(shown.stdout)?;
    assert!(
        shown.contains("Warning: The engine kept 2 of the 3 speakers requested apart"),
        "{shown}"
    );
    assert!(shown.contains("without --speakers"), "{shown}");
    Ok(())
}

#[test]
fn without_the_engine_transcript_speakers_is_a_missing_capability() -> TestResult {
    let root = OwnedRoot::new()?;
    let output = vsift(&root, &["transcript", "speakers", SESSION, "--json"])?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "a usage or capability failure exits 2"
    );
    let failure = json(&output)?;
    validate("operation-response.schema.json", &failure)?;
    assert_eq!(failure["command"], "transcript.speakers");
    assert_eq!(failure["error"]["code"], "MISSING_CAPABILITY");
    let remediation = failure["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("no remediation")?;
    assert!(remediation.contains("--diarizer"), "{remediation}");
    assert!(remediation.contains("Transcripts and search work without them"));
    Ok(())
}

#[tokio::test]
async fn transcript_get_and_search_show_the_labels_and_name_speaker_records_a_name() -> TestResult {
    let root = OwnedRoot::new()?;
    seed(&root, true).await?;
    let page = json(&vsift(
        &root,
        &[
            "transcript",
            "get",
            SESSION,
            "--from",
            "0",
            "--to",
            "12000000",
            "--json",
        ],
    )?)?;
    validate("operation-response.schema.json", &page)?;
    validate("transcript-get-data.schema.json", &page["data"])?;
    let items = page["data"]["items"].as_array().ok_or("no items")?;
    assert_eq!(items[0]["speaker_label"]["display"], "S1, by voice");
    assert_eq!(items[1]["speaker_label"]["label"], "S2");
    assert_eq!(items[2]["speaker_label"]["no_confident_speaker"], "mixed");

    let named = vsift(
        &root,
        &[
            "transcript",
            "name-speaker",
            SESSION,
            "--label",
            "S2",
            "--name",
            "Alice Example",
            "--json",
        ],
    )?;
    assert_eq!(named.status.code(), Some(0));
    let named = json(&named)?;
    validate("operation-response.schema.json", &named)?;
    validate("transcript-speakers-data.schema.json", &named["data"])?;
    assert_eq!(named["status"], "partial");
    assert_eq!(named["data"]["records_in_session"], 2);
    assert_eq!(
        named["data"]["speakers"]["labels"][1]["verified_name"],
        "Alice Example"
    );

    let found = json(&vsift(
        &root,
        &[
            "search",
            SESSION,
            "--query",
            "displayed",
            "--speaker",
            "Alice Example",
            "--json",
        ],
    )?)?;
    validate("search-data.schema.json", &found["data"])?;
    assert_eq!(found["data"]["query"]["speaker"], "S2");
    assert_eq!(found["data"]["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        found["data"]["items"][0]["speaker_label"]["display"],
        "Alice Example (S2, named by the user)"
    );
    let other = json(&vsift(
        &root,
        &[
            "search",
            SESSION,
            "--query",
            "displayed",
            "--speaker",
            "S1",
            "--json",
        ],
    )?)?;
    assert_eq!(other["data"]["items"].as_array().map(Vec::len), Some(0));

    let readable = vsift(
        &root,
        &[
            "transcript",
            "name-speaker",
            SESSION,
            "--label",
            "S2",
            "--clear",
        ],
    )?;
    assert_eq!(readable.status.code(), Some(0));
    let text = String::from_utf8(readable.stdout)?;
    assert!(text.contains("S2, by voice"), "{text}");
    assert!(text.contains("Speaker labels are provisional"), "{text}");
    Ok(())
}

#[tokio::test]
async fn unknown_labels_names_and_records_are_invalid_arguments() -> TestResult {
    let root = OwnedRoot::new()?;
    seed(&root, true).await?;
    for arguments in [
        vec![
            "transcript",
            "name-speaker",
            SESSION,
            "--label",
            "S9",
            "--name",
            "Bob",
            "--json",
        ],
        vec![
            "transcript",
            "name-speaker",
            SESSION,
            "--label",
            "T1",
            "--name",
            "Bob",
            "--json",
        ],
        vec![
            "transcript",
            "name-speaker",
            SESSION,
            "--label",
            "S1",
            "--name",
            "Bob\u{1b}[31m",
            "--json",
        ],
        vec![
            "search",
            SESSION,
            "--query",
            "displayed",
            "--speaker",
            "Nobody",
            "--json",
        ],
    ] {
        let output = vsift(&root, &arguments)?;
        let failure = json(&output)?;
        assert_eq!(
            failure["error"]["code"], "INVALID_ARGUMENT",
            "{arguments:?}"
        );
    }

    let unlabelled = OwnedRoot::new()?;
    seed(&unlabelled, false).await?;
    let failure = json(&vsift(
        &unlabelled,
        &[
            "transcript",
            "name-speaker",
            SESSION,
            "--label",
            "S1",
            "--name",
            "Bob",
            "--json",
        ],
    )?)?;
    assert_eq!(failure["error"]["code"], "INVALID_ARGUMENT");
    let remediation = failure["error"]["remediation"][0]["summary"]
        .as_str()
        .ok_or("no remediation")?;
    assert!(remediation.contains("transcript speakers"), "{remediation}");
    // A revision without speaker labels reads exactly as before.
    let page = json(&vsift(
        &unlabelled,
        &[
            "transcript",
            "get",
            SESSION,
            "--from",
            "0",
            "--to",
            "12000000",
            "--json",
        ],
    )?)?;
    assert!(page["data"].get("speakers").is_none());
    assert!(page["data"]["items"][0].get("speaker_label").is_none());
    Ok(())
}

#[test]
fn the_named_paths_must_exist() -> TestResult {
    let root = OwnedRoot::new()?;
    let missing = root.path("missing-model.onnx");
    let missing = missing.to_str().ok_or("path")?;
    let output = vsift(
        &root,
        &[
            "transcript",
            "speakers",
            SESSION,
            "--diarizer",
            missing,
            "--segmentation-model",
            missing,
            "--embedding-model",
            missing,
            "--json",
        ],
    )?;
    let failure = json(&output)?;
    assert!(
        matches!(
            failure["error"]["code"].as_str(),
            Some("MISSING_CAPABILITY" | "INVALID_ARGUMENT")
        ),
        "{failure}"
    );
    Ok(())
}
