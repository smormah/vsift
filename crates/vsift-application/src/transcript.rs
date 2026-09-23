//! Supplied-transcript import and bounded transcript retrieval.
//!
//! Infrastructure parses sidecar bytes and probes the source; this module turns
//! those observations into an identified [`TranscriptRevision`] through the
//! domain's alignment rules, and pages a committed revision for retrieval.
//! Identities are derived from content with SHA-256 so the same session,
//! sidecar and offset always produce the same revision and segment identities,
//! which lets a later index consumer upsert rather than duplicate them.

use std::{error::Error, fmt, future::Future, num::NonZeroU32};

use sha2::{Digest, Sha256};
use vsift_domain::{
    Confidence, CursorError, CursorToken, MediaTime, PageLimit, ParsedTranscript, QueryDigest,
    SessionId, SidecarIdentity, SourceId, SourceSegment, SourceSegmentId, TimeRange,
    TranscriptImportError, TranscriptOffset, TranscriptRevision, TranscriptRevisionError,
    TranscriptRevisionId, TranscriptRevisionParts, TranscriptSegment, TranscriptSegmentId,
    TranscriptSegmentParts, align_imported_cues,
};

const IDENTITY_HEX_LENGTH: usize = 32;
const HEX: &[u8; 16] = b"0123456789abcdef";

/// A supplied transcript that an adapter has read and parsed, with the
/// identity of the exact bytes it parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SuppliedTranscript {
    /// Ordered, syntactically valid cues.
    pub transcript: ParsedTranscript,
    /// Digest and size of the parsed bytes.
    pub sidecar: SidecarIdentity,
}

/// Why a supplied transcript file could not be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuppliedTranscriptError {
    /// The file was read but its content was rejected by the import policy.
    Rejected(TranscriptImportError),
    /// The path is not an absolute, local, directly named file.
    InvalidPath,
    /// The path names something other than a regular file.
    NotRegularFile,
    /// The file could not be read.
    Io,
}

impl fmt::Display for SuppliedTranscriptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(error) => error.fmt(formatter),
            Self::InvalidPath => formatter.write_str("supplied transcript path is invalid"),
            Self::NotRegularFile => {
                formatter.write_str("supplied transcript is not a regular file")
            }
            Self::Io => formatter.write_str("supplied transcript could not be read"),
        }
    }
}

impl Error for SuppliedTranscriptError {}

/// A supplied transcript and the explicit offset to align it with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptImportRequest {
    /// Parsed sidecar.
    pub supplied: SuppliedTranscript,
    /// Explicit shift from sidecar time to source time.
    pub offset: TranscriptOffset,
}

/// Why the probed source duration is unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceProbeError {
    /// The provider could not describe the source as supported media.
    InvalidSource,
    /// Admission capacity was unavailable.
    Busy,
    /// The probe exceeded its deadline.
    Deadline,
    /// The probe was cancelled.
    Cancelled,
    /// The provider's output exceeded a bound.
    ResourceLimit,
    /// The provider could not be started or its output could not be read.
    Io,
}

impl fmt::Display for SourceProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSource => "source media could not be probed",
            Self::Busy => "source probe admission is busy",
            Self::Deadline => "source probe exceeded its deadline",
            Self::Cancelled => "source probe was cancelled",
            Self::ResourceLimit => "source probe output exceeded its bound",
            Self::Io => "source probe could not run",
        })
    }
}

impl Error for SourceProbeError {}

/// Port that reports the normalized duration of a held source snapshot.
///
/// The duration bounds which supplied cues can be cited: a cue beyond the
/// end of the media cannot refer to anything in it.
pub trait SourceDurationProbe<S>: Send + Sync {
    /// Probes `source` and returns its positive normalized duration.
    fn source_duration(
        &self,
        source: &S,
    ) -> impl Future<Output = Result<MediaTime, SourceProbeError>> + Send;
}

/// Why an imported revision could not be built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptBuildError {
    /// The alignment policy rejected the import.
    Rejected(TranscriptImportError),
    /// The assembled revision violated an invariant; this is an internal fault.
    Invalid(TranscriptRevisionError),
}

impl fmt::Display for TranscriptBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(error) => error.fmt(formatter),
            Self::Invalid(error) => error.fmt(formatter),
        }
    }
}

