//! Chains of retranscriptions over a source (#353): what a search of the newest
//! revision may say about what the runs before it read, left unread and kept.
//!
//! The runs are the application's own (`transcribe_range` and
//! `build_asr_revision`) over fake audio and a scripted recognizer, so what is
//! checked is what the code does along a chain, not what a hand-built revision
//! says. The source is 70 s, which a whole-source run reads as the three chunks
//! 0-30 s, 25-55 s and 50-70 s.

use std::{collections::BTreeSet, future::Future, num::NonZeroU16, num::NonZeroU32};

use vsift_domain::{
    AsrChunkOutcome, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild,
    ChunkPlan, ChunkTime, CueSource, CueText, CueTiming, ImportedCue, LanguageTag, MediaTime,
    ParsedTranscript, PlannedChunk, ProviderChunkOutput, ProviderSegment, SearchCoverage,
    SessionId, Sha256Hex, SidecarIdentity, SourceId, SourceSegment, SourceSegmentId, TimeRange,
    TranscriptFormat, TranscriptOffset, TranscriptProvenance, TranscriptRevision,
    TranscriptRevisionParts, TranscriptWarnings, plan_chunks,
};

use super::{
    AsrCancellation, AsrRevisionRequest, RecognizerIdentity, RevisionSplice, SpeechAudioError,
    SpeechAudioSource, SpeechPcm, SpeechRecognitionError, SpeechRecognizer, TranscribeRangeRequest,
    build_asr_revision, transcribe_range,
};
use crate::{ImportedRevisionRequest, SuppliedTranscript, build_imported_revision};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SECOND: u64 = 1_000_000;
const SOURCE_SECONDS: u64 = 70;

/// What the recogniser does for one chunk of a run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Answer {
    /// One segment of two seconds, one second into the chunk.
    Words,
    /// A segment that starts after the chunk's audio ends: it cannot be placed,
    /// and the chunk is unusable.
    Unplaceable,
    /// The recogniser hears nothing: the chunk is read and holds no text.
    Nothing,
    /// The audio is silent: the recogniser is never asked.
    Silence,
}

struct Audio<'a>(&'a [Answer]);

impl SpeechAudioSource for Audio<'_> {
    fn speech_pcm(
        &self,
        chunk: &PlannedChunk,
    ) -> impl Future<Output = Result<SpeechPcm, SpeechAudioError>> + Send {
        let answer = usize::try_from(chunk.index())
            .ok()
            .and_then(|index| self.0.get(index).copied())
            .unwrap_or(Answer::Words);
        let level = if answer == Answer::Silence { 0 } else { 3_000 };
        std::future::ready(
            usize::try_from(chunk.window().duration_micros().min(4 * SECOND) / 1_000 * 16)
                .map_err(|_| SpeechAudioError::ResourceLimit)
                .map(|count| SpeechPcm {
                    actual_start: chunk.window().start(),
                    samples: vec![level; count],
                }),
        )
    }
}

struct Recogniser<'a>(&'a [Answer]);

impl SpeechRecognizer for Recogniser<'_> {
    fn identity(
        &self,
    ) -> impl Future<Output = Result<RecognizerIdentity, SpeechRecognitionError>> + Send {
        std::future::ready(identity().map_err(|_| SpeechRecognitionError::Io))
    }

    fn recognize(
        &self,
        chunk: &PlannedChunk,
        _pcm: &SpeechPcm,
    ) -> impl Future<Output = Result<ProviderChunkOutput, SpeechRecognitionError>> + Send {
        let answer = usize::try_from(chunk.index())
            .ok()
            .and_then(|index| self.0.get(index).copied())
            .unwrap_or(Answer::Words);
        std::future::ready(output(chunk, answer))
    }
}

