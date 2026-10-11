//! A segment id of a superseded revision still resolves (#372).
//!
//! After `transcript retranscribe --from --to` every segment carried unchanged
//! into the new revision has a **new** `segment_id`, with the old one under
//! `carried_from` (`cli-v1.md`, "Revisions"). A client that collected citations
//! and then re-checked a range holds ids of a superseded revision, so what the
//! contract says about such an id has to be true, and nothing pinned it:
//!
//! - `transcript get --revision <the earlier revision>` still returns the old
//!   segments under their old ids, the replaced one included;
//! - `handoff check --session` accepts a citation of an id of any committed
//!   revision, so a report that cites a superseded id is not refused as unknown
//!   (it may still point at text the run replaced: the risk is meaning, not
//!   acceptance, and the skill says to cite the newest revision's ids);
//! - a retained bundle holds the record of every revision, so those ids are in
//!   it too.
//!
//! The two revisions are committed through the session store, as
//! `search_cli_contract` and `handoff_cli_contract` do, so the journey runs
//! everywhere without `FFmpeg` or whisper.cpp: the first is the F10 import and
//! the second is a range retranscription of its dialog cue, built by the
//! application's own use case. The binary then reads them as it reads any
//! session. No behaviour is changed here; the test records what already holds.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fs,
    num::{NonZeroU16, NonZeroU32},
    path::PathBuf,
    process::Output,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::{Value, json};
use vsift::{
    DurabilityRequirement, MediaTime, OperationId, SessionId, StorageGeneration, TranscriptOffset,
};
use vsift_application::{
    AsrRevisionRequest, AsrTranscription, ForegroundSessionPort, ImportedRevisionRequest,
    InitializeSessionStorage, InitializeSessionStorageRequest, RevisionSplice, build_asr_revision,
    build_imported_revision,
};
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, ChunkTime, CueText, ProviderChunkOutput,
    ProviderSegment, ProviderToken, ProviderTokenKind, SessionArtifactKind, Sha256Hex,
    SourceSegment, TimeRange, merge_chunks, plan_chunks, validate_chunk_output,
};
use vsift_infrastructure::{
    FilesystemSessionStore, SourceSnapshot, encode_transcript_record, read_supplied_transcript,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-superseded-ids-cli-";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
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

/// Runs `vsift` with an isolated per-user base, the root's session store and
/// `input` on standard input.
fn vsift(root: &OwnedRoot, arguments: &[&str], input: &[u8]) -> Built<Output> {
    let base = root.path("user");
    Ok(Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .arg("--json")
        .write_stdin(input.to_vec())
        .output()?)
}

/// A `--json` result that succeeded.
fn answer(output: &Output) -> Built<Value> {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn range(from: u64, to: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from),
        MediaTime::from_micros(to),
    )?)
}

/// A transcription of `covered` whose one chunk heard `words` from 0.25 s to
/// 3.4 s of the chunk.
fn heard(source: &SourceSegment, covered: TimeRange, words: &str) -> Built<AsrTranscription> {
    let planned = plan_chunks(source.id(), covered, ChunkPlan::R0)?;
    let run = AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero")?,
        audio_stream: 1,
        chunks: planned
            .into_iter()
            .map(|chunk| {
                AsrChunkRecord::new(chunk, AsrChunkOutcome::Transcribed { audio: covered })
            })
            .collect(),
    })?;
    let chunk = run.chunks().first().ok_or("no chunk")?.chunk().clone();
    let output = ProviderChunkOutput {
        language: None,
        segments: vec![ProviderSegment {
            start: ChunkTime::from_millis(250).ok_or("time")?,
            end: ChunkTime::from_millis(3_400).ok_or("time")?,
            text: Some(CueText::new(words.to_owned(), words.to_owned())?),
            tokens: vec![ProviderToken {
                kind: ProviderTokenKind::Text,
                probability: 0.875,
            }],
        }],
    };
    let (segments, language, warnings) =
        validate_chunk_output(&chunk, covered, source.range(), output)?.into_parts();
    Ok(AsrTranscription {
        run,
        language,
        segments: merge_chunks(&[segments]).segments,
        warnings,
    })
}

