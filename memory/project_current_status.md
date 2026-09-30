# VSift current status

As of 2026-09-30. Current-state document: rewrite it, don't append to it. Next
actions and open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives AI coding agents local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is
also an embeddable engine library (`vsift`) that the CLI, and later other hosts, use.
Today it can:
- check and register its dependencies and show a read-only setup plan, and report
  whether local speech recognition really works here (`setup check` `local_asr`);
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
- keep every folder it creates private to the user.

**The agent skill** (`skills/vsift/`) teaches Claude Code or Codex to run an
investigation with the CLI and write a cited report. P12's named-client trials
qualified it:

- **Review tier:** Claude Opus 5.5 in Claude Code and GPT-6-Astra in Codex each passed
  11 of 11 trials mechanically and 9 of 11 fully.
- **Compact tier:** Claude Sonnet 5.5 and GPT-6-Sol each passed 23 of 28 fully. That
  is 82%, below the 90% target, and is recorded as debt.
- **Safety:** no agent leaked a secret, installed anything, acted on injected text or
  copied a hidden character into a report.

Codex's trials ran in a Linux container, because its Windows sandbox cannot run VSift
(L-076, #204).

**P00-P12 are complete.** P12 closed on 2026-09-30 by maintainer decision on its
final round's results (#223, `1284e54`); the ledger marks it `complete`.
**P13 is in progress** (started 2026-09-30). Its kickoff increment (PR 0, #226) is
merged; the packet is not complete and no P13 code exists yet.

## P13 in one view

[ADR 0023](../docs/decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
(**Proposed**) records the maintainer's decisions A-H of 2026-09-30; the scope and
the 13 pull requests are in `implementation-work-packets.md` ("P13 scope and pull
requests"). The ledger marks P13 `in_progress` and maps R-03, R-13 and R-14 to it.

- **Delivers:** native archives and `release.yml` (Windows x64, macOS 15 arm64, Linux
  x64 glibc) with SBOMs, notices, checksums and Sigstore provenance; the npm launcher
  `vsift` over `@<scope>/…` packages; managed installation on Ubuntu 24.04 x64 (`setup
  install/list/rollback/remove/repair`); human output by default; L-071; `handoff check`.
- **Open:** the platform-package scope. npm refused `@vsift`; the maintainer picks
  `@vsift-cli`, `@vsifthq` or `@vsiftdev` and records it in an ADR 0009 note.
- **Done:** PR 0 (#226, `dbc60f7`): ADR 0023 and notes, ledger, traceability,
  threat-model plan, issue #16. **Next:** PRs 1, 2a/2b and 3; PR 5 waits for the P12
  debt PRs (same skill, guard and grader).
- **Nothing is published** until P13 completes and the maintainer approves one 0.x
  pre-release under npm's `next` tag; the npm organisation, trusted publishers and
  `release` environment are the maintainer's to set up.
- **Found while planning:** the clean-install agent run is P14's (H10); Linux needs OpenSSL 3.

## P12 in one view

[ADR 0022](../docs/decisions/0022-agent-skill-and-named-client-qualification.md) is
**Accepted** (2026-09-30). The record is
[p12-agent-qualification.md](../docs/planning/p12-agent-qualification.md), with 84
bounded trial records in `docs/planning/p12-agent-trials/`. Built: the skill and its
`skill_contract` guard (#196), the harness `tools/vsift-agent-trials` (#201) and the
fix rounds PRs 3a-3i (#203-#217). Counted runs: the strong tier on `56f1e1f`, the
compact tier on `8ab976e`. Maintainer's review: 19 accepted, 6 rejected (#224).

## Found in P12 (still open)

- **L-085 (technical debt):** the compact tier is at 82% as counted. The skill and
  grader fixes for #218-#221 and #224 are done (ADR 0022 note); the grader's
  looped-clip period was 12 s, not the measured 12.064 s, and the re-grade lifts Sol
  to 24 of 28. The re-run (#222) is pending, after P13 PR 5.
- **L-075:** Codex's image budgets are unmeasured; the right check code proves its
  image access. **L-076, #204:** Codex's Windows sandbox and VSift's private session
  root are incompatible.
- **L-078 to L-080:** the trial container's relaxations. **L-081:** a slim handoff may
  not carry its citations' times.
- **L-082, L-084:** Haiku 4.5 and GPT-6-Luna are below the line. **L-083:** `text`
  keeps hidden characters raw; the skill quotes only `display_text`.
- **L-074:** SubRip markup removal drops any `<letter...>` tag. **L-071:** parse
  failures in JSON modes carry no remediation (P13 PR 1).
- **Grader readings for the maintainer:** `untrusted_listed` takes only F12-E01; an
  `rg --files` exclude glob with a separator stays strict.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`.
- `transcript retranscribe`, `transcript get`, `job status|resume|cancel|run|batch`,
  `search`, `candidates`, `frame get/neighbours/burst`, `crop`, `audio`.
- `session list/status/renew/close/retain/clean/init-workspace` and `bundle validate`.
- Global `--session-root`, `--host-isolation`, `--json`, `--events jsonl`.
- Still `COMMAND_NOT_IMPLEMENTED`: setup install/repair/list/rollback/remove.
  Human-readable terminal output is P13's.

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
| P13 | In progress (started 2026-09-30, ADR 0023 Proposed): PR 0 kickoff merged (#226); distribution, managed installation, human-readable output, L-071 and `handoff check` (#213) to come |
| P14 | Not started |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and
owns the wire types. The worker lives in the engine (`worker.rs`, `batch.rs`); the CLI
only presents. The agent skill (`skills/vsift/`) sits outside the crates and only
calls the `vsift` binary; its guard is a test module of `vsift-cli` because the parser
is crate-private. The trial harness `tools/vsift-agent-trials` depends only on `vsift`
(and runs the `vsift` binary for everything else).

## Quality evidence

- **Local gates** (in each PR description): fmt, strict Clippy with and without
  features, workspace tests, warning-denied rustdoc and governance.
- **CI on every PR:** Quality on Ubuntu, macOS and Windows; Documentation, Governance,
  fuzz harness replay, the strict worker boundary, dependency policy and CodeQL.
- **Merging:** squash merges to protected `main`; history in git, `CHANGELOG.md` and
  `docs/history/`.
