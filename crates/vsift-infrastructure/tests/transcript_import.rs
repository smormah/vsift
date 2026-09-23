//! T-01/T-02 supplied-transcript parsing, the malformed-data policy and its
//! robustness properties.
//!
//! Structural variants live in `tests/data/transcripts`; byte-level variants
//! (byte-order marks, encodings, line endings, control characters) are inline
//! because the repository normalizes text files to LF.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use proptest::prelude::{Just, Strategy, prop, prop_assert, prop_assert_eq, prop_oneof, proptest};
use vsift_application::{SuppliedTranscriptError, whole_file_source_segment};
use vsift_domain::{
    CueMarkup, LanguageTag, MAX_CUE_TEXT_BYTES, MAX_SUPPLIED_TRANSCRIPT_BYTES, MAX_TRANSCRIPT_CUES,
    MediaTime, ParsedTranscript, SourceId, SpeakerLabel, TranscriptFormat, TranscriptImportError,
    TranscriptOffset, TranscriptRejection, TranscriptWarningKind, align_imported_cues,
};
use vsift_infrastructure::{MAX_LINE_BYTES, parse_supplied_transcript, read_supplied_transcript};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SOURCE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/transcripts")
        .join(name)
}

fn corpus(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus/transcripts")
        .join(name)
}

fn parse_file(
    path: &Path,
) -> Result<Result<ParsedTranscript, TranscriptImportError>, std::io::Error> {
    Ok(parse_supplied_transcript(&fs::read(path)?))
}

fn rejected(
    rejection: TranscriptRejection,
    line: u32,
) -> Result<ParsedTranscript, TranscriptImportError> {
    Err(TranscriptImportError::at_line(rejection, line))
}

fn timings(transcript: &ParsedTranscript) -> Vec<(u64, u64)> {
    transcript
        .cues()
        .iter()
        .map(|cue| (cue.timing.start_micros(), cue.timing.end_micros()))
        .collect()
}

fn texts(transcript: &ParsedTranscript) -> Vec<&str> {
    transcript
        .cues()
        .iter()
        .map(|cue| cue.text.text())
        .collect()
}

fn warnings(transcript: &ParsedTranscript) -> Vec<(TranscriptWarningKind, u32, u32)> {
    transcript
        .warnings()
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
        .collect()
}

/// T-01 valid: the F10 `SubRip` and `WebVTT` sidecars are equivalent cue for cue.
#[test]
fn t01_f10_srt_and_webvtt_sidecars_are_equivalent() -> TestResult {
    let srt = read_supplied_transcript(&corpus("F10.srt"))?;
    let vtt = read_supplied_transcript(&corpus("F10.vtt"))?;

    assert_eq!(srt.transcript.format(), TranscriptFormat::Srt);
    assert_eq!(vtt.transcript.format(), TranscriptFormat::WebVtt);
    assert_eq!(timings(&srt.transcript), timings(&vtt.transcript));
    assert_eq!(texts(&srt.transcript), texts(&vtt.transcript));
    assert_eq!(
        timings(&srt.transcript),
        [
            (500_000, 3_500_000),
            (4_500_000, 8_500_000),
            (9_000_000, 11_000_000)
        ]
    );
    assert_eq!(
        texts(&srt.transcript)[0],
        "This synthetic sidecar is aligned with\nan explicit 500 millisecond offset."
    );
    assert!(srt.transcript.warnings().as_slice().is_empty());
    assert!(vtt.transcript.warnings().as_slice().is_empty());
    let bytes = fs::read(corpus("F10.srt"))?;
    assert_eq!(srt.sidecar.bytes(), u64::try_from(bytes.len())?);
    assert_eq!(srt.sidecar.sha256().len(), 64);
    Ok(())
}

