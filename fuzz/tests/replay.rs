//! Stable replay of every fuzz target over its committed seed corpus.
//!
//! libFuzzer needs nightly and runs on a schedule; these tests run the same
//! target bodies over every seed on the repository's stable toolchain, and keep
//! the seeds honest: each is either a byte-for-byte copy of an existing test
//! fixture or an inline test document, or is re-derived here from one.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fmt::Write as _,
    fs,
    future::Future,
    io::{Cursor, Write as _},
    num::{NonZeroU16, NonZeroU32},
    path::{Path, PathBuf},
    pin::pin,
    task::{Context, Poll, Waker},
};

use flate2::{Compression, write::GzEncoder};
use lzma_rust2::{XzOptions, XzWriter};
use serde_json::Value;
use sha2::{Digest, Sha256};
use vsift_application::{
    AsrRevisionRequest, AsrTranscription, ExtendVisualIndexRequest, VisualIndexScope,
    VisualSampler, VisualSamplingError, build_asr_revision, extend_visual_index,
    whole_file_source_segment,
};
use vsift_contract::{
    BatchLine, BundleName, JsonLimits, MAX_BATCH_LINES, RelativeInputPath, SavedSetupPlan,
    WORK_REQUEST_LIMITS, decode_batch_line, decode_strict_json, decode_work_request,
};
use vsift_domain::{
    AsrChunkOutcome, AsrChunkRecord, AsrDecodingProfile, AsrModel, AsrModelProfile, AsrProvider,
    AsrProviderBuild, AsrRun, AsrRunParts, ChunkPlan, CropRect, CursorToken, FrameDimensions,
    JobId, LanguageTag, ListingTail, ManagedVersionKey, MediaTime, OperationId, SearchMatch,
    SearchQuery, SessionId, Sha256Hex, SourceId, TimeRange, VISUAL_BLOCKS, VISUAL_FRAME_BYTES,
    VisualHash, VisualIndexProfile, VisualSample, VisualWindow, merge_chunks, plan_chunks,
    validate_chunk_output,
};
use vsift_fuzz::{
    CROP_FRAME_HEIGHT, CROP_FRAME_WIDTH, EVIDENCE_FUZZ_SESSION, JOB_FUZZ_JOB, JOB_FUZZ_SESSION,
    REQUEST_FUZZ_OPERATION, Target, VISUAL_FUZZ_SESSION, bundle_manifest_input,
    bundle_manifest_is_accepted, bundle_manifest_refusal, png_sequence_input, visual_samples_input,
};
use vsift_infrastructure::{
    ArchiveInventoryBounds, BatchLine as FileLine, BatchLines, FrameListingWindow, MountDevice,
    OsReleaseProfile, SourceContainer, VisualSamplingWindow, WhisperOutputLimits,
    classify_mountinfo, classify_os_release, decode_chunk_checkpoint, decode_evidence_record,
    decode_job_record, decode_request_record, decode_transcript_record, decode_visual_index_record,
    encode_transcript_record, encode_visual_index_record, inspect_gzip_tar_inventory,
    inspect_tar_inventory, inspect_xz_tar_inventory, parse_ashowinfo_start, parse_cgroup_limit,
    parse_cpu_max, parse_ffprobe_metadata, parse_frame_listing, parse_frame_showinfo,
    parse_net_dev, parse_png_sequence, parse_proc_cgroup, parse_supplied_transcript,
    parse_visual_samples, parse_whisper_full_json,
};

type TestResult = Result<(), Box<dyn Error>>;

