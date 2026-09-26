//! Fuzz target bodies for the parsers of untrusted input (ADR 0016, decision 6).
//!
//! Every target is a plain function over bytes, so the libFuzzer entry points in
//! `fuzz_targets/` (nightly, scheduled CI) and the stable replay tests in
//! `tests/replay.rs` (every seed, on any toolchain) run exactly the same code.
//!
//! A target reaches its parser only through the crate's published API, the same
//! surface the infrastructure crate's own integration tests use. A parser may
//! reject any input; a finding is a panic inside the parser or a broken
//! invariant on an accepted result, which the target reports as a typed
//! [`Violation`] rather than by panicking itself.

#![forbid(unsafe_code)]

use std::{collections::BTreeSet, error::Error, fmt};

use vsift_domain::{
    CursorToken, FrameDimensions, ListingTail, MAX_CUE_TEXT_BYTES, MAX_LISTED_FRAMES,
    MAX_SEARCH_QUERY_BYTES, MAX_SEARCH_TERMS, MAX_TRANSCRIPT_CUES, MAX_WINDOW_CANDIDATES,
    MediaStreamKind, MediaTime, PlannedChunk, SearchMatch, SearchQuery, SessionId, SourceSegmentId,
    TimeRange, TranscriptFormat, VISUAL_FRAME_BYTES, VisualCandidate, VisualCandidateId,
    VisualChangePolicy, VisualIndexWindow, VisualSample, VisualWindow, VisualWindowOutcome,
    analyse_window, normalise_search_text, validate_chunk_output,
};
use vsift_infrastructure::{
    FrameListingWindow, MAX_DIAGNOSTIC_BYTES, MAX_LISTING_DIAGNOSTIC_BYTES, SourceContainer,
    VisualSamplingWindow, WhisperOutputLimits, decode_transcript_record,
    decode_visual_index_record, encode_transcript_record, encode_visual_index_record,
    parse_ashowinfo_start, parse_ffprobe_metadata, parse_frame_listing, parse_frame_showinfo,
    parse_png_sequence, parse_supplied_transcript, parse_visual_samples, parse_whisper_full_json,
};

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
const WEBVTT_SIGNATURE: &[u8] = b"WEBVTT";
/// Normalisation lowercases, which can turn one character into up to three
/// (only U+0130 grows beyond one) and never grows a character's UTF-8 form by
/// more than half; three times the input is a safe bound on the words' bytes.
const MAX_NORMALISED_GROWTH: usize = 3;
/// Window of the fixed chunk that whisper output is validated against: the
/// longest R0 chunk, so every in-bounds provider time is reachable.
const WHISPER_CHUNK_MICROS: u64 = 30_000_000;
const WHISPER_SOURCE_SEGMENT: &str = "sgm_0123456789abcdef";
/// Session every fuzzed visual-index record is decoded for; the seeds are
/// encoded for it.
pub const VISUAL_FUZZ_SESSION: &str = "ses_0123456789abcdef";
/// The visual-sampling window every input is parsed against: window 0 of a
/// 60 s source with its origin at zero, so every in-bounds timestamp is
/// reachable.
const VISUAL_WINDOW_MICROS: u64 = 60_000_000;
/// The frame listing every [`Target::FrameListing`] input is parsed against:
/// F01's stream (1/10240, origin zero) over `[0, 60 s)`, so every in-bounds
/// timestamp of a 20 fps listing is reachable.
const LISTING_TIME_BASE_DENOMINATOR: u32 = 10_240;
const LISTING_WINDOW_MICROS: u64 = 60_000_000;
/// What `FFmpeg` writes before an echoed metadata value at `-loglevel info`.
const ECHO_INDENT: &[u8] = b"    title           : ";

