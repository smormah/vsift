//! Normalisation, query validation, tier rules, ranking and coverage.

use std::num::{NonZeroU16, NonZeroU32};

use proptest::{
    collection::vec,
    prelude::{Just, Strategy, any, prop_assert, prop_assert_eq, prop_oneof, proptest},
};

use super::{
    CoverageBasis, MAX_SEARCH_QUERY_BYTES, MAX_SEARCH_TERMS, SearchCoverage, SearchMatch,
    SearchPosition, SearchQuery, SearchQueryRejection, normalise_search_text, search_revision,
};
use crate::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, CarriedFrom, ChunkPlan, ChunkTime, Confidence,
    CueSource, CueText, CueTiming, InheritedRevision, MediaTime, PageLimit, ProviderEndTrim,
    SegmentOrigin, Sha256Hex, SidecarIdentity, SourceId, SourceSegment, SourceSegmentId, TimeRange,
    TranscriptFormat, TranscriptOffset, TranscriptProvenance, TranscriptRevision,
    TranscriptRevisionId, TranscriptRevisionParts, TranscriptSegment, TranscriptSegmentId,
    TranscriptSegmentParts, TranscriptWarnings, plan_chunks,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SECOND: u64 = 1_000_000;

fn range(start: u64, end: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(start),
        MediaTime::from_micros(end),
    )?)
}

fn words(text: &str) -> Vec<String> {
    normalise_search_text(text)
}

fn query(text: &str) -> Built<SearchQuery> {
    Ok(SearchQuery::parse(text)?)
}

fn source_segment(duration: u64) -> Built<SourceSegment> {
    Ok(SourceSegment::whole_file(
        SourceSegmentId::parse("sgm_0123456789abcdef")?,
        MediaTime::from_micros(duration),
    )?)
}

fn cue_segment(ordinal: u32, start: u64, end: u64, words: &str) -> Built<TranscriptSegment> {
    Ok(TranscriptSegment::new(TranscriptSegmentParts {
        id: TranscriptSegmentId::parse(format!("tsg_{ordinal:016x}"))?,
        ordinal: NonZeroU32::new(ordinal).ok_or("zero")?,
        range: range(start, end)?,
        text: CueText::new(words.to_owned(), words.to_owned())?,
        speaker: None,
        confidence: Confidence::unknown(),
        origin: SegmentOrigin::ImportedCue {
            cue: CueSource::new(
                NonZeroU32::new(ordinal).ok_or("zero")?,
                NonZeroU32::new(ordinal * 4).ok_or("zero")?,
            ),
            timing: CueTiming::new(start, end)?,
        },
    }))
}

fn imported_provenance() -> Built<TranscriptProvenance> {
    Ok(TranscriptProvenance::Imported {
        format: TranscriptFormat::Srt,
        sidecar: SidecarIdentity::new(DIGEST, 10)?,
        offset: TranscriptOffset::ZERO,
    })
}

/// An imported revision of a `duration` source whose cues are `texts`, one
/// per second from 1 s, each lasting half a second.
fn imported(duration: u64, texts: &[&str]) -> Built<TranscriptRevision> {
    let mut segments = Vec::new();
    for (ordinal, text) in (1_u32..).zip(texts) {
        let start = u64::from(ordinal) * SECOND;
        segments.push(cue_segment(ordinal, start, start + SECOND / 2, text)?);
    }
    Ok(TranscriptRevision::new(TranscriptRevisionParts {
        id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
        number: NonZeroU32::MIN,
        source_id: SourceId::from_sha256(DIGEST)?,
        source_segment: source_segment(duration)?,
        provenance: imported_provenance()?,
        supersedes: None,
        replaced_range: None,
        inherited: Vec::new(),
        language: None,
        segments,
        warnings: TranscriptWarnings::default(),
    })?)
}