/// Where a committed seed came from.
enum Origin {
    /// A byte-for-byte copy of this repository file.
    Copy(&'static str),
    /// An inline test document that appears verbatim in this repository file.
    InlineIn(&'static str),
    /// The version-2 record `encode_transcript_record` writes for the local-ASR
    /// revision built from the recorded F01 whisper output (see
    /// [`the_local_asr_record_seed_is_the_encoded_f01_revision`]).
    EncodedF01LocalAsr,
    /// A visual-samples input whose diagnostics carry the recorded sample
    /// times of this fixture (see [`the_visual_seeds_derive_from_the_recorded_samples`]).
    RecordedShowinfo(&'static str),
    /// The visual-index record encoded from this fixture's recorded samples.
    RecordedVisualIndex(&'static str),
    /// A `png_sequence` input: this many copies of the recorded 80x80 F01
    /// crop PNG ([`RECORDED_CROP`]), expected as 80x80 images.
    RecordedCrops(u8),
    /// A `crop_rect` input: an outer rectangle that appears verbatim in the
    /// first file, a line feed, and an inner one that appears in the second.
    CropPair {
        outer: &'static str,
        outer_in: &'static str,
        inner: &'static str,
        inner_in: &'static str,
    },
    /// A `search_query` input: a query that appears verbatim in the first
    /// file, a line feed, and segment text that appears verbatim in the second.
    SearchPair {
        query: &'static str,
        query_in: &'static str,
        text: &'static str,
        text_in: &'static str,
    },
    /// A `bundle_manifest` input built here: the manifest `vsift session
    /// retain` writes for an evidence-only bundle (the shape of
    /// `p05_lifecycle.rs`'s own manifests), listing these artifacts as
    /// (kind, example file in `schemas/v1/examples/`), each followed by that
    /// example's bytes as its payload. See [`bundle_image`].
    BundleImage(&'static [(&'static str, &'static str)]),
    /// An archive built here the way the infrastructure crate's own archive
    /// tests build theirs (a `tar::Builder`, then gzip or xz with the
    /// workspace's pinned crates); the named source file holds that test's
    /// archive path. See [`archive_seed`].
    Archive {
        tree: ArchiveTree,
        format: ArchiveFormat,
        mirrors: &'static str,
    },
}

/// What an archive seed holds.
#[derive(Clone, Copy, Debug)]
enum ArchiveTree {
    /// `root/tool`, three bytes: the infrastructure tests' own archive.
    OneFile,
    /// A directory, a nested file and a licence file.
    Tree,
    /// A regular file and a symbolic link to it, which no reviewed alias
    /// allows.
    Link,
}

/// How an archive seed is wrapped.
#[derive(Clone, Copy, Debug)]
enum ArchiveFormat {
    Tar,
    Gzip,
    Xz,
}

struct Seed {
    target: Target,
    file: &'static str,
    origin: Origin,
}

const TRANSCRIPT_DATA: &str = "crates/vsift-infrastructure/tests/data/transcripts";
const WHISPER_FIXTURES: &str = "crates/vsift-infrastructure/tests/fixtures/whisper-1.9.2";
const VISUAL_SAMPLES: &str = "crates/vsift-infrastructure/tests/data/visual_samples";
/// Real `FFmpeg` 9.0 diagnostics of the P09 adapter calls (see
/// `crates/vsift-infrastructure/tests/p09_recorded_diagnostics.rs`).
const FFMPEG_DIAGNOSTICS: &str = "crates/vsift-infrastructure/tests/data/ffmpeg_diagnostics";
/// The recorded `crop_at` output the `png_sequence` seeds repeat.
const RECORDED_CROP: &str =
    "crates/vsift-infrastructure/tests/data/ffmpeg_diagnostics/F01-crop-560-320-80x80.png";
const RECORDED_CROP_SIZE: u16 = 80;

/// The domain file whose crop tests the `crop_rect` seeds quote.
const CROP_TESTS: &str = "crates/vsift-domain/src/timeline.rs";
/// The durable-profile tests whose mount tables and os-release files the
/// `mountinfo` and `os_release` seeds quote.
const MOUNTINFO_TESTS: &str = "crates/vsift-infrastructure/src/durable_profile.rs";
/// The strict-worker attestation tests whose kernel files the
/// `host_attestation` seeds quote (P11 PR 2).
const ATTESTATION_TESTS: &str = "crates/vsift-infrastructure/src/host_attestation.rs";

/// The committed example job records and checkpoints (P11 PR 1, issue #180),
/// pinned to the encoder by `vsift-infrastructure`'s `job_record_examples`.
const JOB_EXAMPLES: &str = "crates/vsift-infrastructure/tests/data/jobs";
/// The committed example transcript records of #353, pinned to the encoder by
/// `vsift-infrastructure`'s `local_asr_store`.
const TRANSCRIPT_RECORD_EXAMPLES: &str =
    "crates/vsift-infrastructure/tests/data/transcript_records";
/// The committed example worker request records (P11 PR 3), pinned to the
/// store's encoder and the contract's recorded form by `vsift`'s
/// `request_record_examples`.
const REQUEST_EXAMPLES: &str = "crates/vsift-infrastructure/tests/data/worker-requests";
/// The frozen worker batch whose lines the request seeds quote.
const BATCH_EXAMPLE: &str = "schemas/v1/examples/job-batch.requests.jsonl";

const SEEDS: &[Seed] = &[
    // The skill's own instructions: a report with one valid handoff block,
    // the REPORT skeleton, and every other kind of Markdown the skill uses.
    seed(
        Target::HandoffCheck,
        "SKILL.md",
        Origin::Copy("skills/vsift"),
    ),
    // The draft behind the frozen `handoff-check.json` example: one finding
    // of each kind.
    seed(
        Target::HandoffCheck,
        "draft-with-findings.md",
        Origin::Copy("crates/vsift-contract/tests/data/handoff"),
    ),
    seed(
        Target::JobRequest,
        "job-request.json",
        Origin::Copy("schemas/v1/examples"),
    ),
    seed(
        Target::JobRequest,
        "f10-ingest-line.json",
        Origin::InlineIn(BATCH_EXAMPLE),
    ),
    seed(
        Target::JobRequest,
        "f01-session-line.json",
        Origin::InlineIn(BATCH_EXAMPLE),
    ),
    seed(
        Target::JobBatchLine,
        "f10-ingest-line.jsonl",
        Origin::InlineIn(BATCH_EXAMPLE),
    ),
    seed(
        Target::JobBatchLine,
        "f01-session-line.jsonl",
        Origin::InlineIn(BATCH_EXAMPLE),
    ),
    seed(
        Target::JobBatchFile,
        "job-batch.requests.jsonl",
        Origin::Copy("schemas/v1/examples"),
    ),
    seed(
        Target::JobBatchFile,
        "job-batch.events.jsonl",
        Origin::Copy("schemas/v1/examples"),
    ),
    seed(
        Target::JobRecord,
        "job-record.succeeded.json",
        Origin::Copy(JOB_EXAMPLES),
    ),
    seed(
        Target::RequestRecord,
        "request-record.running.json",
        Origin::Copy(REQUEST_EXAMPLES),
    ),
    seed(
        Target::RequestRecord,
        "request-record.ended.json",
        Origin::Copy(REQUEST_EXAMPLES),
    ),
    seed(
        Target::JobRecord,
        "job-record.interrupted.json",
        Origin::Copy(JOB_EXAMPLES),
    ),
    seed(
        Target::ChunkCheckpoint,
        "checkpoint.recognised.json",
        Origin::Copy(JOB_EXAMPLES),
    ),
    seed(
        Target::ChunkCheckpoint,
        "checkpoint.silent.json",
        Origin::Copy(JOB_EXAMPLES),
    ),
    seed(
        Target::ChunkCheckpoint,
        "checkpoint.unusable.json",
        Origin::Copy(JOB_EXAMPLES),
    ),
    seed(
        Target::Mountinfo,
        "ext4-and-proc.txt",
        Origin::InlineIn(MOUNTINFO_TESTS),
    ),
    seed(
        Target::Mountinfo,
        "nobarrier-and-xfs.txt",
        Origin::InlineIn(MOUNTINFO_TESTS),
    ),
    seed(
        Target::Mountinfo,
        "missing-separator.txt",
        Origin::InlineIn(MOUNTINFO_TESTS),
    ),
    seed(
        Target::HostAttestation,
        "cgroup-v2.txt",
        Origin::InlineIn(ATTESTATION_TESTS),
    ),
    seed(
        Target::HostAttestation,
        "cpu-max.txt",
        Origin::InlineIn(ATTESTATION_TESTS),
    ),
    seed(
        Target::HostAttestation,
        "memory-max.txt",
        Origin::InlineIn(ATTESTATION_TESTS),
    ),
    seed(
        Target::HostAttestation,
        "net-dev-loopback.txt",
        Origin::InlineIn(ATTESTATION_TESTS),
    ),
    seed(
        Target::OsRelease,
        "noble.txt",
        Origin::InlineIn(MOUNTINFO_TESTS),
    ),
    seed(
        Target::OsRelease,
        "unquoted-space.txt",
        Origin::InlineIn(MOUNTINFO_TESTS),
    ),
    seed(
        Target::EvidenceRecord,
        "bundle-evidence-record.json",
        Origin::Copy("schemas/v1/examples"),
    ),
    seed(
        Target::CropRect,
        "whole-frame-then-corner.txt",
        Origin::CropPair {
            outer: "0,0,1440,900",
            outer_in: CROP_TESTS,
            inner: "1439,899,1,1",
            inner_in: CROP_TESTS,
        },
    ),
    seed(
        Target::CropRect,
        "g18-cell-then-empty.txt",
        Origin::CropPair {
            outer: "850,420,280,70",
            outer_in: CROP_TESTS,
            inner: "0,0,0,10",
            inner_in: CROP_TESTS,
        },
    ),
    seed(
        Target::CropRect,
        "leading-zero-then-outside.txt",
        Origin::CropPair {
            outer: "01,2,3,4",
            outer_in: CROP_TESTS,
            inner: "1,0,1440,900",
            inner_in: CROP_TESTS,
        },
    ),
    seed(
        Target::TranscriptSrt,
        "F10.srt",
        Origin::Copy("fixtures/corpus/transcripts/F10.srt"),
    ),
    seed(
        Target::TranscriptSrt,
        "invalid-timestamp.srt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptSrt,
        "markup.srt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptSrt,
        "out-of-order.srt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptSrt,
        "reversed-timing.srt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptSrt,
        "untimed-text.srt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptWebVtt,
        "F10.vtt",
        Origin::Copy("fixtures/corpus/transcripts/F10.vtt"),
    ),
    seed(
        Target::TranscriptWebVtt,
        "markup.vtt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptWebVtt,
        "missing-arrow.vtt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::TranscriptWebVtt,
        "overlapping.vtt",
        Origin::Copy(TRANSCRIPT_DATA),
    ),
    seed(
        Target::WhisperFullJson,
        "F01.base.json",
        Origin::Copy(WHISPER_FIXTURES),
    ),
    seed(
        Target::WhisperFullJson,
        "F05.base.json",
        Origin::Copy(WHISPER_FIXTURES),
    ),
    seed(
        Target::WhisperFullJson,
        "F08.base.json",
        Origin::Copy(WHISPER_FIXTURES),
    ),
    seed(
        Target::WhisperFullJson,
        "F09.base.json",
        Origin::Copy(WHISPER_FIXTURES),
    ),
    seed(
        Target::TranscriptRecord,
        "bundle-transcript-record.json",
        Origin::Copy("schemas/v1/examples"),
    ),
    seed(
        Target::TranscriptRecord,
        "F01-local-asr-v2.json",
        Origin::EncodedF01LocalAsr,
    ),
    seed(
        Target::TranscriptRecord,
        "bundle-transcript-record.asr.json",
        Origin::Copy("schemas/v1/examples"),
    ),
    seed(
        Target::TranscriptRecord,
        "F01-kept-in-an-unreadable-part.json",
        Origin::Copy(TRANSCRIPT_RECORD_EXAMPLES),
    ),
    seed(
        Target::FfprobeMetadata,
        "F11-excessive-streams.json",
        Origin::Copy("fixtures/corpus/generated"),
    ),
    seed(
        Target::FfprobeMetadata,
        "F11-oversized-dimensions.json",
        Origin::Copy("fixtures/corpus/generated"),
    ),
    seed(
        Target::FfprobeMetadata,
        "two-streams-rotated.json",
        Origin::InlineIn("crates/vsift-infrastructure/src/ffmpeg_media.rs"),
    ),
    seed(
        Target::FfprobeMetadata,
        "audio-only-unknown-codec.json",
        Origin::InlineIn("crates/vsift-infrastructure/src/ffmpeg_media.rs"),
    ),
    seed(
        Target::TranscriptCursor,
        "transcript-get-next-cursor.txt",
        Origin::InlineIn("schemas/v1/examples/transcript-get.json"),
    ),
    seed(
        Target::VisualSamples,
        "F01-showinfo.bin",
        Origin::RecordedShowinfo("F01"),
    ),
    seed(
        Target::VisualSamples,
        "F09-showinfo.bin",
        Origin::RecordedShowinfo("F09"),
    ),
    seed(
        Target::VisualIndexRecord,
        "F06-index.json",
        Origin::RecordedVisualIndex("F06"),
    ),
    seed(
        Target::VisualIndexRecord,
        "F10-index.json",
        Origin::RecordedVisualIndex("F10"),
    ),
    seed(
        Target::FrameShowinfo,
        "F01-frame-1025000.txt",
        Origin::Copy(FFMPEG_DIAGNOSTICS),
    ),
    seed(
        Target::FrameShowinfo,
        "F01-audio-only-wav-0-1000000.txt",
        Origin::Copy(FFMPEG_DIAGNOSTICS),
    ),
    seed(
        Target::FrameShowinfo,
        "forged-title-frame-250000.txt",
        Origin::Copy(FFMPEG_DIAGNOSTICS),
    ),
    seed(
        Target::FrameShowinfo,
        "forged-title-wav-0-1000000.txt",
        Origin::Copy(FFMPEG_DIAGNOSTICS),
    ),
    seed(
        Target::FrameListing,
        "F01-listing-900000-1200000.txt",
        Origin::Copy(FFMPEG_DIAGNOSTICS),
    ),
    seed(
        Target::FrameListing,
        "forged-title-listing-0-1000000.txt",
        Origin::Copy(FFMPEG_DIAGNOSTICS),
    ),
    seed(
        Target::PngSequence,
        "F01-crop-x1.bin",
        Origin::RecordedCrops(1),
    ),
    seed(
        Target::PngSequence,
        "F01-crop-x2.bin",
        Origin::RecordedCrops(2),
    ),
    seed(
        Target::SearchQuery,
        "f10-r-17.txt",
        Origin::SearchPair {
            query: "R-17",
            query_in: "fixtures/corpus/manifest.json",
            text: "Dialog R-17 is displayed now.",
            text_in: "fixtures/corpus/transcripts/F10.srt",
        },
    ),
    seed(
        Target::SearchQuery,
        "f10-dialog-r-17.txt",
        Origin::SearchPair {
            query: "dialog r 17",
            query_in: "crates/vsift-contract/tests/search_contract.rs",
            text: "Dialog R-17 is displayed now.",
            text_in: "fixtures/corpus/transcripts/F10.srt",
        },
    ),
    seed(
        Target::SearchQuery,
        "f01-build-2048.txt",
        Origin::SearchPair {
            query: "build 2,048",
            query_in: "crates/vsift-contract/tests/search_contract.rs",
            text: "The service status is healthy and the build is 2048.",
            text_in: "crates/vsift-infrastructure/tests/fixtures/whisper-1.9.2/F01.base.json",
        },
    ),
    // P14 PR 4. The saved plans of the frozen examples (`setup install --plan`).
    seed(
        Target::SetupPlan,
        "setup-plan.unavailable.json",
        Origin::Copy("schemas/v1/examples"),
    ),
    seed(
        Target::SetupPlan,
        "setup-plan.unqualified.json",
        Origin::Copy("schemas/v1/examples"),
    ),
    // Bundles: no artifact, each record the validation decodes, and both.
    seed(
        Target::BundleManifest,
        "evidence-only.bin",
        Origin::BundleImage(&[]),
    ),
    seed(
        Target::BundleManifest,
        "transcript-record.bin",
        Origin::BundleImage(&[("transcript_record", "bundle-transcript-record.json")]),
    ),
    seed(
        Target::BundleManifest,
        "visual-index-record.bin",
        Origin::BundleImage(&[("visual_index_record", "bundle-visual-index-record.json")]),
    ),
    seed(
        Target::BundleManifest,
        "transcript-and-visual-index.bin",
        Origin::BundleImage(&[
            ("transcript_record", "bundle-transcript-record.json"),
            ("visual_index_record", "bundle-visual-index-record.json"),
        ]),
    ),
    // Archives, in the three wrappers a reviewed artifact comes in.
    seed(
        Target::TarInventory,
        "one-file.tar",
        Origin::Archive {
            tree: ArchiveTree::OneFile,
            format: ArchiveFormat::Tar,
            mirrors: XZ_INVENTORY_TESTS,
        },
    ),
    seed(
        Target::TarInventory,
        "tree.tar",
        Origin::Archive {
            tree: ArchiveTree::Tree,
            format: ArchiveFormat::Tar,
            mirrors: XZ_INVENTORY_TESTS,
        },
    ),
    seed(
        Target::TarInventory,
        "link.tar",
        Origin::Archive {
            tree: ArchiveTree::Link,
            format: ArchiveFormat::Tar,
            mirrors: XZ_INVENTORY_TESTS,
        },
    ),
    seed(
        Target::GzipTarInventory,
        "one-file.tar.gz",
        Origin::Archive {
            tree: ArchiveTree::OneFile,
            format: ArchiveFormat::Gzip,
            mirrors: GZIP_INVENTORY_TESTS,
        },
    ),
    seed(
        Target::GzipTarInventory,
        "tree.tar.gz",
        Origin::Archive {
            tree: ArchiveTree::Tree,
            format: ArchiveFormat::Gzip,
            mirrors: GZIP_INVENTORY_TESTS,
        },
    ),
    seed(
        Target::XzTarInventory,
        "one-file.tar.xz",
        Origin::Archive {
            tree: ArchiveTree::OneFile,
            format: ArchiveFormat::Xz,
            mirrors: XZ_INVENTORY_TESTS,
        },
    ),
    seed(
        Target::XzTarInventory,
        "tree.tar.xz",
        Origin::Archive {
            tree: ArchiveTree::Tree,
            format: ArchiveFormat::Xz,
            mirrors: XZ_INVENTORY_TESTS,
        },
    ),
    // Identifiers that appear in the frozen examples and the catalogue.
    seed(
        Target::Identifiers,
        "session-id.txt",
        Origin::InlineIn("schemas/v1/examples/job-batch.requests.jsonl"),
    ),
    seed(
        Target::Identifiers,
        "operation-id.txt",
        Origin::InlineIn("schemas/v1/examples/job-batch.requests.jsonl"),
    ),
    seed(
        Target::Identifiers,
        "job-id.txt",
        Origin::InlineIn("schemas/v1/examples/job-batch.events.jsonl"),
    ),
    seed(
        Target::Identifiers,
        "source-id.txt",
        Origin::InlineIn("schemas/v1/examples/job-batch.events.jsonl"),
    ),
    seed(
        Target::Identifiers,
        "sha256.txt",
        Origin::InlineIn("schemas/v1/examples/job-batch.events.jsonl"),
    ),
    seed(
        Target::Identifiers,
        "visual-hash.txt",
        Origin::InlineIn("crates/vsift-infrastructure/tests/data/visual_samples/F01.json"),
    ),
    seed(
        Target::Identifiers,
        "managed-version-key.txt",
        Origin::InlineIn("crates/vsift-infrastructure/src/managed_catalogue.rs"),
    ),
    seed(
        Target::Identifiers,
        "language-tag.txt",
        Origin::InlineIn("schemas/v1/examples/transcript-get.asr.json"),
    ),
    seed(
        Target::Identifiers,
        "speaker-label.txt",
        Origin::InlineIn("crates/vsift-infrastructure/tests/data/transcripts/markup.vtt"),
    ),
    // Paths and bundle names: the runbook's own, and the spellings the
    // contained-input tests refuse.
    seed(
        Target::InputPath,
        "walkthrough.txt",
        Origin::InlineIn("docs/operations/worker-host.md"),
    ),
    seed(
        Target::InputPath,
        "bundle-name.txt",
        Origin::InlineIn("docs/operations/worker-host.md"),
    ),
    seed(
        Target::InputPath,
        "outside-secret.txt",
        Origin::InlineIn("crates/vsift-infrastructure/tests/contained_inputs.rs"),
    ),
    seed(
        Target::InputPath,
        "climb-then-leave.txt",
        Origin::InlineIn("crates/vsift-infrastructure/tests/contained_inputs.rs"),
    ),
    seed(
        Target::InputPath,
        "drive.txt",
        Origin::InlineIn("crates/vsift-infrastructure/tests/contained_inputs.rs"),
    ),
    seed(
        Target::InputPath,
        "device-in-directory.txt",
        Origin::InlineIn("crates/vsift-infrastructure/tests/contained_inputs.rs"),
    ),
];

/// The tests of the two compressed-archive readers, whose archives the
/// archive seeds mirror (`root/tool`, three bytes).
const XZ_INVENTORY_TESTS: &str = "crates/vsift-infrastructure/src/xz_tar_inventory.rs";
const GZIP_INVENTORY_TESTS: &str = "crates/vsift-infrastructure/src/gzip_tar_inventory.rs";

const fn seed(target: Target, file: &'static str, origin: Origin) -> Seed {
    Seed {
        target,
        file,
        origin,
    }
}

fn fuzz_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repository(relative: &str) -> PathBuf {
    fuzz_root().join("..").join(relative)
}

fn seed_directory(target: Target) -> PathBuf {
    fuzz_root().join("seeds").join(target.name())
}

fn seed_files(target: Target) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(seed_directory(target))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(format!("{} holds a non-file entry", target.name()).into());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "non-UTF-8 seed name")?;
        names.insert(name);
    }
    Ok(names)
}

/// Every target runs over every committed seed, and over empty input, without
/// a violation (a panic inside a parser fails the test too).
#[test]
fn every_seed_replays_without_a_violation() -> TestResult {
    for target in Target::ALL {
        target.check(&[])?;
        let names = seed_files(target)?;
        assert!(!names.is_empty(), "{} has no seeds", target.name());
        for name in names {
            let data = fs::read(seed_directory(target).join(&name))?;
            if let Err(violation) = target.check(&data) {
                return Err(format!("{}/{name}: {violation}", target.name()).into());
            }
        }
    }
    Ok(())
}

/// The well-formed seeds are accepted, so the targets' invariants are exercised
/// from the first run rather than only once the fuzzer finds a valid input.
#[test]
fn well_formed_seeds_are_accepted() -> TestResult {
    let accepted = [
        (Target::TranscriptSrt, "F10.srt"),
        (Target::TranscriptSrt, "markup.srt"),
        (Target::TranscriptWebVtt, "F10.vtt"),
        (Target::TranscriptWebVtt, "markup.vtt"),
        (Target::TranscriptWebVtt, "overlapping.vtt"),
        (Target::WhisperFullJson, "F01.base.json"),
        (Target::WhisperFullJson, "F05.base.json"),
        (Target::WhisperFullJson, "F08.base.json"),
        (Target::WhisperFullJson, "F09.base.json"),
        (Target::TranscriptRecord, "bundle-transcript-record.json"),
        (Target::TranscriptRecord, "F01-local-asr-v2.json"),
        (
            Target::TranscriptRecord,
            "F01-kept-in-an-unreadable-part.json",
        ),
        (Target::FfprobeMetadata, "two-streams-rotated.json"),
        (Target::FfprobeMetadata, "audio-only-unknown-codec.json"),
        (Target::TranscriptCursor, "transcript-get-next-cursor.txt"),
        (Target::VisualSamples, "F01-showinfo.bin"),
        (Target::VisualSamples, "F09-showinfo.bin"),
        (Target::VisualIndexRecord, "F06-index.json"),
        (Target::VisualIndexRecord, "F10-index.json"),
        (Target::SearchQuery, "f10-r-17.txt"),
        (Target::SearchQuery, "f10-dialog-r-17.txt"),
        (Target::SearchQuery, "f01-build-2048.txt"),
        (Target::FrameShowinfo, "F01-frame-1025000.txt"),
        (Target::FrameShowinfo, "F01-audio-only-wav-0-1000000.txt"),
        (Target::FrameShowinfo, "forged-title-frame-250000.txt"),
        (Target::FrameShowinfo, "forged-title-wav-0-1000000.txt"),
        (Target::FrameListing, "F01-listing-900000-1200000.txt"),
        (Target::FrameListing, "forged-title-listing-0-1000000.txt"),
        (Target::PngSequence, "F01-crop-x1.bin"),
        (Target::PngSequence, "F01-crop-x2.bin"),
        (Target::EvidenceRecord, "bundle-evidence-record.json"),
        (Target::CropRect, "whole-frame-then-corner.txt"),
        (Target::Mountinfo, "ext4-and-proc.txt"),
        (Target::Mountinfo, "nobarrier-and-xfs.txt"),
        (Target::OsRelease, "noble.txt"),
        (Target::HostAttestation, "cgroup-v2.txt"),
        (Target::HostAttestation, "cpu-max.txt"),
        (Target::HostAttestation, "memory-max.txt"),
        (Target::HostAttestation, "net-dev-loopback.txt"),
        (Target::JobRequest, "job-request.json"),
        (Target::JobRequest, "f10-ingest-line.json"),
        (Target::JobRequest, "f01-session-line.json"),
        (Target::JobBatchLine, "f10-ingest-line.jsonl"),
        (Target::JobBatchLine, "f01-session-line.jsonl"),
        (Target::JobBatchFile, "job-batch.requests.jsonl"),
        (Target::JobBatchFile, "job-batch.events.jsonl"),
        (Target::JobRecord, "job-record.succeeded.json"),
        (Target::JobRecord, "job-record.interrupted.json"),
        (Target::RequestRecord, "request-record.running.json"),
        (Target::RequestRecord, "request-record.ended.json"),
        (Target::ChunkCheckpoint, "checkpoint.recognised.json"),
        (Target::ChunkCheckpoint, "checkpoint.silent.json"),
        (Target::ChunkCheckpoint, "checkpoint.unusable.json"),
        (Target::HandoffCheck, "SKILL.md"),
        (Target::HandoffCheck, "draft-with-findings.md"),
        (Target::SetupPlan, "setup-plan.unavailable.json"),
        (Target::BundleManifest, "evidence-only.bin"),
        (Target::BundleManifest, "transcript-record.bin"),
        (Target::BundleManifest, "visual-index-record.bin"),
        (Target::TarInventory, "one-file.tar"),
        (Target::TarInventory, "tree.tar"),
        (Target::GzipTarInventory, "one-file.tar.gz"),
        (Target::GzipTarInventory, "tree.tar.gz"),
        (Target::XzTarInventory, "one-file.tar.xz"),
        (Target::XzTarInventory, "tree.tar.xz"),
        (Target::Identifiers, "session-id.txt"),
        (Target::Identifiers, "operation-id.txt"),
        (Target::Identifiers, "job-id.txt"),
        (Target::Identifiers, "source-id.txt"),
        (Target::Identifiers, "sha256.txt"),
        (Target::Identifiers, "visual-hash.txt"),
        (Target::Identifiers, "managed-version-key.txt"),
        (Target::Identifiers, "language-tag.txt"),
        (Target::Identifiers, "speaker-label.txt"),
        (Target::InputPath, "walkthrough.txt"),
        (Target::InputPath, "bundle-name.txt"),
    ];
    for (target, file) in accepted {
        let data = fs::read(seed_directory(target).join(file))?;
        let is_accepted = is_accepted(target, &data)?;
        assert!(
            is_accepted,
            "{}/{file} is rejected ({})",
            target.name(),
            bundle_manifest_refusal(&data).unwrap_or_default()
        );
    }
    Ok(())
}

/// The batch file target holds the reader to its model at the edges the
/// small limits (4 lines of at most 16 bytes) put within the fuzzer's
/// reach: no line, a final line without a line feed, carriage returns, a
/// line exactly at and one byte over the bound, and one line too many.
#[test]
fn the_batch_file_target_holds_the_reader_to_its_edges() -> TestResult {
    let long = "x".repeat(17);
    let edges = [
        String::new(),
        "\n".to_owned(),
        "a".to_owned(),
        "a\r\n\r\n".to_owned(),
        "\n\n\n\n".to_owned(),
        "\n\n\n\n\n".to_owned(),
        format!("{}\n{long}\nz", "y".repeat(16)),
        format!("{long}{long}{long}"),
        "1\n2\n3\n4\n5".to_owned(),
        "{\"schema_version\":\"1\"}\n\u{feff}\n".to_owned(),
    ];
    for edge in edges {
        Target::JobBatchFile.check(edge.as_bytes())?;
    }
    Ok(())
}

/// A batch file seed is within the production limits, and every line of it
/// is handed out whole.
fn batch_file_is_read_whole(data: &[u8]) -> Result<bool, Box<dyn Error>> {
    let mut lines = BatchLines::scan(
        Cursor::new(data),
        u32::try_from(MAX_BATCH_LINES)?,
        WORK_REQUEST_LIMITS.max_bytes,
    )?;
    let mut whole = true;
    while let Some(line) = lines.next_line()? {
        whole &= matches!(line, FileLine::Line { .. });
    }
    Ok(whole && lines.lines() > 0)
}

/// Whether `target`'s parser accepts a well-formed seed.
#[allow(
    clippy::too_many_lines,
    reason = "One arm for each fuzz target keeps every parser's acceptance test in one place"
)]
fn is_accepted(target: Target, data: &[u8]) -> Result<bool, Box<dyn Error>> {
    Ok(match target {
        Target::TranscriptSrt | Target::TranscriptWebVtt => parse_supplied_transcript(data).is_ok(),
        Target::WhisperFullJson => parse_whisper_full_json(data, WhisperOutputLimits::R0).is_ok(),
        Target::TranscriptRecord => decode_transcript_record(data).is_ok(),
        Target::FfprobeMetadata => parse_ffprobe_metadata(data, SourceContainer::IsoMedia).is_ok(),
        Target::TranscriptCursor => CursorToken::parse(std::str::from_utf8(data)?).is_ok(),
        Target::VisualSamples => match data {
            [count, _, stderr @ ..] => parse_visual_samples(
                &vec![0_u8; usize::from(*count) * VISUAL_FRAME_BYTES],
                stderr,
                &VisualSamplingWindow::new(
                    0,
                    0,
                    MediaTime::from_micros(0),
                    MediaTime::from_micros(60_000_000),
                )?,
            )
            .is_ok_and(|frames| frames.len() == usize::from(*count)),
            _ => false,
        },
        Target::VisualIndexRecord => {
            decode_visual_index_record(data, &SessionId::parse(VISUAL_FUZZ_SESSION)?).is_ok()
        }
        Target::SearchQuery => {
            let (query, text) = std::str::from_utf8(data)?
                .split_once('\n')
                .ok_or("no line feed")?;
            // Each seed matches: two as a phrase, one as all terms.
            SearchQuery::parse(query)
                .ok()
                .and_then(|query| query.classify(text))
                .is_some_and(|tier| SearchMatch::ALL.contains(&tier))
        }
        // Each diagnostics seed is accepted by the parser of its kind.
        Target::FrameShowinfo => {
            parse_frame_showinfo(data).is_ok() || parse_ashowinfo_start(data).is_ok()
        }
        Target::FrameListing => parse_frame_listing(
            data,
            &FrameListingWindow::new(
                0,
                1,
                10_240,
                0,
                TimeRange::new(
                    MediaTime::from_micros(0),
                    MediaTime::from_micros(60_000_000),
                )?,
                ListingTail::EndOfStream,
            )?,
        )
        .is_ok_and(|listing| !listing.frames().is_empty()),
        Target::PngSequence => match data {
            [count, _, _, _, _, stdout @ ..] => parse_png_sequence(
                stdout,
                usize::from(*count),
                FrameDimensions::new(u32::from(RECORDED_CROP_SIZE), u32::from(RECORDED_CROP_SIZE))?,
            )
            .is_ok(),
            _ => false,
        },
        Target::EvidenceRecord => {
            decode_evidence_record(data, &SessionId::parse(EVIDENCE_FUZZ_SESSION)?).is_ok()
        }
        Target::Mountinfo => classify_mountinfo(data, MountDevice::new(8, 1)).is_ok(),
        Target::OsRelease => classify_os_release(data) == Ok(OsReleaseProfile::Ubuntu2404),
        Target::JobRequest => decode_work_request(data).is_ok(),
        Target::JobBatchLine => matches!(decode_batch_line(data), Ok(BatchLine::Request(_))),
        Target::JobBatchFile => batch_file_is_read_whole(data)?,
        // Each seed has one readable handoff block.
        Target::HandoffCheck => {
            vsift_contract::extract_handoff_block(std::str::from_utf8(data)?).is_ok()
        }
        Target::JobRecord => decode_job_record(
            data,
            &JobId::parse(JOB_FUZZ_JOB)?,
            &SessionId::parse(JOB_FUZZ_SESSION)?,
        )
        .is_ok(),
        Target::ChunkCheckpoint => decode_chunk_checkpoint(data, 1).is_some(),
        Target::RequestRecord => decode_request_record(
            data,
            &vsift_domain::OperationId::parse(REQUEST_FUZZ_OPERATION)?,
        )
        .is_ok(),
        // Each seed is one kernel file that its own parser accepts.
        Target::HostAttestation => {
            parse_proc_cgroup(data).is_ok()
                || parse_cpu_max(data).is_ok()
                || parse_cgroup_limit(data).is_ok()
                || parse_net_dev(data).is_ok()
        }
        // The P14 PR 4 targets are read by their own function.
        Target::SetupPlan
        | Target::BundleManifest
        | Target::TarInventory
        | Target::GzipTarInventory
        | Target::XzTarInventory
        | Target::Identifiers
        | Target::InputPath => is_accepted_in_p14(target, data)?,
        // The outer and the inner rectangle are both accepted.
        Target::CropRect => {
            let (outer, inner) = std::str::from_utf8(data)?
                .split_once('\n')
                .ok_or("no line feed")?;
            let frame = FrameDimensions::new(CROP_FRAME_WIDTH, CROP_FRAME_HEIGHT)?;
            CropRect::parse(outer, frame).is_ok_and(|outer| {
                CropRect::parse(inner, outer.dimensions())
                    .is_ok_and(|inner| outer.compose(inner).is_ok())
            })
        }
    })
}

