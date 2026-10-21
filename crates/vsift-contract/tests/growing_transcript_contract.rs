//! Reads of a growing transcript through the v1 contract (ADR 0033): an open
//! revision's `transcript get` and `search` are `partial`, list what is
//! pending, name the earlier revision that still holds text there, and carry a
//! `since` token; a stopped revision is presented with its state; a sealed one
//! exactly as before. Every answer validates against the published schemas,
//! and the frozen examples are what the code gives.

use std::{fs, future::Future, io, num::NonZeroU16, num::NonZeroU32, path::PathBuf, sync::Mutex};

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use vsift_application::{
    AsrCancellation, AsrFailure, AsrRevisionRequest, AsrTranscription, CheckpointScope,
    NoCheckpoints, NoProgress, RecognizerIdentity, RevisionSplice, RunObserver, RunSoFar,
    SpeechAudioError, SpeechAudioSource, SpeechPcm, SpeechRecognitionError, SpeechRecognizer,
    TranscribeRangeRequest, TranscriptPageRequest, build_asr_revision, build_open_asr_revision,
    build_stopped_asr_revision, page_since, page_transcript, since_token,
    transcribe_range_checkpointed, whole_file_source_segment,
};
use vsift_contract::{
    EvidenceStream, LifecycleResponse, PendingCover, ReadGrowth, ResultView, SearchEvidenceStream,
    SearchPresentation, SinceRead, TRANSCRIPTION_PENDING_REASON, TRANSCRIPTION_PENDING_WARNING,
    TranscriptEvidenceStream, TranscriptPresentation, UNTRANSCRIBED_READ_WARNING,
    search_response_with_view, transcript_get_response,
};
use vsift_domain::{
    AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider, AsrProviderBuild, ChunkPlan,
    ChunkTime, CueText, LanguageTag, MAX_EARLY_AUDIO_MICROS, MediaTime, PageLimit, PlannedChunk,
    ProviderChunkOutput, ProviderSegment, RecognitionKey, RevisionState, SearchCoverage,
    SearchQuery, SessionId, Sha256Hex, SourceId, SourceSegment, TimeRange, TranscriptRevision,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Built<T> = Result<T, Box<dyn std::error::Error>>;

const SESSION: &str = "ses_0123456789abcdef0123456789abcdef";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const EXPIRES_AT: &str = "2026-09-25T00:00:00Z";
const CURSOR_EXPIRES_US: u64 = 1_790_294_400_000_000;
const NOW_US: u64 = 1_790_208_000_000_000;
const SECOND: u64 = 1_000_000;
const SCHEMA_BASE: &str = "https://vsift.dev/schemas/v1/";

fn repository(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn load(relative_path: &str) -> Built<Value> {
    Ok(serde_json::from_str(&fs::read_to_string(
        repository("schemas/v1").join(relative_path),
    )?)?)
}

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
            repository("schemas/v1").join(name),
        )?)?)
    }
}

fn validate(schema_path: &str, instance: &Value) -> TestResult {
    let schema = load(schema_path)?;
    jsonschema::options()
        .with_retriever(PublishedSchemas)
        .build(&schema)?
        .validate(instance)
        .map_err(|error| io::Error::other(format!("{schema_path}: {error}")))?;
    Ok(())
}

/// Compares `value` with the frozen example `name` (the parsed JSON).
fn assert_example(name: &str, value: &Value) -> TestResult {
    assert_eq!(*value, load(&format!("examples/{name}"))?, "{name}");
    Ok(())
}

fn identity() -> Built<RecognizerIdentity> {
    Ok(RecognizerIdentity {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, Sha256Hex::parse(DIGEST)?),
        model: AsrModel::new(AsrModelProfile::Base, Sha256Hex::parse(DIGEST)?),
        decoding: AsrDecodingProfile::R0V1,
        threads: NonZeroU16::new(4).ok_or("zero")?,
    })
}

/// Loud audio over every chunk's window.
struct Audio;

