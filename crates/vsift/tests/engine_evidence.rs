//! Evidence navigation through the engine library (P09 PR 2, ADR 0019;
//! V-07 and V-08 at engine level).
//!
//! The default tests run everywhere without `FFmpeg`: the configured "tools"
//! are plain files that fail if the engine ever starts them, a counting test
//! double replaces the reviewed media-tool verifier, and evidence is seeded
//! into the session through the store with the request key and provider
//! fingerprint the engine derives. A repeated request that succeeds therefore
//! proves that warm reuse starts no process, and one that fails with a probe
//! failure proves the engine went on to run a provider.
//!
//! The opt-in tests at the end run real `FFmpeg` and `FFprobe` from `PATH`
//! against the frozen fixture truth that P09 PR 1 recorded:
//!
//! `cargo test -p vsift --locked --test engine_evidence -- --ignored --nocapture`

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    ffi::OsStr,
    fmt::Write as _,
    fs,
    future::{Future, ready},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use vsift::{
    AudioClipRequest, Cancellation, Clock, ClockError, CropEvidenceRequest, CropRectangle, Engine,
    EngineConfig, EngineError, EnginePorts, EvidenceResults, FailureCode, FrameBurstRequest,
    FrameGetRequest, FrameNeighboursRequest, FrameSelection, FrameTarget, HostIsolation,
    IdentifierGenerationError, IdentifierSource, IngestRequest, MediaToolVerification,
    MediaToolVerifier, OperationId, RuntimeDependency, SessionId, SessionRootLocation,
    SessionStorageError, SourceCheck, SourceId, UserConfigurationLocation,
};
use vsift_application::{
    AudioExtractor, EvidenceBudget, EvidenceCall, EvidenceControl, EvidenceExtraction,
    EvidenceMediaError, EvidenceScope, EvidenceStop, ExtractedClip, ExtractedFrame, FrameAtRequest,
    FrameExtractor, VideoStreamFacts, extract_audio, extract_frame_at,
};
use vsift_domain::{
    AudioRange, CropRect, EvidenceMediaKind, EvidenceProfile, EvidenceSubject, FrameDimensions,
    FrameListing, FrameTolerance, ListedFrame, ListingTail, MediaTime, Sha256Hex, TimeBase,
    TimeRange,
};
use vsift_infrastructure::{
    EvidenceMediaFile, ExecutableResolver, FilesystemSessionStore, MAX_EVIDENCE_ARTIFACTS,
    MediaProviderConformance, MediaToolVerificationAuthority, TrustedExecutable,
    encode_evidence_record, media_tool_fingerprint, reviewed_compatibility_policy,
    wav_from_pcm_s16le_mono,
};

type TestResult = Result<(), Box<dyn Error>>;
type Built<T> = Result<T, Box<dyn Error>>;

/// 2027-01-15T08:00:00Z: an arbitrary fixed start for deterministic expiry.
const T0: u64 = 1_800_000_000;
const OWNED_PREFIX: &str = "vsift-engine-evidence-test-";
const FRAME_MICROS: u64 = 50_000;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct FixedClock;

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> Result<u64, ClockError> {
        Ok(T0)
    }
}

#[derive(Clone)]
struct SequentialIdentifiers(Arc<AtomicU64>);

impl IdentifierSource for SequentialIdentifiers {
    fn session_id(&self) -> Result<SessionId, IdentifierGenerationError> {
        SessionId::parse(format!(
            "ses_{:032x}",
            self.0.fetch_add(1, Ordering::SeqCst) + 1
        ))
        .map_err(|_| IdentifierGenerationError::NonCanonical)
    }

    fn operation_id(&self) -> Result<OperationId, IdentifierGenerationError> {
        OperationId::parse(format!(
            "op_{:032x}",
            self.0.fetch_add(1, Ordering::SeqCst) + 1
        ))
        .map_err(|_| IdentifierGenerationError::NonCanonical)
    }
}

/// A verifier double that always passes and counts its runs.
#[derive(Clone)]
struct CountingVerifier(Arc<AtomicUsize>);

