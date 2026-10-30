//! The `watch` use case over fakes of its two ports (ADR 0037 section 14):
//! the order of the steps, refusals before any work, stated gaps, the frame
//! bound with its even spread and its backstop, the snapshots of the files
//! that grow, Ctrl-C keeping the session, a rerun continuing the job, and
//! `--keep`: a bundle only of a sealed job, and a close only after it.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::{Clock, ClockError};

use vsift_domain::{
    CandidateReason, EvidenceId, FailureCode, IdentifierError, JobId, MAX_WATCH_FRAMES, MediaTime,
    SessionId, SourceId, TimeRange, TranscriptRevisionId, VisualCandidateId, VisualIndexId,
    WatchOutputNames, spread_evenly,
};

use super::{
    ClaimedOutputs, FinalisedOutputs, FrameRefusal, OpenedWatchSession, PrepareStop, PreparedRun,
    PreparedTranscript, PublishedPart, ScreenChange, ScreenChangeEntry, ScreenChangeSummary,
    ScreenChanges, SessionFrame, SnapshotState, TranscriptEntry, TranscriptLine, UnfinishedWatch,
    WatchDocuments, WatchError, WatchFile, WatchOutputError, WatchOutputKind, WatchOutputs,
    WatchProgress, WatchProvenance, WatchSnapshot, WatchSteps, run_watch,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The fake engine's error: which step failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StepFailure {
    Source,
    Capabilities,
    Open,
    Prepare,
    Frame,
}

fn seconds(value: u64) -> MediaTime {
    MediaTime::from_micros(value * 1_000_000)
}

fn range(start: u64, end: u64) -> Result<TimeRange, Box<dyn std::error::Error>> {
    Ok(TimeRange::new(seconds(start), seconds(end))?)
}

fn change(position: usize) -> Result<ScreenChange, IdentifierError> {
    Ok(ScreenChange {
        id: VisualCandidateId::parse(format!("vcd_{position:032}"))?,
        time: seconds(u64::try_from(position).unwrap_or(u64::MAX)),
        reason: CandidateReason::SettledAfterMotion,
    })
}

/// The engine's operations, scripted: what each answers, and every call in
/// order.
/// What the fake job tells the use case while it runs, in order.
#[derive(Clone, Copy, Debug)]
enum FakeNotice {
    Published,
    Indexed,
}

/// A clock that moves on by `step` seconds each time it is read.
struct SteppingClock {
    now: Mutex<u64>,
    step: u64,
}

impl Clock for SteppingClock {
    fn now_unix_seconds(&self) -> Result<u64, ClockError> {
        let mut now = self.now.lock().map_err(|_| ClockError::BeforeUnixEpoch)?;
        let read = *now;
        *now += self.step;
        Ok(read)
    }
}

struct FakeSteps {
    fail: Option<StepFailure>,
    /// The caller stops the job (Ctrl-C) after its notices.
    stopped: bool,
    notices: Vec<FakeNotice>,
    published: Option<PublishedPart>,
    unfinished: Option<UnfinishedWatch>,
    clock: SteppingClock,
    transcript: Option<PreparedTranscript>,
    lines: Vec<TranscriptLine>,
    changes: Vec<ScreenChange>,
    evidence_slots: usize,
    /// Frames the session stores before it is full.
    frame_room: usize,
    /// Why the bundle cannot be written.
    keep_failure: Option<FailureCode>,
    calls: Mutex<Vec<String>>,
}

impl FakeSteps {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            fail: None,
            stopped: false,
            notices: vec![FakeNotice::Indexed, FakeNotice::Published],
            published: Some(PublishedPart {
                revision: TranscriptRevisionId::parse("trv_1f2e3d4c5b6a79880f1e2d3c4b5a6978")?,
                lines: vec![TranscriptLine {
                    start: seconds(1),
                    end: seconds(4),
                    text: "Good morning.".to_owned(),
                }],
                published_until: seconds(30),
                duration: seconds(60),
                since: Some("since-1".to_owned()),
            }),
            unfinished: None,
            clock: SteppingClock {
                now: Mutex::new(1_000),
                step: 0,
            },
            transcript: Some(PreparedTranscript {
                revision: TranscriptRevisionId::parse("trv_1f2e3d4c5b6a79880f1e2d3c4b5a6978")?,
                range: range(0, 60)?,
                untranscribed: Vec::new(),
            }),
            lines: vec![TranscriptLine {
                start: seconds(1),
                end: seconds(4),
                text: "Good morning.".to_owned(),
            }],
            changes: vec![change(1)?, change(0)?],
            evidence_slots: 384,
            frame_room: usize::MAX,
            keep_failure: None,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn record(&self, call: impl Into<String>) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call.into());
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }

    fn frames_asked(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|call| call.strip_prefix("take_frame ").map(str::to_owned))
            .collect()
    }

    fn step(&self, name: &str, failure: StepFailure) -> Result<(), StepFailure> {
        self.record(name);
        if self.fail == Some(failure) {
            Err(failure)
        } else {
            Ok(())
        }
    }

    fn session() -> Result<OpenedWatchSession, IdentifierError> {
        Ok(OpenedWatchSession {
            session_id: SessionId::parse("ses_0123456789abcdef")?,
            source_id: SourceId::from_sha256(&"a".repeat(64))?,
        })
    }
}

