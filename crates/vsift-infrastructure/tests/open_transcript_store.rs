//! Open transcript revisions in session storage (ADR 0033): the part record,
//! committing parts, reading the open revision as the newest, sealing it (its
//! parts retired in the same generation), the stopped record, and what the
//! store refuses while a revision is open.

use std::{
    env,
    error::Error,
    fs,
    future::Future,
    num::{NonZeroU16, NonZeroU32},
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use vsift_application::{
    AsrCancellation, AsrFailure, AsrRevisionRequest, AsrTranscription, CheckpointScope,
    InitializeSessionStorage, InitializeSessionStorageRequest, NoCheckpoints, NoProgress,
    RecognizerIdentity, RevisionStore, RunObserver, RunSoFar, SessionStorageError,
    SpeechAudioError, SpeechAudioSource, SpeechPcm, SpeechRecognitionError, SpeechRecognizer,
    TranscribeRangeRequest, build_asr_revision, build_open_asr_revision,
    build_stopped_asr_revision, transcribe_range_checkpointed, whole_file_source_segment,
};
use vsift_domain::{
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild, ChunkPlan,
    ChunkTime, CueText, DurabilityRequirement, JobId, LanguageTag, MAX_EARLY_AUDIO_MICROS,
    MediaTime, OperationId, PlannedChunk, ProviderChunkOutput, ProviderSegment, RecognitionKey,
    RevisionState, SessionArtifactKind, SessionId, Sha256Hex, SourceSegment, StorageGeneration,
    TranscriptRevision,
};
use vsift_infrastructure::{
    BundleSourcePolicy, FilesystemSessionStore, SourceSnapshot, decode_transcript_parts,
    decode_transcript_record, encode_transcript_part, encode_transcript_record,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-open-transcript-store-test-";
const OPENED_AT: u64 = 1_000;
const SECOND: u64 = 1_000_000;
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const JOB: &str = "job_0123456789abcdef0123456789abcdef";

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

/// Each chunk says two sentences of its own, at 2 and 12 s into it.
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
                    segment(2_000, 5_000, format!("chunk {index} begins here"))?,
                    segment(12_000, 15_000, format!("chunk {index} goes on here"))?,
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

/// A session with a 120 s source and no transcript; a whole-source run of it
/// (five chunks), sealed, and the open revision of every cut.
struct Session {
    workspace: PathBuf,
    store: FilesystemSessionStore,
    id: SessionId,
    source: SourceSegment,
    sealed: TranscriptRevision,
    open: Vec<TranscriptRevision>,
    // Last: fields drop in order (#423).
    root: OwnedRoot,
}

async fn session() -> Built<Session> {
    let root = OwnedRoot::new()?;
    let media = root.0.join("source.mp4");
    fs::write(&media, b"\0\0\0\x18ftypisomopen-transcript-store")?;
    let workspace = root.0.join("workspace");
    let store = FilesystemSessionStore::provision_default(&workspace)?;
    let session_id = SessionId::parse("ses_0123456789abcdef")?;
    let registration = store.register_session(
        &session_id,
        &OperationId::parse("op_0123456789abcdef")?,
        OPENED_AT,
    )?;
    InitializeSessionStorage::new(store)
        .execute(InitializeSessionStorageRequest::new(
            session_id.clone(),
            OperationId::parse("op_0123456789abcdef")?,
            DurabilityRequirement::Ephemeral,
        ))
        .await?;
    drop(registration);
    let store = FilesystemSessionStore::open_existing(&workspace)?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        &media,
    )?;
    store.activate_source(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        OPENED_AT,
    )?;
    let source_id = snapshot.id().clone();
    drop(snapshot);
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(120 * SECOND))?;
    let expected = identity()?;
    let cuts = Cuts(Mutex::new(Vec::new()));
    let key = RecognitionKey::new(Sha256Hex::parse(DIGEST)?);
    let (transcription, _) = transcribe_range_checkpointed(
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
            observer: &cuts,
            stopwatch: &vsift_application::StoppedStopwatch,
        },
        &Audio,
        &Recogniser,
        &NotCancelled,
    )
    .await?;
    let request = |transcription| AsrRevisionRequest {
        session_id: &session_id,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription,
        splice: None,
    };
    let sealed = build_asr_revision(request(transcription))?;
    let mut open = Vec::new();
    for cut in cuts.0.into_inner().map_err(|_| "poisoned")? {
        let (transcription, settled_until) = cut?;
        open.push(build_open_asr_revision(
            request(transcription),
            settled_until,
        )?);
    }
    Ok(Session {
        workspace,
        store,
        id: session_id,
        source,
        sealed,
        open,
        root,
    })
}

