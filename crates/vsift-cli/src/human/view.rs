//! Typed views of the v1 results the human renderers read.
//!
//! The contract's data types keep their fields private: their one published
//! form is the v1 JSON, fixed by the schemas in `schemas/v1`. A renderer
//! therefore reads a result back from that form into the views below, each
//! a typed subset of one published shape. Field names are checked by the
//! decoder, so a result that does not match its schema fails to render
//! (as an internal serialization failure) instead of printing half of it.
//!
//! **No view has a field for raw untrusted text.** Segments are read with
//! `display_text` and speakers with `display_label`, as [`DisplayText`];
//! `text`, `original_text`, `label` and search terms have no field here, so
//! a renderer cannot quote them. The one other text a user or the host can
//! influence is a delivered file's `path` (it holds the session root), which
//! renderers write only through the builder's path entry.

use serde::Deserialize;

use super::text::DisplayText;

/// The envelope of a result, with its `data` as `D`.
#[derive(Debug, Deserialize)]
pub(crate) struct Envelope<D> {
    pub(crate) status: Status,
    pub(crate) operation_id: Option<String>,
    pub(crate) data: D,
    pub(crate) warnings: Vec<String>,
    pub(crate) lifecycle: Option<Lifecycle>,
}

/// The envelope's `status`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    Complete,
    Partial,
    Failed,
    Cancelled,
}

/// The envelope's `lifecycle`.
#[derive(Debug, Deserialize)]
pub(crate) struct Lifecycle {
    pub(crate) mode: String,
    pub(crate) expires_at: Option<String>,
}

/// A failed result: its `error` only.
#[derive(Debug, Deserialize)]
pub(crate) struct FailureEnvelope {
    pub(crate) error: Error,
}

/// The envelope's `error`.
#[derive(Debug, Deserialize)]
pub(crate) struct Error {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) affected_ids: Vec<String>,
    pub(crate) remediation: Vec<Remediation>,
}

/// One remediation of an error.
#[derive(Debug, Deserialize)]
pub(crate) struct Remediation {
    pub(crate) summary: String,
    pub(crate) command: Option<SuggestedCommand>,
}

/// A remediation's suggested command: fixed words and validated identifiers.
#[derive(Debug, Deserialize)]
pub(crate) struct SuggestedCommand {
    pub(crate) executable: String,
    pub(crate) arguments: Vec<String>,
}

/// A half-open source range in microseconds.
#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct Range {
    pub(crate) from_us: u64,
    pub(crate) to_us: u64,
}

/// A transcript revision summary (`transcript-revision.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct TranscriptRevision {
    pub(crate) revision_id: String,
    pub(crate) revision: u32,
    pub(crate) alignment: RevisionAlignment,
    pub(crate) sidecar: Option<Sidecar>,
    pub(crate) language: Option<String>,
    pub(crate) segment_count: u64,
    pub(crate) warnings: Vec<RevisionWarning>,
    #[serde(default)]
    pub(crate) local_asr: Option<LocalAsrRun>,
    #[serde(default)]
    pub(crate) supersedes: Option<String>,
    #[serde(default)]
    pub(crate) replaced_range: Option<Range>,
    #[serde(default)]
    pub(crate) carried_segment_count: Option<u64>,
}

/// How a revision is aligned to the source.
#[derive(Debug, Deserialize)]
pub(crate) struct RevisionAlignment {
    pub(crate) origin: String,
    #[serde(default)]
    pub(crate) offset_us: Option<i64>,
}

/// The supplied file an imported revision came from.
#[derive(Debug, Deserialize)]
pub(crate) struct Sidecar {
    pub(crate) format: String,
    pub(crate) sha256: String,
    pub(crate) bytes: u64,
}

/// A typed warning of a revision.
#[derive(Debug, Deserialize)]
pub(crate) struct RevisionWarning {
    pub(crate) code: String,
    pub(crate) count: u64,
    pub(crate) first_cue: u64,
}

