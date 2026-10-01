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
- be installed without Rust: the release workflow builds its archives for the three R0
  targets (P13 PR 8) and the npm packages (P13 PR 9), and **the 0.1.0 pre-release is
  published** (2026-10-01): `npm install --global vsift-cli@next`, and archives on a GitHub
  pre-release, each with a Sigstore attestation and npm provenance.

**The agent skill** (`skills/vsift/`) teaches Claude Code or Codex to run an
investigation with the CLI and write a cited report. P12's named-client trials
qualified it:

- **Review tier:** Claude Opus 5.5 in Claude Code and GPT-6-Astra in Codex each passed
  11 of 11 trials mechanically and 9 of 11 fully.
- **Compact tier:** Claude Sonnet 5.5 and GPT-6-Sol: 82% in P12; the re-run after the
  fixes (#222) meets the 90% target, Sonnet 26 of 28 and Sol 28 of 28 (23 as run).
- **Safety:** no agent leaked a secret, installed anything, acted on injected text or
  copied a hidden character into a report. Codex ran in a Linux container (L-076, #204).

**P00-P13 are complete; P14 is not started.** P12 closed on 2026-09-30 by maintainer
decision (#223, `1284e54`). P13 closed on 2026-10-01 with the publish of the 0.1.0
pre-release; the ledger marks it `complete` with the release commit `011bc4d`. P14, the
release qualification, does not start automatically (governance rule 10): the maintainer
starts it. What remains is in `memory/TODO.md`.

## P13 in one view (complete)

[ADR 0023](../docs/decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
(**Accepted** 2026-10-01) records the maintainer's decisions A-H of 2026-09-30; the scope and
the 13 pull requests are in `implementation-work-packets.md` ("P13 scope and pull
requests"). The ledger maps R-03, R-13 and R-14 to it. **The record
`docs/planning/p13-distribution.md` has the pull requests with merge commits, the evidence
per ID, what each proves and does not, and the account of the first publish, including the
attempt that failed.** The guide for users is `docs/operations/install.md`; the maintainer's
runbook is `docs/operations/release.md`.

- **Delivered:** native archives and `release.yml` (Windows x64, macOS 15 arm64, Linux x64)
  with SBOMs, notices, checksums and Sigstore provenance; the npm launcher over `@vsift/...`
  packages; managed installation on Ubuntu 24.04 x64; human output; `handoff check`; 0.1.0.
- **Names:** scope `@vsift` (owned); launcher package `vsift-cli` (npm refused `vsift`;
  command `vsift`). All four packages hold `0.0.0` placeholders as `latest` and 0.1.0 as
  `next`; the three platform placeholders were published by the maintainer (path A).
- **The first publish (2026-10-01):** tag `v0.1.0` (annotated, on `011bc4d`); dry run 36919612380;
  the first real dispatch, 36922901956, failed with `ENEEDAUTH` before anything was
  published (the maintainer then reported the npm-to-GitHub trusted-publisher connections
  had not been completed; which setting was wrong is not known; L-100); the second,
  36931487439, published all four packages by trusted publishing, no token, then the release.
- **Verified** (one session, the maintainer's Windows 11 machine, no npm login): `npm audit
  signatures` 4 and 4; `npx vsift --version` printed `vsift 0.1.0 (011bc4da1af6)`; checksums
  OK; `gh attestation verify` 10 of 10 files and 4 of 4 tarballs; the builds are reproducible.
- **Earlier hosted evidence** (Ubuntu 24.04): `P13 managed smoke` runs 36734316384 and
  36793180858; `P13 managed power loss` run 36829198545 passed (0 lost, 0 damaged; negative
  control 36 lost) after a first run failed on a verifier defect fixed in #244; the
  twelve-job npm matrix passed on `main` and on the tag.
- **Weaker than it sounds:** the matrix uses a local registry on runners that have Rust on
  `PATH`; the real registry was tried once (npm, Windows 11, a development machine), not
  with pnpm, Yarn or Bun, not on a clean machine; the power-loss claim is ext4 and
  stand-in versions only; `--artifact-dir` never ran with the real artifacts; no archive
  was extracted and run; Smart App Control and the macOS prompts are untried (L-098).
- **Settings (read back 2026-10-01):** the `release` environment, the tag ruleset and fork
  approval. npm's trusted publishers and "disallow bypass 2FA tokens" cannot be read from
  outside (the maintainer's report; the second run working is the proof for the first).

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
| P12 | Complete (2026-09-30, maintainer decision; ADR 0022 Accepted, record `p12-agent-qualification.md`): skill, harness, named-client trials; review tier qualified; compact tier 82%, then 93%/100% on the #222 re-run (L-085 closed); open: L-095 (#224), #219, L-074-L-076, #204, L-078-L-084; merge `1284e54` |
| P13 | **Complete** (2026-10-01, ADR 0023 Accepted): distribution, managed install, `handoff check`, human output; the 0.1.0 pre-release published; release commit `011bc4d` |
| P14 | **Not started**; the maintainer starts it (governance rule 10) |

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
