# VSift current status

As of 2026-09-29. Current-state document: rewrite it, don't append to it. Next
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
harness that runs and grades those clients on synthetic recordings. Dry trials found
and fixed configuration, skill-wording and answer-key problems (PR 3a, PR 3c, PR 3d).
Codex trials run in a Linux container (PR 3b, merged), because Codex's Windows
sandbox cannot run VSift (L-076, #204). Two diagnostic passes (39 Claude Code runs,
11 Codex runs, none counted) showed what small models still get wrong and three
grader false positives; PR 3e (in review) fixes all of it, so the counted campaigns
restart from zero on PR 3e. Until the named-client trials pass, the skill is a
candidate, not a qualified integration.

**P00-P11 are complete** (P11 closed 2026-09-28, merge `40c4038`; SEC-T01's
adversarial evidence is technical debt, #188, L-068). **P12 (agent skill) is in
progress.** PRs 1, 2, 3a, 3b, 3c, 3d and 3e are increments, not the packet.

## P12 in one view

[ADR 0022](../docs/decisions/0022-agent-skill-and-named-client-qualification.md) is
**Proposed** (for maintainer review).

- **PR 1 (#196, `7990fdf`, merged): skill and contract guard.** `skills/vsift/`
  (`SKILL.md` with eight states, `references/` classing every command `free`,
  `explicit` or `never`, budgets, handoff v1 schema, examples, image-check picture);
  guard `crates/vsift-cli/src/skill_contract.rs`; install guide `docs/agents/skill.md`.
- **PR 2 (#201, `9d2f60e`, merged): trial machinery.** `tools/vsift-agent-trials`
  (unpublished): `prepare`, `run`, `grade` (mechanical and interpretation results;
  policy and budgets parsed from the skill, truth from the manifest), `record` (at most
  64 KiB); 21 scenarios; runbook `docs/agents/trials.md`; SEC-T02 suite.
- **PR 3a (#203), 3c (#207), 3d (#208, `ed07c0d`), merged, after dry trials:** trusted
  Claude Code workspaces and one settings source; `client_configuration` invalidates a
  misconfigured trial; search first; nothing but `vsift`, one command per call;
  `wall_time_s` may be `null` (L-077); operation-id grammar; persistent truth events
  (F04-E05, F05-E04, F12-E03); `Glob`/`Grep`/`LS` inside the skill folders are reads.
- **PR 3b (#209, `3f91661`, merged): Codex in Linux.** A pinned two-image `Dockerfile`
  (`agent`: tools only; `harness`: plus the repository), a seccomp profile for
  bubblewrap (L-078), `trial-driver.sh`, `codex-trial.ps1`; three containers per trial
  (`prepare`, `run` with only its trial folder and a tmpfs sign-in copy, `grade`); CI
  workflow `p12-codex-container.yml`.
- **Diagnostic passes (2026-09-29, not counted):** Claude Code on `ed07c0d` (Opus on
  A-08/A-09 passed but for the blurred scenario; Haiku on A-01..A-07 and SEC-T02
  mostly failed) and Codex on image `f2dfb955790f` (11 runs). Raw logs stay local.
- **PR 3e (branch `p12-pr3e-diagnostic-fixes`, this change): every diagnostic fix.**
  Grader: blurred scenario declares `blurred_terms`; the bare `\\?\` prefix is not a
  path; Claude Code's spill files are housekeeping; shell `rg`/`grep` in the skill
  folders are skill reads; Codex's right check code proves image access (image budgets
  unmeasured, L-075); Codex `error` notices about configuration invalidate a trial;
  `--help` forms are free, never piped; `cd` stays unauthorized. Harness: `--disable
  view_image`, `gblur`, `grade --output/--repository/--scenario/--client-home`,
  `codex-trial.ps1` `trial-id` line, `debug -Scenario`, `regrade`. Skill: every stop
  ends in REPORT with an inline minimal handoff, no files, no `cd`, compact limits as
  numbers, `setup check` only, no web addresses, a before-you-send checklist; guard
  tests for all of it. Evidence: 2 `gpt-6-luna` debug runs; every diagnostic run
  re-graded beside its original (table in the pull request).
- **Next:** merge PR 3e, then the counted Claude Code and Codex campaigns from zero on
  its merge commit. The packet completes only when the named-client trials pass.

## Found in P12

- **L-075 (decided 2026-09-29):** codex-cli 0.155 shows no viewed image, in its
  stream or countably in its rollout; the right check code proves Codex's image
  access, and its image budgets are unmeasured.
- **L-076 (trials avoid it; #204 open):** Codex's Windows sandbox and VSift's private
  session root are incompatible. **L-078..L-080:** the container's seccomp relaxation,
  unrestricted container egress, and the agent's read access to its sign-in.
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
| P12 | In progress: PRs 1, 2, 3a, 3b, 3c and 3d merged; 3e (diagnostic fixes) in review; the counted campaigns for both clients restart from zero on 3e |
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

- P12 PR 3e gates on Windows 11 (fmt, strict Clippy with and without features,
  workspace tests, warning-denied rustdoc, governance), the image digests, the debug
  runs and the re-grade table go in the pull request description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
