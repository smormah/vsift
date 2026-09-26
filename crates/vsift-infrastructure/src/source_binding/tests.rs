//! Hash-count regressions for the bracketed source binding (issue #148).
//!
//! They read the snapshot's test-only full-hash counter, which is not part of
//! the public API, so they live beside the implementation.

use std::{
    error::Error,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use vsift_application::{
    InitializeSessionStorage, InitializeSessionStorageRequest, SpeechAudioSource,
    whole_file_source_segment,
};
use vsift_domain::{
    ChunkPlan, DurabilityRequirement, MediaSelection, OperationId, SessionId, StorageGeneration,
    plan_chunks,
};

use super::{BoundSource, EvidenceSourceCheck, SourceBinding};
use crate::{
    ExecutableResolver, FfmpegMedia, FfmpegSpeechAudio, FilesystemSessionStore, HostIsolation,
    MediaProviderConformance, ProcessCancellation, SourceSnapshot,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

const OWNED_PREFIX: &str = "vsift-source-binding-unit-";
const OPENED_AT: u64 = 1_000;
/// A probe plus the chunks of a two-minute run.
const PROVIDER_CALLS: u32 = 1 + 5;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Built<Self> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
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
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(OWNED_PREFIX))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// An open session whose committed source is a copy of `source`.
struct CommittedSession {
    _root: OwnedRoot,
    store: FilesystemSessionStore,
    session_id: SessionId,
}

/// A provisioned root with one initialized session that has no source yet.
async fn initialized_session(root: &OwnedRoot) -> Built<(FilesystemSessionStore, SessionId)> {
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
    Ok((
        FilesystemSessionStore::open_existing(&workspace)?,
        session_id,
    ))
}

async fn committed_session(root: OwnedRoot, source: &Path) -> Built<CommittedSession> {
    let (store, session_id) = initialized_session(&root).await?;
    let snapshot = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_1111111111111111")?,
        source,
    )?;
    store.activate_source(
        &snapshot,
        &OperationId::parse("op_2222222222222222")?,
        StorageGeneration::INITIAL,
        OPENED_AT,
    )?;
    drop(snapshot);
    Ok(CommittedSession {
        _root: root,
        store,
        session_id,
    })
}

async fn placeholder_session() -> Built<CommittedSession> {
    let root = OwnedRoot::new()?;
    let source = root.0.join("source.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomsource-binding-unit")?;
    committed_session(root, &source).await
}

/// Issue #148 (c): a multi-call operation hashes the copy once when it is
/// bound and once before it commits, however many provider calls it makes.
#[tokio::test]
async fn a_bound_operation_hashes_the_copy_exactly_twice() -> TestResult {
    let session = placeholder_session().await?;
    let bound = BoundSource::open_committed(&session.store, &session.session_id, OPENED_AT)?;
    assert_eq!(bound.snapshot().full_hashes(), 1);
    for _ in 0..PROVIDER_CALLS {
        // The check the media adapter makes before every provider call.
        SourceBinding::check_before_provider_call(&bound)?;
    }
    assert_eq!(
        bound.snapshot().full_hashes(),
        1,
        "a call rehashed the copy"
    );
    let released = bound.release_verified()?;
    assert_eq!(released.full_hashes(), 2);
    Ok(())
}

/// The per-call binding single-call operations keep (ADR 0012) still
/// rehashes before every provider call; this is the cost #148 removes from
/// multi-call operations.
#[tokio::test]
async fn the_per_call_binding_still_rehashes_before_every_call() -> TestResult {
    let session = placeholder_session().await?;
    let snapshot = SourceSnapshot::open_committed(&session.store, &session.session_id, OPENED_AT)?;
    assert_eq!(snapshot.full_hashes(), 1);
    for _ in 0..PROVIDER_CALLS {
        SourceBinding::check_before_provider_call(&snapshot)?;
    }
    assert_eq!(snapshot.full_hashes(), 1 + PROVIDER_CALLS);
    Ok(())
}

