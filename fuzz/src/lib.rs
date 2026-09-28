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

use vsift_contract::{
    BatchLine, MAX_REQUEST_STEPS, RelativeInputPath, WORK_REQUEST_LIMITS, WorkTarget,
    decode_batch_line, decode_work_request, validate_steps,
};
use vsift_domain::{
    CropRect, CursorToken, FrameDimensions, JobId, ListingTail, MAX_CUE_TEXT_BYTES,
    MAX_LISTED_FRAMES, MAX_RECORD_ITEMS, MAX_RECORD_SELECTIONS, MAX_SEARCH_QUERY_BYTES,
    MAX_SEARCH_TERMS, MAX_TRANSCRIPT_CUES, MAX_WINDOW_CANDIDATES, MediaStreamKind, MediaTime,
    PlannedChunk, SearchMatch, SearchQuery, SessionId, SourceSegmentId, TimeRange,
    TranscriptFormat, VISUAL_FRAME_BYTES, VisualCandidate, VisualCandidateId, VisualChangePolicy,
    VisualIndexWindow, VisualSample, VisualWindow, VisualWindowOutcome, analyse_window,
    normalise_search_text, validate_chunk_output,
};
use vsift_infrastructure::{
    CgroupLimit, CgroupMembership, FrameListingWindow, MAX_CGROUP_DEPTH, MAX_DIAGNOSTIC_BYTES,
    MAX_LISTING_DIAGNOSTIC_BYTES, MAX_NET_DEV_BYTES, MAX_OS_RELEASE_BYTES, MountDevice,
    NetworkInterfaces, SourceContainer, VisualSamplingWindow, WhisperOutputLimits,
    classify_mountinfo, classify_os_release, classify_root_mount, decode_chunk_checkpoint,
    decode_evidence_record, decode_job_record, decode_transcript_record,
    decode_visual_index_record, encode_chunk_checkpoint, encode_evidence_record, encode_job_record,
    encode_transcript_record, encode_visual_index_record, parse_ashowinfo_start,
    parse_cgroup_limit, parse_cpu_max, parse_ffprobe_metadata, parse_frame_listing,
    parse_frame_showinfo, parse_net_dev, parse_png_sequence, parse_proc_cgroup,
    parse_supplied_transcript, parse_visual_samples, parse_whisper_full_json,
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
/// Session every fuzzed evidence record is decoded for: the session of the
/// frozen bundle example the seed copies.
pub const EVIDENCE_FUZZ_SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
/// The displayed frame every [`Target::CropRect`] outer rectangle is parsed
/// against: the 1440x900 frame of the domain's own crop tests.
pub const CROP_FRAME_WIDTH: u32 = 1_440;
/// See [`CROP_FRAME_WIDTH`].
pub const CROP_FRAME_HEIGHT: u32 = 900;
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
    /// The stored evidence record through `decode_evidence_record`, as a
    /// session read and `bundle validate` read it (P09 PR 2).
    EvidenceRecord,
    /// The `crop --rect` text through `CropRect::parse`, then
    /// `CropRect::compose`. The input is an outer rectangle (parsed against a
    /// fixed 1440x900 frame), a line feed, and an inner rectangle (parsed
    /// against the outer one's size).
    CropRect,
    /// `/proc/self/mountinfo` text through `classify_mountinfo`, which decides
    /// whether a session root may claim OS-crash durability (P10, ADR 0020).
    /// The input is the table.
    Mountinfo,
    /// An `os-release` file through `classify_os_release`, which decides
    /// whether the host is the qualified Ubuntu 24.04 (P10 PR 4). The input
    /// is the file.
    OsRelease,
    /// A worker request (`job run --request`, P11) through
    /// `vsift_contract::decode_work_request`. The input is the document.
    JobRequest,
    /// One line of a `job batch` file (P11) through
    /// `vsift_contract::decode_batch_line`. The input is the line.
    JobBatchLine,
    /// A stored job record (`job.json` v1, P10) through `decode_job_record`,
    /// as the store reads it for a fixed job and session (issue #180).
    JobRecord,
    /// A stored chunk checkpoint (`chunks/<ordinal>.json` v1, P10) through
    /// `decode_chunk_checkpoint`, as a resumed run reads ordinal 1 (issue #180).
    ChunkCheckpoint,
    /// The strict worker attestation's kernel files (P11 PR 2, ADR 0021
    /// section 8): the same bytes through `parse_proc_cgroup`, `parse_cpu_max`,
    /// `parse_cgroup_limit` and `parse_net_dev`. The input is the file.
    HostAttestation,
}