fn job() -> Built<JobId> {
    Ok(JobId::parse(JOB)?)
}

fn part(open: &TranscriptRevision, number: u32, first: usize) -> Built<Vec<u8>> {
    Ok(encode_transcript_part(
        open,
        &job()?,
        NonZeroU32::new(number).ok_or("zero")?,
        first,
        vsift_domain::MERGE_RULES_VERSION,
    )?)
}

/// The cuts of the run publish segments, more at each cut, each a prefix of
/// the sealed revision: the input every test below builds on.
#[tokio::test]
async fn the_cuts_of_a_run_are_open_prefixes_of_its_sealed_revision() -> TestResult {
    let session = session().await?;
    assert_eq!(session.open.len(), 4);
    let mut before = 0;
    for open in &session.open {
        assert_eq!(open.state(), RevisionState::Open);
        let count = open.segments().len();
        assert!(count >= before);
        assert_eq!(
            session.sealed.segments().get(..count),
            Some(open.segments())
        );
        before = count;
    }
    assert!(before > 0, "no cut settled a segment");
    assert!(
        session.source.range().end() > session.open[3].published_until().ok_or("open")?,
        "the last cut is still open"
    );
    Ok(())
}

/// A part holds the revision as it stands and only the segments it adds;
/// parts read back, in order, as the last cut's open revision. Anything out
/// of order, inconsistent or newer is refused.
#[tokio::test]
async fn parts_round_trip_and_are_read_strictly() -> TestResult {
    let session = session().await?;
    let first = &session.open[1];
    let second = &session.open[3];
    let one = part(first, 1, 0)?;
    let two = part(second, 2, first.segments().len())?;
    let read = decode_transcript_parts(&[one.clone(), two.clone()])?;
    assert_eq!(read.revision, *second);
    assert_eq!(read.parts.get(), 2);
    assert_eq!(read.job_id, job()?);
    // The second part stores only what it adds.
    let value: serde_json::Value = serde_json::from_slice(&two)?;
    assert_eq!(
        value["revision"]["segments"].as_array().map(Vec::len),
        Some(second.segments().len() - first.segments().len())
    );
    assert_eq!(value["first_ordinal"], first.segments().len() + 1);

    let damaged = Err(SessionStorageError::IntegrityFailure);
    // Out of order, a gap, a part alone that starts later, none at all.
    assert_eq!(
        decode_transcript_parts(&[two.clone(), one.clone()]),
        damaged
    );
    assert_eq!(
        decode_transcript_parts(&[one.clone(), part(second, 3, first.segments().len())?]),
        damaged
    );
    assert_eq!(decode_transcript_parts(std::slice::from_ref(&two)), damaged);
    assert_eq!(decode_transcript_parts::<Vec<u8>>(&[]), damaged);
    // Another job's part, or one whose time goes back.
    let mut other_job: serde_json::Value = serde_json::from_slice(&two)?;
    other_job["job_id"] = serde_json::json!("job_fedcba9876543210fedcba9876543210");
    assert_eq!(
        decode_transcript_parts(&[one.clone(), serde_json::to_vec(&other_job)?]),
        damaged
    );
    let mut earlier: serde_json::Value = serde_json::from_slice(&two)?;
    earlier["revision"]["published_until_us"] = serde_json::json!(0);
    assert_eq!(
        decode_transcript_parts(&[one.clone(), serde_json::to_vec(&earlier)?]),
        damaged
    );
    // A newer version.
    let mut newer: serde_json::Value = serde_json::from_slice(&one)?;
    newer["schema_version"] = serde_json::json!(2);
    assert_eq!(
        decode_transcript_parts(&[serde_json::to_vec(&newer)?]),
        Err(SessionStorageError::UnsupportedVersion)
    );
    // A sealed revision is not a part.
    assert_eq!(
        encode_transcript_part(
            &session.sealed,
            &job()?,
            NonZeroU32::MIN,
            0,
            vsift_domain::MERGE_RULES_VERSION
        ),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

/// Parts are committed through the store: the open revision is the newest,
/// continued only by its own job, part after part; another record is refused
/// while it is open; and the record that seals it retires its parts in the
/// same generation.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "One scenario: the parts of a revision from the first to the seal"
)]
async fn parts_are_committed_read_as_the_newest_and_retired_by_the_seal() -> TestResult {
    let session = session().await?;
    let store = &session.store;
    let id = &session.id;
    let job = job()?;
    let first = &session.open[1];
    let second = &session.open[3];
    let operation = |text: &str| OperationId::parse(text);
    // A part that does not begin at 1 is refused.
    assert_eq!(
        store.publish_part(
            id,
            &operation("op_3333333333333331")?,
            StorageGeneration::from_value(1),
            first,
            &job,
            NonZeroU32::new(2).ok_or("zero")?,
            0,
            OPENED_AT,
        ),
        Err(SessionStorageError::StateConflict)
    );
    store.publish_part(
        id,
        &operation("op_3333333333333333")?,
        StorageGeneration::from_value(1),
        first,
        &job,
        NonZeroU32::MIN,
        0,
        OPENED_AT,
    )?;
    let (newest, _) = store
        .read_transcript(id, OPENED_AT)?
        .ok_or("no transcript")?;
    assert_eq!(newest, *first);
    // Another job may not continue it, and a part must continue its segments.
    let two = NonZeroU32::new(2).ok_or("zero")?;
    let other = JobId::parse("job_fedcba9876543210fedcba9876543210")?;
    assert_eq!(
        store.publish_part(
            id,
            &operation("op_4444444444444441")?,
            StorageGeneration::from_value(2),
            second,
            &other,
            two,
            first.segments().len(),
            OPENED_AT,
        ),
        Err(SessionStorageError::StateConflict)
    );
    assert_eq!(
        store.publish_part(
            id,
            &operation("op_4444444444444442")?,
            StorageGeneration::from_value(2),
            second,
            &job,
            two,
            0,
            OPENED_AT,
        ),
        Err(SessionStorageError::StateConflict)
    );
    store.publish_part(
        id,
        &operation("op_4444444444444444")?,
        StorageGeneration::from_value(2),
        second,
        &job,
        two,
        first.segments().len(),
        OPENED_AT,
    )?;
    let reopened = FilesystemSessionStore::open_existing(&session.workspace)?;
    let open = reopened
        .read_open_transcript(id, OPENED_AT)?
        .ok_or("no open revision")?;
    assert_eq!(open.revision, *second);
    assert_eq!(open.parts, two);
    assert_eq!(open.job_id, job);
    // While it is open no other record is added, and no bundle is retained.
    assert_eq!(
        reopened.publish_artifact(
            id,
            &operation("op_5555555555555551")?,
            StorageGeneration::from_value(3),
            SessionArtifactKind::TranscriptRecord,
            &encode_transcript_record(&session.sealed)?,
            OPENED_AT,
        ),
        Err(SessionStorageError::StateConflict)
    );
    assert_eq!(
        reopened.retain_bundle(
            id,
            &session.root.0.join("bundle-while-open"),
            BundleSourcePolicy::EvidenceOnly,
        ),
        Err(SessionStorageError::StateConflict)
    );
    // The seal: the sealed revision is the newest, and no part is left.
    reopened.publish_revision(
        id,
        &operation("op_5555555555555555")?,
        StorageGeneration::from_value(3),
        &session.sealed,
        OPENED_AT,
    )?;
    assert_eq!(reopened.read_open_transcript(id, OPENED_AT)?, None);
    let (newest, status) = reopened
        .read_transcript(id, OPENED_AT)?
        .ok_or("no transcript")?;
    assert_eq!(newest, session.sealed);
    assert_eq!(status.generation(), StorageGeneration::from_value(4));
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(
        session
            .workspace
            .join("sessions")
            .join(id.as_str())
            .join("generations")
            .join("4.json"),
    )?)?;
    let kinds: Vec<&str> = manifest["lifecycle"]["artifacts"]
        .as_array()
        .ok_or("no artifacts")?
        .iter()
        .filter_map(|artifact| artifact["kind"].as_str())
        .collect();
    assert!(!kinds.contains(&"transcript_part"), "{kinds:?}");
    assert!(kinds.contains(&"transcript_record"), "{kinds:?}");
    // Sealed, the session is retained as before.
    let bundle = session.root.0.join("bundle");
    reopened.retain_bundle(id, &bundle, BundleSourcePolicy::EvidenceOnly)?;
    FilesystemSessionStore::validate_bundle(&bundle)?;
    Ok(())
}