/// A local-ASR run over `covered` with one outcome per planned chunk.
fn run(covered: TimeRange, outcomes: &[AsrChunkOutcome]) -> Built<AsrRun> {
    let planned = plan_chunks(
        &SourceSegmentId::parse("sgm_0123456789abcdef")?,
        covered,
        ChunkPlan::R0,
    )?;
    Ok(AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero")?,
        audio_stream: 1,
        chunks: planned
            .into_iter()
            .zip(outcomes)
            .map(|(chunk, outcome)| AsrChunkRecord::new(chunk, *outcome))
            .collect(),
    })?)
}

/// A local-ASR revision of a 100 s source, with segments recognised in its
/// chunks at `[start, end)` source times.
fn local_asr(
    run: AsrRun,
    segments: &[(u32, u64, u64, &str)],
    audio_starts: &[u64],
) -> Built<TranscriptRevision> {
    let mut built = Vec::new();
    for (ordinal, &(chunk, start, end, words)) in (1_u32..).zip(segments) {
        let audio_start = *audio_starts
            .get(usize::try_from(chunk)?)
            .ok_or("no audio start")?;
        built.push(TranscriptSegment::new(TranscriptSegmentParts {
            id: TranscriptSegmentId::parse(format!("tsg_{ordinal:016x}"))?,
            ordinal: NonZeroU32::new(ordinal).ok_or("zero")?,
            range: range(start, end)?,
            text: CueText::new(words.to_owned(), words.to_owned())?,
            speaker: None,
            confidence: Confidence::unknown(),
            origin: SegmentOrigin::Asr {
                chunk,
                provider_start: ChunkTime::from_micros(start - audio_start),
                provider_end: ChunkTime::from_micros(end - audio_start),
                trimmed: ProviderEndTrim::Unchanged,
            },
        }));
    }
    Ok(TranscriptRevision::new(TranscriptRevisionParts {
        id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
        number: NonZeroU32::MIN,
        source_id: SourceId::from_sha256(DIGEST)?,
        source_segment: source_segment(100 * SECOND)?,
        provenance: TranscriptProvenance::LocalAsr(run),
        supersedes: None,
        replaced_range: None,
        inherited: Vec::new(),
        language: None,
        segments: built,
        warnings: TranscriptWarnings::default(),
    })?)
}

#[test]
fn normalisation_follows_the_documented_rules() {
    let cases: [(&str, &[&str]); 14] = [
        ("2,048 rows", &["2048", "rows"]),
        ("1,048,576", &["1048576"]),
        ("1,2345", &["1", "2345"]),
        ("E-409", &["e409"]),
        ("AB-731", &["ab731"]),
        ("10:32", &["10", "32"]),
        ("125.00 and 127.50", &["125", "and", "127.5"]),
        ("version 3.14.15", &["version", "3.14.15"]),
        ("Twelve apples, twenty-one", &["12", "apples", "twentyone"]),
        ("thirty-two", &["thirtytwo"]),
        (
            "Dialog R-17 is displayed now.",
            &["dialog", "r17", "is", "displayed", "now"],
        ),
        ("-leading- and trailing.", &["leading", "and", "trailing"]),
        ("Straße ÉCOLE", &["straße", "école"]),
        ("", &[]),
    ];
    for (text, expected) in cases {
        assert_eq!(words(text), expected, "{text}");
    }
}

