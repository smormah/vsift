//! Public CLI contract for `bundle search` (ADR 0035).
//!
//! Sessions are committed directly through the session store and retained by
//! the binary, so every journey runs everywhere without `FFprobe`. The bundles
//! are real ones: `session retain` wrote them.

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
    InitializeSessionStorageRequest, SuppliedTranscript, build_imported_revision,
};
use vsift_domain::{
    CueSource, CueText, CueTiming, ImportedCue, ParsedTranscript, SidecarIdentity,
    TranscriptFormat, TranscriptWarnings,
};
use vsift_infrastructure::{FilesystemSessionStore, SourceSnapshot};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-bundle-search-cli-";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const DIGEST: &str = "95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d";
const SECOND: u64 = 1_000_000;

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

struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
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

/// Runs `vsift` with an isolated per-user base and the root's session store.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Built<Output> {
    let base = root.path("user");
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .output()?)
}

/// Runs `bundle search` over `paths`, with `extra` arguments.
fn search(root: &OwnedRoot, paths: &[&Path], query: &str, extra: &[&str]) -> Built<Output> {
    let mut arguments = vec!["--json", "bundle", "search"];
    for path in paths {
        arguments.push(path.to_str().ok_or("path")?);
    }
    arguments.extend(["--query", query]);
    arguments.extend(extra);
    vsift(root, &arguments)
}

/// Parses a `--json` result, validating the envelope and, for a successful
/// search, its data.
fn parsed(output: &Output) -> Built<Value> {
    assert!(output.stderr.is_empty(), "a JSON command wrote to stderr");
    let value: Value = serde_json::from_slice(&output.stdout)?;
    validate("operation-response.schema.json", &value)?;
    if value["error"].is_null() {
        validate("bundle-search-data.schema.json", &value["data"])?;
    }
    Ok(value)
}

fn revision_texts(
    session_id: &SessionId,
    source_id: &vsift_domain::SourceId,
    texts: &[&str],
) -> Built<vsift::TranscriptRevision> {
    let mut cues = Vec::new();
    for (ordinal, text) in (1_u32..).zip(texts) {
        let start = u64::from(ordinal) * SECOND;
        cues.push(ImportedCue {
            source: CueSource::new(
                NonZeroU32::new(ordinal).ok_or("zero")?,
                NonZeroU32::new(ordinal * 4).ok_or("zero")?,
            ),
            timing: CueTiming::new(start, start + 900_000)?,
            text: CueText::new((*text).to_owned(), (*text).to_owned())?,
            speaker: None,
        });
    }
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::WebVtt,
            None,
            cues,
            TranscriptWarnings::default(),
        )?,
        sidecar: SidecarIdentity::new(DIGEST, 100)?,
    };
    Ok(build_imported_revision(ImportedRevisionRequest {
        session_id,
        source_id,
        source_duration: MediaTime::from_micros((u64::try_from(texts.len())? + 2) * SECOND),
        supplied: &supplied,
        offset: TranscriptOffset::ZERO,
        number: NonZeroU32::MIN,
    })?)
}

/// Opens session number `number` with a transcript of `texts`, retains it to
/// `output` with the binary, and closes nothing: the bundle stands alone.
async fn kept(root: &OwnedRoot, number: u32, texts: &[&str], output: &Path) -> Built<String> {
    let source = root.path(&format!("stand-in-{number}.mp4"));
    fs::write(
        &source,
        format!("\0\0\0\x18ftypisomkept-recording-{number}").as_bytes(),
    )?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let session_id = SessionId::parse(format!("ses_{number:032x}"))?;
    let opener = OperationId::parse(format!("op_{number:016x}"))?;
    let store = if root.sessions().exists() {
        FilesystemSessionStore::open_existing(root.sessions())?
    } else {
        FilesystemSessionStore::provision_default(root.sessions())?
    };
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
        &OperationId::parse(format!("op_1{number:015x}"))?,
        &source,
    )?;
    let revision = revision_texts(&session_id, snapshot.id(), texts)?;
    store.activate_with_transcript(
        &snapshot,
        &OperationId::parse(format!("op_2{number:015x}"))?,
        StorageGeneration::INITIAL,
        now,
        &vsift_domain::TimeAnchors::none(),
        &revision,
    )?;
    let retained = vsift(
        root,
        &[
            "--json",
            "session",
            "retain",
            session_id.as_str(),
            "--output",
            output.to_str().ok_or("path")?,
        ],
    )?;
    assert!(
        retained.status.success(),
        "retain failed: {}",
        String::from_utf8_lossy(&retained.stdout)
    );
    Ok(session_id.as_str().to_owned())
}