/// T-02 offset: F10's explicit +500 ms offset places "Dialog R-17" on its
/// frozen truth window, 5.0 s to 9.0 s.
#[test]
fn t02_f10_offset_aligns_the_dialog_cue_with_fixture_truth() -> TestResult {
    let supplied = read_supplied_transcript(&corpus("F10.vtt"))?;
    let source = whole_file_source_segment(
        &SourceId::from_sha256(SOURCE)?,
        MediaTime::from_micros(12_000_000),
    )?;

    let (aligned, warnings) = align_imported_cues(
        &supplied.transcript,
        TranscriptOffset::from_micros(500_000)?,
        &source,
    )?;

    let dialog = aligned
        .iter()
        .find(|cue| cue.cue.text.text().contains("Dialog R-17"))
        .ok_or("dialog cue missing")?;
    assert_eq!(dialog.range.start().as_micros(), 5_000_000);
    assert_eq!(dialog.range.end().as_micros(), 9_000_000);
    assert_eq!(aligned.len(), 3);
    assert!(warnings.as_slice().is_empty());
    Ok(())
}

/// T-02 before zero: a negative offset that moves cues before the source start
/// excludes them with a warning instead of clamping them to zero.
#[test]
fn t02_offset_before_zero_excludes_rather_than_clamps() -> TestResult {
    let supplied = read_supplied_transcript(&corpus("F10.srt"))?;
    let source = whole_file_source_segment(
        &SourceId::from_sha256(SOURCE)?,
        MediaTime::from_micros(12_000_000),
    )?;

    let (aligned, warnings) = align_imported_cues(
        &supplied.transcript,
        TranscriptOffset::from_micros(-4_000_000)?,
        &source,
    )?;

    // Cue 1 ends at -0.5 s; cue 2 starts at 0.5 s; cue 3 starts at 5.0 s.
    let ranges: Vec<(u64, u64)> = aligned
        .iter()
        .map(|cue| (cue.range.start().as_micros(), cue.range.end().as_micros()))
        .collect();
    assert_eq!(ranges, [(500_000, 4_500_000), (5_000_000, 7_000_000)]);
    let kinds: Vec<_> = warnings
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
        .collect();
    assert_eq!(kinds, [(TranscriptWarningKind::CuesOutsideSource, 1, 1)]);
    Ok(())
}

/// T-02 wrong source duration: cues past the probed end are excluded, and a
/// transcript with nothing inside the source is rejected.
#[test]
fn t02_wrong_source_duration_is_reported_not_fabricated() -> TestResult {
    let supplied = read_supplied_transcript(&corpus("F10.srt"))?;
    let short = whole_file_source_segment(
        &SourceId::from_sha256(SOURCE)?,
        MediaTime::from_micros(6_000_000),
    )?;

    let (aligned, warnings) = align_imported_cues(
        &supplied.transcript,
        TranscriptOffset::from_micros(500_000)?,
        &short,
    )?;
    assert_eq!(aligned.len(), 1);
    let kinds: Vec<_> = warnings
        .as_slice()
        .iter()
        .map(|warning| (warning.kind(), warning.count(), warning.first_cue()))
        .collect();
    assert_eq!(
        kinds,
        [
            (TranscriptWarningKind::CuesCrossingSourceBoundary, 1, 2),
            (TranscriptWarningKind::CuesOutsideSource, 1, 3)
        ]
    );

    let tiny = whole_file_source_segment(
        &SourceId::from_sha256(SOURCE)?,
        MediaTime::from_micros(400_000),
    )?;
    assert_eq!(
        align_imported_cues(&supplied.transcript, TranscriptOffset::ZERO, &tiny),
        Err(TranscriptImportError::new(
            TranscriptRejection::NoCuesWithinSource
        ))
    );
    Ok(())
}

