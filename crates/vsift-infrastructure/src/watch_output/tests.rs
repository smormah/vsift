use std::{error::Error, ffi::OsStr, fs, path::Path, time::Duration};

use vsift_application::{
    ClaimedOutputs, IdentifierGenerationError, IdentifierSource, ScreenChangeEntry,
    ScreenChangeSummary, SnapshotState, TranscriptEntry, WatchDocuments, WatchOutputError,
    WatchOutputKind, WatchOutputs as _, WatchProvenance, WatchSnapshot, WrittenFrame,
};
use vsift_domain::{
    CandidateReason, MediaTime, OperationId, SessionId, TranscriptRevisionId, WatchOutputNames,
};

use super::{
    FolderWatchOutputs, PARTIAL_SUFFIX, WatchClaim, hex_sha256, render_screen_changes,
    render_subrip, render_transcript,
};
use crate::test_folder::TestFolder;

type TestResult = Result<(), Box<dyn Error>>;

fn names() -> Result<WatchOutputNames, Box<dyn Error>> {
    Ok(WatchOutputNames::from_video_name(OsStr::new(
        "meeting.mp4",
    ))?)
}

fn provenance() -> WatchProvenance {
    WatchProvenance {
        video_name: "meeting.mp4".to_owned(),
        made_by: "VSift 0.3.0".to_owned(),
        made_at: "2026-11-20 14:05 UTC".to_owned(),
        video_sha256: "a".repeat(64),
        session_id: "ses_0123456789abcdef".to_owned(),
    }
}

fn line(start_s: u64, end_s: u64, text: &str) -> TranscriptEntry {
    TranscriptEntry::Line {
        start: MediaTime::from_micros(start_s * 1_000_000),
        end: MediaTime::from_micros(end_s * 1_000_000),
        text: text.to_owned(),
    }
}

/// The temporary files' random parts, fixed so a test can take a name first.
struct FixedRandom;

impl IdentifierSource for FixedRandom {
    fn session_id(&self) -> Result<SessionId, IdentifierGenerationError> {
        SessionId::parse("ses_0123456789abcdef")
            .map_err(|_| IdentifierGenerationError::NonCanonical)
    }

    fn operation_id(&self) -> Result<OperationId, IdentifierGenerationError> {
        OperationId::parse("op_00000000000000000000000000000000")
            .map_err(|_| IdentifierGenerationError::NonCanonical)
    }
}

/// The temporary name [`FixedRandom`] gives the transcript.
const FIXED_PARTIAL: &str = ".meeting.txt.0000000000000000.vsift-partial";

fn fixed_outputs() -> FolderWatchOutputs {
    let mut outputs = FolderWatchOutputs::new(Vec::new());
    outputs.identifiers = Box::new(FixedRandom);
    outputs
}

fn snapshot(published_s: u64) -> WatchSnapshot {
    WatchSnapshot {
        state: SnapshotState::Running,
        provenance: provenance(),
        revision: None,
        entries: vec![line(1, 4, "Good morning.")],
        published_until: MediaTime::from_micros(published_s * 1_000_000),
        duration: Some(MediaTime::from_micros(2_880 * 1_000_000)),
    }
}

/// The names in `folder` that are temporary files.
fn partials(folder: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    Ok(fs::read_dir(folder)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|name| name.ends_with(PARTIAL_SUFFIX))
        .collect())
}

#[test]
fn a_claim_creates_the_three_outputs_and_each_write_replaces_the_whole_file() -> TestResult {
    let folder = TestFolder::new("vsift-watch-claim")?;
    let mut claim = WatchClaim::claim(&folder, &names()?)?;
    assert_eq!(fs::read_to_string(folder.join("meeting.txt"))?, "");
    claim.write_transcript("text\n", "r1", Duration::ZERO)?;
    claim.write_subtitles("1\n", "r2", Duration::ZERO)?;
    assert_eq!(fs::read_to_string(folder.join("meeting.txt"))?, "text\n");
    assert_eq!(fs::read_to_string(folder.join("meeting.srt"))?, "1\n");
    assert!(folder.join("meeting-vsift").is_dir());
    claim.write_transcript("again\n", "r3", Duration::ZERO)?;
    assert_eq!(fs::read_to_string(folder.join("meeting.txt"))?, "again\n");
    assert_eq!(
        partials(&folder)?,
        Vec::<String>::new(),
        "no temporary file is left"
    );
    Ok(())
}