/// What a local-ASR run was.
#[derive(Debug, Deserialize)]
pub(crate) struct LocalAsrRun {
    pub(crate) provider: String,
    pub(crate) model_profile: String,
    pub(crate) model_sha256: String,
    pub(crate) executable_sha256: String,
    pub(crate) decoding_profile: String,
    pub(crate) threads: u64,
    pub(crate) chunk_count: u64,
    pub(crate) transcribed_chunks: u64,
    pub(crate) silent_chunks: u64,
    pub(crate) no_audio_chunks: u64,
}

/// One transcript segment, without its raw text.
#[derive(Debug, Deserialize)]
pub(crate) struct TranscriptSegment {
    pub(crate) segment_id: String,
    pub(crate) start_us: u64,
    pub(crate) end_us: u64,
    pub(crate) display_text: DisplayText,
    pub(crate) markup: Markup,
    pub(crate) speaker: Option<Speaker>,
    pub(crate) confidence: Confidence,
    pub(crate) language: Option<String>,
    pub(crate) cue: Option<Cue>,
    #[serde(default)]
    pub(crate) carried_from: Option<CarriedFrom>,
}

/// Whether markup was removed from a segment's text.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Markup {
    None,
    Removed,
}

/// A segment's speaker, without its raw label.
#[derive(Debug, Deserialize)]
pub(crate) struct Speaker {
    pub(crate) display_label: DisplayText,
    pub(crate) origin: String,
}

/// A segment's confidence.
#[derive(Debug, Deserialize)]
pub(crate) struct Confidence {
    pub(crate) value_basis_points: Option<u16>,
}

/// The cue an imported segment came from.
#[derive(Debug, Deserialize)]
pub(crate) struct Cue {
    pub(crate) ordinal: u64,
    pub(crate) line: u64,
}

/// Where a carried segment was first produced.
#[derive(Debug, Deserialize)]
pub(crate) struct CarriedFrom {
    pub(crate) revision_id: String,
}

/// `transcript.get` data.
#[derive(Debug, Deserialize)]
pub(crate) struct TranscriptPage {
    pub(crate) session_id: String,
    pub(crate) revision: TranscriptRevision,
    pub(crate) range: Range,
    pub(crate) items: Vec<TranscriptSegment>,
    pub(crate) next_cursor: Option<String>,
}

/// `transcript.retranscribe` data.
#[derive(Debug, Deserialize)]
pub(crate) struct Retranscription {
    pub(crate) session_id: String,
    pub(crate) requested_range: Option<Range>,
    pub(crate) revision: TranscriptRevision,
    pub(crate) recognised_segment_count: u64,
    pub(crate) job: RetranscriptionJob,
}

/// The job behind a retranscription.
#[derive(Debug, Deserialize)]
pub(crate) struct RetranscriptionJob {
    pub(crate) job_id: String,
    pub(crate) resumed: bool,
    pub(crate) chunks_reused: u64,
    pub(crate) replayed: bool,
}

/// `search` data.
#[derive(Debug, Deserialize)]
pub(crate) struct Search {
    pub(crate) session_id: String,
    pub(crate) revision: TranscriptRevision,
    pub(crate) query: SearchQuery,
    pub(crate) range: Option<Range>,
    pub(crate) items: Vec<TranscriptSegment>,
    pub(crate) hits: Vec<SearchHit>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) transcript_coverage: TranscriptCoverage,
}

/// The normalised query; only how many terms it has is shown, because the
/// terms are the caller's own text.
#[derive(Debug, Deserialize)]
pub(crate) struct SearchQuery {
    pub(crate) terms: Vec<serde::de::IgnoredAny>,
}

/// One search hit.
#[derive(Debug, Deserialize)]
pub(crate) struct SearchHit {
    pub(crate) segment_id: String,
    #[serde(rename = "match")]
    pub(crate) tier: String,
}

