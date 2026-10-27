//! The verifier: holds a session root to every acknowledgement made before
//! a crash, and checks that nothing is half committed.
//!
//! Two independent views are checked:
//!
//! - **on disk**, from the documented layout alone (the commit pointer, the
//!   generation manifests, the artifact files) without the store's code: the
//!   pointer names a manifest whose digest it records, every generation
//!   links to the digest of the one below down to generation 0, every
//!   artifact the head lists is whole, and the session is durable;
//! - **through the store**, as a user would: the session opens, its
//!   evidence records decode, its acknowledged revisions are found and every
//!   job reads (and is reconciled if its owner died);
//! - **against a seal-only run** (ADR 0033): the session's newest revision is
//!   rebuilt in memory from the stand-ins, as a run that publishes only at its
//!   seal makes it, over the range the workload asked for (the whole stand-in
//!   source for a revision published as it goes, the job's own request for a
//!   sealed one), never the range the revision itself declares (RV-1 finding
//!   6). A sealed revision must equal it byte for byte; an open one (rebuilt
//!   from its parts after the crash) must be a prefix of it, with the same
//!   identities, times and text, and nothing of it left to come before the time
//!   it is published until; a stopped one must equal, byte for byte, the
//!   stopped seal of the reference run's own cut after the chunks it reached,
//!   so its published segments, the earlier text after them and its chunk
//!   outcomes are all pinned (RV-1 finding 5). Parts are listed only while a
//!   revision is open, and at most [`MAX_TRANSCRIPT_PARTS`] of them.
//!
//! An acknowledgement is lost when its session is missing or unreadable,
//! the head is below the acknowledged generation, the acknowledged
//! generation's manifest changed, or anything it committed is gone. Any
//! other failure (a broken chain or a damaged file in any session, a job
//! that cannot be read) is damage.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs::File,
    io::Read as _,
    num::NonZeroU32,
    path::Path,
    time::Duration,
};

use serde_json::Value;
use sha2::{Digest as _, Sha256};
use vsift::EngineError;
use vsift_application::{
    AsrFailure, AsrRevisionRequest, AsrTranscription, CheckpointScope, JobStore, NoCheckpoints,
    NoProgress, RevisionSplice, RunObserver, RunSoFar, StoppedStopwatch, TranscribeRangeRequest,
    build_asr_revision, build_open_asr_revision, build_stopped_asr_revision, job_status,
    retranscription_range, transcribe_range_checkpointed, whole_file_source_segment,
};
use vsift_domain::{
    AsrChunkOutcome, ChunkPlan, FailureCode, JobId, MAX_EARLY_AUDIO_MICROS, MediaTime, OperationId,
    PublicationGuarantee, RecognitionKey, RevisionState, SessionId, Sha256Hex, SourceId,
    SourceSegment, TimeRange, TranscriptProvenance, TranscriptRevision,
};
use vsift_infrastructure::{
    FilesystemSessionStore, MAX_TRANSCRIPT_PARTS, encode_transcript_record,
};

use crate::{
    protocol::{Ack, OperationKind},
    standins::{
        NeverStop, SPEECH_MICROS, STAND_IN_DIGEST, StandInAudio, StandInRecognizer,
        recognizer_identity,
    },
};

/// Largest manifest the verifier reads (the store's own bound is 128 KiB).
const MAX_MANIFEST_BYTES: u64 = 128 * 1024;
/// Largest artifact or source copy the verifier hashes.
const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;

/// Why an acknowledgement is not held.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Loss {
    /// The session's directory or commit pointer is gone.
    SessionMissing,
    /// The committed head is below the acknowledged generation.
    HeadBehind {
        /// The head found.
        head: u64,
    },
    /// The acknowledged generation's manifest is missing or different.
    ManifestChanged,
    /// An artifact the operation committed is no longer listed or whole.
    ArtifactMissing(String),
    /// The session is not durable.
    NotDurable,
    /// The store refused the session with this public code.
    StoreRefused(FailureCode),
    /// The acknowledged transcript revision is not found.
    RevisionMissing,
    /// The acknowledged worker request's record is gone or has not ended.
    RequestRecordMissing,
    /// The acknowledged worker request's record holds another result.
    RequestResultChanged,
}