impl MediaToolVerifier for CountingVerifier {
    fn verify(&self) -> impl Future<Output = MediaToolVerification> + Send {
        self.0.fetch_add(1, Ordering::SeqCst);
        ready(MediaToolVerification::Verified)
    }
}

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

/// The start of an 8-bit RGB PNG of `width` x `height`, then `marker`.
fn fake_png(width: u32, height: u32, marker: &str) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&13_u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    png.extend_from_slice(marker.as_bytes());
    png
}

fn time_of(pts: i64) -> MediaTime {
    MediaTime::from_micros(u64::try_from(pts).unwrap_or(0) * FRAME_MICROS)
}

/// A 20 fps, 6 s, 64x36 stream standing in for what `FFmpeg` would decode;
/// only used to build the records a real earlier call would have committed.
struct SeedVideo(VideoStreamFacts);

impl SeedVideo {
    fn new() -> Built<Self> {
        Ok(Self(VideoStreamFacts {
            stream_index: 0,
            time_base: TimeBase::new(1, 20)?,
            displayed: FrameDimensions::new(64, 36)?,
            duration: MediaTime::from_micros(6_000_000),
        }))
    }

    fn listing(&self, requested: TimeRange) -> Result<FrameListing, EvidenceMediaError> {
        let (end, tail) = if requested.end() >= self.0.duration {
            (self.0.duration, ListingTail::EndOfStream)
        } else {
            (requested.end(), ListingTail::MoreMayFollow)
        };
        let covered =
            TimeRange::new(requested.start(), end).map_err(|_| EvidenceMediaError::Invalid)?;
        let frames = (0..120_i64)
            .map(|pts| ListedFrame {
                pts,
                time: time_of(pts),
            })
            .filter(|frame| frame.time >= covered.start() && frame.time < covered.end())
            .collect();
        FrameListing::new(covered, frames, tail).map_err(|_| EvidenceMediaError::Invalid)
    }
}

impl FrameExtractor for SeedVideo {
    fn stream(&self) -> VideoStreamFacts {
        self.0
    }

    fn max_frames_per_run(&self) -> usize {
        8
    }

    fn list_frames(
        &self,
        range: TimeRange,
    ) -> impl Future<Output = Result<FrameListing, EvidenceMediaError>> + Send {
        ready(self.listing(range))
    }

    fn frames(
        &self,
        pts: &[i64],
    ) -> impl Future<Output = Result<Vec<ExtractedFrame>, EvidenceMediaError>> + Send {
        ready(Ok(pts
            .iter()
            .map(|value| ExtractedFrame {
                pts: *value,
                time: time_of(*value),
                png: fake_png(64, 36, &format!("frame {value}")),
            })
            .collect()))
    }

    fn crop(
        &self,
        pts: i64,
        rect: CropRect,
    ) -> impl Future<Output = Result<ExtractedFrame, EvidenceMediaError>> + Send {
        ready(Ok(ExtractedFrame {
            pts,
            time: time_of(pts),
            png: fake_png(rect.width(), rect.height(), "crop"),
        }))
    }
}

struct SeedAudio;

impl AudioExtractor for SeedAudio {
    fn stream_index(&self) -> u32 {
        1
    }

    fn duration(&self) -> MediaTime {
        MediaTime::from_micros(6_000_000)
    }

    fn clip(
        &self,
        range: TimeRange,
    ) -> impl Future<Output = Result<ExtractedClip, EvidenceMediaError>> + Send {
        ready(
            wav_from_pcm_s16le_mono(&[0, 0, 1, 0])
                .map(|wav| ExtractedClip {
                    actual_start: range.start(),
                    wav,
                })
                .map_err(|_| EvidenceMediaError::Invalid),
        )
    }
}

struct Never;

impl EvidenceControl for Never {
    fn stop(&self) -> Option<EvidenceStop> {
        None
    }
}