/// Whether a P14 PR 4 target's parser accepts a well-formed seed.
fn is_accepted_in_p14(target: Target, data: &[u8]) -> Result<bool, Box<dyn Error>> {
    Ok(match target {
        // The saved plan is one `setup install` would accept.
        Target::SetupPlan => decode_strict_json::<SavedSetupPlan>(data, JsonLimits::DOCUMENT)
            .is_ok_and(|plan| plan.validate_envelope().is_ok()),
        // Where the check cannot run (not Unix) it accepts nothing, so only
        // Unix holds the seeds to it.
        Target::BundleManifest => !cfg!(unix) || bundle_manifest_is_accepted(data),
        Target::TarInventory => inspect_tar_inventory(
            Cursor::new(data),
            SEED_STREAM_BYTES,
            seed_archive_bounds()?,
            &[],
        )
        .is_ok(),
        Target::GzipTarInventory => inspect_gzip_tar_inventory(
            Cursor::new(data),
            SEED_COMPRESSED_BYTES,
            SEED_STREAM_BYTES,
            seed_archive_bounds()?,
            &[],
        )
        .is_ok(),
        Target::XzTarInventory => inspect_xz_tar_inventory(
            Cursor::new(data),
            SEED_COMPRESSED_BYTES,
            SEED_STREAM_BYTES,
            seed_archive_bounds()?,
            &[],
        )
        .is_ok(),
        // Each seed is the text of one identifier or key.
        Target::Identifiers => {
            let text = std::str::from_utf8(data)?;
            SessionId::parse(text).is_ok()
                || JobId::parse(text).is_ok()
                || OperationId::parse(text).is_ok()
                || SourceId::parse(text).is_ok()
                || Sha256Hex::parse(text).is_ok()
                || VisualHash::parse_hex(text).is_ok()
                || ManagedVersionKey::parse(text).is_ok()
                || LanguageTag::parse(text).is_ok()
                || vsift_domain::SpeakerLabel::parse(text).is_ok()
        }
        Target::InputPath => {
            let text = std::str::from_utf8(data)?;
            RelativeInputPath::parse(text).is_ok() || BundleName::parse(text).is_ok()
        }
        _ => false,
    })
}