/// What part of the source a search could read.
#[derive(Debug, Deserialize)]
pub(crate) struct TranscriptCoverage {
    pub(crate) basis: String,
    pub(crate) searched_range: Option<Range>,
    pub(crate) untranscribed_ranges: Vec<Range>,
    pub(crate) no_speech_ranges: Vec<Range>,
    pub(crate) ranges_truncated: bool,
}

/// `ingest` data.
#[derive(Debug, Deserialize)]
pub(crate) struct Opened {
    pub(crate) session_id: String,
    pub(crate) source_id: String,
    pub(crate) source_bytes: u64,
    pub(crate) generation: u64,
    pub(crate) publication: String,
    #[serde(default)]
    pub(crate) transcript: Option<TranscriptRevision>,
}

/// A session's committed status (`session status`, `renew`, `close`).
#[derive(Debug, Deserialize)]
pub(crate) struct SessionStatus {
    pub(crate) session_id: String,
    pub(crate) state: String,
    pub(crate) source_id: String,
    pub(crate) source_bytes: u64,
    pub(crate) artifact_count: u64,
    pub(crate) artifact_bytes: u64,
    pub(crate) generation: u64,
    pub(crate) expires_at: String,
    /// Only `session status` lists jobs.
    #[serde(default)]
    pub(crate) jobs: Option<Vec<SessionJob>>,
    #[serde(default)]
    pub(crate) jobs_truncated: bool,
}

/// One job of `session status`.
#[derive(Debug, Deserialize)]
pub(crate) struct SessionJob {
    pub(crate) job_id: String,
    pub(crate) kind: String,
    pub(crate) state: String,
    pub(crate) live_owner: bool,
    pub(crate) resumable: bool,
    pub(crate) resumable_reason: String,
}

/// `session.list` data.
#[derive(Debug, Deserialize)]
pub(crate) struct SessionPage {
    pub(crate) items: Vec<ListedSession>,
    pub(crate) next_cursor: Option<u16>,
}

/// One `session list` item.
#[derive(Debug, Deserialize)]
pub(crate) struct ListedSession {
    pub(crate) session_id: String,
    pub(crate) state: String,
    pub(crate) status: Option<SessionStatus>,
    pub(crate) error_code: Option<String>,
}

/// `session.clean` data.
#[derive(Debug, Deserialize)]
pub(crate) struct CleanPage {
    pub(crate) items: Vec<CleanItem>,
    pub(crate) next_cursor: Option<u16>,
    pub(crate) dry_run: bool,
}

/// One `session clean` item.
#[derive(Debug, Deserialize)]
pub(crate) struct CleanItem {
    pub(crate) session_id: String,
    pub(crate) outcome: String,
    pub(crate) error_code: Option<String>,
}

/// `session.retain` and `bundle.validate` data.
#[derive(Debug, Deserialize)]
pub(crate) struct Bundle {
    pub(crate) session_id: String,
    pub(crate) source_id: String,
    pub(crate) source_bytes: u64,
    pub(crate) source_included: bool,
    pub(crate) artifact_count: u64,
    pub(crate) artifact_bytes: u64,
    pub(crate) publication: String,
}

/// `session.init-workspace` data.
#[derive(Debug, Deserialize)]
pub(crate) struct Workspace {
    pub(crate) profile: String,
    pub(crate) durability: String,
    pub(crate) publication: String,
    pub(crate) admission_capacity: u64,
    pub(crate) session_retention_seconds: u64,
    pub(crate) outcome: String,
}

/// `setup.configure` data.
#[derive(Debug, Deserialize)]
pub(crate) struct ConfiguredSelection {
    pub(crate) dependency: String,
    pub(crate) source: String,
    pub(crate) validation: String,
    pub(crate) next_step: String,
}

/// `setup.configure-model` data.
#[derive(Debug, Deserialize)]
pub(crate) struct ConfiguredModel {
    pub(crate) source: String,
    pub(crate) validation: String,
    pub(crate) next_step: String,
}

