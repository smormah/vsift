//! Evidence navigation (P09, ADR 0019): exact frames, their neighbours,
//! bursts, crops and audio clips of a session's video.
//!
//! Every operation follows the same order, which decides what a call costs
//! and what it may fail with:
//!
//! 1. The request is validated. A frame for a visual candidate reads the
//!    session's newest visual index (no tool) for the candidate's
//!    representative time.
//! 2. `FFmpeg` and `FFprobe` are resolved and the automatic media-tool
//!    preflight runs (no process when a pass is on record); the provider
//!    fingerprint is computed from the tools' on-disk identity (no process).
//! 3. The session's source copy is bound (ADR 0019 D1): an identity
//!    comparison when the session records a verified identity the copy still
//!    has, otherwise one full SHA-256 (a copy whose bytes changed is
//!    `INTEGRITY_FAILURE`).
//! 4. The session's evidence records are read and the request key derived.
//!    A complete record with the same key is returned as `reused` after each
//!    of its files is verified again (INV-02): no provider runs and nothing
//!    is written (V-08).
//! 5. Otherwise the copy is probed, the frames or clip are extracted within
//!    the call's budgets, the copy's identity is compared a last time, and
//!    the new files and the call's record are committed in one generation.
//!    A call that stopped on a budget, the deadline or a cancellation after
//!    extracting something commits that and is `partial`.
//!
//! Files are delivered as absolute paths of the committed session artifacts
//! (D2), valid while the session exists; records never hold a path.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use vsift_application::{
    CropRequest, EvidenceBudget, EvidenceCall, EvidenceControl, EvidenceError, EvidenceExtraction,
    EvidenceMediaError, EvidenceScope, EvidenceStop, FrameAtRequest, SessionStorageError,
    evidence_request_key, extract_audio, extract_burst, extract_crop, extract_frame_at,
    extract_neighbours, find_evidence_item, find_reusable_record,
};
use vsift_domain::{
    AudioRange, BurstCount, BurstRange, CropRect, EvidenceId, EvidenceItem, EvidenceMediaKind,
    EvidenceProfile, EvidenceRecord, EvidenceRequest, EvidenceSubject, FrameSelection,
    FrameTolerance, MediaTime, NeighbourCount, SessionId, Sha256Hex, TimeRange, VisualCandidateId,
    VisualStreamError,
};
use vsift_infrastructure::{
    BoundSource, EvidenceInventory, EvidenceMediaFile, EvidenceSourceCheck, FfmpegAudioExtractor,
    FfmpegFrameExtractor, FfmpegMedia, FilesystemSessionStore, MediaProviderConformance,
    MediaToolVerificationAuthority, ProcessCancellation, SourceError, encode_evidence_record,
    media_tool_fingerprint, reviewed_compatibility_policy,
};

use crate::{
    asr::{open_status, probe_error, snapshot_storage_error},
    engine::Engine,
    error::{EngineError, SessionRootError},
    sessions::SessionSnapshot,
    verification::Cancellation,
};

/// How long one evidence call may run its providers (ADR 0019).
const CALL_DEADLINE: Duration = Duration::from_secs(120);
/// How often a commit that lost a race with another writer is retried.
const MAX_COMMIT_ATTEMPTS: usize = 3;
/// Burst frames when a request names no count (ADR 0019 D6).
pub const DEFAULT_BURST_FRAMES: u8 = 12;
/// Neighbours per side when a request names no count.
pub const DEFAULT_NEIGHBOUR_COUNT: u8 = 1;

/// Which frame a `frame get` names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameTarget {
    /// The frame a time names under a selection policy.
    At {
        /// Requested normalized time in microseconds.
        at_micros: u64,
        /// Selection policy; at-or-after by default (ADR 0019 D3).
        selection: FrameSelection,
        /// How far the frame may lie from the time, at most ten seconds;
        /// `None` allows the full ten seconds.
        tolerance_micros: Option<u64>,
    },
    /// A visual candidate's frame: its representative time, at or after with
    /// tolerance zero. A frame that is not exactly there means the index and
    /// the source disagree.
    Candidate(VisualCandidateId),
}