fn output(
    chunk: &PlannedChunk,
    answer: Answer,
) -> Result<ProviderChunkOutput, SpeechRecognitionError> {
    let start_ms = match answer {
        Answer::Words => 1_000,
        Answer::Unplaceable => chunk.window().duration_micros() / 1_000 + 1_000,
        Answer::Nothing | Answer::Silence => {
            return Ok(ProviderChunkOutput {
                language: LanguageTag::parse("en").ok(),
                segments: Vec::new(),
            });
        }
    };
    let words = format!(
        "heard in the chunk at {}",
        chunk.window().start().as_micros()
    );
    Ok(ProviderChunkOutput {
        language: LanguageTag::parse("en").ok(),
        segments: vec![ProviderSegment {
            start: ChunkTime::from_millis(start_ms).ok_or(SpeechRecognitionError::Io)?,
            end: ChunkTime::from_millis(start_ms + 2_000).ok_or(SpeechRecognitionError::Io)?,
            text: Some(CueText::new(words.clone(), words).map_err(|_| SpeechRecognitionError::Io)?),
            tokens: Vec::new(),
        }],
    })
}

struct NotCancelled;

impl AsrCancellation for NotCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

fn identity() -> Built<RecognizerIdentity> {
    Ok(RecognizerIdentity {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        threads: NonZeroU16::new(4).ok_or("zero")?,
    })
}

fn range(from_seconds: u64, to_seconds: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from_seconds * SECOND),
        MediaTime::from_micros(to_seconds * SECOND),
    )?)
}

/// The whole seconds a list of ranges covers: every time in these tests is a
/// whole number of seconds, so a second is the unit an instant stands for.
fn seconds_of(ranges: impl IntoIterator<Item = TimeRange>) -> BTreeSet<u64> {
    ranges
        .into_iter()
        .flat_map(|range| range.start().as_micros() / SECOND..range.end().as_micros() / SECOND)
        .collect()
}

/// A source, its revisions so far, and what an independent count of the runs
/// says was read, second by second.
#[derive(Clone)]
struct Chain {
    session: SessionId,
    source_id: SourceId,
    source: SourceSegment,
    revisions: Vec<TranscriptRevision>,
    /// The seconds the newest revision holds a read of: where a run it still
    /// holds read the audio, or a supplied transcript is, or it holds text.
    read: BTreeSet<u64>,
}

impl Chain {
    fn parts() -> Built<(SessionId, SourceId, SourceSegment)> {
        Ok((
            SessionId::parse("ses_0123456789abcdef")?,
            SourceId::from_sha256(DIGEST)?,
            SourceSegment::whole_file(
                SourceSegmentId::parse("sgm_0123456789abcdef")?,
                MediaTime::from_micros(SOURCE_SECONDS * SECOND),
            )?,
        ))
    }

    /// A chain that starts with a whole-source run answering `answers`.
    async fn local(answers: &[Answer]) -> Built<Self> {
        let (session, source_id, source) = Self::parts()?;
        let transcription = transcribe(&source, source.range(), answers)
            .await?
            .ok_or("the first run failed")?;
        let revision = build_asr_revision(AsrRevisionRequest {
            session_id: &session,
            source_id: &source_id,
            source_segment: &source,
            number: NonZeroU32::MIN,
            transcription,
            splice: None,
        })?;
        let read = read_by_the_run_of(&revision);
        Ok(Self {
            session,
            source_id,
            source,
            revisions: vec![revision],
            read,
        })
    }

    /// A chain that starts with a run over `from` to `to` seconds of a source
    /// with no earlier revision, which covers only its range.
    async fn bounded(from: u64, to: u64, answers: &[Answer]) -> Built<Self> {
        let (session, source_id, source) = Self::parts()?;
        let transcription = transcribe(&source, range(from, to)?, answers)
            .await?
            .ok_or("the first run failed")?;
        let revision = build_asr_revision(AsrRevisionRequest {
            session_id: &session,
            source_id: &source_id,
            source_segment: &source,
            number: NonZeroU32::MIN,
            transcription,
            splice: None,
        })?;
        let read = read_by_the_run_of(&revision);
        Ok(Self {
            session,
            source_id,
            source,
            revisions: vec![revision],
            read,
        })
    }

    /// Whether a run of the chain read a chunk and could not use its answer.
    fn holds_an_unusable_chunk(&self) -> bool {
        self.revisions.iter().any(|revision| {
            matches!(revision.provenance(), TranscriptProvenance::LocalAsr(run)
            if run.chunks().iter().any(|record| {
                matches!(record.outcome(), AsrChunkOutcome::Unusable { .. })
            }))
        })
    }