/// `setup.install` data (`setup-install.schema.json`), on a success or
/// beside the error of a failure.
#[derive(Debug, Deserialize)]
pub(crate) struct SetupInstall {
    pub(crate) catalogue_revision: Option<String>,
    pub(crate) source: String,
    pub(crate) components: Vec<InstallComponent>,
    pub(crate) cleanup: InstallCleanup,
    pub(crate) next_step: String,
}

/// What `setup install` cleaned up around its transaction (P13 PR 6).
#[derive(Debug, Deserialize)]
pub(crate) struct InstallCleanup {
    pub(crate) stale_stages_removed: u64,
    pub(crate) stale_stages_retained: u64,
    pub(crate) versions: Vec<VersionOutcome>,
}

/// One version a removal or cleanup handled.
#[derive(Debug, Deserialize)]
pub(crate) struct VersionOutcome {
    pub(crate) component: String,
    pub(crate) version: String,
    pub(crate) status: String,
}

/// `setup.list` data (`setup-list.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct SetupList {
    pub(crate) managed_install: String,
    pub(crate) managed_folder: String,
    pub(crate) components: Vec<ListComponent>,
    pub(crate) stale_stages: u64,
    pub(crate) retained_stages: u64,
    pub(crate) next_step: String,
}

/// One component of a `setup.list` result.
#[derive(Debug, Deserialize)]
pub(crate) struct ListComponent {
    pub(crate) component: String,
    pub(crate) selection: String,
    pub(crate) versions: Vec<ListVersion>,
}

/// One published version of a `setup.list` result.
#[derive(Debug, Deserialize)]
pub(crate) struct ListVersion {
    pub(crate) version: String,
    pub(crate) selected: bool,
    pub(crate) previous: bool,
    pub(crate) state: String,
    pub(crate) fault: Option<String>,
}

/// `setup.rollback` data (`setup-rollback.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct SetupRollback {
    pub(crate) component: String,
    pub(crate) status: String,
    pub(crate) selected_version: String,
    pub(crate) replaced_version: Option<String>,
    pub(crate) next_step: String,
}

/// `setup.remove` data (`setup-remove.schema.json`), on a success or beside
/// the error of a failure.
#[derive(Debug, Deserialize)]
pub(crate) struct SetupRemove {
    pub(crate) target: String,
    pub(crate) component: Option<String>,
    pub(crate) deselected: bool,
    pub(crate) versions: Vec<VersionOutcome>,
    pub(crate) stages: Option<StageSweep>,
    pub(crate) next_step: String,
}

/// What a stale-stage sweep did.
#[derive(Debug, Deserialize)]
pub(crate) struct StageSweep {
    pub(crate) removed: u64,
    pub(crate) retained: u64,
    pub(crate) retention_reasons: Vec<String>,
    pub(crate) interrupted_selections_removed: u64,
}

/// `setup.repair` data (`setup-repair.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct SetupRepair {
    pub(crate) managed_install: String,
    pub(crate) status: String,
    pub(crate) findings: Vec<RepairFinding>,
    pub(crate) next_step: String,
}

/// One finding of a `setup.repair` result.
#[derive(Debug, Deserialize)]
pub(crate) struct RepairFinding {
    pub(crate) kind: String,
    pub(crate) summary: String,
    pub(crate) command: Option<SuggestedCommand>,
}

/// One component of a `setup.install` result.
#[derive(Debug, Deserialize)]
pub(crate) struct InstallComponent {
    pub(crate) component: String,
    pub(crate) version: String,
    pub(crate) status: String,
    pub(crate) step: Option<String>,
    pub(crate) reason: Option<String>,
    pub(crate) failure_code: Option<String>,
    pub(crate) smoke_check: Option<String>,
    pub(crate) stage: Option<String>,
    pub(crate) retention_reason: Option<String>,
}

