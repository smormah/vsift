//! The frame step's backstop (ADR 0037 section 6, known limit L-234): a
//! session that refuses a frame for room ends the frames, not the run.

use std::path::PathBuf;

use vsift_domain::{EvidenceId, MediaTime, VisualCandidateId};

use super::{FrameTaker, WatchFrame, take_frames};
use crate::error::EngineError;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A change, as the tests name it: its position.
struct Change(usize);

/// Takes frames until `room` runs out, then answers as a full session does;
/// or fails every frame with `failure` once `room` runs out.
struct Fake {
    room: usize,
    failure: fn() -> EngineError,
    asked: Vec<usize>,
}

fn frame(position: usize) -> Result<WatchFrame, EngineError> {
    let id = |prefix: &str| format!("{prefix}{position:032}");
    Ok(WatchFrame {
        time: MediaTime::from_micros(u64::try_from(position).unwrap_or(u64::MAX) * 1_000_000),
        candidate_id: VisualCandidateId::parse(id("vcd_"))
            .map_err(|_| EngineError::EvidenceBudgetExhausted)?,
        evidence_id: EvidenceId::parse(id("evd_"))
            .map_err(|_| EngineError::EvidenceBudgetExhausted)?,
        path: PathBuf::from(format!("frame-{position}.png")),
        sha256: "0".repeat(64),
    })
}

impl FrameTaker<Change> for Fake {
    fn take(
        &mut self,
        change: &Change,
    ) -> impl Future<Output = Result<WatchFrame, EngineError>> + Send {
        self.asked.push(change.0);
        let answer = if self.asked.len() > self.room {
            Err((self.failure)())
        } else {
            frame(change.0)
        };
        std::future::ready(answer)
    }
}

fn changes(count: usize) -> Vec<Change> {
    (0..count).map(Change).collect()
}

#[tokio::test]
async fn a_session_out_of_room_ends_the_frames_not_the_run() -> TestResult {
    let mut fake = Fake {
        room: 3,
        failure: || EngineError::EvidenceBudgetExhausted,
        asked: Vec::new(),
    };
    let (taken, stopped) = take_frames(&mut fake, &changes(10), &[0, 2, 4, 6, 8]).await?;
    assert!(stopped, "the run says the frames stopped for room");
    assert_eq!(
        taken
            .iter()
            .map(|(position, _)| *position)
            .collect::<Vec<_>>(),
        vec![0, 2, 4],
        "the frames taken before the session was full are kept, at their positions"
    );
    assert_eq!(
        fake.asked,
        vec![0, 2, 4, 6],
        "no frame is asked for after the refusal"
    );
    Ok(())
}

#[tokio::test]
async fn every_frame_taken_when_the_room_holds() -> TestResult {
    let mut fake = Fake {
        room: usize::MAX,
        failure: || EngineError::EvidenceBudgetExhausted,
        asked: Vec::new(),
    };
    let (taken, stopped) = take_frames(&mut fake, &changes(4), &[0, 1, 2, 3]).await?;
    assert!(!stopped);
    assert_eq!(taken.len(), 4);
    Ok(())
}

#[tokio::test]
async fn any_other_failure_still_ends_the_run() {
    let mut fake = Fake {
        room: 1,
        failure: || EngineError::NoVideoStream,
        asked: Vec::new(),
    };
    let result = take_frames(&mut fake, &changes(3), &[0, 1, 2]).await;
    assert!(matches!(result, Err(EngineError::NoVideoStream)));
}