/// A folder holding bundles `call-1` to `call-<texts.len()>`.
async fn kept_folder(root: &OwnedRoot, texts: &[&[&str]]) -> Built<PathBuf> {
    let folder = root.path("kept");
    fs::create_dir_all(&folder)?;
    for (number, text) in (1_u32..).zip(texts) {
        kept(root, number, text, &folder.join(format!("call-{number}"))).await?;
    }
    Ok(folder)
}

fn hit_texts(value: &Value) -> Built<Vec<String>> {
    value["data"]["hits"]
        .as_array()
        .ok_or("hits")?
        .iter()
        .map(|hit| {
            Ok(format!(
                "{}: {}",
                hit["bundle"].as_str().ok_or("bundle")?,
                hit["segment"]["text"].as_str().ok_or("text")?
            ))
        })
        .collect()
}

/// The story: one question over a folder of kept recordings, every hit naming
/// where it came from, ranked across the recordings. A session in the root that
/// nobody kept is never looked at, however well it matches.
#[tokio::test]
async fn a_folder_of_kept_recordings_answers_one_question() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = kept_folder(
        &root,
        &[
            &["limit of the refund", "the refund limit is 50"],
            &["nothing here"],
            &["refund limit raised"],
        ],
    )
    .await?;
    // A session still open in the root, never retained.
    kept(
        &root,
        9,
        &["the refund limit in a disposable session"],
        &root.path("elsewhere"),
    )
    .await?;
    let output = search(&root, &[&folder], "refund limit", &[])?;
    assert!(output.status.success());
    let value = parsed(&output)?;
    assert_eq!(value["command"], "bundle.search");
    assert_eq!(value["lifecycle"]["mode"], "retained");
    assert_eq!(value["status"], "complete");
    assert_eq!(
        hit_texts(&value)?,
        [
            "call-1: the refund limit is 50",
            "call-3: refund limit raised",
            "call-1: limit of the refund",
        ]
    );
    let first = &value["data"]["hits"][0];
    assert_eq!(first["kind"], "transcript");
    assert_eq!(first["match"], "phrase");
    assert_eq!(first["time_us"], 2 * SECOND);
    assert!(
        first["evidence_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("tsg_"))
    );
    assert_eq!(first["evidence_id"], first["segment"]["segment_id"]);
    assert_eq!(value["data"]["bundles"]["searched"], 3);
    Ok(())
}

/// Nothing was kept, and the session root does not even exist: the command
/// neither needs it nor makes it.
#[tokio::test]
async fn the_session_root_is_never_created_or_read() -> TestResult {
    let keeper = OwnedRoot::new()?;
    let folder = kept_folder(&keeper, &[&["refund limit"]]).await?;
    let root = OwnedRoot::new()?;
    let output = search(&root, &[&folder], "refund limit", &[])?;
    assert!(output.status.success());
    parsed(&output)?;
    assert!(!root.sessions().exists());
    Ok(())
}

/// On-screen text never changes the status: a transcript-only result is
/// `complete`, says the screen was not read, and the human view states it.
#[tokio::test]
async fn on_screen_gaps_are_stated_and_do_not_make_a_result_partial() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = kept_folder(&root, &[&["refund"], &["refund"]]).await?;
    let value = parsed(&search(&root, &[&folder], "refund", &[])?)?;
    assert_eq!(value["status"], "complete");
    assert_eq!(
        value["coverage"],
        json!({"truncated": false, "gaps": [], "reasons": []})
    );
    assert_eq!(value["data"]["bundles"]["screen_text_incomplete"], 2);
    assert_eq!(
        value["data"]["sessions"][0]["screen_text"]["state"],
        "not_run"
    );

    let human = vsift(
        &root,
        &[
            "bundle",
            "search",
            folder.to_str().ok_or("path")?,
            "--query",
            "refund",
        ],
    )?;
    let text = String::from_utf8(human.stdout)?;
    assert!(
        text.contains("On-screen text was not read in full for 2 of 2 recordings"),
        "{text}"
    );
    assert!(text.contains("A hit is a lead, not a citation"), "{text}");
    Ok(())
}