/// T-02 untimed text: prose without cue timing cannot support a citation.
#[test]
fn t02_untimed_text_is_rejected() -> TestResult {
    assert_eq!(
        parse_file(&data("untimed-text.srt"))?,
        rejected(TranscriptRejection::UntimedText, 1)
    );
    assert_eq!(
        parse_supplied_transcript(
            b"1\n00:00:01,000 --> 00:00:02,000\nCue text\n\nstray paragraph\n"
        ),
        rejected(TranscriptRejection::UntimedText, 5)
    );
    assert_eq!(
        parse_supplied_transcript(b"WEBVTT\n\nidentifier\nno timing here\n"),
        rejected(TranscriptRejection::UntimedText, 3)
    );
    Ok(())
}

/// T-01 invalid timestamps: malformed, out-of-range and reversed timing reject
/// the whole file with the offending line.
#[test]
fn t01_invalid_and_reversed_timestamps_are_rejected_with_their_line() -> TestResult {
    assert_eq!(
        parse_file(&data("invalid-timestamp.srt"))?,
        rejected(TranscriptRejection::InvalidTimestamp, 6)
    );
    assert_eq!(
        parse_file(&data("reversed-timing.srt"))?,
        rejected(TranscriptRejection::NonPositiveDuration, 2)
    );
    assert_eq!(
        parse_file(&data("missing-arrow.vtt"))?,
        rejected(TranscriptRejection::UntimedText, 3)
    );
    for (input, rejection, line) in [
        (
            &b"1\n00:00:01.000 --> 00:00:02,000\nx\n"[..],
            TranscriptRejection::InvalidTimestamp,
            2,
        ),
        (
            b"1\n00:00:01,00 --> 00:00:02,000\nx\n",
            TranscriptRejection::InvalidTimestamp,
            2,
        ),
        (
            b"1\n00:00:61,000 --> 00:00:62,000\nx\n",
            TranscriptRejection::InvalidTimestamp,
            2,
        ),
        (
            b"1\n00:00:01,000 --> 00:00:02,000 X1:10 X2:20\nx\n",
            TranscriptRejection::InvalidTimingLine,
            2,
        ),
        (
            b"1\n00:00:01,000 -> 00:00:02,000\nx\n",
            TranscriptRejection::InvalidTimingLine,
            2,
        ),
        (
            b"1\n\n00:00:01,000 --> 00:00:02,000\nx\n",
            TranscriptRejection::InvalidTimingLine,
            1,
        ),
        (
            b"00:00:01,000 --> 00:00:02,000\nx\n",
            TranscriptRejection::InvalidCueNumber,
            1,
        ),
        (
            b"1\n00:00:01,000 --> 00:00:01,000\nx\n",
            TranscriptRejection::NonPositiveDuration,
            2,
        ),
        (
            b"WEBVTT\n\n0:00:01.000 --> 0:00:02.000\nx\n",
            TranscriptRejection::InvalidTimestamp,
            3,
        ),
        (
            b"WEBVTT\n\n00:01,000 --> 00:02,000\nx\n",
            TranscriptRejection::InvalidTimestamp,
            3,
        ),
        (
            b"WEBVTTX\n\n00:01.000 --> 00:02.000\nx\n",
            TranscriptRejection::InvalidHeader,
            1,
        ),
        (
            b"WEBVTT\nX-Header: 00:01.000 --> 00:02.000\n\n00:01.000 --> 00:02.000\nx\n",
            TranscriptRejection::InvalidHeader,
            2,
        ),
    ] {
        assert_eq!(
            parse_supplied_transcript(input),
            rejected(rejection, line),
            "{}",
            String::from_utf8_lossy(input)
        );
    }
    Ok(())
}