impl Error for TranscriptBuildError {}

/// Inputs that identify and place one imported revision.
#[derive(Clone, Copy, Debug)]
pub struct ImportedRevisionRequest<'a> {
    /// Session that will hold the revision.
    pub session_id: &'a SessionId,
    /// Source the revision describes.
    pub source_id: &'a SourceId,
    /// Probed normalized source duration.
    pub source_duration: MediaTime,
    /// Parsed sidecar and its identity.
    pub supplied: &'a SuppliedTranscript,
    /// Explicit offset.
    pub offset: TranscriptOffset,
    /// Revision number within the session.
    pub number: NonZeroU32,
}

/// Returns the single closed source segment of a finite file.
///
/// The identity depends only on the source bytes and the segment index, so
/// every session over the same file names the segment identically.
///
/// # Errors
///
/// Returns [`TranscriptRevisionError::EmptySourceSegment`] for a zero duration.
pub fn whole_file_source_segment(
    source_id: &SourceId,
    duration: MediaTime,
) -> Result<SourceSegment, TranscriptRevisionError> {
    let id = SourceSegmentId::parse(derived_identity(
        "sgm_",
        "vsift.source-segment.v1",
        &[source_id.as_str(), "0"],
    ))
    .map_err(|_| TranscriptRevisionError::InvalidIdentity)?;
    SourceSegment::whole_file(id, duration)
}

/// Aligns a supplied transcript and assembles its identified revision.
///
/// Every segment carries unknown confidence with origin `unavailable`,
/// because a supplied file states no score.
///
/// # Errors
///
/// Returns [`TranscriptBuildError::Rejected`] when the alignment policy leaves
/// nothing to import, and [`TranscriptBuildError::Invalid`] on an internal fault.
pub fn build_imported_revision(
    request: ImportedRevisionRequest<'_>,
) -> Result<TranscriptRevision, TranscriptBuildError> {
    let source_segment = whole_file_source_segment(request.source_id, request.source_duration)
        .map_err(TranscriptBuildError::Invalid)?;
    let transcript = &request.supplied.transcript;
    let (aligned, warnings) = align_imported_cues(transcript, request.offset, &source_segment)
        .map_err(TranscriptBuildError::Rejected)?;
    let revision_id = TranscriptRevisionId::parse(derived_identity(
        "trv_",
        "vsift.transcript-revision.v1",
        &[
            request.session_id.as_str(),
            &request.number.get().to_string(),
            request.supplied.sidecar.sha256(),
            transcript.format().identifier(),
            &request.offset.as_micros().to_string(),
        ],
    ))
    .map_err(|_| TranscriptBuildError::Invalid(TranscriptRevisionError::InvalidIdentity))?;
    let mut segments = Vec::with_capacity(aligned.len());
    for (index, aligned) in aligned.into_iter().enumerate() {
        let ordinal = u32::try_from(index + 1)
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or(TranscriptBuildError::Invalid(
                TranscriptRevisionError::TooManySegments,
            ))?;
        segments.push(TranscriptSegment::new(TranscriptSegmentParts {
            id: transcript_segment_id(&revision_id, ordinal.get())
                .map_err(TranscriptBuildError::Invalid)?,
            ordinal,
            range: aligned.range,
            text: aligned.cue.text,
            speaker: aligned.cue.speaker,
            confidence: Confidence::unknown(),
            cue: aligned.cue.source,
            cue_timing: aligned.cue.timing,
        }));
    }
    TranscriptRevision::new(TranscriptRevisionParts {
        id: revision_id,
        number: request.number,
        source_id: request.source_id.clone(),
        source_segment,
        origin: transcript.format().alignment_origin(),
        offset: request.offset,
        sidecar: request.supplied.sidecar.clone(),
        language: transcript.language().cloned(),
        segments,
        warnings,
    })
    .map_err(TranscriptBuildError::Invalid)
}

/// Derives the identity of the segment at `ordinal` in `revision`.
///
/// # Errors
///
/// Returns [`TranscriptRevisionError::InvalidIdentity`] if the derived value
/// is not canonical, which would indicate an internal fault.
pub fn transcript_segment_id(
    revision: &TranscriptRevisionId,
    ordinal: u32,
) -> Result<TranscriptSegmentId, TranscriptRevisionError> {
    TranscriptSegmentId::parse(derived_identity(
        "tsg_",
        "vsift.transcript-segment.v1",
        &[revision.as_str(), &ordinal.to_string()],
    ))
    .map_err(|_| TranscriptRevisionError::InvalidIdentity)
}