/// A request for one frame.
#[derive(Clone, Debug)]
pub struct FrameGetRequest {
    /// Session whose video is read.
    pub session: SessionId,
    /// The frame.
    pub target: FrameTarget,
    /// Stops the call at its next provider run.
    pub cancellation: Cancellation,
}

/// A request for the frames around an earlier frame item.
#[derive(Clone, Debug)]
pub struct FrameNeighboursRequest {
    /// Session whose video is read.
    pub session: SessionId,
    /// The frame item to take neighbours around.
    pub anchor: EvidenceId,
    /// Frames per side, 1 through 20; `None` is
    /// [`DEFAULT_NEIGHBOUR_COUNT`].
    pub count: Option<u8>,
    /// Stops the call at its next provider run.
    pub cancellation: Cancellation,
}

/// A request for frames spread evenly over a range.
#[derive(Clone, Debug)]
pub struct FrameBurstRequest {
    /// Session whose video is read.
    pub session: SessionId,
    /// Inclusive start in microseconds.
    pub from_micros: u64,
    /// Exclusive end in microseconds, at most sixty seconds after the start.
    pub to_micros: u64,
    /// Most frames, 1 through 100; `None` is [`DEFAULT_BURST_FRAMES`].
    pub max_frames: Option<u8>,
    /// Stops the call at its next provider run.
    pub cancellation: Cancellation,
}

/// A rectangle in an image's pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CropRectangle {
    /// Left coordinate.
    pub x: u32,
    /// Top coordinate.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// A request for a rectangle of an earlier frame or crop item.
#[derive(Clone, Debug)]
pub struct CropEvidenceRequest {
    /// Session whose video is read.
    pub session: SessionId,
    /// The frame or crop item to cut from.
    pub parent: EvidenceId,
    /// The rectangle, in the parent image's pixels.
    pub rect: CropRectangle,
    /// Stops the call at its next provider run.
    pub cancellation: Cancellation,
}

/// A request for an audio clip.
#[derive(Clone, Debug)]
pub struct AudioClipRequest {
    /// Session whose audio is read.
    pub session: SessionId,
    /// Inclusive start in microseconds.
    pub from_micros: u64,
    /// Exclusive end in microseconds, at most thirty seconds after the start;
    /// a range past the end of the source is clipped to it.
    pub to_micros: u64,
    /// Stops the call at its next provider run.
    pub cancellation: Cancellation,
}

/// One delivered evidence file (ADR 0019 D2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceFile {
    evidence_id: EvidenceId,
    kind: EvidenceMediaKind,
    path: PathBuf,
    sha256: Sha256Hex,
    bytes: u64,
}

impl EvidenceFile {
    /// The item the file shows or plays.
    #[must_use]
    pub const fn evidence_id(&self) -> &EvidenceId {
        &self.evidence_id
    }

    /// What the file is.
    #[must_use]
    pub const fn kind(&self) -> EvidenceMediaKind {
        self.kind
    }

    /// The absolute path of the committed session artifact, valid while the
    /// session exists. It is verified when returned and must not be written.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The file's SHA-256.
    #[must_use]
    pub const fn sha256(&self) -> &Sha256Hex {
        &self.sha256
    }

    /// The file's size in bytes.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// The result of an evidence call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceResults {
    session: SessionSnapshot,
    record: EvidenceRecord,
    reused: bool,
    files: Vec<EvidenceFile>,
}

impl EvidenceResults {
    /// Session state observed after the call.
    #[must_use]
    pub const fn session(&self) -> &SessionSnapshot {
        &self.session
    }

    /// The call's lineage record: request, selections, items and why it
    /// stopped short, if it did.
    #[must_use]
    pub const fn record(&self) -> &EvidenceRecord {
        &self.record
    }

    /// Whether the record was committed by an earlier call with the same
    /// request key and returned without running a provider.
    #[must_use]
    pub const fn reused(&self) -> bool {
        self.reused
    }

    /// One verified file per distinct item, in the record's item order.
    #[must_use]
    pub fn files(&self) -> &[EvidenceFile] {
        &self.files
    }
}

