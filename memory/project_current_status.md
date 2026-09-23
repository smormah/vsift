# VSift current status

As of 2026-09-24. Current-state document: rewrite it, don't append to it. Next
actions and open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives AI coding agents local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is
also an embeddable engine library (`vsift`) that the CLI, and later other hosts, use.
Today it can:
- check and register its dependencies and show a read-only setup plan;
- copy a video into a private, disposable session;
- import an existing SRT or WebVTT transcript with the video, aligned by an explicit
  offset, and return timestamped transcript segments for a time range;
- manage the session's lifetime and retention, and validate retained bundles.

It cannot yet transcribe speech itself, search, or hand frames, crops or audio to an
agent. Local ASR is the next P07 increment; search and visuals are P08-P09.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model` and the read-only
  `setup plan`, unchanged since P06.
- `ingest <video>`: stages and hashes one local MP4/Matroska source into a disposable
  session (24 idle hours, at most 7 days).
- `ingest <video> --transcript <file> [--transcript-offset <signed us>]` (P07
  increment 2): parses the sidecar under a bounded, strict policy, locates FFmpeg and
  FFprobe (configured first, then `PATH`; Whisper is never needed), probes the staged
  video's duration, keeps only cues wholly inside it after the offset (others are
  reported, never clamped), and commits source and transcript in one generation.
  Rejections carry a typed reason and line in a fixed-prose remediation.
- `transcript get <session> --from --to [--limit 1..100] [--cursor]`: a bounded page
  of self-describing segment records; cursors are bound to session, revision, range
  and expiry.
- `session list/status/renew/close/retain/clean` and `bundle validate`.
- Still `COMMAND_NOT_IMPLEMENTED`: `transcript retranscribe`, search, candidates,
  frame, audio, crop, job and setup install/repair/list/rollback/remove.

## The engine library and contract

- **`crates/vsift`:** `Engine::new(EngineConfig, EnginePorts)` with injected `Clock`
  and `IdentifierSource`. Operations: setup (`check_setup`, `plan_setup`,
  `configure_executable`, `configure_model`), sessions (`ingest` returning
  `IngestOutcome`, `list_sessions`, `session_status`, `renew_session`,
  `close_session`, `retain_session`, `clean_sessions`), `transcript`,
  `validate_bundle`, `verify_media_tools` and `identify_model`. One typed
  `EngineError`; `failure_code()` is the single public-code mapping, and
  `transcript_rejection()`/`missing_media_tool()` feed remediation. The API is 0.x.
- **`crates/vsift-contract`:** every v1 wire type, including `TranscriptSegmentData`
  (the first published evidence record), `TranscriptRevisionData`,
  `TranscriptPageData`, `OpenData::with_transcript`, fixed-prose import warnings and
  `OperationResponse::failure_with_remediation`.
- **`vsift-cli`** is a thin host: parsing, configuration precedence, presentation,
  exit codes and `CommandFailure` (code plus optional typed remediation).

## Transcript design (P07 increment 2)

- Domain `transcript` module: `TranscriptOffset::shift` (the one file-to-source
  conversion), cue order (out of order rejected) and overlap (warned) policy,
  `align_imported_cues` (T-02: nothing clamped), `SourceSegment` (a finite file is
  one closed segment), `TranscriptRevision` with invariants re-checked on read
  (offset consistency, unknown confidence, no SRT speakers). New IDs `trv_`, `tsg_`,
  `sgm_`, derived from content with SHA-256 in the application layer.
- Application: `OpenSession::execute_with_transcript` (stage, probe, align, then one
  atomic activation), `build_imported_revision`, `page_transcript`, and the
  `SourceDurationProbe` port.
- Infrastructure: streaming bounded parsers `transcript_sidecar` (`srt`, `webvtt`),
  `FfprobeSourceDuration`, the versioned `transcript_record` artifact
  (`encode/decode_transcript_record`, 24 MiB bound) and
  `FilesystemSessionStore::{activate_source_with_artifact, read_transcript}`.
- Schemas: `ingest-data`, `transcript-get-data`, `transcript-revision`,
  `transcript-segment`, with F10-based frozen examples.
- Fixtures: `fixtures/corpus/transcripts/F10.srt` and `F10.vtt` (+500 ms offset onto
  F10-E01); malformed variants in `crates/vsift-infrastructure/tests/data/transcripts`.

## What exists internally (not exposed by the CLI)

- **P02:** shell-free process supervision with Windows Job Object and Unix
  process-group containment, bounded output, one deadline and cancellation.
- **P03:** private storage roots, cross-process locks, weighted admission,
  immutable generations and process-crash recovery (ephemeral profile only; FS-01).
- **P04:** restricted FFprobe/FFmpeg metadata, frame and audio operations that
  report observed timestamps; the transcript import reuses its probe.
- **Tool verification (P06):** the F01 fixture verifier and model identification;
  no command calls them yet (the P07 preflight will).
- **Managed-installer foundations** (owned by P13): reviewed Ubuntu catalogue,
  plan acceptance, bounded transfer and archive inspection, staging, install guard,
  immutable versions, rollback selection and removal fencing.

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | In progress: increments 1a, 1b and 2 done; local ASR (increment 3) next |
| P08–P12, P14 | Not started |
| P13 | Not started; now also delivers managed dependency installation |

## Architecture snapshot

`vsift-domain` (values, no I/O) ← `vsift-application` (use cases and ports) ←
`vsift-infrastructure` (OS, processes, storage, providers, parsers) ← `vsift` (engine)
← `vsift-cli` (parse, present). `vsift-contract` sits beside the engine and depends on
domain and application only. `tools/vsift-governance` checks the ledger and the size
of these files. The largest modules are `filesystem_session_store.rs` and
`managed_artifact_store.rs`, about 3.8k and 3.6k lines including tests.

## Quality evidence

- Local gates on Windows 11 for increment 2: fmt, strict Clippy (pedantic as errors),
  `cargo test --workspace` (350 passed, 19 opt-in ignored), warning-denied rustdoc,
  `cargo deny` and the governance check all pass.
- Increment 2 tests by layer: domain transcript rules and a property test;
  application use-case, identity and cursor tests (C-03); parser T-01/T-02 tests with
  three `proptest` properties; store tests for atomic publication, bundles, tamper
  and strict decoding; contract tests against the new schemas and frozen examples;
  engine and CLI tests for pre-mutation rejections. Opt-in with real FFprobe: engine
  `engine_transcript`, CLI `transcript_cli_contract` and the P07 E2E stage.
- Opt-in checkpoints P04, P05, P06 and P07 (supplied transcript) passed on Windows 11
  with FFmpeg/FFprobe 9.0 and no whisper.cpp. The P07 stage imports F10 via SRT and
  WebVTT with an empty `PATH` and cites F10-E01 exactly; a wrong offset is rejected.
- Intentional contract change: the P05 CLI test that asserted
  `ingest --transcript` returned `COMMAND_NOT_IMPLEMENTED` now asserts the typed
  failure for an unreadable sidecar; all other existing contract and schema tests
  pass unchanged, and plain `ingest` output is byte-identical.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance and
  the strict worker boundary; dependency policy and review; CodeQL. Merges go through
  protected `main` with squash merges.
- Qualification records are in `docs/planning/` (P03-P06).

## Where history lives

- Git history and pull requests (every merge is a squash with a PR link).
- `CHANGELOG.md` for user-visible changes.
- `docs/history/2026-09-09-to-23-delivery-log.md` for the log up to the re-plan.