/// T-01 overlaps and order: overlaps are legitimate and warned; a cue that
/// starts before its predecessor rejects the file.
#[test]
fn t01_overlaps_are_warned_and_out_of_order_cues_are_rejected() -> TestResult {
    let overlapping = parse_file(&data("overlapping.vtt"))??;
    assert_eq!(
        timings(&overlapping),
        [
            (1_000_000, 4_000_000),
            (2_000_000, 3_000_000),
            (5_000_000, 6_000_000)
        ]
    );
    assert_eq!(
        warnings(&overlapping),
        [
            (TranscriptWarningKind::MarkupRemoved, 3, 1),
            (TranscriptWarningKind::OverlappingCues, 1, 2)
        ]
    );
    let speakers: Vec<Option<&str>> = overlapping
        .cues()
        .iter()
        .map(|cue| cue.speaker.as_ref().map(SpeakerLabel::as_str))
        .collect();
    assert_eq!(
        speakers,
        [Some("Operator"), Some("Reviewer"), Some("Operator")]
    );

    assert_eq!(
        parse_file(&data("out-of-order.srt"))?,
        rejected(TranscriptRejection::OutOfOrder, 6)
    );
    Ok(())
}

/// T-01 embedded markup: recognised markup is removed from the text, the
/// original payload is preserved, and only unambiguous voices become labels.
#[test]
fn t01_markup_is_removed_from_text_and_the_original_is_preserved() -> TestResult {
    let vtt = parse_file(&data("markup.vtt"))??;
    assert_eq!(vtt.language().map(LanguageTag::as_str), Some("en-GB"));
    assert_eq!(
        texts(&vtt),
        [
            "Bold & italic R-17 <tag>",
            "Karaoke timing \u{263a} &unknown; a < b",
            "One Two"
        ]
    );
    let first = &vtt.cues()[0];
    assert_eq!(first.text.markup(), CueMarkup::Removed);
    assert_eq!(
        first.text.original(),
        Some("<v.loud Narrator><b>Bold</b> &amp; <i>italic</i> <c.code>R-17</c> &lt;tag&gt;")
    );
    assert_eq!(
        first.speaker.as_ref().map(SpeakerLabel::as_str),
        Some("Narrator")
    );
    assert_eq!(first.timing.end_micros(), 2_500_000);
    assert!(vtt.cues()[2].speaker.is_none());
    assert_eq!(
        warnings(&vtt),
        [
            (TranscriptWarningKind::MarkupRemoved, 3, 1),
            (TranscriptWarningKind::SpeakerLabelDiscarded, 1, 3),
            (TranscriptWarningKind::EmptyCuesSkipped, 1, 4)
        ]
    );

    let srt = parse_file(&data("markup.srt"))??;
    assert_eq!(
        texts(&srt),
        [
            "Positioned italic text",
            "Red &amp; plain",
            "Narrator: speaker prefixes stay text."
        ]
    );
    assert!(srt.cues().iter().all(|cue| cue.speaker.is_none()));
    assert_eq!(srt.cues()[2].text.markup(), CueMarkup::None);
    assert_eq!(
        warnings(&srt),
        [(TranscriptWarningKind::MarkupRemoved, 2, 1)]
    );
    Ok(())
}

/// T-01 embedded markup: unclosed or oversized markup is kept as text, and a
/// file made of unclosed delimiters still parses in bounded work.
#[test]
fn t01_unclosed_and_oversized_markup_stays_text() -> TestResult {
    let unclosed = "<".repeat(4_000);
    let oversized_tag = format!("<b {}>x", "a".repeat(300));
    let braces = "{\\".repeat(2_000);
    let mut file = String::new();
    for (index, payload) in [&unclosed, &oversized_tag, &braces].iter().enumerate() {
        write!(
            file,
            "{}\n00:00:0{index},000 --> 00:00:0{index},500\n{payload}\n\n",
            index + 1
        )?;
    }
    for index in 3..500 {
        let (minutes, seconds) = (index / 60, index % 60);
        write!(
            file,
            "{}\n00:{minutes:02}:{seconds:02},000 --> 00:{minutes:02}:{seconds:02},500\n{unclosed}\n\n",
            index + 1
        )?;
    }
    let parsed = parse_supplied_transcript(file.as_bytes())?;

    assert_eq!(parsed.cues().len(), 500);
    assert_eq!(texts(&parsed)[0], unclosed);
    assert_eq!(texts(&parsed)[1], oversized_tag);
    assert_eq!(texts(&parsed)[2], braces);
    assert!(parsed.warnings().as_slice().is_empty());
    Ok(())
}

