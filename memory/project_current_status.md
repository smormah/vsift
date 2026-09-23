# VSift current status

As of 2026-09-23. Current-state document: rewrite it, don't append to it. Next
actions and open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives AI coding agents local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is
becoming an embeddable engine as well. Today it can:
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
  rollback/remove.

## What exists internally (not exposed)

- **P02:** shell-free process supervision with Windows Job Object and Unix
  process-group containment, bounded output, one deadline and cancellation.
- **P03:** private storage roots, cross-process locks, weighted admission,
  immutable generations and process-crash recovery. The desktop ephemeral profile
  only; strict durability is gated (FS-01).
- **Locks:** every OS file lock goes through one type that unlocks explicitly
  on release, so a lock never outlives its owner (issue #66).
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
| P06 | Open, closing on verification and guidance ([ADR 0015](../docs/decisions/0015-r0-delivery-replan.md)) |
| P07 | Not started; next after P06, begins with the engine boundary |
| P08–P12, P14 | Not started |
| P13 | Not started; now also delivers managed dependency installation |

## Architecture snapshot

`vsift-domain` (values, no I/O) ← `vsift-application` (use cases, ports) ←
`vsift-infrastructure` (OS, processes, storage, providers) ← `vsift-cli` (parse,
compose, present). `tools/vsift-governance` checks the delivery ledger and the size
of these handoff files. ADR 0016 adds an engine facade crate and a contract crate at
the start of P07. The largest modules are `filesystem_session_store.rs` and
`managed_artifact_store.rs`, about 3.6k lines each including tests.

## Quality evidence

- Local gates on Windows 11: fmt, strict Clippy with pedantic lints as errors,
  the workspace tests and the governance check all pass.
- CI on every PR:
  - Quality on Ubuntu, macOS and Windows;
  - Documentation, Governance and the strict worker boundary;
  - dependency policy and review, CodeQL and Rust analysis.
  - Merges go through protected `main`: eight required checks, squash merges and
    linear history.
- Opt-in real-media checkpoints exist for P04 and P05
  ([E2E spine](../docs/planning/e2e-test-spine.md)).
- Qualification records are in `docs/planning/`: `p03-storage-feasibility.md`,
  `p04-media-qualification.md`, `p05-session-qualification.md`,
  `p06-provisioning-source-review.md` and the P06 Ubuntu and Windows candidate
  records.

## Where history lives

- Git history and pull requests (every merge is a squash with a PR link).
- `CHANGELOG.md` for user-visible changes.
- `docs/history/2026-09-09-to-23-delivery-log.md` for the day-by-day log up to the
  re-plan.
