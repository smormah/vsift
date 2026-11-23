//! Conformance of the `bundle search` contract (ADR 0035): the result, full and
//! brief, against the published v1 schema and the frozen example
//! `examples/bundle-search.json`; the status, which follows the transcript and
//! never the screen; and the largest page, within a host's 1 MiB result.

use std::{fs, io, num::NonZeroU32, path::PathBuf};

use jsonschema::{Retrieve, Uri};
use serde_json::{Value, json};
use vsift_application::{
    BundleHit, BundleHitEvidence, BundleIdentity, BundleNotSearched, BundleNotSearchedReason,
    BundleScreenText, BundleScreenTextHit, BundleSearchCounts, BundleSearchOutcome,
    BundleSessionCoverage, BundleSessionEntry, BundleTranscript, BundleUnreadable,
    ImportedRevisionRequest, SuppliedTranscript, build_imported_revision,
};
use vsift_contract::{
    BUNDLE_NOT_PRIVATE_WARNING, BUNDLE_SEARCH_PARTIAL_WARNING, BundleSearchData, ResultView,
    TranscriptSegmentData, bundle_search_response,
};
use vsift_domain::{
    BundleHitPosition, Confidence, ConfidenceOrigin, CoverageBasis, CueSource, CueText, CueTiming,
    FrameDimensions, FrameRef, ImportedCue, MediaTime, ParsedTranscript, ScreenText,
    ScreenTextIndexId, ScreenTextLine, ScreenTextLineId, SearchMatch, SearchQuery, SessionId,
    SidecarIdentity, SourceId, TextRegion, TimeBase, TranscriptFormat, TranscriptOffset,
    TranscriptRevision, TranscriptWarnings,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SOURCE: &str = "src_sha256_d7ccece71288c5ff35d7b16c871a93a8ed48bbb7380617899575069b4d6545f4";
const DIGEST: &str = "95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d";
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";
const EXAMPLE: &str = "examples/bundle-search.json";
const SECOND: u64 = 1_000_000;
/// A host's result limit (`MAX_RESULT_BYTES` in `vsift-cli`).
const RESULT_LIMIT: usize = 1_048_576;

fn schema_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1")
}

fn load(relative_path: &str) -> Built<Value> {
    Ok(serde_json::from_str(&fs::read_to_string(
        schema_root().join(relative_path),
    )?)?)
}

/// Resolves sibling `$ref`s by their published identifier to the local copy.
struct PublishedSchemas;

impl Retrieve for PublishedSchemas {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let name = uri
            .as_str()
            .strip_prefix(SCHEMA_BASE)
            .filter(|name| !name.contains(['/', '\\']))
            .ok_or_else(|| format!("unpublished schema reference: {uri}"))?;
        Ok(serde_json::from_str(&fs::read_to_string(
            schema_root().join(name),
        )?)?)
    }
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&load(schema_path)?)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

fn session(number: u32) -> Built<SessionId> {
    Ok(SessionId::parse(format!("ses_{number:032x}"))?)
}

fn identity(number: u32) -> Built<BundleIdentity> {
    Ok(BundleIdentity {
        session_id: session(number)?,
        source_id: SourceId::parse(SOURCE)?,
        source_included: false,
        manifest_sha256: format!("{number:064x}"),
    })
}

/// A revision of one cue saying `text` at 2 s.
fn revision(number: u32, text: &str) -> Built<TranscriptRevision> {
    let cues = vec![ImportedCue {
        source: CueSource::new(NonZeroU32::MIN, NonZeroU32::MIN),
        timing: CueTiming::new(2 * SECOND, 3 * SECOND)?,
        text: CueText::new(text.to_owned(), text.to_owned())?,
        speaker: None,
    }];
    let supplied = SuppliedTranscript {
        transcript: ParsedTranscript::new(
            TranscriptFormat::WebVtt,
            None,
            cues,
            TranscriptWarnings::default(),
        )?,
        sidecar: SidecarIdentity::new(DIGEST, 100)?,
    };
    Ok(build_imported_revision(ImportedRevisionRequest {
        session_id: &session(number)?,
        source_id: &SourceId::parse(SOURCE)?,
        source_duration: MediaTime::from_micros(10 * SECOND),
        supplied: &supplied,
        offset: TranscriptOffset::ZERO,
        number: NonZeroU32::MIN,
    })?)
}