    /// The newest revision as it was written before the record of untranscribed
    /// parts was narrowed: with all that its superseded revision left
    /// untranscribed outside the replaced range, whether or not anything
    /// carried would be counted over it. `None` for a first revision.
    fn newest_with_the_whole_record(&self) -> Built<Option<TranscriptRevision>> {
        let [.., base, newest] = self.revisions.as_slice() else {
            return Ok(None);
        };
        let Some(replaced) = newest.replaced_range() else {
            return Ok(None);
        };
        let mut whole = Vec::new();
        for gap in SearchCoverage::of(base, None).untranscribed() {
            if gap.start() < replaced.start() {
                whole.push(TimeRange::new(
                    gap.start(),
                    gap.end().min(replaced.start()),
                )?);
            }
            if gap.end() > replaced.end() {
                whole.push(TimeRange::new(gap.start().max(replaced.end()), gap.end())?);
            }
        }
        Ok(Some(TranscriptRevision::new(TranscriptRevisionParts {
            id: newest.id().clone(),
            number: NonZeroU32::new(newest.number()).ok_or("zero")?,
            source_id: newest.source_id().clone(),
            source_segment: newest.source_segment().clone(),
            provenance: newest.provenance().clone(),
            supersedes: newest.supersedes().cloned(),
            replaced_range: newest.replaced_range(),
            inherited: newest.inherited().to_vec(),
            carried_untranscribed: whole,
            language: newest.language().cloned(),
            segments: newest.segments().to_vec(),
            warnings: newest.warnings().clone(),
        })?))
    }

    /// A chain that starts with an imported transcript whose cues are `cues`,
    /// each `(start s, end s)`.
    fn imported(cues: &[(u64, u64)]) -> Built<Self> {
        let micros: Vec<(u64, u64)> = cues
            .iter()
            .map(|&(start, end)| (start * SECOND, end * SECOND))
            .collect();
        Self::imported_micros(&micros)
    }

    /// Like [`Chain::imported`], with the cues in microseconds.
    fn imported_micros(cues: &[(u64, u64)]) -> Built<Self> {
        let (session, source_id, _) = Self::parts()?;
        let mut parsed = Vec::new();
        for (ordinal, &(start, end)) in (1_u32..).zip(cues) {
            let words = format!("imported cue {ordinal}");
            parsed.push(ImportedCue {
                source: CueSource::new(
                    NonZeroU32::new(ordinal).ok_or("zero")?,
                    NonZeroU32::new(ordinal * 4).ok_or("zero")?,
                ),
                timing: CueTiming::new(start, end)?,
                text: CueText::new(words.clone(), words)?,
                speaker: None,
            });
        }
        let supplied = SuppliedTranscript {
            transcript: ParsedTranscript::new(
                TranscriptFormat::Srt,
                LanguageTag::parse("en").ok(),
                parsed,
                TranscriptWarnings::default(),
            )?,
            sidecar: SidecarIdentity::new(DIGEST, 100)?,
        };
        let revision = build_imported_revision(ImportedRevisionRequest {
            session_id: &session,
            source_id: &source_id,
            source_duration: MediaTime::from_micros(SOURCE_SECONDS * SECOND),
            supplied: &supplied,
            offset: TranscriptOffset::ZERO,
            number: NonZeroU32::MIN,
        })?;
        Ok(Self {
            session,
            source_id,
            // The import names the source segment it covers, which later runs
            // must name too.
            source: revision.source_segment().clone(),
            revisions: vec![revision],
            read: (0..SOURCE_SECONDS).collect(),
        })
    }

    fn newest(&self) -> Built<&TranscriptRevision> {
        Ok(self.revisions.last().ok_or("a chain has a revision")?)
    }