struct Harness {
    root: OwnedRoot,
    engine: Engine,
    verifications: Arc<AtomicUsize>,
    session: SessionId,
    source: SourceId,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl Harness {
    /// An engine with stand-in tools over one session of a stand-in source.
    async fn open() -> Built<Self> {
        let root = OwnedRoot::new()?;
        let verifications = Arc::new(AtomicUsize::new(0));
        let engine = Engine::new(
            EngineConfig {
                session_root: SessionRootLocation::Explicit(root.path("sessions")),
                user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
                host_isolation: HostIsolation::ProcessOnly,
            },
            EnginePorts::new(
                FixedClock,
                SequentialIdentifiers(Arc::new(AtomicU64::new(0))),
            )
            .with_media_tool_verifier(CountingVerifier(verifications.clone())),
        );
        let ffmpeg = root.path("ffmpeg-stand-in.exe");
        let ffprobe = root.path("ffprobe-stand-in.exe");
        fs::write(&ffmpeg, b"not a program: ffmpeg")?;
        fs::write(&ffprobe, b"not a program: ffprobe")?;
        engine.configure_executable(RuntimeDependency::Ffmpeg, &ffmpeg)?;
        engine.configure_executable(RuntimeDependency::Ffprobe, &ffprobe)?;
        let stand_in = root.path("stand-in.mp4");
        fs::write(&stand_in, b"\0\0\0\x18ftypisomengine-evidence")?;
        let opened = engine
            .ingest(IngestRequest {
                source: stand_in,
                transcript: None,
            })
            .await?;
        Ok(Self {
            root,
            engine,
            verifications,
            session: opened.session.session_id,
            source: opened.session.source_id,
            ffmpeg,
            ffprobe,
        })
    }

    fn store(&self) -> Built<FilesystemSessionStore> {
        Ok(FilesystemSessionStore::open_existing(
            self.root.path("sessions"),
        )?)
    }

    /// The fingerprint the engine derives for the configured stand-ins.
    fn fingerprint(&self) -> Built<Sha256Hex> {
        let tools = MediaProviderConformance::r0(
            TrustedExecutable::explicit(&self.ffmpeg)?,
            TrustedExecutable::explicit(&self.ffprobe)?,
        );
        let fingerprint = media_tool_fingerprint(
            &tools,
            vsift_infrastructure::HostIsolation::ProcessOnly,
            &reviewed_compatibility_policy()?,
            MediaToolVerificationAuthority::HostSupplied,
        )
        .ok_or("no fingerprint")?;
        let mut text = String::new();
        for byte in fingerprint.digest() {
            write!(text, "{byte:02x}")?;
        }
        Ok(Sha256Hex::parse(text)?)
    }

    /// Commits what an earlier real call would have: the extraction's files
    /// and record, plus `fillers` extra images.
    fn seed(&self, extraction: &EvidenceExtraction, fillers: usize) -> TestResult {
        let store = self.store()?;
        let status = store.session_status(&self.session)?;
        let filler_bytes: Vec<Vec<u8>> = (0..fillers)
            .map(|number| fake_png(64, 36, &format!("filler {number}")))
            .collect();
        let mut media: Vec<EvidenceMediaFile<'_>> = extraction
            .media
            .iter()
            .map(|file| EvidenceMediaFile {
                kind: file.kind,
                bytes: &file.bytes,
            })
            .collect();
        media.extend(filler_bytes.iter().map(|bytes| EvidenceMediaFile {
            kind: EvidenceMediaKind::FramePng,
            bytes,
        }));
        store.publish_evidence(
            &self.session,
            &OperationId::parse("op_7777777777777777")?,
            status.generation(),
            &media,
            &encode_evidence_record(&extraction.record)?,
            None,
            T0,
        )?;
        Ok(())
    }

    async fn seeded_frame(&self, at_micros: u64) -> Built<EvidenceExtraction> {
        let fingerprint = self.fingerprint()?;
        let known = BTreeSet::new();
        let call = EvidenceCall {
            scope: EvidenceScope {
                session_id: &self.session,
                source_id: &self.source,
                profile: EvidenceProfile::P09R0,
                tool_fingerprint: Some(&fingerprint),
            },
            source_check: SourceCheck::FullHash,
            budget: EvidenceBudget::per_call(MAX_EVIDENCE_ARTIFACTS, u64::MAX),
            known_media: &known,
            control: &Never,
        };
        Ok(extract_frame_at(
            &call,
            &SeedVideo::new()?,
            FrameAtRequest {
                at: MediaTime::from_micros(at_micros),
                selection: FrameSelection::AtOrAfter,
                tolerance: FrameTolerance::MAX,
                candidate: None,
            },
        )
        .await?)
    }

    async fn seeded_audio(&self) -> Built<EvidenceExtraction> {
        let fingerprint = self.fingerprint()?;
        let known = BTreeSet::new();
        let call = EvidenceCall {
            scope: EvidenceScope {
                session_id: &self.session,
                source_id: &self.source,
                profile: EvidenceProfile::P09R0,
                tool_fingerprint: Some(&fingerprint),
            },
            source_check: SourceCheck::FullHash,
            budget: EvidenceBudget::per_call(MAX_EVIDENCE_ARTIFACTS, u64::MAX),
            known_media: &known,
            control: &Never,
        };
        Ok(extract_audio(
            &call,
            &SeedAudio,
            AudioRange::new(TimeRange::new(
                MediaTime::from_micros(0),
                MediaTime::from_micros(1_000_000),
            )?)?,
        )
        .await?)
    }

    fn frame_get(&self, at_micros: u64) -> FrameGetRequest {
        FrameGetRequest {
            session: self.session.clone(),
            target: FrameTarget::At {
                at_micros,
                selection: FrameSelection::AtOrAfter,
                tolerance_micros: None,
            },
            cancellation: Cancellation::new(),
        }
    }

    fn artifact_count(&self) -> Built<usize> {
        Ok(self
            .store()?
            .session_status(&self.session)?
            .artifact_count())
    }

    /// The session's private source copy.
    fn source_copy(&self) -> Built<PathBuf> {
        let artifacts = self
            .root
            .path("sessions")
            .join("sessions")
            .join(self.session.as_str())
            .join("artifacts");
        for entry in fs::read_dir(artifacts)? {
            let path = entry?.path();
            if path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with("source-"))
            {
                return Ok(path);
            }
        }
        Err("no source copy".into())
    }
}