fn transcript_hit(
    number: u32,
    bundle: &str,
    rank: u16,
    text: &str,
) -> Built<BundleHit<TranscriptSegmentData>> {
    let revision = revision(number, text)?;
    let segment = revision.segments().first().ok_or("no segment")?;
    Ok(BundleHit {
        position: BundleHitPosition::new(SearchMatch::Phrase, rank, 1),
        bundle: bundle.to_owned(),
        identity: identity(number)?,
        evidence_id: segment.id().as_str().to_owned(),
        time: segment.range().start(),
        evidence: BundleHitEvidence::Transcript(TranscriptSegmentData::new(&revision, segment)),
        continuation: format!("cursor-after-{bundle}"),
    })
}

fn screen_hit(number: u32, bundle: &str, rank: u16) -> Built<BundleHit<TranscriptSegmentData>> {
    let dimensions = FrameDimensions::new(1280, 720)?;
    let time = MediaTime::from_micros(5 * SECOND);
    let line = ScreenTextLine {
        id: ScreenTextLineId::parse(format!("stl_{}", "c".repeat(32)))?,
        region: TextRegion::new(10, 20, 200, 24, dimensions)?,
        text: ScreenText::parse(String::from("Refund limit: 50"))?,
        confidence: Confidence::provider_score(9_000, ConfidenceOrigin::ProviderUncalibrated)?,
    };
    Ok(BundleHit {
        position: BundleHitPosition::new(SearchMatch::AllTerms, rank, 1),
        bundle: bundle.to_owned(),
        identity: identity(number)?,
        evidence_id: line.id.as_str().to_owned(),
        time,
        evidence: BundleHitEvidence::ScreenText(BundleScreenTextHit {
            line,
            frame: FrameRef {
                stream_index: 0,
                time_base: TimeBase::new(1, 1)?,
                pts: 5,
                time,
                dimensions,
            },
            candidates: Vec::new(),
            last_time: time,
            frames: 1,
        }),
        continuation: format!("cursor-after-{bundle}"),
    })
}

fn complete_transcript(number: u32) -> Built<BundleTranscript> {
    Ok(BundleTranscript::Read {
        revision_id: revision(number, "refund limit")?.id().clone(),
        basis: CoverageBasis::SuppliedTranscript,
        untranscribed_ranges: 0,
        untranscribed_micros: 0,
    })
}

fn searched(
    number: u32,
    bundle: &str,
    hits: usize,
    transcript: BundleTranscript,
    screen_text: BundleScreenText,
) -> Built<BundleSessionEntry> {
    Ok(BundleSessionEntry::Searched(Box::new(
        BundleSessionCoverage {
            bundle: bundle.to_owned(),
            identity: identity(number)?,
            hits,
            transcript,
            screen_text,
        },
    )))
}

/// Three recordings: `call-1`, heard and read; `call-2`, with a gap, whose
/// on-screen text alone holds the query; and `call-3`, not private.
fn outcome() -> Built<BundleSearchOutcome<TranscriptSegmentData>> {
    Ok(BundleSearchOutcome {
        hits: vec![
            transcript_hit(1, "call-1", 0, "The refund limit is 50.")?,
            screen_hit(2, "call-2", 1)?,
        ],
        total_hits: 3,
        has_more: true,
        counts: BundleSearchCounts {
            found: 4,
            searched: 2,
            unavailable: 1,
            not_searched: 1,
            duplicates: 0,
            skipped_entries: 1,
            screen_text_incomplete: 1,
        },
        sessions: vec![
            searched(
                1,
                "call-1",
                2,
                complete_transcript(1)?,
                BundleScreenText::NotRun,
            )?,
            searched(
                2,
                "call-2",
                1,
                BundleTranscript::Read {
                    revision_id: revision(2, "refund limit")?.id().clone(),
                    basis: CoverageBasis::LocalAsr,
                    untranscribed_ranges: 2,
                    untranscribed_micros: 14 * SECOND,
                },
                BundleScreenText::Read {
                    index_id: ScreenTextIndexId::parse(format!("sti_{}", "d".repeat(32)))?,
                    period_micros: 10 * SECOND,
                    coverage: vsift_domain::ScreenTextCoverage {
                        planned: 6,
                        read: 6,
                        ..Default::default()
                    },
                    candidates_complete: true,
                    visual_unavailable: None,
                },
            )?,
            BundleSessionEntry::Unavailable {
                bundle: String::from("call-3"),
                reason: BundleUnreadable::NotPrivate,
            },
        ],
        not_searched: vec![BundleNotSearched {
            bundle: String::from("call-4"),
            reason: BundleNotSearchedReason::ReadBudget,
        }],
    })
}