    /// Retranscribes `from` to `to` seconds, widened to whole segments, with the
    /// chunks answering `answers`. `false` when the run failed and so
    /// committed nothing.
    async fn step(&mut self, from: u64, to: u64, answers: &[Answer]) -> Built<bool> {
        let base = self.newest()?;
        let replaced = base.snap_to_segments(range(from, to)?);
        let Some(transcription) = transcribe(&self.source, replaced, answers).await? else {
            return Ok(false);
        };
        let number = NonZeroU32::new(base.number().saturating_add(1)).ok_or("zero")?;
        let revision = build_asr_revision(AsrRevisionRequest {
            session_id: &self.session,
            source_id: &self.source_id,
            source_segment: &self.source,
            number,
            transcription,
            splice: Some(RevisionSplice {
                base,
                replaced_range: replaced,
            }),
        })?;
        // What the count says is read now: what was read outside the replaced
        // range, what this run read, and the text it kept where it could not.
        let mut read: BTreeSet<u64> = self
            .read
            .difference(&seconds_of([replaced]))
            .copied()
            .collect();
        read.extend(read_by_the_run_of(&revision));
        read.extend(seconds_of(
            revision
                .segments()
                .iter()
                .filter(|segment| segment.carried_from().is_some())
                .filter(|segment| segment.intersects(replaced))
                .map(vsift_domain::TranscriptSegment::range),
        ));
        self.read = read;
        self.revisions.push(revision);
        Ok(true)
    }

    /// What a search of the newest revision says, as seconds.
    fn coverage(&self) -> Built<(BTreeSet<u64>, BTreeSet<u64>)> {
        let coverage = SearchCoverage::of(self.newest()?, None);
        Ok((
            seconds_of(coverage.transcribed().iter().copied()),
            seconds_of(coverage.untranscribed().iter().copied()),
        ))
    }

    /// Whether the newest revision holds text of an earlier one inside the
    /// range its run replaced, which it keeps only where its run could not read.
    fn kept_text(&self) -> Built<bool> {
        let newest = self.newest()?;
        Ok(newest.replaced_range().is_some_and(|replaced| {
            newest
                .segments()
                .iter()
                .any(|segment| segment.carried_from().is_some() && segment.intersects(replaced))
        }))
    }

    /// The seconds the newest revision has text in.
    fn text(&self) -> Built<BTreeSet<u64>> {
        Ok(seconds_of(
            self.newest()?
                .segments()
                .iter()
                .map(vsift_domain::TranscriptSegment::range),
        ))
    }
}

/// The seconds of the windows of `revision`'s own run that were read: the
/// chunks transcribed or found quiet, not the unusable ones.
fn read_by_the_run_of(revision: &TranscriptRevision) -> BTreeSet<u64> {
    let TranscriptProvenance::LocalAsr(run) = revision.provenance() else {
        return BTreeSet::new();
    };
    seconds_of(
        run.chunks()
            .iter()
            .filter(|record| !matches!(record.outcome(), AsrChunkOutcome::Unusable { .. }))
            .map(|record| record.chunk().window()),
    )
}

/// A run over `requested` with the chunks answering `answers`; `None` when it
/// failed.
async fn transcribe(
    source: &SourceSegment,
    requested: TimeRange,
    answers: &[Answer],
) -> Built<Option<super::AsrTranscription>> {
    let expected = identity()?;
    Ok(transcribe_range(
        TranscribeRangeRequest {
            source_segment: source,
            range: requested,
            plan: ChunkPlan::R0,
            audio_stream: 1,
            expected: &expected,
        },
        &Audio(answers),
        &Recogniser(answers),
        &NotCancelled,
    )
    .await
    .ok())
}

fn seconds(span: std::ops::Range<u64>) -> BTreeSet<u64> {
    span.collect()
}

fn union(parts: &[BTreeSet<u64>]) -> BTreeSet<u64> {
    parts.iter().flatten().copied().collect()
}