impl WatchSteps for FakeSteps {
    type Error = StepFailure;

    fn check_source(&self, _video: &Path) -> impl Future<Output = Result<(), StepFailure>> + Send {
        let answer = self.step("check_source", StepFailure::Source);
        std::future::ready(answer)
    }

    fn check_capabilities(&self) -> Result<(), StepFailure> {
        self.step("check_capabilities", StepFailure::Capabilities)
    }

    fn open_session(
        &self,
        _video: &Path,
    ) -> impl Future<Output = Result<OpenedWatchSession, StepFailure>> + Send {
        let answer = (|| {
            self.step("open_session", StepFailure::Open)?;
            Self::session().map_err(|_| StepFailure::Open)
        })();
        std::future::ready(answer)
    }

    fn start_job(&self, _session: &SessionId) -> Result<JobId, StepFailure> {
        self.record("start_job");
        JobId::parse("job_0123456789abcdef").map_err(|_| StepFailure::Prepare)
    }

    fn record_claim(
        &self,
        _session: &SessionId,
        job: &JobId,
        _claimed: &ClaimedOutputs,
    ) -> Result<(), StepFailure> {
        self.record(format!("record_claim {}", job.as_str()));
        Ok(())
    }

    fn prepare(
        &self,
        _session: &SessionId,
        job: &JobId,
        progress: &dyn WatchProgress,
    ) -> impl Future<Output = Result<PreparedRun, PrepareStop<StepFailure>>> + Send {
        let answer = (|| {
            self.step(&format!("prepare {}", job.as_str()), StepFailure::Prepare)
                .map_err(PrepareStop::Failed)?;
            for notice in &self.notices {
                match notice {
                    FakeNotice::Published => progress.published(),
                    FakeNotice::Indexed => progress.visual_indexed(),
                }
            }
            if self.stopped {
                return Err(PrepareStop::Interrupted);
            }
            Ok(PreparedRun {
                job_id: job.clone(),
                transcript: self.transcript.clone(),
            })
        })();
        std::future::ready(answer)
    }

    fn published(
        &self,
        _session: &SessionId,
        since: Option<&str>,
    ) -> Result<Option<PublishedPart>, StepFailure> {
        self.record(format!("published {}", since.unwrap_or("-")));
        Ok(self.published.clone())
    }

    fn find_unfinished(
        &self,
        _video: &Path,
        _claimed: &ClaimedOutputs,
    ) -> impl Future<Output = Result<Option<UnfinishedWatch>, StepFailure>> + Send {
        self.record("find_unfinished");
        std::future::ready(Ok(self.unfinished.clone()))
    }

    fn clock(&self) -> &dyn Clock {
        &self.clock
    }

    fn transcript_lines(
        &self,
        _session: &SessionId,
        _revision: &TranscriptRevisionId,
        _range: TimeRange,
    ) -> Result<Vec<TranscriptLine>, StepFailure> {
        self.record("transcript_lines");
        Ok(self.lines.clone())
    }

    fn screen_changes(
        &self,
        _session: &SessionId,
    ) -> impl Future<Output = Result<Option<ScreenChanges>, StepFailure>> + Send {
        let answer = (|| {
            self.record("screen_changes");
            Ok(Some(ScreenChanges {
                index: VisualIndexId::parse("vix_31b0dcc02c3694707c889ba38d140c4f")
                    .map_err(|_| StepFailure::Prepare)?,
                covered_to: seconds(60),
                changes: self.changes.clone(),
            }))
        })();
        std::future::ready(answer)
    }

    fn evidence_slots_left(&self, _session: &SessionId) -> Result<usize, StepFailure> {
        self.record("evidence_slots_left");
        Ok(self.evidence_slots)
    }

    fn take_frame(
        &self,
        _session: &SessionId,
        change: &VisualCandidateId,
    ) -> impl Future<Output = Result<SessionFrame, FrameRefusal<StepFailure>>> + Send {
        let answer = (|| {
            let taken = self.frames_asked().len();
            self.record(format!("take_frame {}", change.as_str()));
            if self.fail == Some(StepFailure::Frame) {
                return Err(FrameRefusal::Failed(StepFailure::Frame));
            }
            if taken >= self.frame_room {
                return Err(FrameRefusal::NoRoom);
            }
            Ok(SessionFrame {
                evidence_id: EvidenceId::parse(format!("evd_{taken:032}"))
                    .map_err(|_| FrameRefusal::Failed(StepFailure::Frame))?,
                path: PathBuf::from(format!("session/frame-{taken}.png")),
                sha256: "0".repeat(64),
            })
        })();
        std::future::ready(answer)
    }