/// What an evidence call needs once its request is validated.
enum Operation {
    Frame(FrameAtRequest),
    Neighbours {
        anchor: EvidenceId,
        count: NeighbourCount,
    },
    Burst {
        range: BurstRange,
        max_frames: BurstCount,
    },
    Crop {
        parent: EvidenceId,
        rect: CropRectangle,
    },
    Audio(AudioRange),
}

/// The bound, verified state of a call before anything is extracted.
struct Prepared {
    store: FilesystemSessionStore,
    tools: MediaProviderConformance,
    fingerprint: Option<Sha256Hex>,
    bound: BoundSource,
    check: EvidenceSourceCheck,
    inventory: EvidenceInventory,
}

/// The engine's deadline and cancellation for one call.
struct CallControl {
    deadline: Instant,
    cancellation: ProcessCancellation,
}

impl EvidenceControl for CallControl {
    fn stop(&self) -> Option<EvidenceStop> {
        if self.cancellation.is_cancelled() {
            Some(EvidenceStop::Cancelled)
        } else if Instant::now() >= self.deadline {
            Some(EvidenceStop::DeadlineExceeded)
        } else {
            None
        }
    }
}

impl Engine {
    /// Extracts the frame a time or a visual candidate names (see the module
    /// documentation for the order of steps).
    ///
    /// # Errors
    ///
    /// Fails for a tolerance over ten seconds, a candidate the session's
    /// index does not hold, a missing root or session, a closed or expired
    /// session, missing or failing media tools, a changed source copy, a
    /// source without a decodable video stream, a time no frame satisfies
    /// (`FrameNotSelected`), a candidate frame that is not at its time
    /// (`INTEGRITY_FAILURE`), an exhausted evidence budget, a provider
    /// failure with nothing extracted, and a stored record that fails its
    /// integrity checks.
    pub async fn frame_get(
        &self,
        request: FrameGetRequest,
    ) -> Result<EvidenceResults, EngineError> {
        let (store, now) = self.evidence_store()?;
        let frame = match request.target {
            FrameTarget::At {
                at_micros,
                selection,
                tolerance_micros,
            } => FrameAtRequest {
                at: MediaTime::from_micros(at_micros),
                selection,
                tolerance: tolerance_micros
                    .map_or(Ok(FrameTolerance::MAX), FrameTolerance::new)
                    .map_err(EngineError::InvalidNavigation)?,
                candidate: None,
            },
            FrameTarget::Candidate(candidate) => {
                let (index, _) = store
                    .read_visual_index(&request.session, now)?
                    .ok_or(EngineError::CandidateNotFound)?;
                let representative = index
                    .windows()
                    .iter()
                    .flat_map(vsift_domain::VisualIndexWindow::candidates)
                    .find(|known| *known.id() == candidate)
                    .ok_or(EngineError::CandidateNotFound)?
                    .representative();
                FrameAtRequest {
                    at: representative,
                    selection: FrameSelection::AtOrAfter,
                    tolerance: FrameTolerance::EXACT,
                    candidate: Some(candidate),
                }
            }
        };
        self.run_evidence(
            store,
            &request.session,
            Operation::Frame(frame),
            &request.cancellation,
        )
        .await
    }

    /// Extracts up to `count` consecutive frames on each side of an earlier
    /// frame item.
    ///
    /// # Errors
    ///
    /// As [`Engine::frame_get`], and for a count outside 1 through 20, an
    /// anchor the session does not hold (`EvidenceNotFound`) or that is not a
    /// whole frame (`EvidenceKindMismatch`).
    pub async fn frame_neighbours(
        &self,
        request: FrameNeighboursRequest,
    ) -> Result<EvidenceResults, EngineError> {
        let count = NeighbourCount::new(request.count.unwrap_or(DEFAULT_NEIGHBOUR_COUNT))
            .map_err(EngineError::InvalidNavigation)?;
        let (store, _) = self.evidence_store()?;
        self.run_evidence(
            store,
            &request.session,
            Operation::Neighbours {
                anchor: request.anchor,
                count,
            },
            &request.cancellation,
        )
        .await
    }

