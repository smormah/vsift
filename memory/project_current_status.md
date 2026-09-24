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
- prove, automatically and once per tool pair, that FFmpeg/FFprobe really work
  before it runs them on a user's video;
- manage the session's lifetime and retention, and validate retained bundles.

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
  expiry.
- `session list/status/renew/close/retain/clean` and `bundle validate`.
- Still `COMMAND_NOT_IMPLEMENTED`: `transcript retranscribe`, search, candidates,
  frame, audio, crop, job and setup install/repair/list/rollback/remove.

## Media-tool preflight (P07 increment 2b)

- Application: `preflight_media_tools(verifier, cache, fingerprint, now)` returns
  `AlreadyVerified` or `VerifiedNow(record outcome)`, or a
  `MediaToolPreflightFailure { check, failure }`. New port
  `MediaToolVerificationCache` (lookup, record; never errors, fails closed); values
  `MediaToolFingerprint`, `CachedMediaToolVerification`, `VerificationRecord(Skip)`.
- Infrastructure `media_tool_verification_cache`: `media_tool_fingerprint` (SHA-256
  over canonical paths, size, mtime, Unix dev/ino/mode/uid/ctime or Windows
  creation time/attributes, adapter profile, every reviewed-policy field, host
  isolation, verifier authority, profile version 1, `CARGO_PKG_VERSION`; no
  content hash) and `FilesystemMediaToolVerificationCache`, opened through
  `UserDependencyConfigStore::media_tool_verification_state`. Record
  `media-tool-verification/verified-v1.json`: strict JSON, schema 1, <= 8 entries,
  <= 4 KiB, no-follow, single-link, 7-day age limit, future-dated entries rejected;
  writes under a non-blocking `HeldFileLock` (busy = skip), create-new temp +
  fsync + rename, stale pending files swept under the lock. An unusable state
  directory disables recording; verification then runs in the private root itself.
- Engine: `ensure_media_tools_verified` runs in `ingest` with a transcript, after
  parsing and tool resolution, before the session root. `EngineError::
  MediaToolVerificationFailed` maps by reason: tool faults `MISSING_CAPABILITY`,
  `Deadline` `DEADLINE_EXCEEDED`, `Workspace` `STORAGE_IO`, `Cancelled`, and
  `FixtureIntegrity` `INTERNAL`. `EnginePorts::with_media_tool_verifier` injects
  a verifier (tests, other hosts); its passes use a separate fingerprint.
- Contract: `media_tool_verification_summary` gives fixed prose starting
  `... at the <check> step (<reason>).`; frozen example
  `schemas/v1/examples/media-tool-verification-failed.json`. No schema change.
- Measured on Windows 11, FFmpeg 9.0: first F10 import 2.5 s, cached 0.7 s.

## The engine library and contract

- **`crates/vsift`:** `Engine::new(EngineConfig, EnginePorts)` with injected `Clock`,
  `IdentifierSource` and optional media-tool verifier. Operations: setup
  (`check_setup`, `plan_setup`, `configure_executable`, `configure_model`), sessions
  (`ingest` -> `IngestOutcome`, `list_sessions`, `session_status`, `renew_session`,
  `close_session`, `retain_session`, `clean_sessions`), `transcript`,
  `validate_bundle`, `verify_media_tools`, `identify_model`. One typed
  `EngineError`; `failure_code()` is the single public-code mapping;
  `transcript_rejection()`, `missing_media_tool()` and
  `media_tool_verification_failure()` feed remediation. The API is 0.x.
- **`crates/vsift-contract`:** every v1 wire type, fixed-prose warnings and
  remediation, `OperationResponse::failure_with_remediation`.
- **`vsift-cli`** is a thin host: parsing, configuration precedence, presentation,
  exit codes and `CommandFailure` (code plus optional typed remediation).

## Transcript design (P07 increment 2)

Domain `transcript` (offset shift, cue order/overlap policy, `align_imported_cues`
clamps nothing, `TranscriptRevision` re-checked on read, content-derived `trv_`,
`tsg_`, `sgm_`); application `OpenSession::execute_with_transcript`,
`page_transcript`, `SourceDurationProbe`; infrastructure bounded parsers,
`FfprobeSourceDuration` and the versioned `transcript_record` artifact; four v1
schemas with F10-based frozen examples.

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
| P07 | In progress: increments 1a, 1b, 2 and 2b (preflight) done; local ASR next |
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

- Preflight increment, Windows 11: fmt, strict Clippy (pedantic as errors),
  `cargo test --workspace` (381 passed, 22 opt-in ignored), warning-denied rustdoc
  and the governance check pass. New tests: 3 application use-case, 9
  infrastructure record/fingerprint (ageing, corrupt/oversized/linked, busy lock,
  8 concurrent writer threads), 1 engine code mapping, 5 engine with a verifier
  double (miss/hit, changed or reselected tool, ageing, failure before any write
  and not cached, defective/hard-linked record, 6 concurrent engines, plain ingest
  untouched), 2 CLI contract (stand-in tool failure in JSON/JSONL/human; plain
  ingest), 3 contract (frozen example, every check x reason schema-valid).
  Opt-in with real FFmpeg/FFprobe 9.0: engine verify-once-then-reuse and
  FFmpeg-as-FFprobe (fails at probe, `provider_rejected`), the CLI
  FFmpeg-as-FFprobe contract, and the P07 transcript E2E stage all pass.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance and
  the strict worker boundary; dependency policy and review; CodeQL. Merges go through
  protected `main` with squash merges.
- Qualification records are in `docs/planning/` (P03-P06).

History lives in git and pull requests (squash merges with PR links), `CHANGELOG.md`
and `docs/history/2026-09-09-to-23-delivery-log.md`.