/// `setup.plan` data, in both published forms (`setup-plan.schema.json`
/// and `setup-plan-unqualified.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct SetupPlan {
    pub(crate) profile: String,
    pub(crate) readiness: String,
    pub(crate) verification_scope: String,
    #[serde(default)]
    pub(crate) target: Option<String>,
    pub(crate) local_asr_model: PlanModel,
    pub(crate) managed_install: String,
    #[serde(default)]
    pub(crate) catalogue_revision: Option<String>,
    #[serde(default)]
    pub(crate) stop_new_plans_at: Option<String>,
    pub(crate) plan_digest: Option<String>,
    /// Absent from plans made before P13 PR 4.
    #[serde(default)]
    pub(crate) install_needed: Option<bool>,
    pub(crate) actions: Vec<PlanAction>,
    pub(crate) dependencies: Vec<PlanDependency>,
}

/// The plan's local-ASR model: a summary identifier in the unqualified
/// form, a planned step in the reviewed form.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum PlanModel {
    Summary(String),
    Step(PlanStep),
}

/// A dependency's planned step.
#[derive(Debug, Deserialize)]
pub(crate) struct PlanStep {
    pub(crate) status: String,
    pub(crate) disposition: String,
    pub(crate) required_authority: Option<String>,
    pub(crate) next_step: String,
}

/// One dependency of the plan.
#[derive(Debug, Deserialize)]
pub(crate) struct PlanDependency {
    pub(crate) dependency: String,
    #[serde(flatten)]
    pub(crate) step: PlanStep,
}

/// One reviewed managed action of the plan.
#[derive(Debug, Deserialize)]
pub(crate) struct PlanAction {
    pub(crate) id: String,
    /// `pending` or `current`; absent from plans made before P13 PR 4.
    #[serde(default)]
    pub(crate) state: Option<String>,
    pub(crate) component: String,
    pub(crate) version: String,
    pub(crate) publisher: String,
    pub(crate) source_url: String,
    pub(crate) bytes: u64,
    pub(crate) sha256: String,
    pub(crate) licence: String,
    pub(crate) notice_url: String,
    pub(crate) trust_limit: String,
    pub(crate) files: Vec<PlanFile>,
}

/// One file an action installs.
#[derive(Debug, Deserialize)]
pub(crate) struct PlanFile {
    pub(crate) name: String,
    pub(crate) bytes: u64,
    pub(crate) mode: String,
}

/// One delivered file of an evidence result (`files[]`): the absolute path
/// of the committed artifact, valid while the session exists.
#[derive(Debug, Deserialize)]
pub(crate) struct DeliveredFile {
    pub(crate) evidence_id: String,
    pub(crate) media_type: String,
    pub(crate) path: String,
}

/// Which item a requested time resolved to.
#[derive(Debug, Deserialize)]
pub(crate) struct Selection {
    pub(crate) role: String,
    pub(crate) evidence_id: String,
    pub(crate) requested_us: u64,
    pub(crate) actual_us: u64,
    pub(crate) delta_us: i64,
}

/// The request of a frame or crop result: the members of every
/// operation's request shape, each present only for its operations.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct FrameRequest {
    #[serde(default)]
    pub(crate) at_us: Option<u64>,
    #[serde(default)]
    pub(crate) policy: Option<String>,
    #[serde(default)]
    pub(crate) tolerance_us: Option<u64>,
    #[serde(default)]
    pub(crate) candidate_id: Option<String>,
    #[serde(default)]
    pub(crate) anchor_evidence_id: Option<String>,
    #[serde(default)]
    pub(crate) count: Option<u64>,
    #[serde(default)]
    pub(crate) from_us: Option<u64>,
    #[serde(default)]
    pub(crate) to_us: Option<u64>,
    #[serde(default)]
    pub(crate) max_frames: Option<u64>,
    #[serde(default)]
    pub(crate) parent_evidence_id: Option<String>,
    #[serde(default)]
    pub(crate) rect: Option<Rectangle>,
}