/// One open session holding two committed revisions: the F10 import, then a
/// retranscription of the 6-7 s range, which is snapped to the import's
/// whole 5-9 s dialog cue, so the two cues around it are carried unchanged.
async fn seed_session(root: &OwnedRoot) -> Built<()> {
    let source = root.path("stand-in.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomsuperseded-ids")?;
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
    let import = build_imported_revision(ImportedRevisionRequest {
        session_id: &session_id,
        source_id: snapshot.id(),
        source_duration: MediaTime::from_micros(12 * SECOND),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?;
    store.activate_with_transcript(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        now,
        &import,
    )?;

    let replaced = import.snap_to_segments(range(6 * SECOND, 7 * SECOND)?);
    assert_eq!(replaced, range(5 * SECOND, 9 * SECOND)?);
    let source_segment = import.source_segment().clone();
    let second = build_asr_revision(AsrRevisionRequest {
        session_id: &session_id,
        source_id: import.source_id(),
        source_segment: &source_segment,
        number: NonZeroU32::new(2).ok_or("zero")?,
        transcription: heard(&source_segment, replaced, "Dialog R-17 is shown now.")?,
        splice: Some(RevisionSplice {
            base: &import,
            replaced_range: replaced,
        }),
    })?;
    store.publish_artifact(
        &session_id,
        &OperationId::parse("op_3333333333333333")?,
        StorageGeneration::from_value(1),
        SessionArtifactKind::TranscriptRecord,
        &encode_transcript_record(&second)?,
        now + 1,
    )?;
    Ok(())
}

/// The revision a `transcript get` read: its id and its items.
struct Read {
    revision_id: String,
    supersedes: Option<String>,
    items: Vec<Value>,
}

impl Read {
    fn ids(&self) -> Built<BTreeSet<String>> {
        self.items
            .iter()
            .map(|item| {
                item["segment_id"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "an item has no segment_id".into())
            })
            .collect()
    }
}

fn read(root: &OwnedRoot, revision: Option<&str>) -> Built<Read> {
    let mut arguments = vec![
        "transcript",
        "get",
        SESSION,
        "--from",
        "0",
        "--to",
        "12000000",
    ];
    if let Some(revision) = revision {
        arguments.extend(["--revision", revision]);
    }
    let value = answer(&vsift(root, &arguments, b"")?)?;
    assert_eq!(
        value["data"]["next_cursor"],
        Value::Null,
        "one page holds them all"
    );
    let revision = &value["data"]["revision"];
    Ok(Read {
        revision_id: revision["revision_id"]
            .as_str()
            .ok_or("no revision id")?
            .to_owned(),
        supersedes: revision["supersedes"].as_str().map(str::to_owned),
        items: value["data"]["items"].as_array().ok_or("no items")?.clone(),
    })
}

/// A handoff draft that cites each id in `segments`, as the skill's report
/// does: one claim over one `transcript_segment` citation per id.
fn draft(segments: &[&str]) -> String {
    let citations: Vec<Value> = segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            json!({"id": format!("e{}", index + 1), "type": "transcript_segment", "segment_id": segment})
        })
        .collect();
    let cited: Vec<String> = (1..=segments.len())
        .map(|index| format!("e{index}"))
        .collect();
    let handoff = json!({
        "handoff_version": "1", "status": "complete",
        "question": "What does the speaker name?",
        "capabilities": {"image_access": "unavailable"},
        "claims": [{"id": "c1", "section": "actual", "kind": "observed", "support": "supported",
            "certainty": "medium", "statement": "The speaker names dialog R-17.", "citations": cited}],
        "citations": citations,
        "gaps": [{"kind": "image_access", "reason": "image_access_unavailable", "note": null}],
        "untrusted_instructions": [],
        "lifecycle": {"action": "left_open"}
    });
    format!(
        "## Problem\n\nThe speaker names the dialog [c1: e1].\n\n```vsift-handoff\n{handoff}\n```\n"
    )
}