    fn provenance(
        &self,
        video_name: &OsStr,
        _source: &SourceId,
        session: &SessionId,
    ) -> Result<WatchProvenance, StepFailure> {
        self.record("provenance");
        Ok(WatchProvenance {
            video_name: video_name.to_string_lossy().into_owned(),
            made_by: "VSift 0.3.0".to_owned(),
            made_at: "2026-11-20 14:05 UTC".to_owned(),
            video_sha256: "a".repeat(64),
            session_id: session.as_str().to_owned(),
        })
    }

    fn keep_session(&self, _session: &SessionId, bundle: &Path) -> Result<(), FailureCode> {
        self.record(format!("keep_session {}", bundle.display()));
        self.keep_failure.map_or(Ok(()), Err)
    }

    fn close_session(&self, _session: &SessionId) -> bool {
        self.record("close_session");
        true
    }
}

/// The person's folder, faked: every call in order, a name already taken on
/// cue, and the documents it was asked to finish.
#[derive(Default)]
struct FakeOutputs {
    taken: bool,
    /// The keep check's refusal.
    keep_refused: Option<WatchOutputError>,
    /// The final write fails, as for a file held open.
    final_write_fails: bool,
    calls: Vec<String>,
    documents: Option<WatchDocuments>,
    listed: Option<(ScreenChangeSummary, Vec<ScreenChangeEntry>)>,
    snapshots: Vec<WatchSnapshot>,
}

fn file(name: &str) -> WatchFile {
    WatchFile {
        path: PathBuf::from(name),
        sha256: Some("b".repeat(64)),
    }
}

impl WatchOutputs for FakeOutputs {
    fn claim(
        &mut self,
        _video: &Path,
        _output: Option<&Path>,
        names: &WatchOutputNames,
    ) -> Result<ClaimedOutputs, WatchOutputError> {
        self.calls.push("claim".to_owned());
        if self.taken {
            return Err(WatchOutputError::Exists(WatchOutputKind::Transcript));
        }
        Ok(ClaimedOutputs {
            transcript: PathBuf::from(names.transcript()),
            subtitles: PathBuf::from(names.subtitles()),
            folder: PathBuf::from(names.folder()),
        })
    }

    fn planned(
        &mut self,
        _video: &Path,
        _output: Option<&Path>,
        names: &WatchOutputNames,
    ) -> Result<ClaimedOutputs, WatchOutputError> {
        self.calls.push("planned".to_owned());
        Ok(ClaimedOutputs {
            transcript: PathBuf::from(names.transcript()),
            subtitles: PathBuf::from(names.subtitles()),
            folder: PathBuf::from(names.folder()),
        })
    }

    fn reclaim(&mut self, _claimed: &ClaimedOutputs) -> Result<(), WatchOutputError> {
        self.calls.push("reclaim".to_owned());
        Ok(())
    }

    fn prepare_keep(
        &mut self,
        folder: &Path,
        names: &WatchOutputNames,
    ) -> Result<PathBuf, WatchOutputError> {
        self.calls.push("prepare_keep".to_owned());
        self.keep_refused
            .take()
            .map_or_else(|| Ok(folder.join(names.bundle())), Err)
    }

    fn write_snapshot(&mut self, snapshot: &WatchSnapshot) -> Result<(), WatchOutputError> {
        self.calls.push("write_snapshot".to_owned());
        self.snapshots.push(snapshot.clone());
        Ok(())
    }

    fn write_screen_changes(
        &mut self,
        _provenance: &WatchProvenance,
        summary: &ScreenChangeSummary,
        changes: &[ScreenChangeEntry],
    ) -> Result<WatchFile, WatchOutputError> {
        self.calls.push("write_screen_changes".to_owned());
        self.listed = Some((summary.clone(), changes.to_vec()));
        Ok(file("screen-changes.txt"))
    }

    fn copy_frame(
        &mut self,
        _frame: &SessionFrame,
        file_name: &str,
    ) -> Result<PathBuf, WatchOutputError> {
        self.calls.push(format!("copy_frame {file_name}"));
        Ok(PathBuf::from(file_name))
    }

    fn finalise(
        &mut self,
        documents: &WatchDocuments,
    ) -> Result<FinalisedOutputs, WatchOutputError> {
        self.calls.push("finalise".to_owned());
        if self.final_write_fails {
            return Err(WatchOutputError::Held(WatchOutputKind::Transcript));
        }
        self.documents = Some(documents.clone());
        Ok(FinalisedOutputs {
            transcript: file("meeting.txt"),
            subtitles: documents
                .transcript_revision
                .as_ref()
                .map(|_| file("meeting.srt")),
        })
    }

    fn release(&mut self) {
        self.calls.push("release".to_owned());
    }
}

const VIDEO: &str = "/videos/meeting.mp4";