#[test]
fn a_snapshot_says_it_is_unfinished_and_the_final_write_does_not() -> TestResult {
    let folder = TestFolder::new("vsift-watch-snapshot")?;
    let mut outputs = fixed_outputs();
    outputs.claim(&folder.join("meeting.mp4"), None, &names()?)?;
    outputs.write_snapshot(&snapshot(750))?;
    let text = fs::read_to_string(folder.join("meeting.txt"))?;
    assert!(
        text.starts_with("IN PROGRESS: transcribed up to 00:12:30 of 00:48:00. VSift is still working; this file is replaced when it finishes.\n"),
        "{text}"
    );
    assert!(text.contains("[00:00:01] Good morning."), "{text}");
    let subtitles = fs::read_to_string(folder.join("meeting.srt"))?;
    assert!(
        subtitles.ends_with(
            "2\n00:12:30,000 --> 00:48:00,000\n(VSift is still transcribing from here)\n\n"
        ),
        "{subtitles}"
    );
    assert_eq!(partials(&folder)?, Vec::<String>::new());

    outputs.finalise(&documents(true)?)?;
    let text = fs::read_to_string(folder.join("meeting.txt"))?;
    assert!(!text.contains("IN PROGRESS"), "{text}");
    let subtitles = fs::read_to_string(folder.join("meeting.srt"))?;
    assert!(!subtitles.contains("still transcribing"), "{subtitles}");
    Ok(())
}

/// The markers stay true (the planner's condition of 2026-10-29): a
/// stopped run's files say where it stopped and that the same command
/// continues it, a failed run's that they will not be finished; neither
/// says `VSift` is still working.
#[test]
fn each_state_says_what_became_of_the_run() {
    let render = |state| {
        let snapshot = WatchSnapshot {
            state,
            ..snapshot(750)
        };
        (
            super::render_transcript_snapshot(&snapshot),
            super::render_subrip_snapshot(&snapshot),
        )
    };
    let (text, subtitles) = render(SnapshotState::Stopped);
    assert!(
        text.starts_with("STOPPED: transcribed up to 00:12:30 of 00:48:00, where the run was stopped. Run the same command again to continue;"),
        "{text}"
    );
    assert!(
        subtitles.ends_with("(VSift stopped here; run the same command again to continue)\n\n"),
        "{subtitles}"
    );
    let (failed, failed_subtitles) = render(SnapshotState::Failed);
    assert!(
        failed.starts_with("STOPPED WITH AN ERROR: transcribed up to 00:12:30 of 00:48:00. This file will not be finished;"),
        "{failed}"
    );
    assert!(
        failed_subtitles.ends_with(
            "(VSift stopped here with an error; these subtitles will not be finished)\n\n"
        ),
        "{failed_subtitles}"
    );
    for written in [&text, &subtitles, &failed, &failed_subtitles] {
        assert!(!written.contains("still"), "{written}");
    }
    let (running, _) = render(SnapshotState::Running);
    assert!(running.contains("VSift is still working"), "{running}");
}

/// The planner's condition B: a temporary name taken in advance fails the
/// update; it is never written through, and the claimed file keeps what it
/// held.
#[test]
fn a_temporary_name_taken_in_advance_fails_the_update() -> TestResult {
    let folder = TestFolder::new("vsift-watch-partial-taken")?;
    let mut outputs = fixed_outputs();
    outputs.claim(&folder.join("meeting.mp4"), None, &names()?)?;
    fs::write(folder.join(FIXED_PARTIAL), "someone else's")?;
    assert!(matches!(
        outputs.write_snapshot(&snapshot(30)),
        Err(WatchOutputError::Exists(WatchOutputKind::Transcript))
    ));
    assert_eq!(
        fs::read_to_string(folder.join(FIXED_PARTIAL))?,
        "someone else's"
    );
    assert_eq!(fs::read_to_string(folder.join("meeting.txt"))?, "");
    Ok(())
}

