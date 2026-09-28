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
harness that runs and grades those clients on synthetic recordings. One dry trial per
client has run; they exposed three problems that PR 3a fixes, and a fourth (Codex's
Windows sandbox cannot run VSift, L-076) that waits for a maintainer decision. No
counted trial has run, so the skill is a candidate, not a qualified integration.

**P00-P11 are complete** (P11 closed 2026-09-28, merge `40c4038`; SEC-T01's
adversarial evidence is technical debt, #188, L-068). **P12 (agent skill) is in
progress.** PR 1, PR 2 and PR 3a are increments, not the packet.

## P12 in one view

[ADR 0022](../docs/decisions/0022-agent-skill-and-named-client-qualification.md) is
**Proposed** (for maintainer review).

- **PR 1 (#196, `7990fdf`, merged): skill and contract guard.** `skills/vsift/`:
  `SKILL.md` (eight states, each with allowed commands and a stopping condition),
  `references/` (`commands.md` classes every public command `free`, `explicit` or
  `never`; `budgets.md` `compact` default and `standard`; `handoff.md`; `safety.md`;
  `resume.md`; `lifecycle.md`), `handoff.schema.json` (handoff v1, owned by the
  skill), two example handoffs, the image-check picture and Codex metadata. Guard
  `crates/vsift-cli/src/skill_contract.rs` (now 12 tests): commands parse with the real
  parser and respect their class, names resolve in the schemas, examples validate,
  the image code is in no text, `SKILL.md` within 300 lines, FIND_SPOKEN_SPANS
  searches first. Install guide `docs/agents/skill.md`.
- **PR 2 (#201, `9d2f60e`, merged): trial machinery.** `tools/vsift-agent-trials`
  (unpublished): `prepare` (neutral root, skill in both clients' folders, isolated
  per-user base, clips, expired session, interrupted job, installer and canaries),
  `run` (explicit executable and arguments, cleared environment, timeout, raw logs),
  `grade` (mechanical and interpretation results; policy and budgets parsed from the
  skill, truth from the manifest), `record` (at most 64 KiB); 21 scenarios; runbook
  `docs/agents/trials.md`; SEC-T02 tool suite; procedure checkpoint. L-072..L-075.
- **Dry trials (2026-09-28, maintainer):** one A-08 run per client, kept locally.
  Claude Code (`claude-opus-5-5`) produced a correct report but ran without the
  workspace's allow rules (untrusted workspace; the rules came from a duplicate
  `--settings` copy) and never ran `search`. Codex (`gpt-6-astra`) had every command
  rejected ("blocked by policy") and stopped after 27 s.
- **PR 3a (branch `p12-pr3a-dryrun-fixes`, this change): dry-trial fixes.**
  - `run` trusts each Claude Code workspace in the client home's `.claude.json`
    (`claude_trust.rs`: one key, other members copied byte for byte, atomic rename,
    never logged) and drops `--settings`: the project file is the one source. A real
    run then showed no warning, `vsift` allowed, `mkdir` and a `.env` read denied.
  - Grader check `client_configuration` (`client_warnings.rs`): a client's own
    report that it ignored settings, permissions, sandbox or skill makes the trial
    invalid (`invalid_reasons`, record `"valid": false`). Re-grading the Claude dry
    run gives `INVALID TRIAL` with only that check and `commands_required` failing.
  - Codex on Windows gets `windows.sandbox="unelevated"` and no `TEMP` or `/tmp`
    writable root: commands run, writes stay in the workspace. But VSift cannot use its
    private session root inside that sandbox and the network is not enforced (L-076):
    **Codex trials on Windows need a maintainer decision** (WSL/Ubuntu,
    `danger-full-access` graded only, or the elevated sandbox with admin setup).
  - Skill: FIND_SPOKEN_SPANS always starts with `vsift search`; new guard test.
  - Budget used: 3 Codex `exec` and 3 Claude Code `-p` calls, small models.
- **Next:** the maintainer re-runs the Claude dry trial, decides L-076, then the
  counted trials. The packet completes only when the named-client trials pass.

## Found in P12

- **L-076 (open, decision):** Codex's unelevated Windows sandbox and VSift's private
  session root are incompatible; the network is off only through proxy variables.
- **L-074 (open):** SubRip markup removal drops any `<letter...>` tag, broader than the
  contract's list; `original_text` keeps the payload (found by the SEC-T02 suite).
- **L-070 (fixed, #199):** the `job resume` remediation for a closed or expired session
  now says to open a new session with `ingest`.
- **#197 (fixed, #200 `98525dc`):** a process killed while registering a session could
  leave an empty index marker that failed every later listing with `INTEGRITY_FAILURE`;
  the marker is now staged and renamed into its bucket. The P10 durability campaign
  passed on the fix (run 36447992132).
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
| P12 | In progress: PR 1 (skill, guard) and PR 2 (harness) merged; PR 3a (dry-trial fixes) in review; L-076 decision, then the counted trials |
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

- P12 PR 3a gates on Windows 11 (fmt, strict Clippy with and without features,
  workspace tests, warning-denied rustdoc, governance) and the real-client evidence
  for each fix go in the pull request description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