/// One fuzzed parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    /// `SubRip` import through `parse_supplied_transcript`.
    TranscriptSrt,
    /// `WebVTT` import through `parse_supplied_transcript`.
    TranscriptWebVtt,
    /// whisper.cpp `-ojf` output through `parse_whisper_full_json`, then the
    /// domain's chunk validation.
    WhisperFullJson,
    /// The stored transcript record (versions 1 and 2) through
    /// `decode_transcript_record`, as `bundle validate` reads it.
    TranscriptRecord,
    /// `FFprobe` metadata through `parse_ffprobe_metadata`, for both containers.
    FfprobeMetadata,
    /// The `transcript get --cursor` continuation token through `CursorToken::parse`.
    TranscriptCursor,
    /// One visual window's `FFmpeg` output through `parse_visual_samples`, then
    /// the domain's window analysis. See [`visual_samples_input`] for the
    /// input layout.
    VisualSamples,
    /// The stored visual-index record through `decode_visual_index_record`,
    /// as a session read and `bundle validate` read it.
    VisualIndexRecord,
    /// The `search --query` text through `SearchQuery::parse`, then matched
    /// against segment text. The input is the query, a line feed, and the
    /// segment text.
    SearchQuery,
    /// `showinfo` and `ashowinfo` diagnostics through the single-frame and
    /// first-audio-sample parsers (`parse_frame_showinfo`,
    /// `parse_ashowinfo_start`). The input is the diagnostics.
    FrameShowinfo,
    /// A frame listing's `showinfo` diagnostics through `parse_frame_listing`,
    /// against a fixed 60 s window of F01's stream. The input is the diagnostics.
    FrameListing,
    /// An extraction run's standard output through `parse_png_sequence`. See
    /// [`png_sequence_input`] for the input layout.
    PngSequence,
}

impl Target {
    /// Every target, in the order CI runs them.
    pub const ALL: [Self; 12] = [
        Self::TranscriptSrt,
        Self::TranscriptWebVtt,
        Self::WhisperFullJson,
        Self::TranscriptRecord,
        Self::FfprobeMetadata,
        Self::TranscriptCursor,
        Self::VisualSamples,
        Self::VisualIndexRecord,
        Self::SearchQuery,
        Self::FrameShowinfo,
        Self::FrameListing,
        Self::PngSequence,
    ];

    /// The target's `cargo fuzz` name, which is also its seed directory name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TranscriptSrt => "transcript_srt",
            Self::TranscriptWebVtt => "transcript_webvtt",
            Self::WhisperFullJson => "whisper_full_json",
            Self::TranscriptRecord => "transcript_record",
            Self::FfprobeMetadata => "ffprobe_metadata",
            Self::TranscriptCursor => "transcript_cursor",
            Self::VisualSamples => "visual_samples",
            Self::VisualIndexRecord => "visual_index_record",
            Self::SearchQuery => "search_query",
            Self::FrameShowinfo => "frame_showinfo",
            Self::FrameListing => "frame_listing",
            Self::PngSequence => "png_sequence",
        }
    }

    /// Runs the parser over `data` and checks the invariants of an accepted result.
    ///
    /// # Errors
    ///
    /// Returns the first [`Violation`]. A parser rejecting `data` is not a violation.
    pub fn check(self, data: &[u8]) -> Result<(), Violation> {
        match self {
            Self::TranscriptSrt => check_srt(data),
            Self::TranscriptWebVtt => check_webvtt(data),
            Self::WhisperFullJson => check_whisper_full_json(data),
            Self::TranscriptRecord => check_transcript_record(data),
            Self::FfprobeMetadata => check_ffprobe_metadata(data),
            Self::TranscriptCursor => check_transcript_cursor(data),
            Self::VisualSamples => check_visual_samples(data),
            Self::VisualIndexRecord => check_visual_index_record(data),
            Self::SearchQuery => check_search_query(data),
            Self::FrameShowinfo => check_frame_showinfo(data),
            Self::FrameListing => check_frame_listing(data),
            Self::PngSequence => check_png_sequence(data),
        }
    }
}

