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
**P13 is next;** the maintainer starts it.

## P12 in one view

[ADR 0022](../docs/decisions/0022-agent-skill-and-named-client-qualification.md) is
**Accepted** (2026-09-30), with a dated note listing the maintainer's P12 decisions.
The record is
[p12-agent-qualification.md](../docs/planning/p12-agent-qualification.md), and 84
bounded trial records are in `docs/planning/p12-agent-trials/`.

- **What was built:** the skill and its `skill_contract` guard (PR 1, #196); the
  harness `tools/vsift-agent-trials` (PR 2, #201); and PRs 3a-3i (#203-#217), which
  fixed each trial round's findings: trusted Claude Code workspaces, Codex in a Linux
  container, orientation as housekeeping, the slim handoff and its vocabulary,
  `display_text`, a redrawn check image and resume guidance. The history table is in
  the record.
- **Counted runs:** the strong tier's final campaign on `56f1e1f` (graded with PR 3i's
  grader) and the compact tier's final round on `8ab976e`, with Claude Code 2.1.284 on
  Windows and codex-cli 0.155.0-alpha.16 in Docker images pinned by digest.
- **Completion change (branch `p12-completion`):**
  - the record, the trial records and ADR 0022 accepted;
  - `verification.md` §6's outcome;
  - known limits: L-085 new, L-039 deleted, and L-007 and L-075 rewritten;
  - the supported-models table in `docs/agents/skill.md`;
  - issues #218-#222 and #224 (blurred-content overclaim);
  - the maintainer's review (2026-09-30): 19 accepted, 6 rejected;
  - `record` now replaces every check image's code with `<check-code>`.
- **Open:**
  - the compact tier's debt (L-085).

## Found in P12 (still open)

- **L-085 (technical debt):** the compact tier is at 82%. The causes:
  - invented claim shapes (#218);
  - A-02 resume, Sonnet 2 of 3 and Sol 1 of 3 (#219);
  - Sol's SEC-T02 slips (#220);
  - the `hxxps://` form in JSON (#221).

  Next, the re-run (#222) before P14.
- **L-075:** Codex's image budgets are unmeasured; the right check code proves its
  image access. **L-076, #204:** Codex's Windows sandbox and VSift's private session
  root are incompatible.
- **L-078 to L-080:** the trial container's relaxations. **L-081:** a slim handoff may
  not carry its citations' times.
- **L-082, L-084:** Haiku 4.5 and GPT-6-Luna are below the line. **L-083:** `text`
  keeps hidden characters raw; the skill quotes only `display_text`.
- **L-074:** SubRip markup removal drops any `<letter...>` tag. **L-071:** parse
  failures in JSON modes carry no remediation (P13).
- **Grader readings for the maintainer:**
  - `untrusted_listed` takes only F12-E01;
  - an `rg --files` exclude glob with a separator stays strict;
  - A-02's windows cover only the first loop.

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
| P13 | Next, not started: distribution, managed installation, human-readable output, the handoff validator (#213); npm launcher pattern and name checklist planned (2026-09-28) |
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

- **The P12 completion change's gates** on Windows 11 are in the pull request
  description: fmt, strict Clippy with and without features, workspace tests,
  warning-denied rustdoc and governance.
- **CI on every PR:**
  - Quality on Ubuntu, macOS and Windows;
  - Documentation, Governance, fuzz harness replay, the strict worker boundary,
    dependency policy and CodeQL.
- **Merging:** squash merges to protected `main`. History is in git, `CHANGELOG.md`
  and `docs/history/`.
