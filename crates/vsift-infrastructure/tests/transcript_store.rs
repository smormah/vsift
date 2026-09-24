//! Transcript revisions committed through session generations, read back,
//! retained in bundles and protected by integrity checks.

use std::{
    env,
    error::Error,
    fs,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use vsift_application::{
    ForegroundSessionPort, ImportedRevisionRequest, InitializeSessionStorage,
    InitializeSessionStorageRequest, OpenSessionError, SessionStorageError,
    build_imported_revision,
};
use vsift_domain::{
    DurabilityRequirement, MediaTime, OperationId, SessionId, SourceId, StorageGeneration,
    TranscriptOffset, TranscriptRevision,
};
use vsift_infrastructure::{
    BundleSourcePolicy, FilesystemSessionStore, SourceSnapshot, decode_transcript_record,
    encode_transcript_record, read_supplied_transcript,
};

type TestResult = Result<(), Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-transcript-store-test-";
const OPENED_AT: u64 = 1_000;

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

fn f10_sidecar() -> PathBuf {
    repository("fixtures/corpus/transcripts/F10.vtt")
}

fn load_json(relative: &str) -> Result<serde_json::Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&fs::read(repository(relative))?)?)
}

/// Whether `instance` conforms to the published bundle transcript record schema.
fn conforms_to_record_schema(instance: &serde_json::Value) -> Result<bool, Box<dyn Error>> {
    let schema = load_json("schemas/v1/bundle-transcript-record.schema.json")?;
    Ok(jsonschema::validator_for(&schema)?.is_valid(instance))
}

/// The one transcript record artifact of a retained bundle: its file name and
/// its content as JSON.
fn bundle_record(bundle: &Path) -> Result<(String, serde_json::Value), Box<dyn Error>> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    let artifact = &manifest["artifacts"][0];
    if artifact["kind"] != "transcript_record" {
        return Err("the bundle's first artifact is not a transcript record".into());
    }
    let name = artifact["name"]
        .as_str()
        .ok_or("artifact name missing")?
        .to_owned();
    let record = serde_json::from_slice(&fs::read(bundle.join(&name))?)?;
    Ok((name, record))
}

/// Replaces the bundle's transcript record with `record` and rewrites the
/// manifest so every recorded name, size and digest matches the new bytes:
/// only the record's content can then make validation fail.
fn rewrite_bundle_record(bundle: &Path, old_name: &str, record: &serde_json::Value) -> TestResult {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let bytes = serde_json::to_vec(record)?;
    let sha256: String = Sha256::digest(&bytes)
        .iter()
        .flat_map(|byte| {
            [
                char::from(HEX_DIGITS[usize::from(byte >> 4)]),
                char::from(HEX_DIGITS[usize::from(byte & 0x0f)]),
            ]
        })
        .collect();
    let name = format!("artifact-{sha256}.json");
    fs::remove_file(bundle.join(old_name))?;
    fs::write(bundle.join(&name), &bytes)?;
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    manifest["artifacts"][0]["name"] = serde_json::Value::from(name);
    manifest["artifacts"][0]["sha256"] = serde_json::Value::from(sha256);
    manifest["artifacts"][0]["bytes"] = serde_json::Value::from(bytes.len());
    fs::write(bundle.join("bundle.json"), serde_json::to_vec(&manifest)?)?;
    Ok(())
}

/// An initialized session with a staged, not yet activated source.
struct Staged {
    root: OwnedRoot,
    workspace: PathBuf,
    store: FilesystemSessionStore,
    session_id: SessionId,
    snapshot: SourceSnapshot,
}

async fn staged() -> Result<Staged, Box<dyn Error>> {
    let root = OwnedRoot::new()?;
    let source = root.0.join("source.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomtranscript-store")?;
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
    Ok(Staged {
        root,
        workspace,
        store,
        session_id,
        snapshot,
    })
}

