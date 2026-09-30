//! The durable workload: opens durable sessions through the engine, commits
//! evidence, runs checkpointed retranscription jobs, renews sessions and
//! runs worker requests (P11 PR 3), and acknowledges every operation that
//! succeeded.
//!
//! The root is a durable worker workspace (ADR 0021 section 3), created by
//! the first run: worker requests run only in one, and every session of a
//! durable workspace is durable, as `IngestRequest`'s durability also asks.
//! A request is acknowledged only once its result is recorded, so its
//! record is part of what a crash must keep.
//!
//! Durable mode is requested at the engine level only (`IngestRequest`'s
//! durability, ADR 0020 D-3); every later commit follows the session's own
//! durability. After each success the workload writes an `ACK` line to its
//! acknowledgement channel (and, for the power-loss layer, records a
//! dm-log-writes mark) before it starts the next operation, so everything
//! it acknowledged is on the host's side of a crash.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use vsift::{
    AdmissionWait, Cancellation, Engine, EngineConfig, EngineError, EnginePorts, HostIsolation,
    IngestRequest, ManagedRootLocation, ProgressObserver, SessionRootError, SessionRootLocation,
    UserConfigurationLocation, WorkRequestRun, WorkspaceInitRequest, WorkspacePolicy,
    WorkspaceRetention,
};
use vsift_application::{
    EvidenceBudget, EvidenceCall, EvidenceScope, FrameAtRequest, JobRequest, JobRunError, JobSpec,
    RecognitionScope, RetranscriptionPorts, RetranscriptionRun, RevisionStore, SessionStorageError,
    TranscribeRangeRequest, extract_frame_at, job_id, recognition_key, retranscribe_operation_key,
    retranscribe_request_digest, retranscription_range, run_retranscription,
    whole_file_source_segment,
};
use vsift_contract::decode_work_request;
use vsift_domain::{
    ChunkPlan, DurabilityRequirement, EvidenceProfile, FailureCode, FrameSelection, FrameTolerance,
    MediaTime, OperationId, PublicationGuarantee, SessionId, Sha256Hex, SourceCheck,
    StorageGeneration, TimeRange, TranscriptRevision, plan_chunks,
};
use vsift_infrastructure::{
    EvidenceMediaFile, FilesystemSessionStore, MAX_EVIDENCE_ARTIFACTS, encode_evidence_record,
};

use crate::{
    error::CampaignError,
    protocol::{Ack, OperationKind, RequestAck, parse_events},
    rng::SplitMix64,
    standins::{
        NeverStop, NoBackoff, SPEECH_MICROS, STAND_IN_DIGEST, SourceUnchanged, StandInAudio,
        StandInRecognizer, StandInVideo, VIDEO_FRAMES, recognizer_identity,
    },
    verify::sha256_hex,
};

/// The workload's settings.
#[derive(Clone, Debug)]
pub struct WorkloadConfig {
    /// The session root, on the filesystem under test.
    pub root: PathBuf,
    /// Scratch space off the filesystem under test, for source files.
    pub scratch: PathBuf,
    /// Where protocol lines go: a file or a serial port.
    pub ack_out: PathBuf,
    /// Wait for a serial port to transmit each line before continuing.
    pub drain: bool,
    /// The dm-log-writes device to mark after each acknowledgement.
    pub mark_device: Option<String>,
    /// The `dmsetup` executable.
    pub dmsetup: PathBuf,
    /// Earlier acknowledgements, whose sessions the workload continues.
    pub known_acks: Option<PathBuf>,
    /// Sequence number of the first operation.
    pub first_seq: u64,
    /// Stop after this many operations.
    pub max_ops: Option<u64>,
    /// Stop after this many consecutive failed operations.
    pub stop_after_failures: Option<u32>,
    /// Seed of the operation mix.
    pub seed: u64,
    /// Time the stand-in recognizer takes per chunk.
    pub recognizer_delay: Duration,
    /// A session takes no new work once it has this many generations.
    pub rotate_after: u64,
    /// Source copies are between these sizes in KiB.
    pub source_kib: (u64, u64),
    /// Which operations the mix draws from.
    pub mix: Mix,
}