impl SpeechAudioSource for Audio {
    fn speech_pcm(
        &self,
        chunk: &PlannedChunk,
    ) -> impl Future<Output = Result<SpeechPcm, SpeechAudioError>> + Send {
        std::future::ready(
            usize::try_from(chunk.window().duration_micros() / 1_000 * 16)
                .map_err(|_| SpeechAudioError::ResourceLimit)
                .map(|count| SpeechPcm {
                    actual_start: chunk.window().start(),
                    samples: vec![3_000; count],
                }),
        )
    }
}

/// Each chunk says two sentences of its own, at 2 and 12 s into it; the
/// second run (`again`) says them in other words.
struct Recogniser {
    again: bool,
}

impl SpeechRecognizer for Recogniser {
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
        let index = chunk.index();
        let said = if self.again { "said again" } else { "said" };
        let segment = |start_ms: u64, end_ms: u64, words: String| {
            Ok::<_, SpeechRecognitionError>(ProviderSegment {
                start: ChunkTime::from_millis(start_ms).ok_or(SpeechRecognitionError::Io)?,
                end: ChunkTime::from_millis(end_ms).ok_or(SpeechRecognitionError::Io)?,
                text: Some(
                    CueText::new(words.clone(), words).map_err(|_| SpeechRecognitionError::Io)?,
                ),
                tokens: Vec::new(),
            })
        };
        std::future::ready((|| {
            Ok(ProviderChunkOutput {
                language: LanguageTag::parse("en").ok(),
                segments: vec![
                    segment(2_000, 5_000, format!("chunk {index} {said} first"))?,
                    segment(12_000, 15_000, format!("chunk {index} {said} second"))?,
                ],
            })
        })())
    }
}

struct NotCancelled;

impl AsrCancellation for NotCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Records what an open revision would hold after every chunk but the last.
struct Cuts(Mutex<Vec<Result<(AsrTranscription, MediaTime), AsrFailure>>>);

impl RunObserver for Cuts {
    fn chunk_finished(&self, run: &RunSoFar<'_>) {
        if run.next().is_some()
            && let Ok(mut cuts) = self.0.lock()
        {
            cuts.push(run.open(MAX_EARLY_AUDIO_MICROS));
        }
    }
}

/// A whole-source run over the 120 s stand-in, superseding `base` when given:
/// the sealed revision and the open revision of every cut.
async fn run(
    source: &SourceSegment,
    number: u32,
    base: Option<&TranscriptRevision>,
    again: bool,
) -> Built<(TranscriptRevision, Vec<TranscriptRevision>)> {
    let expected = identity()?;
    let cuts = Cuts(Mutex::new(Vec::new()));
    let key = RecognitionKey::new(Sha256Hex::parse(DIGEST)?);
    let (transcription, _) = transcribe_range_checkpointed(
        TranscribeRangeRequest {
            source_segment: source,
            range: source.range(),
            plan: ChunkPlan::R0,
            audio_stream: 1,
            expected: &expected,
        },
        CheckpointScope {
            checkpoints: &NoCheckpoints,
            key: &key,
            progress: &NoProgress,
            observer: &cuts,
            stopwatch: &vsift_application::StoppedStopwatch,
        },
        &Audio,
        &Recogniser { again },
        &NotCancelled,
    )
    .await?;
    let session = SessionId::parse(SESSION)?;
    let source_id = SourceId::from_sha256(DIGEST)?;
    let request = |transcription| AsrRevisionRequest {
        session_id: &session,
        source_id: &source_id,
        source_segment: source,
        number: NonZeroU32::new(number).unwrap_or(NonZeroU32::MIN),
        transcription,
        splice: base.map(|base| RevisionSplice {
            base,
            replaced_range: source.range(),
        }),
    };
    let sealed = build_asr_revision(request(transcription))?;
    let mut open = Vec::new();
    for cut in cuts.0.into_inner().map_err(|_| "poisoned")? {
        let (transcription, settled_until) = cut?;
        open.push(build_open_asr_revision(
            request(transcription),
            settled_until,
        )?);
    }
    Ok((sealed, open))
}

fn source() -> Built<SourceSegment> {
    Ok(whole_file_source_segment(
        &SourceId::from_sha256(DIGEST)?,
        MediaTime::from_micros(120 * SECOND),
    )?)
}

