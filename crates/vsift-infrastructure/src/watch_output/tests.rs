use std::{error::Error, ffi::OsStr, fs};

use vsift_application::{
    ScreenChangeEntry, ScreenChangeSummary, TranscriptEntry, WatchDocuments, WatchOutputError,
    WatchOutputKind, WatchOutputs as _, WatchProvenance, WrittenFrame,
};
use vsift_domain::{MediaTime, TranscriptRevisionId, WatchOutputNames};

use super::{
    FolderWatchOutputs, WatchClaim, hex_sha256, render_screen_changes, render_subrip,
    render_transcript,
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

#[test]
fn a_claim_creates_the_three_outputs_and_writes_each_once() -> TestResult {
    let folder = TestFolder::new("vsift-watch-claim")?;
    let mut claim = WatchClaim::claim(&folder, &names()?)?;
    claim.write_transcript("text\n")?;
    claim.write_subtitles("1\n")?;
    assert_eq!(fs::read_to_string(folder.join("meeting.txt"))?, "text\n");
    assert_eq!(fs::read_to_string(folder.join("meeting.srt"))?, "1\n");
    assert!(folder.join("meeting-vsift").is_dir());
    assert!(matches!(
        claim.write_transcript("again"),
        Err(WatchOutputError::Refused(WatchOutputKind::Transcript, _))
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
    claim.write_screen_changes("list")?;
    claim.release();
    assert_eq!(fs::read_dir(&*folder)?.count(), 0);

    let mut claim = WatchClaim::claim(&folder, &names()?)?;
    claim.write_transcript("kept")?;
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
            frame: Some(WrittenFrame {
                file_name: "00-01-12.480.png".to_owned(),
                evidence_id: "evd_c19a".to_owned(),
                sha256: "b".repeat(64),
            }),
        },
        ScreenChangeEntry {
            time: MediaTime::from_micros(295_040_000),
            candidate_id: "vcd_a81d".to_owned(),
            frame: None,
        },
    ];
    let summary = ScreenChangeSummary {
        index_id: "vix_02b7".to_owned(),
        beyond_index_bound: true,
        stopped_for_room: true,
    };
    let text = render_screen_changes(&provenance(), &summary, &changes);
    assert!(text.starts_with(
        "Screen changes in meeting.mp4, found by VSift 0.3.0, 2026-11-20 14:05 UTC: 2 changes, 1 frames.\n\
         1 of 2 frames written, spread evenly over the video; the others are listed without one.\n\
         VSift's private copy of the video ran out of room for frames, so the frames stop early; every change is still listed.\n\
         Screen changes are found in the first four hours of a video only.\n"
    ));
    assert!(text.contains("Visual index     vix_02b7   in session ses_0123456789abcdef"));
    assert!(text.contains("00:01:12    00-01-12.480.png    vcd_7f20"));
    assert!(text.contains(&format!("evd_c19a{}  {}", " ".repeat(28), "b".repeat(64))));
    assert!(text.contains("00:04:55    (not written)       vcd_a81d"));
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
        summary: ScreenChangeSummary {
            index_id: "none".to_owned(),
            beyond_index_bound: false,
            stopped_for_room: false,
        },
        changes: Vec::new(),
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
    let finished = outputs.finalise(&documents(true)?)?;
    let text = fs::read_to_string(&finished.transcript.path)?;
    assert!(text.contains("[00:00:01] Good morning."), "{text}");
    assert_eq!(
        finished.transcript.sha256.as_deref(),
        Some(hex_sha256(text.as_bytes()).as_str())
    );
    assert!(finished.subtitles.is_some());
    assert!(finished.screen_changes.path.is_file());
    outputs.release();
    assert!(
        finished.screen_changes.path.is_file(),
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
