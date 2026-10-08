# VSift current status

As of 2026-10-08. Current-state document: rewrite it, don't append to it. Next actions and
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
  **P14's batch 2 (a clean install of each of the first two candidates) narrows this: Astra, Sonnet and Sol met their gates; Claude Opus missed two review-tier gates on both (L-139). The third candidate's skill has two more evidence rules for that; no trial has run with them yet.**

## Where the project stands

**P00-P13 are complete. P14, the R0 qualification (#17), is in progress** (started 2026-10-02; decisions A-H of ADR 0024 confirmed). Its plan is 14 pull requests (0-13); **PRs 0-10
are done. The third candidate `0.2.0-rc.3` was published and verified on 2026-10-08** (tag `v0.2.0-rc.3` at `83dca856e7a0`, publish run 37746979716): npm `next` is rc.3 on all four packages and
`latest` is the empty `0.0.0`. The first two candidates are published, superseded and not deprecated: `0.2.0-rc.1` (2026-10-05, `d5792ce31db1`) and `0.2.0-rc.2` (2026-10-07, `7c722d1fc46a`). The packet is
not complete. Where things stand:
- **Why a third candidate:** batch 2 (34 runs with the skill) ran on rc.1 and rc.2; Codex met its gates both times, **Claude Opus 5.5 missed two review-tier gates both times** (rc.2: 4 of 6
  mechanically, 1 of 3 on the blurred banner), so RQ-15 is `failed` for rc.2. The maintainer chose to improve the skill and run the round again: no waiver, no exclusion.
- **What rc.3 is:** rc.2 plus two evidence rules in the skill (an unreadable region proves nothing either way; a claim states only what its own citations show or say), a floor for short
  audio (under 100 ms is a gap, never recognised: #322, L-137), a refusal of an `audio` range too short to hold a sample (#332; one published code replaced for that request), a remediation
  for a copy that runs out of time (#325 step 1, L-140) and three test or tool fixes (#321, the campaign's no-room case, #327). No re-pin of FFmpeg or whisper.cpp, no Dependabot.
- **What is shown for rc.3 (plan section 29, 2026-10-08; work record only):** the whole hosted part of PR 11 ran and was green: the publish verified (20 checks), installs, archives, offline install
  and upgrades from 0.1.0 and rc.2 (the package carries the tag's skill byte for byte), journeys on three systems, managed smoke, fuzzing (3.49 billion runs), **the stress run (20,100 repetitions, none
  failed; the fixed Windows supervisor test ran 3,000 clean, #321)**, load, runbook walk, both fault campaigns, the scan reading. RQ-01 to RQ-09, RQ-11 to RQ-13, RQ-18 and RQ-19 are `passed` for rc.3; RQ-14
  waived. **RQ-10 is `waived` for rc.3 only (maintainer, 2026-10-08), for the link's `STORAGE_IO` alone** (the rule names three codes; the run is green otherwise; plan 29.5); RQ-15 is rc.2's `failed`; RQ-16 and
  RQ-17 never ran. `release-evidence --complete-for 0.2.0-rc.3` names those three. **Still to do:** batch 2 (new freeze), batch 3, the clean-machine try-out, the register pass.
- **What is weak:** the skill change is a hypothesis until batch 2 runs; the floor was not run with whisper.cpp (L-137, L-141); hosted images
  are not clean machines (L-112); Smart App Control and the macOS prompts are unseen (L-098); **a synthetic corpus and voice only** (L-020,
  L-022). **Open:** #312 (L-135; one failure in 4,500 loaded Windows repetitions across three candidates; not fixed). CVE-2026-38350 is accepted (L-122). **`latest` has never moved** (L-105). Decisions: `TODO.md`.

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
the pages show. **PR 10 (the candidate; rc.1 published 2026-10-05, rc.2 on 2026-10-07, rc.3 on 2026-10-08).**
The version is `0.2.0-rc.3` everywhere, the guide's marker is the release `0.2.0`, rung `candidate`. **The freeze of batches 2 and 3 is new:**
the skill's digest changed on purpose, the other six components are rc.2's, and a test pins the whole digest (`654955dd...`); the earlier
batch-2 records are in `batch-2-rc.1/` and `batch-2-rc.2/`. The allowed lists of the candidate-to-stable check (a work record and
`install.md` are allowed; nothing else may merge from the tag to the stable, so Dependabot waits: #192, #193, #194 after the stable) are
unchanged, and the check compares the stable with the highest `-rc.N` tag: `v0.2.0-rc.3` (`release.md` 6.12).

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
| P14 | **In progress** (started 2026-10-02): PRs 0-10 done (PR 7 = the fixes of the campaigns and batch 1); 10: rc.1 and rc.2 published and superseded; 11 repeated on rc.2: batch 2 read (RQ-15 failed: Claude Opus missed two gates); **rc.3 (the skill's two evidence rules and five fixes) published 2026-10-08 and its hosted evidence recorded (RQ-08 passed, RQ-10 waived for the link case only)**; still to do: batch 2 and 3, the try-out, the register pass; then 12-13 |

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