/// A bounded request for transcript segments in a source range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptPageRequest {
    /// Half-open source range; segments intersecting it are returned.
    pub range: TimeRange,
    /// Maximum segments on this page.
    pub limit: PageLimit,
    /// Opaque continuation token from the previous page of the same query.
    pub cursor: Option<String>,
}

/// One bounded page of transcript segments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptPage<'a> {
    /// Segments in ordinal order.
    pub segments: Vec<&'a TranscriptSegment>,
    /// Token for the next page, absent on the last page.
    pub next_cursor: Option<CursorToken>,
}

/// Why a transcript page request was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranscriptQueryError {
    /// The cursor is malformed, expired, or belongs to another query, session
    /// or revision.
    Cursor(CursorError),
}

impl fmt::Display for TranscriptQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cursor(error) => error.fmt(formatter),
        }
    }
}

impl Error for TranscriptQueryError {}

/// Returns one page of `revision` for `request`.
///
/// A cursor is bound to the session, the revision number (its immutable
/// snapshot), a digest of the revision identity and range, and an expiry. A
/// cursor from any other query, session or revision is rejected rather than
/// silently restarting, as the public paging contract requires.
///
/// # Errors
///
/// Returns [`TranscriptQueryError::Cursor`] for a rejected cursor.
pub fn page_transcript<'a>(
    session_id: &SessionId,
    revision: &'a TranscriptRevision,
    request: &TranscriptPageRequest,
    cursor_expires_at_micros: u64,
    now_micros: u64,
) -> Result<TranscriptPage<'a>, TranscriptQueryError> {
    let digest = transcript_query_digest(revision.id(), request.range)
        .map_err(TranscriptQueryError::Cursor)?;
    let snapshot = u64::from(revision.number());
    let after = match &request.cursor {
        None => None,
        Some(encoded) => {
            let token = CursorToken::parse(encoded).map_err(TranscriptQueryError::Cursor)?;
            token
                .validate_scope(session_id, snapshot, &digest, now_micros)
                .map_err(TranscriptQueryError::Cursor)?;
            Some(parse_ordinal(token.last_item_key(), revision)?)
        }
    };
    let slice = revision.page(request.range, after, request.limit);
    let next_cursor = match (slice.has_more, slice.segments.last()) {
        (true, Some(last)) => Some(
            CursorToken::new(
                session_id.clone(),
                snapshot,
                digest,
                last.ordinal().to_string(),
                cursor_expires_at_micros,
            )
            .map_err(TranscriptQueryError::Cursor)?,
        ),
        _ => None,
    };
    Ok(TranscriptPage {
        segments: slice.segments,
        next_cursor,
    })
}

fn parse_ordinal(key: &str, revision: &TranscriptRevision) -> Result<u32, TranscriptQueryError> {
    let canonical = !key.starts_with('0') && key.bytes().all(|byte| byte.is_ascii_digit());
    let ordinal = key
        .parse::<u32>()
        .ok()
        .filter(|ordinal| canonical && *ordinal >= 1)
        .ok_or(TranscriptQueryError::Cursor(
            CursorError::InvalidLastItemKey,
        ))?;
    if usize::try_from(ordinal).map_or(true, |ordinal| ordinal > revision.segments().len()) {
        return Err(TranscriptQueryError::Cursor(
            CursorError::InvalidLastItemKey,
        ));
    }
    Ok(ordinal)
}

/// Digest of the canonical transcript query: revision identity and range.
///
/// The page size is deliberately excluded, so a caller may change it between
/// pages without invalidating its cursor.
///
/// # Errors
///
/// Returns [`CursorError::InvalidQueryDigest`] only if the digest were not
/// canonical, which would be an internal fault.
pub fn transcript_query_digest(
    revision: &TranscriptRevisionId,
    range: TimeRange,
) -> Result<QueryDigest, CursorError> {
    QueryDigest::parse(sha256_hex(
        format!(
            "vsift.transcript.get.v1
{}
{}
{}",
            revision.as_str(),
            range.start().as_micros(),
            range.end().as_micros()
        )
        .as_bytes(),
    ))
}

