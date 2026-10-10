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
        carried_untranscribed: Vec::new(),
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
        carried_untranscribed: Vec::new(),
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
        carried_untranscribed: Vec::new(),
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

/// #353: a window whose recognised output was unusable was examined and is not
/// covered, and it is not no speech either: the recognizer may have been given
/// speech it could not place. What the neighbouring windows (which overlap it
/// by five seconds) transcribed stays covered.
#[test]
fn an_unusable_window_is_untranscribed_not_silent() -> TestResult {
    // Chunks [0,30) transcribed, [25,55) unusable, [50,70) transcribed.
    let asr = run(
        range(0, 70 * SECOND)?,
        &[
            AsrChunkOutcome::Transcribed {
                audio: range(0, 30 * SECOND)?,
            },
            AsrChunkOutcome::Unusable {
                audio: range(25 * SECOND, 55 * SECOND)?,
            },
            AsrChunkOutcome::Transcribed {
                audio: range(50 * SECOND, 70 * SECOND)?,
            },
        ],
    )?;
    let revision = local_asr(
        asr,
        &[
            (0, 2 * SECOND, 3 * SECOND, "first words"),
            (2, 60 * SECOND, 62 * SECOND, "last words"),
        ],
        &[0, 25 * SECOND, 50 * SECOND],
    )?;
    let coverage = SearchCoverage::of(&revision, None);
    assert_eq!(coverage.basis(), CoverageBasis::LocalAsr);
    assert_eq!(
        coverage.transcribed(),
        [range(0, 30 * SECOND)?, range(50 * SECOND, 70 * SECOND)?]
    );
    // The middle of the unusable window, and the tail nothing examined.
    assert_eq!(
        coverage.untranscribed(),
        [
            range(30 * SECOND, 50 * SECOND)?,
            range(70 * SECOND, 100 * SECOND)?
        ]
    );
    assert!(coverage.no_speech().is_empty());
    assert!(!coverage.is_complete());

    // Beside a silent neighbour, the unusable window is still not no speech,
    // and what the silent window overlaps of it is silent.
    let asr = run(
        range(0, 70 * SECOND)?,
        &[
            AsrChunkOutcome::Silent {
                audio: range(0, 30 * SECOND)?,
            },
            AsrChunkOutcome::Unusable {
                audio: range(25 * SECOND, 55 * SECOND)?,
            },
            AsrChunkOutcome::Transcribed {
                audio: range(50 * SECOND, 70 * SECOND)?,
            },
        ],
    )?;
    let revision = local_asr(
        asr,
        &[(2, 60 * SECOND, 62 * SECOND, "last words")],
        &[0, 25 * SECOND, 50 * SECOND],
    )?;
    let coverage = SearchCoverage::of(&revision, None);
    assert_eq!(
        coverage.untranscribed(),
        [
            range(30 * SECOND, 50 * SECOND)?,
            range(70 * SECOND, 100 * SECOND)?
        ]
    );
    assert_eq!(coverage.no_speech(), [range(0, 30 * SECOND)?]);
    Ok(())
}

type Spliced = Result<TranscriptRevision, crate::TranscriptRevisionError>;

/// A revision that supersedes a four-chunk run over 0-100 s (chunk windows start
/// at 0, 25, 50 and 75 s, and every one was read) with a run of its own over
/// `own_range`. Times are microseconds.
struct Splice<'a> {
    own_range: TimeRange,
    own_outcomes: &'a [AsrChunkOutcome],
    /// Segments carried from the earlier run: its chunk, start and end.
    carried: &'a [(u32, u64, u64)],
    /// Segments the run wrote itself: its chunk, start and end.
    written: &'a [(u32, u64, u64)],
    /// What the superseded revision did not cover outside the range.
    carried_untranscribed: &'a [(u64, u64)],
}