#[test]
fn queries_are_bounded_and_rejections_are_typed() {
    assert_eq!(SearchQuery::parse(""), Err(SearchQueryRejection::Empty));
    assert_eq!(
        SearchQuery::parse(" -- !! "),
        Err(SearchQueryRejection::Empty)
    );
    assert_eq!(
        SearchQuery::parse(&"a".repeat(MAX_SEARCH_QUERY_BYTES + 1)),
        Err(SearchQueryRejection::TooLong)
    );
    assert!(SearchQuery::parse(&"a".repeat(MAX_SEARCH_QUERY_BYTES)).is_ok());
    // Multi-byte characters count by bytes, not characters.
    assert_eq!(
        SearchQuery::parse(&"é".repeat(129)),
        Err(SearchQueryRejection::TooLong)
    );
    for control in ["a\tb", "a\nb", "a\u{1b}[31mb", "a\u{7f}", "a\u{85}b"] {
        assert_eq!(
            SearchQuery::parse(control),
            Err(SearchQueryRejection::ControlCharacter),
            "{control:?}"
        );
    }
    let sixteen = vec!["w"; MAX_SEARCH_TERMS].join(" ");
    assert!(SearchQuery::parse(&sixteen).is_ok());
    assert_eq!(
        SearchQuery::parse(&format!("{sixteen} w")),
        Err(SearchQueryRejection::TooManyTerms)
    );
    let identifiers: Vec<&str> = SearchQueryRejection::ALL
        .into_iter()
        .map(SearchQueryRejection::identifier)
        .collect();
    assert_eq!(
        identifiers,
        ["empty", "too_long", "too_many_terms", "control_character"]
    );
}

#[test]
fn phrases_match_consecutive_words_and_never_part_of_a_word() -> TestResult {
    let text = "Dialog R-17 is displayed now.";
    assert_eq!(query("R-17")?.classify(text), Some(SearchMatch::Phrase));
    assert_eq!(
        query("dialog r 17")?.classify(text),
        Some(SearchMatch::Phrase)
    );
    assert_eq!(
        query("DIALOG R17 IS")?.classify(text),
        Some(SearchMatch::Phrase)
    );
    assert_eq!(
        query("AB 731")?.classify("Order AB-731."),
        Some(SearchMatch::Phrase)
    );
    assert_eq!(query("407")?.classify("invoice 4407"), None);
    assert_eq!(query("440")?.classify("invoice 4407"), None);
    assert_eq!(query("r 1")?.classify(text), None);
    assert_eq!(
        query("seventeen")?.classify("R 17"),
        Some(SearchMatch::Phrase)
    );
    assert_eq!(
        query("2048")?.classify("2,048 rows"),
        Some(SearchMatch::Phrase)
    );
    assert_eq!(
        query("127.5")?.classify("127.50 ms"),
        Some(SearchMatch::Phrase)
    );
    Ok(())
}

#[test]
fn all_terms_match_every_word_in_any_order() -> TestResult {
    let text = "Dialog R-17 is displayed now.";
    assert_eq!(
        query("displayed dialog")?.classify(text),
        Some(SearchMatch::AllTerms)
    );
    assert_eq!(
        query("now r17")?.classify(text),
        Some(SearchMatch::AllTerms)
    );
    assert_eq!(query("dialog missing")?.classify(text), None);
    // A single word is a phrase or nothing.
    assert_eq!(query("dialog")?.classify(text), Some(SearchMatch::Phrase));
    assert_eq!(SearchMatch::Phrase.tier(), 1);
    assert_eq!(SearchMatch::AllTerms.tier(), 2);
    assert_eq!(SearchMatch::from_tier(3), None);
    for tier in SearchMatch::ALL {
        assert_eq!(SearchMatch::from_tier(tier.tier()), Some(tier));
    }
    Ok(())
}

/// A phrase split across two segments is not found: each segment is matched
/// alone (documented limit).
#[test]
fn phrases_do_not_cross_segments() -> TestResult {
    let revision = imported(10 * SECOND, &["open dialog", "R-17 now"])?;
    let slice = search_revision(
        &revision,
        &query("dialog r17")?,
        None,
        None,
        PageLimit::DEFAULT,
    );
    assert!(slice.hits.is_empty());
    Ok(())
}