/// A crop rectangle in the parent image's displayed pixels.
#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct Rectangle {
    pub(crate) x: u64,
    pub(crate) y: u64,
    pub(crate) width: u64,
    pub(crate) height: u64,
}

/// Why each side of `frame neighbours` stopped short, if it did.
#[derive(Debug, Deserialize)]
pub(crate) struct NeighbourStops {
    pub(crate) before_stop: Option<String>,
    pub(crate) after_stop: Option<String>,
}

/// The plan of `frame burst`.
#[derive(Debug, Deserialize)]
pub(crate) struct BurstPlan {
    pub(crate) planned: SpanRange,
    pub(crate) extent: String,
    pub(crate) targets: u64,
    pub(crate) distinct: u64,
}

/// A half-open range written with `start_us` and `end_us`.
#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct SpanRange {
    pub(crate) start_us: u64,
    pub(crate) end_us: u64,
}

impl From<SpanRange> for Range {
    fn from(span: SpanRange) -> Self {
        Self {
            from_us: span.start_us,
            to_us: span.end_us,
        }
    }
}

/// One frame or crop evidence record (`frame-evidence.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct FrameEvidence {
    pub(crate) evidence_id: String,
    pub(crate) stream_index: u64,
    pub(crate) frame: FrameTime,
    pub(crate) crop: Option<CropGeometry>,
    pub(crate) image: ImageFacts,
}

/// The decoded frame an item was taken from.
#[derive(Debug, Deserialize)]
pub(crate) struct FrameTime {
    pub(crate) pts: i64,
    pub(crate) time_us: u64,
    pub(crate) width: u64,
    pub(crate) height: u64,
}

/// Where a crop lies in its parent image and in the frame.
#[derive(Debug, Deserialize)]
pub(crate) struct CropGeometry {
    pub(crate) parent_evidence_id: String,
    pub(crate) x: u64,
    pub(crate) y: u64,
    pub(crate) width: u64,
    pub(crate) height: u64,
    pub(crate) frame_x: u64,
    pub(crate) frame_y: u64,
}

/// The delivered image of a frame or crop.
#[derive(Debug, Deserialize)]
pub(crate) struct ImageFacts {
    pub(crate) width: u64,
    pub(crate) height: u64,
    pub(crate) sha256: String,
    pub(crate) bytes: u64,
}

/// `frame.get`, `frame.neighbours`, `frame.burst` and `crop` data
/// (`frame-data.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct FrameData {
    pub(crate) session_id: String,
    pub(crate) source_id: String,
    pub(crate) request: FrameRequest,
    pub(crate) reused: bool,
    pub(crate) profile: String,
    pub(crate) source_check: String,
    pub(crate) selections: Vec<Selection>,
    pub(crate) neighbours: Option<NeighbourStops>,
    pub(crate) burst: Option<BurstPlan>,
    pub(crate) items: Vec<FrameEvidence>,
    pub(crate) files: Vec<DeliveredFile>,
    pub(crate) partial_reason: Option<String>,
}

/// One audio evidence record (`audio-evidence.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct AudioEvidence {
    pub(crate) evidence_id: String,
    pub(crate) stream_index: u64,
    pub(crate) range: SpanRange,
    pub(crate) actual_start_us: u64,
    pub(crate) audio: AudioFacts,
}

/// The delivered clip of an audio item.
#[derive(Debug, Deserialize)]
pub(crate) struct AudioFacts {
    pub(crate) sample_rate: u64,
    pub(crate) channels: u64,
    pub(crate) sample_format: String,
    pub(crate) sha256: String,
    pub(crate) bytes: u64,
}

/// `audio` data (`audio-data.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct AudioData {
    pub(crate) session_id: String,
    pub(crate) source_id: String,
    pub(crate) request: Range,
    pub(crate) reused: bool,
    pub(crate) profile: String,
    pub(crate) source_check: String,
    pub(crate) selections: Vec<Selection>,
    pub(crate) range_clipped: bool,
    pub(crate) items: Vec<AudioEvidence>,
    pub(crate) files: Vec<DeliveredFile>,
    pub(crate) partial_reason: Option<String>,
}

