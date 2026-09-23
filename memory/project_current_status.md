# VSift current status

As of 2026-09-23. Current-state document: rewrite it, don't append to it. Next
actions and open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives AI coding agents local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is
now also an embeddable engine library (`vsift`) that the CLI, and later other hosts,
use. Today it can:
- check and register its dependencies and show a read-only setup plan;
- copy a video into a private, disposable session;
- manage that session's lifetime and retention, and validate retained bundles.

It cannot yet transcribe, search, or hand frames, crops or audio to an agent.
Those arrive with P07–P09.

## What works (public CLI)

- `setup check`: probes FFmpeg, FFprobe and whisper.cpp. An explicit or configured
  path wins over the filtered `PATH`. This is an executable probe only; the model
  is not checked yet.
- `setup configure` / `setup configure-model`: save user-managed executable and
  model paths in private per-user configuration without running them.
- `setup plan`: a read-only plan. On Ubuntu 24.04 x86-64 it lists reviewed
  managed actions and an acceptance digest; elsewhere it gives typed manual
  guidance.
- `ingest`: stages and hashes one local MP4/Matroska source into a disposable
  session. Sessions expire after 24 idle hours, at most 7 days after opening.
- `session list/status/renew/close/retain/clean` and `bundle validate`.
- Every other command parses but returns `COMMAND_NOT_IMPLEMENTED`: transcript,
  search, candidates, frame, audio, crop, job and setup install/repair/list/
  rollback/remove. `setup install` still revalidates a saved plan first.

## The engine library (P07 increments 1a and 1b)

- **`crates/vsift` (increment 1b, in review):** the one API a host uses. A host
  builds `Engine::new(EngineConfig, EnginePorts)` with an explicit session-root
  location, per-user configuration location and host isolation, plus injected
  `Clock` and `IdentifierSource` ports (`EnginePorts::system()` in production).
  Operations: `check_setup`, `plan_setup` (+ `EvaluatedSetupPlan::validate_acceptance`),
  `configure_executable`, `configure_model`, `ingest`, `list_sessions`,
  `session_status`, `renew_session`, `close_session`, `retain_session`,
  `clean_sessions`, `validate_bundle`, and, for later hosts and the P07 preflight,
  `verify_media_tools` and `identify_model`. Every operation fails with one typed
  `EngineError`; `failure_code()` is the single mapping to public codes. The library
  API is 0.x and unstable.
- **`crates/vsift-contract` (increment 1a, PR #126):** every v1 JSON wire type and
  the mapping into it from domain and application values.
- **`vsift-cli`** is a thin host: clap parsing, configuration precedence, human/JSON/
  JSONL presentation through `vsift-contract`, RFC 3339 formatting, exit codes, the
  bounded writer and reading the saved-plan file. Its normal dependencies are `vsift`
  and `vsift-contract` only.

## What exists internally (not exposed by the CLI)

- **P02:** shell-free process supervision with Windows Job Object and Unix
  process-group containment, bounded output, one deadline and cancellation.
- **P03:** private storage roots, cross-process locks, weighted admission,
  immutable generations and process-crash recovery. The desktop ephemeral profile
  only; strict durability is gated (FS-01).
- **Locks:** every OS file lock goes through one type that unlocks explicitly
  on release, so a lock never outlives its owner (issue #66).
- **Tool verification (P06):** the engine proves selected FFmpeg/FFprobe work by
  running the embedded F01 fixture through the real probe, frame and audio steps,
  and identifies a registered model against the reviewed pin. No CLI command calls
  it yet; the P07 preflight will.
- **P04:** restricted FFprobe/FFmpeg metadata, frame and audio operations that
  report observed timestamps.
- **Managed-installer foundations** (built under P06, now owned by P13):
  - the reviewed Ubuntu catalogue with its compatibility policy;
  - plan acceptance and revalidation;
  - bounded HTTPS transfer and tar/gzip/XZ inspection;
  - owned staging and runtime layout;
  - the install guard, immutable version publication, rollback selection and
    removal fencing.

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`; [ADR 0015](../docs/decisions/0015-r0-delivery-replan.md)) |
| P07 | In progress: increments 1a (contract crate, `00e707f`) and 1b (engine facade, in review) done; transcription next |
| P08–P12, P14 | Not started |
| P13 | Not started; now also delivers managed dependency installation |

## Architecture snapshot

`vsift-domain` (values, no I/O) ← `vsift-application` (use cases and ports, including
`Clock` and `IdentifierSource`) ← `vsift-infrastructure` (OS, processes, storage,
providers, system clock, random identifiers, session-root location) ← `vsift` (engine
facade and use-case composition) ← `vsift-cli` (parse, present). `vsift-contract`
(v1 wire types) sits beside the engine and depends on domain and application only.
`tools/vsift-governance` checks the delivery ledger and the size of these handoff
files. The largest modules are `filesystem_session_store.rs` and
`managed_artifact_store.rs`, about 3.6k lines each including tests.

## Quality evidence

- Local gates on Windows 11: fmt, strict Clippy with pedantic lints as errors,
  the workspace tests (296 passed, 15 opt-in ignored), warning-denied rustdoc,
  `cargo deny` and the governance check all pass.
- Increment 1b evidence:
  - the unchanged `cli_contract`, `schema_contract`, `p05_cli_contract` and
    `schema_conformance` suites pass;
  - `crates/vsift/tests/engine_lifecycle.rs` drives the library without the CLI,
    with a controlled clock and sequential identifiers: open → status → renew →
    close → clean with exact identities and expiry, expiry-based cleanup, bundle
    round trip, typed failures, setup check with explicit missing paths and model
    identification; its real-FFmpeg verification test is opt-in and passed locally;
  - a differential run of 86 CLI invocations (setup, ingest, session lifecycle,
    bundle, parse errors; human, JSON and JSONL) against `00e707f` gave identical
    output and exit codes after normalizing identifiers, timestamps and index
    placement;
  - the P06 E2E stage and the P05 session checkpoint passed on Windows 11 with
    FFmpeg/FFprobe 9.0 and no whisper.cpp.
- CI on every PR:
  - Quality on Ubuntu, macOS and Windows;
  - Documentation, Governance and the strict worker boundary;
  - dependency policy and review, CodeQL and Rust analysis.
  - Merges go through protected `main`: eight required checks, squash merges and
    linear history.
- Opt-in checkpoints exist for P04, P05 and P06
  ([E2E spine](../docs/planning/e2e-test-spine.md)).
- P06 ledger tests D-01, the D-07 guidance clause, D-09 and D-10 map to named
  tests in `docs/planning/verification.md`; their managed-install parts are P13's.
  The D-07 plan-guidance unit tests now live in the engine (`crates/vsift/src/setup.rs`).
- Qualification records are in `docs/planning/`: `p03-storage-feasibility.md`,
  `p04-media-qualification.md`, `p05-session-qualification.md`,
  `p06-provisioning-source-review.md` and the P06 Ubuntu and Windows candidate
  records.

## Where history lives

- Git history and pull requests (every merge is a squash with a PR link).
- `CHANGELOG.md` for user-visible changes.
- `docs/history/2026-09-09-to-23-delivery-log.md` for the day-by-day log up to the
  re-plan.