#[tokio::test]
async fn the_steps_run_in_their_order_and_the_session_is_closed() -> TestResult {
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs::default();
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        steps.calls(),
        vec![
            "check_source",
            "check_capabilities",
            "open_session",
            "start_job",
            "record_claim job_0123456789abcdef",
            "prepare job_0123456789abcdef",
            "screen_changes",
            "evidence_slots_left",
            "take_frame vcd_00000000000000000000000000000000",
            "take_frame vcd_00000000000000000000000000000001",
            "provenance",
            "published -",
            "provenance",
            "transcript_lines",
            "provenance",
            "close_session",
        ]
    );
    assert_eq!(
        outputs.calls,
        vec![
            "claim",
            "copy_frame 00-00-00.000.png",
            "copy_frame 00-00-01.000.png",
            "write_screen_changes",
            "write_snapshot",
            "finalise",
        ],
        "the names are claimed before the session opens, and nothing is released"
    );
    assert!(outcome.session_closed);
    assert_eq!(outcome.changes_found, 2);
    assert_eq!(outcome.frames.len(), 2);
    assert!(outcome.subtitles.is_some());
    assert!(!outcome.frames_stopped_for_room);
    Ok(())
}

#[tokio::test]
async fn a_missing_capability_refuses_before_anything_is_claimed() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.fail = Some(StepFailure::Capabilities);
    let mut outputs = FakeOutputs::default();
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None).await;
    assert!(matches!(
        result,
        Err(WatchError::Step(StepFailure::Capabilities))
    ));
    assert_eq!(
        outputs.calls,
        Vec::<String>::new(),
        "nothing is claimed or written"
    );
    assert!(!steps.calls().contains(&"open_session".to_owned()));
    Ok(())
}

#[tokio::test]
async fn a_name_that_is_taken_refuses_before_any_session() -> TestResult {
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs {
        taken: true,
        ..FakeOutputs::default()
    };
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None).await;
    assert!(matches!(
        result,
        Err(WatchError::Output(WatchOutputError::Exists(
            WatchOutputKind::Transcript
        )))
    ));
    assert_eq!(
        outputs.calls,
        vec!["claim", "planned"],
        "nothing is written or released"
    );
    assert_eq!(
        steps.calls(),
        vec!["check_source", "check_capabilities", "find_unfinished"],
        "no session is opened"
    );
    Ok(())
}

#[tokio::test]
async fn a_source_that_is_not_a_video_file_refuses_first() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.fail = Some(StepFailure::Source);
    let mut outputs = FakeOutputs::default();
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None).await;
    assert!(matches!(result, Err(WatchError::Step(StepFailure::Source))));
    assert_eq!(steps.calls(), vec!["check_source"]);
    assert_eq!(outputs.calls, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn a_name_with_nothing_before_its_extension_refuses_before_the_claim() -> TestResult {
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs::default();
    let result = run_watch(
        &steps,
        &mut outputs,
        Path::new("/videos/...mp4"),
        None,
        None,
    )
    .await;
    assert!(matches!(result, Err(WatchError::UnusableName(_))));
    assert_eq!(outputs.calls, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn a_failure_after_the_claim_releases_it_and_closes_the_session() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.fail = Some(StepFailure::Prepare);
    let mut outputs = FakeOutputs::default();
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None).await;
    assert!(matches!(
        result,
        Err(WatchError::Step(StepFailure::Prepare))
    ));
    assert_eq!(outputs.calls, vec!["claim", "release"]);
    assert_eq!(
        steps.calls().last().map(String::as_str),
        Some("close_session")
    );
    Ok(())
}

#[tokio::test]
async fn a_session_that_cannot_open_releases_the_claim() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.fail = Some(StepFailure::Open);
    let mut outputs = FakeOutputs::default();
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None).await;
    assert!(matches!(result, Err(WatchError::Step(StepFailure::Open))));
    assert_eq!(outputs.calls, vec!["claim", "release"]);
    assert!(!steps.calls().contains(&"close_session".to_owned()));
    Ok(())
}

#[tokio::test]
async fn a_failed_chunk_becomes_a_stated_gap_in_time_order() -> TestResult {
    let mut steps = FakeSteps::new()?;
    let gap = range(10, 20)?;
    if let Some(transcript) = steps.transcript.as_mut() {
        transcript.untranscribed = vec![gap];
    }
    steps.lines.push(TranscriptLine {
        start: seconds(25),
        end: seconds(28),
        text: "After the gap.".to_owned(),
    });
    let mut outputs = FakeOutputs::default();
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(outcome.not_transcribed, vec![gap]);
    let entries = outputs
        .documents
        .map(|documents| documents.entries)
        .unwrap_or_default();
    let starts: Vec<(u64, bool)> = entries
        .iter()
        .map(|entry| match entry {
            TranscriptEntry::Line { start, .. } => (start.as_micros() / 1_000_000, false),
            TranscriptEntry::Gap { start, .. } => (start.as_micros() / 1_000_000, true),
        })
        .collect();
    assert_eq!(
        starts,
        vec![(1, false), (10, true), (25, false)],
        "the gap is stated where it falls"
    );
    Ok(())
}

#[tokio::test]
async fn a_video_with_no_sound_track_has_no_subtitles() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.transcript = None;
    let mut outputs = FakeOutputs::default();
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert!(outcome.subtitles.is_none());
    assert!(outcome.transcript_revision.is_none());
    assert!(!steps.calls().contains(&"transcript_lines".to_owned()));
    assert_eq!(
        outcome.duration,
        seconds(60),
        "the visual index gives the length"
    );
    Ok(())
}