fn f10_revision(staged: &Staged) -> Result<TranscriptRevision, Box<dyn Error>> {
    let supplied = read_supplied_transcript(&f10_sidecar())?;
    Ok(build_imported_revision(ImportedRevisionRequest {
        session_id: &staged.session_id,
        source_id: staged.snapshot.id(),
        source_duration: MediaTime::from_micros(12_000_000),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?)
}

fn activate(
    staged: &Staged,
    revision: &TranscriptRevision,
) -> Result<StorageGeneration, OpenSessionError> {
    staged.store.activate_with_transcript(
        &staged.snapshot,
        &OperationId::parse("op_2222222222222222").map_err(|_| OpenSessionError::InvalidSource)?,
        StorageGeneration::INITIAL,
        OPENED_AT,
        revision,
    )
}

#[tokio::test]
async fn transcript_is_committed_with_activation_read_back_and_retained() -> TestResult {
    let staged = staged().await?;
    let revision = f10_revision(&staged)?;

    let generation = activate(&staged, &revision)?;
    assert_eq!(generation, StorageGeneration::from_value(1));

    let reopened = FilesystemSessionStore::open_existing(&staged.workspace)?;
    let status = reopened.session_status(&staged.session_id)?;
    assert_eq!(status.generation(), generation);
    assert_eq!(status.artifact_count(), 1);
    let (read, read_status) = reopened
        .read_transcript(&staged.session_id, OPENED_AT + 1)?
        .ok_or("transcript missing")?;
    assert_eq!(read, revision);
    assert_eq!(read_status, status);

    let bundle = staged.root.0.join("bundle");
    let retained = reopened.retain_bundle(
        &staged.session_id,
        &bundle,
        BundleSourcePolicy::EvidenceOnly,
    )?;
    assert_eq!(retained.artifact_count(), 1);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("bundle.json"))?)?;
    assert_eq!(manifest["artifacts"][0]["kind"], "transcript_record");
    let name = manifest["artifacts"][0]["name"]
        .as_str()
        .ok_or("artifact name missing")?;
    assert_eq!(
        decode_transcript_record(&fs::read(bundle.join(name))?)?,
        revision
    );
    Ok(())
}