/// Something half committed or damaged, whether acknowledged or not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Damage {
    /// The session root cannot be opened although something was acknowledged.
    RootUnopenable,
    /// A session's chain does not validate on disk.
    Chain {
        /// The session.
        session: String,
        /// The generation where it breaks.
        generation: u64,
        /// What is wrong.
        what: &'static str,
    },
    /// An artifact the head lists is missing, short or changed.
    Artifact {
        /// The session.
        session: String,
        /// The artifact's file name.
        name: String,
    },
    /// The store refuses a session whose chain is whole.
    Session {
        /// The session.
        session: String,
        /// The public code.
        code: FailureCode,
    },
    /// A job cannot be read or reconciled.
    Job {
        /// The session.
        session: String,
        /// The job directory's name.
        job: String,
        /// The public code.
        code: FailureCode,
    },
    /// A worker request record cannot be read.
    Request {
        /// The request's operation id.
        operation: String,
        /// The public code.
        code: FailureCode,
    },
    /// An acknowledged session refused a new commit after the crash.
    Probe {
        /// The session.
        session: String,
        /// The public code.
        code: FailureCode,
    },
    /// A session's newest revision is not what a seal-only run makes, or not
    /// a prefix of it (ADR 0033).
    Revision {
        /// The session.
        session: String,
        /// What is wrong.
        what: &'static str,
    },
}

impl fmt::Display for Damage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootUnopenable => formatter.write_str("root-unopenable"),
            Self::Chain {
                session,
                generation,
                what,
            } => write!(
                formatter,
                "chain session={session} generation={generation} {what}"
            ),
            Self::Artifact { session, name } => {
                write!(formatter, "artifact session={session} name={name}")
            }
            Self::Session { session, code } => {
                write!(
                    formatter,
                    "session session={session} code={}",
                    code.identifier()
                )
            }
            Self::Job { session, job, code } => write!(
                formatter,
                "job session={session} job={job} code={}",
                code.identifier()
            ),
            Self::Probe { session, code } => write!(
                formatter,
                "probe session={session} code={}",
                code.identifier()
            ),
            Self::Request { operation, code } => write!(
                formatter,
                "request operation={operation} code={}",
                code.identifier()
            ),
            Self::Revision { session, what } => {
                write!(formatter, "revision session={session} {what}")
            }
        }
    }
}

/// Commits one renewal to each of the (at most four) most recently
/// acknowledged sessions, so a crash can be seen to leave every session
/// writable, not only readable: leftover staged files, an abandoned manifest
/// or an interrupted job must not block the next commit. Only for a
/// disposable copy of the filesystem.
pub fn probe_writes(root: &Path, acks: &[Ack], now: u64, findings: &mut Findings) {
    let Ok(store) = FilesystemSessionStore::open_existing(root) else {
        return;
    };
    let mut probed: Vec<&SessionId> = Vec::new();
    for ack in acks.iter().rev() {
        if probed.len() == 4 {
            break;
        }
        if probed.contains(&&ack.session) {
            continue;
        }
        probed.push(&ack.session);
        let outcome = store.session_status(&ack.session).and_then(|status| {
            let operation = OperationId::parse(format!("op_{:032x}", ack.seq))
                .map_err(|_| vsift_application::SessionStorageError::Io)?;
            store.renew_session(&ack.session, &operation, status.generation(), now)
        });
        if let Err(error) = outcome {
            findings.damage.push(Damage::Probe {
                session: ack.session.as_str().to_owned(),
                code: code(error),
            });
        }
    }
}

impl fmt::Display for Loss {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionMissing => formatter.write_str("session-missing"),
            Self::HeadBehind { head } => write!(formatter, "head-behind head={head}"),
            Self::ManifestChanged => formatter.write_str("manifest-changed"),
            Self::ArtifactMissing(digest) => write!(formatter, "artifact-missing {digest}"),
            Self::NotDurable => formatter.write_str("not-durable"),
            Self::StoreRefused(code) => write!(formatter, "store-refused {}", code.identifier()),
            Self::RevisionMissing => formatter.write_str("revision-missing"),
            Self::RequestRecordMissing => formatter.write_str("request-record-missing"),
            Self::RequestResultChanged => formatter.write_str("request-result-changed"),
        }
    }
}

/// What one verification found.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Findings {
    /// Lost acknowledgements, by sequence number.
    pub lost: Vec<(u64, Loss)>,
    /// Damage, acknowledged or not.
    pub damage: Vec<Damage>,
    /// Acknowledgements checked.
    pub acks: usize,
    /// Sessions checked on disk.
    pub sessions: usize,
    /// Generations checked on disk.
    pub generations: u64,
    /// Artifact files hashed.
    pub artifacts: usize,
    /// Jobs read through the store.
    pub jobs: usize,
    /// Worker request records read through the store.
    pub requests: usize,
}