/// The planner's condition B: a link planted at the temporary name is never
/// followed.
#[cfg(unix)]
#[test]
fn a_link_planted_at_the_temporary_name_is_not_followed() -> TestResult {
    let folder = TestFolder::new("vsift-watch-partial-link")?;
    let mut outputs = fixed_outputs();
    outputs.claim(&folder.join("meeting.mp4"), None, &names()?)?;
    let target = folder.join("elsewhere.txt");
    std::os::unix::fs::symlink(&target, folder.join(FIXED_PARTIAL))?;
    assert!(matches!(
        outputs.write_snapshot(&snapshot(30)),
        Err(WatchOutputError::Exists(WatchOutputKind::Transcript))
    ));
    assert!(!target.exists(), "nothing was written through the link");
    Ok(())
}

/// A reader never sees a partial file: every read, however it falls between
/// the updates, is one whole snapshot.
#[cfg(unix)]
#[test]
fn a_reader_never_sees_a_partial_file() -> TestResult {
    let folder = TestFolder::new("vsift-watch-reader")?;
    let mut outputs = FolderWatchOutputs::new(Vec::new());
    outputs.claim(&folder.join("meeting.mp4"), None, &names()?)?;
    outputs.write_snapshot(&snapshot(1))?;
    let path = folder.join("meeting.txt");
    let wholes: Vec<String> = (1..=40)
        .map(|seconds| super::render_transcript_snapshot(&snapshot(seconds)))
        .collect();
    let reader = {
        let path = path.clone();
        std::thread::spawn(move || {
            (0..400)
                .map(|_| fs::read_to_string(&path).unwrap_or_default())
                .collect::<Vec<_>>()
        })
    };
    for seconds in 1..=40 {
        outputs.write_snapshot(&snapshot(seconds))?;
    }
    let reads = reader.join().map_err(|_| "the reader stopped")?;
    for read in reads {
        assert!(wholes.contains(&read), "a partial read: {read:?}");
    }
    Ok(())
}

/// A file another program holds open (a Windows editor that locks it) skips
/// the snapshot, and the final write fails with `Held` after its wait,
/// leaving the last snapshot (known limit L-237).
#[cfg(windows)]
#[test]
fn a_file_held_open_skips_the_snapshot_and_fails_the_final_write() -> TestResult {
    use std::os::windows::fs::OpenOptionsExt as _;

    const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    let folder = TestFolder::new("vsift-watch-held")?;
    let mut outputs = FolderWatchOutputs::new(Vec::new());
    outputs.held_wait = Duration::from_millis(600);
    outputs.claim(&folder.join("meeting.mp4"), None, &names()?)?;
    outputs.write_snapshot(&snapshot(30))?;
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ_WRITE)
        .open(folder.join("meeting.txt"))?;
    assert!(matches!(
        outputs.write_snapshot(&snapshot(60)),
        Err(WatchOutputError::Held(WatchOutputKind::Transcript))
    ));
    assert!(matches!(
        outputs.finalise(&documents(true)?),
        Err(WatchOutputError::Held(WatchOutputKind::Transcript))
    ));
    drop(held);
    let text = fs::read_to_string(folder.join("meeting.txt"))?;
    assert!(
        text.starts_with("IN PROGRESS: transcribed up to 00:00:30"),
        "the last snapshot stays: {text}"
    );
    assert_eq!(partials(&folder)?, Vec::<String>::new());
    Ok(())
}

#[test]
fn a_rerun_takes_over_only_what_the_interrupted_run_created() -> TestResult {
    let folder = TestFolder::new("vsift-watch-take-over")?;
    let source = folder.join("artifact.png");
    fs::write(&source, b"png bytes")?;
    let digest = hex_sha256(b"png bytes");
    let mut first = FolderWatchOutputs::new(Vec::new());
    let claimed = first.claim(&folder.join("meeting.mp4"), None, &names()?)?;
    first
        .claimed()?
        .copy_frame(&source, "00-00-01.000.png", &digest)?;

    let mut again = FolderWatchOutputs::new(Vec::new());
    assert!(matches!(
        again.claim(&folder.join("meeting.mp4"), None, &names()?),
        Err(WatchOutputError::Exists(WatchOutputKind::Transcript))
    ));
    let planned = again.planned(&folder.join("meeting.mp4"), None, &names()?)?;
    assert_eq!(planned, claimed);
    again.reclaim(&planned)?;
    let copied = again
        .claimed()?
        .copy_frame(&source, "00-00-01.000.png", &digest)?;
    assert_eq!(
        fs::read(copied)?,
        b"png bytes",
        "the same frame is accepted"
    );
    again.release();
    assert!(
        folder
            .join("meeting-vsift")
            .join("00-00-01.000.png")
            .is_file(),
        "a release never removes what the earlier run wrote"
    );

    fs::remove_file(&claimed.subtitles)?;
    fs::create_dir(&claimed.subtitles)?;
    assert!(matches!(
        FolderWatchOutputs::new(Vec::new()).reclaim(&ClaimedOutputs { ..claimed }),
        Err(WatchOutputError::Exists(WatchOutputKind::Subtitles))
    ));
    Ok(())
}