#[tokio::test]
async fn more_changes_than_the_bound_get_frames_spread_evenly_and_all_are_listed() -> TestResult {
    let mut steps = FakeSteps::new()?;
    let count = 576;
    steps.changes = (0..count).map(change).collect::<Result<_, _>>()?;
    let mut outputs = FakeOutputs::default();
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    let expected: Vec<String> = spread_evenly(count, MAX_WATCH_FRAMES)
        .into_iter()
        .map(|position| format!("vcd_{position:032}"))
        .collect();
    assert_eq!(steps.frames_asked(), expected);
    assert_eq!(outcome.frames.len(), MAX_WATCH_FRAMES);
    assert_eq!(outcome.changes_found, count);
    let listed = outputs
        .listed
        .map(|(_, changes)| changes)
        .unwrap_or_default();
    assert_eq!(listed.len(), count, "every change is listed");
    assert_eq!(
        listed.iter().filter(|entry| entry.frame.is_some()).count(),
        MAX_WATCH_FRAMES
    );
    assert!(!outcome.frames_stopped_for_room);
    Ok(())
}

#[tokio::test]
async fn the_frames_fit_the_room_measured_before_the_first() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.changes = (0..40).map(change).collect::<Result<_, _>>()?;
    steps.evidence_slots = 21;
    let mut outputs = FakeOutputs::default();
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        outcome.frames.len(),
        10,
        "two evidence slots a frame: 21 slots hold 10 frames"
    );
    assert!(!outcome.frames_stopped_for_room);
    Ok(())
}

#[tokio::test]
async fn a_session_out_of_room_ends_the_frames_not_the_run() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.changes = (0..10).map(change).collect::<Result<_, _>>()?;
    steps.frame_room = 3;
    let mut outputs = FakeOutputs::default();
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert!(
        outcome.frames_stopped_for_room,
        "the run says why and is partial"
    );
    assert_eq!(outcome.frames.len(), 3, "the frames taken before are kept");
    assert_eq!(
        steps.frames_asked().len(),
        4,
        "no frame is asked for after the refusal"
    );
    let (summary, changes) = outputs.listed.ok_or("the list was written")?;
    assert!(summary.stopped_for_room);
    assert_eq!(changes.len(), 10, "every change is still listed");
    Ok(())
}

#[tokio::test]
async fn any_other_frame_failure_ends_the_run_and_releases_the_claim() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.fail = Some(StepFailure::Frame);
    let mut outputs = FakeOutputs::default();
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None).await;
    assert!(matches!(result, Err(WatchError::Step(StepFailure::Frame))));
    assert_eq!(
        outputs.calls,
        vec!["claim", "write_snapshot", "write_snapshot", "release"]
    );
    assert_eq!(
        outputs
            .snapshots
            .iter()
            .map(|snapshot| snapshot.state)
            .collect::<Vec<_>>(),
        vec![SnapshotState::Running, SnapshotState::Failed],
        "a failure after a snapshot leaves the files saying so"
    );
    assert_eq!(
        steps.calls().last().map(String::as_str),
        Some("close_session")
    );
    Ok(())
}

#[tokio::test]
async fn the_list_holds_the_settled_screens_and_only_they_get_frames() -> TestResult {
    let mut steps = FakeSteps::new()?;
    let reasons = [
        CandidateReason::FirstFrame,
        CandidateReason::MotionStart,
        CandidateReason::VisualChange,
        CandidateReason::SettledAfterMotion,
        CandidateReason::VisualChange,
        CandidateReason::VisualChange,
    ];
    let times = [0, 1, 2, 4, 7, 9];
    steps.changes = reasons
        .iter()
        .zip(times)
        .enumerate()
        .map(|(position, (&reason, time))| {
            change(position).map(|change| ScreenChange {
                time: seconds(time),
                reason,
                ..change
            })
        })
        .collect::<Result<_, _>>()?;
    let mut outputs = FakeOutputs::default();
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(outcome.changes_found, 6, "every change found is counted");
    assert_eq!(
        outcome.changes_listed, 3,
        "the first frame, the settled screen and the change 5 s after it"
    );
    assert_eq!(
        steps.frames_asked(),
        vec![
            "vcd_00000000000000000000000000000000",
            "vcd_00000000000000000000000000000003",
            "vcd_00000000000000000000000000000005",
        ]
    );
    let (summary, changes) = outputs.listed.ok_or("the list was written")?;
    assert_eq!(summary.changes_found, 6);
    assert_eq!(
        changes.iter().map(|entry| entry.reason).collect::<Vec<_>>(),
        vec![
            CandidateReason::FirstFrame,
            CandidateReason::SettledAfterMotion,
            CandidateReason::VisualChange,
        ]
    );
    Ok(())
}

#[tokio::test]
async fn a_publication_writes_a_snapshot_of_what_was_published() -> TestResult {
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs::default();
    run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    let snapshot = outputs.snapshots.first().ok_or("a snapshot was written")?;
    assert_eq!(snapshot.state, SnapshotState::Running);
    assert_eq!(snapshot.published_until, seconds(30));
    assert_eq!(snapshot.duration, Some(seconds(60)));
    assert_eq!(snapshot.entries.len(), 1);
    assert!(
        steps.calls().contains(&"published -".to_owned()),
        "the first read reads everything"
    );
    Ok(())
}