/// The seed directories hold exactly the listed seeds, and each listed copy or
/// inline document still matches the fixture it was taken from.
#[test]
fn seeds_are_listed_and_match_their_fixtures() -> TestResult {
    for target in Target::ALL {
        let listed: BTreeSet<String> = SEEDS
            .iter()
            .filter(|seed| seed.target == target)
            .map(|seed| seed.file.to_owned())
            .collect();
        assert!(
            listed == seed_files(target)?,
            "{} seeds differ from the list",
            target.name()
        );
    }
    for seed in SEEDS {
        let data = fs::read(seed_directory(seed.target).join(seed.file))?;
        let matches = match seed.origin {
            Origin::Copy(path) => {
                let path = repository(path);
                let source = if path.is_dir() {
                    path.join(seed.file)
                } else {
                    path
                };
                fs::read(source)? == data
            }
            Origin::InlineIn(path) => {
                fs::read_to_string(repository(path))?.contains(std::str::from_utf8(&data)?)
            }
            Origin::EncodedF01LocalAsr
            | Origin::RecordedShowinfo(_)
            | Origin::RecordedVisualIndex(_) => true,
            Origin::RecordedCrops(count) => data == crops_seed(count)?,
            Origin::CropPair {
                outer,
                outer_in,
                inner,
                inner_in,
            } => {
                data == format!("{outer}\n{inner}").as_bytes()
                    && fs::read_to_string(repository(outer_in))?.contains(outer)
                    && fs::read_to_string(repository(inner_in))?.contains(inner)
            }
            Origin::SearchPair {
                query,
                query_in,
                text,
                text_in,
            } => {
                data == format!("{query}\n{text}").as_bytes()
                    && fs::read_to_string(repository(query_in))?.contains(query)
                    && fs::read_to_string(repository(text_in))?.contains(text)
            }
            Origin::BundleImage(artifacts) => data == bundle_image(artifacts)?,
            Origin::Archive {
                tree,
                format,
                mirrors,
            } => {
                data == archive_seed(tree, format)?
                    && fs::read_to_string(repository(mirrors))?.contains("root/tool")
            }
        };
        assert!(matches, "{} no longer matches its origin", seed.file);
    }
    Ok(())
}