/// Binding an already staged snapshot is one full hash, like reopening a
/// committed one.
#[tokio::test]
async fn binding_a_staged_snapshot_hashes_it_once() -> TestResult {
    let root = OwnedRoot::new()?;
    let (store, session_id) = initialized_session(&root).await?;
    let source = root.0.join("staged.mp4");
    fs::write(&source, b"\0\0\0\x18ftypisomstaged-copy")?;
    let staged = SourceSnapshot::stage(
        &store,
        &session_id,
        &OperationId::parse("op_3333333333333333")?,
        &source,
    )?;
    assert_eq!(staged.full_hashes(), 0, "staging hashes while copying");
    let bound = BoundSource::bind(staged)?;
    assert_eq!(bound.snapshot().full_hashes(), 1);
    bound.check_identity()?;
    assert_eq!(bound.release_verified()?.full_hashes(), 2);
    Ok(())
}

/// Commits an evidence generation that records `check`'s verified identity.
fn record_identity(
    session: &CommittedSession,
    check: &EvidenceSourceCheck,
    operation: &str,
) -> TestResult {
    let status = session.store.session_status(&session.session_id)?;
    let identity = check.verified_identity().ok_or("no verified identity")?;
    session.store.publish_evidence(
        &session.session_id,
        &OperationId::parse(operation)?,
        status.generation(),
        &[],
        format!("{{\"stand_in_record\":\"{operation}\"}}").as_bytes(),
        Some(identity),
        OPENED_AT,
    )?;
    Ok(())
}

/// ADR 0019 D1: after one full hash is committed with evidence, a later
/// evidence call compares the copy's identity only; a changed identity with
/// the same bytes is hashed in full and proceeds; changed bytes fail.
#[tokio::test]
async fn evidence_calls_hash_the_copy_only_when_its_identity_is_new_or_changed() -> TestResult {
    let session = placeholder_session().await?;
    let (first, check) =
        BoundSource::open_for_evidence(&session.store, &session.session_id, OPENED_AT)?;
    assert_eq!(check.check(), vsift_domain::SourceCheck::FullHash);
    assert_eq!(first.snapshot().full_hashes(), 1);
    let copy = first.snapshot().provider_path();
    drop(first.release_identity_checked()?);
    record_identity(&session, &check, "op_4444444444444444")?;

    // Identity unchanged: no full hash, before or after the provider calls.
    let (bound, check) =
        BoundSource::open_for_evidence(&session.store, &session.session_id, OPENED_AT)?;
    assert_eq!(check, EvidenceSourceCheck::Identity);
    for _ in 0..PROVIDER_CALLS {
        SourceBinding::check_before_provider_call(&bound)?;
    }
    let released = bound.release_identity_checked()?;
    assert_eq!(
        released.full_hashes(),
        0,
        "an identity check read the bytes"
    );
    drop(released);

    // Identity changed, same bytes: one full hash, then the call proceeds and
    // the new identity is recorded.
    let later = SystemTime::now() + std::time::Duration::from_secs(120);
    fs::File::options()
        .write(true)
        .open(&copy)?
        .set_modified(later)?;
    let (bound, check) =
        BoundSource::open_for_evidence(&session.store, &session.session_id, OPENED_AT)?;
    assert_eq!(check.check(), vsift_domain::SourceCheck::FullHash);
    assert_eq!(bound.snapshot().full_hashes(), 1);
    drop(bound.release_identity_checked()?);
    record_identity(&session, &check, "op_5555555555555555")?;
    let (bound, check) =
        BoundSource::open_for_evidence(&session.store, &session.session_id, OPENED_AT)?;
    assert_eq!(check, EvidenceSourceCheck::Identity);
    drop(bound);

    // Bytes changed: the full hash fails and nothing may be committed.
    let mut bytes = fs::read(&copy)?;
    if let Some(last) = bytes.last_mut() {
        *last ^= 0x55;
    }
    fs::write(&copy, &bytes)?;
    assert!(matches!(
        BoundSource::open_for_evidence(&session.store, &session.session_id, OPENED_AT),
        Err(crate::SourceError::SnapshotChanged)
    ));
    Ok(())
}