#[tokio::test]
async fn snapshots_are_at_least_ten_seconds_apart() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.notices = vec![FakeNotice::Published; 5];
    let mut outputs = FakeOutputs::default();
    run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        outputs.snapshots.len(),
        1,
        "the notices came within ten seconds"
    );

    let mut steps = FakeSteps::new()?;
    steps.notices = vec![FakeNotice::Published];
    steps.clock.step = 10;
    let mut outputs = FakeOutputs::default();
    let signal = super::WatchSignal::default();
    let run = super::WatchRun {
        steps: &steps,
        session: &FakeSteps::session()?,
        file_name: OsStr::new("meeting.mp4"),
        names: &WatchOutputNames::from_video_name(OsStr::new("meeting.mp4"))?,
        claimed: ClaimedOutputs {
            transcript: PathBuf::from("meeting.txt"),
            subtitles: PathBuf::from("meeting.srt"),
            folder: PathBuf::from("meeting-vsift"),
        },
        taken_over: false,
    };
    let mut following = super::Following {
        revision: None,
        lines: Vec::new(),
        since: None,
        published_until: seconds(0),
        duration: None,
        last_snapshot_at: None,
        snapshot_written: false,
        sealed: false,
        screens: None,
    };
    for _ in 0..3 {
        signal.published();
    }
    signal.close();
    run.follow(&mut outputs, &signal, &mut following).await;
    assert_eq!(outputs.snapshots.len(), 1, "one notice, one snapshot");
    signal_twice_ten_seconds_apart(&run, &mut outputs, &mut following).await;
    assert_eq!(outputs.snapshots.len(), 2);
    assert!(
        steps.calls().contains(&"published since-1".to_owned()),
        "a later read follows the revision with its token"
    );
    Ok(())
}

async fn signal_twice_ten_seconds_apart(
    run: &super::WatchRun<'_, FakeSteps>,
    outputs: &mut FakeOutputs,
    following: &mut super::Following<StepFailure>,
) {
    let signal = super::WatchSignal::default();
    signal.published();
    signal.close();
    run.follow(outputs, &signal, following).await;
}

#[tokio::test]
async fn the_frames_are_written_when_the_visual_stream_is_done() -> TestResult {
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs::default();
    run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    let calls = steps.calls();
    let frames = calls
        .iter()
        .position(|call| call.starts_with("take_frame"))
        .ok_or("frames were taken")?;
    let sealed = calls
        .iter()
        .position(|call| call == "transcript_lines")
        .ok_or("the sealed transcript was read")?;
    assert!(frames < sealed, "the frames come before the sealed read");
    let list = outputs
        .calls
        .iter()
        .position(|call| call == "write_screen_changes")
        .ok_or("the list was written")?;
    let finished = outputs
        .calls
        .iter()
        .position(|call| call == "finalise")
        .ok_or("the files were finished")?;
    assert!(list < finished, "the list is written once the frames are");
    Ok(())
}

#[tokio::test]
async fn ctrl_c_keeps_the_files_and_the_session() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.stopped = true;
    let mut outputs = FakeOutputs::default();
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None).await;
    assert!(matches!(
        result,
        Err(WatchError::Stopped {
            transcribed_to,
            duration: Some(duration),
        }) if transcribed_to == seconds(30) && duration == seconds(60)
    ));
    assert!(!outputs.calls.contains(&"release".to_owned()));
    assert!(!outputs.calls.contains(&"finalise".to_owned()));
    assert_eq!(
        outputs.calls.last().map(String::as_str),
        Some("write_snapshot"),
        "the files keep the last snapshot"
    );
    assert_eq!(
        outputs.snapshots.last().map(|snapshot| snapshot.state),
        Some(SnapshotState::Stopped),
        "and say where the run stopped"
    );
    assert!(!steps.calls().contains(&"close_session".to_owned()));
    assert!(
        steps
            .calls()
            .contains(&"record_claim job_0123456789abcdef".to_owned()),
        "the job's record keeps the claimed names"
    );
    Ok(())
}

#[tokio::test]
async fn a_rerun_continues_the_unfinished_job_of_the_same_names() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.unfinished = Some(UnfinishedWatch {
        session: FakeSteps::session()?,
        job: JobId::parse("job_fedcba9876543210")?,
    });
    let mut outputs = FakeOutputs {
        taken: true,
        ..FakeOutputs::default()
    };
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        outputs.calls.get(..3),
        Some(
            ["claim", "planned", "reclaim"]
                .map(str::to_owned)
                .as_slice()
        )
    );
    let calls = steps.calls();
    assert!(calls.contains(&"prepare job_fedcba9876543210".to_owned()));
    assert!(
        !calls.contains(&"open_session".to_owned()),
        "no new session"
    );
    assert!(!calls.contains(&"start_job".to_owned()), "no new job");
    assert!(outcome.session_closed);
    Ok(())
}