fn derived_identity(prefix: &str, domain: &str, parts: &[&str]) -> String {
    let mut material = String::from(domain);
    for part in parts {
        material.push('\n');
        material.push_str(part);
    }
    let digest = sha256_hex(material.as_bytes());
    // A truncated SHA-256 keeps identities within the public 16..64 suffix
    // bound; an empty suffix (impossible for a 64-digit digest) fails parsing.
    format!(
        "{prefix}{}",
        digest.get(..IDENTITY_HEX_LENGTH).unwrap_or_default()
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    text
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use vsift_domain::{
        CueSource, CueText, CueTiming, CursorError, CursorToken, ImportedCue, MediaTime, PageLimit,
        ParsedTranscript, SessionId, SidecarIdentity, SourceId, TimeRange, TranscriptFormat,
        TranscriptOffset, TranscriptRevision, TranscriptWarnings,
    };

    use super::{
        ImportedRevisionRequest, SuppliedTranscript, TranscriptPageRequest, TranscriptQueryError,
        build_imported_revision, page_transcript,
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const EXPIRES: u64 = 2_000_000_000_000_000;
    const NOW: u64 = 1_000_000_000_000_000;

    fn supplied(count: u32) -> Result<SuppliedTranscript, Box<dyn std::error::Error>> {
        let mut cues = Vec::new();
        for ordinal in 1..=count {
            let start = u64::from(ordinal) * 1_000_000;
            cues.push(ImportedCue {
                source: CueSource::new(
                    NonZeroU32::new(ordinal).ok_or("zero")?,
                    NonZeroU32::new(ordinal * 4).ok_or("zero")?,
                ),
                timing: CueTiming::new(start, start + 900_000)?,
                text: CueText::new(format!("cue {ordinal}"), format!("cue {ordinal}"))?,
                speaker: None,
            });
        }
        Ok(SuppliedTranscript {
            transcript: ParsedTranscript::new(
                TranscriptFormat::WebVtt,
                None,
                cues,
                TranscriptWarnings::default(),
            )?,
            sidecar: SidecarIdentity::new(DIGEST, 100)?,
        })
    }

    fn session(suffix: &str) -> Result<SessionId, Box<dyn std::error::Error>> {
        Ok(SessionId::parse(format!("ses_{suffix}"))?)
    }

    fn revision(
        session_id: &SessionId,
        offset: i64,
    ) -> Result<TranscriptRevision, Box<dyn std::error::Error>> {
        let supplied = supplied(8)?;
        Ok(build_imported_revision(ImportedRevisionRequest {
            session_id,
            source_id: &SourceId::from_sha256(DIGEST)?,
            source_duration: MediaTime::from_micros(60_000_000),
            supplied: &supplied,
            offset: TranscriptOffset::from_micros(offset)?,
            number: NonZeroU32::MIN,
        })?)
    }

    fn range(from: u64, to: u64) -> Result<TimeRange, Box<dyn std::error::Error>> {
        Ok(TimeRange::new(
            MediaTime::from_micros(from),
            MediaTime::from_micros(to),
        )?)
    }

    #[test]
    fn identities_are_derived_from_session_sidecar_and_offset() -> TestResult {
        let session_a = session("0123456789abcdef")?;
        let first = revision(&session_a, 0)?;
        let again = revision(&session_a, 0)?;
        let shifted = revision(&session_a, 1)?;
        let other_session = revision(&session("fedcba9876543210")?, 0)?;

        assert_eq!(first, again);
        assert!(first.id().as_str().starts_with("trv_"));
        assert!(first.segments()[0].id().as_str().starts_with("tsg_"));
        assert!(first.source_segment().id().as_str().starts_with("sgm_"));
        assert_ne!(first.id(), shifted.id());
        assert_ne!(first.id(), other_session.id());
        // The source segment is a property of the source bytes alone.
        assert_eq!(
            first.source_segment().id(),
            other_session.source_segment().id()
        );
        assert_ne!(first.segments()[0].id(), first.segments()[1].id());
        Ok(())
    }

    /// C-03: pages have no gaps or repeats and cursors are scoped to their query.
    #[test]
    fn cursor_pages_cover_the_range_exactly_once() -> TestResult {
        let session_id = session("0123456789abcdef")?;
        let revision = revision(&session_id, 0)?;
        let mut request = TranscriptPageRequest {
            range: range(2_500_000, 7_000_000)?,
            limit: PageLimit::new(2)?,
            cursor: None,
        };
        let mut seen = Vec::new();
        loop {
            let page = page_transcript(&session_id, &revision, &request, EXPIRES, NOW)?;
            seen.extend(page.segments.iter().map(|segment| segment.ordinal()));
            match page.next_cursor {
                Some(cursor) => request.cursor = Some(cursor.encode()),
                None => break,
            }
        }
        // Cues 2..=6 start at 2..6 s and last 0.9 s; 2 ends at 2.9 s > 2.5 s.
        assert_eq!(seen, [2, 3, 4, 5, 6]);

        let single = page_transcript(
            &session_id,
            &revision,
            &TranscriptPageRequest {
                range: range(0, 60_000_000)?,
                limit: PageLimit::new(100)?,
                cursor: None,
            },
            EXPIRES,
            NOW,
        )?;
        assert_eq!(single.segments.len(), 8);
        assert!(single.next_cursor.is_none());
        Ok(())
    }

    #[test]
    fn cursors_from_another_query_session_revision_or_time_are_rejected() -> TestResult {
        let session_id = session("0123456789abcdef")?;
        let revision = revision(&session_id, 0)?;
        let request = TranscriptPageRequest {
            range: range(0, 60_000_000)?,
            limit: PageLimit::new(1)?,
            cursor: None,
        };
        let first = page_transcript(&session_id, &revision, &request, EXPIRES, NOW)?;
        let cursor = first.next_cursor.ok_or("expected a cursor")?.encode();
        let continued = |range: TimeRange, session_id: &SessionId, now: u64| {
            page_transcript(
                session_id,
                &revision,
                &TranscriptPageRequest {
                    range,
                    limit: PageLimit::new(1).unwrap_or(PageLimit::DEFAULT),
                    cursor: Some(cursor.clone()),
                },
                EXPIRES,
                now,
            )
            .map(|page| page.segments.len())
        };

        assert_eq!(continued(range(0, 60_000_000)?, &session_id, NOW), Ok(1));
        assert_eq!(
            continued(range(0, 30_000_000)?, &session_id, NOW),
            Err(TranscriptQueryError::Cursor(CursorError::WrongQuery))
        );
        assert_eq!(
            continued(range(0, 60_000_000)?, &session("fedcba9876543210")?, NOW),
            Err(TranscriptQueryError::Cursor(CursorError::WrongSession))
        );
        assert_eq!(
            continued(range(0, 60_000_000)?, &session_id, EXPIRES),
            Err(TranscriptQueryError::Cursor(CursorError::Expired))
        );

        let forged = CursorToken::parse(&cursor)?;
        for key in ["9", "0", "01", "abc"] {
            let token = CursorToken::new(
                session_id.clone(),
                1,
                super::transcript_query_digest(revision.id(), range(0, 60_000_000)?)?,
                key,
                EXPIRES,
            )?;
            assert_ne!(token, forged);
            let result = page_transcript(
                &session_id,
                &revision,
                &TranscriptPageRequest {
                    range: range(0, 60_000_000)?,
                    limit: PageLimit::DEFAULT,
                    cursor: Some(token.encode()),
                },
                EXPIRES,
                NOW,
            );
            assert_eq!(
                result.map(|page| page.segments.len()),
                Err(TranscriptQueryError::Cursor(
                    CursorError::InvalidLastItemKey
                )),
                "{key}"
            );
        }
        let other_revision = CursorToken::new(
            session_id.clone(),
            2,
            super::transcript_query_digest(revision.id(), range(0, 60_000_000)?)?,
            "1",
            EXPIRES,
        )?;
        assert_eq!(
            page_transcript(
                &session_id,
                &revision,
                &TranscriptPageRequest {
                    range: range(0, 60_000_000)?,
                    limit: PageLimit::DEFAULT,
                    cursor: Some(other_revision.encode()),
                },
                EXPIRES,
                NOW,
            )
            .map(|page| page.segments.len()),
            Err(TranscriptQueryError::Cursor(CursorError::WrongGeneration))
        );
        Ok(())
    }
}