impl Findings {
    /// Whether nothing was lost or damaged.
    #[must_use]
    pub fn clean(&self) -> bool {
        self.lost.is_empty() && self.damage.is_empty()
    }

    /// The report lines: one per finding, then a summary.
    #[must_use]
    pub fn lines(&self, prefix: &str) -> Vec<String> {
        let mut lines: Vec<String> = self
            .lost
            .iter()
            .map(|(seq, loss)| format!("{prefix}-LOST seq={seq} {loss}"))
            .chain(
                self.damage
                    .iter()
                    .map(|damage| format!("{prefix}-DAMAGE {damage}")),
            )
            .collect();
        lines.push(format!(
            "{prefix} {} acks={} sessions={} generations={} artifacts={} jobs={} requests={} lost={} damaged={}",
            if self.clean() { "OK" } else { "FAIL" },
            self.acks,
            self.sessions,
            self.generations,
            self.artifacts,
            self.jobs,
            self.requests,
            self.lost.len(),
            self.damage.len()
        ));
        lines
    }
}

/// What the on-disk walk of one session found.
struct SessionOnDisk {
    head: u64,
    /// Manifest digest of every generation, from 0 up.
    manifests: Vec<String>,
    /// Digests of every artifact the head lists, and of the source copy.
    artifacts: BTreeSet<String>,
    durable: bool,
}

/// Verifies `root` against `acks` at time `now`, whose sessions must give
/// `guarantee`: [`PublicationGuarantee::OsCrashDurable`] for the campaign
/// itself; the process-kill tests run on an ordinary root, whose sessions are
/// crash-consistent only.
#[must_use]
pub fn verify(root: &Path, acks: &[Ack], now: u64, guarantee: PublicationGuarantee) -> Findings {
    let mut findings = Findings {
        acks: acks.len(),
        ..Findings::default()
    };
    let sessions_directory = root.join("sessions");
    let mut on_disk: BTreeMap<String, Result<SessionOnDisk, ()>> = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(&sessions_directory) {
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| SessionId::parse(name.as_str()).is_ok())
            .collect();
        names.sort();
        for name in names {
            let walked = walk_session(&sessions_directory.join(&name), &name, &mut findings);
            on_disk.insert(name, walked);
        }
    }
    findings.sessions = on_disk.len();
    let store = if root.exists() {
        FilesystemSessionStore::open_existing(root).ok()
    } else {
        None
    };
    if store.is_none() && (!acks.is_empty() || !on_disk.is_empty()) {
        findings.damage.push(Damage::RootUnopenable);
    }
    for ack in acks {
        if let Some(loss) = check_ack(
            ack,
            on_disk.get(ack.session.as_str()),
            store.as_ref(),
            now,
            guarantee,
        ) {
            findings.lost.push((ack.seq, loss));
        }
    }
    if let Some(store) = &store {
        check_request_records(root, store, &mut findings);
        let acked: BTreeSet<&str> = acks.iter().map(|ack| ack.session.as_str()).collect();
        for (name, walked) in &on_disk {
            let Ok(session) = SessionId::parse(name.as_str()) else {
                continue;
            };
            if walked.is_ok() {
                check_through_store(
                    root,
                    store,
                    &session,
                    acked.contains(name.as_str()),
                    now,
                    &mut findings,
                );
            }
        }
    }
    findings
}

fn code(error: impl Into<EngineError>) -> FailureCode {
    FailureCode::from(error.into())
}

