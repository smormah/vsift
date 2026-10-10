//! Local-ASR revisions in session storage (P07 increment 3b): version-2
//! records with carried segments, reading any revision by identity, retained
//! bundles, the private work directory a run holds, and reopening the
//! committed source copy.

use std::{
    env,
    error::Error,
    fs,
    num::{NonZeroU16, NonZeroU32},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use vsift_application::{
    AsrRevisionRequest, AsrTranscription, ForegroundSessionPort, ImportedRevisionRequest,
    InitializeSessionStorage, InitializeSessionStorageRequest, RevisionSplice, SessionStorageError,
    build_asr_revision, build_imported_revision, whole_file_source_segment,
};
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, ChunkTime, CueText, DurabilityRequirement,
    LanguageTag, MediaTime, OperationId, ProviderChunkOutput, ProviderSegment, ProviderToken,
    ProviderTokenKind, SessionArtifactKind, SessionId, Sha256Hex, SourceId, SourceSegment,
    StorageGeneration, TimeRange, TranscriptOffset, TranscriptRevision, TranscriptRevisionId,
    TranscriptWarningKind, TranscriptWarnings, merge_chunks, plan_chunks, validate_chunk_output,
};
use vsift_infrastructure::{
    BundleSourcePolicy, FilesystemSessionStore, SourceError, SourceSnapshot, WhisperOutputLimits,
    decode_transcript_record, encode_transcript_record, parse_whisper_full_json,
    read_supplied_transcript,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-local-asr-store-test-";
const OPENED_AT: u64 = 1_000;
const SECOND: u64 = 1_000_000;
const EXAMPLE_SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const F01_SPEECH_SOURCE: &str =
    "src_sha256_f8222a928243160c8dbf5b1a9bc24277e49ac47c11fe5526826462877678e881";
const WHISPER_SHA256: &str = "95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d";
const BASE_MODEL_SHA256: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";
const Q5_1_MODEL_SHA256: &str = "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898";

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

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn load_json(relative: &str) -> Built<serde_json::Value> {
    Ok(serde_json::from_slice(&fs::read(repository(relative))?)?)
}

fn conforms_to_record_schema(instance: &serde_json::Value) -> Built<bool> {
    let schema = load_json("schemas/v1/bundle-transcript-record.schema.json")?;
    Ok(jsonschema::validator_for(&schema)?.is_valid(instance))
}

fn range(from: u64, to: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from),
        MediaTime::from_micros(to),
    )?)
}

fn run(source: &SourceSegment, covered: TimeRange, outcome: AsrChunkOutcome) -> Built<AsrRun> {
    run_as(
        AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(BASE_MODEL_SHA256)?),
        source,
        covered,
        outcome,
    )
}

fn run_as(
    model: AsrModel,
    source: &SourceSegment,
    covered: TimeRange,
    outcome: AsrChunkOutcome,
) -> Built<AsrRun> {
    let planned = plan_chunks(source.id(), covered, ChunkPlan::R0)?;
    Ok(AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(WHISPER_SHA256)?),
        model,
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero")?,
        audio_stream: 1,
        chunks: planned
            .into_iter()
            .map(|chunk| AsrChunkRecord::new(chunk, outcome))
            .collect(),
    })?)
}

/// A transcription of `covered` whose one chunk heard `output`.
fn heard(
    source: &SourceSegment,
    covered: TimeRange,
    output: ProviderChunkOutput,
) -> Built<AsrTranscription> {
    let run = run(
        source,
        covered,
        AsrChunkOutcome::Transcribed { audio: covered },
    )?;
    let chunk = run.chunks().first().ok_or("no chunk")?.chunk().clone();
    let (segments, language, warnings) =
        validate_chunk_output(&chunk, covered, source.range(), output)?.into_parts();
    Ok(AsrTranscription {
        run,
        language,
        segments: merge_chunks(&[segments]).segments,
        warnings,
    })
}

fn recorded_f01() -> Built<ProviderChunkOutput> {
    Ok(parse_whisper_full_json(
        &fs::read(repository(
            "crates/vsift-infrastructure/tests/fixtures/whisper-1.9.2/F01.base.json",
        ))?,
        WhisperOutputLimits::R0,
    )?)
}