/// The version-2 record seed is what the encoder writes for the local-ASR
/// revision built from the recorded F01 output, as in the infrastructure
/// crate's own round-trip test, so the fuzzer starts from a valid record.
#[test]
fn the_local_asr_record_seed_is_the_encoded_f01_revision() -> TestResult {
    let source_id =
        SourceId::from_sha256("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")?;
    let source = whole_file_source_segment(&source_id, MediaTime::from_micros(6_000_000))?;
    let planned = plan_chunks(source.id(), source.range(), ChunkPlan::R0)?;
    let first = planned.first().ok_or("no planned chunk")?;
    let audio = TimeRange::new(MediaTime::from_micros(0), MediaTime::from_micros(6_000_000))?;
    let output = parse_whisper_full_json(
        &fs::read(repository(WHISPER_FIXTURES).join("F01.base.json"))?,
        WhisperOutputLimits::R0,
    )?;
    let validated = validate_chunk_output(first, audio, source.range(), output)?;
    let (segments, language, warnings) = validated.into_parts();
    let merged = merge_chunks(&[segments]);
    let digest =
        Sha256Hex::parse("95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d")?;
    let run = AsrRun::new(AsrRunParts {
        provider: AsrProviderBuild::new(AsrProvider::WhisperCpp, digest.clone()),
        model: AsrModel::new(AsrModelProfile::Base, digest),
        decoding: AsrDecodingProfile::R0V1,
        plan: ChunkPlan::R0,
        threads: NonZeroU16::new(4).ok_or("zero threads")?,
        audio_stream: 1,
        chunks: vec![AsrChunkRecord::new(
            first.clone(),
            AsrChunkOutcome::Transcribed { audio },
        )],
    })?;
    let revision = build_asr_revision(AsrRevisionRequest {
        session_id: &SessionId::parse("ses_0123456789abcdef")?,
        source_id: &source_id,
        source_segment: &source,
        number: NonZeroU32::MIN,
        transcription: AsrTranscription {
            run,
            language,
            segments: merged.segments,
            warnings,
        },
        splice: None,
    })?;
    let encoded = encode_transcript_record(&revision)?;
    let committed =
        fs::read(seed_directory(Target::TranscriptRecord).join("F01-local-asr-v2.json"))?;
    assert!(
        committed == encoded,
        "regenerate the seed: the encoder no longer writes the committed bytes"
    );
    Ok(())
}