/// The store-level view of one session whose chain is whole on disk.
fn check_through_store(
    root: &Path,
    store: &FilesystemSessionStore,
    session: &SessionId,
    acknowledged: bool,
    now: u64,
    findings: &mut Findings,
) {
    match store.session_status(session) {
        Ok(_) => {
            if let Err(error) = store.read_evidence_records(session, now) {
                findings.damage.push(Damage::Session {
                    session: session.as_str().to_owned(),
                    code: code(error),
                });
            }
            if let Err(what) = check_newest_revision(store, session, now) {
                findings.damage.push(Damage::Revision {
                    session: session.as_str().to_owned(),
                    what,
                });
            }
        }
        // A session whose opening never finished (generation 0 without a
        // source) is refused as a state conflict; only an acknowledged
        // session must open.
        Err(error)
            if acknowledged || error != vsift_application::SessionStorageError::StateConflict =>
        {
            findings.damage.push(Damage::Session {
                session: session.as_str().to_owned(),
                code: code(error),
            });
            return;
        }
        Err(_) => return,
    }
    let jobs_directory = root.join("sessions").join(session.as_str()).join("jobs");
    let Ok(entries) = std::fs::read_dir(&jobs_directory) else {
        return;
    };
    let mut jobs: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| JobId::parse(name.as_str()).is_ok())
        .collect();
    jobs.sort();
    for name in jobs {
        let Ok(job) = JobId::parse(name.as_str()) else {
            continue;
        };
        findings.jobs += 1;
        match job_status(store, session, &job, now) {
            Ok(_) | Err(vsift_application::JobStoreError::NotFound) => {}
            Err(error) => findings.damage.push(Damage::Job {
                session: session.as_str().to_owned(),
                job: name,
                code: code(error),
            }),
        }
    }
    if let Err(error) = store.session_jobs(session) {
        findings.damage.push(Damage::Job {
            session: session.as_str().to_owned(),
            job: "*".to_owned(),
            code: code(error),
        });
    }
}

fn check_ack(
    ack: &Ack,
    on_disk: Option<&Result<SessionOnDisk, ()>>,
    store: Option<&FilesystemSessionStore>,
    now: u64,
    guarantee: PublicationGuarantee,
) -> Option<Loss> {
    let Some(walked) = on_disk else {
        return Some(Loss::SessionMissing);
    };
    let Ok(walked) = walked else {
        return Some(Loss::StoreRefused(FailureCode::IntegrityFailure));
    };
    if guarantee == PublicationGuarantee::OsCrashDurable && !walked.durable {
        return Some(Loss::NotDurable);
    }
    if walked.head < ack.generation {
        return Some(Loss::HeadBehind { head: walked.head });
    }
    let recorded = usize::try_from(ack.generation)
        .ok()
        .and_then(|index| walked.manifests.get(index));
    if recorded != Some(&ack.manifest_sha256) {
        return Some(Loss::ManifestChanged);
    }
    if let Some(missing) = ack
        .artifacts
        .iter()
        .find(|digest| !walked.artifacts.contains(*digest))
    {
        return Some(Loss::ArtifactMissing(missing.clone()));
    }
    let store = store?;
    if let Err(error) = store.session_status(&ack.session) {
        return Some(Loss::StoreRefused(code(error)));
    }
    if matches!(
        ack.kind,
        OperationKind::Retranscribe | OperationKind::Grow | OperationKind::Stop
    ) {
        let revision = ack.revision.as_ref()?;
        match store.read_transcript_revision(&ack.session, revision, now) {
            Ok(Some(_)) => {}
            Ok(None) => return Some(Loss::RevisionMissing),
            Err(error) => return Some(Loss::StoreRefused(code(error))),
        }
    }
    if ack.kind == OperationKind::Part {
        // The part's open revision is still the newest (open, or sealed or
        // stopped with the same identity), or a later revision superseded the
        // sealed or stopped one.
        let revision = ack.revision.as_ref()?;
        let newest = match store.read_transcript_head(&ack.session, now) {
            Ok((newest, _)) => newest,
            Err(error) => return Some(Loss::StoreRefused(code(error))),
        };
        if newest.as_ref().map(TranscriptRevision::id) != Some(revision) {
            match store.read_transcript_revision(&ack.session, revision, now) {
                Ok(Some(_)) => {}
                Ok(None) => return Some(Loss::RevisionMissing),
                Err(error) => return Some(Loss::StoreRefused(code(error))),
            }
        }
    }
    if let Some(request) = &ack.request {
        match store.read_worker_request(&request.operation_id) {
            Ok(Some(record)) => match &record.result {
                Some(result) if result.sha256().as_str() == request.result_sha256 => {}
                Some(_) => return Some(Loss::RequestResultChanged),
                None => return Some(Loss::RequestRecordMissing),
            },
            Ok(None) => return Some(Loss::RequestRecordMissing),
            Err(error) => return Some(Loss::StoreRefused(code(error))),
        }
    }
    None
}