fn window(from: u64, to: u64) -> Built<TimeRange> {
    Ok(TimeRange::new(
        MediaTime::from_micros(from * SECOND),
        MediaTime::from_micros(to * SECOND),
    )?)
}

/// `transcript get` of `range` of `revision` in `view`, with `growth`.
fn get(
    revision: &TranscriptRevision,
    range: TimeRange,
    view: ResultView,
    growth: ReadGrowth<'_>,
) -> Built<Value> {
    let session = SessionId::parse(SESSION)?;
    let page = page_transcript(
        &session,
        revision,
        &TranscriptPageRequest {
            range,
            limit: PageLimit::new(100)?,
            cursor: None,
        },
        CURSOR_EXPIRES_US,
        NOW_US,
    )?;
    let segments: Vec<_> = page.segments.into_iter().cloned().collect();
    Ok(serde_json::to_value(transcript_get_response(
        &TranscriptPresentation {
            session_id: &session,
            revision,
            range,
            segments: &segments,
            next_cursor: None,
            coverage: &page.coverage,
            growth,
        },
        LifecycleResponse::ephemeral(EXPIRES_AT.to_owned()),
        view,
    )?)?)
}

/// The growth an open read of `revision` reports: its since token, and
/// `cover` over what is pending.
fn open_growth<'a>(token: &'a str, cover: Option<PendingCover<'a>>) -> ReadGrowth<'a> {
    ReadGrowth {
        pending_covered_by: cover,
        since: Some(SinceRead {
            token,
            state: RevisionState::Open,
            more: false,
        }),
    }
}

/// An open revision's read is partial: what is not published yet is
/// untranscribed and pending, under its own reason and warning, the revision
/// says it is open and until when, and the answer carries a since token. In
/// every view and as a stream it validates; the full view is the frozen
/// example.
#[tokio::test]
async fn an_open_revision_is_read_as_partial_with_what_is_pending() -> TestResult {
    let source = source()?;
    let (_, open) = run(&source, 1, None, false).await?;
    let open = open.get(2).ok_or("no third cut")?;
    assert_eq!(open.state(), RevisionState::Open);
    let session = SessionId::parse(SESSION)?;
    let token = since_token(&session, open, open.segments().len(), CURSOR_EXPIRES_US)?.encode();
    let full = get(
        open,
        source.range(),
        ResultView::Full,
        open_growth(&token, None),
    )?;
    validate("operation-response.schema.json", &full)?;
    validate("transcript-get-data.schema.json", &full["data"])?;
    assert_eq!(full["status"], "partial");
    assert_eq!(full["data"]["revision"]["state"], "open");
    assert_eq!(
        full["data"]["revision"]["published_until_us"],
        open.published_until().ok_or("open")?.as_micros()
    );
    let pending = &full["data"]["transcript_coverage"]["pending_ranges"];
    assert_eq!(
        pending[0]["from_us"],
        open.published_until().ok_or("open")?.as_micros()
    );
    assert_eq!(pending[0]["to_us"], 120 * SECOND);
    assert_eq!(
        full["coverage"]["reasons"],
        serde_json::json!(["untranscribed_range", TRANSCRIPTION_PENDING_REASON])
    );
    assert_eq!(
        full["warnings"],
        serde_json::json!([TRANSCRIPTION_PENDING_WARNING])
    );
    assert_eq!(full["data"]["since"]["revision_state"], "open");
    assert_eq!(full["data"]["since"]["since_token"], token);
    assert_example("transcript-get.open.json", &full)?;

    let brief = get(
        open,
        source.range(),
        ResultView::Brief,
        open_growth(&token, None),
    )?;
    validate("transcript-get-brief-data.schema.json", &brief["data"])?;
    assert_eq!(brief["data"]["since"], full["data"]["since"]);

    let page = page_transcript(
        &session,
        open,
        &TranscriptPageRequest {
            range: source.range(),
            limit: PageLimit::new(100)?,
            cursor: None,
        },
        CURSOR_EXPIRES_US,
        NOW_US,
    )?;
    let segments: Vec<_> = page.segments.into_iter().cloned().collect();
    let stream = TranscriptEvidenceStream::new(
        &TranscriptPresentation {
            session_id: &session,
            revision: open,
            range: source.range(),
            segments: &segments,
            next_cursor: None,
            coverage: &page.coverage,
            growth: open_growth(&token, None),
        },
        LifecycleResponse::ephemeral(EXPIRES_AT.to_owned()),
    )?;
    let terminal = serde_json::to_value(stream.terminal())?;
    validate(
        "transcript-get-stream-data.schema.json",
        &terminal["result"]["data"],
    )?;
    assert_eq!(terminal["result"]["status"], "partial");
    Ok(())
}

