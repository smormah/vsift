# VSift current status

As of 2026-10-01. Current-state document: rewrite it, don't append to it. Next
actions and open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives AI coding agents local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is
also an embeddable engine library (`vsift`) that the CLI, and later other hosts, use.
Today it can:
- check and register its dependencies, show a read-only setup plan, report whether
  local speech recognition really works here (`setup check` `local_asr`), and on
  Ubuntu 24.04 x86-64 install the reviewed FFmpeg, whisper.cpp and model itself
  (`setup install`, P13 PR 4), then list, roll back, remove and diagnose them (`setup
  list/rollback/remove/repair`, P13 PR 6); that store survives kills at every step and,
  on Ubuntu 24.04 with ext4, power loss (P13 PR 7);
- copy a video into a private, disposable session;
- import an existing SRT or WebVTT transcript with the video, aligned by an offset;
- transcribe the video's speech itself with whisper.cpp (`transcript retranscribe`),
  continue an interrupted transcription where it stopped, and report, resume or cancel
  that work by its job id (`job status/resume/cancel`), with chunk progress;
- return timestamped transcript segments, search them, list the moments where the
  screen changed, and return exact frames, neighbours, bursts, crops and WAV clips;
- manage the session's lifetime and retention, and validate retained bundles;
- stop a long command cleanly on Ctrl-C or `SIGTERM`, never leaving a helper running;
- run as a worker under an external supervisor (`session init-workspace`, `job run`,
  `job batch`), each step at most once per operation id, with weighted admission, a
  resumable shutdown and (Ubuntu 24.04, local ext4) results that survive power loss;
- refuse to claim strict worker isolation unless the Linux kernel attests it;
- keep every folder it creates private to the user;
- check an agent's draft report before it is sent (`handoff check`, P13 PR 5);
- build, in CI, its own release archives for the three R0 targets (P13 PR 8) and the npm
  packages made from them, install and run those with npm, pnpm, Yarn and Bun on Windows,
  macOS and Ubuntu from a local registry (P13 PR 9), and plan, attest and publish them when
  the maintainer releases (P13 PR 10). **Nothing is published yet** (only a `0.0.0`
  placeholder); the attest and publish path has never run.

**The agent skill** (`skills/vsift/`) teaches Claude Code or Codex to run an
investigation with the CLI and write a cited report. P12's named-client trials
qualified it:

- **Review tier:** Claude Opus 5.5 in Claude Code and GPT-6-Astra in Codex each passed
  11 of 11 trials mechanically and 9 of 11 fully.
