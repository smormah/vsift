# VSift current status

As of 2026-10-02. Current-state document: rewrite it, don't append to it. Next actions and
open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives AI coding agents local, source-grounded access to
the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is also an
embeddable engine library (`vsift`) that the CLI, and later other hosts, use. Today it can:
- check and register its dependencies, show a read-only setup plan, report whether local speech
  recognition really works here (`setup check` `local_asr`), and on Ubuntu 24.04 x86-64 install
  the reviewed FFmpeg, whisper.cpp and model itself (`setup install`), then list, roll back,
  remove and diagnose them (`setup list/rollback/remove/repair`); that store survives kills at
  every step and, on Ubuntu 24.04 with ext4, power loss;
- copy a video into a private, disposable session;
- import an existing SRT or WebVTT transcript with the video, aligned by an offset;
- transcribe the video's speech itself with whisper.cpp (`transcript retranscribe`), continue an
  interrupted transcription where it stopped, and report, resume or cancel that work by its job
  id (`job status/resume/cancel`), with chunk progress;
- return timestamped transcript segments, search them, list the moments where the screen
  changed, and return exact frames, neighbours, bursts, crops and WAV clips;
- manage the session's lifetime and retention, and validate retained bundles;
- stop a long command cleanly on Ctrl-C or `SIGTERM`, never leaving a helper running;
- run as a worker under an external supervisor (`session init-workspace`, `job run`,
  `job batch`), each step at most once per operation id, with weighted admission, a resumable
  shutdown and (Ubuntu 24.04, local ext4) results that survive power loss;
- refuse to claim strict worker isolation unless the Linux kernel attests it;
- keep every folder it creates private to the user;
- check an agent's draft report before it is sent (`handoff check`);
- be installed without Rust: native archives for the three R0 targets and the npm packages;
  **the 0.1.0 pre-release is published** (2026-10-01): `npm install --global vsift-cli@next`,
  and archives on a GitHub pre-release, each with a Sigstore attestation and npm provenance.