/// The reviewer's chain over a whole-source run (revision 1: segments in each
/// of the three chunks, at 1, 26 and 51 s): revision 2 re-reads 10-65 s and its
/// first chunk (10-40 s) is unusable, so 10-35 s is unread and the segment at
/// 26 s is kept; revision 3 re-reads 0-8 s, far from the unread part.
///
/// Before, the search of revision 3 said the whole source was transcribed,
/// because revision 1's windows, which revision 3 still carries text from, were
/// counted over a part that revision 2 had left untranscribed and that no
/// run revision 3 holds says anything about.
#[tokio::test]
async fn a_later_run_far_away_does_not_make_an_unread_part_transcribed() -> TestResult {
    let mut chain = Chain::local(&[Answer::Words; 3]).await?;
    assert!(
        chain
            .step(10, 65, &[Answer::Unplaceable, Answer::Words])
            .await?
    );
    // Revision 2: 10-35 s was not read; the kept segment (26-28 s) is text.
    let (transcribed, untranscribed) = chain.coverage()?;
    assert_eq!(untranscribed, union(&[seconds(10..26), seconds(28..35)]));
    assert_eq!(
        transcribed,
        union(&[seconds(0..10), seconds(26..28), seconds(35..70)])
    );

    assert!(chain.step(0, 8, &[Answer::Words]).await?);
    let (transcribed, untranscribed) = chain.coverage()?;
    assert_eq!(untranscribed, union(&[seconds(10..26), seconds(28..35)]));
    assert_eq!(
        transcribed,
        union(&[seconds(0..10), seconds(26..28), seconds(35..70)])
    );
    // The record that says so is the revision's own, for the next run to start from.
    let recorded: Vec<(u64, u64)> = chain
        .newest()?
        .carried_untranscribed()
        .iter()
        .map(|range| {
            (
                range.start().as_micros() / SECOND,
                range.end().as_micros() / SECOND,
            )
        })
        .collect();
    assert_eq!(recorded, [(10, 26), (28, 35)]);
    Ok(())
}

/// The same chain over an imported transcript: the file covers the whole source,
/// but not what a run replaced and could not read, nor after another run.
#[tokio::test]
async fn an_imported_file_is_not_counted_over_what_a_run_replaced_and_could_not_read() -> TestResult
{
    let mut chain = Chain::imported(&[(5, 7), (12, 14), (40, 42), (60, 62)])?;
    assert!(
        chain
            .step(10, 65, &[Answer::Unplaceable, Answer::Words])
            .await?
    );
    let (_, untranscribed) = chain.coverage()?;
    assert_eq!(untranscribed, union(&[seconds(10..12), seconds(14..35)]));
    assert!(chain.step(0, 8, &[Answer::Words]).await?);
    let (transcribed, untranscribed) = chain.coverage()?;
    assert_eq!(untranscribed, union(&[seconds(10..12), seconds(14..35)]));
    assert_eq!(
        transcribed,
        union(&[seconds(0..10), seconds(12..14), seconds(35..70)])
    );
    // The text that was kept is the file's cue, and it is still there.
    assert!(chain.text()?.is_superset(&seconds(12..14)));
    Ok(())
}

/// Three retranscriptions that each leave a different part unread: the parts
/// stay untranscribed through every run after them, however far from them.
#[tokio::test]
async fn parts_left_unread_by_different_runs_all_stay_untranscribed() -> TestResult {
    let mut chain = Chain::imported(&[(5, 7), (12, 14), (34, 36), (56, 58)])?;
    // 10-65 s with its first chunk unusable: 10-35 s unread, 12-14 s kept.
    assert!(
        chain
            .step(10, 65, &[Answer::Unplaceable, Answer::Words])
            .await?
    );
    // 30-70 s with its last chunk unusable (55-70 s): 60-70 s unread, nothing
    // of the earlier text lies there.
    assert!(
        chain
            .step(30, 70, &[Answer::Words, Answer::Unplaceable])
            .await?
    );
    let (_, untranscribed) = chain.coverage()?;
    assert!(untranscribed.is_superset(&union(&[
        seconds(10..12),
        seconds(14..30),
        seconds(60..70)
    ])));
    // A third run far from both.
    assert!(chain.step(0, 8, &[Answer::Words]).await?);
    let (transcribed, untranscribed) = chain.coverage()?;
    assert_eq!(
        untranscribed,
        union(&[seconds(10..12), seconds(14..30), seconds(60..70)])
    );
    assert!(transcribed.is_disjoint(&untranscribed));
    assert_eq!(chain.newest()?.number(), 4);
    Ok(())
}