/// A folder name is all that says where a bundle is: no output names a path.
#[tokio::test]
async fn no_result_holds_an_absolute_path() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = kept_folder(&root, &[&["refund"]]).await?;
    let good = search(&root, &[&folder], "refund", &[])?;
    let stdout = String::from_utf8(good.stdout)?;
    let prefix = root.0.file_name().ok_or("name")?.to_str().ok_or("name")?;
    assert!(!stdout.contains(prefix), "{stdout}");
    let refused = search(&root, &[&root.path("missing")], "refund", &[])?;
    let stdout = String::from_utf8(refused.stdout)?;
    assert!(!stdout.contains(prefix), "{stdout}");
    Ok(())
}

/// A wrong folder must never read as "nobody said it".
#[tokio::test]
async fn a_path_with_no_bundle_is_refused() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = kept_folder(&root, &[&["refund"]]).await?;
    let empty = root.path("empty");
    fs::create_dir(&empty)?;
    for wrong in [empty, root.path("missing")] {
        let output = search(&root, &[&folder, &wrong], "refund", &[])?;
        let value: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
        assert!(
            value["error"]["remediation"][0]["summary"]
                .as_str()
                .is_some_and(|text| text.contains("session retain")),
            "{value}"
        );
    }
    Ok(())
}

/// One damaged bundle costs that bundle, never the search, and the result says
/// so; a bundle hit by the manifest check is not read at all.
#[tokio::test]
async fn a_damaged_bundle_is_listed_and_the_rest_are_searched() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = kept_folder(&root, &[&["refund"], &["refund"]]).await?;
    fs::write(folder.join("call-1").join("bundle.json"), b"{}")?;
    let value = parsed(&search(&root, &[&folder], "refund", &[])?)?;
    assert_eq!(value["status"], "partial");
    assert_eq!(hit_texts(&value)?, ["call-2: refund"]);
    assert_eq!(value["data"]["bundles"]["unavailable"], 1);
    assert_eq!(value["data"]["sessions"][0]["state"], "unavailable");
    assert_eq!(value["data"]["sessions"][0]["reason"], "integrity_failure");
    assert_eq!(value["coverage"]["reasons"], json!(["bundle_unavailable"]),);
    Ok(())
}

/// A transcript record changed after retaining is caught by its digest.
#[tokio::test]
async fn a_changed_transcript_record_is_refused_by_its_digest() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = kept_folder(&root, &[&["refund"]]).await?;
    let bundle = folder.join("call-1");
    let manifest: Value = serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    let record = manifest["artifacts"][0]["name"]
        .as_str()
        .ok_or("artifact")?;
    fs::write(bundle.join(record), b"{\"changed\": true}")?;
    let value = parsed(&search(&root, &[&folder], "refund", &[])?)?;
    assert_eq!(value["status"], "partial");
    assert_eq!(value["data"]["total_hits"], 0);
    assert_eq!(
        value["data"]["sessions"][0]["transcript"]["state"],
        "unavailable"
    );
    assert_eq!(
        value["data"]["sessions"][0]["transcript"]["reason"],
        "integrity_failure"
    );
    Ok(())
}

/// The order of the paths changes nothing, and neither does naming a bundle
/// both directly and through its folder.
#[tokio::test]
async fn the_order_of_the_paths_changes_nothing() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = kept_folder(&root, &[&["refund"], &["refund"]]).await?;
    let one = folder.join("call-1");
    let two = folder.join("call-2");
    let forward = parsed(&search(&root, &[&one, &two], "refund", &[])?)?;
    let backward = parsed(&search(&root, &[&two, &one], "refund", &[])?)?;
    assert_eq!(forward["data"]["hits"], backward["data"]["hits"]);
    let twice = parsed(&search(&root, &[&one, &folder], "refund", &[])?)?;
    assert_eq!(twice["data"]["total_hits"], 2);
    assert_eq!(twice["data"]["bundles"]["searched"], 2);
    Ok(())
}

/// Two folders with one name under different parents are told apart by path,
/// without a manifest being read, whichever is named first.
#[tokio::test]
async fn equal_folder_names_are_ordered_by_their_path() -> TestResult {
    let root = OwnedRoot::new()?;
    let (first, second) = (root.path("a"), root.path("b"));
    fs::create_dir(&first)?;
    fs::create_dir(&second)?;
    kept(&root, 1, &["refund first"], &first.join("call")).await?;
    kept(&root, 2, &["refund second"], &second.join("call")).await?;
    let forward = parsed(&search(&root, &[&first, &second], "refund", &[])?)?;
    let backward = parsed(&search(&root, &[&second, &first], "refund", &[])?)?;
    assert_eq!(forward["data"]["hits"], backward["data"]["hits"]);
    assert_eq!(
        hit_texts(&forward)?,
        ["call: refund first", "call: refund second"]
    );
    Ok(())
}