/// T-01 BOM and encoding: a UTF-8 BOM is accepted; UTF-16 and invalid UTF-8
/// are rejected rather than repaired.
#[test]
fn t01_bom_is_stripped_and_other_encodings_are_rejected() -> TestResult {
    let with_bom =
        parse_supplied_transcript(b"\xef\xbb\xbfWEBVTT\n\n00:01.000 --> 00:02.000\nHi\n")?;
    assert_eq!(with_bom.format(), TranscriptFormat::WebVtt);
    let srt_bom = parse_supplied_transcript(b"\xef\xbb\xbf1\n00:00:01,000 --> 00:00:02,000\nHi\n")?;
    assert_eq!(texts(&srt_bom), ["Hi"]);

    for input in [
        &b"\xff\xfe1\x00\n\x00"[..],
        b"\xfe\xff\x001",
        b"\0\0\xfe\xff",
    ] {
        assert_eq!(
            parse_supplied_transcript(input),
            Err(TranscriptImportError::new(
                TranscriptRejection::UnsupportedEncoding
            ))
        );
    }
    assert_eq!(
        parse_supplied_transcript(b"1\r\n00:00:01,000 --> 00:00:02,000\r\nbad \xff byte\r\n"),
        rejected(TranscriptRejection::InvalidUtf8, 3)
    );
    Ok(())
}

/// T-01 line endings: LF, CRLF and CR produce identical cues.
#[test]
fn t01_line_endings_are_equivalent() -> TestResult {
    let lf = parse_supplied_transcript(
        b"1\n00:00:01,000 --> 00:00:02,000\nOne\nTwo\n\n2\n00:00:03,000 --> 00:00:04,000\nThree",
    )?;
    let crlf = parse_supplied_transcript(b"1\r\n00:00:01,000 --> 00:00:02,000\r\nOne\r\nTwo\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\nThree\r\n")?;
    let cr = parse_supplied_transcript(
        b"1\r00:00:01,000 --> 00:00:02,000\rOne\rTwo\r\r2\r00:00:03,000 --> 00:00:04,000\rThree\r",
    )?;
    assert_eq!(lf, crlf);
    assert_eq!(lf, cr);
    assert_eq!(texts(&lf), ["One\nTwo", "Three"]);
    Ok(())
}

/// T-01 control characters: terminal controls, NUL and Unicode line separators
/// reject the file at their line; tabs are kept in the original only.
#[test]
fn t01_control_characters_are_rejected_and_tabs_normalized() -> TestResult {
    for (input, line) in [
        (
            &b"1\n00:00:01,000 --> 00:00:02,000\nred \x1b[31m text\n"[..],
            3,
        ),
        (b"1\n00:00:01,000 --> 00:00:02,000\nnul \x00 byte\n", 3),
        (b"WEBVTT\n\n00:01.000 --> 00:02.000\nc1 \xc2\x9b csi\n", 4),
        (
            b"WEBVTT\n\n00:01.000 --> 00:02.000\nsep \xe2\x80\xa8 here\n",
            4,
        ),
        (
            b"WEBVTT\n\n00:01.000 --> 00:02.000\nescape <c>&#x1b;</c>\n",
            0,
        ),
    ] {
        let result = parse_supplied_transcript(input);
        if line == 0 {
            // A character reference to a control is not decoded; it stays text.
            let parsed = result?;
            assert_eq!(texts(&parsed), ["escape &#x1b;"]);
        } else {
            assert_eq!(
                result,
                rejected(TranscriptRejection::ControlCharacter, line)
            );
        }
    }
    let tabbed = parse_supplied_transcript(b"1\n00:00:01,000 --> 00:00:02,000\ncol\tumn\n")?;
    assert_eq!(texts(&tabbed), ["col umn"]);
    assert_eq!(tabbed.cues()[0].text.original(), Some("col\tumn"));
    Ok(())
}