/// A later run that reads what an earlier one could not makes it transcribed,
/// and records nothing untranscribed any more.
#[tokio::test]
async fn a_later_run_that_reads_the_unread_part_covers_it() -> TestResult {
    let mut chain = Chain::local(&[Answer::Words; 3]).await?;
    assert!(
        chain
            .step(10, 65, &[Answer::Unplaceable, Answer::Words])
            .await?
    );
    assert!(!chain.coverage()?.1.is_empty());
    // 5-50 s, read in full: its window covers 10-35 s.
    assert!(chain.step(5, 50, &[Answer::Words, Answer::Words]).await?);
    let (transcribed, untranscribed) = chain.coverage()?;
    assert!(untranscribed.is_empty(), "{untranscribed:?}");
    assert_eq!(transcribed, seconds(0..70));
    assert!(chain.newest()?.carried_untranscribed().is_empty());
    Ok(())
}

/// A chain in which no run left anything unread is written and presented as
/// before: nothing is recorded, and the source is covered after every run (the
/// runs keep the first run's, or the file's, text at 1-3 s or 5-7 s, so that
/// what they cover is still recorded in the revision).
#[tokio::test]
async fn a_chain_without_an_unread_part_records_nothing_and_covers_the_source() -> TestResult {
    for base in [
        Chain::local(&[Answer::Words; 3]).await?,
        Chain::imported(&[(5, 7), (12, 14), (40, 42), (60, 62)])?,
    ] {
        let mut chain = base;
        for (from, to, answers) in [
            (10, 65, vec![Answer::Words, Answer::Nothing]),
            (30, 70, vec![Answer::Silence, Answer::Words]),
            (20, 50, vec![Answer::Words, Answer::Words]),
        ] {
            assert!(chain.step(from, to, &answers).await?);
            let (transcribed, untranscribed) = chain.coverage()?;
            assert!(untranscribed.is_empty(), "{from}-{to}: {untranscribed:?}");
            assert_eq!(transcribed, seconds(0..70));
            assert!(chain.newest()?.carried_untranscribed().is_empty());
        }
    }
    Ok(())
}

/// Over every chain of up to three retranscriptions from a small grid of
/// ranges and chunk answers, from three starting points, a search of the
/// newest revision never claims a second that it holds no read of: text it
/// holds is covered, and what is covered was read by a run it still holds (or
/// is a supplied file's), by an independent count along the chain. It may say
/// less, which it does where a run that read a part is no longer held.
#[tokio::test]
async fn no_chain_of_runs_makes_search_claim_what_no_run_read() -> TestResult {
    let ranges = [(0, 8), (10, 65), (30, 70), (5, 50)];
    let patterns: [fn(usize) -> Vec<Answer>; 3] = [
        |_| Vec::new(),
        |_| vec![Answer::Unplaceable],
        |chunks| {
            let mut answers = vec![Answer::Words; chunks];
            if let Some(last) = answers.last_mut() {
                *last = Answer::Unplaceable;
            }
            answers
        },
    ];
    let starts = [
        Chain::local(&[Answer::Words; 3]).await?,
        Chain::local(&[Answer::Unplaceable, Answer::Words, Answer::Words]).await?,
        Chain::imported(&[(5, 7), (12, 14), (20, 36), (34, 36), (56, 58)])?,
        // Chains that began with a range, of which most of the source was
        // never examined.
        Chain::bounded(0, 20, &[]).await?,
        Chain::bounded(30, 60, &[]).await?,
    ];
    let mut checked = 0_u32;
    let mut with_unread_parts = 0_u32;
    let mut with_kept_text = 0_u32;
    let mut stack: Vec<Chain> = starts.to_vec();
    while let Some(chain) = stack.pop() {
        let (transcribed, untranscribed) = chain.coverage()?;
        // Only what was read is covered, and covered and not are the source.
        assert!(
            transcribed.is_subset(&chain.read),
            "revision {}: claims {:?} that no run read",
            chain.newest()?.number(),
            transcribed.difference(&chain.read).collect::<Vec<_>>()
        );
        assert!(transcribed.is_disjoint(&untranscribed));
        assert_eq!(
            union(&[transcribed.clone(), untranscribed.clone()]),
            seconds(0..70)
        );
        // Text is covered.
        assert!(chain.text()?.is_subset(&transcribed));
        // Nothing is recorded as untranscribed in a chain in which no run could
        // not use an answer: the record exists for the parts such a run left
        // under a window or file that a later revision still carries.
        if !chain.holds_an_unusable_chunk() {
            assert!(
                chain.newest()?.carried_untranscribed().is_empty(),
                "revision {} records {:?} in a chain with no unusable chunk",
                chain.newest()?.number(),
                chain.newest()?.carried_untranscribed()
            );
        }
        // And what a search says is the same as if everything that the
        // superseded revision left untranscribed had been recorded.
        if let Some(whole) = chain.newest_with_the_whole_record()? {
            assert_eq!(
                SearchCoverage::of(chain.newest()?, None),
                SearchCoverage::of(&whole, None),
                "revision {}",
                whole.number()
            );
        }
        checked += 1;
        with_unread_parts += u32::from(!untranscribed.is_empty());
        with_kept_text += u32::from(chain.kept_text()?);
        if chain.revisions.len() >= 4 {
            continue;
        }
        for (from, to) in ranges {
            let replaced = chain.newest()?.snap_to_segments(range(from, to)?);
            let chunks = plan_chunks(chain.source.id(), replaced, ChunkPlan::R0)?.len();
            for pattern in patterns {
                let mut next = chain.clone();
                if next.step(from, to, &pattern(chunks)).await? {
                    stack.push(next);
                }
            }
        }
    }
    // The grid is not empty of what it is for.
    assert!(checked > 1_000, "{checked}");
    assert!(with_unread_parts > 100, "{with_unread_parts}");
    assert!(with_kept_text > 10, "{with_kept_text}");
    Ok(())
}

