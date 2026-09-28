# VSift current status

As of 2026-09-28. Current-state document: rewrite it, don't append to it. Next
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

There is now also an **agent skill** (`skills/vsift/`) that teaches Claude Code or
Codex to run an investigation with the CLI and write a cited report, and a trial
harness that runs and grades those clients on synthetic recordings. Three dry trials
have run (two Claude Code, one Codex); PR 3a fixed what the first pair showed, PR 3c
what the second Claude run showed, and Codex's Windows sandbox cannot run VSift
(L-076, maintainer decision; PR 3b builds a Linux container). No counted trial has
run, so the skill is a candidate, not a qualified integration.

**P00-P11 are complete** (P11 closed 2026-09-28, merge `40c4038`; SEC-T01's
adversarial evidence is technical debt, #188, L-068). **P12 (agent skill) is in
progress.** PR 1, PR 2, PR 3a and PR 3c are increments, not the packet.

## P12 in one view

[ADR 0022](../docs/decisions/0022-agent-skill-and-named-client-qualification.md) is
**Proposed** (for maintainer review).

- **PR 1 (#196, `7990fdf`, merged): skill and contract guard.** `skills/vsift/`:
  `SKILL.md` (eight states, each with allowed commands and a stopping condition),
  `references/` (`commands.md` classes every public command `free`, `explicit` or
  `never`; `budgets.md` `compact` default and `standard`; `handoff.md`; `safety.md`;
  `resume.md`; `lifecycle.md`), `handoff.schema.json` (handoff v1, owned by the
  skill), two example handoffs, the image-check picture and Codex metadata. Guard
  `crates/vsift-cli/src/skill_contract.rs`: commands parse with the real
  parser and respect their class, names resolve in the schemas, examples validate,
  the image code is in no text, `SKILL.md` within 300 lines, FIND_SPOKEN_SPANS
  searches first. Install guide `docs/agents/skill.md`.
- **PR 2 (#201, `9d2f60e`, merged): trial machinery.** `tools/vsift-agent-trials`
  (unpublished): `prepare` (neutral root, skill in both clients' folders, isolated
  per-user base, clips, expired session, interrupted job, installer and canaries),
  `run` (explicit executable and arguments, cleared environment, timeout, raw logs),
  `grade` (mechanical and interpretation results; policy and budgets parsed from the
  skill, truth from the manifest), `record` (at most 64 KiB); 21 scenarios; runbook
  `docs/agent- **First dry trials (2026-09-28):** Claude Code (`claude-opus-5-5`) ran without the
  workspace's allow rules and never ran `search`; Codex had every command rejected.
- **PR 3a (#203, `67d56c3`, merged):** `run` trusts each Claude Code workspace in the
  client home's `.claude.json` (`claude_trust.rs`) and drops `--settings`; grader check
  `client_configuration` makes a trial invalid when a client says it ignored its
  configuration; Codex on Windows gets `windows.sandbox="unelevated"` (still unusable
  for VSift, L-076); FIND_SPOKEN_SPANS always starts with `vsift search`.
- **Second Claude dry trial (2026-09-28):** valid, correct report; mechanical fail on
  `command_policy` (it chained `date +%s` to time itself) and
  `citation_times_in_truth_windows` (frames at 9 s and 19 s show `INVOICE 4407`, but
  the manifest placed it only in F05-E01, 0-5 s). It also first sent the invalid
  `--operation-id op-asr-walkthrough-1`.
- **PR 3c (branch `p12-pr3c-dryrun2-fixes`, this change):**
  - Skill: nothing but `vsift`, one command per call, never chained or piped except
    `| tail -n 1` after `--events jsonl`; the host keeps the wall time and handoff v1
    takes `null` for unmeasured `wall_time_s` (L-077); the operation-id grammar and a
    valid example where `SKILL.md` first uses one. Guard now 15 tests.
  - Corpus truth amended (reviewed in the corpus README): event kind `persistent`
    (never critical, never a generated scene, not scored by recall, in no scenario's
    `truth_events`): F04-E05 header, F05-E04 invoice 4407, F12-E03 SAFE-12. The P04
    generator reproduced every file byte for byte; the four generation records name
    the new manifest digest; both verifiers passed. Grader unchanged; regression tests
    pin the dry trial's citations and a term cited only where it is absent.
- **Next:** the maintainer re-runs the Claude dry trial on PR 3c, decides L-076, then
  the counted trials. The packet completes only when the named-client trials pass.

## Found in P12

- **L-076 (open, decision):** Codex's unelevated Windows sandbox and VSift's private
  session root are incompatible; the network is off only through proxy variables.
- **L-074 (open):** SubRip markup removal drops any `<letter...>` tag, broader than the
  contract's list; `original_text` keeps the payload (found by the SEC-T02 suite).
- **Open grader readings (PR 3c):** `untrusted_listed` takes only F12-E01 (0-8 s)
  though the on-screen instructions last to 12 s; a frame before 8 s binds `install`.
- **#197 (fixed, #200 `98525dc`):** a torn session-index marker no longer fails later
  listings; the P10 durability campaign passed on the fix (run 36447992132).
- **L-071:** parse failures in JSON modes carry no remediation; the skill tells agents
  to quote `--rect` on PowerShell and check commands against its reference.

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
| P12 | In progress: PR 1 (skill, guard), PR 2 (harness) and PR 3a (dry-trial fixes) merged; PR 3c (second dry-trial fixes, truth amendment) in review; L-076 decision, then the counted trials |
| P13 | Not started; also delivers managed installation and human-readable output. Its plan now fixes the npm launcher pattern and a name checklist (2026-09-28) |
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

- P12 PR 3c gates on Windows 11 (fmt, strict Clippy with and without features,
  workspace tests, warning-denied rustdoc, governance), both corpus verifiers and the
  dry-trial evidence go in the pull request description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