/// A cursor goes on from its page, and a bundle moved away refuses it rather
/// than restarting the search.
#[tokio::test]
async fn pages_follow_the_cursor_and_refuse_a_changed_set() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = kept_folder(&root, &[&["one", "one"], &["one"]]).await?;
    let first = parsed(&search(&root, &[&folder], "one", &["--limit", "2"])?)?;
    assert_eq!(first["data"]["total_hits"], 3);
    let cursor = first["data"]["next_cursor"].as_str().ok_or("cursor")?;
    let second = parsed(&search(
        &root,
        &[&folder],
        "one",
        &["--limit", "2", "--cursor", cursor],
    )?)?;
    assert_eq!(second["data"]["hits"].as_array().map(Vec::len), Some(1));
    assert!(second["data"]["next_cursor"].is_null());

    fs::rename(folder.join("call-2"), root.path("moved"))?;
    let refused: Value = serde_json::from_slice(
        &search(
            &root,
            &[&folder],
            "one",
            &["--limit", "2", "--cursor", cursor],
        )?
        .stdout,
    )?;
    assert_eq!(refused["error"]["code"], "INVALID_ARGUMENT");
    Ok(())
}

/// The bound on opening: a bundle past the 64th is named and never opened,
/// which a broken manifest proves, because opening it would report it
/// `unavailable`.
#[tokio::test]
async fn a_bundle_past_the_limit_is_never_opened() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = root.path("kept");
    fs::create_dir(&folder)?;
    let first = folder.join("call-00");
    kept(&root, 1, &["one"], &first).await?;
    for at in 1..=64 {
        let copy = folder.join(format!("call-{at:02}"));
        fs::create_dir(&copy)?;
        fs::write(copy.join("bundle.json"), b"not json")?;
    }
    let value = parsed(&search(&root, &[&folder], "one", &[])?)?;
    assert_eq!(value["status"], "partial");
    assert_eq!(value["data"]["bundles"]["found"], 65);
    assert_eq!(value["data"]["bundles"]["searched"], 1);
    assert_eq!(value["data"]["bundles"]["unavailable"], 63);
    assert_eq!(value["data"]["bundles"]["not_searched"], 1);
    assert_eq!(
        value["data"]["not_searched"],
        json!([{"bundle": "call-64", "reason": "bundle_limit"}])
    );
    assert_eq!(
        value["coverage"]["reasons"],
        json!(["bundle_unavailable", "bundle_limit"])
    );
    Ok(())
}

/// More entries than the whole call may look at are refused, whatever they are:
/// a cut would depend on the order the filesystem lists them in.
#[tokio::test]
async fn more_entries_than_the_bound_are_refused() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = root.path("crowded");
    fs::create_dir(&folder)?;
    for at in 0..=4_096 {
        fs::write(folder.join(format!("entry-{at}")), b"")?;
    }
    let output = search(&root, &[&folder], "one", &[])?;
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        value["error"]["remediation"][0]["summary"]
            .as_str()
            .is_some_and(|text| text.contains("4,096")),
        "{value}"
    );
    Ok(())
}

/// Over the path bound nothing is read.
#[test]
fn more_paths_than_the_bound_are_refused() -> TestResult {
    let root = OwnedRoot::new()?;
    let paths: Vec<PathBuf> = (0..65).map(|at| root.path(&format!("path-{at}"))).collect();
    let borrowed: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let value: Value = serde_json::from_slice(&search(&root, &borrowed, "one", &[])?.stdout)?;
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    assert!(
        value["error"]["remediation"][0]["summary"]
            .as_str()
            .is_some_and(|text| text.contains("at most 64")),
        "{value}"
    );
    Ok(())
}