/// The newest revision's segments as `(start, end, text, carried)` in
/// microseconds.
fn segments_of(chain: &Chain) -> Built<Vec<(u64, u64, String, bool)>> {
    Ok(chain
        .newest()?
        .segments()
        .iter()
        .map(|segment| {
            (
                segment.range().start().as_micros(),
                segment.range().end().as_micros(),
                segment.text().text().to_owned(),
                segment.carried_from().is_some(),
            )
        })
        .collect())
}

/// The reviewer's straddlers: an imported transcript with the cues 12-14 s,
/// 16-35 s and 20 s to 35 s and a microsecond, over 10-65 s whose first chunk
/// (10-40 s) is unusable, so 10-35 s is unread. The third cue crosses the gap's
/// edge by a microsecond. Before, it was dropped, nearly 15 s of it in audio
/// that nobody read again; now it is kept whole with the other two, because the
/// run's own text (36-38 s) does not overlap it.
#[tokio::test]
async fn a_cue_that_crosses_the_edge_of_an_unread_part_by_a_microsecond_is_kept() -> TestResult {
    let mut chain = Chain::imported_micros(&[
        (12 * SECOND, 14 * SECOND),
        (16 * SECOND, 35 * SECOND),
        (20 * SECOND, 35 * SECOND + 1),
    ])?;
    assert!(
        chain
            .step(10, 65, &[Answer::Unplaceable, Answer::Words])
            .await?
    );
    let kept = [
        (12 * SECOND, 14 * SECOND, "imported cue 1".to_owned(), true),
        (16 * SECOND, 35 * SECOND, "imported cue 2".to_owned(), true),
        (
            20 * SECOND,
            35 * SECOND + 1,
            "imported cue 3".to_owned(),
            true,
        ),
        (
            36 * SECOND,
            38 * SECOND,
            "heard in the chunk at 35000000".to_owned(),
            false,
        ),
    ];
    assert_eq!(segments_of(&chain)?, kept);
    // What is not text and was not read is what a search lists: 10-12 s and
    // 14-16 s; the kept cues cover the rest of the gap.
    let (_, untranscribed) = chain.coverage()?;
    assert_eq!(untranscribed, union(&[seconds(10..12), seconds(14..16)]));
    // A later run far from it changes none of that.
    assert!(chain.step(0, 8, &[Answer::Words]).await?);
    let (_, untranscribed) = chain.coverage()?;
    assert_eq!(untranscribed, union(&[seconds(10..12), seconds(14..16)]));
    assert_eq!(segments_of(&chain)?.len(), 5);
    Ok(())
}