/// The F01 speech revisions of the frozen examples: the whole clip from the
/// recorded whisper output, then 5.5-6 s retranscribed as silence.
fn f01_revisions() -> Built<(TranscriptRevision, TranscriptRevision)> {
    let session = SessionId::parse(EXAMPLE_SESSION)?;
    let source_id = SourceId::parse(F01_SPEECH_SOURCE)?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(6 * SECOND))?;
    let first = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription: heard(&source, source.range(), recorded_f01()?)?,
        splice: None,
    })?;
    let replaced = first.snap_to_segments(range(5_500_000, 6 * SECOND)?);
    let mut warnings = TranscriptWarnings::default();
    warnings.add(
        TranscriptWarningKind::SilentChunksSkipped,
        1,
        NonZeroU32::MIN,
    );
    let second = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::new(2).ok_or("zero")?,
        transcription: AsrTranscription {
            run: run(
                &source,
                replaced,
                AsrChunkOutcome::Silent { audio: replaced },
            )?,
            language: None,
            segments: Vec::new(),
            warnings,
        },
        splice: Some(RevisionSplice {
            base: &first,
            replaced_range: replaced,
        }),
    })?;
    Ok((first, second))
}

#[test]
fn the_frozen_local_asr_record_example_is_the_encoded_spliced_revision() -> TestResult {
    let (_, second) = f01_revisions()?;
    let encoded: serde_json::Value = serde_json::from_slice(&encode_transcript_record(&second)?)?;
    let example = load_json("schemas/v1/examples/bundle-transcript-record.asr.json")?;
    assert!(
        encoded == example,
        "the encoded record differs from the frozen example"
    );
    assert!(conforms_to_record_schema(&example)?);
    assert_eq!(example["schema_version"], 2);
    assert_eq!(
        example["inherited"][0]["provenance"]["local_asr"]["threads"],
        4
    );
    assert!(
        decode_transcript_record(&serde_json::to_vec(&example)?)? == second,
        "the frozen example does not decode to the revision"
    );
    // A revision that carries nothing is written without `inherited`, as in 3a.
    let (first, _) = f01_revisions()?;
    let plain: serde_json::Value = serde_json::from_slice(&encode_transcript_record(&first)?)?;
    assert!(plain.get("inherited").is_none());
    assert!(conforms_to_record_schema(&plain)?);
    Ok(())
}

/// D6: a run with the quantized profile is recorded as `base_q5_1`, conforms
/// to the published record schema and decodes to the same revision.
#[test]
fn a_quantized_profile_run_round_trips_as_base_q5_1() -> TestResult {
    let session = SessionId::parse(EXAMPLE_SESSION)?;
    let source_id = SourceId::parse(F01_SPEECH_SOURCE)?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(6 * SECOND))?;
    let silent = AsrChunkOutcome::Silent {
        audio: source.range(),
    };
    let revision = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription: AsrTranscription {
            run: run_as(
                AsrModel::new(
                    AsrModelProfile::BaseQ5_1,
                    Sha256Hex::parse(Q5_1_MODEL_SHA256)?,
                ),
                &source,
                source.range(),
                silent,
            )?,
            language: None,
            segments: Vec::new(),
            warnings: TranscriptWarnings::default(),
        },
        splice: None,
    })?;
    let encoded = encode_transcript_record(&revision)?;
    let value: serde_json::Value = serde_json::from_slice(&encoded)?;
    assert_eq!(value["run"]["model_profile"], "base_q5_1");
    assert!(conforms_to_record_schema(&value)?);
    assert_eq!(decode_transcript_record(&encoded)?, revision);
    Ok(())
}

