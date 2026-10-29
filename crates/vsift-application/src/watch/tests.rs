//! The `watch` use case over fakes of its two ports (ADR 0037 section 14):
//! the order of the steps, refusals before any work, stated gaps, and the
//! frame bound with its even spread and its backstop.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Mutex,
};

use vsift_domain::{
    CandidateReason, EvidenceId, IdentifierError, JobId, MAX_WATCH_FRAMES, MediaTime, SessionId,
    SourceId, TimeRange, TranscriptRevisionId, VisualCandidateId, VisualIndexId, WatchOutputNames,
    spread_evenly,
};

use super::{
    ClaimedOutputs, FinalisedOutputs, FrameRefusal, OpenedWatchSession, PreparedRun,
    PreparedTranscript, ScreenChange, ScreenChanges, SessionFrame, TranscriptEntry, TranscriptLine,
    WatchDocuments, WatchError, WatchFile, WatchOutputError, WatchOutputKind, WatchOutputs,
    WatchProvenance, WatchSteps, run_watch,
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
struct FakeSteps {
    fail: Option<StepFailure>,
    transcript: Option<PreparedTranscript>,
    lines: Vec<TranscriptLine>,
    changes: Vec<ScreenChange>,
    evidence_slots: usize,
    /// Frames the session stores before it is full.
    frame_room: usize,
    calls: Mutex<Vec<String>>,
}

impl FakeSteps {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            fail: None,
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

    fn prepare(
        &self,
        _session: &SessionId,
    ) -> impl Future<Output = Result<PreparedRun, StepFailure>> + Send {
        let answer = (|| {
            self.step("prepare", StepFailure::Prepare)?;
            Ok(PreparedRun {
                job_id: JobId::parse("job_0123456789abcdef").map_err(|_| StepFailure::Prepare)?,
                transcript: self.transcript.clone(),
            })
        })();
        std::future::ready(answer)
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
    calls: Vec<String>,
    documents: Option<WatchDocuments>,
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
        self.documents = Some(documents.clone());
        Ok(FinalisedOutputs {
            transcript: file("meeting.txt"),
            subtitles: documents
                .transcript_revision
                .as_ref()
                .map(|_| file("meeting.srt")),
            screen_changes: file("screen-changes.txt"),
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
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        steps.calls(),
        vec![
            "check_source",
            "check_capabilities",
            "open_session",
            "prepare",
            "transcript_lines",
            "screen_changes",
            "evidence_slots_left",
            "take_frame vcd_00000000000000000000000000000000",
            "take_frame vcd_00000000000000000000000000000001",
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
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None).await;
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
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None).await;
    assert!(matches!(
        result,
        Err(WatchError::Output(WatchOutputError::Exists(
            WatchOutputKind::Transcript
        )))
    ));
    assert_eq!(
        outputs.calls,
        vec!["claim"],
        "nothing is written or released"
    );
    assert_eq!(
        steps.calls(),
        vec!["check_source", "check_capabilities"],
        "no session is opened"
    );
    Ok(())
}

#[tokio::test]
async fn a_source_that_is_not_a_video_file_refuses_first() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.fail = Some(StepFailure::Source);
    let mut outputs = FakeOutputs::default();
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None).await;
    assert!(matches!(result, Err(WatchError::Step(StepFailure::Source))));
    assert_eq!(steps.calls(), vec!["check_source"]);
    assert_eq!(outputs.calls, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn a_name_with_nothing_before_its_extension_refuses_before_the_claim() -> TestResult {
    let steps = FakeSteps::new()?;
    let mut outputs = FakeOutputs::default();
    let result = run_watch(&steps, &mut outputs, Path::new("/videos/...mp4"), None).await;
    assert!(matches!(result, Err(WatchError::UnusableName(_))));
    assert_eq!(outputs.calls, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn a_failure_after_the_claim_releases_it_and_closes_the_session() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.fail = Some(StepFailure::Prepare);
    let mut outputs = FakeOutputs::default();
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None).await;
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
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None).await;
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
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None)
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
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None)
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
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None)
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
        .documents
        .map(|documents| documents.changes)
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
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None)
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
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None)
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
    let documents = outputs.documents.ok_or("the files were finished")?;
    assert!(documents.summary.stopped_for_room);
    assert_eq!(documents.changes.len(), 10, "every change is still listed");
    Ok(())
}

#[tokio::test]
async fn any_other_frame_failure_ends_the_run_and_releases_the_claim() -> TestResult {
    let mut steps = FakeSteps::new()?;
    steps.fail = Some(StepFailure::Frame);
    let mut outputs = FakeOutputs::default();
    let result = run_watch(&steps, &mut outputs, Path::new(VIDEO), None).await;
    assert!(matches!(result, Err(WatchError::Step(StepFailure::Frame))));
    assert_eq!(outputs.calls, vec!["claim", "release"]);
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
    let outcome = run_watch(&steps, &mut outputs, Path::new(VIDEO), None)
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
    let documents = outputs.documents.ok_or("the files were finished")?;
    assert_eq!(documents.summary.changes_found, 6);
    assert_eq!(
        documents
            .changes
            .iter()
            .map(|entry| entry.reason)
            .collect::<Vec<_>>(),
        vec![
            CandidateReason::FirstFrame,
            CandidateReason::SettledAfterMotion,
            CandidateReason::VisualChange,
        ]
    );
    Ok(())
}