fn ranked(
    revision: &TranscriptRevision,
    query: &SearchQuery,
    window: Option<TimeRange>,
) -> Vec<(SearchMatch, u32)> {
    search_revision(
        revision,
        query,
        window,
        None,
        PageLimit::new(PageLimit::MAX).unwrap_or(PageLimit::DEFAULT),
    )
    .hits
    .iter()
    .map(|hit| (hit.tier(), hit.segment().ordinal()))
    .collect()
}

#[test]
fn hits_rank_by_tier_then_start_and_respect_the_window() -> TestResult {
    let revision = imported(
        20 * SECOND,
        &[
            "error code AB then 731",
            "the AB-731 error",
            "nothing here",
            "731 AB again",
            "AB 731 repeated",
        ],
    )?;
    let search = query("AB 731")?;
    assert_eq!(
        ranked(&revision, &search, None),
        [
            (SearchMatch::Phrase, 2),
            (SearchMatch::Phrase, 5),
            (SearchMatch::AllTerms, 1),
            (SearchMatch::AllTerms, 4),
        ]
    );
    // Segments 2..=4 start at 2..=4 s; [2.2 s, 4.1 s) intersects 2, 3 and 4.
    assert_eq!(
        ranked(&revision, &search, Some(range(2_200_000, 4_100_000)?)),
        [(SearchMatch::Phrase, 2), (SearchMatch::AllTerms, 4)]
    );
    assert_eq!(
        ranked(&revision, &search, None),
        ranked(&revision, &search, None),
        "ranking is deterministic"
    );
    Ok(())
}

#[test]
fn pages_continue_after_their_last_position() -> TestResult {
    let revision = imported(
        20 * SECOND,
        &["a b", "b a", "a b", "b x a", "a b", "a", "b a"],
    )?;
    let search = query("a b")?;
    let full = ranked(&revision, &search, None);
    assert_eq!(full.len(), 6);
    for limit in 1..=7 {
        let mut after: Option<SearchPosition> = None;
        let mut seen = Vec::new();
        loop {
            let slice = search_revision(&revision, &search, None, after, PageLimit::new(limit)?);
            assert!(slice.hits.len() <= usize::from(limit));
            seen.extend(
                slice
                    .hits
                    .iter()
                    .map(|hit| (hit.tier(), hit.segment().ordinal())),
            );
            match (slice.has_more, slice.hits.last()) {
                (true, Some(last)) => after = Some(last.position()),
                (false, _) => break,
                (true, None) => return Err("more hits but an empty page".into()),
            }
        }
        assert_eq!(seen, full, "limit {limit}");
    }
    Ok(())
}

#[test]
fn a_supplied_transcript_covers_its_whole_source_unverified() -> TestResult {
    let revision = imported(12 * SECOND, &["one", "two"])?;
    let whole = SearchCoverage::of(&revision, None);
    assert_eq!(whole.basis(), CoverageBasis::SuppliedTranscript);
    assert_eq!(whole.searched(), Some(range(0, 12 * SECOND)?));
    assert_eq!(whole.transcribed(), [range(0, 12 * SECOND)?]);
    assert!(whole.untranscribed().is_empty() && whole.no_speech().is_empty());
    assert!(whole.is_complete());

    // A window past the end of the source is clipped to it.
    let clipped = SearchCoverage::of(&revision, Some(range(10 * SECOND, 90 * SECOND)?));
    assert_eq!(clipped.searched(), Some(range(10 * SECOND, 12 * SECOND)?));
    assert!(clipped.is_complete());

    let outside = SearchCoverage::of(&revision, Some(range(20 * SECOND, 30 * SECOND)?));
    assert_eq!(outside.searched(), None);
    assert!(outside.transcribed().is_empty() && outside.is_complete());
    let identifiers: Vec<&str> = CoverageBasis::ALL
        .into_iter()
        .map(CoverageBasis::identifier)
        .collect();
    assert_eq!(identifiers, ["supplied_transcript", "local_asr", "mixed"]);
    Ok(())
}