/// The budget at the command: manifests that declare records at their limits
/// stop at 1 GiB, and what is left over is named, with no record read. The
/// declared files do not exist, so a bundle read at all reports `unavailable`.
#[tokio::test]
async fn the_read_budget_stops_at_declared_sizes() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = root.path("kept");
    fs::create_dir(&folder)?;
    let template = folder.join("call-00");
    kept(&root, 1, &["one"], &template).await?;
    let mut manifest: Value = serde_json::from_slice(&fs::read(template.join("bundle.json"))?)?;
    let digest = "a".repeat(64);
    manifest["artifacts"] = json!([{
        "kind": "transcript_record",
        "name": format!("artifact-{digest}.json"),
        "sha256": digest,
        "bytes": 24 * 1024 * 1024,
    }, {
        "kind": "screen_text_index_record",
        "name": format!("artifact-{digest}.json"),
        "sha256": digest,
        "bytes": 48 * 1024 * 1024,
    }, {
        "kind": "visual_index_record",
        "name": format!("artifact-{digest}.json"),
        "sha256": digest,
        "bytes": 8 * 1024 * 1024,
    }]);
    for at in 0..64 {
        // A different session on each makes every manifest its own bundle.
        manifest["session_id"] = json!(format!("ses_{at:032x}"));
        let bundle = folder.join(format!("call-{at:02}"));
        if at > 0 {
            fs::create_dir(&bundle)?;
            restrict_like(&template, &bundle)?;
        }
        fs::write(bundle.join("bundle.json"), serde_json::to_vec(&manifest)?)?;
    }
    let value = parsed(&search(&root, &[&folder], "one", &[])?)?;
    assert_eq!(value["status"], "partial");
    assert_eq!(value["data"]["bundles"]["searched"], 12);
    assert_eq!(value["data"]["bundles"]["not_searched"], 52);
    assert_eq!(value["data"]["not_searched"][0]["reason"], "read_budget");
    assert_eq!(
        value["coverage"]["reasons"],
        json!(["bundle_unavailable", "read_budget"])
    );
    Ok(())
}

/// Gives `bundle` the private permissions a bundle has.
#[cfg(unix)]
fn restrict_like(_template: &Path, bundle: &Path) -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(bundle, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Gives `bundle` the private permissions a bundle has: the only way a test can
/// on Windows is `session retain` itself, which wrote `template`.
#[cfg(windows)]
fn restrict_like(template: &Path, bundle: &Path) -> TestResult {
    let copied = std::process::Command::new("icacls")
        .arg(bundle)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("{}:(OI)(CI)F", env::var("USERNAME")?))
        .output()?;
    assert!(copied.status.success(), "icacls failed for {template:?}");
    Ok(())
}

/// A bundle whose folder is open to others is left out, with what to do.
#[cfg(unix)]
#[tokio::test]
async fn a_bundle_not_private_is_refused_and_says_what_to_do() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let root = OwnedRoot::new()?;
    let folder = kept_folder(&root, &[&["one"], &["one"]]).await?;
    fs::set_permissions(folder.join("call-1"), fs::Permissions::from_mode(0o755))?;
    let value = parsed(&search(&root, &[&folder], "one", &[])?)?;
    assert_eq!(value["data"]["sessions"][0]["reason"], "not_private");
    assert_eq!(value["status"], "partial");
    assert!(
        value["warnings"]
            .as_array()
            .is_some_and(|warnings| warnings.iter().any(|warning| warning
                .as_str()
                .is_some_and(|text| text.contains("Retain the session again")))),
        "{value}"
    );
    Ok(())
}

/// A link is never followed, neither named nor inside a folder of bundles.
#[cfg(unix)]
#[tokio::test]
async fn a_link_is_never_followed() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = kept_folder(&root, &[&["one"]]).await?;
    let link = root.path("link");
    std::os::unix::fs::symlink(folder.join("call-1"), &link)?;
    let named: Value = serde_json::from_slice(&search(&root, &[&link], "one", &[])?.stdout)?;
    assert_eq!(named["error"]["code"], "INVALID_ARGUMENT");
    let inner = root.path("links");
    fs::create_dir(&inner)?;
    std::os::unix::fs::symlink(folder.join("call-1"), inner.join("call"))?;
    let listed: Value = serde_json::from_slice(&search(&root, &[&inner], "one", &[])?.stdout)?;
    assert_eq!(listed["error"]["code"], "INVALID_ARGUMENT");
    Ok(())
}