fn query() -> Built<SearchQuery> {
    Ok(SearchQuery::parse("refund limit")?)
}

#[test]
fn a_page_matches_the_schema_and_the_frozen_example() -> TestResult {
    let response = bundle_search_response(&query()?, &outcome()?, ResultView::Full)?;
    let value = serde_json::to_value(&response)?;
    validate("operation-response.schema.json", &value)?;
    validate("bundle-search-data.schema.json", &value["data"])?;
    assert_eq!(value, load(EXAMPLE)?, "the result differs from {EXAMPLE}");
    assert_eq!(value["command"], "bundle.search");
    assert_eq!(value["lifecycle"]["mode"], "retained");
    assert_eq!(value["status"], "partial");
    assert_eq!(
        value["warnings"],
        json!([BUNDLE_SEARCH_PARTIAL_WARNING, BUNDLE_NOT_PRIVATE_WARNING])
    );
    assert_eq!(
        value["coverage"],
        json!({
            "truncated": true,
            "gaps": [],
            "reasons": ["untranscribed_range", "bundle_unavailable", "read_budget"],
        })
    );
    Ok(())
}

#[test]
fn the_brief_view_changes_only_the_segments() -> TestResult {
    let full = serde_json::to_value(bundle_search_response(
        &query()?,
        &outcome()?,
        ResultView::Full,
    )?)?;
    let brief = serde_json::to_value(bundle_search_response(
        &query()?,
        &outcome()?,
        ResultView::Brief,
    )?)?;
    validate("bundle-search-data.schema.json", &brief["data"])?;
    assert_eq!(brief["data"]["view"], "brief");
    assert!(brief["data"]["hits"][0]["segment"]["revision_id"].is_null());
    assert_eq!(brief["data"]["sessions"], full["data"]["sessions"]);
    assert_eq!(brief["data"]["hits"][1], full["data"]["hits"][1]);
    assert_eq!(brief["coverage"], full["coverage"]);
    Ok(())
}

/// The planner's ruling: on-screen text never changes the status. Nothing here
/// but bundles that `ocr` never read, and the result is `complete`, its gap
/// counted and in each entry.
#[test]
fn on_screen_gaps_never_change_the_status() -> TestResult {
    let outcome = BundleSearchOutcome {
        hits: Vec::new(),
        total_hits: 0,
        has_more: false,
        counts: BundleSearchCounts {
            found: 1,
            searched: 1,
            screen_text_incomplete: 1,
            ..BundleSearchCounts::default()
        },
        sessions: vec![searched(
            1,
            "call-1",
            0,
            complete_transcript(1)?,
            BundleScreenText::NotRun,
        )?],
        not_searched: Vec::new(),
    };
    let value = serde_json::to_value(bundle_search_response(
        &query()?,
        &outcome,
        ResultView::Full,
    )?)?;
    validate("bundle-search-data.schema.json", &value["data"])?;
    assert_eq!(value["status"], "complete");
    assert_eq!(value["warnings"], json!([]));
    assert_eq!(
        value["coverage"],
        json!({"truncated": false, "gaps": [], "reasons": []})
    );
    assert_eq!(value["data"]["bundles"]["screen_text_incomplete"], 1);
    assert_eq!(
        value["data"]["sessions"][0]["screen_text"]["state"],
        "not_run"
    );
    Ok(())
}