#[test]
fn local_asr_coverage_is_what_its_chunks_examined() -> TestResult {
    // Chunks [0,30) silent, [25,55) transcribed, [50,70) without audio.
    let asr = run(
        range(0, 70 * SECOND)?,
        &[
            AsrChunkOutcome::Silent {
                audio: range(0, 30 * SECOND)?,
            },
            AsrChunkOutcome::Transcribed {
                audio: range(25 * SECOND, 55 * SECOND)?,
            },
            AsrChunkOutcome::NoAudio,
        ],
    )?;
    let revision = local_asr(
        asr,
        &[(1, 30 * SECOND, 32 * SECOND, "hello there")],
        &[0, 25 * SECOND, 50 * SECOND],
    )?;
    let coverage = SearchCoverage::of(&revision, None);
    assert_eq!(coverage.basis(), CoverageBasis::LocalAsr);
    assert_eq!(coverage.transcribed(), [range(0, 70 * SECOND)?]);
    assert_eq!(
        coverage.untranscribed(),
        [range(70 * SECOND, 100 * SECOND)?]
    );
    assert_eq!(
        coverage.no_speech(),
        [range(0, 25 * SECOND)?, range(55 * SECOND, 70 * SECOND)?]
    );
    assert!(!coverage.is_complete());

    let inside = SearchCoverage::of(&revision, Some(range(10 * SECOND, 60 * SECOND)?));
    assert!(inside.is_complete());
    assert_eq!(
        inside.no_speech(),
        [
            range(10 * SECOND, 25 * SECOND)?,
            range(55 * SECOND, 60 * SECOND)?
        ]
    );
    let straddling = SearchCoverage::of(&revision, Some(range(65 * SECOND, 80 * SECOND)?));
    assert_eq!(straddling.transcribed(), [range(65 * SECOND, 70 * SECOND)?]);
    assert_eq!(
        straddling.untranscribed(),
        [range(70 * SECOND, 80 * SECOND)?]
    );
    Ok(())
}

/// A retranscribed range spliced into an imported revision covers the whole
/// source, but part of it rests on the unverified supplied file.
#[test]
fn a_spliced_revision_carrying_supplied_text_is_mixed() -> TestResult {
    let own_run = run(
        range(20 * SECOND, 30 * SECOND)?,
        &[AsrChunkOutcome::Silent {
            audio: range(20 * SECOND, 30 * SECOND)?,
        }],
    )?;
    let base = TranscriptRevisionId::parse("trv_1111111111111111")?;
    let carried = cue_segment(1, 2 * SECOND, 3 * SECOND, "supplied words")?.with_carried_from(
        CarriedFrom::new(
            base.clone(),
            TranscriptSegmentId::parse("tsg_1111111111111111")?,
        ),
    );
    let revision = TranscriptRevision::new(TranscriptRevisionParts {
        id: TranscriptRevisionId::parse("trv_2222222222222222")?,
        number: NonZeroU32::new(2).ok_or("zero")?,
        source_id: SourceId::from_sha256(DIGEST)?,
        source_segment: source_segment(100 * SECOND)?,
        provenance: TranscriptProvenance::LocalAsr(own_run),
        supersedes: Some(base.clone()),
        replaced_range: Some(range(20 * SECOND, 30 * SECOND)?),
        inherited: vec![InheritedRevision::new(base, imported_provenance()?, None)],
        language: None,
        segments: vec![carried],
        warnings: TranscriptWarnings::default(),
    })?;
    let coverage = SearchCoverage::of(&revision, None);
    assert_eq!(coverage.basis(), CoverageBasis::Mixed);
    assert_eq!(coverage.transcribed(), [range(0, 100 * SECOND)?]);
    assert!(coverage.is_complete());
    assert_eq!(coverage.no_speech(), [range(20 * SECOND, 30 * SECOND)?]);
    Ok(())
}

