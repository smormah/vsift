//! Public CLI contract for a paged read of an open revision (ADR 0033, RV-1
//! finding 4): `transcript get` and `search` give `data.since` only on the
//! answer that ends the read, and a reader who follows the pages and then the
//! token is given every published segment exactly once.
//!
//! The session and its parts are committed directly through the session store,
//! from a run recognised in memory by stand-ins, so the journey runs
//! everywhere without `FFmpeg`: the binary then reads the open revision
//! exactly as it reads one a running job publishes.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fs,
    future::Future,
    num::{NonZeroU16, NonZeroU32},
    path::PathBuf,
    process::Output,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::Command;
use serde_json::Value;
use vsift_application::{
    AsrCancellation, AsrFailure, AsrRevisionRequest, AsrTranscription, CheckpointScope,
    InitializeSessionStorage, InitializeSessionStorageRequest, NoCheckpoints, NoProgress,
    RecognizerIdentity, RevisionStore, RunObserver, RunSoFar, SpeechAudioError, SpeechAudioSource,
    SpeechPcm, SpeechRecognitionError, SpeechRecognizer, TranscribeRangeRequest,
    build_open_asr_revision, transcribe_range_checkpointed, whole_file_source_segment,
};
use vsift_domain::{
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild, ChunkPlan,
    ChunkTime, CueText, DurabilityRequirement, JobId, LanguageTag, MAX_EARLY_AUDIO_MICROS,
    MediaTime, OperationId, PlannedChunk, ProviderChunkOutput, ProviderSegment, RecognitionKey,
    SessionId, Sha256Hex, StorageGeneration, TranscriptRevision,
};
use vsift_infrastructure::{FilesystemSessionStore, SourceSnapshot};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-growing-read-cli-";
const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const JOB: &str = "job_0123456789abcdef0123456789abcdef";
const SECOND: u64 = 1_000_000;
const SOURCE_SECONDS: u64 = 120;

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
        self.path("sessions")
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

fn identity() -> Built<RecognizerIdentity> {
    Ok(RecognizerIdentity {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        threads: NonZeroU16::new(4).ok_or("zero")?,
    })
}

/// Loud audio over every chunk's window.
struct Audio;

impl SpeechAudioSource for Audio {
    fn speech_pcm(
        &self,
        chunk: &PlannedChunk,
    ) -> impl Future<Output = Result<SpeechPcm, SpeechAudioError>> + Send {
        std::future::ready(
            usize::try_from(chunk.window().duration_micros() / 1_000 * 16)
                .map_err(|_| SpeechAudioError::ResourceLimit)
                .map(|count| SpeechPcm {
                    actual_start: chunk.window().start(),
                    samples: vec![3_000; count],
                }),
        )
    }
}

/// Each chunk says two sentences of its own, at 2 and 12 s into it, both
/// holding the word "signal".
struct Recogniser;

impl SpeechRecognizer for Recogniser {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send {
        std::future::ready(identity().map_err(|_| SpeechRecognitionError::Io))
    }

    fn recognize(
        &self,
        chunk: &PlannedChunk,
        _pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        let segment = |start_ms: u64, end_ms: u64, words: String| {
            Ok::<_, SpeechRecognitionError>(ProviderSegment {
                start: ChunkTime::from_millis(start_ms).ok_or(SpeechRecognitionError::Io)?,
                end: ChunkTime::from_millis(end_ms).ok_or(SpeechRecognitionError::Io)?,
                text: Some(
                    CueText::new(words.clone(), words).map_err(|_| SpeechRecognitionError::Io)?,
                ),
                tokens: Vec::new(),
            })
        };
        let index = chunk.index();
        std::future::ready((|| {
            Ok(ProviderChunkOutput {
                language: LanguageTag::parse("en").ok(),
                segments: vec![
                    segment(2_000, 5_000, format!("signal {index} begins"))?,
                    segment(12_000, 15_000, format!("signal {index} goes on"))?,
                ],
            })
        })())
    }
}