/// Holds a session's newest revision to the seal-only run of the stand-ins
/// (ADR 0033): see the module.
///
/// # Errors
///
/// What is wrong, as a short fixed name.
pub fn check_newest_revision(
    store: &FilesystemSessionStore,
    session: &SessionId,
    now: u64,
) -> Result<(), &'static str> {
    let open = store
        .read_open_transcript(session, now)
        .map_err(|_| "open-unreadable")?;
    let (newest, _) = store
        .read_transcript_head(session, now)
        .map_err(|_| "head-unreadable")?;
    let Some(newest) = newest else {
        return if open.is_some() {
            Err("parts-without-a-revision")
        } else {
            Ok(())
        };
    };
    match (newest.state(), &open) {
        (RevisionState::Open, Some(open)) if open.revision == newest => {
            if usize::try_from(open.parts.get()).map_or(true, |parts| parts > MAX_TRANSCRIPT_PARTS)
            {
                return Err("too-many-parts");
            }
        }
        (RevisionState::Open, _) => return Err("open-not-rebuilt-from-its-parts"),
        (RevisionState::Sealed | RevisionState::Stopped, Some(_)) => {
            return Err("parts-listed-after-the-seal");
        }
        (RevisionState::Sealed | RevisionState::Stopped, None) => {}
    }
    check_revision(store, session, &newest, now)
}

/// Holds `revision`, as the newest revision of `session`, to the seal-only
/// run of the range its workload asked for: see the module.
///
/// # Errors
///
/// What is wrong, as a short fixed name.
pub(crate) fn check_revision(
    store: &FilesystemSessionStore,
    session: &SessionId,
    revision: &TranscriptRevision,
    now: u64,
) -> Result<(), &'static str> {
    let reference = Reference::of(store, session, revision, now)?;
    if revision.id() != reference.sealed.id() {
        return Err("identity-differs");
    }
    match revision.state() {
        RevisionState::Sealed => {
            let stored = encode_transcript_record(revision).map_err(|_| "unencodable")?;
            let expected =
                encode_transcript_record(&reference.sealed).map_err(|_| "unencodable")?;
            if stored != expected {
                return Err("sealed-differs-from-a-seal-only-run");
            }
        }
        RevisionState::Open => {
            let until = revision.published_until().ok_or("open-without-until")?;
            let count = revision.segments().len();
            if reference.sealed.segments().get(..count) != Some(revision.segments()) {
                return Err("open-not-a-prefix");
            }
            if reference
                .sealed
                .segments()
                .get(count..)
                .unwrap_or_default()
                .iter()
                .any(|segment| segment.range().start() < until)
            {
                return Err("open-published-too-far");
            }
        }
        RevisionState::Stopped => {
            let TranscriptProvenance::LocalAsr(run) = revision.provenance() else {
                return Err("not-recognised");
            };
            // The run's chunks it had reached when it published last; the
            // rest are `not_reached`.
            let reached = run
                .chunks()
                .iter()
                .filter(|record| !matches!(record.outcome(), AsrChunkOutcome::NotReached))
                .count();
            let open = reference.open_after(session, reached)?;
            let expected = build_stopped_asr_revision(&open, reference.base.as_ref())
                .map_err(|_| "unbuildable")?;
            let stored = encode_transcript_record(revision).map_err(|_| "unencodable")?;
            let expected = encode_transcript_record(&expected).map_err(|_| "unencodable")?;
            if stored != expected {
                return Err("stopped-differs-from-the-reference-cut");
            }
        }
    }
    Ok(())
}

/// The range the workload asked `revision`'s run to replace, never the range
/// the revision declares (RV-1 finding 6): only the growing workloads (`grow`,
/// `part`, `stop`) publish as they go, and they transcribe the whole stand-in
/// source; a sealed revision's range is its job's own request, found by the
/// revision number the job committed. Widened over `base` as the run widened
/// it.
fn workload_range(
    store: &FilesystemSessionStore,
    session: &SessionId,
    revision: &TranscriptRevision,
    base: Option<&TranscriptRevision>,
    source: TimeRange,
    now: u64,
) -> Result<TimeRange, &'static str> {
    let requested = match revision.state() {
        RevisionState::Open | RevisionState::Stopped => None,
        RevisionState::Sealed => {
            let mut requested = None;
            for view in store.session_jobs(session).map_err(|_| "jobs-unreadable")? {
                let Some(commit) = &view.record.commit else {
                    continue;
                };
                let committed = store
                    .read_transcript_revision(session, &commit.revision_id, now)
                    .map_err(|_| "committed-unreadable")?;
                if committed.is_some_and(|(committed, _)| committed.number() == revision.number()) {
                    requested = Some(view.record.request.range());
                    break;
                }
            }
            requested.ok_or("sealed-without-its-job")?
        }
    };
    Ok(retranscription_range(base, requested, source))
}

