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
and fixed configuration, skill-wording and answer-key problems (PR 3a, PR 3c, PR 3d);
the third Claude Code dry trial passed every check. The counted Claude Code trials
(39 runs) restart after PR 3d. Codex trials run in a Linux container (PR 3b), because
Codex's Windows sandbox cannot run VSift (L-076, #204); its first dry trial passed all
but the image check (L-075). Until the named-client trials pass, the skill is a
candidate, not a qualified integration.

**P00-P11 are complete** (P11 closed 2026-09-28, merge `40c4038`; SEC-T01's
adversarial evidence is technical debt, #188, L-068). **P12 (agent skill) is in
progress.** PRs 1, 2, 3a, 3c, 3d and 3b are increments, not the packet.

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
- **PR 3a (#203, `67d56c3`, merged), after the first dry trials:** Claude Code
  workspaces trusted in the client home, one settings source; `client_configuration`
  makes a trial invalid when a client ignored its configuration; search first.
- **PR 3c (#207, `9791f70`, merged), after the second Claude dry trial:** nothing but
  `vsift`, one command per call; `wall_time_s` may be `null` (L-077); operation-id
  grammar in `SKILL.md`; corpus truth amended with `persistent` events (F04-E05,
  F05-E04, F12-E03; reviewed in the corpus README).
- **Third Claude dry trial (2026-09-29):** every check passed. **First counted run:**
  it listed the skill's `examples/` with `Glob`; campaign stopped, not counted.
- **PR 3d (#208, `ed07c0d`, merged):** `Glob`/`Grep`/`LS` inside the skill folders
  count as a skill read; the skill says to read its files with the file-reading tool.
- **PR 3b (branch `p12-pr3b-codex-container`, this change): Codex in Linux.**
  `tools/vsift-agent-trials/containers/codex/`: a pinned multi-stage `Dockerfile`
  (`agent` image: tools only; `harness` image: plus the repository), a seccomp
  profile allowing bubblewrap's user namespaces (L-078), `trial-driver.sh` and the
  wrapper `codex-trial.ps1`. A trial is three containers: `prepare` and `grade` in the
  harness image, `run` in the agent image with only its trial folder, the model and a
  tmpfs copy of `auth.json`. Harness: Linux writable root `.home`, `--debug-prompt`,
  a sign-in value scan (`no_canary`), bubblewrap failures invalidate a trial. CI
  workflow `p12-codex-container.yml` builds both images. Evidence: 5 `gpt-6-luna`
  debug runs and a `gpt-6-astra` dry A-08 trial (all mechanical checks but
  `image_check`, interpretation passed).
- **Next:** restart the 39 counted Claude Code runs from zero; the maintainer decides
  how to grade Codex's images (L-075), then the Codex counted runs in the container.
  The packet completes only when the named-client trials pass.

## Found in P12

- **L-075 (open, decision):** codex-cli 0.155's `exec --json` stream has no event for
  a viewed image, so `image_check` fails and image budgets cannot be counted.
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
| P12 | In progress: PRs 1, 2, 3a, 3c and 3d merged, 3b (Codex Linux container) in review; counted Claude Code trials restart after 3d; Codex trials in the container after the L-075 decision |
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

- P12 PR 3b gates on Windows 11 (fmt, strict Clippy with and without features,
  workspace tests, warning-denied rustdoc, governance), the image build and the
  container evidence go in the pull request description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
