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
  offset, and return timestamped transcript segments for a time range, either as one
  page or as a JSON Lines stream of keyed evidence records an indexer can upsert;
- prove, automatically and once per tool pair, that FFmpeg/FFprobe really work
  before it runs them on a user's video;
- manage the session's lifetime and retention, and validate retained bundles,
  including the content of their transcript records.

It cannot yet transcribe speech itself, search, or hand frames, crops or audio to an
agent. Local ASR is the next P07 increment; search and visuals are P08-P09.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model` and the read-only
  `setup plan`, unchanged since P06 (`setup check` still reports only
  `executable_probe_only`).
- `ingest <video>`: stages and hashes one local MP4/Matroska source into a disposable
  session (24 idle hours, at most 7 days). Runs no provider.
- `ingest <video> --transcript <file> [--transcript-offset <signed us>]`: parses the
  sidecar under a bounded strict policy, locates FFmpeg/FFprobe (configured first,
  then `PATH`; never Whisper), runs the media-tool preflight, probes the staged
  video's duration, keeps only cues wholly inside it, and commits source and
  transcript in one generation. Rejections carry a typed reason in fixed prose.
- `transcript get <session> --from --to [--limit 1..100] [--cursor]`: a bounded page
  of self-describing segment records; cursors bound to session, revision, range and
  expiry. With `--events jsonl` the page is an evidence stream (below).
- `session list/status/renew/close/retain/clean` and `bundle validate`.
- Still `COMMAND_NOT_IMPLEMENTED`: `transcript retranscribe`, search, candidates,
  frame, audio, crop, job and setup install/repair/list/rollback/remove.

## Evidence stream and bundle record (P07 increment 2c)

- `transcript get --events jsonl`: one evidence event per segment, then exactly one
  terminal event. Evidence event = `schema_version`, `event: "evidence"`, contiguous
  `sequence` from 0, `command`, `operation_id`, `record_type: "transcript_segment"`,
  upsert `key` (= `segment_id`) and `record` (the published segment). The terminal
  event has `sequence` = record count; its data is the page without `items`, plus
  `record_count` and `next_cursor` (`transcript-get-stream-data.schema.json`).
  Failures and other commands stay one terminal event; empty ranges are one complete
  terminal event with `record_count: 0`. No progress events. `--json` and human
  output unchanged. The stream is assembled (each line within 1 MiB) before writing.
- Contract: `vsift-contract::stream` owns `EventKind`, `EvidenceRecordType` (both
  with `ALL` and compile-time ordinal guards), `EvidenceEventResponse`,
  `TranscriptStreamData` and `TranscriptEvidenceStream` (the sequencing).
  `TerminalEventResponse::at_sequence` is crate-private. The CLI's `JsonLines`
  buffer and `write_evidence_stream` only serialize and write.
- Bundle: `bundle-transcript-record.schema.json` publishes the stored record.
  `FilesystemSessionStore::validate_bundle` now decodes every `transcript_record`
  through the shared `read_transcript_artifact` (size, digest, strict decode, same
  source); non-conforming is `IntegrityFailure`, newer is `UnsupportedVersion`.
- Schemas/examples: `evidence-event`, `transcript-get-stream-data`,
  `bundle-transcript-record`; `examples/transcript-get.events.jsonl` (byte-exact)
  and `examples/bundle-transcript-record.json`. `terminal-event` gained only a
  description.

## Media-tool preflight (P07 increment 2b)

- Engine `ensure_media_tools_verified` runs in `ingest` with a transcript, after
  parsing and tool resolution, before the session root. It runs the F01 fixture
  through probe, frame and audio checks once per tool identity and records the pass
  in `<per-user vsift dir>/media-tool-verification/verified-v1.json` (strict, <= 8
  entries, <= 4 KiB, 7-day age limit, fails closed, non-blocking lock).
- Failures map by reason to `MISSING_CAPABILITY`, `DEADLINE_EXCEEDED`, `STORAGE_IO`,
  `CANCELLED` or `INTERNAL`, with fixed-prose remediation naming check and reason.
- Measured on Windows 11, FFmpeg 9.0: first F10 import 2.5 s, cached 0.7 s.

## The engine library and contract

- **`crates/vsift`:** `Engine::new(EngineConfig, EnginePorts)` with injected `Clock`,
  `IdentifierSource` and optional media-tool verifier. Operations: setup
  (`check_setup`, `plan_setup`, `configure_executable`, `configure_model`), sessions
  (`ingest` -> `IngestOutcome`, `list_sessions`, `session_status`, `renew_session`,
  `close_session`, `retain_session`, `clean_sessions`), `transcript`,
  `validate_bundle`, `verify_media_tools`, `identify_model`. One typed
  `EngineError`; `failure_code()` is the single public-code mapping. The API is 0.x.
- **`crates/vsift-contract`:** every v1 wire type, the evidence stream sequencing,
  fixed-prose warnings and remediation.
- **`vsift-cli`** is a thin host: parsing, configuration precedence, presentation,
  exit codes and `CommandFailure` (code plus optional typed remediation).

Transcripts (increment 2): domain `transcript` (alignment clamps nothing; content-
derived `trv_`, `tsg_`, `sgm_`), application `page_transcript`, infrastructure
bounded SRT/VTT parsers, `FfprobeSourceDuration` and the `transcript_record` artifact.

## What exists internally (not exposed by the CLI)

- **P02:** shell-free process supervision with Windows Job Object and Unix
  process-group containment, bounded output, one deadline and cancellation.
- **P03:** private storage roots, cross-process locks, weighted admission,
  immutable generations and process-crash recovery (ephemeral profile only; FS-01).
- **P04:** restricted FFprobe/FFmpeg metadata, frame and audio operations.
- **P06:** model identification (not yet consumed); its F01 verifier feeds the preflight.
- **Managed-installer foundations** (owned by P13): reviewed Ubuntu catalogue,
  plan acceptance, bounded transfer and archive inspection, staging, install guard,
  immutable versions, rollback selection and removal fencing.

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | In progress: increments 1a, 1b, 2, 2b and 2c (evidence stream) done; local ASR next |
| P08–P12, P14 | Not started |
| P13 | Not started; now also delivers managed dependency installation |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present). `vsift-contract` sits beside the engine and depends on
domain and application only. `tools/vsift-governance` checks the ledger and the size
of these files. The largest modules are `filesystem_session_store.rs` and
`managed_artifact_store.rs`, about 3.8k and 3.6k lines including tests.

## Quality evidence

- Evidence-stream increment, Windows 11: fmt, strict Clippy (pedantic as errors),
  `cargo test --workspace` (403 passed, 22 opt-in ignored), warning-denied rustdoc and the
  governance check pass. New tests: 2 contract unit, 10 contract stream conformance
  (byte-exact example, every line schema-valid, records equal `--json` items,
  cursor continuation, empty range, page-limit bound, unknown fields and foreign
  keys rejected, `EventKind`/`EvidenceRecordType` drift guards, failure event),
  3 CLI output unit (`JsonLines` order, oversized line writes nothing, broken pipe),
  4 CLI binary contract on a store-seeded session (records then one terminal,
  continuation, limits 1/2/3/100, empty range, failures), 3 infrastructure
  (frozen record example, real retained record conforms, `bundle validate` rejects
  six non-conforming records and accepts the rewritten original).
- Opt-in with FFmpeg/FFprobe 9.0: the P07 transcript E2E now also consumes each
  import as a JSONL stream, retains it, runs `bundle validate` and checks the record
  schema; SRT, WebVTT and wrong-offset journeys all pass.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance,
  strict worker boundary, dependency policy and CodeQL; squash merges to protected
  `main`. Qualification records are in `docs/planning/`; history in git,
  `CHANGELOG.md` and `docs/history/2026-09-09-to-23-delivery-log.md`.
