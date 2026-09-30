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

There is also an **agent skill** (`skills/vsift/`) that teaches Claude Code or Codex
to run an investigation with the CLI and write a cited report, and a trial harness
that runs and grades those clients on synthetic recordings (Codex in a Linux
container: L-076, #204). In the final counted campaign (on `56f1e1f`) the strong
models (Opus 5.5, GPT-6-Astra) passed 9 of 11; Claude Sonnet 5.5 passed 25 of 28,
missing only the second, resumed half of A-02; GPT-6-Sol answered 25 of 28 right but
passed fully only 15, failing on harmless look-around commands, a misread check-image
code and the same resumed half; GPT-6-Luna passed 15. The compact tier is now Sonnet
5.5 and GPT-6-Sol; Luna and Haiku 4.5 are below the line. **PR 3i (in review)** is the
last fix round: the grader accepts those harmless probes, the check image is redrawn
so its letters cannot be confused, the skill tells a resumed run that it has its own
budget and must check the earlier findings again, and the grader reads numbers
written in words and times like `10.32`. Its re-grade lifts Sol to 21 of 28; the rest
needs the compact tier's re-run. Until the named-client trials pass, the skill is a
candidate, not a qualified integration.

**P00-P11 are complete** (P11 closed 2026-09-28, merge `40c4038`; SEC-T01's
adversarial evidence is technical debt, #188, L-068). **P12 (agent skill) is in
progress.** PRs 1, 2, 3a-3h (merged) and 3i are increments, not the packet.

## P12 in one view

[ADR 0022](../docs/decisions/0022-agent-skill-and-named-client-qualification.md) is
**Proposed** (for maintainer review).

- **PR 1 (#196, `7990fdf`): skill and guard.** `skills/vsift/` (`SKILL.md`, eight
  states, `references/`, handoff v1 schema, examples, image check); guard
  `crates/vsift-cli/src/skill_contract.rs`; install guide `docs/agents/skill.md`.
- **PR 2 (#201, `9d2f60e`): trial machinery.** `tools/vsift-agent-trials` (`prepare`,
  `run`, `grade` with mechanical and interpretation results, `record`); 21 scenarios;
  runbook `docs/agents/trials.md`; SEC-T02 suite.
- **PR 3a-3e:** trusted Claude Code workspaces and one settings source; search first;
  nothing but `vsift`; Codex in Linux (PR 3b, #209: pinned images, seccomp L-078, three
  containers per trial); every diagnostic-pass fix (PR 3e, #211).
- **PR 3f (#212, `b68d746`):** start-folder orientation is housekeeping; handoff v1
  requires only what the agent alone knows (L-081).
- **PR 3g (#214, `f018e0d`):** the handoff vocabulary shown and guarded; one exact
  resume card, required only when work can continue; unused citations a warning.
- **PR 3h (#216, `56f1e1f`): `display_text`** (ADR 0008 note): hidden characters shown
  as `<U+XXXX>`; the skill quotes only it. No final-campaign report held one (L-083).
- **Final campaign (on `56f1e1f`):** full passes Sonnet 5.5 25/28, GPT-6-Sol 15/28 (25
  answers right), GPT-6-Luna 15/28, Opus 5.5 9/11, GPT-6-Astra 9/11.
- **PR 3i (branch `p12-pr3i-final-fixes`, this change): the maintainer's decisions of
  2026-09-30.** Grader: `command -v`/`which <name>`, `ls -l`/`-a` of named files and
  `true`/`:` are orientation (compound only with free vsift); the image check reads the
  code of the image each trial's workspace received (a table by SHA-256); cardinal
  numbers in words and `H.MM` times match key facts. Skill: a redrawn check image
  (glyphs never confused, 776x168, command in `docs/agents/skill.md`); `resume.md`: a
  new run has its own budget, repeats the image check and verifies each earlier finding
  with one command; the card may carry `to_verify` (optional). Re-grade `grade-3i.json`:
  Sol 21/28, Luna 16/28, others unchanged. L-084 (Luna below the line).
- **Next:** merge PR 3i, then re-run the compact tier on its merge commit. The packet
  completes only when the named-client trials pass.

## Found in P12

- **L-075 (decided):** Codex shows no viewed image; the right check code proves its
  image access, and its image budgets are unmeasured. PR 3i keys the code by image.
- **L-076 (trials avoid it; #204 open):** Codex's Windows sandbox and VSift's private
  session root are incompatible. **L-078..L-080:** the container's seccomp relaxation,
  unrestricted container egress, and the agent's read access to its sign-in. **L-081:**
  a slim handoff alone may not carry its citations' times (resolve through the bundle).
- **L-082, L-084:** Haiku 4.5 and GPT-6-Luna are below the compact line. **L-083
  (accepted residual):** `text` and `original_text` keep raw hidden characters.
- **L-074 (open):** SubRip markup removal drops any `<letter...>` tag. **L-071:** parse
  failures in JSON modes carry no remediation (the skill works around it).
- **Open grader readings:** `untrusted_listed` takes only F12-E01 (0-8 s) though the
  on-screen instructions last to 12 s; an `rg --files` exclude glob with a separator
  stays strict (PR 3i, for the maintainer).

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
| P12 | In progress: PRs 1, 2, 3a-3h merged; final campaign on `56f1e1f`; PR 3i (final fixes) in review, then the compact tier (Sonnet 5.5, GPT-6-Sol) re-runs |
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

- P12 PR 3i gates on Windows 11 (fmt, strict Clippy with and without features,
  workspace tests, warning-denied rustdoc, governance) go in the pull request
  description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