struct NotCancelled;

impl AsrCancellation for NotCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Records what an open revision would hold after every chunk but the last.
struct Cuts(Mutex<Vec<Result<(AsrTranscription, MediaTime), AsrFailure>>>);

impl RunObserver for Cuts {
    fn chunk_finished(&self, run: &RunSoFar<'_>) {
        if run.next().is_some()
            && let Ok(mut cuts) = self.0.lock()
        {
            cuts.push(run.open(MAX_EARLY_AUDIO_MICROS));
        }
    }
}

/// A session over a 120 s stand-in source with no transcript, its store, and
/// the open revision of every cut of a whole-source run of it.
struct Seeded {
    store: FilesystemSessionStore,
    session: SessionId,
    open: Vec<TranscriptRevision>,
    now: u64,
}

async fn seed(root: &OwnedRoot) -> Built<Seeded> {
    let media = root.path("stand-in.mp4");
    fs::write(&media, b"\0\0\0\x18ftypisomgrowing-read")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let session = SessionId::parse(SESSION)?;
    let opener = OperationId::parse("op_0123456789abcdef")?;
    let store = FilesystemSessionStore::provision_default(root.sessions())?;
    let registration = store.register_session(&session, &opener, now)?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session.clone(),
            opener,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    drop(registration);
    let store = FilesystemSessionStore::open_existing(root.sessions())?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session,
        &OperationId::parse("op_1111111111111111")?,
        &media,
    )?;
    store.activate_source(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        now,
    )?;
    let source_id = snapshot.id().clone();
    drop(snapshot);
    let source =
        whole_file_source_segment(&source_id, MediaTime::from_micros(SOURCE_SECONDS * SECOND))?;
    let expected = identity()?;
    let cuts = Cuts(Mutex::new(Vec::new()));
    let key = RecognitionKey::new(Sha256Hex::parse(DIGEST)?);
    transcribe_range_checkpointed(
        TranscribeRangeRequest {
            source_segment: &source,
            range: source.range(),
            plan: ChunkPlan::R0,
            audio_stream: 1,
            expected: &expected,
        },
        CheckpointScope {
            checkpoints: &NoCheckpoints,
            key: &key,
            progress: &NoProgress,
            stopwatch: &vsift_application::StoppedStopwatch,
            seed: None,
            observer: &cuts,
        },
        &Audio,
        &Recogniser,
        &NotCancelled,
    )
    .await?;
    let mut open = Vec::new();
    for cut in cuts.0.into_inner().map_err(|_| "poisoned")? {
        let (transcription, settled_until) = cut?;
        open.push(build_open_asr_revision(
            AsrRevisionRequest {
                session_id: &session,
                source_id: &source_id,
                source_segment: &source,
                number: NonZeroU32::MIN,
                transcription,
                splice: None,
            },
            settled_until,
        )?);
    }
    Ok(Seeded {
        store,
        session,
        open,
        now,
    })
}

impl Seeded {
    /// Commits `open` as part `number`, continuing the parts before it.
    fn publish(
        &self,
        generation: u64,
        open: &TranscriptRevision,
        number: u32,
        first: usize,
    ) -> TestResult {
        self.store.publish_part(
            &self.session,
            &OperationId::parse(format!("op_{:016x}", 0x3333_0000 + u64::from(number)))?,
            StorageGeneration::from_value(generation),
            open,
            &JobId::parse(JOB)?,
            NonZeroU32::new(number).ok_or("zero")?,
            first,
            self.now,
        )?;
        Ok(())
    }
}

/// Runs `vsift --json` with an isolated per-user base and the root's store,
/// and returns the successful answer's data.
fn vsift(root: &OwnedRoot, arguments: &[&str]) -> Built<Value> {
    let base = root.path("user");
    let output: Output = Command::cargo_bin("vsift")?
        .env("LOCALAPPDATA", &base)
        .env("XDG_CONFIG_HOME", &base)
        .env("HOME", &base)
        .arg("--session-root")
        .arg(root.sessions())
        .args(arguments)
        .arg("--json")
        .output()?;
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert!(value["error"].is_null(), "the command failed: {value}");
    Ok(value["data"].clone())
}

