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
Codex to run an investigation with the CLI and write a cited report. It has not yet
been tried with real agents, so it is a candidate, not a qualified integration.

**P00-P11 are complete** (P11 closed 2026-09-28, merge `40c4038`; SEC-T01's
adversarial evidence is technical debt, #188, L-068). **P12 (agent skill) is in
progress.** PR 1 is an increment, not the packet.

## P12 in one view

[ADR 0022](../docs/decisions/0022-agent-skill-and-named-client-qualification.md) is
**Proposed** (for maintainer review).

- **PR 1 (branch `p12-pr1-skill`, this change): skill and contract guard.**
  - `skills/vsift/SKILL.md`: trigger description and eight states
    (CHECK_CAPABILITIES, PREPARE, FIND_SPOKEN_SPANS, INSPECT_CARDS, VERIFY_SOURCE,
    REFINE_OR_STOP, REPORT, CLOSE_OR_RETAIN), each with allowed commands and a
    stopping condition; error handling by code.
  - `references/`: `commands.md` (every public command `free`, `explicit` or
    `never`; the allowed command forms), `budgets.md` (`compact` default: 1 image per
    step, 6 in total, 12 MiB, pages of 20, 30 tool calls, depth 2, 15 min, bursts of 4;
    `standard`: 4/24/48 MiB/50/80/4/30 min/12), `handoff.md` (eight sections plus a
    `vsift-handoff` JSON block), `safety.md`, `resume.md`, `lifecycle.md` (includes
    the cleanup routine, closing L-009's P12 step).
  - `handoff.schema.json` (handoff v1, owned by the skill), two example handoffs built
    from real CLI output on F10 (supplied transcript, images) and F03-speech (no
    whisper, no image access), `assets/image-check.png` (code word only in pixels),
    `agents/openai.yaml` (Codex metadata, no tool dependencies).
  - Guard `crates/vsift-cli/src/skill_contract.rs` (11 unit tests): console command
    lines parse with the real parser and respect their class; inline commands and
    flags exist; the class table covers `CommandName::ALL` once and fixes the `never`
    and `explicit` sets; upper-case codes are `FailureCode::ALL` or states; field and
    reason names resolve in `schemas/v1`, `cli-v1.md` or the handoff schema; examples
    validate; the schema refuses paths, links and hidden characters; the image code is
    in no text; `SKILL.md` within 300 lines with only `name` and `description`. A
    15-mutation check showed each rule catches its breakage.
  - Docs: `docs/agents/skill.md` (install for Claude Code and Codex, requirements,
    guard), README status, ADR 0016 note, threat-model P12 note, known limits
    L-007/L-009/L-039 rewritten and L-071 added (L-070 added then fixed), CHANGELOG, ledger P12
    `in_progress` with the skill files as source documents.
- **Next increment:** named-client trials (ADR 0022 decision 7): A-01..A-09 and SEC-T02
  in named Claude Code and Codex, compact and review models, five trials per scenario,
  mechanical and interpretation results separate, attempted out-of-policy actions fail.
  The packet completes only when those pass.

## Found while writing the skill

- **L-070 (fixed):** the `job resume` remediation for a closed or expired session now
  says to open a new session with `ingest`; it no longer advises a renewal the CLI
  refuses.
- **#197 (fix in review, PR #200):** a real defect, not a flake. A process killed while
  it registered a session could leave an empty index marker, after which every session
  listing and that registration's cleanup failed with `INTEGRITY_FAILURE` for good; the
  P11 batch kill test hit it on macOS and it reproduces on Windows. The marker is now
  staged in `session-index/.registering.tmp` and renamed into its bucket (store test
  `a_kill_while_registering_leaves_the_index_readable`). Complete once PR #200 merges.
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
| P12 | In progress: PR 1 (skill and guard) in review; named-client trials next |
| P13 | Not started; also delivers managed installation and human-readable output |
| P14 | Not started |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and
owns the wire types. The worker lives in the engine (`worker.rs`, `batch.rs`); the CLI
only presents. The agent skill (`skills/vsift/`) sits outside the crates and only
calls the `vsift` binary; its guard is a test module of `vsift-cli` because the parser
is crate-private.

## Quality evidence

- P12 PR 1 gates on Windows 11 (fmt, strict Clippy with and without features,
  workspace tests, warning-denied rustdoc, governance) go in the pull request
  description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