- **Compact tier:** Claude Sonnet 5.5 and GPT-6-Sol: 82% in P12; the re-run after the
  fixes (#222) meets the 90% target, Sonnet 26 of 28 and Sol 28 of 28 (23 as run).
- **Safety:** no agent leaked a secret, installed anything, acted on injected text or
  copied a hidden character into a report. Codex ran in a Linux container (L-076, #204).

**P00-P12 are complete.** P12 closed on 2026-09-30 by maintainer decision on its
final round's results (#223, `1284e54`); the ledger marks it `complete`.
**P13 is in progress and is not complete** (started 2026-09-30). All its code, tests
and documentation are merged or in the PR 11 change (PRs 0-11): human output, `handoff
check`, `setup install` and its lifecycle with kill and power-loss tests, `release.yml`, the
workflow lint, the npm packages, the publish wiring, and the user guide and record. It
completes only when the maintainer has made the release settings and published the 0.x
pre-release, and PR 12 has recorded it. What remains is listed in `memory/TODO.md`.

## P13 in one view

[ADR 0023](../docs/decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
(**Proposed**; Accepted by PR 12 at completion) records the maintainer's decisions A-H of
2026-09-30; the scope and the 13 pull requests are in `implementation-work-packets.md`
("P13 scope and pull requests"). The ledger marks P13 `in_progress` and maps R-03, R-13
and R-14 to it. **The record `docs/planning/p13-distribution.md` has the pull requests with
merge commits, the evidence per ID, what each proves and does not, and the pending
"First publish".** The guide for users is `docs/operations/install.md`; the maintainer's
runbook is `docs/operations/release.md`.

- **Delivers:** native archives and `release.yml` (Windows x64, macOS 15 arm64, Linux x64
  glibc) with SBOMs, notices, checksums and Sigstore provenance; the npm launcher over
  `@vsift/...` packages; managed installation on Ubuntu 24.04 x64; human output by
  default; `handoff check`.
- **Names:** scope `@vsift` (owned, #237); launcher package `vsift-cli` (npm refused
  `vsift`; placeholder `0.0.0` held, `latest`; command `vsift`; ADR 0009 notes). The
  three `@vsift/...` packages are not on the registry yet.
- **Hosted evidence (all Ubuntu 24.04 unless noted):** `P13 managed smoke` runs 36734316384
  (`d43a518`) and 36793180858 (`01656d6`, with `install-e2e`); `P13 managed power loss` run
  36793177930 failed on a verifier defect (53 "lost" acks, none an older state), fixed in
  #244, then passed on `6de55da` (run 36829198545: 0 lost, 0 damaged; negative control 36
  lost); `Release` runs 36786019996 and 36797351652 on `main` (the twelve-job npm matrix on
  Windows, macOS and Ubuntu, and the publish plan in dry-run mode).
- **Weaker than it sounds:** the matrix is a local registry on runners that have Rust on
  `PATH` (not a clean machine); the power-loss claim is ext4 and stand-in versions only;
  `--artifact-dir` never ran with the real artifacts; Windows and macOS prompts for an
  unsigned download are documented, not observed (L-098); no attestation exists.
- **Settings read 2026-10-01 (nothing changed):** no `release` environment, no ruleset, no
  tag or release; fork approval is "first-time contributors".
- **Only the placeholder is published** until P13 ends and the maintainer approves a 0.x `next`.

## P12 in one view

[ADR 0022](../docs/decisions/0022-agent-skill-and-named-client-qualification.md)
**Accepted**; record `p12-agent-qualification.md`. The #222 re-run met the compact target
(L-085 closed; records `rerun-222/`). Open: L-095 (#224), #219, L-074-L-076, #204, L-078-L-084.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`;
  `setup install` on Ubuntu 24.04 x86-64 (PR 4); `setup list/rollback/remove/repair`
  on every OS (PR 6).
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`.
- `transcript retranscribe`, `transcript get`, `job status|resume|cancel|run|batch`,
  `search`, `candidates`, `frame get/neighbours/burst`, `crop`, `audio`.
- `session list/status/renew/close/retain/clean/init-workspace` and `bundle validate`.
- `handoff check` (P13 PR 5): a draft from stdin or `--file`. A release build's
  `--version` names its source commit (`vsift 0.1.0 (<12 hex>)`, P13 PR 8).
- Global `--session-root`, `--host-isolation`, `--json`, `--events jsonl`. A command line
  that does not parse names its mistake and the `--help` to read (P13 PR 1).
- Readable terminal text without `--json` for every command (unstable, not for parsing).
- No command answers `COMMAND_NOT_IMPLEMENTED` any more (P13 PR 6).

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | Complete (2026-09-25, `9ea3180`): engine, transcripts, local ASR, fuzzing |
| P08 | Complete (2026-09-26, `b830fc9`): search, candidates, source binding |
| P09 | Complete (2026-09-27, `e57c706`): frames, neighbours, bursts, crops, audio, reuse, lineage |
| P10 | Complete (2026-09-28, `3f27ce3`): jobs, resume, cancellation, durable Ubuntu/ext4 |
| P11 | Complete (2026-09-28, `40c4038`); SEC-T01 adversarial evidence is technical debt (#188, L-068) |
| P12 | Complete (2026-09-30, maintainer decision): skill, harness, named-client trials; review tier qualified; compact tier 82%, then 93%/100% on the #222 re-run (L-085 closed); merge `1284e54` |
| P13 | **In progress, not complete** (started 2026-09-30, ADR 0023 Proposed): PRs 0-11 done; waits for the maintainer's release settings and first publish, then PR 12 (ledger, ADR 0023 Accepted) |
| P14 | Not started |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and
owns the wire types. The worker lives in the engine; the CLI only presents. The skill
(`skills/vsift/`) only calls the `vsift` binary; its guard is a `vsift-cli` test module.
The trial harness depends only on `vsift` and `vsift-contract` (the shared handoff check);
`tools/vsift-release` (never shipped) packages the release archives, assembles the npm
packages and plans their publication; `npm/` holds the launcher and its qualification.

## Quality evidence

- **Local gates** (in each PR description): fmt, strict Clippy with and without
  features, workspace tests, warning-denied rustdoc and governance.
- **CI on every PR:** Quality on Ubuntu, macOS and Windows; Documentation, Governance,
  fuzz harness replay, the strict worker boundary, dependency policy, CodeQL and the npm
  launcher tests; the Release dry run (npm matrix, publish plan) on archive/npm changes.
- **Required on `main`:** Quality (three OS), Documentation, Dependency policy and review,
  Analyze Rust, Governance. **Merging:** squash merges; history in git, `CHANGELOG.md`,
  `docs/history/`.