/// A `png_sequence` seed: `count` copies of the recorded crop.
fn crops_seed(count: u8) -> Result<Vec<u8>, Box<dyn Error>> {
    let png = fs::read(repository(RECORDED_CROP))?;
    Ok(png_sequence_input(
        count,
        RECORDED_CROP_SIZE,
        RECORDED_CROP_SIZE,
        &png.repeat(usize::from(count)),
    ))
}

/// Each target has a libFuzzer entry point of the same name.
#[test]
fn every_target_has_a_libfuzzer_entry_point() -> TestResult {
    let manifest = fs::read_to_string(fuzz_root().join("Cargo.toml"))?;
    for target in Target::ALL {
        let entry = Path::new("fuzz_targets").join(format!("{}.rs", target.name()));
        assert!(
            fuzz_root().join(&entry).is_file(),
            "{} is missing",
            entry.display()
        );
        assert!(
            manifest.contains(&format!("name = \"{}\"", target.name())),
            "{} has no [[bin]]",
            target.name()
        );
    }
    Ok(())
}

fn recorded_samples(fixture: &str) -> Result<Vec<VisualSample>, Box<dyn Error>> {
    let value: Value = serde_json::from_slice(&fs::read(
        repository(VISUAL_SAMPLES).join(format!("{fixture}.json")),
    )?)?;
    let mut samples = Vec::new();
    for window in value["windows"].as_array().ok_or("no windows")? {
        for sample in window["samples"].as_array().ok_or("no samples")? {
            let text = sample["blocks"].as_str().ok_or("no blocks")?;
            let mut blocks = [0_u8; VISUAL_BLOCKS];
            for (position, block) in blocks.iter_mut().enumerate() {
                *block = u8::from_str_radix(
                    text.get(position * 2..position * 2 + 2)
                        .ok_or("short blocks")?,
                    16,
                )?;
            }
            samples.push(VisualSample::from_parts(
                MediaTime::from_micros(sample["time_us"].as_u64().ok_or("no time")?),
                blocks,
                VisualHash::parse_hex(sample["hash"].as_str().ok_or("no hash")?)?,
            ));
        }
    }
    Ok(samples)
}

