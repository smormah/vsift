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
sandbox cannot run VSift (L-076, #204). The strong models (Opus 5.5, GPT-6-Astra)
pass their counted runs. The smaller models ran next: Claude Sonnet 5.5 answered every
question right but often wrote the report's JSON block with words the skill never
listed; GPT-6-Luna did well; Claude Haiku 4.5 did not follow the procedure. The
maintainer made Sonnet 5.5 and GPT-6-Luna the compact tier (Haiku is below the line)
and asked for the vocabulary fix now. PR 3g (merged) did that and re-graded every
run; the compact tier is re-run next. Until the named-client trials pass, the skill is
a candidate, not a qualified integration. Separately, a fix for #210 (PR #215, in
review) makes Windows image and audio paths plain `C:\...` whenever that is exact.

**P00-P11 are complete** (P11 closed 2026-09-28, merge `40c4038`; SEC-T01's
adversarial evidence is technical debt, #188, L-068). **P12 (agent skill) is in
progress.** PRs 1, 2 and 3a-3g (all merged) are increments, not the packet.

## P12 in one view

[ADR 0022](../docs/decisions/0022-agent-skill-and-named-client-qualification.md) is
**Proposed** (for maintainer review).

- **PR 1 (#196, `7990fdf`): skill and guard.** `skills/vsift/` (`SKILL.md`, eight
  states, `references/`, handoff v1 schema, examples, image check); guard
  `crates/vsift-cli/src/skill_contract.rs`; install guide `docs/agents/skill.md`.
- **PR 2 (#201, `9d2f60e`): trial machinery.** `tools/vsift-agent-trials` (`prepare`,
  `run`, `grade` with mechanical and interpretation results, `record`); 21 scenarios;
  runbook `docs/agents/trials.md`; SEC-T02 suite.
- **PR 3a, 3c, 3d (`ed07c0d`), after dry trials:** trusted Claude Code workspaces, one
  settings source, `client_configuration`; search first; nothing but `vsift`;
  `wall_time_s` may be `null` (L-077); operation-id grammar; persistent truth events.
- **PR 3b (#209, `3f91661`): Codex in Linux** (pinned `agent`/`harness` images, seccomp
  profile L-078, three containers per trial, `codex-trial.ps1`).
- **PR 3e (#211, `261b50d`): every diagnostic-pass fix** (grader false positives,
  Codex image code and notices, free help forms, the skill's inline handoff).
- **Counted campaigns (on `261b50d`):** Opus 5.5 and GPT-6-Astra on A-08/A-09 (the
  review tier; 11/11 each pass both results after PR 3f's re-grade), Haiku 4.5 and
  GPT-6-Luna on A-01..A-07 and SEC-T02. Raw logs stay local.
- **PR 3f (#212, `b68d746`, merged):** orientation in the starting folder is
  housekeeping; handoff v1 requires only what the agent alone knows (recorded values
  optional, checked when given, resolved from the bundle otherwise); L-081.
- **Compact-tier runs (on `b68d746`, 28 trials each):** Sonnet 5.5 (Claude Code
  2.1.284) 28/28 answers, 9/28 full passes, 18 failing only on the handoff; GPT-6-Luna
  24/28 and 11/28; Haiku 4.5 6/28 and 2/28.
- **PR 3g (#214, `f018e0d`, merged): maintainer decisions of 2026-09-29.** Compact
  tier Sonnet 5.5, GPT-6-Luna; Haiku 4.5 below the line (L-082); validator P13 (#213). `SKILL.md` lists the words
  of fifteen closed members beside the skeleton, `handoff.md` all of them (guard:
  exactly the schema's); `resume.md` shows one exact card; the failure-code table moved
  to `commands.md`. Schema: gap notes 600 characters; the resume card accepts
  `remaining.images`, null counts and no `job_id`, and is required only when work can
  continue (supervisor). Grader: letter case normalised with a warning, synonyms refused;
  unused citations are warnings. Trial settings set `disableBundledSkills`. L-083 (raw
  hidden characters; a proposal, not implemented). Every compact-tier and review-tier
  run re-graded beside its original (`grade-3g.json`).
- **Next:** re-run the compact tier on the latest `main` (after PR #215 if it merges
  first). The packet completes only when the named-client trials pass.

## Found in P12

- **L-075 (decided):** Codex shows no viewed image; the right check code proves its
  image access, and its image budgets are unmeasured.
- **L-076 (trials avoid it; #204 open):** Codex's Windows sandbox and VSift's private
  session root are incompatible. **L-078..L-080:** the container's seccomp relaxation,
  unrestricted container egress, and the agent's read access to its sign-in. **L-081:**
  a slim handoff alone may not carry its citations' times (resolve through the bundle).
- **L-082:** Haiku 4.5 is not supported. **L-083:** VSift's `text` keeps raw bidi and
  zero-width characters (and decodes WebVTT references into them); models copied one
  into reports. The proposal (render them as `<U+XXXX>` in `text`) awaits a decision.
- **L-074 (open):** SubRip markup removal drops any `<letter...>` tag. **L-071:** parse
  failures in JSON modes carry no remediation (the skill works around it).
- **Open grader readings (PR 3c):** `untrusted_listed` takes only F12-E01 (0-8 s)
  though the on-screen instructions last to 12 s; a frame before 8 s binds `install`.

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
| P12 | In progress: PRs 1, 2, 3a-3g merged; review tier passes; compact tier (Sonnet 5.5, GPT-6-Luna) to be re-run on the vocabulary fix |
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

- PR #215 (#210): Windows 11 gates (fmt, strict Clippy with and without features,
  workspace tests, warning-denied rustdoc, governance) are in its description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
