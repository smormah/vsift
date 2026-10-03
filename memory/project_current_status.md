# VSift current status

As of 2026-10-03. Current-state document: rewrite it, don't append to it. Next actions and
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
  hidden character into a report. Codex ran in a Linux container (L-076, #204). **Batch 1 (the
  published 0.1.0, a baseline):** skill pilots 4 of 4; cold, useful 1 of 6 (Claude), 2 of 6 (Codex).

## Where the project stands

**P00-P13 are complete. P14, the R0 qualification (#17), is in progress** (started 2026-10-02;
decisions A-H of ADR 0024 confirmed). Its plan is 14 pull requests (0-13); **PRs 0 (#250), 1
(#251), 2 (#255, published artifacts), 3 (#254, journeys on the published binary), 5 (#276), 6
(#262, the trial harness) and 8 (#252) are merged; PR 4 (#259, the robustness campaigns) is ready
for review; PR 7 (fixes) is under way; the whole packet is not complete.** What it must show, and
what is weaker than it sounds:
- **The published 0.1.0 has run on a video, but only that:** PR 3 ran the real-tool checkpoints
  against it on hosted Ubuntu 24.04, Windows and macOS 15 (tests from a later commit; no launcher
  or archives, L-115); PR 2 installed it on hosted runners, not clean machines. Smart App Control
  and the macOS prompts are unseen on a real machine (L-098).
- **Platforms:** the P08/P09/P11 numbers and the Claude Code trials are Windows 11; hosted
  Ubuntu, Windows and macOS passed on 0.1.0 (macOS with Homebrew's tools, L-114); P11's durable
  stage cannot run on a hosted runner (#258, L-113); the matrix rules are not met (L-035).
- **A synthetic corpus and synthetic voice only** (L-020, L-022, L-028, L-030). **Trials tuned on
  their own scenarios;** the hold-outs are unrun and the skill-less run ran only as batch 1's
  baseline; the review tier's blurred-banner re-run is missing (L-095).
- **SEC-T01 is half done and stays so in R0** (L-068, #188 in R1; RQ-14 waived). **The campaigns
  have now run once on 0.1.0 and found things:** Windows races (#206, #271), the FFmpeg snapshot
  (#272), three hostile-source cases (#264-#266) and a recognition range failure (#274); none is
  fixed (PR 7), so RQ-08, RQ-10 and RQ-13 are `failed`.
- **`latest` has never moved;** that path never ran for real (L-105). Decisions A-H: `TODO.md`.

## P14 PRs 1, 2, 3, 4, 6 and 8 in one view

**PR 1 (merged, #251):** the evidence ledger (RQ-01..RQ-20) and the claims registry (rung `now`),
checked on every pull request (L-101). **PR 2 (merged, #255):** RQ-01 clean install from the real
registry (npm, pnpm, Yarn, Bun; three systems), RQ-02 the archives, RQ-03 the offline install, RQ-04
upgrade and a frozen v0.1.0 compatibility test, RQ-19 a second verifier; all `passed` for 0.1.0 only
(plan section 15; #256 fixed, #257 documented; L-109 to L-112). **PR 8 (merged, #252):** `release.md`
6.7-6.9; the version alone decides the channel (a suffix means `next`, none moves `latest`), guarded
by the candidate delta and the complete ledger (L-103); a real stable publish is not done (L-105).

**PR 3 (merged, #254).** `VSIFT_E2E_BINARY` makes the real-tool checkpoints drive an installed `vsift`;
`P14 journeys` installs `vsift-cli@<version>` from the real registry on Ubuntu 24.04 (managed tools),
Windows (pinned) and macOS 15 (Homebrew) and runs P06-P11 against its native executable; both it and `P13
managed smoke` run weekly. **On 0.1.0:** 53 stages passed on each system, RQ-06 `passed`, RQ-05 `running`
(P11's durable stage is blocked, #258); one Windows stall (#263).

**PR 6 (merged, #262; `docs/agents/trials.md`) and batch 1.** `tools/vsift-agent-trials` installs the
published package and proves it in every record, runs a cold agent (`C-01..C-03`: **Claude strict**,
**Codex realistic**, L-125), keeps hold-outs, freezes inputs by digest, captures usage and plans three
batches. Weak points: L-117 to L-120. **Batch 1** (2026-10-03, 0.1.0, 20 runs, a baseline;
`p14-agent-trials/batch-1-reading.md`): skill pilots 4 of 4; cold useful 1 of 6 (Claude) and 2 of 6
(Codex); none installed anything; safety "not met" on three grader classes for the maintainer (#282).

**PR 4 (#259, ready for review; hosted runners only, nothing published, no product code changed;
plan section 18).** The robustness campaigns on 0.1.0 (`tools/p14-campaigns/`, six read-only workflows):
- **RQ-07 passed.** The gap review added seven fuzz targets (31 in all): 3,601 s each, 3.68 billion
  runs, no crash; 19 were still finding coverage at the end; three record kinds have no target (L-128).
- **RQ-08 failed.** 6,700 repetitions per system on Windows, Ubuntu and macOS: Windows failed root creation
  7 of 1,500 (#206 reproduced) and 2 of 1,500 loaded, and weighted admission 2 of 200 (#271); #128 did not
  reproduce (0 in 3,000); L-123.
- **RQ-09 passed.** Ladder 1-8, a 100-request batch, a cancel, a warm page (p95 5 ms) and a 1,000-request
  soak with kills: every gate held (two longer soaks: plan 18.3). Found on the way: #274 (a 5 s range of
  three clips fails as `MISSING_CAPABILITY`, L-124), #277 (sessions left `initializing`), #286 (dedupe window).
- **RQ-10 failed.** 96 hostile inputs, no network, bounded: 93 held; a pipe hangs `ingest` (#264), a link
  is `STORAGE_IO` (#265), a full disk `INTEGRITY_FAILURE` (#266); L-127.
- **RQ-12 passed.** The runbook walked in 18 steps (durable workspace on an ext4 volume in a file); ten
  divergences fixed. The ledger commits of RQ-07, 08, 09 and 12 are PR heads: re-point them after the squash.
- **RQ-13 failed.** The first scan reading (`p14-scan-reading-2026-10-02.md`): all clean but the FFmpeg
  snapshot, which lacks 17 upstream fixes; 18 more records give no fix reference (#272, L-122).

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
| P14 | **In progress** (started 2026-10-02): PRs 0-3, 5, 6 and 8 merged; PR 4 (#259) ready for review; PR 7 under way; agent batch 1 ran (baseline) |

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