    /// Extracts the distinct frames up to `max_frames` evenly spaced targets
    /// over a range name.
    ///
    /// # Errors
    ///
    /// As [`Engine::frame_get`], and for an empty or reversed range, a range
    /// over sixty seconds (`NavigationRangeTooLong`) and a count outside 1
    /// through 100.
    pub async fn frame_burst(
        &self,
        request: FrameBurstRequest,
    ) -> Result<EvidenceResults, EngineError> {
        let range = requested_range(request.from_micros, request.to_micros)?;
        let range = BurstRange::new(range).map_err(|_| EngineError::NavigationRangeTooLong)?;
        let max_frames = BurstCount::new(request.max_frames.unwrap_or(DEFAULT_BURST_FRAMES))
            .map_err(EngineError::InvalidNavigation)?;
        let (store, _) = self.evidence_store()?;
        self.run_evidence(
            store,
            &request.session,
            Operation::Burst { range, max_frames },
            &request.cancellation,
        )
        .await
    }

    /// Extracts a rectangle of an earlier frame or crop item by decoding the
    /// frame again (ADR 0019 D7).
    ///
    /// # Errors
    ///
    /// As [`Engine::frame_get`], and for a parent the session does not hold
    /// (`EvidenceNotFound`), an audio parent (`EvidenceKindMismatch`) and a
    /// rectangle outside the parent image (`CropOutsideParent`).
    pub async fn crop(&self, request: CropEvidenceRequest) -> Result<EvidenceResults, EngineError> {
        let (store, _) = self.evidence_store()?;
        self.run_evidence(
            store,
            &request.session,
            Operation::Crop {
                parent: request.parent,
                rect: request.rect,
            },
            &request.cancellation,
        )
        .await
    }

    /// Extracts a WAV clip (16 kHz mono signed 16-bit) of at most thirty
    /// seconds, clipped to the source.
    ///
    /// # Errors
    ///
    /// As [`Engine::frame_get`], and for an empty or reversed range, a range
    /// over thirty seconds (`NavigationRangeTooLong`), a range that starts at
    /// or after the end of the source and a source without an audio stream
    /// (`NoAudioStream`).
    pub async fn audio(&self, request: AudioClipRequest) -> Result<EvidenceResults, EngineError> {
        let range = requested_range(request.from_micros, request.to_micros)?;
        let range = AudioRange::new(range).map_err(|_| EngineError::NavigationRangeTooLong)?;
        let (store, _) = self.evidence_store()?;
        self.run_evidence(
            store,
            &request.session,
            Operation::Audio(range),
            &request.cancellation,
        )
        .await
    }

    fn evidence_store(&self) -> Result<(FilesystemSessionStore, u64), EngineError> {
        let (store, now) = self.existing_store()?;
        Ok((
            store.ok_or(EngineError::SessionRoot(SessionRootError::Missing))?,
            now,
        ))
    }

    /// Steps 2-5 of the module documentation.
    async fn run_evidence(
        &self,
        store: FilesystemSessionStore,
        session: &SessionId,
        operation: Operation,
        cancellation: &Cancellation,
    ) -> Result<EvidenceResults, EngineError> {
        let prepared = self.prepare_evidence(store, session, cancellation).await?;
        let parent = match &operation {
            Operation::Neighbours { anchor: id, .. } | Operation::Crop { parent: id, .. } => Some(
                find_evidence_item(prepared.inventory.records(), id)
                    .ok_or(EngineError::EvidenceNotFound)?
                    .clone(),
            ),
            Operation::Frame(_) | Operation::Burst { .. } | Operation::Audio(_) => None,
        };
        let request = domain_request(&operation, parent.as_ref())?;
        let scope = EvidenceScope {
            session_id: session,
            source_id: prepared.inventory.status().source_id(),
            profile: EvidenceProfile::P09R0,
            tool_fingerprint: prepared.fingerprint.as_ref(),
        };
        let key =
            evidence_request_key(&scope, &request).map_err(|_| EngineError::EvidenceAssembly)?;
        if let Some(record) = find_reusable_record(
            prepared.inventory.records(),
            &key,
            prepared.fingerprint.as_ref(),
        ) {
            let now = self.now_unix_seconds()?;
            let files = delivered_files(&prepared.store, session, record, now)?;
            return Ok(EvidenceResults {
                session: SessionSnapshot::observe(prepared.inventory.status(), now),
                record: record.clone(),
                reused: true,
                files,
            });
        }
        let slots = prepared.inventory.evidence_slots_left();
        if slots < 2 {
            return Err(EngineError::EvidenceBudgetExhausted);
        }
        self.extract_and_commit(prepared, session, operation, parent.as_ref(), cancellation)
            .await
    }