/// Text built from words that normalise to themselves, joined by the
/// separators the rules treat specially.
fn text_strategy() -> impl Strategy<Value = String> {
    let word = prop_oneof![
        Just("AB".to_owned()),
        Just("731".to_owned()),
        Just("r".to_owned()),
        Just("17".to_owned()),
        Just("Twelve".to_owned()),
        Just("2".to_owned()),
        Just("048".to_owned()),
        Just("125.00".to_owned()),
        Just("Straße".to_owned()),
        "[a-zA-Z0-9]{1,6}",
    ];
    let separator = prop_oneof![
        Just(" "),
        Just("-"),
        Just(","),
        Just(":"),
        Just("."),
        Just("\n"),
        Just("!"),
    ];
    vec((word, separator), 0..12).prop_map(|parts| {
        parts
            .into_iter()
            .fold(String::new(), |mut text, (word, separator)| {
                text.push_str(&word);
                text.push_str(separator);
                text
            })
    })
}

proptest! {
    /// Normalising the words of a normalised text again changes nothing, and
    /// every word is non-empty and holds only letters, digits and points.
    #[test]
    fn normalisation_is_idempotent(text in any::<String>()) {
        let first = words(&text);
        prop_assert_eq!(words(&first.join(" ")), first.clone());
        for word in &first {
            prop_assert!(!word.is_empty());
            prop_assert!(word.chars().all(|c| c.is_alphanumeric() || c == '.'));
        }
    }

    #[test]
    fn structured_normalisation_is_idempotent(text in text_strategy()) {
        let first = words(&text);
        prop_assert_eq!(words(&first.join(" ")), first);
    }

    /// Any run of consecutive words of a text, used as a query, is a phrase
    /// match of that text; its words in reverse order match at least as all
    /// terms.
    #[test]
    fn consecutive_words_are_phrase_matches(
        text in text_strategy(),
        start in 0_usize..12,
        length in 1_usize..5,
    ) {
        let normalised = words(&text);
        let run = normalised
            .get(start..(start + length).min(normalised.len()))
            .unwrap_or_default();
        if !run.is_empty() {
            if let Ok(phrase) = SearchQuery::parse(&run.join(" ")) {
                prop_assert_eq!(phrase.classify(&text), Some(SearchMatch::Phrase));
            }
            let reversed: Vec<&str> = run.iter().rev().map(String::as_str).collect();
            if let Ok(reversed) = SearchQuery::parse(&reversed.join(" ")) {
                prop_assert!(reversed.classify(&text).is_some());
            }
        }
    }

    /// Paging any query at any limit visits exactly the single-page ranking:
    /// no gap, no duplicate, the same order.
    #[test]
    fn paging_is_deterministic_and_complete(
        texts in vec(text_strategy(), 1..30),
        query_text in text_strategy(),
        limit in 1_u16..8,
    ) {
        let texts: Vec<String> = texts
            .into_iter()
            .map(|text| if words(&text).is_empty() { "filler".to_owned() } else { text })
            .collect();
        let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
        let Ok(revision) = imported(40 * SECOND, &borrowed) else {
            return Err(proptest::test_runner::TestCaseError::fail("revision"));
        };
        let Ok(search) = SearchQuery::parse(&query_text) else {
            return Ok(());
        };
        let full = ranked(&revision, &search, None);
        let mut sorted = full.clone();
        sorted.sort_unstable();
        prop_assert_eq!(&sorted, &full);
        let Ok(limit) = PageLimit::new(limit) else {
            return Err(proptest::test_runner::TestCaseError::fail("limit"));
        };
        let mut after = None;
        let mut seen = Vec::new();
        loop {
            let slice = search_revision(&revision, &search, None, after, limit);
            seen.extend(slice.hits.iter().map(|hit| (hit.tier(), hit.segment().ordinal())));
            match (slice.has_more, slice.hits.last()) {
                (true, Some(last)) => after = Some(last.position()),
                _ => break,
            }
        }
        prop_assert_eq!(seen, full);
    }
}