fn is_probe_failure(result: &Result<EvidenceResults, EngineError>) -> bool {
    matches!(result, Err(EngineError::SourceProbe(_)))
}

/// V-08: a repeated request is answered from its committed record without
/// running a provider or writing anything; another request runs a provider.
#[tokio::test]
async fn a_repeated_request_is_answered_from_its_record_without_any_process() -> TestResult {
    let harness = Harness::open().await?;
    let seeded = harness.seeded_frame(1_025_000).await?;
    harness.seed(&seeded, 0)?;
    let before = harness.artifact_count()?;

    let reused = harness
        .engine
        .frame_get(harness.frame_get(1_025_000))
        .await?;
    assert!(reused.reused());
    assert_eq!(reused.record(), &seeded.record);
    assert_eq!(
        harness.artifact_count()?,
        before,
        "a reused call writes nothing"
    );
    let file = reused.files().first().ok_or("no file")?;
    assert!(file.path().is_absolute());
    assert_eq!(
        fs::read(file.path())?,
        seeded.media.first().ok_or("no image")?.bytes
    );
    assert_eq!(harness.verifications.load(Ordering::SeqCst), 1);

    // Another time is another key: the engine goes on to the provider, which
    // the stand-in tools cannot be.
    assert!(is_probe_failure(
        &harness.engine.frame_get(harness.frame_get(2_000_000)).await
    ));
    Ok(())
}