/// `showinfo` diagnostics for the recorded sample times, in `FFmpeg` 9.0's
/// line format, with the fixture's stream time base (Matroska's 1/1000 for
/// F09, the MP4 fixtures' 1/10240 otherwise).
fn showinfo_seed(fixture: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let samples = recorded_samples(fixture)?;
    let denominator: u64 = if fixture == "F09" { 1_000 } else { 10_240 };
    let mut text = String::new();
    writeln!(
        text,
        "[Parsed_showinfo_3 @ 0000000000000000] config in time_base: 1/{denominator}, frame_rate: 20/1"
    )?;
    writeln!(
        text,
        "[Parsed_showinfo_3 @ 0000000000000000] config out time_base: 0/0, frame_rate: 0/0"
    )?;
    for (number, sample) in samples.iter().enumerate() {
        let micros = sample.time().as_micros();
        let pts = micros * denominator / 1_000_000;
        writeln!(
            text,
            "[Parsed_showinfo_3 @ 0000000000000000] n:{number:>4} pts:{pts:>7} pts_time:{}.{:06} fmt:gray s:128x72 i:P iskey:0 type:P",
            micros / 1_000_000,
            micros % 1_000_000
        )?;
    }
    Ok(visual_samples_input(
        u8::try_from(samples.len())?,
        0x40,
        text.as_bytes(),
    ))
}

/// Replays recorded samples; every window of a short fixture is window 0.
struct Recorded(Vec<VisualSample>);

impl VisualSampler for Recorded {
    fn window_samples(
        &self,
        _window: VisualWindow,
    ) -> impl Future<Output = Result<Vec<VisualSample>, VisualSamplingError>> + Send {
        std::future::ready(Ok(self.0.clone()))
    }
}

/// Completes a future that never waits, such as an extension over a
/// sampler whose answers are ready, without an async runtime.
fn ready<F: Future>(future: F) -> Result<F::Output, Box<dyn Error>> {
    let mut context = Context::from_waker(Waker::noop());
    match pin!(future).poll(&mut context) {
        Poll::Ready(output) => Ok(output),
        Poll::Pending => Err("the future waited".into()),
    }
}

fn visual_index_seed(fixture: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let samples = recorded_samples(fixture)?;
    let value: Value = serde_json::from_slice(&fs::read(
        repository(VISUAL_SAMPLES).join(format!("{fixture}.json")),
    )?)?;
    let duration = MediaTime::from_micros(value["duration_us"].as_u64().ok_or("no duration")?);
    let session = SessionId::parse(VISUAL_FUZZ_SESSION)?;
    let source = SourceId::from_sha256(value["source_sha256"].as_str().ok_or("no source")?)?;
    let extension = ready(extend_visual_index(
        ExtendVisualIndexRequest {
            scope: VisualIndexScope {
                session_id: &session,
                source_id: &source,
                stream_index: 0,
                // F06 and F10 are 1280x720 in the corpus manifest.
                displayed_dimensions: FrameDimensions::new(1280, 720)?,
                duration,
                profile: VisualIndexProfile::R0,
            },
            previous: None,
            range: TimeRange::new(MediaTime::from_micros(0), duration)?,
        },
        &Recorded(samples),
    ))??;
    let index = extension.revision.ok_or("no revision")?;
    Ok(encode_visual_index_record(&session, &index)?)
}

/// The visual seeds are re-derived from the recorded samples of
/// `crates/vsift-infrastructure/tests/data/visual_samples/`. After a
/// re-recording or an encoder change, run this test with
/// `VSIFT_REGENERATE_FUZZ_SEEDS=1` to rewrite them, and review the diff.
#[test]
fn the_visual_seeds_derive_from_the_recorded_samples() -> TestResult {
    let regenerate = env::var("VSIFT_REGENERATE_FUZZ_SEEDS").is_ok_and(|value| value == "1");
    for seed in SEEDS {
        let derived = match seed.origin {
            Origin::RecordedShowinfo(fixture) => showinfo_seed(fixture)?,
            Origin::RecordedVisualIndex(fixture) => visual_index_seed(fixture)?,
            Origin::Copy(_)
            | Origin::InlineIn(_)
            | Origin::EncodedF01LocalAsr
            | Origin::RecordedCrops(_)
            | Origin::CropPair { .. }
            | Origin::SearchPair { .. }
            | Origin::BundleImage(_)
            | Origin::Archive { .. } => continue,
        };
        let path = seed_directory(seed.target).join(seed.file);
        if regenerate {
            fs::create_dir_all(seed_directory(seed.target))?;
            fs::write(&path, &derived)?;
        }
        assert!(
            fs::read(&path)? == derived,
            "{} no longer derives from the recorded samples",
            seed.file
        );
    }
    Ok(())
}