/// Records what a part published after every chunk would hold.
struct Cuts(std::sync::Mutex<Vec<Result<(AsrTranscription, MediaTime), AsrFailure>>>);

impl RunObserver for Cuts {
    fn chunk_finished(&self, run: &RunSoFar<'_>) {
        if let Ok(mut cuts) = self.0.lock() {
            cuts.push(run.open(MAX_EARLY_AUDIO_MICROS));
        }
    }
}

/// The seal-only run of a revision's workload range: the revision it seals,
/// and what it had settled after each of its chunks.
pub(crate) struct Reference {
    /// The revision the run seals.
    pub(crate) sealed: TranscriptRevision,
    /// After chunk `n` (index `n - 1`), what a part published then holds.
    cuts: Vec<Result<(AsrTranscription, MediaTime), AsrFailure>>,
    base: Option<TranscriptRevision>,
    source_id: SourceId,
    segment: SourceSegment,
    number: NonZeroU32,
    replaced: TimeRange,
}

impl Reference {
    /// The reference of `revision`, the newest revision of `session`.
    ///
    /// # Errors
    ///
    /// What is wrong, as a short fixed name.
    pub(crate) fn of(
        store: &FilesystemSessionStore,
        session: &SessionId,
        revision: &TranscriptRevision,
        now: u64,
    ) -> Result<Self, &'static str> {
        let TranscriptProvenance::LocalAsr(_) = revision.provenance() else {
            return Err("not-recognised");
        };
        let status = store
            .session_status(session)
            .map_err(|_| "status-unreadable")?;
        let segment =
            whole_file_source_segment(status.source_id(), MediaTime::from_micros(SPEECH_MICROS))
                .map_err(|_| "segment")?;
        let base = match revision.supersedes() {
            Some(id) => Some(
                store
                    .read_transcript_revision(session, id, now)
                    .map_err(|_| "base-unreadable")?
                    .map(|(base, _)| base)
                    .ok_or("base-missing")?,
            ),
            None => None,
        };
        let replaced = workload_range(
            store,
            session,
            revision,
            base.as_ref(),
            segment.range(),
            now,
        )?;
        Self::over(
            session,
            status.source_id(),
            segment,
            NonZeroU32::new(revision.number()).ok_or("number")?,
            base,
            replaced,
        )
    }

    /// The seal-only run of `replaced` of the stand-in source, as revision
    /// `number` of `session` over `base`.
    ///
    /// # Errors
    ///
    /// What is wrong, as a short fixed name.
    pub(crate) fn over(
        session: &SessionId,
        source_id: &SourceId,
        segment: SourceSegment,
        number: NonZeroU32,
        base: Option<TranscriptRevision>,
        replaced: TimeRange,
    ) -> Result<Self, &'static str> {
        let identity = recognizer_identity().map_err(|_| "identity")?;
        let key = RecognitionKey::new(Sha256Hex::parse(STAND_IN_DIGEST).map_err(|_| "key")?);
        let cuts = Cuts(std::sync::Mutex::new(Vec::new()));
        // The stand-in recognizer waits on a Tokio timer, and the verifier may
        // be called from inside a runtime or outside one: the recognition runs
        // on a thread and a runtime of its own.
        let transcription = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_time()
                        .build()
                        .map_err(|_| "runtime")?;
                    runtime
                        .block_on(transcribe_range_checkpointed(
                            TranscribeRangeRequest {
                                source_segment: &segment,
                                range: replaced,
                                plan: ChunkPlan::R0,
                                audio_stream: 1,
                                expected: &identity,
                            },
                            CheckpointScope {
                                checkpoints: &NoCheckpoints,
                                key: &key,
                                progress: &NoProgress,
                                observer: &cuts,
                                stopwatch: &StoppedStopwatch,
                                seed: None,
                                gate: None,
                            },
                            &StandInAudio,
                            &StandInRecognizer {
                                delay: Duration::ZERO,
                            },
                            &NeverStop,
                        ))
                        .map(|(transcription, _)| transcription)
                        .map_err(|_| "recognition")
                })
                .join()
                .map_err(|_| "recognition")?
        })?;
        let sealed = build_asr_revision(AsrRevisionRequest {
            session_id: session,
            source_id,
            source_segment: &segment,
            number,
            transcription,
            splice: base.as_ref().map(|base| RevisionSplice {
                base,
                replaced_range: replaced,
            }),
        })
        .map_err(|_| "unbuildable")?;
        Ok(Self {
            sealed,
            cuts: cuts.0.into_inner().map_err(|_| "recognition")?,
            base,
            source_id: source_id.clone(),
            segment,
            number,
            replaced,
        })
    }

    /// The open revision a part published after the first `chunks` chunks of
    /// the run holds.
    ///
    /// # Errors
    ///
    /// `no-such-cut` when the run has no such cut.
    pub(crate) fn open_after(
        &self,
        session: &SessionId,
        chunks: usize,
    ) -> Result<TranscriptRevision, &'static str> {
        let (transcription, settled_until) = chunks
            .checked_sub(1)
            .and_then(|index| self.cuts.get(index))
            .ok_or("no-such-cut")?
            .clone()
            .map_err(|_| "no-such-cut")?;
        build_open_asr_revision(
            AsrRevisionRequest {
                session_id: session,
                source_id: &self.source_id,
                source_segment: &self.segment,
                number: self.number,
                transcription,
                splice: self.base.as_ref().map(|base| RevisionSplice {
                    base,
                    replaced_range: self.replaced,
                }),
            },
            settled_until,
        )
        .map_err(|_| "unbuildable")
    }
}