/// V-08: replacing `FFmpeg` is another provider, so the request is not
/// reused (and the new tools run); the earlier item stays readable.
#[tokio::test]
async fn a_replaced_ffmpeg_is_a_new_provider_and_the_old_item_stays_readable() -> TestResult {
    let harness = Harness::open().await?;
    let seeded = harness.seeded_frame(1_025_000).await?;
    harness.seed(&seeded, 0)?;
    fs::write(
        &harness.ffmpeg,
        b"a different ffmpeg build, larger than before",
    )?;

    assert!(is_probe_failure(
        &harness.engine.frame_get(harness.frame_get(1_025_000)).await
    ));
    assert_eq!(
        harness.verifications.load(Ordering::SeqCst),
        1,
        "the replaced tools were verified before use"
    );
    let store = harness.store()?;
    let inventory = store.read_evidence_records(&harness.session, T0)?;
    assert_eq!(inventory.records(), std::slice::from_ref(&seeded.record));
    let item = seeded.record.items().first().ok_or("no item")?;
    store.verified_artifact_path(
        &harness.session,
        item.media().kind(),
        item.media().sha256().as_str(),
        item.media().bytes(),
        T0,
    )?;
    Ok(())
}

/// V-08: a private copy whose bytes changed is an integrity failure, even
/// for a request its record could answer, and nothing is committed.
#[tokio::test]
async fn a_modified_source_copy_is_an_integrity_failure_and_nothing_is_committed() -> TestResult {
    let harness = Harness::open().await?;
    let seeded = harness.seeded_frame(1_025_000).await?;
    harness.seed(&seeded, 0)?;
    let before = harness.artifact_count()?;
    let copy = harness.source_copy()?;
    let mut bytes = fs::read(&copy)?;
    bytes.push(b'!');
    fs::write(&copy, &bytes)?;

    let result = harness.engine.frame_get(harness.frame_get(1_025_000)).await;
    assert_eq!(
        result.as_ref().err().map(EngineError::failure_code),
        Some(FailureCode::IntegrityFailure)
    );
    assert_eq!(
        result.err(),
        Some(EngineError::Storage(SessionStorageError::IntegrityFailure))
    );
    assert_eq!(harness.artifact_count()?, before);
    Ok(())
}

/// ADR 0019 D4: a session without room for a record and one file fails with
/// `RESOURCE_LIMIT` before any provider runs.
#[tokio::test]
async fn an_exhausted_evidence_budget_fails_before_any_process() -> TestResult {
    let harness = Harness::open().await?;
    let seeded = harness.seeded_frame(1_025_000).await?;
    // One image, one record and fillers: one slot left.
    harness.seed(&seeded, MAX_EVIDENCE_ARTIFACTS - 3)?;
    let result = harness.engine.frame_get(harness.frame_get(3_000_000)).await;
    assert_eq!(
        result.as_ref().err(),
        Some(&EngineError::EvidenceBudgetExhausted)
    );
    assert_eq!(
        result.err().map(|error| error.failure_code()),
        Some(FailureCode::ResourceLimit)
    );
    // The committed request is still answered.
    assert!(
        harness
            .engine
            .frame_get(harness.frame_get(1_025_000))
            .await?
            .reused()
    );
    Ok(())
}