/// `candidates` data (`candidates-data.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct CandidatesPage {
    pub(crate) session_id: String,
    pub(crate) range: Range,
    pub(crate) index: VisualIndex,
    pub(crate) coverage: CandidatesCoverage,
    pub(crate) items: Vec<VisualCandidate>,
    pub(crate) next_cursor: Option<String>,
}

/// The visual-index revision a candidates page was read from.
#[derive(Debug, Deserialize)]
pub(crate) struct VisualIndex {
    pub(crate) index_id: String,
    pub(crate) number: u64,
    pub(crate) profile: String,
    pub(crate) duration_us: u64,
}

/// What part of the range a candidates page could analyse.
#[derive(Debug, Deserialize)]
pub(crate) struct CandidatesCoverage {
    pub(crate) searched_range: Range,
    pub(crate) analyzed: Vec<Range>,
    pub(crate) gaps: Vec<CandidatesGap>,
    pub(crate) ranges_truncated: bool,
}

/// One typed gap of a candidates page's coverage.
#[derive(Debug, Deserialize)]
pub(crate) struct CandidatesGap {
    pub(crate) from_us: u64,
    pub(crate) to_us: u64,
    pub(crate) reason: String,
    pub(crate) dropped_candidates: u64,
}

/// One visual candidate (`visual-candidate.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct VisualCandidate {
    pub(crate) candidate_id: String,
    pub(crate) representative_us: u64,
    pub(crate) span: Range,
    pub(crate) change_window: Option<Range>,
    pub(crate) reasons: Vec<String>,
    pub(crate) stability: String,
    pub(crate) change: Option<VisualChange>,
    pub(crate) visual_hash: String,
    pub(crate) sample_count: u64,
    pub(crate) displayed_dimensions: Dimensions,
}

/// How much a change candidate's frame changed.
#[derive(Debug, Deserialize)]
pub(crate) struct VisualChange {
    pub(crate) changed_blocks: u64,
    pub(crate) max_block_delta: u64,
}

/// Displayed width and height in pixels.
#[derive(Clone, Copy, Debug, Deserialize)]
pub(crate) struct Dimensions {
    pub(crate) width: u64,
    pub(crate) height: u64,
}

/// A recoverable job (`job-data.schema.json`): `job status`, `job cancel`
/// and the job of `job resume`.
#[derive(Debug, Deserialize)]
pub(crate) struct RecoverableJob {
    pub(crate) job_id: String,
    pub(crate) session_id: String,
    pub(crate) kind: String,
    pub(crate) state: String,
    pub(crate) live_owner: bool,
    pub(crate) resumable: bool,
    pub(crate) resumable_reason: String,
    pub(crate) operation_id: Option<String>,
    pub(crate) request: JobRequest,
    pub(crate) progress: JobProgress,
    pub(crate) attempts: u64,
    pub(crate) result: Option<JobResult>,
    pub(crate) failure: Option<JobFailure>,
}

/// What a job was asked to do.
#[derive(Debug, Deserialize)]
pub(crate) struct JobRequest {
    pub(crate) range: Option<Range>,
}

/// How far a job has come.
#[derive(Debug, Deserialize)]
pub(crate) struct JobProgress {
    pub(crate) chunks_total: Option<u64>,
    pub(crate) chunks_checkpointed: u64,
}

/// What a succeeded job committed.
#[derive(Debug, Deserialize)]
pub(crate) struct JobResult {
    pub(crate) revision_id: String,
    pub(crate) generation: u64,
}

/// The failure that ended a job's last attempt.
#[derive(Debug, Deserialize)]
pub(crate) struct JobFailure {
    pub(crate) code: String,
    pub(crate) retryable: bool,
}

/// `job.resume` data (`job-resume-data.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct JobResume {
    pub(crate) job: RecoverableJob,
    pub(crate) outcome: Retranscription,
}