/// What a range retranscription did to the ids, as the binary shows it.
struct Revisions {
    first: Read,
    newest: Read,
    /// `(new id, old id)` of each segment carried unchanged into the newest
    /// revision.
    carried: Vec<(String, String)>,
    /// The id, in the newest revision, of the segment the run replaced.
    replaced_new: String,
    /// The id of that segment in the first revision: the one id that only a
    /// superseded revision holds, and the one that names text the run replaced.
    replaced_old: String,
}

/// Reads both revisions and checks the premise of the issue: every segment
/// carried into the newest revision has a **new** id, with the old one under
/// `carried_from`, and the segment the run replaced has a new id and no
/// `carried_from`. No segment id is shared by the two revisions.
fn read_revisions(root: &OwnedRoot) -> Built<Revisions> {
    let newest = read(root, None)?;
    let first_revision = newest
        .supersedes
        .clone()
        .ok_or("the newest revision supersedes nothing")?;
    assert_ne!(newest.revision_id, first_revision);
    let first = read(root, Some(&first_revision))?;
    assert_eq!(first.revision_id, first_revision);
    assert_eq!(first.supersedes, None, "the import is the first revision");
    assert_eq!(first.items.len(), 3, "the F10 import has three cues");
    assert_eq!(newest.items.len(), 3, "and so does the splice");

    let first_ids = first.ids()?;
    let mut carried = Vec::new();
    let mut replaced_new = Vec::new();
    for item in &newest.items {
        let id = item["segment_id"].as_str().ok_or("no id")?;
        match item["carried_from"].as_object() {
            Some(from) => {
                let old = from["segment_id"].as_str().ok_or("no carried id")?;
                assert_eq!(from["revision_id"], first_revision.as_str());
                assert_ne!(id, old, "a carried segment keeps its id");
                assert!(
                    first_ids.contains(old),
                    "carried_from names a first-revision id"
                );
                carried.push((id.to_owned(), old.to_owned()));
            }
            None => replaced_new.push(id.to_owned()),
        }
    }
    assert_eq!(
        carried.len(),
        2,
        "the two cues around the range are carried"
    );
    assert_eq!(replaced_new.len(), 1, "the dialog cue is the one replaced");
    assert!(
        first_ids.is_disjoint(&newest.ids()?),
        "no segment id is shared by the two revisions"
    );
    let replaced_old = first_ids
        .iter()
        .find(|id| carried.iter().all(|(_, old)| old != *id))
        .ok_or("the replaced cue has no id in the first revision")?
        .clone();
    Ok(Revisions {
        first,
        newest,
        carried,
        replaced_new: replaced_new.remove(0),
        replaced_old,
    })
}

/// A read of the earlier revision still returns the old segments under their
/// old ids, the replaced one included, with the text it held; the newest holds
/// the new text under the new id.
fn assert_the_earlier_revision_is_still_readable(revisions: &Revisions) -> TestResult {
    let old = revisions
        .first
        .items
        .iter()
        .find(|item| item["segment_id"] == revisions.replaced_old.as_str())
        .ok_or("the replaced segment is not readable in the first revision")?;
    assert_eq!(old["text"], "Dialog R-17 is displayed now.");
    let new = revisions
        .newest
        .items
        .iter()
        .find(|item| item["segment_id"] == revisions.replaced_new.as_str())
        .ok_or("the new segment is not readable in the newest revision")?;
    assert_eq!(new["text"], "Dialog R-17 is shown now.");
    Ok(())
}