/// Requests are validated before any tool, and parents are looked up in the
/// session's records before any provider runs.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "One table of every rejected request and its typed error"
)]
async fn invalid_requests_are_rejected_with_existing_codes() -> TestResult {
    let harness = Harness::open().await?;
    let frame = harness.seeded_frame(1_025_000).await?;
    harness.seed(&frame, 0)?;
    let audio = harness.seeded_audio().await?;
    let store = harness.store()?;
    let status = store.session_status(&harness.session)?;
    store.publish_evidence(
        &harness.session,
        &OperationId::parse("op_8888888888888888")?,
        status.generation(),
        &audio
            .media
            .iter()
            .map(|file| EvidenceMediaFile {
                kind: file.kind,
                bytes: &file.bytes,
            })
            .collect::<Vec<_>>(),
        &encode_evidence_record(&audio.record)?,
        None,
        T0,
    )?;
    let frame_id = frame.record.items().first().ok_or("no frame")?.id().clone();
    let audio_id = audio.record.items().first().ok_or("no clip")?.id().clone();
    let unknown = vsift::EvidenceId::parse("evd_ffffffffffffffffffffffffffffffff")?;
    let session = harness.session.clone();
    let cancellation = Cancellation::new;

    let mut too_tolerant = harness.frame_get(0);
    too_tolerant.target = FrameTarget::At {
        at_micros: 0,
        selection: FrameSelection::DisplayedAt,
        tolerance_micros: Some(10_000_001),
    };
    let outcomes = [
        harness.engine.frame_get(too_tolerant).await.err(),
        harness
            .engine
            .frame_get(FrameGetRequest {
                session: session.clone(),
                target: FrameTarget::Candidate(vsift::VisualCandidateId::parse(
                    "vcd_0123456789abcdef",
                )?),
                cancellation: cancellation(),
            })
            .await
            .err(),
        harness
            .engine
            .frame_burst(FrameBurstRequest {
                session: session.clone(),
                from_micros: 0,
                to_micros: 60_000_001,
                max_frames: None,
                cancellation: cancellation(),
            })
            .await
            .err(),
        harness
            .engine
            .frame_burst(FrameBurstRequest {
                session: session.clone(),
                from_micros: 0,
                to_micros: 1_000_000,
                max_frames: Some(101),
                cancellation: cancellation(),
            })
            .await
            .err(),
        harness
            .engine
            .audio(AudioClipRequest {
                session: session.clone(),
                from_micros: 0,
                to_micros: 30_000_001,
                cancellation: cancellation(),
            })
            .await
            .err(),
        harness
            .engine
            .audio(AudioClipRequest {
                session: session.clone(),
                from_micros: 5,
                to_micros: 5,
                cancellation: cancellation(),
            })
            .await
            .err(),
        harness
            .engine
            .frame_neighbours(FrameNeighboursRequest {
                session: session.clone(),
                anchor: frame_id.clone(),
                count: Some(21),
                cancellation: cancellation(),
            })
            .await
            .err(),
        harness
            .engine
            .frame_neighbours(FrameNeighboursRequest {
                session: session.clone(),
                anchor: unknown.clone(),
                count: None,
                cancellation: cancellation(),
            })
            .await
            .err(),
        harness
            .engine
            .frame_neighbours(FrameNeighboursRequest {
                session: session.clone(),
                anchor: audio_id.clone(),
                count: None,
                cancellation: cancellation(),
            })
            .await
            .err(),
        harness
            .engine
            .crop(CropEvidenceRequest {
                session: session.clone(),
                parent: frame_id.clone(),
                rect: CropRectangle {
                    x: 60,
                    y: 0,
                    width: 5,
                    height: 5,
                },
                cancellation: cancellation(),
            })
            .await
            .err(),
        harness
            .engine
            .crop(CropEvidenceRequest {
                session: session.clone(),
                parent: audio_id,
                rect: CropRectangle {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                cancellation: cancellation(),
            })
            .await
            .err(),
    ];
    let names: Vec<&str> = outcomes
        .iter()
        .map(|outcome| match outcome {
            Some(EngineError::InvalidNavigation(_)) => "invalid_navigation",
            Some(EngineError::CandidateNotFound) => "candidate_not_found",
            Some(EngineError::NavigationRangeTooLong) => "range_too_long",
            Some(EngineError::InvalidTimeRange) => "invalid_time_range",
            Some(EngineError::EvidenceNotFound) => "evidence_not_found",
            Some(EngineError::EvidenceKindMismatch) => "kind_mismatch",
            Some(EngineError::CropOutsideParent) => "crop_outside_parent",
            Some(_) => "other",
            None => "succeeded",
        })
        .collect();
    assert_eq!(
        names,
        [
            "invalid_navigation",
            "candidate_not_found",
            "range_too_long",
            "invalid_navigation",
            "range_too_long",
            "invalid_time_range",
            "invalid_navigation",
            "evidence_not_found",
            "kind_mismatch",
            "crop_outside_parent",
            "kind_mismatch",
        ]
    );
    for outcome in outcomes.iter().flatten() {
        assert_eq!(outcome.failure_code(), FailureCode::InvalidArgument);
    }
    Ok(())
}

// Opt-in real-FFmpeg tests.

fn fixture(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus/generated")
        .join(relative)
}

fn media_tools_on_path() -> bool {
    let resolver = ExecutableResolver::from_current_path();
    resolver.resolve(OsStr::new("ffmpeg")).is_ok()
        && resolver.resolve(OsStr::new("ffprobe")).is_ok()
}

/// An engine with the tools on `PATH` and the reviewed verifier over one
/// session of `source`.
async fn real_session(root: &OwnedRoot, source: &Path) -> Built<(Engine, SessionId)> {
    let engine = Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(root.path("sessions")),
            user_configuration: UserConfigurationLocation::Explicit(root.path("config")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::new(
            FixedClock,
            SequentialIdentifiers(Arc::new(AtomicU64::new(0))),
        ),
    );
    let opened = engine
        .ingest(IngestRequest {
            source: source.to_path_buf(),
            transcript: None,
        })
        .await?;
    Ok((engine, opened.session.session_id))
}

/// The width and height a PNG's `IHDR` states.
fn png_size(path: &Path) -> Built<(u32, u32)> {
    let bytes = fs::read(path)?;
    let width = bytes.get(16..20).ok_or("short png")?;
    let height = bytes.get(20..24).ok_or("short png")?;
    Ok((
        u32::from_be_bytes(width.try_into()?),
        u32::from_be_bytes(height.try_into()?),
    ))
}

fn actual_times(results: &EvidenceResults) -> Vec<u64> {
    results
        .record()
        .selections()
        .iter()
        .map(|selection| selection.timing().actual().as_micros())
        .collect()
}

/// V-01, V-06..V-08 through the engine with `FFmpeg` 9.0 over F01 (20 fps,
/// 1280x720, 6 s): at-or-after selection, one item for two requests naming
/// one frame, warm reuse, D1 identity checks, consecutive neighbours, an even
/// burst, a crop of a crop in source pixels, and a validated bundle.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
#[allow(
    clippy::too_many_lines,
    reason = "One session's journey through every operation, in order"
)]
async fn real_frames_neighbours_bursts_and_crops_follow_the_frozen_truth() -> TestResult {
    if !media_tools_on_path() {
        return Err("FFmpeg and FFprobe must be on PATH".into());
    }
    let root = OwnedRoot::new()?;
    let (engine, session) = real_session(&root, &fixture("F01.mp4")).await?;
    let get = |at_micros| FrameGetRequest {
        session: session.clone(),
        target: FrameTarget::At {
            at_micros,
            selection: FrameSelection::AtOrAfter,
            tolerance_micros: None,
        },
        cancellation: Cancellation::new(),
    };
    let store = FilesystemSessionStore::open_existing(root.path("sessions"))?;
    let count = || -> Built<usize> { Ok(store.session_status(&session)?.artifact_count()) };

    let first = engine.frame_get(get(1_025_000)).await?;
    assert!(!first.reused());
    assert_eq!(actual_times(&first), [1_050_000]);
    let selection = first.record().selections().first().ok_or("no selection")?;
    assert_eq!(selection.timing().delta_micros(), 25_000);
    let item = first.record().items().first().ok_or("no item")?;
    assert_eq!(item.source_check(), SourceCheck::FullHash);
    let file = first.files().first().ok_or("no file")?;
    assert_eq!(png_size(file.path())?, (1_280, 720));
    assert_eq!(count()?, 2, "one image and one record");

    // Another request naming the same frame: one item, no second image; the
    // copy's identity recorded by the first call spares a full hash.
    let second = engine.frame_get(get(1_040_000)).await?;
    assert_eq!(
        second.record().items().first().map(vsift::EvidenceItem::id),
        Some(item.id())
    );
    assert_eq!(
        second
            .record()
            .items()
            .first()
            .map(vsift::EvidenceItem::source_check),
        Some(SourceCheck::Identity)
    );
    assert_eq!(count()?, 3, "only the second record is new");

    // The same request again is reused and writes nothing.
    let again = engine.frame_get(get(1_025_000)).await?;
    assert!(again.reused());
    assert_eq!(count()?, 3);

    let neighbours = engine
        .frame_neighbours(FrameNeighboursRequest {
            session: session.clone(),
            anchor: item.id().clone(),
            count: Some(2),
            cancellation: Cancellation::new(),
        })
        .await?;
    assert_eq!(
        actual_times(&neighbours),
        [950_000, 1_000_000, 1_100_000, 1_150_000]
    );

    let burst = engine
        .frame_burst(FrameBurstRequest {
            session: session.clone(),
            from_micros: 0,
            to_micros: 1_000_000,
            max_frames: Some(4),
            cancellation: Cancellation::new(),
        })
        .await?;
    assert_eq!(actual_times(&burst), [0, 250_000, 500_000, 750_000]);

    let crop = engine
        .crop(CropEvidenceRequest {
            session: session.clone(),
            parent: item.id().clone(),
            rect: CropRectangle {
                x: 100,
                y: 50,
                width: 400,
                height: 200,
            },
            cancellation: Cancellation::new(),
        })
        .await?;
    let outer = crop.record().items().first().ok_or("no crop")?;
    assert_eq!(
        png_size(crop.files().first().ok_or("no file")?.path())?,
        (400, 200)
    );
    let inner = engine
        .crop(CropEvidenceRequest {
            session: session.clone(),
            parent: outer.id().clone(),
            rect: CropRectangle {
                x: 10,
                y: 20,
                width: 30,
                height: 40,
            },
            cancellation: Cancellation::new(),
        })
        .await?;
    let Some(EvidenceSubject::Crop { region, .. }) = inner
        .record()
        .items()
        .first()
        .map(vsift::EvidenceItem::subject)
    else {
        return Err("expected a crop".into());
    };
    assert_eq!((region.frame_rect.x(), region.frame_rect.y()), (110, 70));

    let bundle = root.path("bundle");
    engine.retain_session(&session, &bundle, vsift::SourceRetention::EvidenceOnly)?;
    engine.validate_bundle(&bundle)?;
    Ok(())
}