/// An invariant an accepted parse result broke.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Violation {
    /// The harness's own fixed values were rejected; the target cannot run.
    HarnessSetup,
    /// A transcript was parsed as the other sidecar format.
    WrongTranscriptFormat,
    /// An accepted transcript has no cues or more than the cue bound.
    CueCountOutOfBounds,
    /// A cue does not end after it starts.
    CueNotPositive,
    /// A cue starts before its predecessor.
    CuesOutOfOrder,
    /// Cue or segment text is blank or longer than the cue-text bound.
    TextOutOfBounds,
    /// Plain cue text is longer, in total, than the sidecar it came from.
    TextLargerThanInput,
    /// Parsed whisper output exceeds its segment or token bound.
    WhisperOutputOutOfBounds,
    /// A validated whisper segment lies outside the chunk's source range, or
    /// validation produced more segments than the provider reported.
    ValidatedSegmentOutOfBounds,
    /// An accepted transcript record could not be encoded again.
    RecordNotReencodable,
    /// An accepted transcript record changed when encoded and decoded again.
    RecordRoundTripChanged,
    /// Accepted `FFprobe` metadata has a zero duration.
    ZeroDuration,
    /// Accepted `FFprobe` metadata repeats a stream index.
    DuplicateStreamIndex,
    /// A video stream lacks dimensions or another stream has them.
    DimensionsMismatch,
    /// An accepted cursor did not decode to itself after encoding.
    CursorRoundTripChanged,
    /// Accepted visual samples do not match the decoded frames: another
    /// count, other pixels, unordered times or a time outside the window.
    VisualSamplesMismatch,
    /// The window analysis rejected accepted samples, or its result failed
    /// the index's own validation.
    VisualAnalysisInvalid,
    /// An accepted visual-index record could not be encoded again.
    VisualRecordNotReencodable,
    /// An accepted visual-index record changed in a round trip, or a window
    /// exceeds its candidate budget.
    VisualRecordRoundTripChanged,
    /// An accepted query is too long or has no words or too many, or a
    /// normalised word is empty, holds a character normalisation never keeps,
    /// or the words outgrow their text.
    SearchWordsOutOfBounds,
    /// Normalising a query's or text's normalised words again changed them.
    SearchNormalisationNotIdempotent,
    /// A match tier does not hold for the words it was decided on.
    SearchMatchInconsistent,
    /// Indented copies of the diagnostic lines, as `FFmpeg` echoes source
    /// metadata, changed what a diagnostics parser accepted (SEC-17).
    EchoedDiagnosticsChangedResult,
    /// An accepted frame listing holds too many frames, frames out of order,
    /// or frames or a covered range outside the listed window.
    ListingOutOfBounds,
    /// Accepted PNG images are not exactly the input, split into the
    /// expected number of images of the expected size.
    PngSequenceMismatch,
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::HarnessSetup => "the harness's fixed values were rejected",
            Self::WrongTranscriptFormat => "the transcript was parsed as the other format",
            Self::CueCountOutOfBounds => "an accepted transcript has no cues or too many",
            Self::CueNotPositive => "a cue does not end after it starts",
            Self::CuesOutOfOrder => "a cue starts before its predecessor",
            Self::TextOutOfBounds => "text is blank or longer than the cue-text bound",
            Self::TextLargerThanInput => "cue text is larger than the sidecar",
            Self::WhisperOutputOutOfBounds => "parsed whisper output exceeds its bounds",
            Self::ValidatedSegmentOutOfBounds => "a validated segment is out of bounds",
            Self::RecordNotReencodable => "an accepted record could not be encoded again",
            Self::RecordRoundTripChanged => "an accepted record changed in a round trip",
            Self::ZeroDuration => "accepted metadata has a zero duration",
            Self::DuplicateStreamIndex => "accepted metadata repeats a stream index",
            Self::DimensionsMismatch => "stream dimensions do not match the stream kind",
            Self::CursorRoundTripChanged => "an accepted cursor changed in a round trip",
            Self::VisualSamplesMismatch => "accepted visual samples do not match the frames",
            Self::VisualAnalysisInvalid => "the window analysis of accepted samples is invalid",
            Self::VisualRecordNotReencodable => {
                "an accepted visual-index record could not be encoded again"
            }
            Self::VisualRecordRoundTripChanged => {
                "an accepted visual-index record changed in a round trip"
            }
            Self::SearchWordsOutOfBounds => "search words are empty, invalid or unbounded",
            Self::SearchNormalisationNotIdempotent => "normalising normalised words changed them",
            Self::SearchMatchInconsistent => "a search match does not hold for its words",
            Self::EchoedDiagnosticsChangedResult => {
                "echoed metadata lines changed a diagnostics parser's result"
            }
            Self::ListingOutOfBounds => "an accepted frame listing is out of bounds",
            Self::PngSequenceMismatch => "accepted PNG images do not match the output",
        })
    }
}

