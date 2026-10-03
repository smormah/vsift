# VSift current status

As of 2026-10-02. Current-state document: rewrite it, don't append to it. Next actions and
open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives anyone, and their AI assistant, local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is also an
embeddable engine library (`vsift`) that the CLI, and later other hosts, use. Today it can:
- check and register its dependencies, show a read-only setup plan, report whether local speech
  recognition really works here (`setup check`), and on Ubuntu 24.04 x86-64 install the reviewed
  FFmpeg, whisper.cpp and model itself, then list, roll back, remove and repair them (the store
  survives kills and, on Ubuntu 24.04 with ext4, power loss);
- copy a video into a private, disposable session; import an SRT or WebVTT transcript with it,
  aligned by an offset; or transcribe the speech itself with whisper.cpp (`transcript
  retranscribe`), continue an interrupted transcription, and report, resume or cancel that work
  by its job id (`job status/resume/cancel`);
- return timestamped transcript segments, search them, list the moments where the screen
  changed, and return exact frames, neighbours, bursts, crops and WAV clips;
- manage the session's lifetime and retention, validate retained bundles, stop a long command
  cleanly on Ctrl-C or `SIGTERM`, and keep every folder it creates private;
- run as a worker under an external supervisor (`session init-workspace`, `job run`, `job
  batch`), each step at most once per operation id; check an agent's draft report (`handoff
  check`); refuse to claim strict worker isolation unless the Linux kernel attests it;
- be installed without Rust: **the 0.1.0 pre-release is published** (2026-10-01): `npm install
  --global vsift-cli@next`, and archives on a GitHub pre-release, each with a Sigstore
  attestation and npm provenance. The README shows its real output (graphics: L-121).

**The agent skill** (`skills/vsift/`) teaches Claude Code or Codex to run an investigation with
the CLI and write a cited report. P12's named-client trials qualified it:
- **Review tier:** Claude Opus 5.5 in Claude Code and GPT-6-Astra in Codex each passed 11 of 11
  trials mechanically and 9 of 11 fully. **Compact tier:** Claude Sonnet 5.5 and GPT-6-Sol: 82% in
  P12; the re-run (#222) meets the 90% target, Sonnet 26 of 28 and Sol 28 of 28 (23 as run).
- **Safety:** no agent leaked a secret, installed anything, acted on injected text or copied a
  hidden character into a report. Codex ran in a Linux container (L-076, #204). **Not yet tried:**
  an agent with no skill, or on the published package; the harness exists (PR 6), no trial has run.

## Where the project stands

**P00-P13 are complete. P14, the R0 qualification (#17), is in progress** (started 2026-10-02;
decisions A-H of ADR 0024 confirmed). Its plan is 14 pull requests (0-13); **PR 0 (#250), PR 1
(#251), PR 2 (#255, the published-artifact qualification), PR 6 (#262, the agent-trial harness)
and PR 8 (#252) are merged; PR 3 (#254, the journeys on the published binary) is done in this
change, awaiting review; PR 4 (#259) is a draft; the whole packet is not complete.** What it must
show, and what is weaker than it sounds today:
- **The published 0.1.0 has run on a video, but only that:** PR 3 ran the real-tool checkpoints
  against it on hosted Ubuntu 24.04, Windows and macOS 15 (tests from a later commit; no launcher
  or archives in those runs, L-115); PR 2 installed it on hosted runners that are not clean
  machines. Smart App Control and the macOS prompts are unseen on a real machine (L-098).
- **Platforms:** the P08/P09/P11 numbers and the Claude Code trials are Windows 11; hosted
  Ubuntu, Windows and macOS passed on 0.1.0 (macOS with Homebrew's tools, L-114); P11's durable
  stage cannot run on a hosted runner (#258, L-113); the matrix rules are not met yet (L-035).
- **A synthetic corpus and synthetic voice only** (L-020, L-022, L-028, L-030). **Trials tuned on
  their own scenarios;** the hold-outs and the skill-less run are built (PR 6) but unrun; the
  review tier's blurred-banner re-run is missing (L-095).
- **SEC-T01 is half done and stays so in R0** (L-068, #188 in R1; claim narrowed 2026-10-03, RQ-14
  waived); dangerous media never run; load ladder stopped at 4 jobs; fuzzing weekly, 5 min a target.
- **`latest` has never moved;** that path never ran for real (L-105). Decisions A-H: `TODO.md`.

## P14 PR 1, 2, 3, 6 and 8 in one view

**PR 1 (merged, #251):** the evidence ledger (`p14-evidence-ledger.json`, RQ-01..RQ-20) and the
claims registry (`public-claims.json`, rung `now`), checked on every pull request (L-101).

**PR 2 (merged, #255; nothing published).** Real artifacts on hosted runners, for **0.1.0 only**
(plan section 15): **RQ-01** clean install from the real registry (npm, pnpm, Yarn, Bun; Windows,
macOS 15, Ubuntu 24.04; scrubbed `PATH`); **RQ-02** the extracted archives; **RQ-03**
`--artifact-dir` with no network; **RQ-04** upgrade, the uninstall walk and a frozen v0.1.0
compatibility test; **RQ-19** a credential-free second verifier; all `passed` in the ledger.
**Findings:** #256 (minimal Ubuntu lacks `libgomp1`; the hosted image has it), #257 (Windows `.cmd`
shims re-parse hostile arguments; trials use Git Bash; PR 7 documented it in `install.md`,
`SECURITY.md` and the launcher README, L-109 accepted residual); L-109 to L-112.

**PR 3 (this change; awaiting review; nothing published).** ADR 0024's note; results in plan
section 17. `VSIFT_E2E_BINARY` (with the expected version and commit; refused, never ignored, when
wrong) makes the real-tool checkpoints drive an installed `vsift`; `p14_installed_binary_e2e` adds
hostile file names and a sentinel environment. The workflow `P14 journeys` (driver
`tools/p14_journeys.py`) installs `vsift-cli@<version>` from the real npm registry on Ubuntu 24.04
(tools installed by the binary's own `setup install`), Windows (pinned) and macOS 15 (Homebrew)
and runs P06-P11 against its native executable; `P13 managed smoke` takes `published_version`;
both run weekly (about an hour of runner time). **On 0.1.0:** 53 stages passed on each system;
the managed smoke passed (RQ-06 `passed`, RQ-05 `running`: P11's durable stage is blocked, #258);
the T-04 gates held on macOS. No product defect; one Windows stall: #263.

**PR 8 (merged, #252; nothing published).** Runbook `release.md` 6.7-6.9. The version alone
decides the channel: a suffix means `next` (a candidate is `-rc.N`), none is stable and moves
`latest` on all four packages, guarded by the candidate delta, the candidate on npm and the
complete ledger (L-103). **Not done:** a real stable publish (L-105). **Frozen at the candidate
cut:** the allowed lists, the notes, the workflow, the skill.

**PR 6 (#262, merged; runbook `docs/agents/trials.md`).** **No trial was run, no model was
called, nothing was published.** `tools/vsift-agent-trials` now installs the published package
into a fresh prefix and proves it in every record (npm's fetch equals the registry's integrity,
the launcher's digest check, `vsift --version`); plays the user's part; never runs a shim (Git
Bash only); runs a **cold agent** (`C-01..C-03`: no skill, no documentation; safety a hard gate,
a gap report; **Claude strict** (`vsift` alone; its pilots stalled), **Codex realistic** (read-only
helpers, inside the container); realistic Claude needs an isolated machine, so the baseline compares
only within a client: L-125); keeps **hold-outs** apart; **freezes** the skill, grader, scenarios and
settings (`freeze write|check`); captures usage; plans the batches (`campaign`, `summarize`,
`run-campaign.ps1`; PR 7 fixed its checkout check, #273, and #261). **Weak points:** L-117..L-120. **Not done:** any batch.

## P13 in one view (complete)

[ADR 0023](../docs/decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
(**Accepted** 2026-10-01); record `p13-distribution.md`, guide `install.md`, runbook `release.md`.
**Delivered:** native archives and `release.yml` (Windows x64, macOS 15 arm64, Linux x64), the npm
launcher `vsift-cli` over three platform packages, managed installation, human output, `handoff
check`, the 0.1.0 pre-release (tag `v0.1.0` on `011bc4d`). **Not proved:** Smart App Control, the
macOS prompts (L-098); power loss beyond Ubuntu 24.04 ext4 (L-037).

## What works (public CLI)

- `setup check|configure|configure-model|plan`; `setup install` on Ubuntu 24.04 x86-64;
  `setup list|rollback|remove|repair` on every OS; `ingest <video> [--transcript <file>
  [--transcript-offset <signed us>]]`; `transcript retranscribe|get`; `job status|resume|cancel|
  run|batch`; `search`, `candidates`, `frame get|neighbours|burst`, `crop`, `audio`; `session
  list|status|renew|close|retain|clean|init-workspace`; `bundle validate`.
- `handoff check`: a draft from stdin or `--file`. A release build's `--version` names its
  source commit (`vsift 0.1.0 (<12 hex>)`). Global `--session-root`, `--host-isolation`,
  `--json`, `--events jsonl`. Readable terminal text without `--json` (unstable).

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00-P05 | Complete; merge commits and evidence are in the ledger |
| P06-P10 | Complete: detect, select, verify, guide (`b73df52`); engine, transcripts, local ASR (`9ea3180`); search, candidates (`b830fc9`); frames, crops, audio (`e57c706`); jobs, resume, durable Ubuntu/ext4 (`3f27ce3`) |
| P11 | Complete (`40c4038`); SEC-T01 adversarial evidence is technical debt, moved to R1 (#188, L-068) |
| P12 | Complete (2026-09-30, ADR 0022 Accepted): skill, harness, named-client trials; review tier qualified, compact tier 93% and 100% on the #222 re-run; open: L-095 (#224), #219, #204 (`1284e54`) |
| P13 | Complete (2026-10-01, ADR 0023 Accepted): distribution, managed install, `handoff check`, human output; 0.1.0 published; release commit `011bc4d` |
| P14 | **In progress** (started 2026-10-02): PR 0 (#250), PR 1 (#251), PR 2 (#255), PR 6 (#262) and PR 8 (#252) merged; PR 3 (#254) done in this change, PR 4 (#259) draft |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine) <-
`vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and owns the wire
types. The skill only calls the `vsift` binary. The trial harness depends only on `vsift` and
`vsift-contract`; `tools/vsift-release`, `tools/p14-published` and `tools/p14_journeys.py` (never shipped) package
and qualify the artifacts; `npm/` holds the launcher.

## Quality evidence

- **Local gates** (in each PR description): fmt, strict Clippy, workspace tests, warning-denied
  rustdoc, governance. **CI on every PR:** Quality (three OS), Documentation, Governance, fuzz
  replay, worker boundary, dependency policy, CodeQL, npm launcher tests; the Release dry run and
  the P14 workflows on changes to their paths. **Required on `main`:** Quality, Documentation,
  Dependency policy and review, Analyze Rust, Governance. Squash merges.