/// `handoff check --session` resolves a citation of every id of every
/// committed revision: the replaced segment's id (only the first revision holds
/// it), a carried segment's old id and its new one, and the new segment's id.
fn assert_a_handoff_check_resolves_every_revision(
    root: &OwnedRoot,
    revisions: &Revisions,
) -> TestResult {
    let cited = [
        revisions.replaced_old.as_str(),
        revisions.carried[0].1.as_str(),
        revisions.carried[0].0.as_str(),
        revisions.replaced_new.as_str(),
    ];
    let checked = answer(&vsift(
        root,
        &["handoff", "check", "--session", SESSION],
        draft(&cited).as_bytes(),
    )?)?;
    assert_eq!(checked["data"]["valid"], true, "{checked}");
    assert_eq!(checked["data"]["session"]["resolved"], true, "{checked}");
    assert_eq!(
        checked["data"]["session"]["identities_checked"],
        cited.len(),
        "{checked}"
    );
    assert_eq!(checked["data"]["errors"], json!([]), "{checked}");

    // The control: an id no revision holds is still refused, so the check
    // above is not one that accepts everything.
    let unknown = answer(&vsift(
        root,
        &["handoff", "check", "--session", SESSION],
        draft(&["tsg_ffffffffffffffffffffffffffffffff"]).as_bytes(),
    )?)?;
    assert_eq!(
        unknown["data"]["errors"][0]["rule"], "segment_not_in_session",
        "{unknown}"
    );
    Ok(())
}

/// A retained bundle holds the record of every revision, so every id of both
/// is in it, and it validates.
fn assert_a_retained_bundle_holds_every_revision(
    root: &OwnedRoot,
    revisions: &Revisions,
) -> TestResult {
    let bundle = root.path("bundle");
    let bundle_arg = bundle.to_str().ok_or("a path is not UTF-8")?;
    answer(&vsift(
        root,
        &["session", "retain", SESSION, "--output", bundle_arg],
        b"",
    )?)?;
    answer(&vsift(root, &["bundle", "validate", bundle_arg], b"")?)?;

    let manifest: Value = serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    let mut retained_revisions = BTreeSet::new();
    let mut retained_ids = BTreeSet::new();
    for artifact in manifest["artifacts"].as_array().ok_or("no artifacts")? {
        let name = artifact["name"].as_str().ok_or("no artifact name")?;
        let record: Value = serde_json::from_slice(&fs::read(bundle.join(name))?)?;
        if record["format"] != "vsift.transcript_record" {
            continue;
        }
        retained_revisions.insert(
            record["revision_id"]
                .as_str()
                .ok_or("a record has no revision id")?
                .to_owned(),
        );
        for segment in record["segments"]
            .as_array()
            .ok_or("a record has no segments")?
        {
            retained_ids.insert(
                segment["id"]
                    .as_str()
                    .ok_or("a segment has no id")?
                    .to_owned(),
            );
        }
    }
    assert_eq!(
        retained_revisions,
        BTreeSet::from([
            revisions.first.revision_id.clone(),
            revisions.newest.revision_id.clone()
        ]),
        "the bundle holds the record of every revision"
    );
    let every_id: BTreeSet<String> = revisions
        .first
        .ids()?
        .union(&revisions.newest.ids()?)
        .cloned()
        .collect();
    assert_eq!(every_id.len(), 6, "three segments in each revision");
    assert_eq!(retained_ids, every_id, "and every id of both");
    Ok(())
}

/// The whole of #372, through the binary: what changes in the ids when a range
/// is retranscribed, and that the ids of the superseded revision still resolve
/// in a read, in a handoff check and in a retained bundle.
#[tokio::test]
async fn an_id_of_a_superseded_revision_still_resolves() -> TestResult {
    let root = OwnedRoot::new()?;
    seed_session(&root).await?;
    let revisions = read_revisions(&root)?;
    assert_the_earlier_revision_is_still_readable(&revisions)?;
    assert_a_handoff_check_resolves_every_revision(&root, &revisions)?;
    assert_a_retained_bundle_holds_every_revision(&root, &revisions)
}