/// What a person meets on Windows when they gather bundles in one folder that
/// others can read: a move within the drive keeps a bundle's own permissions and
/// a copy takes the folder's, so the copy alone is left out, and told why.
#[cfg(windows)]
#[tokio::test]
async fn on_windows_a_move_keeps_a_bundle_private_and_a_copy_does_not() -> TestResult {
    let root = OwnedRoot::new()?;
    let bundle = root.path("call");
    kept(&root, 1, &["refund"], &bundle).await?;
    let copied = root.path("copied");
    fs::create_dir(&copied)?;
    for entry in fs::read_dir(&bundle)? {
        let entry = entry?;
        fs::copy(entry.path(), copied.join(entry.file_name()))?;
    }
    let shared = root.path("shared");
    fs::create_dir(&shared)?;
    // BUILTIN\Users, inherited by everything created in the folder.
    let granted = std::process::Command::new("icacls")
        .arg(&shared)
        .args(["/grant", "*S-1-5-32-545:(OI)(CI)RX"])
        .output()?;
    assert!(granted.status.success());
    fs::rename(&bundle, shared.join("moved"))?;
    let inherited = shared.join("copy");
    fs::create_dir(&inherited)?;
    for entry in fs::read_dir(&copied)? {
        let entry = entry?;
        fs::copy(entry.path(), inherited.join(entry.file_name()))?;
    }
    let value = parsed(&search(&root, &[&shared], "refund", &[])?)?;
    assert_eq!(value["data"]["bundles"]["searched"], 1);
    assert_eq!(value["data"]["hits"][0]["bundle"], "moved");
    assert_eq!(value["data"]["bundles"]["unavailable"], 1);
    assert_eq!(value["data"]["sessions"][0]["bundle"], "copy");
    assert_eq!(value["data"]["sessions"][0]["reason"], "not_private");
    assert!(
        value["warnings"]
            .as_array()
            .is_some_and(|warnings| warnings.iter().any(|warning| warning
                .as_str()
                .is_some_and(|text| text.contains("Retain the session again")))),
        "{value}"
    );
    Ok(())
}

/// A relative path works, as for `bundle validate`, and so does one that climbs.
#[tokio::test]
async fn a_relative_path_is_read_from_the_working_directory() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = kept_folder(&root, &[&["refund"]]).await?;
    let below = root.path("below");
    fs::create_dir(&below)?;
    let base = root.path("user");
    let output = Command::cargo_bin("vsift")?
        .current_dir(&below)
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .args(["--json", "bundle", "search", "../kept", "--query", "refund"])
        .output()?;
    let value = parsed(&output)?;
    assert_eq!(value["data"]["total_hits"], 1);
    assert!(folder.exists());
    Ok(())
}

/// A hit's ids belong to a session that is gone, so a handoff cannot be checked
/// against one: a hit is a lead and a new session cites.
#[tokio::test]
async fn a_hit_is_a_lead_and_says_so() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = kept_folder(&root, &[&["refund"]]).await?;
    let help = vsift(&root, &["bundle", "search", "--help"])?;
    let text = String::from_utf8(help.stdout)?;
    assert!(text.contains("a lead, not a citation"), "{text}");
    let human = vsift(
        &root,
        &[
            "bundle",
            "search",
            folder.to_str().ok_or("path")?,
            "--query",
            "refund",
        ],
    )?;
    assert!(String::from_utf8(human.stdout)?.contains("open the source with"));
    Ok(())
}

/// The speed check: 64 bundles of an hour of speech each, the most one call
/// opens. It is load, so it is ignored and never runs on a development machine:
/// a hosted runner or LOKI runs it and reads the printed times (ADR 0035).
#[tokio::test]
#[ignore = "load: run on a hosted runner or LOKI only"]
async fn speed_with_the_most_bundles_one_call_opens() -> TestResult {
    let root = OwnedRoot::new()?;
    let folder = root.path("kept");
    fs::create_dir(&folder)?;
    let cue = "the refund limit was discussed again in this part of a long customer call";
    let texts = vec![cue; 1_200];
    for number in 1..=64 {
        kept(
            &root,
            number,
            &texts,
            &folder.join(format!("call-{number:02}")),
        )
        .await?;
    }
    for (label, query) in [("common", "refund limit"), ("absent", "nothing-like-this")] {
        let started = std::time::Instant::now();
        let value = parsed(&search(&root, &[&folder], query, &["--limit", "100"])?)?;
        println!(
            "{label}: {} ms, {} hits, {} bundles searched",
            started.elapsed().as_millis(),
            value["data"]["total_hits"],
            value["data"]["bundles"]["searched"]
        );
        assert_eq!(value["data"]["bundles"]["searched"], 64);
    }
    Ok(())
}

/// Ctrl-C cannot be timed against a short search here, so this pins the code
/// the application's cancellation tests reach: the engine's failure is
/// `CANCELLED`, the code every command ends with on exit 6
/// (`output.rs` pins the exit of the class).
#[test]
fn a_cancelled_search_is_the_published_failure() {
    assert_eq!(
        vsift::EngineError::BundleSearch(vsift_application::BundleSearchError::Cancelled)
            .failure_code(),
        vsift::FailureCode::Cancelled
    );
}