/// Reads every worker request record on disk through the store (P11 PR 3):
/// a crash may leave a record's old or new version, never a torn one, so
/// any record that does not read is damage.
fn check_request_records(root: &Path, store: &FilesystemSessionStore, findings: &mut Findings) {
    let Ok(buckets) = std::fs::read_dir(root.join("worker-requests")) else {
        return;
    };
    for bucket in buckets.filter_map(Result::ok) {
        let Ok(entries) = std::fs::read_dir(bucket.path()) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let Some(operation) = entry
                .file_name()
                .into_string()
                .ok()
                .and_then(|name| name.strip_suffix(".json").map(str::to_owned))
            else {
                continue;
            };
            let Ok(operation_id) = OperationId::parse(operation) else {
                continue;
            };
            findings.requests += 1;
            if let Err(error) = store.read_worker_request(&operation_id) {
                findings.damage.push(Damage::Request {
                    operation: operation_id.as_str().to_owned(),
                    code: code(error),
                });
            }
        }
    }
}

/// Lowercase hex SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (u64::try_from(bytes.len()).ok()? <= limit).then_some(bytes)
}

/// A plain file name: no separators, not `.` or `..`.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// Walks one session on disk from its pointer down to generation 0 and
/// hashes every artifact the head lists. Damage is recorded in `findings`;
/// `Err` means the session has no valid committed state.
fn walk_session(
    directory: &Path,
    name: &str,
    findings: &mut Findings,
) -> Result<SessionOnDisk, ()> {
    let broken = |findings: &mut Findings, generation: u64, what: &'static str| {
        findings.damage.push(Damage::Chain {
            session: name.to_owned(),
            generation,
            what,
        });
        Err(())
    };
    let Some(pointer) = read_bounded(&directory.join("current.json"), 64 * 1024)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return broken(findings, 0, "pointer-unreadable");
    };
    let (Some(head), Some(head_digest)) = (
        pointer.get("generation").and_then(Value::as_u64),
        pointer.get("manifest_sha256").and_then(Value::as_str),
    ) else {
        return broken(findings, 0, "pointer-malformed");
    };
    let mut manifests = Vec::new();
    let mut expected = head_digest.to_owned();
    let mut head_manifest = None;
    let mut durable = false;
    for generation in (0..=head).rev() {
        let path = directory
            .join("generations")
            .join(format!("{generation}.json"));
        let Some(bytes) = read_bounded(&path, MAX_MANIFEST_BYTES) else {
            return broken(findings, generation, "manifest-missing");
        };
        let digest = sha256_hex(&bytes);
        if digest != expected {
            return broken(findings, generation, "manifest-digest");
        }
        let Ok(manifest) = serde_json::from_slice::<Value>(&bytes) else {
            return broken(findings, generation, "manifest-malformed");
        };
        if manifest.get("generation").and_then(Value::as_u64) != Some(generation)
            || manifest.get("session_id").and_then(Value::as_str) != Some(name)
        {
            return broken(findings, generation, "manifest-identity");
        }
        let previous = manifest
            .get("previous_manifest_sha256")
            .and_then(Value::as_str)
            .map(str::to_owned);
        match (generation, previous) {
            (0, None) => {}
            (0, Some(_)) | (_, None) => {
                return broken(findings, generation, "manifest-link");
            }
            (_, Some(link)) => expected = link,
        }
        if generation == 0 {
            durable = manifest.get("durability").and_then(Value::as_str) == Some("durable");
        }
        manifests.push(digest);
        findings.generations += 1;
        if head_manifest.is_none() {
            head_manifest = Some(manifest);
        }
    }
    manifests.reverse();
    let artifacts = match head_manifest
        .as_ref()
        .and_then(|manifest| manifest.get("lifecycle"))
    {
        Some(lifecycle) => check_lifecycle(directory, name, head, lifecycle, findings)?,
        None => BTreeSet::new(),
    };
    Ok(SessionOnDisk {
        head,
        manifests,
        artifacts,
        durable,
    })
}