/// T-01 large line and bounds: line, cue-text, cue-count and file-size limits
/// are enforced while reading, with typed resource rejections.
#[test]
fn t01_large_lines_cues_and_files_hit_typed_limits() -> TestResult {
    let at_limit = format!(
        "1\n00:00:01,000 --> 00:00:02,000\n{}\n",
        "a".repeat(MAX_LINE_BYTES)
    );
    assert!(parse_supplied_transcript(at_limit.as_bytes()).is_ok());
    let over = format!(
        "1\n00:00:01,000 --> 00:00:02,000\n{}\n",
        "a".repeat(MAX_LINE_BYTES + 1)
    );
    assert_eq!(
        parse_supplied_transcript(over.as_bytes()),
        rejected(TranscriptRejection::LineTooLong, 3)
    );

    let long_cue = format!(
        "1\n00:00:01,000 --> 00:00:02,000\n{}\n{}\n",
        "a".repeat(MAX_CUE_TEXT_BYTES / 2),
        "b".repeat(MAX_CUE_TEXT_BYTES / 2)
    );
    assert_eq!(
        parse_supplied_transcript(long_cue.as_bytes()),
        rejected(TranscriptRejection::CueTextTooLong, 2)
    );

    let mut many = String::new();
    for index in 0..=MAX_TRANSCRIPT_CUES {
        let second = index % 60;
        let minute = (index / 60) % 60;
        let hour = index / 3600;
        write!(
            many,
            "{}\n{hour:02}:{minute:02}:{second:02},000 --> {hour:02}:{minute:02}:{second:02},500\nx\n\n",
            index + 1
        )?;
    }
    let too_many = parse_supplied_transcript(many.as_bytes());
    assert_eq!(
        too_many
            .map_err(TranscriptImportError::rejection)
            .map(|_| ()),
        Err(TranscriptRejection::TooManyCues)
    );

    let oversized = vec![b'a'; usize::try_from(MAX_SUPPLIED_TRANSCRIPT_BYTES)? + 1];
    assert_eq!(
        parse_supplied_transcript(&oversized),
        Err(TranscriptImportError::new(TranscriptRejection::TooLarge))
    );
    Ok(())
}

#[test]
fn t01_files_without_text_cues_are_rejected() {
    for input in [
        &b""[..],
        b"\n\n",
        b"WEBVTT\n",
        b"WEBVTT\n\nNOTE only a comment\n",
        b"1\n00:00:01,000 --> 00:00:02,000\n\n",
    ] {
        assert_eq!(
            parse_supplied_transcript(input),
            Err(TranscriptImportError::new(TranscriptRejection::NoCues)),
            "{}",
            String::from_utf8_lossy(input)
        );
    }
}

#[test]
fn webvtt_settings_notes_and_identifiers_are_accepted() -> TestResult {
    let parsed = parse_supplied_transcript(
        b"WEBVTT\n\nNOTE\nmulti-line\ncomment\n\nREGION\nid:one\n\n1\n00:00:01.000 --> 00:00:02.000 line:0 region:one\nText\n\n01:00:00.000 --> 01:00:01.000\nHour cue\n",
    )?;
    assert_eq!(
        timings(&parsed),
        [(1_000_000, 2_000_000), (3_600_000_000, 3_601_000_000)]
    );
    assert!(parsed.language().is_none());
    Ok(())
}