/// The operations a workload draws from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum Mix {
    /// Ingests, evidence, retranscription jobs and renewals.
    All,
    /// Everything but retranscription jobs, whose many flushes otherwise make
    /// them the operation a randomly timed write error almost always hits
    /// first (layer C alternates the two mixes).
    NoJobs,
}

/// Why one operation did not acknowledge.
enum OperationStop {
    /// `VSift` answered with a typed failure; the operation is not acknowledged.
    Failed(FailureCode),
    /// The harness itself failed.
    Harness(CampaignError),
}

impl From<CampaignError> for OperationStop {
    fn from(value: CampaignError) -> Self {
        Self::Harness(value)
    }
}

fn storage(error: SessionStorageError) -> OperationStop {
    OperationStop::Failed(FailureCode::from(EngineError::Storage(error)))
}

fn engine(error: EngineError) -> OperationStop {
    OperationStop::Failed(FailureCode::from(error))
}

/// The engine's single public mapping of a job failure.
fn job_failure(error: &JobRunError) -> FailureCode {
    FailureCode::from(EngineError::from(error.clone()))
}

/// A session the workload may add to, with its newest known generation.
#[derive(Clone, Debug)]
struct ActiveSession {
    id: SessionId,
    generation: u64,
}

/// The acknowledgement channel.
pub(crate) struct Channel {
    file: File,
    drain: bool,
}

impl Channel {
    pub(crate) fn open(path: &Path, drain: bool) -> Result<Self, CampaignError> {
        let file = OpenOptions::new()
            .append(true)
            .create(!drain)
            .open(path)
            .map_err(CampaignError::io(
                "opening the acknowledgement channel",
                path,
            ))?;
        Ok(Self { file, drain })
    }

    /// Writes one line and, on a serial port, waits until it has left.
    pub(crate) fn line(&mut self, text: &str) -> Result<(), CampaignError> {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(b'\n');
        self.file
            .write_all(&bytes)
            .map_err(CampaignError::io("writing a protocol line", "ack channel"))?;
        if self.drain {
            drain(&self.file)?;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn drain(file: &File) -> Result<(), CampaignError> {
    rustix::termios::tcdrain(file).map_err(|error| CampaignError::Io {
        context: "draining the serial port",
        path: PathBuf::from("ack channel"),
        source: error.into(),
    })
}

#[cfg(not(target_os = "linux"))]
fn drain(_file: &File) -> Result<(), CampaignError> {
    Err(CampaignError::Io {
        context: "draining a serial port",
        path: PathBuf::from("ack channel"),
        source: std::io::ErrorKind::Unsupported.into(),
    })
}

pub(crate) fn unix_seconds() -> Result<u64, CampaignError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CampaignError::Clock)?
        .as_secs())
}

pub(crate) fn unix_nanos() -> Result<u128, CampaignError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CampaignError::Clock)?
        .as_nanos())
}

