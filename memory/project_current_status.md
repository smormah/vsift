# VSift current status

As of 2026-09-30. Current-state document: rewrite it, don't append to it. Next
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
  list/rollback/remove/repair`, P13 PR 6, in review);
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
  `job batch`), each step at most once per operation id, with weighted admission and a
  shutdown that leaves every started request resumable;
- on Ubuntu 24.04 with local ext4, keep every acknowledged result of a durable
  workspace through an OS crash or power loss;
- refuse to claim strict worker isolation unless the Linux kernel attests it;
- keep every folder it creates private to the user;
- check an agent's draft report before it is sent (`handoff check`, P13 PR 5);
- install its media tools and speech model itself on Ubuntu 24.04 x86-64 (P13 PR 4);
- build, in CI, its own release archives for the three R0 targets (P13 PR 8; unpublished).

**The agent skill** (`skills/vsift/`) teaches Claude Code or Codex to run an
investigation with the CLI and write a cited report. P12's named-client trials
qualified it:

- **Review tier:** Claude Opus 5.5 in Claude Code and GPT-6-Astra in Codex each passed
  11 of 11 trials mechanically and 9 of 11 fully.
- **Compact tier:** Claude Sonnet 5.5 and GPT-6-Sol each passed 23 of 28 fully. That
  is 82%, below the 90% target, and is recorded as debt.
- **Safety:** no agent leaked a secret, installed anything, acted on injected text or
  copied a hidden character into a report. Codex ran in a Linux container (L-076, #204).

**P00-P12 are complete.** P12 closed on 2026-09-30 by maintainer decision on its
final round's results (#223, `1284e54`); the ledger marks it `complete`.
**P13 is in progress** (started 2026-09-30). PRs 0-5, PR 8 and the P12 debt fixes are
merged (human output, `handoff check`, `setup install`, `release.yml` and the workflow
lint are done); PR 6 (managed lifecycle) is in review. The packet is not complete.

## P13 in one view

[ADR 0023](../docs/decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
(**Proposed**) records the maintainer's decisions A-H of 2026-09-30; the scope and
the 13 pull requests are in `implementation-work-packets.md` ("P13 scope and pull
requests"). The ledger marks P13 `in_progress` and maps R-03, R-13 and R-14 to it.

- **Delivers:** native archives and `release.yml` (Windows x64, macOS 15 arm64, Linux
  x64 glibc) with SBOMs, notices, checksums and Sigstore provenance; the npm launcher
  `vsift` over `@vsift/…` packages; managed installation on Ubuntu 24.04 x64 (`setup
  install/list/rollback/remove/repair`); human output by default; `handoff check`.
- **Names (#237):** scope `@vsift` (owned); the placeholder `vsift@0.0.0` (ADR 0009 note).
- **Done:** PR 0 (#226, ADR 0023); PR 1 (#228, parse remediation, L-071); PRs 2a, 2b
  (#229, #231, `TerminalText`, SEC-T02, L-073); PR 3 (#230, smoke and failure cleanup);
  PR 5 (#233, `handoff check` shared with the grader, `regex` in production, L-086).
- **PR 4 (#234, `d43a518`):** `setup install --plan --accept-plan [--artifact-dir]`:
  HTTPS download or import with exact size and SHA-256, stage, smoke,
  `publish_and_select`, per component; `DOWNLOAD_FAILED` (exit 7); lookup order per
  call, configured, managed, `PATH`. `P13 managed smoke` run 36734316384 (hosted Ubuntu
  24.04, 2026-09-30) passed both jobs: real plan, install, check and rerun activated all
  three components; the negative control activated nothing; warm `setup check` 0.65 s,
  `frame get` 0.29 s then 0.17 s (L-087 stays monitoring).
- **PR 8 (#236, `772ead2`):** `release.yml` builds `vsift` per target twice (identical
  bytes), `--version` names the commit, notices and a CycloneDX SBOM per target,
  deterministic `.tar.gz` archives and `SHA256SUMS` (`tools/vsift-release`); no write
  scope, OIDC token or secret. The governance checker lints every workflow. L-089.
- **In review: PR 6** (`p13-pr6-lifecycle`, steps 5-6). `setup list` and `setup repair`
  only read (repair names the existing command that fixes each finding); the skill
  classes both `free`. `setup rollback <component> [--version]` selects the previous (or
  a named) version only if it verifies, in one atomic rename (pointer v2). `setup remove
  <component> [--version] | --stale-stages` deselects first, never removes a version a
  job holds (`BUSY`), keeps content it cannot prove its own (L-090); removal proves
  ownership, not integrity. Every accepted install sweeps abandoned stages first and
  keeps the selected and previous versions. Works on every OS. **Next:** PR 7 (kill and
  power-loss tests at the crash points in ADR 0023's PR 6 note).
- **Only the placeholder is published** until P13 ends and the maintainer approves a 0.x `next`.

## P12 in one view

[ADR 0022](../docs/decisions/0022-agent-skill-and-named-client-qualification.md)
**Accepted**; record `docs/planning/p12-agent-qualification.md` (84 trial records): the
skill and guard (#196), harness `tools/vsift-agent-trials` (#201), fixes #203-#217.
- **L-085 (debt):** compact tier 82%; fixes for #218-#221 and #224 done, the re-run
  (#222) is due now. **Also open:** L-074, L-075, L-076 and #204, L-078 to L-084;
  grader readings (`untrusted_listed`, an `rg --files` exclude glob) in `memory/TODO.md`.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`;
  `setup install` on Ubuntu 24.04 x86-64 (PR 4); `setup list/rollback/remove/repair`
  on every OS (PR 6, in review).
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
| P12 | Complete (2026-09-30, maintainer decision): skill, harness, named-client trials; review tier qualified, compact tier 82% (debt, L-085); merge `1284e54` |
| P13 | In progress (started 2026-09-30, ADR 0023 Proposed): PRs 0-5 and 8 merged (#226, #228-#231, #233, #234, #236); PR 6 (managed lifecycle) in review; kill/power-loss tests, npm and publishing to come |
| P14 | Not started |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and
owns the wire types. The worker lives in the engine; the CLI only presents. The skill
(`skills/vsift/`) only calls the `vsift` binary; its guard is a `vsift-cli` test module.
The trial harness depends only on `vsift` and `vsift-contract` (the shared handoff check);
`tools/vsift-release` (never shipped) packages the release archives.

## Quality evidence

- **Local gates** (in each PR description): fmt, strict Clippy with and without
  features, workspace tests, warning-denied rustdoc and governance.
- **CI on every PR:** Quality on Ubuntu, macOS and Windows; Documentation, Governance,
  fuzz harness replay, the strict worker boundary, dependency policy and CodeQL; the
  Release dry run when an archive input changes.
- **Merging:** squash merges to protected `main`; history in git, `CHANGELOG.md`, `docs/history/`.