/// The part records committed as fuzz seeds (`fuzz/seeds/transcript_part`)
/// are what the encoder writes for this run's cuts: one part alone, and two
/// parts separated by 0x1E, so the fuzzer starts from valid parts.
#[tokio::test]
async fn the_committed_part_records_are_what_the_encoder_writes() -> TestResult {
    let session = session().await?;
    let first = &session.open[1];
    let one = part(first, 1, 0)?;
    let mut two = one.clone();
    two.push(0x1e);
    two.extend(part(&session.open[3], 2, first.segments().len())?);
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/transcript_parts");
    for (name, bytes) in [("one-part.json", &one), ("two-parts.bin", &two)] {
        assert!(
            fs::read(directory.join(name))? == *bytes,
            "regenerate {name}: the encoder no longer writes its bytes"
        );
    }
    Ok(())
}

/// A stopped revision (ADR 0033, D1) is a sealed record with its chunks not
/// reached and the time it was published until; it reads back, and its record
/// is read strictly.
#[tokio::test]
async fn a_stopped_revision_round_trips_as_a_record() -> TestResult {
    let session = session().await?;
    let stopped = build_stopped_asr_revision(&session.open[2], None)?;
    assert_eq!(stopped.state(), RevisionState::Stopped);
    let encoded = encode_transcript_record(&stopped)?;
    let value: serde_json::Value = serde_json::from_slice(&encoded)?;
    assert_eq!(
        value["published_until_us"],
        session.open[2].published_until().ok_or("open")?.as_micros()
    );
    assert!(
        value["run"]["chunks"]
            .as_array()
            .ok_or("no chunks")?
            .iter()
            .any(|chunk| chunk["outcome"] == "not_reached")
    );
    assert_eq!(decode_transcript_record(&encoded)?, stopped);
    // A sealed record never names a time it is published until.
    let sealed: serde_json::Value =
        serde_json::from_slice(&encode_transcript_record(&session.sealed)?)?;
    assert!(sealed.get("published_until_us").is_none());
    // A stopped record without its time is damaged.
    let mut without = value.clone();
    without
        .as_object_mut()
        .ok_or("not an object")?
        .remove("published_until_us");
    assert_eq!(
        decode_transcript_record(&serde_json::to_vec(&without)?),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}