impl Splice<'_> {
    /// `Err` inside is the revision's own refusal of what it is given.
    fn build(&self) -> Built<Spliced> {
        let base_run = run(
            range(0, 100 * SECOND)?,
            &[
                AsrChunkOutcome::Transcribed {
                    audio: range(0, 30 * SECOND)?,
                },
                AsrChunkOutcome::Transcribed {
                    audio: range(25 * SECOND, 55 * SECOND)?,
                },
                AsrChunkOutcome::Transcribed {
                    audio: range(50 * SECOND, 80 * SECOND)?,
                },
                AsrChunkOutcome::Transcribed {
                    audio: range(75 * SECOND, 100 * SECOND)?,
                },
            ],
        )?;
        let own_run = run(self.own_range, self.own_outcomes)?;
        let base = TranscriptRevisionId::parse("trv_1111111111111111")?;
        // (start, end, chunk, carried: the position among the carried ones).
        let mut placed: Vec<(u64, u64, u32, Option<u32>)> = Vec::new();
        for (index, &(chunk, start, end)) in (1_u32..).zip(self.carried) {
            placed.push((start, end, chunk, Some(index)));
        }
        for &(chunk, start, end) in self.written {
            placed.push((start, end, chunk, None));
        }
        placed.sort_unstable();
        let mut segments = Vec::new();
        for (ordinal, &(start, end, chunk, carried)) in (1_u32..).zip(&placed) {
            let audio_start = match carried {
                Some(_) => u64::from(chunk) * 25 * SECOND,
                None => match own_run.chunks().get(usize::try_from(chunk)?) {
                    Some(record) => record.chunk().window().start().as_micros(),
                    None => return Err("no such chunk of the run".into()),
                },
            };
            let segment = TranscriptSegment::new(TranscriptSegmentParts {
                id: TranscriptSegmentId::parse(format!("tsg_{ordinal:016x}"))?,
                ordinal: NonZeroU32::new(ordinal).ok_or("zero")?,
                range: range(start, end)?,
                text: CueText::new("words".to_owned(), "words".to_owned())?,
                speaker: None,
                confidence: Confidence::unknown(),
                origin: SegmentOrigin::Asr {
                    chunk,
                    provider_start: ChunkTime::from_micros(start - audio_start),
                    provider_end: ChunkTime::from_micros(end - audio_start),
                    trimmed: ProviderEndTrim::Unchanged,
                },
            });
            segments.push(match carried {
                Some(index) => segment.with_carried_from(CarriedFrom::new(
                    base.clone(),
                    TranscriptSegmentId::parse(format!("tsg_1{index:015x}"))?,
                )),
                None => segment,
            });
        }
        let mut unread = Vec::new();
        for &(start, end) in self.carried_untranscribed {
            unread.push(range(start, end)?);
        }
        Ok(TranscriptRevision::new(TranscriptRevisionParts {
            id: TranscriptRevisionId::parse("trv_2222222222222222")?,
            number: NonZeroU32::new(2).ok_or("zero")?,
            source_id: SourceId::from_sha256(DIGEST)?,
            source_segment: source_segment(100 * SECOND)?,
            provenance: TranscriptProvenance::LocalAsr(own_run),
            supersedes: Some(base.clone()),
            replaced_range: Some(self.own_range),
            inherited: if self.carried.is_empty() {
                Vec::new()
            } else {
                vec![InheritedRevision::new(
                    base,
                    TranscriptProvenance::LocalAsr(base_run),
                    None,
                )]
            },
            carried_untranscribed: unread,
            language: None,
            segments,
            warnings: TranscriptWarnings::default(),
        }))
    }
}

/// The revision of a run that re-examined 20-50 s as a single chunk whose answer
/// was unusable, carrying the earlier segments `(earlier chunk, start s, end
/// s)`.
fn spliced_over_an_unusable_window(carried: &[(u32, u64, u64)]) -> Built<Spliced> {
    spliced_over(
        range(20 * SECOND, 50 * SECOND)?,
        &[AsrChunkOutcome::Unusable {
            audio: range(20 * SECOND, 50 * SECOND)?,
        }],
        carried,
    )
}

/// Like [`spliced_over_an_unusable_window`], for a run that re-examined
/// `own_range` as the chunks `own_outcomes` and wrote no text. Times in seconds.
fn spliced_over(
    own_range: TimeRange,
    own_outcomes: &[AsrChunkOutcome],
    carried: &[(u32, u64, u64)],
) -> Built<Spliced> {
    let in_micros: Vec<(u32, u64, u64)> = carried
        .iter()
        .map(|&(chunk, start, end)| (chunk, start * SECOND, end * SECOND))
        .collect();
    Splice {
        own_range,
        own_outcomes,
        carried: &in_micros,
        written: &[],
        carried_untranscribed: &[],
    }
    .build()
}

/// The two-chunk run of 20-75 s of these tests: 20-50 s unusable, 45-75 s read
/// (transcribed, or silent). Its gap is 20-45 s.
fn read_after_an_unusable_window(
    read: fn(TimeRange) -> AsrChunkOutcome,
) -> Built<[AsrChunkOutcome; 2]> {
    Ok([
        AsrChunkOutcome::Unusable {
            audio: range(20 * SECOND, 50 * SECOND)?,
        },
        read(range(45 * SECOND, 75 * SECOND)?),
    ])
}