/// A read of a range the open revision has fully published is complete; a
/// range that is partly untranscribed for another reason and partly pending
/// carries both warnings.
#[tokio::test]
async fn only_what_is_pending_makes_an_open_read_partial() -> TestResult {
    let source = source()?;
    let (_, open) = run(&source, 1, None, false).await?;
    let open = open.get(2).ok_or("no third cut")?;
    let until = open.published_until().ok_or("open")?.as_micros() / SECOND;
    let settled = get(
        open,
        window(0, until)?,
        ResultView::Full,
        ReadGrowth::default(),
    )?;
    assert_eq!(settled["status"], "complete");
    assert!(
        settled["data"]["transcript_coverage"]
            .get("pending_ranges")
            .is_none()
    );
    Ok(())
}

/// Over an open revision that supersedes a sealed one, the answer names the
/// earlier revision and the pending parts it holds text over.
#[tokio::test]
async fn the_earlier_revision_over_what_is_pending_is_named() -> TestResult {
    let source = source()?;
    let (base, _) = run(&source, 1, None, false).await?;
    let (_, open) = run(&source, 2, Some(&base), true).await?;
    let open = open.get(1).ok_or("no second cut")?;
    assert_eq!(open.supersedes(), Some(base.id()));
    let coverage = SearchCoverage::of(open, None);
    let pending = coverage.pending().to_vec();
    let session = SessionId::parse(SESSION)?;
    let token = since_token(&session, open, open.segments().len(), CURSOR_EXPIRES_US)?.encode();
    let response = get(
        open,
        source.range(),
        ResultView::Full,
        open_growth(
            &token,
            Some(PendingCover {
                revision: base.id(),
                ranges: &pending,
            }),
        ),
    )?;
    validate("transcript-get-data.schema.json", &response["data"])?;
    let cover = &response["data"]["transcript_coverage"]["pending_covered_by"];
    assert_eq!(cover["revision_id"], base.id().as_str());
    assert_eq!(
        cover["ranges"][0]["from_us"],
        open.published_until().ok_or("open")?.as_micros()
    );
    Ok(())
}