**The agent skill** (`skills/vsift/`) teaches Claude Code or Codex to run an investigation with
the CLI and write a cited report. P12's named-client trials qualified it:
- **Review tier:** Claude Opus 5.5 in Claude Code and GPT-6-Astra in Codex each passed 11 of 11
  trials mechanically and 9 of 11 fully. **Compact tier:** Claude Sonnet 5.5 and GPT-6-Sol: 82% in
  P12; the re-run (#222) meets the 90% target, Sonnet 26 of 28 and Sol 28 of 28 (23 as run).
- **Safety:** no agent leaked a secret, installed anything, acted on injected text or copied a
  hidden character into a report. Codex ran in a Linux container (L-076, #204). **Not yet tried:**
  an agent with no skill (the cold-agent variant is in P14), and any agent on the published package.

## Where the project stands

**P00-P13 are complete. P14, the R0 qualification (#17), is in progress** (started 2026-10-02;
decisions A-H of ADR 0024 confirmed as recommended). Its plan is 14 pull requests (0-13); **PR 0
(#250) is merged; PR 8, the release machinery for a candidate and the stable (#252), is done in
this change, awaiting review; PR 1 is being built in parallel; the whole packet is not complete.**
What it must show, and what is weaker than it sounds today:
- **No checkpoint has run the published artifact on a video:** every real-tool test runs a
  Cargo-built `vsift`, and no install has run on a clean machine (hosted runners carry Rust).
  The one real-registry install was npm on the maintainer's Windows 11 machine, where Smart App
  Control is Off (read 2026-10-02), so it says nothing about that (L-098).
- **One real platform:** the P08/P09/P11 numbers and the Claude Code trials are Windows 11;
  Ubuntu 24.04 has local ASR, the managed install and the A-08 journey; macOS has no media,
  speech or evidence run (L-035).
- **A synthetic corpus and synthetic voice only** (L-020, L-022, L-028, L-030).
- **Trials tuned on their own scenarios;** no hold-out, no skill-less run; the review tier's
  blurred-banner re-run is missing (L-095).
- **SEC-T01 is half done** (L-068, #188) and dangerous media has never been run; the load
  ladder stopped at 4 jobs; fuzzing is weekly at five minutes per target.
- **`latest` has never moved.** The path that moves it is built and tested (PR 8) but has never
  run for real (L-105); the Release workflow does not check the evidence ledger (L-106).

**Decisions (ADR 0024, Proposed until P14 completes):** all eight confirmed as recommended on
2026-10-02; the list is in `memory/TODO.md` (R0 ships as `0.2.0` on `latest` after a published
`0.2.0-rc.N`; no signing unless the try-outs show a block; 84 agent runs; "supported" per cell).

## P14 PR 8 in one view (done in this change)

[ADR 0024](../docs/decisions/0024-r0-qualification-and-release-candidate.md)'s note of 2026-10-02;
the runbook is `docs/operations/release.md` 6.7-6.9. **Nothing was published.** The version alone
decides the channel: a suffix publishes under `next` (a candidate is `-rc.N`), none is stable and
moves `latest` on all four packages (`0.0.0` and ambiguous versions are refused). A stable plan
states what moves and is guarded: the accepted candidate (highest `v<X.Y.Z>-rc.<N>`, an ancestor,
differing only in version strings and the launcher's README), the candidate on npm, `latest`
forward-only; enforced on a publish and on a dispatch of the tag even as a dry run. No workflow may
run `npm dist-tag` (lint rule 8); 61 lint mutations; the shell runs against stubs in CI. **Not done:**
a real stable publish (L-105); the evidence-ledger guard (L-106, before PR 10). **Frozen at the
candidate cut:** the allowed lists, the release notes, the workflow, the skill. `0.1.0` is stable by
shape, so every pull request's plan says it would be refused.

## P13 in one view (complete)

[ADR 0023](../docs/decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
(**Accepted** 2026-10-01); the record is `docs/planning/p13-distribution.md`, the guide for users
`docs/operations/install.md`, the maintainer's runbook `docs/operations/release.md`.
- **Delivered:** native archives and `release.yml` (Windows x64, macOS 15 arm64, Linux x64) with
  SBOMs, notices, checksums and Sigstore provenance; the npm launcher `vsift-cli` over
  `@vsift/win32-x64`, `@vsift/darwin-arm64` and `@vsift/linux-x64`; managed installation on Ubuntu
  24.04 x64; human output; `handoff check`; the 0.1.0 pre-release (2026-10-01, tag `v0.1.0` on
  `011bc4d`; the first dispatch failed with `ENEEDAUTH` before anything was published, L-100; the
  second published by trusted publishing, no token). Verified once: `npm audit signatures` 4 and 4,
  `gh attestation verify` 10 of 10 files and 4 of 4 tarballs. Hosted: the managed smoke and
  power-loss runs on Ubuntu 24.04, the twelve-job npm matrix (a local registry, runners with Rust).
- **Not proved:** a clean-machine install; pnpm, Yarn or Bun on the real registry; an extracted
  archive run; the offline install with real artifacts; Smart App Control and the macOS prompts
  (L-098); power loss beyond Ubuntu 24.04 ext4 (L-037).
## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`;
  `setup install` on Ubuntu 24.04 x86-64; `setup list/rollback/remove/repair` on every OS.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`.
- `transcript retranscribe`, `transcript get`, `job status|resume|cancel|run|batch`,
  `search`, `candidates`, `frame get/neighbours/burst`, `crop`, `audio`.
- `session list/status/renew/close/retain/clean/init-workspace` and `bundle validate`.
- `handoff check`: a draft from stdin or `--file`. A release build's `--version` names its
  source commit (`vsift 0.1.0 (<12 hex>)`).
- Global `--session-root`, `--host-isolation`, `--json`, `--events jsonl`. A command line that
  does not parse names its mistake and the `--help` to read.
- Readable terminal text without `--json` for every command (unstable, not for parsing).

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00-P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (`b73df52`) |
| P07 | Complete (`9ea3180`): engine, transcripts, local ASR, fuzzing |
| P08 | Complete (`b830fc9`): search, candidates, source binding |
| P09 | Complete (`e57c706`): frames, neighbours, bursts, crops, audio, reuse, lineage |
| P10 | Complete (`3f27ce3`): jobs, resume, cancellation, durable Ubuntu/ext4 |
| P11 | Complete (`40c4038`); SEC-T01 adversarial evidence is technical debt (#188, L-068) |
| P12 | Complete (2026-09-30, ADR 0022 Accepted): skill, harness, named-client trials; review tier qualified, compact tier 93% and 100% on the #222 re-run; open: L-095 (#224), #219, #204 (`1284e54`) |
| P13 | Complete (2026-10-01, ADR 0023 Accepted): distribution, managed install, `handoff check`, human output; 0.1.0 published; release commit `011bc4d` |
| P14 | **In progress** (started 2026-10-02): PR 0 merged (#250); PR 8 done in this change (#252); PR 1 in parallel; the rest follow |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine) <-
`vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and owns the wire
types. The worker lives in the engine; the CLI only presents. The skill (`skills/vsift/`) only
calls the `vsift` binary; its guard is a `vsift-cli` test module. The trial harness depends only
on `vsift` and `vsift-contract`; `tools/vsift-release` (never shipped) packages the release
archives, assembles the npm packages and plans their publication (a version without a suffix is
stable and moves `latest`; guards, the candidate comparison and release notes live there); `npm/` holds the launcher and its qualification.

## Quality evidence

- **Local gates** (in each PR description): fmt, strict Clippy, workspace tests, warning-denied
  rustdoc and governance. **CI on every PR:** Quality on Ubuntu, macOS and Windows;
  Documentation, Governance, fuzz harness replay, the strict worker boundary, dependency
  policy, CodeQL and the npm launcher tests; the Release dry run on archive/npm changes.
- **Required on `main`:** Quality (three OS), Documentation, Dependency policy and review,
  Analyze Rust, Governance. Squash merges; history in git, `CHANGELOG.md`, `docs/history/`.