#[tokio::test]
async fn a_changed_transcript_record_is_an_integrity_failure() -> TestResult {
    let staged = staged().await?;
    let revision = f10_revision(&staged)?;
    activate(&staged, &revision)?;
    let artifacts = staged
        .workspace
        .join("sessions")
        .join(staged.session_id.as_str())
        .join("artifacts");
    let record = fs::read_dir(&artifacts)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .ok_or("record missing")?;
    let original = fs::read(&record)?;
    let tampered = String::from_utf8(original)?.replace("Dialog R-17", "Dialog R-18");
    fs::write(&record, tampered)?;

    assert_eq!(
        staged
            .store
            .read_transcript(&staged.session_id, OPENED_AT + 1)
            .map(|found| found.is_some()),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

#[tokio::test]
async fn sessions_without_transcripts_and_closed_or_expired_sessions_are_distinguished()
-> TestResult {
    let staged = staged().await?;
    let generation = staged.store.activate_source(
        &staged.snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        OPENED_AT,
    )?;
    assert_eq!(
        staged
            .store
            .read_transcript(&staged.session_id, OPENED_AT + 1)
            .map(|found| found.is_some()),
        Ok(false)
    );
    let status = staged.store.session_status(&staged.session_id)?;
    assert_eq!(
        staged
            .store
            .read_transcript(
                &staged.session_id,
                status.lifetime().expires_at_unix_seconds()
            )
            .map(|found| found.is_some()),
        Err(SessionStorageError::StateConflict)
    );
    drop(staged.snapshot);
    staged.store.close_session(
        &staged.session_id,
        &OperationId::parse("op_3333333333333333")?,
        generation,
    )?;
    assert_eq!(
        staged
            .store
            .read_transcript(&staged.session_id, OPENED_AT + 1)
            .map(|found| found.is_some()),
        Err(SessionStorageError::StateConflict)
    );
    Ok(())
}

#[tokio::test]
async fn stored_records_are_strict_and_versioned() -> TestResult {
    let staged = staged().await?;
    let revision = f10_revision(&staged)?;
    let encoded = encode_transcript_record(&revision)?;
    assert_eq!(decode_transcript_record(&encoded)?, revision);
    let value: serde_json::Value = serde_json::from_slice(&encoded)?;

    let mut future = value.clone();
    future["schema_version"] = serde_json::json!(3);
    assert_eq!(
        decode_transcript_record(&serde_json::to_vec(&future)?),
        Err(SessionStorageError::UnsupportedVersion)
    );
    // Version 2 is the local-ASR record; an import's fields do not satisfy it.
    let mut relabelled = value.clone();
    relabelled["schema_version"] = serde_json::json!(2);
    assert_eq!(
        decode_transcript_record(&serde_json::to_vec(&relabelled)?),
        Err(SessionStorageError::IntegrityFailure)
    );
    // Version 1 is the import record: a local-ASR warning kind cannot appear in it.
    let mut asr_warning = value.clone();
    asr_warning["warnings"] = serde_json::json!([
        {"kind": "seam_duplicates_removed", "count": 1, "first_cue": 1}
    ]);
    assert_eq!(
        decode_transcript_record(&serde_json::to_vec(&asr_warning)?),
        Err(SessionStorageError::IntegrityFailure)
    );

    for pointer in ["", "/segments/0", "/source_segment", "/sidecar"] {
        let mut extended = value.clone();
        extended
            .pointer_mut(pointer)
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("not an object")?
            .insert("unreviewed".to_owned(), serde_json::Value::Bool(true));
        assert_eq!(
            decode_transcript_record(&serde_json::to_vec(&extended)?),
            Err(SessionStorageError::IntegrityFailure),
            "{pointer}"
        );
    }

    // A range that is not the cue timing plus the offset is rejected by the domain.
    let mut shifted = value.clone();
    shifted["segments"][0]["start_us"] = serde_json::json!(1_000_001);
    assert_eq!(
        decode_transcript_record(&serde_json::to_vec(&shifted)?),
        Err(SessionStorageError::IntegrityFailure)
    );

    // Imported SRT/VTT segments cannot claim a speaker the format cannot state.
    let mut srt_speaker = value;
    srt_speaker["origin"] = serde_json::json!("imported_srt");
    srt_speaker["segments"][0]["speaker"] = serde_json::json!("Narrator");
    assert_eq!(
        decode_transcript_record(&serde_json::to_vec(&srt_speaker)?),
        Err(SessionStorageError::IntegrityFailure)
    );
    Ok(())
}

/// The session and source identities of the published v1 examples: the F10
/// fixture video, whose real source digest this is.
const EXAMPLE_SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const F10_SOURCE: &str =
    "src_sha256_d7ccece71288c5ff35d7b16c871a93a8ed48bbb7380617899575069b4d6545f4";

#[test]
fn the_frozen_bundle_record_example_is_the_encoded_f10_revision() -> TestResult {
    let supplied = read_supplied_transcript(&repository("fixtures/corpus/transcripts/F10.srt"))?;
    let revision = build_imported_revision(ImportedRevisionRequest {
        session_id: &SessionId::parse(EXAMPLE_SESSION)?,
        source_id: &SourceId::parse(F10_SOURCE)?,
        source_duration: MediaTime::from_micros(12_000_000),
        supplied: &supplied,
        offset: TranscriptOffset::from_micros(500_000)?,
        number: NonZeroU32::MIN,
    })?;
    let encoded: serde_json::Value = serde_json::from_slice(&encode_transcript_record(&revision)?)?;
    let example = load_json("schemas/v1/examples/bundle-transcript-record.json")?;

    assert!(
        encoded == example,
        "the encoded record differs from the frozen example"
    );
    assert!(conforms_to_record_schema(&example)?);
    assert!(
        decode_transcript_record(&serde_json::to_vec(&example)?)? == revision,
        "the frozen example does not decode to the revision"
    );
    Ok(())
}

/// Local ASR changed the revision model; imports must not notice. These
/// identities, record sizes and record digests were produced by the importer
/// at `9dad79e`, before the change, from the same inputs.
#[test]
fn imports_keep_their_identities_and_version_1_record_bytes() -> TestResult {
    let pinned = [
        (
            "fixtures/corpus/transcripts/F10.srt",
            500_000,
            1_304,
            "87d3d79e8dc9b5dff33eb1fc23b6bc9622e31d022f2049f5e0db5bedc9b3685f",
            "trv_663ae41bbedc740b651fe61999d395b0",
            [
                "tsg_e88330d57e140d6e7e2ed4447950c5b8",
                "tsg_f9644c630cd9023d38e2eb89741c9b7e",
                "tsg_a394939677bf53f290007e39739ab6fd",
            ],
        ),
        (
            "fixtures/corpus/transcripts/F10.vtt",
            -250_000,
            1_308,
            "9ba06a100478c6ae55bbc15a9d65caed798bd5f3858878b24e588085019dd1a5",
            "trv_d8bc2bf3ed6ac22bbe54eed00d9e2549",
            [
                "tsg_aa421509341dbf835be55b9475a2d084",
                "tsg_882aa4faec47f96751298cd45cc1570b",
                "tsg_c5ebe6b131ed38f3ab0100ec0e642f02",
            ],
        ),
    ];
    for (sidecar, offset, bytes, digest, revision_id, segment_ids) in pinned {
        let supplied = read_supplied_transcript(&repository(sidecar))?;
        let revision = build_imported_revision(ImportedRevisionRequest {
            session_id: &SessionId::parse(EXAMPLE_SESSION)?,
            source_id: &SourceId::parse(F10_SOURCE)?,
            source_duration: MediaTime::from_micros(12_000_000),
            supplied: &supplied,
            offset: TranscriptOffset::from_micros(offset)?,
            number: NonZeroU32::MIN,
        })?;
        assert_eq!(revision.id().as_str(), revision_id, "{sidecar}");
        assert_eq!(
            revision.source_segment().id().as_str(),
            "sgm_813c2dbd740e3afc0f67a52278553bf6"
        );
        let segments: Vec<&str> = revision
            .segments()
            .iter()
            .map(|segment| segment.id().as_str())
            .collect();
        assert_eq!(segments, segment_ids, "{sidecar}");
        let encoded = encode_transcript_record(&revision)?;
        let encoded_digest: String = Sha256::digest(&encoded)
            .iter()
            .flat_map(|byte| [byte >> 4, byte & 0x0f])
            .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
            .collect();
        assert_eq!((encoded.len(), encoded_digest.as_str()), (bytes, digest));
        assert_eq!(decode_transcript_record(&encoded)?, revision);
    }
    Ok(())
}

#[tokio::test]
async fn a_retained_transcript_record_conforms_to_its_published_schema() -> TestResult {
    let staged = staged().await?;
    let revision = f10_revision(&staged)?;
    activate(&staged, &revision)?;
    let bundle = staged.root.0.join("bundle");
    staged.store.retain_bundle(
        &staged.session_id,
        &bundle,
        BundleSourcePolicy::EvidenceOnly,
    )?;

    let (_, record) = bundle_record(&bundle)?;
    assert!(conforms_to_record_schema(&record)?);
    for pointer in ["", "/segments/0", "/source_segment", "/sidecar"] {
        let mut extended = record.clone();
        extended
            .pointer_mut(pointer)
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("not an object")?
            .insert("unreviewed".to_owned(), serde_json::Value::Bool(true));
        assert!(!conforms_to_record_schema(&extended)?, "{pointer}");
    }
    Ok(())
}

/// `bundle validate` decodes every transcript record: a record whose bytes
/// match a rewritten manifest but whose content does not conform is rejected,
/// while the unmodified record rewritten the same way still validates.
#[tokio::test]
async fn bundle_validation_rejects_a_nonconforming_transcript_record() -> TestResult {
    let staged = staged().await?;
    let revision = f10_revision(&staged)?;
    activate(&staged, &revision)?;
    let bundle = staged.root.0.join("bundle");
    staged.store.retain_bundle(
        &staged.session_id,
        &bundle,
        BundleSourcePolicy::EvidenceOnly,
    )?;
    let (mut name, original) = bundle_record(&bundle)?;

    let mut extended = original.clone();
    extended["segments"][0]["unreviewed"] = serde_json::Value::Bool(true);
    let mut shifted = original.clone();
    shifted["segments"][0]["start_us"] = serde_json::json!(1_000_001);
    let mut foreign_source = original.clone();
    foreign_source["source_id"] = serde_json::Value::from(F10_SOURCE);
    let mut renamed_format = original.clone();
    renamed_format["format"] = serde_json::Value::from("vsift.other_record");
    let mut empty = original.clone();
    empty["segments"] = serde_json::json!([]);
    let mut future = original.clone();
    future["schema_version"] = serde_json::json!(3);

    for (label, record, expected) in [
        (
            "unknown field",
            extended,
            SessionStorageError::IntegrityFailure,
        ),
        (
            "shifted segment",
            shifted,
            SessionStorageError::IntegrityFailure,
        ),
        (
            "foreign source",
            foreign_source,
            SessionStorageError::IntegrityFailure,
        ),
        (
            "other format",
            renamed_format,
            SessionStorageError::IntegrityFailure,
        ),
        ("no segments", empty, SessionStorageError::IntegrityFailure),
        (
            "future version",
            future,
            SessionStorageError::UnsupportedVersion,
        ),
    ] {
        rewrite_bundle_record(&bundle, &name, &record)?;
        name = bundle_record(&bundle)?.0;
        assert_eq!(
            FilesystemSessionStore::validate_bundle(&bundle).map(|status| status.artifact_count()),
            Err(expected),
            "{label}"
        );
    }

    rewrite_bundle_record(&bundle, &name, &original)?;
    assert_eq!(
        FilesystemSessionStore::validate_bundle(&bundle).map(|status| status.artifact_count()),
        Ok(1)
    );
    Ok(())
}