const fn transcribed(audio: TimeRange) -> AsrChunkOutcome {
    AsrChunkOutcome::Transcribed { audio }
}

const fn silent(audio: TimeRange) -> AsrChunkOutcome {
    AsrChunkOutcome::Silent { audio }
}

/// #353: text the earlier revision had inside a gap of the run (a part of its
/// range that its unusable chunks left unread, no neighbour covering it) is
/// kept, and covered, as itself; the rest of the gap stays untranscribed. A
/// carried segment that lies in the replaced range anywhere else is refused:
/// the run's own text replaced it.
#[test]
fn text_kept_inside_an_unusable_gap_is_covered_and_the_rest_of_the_gap_is_not() -> TestResult {
    // A segment of the earlier chunk 1 (25-55 s), at 30-32 s, inside the gap.
    let revision = spliced_over_an_unusable_window(&[(0, 5, 7), (1, 30, 32)])??;
    let coverage = SearchCoverage::of(&revision, None);
    assert_eq!(
        coverage.transcribed(),
        [
            range(0, 20 * SECOND)?,
            range(30 * SECOND, 32 * SECOND)?,
            range(50 * SECOND, 100 * SECOND)?
        ]
    );
    assert_eq!(
        coverage.untranscribed(),
        [
            range(20 * SECOND, 30 * SECOND)?,
            range(32 * SECOND, 50 * SECOND)?
        ]
    );
    assert!(coverage.no_speech().is_empty());
    // The kept text is what a search finds there.
    assert_eq!(
        revision
            .segments()
            .iter()
            .filter(|segment| segment.carried_from().is_some())
            .count(),
        2
    );

    // A segment that crosses the edge of the replaced range itself is never
    // carried: a range is widened to whole segments, so none does.
    let across_the_range = spliced_over_an_unusable_window(&[(1, 48, 52)])?;
    assert_eq!(
        across_the_range,
        Err(crate::TranscriptRevisionError::InvalidCarriedSegment)
    );
    Ok(())
}

/// #353: earlier text in the part of the range the run read is replaced by what
/// it read, so a carried segment there that touches no gap is refused.
#[test]
fn earlier_text_wholly_in_what_the_run_read_is_refused() -> TestResult {
    let own = range(20 * SECOND, 75 * SECOND)?;
    let outcomes = read_after_an_unusable_window(transcribed)?;
    // 60-62 s lies in the replaced range, where the run read the audio itself.
    assert_eq!(
        spliced_over(own, &outcomes, &[(2, 60, 62)])?,
        Err(crate::TranscriptRevisionError::InvalidCarriedSegment)
    );
    // 45-47 s starts where the gap ends: it touches the gap at one point only.
    assert_eq!(
        spliced_over(own, &outcomes, &[(1, 45, 47)])?,
        Err(crate::TranscriptRevisionError::InvalidCarriedSegment)
    );
    Ok(())
}

/// #353, the rule for text that crosses the edge of a gap: the gap's edge is the
/// edge of a neighbouring window, so a sentence there is read only in part, and
/// a run replaces what it read in full. Text that reaches into a gap is kept
/// whole, by however little it does, when none of the run's own text overlaps
/// it, and it is covered, so the part of the gap it lies in is not untranscribed.
#[test]
fn earlier_text_that_crosses_the_edge_of_a_gap_is_kept_whole_and_covered() -> TestResult {
    let own = range(20 * SECOND, 75 * SECOND)?;
    for (label, outcomes) in [
        ("read", read_after_an_unusable_window(transcribed)?),
        ("silent", read_after_an_unusable_window(silent)?),
    ] {
        // One microsecond, one second, and nearly the end of the earlier chunk.
        for end in [45 * SECOND + 1, 46 * SECOND, 54 * SECOND] {
            let revision = Splice {
                own_range: own,
                own_outcomes: &outcomes,
                carried: &[(1, 30 * SECOND, end)],
                written: &[],
                carried_untranscribed: &[],
            }
            .build()?
            .map_err(|error| format!("{label} {end}: {error:?}"))?;
            assert_eq!(revision.segments().len(), 1, "{label} {end}");
            let coverage = SearchCoverage::of(&revision, None);
            // The kept text is covered whole and so is the window the run
            // read: what is left of the gap is the part before the text.
            assert_eq!(
                coverage.untranscribed(),
                [range(20 * SECOND, 30 * SECOND)?],
                "{label} {end}"
            );
        }
    }
    Ok(())
}