impl Error for Violation {}

/// libFuzzer entry: runs `target` and turns a [`Violation`] into a crash.
///
/// libFuzzer records an input only when the process crashes, so a broken
/// invariant must end the process. Aborting, rather than panicking, keeps the
/// harness within the repository's no-panic rule while giving libFuzzer the
/// same signal it receives from a panic inside a parser.
pub fn run(target: Target, data: &[u8]) {
    if let Err(violation) = target.check(data) {
        eprintln!("{}: {violation}", target.name());
        std::process::abort();
    }
}

fn without_bom(data: &[u8]) -> &[u8] {
    data.strip_prefix(UTF8_BOM).unwrap_or(data)
}

/// Feeds everything except `WebVTT`-signed input, so the fuzzer's effort stays
/// in the `SubRip` parser; the `WebVTT` target covers the rest.
fn check_srt(data: &[u8]) -> Result<(), Violation> {
    if without_bom(data).starts_with(WEBVTT_SIGNATURE) {
        return Ok(());
    }
    check_transcript(data, TranscriptFormat::Srt)
}

/// Adds the `WebVTT` signature when it is missing, so every input reaches the
/// `WebVTT` parser rather than being rejected as malformed `SubRip`.
fn check_webvtt(data: &[u8]) -> Result<(), Violation> {
    if without_bom(data).starts_with(WEBVTT_SIGNATURE) {
        return check_transcript(data, TranscriptFormat::WebVtt);
    }
    let mut signed = Vec::with_capacity(data.len().saturating_add(WEBVTT_SIGNATURE.len() + 1));
    signed.extend_from_slice(WEBVTT_SIGNATURE);
    signed.push(b'\n');
    signed.extend_from_slice(data);
    check_transcript(&signed, TranscriptFormat::WebVtt)
}

fn check_transcript(data: &[u8], expected: TranscriptFormat) -> Result<(), Violation> {
    let Ok(parsed) = parse_supplied_transcript(data) else {
        return Ok(());
    };
    if parsed.format() != expected {
        return Err(Violation::WrongTranscriptFormat);
    }
    let cues = parsed.cues();
    if cues.is_empty() || cues.len() > MAX_TRANSCRIPT_CUES {
        return Err(Violation::CueCountOutOfBounds);
    }
    let mut previous_start = 0_u64;
    let mut text_bytes = 0_usize;
    for cue in cues {
        let (start, end) = (cue.timing.start_micros(), cue.timing.end_micros());
        if end <= start {
            return Err(Violation::CueNotPositive);
        }
        if start < previous_start {
            return Err(Violation::CuesOutOfOrder);
        }
        previous_start = start;
        let text = cue.text.text();
        if text.trim().is_empty() || text.len() > MAX_CUE_TEXT_BYTES {
            return Err(Violation::TextOutOfBounds);
        }
        text_bytes = text_bytes.saturating_add(text.len());
    }
    // Markup removal and character references only ever shorten text.
    if text_bytes > data.len() {
        return Err(Violation::TextLargerThanInput);
    }
    Ok(())
}

fn check_whisper_full_json(data: &[u8]) -> Result<(), Violation> {
    let limits = WhisperOutputLimits::R0;
    let Ok(output) = parse_whisper_full_json(data, limits) else {
        return Ok(());
    };
    if output.segments.len() > limits.max_segments
        || output
            .segments
            .iter()
            .any(|segment| segment.tokens.len() > limits.max_tokens)
    {
        return Err(Violation::WhisperOutputOutOfBounds);
    }
    if output.segments.iter().any(|segment| {
        segment.text.as_ref().is_some_and(|text| {
            text.text().trim().is_empty() || text.text().len() > MAX_CUE_TEXT_BYTES
        })
    }) {
        return Err(Violation::TextOutOfBounds);
    }
    let window = TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(WHISPER_CHUNK_MICROS),
    )
    .map_err(|_| Violation::HarnessSetup)?;
    let segment =
        SourceSegmentId::parse(WHISPER_SOURCE_SEGMENT).map_err(|_| Violation::HarnessSetup)?;
    let chunk = PlannedChunk::new(segment, 0, window);
    let reported = output.segments.len();
    let Ok(validated) = validate_chunk_output(&chunk, window, window, output) else {
        return Ok(());
    };
    if validated.segments().len() > reported
        || validated
            .segments()
            .iter()
            .any(|draft| draft.range().end() > window.end())
    {
        return Err(Violation::ValidatedSegmentOutOfBounds);
    }
    Ok(())
}