/// A bundle with no transcript is a transcript gap, and says which.
#[test]
fn a_bundle_with_no_transcript_makes_the_result_partial() -> TestResult {
    let outcome = BundleSearchOutcome {
        hits: Vec::new(),
        total_hits: 0,
        has_more: false,
        counts: BundleSearchCounts::default(),
        sessions: vec![searched(
            1,
            "call-1",
            0,
            BundleTranscript::None,
            BundleScreenText::NotRun,
        )?],
        not_searched: Vec::new(),
    };
    let value = serde_json::to_value(bundle_search_response(
        &query()?,
        &outcome,
        ResultView::Full,
    )?)?;
    assert_eq!(value["status"], "partial");
    assert_eq!(value["coverage"]["reasons"], json!(["no_transcript"]));
    Ok(())
}

/// The largest page: 100 hits of the longest text, written as hidden characters
/// that a display form makes eight times longer, beside 64 coverage entries and
/// 64 bundles not searched, all with the longest names. It does not fit whole:
/// what gives way is the size of the page, and its cursor goes on from the last
/// hit shown, so no hit is lost.
#[test]
fn the_largest_page_fits_the_result_limit_and_loses_no_hit() -> TestResult {
    let name = "n".repeat(255);
    let text = "\u{202e}".repeat(1_365);
    let mut hits = Vec::new();
    for rank in 0..100_u16 {
        let mut hit = transcript_hit(u32::from(rank) + 1, &name, rank, &text)?;
        hit.continuation = format!("cursor-{rank}");
        hits.push(hit);
    }
    let mut sessions = Vec::new();
    for number in 1..=64 {
        sessions.push(searched(
            number,
            &name,
            1,
            BundleTranscript::Read {
                revision_id: revision(number, "x")?.id().clone(),
                basis: CoverageBasis::Mixed,
                untranscribed_ranges: 100,
                untranscribed_micros: u64::MAX,
            },
            BundleScreenText::Read {
                index_id: ScreenTextIndexId::parse(format!("sti_{}", "d".repeat(32)))?,
                period_micros: 10 * SECOND,
                coverage: vsift_domain::ScreenTextCoverage {
                    planned: 4_096,
                    read: 4_096,
                    ..Default::default()
                },
                candidates_complete: true,
                visual_unavailable: None,
            },
        )?);
    }
    let not_searched = vec![
        BundleNotSearched {
            bundle: name.clone(),
            reason: BundleNotSearchedReason::BundleLimit,
        };
        64
    ];
    let outcome = BundleSearchOutcome {
        hits,
        total_hits: 5_000,
        has_more: true,
        counts: BundleSearchCounts {
            found: 4_096,
            searched: 64,
            not_searched: 4_032,
            ..BundleSearchCounts::default()
        },
        sessions,
        not_searched,
    };
    let response = bundle_search_response(&query()?, &outcome, ResultView::Full)?;
    let bytes = serde_json::to_vec(&response)?;
    assert!(
        bytes.len() <= RESULT_LIMIT,
        "{} bytes is over the limit",
        bytes.len()
    );
    let value = serde_json::to_value(&response)?;
    validate("bundle-search-data.schema.json", &value["data"])?;
    let shown = value["data"]["hits"].as_array().map_or(0, Vec::len);
    assert!((1..100).contains(&shown), "{shown} hits were shown");
    assert_eq!(
        value["data"]["next_cursor"],
        format!("cursor-{}", shown - 1),
        "the cursor must go on from the last hit shown"
    );
    Ok(())
}

/// A short last page ends without a cursor; a page the limit filled has one.
#[test]
fn the_cursor_is_there_only_when_a_hit_follows() -> TestResult {
    let mut last_page = outcome()?;
    last_page.has_more = false;
    let data = serde_json::to_value(BundleSearchData::new(
        &query()?,
        &last_page,
        ResultView::Full,
    ))?;
    assert!(data["next_cursor"].is_null());
    let data = serde_json::to_value(BundleSearchData::new(
        &query()?,
        &outcome()?,
        ResultView::Full,
    ))?;
    assert_eq!(data["next_cursor"], "cursor-after-call-2");
    Ok(())
}