/// #353: when the run's own text overlaps text that crosses a gap's edge, the
/// run has heard the same words and its text replaces the earlier text, as it
/// replaces any other. Text the run's own text only touches is not overlapped.
#[test]
fn earlier_text_that_the_runs_own_text_overlaps_is_replaced() -> TestResult {
    let own = range(20 * SECOND, 75 * SECOND)?;
    let outcomes = read_after_an_unusable_window(transcribed)?;
    // The run's chunk 1 (audio from 45 s) wrote 46-47 s. The earlier segment
    // 44 s to 46 s plus one microsecond overlaps it by that microsecond.
    let overlapped = Splice {
        own_range: own,
        own_outcomes: &outcomes,
        carried: &[(1, 44 * SECOND, 46 * SECOND + 1)],
        written: &[(1, 46 * SECOND, 47 * SECOND)],
        carried_untranscribed: &[],
    }
    .build()?;
    assert_eq!(
        overlapped,
        Err(crate::TranscriptRevisionError::InvalidCarriedSegment)
    );
    // Ending where the run's text starts, it only touches it and is kept.
    let touching = Splice {
        own_range: own,
        own_outcomes: &outcomes,
        carried: &[(1, 44 * SECOND, 46 * SECOND)],
        written: &[(1, 46 * SECOND, 47 * SECOND)],
        carried_untranscribed: &[],
    }
    .build()??;
    assert_eq!(touching.segments().len(), 2);
    Ok(())
}

/// #353: an earlier run's windows are not counted over a part the superseded
/// revision did not cover. The revision records those parts, and the windows of
/// the earlier run it still carries text from, which cover the whole source
/// here, are counted outside them only.
#[test]
fn an_earlier_window_is_not_counted_over_what_the_superseded_revision_left_untranscribed()
-> TestResult {
    let outcomes = [AsrChunkOutcome::Transcribed {
        audio: range(60 * SECOND, 70 * SECOND)?,
    }];
    let build = |unread: &[(u64, u64)]| -> Built<TranscriptRevision> {
        Ok(Splice {
            own_range: range(60 * SECOND, 70 * SECOND)?,
            own_outcomes: &outcomes,
            carried: &[(0, 5 * SECOND, 7 * SECOND), (3, 80 * SECOND, 82 * SECOND)],
            written: &[],
            carried_untranscribed: unread,
        }
        .build()??)
    };
    // Without the record, the earlier run's windows cover every instant.
    let overstated = SearchCoverage::of(&build(&[])?, None);
    assert!(overstated.is_complete());
    // With it, the part stays untranscribed, however far from the run's range.
    let revision = build(&[(10 * SECOND, 35 * SECOND)])?;
    let coverage = SearchCoverage::of(&revision, None);
    assert_eq!(coverage.untranscribed(), [range(10 * SECOND, 35 * SECOND)?]);
    assert_eq!(
        coverage.transcribed(),
        [range(0, 10 * SECOND)?, range(35 * SECOND, 100 * SECOND)?]
    );
    assert!(coverage.no_speech().is_empty());
    Ok(())
}

/// #353: what a revision records is validated whole, because a stored record is
/// rebuilt through the constructor: the parts are in order, merged, inside the
/// source and outside the replaced range, there are at most
/// `MAX_CARRIED_UNTRANSCRIBED` of them, and only a spliced revision has any.
#[test]
fn what_a_revision_records_as_untranscribed_is_validated() -> TestResult {
    let outcomes = [AsrChunkOutcome::Transcribed {
        audio: range(60 * SECOND, 70 * SECOND)?,
    }];
    let build = |unread: &[(u64, u64)]| -> Built<Spliced> {
        Splice {
            own_range: range(60 * SECOND, 70 * SECOND)?,
            own_outcomes: &outcomes,
            carried: &[(0, 5 * SECOND, 7 * SECOND)],
            written: &[],
            carried_untranscribed: unread,
        }
        .build()
    };
    let invalid = Err(crate::TranscriptRevisionError::InvalidSupersession);
    let ok = |unread: &[(u64, u64)]| build(unread).map(|built| built.is_ok());
    assert!(ok(&[
        (10 * SECOND, 20 * SECOND),
        (30 * SECOND, 40 * SECOND)
    ])?);
    // Overlapping the replaced range, or reaching into it from either side.
    for bad in [
        &[(65 * SECOND, 80 * SECOND)][..],
        &[(50 * SECOND, 61 * SECOND)],
        &[(50 * SECOND, 100 * SECOND)],
        // Not in order, overlapping, touching, or outside the source.
        &[(30 * SECOND, 40 * SECOND), (10 * SECOND, 20 * SECOND)],
        &[(10 * SECOND, 25 * SECOND), (20 * SECOND, 30 * SECOND)],
        &[(10 * SECOND, 20 * SECOND), (20 * SECOND, 30 * SECOND)],
        &[(90 * SECOND, 101 * SECOND)],
    ] {
        assert_eq!(build(bad)?, invalid, "{bad:?}");
    }
    // At most MAX_CARRIED_UNTRANSCRIBED: one more than that, each a microsecond
    // long and apart from the next, inside the source and in order, fails only
    // for its number.
    let most = u64::try_from(crate::MAX_CARRIED_UNTRANSCRIBED)?;
    let listed = |count: u64| -> Vec<(u64, u64)> {
        (0..count).map(|index| (index * 2, index * 2 + 1)).collect()
    };
    assert!(ok(&listed(most))?);
    assert_eq!(build(&listed(most + 1))?, invalid);
    // An import is not spliced: it records nothing.
    let mut parts = imported_parts()?;
    parts.carried_untranscribed = vec![range(SECOND, 2 * SECOND)?];
    assert_eq!(TranscriptRevision::new(parts), invalid);
    Ok(())
}