/// Reads a text file or block device up to its first NUL byte, at most
/// `limit` bytes.
pub(crate) fn read_text(path: &Path, limit: u64) -> Result<String, CampaignError> {
    use std::io::Read as _;
    let file = File::open(path).map_err(CampaignError::io("reading acknowledgements", path))?;
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(CampaignError::io("reading acknowledgements", path))?;
    if let Some(end) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(end);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Most bytes of acknowledgements the workload and verifier read: 64 MiB.
pub const MAX_ACK_BYTES: u64 = 64 * 1024 * 1024;

/// Runs the workload until its operation budget or failure budget is spent.
///
/// # Errors
///
/// A [`CampaignError`] when the harness itself fails; typed `VSift` failures
/// are reported as `FAIL` lines and never stop the workload by themselves.
pub async fn run(config: &WorkloadConfig) -> Result<u64, CampaignError> {
    fs::create_dir_all(&config.scratch)
        .map_err(CampaignError::io("creating scratch space", &config.scratch))?;
    let mut channel = Channel::open(&config.ack_out, config.drain)?;
    let mut active = known_sessions(config)?;
    let engine = Engine::new(
        EngineConfig {
            session_root: SessionRootLocation::Explicit(config.root.clone()),
            user_configuration: UserConfigurationLocation::Explicit(config.scratch.join("config")),
            managed_root: ManagedRootLocation::Explicit(config.scratch.join("managed")),
            host_isolation: HostIsolation::ProcessOnly,
        },
        EnginePorts::system(),
    );
    ensure_workspace(&engine, config)?;
    let mut rng = SplitMix64::new(config.seed);
    let mut failures_in_a_row = 0_u32;
    channel.line(&format!("WORKLOAD-START seq={}", config.first_seq))?;
    let mut done = 0_u64;
    while config.max_ops.is_none_or(|max| done < max) {
        let seq = config.first_seq + done;
        done += 1;
        let (kind, target) = choose(&mut rng, &active, config.mix);
        channel.line(&format!("START {seq} {} {kind}", unix_nanos()?))?;
        let outcome = match (kind, active.get(target)) {
            (OperationKind::Ingest, _) | (_, None) => ingest(&engine, config, &mut rng, seq).await,
            (OperationKind::Evidence, Some(session)) => {
                evidence(config, &session.id, &mut rng, seq).await
            }
            (OperationKind::Retranscribe, Some(session)) => {
                retranscribe(config, &session.id, &mut rng, seq).await
            }
            (OperationKind::Renew, Some(session)) => renew(config, &session.id, &mut rng, seq),
            (OperationKind::Request, _) => request(&engine, config, &mut rng, seq).await,
        };
        match outcome {
            Ok(ack) => {
                failures_in_a_row = 0;
                channel.line(&ack.line())?;
                if let Some(device) = &config.mark_device {
                    mark(&config.dmsetup, device, &ack.mark())?;
                }
                advance(&mut active, &ack, config.rotate_after);
            }
            Err(OperationStop::Failed(code)) => {
                channel.line(&format!(
                    "FAIL {seq} {} {kind} {}",
                    unix_nanos()?,
                    code.identifier()
                ))?;
                failures_in_a_row += 1;
                if config
                    .stop_after_failures
                    .is_some_and(|limit| failures_in_a_row >= limit)
                {
                    break;
                }
            }
            Err(OperationStop::Harness(error)) => return Err(error),
        }
    }
    channel.line(&format!("WORKLOAD-END operations={done}"))?;
    Ok(done)
}

/// The sessions earlier acknowledgements opened that still take work.
fn known_sessions(config: &WorkloadConfig) -> Result<Vec<ActiveSession>, CampaignError> {
    let mut sessions: BTreeMap<SessionId, u64> = BTreeMap::new();
    if let Some(path) = &config.known_acks {
        let events = parse_events(&read_text(path, MAX_ACK_BYTES)?);
        if !events.malformed.is_empty() {
            return Err(CampaignError::MalformedAcks(events.malformed));
        }
        for ack in events.acks {
            let entry = sessions.entry(ack.session).or_insert(0);
            *entry = (*entry).max(ack.generation);
        }
    }
    Ok(sessions
        .into_iter()
        .filter(|(_, generation)| *generation < config.rotate_after)
        .map(|(id, generation)| ActiveSession { id, generation })
        .collect())
}

/// The next operation and the active session it works on: mostly evidence
/// and retranscriptions, some renewals, and a new session now and then
/// (always while fewer than two sessions take work).
fn choose(rng: &mut SplitMix64, active: &[ActiveSession], mix: Mix) -> (OperationKind, usize) {
    let kind = if active.is_empty() || (active.len() < 2 && rng.below(4) == 0) {
        OperationKind::Ingest
    } else {
        match (rng.below(100), mix) {
            (0..6, _) => OperationKind::Ingest,
            (6..12, _) => OperationKind::Request,
            (12..56, _) | (56..84, Mix::NoJobs) => OperationKind::Evidence,
            (56..84, Mix::All) => OperationKind::Retranscribe,
            _ => OperationKind::Renew,
        }
    };
    let target = usize::try_from(rng.below(u64::try_from(active.len()).unwrap_or(0))).unwrap_or(0);
    (kind, target)
}

/// Follows an acknowledgement: its session takes work until it reaches
/// `rotate_after` generations, and at most four sessions take work at once.
fn advance(active: &mut Vec<ActiveSession>, ack: &Ack, rotate_after: u64) {
    match active.iter_mut().find(|session| session.id == ack.session) {
        Some(session) => session.generation = ack.generation,
        None => active.push(ActiveSession {
            id: ack.session.clone(),
            generation: ack.generation,
        }),
    }
    active.retain(|session| session.generation < rotate_after);
    if active.len() > 4 {
        active.remove(0);
    }
}

/// Creates the root as a durable worker workspace when it does not exist
/// yet (the first run of a campaign); later runs find it.
fn ensure_workspace(engine: &Engine, config: &WorkloadConfig) -> Result<(), CampaignError> {
    if config.root.exists() {
        return Ok(());
    }
    let policy = WorkspacePolicy::new(
        DurabilityRequirement::Durable,
        std::num::NonZeroU16::new(4).ok_or(CampaignError::StandIn)?,
        WorkspaceRetention::from_hours(168).map_err(|_| CampaignError::StandIn)?,
    )
    .map_err(|_| CampaignError::StandIn)?;
    engine
        .init_workspace(WorkspaceInitRequest { policy })
        .map_err(|_| CampaignError::NotDurable)?;
    Ok(())
}

/// Records a dm-log-writes mark, so the replay knows where in the write
/// stream the acknowledgement happened.
pub(crate) fn mark(dmsetup: &Path, device: &str, text: &str) -> Result<(), CampaignError> {
    let status = Command::new(dmsetup)
        .args(["message", device, "0", "mark", text])
        .status()
        .map_err(|_| CampaignError::Command {
            program: "dmsetup",
            status: None,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(CampaignError::Command {
            program: "dmsetup",
            status: status.code(),
        })
    }
}

fn operation_id(rng: &mut SplitMix64) -> Result<OperationId, CampaignError> {
    OperationId::parse(format!("op_{:016x}{:016x}", rng.next(), rng.next()))
        .map_err(|_| CampaignError::Identifier)
}

fn open_store(root: &Path) -> Result<FilesystemSessionStore, OperationStop> {
    FilesystemSessionStore::open_existing(root)
        .map_err(|error| engine(EngineError::SessionRoot(SessionRootError::from(error))))
}

/// The committed head's generation and manifest digest, read back after a
/// commit (the workload is the only writer).
fn committed(
    store: &FilesystemSessionStore,
    session: &SessionId,
    generation: StorageGeneration,
) -> Result<(u64, String), OperationStop> {
    let hold = store.acquire_read(session).map_err(storage)?;
    if hold.generation() != generation {
        return Err(OperationStop::Failed(FailureCode::Internal));
    }
    Ok((hold.generation().value(), hold.manifest_sha256().to_owned()))
}

async fn ingest(
    engine_handle: &Engine,
    config: &WorkloadConfig,
    rng: &mut SplitMix64,
    seq: u64,
) -> Result<Ack, OperationStop> {
    let (low, high) = config.source_kib;
    let kib = low + rng.below(high.saturating_sub(low).max(1));
    let mut bytes = b"\0\0\0\x18ftypisomvsift-campaign".to_vec();
    let size = usize::try_from(kib * 1024).map_err(|_| CampaignError::StandIn)?;
    while bytes.len() < size {
        bytes.extend_from_slice(&rng.next().to_le_bytes());
    }
    let source = config.scratch.join(format!("source-{seq}.mp4"));
    fs::write(&source, &bytes).map_err(CampaignError::io("writing a source", &source))?;
    let opened = engine_handle
        .ingest(IngestRequest {
            source: source.clone(),
            transcript: None,
            cancellation: Cancellation::new(),
            durability: DurabilityRequirement::Durable,
        })
        .await;
    let _ = fs::remove_file(&source);
    let opened = opened.map_err(engine)?.session;
    if opened.publication != PublicationGuarantee::OsCrashDurable {
        return Err(CampaignError::NotDurable.into());
    }
    let store = open_store(&config.root)?;
    let (generation, manifest) = committed(&store, &opened.session_id, opened.generation)?;
    let digest = opened
        .source_id
        .as_str()
        .strip_prefix("src_sha256_")
        .ok_or(CampaignError::Identifier)?
        .to_owned();
    Ok(Ack {
        seq,
        unix_ns: unix_nanos()?,
        kind: OperationKind::Ingest,
        session: opened.session_id,
        generation,
        manifest_sha256: manifest,
        artifacts: vec![digest],
        revision: None,
        request: None,
    })
}

/// A worker request: an ingest of a fresh source from the input root,
/// through `Engine::run_work_request`, acknowledged only when it completed
/// and its result is recorded.
async fn request(
    engine_handle: &Engine,
    config: &WorkloadConfig,
    rng: &mut SplitMix64,
    seq: u64,
) -> Result<Ack, OperationStop> {
    let inputs = config.scratch.join("inputs");
    fs::create_dir_all(&inputs).map_err(CampaignError::io("creating the input root", &inputs))?;
    let (low, high) = config.source_kib;
    let kib = low + rng.below(high.saturating_sub(low).max(1));
    let mut bytes = b"\0\0\0\x18ftypisomvsift-campaign-request".to_vec();
    let size = usize::try_from(kib * 1024).map_err(|_| CampaignError::StandIn)?;
    while bytes.len() < size {
        bytes.extend_from_slice(&rng.next().to_le_bytes());
    }
    let name = format!("request-{seq}.mp4");
    let source = inputs.join(&name);
    fs::write(&source, &bytes).map_err(CampaignError::io("writing a source", &source))?;
    let operation = operation_id(rng)?;
    let request = decode_work_request(
        format!(
            r#"{{"schema_version":"1","operation_id":"{}","durability":"durable","target":{{"ingest":{{"source":"{name}","transcript":null}}}},"steps":[]}}"#,
            operation.as_str()
        )
        .as_bytes(),
    )
    .map_err(|_| CampaignError::StandIn)?;
    let outcome = engine_handle
        .run_work_request(WorkRequestRun {
            request,
            input_root: inputs,
            bundle_root: None,
            admission: AdmissionWait::Immediate,
            concurrency: std::num::NonZeroU16::MIN,
            stop: Cancellation::new(),
            cancellation: Cancellation::new(),
            progress: ProgressObserver::none(),
        })
        .await;
    let _ = fs::remove_file(&source);
    if let Some(code) = outcome.result().failure_code() {
        return Err(OperationStop::Failed(code));
    }
    if outcome.unrecorded().is_some() {
        return Err(OperationStop::Failed(FailureCode::StorageIo));
    }
    let store = open_store(&config.root)?;
    let record = store
        .read_worker_request(&operation)
        .map_err(storage)?
        .and_then(|record| record.result)
        .ok_or(OperationStop::Failed(FailureCode::Internal))?;
    let session = record_session(&store, &operation)?;
    let status = store.session_status(&session).map_err(storage)?;
    let (generation, manifest) = committed(&store, &session, status.generation())?;
    let digest = status
        .source_id()
        .as_str()
        .strip_prefix("src_sha256_")
        .ok_or(CampaignError::Identifier)?
        .to_owned();
    Ok(Ack {
        seq,
        unix_ns: unix_nanos()?,
        kind: OperationKind::Request,
        session,
        generation,
        manifest_sha256: manifest,
        artifacts: vec![digest],
        revision: None,
        request: Some(RequestAck {
            operation_id: operation,
            result_sha256: record.sha256().as_str().to_owned(),
        }),
    })
}

/// The session a request's record names.
fn record_session(
    store: &FilesystemSessionStore,
    operation: &OperationId,
) -> Result<SessionId, OperationStop> {
    store
        .read_worker_request(operation)
        .map_err(storage)?
        .and_then(|record| record.session_id)
        .ok_or(OperationStop::Failed(FailureCode::Internal))
}

async fn evidence(
    config: &WorkloadConfig,
    session: &SessionId,
    rng: &mut SplitMix64,
    seq: u64,
) -> Result<Ack, OperationStop> {
    let store = open_store(&config.root)?;
    let now = unix_seconds()?;
    let inventory = store.read_evidence_records(session, now).map_err(storage)?;
    let status = inventory.status().clone();
    let fingerprint = Sha256Hex::parse(STAND_IN_DIGEST).map_err(|_| CampaignError::StandIn)?;
    let known = inventory.known_media().clone();
    let call = EvidenceCall {
        scope: EvidenceScope {
            session_id: session,
            source_id: status.source_id(),
            profile: EvidenceProfile::P09R0,
            tool_fingerprint: Some(&fingerprint),
        },
        source_check: SourceCheck::FullHash,
        budget: EvidenceBudget::per_call(
            inventory.evidence_slots_left().min(MAX_EVIDENCE_ARTIFACTS),
            inventory.bytes_left(),
        ),
        known_media: &known,
        control: &NeverStop,
    };
    let video = StandInVideo::new(status.source_id().as_str())?;
    let pts = rng.below(u64::try_from(VIDEO_FRAMES).unwrap_or(1));
    let extraction = extract_frame_at(
        &call,
        &video,
        FrameAtRequest {
            at: MediaTime::from_micros(pts * 50_000),
            selection: FrameSelection::AtOrAfter,
            tolerance: FrameTolerance::MAX,
            candidate: None,
        },
    )
    .await
    .map_err(CampaignError::Evidence)?;
    let record = encode_evidence_record(&extraction.record).map_err(|_| CampaignError::StandIn)?;
    let media: Vec<EvidenceMediaFile<'_>> = extraction
        .media
        .iter()
        .map(|file| EvidenceMediaFile {
            kind: file.kind,
            bytes: &file.bytes,
        })
        .collect();
    let generation = store
        .publish_evidence(
            session,
            &operation_id(rng)?,
            status.generation(),
            &media,
            &record,
            None,
            now,
        )
        .map_err(storage)?;
    let (generation, manifest) = committed(&store, session, generation)?;
    let mut artifacts: Vec<String> = extraction
        .media
        .iter()
        .map(|file| file.sha256.as_str().to_owned())
        .collect();
    artifacts.push(sha256_hex(&record));
    Ok(Ack {
        seq,
        unix_ns: unix_nanos()?,
        kind: OperationKind::Evidence,
        session: session.clone(),
        generation,
        manifest_sha256: manifest,
        artifacts,
        revision: None,
        request: None,
    })
}

async fn retranscribe(
    config: &WorkloadConfig,
    session: &SessionId,
    rng: &mut SplitMix64,
    seq: u64,
) -> Result<Ack, OperationStop> {
    let store = open_store(&config.root)?;
    let now = unix_seconds()?;
    let status = store.session_status(session).map_err(storage)?;
    let segment =
        whole_file_source_segment(status.source_id(), MediaTime::from_micros(SPEECH_MICROS))
            .map_err(|_| CampaignError::StandIn)?;
    let requested = if rng.below(2) == 0 {
        None
    } else {
        let start = rng.below(50) * 1_000_000;
        let length = (1 + rng.below(10)) * 1_000_000;
        Some(
            TimeRange::new(
                MediaTime::from_micros(start),
                MediaTime::from_micros(start + length),
            )
            .map_err(|_| CampaignError::StandIn)?,
        )
    };
    let head = store.head(session, now).map_err(storage)?;
    let replaced = retranscription_range(head.newest.as_ref(), requested, segment.range());
    let identity = recognizer_identity().map_err(|_| CampaignError::StandIn)?;
    let key = recognition_key(&RecognitionScope {
        session_id: session,
        source_id: status.source_id(),
        audio_stream: 1,
        replaced_range: replaced,
        plan: ChunkPlan::R0,
        recognizer: &identity,
        verification: None,
    })
    .map_err(CampaignError::JobKey)?;
    let operation_key =
        retranscribe_operation_key(&key, head.newest.as_ref().map(TranscriptRevision::id))
            .map_err(CampaignError::JobKey)?;
    let planned =
        plan_chunks(segment.id(), replaced, ChunkPlan::R0).map_err(|_| CampaignError::StandIn)?;
    let spec = JobSpec {
        session_id: session.clone(),
        job_id: job_id(session, &operation_key).map_err(CampaignError::JobKey)?,
        request_digest: retranscribe_request_digest(session, requested)
            .map_err(CampaignError::JobKey)?,
        operation_key,
        recognition_key: key,
        request: JobRequest::Retranscribe { range: requested },
        planned_chunks: Some(u32::try_from(planned.len()).map_err(|_| CampaignError::StandIn)?),
    };
    let outcome = run_retranscription(
        RetranscriptionRun {
            spec: &spec,
            operation_id: None,
            transcribe: TranscribeRangeRequest {
                source_segment: &segment,
                range: replaced,
                plan: ChunkPlan::R0,
                audio_stream: 1,
                expected: &identity,
            },
            source_id: status.source_id(),
            requested,
            base: head.newest.as_ref(),
            observed: head.generation,
            now,
            admission: vsift_domain::AdmissionWait::Immediate,
        },
        RetranscriptionPorts {
            store: &store,
            audio: &StandInAudio,
            recognizer: &StandInRecognizer {
                delay: config.recognizer_delay,
            },
            cancellation: &NeverStop,
            timer: &NoBackoff,
            classify: job_failure,
            progress: &vsift_application::NoProgress,
        },
        &mut SourceUnchanged,
    )
    .await
    .map_err(|error| OperationStop::Failed(job_failure(&error)))?;
    let generation = store.session_status(session).map_err(storage)?.generation();
    let (generation, manifest) = committed(&store, session, generation)?;
    Ok(Ack {
        seq,
        unix_ns: unix_nanos()?,
        kind: OperationKind::Retranscribe,
        session: session.clone(),
        generation,
        manifest_sha256: manifest,
        artifacts: Vec::new(),
        revision: Some(outcome.revision.id().clone()),
        request: None,
    })
}

fn renew(
    config: &WorkloadConfig,
    session: &SessionId,
    rng: &mut SplitMix64,
    seq: u64,
) -> Result<Ack, OperationStop> {
    let store = open_store(&config.root)?;
    let status = store.session_status(session).map_err(storage)?;
    let generation = store
        .renew_session(
            session,
            &operation_id(rng)?,
            status.generation(),
            unix_seconds()?,
        )
        .map_err(storage)?;
    let (generation, manifest) = committed(&store, session, generation)?;
    Ok(Ack {
        seq,
        unix_ns: unix_nanos()?,
        kind: OperationKind::Renew,
        session: session.clone(),
        generation,
        manifest_sha256: manifest,
        artifacts: Vec::new(),
        revision: None,
        request: None,
    })
}