/// Hashes every artifact the head's lifecycle lists, and the source copy;
/// returns their digests.
fn check_lifecycle(
    directory: &Path,
    name: &str,
    head: u64,
    lifecycle: &Value,
    findings: &mut Findings,
) -> Result<BTreeSet<String>, ()> {
    let broken = |findings: &mut Findings, what: &'static str| {
        findings.damage.push(Damage::Chain {
            session: name.to_owned(),
            generation: head,
            what,
        });
        Err(())
    };
    let mut artifacts = BTreeSet::new();
    let listed = lifecycle
        .get("artifacts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for artifact in listed {
        let (Some(file), Some(digest), Some(bytes)) = (
            artifact.get("name").and_then(Value::as_str),
            artifact.get("sha256").and_then(Value::as_str),
            artifact.get("bytes").and_then(Value::as_u64),
        ) else {
            return broken(findings, "artifact-entry");
        };
        if !check_file(directory, name, file, digest, bytes, findings) {
            return Err(());
        }
        artifacts.insert(digest.to_owned());
    }
    let (Some(source), Some(source_id), Some(source_bytes)) = (
        lifecycle.get("source_name").and_then(Value::as_str),
        lifecycle.get("source_id").and_then(Value::as_str),
        lifecycle.get("source_bytes").and_then(Value::as_u64),
    ) else {
        return broken(findings, "source-entry");
    };
    let Some(digest) = source_id.strip_prefix("src_sha256_") else {
        return broken(findings, "source-id");
    };
    if !check_file(directory, name, source, digest, source_bytes, findings) {
        return Err(());
    }
    artifacts.insert(digest.to_owned());
    Ok(artifacts)
}

fn check_file(
    directory: &Path,
    session: &str,
    file: &str,
    digest: &str,
    bytes: u64,
    findings: &mut Findings,
) -> bool {
    let whole = plain_name(file)
        && read_bounded(&directory.join("artifacts").join(file), MAX_ARTIFACT_BYTES).is_some_and(
            |content| {
                u64::try_from(content.len()).ok() == Some(bytes) && sha256_hex(&content) == digest
            },
        );
    findings.artifacts += 1;
    if !whole {
        findings.damage.push(Damage::Artifact {
            session: session.to_owned(),
            name: file.to_owned(),
        });
    }
    whole
}

#[cfg(test)]
mod tests {
    use super::{Findings, Loss, plain_name};

    #[test]
    fn artifact_names_cannot_leave_the_directory() {
        assert!(plain_name("artifact-00.png"));
        for name in ["", ".", "..", "../x", "a/b", "a\\b", "a\0b"] {
            assert!(!plain_name(name), "{name:?}");
        }
    }

    #[test]
    fn findings_report_every_loss_and_a_summary() {
        let findings = Findings {
            lost: vec![(3, Loss::HeadBehind { head: 1 })],
            acks: 4,
            ..Findings::default()
        };
        assert!(!findings.clean());
        assert_eq!(
            findings.lines("VERIFY"),
            vec![
                "VERIFY-LOST seq=3 head-behind head=1".to_owned(),
                "VERIFY FAIL acks=4 sessions=0 generations=0 artifacts=0 jobs=0 requests=0 lost=1 damaged=0"
                    .to_owned()
            ]
        );
    }
}