/// A since read answers only what was published after the token, in start
/// order, and a token that follows a revision into its sealed form reads the
/// rest; the search over an open revision is partial and validates in every
/// view, and a since search finds only the new hits.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "One scenario: the since reads of a run, then the search of the same cut"
)]
async fn since_reads_answer_what_was_published_after_the_token() -> TestResult {
    let source = source()?;
    let (sealed, open) = run(&source, 1, None, false).await?;
    let session = SessionId::parse(SESSION)?;
    let early = open.first().ok_or("no cut")?;
    let later = open.get(2).ok_or("no third cut")?;
    let seen = early.segments().len();
    // What the later cut published after the early one.
    let page = page_since(
        &session,
        later,
        seen,
        None,
        None,
        PageLimit::new(100)?,
        CURSOR_EXPIRES_US,
    )?;
    let new: Vec<_> = page
        .segments
        .iter()
        .map(|(segment, _)| (*segment).clone())
        .collect();
    assert_eq!(new.as_slice(), &later.segments()[seen..]);
    assert_ne!(new, Vec::new());
    assert!(!page.more);
    // The sealed revision continues where the open one stopped.
    let rest = page_since(
        &session,
        &sealed,
        later.segments().len(),
        None,
        None,
        PageLimit::new(100)?,
        CURSOR_EXPIRES_US,
    )?;
    assert_eq!(
        rest.segments.len(),
        sealed.segments().len() - later.segments().len()
    );
    // A limit cuts the answer and says more is there.
    let cut = page_since(
        &session,
        &sealed,
        0,
        None,
        None,
        PageLimit::new(1)?,
        CURSOR_EXPIRES_US,
    )?;
    assert_eq!(cut.segments.len(), 1);
    assert!(cut.more);
    // A since search finds only new hits.
    let query = SearchQuery::parse("second")?;
    let hits = page_since(
        &session,
        later,
        seen,
        None,
        Some(&query),
        PageLimit::new(100)?,
        CURSOR_EXPIRES_US,
    )?;
    assert!(
        hits.segments
            .iter()
            .all(|(segment, tier)| tier.is_some() && segment.text().text().contains("second"))
    );
    // A search of the open revision, in each view and as a stream.
    let token = page.next.encode();
    let search_hits: Vec<_> = hits
        .segments
        .iter()
        .filter_map(|(segment, tier)| tier.map(|tier| (*segment, tier)))
        .collect();
    let coverage = SearchCoverage::of(later, None);
    let presentation = SearchPresentation {
        session_id: &session,
        revision: later,
        query: &query,
        range: None,
        hits: search_hits,
        next_cursor: None,
        coverage: &coverage,
        growth: open_growth(&token, None),
    };
    for view in [ResultView::Full, ResultView::Brief] {
        let response = serde_json::to_value(search_response_with_view(
            &presentation,
            LifecycleResponse::ephemeral(EXPIRES_AT.to_owned()),
            view,
        )?)?;
        validate("operation-response.schema.json", &response)?;
        let schema = match view {
            ResultView::Full => "search-data.schema.json",
            ResultView::Brief => "search-brief-data.schema.json",
        };
        validate(schema, &response["data"])?;
        assert_eq!(response["status"], "partial");
        assert_eq!(
            response["warnings"],
            serde_json::json!([TRANSCRIPTION_PENDING_WARNING])
        );
        if view == ResultView::Full {
            assert_example("search.open.json", &response)?;
        }
    }
    let stream = SearchEvidenceStream::new(
        &presentation,
        LifecycleResponse::ephemeral(EXPIRES_AT.to_owned()),
    )?;
    let terminal = serde_json::to_value(stream.terminal())?;
    validate(
        "search-stream-data.schema.json",
        &terminal["result"]["data"],
    )?;
    Ok(())
}

/// A stopped revision is presented with its state and the time it was
/// published until; no part of it is pending, and what its run did not reach
/// is untranscribed under the read's own warning.
#[tokio::test]
async fn a_stopped_revision_is_presented_with_its_state() -> TestResult {
    let source = source()?;
    let (_, open) = run(&source, 1, None, false).await?;
    let stopped = build_stopped_asr_revision(open.get(1).ok_or("no second cut")?, None)?;
    let response = get(
        &stopped,
        source.range(),
        ResultView::Full,
        ReadGrowth::default(),
    )?;
    validate("operation-response.schema.json", &response)?;
    validate("transcript-get-data.schema.json", &response["data"])?;
    assert_eq!(response["data"]["revision"]["state"], "stopped");
    assert!(response["data"]["revision"]["local_asr"]["not_reached_chunks"].is_number());
    assert!(
        response["data"]["transcript_coverage"]
            .get("pending_ranges")
            .is_none()
    );
    assert_eq!(response["status"], "partial");
    assert_eq!(
        response["warnings"],
        serde_json::json!([UNTRANSCRIBED_READ_WARNING])
    );
    Ok(())
}

/// A sealed revision read without --since is presented exactly as before 0.3.0:
/// no state, no time it is published until, no pending ranges and no since.
#[tokio::test]
async fn a_sealed_revision_is_presented_as_before() -> TestResult {
    let source = source()?;
    let (sealed, _) = run(&source, 1, None, false).await?;
    let response = get(
        &sealed,
        source.range(),
        ResultView::Full,
        ReadGrowth::default(),
    )?;
    let data = &response["data"];
    for member in ["state", "published_until_us"] {
        assert!(data["revision"].get(member).is_none(), "{member}");
    }
    assert!(data.get("since").is_none());
    assert!(data["transcript_coverage"].get("pending_ranges").is_none());
    assert_eq!(response["status"], "complete");
    Ok(())
}