    /// Steps 2-4: tools, preflight, fingerprint, source binding, records.
    async fn prepare_evidence(
        &self,
        store: FilesystemSessionStore,
        session: &SessionId,
        cancellation: &Cancellation,
    ) -> Result<Prepared, EngineError> {
        let now = self.now_unix_seconds()?;
        open_status(&store, session, now)?;
        let tools = self.media_tools()?;
        self.ensure_media_tools_verified_with(&tools, &cancellation.0)
            .await?;
        let fingerprint = self.evidence_tool_fingerprint(&tools);
        let (bound, check) = BoundSource::open_for_evidence(&store, session, now)
            .map_err(|error| EngineError::Storage(snapshot_storage_error(&error)))?;
        let inventory = store.read_evidence_records(session, now)?;
        Ok(Prepared {
            store,
            tools,
            fingerprint,
            bound,
            check,
            inventory,
        })
    }

    /// The media provider's fingerprint, as the preflight keys it: the
    /// executables' identity, the adapter, the reviewed policy, the host
    /// isolation and the verifying authority. `None` when an executable's
    /// identity cannot be read; nothing is then reused.
    fn evidence_tool_fingerprint(&self, tools: &MediaProviderConformance) -> Option<Sha256Hex> {
        let policy = reviewed_compatibility_policy().ok()?;
        let authority = if self.host_media_tool_verifier().is_some() {
            MediaToolVerificationAuthority::HostSupplied
        } else {
            MediaToolVerificationAuthority::ReviewedFixture
        };
        let fingerprint = media_tool_fingerprint(
            tools,
            self.config().host_isolation.into_infrastructure(),
            &policy,
            authority,
        )?;
        let mut text = String::with_capacity(64);
        for byte in fingerprint.digest() {
            text.push(char::from(HEX[usize::from(byte >> 4)]));
            text.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        Sha256Hex::parse(text).ok()
    }

    /// Step 5: probe, extract, verify the copy's identity, commit.
    #[allow(
        clippy::too_many_lines,
        reason = "The stage order is the contract; keep it visible in one place"
    )]
    async fn extract_and_commit(
        &self,
        prepared: Prepared,
        session: &SessionId,
        operation: Operation,
        parent: Option<&EvidenceItem>,
        cancellation: &Cancellation,
    ) -> Result<EvidenceResults, EngineError> {
        let Prepared {
            store,
            tools,
            fingerprint,
            bound,
            check,
            inventory,
        } = prepared;
        let process_cancellation = cancellation.0.clone();
        let media = FfmpegMedia::new(
            tools,
            self.config().host_isolation.into_infrastructure(),
            &store,
        );
        let description = match media.probe(&bound, process_cancellation.clone()).await {
            Ok(description) => description,
            Err(_) if bound.check_identity().is_err() => {
                return Err(EngineError::Storage(SessionStorageError::IntegrityFailure));
            }
            Err(error) => return Err(EngineError::SourceProbe(probe_error(&error))),
        };
        let control = CallControl {
            deadline: Instant::now() + CALL_DEADLINE,
            cancellation: process_cancellation.clone(),
        };
        let call = EvidenceCall {
            scope: EvidenceScope {
                session_id: session,
                source_id: inventory.status().source_id(),
                profile: EvidenceProfile::P09R0,
                tool_fingerprint: fingerprint.as_ref(),
            },
            source_check: check.check(),
            budget: EvidenceBudget::per_call(
                inventory.evidence_slots_left(),
                inventory.bytes_left(),
            ),
            known_media: inventory.known_media(),
            control: &control,
        };
        let extraction = match &operation {
            Operation::Audio(range) => {
                let stream = description
                    .speech_audio_stream()
                    .ok_or(EngineError::NoAudioStream)?;
                let extractor = FfmpegAudioExtractor::new(
                    &media,
                    &bound,
                    &description,
                    stream,
                    process_cancellation,
                );
                extract_audio(&call, &extractor, *range).await
            }
            video => {
                let (stream, displayed) =
                    description
                        .visual_video_stream()
                        .map_err(|error| match error {
                            VisualStreamError::Absent => EngineError::NoVideoStream,
                            VisualStreamError::Unsupported => EngineError::UnsupportedVideoStream,
                        })?;
                let extractor = FfmpegFrameExtractor::new(
                    &media,
                    &bound,
                    &description,
                    stream,
                    displayed,
                    process_cancellation,
                )
                .map_err(EngineError::EvidenceMedia)?;
                match (video, parent) {
                    (Operation::Frame(frame), _) => {
                        extract_frame_at(&call, &extractor, frame.clone()).await
                    }
                    (Operation::Neighbours { count, .. }, Some(anchor)) => {
                        extract_neighbours(&call, &extractor, anchor, *count).await
                    }
                    (Operation::Burst { range, max_frames }, _) => {
                        extract_burst(&call, &extractor, *range, *max_frames).await
                    }
                    (Operation::Crop { rect, .. }, Some(parent)) => {
                        extract_crop(
                            &call,
                            &extractor,
                            CropRequest {
                                parent,
                                x: rect.x,
                                y: rect.y,
                                width: rect.width,
                                height: rect.height,
                            },
                        )
                        .await
                    }
                    _ => return Err(EngineError::EvidenceNotFound),
                }
            }
        };
        let extraction = extraction.map_err(|error| {
            if bound.check_identity().is_err() {
                EngineError::Storage(SessionStorageError::IntegrityFailure)
            } else {
                evidence_error(error)
            }
        })?;
        drop(media);
        // The closing identity comparison: nothing is committed unless the
        // copy still has the identity it was bound with. The snapshot keeps
        // the session held until the commit is done.
        let snapshot = bound
            .release_identity_checked()
            .map_err(|error| source_changed(&error))?;
        self.commit_evidence(&store, session, &extraction, &check)?;
        drop(snapshot);
        let now = self.now_unix_seconds()?;
        let files = delivered_files(&store, session, &extraction.record, now)?;
        let status = store.session_status(session)?;
        Ok(EvidenceResults {
            session: SessionSnapshot::observe(&status, now),
            record: extraction.record,
            reused: false,
            files,
        })
    }

    /// Commits an extraction's new files and record in one generation,
    /// merging onto another writer's generation at most three times.
    fn commit_evidence(
        &self,
        store: &FilesystemSessionStore,
        session: &SessionId,
        extraction: &EvidenceExtraction,
        check: &EvidenceSourceCheck,
    ) -> Result<(), EngineError> {
        let record = encode_evidence_record(&extraction.record)?;
        let media: Vec<EvidenceMediaFile<'_>> = extraction
            .media
            .iter()
            .map(|file| EvidenceMediaFile {
                kind: file.kind,
                bytes: &file.bytes,
            })
            .collect();
        for _ in 0..MAX_COMMIT_ATTEMPTS {
            let status = store.session_status(session)?;
            match store.publish_evidence(
                session,
                &self.new_operation_id()?,
                status.generation(),
                &media,
                &record,
                check.verified_identity(),
                self.now_unix_seconds()?,
            ) {
                Ok(_) => return Ok(()),
                // Another writer moved the generation; evidence does not
                // depend on what it committed, so the commit is retried on
                // top of it.
                Err(SessionStorageError::StateConflict) => {
                    open_status(store, session, self.now_unix_seconds()?)?;
                }
                Err(SessionStorageError::CapacityExhausted) => {
                    return Err(EngineError::EvidenceBudgetExhausted);
                }
                Err(error) => return Err(EngineError::Storage(error)),
            }
        }
        Err(EngineError::Storage(SessionStorageError::Busy))
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn requested_range(from_micros: u64, to_micros: u64) -> Result<TimeRange, EngineError> {
    TimeRange::new(
        MediaTime::from_micros(from_micros),
        MediaTime::from_micros(to_micros),
    )
    .map_err(|_| EngineError::InvalidTimeRange)
}

/// The canonical request a key is derived from.
fn domain_request(
    operation: &Operation,
    parent: Option<&EvidenceItem>,
) -> Result<EvidenceRequest, EngineError> {
    Ok(match operation {
        Operation::Frame(frame) => EvidenceRequest::FrameAt {
            at: frame.at,
            selection: frame.selection,
            tolerance: frame.tolerance,
            candidate: frame.candidate.clone(),
        },
        Operation::Neighbours { anchor, count } => {
            if !matches!(
                parent.map(EvidenceItem::subject),
                Some(EvidenceSubject::Frame(_))
            ) {
                return Err(EngineError::EvidenceKindMismatch);
            }
            EvidenceRequest::Neighbours {
                anchor: anchor.clone(),
                count: *count,
            }
        }
        Operation::Burst { range, max_frames } => EvidenceRequest::Burst {
            range: *range,
            max_frames: *max_frames,
        },
        Operation::Crop { parent: id, rect } => {
            let dimensions = parent
                .and_then(|item| item.subject().image_dimensions())
                .ok_or(EngineError::EvidenceKindMismatch)?;
            EvidenceRequest::Crop {
                parent: id.clone(),
                rect: CropRect::new(rect.x, rect.y, rect.width, rect.height, dimensions)
                    .map_err(|_| EngineError::CropOutsideParent)?,
            }
        }
        Operation::Audio(range) => EvidenceRequest::Audio { range: *range },
    })
}

/// Verifies and delivers each distinct item's file (INV-02, D2).
fn delivered_files(
    store: &FilesystemSessionStore,
    session: &SessionId,
    record: &EvidenceRecord,
    now: u64,
) -> Result<Vec<EvidenceFile>, EngineError> {
    record
        .items()
        .iter()
        .map(|item| {
            let media = item.media();
            let path = store.verified_artifact_path(
                session,
                media.kind(),
                media.sha256().as_str(),
                media.bytes(),
                now,
            )?;
            Ok(EvidenceFile {
                evidence_id: item.id().clone(),
                kind: media.kind(),
                path,
                sha256: media.sha256().clone(),
                bytes: media.bytes(),
            })
        })
        .collect()
}

/// A copy that changed during the call is an integrity failure.
const fn source_changed(error: &SourceError) -> EngineError {
    EngineError::Storage(snapshot_storage_error(error))
}

/// Maps an evidence use-case failure to the engine's typed error.
const fn evidence_error(error: EvidenceError) -> EngineError {
    match error {
        EvidenceError::Selection(selection) => EngineError::FrameNotSelected(selection),
        EvidenceError::Media(media) => EngineError::EvidenceMedia(media),
        EvidenceError::Stopped(EvidenceStop::DeadlineExceeded) => {
            EngineError::EvidenceMedia(EvidenceMediaError::Deadline)
        }
        EvidenceError::Stopped(EvidenceStop::Cancelled) => {
            EngineError::EvidenceMedia(EvidenceMediaError::Cancelled)
        }
        EvidenceError::BudgetExhausted => EngineError::EvidenceBudgetExhausted,
        EvidenceError::CandidateMismatch | EvidenceError::ParentMismatch => {
            EngineError::Storage(SessionStorageError::IntegrityFailure)
        }
        EvidenceError::KindMismatch => EngineError::EvidenceKindMismatch,
        EvidenceError::CropOutsideParent => EngineError::CropOutsideParent,
        EvidenceError::RangeOutsideSource => EngineError::RangeOutsideSource,
        EvidenceError::Record(_) | EvidenceError::Identity(_) => EngineError::EvidenceAssembly,
    }
}