/// V-07 at engine level: a burst over the whole of F01 with the maximum
/// budget stays within it and reports the distinct frames; an audio clip of
/// the audio-only variant reports its first decoded sample (64 ms, AAC
/// priming) and is clipped to the source.
#[tokio::test]
#[ignore = "requires FFmpeg and FFprobe on PATH"]
async fn real_bursts_stay_in_budget_and_clips_report_their_first_sample() -> TestResult {
    if !media_tools_on_path() {
        return Err("FFmpeg and FFprobe must be on PATH".into());
    }
    let root = OwnedRoot::new()?;
    let (engine, session) = real_session(&root, &fixture("F01.mp4")).await?;
    let burst = engine
        .frame_burst(FrameBurstRequest {
            session: session.clone(),
            from_micros: 0,
            to_micros: 6_000_000,
            max_frames: Some(100),
            cancellation: Cancellation::new(),
        })
        .await?;
    assert_eq!(burst.record().partial(), None);
    let vsift::EvidenceDetail::Burst {
        targets, distinct, ..
    } = burst.record().detail()
    else {
        return Err("expected a burst".into());
    };
    assert_eq!(targets, 100);
    assert_eq!(usize::from(distinct), burst.record().items().len());
    assert!(burst.record().items().len() <= 100);

    let audio_root = OwnedRoot::new()?;
    let (audio_engine, audio_session) =
        real_session(&audio_root, &fixture("F01-audio-only.m4a")).await?;
    let clip = audio_engine
        .audio(AudioClipRequest {
            session: audio_session.clone(),
            from_micros: 0,
            to_micros: 1_000_000,
            cancellation: Cancellation::new(),
        })
        .await?;
    assert_eq!(actual_times(&clip), [64_000]);
    let file = clip.files().first().ok_or("no clip")?;
    let wav = fs::read(file.path())?;
    assert_eq!(wav.get(..4), Some(b"RIFF".as_slice()));
    assert_eq!(wav.get(24..28), Some(16_000_u32.to_le_bytes().as_slice()));
    let video_only = audio_engine
        .frame_get(FrameGetRequest {
            session: audio_session,
            target: FrameTarget::At {
                at_micros: 0,
                selection: FrameSelection::AtOrAfter,
                tolerance_micros: None,
            },
            cancellation: Cancellation::new(),
        })
        .await;
    assert_eq!(video_only.err(), Some(EngineError::NoVideoStream));
    Ok(())
}
