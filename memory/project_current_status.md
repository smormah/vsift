# VSift current status

As of 2026-10-07. Current-state document: rewrite it, don't append to it. Next actions and
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
  retranscribe`), continue an interrupted transcription, and report, resume or cancel that work;
- return timestamped transcript segments, search them, list the moments where the screen changed,
  and return exact frames, neighbours, bursts, crops and WAV clips;
- manage the session's lifetime and retention, validate retained bundles, stop a long command
  cleanly on Ctrl-C or `SIGTERM`, and keep every folder it creates private;
- run as a worker under an external supervisor (`session init-workspace`, `job run`, `job batch`),
  each step at most once per operation id; check an agent's draft report (`handoff check`); refuse
  to claim strict worker isolation unless the Linux kernel attests it;
- be installed without Rust: **the 0.1.0 pre-release is published** (2026-10-01): `npm install
  --global vsift-cli@next`, and archives on a GitHub pre-release, each with a Sigstore
  attestation and npm provenance. The README shows its real output (graphics: L-121).

**The agent skill** (`skills/vsift/`) teaches Claude Code or Codex to run an investigation with
the CLI and write a cited report. P12's named-client trials qualified it:
- **Review tier:** Claude Opus 5.5 in Claude Code and GPT-6-Astra in Codex (a Linux container, L-076) each passed
  11 of 11 trials mechanically and 9 of 11 fully. **Compact tier:** Claude Sonnet 5.5 and GPT-6-Sol: 82% in P12;
  the re-run (#222) meets the 90% target, Sonnet 26 of 28 and Sol 28 of 28 (23 as run).
- **Safety:** no agent leaked a secret, installed anything, acted on injected text or copied a hidden character.
  **P14's batch 2 (a clean install of each candidate) narrows this: Astra, Sonnet and Sol met their gates; Claude Opus missed two review-tier gates and is excluded from R0's claims (L-139).**

## Where the project stands

**P00-P13 are complete. P14, the R0 qualification (#17), is in progress** (started 2026-10-02;
decisions A-H of ADR 0024 confirmed). Its plan is 14 pull requests (0-13); **PRs 0-9 are merged. PR 10, the candidate, was done twice and
both are published: `0.2.0-rc.1` (2026-10-05, `v0.2.0-rc.1` at `d5792ce31db1`; superseded, stays on npm) and `0.2.0-rc.2` (2026-10-07,
`v0.2.0-rc.2` at `7c722d1fc46a`, npm `next` on all four packages, `latest` untouched): rc.1 plus the fixes of #314 and #310 and nothing else.
PR 11, the qualification, is repeated on rc.2: its hosted part (plan section 26) and agent-trial batch 2 (section 27) are done and recorded;
batch 3 (the cold round) and the try-outs are not; PRs 12-13 remain; the packet is not complete.** What it shows, and what is weaker than it sounds:
- **On rc.2, hosted:** the second verification of the publish (RQ-19), clean installs, archives, the offline install, the upgrades from
  0.1.0 and from rc.1 (npm only), the journeys on three systems (macOS with Homebrew's tools, L-114; P11's durable stage cannot run hosted,
  L-113), fuzzing (18 of 31 targets still growing, L-128), load, the runbook walk, the media run (green), both fault campaigns (hosted verdict
  jobs ran) and the scan reading **passed; not clean machines** (L-112). **The stress run failed** one Windows repetition of 20,100 (#321,
  L-138: a race in a supervisor test, read from the source), while the lock suite (#314) was 200 of 200 and #312 did not recur. Smart App
  Control and the macOS prompts are unseen (L-098). **A synthetic corpus and voice only** (L-020, L-022).
- **On rc.2, agent-trial batch 2 (34 runs with the skill, 2026-10-07):** 28 passed fully as graded; none installed anything or leaked. Codex met
  its gates; **Claude Opus 5.5 did not meet two review-tier gates (4 of 6 mechanically; 1 of 3 on the blurred banner, as on rc.1) and is excluded.**
- **Four of twenty items are waived, by the maintainer (2026-10-07 for all but RQ-14):** RQ-08 (#321, a test race), RQ-10 (two answers outside the
  codes its rule names: #265, L-134), RQ-14 (SEC-T01, L-068) and RQ-15 (the two Opus gates; a `printf` header and a hold-out's wording count as met
  after the maintainer's reading). `release-evidence --complete-for 0.2.0-rc.2` fails on exactly two items: RQ-16 and RQ-17. **Accepted for R0:**
  one reachable whisper.cpp heap read (#322, L-137). **Open:** #312 (L-135). CVE-2026-38350 is accepted (L-122). **`latest` has never moved** (L-105). Decisions: `TODO.md`.

## P14 PRs 1 to 6 and 8 in one view

**PR 1 (#251):** the evidence ledger (RQ-01..RQ-20) and claims registry (rung `now`, `candidate` since PR 10b), checked on every
PR (L-101). **PR 2 (#255):** clean installs from the real registry (npm, pnpm, Yarn, Bun; three systems),
archives, offline install, upgrade and a second verifier (RQ-01..04, RQ-19), `passed` for 0.1.0 only (L-109
to L-112). **PR 8 (#252):** the version alone decides the channel (a suffix means `next`, none moves
`latest`), guarded by the candidate delta and the complete ledger (L-103); no real stable publish yet
(L-105). **PR 3 (#254):** `P14 journeys` runs the real-tool checkpoints against `vsift-cli@<version>` from
the registry on three systems, weekly too; on 0.1.0 53 stages passed on each (RQ-06 `passed`, RQ-05
`running`: P11's durable stage is blocked, #258).

**PR 6 (#262; `docs/agents/trials.md`) and batch 1.** `tools/vsift-agent-trials` installs the published
package, runs a cold agent (**Claude strict**, **Codex realistic**, L-125), keeps hold-outs, freezes inputs by
digest, captures usage and plans three batches (L-117 to L-120). **Batch 1** (2026-10-03, 0.1.0, 20 runs, a
baseline): skill pilots 4 of 4; cold useful 1 of 6 (Claude), 2 of 6 (Codex); none installed anything; the
three grader classes were ruled and implemented (#298); batch 2 needs a fresh `freeze write`.

**PR 4 (#259; plan section 18; hosted runners only)** ran the campaigns on 0.1.0 (`tools/p14-campaigns/`):
RQ-07 passed (31 fuzz targets, no crash; 19 still growing: L-128); RQ-08 failed (Windows #206, #271); RQ-09
passed (found #274, #277, #286); RQ-10 failed (96 hostile inputs, 93 held: #264-#266); RQ-12 passed (runbook
walked); RQ-13 failed (#272).

## P14 PRs 9 and 10 in one view

**PR 9 (#302, #304, #306).** The support matrix (`support-and-resource-profiles.md`: no cell may say "supported"
yet), the install guide, `SECURITY.md`, the runbook, the skill guide and the README facts brought to it; the claims
check reads the launcher's messages and the README graphics' text (L-121) and each claim lists the register entries it
leans on (`limits`); `register-review-sheet.md` (every review `pending`). **Decided 2026-10-04:** the macOS wording,
the versions policy and RQ-05's per-system rule (still `running`, plan section 21). `docs/guide/`
(twelve pages, two generated) is held to the code by the `Guide` workflow (`tools/guide/`): 40 marked commands print what
the pages show. **PR 10 (the candidate; rc.1 published 2026-10-05, rc.2 on 2026-10-07).** The version is `0.2.0-rc.2`
everywhere, the guide's marker is the release `0.2.0`, rung `candidate`, the freeze of batches 2 and 3 is committed with the same
digests as for rc.1 (a test holds it) and rc.1's batch-2 records are in `batch-2-rc.1/`. The allowed lists of the candidate-to-stable
check (a work record and `install.md` are allowed; nothing else may merge from the tag to the stable, so Dependabot waits: #192, #193,
#194 after the stable) are unchanged, and the check compares the stable with the highest `-rc.N` tag, which is `v0.2.0-rc.2`.

## P14 PR 7 in one view (every finding by outcome; one pull request and one regression test each)

**Rule of 2026-10-04: a published failure code stays (v1 is additive, L-126); the remediation carries the fix (L-127).**
- **Fixed:** #264 a pipe no longer hangs `ingest` (#290); #274 a range cut mid-speech keeps its last segment (#289,
  L-130); #277 a failed open removes its registration (#293, L-131); #268 the shutdown remediation (#288); #256 a missing
  shared library is named (#280); #261 a foreign session root explains itself (#279, L-126); #273, #282, #283, #285
  tool fixes (#275, #287). **Answer fixed, code kept (L-127):** #265 a link (#295), #266 no room, best effort on Unix
  (#291, L-061), #277's not-published answer: all `STORAGE_IO`. **Narrowed, not proven gone:** #206 the root's DACL is
  read back and repeated (#301, L-005); #253 the kill test ends its strays (#300, L-129). **Tests or documents only:**
  #271 (#294, L-060); #286 the dedupe window (#299, L-063: an R1 stub is the maintainer's call); #257 the `vsift.cmd` shim
  (#281, L-109). **PR 7b, #272 (not fixed):** 46 of 47 scan records are fixed in the shipped FFmpeg (L-122); the next
  month-end build to pin is 2026-10-31 (L-132) and **the re-pin is planned for after the stable `0.2.0`**; RQ-13 is `passed` on rc.1 with that residual accepted.
- **Weak points that remain:** the codes of L-127; #206's cause is unproven; #253 is random; no free-space check on Windows (L-061); L-131.

## P13 in one view (complete)

[ADR 0023](../docs/decisions/0023-r0-distribution-managed-installation-and-handoff-check.md) (**Accepted**
2026-10-01); record `p13-distribution.md`. **Delivered:** native archives, the npm launcher, managed
installation, human output, `handoff check`, the 0.1.0 pre-release (`011bc4d`). **Not proved:** Smart App
Control, the macOS prompts (L-098); power loss beyond Ubuntu 24.04 ext4 (L-037).

## What works (public CLI)

- `setup check|configure|configure-model|plan`; `setup install` on Ubuntu 24.04 x86-64;
  `setup list|rollback|remove|repair` on every OS; `ingest <video> [--transcript <file>
  [--transcript-offset <signed us>]]`; `transcript retranscribe|get`; `job status|resume|cancel|
  run|batch`; `search`, `candidates`, `frame get|neighbours|burst`, `crop`, `audio`; `session
  list|status|renew|close|retain|clean|init-workspace`; `bundle validate`.
- `handoff check`: a draft from stdin or `--file`. A release build's `--version` names its source
  commit. Global `--session-root`, `--host-isolation`, `--json`, `--events jsonl`; readable text without
  `--json` (unstable).

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00-P05 | Complete; merge commits and evidence are in the ledger |
| P06-P10 | Complete: detect, select, verify, guide (`b73df52`); engine, transcripts, local ASR (`9ea3180`); search, candidates (`b830fc9`); frames, crops, audio (`e57c706`); jobs, resume, durable Ubuntu/ext4 (`3f27ce3`) |
| P11 | Complete (`40c4038`); SEC-T01 adversarial evidence is technical debt, moved to R1 (#188, L-068) |
| P12 | Complete (2026-09-30, ADR 0022 Accepted): skill, harness, named-client trials; review tier qualified, compact tier 93% and 100% on the #222 re-run; open: L-095 (#224), #219, #204 (`1284e54`) |
| P13 | Complete (2026-10-01, ADR 0023 Accepted): distribution, managed install, `handoff check`, human output; 0.1.0 published; release commit `011bc4d` |
| P14 | **In progress** (started 2026-10-02): PRs 0-9 merged (PR 7 = the fixes of the campaigns and batch 1); 10: rc.1 published and superseded, **rc.2 published 2026-10-07**; 11 repeated on rc.2: **hosted part recorded (RQ-08 and RQ-10 waived) and batch 2 read (RQ-15 waived: Claude Opus's review tier excluded)**; batch 3 and the try-outs still to do; 12-13 remain |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <- `vsift-infrastructure`
(OS, processes, storage, providers, parsers) <- `vsift` (engine) <- `vsift-cli` (parse, present, signals).
`vsift-contract` sits beside the engine and owns the wire types. The skill only calls the `vsift` binary;
the trial harness uses only `vsift` and `vsift-contract`; `tools/` (never shipped) qualifies; `npm/` is the launcher.

## Quality evidence

- **Local gates** (in each PR description): fmt, strict Clippy, workspace tests, warning-denied rustdoc, governance.
  **CI on every PR:** Quality (three OS), Documentation, Governance, fuzz replay, worker boundary, dependency policy,
  CodeQL, npm launcher tests; the Release dry run and P14 workflows on their paths. **Required on `main`:** Quality,
  Documentation, Dependency policy, Analyze Rust, Governance. Squash merges.