#[tokio::test]
async fn taken_names_without_an_unfinished_job_are_refused() -> TestResult {
    // An expired session is gone, so no unfinished job is found and the
    // files are refused, whatever their text says (the planner's condition A).
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs {
        taken: true,
        ..FakeOutputs::default()
    };
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None).await;
    assert!(matches!(
        result,
        Err(WatchError::Output(WatchOutputError::Exists(
            WatchOutputKind::Transcript
        )))
    ));
    assert_eq!(outputs.calls, vec!["claim", "planned"]);
    assert!(!steps.calls().contains(&"open_session".to_owned()));
    Ok(())
}

#[tokio::test]
async fn a_failed_rerun_leaves_the_taken_over_files_saying_so() -> TestResult {
    // The files of the interrupted run say it can be continued; once this
    // run fails, its session is closed and that is no longer true.
    let mut steps = FakeSteps::new()?;
    steps.fail = Some(StepFailure::Prepare);
    steps.unfinished = Some(UnfinishedWatch {
        session: FakeSteps::session()?,
        job: JobId::parse("job_fedcba9876543210")?,
    });
    let mut outputs = FakeOutputs {
        taken: true,
        ..FakeOutputs::default()
    };
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None).await;
    assert!(matches!(
        result,
        Err(WatchError::Step(StepFailure::Prepare))
    ));
    assert_eq!(
        outputs
            .snapshots
            .iter()
            .map(|snapshot| snapshot.state)
            .collect::<Vec<_>>(),
        vec![SnapshotState::Failed]
    );
    assert_eq!(outputs.calls.last().map(String::as_str), Some("release"));
    Ok(())
}

#[tokio::test]
async fn a_failure_before_any_snapshot_writes_none() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.fail = Some(StepFailure::Prepare);
    let mut outputs = FakeOutputs::default();
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None).await;
    assert!(matches!(
        result,
        Err(WatchError::Step(StepFailure::Prepare))
    ));
    assert_eq!(outputs.snapshots, Vec::new(), "the empty claim is released");
    Ok(())
}

const KEEP: &str = "/kept-calls";

fn bundle_path() -> PathBuf {
    Path::new(KEEP).join("meeting")
}

fn keep_call() -> String {
    format!("keep_session {}", bundle_path().display())
}

fn position(calls: &[String], call: &str) -> Result<usize, String> {
    calls
        .iter()
        .position(|made| made == call)
        .ok_or(format!("{call} was not made: {calls:?}"))
}

async fn run_keeping(
    steps: &FakeSteps,
    outputs: &mut FakeOutputs,
) -> Result<super::WatchOutcome, WatchError<StepFailure>> {
    run_watch(
        steps,
        outputs,
        Path::new(VIDEO),
        None,
        Some(Path::new(KEEP)),
    )
    .await
}

#[tokio::test]
async fn the_bundle_comes_after_the_outputs_and_before_the_close() -> TestResult {
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs::default();
    let outcome = run_keeping(&steps, &mut outputs)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        outcome.kept.as_ref().map(|kept| kept.path.clone()),
        Some(bundle_path())
    );
    assert_eq!(outcome.kept.and_then(|kept| kept.failure), None);
    assert!(outcome.session_closed);
    assert!(outputs.calls.contains(&"finalise".to_owned()));
    assert!(!outputs.calls.contains(&"release".to_owned()));
    let calls = steps.calls();
    assert!(position(&calls, &keep_call())? < position(&calls, "close_session")?);
    Ok(())
}

#[tokio::test]
async fn without_the_option_no_bundle_is_written() -> TestResult {
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs::default();
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(outcome.kept, None);
    assert!(!outputs.calls.contains(&"prepare_keep".to_owned()));
    assert!(!steps.calls().iter().any(|call| call.starts_with("keep_")));
    Ok(())
}

#[tokio::test]
async fn a_bundle_that_cannot_be_written_leaves_the_outputs_and_the_session_open() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.keep_failure = Some(FailureCode::StorageIo);
    let mut outputs = FakeOutputs::default();
    let outcome = run_keeping(&steps, &mut outputs)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        outcome.kept.as_ref().and_then(|kept| kept.failure),
        Some(FailureCode::StorageIo)
    );
    assert!(!outcome.session_closed, "the work is not lost");
    assert!(!steps.calls().contains(&"close_session".to_owned()));
    assert!(
        !outputs.calls.contains(&"release".to_owned()),
        "the outputs stand"
    );
    Ok(())
}

#[tokio::test]
async fn a_keep_folder_that_is_refused_stops_the_run_before_any_session() -> TestResult {
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs {
        keep_refused: Some(WatchOutputError::Exists(WatchOutputKind::Kept)),
        ..FakeOutputs::default()
    };
    let result = run_keeping(&steps, &mut outputs).await;
    assert!(matches!(
        result,
        Err(WatchError::Output(WatchOutputError::Exists(
            WatchOutputKind::Kept
        )))
    ));
    assert_eq!(
        outputs.calls,
        vec!["claim", "prepare_keep", "release"],
        "the claim comes first and is given back"
    );
    assert_eq!(steps.calls(), vec!["check_source", "check_capabilities"]);
    Ok(())
}