/// The segment ids of an answer's items, in order.
fn ids(data: &Value) -> Built<Vec<String>> {
    data["items"]
        .as_array()
        .ok_or("no items")?
        .iter()
        .map(|item| {
            item["segment_id"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "an item without a segment id".into())
        })
        .collect()
}

/// Follows a plain read page by page (`first` the first page's arguments,
/// `--cursor` appended after it): every page but the last has no since token.
/// Returns the segment ids of every page and the last page's since token.
fn read_every_page(root: &OwnedRoot, first: &[&str]) -> Built<(Vec<String>, String)> {
    let mut given = Vec::new();
    let mut data = vsift(root, first)?;
    let mut pages = 1;
    loop {
        assert_eq!(data["revision"]["state"], "open");
        given.extend(ids(&data)?);
        let Some(cursor) = data["next_cursor"].as_str().map(str::to_owned) else {
            break;
        };
        assert!(
            data.get("since").is_none(),
            "page {pages} has a next page and a since token: {}",
            data["since"]
        );
        let mut arguments = first.to_vec();
        arguments.extend(["--cursor", cursor.as_str()]);
        data = vsift(root, &arguments)?;
        pages += 1;
    }
    assert!(pages > 1, "the read was not paged");
    let token = data["since"]["since_token"]
        .as_str()
        .ok_or("the last page has no since token")?
        .to_owned();
    assert_eq!(data["since"]["revision_state"], "open");
    Ok((given, token))
}

/// `transcript get` and `search` page an open revision; the since token of the
/// last page then gives exactly what was published after the read.
#[tokio::test]
async fn a_paged_read_of_an_open_revision_gives_its_since_token_on_the_last_page() -> TestResult {
    let root = OwnedRoot::new()?;
    let seeded = seed(&root).await?;
    assert_eq!(seeded.open.len(), 4);
    let first = &seeded.open[1];
    let later = &seeded.open[3];
    assert!(first.segments().len() >= 3, "too few segments to page");
    assert!(later.segments().len() > first.segments().len());
    seeded.publish(1, first, 1, 0)?;
    let published: Vec<String> = first
        .segments()
        .iter()
        .map(|segment| segment.id().as_str().to_owned())
        .collect();
    let added: Vec<String> = later
        .segments()
        .get(first.segments().len()..)
        .ok_or("the later cut is shorter")?
        .iter()
        .map(|segment| segment.id().as_str().to_owned())
        .collect();
    let to = (SOURCE_SECONDS * SECOND).to_string();

    let get = [
        "transcript",
        "get",
        SESSION,
        "--from",
        "0",
        "--to",
        to.as_str(),
        "--limit",
        "2",
    ];
    let (given, get_token) = read_every_page(&root, &get)?;
    assert_eq!(
        given, published,
        "the pages did not give every segment once"
    );

    let search = ["search", SESSION, "--query", "signal", "--limit", "1"];
    let (hits, search_token) = read_every_page(&root, &search)?;
    assert_eq!(
        hits.iter().collect::<BTreeSet<_>>(),
        published.iter().collect::<BTreeSet<_>>(),
        "the pages did not give every hit"
    );
    assert_eq!(hits.len(), published.len(), "a hit was given twice");

    // The run publishes its next part; each token reads exactly what it added.
    seeded.publish(2, later, 2, first.segments().len())?;
    let since = vsift(
        &root,
        &["transcript", "get", SESSION, "--since", &get_token],
    )?;
    assert_eq!(ids(&since)?, added);
    let since = vsift(
        &root,
        &[
            "search",
            SESSION,
            "--query",
            "signal",
            "--since",
            &search_token,
        ],
    )?;
    assert_eq!(ids(&since)?, added);
    Ok(())
}