fn check_transcript_record(data: &[u8]) -> Result<(), Violation> {
    let Ok(revision) = decode_transcript_record(data) else {
        return Ok(());
    };
    let encoded =
        encode_transcript_record(&revision).map_err(|_| Violation::RecordNotReencodable)?;
    match decode_transcript_record(&encoded) {
        Ok(decoded) if decoded == revision => Ok(()),
        _ => Err(Violation::RecordRoundTripChanged),
    }
}

fn check_ffprobe_metadata(data: &[u8]) -> Result<(), Violation> {
    for container in [SourceContainer::IsoMedia, SourceContainer::Matroska] {
        let Ok(description) = parse_ffprobe_metadata(data, container) else {
            continue;
        };
        if description.duration.as_micros() == 0 {
            return Err(Violation::ZeroDuration);
        }
        let mut indexes = BTreeSet::new();
        for stream in &description.streams {
            if !indexes.insert(stream.index) {
                return Err(Violation::DuplicateStreamIndex);
            }
            if (stream.kind == MediaStreamKind::Video) != stream.encoded_dimensions.is_some() {
                return Err(Violation::DimensionsMismatch);
            }
        }
    }
    Ok(())
}

fn check_transcript_cursor(data: &[u8]) -> Result<(), Violation> {
    let Ok(text) = std::str::from_utf8(data) else {
        return Ok(());
    };
    let Ok(token) = CursorToken::parse(text) else {
        return Ok(());
    };
    match CursorToken::parse(&token.encode()) {
        Ok(reparsed) if reparsed == token => Ok(()),
        _ => Err(Violation::CursorRoundTripChanged),
    }
}

/// Builds a [`Target::VisualSamples`] input: one byte with the number of
/// decoded frames, one byte from which each frame's uniform pixel value is
/// derived, then the diagnostics (`showinfo` text).
///
/// The fuzzer mutates the diagnostics freely while the frames stay cheap:
/// the target writes `count` frames of 9,216 bytes itself rather than
/// expecting megabytes of input.
#[must_use]
pub fn visual_samples_input(count: u8, pixel_seed: u8, stderr: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(stderr.len().saturating_add(2));
    data.push(count);
    data.push(pixel_seed);
    data.extend_from_slice(stderr);
    data
}

fn visual_frames(count: u8, pixel_seed: u8) -> Vec<u8> {
    let mut stdout = Vec::with_capacity(usize::from(count) * VISUAL_FRAME_BYTES);
    for frame in 0..count {
        stdout.extend(std::iter::repeat_n(
            pixel_seed.wrapping_add(frame.wrapping_mul(7)),
            VISUAL_FRAME_BYTES,
        ));
    }
    stdout
}