#[test]
fn supplied_paths_follow_the_source_path_policy() {
    assert_eq!(
        read_supplied_transcript(Path::new("relative.srt")),
        Err(SuppliedTranscriptError::InvalidPath)
    );
    assert_eq!(
        read_supplied_transcript(&data("does-not-exist.srt")),
        Err(SuppliedTranscriptError::Io)
    );
    assert_eq!(
        read_supplied_transcript(&data("untimed-text.srt")),
        Err(SuppliedTranscriptError::Rejected(
            TranscriptImportError::at_line(TranscriptRejection::UntimedText, 1)
        ))
    );
}

/// Checks every invariant an accepted transcript must satisfy.
fn assert_accepted_invariants(transcript: &ParsedTranscript) -> Result<(), String> {
    if transcript.cues().is_empty() || transcript.cues().len() > MAX_TRANSCRIPT_CUES {
        return Err("cue count out of bounds".to_owned());
    }
    let mut previous = 0_u64;
    for cue in transcript.cues() {
        let text = cue.text.text();
        if cue.timing.start_micros() >= cue.timing.end_micros()
            || cue.timing.start_micros() < previous
            || text.trim().is_empty()
            || text.len() > MAX_CUE_TEXT_BYTES
            || text
                .chars()
                .any(|character| character.is_control() && character != '\n')
            || cue
                .text
                .original()
                .is_some_and(|original| original.len() > MAX_CUE_TEXT_BYTES)
        {
            return Err(format!(
                "cue {} violates an invariant",
                cue.source.ordinal()
            ));
        }
        previous = cue.timing.start_micros();
    }
    Ok(())
}

fn timestamp(format: TranscriptFormat, micros: u64) -> String {
    let millis = micros / 1_000;
    let (hours, rest) = (millis / 3_600_000, millis % 3_600_000);
    let (minutes, rest) = (rest / 60_000, rest % 60_000);
    let (seconds, millis) = (rest / 1_000, rest % 1_000);
    match format {
        TranscriptFormat::Srt => format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}"),
        TranscriptFormat::WebVtt => format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}"),
    }
}

fn render(
    format: TranscriptFormat,
    cues: &[(u64, u64, String)],
    ending: &str,
) -> Result<String, std::fmt::Error> {
    let mut text = match format {
        TranscriptFormat::Srt => String::new(),
        TranscriptFormat::WebVtt => format!("WEBVTT{ending}{ending}"),
    };
    for (index, (start, end, words)) in cues.iter().enumerate() {
        if format == TranscriptFormat::Srt {
            write!(text, "{}{ending}", index + 1)?;
        }
        write!(
            text,
            "{} --> {}{ending}{words}{ending}{ending}",
            timestamp(format, *start),
            timestamp(format, *end)
        )?;
    }
    Ok(text)
}

fn cue_strategy() -> impl Strategy<Value = Vec<(u64, u64, String)>> {
    prop::collection::vec(
        (
            0_u64..36_000_000,
            1_u64..10_000_000,
            "[a-zA-Z0-9 ,.?!-]{1,40}",
        ),
        1..30,
    )
    .prop_map(|mut cues| {
        cues.sort_by_key(|(start, _, _)| *start);
        cues.into_iter()
            .map(|(start, length, words)| {
                let start = start / 1_000 * 1_000;
                let length = (length / 1_000).max(1) * 1_000;
                let words = if words.trim().is_empty() {
                    "x".to_owned()
                } else {
                    words.trim().to_owned()
                };
                (start, start + length, words)
            })
            .collect()
    })
}

fn line_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("WEBVTT".to_owned()),
        Just("NOTE x".to_owned()),
        "[0-9]{1,3}",
        "[0-9]{2}:[0-9]{2}:[0-9]{2},[0-9]{3} --> [0-9]{2}:[0-9]{2}:[0-9]{2},[0-9]{3}",
        "[0-9]{2}:[0-9]{2}\\.[0-9]{3} --> [0-9]{2}:[0-9]{2}\\.[0-9]{3}( align:start)?",
        "(<[a-z/.0-9 ]{0,8}>|&[a-z#0-9]{0,6};?|\\{\\\\[a-z0-9]{0,3}\\}|[ -~]){0,20}",
        "\\PC{0,20}",
    ]
}

proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

    /// Arbitrary bytes never panic, and anything accepted satisfies the invariants.
    #[test]
    fn arbitrary_bytes_never_panic_or_escape_bounds(bytes in prop::collection::vec(proptest::num::u8::ANY, 0..2_048)) {
        if let Ok(transcript) = parse_supplied_transcript(&bytes) {
            prop_assert_eq!(assert_accepted_invariants(&transcript), Ok(()));
        }
    }

    /// Structured near-miss input never panics, and accepted cues are valid.
    #[test]
    fn structured_noise_never_panics_or_escapes_bounds(
        lines in prop::collection::vec(line_strategy(), 0..60),
        ending in prop_oneof![Just("\n"), Just("\r\n"), Just("\r")],
        bom in proptest::bool::ANY,
    ) {
        let mut text = if bom { "\u{feff}".to_owned() } else { String::new() };
        text.push_str(&lines.join(ending));
        match parse_supplied_transcript(text.as_bytes()) {
            Ok(transcript) => prop_assert_eq!(assert_accepted_invariants(&transcript), Ok(())),
            Err(error) => {
                let located = error.line().is_none_or(|line| {
                    usize::try_from(line).is_ok_and(|line| line >= 1 && line <= lines.len().max(1))
                });
                prop_assert!(located, "line out of range: {:?}", error);
            }
        }
    }

    /// Valid generated SRT and WebVTT parse to identical cues, and every
    /// aligned cue is its written timing plus the offset, inside the source.
    #[test]
    fn generated_sidecars_round_trip_and_align_within_bounds(
        cues in cue_strategy(),
        ending in prop_oneof![Just("\n"), Just("\r\n"), Just("\r")],
        offset in -40_000_000_i64..40_000_000,
        duration in 1_u64..50_000_000,
    ) {
        let failed = |error: std::fmt::Error| proptest::test_runner::TestCaseError::fail(error.to_string());
        let srt = parse_supplied_transcript(render(TranscriptFormat::Srt, &cues, ending).map_err(failed)?.as_bytes());
        let vtt = parse_supplied_transcript(render(TranscriptFormat::WebVtt, &cues, ending).map_err(failed)?.as_bytes());
        prop_assert!(srt.is_ok());
        prop_assert!(vtt.is_ok());
        let (Ok(srt), Ok(vtt)) = (srt, vtt) else {
            return Ok(());
        };
        let expected: Vec<(u64, u64)> = cues.iter().map(|(start, end, _)| (*start, *end)).collect();
        prop_assert_eq!(timings(&srt), expected.clone());
        prop_assert_eq!(timings(&vtt), expected);
        prop_assert_eq!(texts(&srt), texts(&vtt));

        let source = whole_file_source_segment(
            &SourceId::from_sha256(SOURCE).map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?,
            MediaTime::from_micros(duration),
        ).map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?;
        let offset = TranscriptOffset::from_micros(offset)
            .map_err(|error| proptest::test_runner::TestCaseError::fail(error.to_string()))?;
        match align_imported_cues(&srt, offset, &source) {
            Ok((aligned, _)) => {
                for cue in aligned {
                    prop_assert!(cue.range.start() < cue.range.end());
                    prop_assert!(cue.range.end().as_micros() <= duration);
                    prop_assert_eq!(
                        i128::from(cue.range.start().as_micros()),
                        offset.shift(cue.cue.timing.start_micros())
                    );
                    prop_assert_eq!(
                        i128::from(cue.range.end().as_micros()),
                        offset.shift(cue.cue.timing.end_micros())
                    );
                }
            }
            Err(error) => prop_assert_eq!(error.rejection(), TranscriptRejection::NoCuesWithinSource),
        }
    }
}