/// The rule for a cue that crosses the edge of an unread part and the run read
/// the audio past it: the run's own text replaces the cue it overlaps, and a cue
/// the run's text only touches, or one over audio the run read and found empty,
/// is kept whole.
#[tokio::test]
async fn a_cue_over_the_runs_own_text_is_replaced_and_one_beside_it_or_over_nothing_is_kept()
-> TestResult {
    let cues = [(30 * SECOND, 36 * SECOND), (34 * SECOND, 42 * SECOND)];
    // Chunk 1 (35-65 s) writes 36-38 s: the second cue (34-42 s) overlaps it
    // and is replaced; the first (30-36 s) ends where the text starts.
    let mut written = Chain::imported_micros(&cues)?;
    assert!(
        written
            .step(10, 65, &[Answer::Unplaceable, Answer::Words])
            .await?
    );
    assert_eq!(
        segments_of(&written)?
            .iter()
            .map(|(start, _, _, carried)| (*start / SECOND, *carried))
            .collect::<Vec<_>>(),
        [(30, true), (36, false)]
    );
    // The same chunk read and heard nothing: both cues are kept whole.
    let mut empty = Chain::imported_micros(&cues)?;
    assert!(
        empty
            .step(10, 65, &[Answer::Unplaceable, Answer::Nothing])
            .await?
    );
    assert_eq!(
        segments_of(&empty)?
            .iter()
            .map(|(start, end, _, carried)| (*start / SECOND, *end / SECOND, *carried))
            .collect::<Vec<_>>(),
        [(30, 36, true), (34, 42, true)]
    );
    // (A chunk read as silent is the same, which the domain's rule test checks:
    // beside an unusable chunk it cannot be run here, because a silent chunk is
    // not an answer, and one unusable answer of one fails the run.)
    Ok(())
}

/// The reviewer's chain: a retranscription over a range, then one over another
/// range apart from it, every chunk read. Nothing of the source between or
/// beyond was ever examined, and no earlier window claims it, so the revision
/// records nothing: it is written as it always was, and a release before 0.2.1
/// still reads it.
#[tokio::test]
async fn ranges_apart_record_nothing_and_cover_what_was_read() -> TestResult {
    let mut chain = Chain::bounded(0, 10, &[]).await?;
    assert!(chain.step(20, 30, &[]).await?);
    assert!(chain.newest()?.carried_untranscribed().is_empty());
    let (transcribed, untranscribed) = chain.coverage()?;
    assert_eq!(transcribed, union(&[seconds(0..10), seconds(20..30)]));
    assert_eq!(untranscribed, union(&[seconds(10..20), seconds(30..70)]));
    assert!(chain.step(40, 50, &[Answer::Nothing]).await?);
    assert!(chain.newest()?.carried_untranscribed().is_empty());
    // Over an imported file, with every run read, there is nothing unread to record.
    let mut file = Chain::imported(&[(5, 7), (40, 42)])?;
    assert!(file.step(10, 20, &[]).await?);
    assert!(file.step(30, 38, &[Answer::Nothing]).await?);
    assert!(file.newest()?.carried_untranscribed().is_empty());
    assert!(file.coverage()?.1.is_empty());
    Ok(())
}

/// A part some run could not use is recorded under what a later revision
/// carries, and only then: the same chain with the first run's answer unusable
/// records the part that the file would otherwise be counted over.
#[tokio::test]
async fn an_unread_part_under_a_carried_file_is_the_one_thing_that_is_recorded() -> TestResult {
    let mut chain = Chain::imported(&[(5, 7), (12, 14), (40, 42)])?;
    assert!(
        chain
            .step(10, 65, &[Answer::Unplaceable, Answer::Words])
            .await?
    );
    assert!(chain.holds_an_unusable_chunk());
    // The file is carried (the cue at 5 s), and claims the whole source.
    assert!(chain.step(0, 3, &[Answer::Words]).await?);
    assert!(!chain.newest()?.carried_untranscribed().is_empty());
    Ok(())
}