#[test]
fn a_taken_name_is_refused_and_nothing_is_replaced_or_left_behind() -> TestResult {
    for taken in ["meeting.txt", "meeting.srt", "meeting-vsift"] {
        let folder = TestFolder::new("vsift-watch-taken")?;
        fs::write(folder.join(taken), "the person's own file")?;
        let refused = WatchClaim::claim(&folder, &names()?);
        assert!(
            matches!(refused, Err(WatchOutputError::Exists(_))),
            "{taken}: {refused:?}"
        );
        assert_eq!(
            fs::read_to_string(folder.join(taken))?,
            "the person's own file"
        );
        let left: Vec<_> = fs::read_dir(&*folder)?.collect::<Result<_, _>>()?;
        assert_eq!(left.len(), 1, "{taken}: only the person's file is there");
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_link_at_an_output_name_is_refused_not_followed() -> TestResult {
    let folder = TestFolder::new("vsift-watch-link")?;
    let target = folder.join("elsewhere.txt");
    std::os::unix::fs::symlink(&target, folder.join("meeting.txt"))?;
    assert!(matches!(
        WatchClaim::claim(&folder, &names()?),
        Err(WatchOutputError::Exists(WatchOutputKind::Transcript))
    ));
    assert!(!target.exists(), "nothing was written through the link");
    Ok(())
}

#[test]
fn a_release_removes_what_the_claim_created_and_never_written_text() -> TestResult {
    let folder = TestFolder::new("vsift-watch-release")?;
    let mut claim = WatchClaim::claim(&folder, &names()?)?;
    claim.write_screen_changes("list", "r1")?;
    claim.release();
    assert_eq!(fs::read_dir(&*folder)?.count(), 0);

    let mut claim = WatchClaim::claim(&folder, &names()?)?;
    claim.write_transcript("kept", "r2", Duration::ZERO)?;
    claim.release();
    assert_eq!(fs::read_to_string(folder.join("meeting.txt"))?, "kept");
    assert!(!folder.join("meeting.srt").exists());
    Ok(())
}

#[test]
fn a_frame_is_copied_only_with_its_recorded_digest() -> TestResult {
    let folder = TestFolder::new("vsift-watch-frame")?;
    let source = folder.join("artifact.png");
    fs::write(&source, b"png bytes")?;
    let digest = hex_sha256(b"png bytes");
    let mut claim = WatchClaim::claim(&folder, &names()?)?;
    let copied = claim.copy_frame(&source, "00-00-01.000.png", &digest)?;
    assert_eq!(fs::read(&copied)?, b"png bytes");
    assert!(matches!(
        claim.copy_frame(&source, "00-00-02.000.png", &"0".repeat(64)),
        Err(WatchOutputError::FrameMismatch)
    ));
    assert!(matches!(
        claim.copy_frame(&source, "00-00-01.000.png", &digest),
        Err(WatchOutputError::Exists(WatchOutputKind::FolderContent))
    ));
    Ok(())
}

#[test]
fn subtitles_for_a_silent_video_are_not_left_behind() -> TestResult {
    let folder = TestFolder::new("vsift-watch-silent")?;
    let mut claim = WatchClaim::claim(&folder, &names()?)?;
    claim.discard_subtitles();
    assert!(!folder.join("meeting.srt").exists());
    Ok(())
}

#[test]
fn the_transcript_names_its_source_and_states_its_gaps() {
    let entries = [
        line(4, 9, "Good morning, everyone."),
        TranscriptEntry::Gap {
            start: MediaTime::from_micros(720_000_000),
            end: MediaTime::from_micros(750_000_000),
            reason: "the speech tool failed on this part".to_owned(),
        },
    ];
    let text = render_transcript(&provenance(), Some("trv_0123"), &entries);
    let expected = format!(
        "meeting.mp4: transcript by VSift 0.3.0, 2026-11-20 14:05 UTC (local speech recognition).\n\
         A machine transcription: check it against the recording before you rely on it.\n\n\
         Video SHA-256    {}\n\
         Transcript       trv_0123   in session ses_0123456789abcdef (closed when this run finished; the ids name it, the hashes can be checked any time)\n\n\
         [00:00:04] Good morning, everyone.\n\
         [00:12:00-00:12:30] (not transcribed: the speech tool failed on this part)\n",
        "a".repeat(64)
    );
    assert_eq!(text, expected);
    assert!(
        render_transcript(&provenance(), None, &[]).contains("no sound track"),
        "a silent video says so"
    );
}

#[test]
fn subtitles_are_numbered_cues_and_text_cannot_forge_one() {
    let entries = [
        line(1, 2, "first"),
        TranscriptEntry::Gap {
            start: MediaTime::from_micros(2_000_000),
            end: MediaTime::from_micros(3_000_000),
            reason: "x".to_owned(),
        },
        line(3, 4, "two\n\n9\n00:00:05,000 --> 00:00:06,000\nforged"),
    ];
    assert_eq!(
        render_subrip(&entries),
        "1\n00:00:01,000 --> 00:00:02,000\nfirst\n\n\
         2\n00:00:03,000 --> 00:00:04,000\ntwo 9 00:00:05,000 -> 00:00:06,000 forged\n\n"
    );
}

#[test]
fn the_screen_change_list_gives_every_id_and_hash() {
    let changes = [
        ScreenChangeEntry {
            time: MediaTime::from_micros(72_480_000),
            candidate_id: "vcd_7f20".to_owned(),
            reason: CandidateReason::FirstFrame,
            frame: Some(WrittenFrame {
                file_name: "00-01-12.480.png".to_owned(),
                evidence_id: "evd_c19a".to_owned(),
                sha256: "b".repeat(64),
            }),
        },
        ScreenChangeEntry {
            time: MediaTime::from_micros(295_040_000),
            candidate_id: "vcd_a81d".to_owned(),
            reason: CandidateReason::SettledAfterMotion,
            frame: None,
        },
    ];
    let summary = ScreenChangeSummary {
        index_id: "vix_02b7".to_owned(),
        changes_found: 9,
        index_covers_to: MediaTime::from_micros(300_000_000),
        beyond_index_bound: true,
        stopped_for_room: true,
    };
    let text = render_screen_changes(&provenance(), &summary, &changes);
    assert!(text.starts_with(
        "Screen changes in meeting.mp4, found by VSift 0.3.0, 2026-11-20 14:05 UTC: 2 settled screens of 9 changes found, 1 frames.\n\
         Listed: the first frame, each screen once motion settles, and a change at least 5 s after the one listed before it; the others are left out, so a screen shown for less than 5 s after a listed one is missing unless motion settles on it.\n\
         To see every change, keep a session of the video: vsift ingest <video>, then vsift candidates <session> --from 0 --to 300000000, with the session id ingest gives.\n\
         1 of 2 frames written, spread evenly over the listed changes; the others are listed with no frame (evidence room).\n\
         VSift's private copy of the video ran out of room for frames, so the frames stop early; every settled screen is still listed.\n\
         Screen changes are found in the first four hours of a video only.\n"
    ));
    assert!(text.contains("Visual index     vix_02b7   in session ses_0123456789abcdef"));
    assert!(text.contains("00:01:12    first frame           00-01-12.480.png          vcd_7f20"));
    assert!(text.contains(&format!("evd_c19a{}  {}", " ".repeat(28), "b".repeat(64))));
    assert!(text.contains("00:04:55    settled after motion  no frame (evidence room)  vcd_a81d"));
}

fn documents(revision: bool) -> Result<WatchDocuments, Box<dyn Error>> {
    Ok(WatchDocuments {
        provenance: provenance(),
        transcript_revision: if revision {
            Some(TranscriptRevisionId::parse(
                "trv_1f2e3d4c5b6a79880f1e2d3c4b5a6978",
            )?)
        } else {
            None
        },
        entries: vec![line(1, 4, "Good morning.")],
    })
}

#[test]
fn the_folder_port_writes_beside_the_video_and_finishes_every_file() -> TestResult {
    let folder = TestFolder::new("vsift-watch-port")?;
    let video = folder.join("meeting.mp4");
    let mut outputs = FolderWatchOutputs::new(Vec::new());
    let claimed = outputs.claim(&video, None, &names()?)?;
    assert_eq!(
        claimed.transcript.file_name(),
        Some(OsStr::new("meeting.txt"))
    );
    let list = outputs.write_screen_changes(
        &provenance(),
        &ScreenChangeSummary {
            index_id: "none".to_owned(),
            changes_found: 0,
            index_covers_to: MediaTime::from_micros(0),
            beyond_index_bound: false,
            stopped_for_room: false,
        },
        &[],
    )?;
    let finished = outputs.finalise(&documents(true)?)?;
    let text = fs::read_to_string(&finished.transcript.path)?;
    assert!(text.contains("[00:00:01] Good morning."), "{text}");
    assert_eq!(
        finished.transcript.sha256.as_deref(),
        Some(hex_sha256(text.as_bytes()).as_str())
    );
    assert!(finished.subtitles.is_some());
    assert!(list.path.is_file());
    outputs.release();
    assert!(
        list.path.is_file(),
        "a release after the files are finished removes nothing"
    );
    Ok(())
}

#[test]
fn a_video_with_no_sound_track_leaves_no_subtitles_file() -> TestResult {
    let folder = TestFolder::new("vsift-watch-port-silent")?;
    let mut outputs = FolderWatchOutputs::new(Vec::new());
    outputs.claim(&folder.join("meeting.mp4"), None, &names()?)?;
    let finished = outputs.finalise(&documents(false)?)?;
    assert!(finished.subtitles.is_none());
    assert!(!folder.join("meeting.srt").exists());
    Ok(())
}

#[test]
fn an_output_folder_is_created_once_and_never_inside_vsifts_roots() -> TestResult {
    let folder = TestFolder::new("vsift-watch-port-output")?;
    let root = fs::canonicalize(&*folder)?.join("sessions");
    fs::create_dir(&root)?;
    let video = folder.join("meeting.mp4");
    let mut outputs = FolderWatchOutputs::new(vec![root.clone()]);
    assert!(matches!(
        outputs.claim(&video, Some(&root.join("inside")), &names()?),
        Err(WatchOutputError::InsideVsift)
    ));
    assert!(matches!(
        outputs.claim(
            &video,
            Some(&folder.join("no-parent").join("out")),
            &names()?
        ),
        Err(WatchOutputError::FolderUnusable)
    ));
    fs::write(folder.join("a-file"), "")?;
    assert!(matches!(
        outputs.claim(&video, Some(&folder.join("a-file")), &names()?),
        Err(WatchOutputError::FolderUnusable)
    ));
    let claimed = outputs.claim(&video, Some(&folder.join("out")), &names()?)?;
    assert!(
        claimed
            .transcript
            .starts_with(fs::canonicalize(folder.join("out"))?)
    );
    Ok(())
}

#[test]
fn a_release_after_a_failure_leaves_only_what_was_there() -> TestResult {
    let folder = TestFolder::new("vsift-watch-port-release")?;
    let mut outputs = FolderWatchOutputs::new(Vec::new());
    outputs.claim(&folder.join("meeting.mp4"), None, &names()?)?;
    outputs.release();
    let left: Vec<_> = fs::read_dir(&*folder)?.collect::<Result<_, _>>()?;
    assert!(left.is_empty(), "{left:?}");
    assert!(matches!(
        outputs.finalise(&documents(true)?),
        Err(WatchOutputError::Refused(WatchOutputKind::FolderContent, _))
    ));
    Ok(())
}

fn keep_names(video: &str) -> Result<WatchOutputNames, Box<dyn Error>> {
    Ok(WatchOutputNames::from_video_name(OsStr::new(video))?)
}

fn entries(folder: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    Ok(fs::read_dir(folder)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, std::io::Error>>()?)
}

#[test]
fn a_keep_target_is_named_after_the_stem_and_leaves_nothing_there() -> TestResult {
    let folder = TestFolder::new("vsift-keep-target")?;
    let target = fixed_outputs().prepare_keep(&folder, &keep_names("meeting.mp4")?)?;
    assert_eq!(target, fs::canonicalize(&*folder)?.join("meeting"));
    assert_eq!(entries(&folder)?, Vec::<String>::new());
    Ok(())
}

#[test]
fn a_new_keep_folder_is_created_with_its_parent_required() -> TestResult {
    let base = TestFolder::new("vsift-keep-new")?;
    let new = base.join("kept");
    fixed_outputs().prepare_keep(&new, &keep_names("meeting.mp4")?)?;
    assert!(new.is_dir());
    let result = fixed_outputs().prepare_keep(&base.join("no/parent"), &keep_names("meeting.mp4")?);
    assert!(matches!(result, Err(WatchOutputError::KeepFolderUnusable)));
    Ok(())
}

#[test]
fn a_taken_name_is_refused_and_replaces_nothing() -> TestResult {
    let folder = TestFolder::new("vsift-keep-taken")?;
    fs::create_dir(folder.join("a folder"))?;
    fs::write(folder.join("a folder").join("inside.txt"), "kept")?;
    fs::write(folder.join("a file"), "kept")?;
    for video in ["a folder.mp4", "a file.mp4"] {
        let result = fixed_outputs().prepare_keep(&folder, &keep_names(video)?);
        assert!(
            matches!(result, Err(WatchOutputError::Exists(WatchOutputKind::Kept))),
            "{result:?}"
        );
    }
    assert_eq!(
        fs::read_to_string(folder.join("a folder").join("inside.txt"))?,
        "kept"
    );
    assert_eq!(fs::read_to_string(folder.join("a file"))?, "kept");
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_link_at_the_name_is_taken_and_never_followed() -> TestResult {
    let folder = TestFolder::new("vsift-keep-link")?;
    let elsewhere = TestFolder::new("vsift-keep-elsewhere")?;
    std::os::unix::fs::symlink(elsewhere.join("gone"), folder.join("meeting"))?;
    let result = fixed_outputs().prepare_keep(&folder, &keep_names("meeting.mp4")?);
    assert!(matches!(
        result,
        Err(WatchOutputError::Exists(WatchOutputKind::Kept))
    ));
    assert_eq!(entries(&elsewhere)?, Vec::<String>::new());
    Ok(())
}

#[test]
fn something_that_is_not_a_folder_is_refused() -> TestResult {
    let base = TestFolder::new("vsift-keep-file")?;
    let file = base.join("not a folder");
    fs::write(&file, "text")?;
    let result = fixed_outputs().prepare_keep(&file, &keep_names("meeting.mp4")?);
    assert!(matches!(result, Err(WatchOutputError::KeepFolderUnusable)));
    assert_eq!(fs::read_to_string(&file)?, "text");
    Ok(())
}

#[test]
fn a_keep_folder_inside_vsifts_roots_is_refused_and_a_new_one_removed() -> TestResult {
    let root = TestFolder::new("vsift-keep-root")?;
    let mut outputs = FolderWatchOutputs::new(vec![fs::canonicalize(&*root)?]);
    let new = root.join("kept");
    let result = outputs.prepare_keep(&new, &keep_names("meeting.mp4")?);
    assert!(matches!(result, Err(WatchOutputError::KeepInsideVsift)));
    assert!(!new.exists(), "a folder this run made is removed");
    Ok(())
}

#[test]
fn a_name_a_bundle_may_not_have_is_refused_before_any_work() -> TestResult {
    let base = TestFolder::new("vsift-keep-device")?;
    let new = base.join("kept");
    let result = fixed_outputs().prepare_keep(&new, &keep_names("aux.v2.mp4")?);
    assert!(matches!(result, Err(WatchOutputError::KeepNameNotAllowed)));
    assert!(!new.exists(), "a folder this run made is removed");
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_folder_that_refuses_the_write_is_refused() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let folder = TestFolder::new("vsift-keep-readonly")?;
    fs::set_permissions(&*folder, fs::Permissions::from_mode(0o500))?;
    let result = fixed_outputs().prepare_keep(&folder, &keep_names("meeting.mp4")?);
    fs::set_permissions(&*folder, fs::Permissions::from_mode(0o700))?;
    assert!(matches!(
        result,
        Err(WatchOutputError::Refused(WatchOutputKind::Kept, _))
    ));
    Ok(())
}
