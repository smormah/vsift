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
sandbox cannot run VSift (L-076, #204). Two diagnostic passes led to PR 3e (merged),
and the counted campaigns ran on it. The strong models pass; after them the
maintainer decided that harmless orientation commands are not failures and that the
report states only what the agent alone knows. PR 3f (in review) implements both and
re-grades every counted run; the small models are re-run next. Until the named-client
trials pass, the skill is a candidate, not a qualified integration.

**P00-P11 are complete** (P11 closed 2026-09-28, merge `40c4038`; SEC-T01's
adversarial evidence is technical debt, #188, L-068). **P12 (agent skill) is in
progress.** PRs 1, 2, 3a-3e (merged) and 3f are increments, not the packet.

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
- **PR 3e (#211, `261b50d`, merged): every diagnostic-pass fix** (grader false
  positives, Codex image code and notices, free help forms, the skill's inline handoff).
- **Counted campaigns (2026-09-29, on `261b50d`):** Claude Code (Opus 5.5 on A-08/A-09,
  Haiku 4.5 on A-01..A-07 and SEC-T02) and Codex (GPT-6-Astra, GPT-6-Luna, the same
  split). Raw logs stay local; the results are in PR 3f's re-grade table.
- **PR 3f (branch `p12-pr3f-orientation-slim-handoff`, this change): two maintainer
  decisions.** Grader: `pwd`, `cd` to the starting folder and a listing of its file
  names (`rg --files` with globs that cannot open `.home`, `ls`/`dir`/`Get-ChildItem`
  without recursion) are housekeeping; `cd` elsewhere, `command -v`, contents and other
  listings stay strict. Handoff v1 (unreleased) revised in place: only the agent's own
  knowledge is required (claims, identities, `pixels_inspected`, gaps, untrusted
  instructions, `lifecycle.action`, the resume card when partial or exhausted); VSift's
  recorded values are optional, checked when given and resolved from the bundle
  otherwise; the harness finds the session from the commands when the handoff names
  none. Skill skeleton, `handoff.md` and examples slimmed; guard tests; L-081. Every
  counted run re-graded beside its original (`grade-3f.json`).
- **Next:** merge PR 3f, then re-run the small models on its merge commit. The packet
  completes only when the named-client trials pass.

## Found in P12

- **L-075 (decided 2026-09-29):** codex-cli 0.155 shows no viewed image, in its
  stream or countably in its rollout; the right check code proves Codex's image
  access, and its image budgets are unmeasured.
- **L-076 (trials avoid it; #204 open):** Codex's Windows sandbox and VSift's private
  session root are incompatible. **L-078..L-080:** the container's seccomp relaxation,
  unrestricted container egress, and the agent's read access to its sign-in. **L-081:**
  a slim handoff alone may not carry its citations' times (resolve through the bundle).
- **L-074 (open):** SubRip markup removal drops any `<letter...>` tag (SEC-T02 suite).
- **Open grader readings (PR 3c):** `untrusted_listed` takes only F12-E01 (0-8 s)
  though the on-screen instructions last to 12 s; a frame before 8 s binds `install`.
- **#197 (fixed, #200 `98525dc`):** a torn session-index marker no longer fails later
  listings; the P10 durability campaign passed on the fix (run 36447992132).
- **L-071:** parse failures in JSON modes carry no remediation (the skill works around it).

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
| P12 | In progress: PRs 1, 2, 3a-3e merged; counted campaigns ran on 3e; 3f (the maintainer's orientation and slim-handoff decisions) in review; the small models are re-run next |
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

- P12 PR 3f gates on Windows 11 (fmt, strict Clippy with and without features,
  workspace tests, warning-denied rustdoc, governance), the image digests and the
  re-grade table go in the pull request description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