/// #353: a revision with a chunk whose recognised output was unusable stores
/// the outcome `unusable` with its decoded range and the warning
/// `provider_chunks_rejected`, conforms to the published record schema and
/// decodes to the same revision. Every other record is written as before, so
/// only a session that holds such a chunk is unreadable by an earlier release
/// (which reads an unknown outcome as damage), as the record's schema says.
#[test]
fn an_unusable_chunk_round_trips_and_conforms_to_the_record_schema() -> TestResult {
    let session = SessionId::parse(EXAMPLE_SESSION)?;
    let source_id = SourceId::parse(F01_SPEECH_SOURCE)?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(55 * SECOND))?;
    let planned = plan_chunks(source.id(), source.range(), ChunkPlan::R0)?;
    let outcomes = [
        AsrChunkOutcome::Silent {
            audio: planned[0].window(),
        },
        AsrChunkOutcome::Unusable {
            audio: planned[1].window(),
        },
    ];
    let run = AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(WHISPER_SHA256)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(BASE_MODEL_SHA256)?),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero")?,
        audio_stream: 1,
        chunks: planned
            .into_iter()
            .zip(outcomes)
            .map(|(chunk, outcome)| AsrChunkRecord::new(chunk, outcome))
            .collect(),
    })?;
    let mut warnings = TranscriptWarnings::default();
    warnings.add(
        TranscriptWarningKind::SilentChunksSkipped,
        1,
        NonZeroU32::MIN,
    );
    warnings.add(
        TranscriptWarningKind::ProviderChunksRejected,
        1,
        NonZeroU32::new(2).ok_or("zero")?,
    );
    let revision = build_asr_revision(AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription: AsrTranscription {
            run,
            language: None,
            segments: Vec::new(),
            warnings,
        },
        splice: None,
    })?;
    let encoded = encode_transcript_record(&revision)?;
    let value: serde_json::Value = serde_json::from_slice(&encoded)?;
    assert_eq!(value["run"]["chunks"][0]["outcome"], "silent");
    assert_eq!(value["run"]["chunks"][1]["outcome"], "unusable");
    assert_eq!(
        value["run"]["chunks"][1]["audio"],
        serde_json::json!({"start_us": 25_000_000, "end_us": 55_000_000})
    );
    assert!(
        value["warnings"]
            .as_array()
            .ok_or("no warnings")?
            .iter()
            .any(|warning| warning["kind"] == "provider_chunks_rejected" && warning["count"] == 1)
    );
    assert!(conforms_to_record_schema(&value)?);
    assert_eq!(decode_transcript_record(&encoded)?, revision);

    // An unusable chunk is a decoded chunk: without its decoded range, or with
    // one that is not in the chunk's window, the record is damaged.
    let mut without_audio = value.clone();
    without_audio["run"]["chunks"][1]["audio"] = serde_json::Value::Null;
    assert_eq!(
        decode_transcript_record(&serde_json::to_vec(&without_audio)?),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

/// #353: text the earlier revision had inside a part of the replaced range
/// that the run could not read is carried into the new revision, and the
/// stored record reads back. The rule is checked again on reading: a record
/// whose chunk is changed to one that was read leaves the carried segment
/// inside a range the run replaced, which is damage, not a different revision.
#[test]
fn text_kept_in_an_unreadable_part_round_trips_and_is_checked_on_reading() -> TestResult {
    let (first, _) = f01_revisions()?;
    let source = first.source_segment().clone();
    // 1-2 s cuts the first revision's 0-5.26 s segment, so the range widens to
    // all of it; the run could not read it, so the segment is kept.
    let replaced = first.snap_to_segments(range(SECOND, 2 * SECOND)?);
    assert_eq!(replaced, range(0, 5_260_000)?);
    let mut warnings = TranscriptWarnings::default();
    warnings.add(
        TranscriptWarningKind::ProviderChunksRejected,
        1,
        NonZeroU32::MIN,
    );
    let kept = build_asr_revision(AsrRevisionRequest {
        session_id: &SessionId::parse(EXAMPLE_SESSION)?,
        source_id: &SourceId::parse(F01_SPEECH_SOURCE)?,
        source_segment: &source,
        number: NonZeroU32::new(2).ok_or("zero")?,
        transcription: AsrTranscription {
            run: run(
                &source,
                replaced,
                AsrChunkOutcome::Unusable { audio: replaced },
            )?,
            language: None,
            segments: Vec::new(),
            warnings,
        },
        splice: Some(RevisionSplice {
            base: &first,
            replaced_range: replaced,
        }),
    })?;
    assert_eq!(kept.segments().len(), 1);
    assert!(kept.segments()[0].carried_from().is_some());
    assert_eq!(kept.replaced_range(), Some(replaced));

    let encoded = encode_transcript_record(&kept)?;
    let value: serde_json::Value = serde_json::from_slice(&encoded)?;
    assert!(conforms_to_record_schema(&value)?);
    assert_eq!(decode_transcript_record(&encoded)?, kept);
    // The record is committed as a fuzz seed, so the fuzzer starts from a
    // valid record of this kind.
    assert!(
        fs::read(
            repository("crates/vsift-infrastructure/tests/data/transcript_records")
                .join("F01-kept-in-an-unreadable-part.json")
        )? == encoded,
        "regenerate the committed record: the encoder no longer writes its bytes"
    );

    for read_instead in ["transcribed", "silent"] {
        let mut changed = value.clone();
        changed["run"]["chunks"][0]["outcome"] = serde_json::Value::from(read_instead);
        assert_eq!(
            decode_transcript_record(&serde_json::to_vec(&changed)?),
            Err(SessionStorageError::IntegrityFailure),
            "{read_instead}"
        );
    }
    Ok(())
}

/// #353: a revision spliced from one that left a part untranscribed records the
/// part (`carried_untranscribed`), because it keeps the provenance only of the
/// runs whose text it carries and the first run's window would otherwise be
/// counted over it. The record round trips, conforms to the published schema
/// and is checked on reading: the parts are in order, merged, inside the source
/// and outside the replaced range.
#[test]
fn what_an_earlier_revision_left_untranscribed_is_recorded_and_checked_on_reading() -> TestResult {
    let (first, _) = f01_revisions()?;
    let source = first.source_segment().clone();
    let session = SessionId::parse(EXAMPLE_SESSION)?;
    let source_id = SourceId::parse(F01_SPEECH_SOURCE)?;
    let splice = |base: &TranscriptRevision,
                  number: u32,
                  replaced: TimeRange,
                  outcome: AsrChunkOutcome|
     -> Built<TranscriptRevision> {
        Ok(build_asr_revision(AsrRevisionRequest {
            session_id: &session,
            source_id: &source_id,
            source_segment: &source,
            number: NonZeroU32::new(number).ok_or("zero")?,
            transcription: AsrTranscription {
                run: run(&source, replaced, outcome)?,
                language: None,
                segments: Vec::new(),
                warnings: TranscriptWarnings::default(),
            },
            splice: Some(RevisionSplice {
                base,
                replaced_range: replaced,
            }),
        })?)
    };
    // 5.5-6 s could not be read, and there was no text there to keep.
    let unread = range(5_500_000, 6 * SECOND)?;
    let second = splice(
        &first,
        2,
        unread,
        AsrChunkOutcome::Unusable { audio: unread },
    )?;
    assert!(second.carried_untranscribed().is_empty());
    // A later run over 5.3-5.4 s, far from it, silent.
    let quiet = range(5_300_000, 5_400_000)?;
    let third = splice(&second, 3, quiet, AsrChunkOutcome::Silent { audio: quiet })?;
    assert_eq!(third.carried_untranscribed(), [unread]);

    let encoded = encode_transcript_record(&third)?;
    let value: serde_json::Value = serde_json::from_slice(&encoded)?;
    assert_eq!(
        value["carried_untranscribed"],
        serde_json::json!([{"start_us": 5_500_000, "end_us": 6_000_000}])
    );
    assert!(conforms_to_record_schema(&value)?);
    assert_eq!(decode_transcript_record(&encoded)?, third);
    // A revision that records nothing is written without the member, as before.
    let before: serde_json::Value = serde_json::from_slice(&encode_transcript_record(&second)?)?;
    assert!(before.get("carried_untranscribed").is_none());
    // The record is committed as a fuzz seed.
    assert!(
        fs::read(
            repository("crates/vsift-infrastructure/tests/data/transcript_records")
                .join("F01-untranscribed-carried.json")
        )? == encoded,
        "regenerate the committed record: the encoder no longer writes its bytes"
    );

    for (label, listed) in [
        (
            "into the replaced range",
            serde_json::json!([{"start_us": 5_350_000, "end_us": 6_000_000}]),
        ),
        (
            "beyond the source",
            serde_json::json!([{"start_us": 5_500_000, "end_us": 7_000_000}]),
        ),
        (
            "out of order",
            serde_json::json!([
                {"start_us": 5_500_000, "end_us": 5_700_000},
                {"start_us": 1_000_000, "end_us": 2_000_000}
            ]),
        ),
        (
            "touching",
            serde_json::json!([
                {"start_us": 5_500_000, "end_us": 5_700_000},
                {"start_us": 5_700_000, "end_us": 6_000_000}
            ]),
        ),
    ] {
        let mut changed = value.clone();
        changed["carried_untranscribed"] = listed;
        assert_eq!(
            decode_transcript_record(&serde_json::to_vec(&changed)?),
            Err(SessionStorageError::IntegrityFailure),
            "{label}"
        );
    }
    // A record of a revision that is not spliced carries none.
    let mut plain: serde_json::Value = serde_json::from_slice(&encode_transcript_record(&first)?)?;
    plain["carried_untranscribed"] = serde_json::json!([{"start_us": 1, "end_us": 2}]);
    assert_eq!(
        decode_transcript_record(&serde_json::to_vec(&plain)?),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

/// Strict decoding re-checks every carried segment against its inherited
/// provenance and the replaced range.
#[test]
fn modified_spliced_records_are_integrity_failures() -> TestResult {
    let (_, second) = f01_revisions()?;
    let original: serde_json::Value = serde_json::from_slice(&encode_transcript_record(&second)?)?;
    let mut into_range = original.clone();
    into_range["replaced_range"] = serde_json::json!({"start_us": 0, "end_us": 6_000_000});
    let mut shifted = original.clone();
    shifted["segments"][0]["start_us"] = serde_json::json!(1);
    let mut orphaned = original.clone();
    orphaned["inherited"] = serde_json::json!([]);
    let mut foreign = original.clone();
    foreign["segments"][0]["carried_from"]["revision_id"] =
        serde_json::Value::from("trv_2222222222222222");
    let mut extra = original.clone();
    extra["segments"][0]["unreviewed"] = serde_json::Value::Bool(true);
    for (label, record) in [
        ("carried segment inside the replaced range", into_range),
        ("shifted carried segment", shifted),
        ("empty inherited", orphaned),
        ("unknown originating revision", foreign),
        ("unknown field", extra),
    ] {
        assert_eq!(
            decode_transcript_record(&serde_json::to_vec(&record)?),
            Err(SessionStorageError::IntegrityFailure),
            "{label}"
        );
    }
    Ok(())
}

/// An initialized session with an activated F10 import (revision 1).
struct Imported {
    root: OwnedRoot,
    workspace: PathBuf,
    store: FilesystemSessionStore,
    session_id: SessionId,
    import: TranscriptRevision,
}

async fn imported_session() -> Built<Imported> {
    let root = OwnedRoot::new()?;
    let source = root.0.join("source.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomlocal-asr-store")?;
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
        OPENED_AT,
        &import,
    )?;
    drop(snapshot);
    Ok(Imported {
        root,
        workspace,
        store,
        session_id,
        import,
    })
}

/// One recognised segment at `[start_ms, end_ms)` of its chunk.
fn spoken(words: &str, start_ms: u64, end_ms: u64) -> Built<ProviderChunkOutput> {
    Ok(ProviderChunkOutput {
        language: LanguageTag::parse("en").ok(),
        segments: vec![ProviderSegment {
            start: ChunkTime::from_millis(start_ms).ok_or("time")?,
            end: ChunkTime::from_millis(end_ms).ok_or("time")?,
            text: Some(CueText::new(words.to_owned(), words.to_owned())?),
            tokens: vec![ProviderToken {
                kind: ProviderTokenKind::Text,
                probability: 0.875,
            }],
        }],
    })
}

/// Revision 2: the import's 5-9 s dialog cue retranscribed.
fn spliced_over_import(session: &Imported) -> Built<TranscriptRevision> {
    let replaced = session
        .import
        .snap_to_segments(range(6 * SECOND, 7 * SECOND)?);
    assert_eq!(replaced, range(5 * SECOND, 9 * SECOND)?);
    let source = session.import.source_segment().clone();
    Ok(build_asr_revision(AsrRevisionRequest {
        session_id: &session.session_id,
        source_id: session.import.source_id(),
        source_segment: &source,
        number: NonZeroU32::new(2).ok_or("zero")?,
        transcription: heard(
            &source,
            replaced,
            spoken("Dialog R-17 is displayed now.", 250, 3_400)?,
        )?,
        splice: Some(RevisionSplice {
            base: &session.import,
            replaced_range: replaced,
        }),
    })?)
}

/// T-06/D3: the new revision is the default read, the import stays readable
/// by identity, both records travel into the bundle, and the bundle validates.
#[tokio::test]
async fn spliced_revisions_are_committed_read_by_identity_and_retained() -> TestResult {
    let session = imported_session().await?;
    let second = spliced_over_import(&session)?;
    let carried: Vec<_> = second
        .segments()
        .iter()
        .filter_map(|segment| segment.carried_from().map(|from| from.segment().clone()))
        .collect();
    assert_eq!(
        carried,
        [
            session.import.segments()[0].id().clone(),
            session.import.segments()[2].id().clone()
        ]
    );
    session.store.publish_artifact(
        &session.session_id,
        &OperationId::parse("op_3333333333333333")?,
        StorageGeneration::from_value(1),
        SessionArtifactKind::TranscriptRecord,
        &encode_transcript_record(&second)?,
        OPENED_AT + 1,
    )?;
    let store = FilesystemSessionStore::open_existing(&session.workspace)?;
    let (newest, _) = store
        .read_transcript(&session.session_id, OPENED_AT + 2)?
        .ok_or("no transcript")?;
    assert_eq!(newest, second);
    let (older, _) = store
        .read_transcript_revision(&session.session_id, session.import.id(), OPENED_AT + 2)?
        .ok_or("import unreadable")?;
    assert_eq!(older, session.import);
    assert_eq!(
        store
            .read_transcript_revision(
                &session.session_id,
                &TranscriptRevisionId::parse("trv_2222222222222222")?,
                OPENED_AT + 2
            )?
            .map(|(revision, _)| revision.number()),
        None
    );
    // A stale generation cannot commit a racing revision.
    assert_eq!(
        store.publish_artifact(
            &session.session_id,
            &OperationId::parse("op_4444444444444444")?,
            StorageGeneration::from_value(1),
            SessionArtifactKind::TranscriptRecord,
            &encode_transcript_record(&second)?,
            OPENED_AT + 2,
        ),
        Err(SessionStorageError::StateConflict)
    );

    let bundle = session.root.0.join("bundle");
    store.retain_bundle(
        &session.session_id,
        &bundle,
        BundleSourcePolicy::EvidenceOnly,
    )?;
    let validated = FilesystemSessionStore::validate_bundle(&bundle)?;
    assert_eq!(validated.artifact_count(), 2);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    for artifact in manifest["artifacts"].as_array().ok_or("no artifacts")? {
        let name = artifact["name"].as_str().ok_or("no name")?;
        let record: serde_json::Value = serde_json::from_slice(&fs::read(bundle.join(name))?)?;
        assert!(conforms_to_record_schema(&record)?, "{name}");
    }
    Ok(())
}

/// The private work directory holds the session: close is busy while it
/// exists, it is removed on drop, and a leftover from a killed run is swept
/// by the next one while look-alikes are never touched.
#[tokio::test]
async fn work_directories_hold_the_session_and_are_removed() -> TestResult {
    let session = imported_session().await?;
    let work = session
        .store
        .session_work_directory(&session.session_id, OPENED_AT + 1)?;
    let path = work.path().to_path_buf();
    assert!(path.is_dir());
    assert!(path.starts_with(fs::canonicalize(&session.workspace)?));
    let generation = session
        .store
        .session_status(&session.session_id)?
        .generation();
    assert_eq!(
        session.store.close_session(
            &session.session_id,
            &OperationId::parse("op_5555555555555555")?,
            generation
        ),
        Err(SessionStorageError::Busy)
    );
    drop(work);
    assert!(!path.exists());

    let area = path.parent().ok_or("no work area")?.to_path_buf();
    let leftover = area.join("asr-0123456789abcdef");
    fs::create_dir(&leftover)?;
    fs::write(leftover.join("work.lock"), b"")?;
    fs::write(leftover.join("chunk-00000.wav"), b"RIFF")?;
    let look_alike = area.join("asr-not-a-work-directory");
    fs::create_dir(&look_alike)?;
    let next = session
        .store
        .session_work_directory(&session.session_id, OPENED_AT + 1)?;
    assert!(!leftover.exists());
    assert!(look_alike.is_dir());
    drop(next);

    // A closed session has no work directory.
    session.store.close_session(
        &session.session_id,
        &OperationId::parse("op_6666666666666666")?,
        generation,
    )?;
    assert_eq!(
        session
            .store
            .session_work_directory(&session.session_id, OPENED_AT + 1)
            .err(),
        Some(SessionStorageError::StateConflict)
    );
    Ok(())
}

/// Later operations reopen the committed private copy, never the original,
/// and refuse a copy that no longer matches its identity.
#[tokio::test]
async fn the_committed_source_copy_is_reopened_and_rechecked() -> TestResult {
    let session = imported_session().await?;
    let reopened = SourceSnapshot::open_committed(&session.store, &session.session_id, OPENED_AT)?;
    assert_eq!(reopened.id(), session.import.source_id());
    let copy = reopened.provider_path();
    drop(reopened);
    let mut bytes = fs::read(&copy)?;
    bytes.push(0);
    fs::write(&copy, &bytes)?;
    assert!(matches!(
        SourceSnapshot::open_committed(&session.store, &session.session_id, OPENED_AT),
        Err(SourceError::SnapshotChanged)
    ));
    Ok(())
}