/// The session every bundle seed names: that of the visual-index example,
/// whose record is decoded for it.
const BUNDLE_SESSION: &str = "ses_0000000000000000visual";
/// The source a bundle seed with no artifact names: that of the visual-index
/// example. A bundle with artifacts names the source of its first one, since a
/// record must describe the bundle's source.
const BUNDLE_SOURCE: &str =
    "src_sha256_aec1a03817bbe77366bd29ca7b8254537de50b6f941b0b262fadd82cb652ea60";

/// Lower-case hexadecimal digits.
fn hex_digits(bytes: &[u8]) -> Result<String, Box<dyn Error>> {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(text, "{byte:02x}")?;
    }
    Ok(text)
}

/// A `bundle_manifest` seed: the evidence-only manifest listing `artifacts`
/// as (kind, example file), each artifact named by the digest of the
/// example's bytes as the store names its files, then those bytes.
fn bundle_image(artifacts: &[(&str, &str)]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut listed = Vec::new();
    let mut payloads = Vec::new();
    let mut source = BUNDLE_SOURCE.to_owned();
    for (kind, example) in artifacts {
        let payload = fs::read(repository("schemas/v1/examples").join(example))?;
        if payloads.is_empty() {
            let record: Value = serde_json::from_slice(&payload)?;
            let named = record["source_id"]
                .as_str()
                .ok_or("the first example names no source")?;
            named.clone_into(&mut source);
        }
        let digest = hex_digits(&Sha256::digest(&payload))?;
        listed.push(serde_json::json!({
            "kind": kind,
            "name": format!("artifact-{digest}.json"),
            "sha256": digest,
            "bytes": payload.len(),
        }));
        payloads.push(payload);
    }
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "format": "vsift.bundle",
        "session_id": BUNDLE_SESSION,
        "source_id": source,
        "source_bytes": 1_234_567,
        "source_included": false,
        "publication": "process_crash_consistent",
        "artifacts": listed,
    }))?;
    let borrowed: Vec<&[u8]> = payloads.iter().map(Vec::as_slice).collect();
    Ok(bundle_manifest_input(&manifest, &borrowed))
}

/// Adds one regular file of `content` named `path`.
fn append_file(
    builder: &mut tar::Builder<Vec<u8>>,
    path: &str,
    content: &[u8],
) -> Result<(), Box<dyn Error>> {
    let mut header = tar::Header::new_gnu();
    header.set_path(path)?;
    header.set_size(u64::try_from(content.len())?);
    header.set_mode(0o600);
    header.set_cksum();
    builder.append(&header, content)?;
    Ok(())
}

/// A tar of one of the shapes the archive seeds hold.
fn tar_seed(tree: ArchiveTree) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut builder = tar::Builder::new(Vec::new());
    match tree {
        ArchiveTree::OneFile => append_file(&mut builder, "root/tool", b"abc")?,
        ArchiveTree::Tree => {
            let mut directory = tar::Header::new_gnu();
            directory.set_path("root/")?;
            directory.set_entry_type(tar::EntryType::Directory);
            directory.set_size(0);
            directory.set_mode(0o700);
            directory.set_cksum();
            builder.append(&directory, std::io::empty())?;
            append_file(&mut builder, "root/bin/tool", b"abc")?;
            append_file(&mut builder, "root/LICENSE.txt", b"A synthetic licence.\n")?;
        }
        ArchiveTree::Link => {
            append_file(&mut builder, "root/lib.so.1", b"abc")?;
            let mut link = tar::Header::new_gnu();
            link.set_path("root/lib.so")?;
            link.set_entry_type(tar::EntryType::Symlink);
            link.set_link_name("lib.so.1")?;
            link.set_size(0);
            link.set_mode(0o777);
            link.set_cksum();
            builder.append(&link, std::io::empty())?;
        }
    }
    Ok(builder.into_inner()?)
}

/// An archive seed: the tar, wrapped as the format says.
fn archive_seed(tree: ArchiveTree, format: ArchiveFormat) -> Result<Vec<u8>, Box<dyn Error>> {
    let tar = tar_seed(tree)?;
    match format {
        ArchiveFormat::Tar => Ok(tar),
        ArchiveFormat::Gzip => {
            let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
            gzip.write_all(&tar)?;
            Ok(gzip.finish()?)
        }
        ArchiveFormat::Xz => {
            let mut xz = XzWriter::new(Vec::new(), XzOptions::with_preset(1))?;
            xz.write_all(&tar)?;
            Ok(xz.finish()?)
        }
    }
}

/// The budget the well-formed archive seeds are accepted under: the
/// fuzzer's own small one.
const SEED_STREAM_BYTES: u64 = 8_192;
const SEED_COMPRESSED_BYTES: u64 = 65_536;

fn seed_archive_bounds() -> Result<ArchiveInventoryBounds, Box<dyn Error>> {
    Ok(ArchiveInventoryBounds::new(8, 4_096)?)
}

/// The bundle and archive seeds are rebuilt by this test from the examples
/// and the archive crates. After an encoder or example change, run it with
/// `VSIFT_REGENERATE_FUZZ_SEEDS=1` to rewrite them, and review the diff.
#[test]
fn the_derived_seeds_are_rebuilt_by_the_harness() -> TestResult {
    let regenerate = env::var("VSIFT_REGENERATE_FUZZ_SEEDS").is_ok_and(|value| value == "1");
    for seed in SEEDS {
        let derived = match seed.origin {
            Origin::BundleImage(artifacts) => bundle_image(artifacts)?,
            Origin::Archive { tree, format, .. } => archive_seed(tree, format)?,
            Origin::Copy(_)
            | Origin::InlineIn(_)
            | Origin::EncodedF01LocalAsr
            | Origin::RecordedShowinfo(_)
            | Origin::RecordedVisualIndex(_)
            | Origin::RecordedCrops(_)
            | Origin::CropPair { .. }
            | Origin::SearchPair { .. } => continue,
        };
        let path = seed_directory(seed.target).join(seed.file);
        if regenerate {
            fs::create_dir_all(seed_directory(seed.target))?;
            fs::write(&path, &derived)?;
        }
        assert!(
            fs::read(&path)? == derived,
            "{} no longer derives from the examples and archive crates",
            seed.file
        );
    }
    Ok(())
}

/// The `Fuzz` workflow's matrix names every target, so a new target cannot
/// be left out of the weekly and the long campaigns.
#[test]
fn the_fuzz_workflow_runs_every_target() -> TestResult {
    let workflow = fs::read_to_string(repository(".github/workflows/fuzz.yml"))?;
    for target in Target::ALL {
        let entry = format!("          - {}\n", target.name());
        assert!(
            workflow.contains(&entry),
            "{} is not in the Fuzz workflow's matrix",
            target.name()
        );
    }
    Ok(())
}