impl Target {
    /// Every target, in the order CI runs them.
    pub const ALL: [Self; 21] = [
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
        Self::EvidenceRecord,
        Self::CropRect,
        Self::Mountinfo,
        Self::OsRelease,
        Self::JobRequest,
        Self::JobBatchLine,
        Self::JobRecord,
        Self::ChunkCheckpoint,
        Self::HostAttestation,
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
            Self::EvidenceRecord => "evidence_record",
            Self::CropRect => "crop_rect",
            Self::Mountinfo => "mountinfo",
            Self::OsRelease => "os_release",
            Self::JobRequest => "job_request",
            Self::JobBatchLine => "job_batch_line",
            Self::JobRecord => "job_record",
            Self::ChunkCheckpoint => "chunk_checkpoint",
            Self::HostAttestation => "host_attestation",
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
            Self::EvidenceRecord => check_evidence_record(data),
            Self::CropRect => check_crop_rect(data),
            Self::Mountinfo => check_mountinfo(data),
            Self::OsRelease => check_os_release(data),
            Self::JobRequest => check_job_request(data),
            Self::JobBatchLine => check_job_batch_line(data),
            Self::JobRecord => check_job_record(data),
            Self::ChunkCheckpoint => check_chunk_checkpoint(data),
            Self::HostAttestation => check_host_attestation(data),
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
    /// An accepted evidence record could not be encoded again.
    EvidenceRecordNotReencodable,
    /// An accepted evidence record changed in a round trip, or holds more
    /// items or selections than its bound.
    EvidenceRecordRoundTripChanged,
    /// An accepted crop is outside its frame, has another canonical
    /// spelling, or composes to a rectangle that is not the inner one moved
    /// by the outer one's origin inside the frame.
    CropRectInvalid,
    /// The mount-table verdict depended on the device asked about in a way
    /// whole-table parsing forbids, or changed when the table was repeated.
    MountinfoInconsistent,
    /// The os-release verdict changed when a comment was appended or the
    /// file was repeated.
    OsReleaseInconsistent,
    /// An accepted job request broke its own bounds or step order, or
    /// decoded differently when decoded again or with whitespace appended.
    WorkRequestInconsistent,
    /// An accepted batch line was not the request its bytes decode to, or a
    /// blank line held more than whitespace.
    BatchLineInconsistent,
    /// An accepted job record could not be encoded again.
    JobRecordNotReencodable,
    /// An accepted job record changed in a round trip or names another job.
    JobRecordRoundTripChanged,
    /// An accepted checkpoint could not be encoded again.
    CheckpointNotReencodable,
    /// An accepted checkpoint changed in a round trip or is of another chunk.
    CheckpointRoundTripChanged,
    /// An accepted cgroup, limit or network file broke its bounds, did not
    /// read back from its canonical form, or changed its verdict when a line
    /// was added.
    HostAttestationInconsistent,
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
            Self::EvidenceRecordNotReencodable => {
                "an accepted evidence record could not be encoded again"
            }
            Self::EvidenceRecordRoundTripChanged => {
                "an accepted evidence record changed in a round trip or is unbounded"
            }
            Self::CropRectInvalid => "an accepted crop is outside its frame or not canonical",
            Self::MountinfoInconsistent => "the mount-table verdict is inconsistent",
            Self::OsReleaseInconsistent => "the os-release verdict is inconsistent",
            Self::WorkRequestInconsistent => "an accepted job request is inconsistent",
            Self::BatchLineInconsistent => "an accepted batch line is inconsistent",
            Self::JobRecordNotReencodable => "an accepted job record could not be encoded again",
            Self::JobRecordRoundTripChanged => "an accepted job record changed in a round trip",
            Self::CheckpointNotReencodable => "an accepted checkpoint could not be encoded again",
            Self::CheckpointRoundTripChanged => "an accepted checkpoint changed in a round trip",
            Self::HostAttestationInconsistent => {
                "an accepted kernel file broke its bounds or its canonical form"
            }
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

fn check_evidence_record(data: &[u8]) -> Result<(), Violation> {
    let session = SessionId::parse(EVIDENCE_FUZZ_SESSION).map_err(|_| Violation::HarnessSetup)?;
    let Ok(record) = decode_evidence_record(data, &session) else {
        return Ok(());
    };
    if record.items().len() > MAX_RECORD_ITEMS || record.selections().len() > MAX_RECORD_SELECTIONS
    {
        return Err(Violation::EvidenceRecordRoundTripChanged);
    }
    let encoded =
        encode_evidence_record(&record).map_err(|_| Violation::EvidenceRecordNotReencodable)?;
    match decode_evidence_record(&encoded, &session) {
        Ok(decoded) if decoded == record => Ok(()),
        _ => Err(Violation::EvidenceRecordRoundTripChanged),
    }
}

/// The canonical `x,y,width,height` spelling of a rectangle.
fn crop_text(rect: CropRect) -> String {
    format!(
        "{},{},{},{}",
        rect.x(),
        rect.y(),
        rect.width(),
        rect.height()
    )
}

fn check_crop_rect(data: &[u8]) -> Result<(), Violation> {
    let frame = FrameDimensions::new(CROP_FRAME_WIDTH, CROP_FRAME_HEIGHT)
        .map_err(|_| Violation::HarnessSetup)?;
    let Ok(text) = std::str::from_utf8(data) else {
        return Ok(());
    };
    let (outer_text, inner_text) = text.split_once('\n').unwrap_or((text, ""));
    let Ok(outer) = CropRect::parse(outer_text, frame) else {
        return Ok(());
    };
    let contained = |rect: CropRect, within: FrameDimensions| {
        u64::from(rect.x()) + u64::from(rect.width()) <= u64::from(within.width())
            && u64::from(rect.y()) + u64::from(rect.height()) <= u64::from(within.height())
    };
    if !contained(outer, frame) || crop_text(outer) != outer_text {
        return Err(Violation::CropRectInvalid);
    }
    let Ok(inner) = CropRect::parse(inner_text, outer.dimensions()) else {
        return Ok(());
    };
    if crop_text(inner) != inner_text {
        return Err(Violation::CropRectInvalid);
    }
    let composed = outer
        .compose(inner)
        .map_err(|_| Violation::CropRectInvalid)?;
    let moved = composed.x().checked_sub(outer.x()) == Some(inner.x())
        && composed.y().checked_sub(outer.y()) == Some(inner.y())
        && composed.dimensions() == inner.dimensions();
    if !moved || !contained(composed, frame) {
        return Err(Violation::CropRectInvalid);
    }
    Ok(())
}

/// The devices every mount table is classified for: the first disk's first
/// partition and the anonymous device of `/proc`.
const MOUNTINFO_DEVICES: [(u32, u32); 2] = [(8, 1), (0, 21)];

fn check_mountinfo(data: &[u8]) -> Result<(), Violation> {
    let verdicts = MOUNTINFO_DEVICES
        .map(|(major, minor)| classify_mountinfo(data, MountDevice::new(major, minor)));
    // The whole table is parsed whatever the device, so a table is accepted
    // or refused (with the same error) for every device alike.
    let [first, second] = verdicts;
    if first.is_ok() != second.is_ok() || (first.is_err() && first != second) {
        return Err(Violation::MountinfoInconsistent);
    }
    // The strict worker's root-mount reading (P11) parses the same table
    // with the same parser: it accepts and refuses exactly alike.
    if classify_root_mount(data).map(|_| ()) != first.map(|_| ()) {
        return Err(Violation::MountinfoInconsistent);
    }
    let Ok(first) = first else {
        return Ok(());
    };
    // Repeating a table's lines repeats its mounts: the verdict stays.
    if data.last() == Some(&b'\n') || data.is_empty() {
        let mut twice = data.to_vec();
        twice.extend_from_slice(data);
        let (major, minor) = MOUNTINFO_DEVICES[0];
        match classify_mountinfo(&twice, MountDevice::new(major, minor)) {
            Ok(repeated) if repeated == first => {}
            Err(_) if twice.len() > vsift_infrastructure::MAX_MOUNTINFO_BYTES => {}
            _ => return Err(Violation::MountinfoInconsistent),
        }
    }
    Ok(())
}

/// One `lo` line of `/proc/<pid>/net/dev`, with its sixteen counters.
const NET_DEV_LOOPBACK_LINE: &[u8] = b"    lo: 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n";
/// One line of another interface.
const NET_DEV_OTHER_LINE: &[u8] = b"  eth9: 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n";

fn check_host_attestation(data: &[u8]) -> Result<(), Violation> {
    let inconsistent = Err(Violation::HostAttestationInconsistent);
    if let Ok(CgroupMembership::Unified(components)) = parse_proc_cgroup(data)
        && (components.len() > MAX_CGROUP_DEPTH
            || components
                .iter()
                .any(|component| component.is_empty() || component == "." || component == ".."))
    {
        return inconsistent;
    }
    // An accepted limit reads back from its canonical form.
    let canonical_limit = |limit: CgroupLimit| match limit {
        CgroupLimit::Unlimited => b"max\n".to_vec(),
        CgroupLimit::Finite(value) => format!("{value}\n").into_bytes(),
    };
    if let Ok(limit) = parse_cgroup_limit(data)
        && parse_cgroup_limit(&canonical_limit(limit)) != Ok(limit)
    {
        return inconsistent;
    }
    if let Ok(limit) = parse_cpu_max(data) {
        let mut canonical = canonical_limit(limit);
        canonical.pop();
        canonical.extend_from_slice(b" 100000\n");
        if parse_cpu_max(&canonical) != Ok(limit) {
            return inconsistent;
        }
    }
    // Adding an interface line decides the verdict as documented.
    if let Ok(verdict) = parse_net_dev(data)
        && data.last() == Some(&b'\n')
        && data.len() + NET_DEV_OTHER_LINE.len() <= MAX_NET_DEV_BYTES
    {
        let with = |line: &[u8]| {
            let mut extended = data.to_vec();
            extended.extend_from_slice(line);
            parse_net_dev(&extended)
        };
        let loopback = match verdict {
            NetworkInterfaces::None | NetworkInterfaces::LoopbackOnly => {
                NetworkInterfaces::LoopbackOnly
            }
            NetworkInterfaces::Other => NetworkInterfaces::Other,
        };
        if with(NET_DEV_LOOPBACK_LINE) != Ok(loopback)
            || with(NET_DEV_OTHER_LINE) != Ok(NetworkInterfaces::Other)
        {
            return inconsistent;
        }
    }
    Ok(())
}

fn check_os_release(data: &[u8]) -> Result<(), Violation> {
    let Ok(verdict) = classify_os_release(data) else {
        return Ok(());
    };
    // Only whole lines are appended or repeated, so the file must end with
    // one (or be empty); a result over the bound may be refused instead.
    if !(data.is_empty() || data.last() == Some(&b'\n')) {
        return Ok(());
    }
    let unchanged = |extended: &[u8]| match classify_os_release(extended) {
        Ok(again) => again == verdict,
        Err(_) => extended.len() > MAX_OS_RELEASE_BYTES,
    };
    // A comment and a blank line change nothing.
    let mut commented = data.to_vec();
    commented.extend_from_slice(b"# comment\n\n");
    // Repeating every assignment in order leaves the last value of each key.
    let mut twice = data.to_vec();
    twice.extend_from_slice(data);
    if unchanged(&commented) && unchanged(&twice) {
        Ok(())
    } else {
        Err(Violation::OsReleaseInconsistent)
    }
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

/// The session every fuzzed job record and checkpoint belongs to: that of
/// the committed example records the seeds copy.
pub const JOB_FUZZ_SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
/// The job every fuzzed job record is decoded for: the example records' job.
pub const JOB_FUZZ_JOB: &str = "job_82001d205f5092aaaf15197d26ba34fd";

/// An accepted request keeps its own bounds and step order, decodes to the
/// same request again, and to the same request with trailing whitespace
/// (the digest is whitespace-insensitive) while that stays within the bound.
fn check_job_request(data: &[u8]) -> Result<(), Violation> {
    let Ok(request) = decode_work_request(data) else {
        return Ok(());
    };
    let consistent = data.len() <= WORK_REQUEST_LIMITS.max_bytes
        && request.steps().len() <= MAX_REQUEST_STEPS
        && validate_steps(request.target(), request.steps()).is_ok()
        && request.digest().as_str().len() == 64
        && request
            .digest()
            .as_str()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && paths_reparse(request.target())
        && decode_work_request(data).as_ref() == Ok(&request);
    if !consistent {
        return Err(Violation::WorkRequestInconsistent);
    }
    let mut padded = data.to_vec();
    padded.extend_from_slice(b" \n\t");
    if padded.len() <= WORK_REQUEST_LIMITS.max_bytes
        && decode_work_request(&padded).as_ref() != Ok(&request)
    {
        return Err(Violation::WorkRequestInconsistent);
    }
    Ok(())
}

/// Every path of an accepted target is itself a valid relative input path.
fn paths_reparse(target: &WorkTarget) -> bool {
    let reparses =
        |path: &RelativeInputPath| RelativeInputPath::parse(path.as_str()).as_ref() == Ok(path);
    match target {
        WorkTarget::Ingest { source, transcript } => {
            reparses(source)
                && transcript
                    .as_ref()
                    .is_none_or(|transcript| reparses(transcript.path()))
        }
        WorkTarget::Session(_) => true,
    }
}

/// An accepted line is the request its bytes (without one trailing carriage
/// return) decode to; a blank line is only whitespace.
fn check_job_batch_line(data: &[u8]) -> Result<(), Violation> {
    let Ok(line) = decode_batch_line(data) else {
        return Ok(());
    };
    let body = data.strip_suffix(b"\r").unwrap_or(data);
    let consistent = !body.contains(&b'\n')
        && match line {
            BatchLine::Blank => body.iter().all(u8::is_ascii_whitespace),
            BatchLine::Request(request) => decode_work_request(body).as_ref() == Ok(&*request),
        };
    if consistent {
        Ok(())
    } else {
        Err(Violation::BatchLineInconsistent)
    }
}

fn check_job_record(data: &[u8]) -> Result<(), Violation> {
    let session = SessionId::parse(JOB_FUZZ_SESSION).map_err(|_| Violation::HarnessSetup)?;
    let job = JobId::parse(JOB_FUZZ_JOB).map_err(|_| Violation::HarnessSetup)?;
    let Ok(record) = decode_job_record(data, &job, &session) else {
        return Ok(());
    };
    if record.job_id != job || record.session_id != session {
        return Err(Violation::JobRecordRoundTripChanged);
    }
    let encoded = encode_job_record(&record).map_err(|_| Violation::JobRecordNotReencodable)?;
    match decode_job_record(&encoded, &job, &session) {
        Ok(decoded) if decoded == record => Ok(()),
        _ => Err(Violation::JobRecordRoundTripChanged),
    }
}

fn check_chunk_checkpoint(data: &[u8]) -> Result<(), Violation> {
    let Some(checkpoint) = decode_chunk_checkpoint(data, 1) else {
        return Ok(());
    };
    if checkpoint.index() != 0 || decode_chunk_checkpoint(data, 2).is_some() {
        return Err(Violation::CheckpointRoundTripChanged);
    }
    let encoded =
        encode_chunk_checkpoint(&checkpoint).ok_or(Violation::CheckpointNotReencodable)?;
    if decode_chunk_checkpoint(&encoded, 1) == Some(checkpoint) {
        Ok(())
    } else {
        Err(Violation::CheckpointRoundTripChanged)
    }
}