/// A worker request's result (`job-result.schema.json`), the data of
/// `job run`. Its operation id and lifecycle are the envelope's too, and are
/// written from there.
#[derive(Debug, Deserialize)]
pub(crate) struct WorkResult {
    pub(crate) request_digest: String,
    pub(crate) status: String,
    pub(crate) replayed: bool,
    pub(crate) attempt: u64,
    pub(crate) session_id: Option<String>,
    pub(crate) source_id: Option<String>,
    pub(crate) publication: Option<String>,
    pub(crate) steps: Vec<WorkStep>,
    pub(crate) failure: Option<WorkFailure>,
    pub(crate) controls: WorkControls,
}

/// One step of a worker request.
#[derive(Debug, Deserialize)]
pub(crate) struct WorkStep {
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) elapsed_ms: u64,
    pub(crate) admission_wait_ms: u64,
    pub(crate) job_id: Option<String>,
    pub(crate) outputs: Option<StepOutputs>,
    pub(crate) coverage: Option<StepCoverage>,
    pub(crate) failure: Option<StepFailure>,
}

/// A step's typed outputs: the members of every step kind's outputs, each
/// present only for its kind.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct StepOutputs {
    #[serde(default)]
    pub(crate) generation: Option<u64>,
    #[serde(default)]
    pub(crate) revision_id: Option<String>,
    #[serde(default)]
    pub(crate) chunks_reused: Option<u64>,
    #[serde(default)]
    pub(crate) visual_index_id: Option<String>,
    #[serde(default)]
    pub(crate) candidate_count: Option<u64>,
    #[serde(default)]
    pub(crate) bundle_name: Option<String>,
    #[serde(default)]
    pub(crate) bundle_sha256: Option<String>,
    #[serde(default)]
    pub(crate) artifact_count: Option<u64>,
}

/// What part of a step's range stayed uncovered.
#[derive(Debug, Deserialize)]
pub(crate) struct StepCoverage {
    pub(crate) truncated: bool,
    pub(crate) gaps: Vec<String>,
    pub(crate) reasons: Vec<String>,
}

/// The failure of one step.
#[derive(Debug, Deserialize)]
pub(crate) struct StepFailure {
    pub(crate) code: String,
    pub(crate) retryable: bool,
    pub(crate) retry_after_ms: Option<u64>,
}

/// The failure that ended a worker request.
#[derive(Debug, Deserialize)]
pub(crate) struct WorkFailure {
    pub(crate) code: String,
    pub(crate) retryable: bool,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) step: Option<u64>,
    pub(crate) rejection: Option<String>,
}

/// The controls a worker request ran under.
#[derive(Debug, Deserialize)]
pub(crate) struct WorkControls {
    pub(crate) isolation: String,
    pub(crate) admission_capacity: u64,
    pub(crate) concurrency: u64,
    pub(crate) resource_limits: String,
    pub(crate) free_space_reserve: String,
}

/// `job.batch` data (`job-batch-data.schema.json`).
#[derive(Debug, Deserialize)]
pub(crate) struct BatchSummary {
    pub(crate) counts: BatchCounts,
    pub(crate) items: Vec<BatchItem>,
    pub(crate) not_started_from_line: Option<u64>,
    pub(crate) termination_reason: String,
}

/// How many lines of a batch ended each way.
#[derive(Debug, Deserialize)]
pub(crate) struct BatchCounts {
    pub(crate) complete: u64,
    pub(crate) partial: u64,
    pub(crate) failed: u64,
    pub(crate) cancelled: u64,
    pub(crate) rejected: u64,
}

/// One processed line of a batch.
#[derive(Debug, Deserialize)]
pub(crate) struct BatchItem {
    pub(crate) line: u64,
    pub(crate) operation_id: Option<String>,
    pub(crate) status: String,
    pub(crate) code: Option<String>,
    pub(crate) rejection: Option<String>,
}