fn check_visual_samples(data: &[u8]) -> Result<(), Violation> {
    let [count, pixel_seed, stderr @ ..] = data else {
        return Ok(());
    };
    let stdout = visual_frames(*count, *pixel_seed);
    let end = MediaTime::from_micros(VISUAL_WINDOW_MICROS);
    let sampling = VisualSamplingWindow::new(0, 0, MediaTime::from_micros(0), end)
        .map_err(|_| Violation::HarnessSetup)?;
    let Ok(frames) = parse_visual_samples(&stdout, stderr, &sampling) else {
        return Ok(());
    };
    if frames.len() != usize::from(*count)
        || frames
            .iter()
            .zip(stdout.chunks(VISUAL_FRAME_BYTES))
            .any(|(frame, pixels)| frame.pixels.as_slice() != pixels || frame.time >= end)
        || frames.windows(2).any(|pair| match pair {
            [before, after] => before.time >= after.time,
            _ => false,
        })
    {
        return Err(Violation::VisualSamplesMismatch);
    }
    let window = VisualWindow::new(0, end).map_err(|_| Violation::HarnessSetup)?;
    let samples: Vec<VisualSample> = frames
        .iter()
        .map(|frame| VisualSample::from_gray(frame.time, &frame.pixels))
        .collect();
    let analysis = analyse_window(window, &samples, VisualChangePolicy::R0)
        .map_err(|_| Violation::VisualAnalysisInvalid)?;
    let mut candidates = Vec::with_capacity(analysis.candidates.len());
    for (position, draft) in analysis.candidates.into_iter().enumerate() {
        let id = VisualCandidateId::parse(format!("vcd_{position:032x}"))
            .map_err(|_| Violation::HarnessSetup)?;
        candidates.push(VisualCandidate::new(id, draft));
    }
    VisualIndexWindow::new(
        window,
        VisualWindowOutcome::Analysed {
            sample_count: analysis.sample_count,
            candidates,
            dropped_candidates: analysis.dropped_candidates,
            frameless_cells: analysis.frameless_cells,
        },
        VisualChangePolicy::R0,
    )
    .map_err(|_| Violation::VisualAnalysisInvalid)?;
    Ok(())
}

fn check_visual_index_record(data: &[u8]) -> Result<(), Violation> {
    let session = SessionId::parse(VISUAL_FUZZ_SESSION).map_err(|_| Violation::HarnessSetup)?;
    let Ok(index) = decode_visual_index_record(data, &session) else {
        return Ok(());
    };
    if index
        .windows()
        .iter()
        .any(|window| window.candidates().len() > MAX_WINDOW_CANDIDATES)
    {
        return Err(Violation::VisualRecordRoundTripChanged);
    }
    let encoded = encode_visual_index_record(&session, &index)
        .map_err(|_| Violation::VisualRecordNotReencodable)?;
    match decode_visual_index_record(&encoded, &session) {
        Ok(decoded) if decoded == index => Ok(()),
        _ => Err(Violation::VisualRecordRoundTripChanged),
    }
}

/// Words normalisation may keep: letters, digits and a decimal point.
fn is_search_word(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|character| character.is_alphanumeric() || character == '.')
}

/// Normalises `text`, checking the words are bounded, well formed and
/// unchanged by normalising them again.
fn checked_words(text: &str) -> Result<Vec<String>, Violation> {
    let words = normalise_search_text(text);
    let bytes: usize = words.iter().map(String::len).sum();
    if bytes > text.len().saturating_mul(MAX_NORMALISED_GROWTH)
        || !words.iter().all(|word| is_search_word(word))
    {
        return Err(Violation::SearchWordsOutOfBounds);
    }
    if normalise_search_text(&words.join(" ")) != words {
        return Err(Violation::SearchNormalisationNotIdempotent);
    }
    Ok(words)
}

fn check_search_query(data: &[u8]) -> Result<(), Violation> {
    let Ok(input) = std::str::from_utf8(data) else {
        return Ok(());
    };
    let (query_text, text) = input.split_once('\n').unwrap_or((input, ""));
    let words = checked_words(text)?;
    let Ok(query) = SearchQuery::parse(query_text) else {
        return Ok(());
    };
    let terms = query.terms();
    if query_text.len() > MAX_SEARCH_QUERY_BYTES
        || terms.is_empty()
        || terms.len() > MAX_SEARCH_TERMS
        || checked_words(query_text)? != terms
    {
        return Err(Violation::SearchWordsOutOfBounds);
    }
    let consistent = match query.classify(text) {
        // A phrase is a run of consecutive words, so it lies within them joined.
        Some(SearchMatch::Phrase) => words.concat().contains(&terms.concat()),
        Some(SearchMatch::AllTerms) => terms.iter().all(|term| words.contains(term)),
        // Every query word being a text word is at least an all-terms match.
        None => !terms.iter().all(|term| words.contains(term)),
    };
    if consistent {
        Ok(())
    } else {
        Err(Violation::SearchMatchInconsistent)
    }
}