#[tokio::test]
async fn a_rerun_refused_for_its_keep_folder_leaves_the_interrupted_job_alone() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.unfinished = Some(UnfinishedWatch {
        session: FakeSteps::session()?,
        job: JobId::parse("job_fedcba9876543210")?,
    });
    let mut outputs = FakeOutputs {
        taken: true,
        keep_refused: Some(WatchOutputError::Exists(WatchOutputKind::Kept)),
        ..FakeOutputs::default()
    };
    let result = run_keeping(&steps, &mut outputs).await;
    assert!(matches!(result, Err(WatchError::Output(_))));
    let calls = steps.calls();
    assert!(!calls.contains(&"close_session".to_owned()));
    assert!(!calls.iter().any(|call| call.starts_with("prepare")));
    Ok(())
}

#[tokio::test]
async fn a_stopped_run_keeps_nothing() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.stopped = true;
    let mut outputs = FakeOutputs::default();
    let result = run_keeping(&steps, &mut outputs).await;
    assert!(matches!(result, Err(WatchError::Stopped { .. })));
    let calls = steps.calls();
    assert!(!calls.iter().any(|call| call.starts_with("keep_")));
    assert!(!calls.contains(&"close_session".to_owned()));
    Ok(())
}

#[tokio::test]
async fn a_run_that_fails_before_the_seal_keeps_nothing_and_closes_as_before() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.fail = Some(StepFailure::Prepare);
    let mut outputs = FakeOutputs::default();
    let result = run_keeping(&steps, &mut outputs).await;
    assert!(matches!(
        result,
        Err(WatchError::Step(StepFailure::Prepare))
    ));
    let calls = steps.calls();
    assert!(!calls.iter().any(|call| call.starts_with("keep_")));
    assert!(calls.contains(&"close_session".to_owned()));
    Ok(())
}

#[tokio::test]
async fn a_final_write_that_fails_on_a_sealed_job_still_keeps_the_recording() -> TestResult {
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs {
        final_write_fails: true,
        ..FakeOutputs::default()
    };
    let result = run_keeping(&steps, &mut outputs).await;
    let Err(WatchError::FailedAfterKeep {
        error,
        kept,
        session_closed,
        ..
    }) = result
    else {
        return Err(format!("{result:?}").into());
    };
    assert!(
        matches!(
            *error,
            WatchError::Output(WatchOutputError::Held(WatchOutputKind::Transcript))
        ),
        "the failed output is reported as it is today"
    );
    assert_eq!(kept.path, bundle_path());
    assert_eq!(kept.failure, None);
    assert!(session_closed, "the bundle is committed, so it closes");
    let calls = steps.calls();
    assert!(position(&calls, &keep_call())? < position(&calls, "close_session")?);
    Ok(())
}

#[tokio::test]
async fn a_final_write_and_a_bundle_that_both_fail_leave_the_session_open() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.keep_failure = Some(FailureCode::StorageIo);
    let mut outputs = FakeOutputs {
        final_write_fails: true,
        ..FakeOutputs::default()
    };
    let result = run_keeping(&steps, &mut outputs).await;
    let Err(WatchError::FailedAfterKeep {
        kept,
        session_closed,
        ..
    }) = result
    else {
        return Err(format!("{result:?}").into());
    };
    assert_eq!(kept.failure, Some(FailureCode::StorageIo));
    assert!(!session_closed);
    assert!(!steps.calls().contains(&"close_session".to_owned()));
    Ok(())
}

#[tokio::test]
async fn a_final_write_that_fails_without_the_option_closes_as_before() -> TestResult {
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs {
        final_write_fails: true,
        ..FakeOutputs::default()
    };
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None, None).await;
    assert!(matches!(
        result,
        Err(WatchError::Output(WatchOutputError::Held(_)))
    ));
    assert!(steps.calls().contains(&"close_session".to_owned()));
    Ok(())
}

/// The rerun table: the run that finishes does what its own command line
/// says, and finds its job by the output names alone.
#[tokio::test]
async fn a_rerun_keeps_where_its_own_command_line_says() -> TestResult {
    for keep in [Some("/a"), Some("/b"), None] {
        let mut steps = FakeSteps::new()?;
        steps.unfinished = Some(UnfinishedWatch {
            session: FakeSteps::session()?,
            job: JobId::parse("job_fedcba9876543210")?,
        });
        let mut outputs = FakeOutputs {
            taken: true,
            ..FakeOutputs::default()
        };
        let outcome = run_watch(
            &steps,
            &mut outputs,
            Path::new(VIDEO),
            None,
            keep.map(Path::new),
        )
        .await
        .map_err(|error| format!("{error:?}"))?;
        let calls = steps.calls();
        assert!(calls.contains(&"prepare job_fedcba9876543210".to_owned()));
        assert!(!calls.contains(&"open_session".to_owned()));
        assert_eq!(
            outcome.kept.map(|kept| kept.path),
            keep.map(|folder| Path::new(folder).join("meeting"))
        );
        assert!(outcome.session_closed);
    }
    Ok(())
}