/// Builds a clip of several R0 chunks by looping a committed speech fixture.
fn looped_clip(ffmpeg: &Path, work: &Path) -> Built<PathBuf> {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus/generated/F05-speech.mp4");
    let clip = work.join("looped.mp4");
    let status = Command::new(ffmpeg)
        .args(["-v", "error", "-n", "-stream_loop", "5", "-i"])
        .arg(&fixture)
        .args(["-c", "copy"])
        .arg(&clip)
        .stdin(Stdio::null())
        .status()?;
    if !status.success() {
        return Err("ffmpeg could not build the looped clip".into());
    }
    Ok(clip)
}

/// Issue #148 (c) through the real adapters: probing and decoding every chunk
/// of a multi-chunk source reads the copy whole exactly twice, where the
/// per-call binding reads it once per call. Prints both timings.
///
/// `VSIFT_TEST_BINDING_CLIP` may name a longer local clip to measure instead.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn a_real_multi_chunk_decode_hashes_the_copy_exactly_twice() -> TestResult {
    let resolver = ExecutableResolver::from_current_path();
    let ffmpeg = resolver.resolve(OsStr::new("ffmpeg"))?;
    let ffprobe = resolver.resolve(OsStr::new("ffprobe"))?;
    let root = OwnedRoot::new()?;
    let clip = match std::env::var_os("VSIFT_TEST_BINDING_CLIP") {
        Some(clip) => PathBuf::from(clip),
        None => looped_clip(ffmpeg.path(), &root.0)?,
    };
    let source_bytes = fs::metadata(&clip)?.len();
    let session = committed_session(root, &clip).await?;
    let media = FfmpegMedia::new(
        MediaProviderConformance::r0(ffmpeg, ffprobe),
        HostIsolation::ProcessOnly,
        &session.store,
    );

    let started = Instant::now();
    let bound = BoundSource::open_committed(&session.store, &session.session_id, OPENED_AT)?;
    let description = media.probe(&bound, ProcessCancellation::new()).await?;
    let stream = description
        .speech_audio_stream()
        .ok_or("clip has no decodable audio")?;
    let source = whole_file_source_segment(bound.snapshot().id(), description.duration)?;
    let chunks = plan_chunks(source.id(), source.range(), ChunkPlan::R0)?;
    let selection = MediaSelection {
        video: None,
        audio: Some(stream),
    };
    let audio = FfmpegSpeechAudio::new(
        &media,
        &bound,
        &description,
        selection,
        ProcessCancellation::new(),
    );
    let mut bound_starts = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        bound_starts.push(audio.speech_pcm(chunk).await?.actual_start);
    }
    drop(audio);
    assert_eq!(
        bound.snapshot().full_hashes(),
        1,
        "a chunk rehashed the copy"
    );
    let released = bound.release_verified()?;
    assert_eq!(released.full_hashes(), 2);
    let bound_elapsed = started.elapsed();
    drop(released);

    let started = Instant::now();
    let snapshot = SourceSnapshot::open_committed(&session.store, &session.session_id, OPENED_AT)?;
    let description = media.probe(&snapshot, ProcessCancellation::new()).await?;
    let mut per_call_starts = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        per_call_starts.push(
            media
                .speech_pcm(
                    &snapshot,
                    &description,
                    selection,
                    chunk.window(),
                    ProcessCancellation::new(),
                )
                .await?
                .actual_start,
        );
    }
    let per_call_elapsed = started.elapsed();
    let per_call_hashes = snapshot.full_hashes();

    assert!(chunks.len() >= 3, "the clip needs several chunks");
    assert_eq!(bound_starts, per_call_starts);
    assert_eq!(
        usize::try_from(per_call_hashes)?,
        1 + 1 + chunks.len(),
        "open, probe and one rehash per chunk"
    );
    println!(
        "source {source_bytes} bytes, {} chunks: bound {} ms (2 full hashes), per-call {} ms ({per_call_hashes} full hashes)",
        chunks.len(),
        bound_elapsed.as_millis(),
        per_call_elapsed.as_millis(),
    );
    Ok(())
}