/// `data` followed by an indented copy of each of its lines, the way `FFmpeg`
/// echoes a metadata value (`title`, chapter names) around the filter's own
/// lines. A diagnostics parser must give the same result for both.
fn with_echoed_lines(data: &[u8]) -> Vec<u8> {
    let mut echoed = Vec::with_capacity(data.len().saturating_mul(2).saturating_add(64));
    echoed.extend_from_slice(data);
    echoed.push(b'\n');
    for line in data.split(|byte| *byte == b'\n') {
        echoed.extend_from_slice(ECHO_INDENT);
        echoed.extend_from_slice(line);
        echoed.push(b'\n');
    }
    echoed
}

fn check_frame_showinfo(data: &[u8]) -> Result<(), Violation> {
    let frame = parse_frame_showinfo(data).ok();
    let audio = parse_ashowinfo_start(data).ok();
    let echoed = with_echoed_lines(data);
    if echoed.len() <= MAX_DIAGNOSTIC_BYTES
        && (parse_frame_showinfo(&echoed).ok() != frame
            || parse_ashowinfo_start(&echoed).ok() != audio)
    {
        return Err(Violation::EchoedDiagnosticsChangedResult);
    }
    Ok(())
}

fn check_frame_listing(data: &[u8]) -> Result<(), Violation> {
    let covered = TimeRange::new(
        MediaTime::from_micros(0),
        MediaTime::from_micros(LISTING_WINDOW_MICROS),
    )
    .map_err(|_| Violation::HarnessSetup)?;
    let window = FrameListingWindow::new(
        0,
        1,
        LISTING_TIME_BASE_DENOMINATOR,
        0,
        covered,
        ListingTail::EndOfStream,
    )
    .map_err(|_| Violation::HarnessSetup)?;
    let listing = parse_frame_listing(data, &window).ok();
    if let Some(listing) = &listing {
        let frames = listing.frames();
        let inside = listing.covered().start() == covered.start()
            && listing.covered().end() <= covered.end()
            && frames.iter().all(|frame| {
                frame.pts >= window.first_pts()
                    && frame.pts < window.end_pts()
                    && frame.time < listing.covered().end()
            });
        let ordered = frames.windows(2).all(|pair| match pair {
            [before, after] => before.pts < after.pts && before.time <= after.time,
            _ => true,
        });
        let truncated = listing.tail() == ListingTail::MoreMayFollow;
        if frames.len() > MAX_LISTED_FRAMES
            || !inside
            || !ordered
            || (truncated && listing.covered() == covered)
        {
            return Err(Violation::ListingOutOfBounds);
        }
    }
    let echoed = with_echoed_lines(data);
    if echoed.len() <= MAX_LISTING_DIAGNOSTIC_BYTES
        && parse_frame_listing(&echoed, &window).ok() != listing
    {
        return Err(Violation::EchoedDiagnosticsChangedResult);
    }
    Ok(())
}

/// Builds a [`Target::PngSequence`] input: one byte with the expected number
/// of images, the expected width and height as big-endian 16-bit integers,
/// then the standard output to split.
#[must_use]
pub fn png_sequence_input(expected: u8, width: u16, height: u16, stdout: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(stdout.len().saturating_add(5));
    data.push(expected);
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(stdout);
    data
}

fn check_png_sequence(data: &[u8]) -> Result<(), Violation> {
    let [
        expected,
        width_high,
        width_low,
        height_high,
        height_low,
        stdout @ ..,
    ] = data
    else {
        return Ok(());
    };
    let width = u16::from_be_bytes([*width_high, *width_low]);
    let height = u16::from_be_bytes([*height_high, *height_low]);
    let Ok(dimensions) = FrameDimensions::new(u32::from(width), u32::from(height)) else {
        return Ok(());
    };
    let Ok(images) = parse_png_sequence(stdout, usize::from(*expected), dimensions) else {
        return Ok(());
    };
    let size = |image: &[u8]| {
        let field = |range: std::ops::Range<usize>| {
            image
                .get(range)
                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                .map(u32::from_be_bytes)
        };
        (field(16..20), field(20..24))
    };
    if images.len() != usize::from(*expected)
        || images.concat() != stdout
        || images.iter().any(|image| {
            !image.starts_with(b"\x89PNG\r\n\x1a\n")
                || size(image) != (Some(u32::from(width)), Some(u32::from(height)))
        })
    {
        return Err(Violation::PngSequenceMismatch);
    }
    Ok(())
}