/// The parts of a one-cue import of a 100 s source.
fn imported_parts() -> Built<TranscriptRevisionParts> {
    Ok(TranscriptRevisionParts {
        id: TranscriptRevisionId::parse("trv_0123456789abcdef")?,
        number: NonZeroU32::MIN,
        source_id: SourceId::from_sha256(DIGEST)?,
        source_segment: source_segment(100 * SECOND)?,
        provenance: imported_provenance()?,
        supersedes: None,
        replaced_range: None,
        inherited: Vec::new(),
        carried_untranscribed: Vec::new(),
        language: None,
        segments: vec![cue_segment(1, SECOND, 2 * SECOND, "words")?],
        warnings: TranscriptWarnings::default(),
    })
}

/// #353: what a revision that supersedes this one over a range records is what
/// a search of this one lists as untranscribed outside the range.
#[test]
fn untranscribed_outside_is_what_a_search_lists_there() -> TestResult {
    let revision = spliced_over_an_unusable_window(&[(0, 5, 7)])??;
    assert_eq!(
        SearchCoverage::of(&revision, None).untranscribed(),
        [range(20 * SECOND, 50 * SECOND)?]
    );
    for (replaced, expected) in [
        // Far from the part: all of it.
        (
            range(60 * SECOND, 70 * SECOND)?,
            vec![range(20 * SECOND, 50 * SECOND)?],
        ),
        // Inside it: the two sides.
        (
            range(30 * SECOND, 40 * SECOND)?,
            vec![
                range(20 * SECOND, 30 * SECOND)?,
                range(40 * SECOND, 50 * SECOND)?,
            ],
        ),
        // Over an end of it, or all of it: what is left, or nothing.
        (
            range(10 * SECOND, 25 * SECOND)?,
            vec![range(25 * SECOND, 50 * SECOND)?],
        ),
        (range(0, 100 * SECOND)?, Vec::new()),
    ] {
        assert_eq!(revision.untranscribed_outside(replaced), expected);
    }
    Ok(())
}

/// #353: an earlier revision's coverage is never counted inside the windows a
/// run examined, an unusable one included. The run replaced what the earlier
/// revision said there, so counting it would report as transcribed a part that
/// now has no words (coverage may understate, never overstate).
#[test]
fn an_unusable_window_does_not_inherit_the_coverage_of_the_revision_it_replaced() -> TestResult {
    let revision = spliced_over_an_unusable_window(&[(0, 5, 7)])??;
    let coverage = SearchCoverage::of(&revision, None);
    assert_eq!(
        coverage.transcribed(),
        [range(0, 20 * SECOND)?, range(50 * SECOND, 100 * SECOND)?]
    );
    assert_eq!(coverage.untranscribed(), [range(20 * SECOND, 50 * SECOND)?]);
    // Asked only about the run's own range, the answer is the same: this is
    // what `transcript retranscribe` reports as its gaps.
    let own = SearchCoverage::of(&revision, Some(range(20 * SECOND, 50 * SECOND)?));
    assert_eq!(own.untranscribed(), [range(20 * SECOND, 50 * SECOND)?]);
    assert!(own.transcribed().is_empty());
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
