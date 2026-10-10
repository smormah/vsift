# VSift current status

As of 2026-10-09. Current-state document: rewrite it, don't append to it. Next actions and
open decisions are in `memory/TODO.md`.

**Note, 2026-10-10:** `0.2.0` was published on 2026-10-09 (tag `v0.2.0` at `eeb2a22a46a8`, the merge of #344; the GitHub release is not a pre-release). What this page says of PR 12 being "prepared", of nothing being published and of `latest` being `0.0.0` was written before the publish; PR 13 records the publish and its checks and rewrites it. The changes of 2026-10-10 here: the RQ-10 waiver, and **`0.2.0-rc.1` and `0.2.0-rc.2` are deprecated on all four packages (2026-10-10, read back; rc.3 and `0.2.0` are not)**.

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
- be installed without Rust: **the 0.1.0 pre-release (2026-10-01) and three release candidates are
  published under `next`** (`npm install --global vsift-cli@next`); **`0.2.0` is prepared, not published**, and
  from its publish `npm install --global vsift-cli` with no tag installs it (`latest` is still the empty
  `0.0.0`). Archives on a GitHub release carry a Sigstore attestation and npm provenance. The README shows
  0.1.0's real output (graphics: L-121).

**The agent skill** (`skills/vsift/`) teaches Claude Code or Codex to run an investigation with
the CLI and write a cited report. P12's named-client trials qualified it:
- **Review tier:** Claude Opus 5.5 in Claude Code and GPT-6-Astra in Codex (a Linux container, L-076) each passed
  11 of 11 trials mechanically and 9 of 11 fully. **Compact tier:** Claude Sonnet 5.5 and GPT-6-Sol: 82% in P12;
  the re-run (#222) meets the 90% target, Sonnet 26 of 28 and Sol 28 of 28 (23 as run).
- **Safety:** no agent leaked a secret, installed anything, acted on injected text or copied a hidden character.
  **P14's batch 2 (a clean install of each candidate) refined this: Claude Opus missed two review-tier gates on rc.1 and rc.2 (L-139); the third candidate's skill has two more evidence rules for that, and on 2026-10-08 all four models met every gate on it (34 of 34 runs; three runs per scenario, so a small sample, not a proof).**
  **Without the skill (batch 3, the cold round, rc.3): no cold agent installed anything or accepted a plan; one of 18 read an audio clip file with `base64`, which the hard gate counts, and RQ-16 is waived for rc.3 and the stable `0.2.0` (same bytes), for that one action only (L-142, #340).**

## Where the project stands

**P00-P13 are complete. P14, the R0 qualification (#17), is in progress** (started 2026-10-02; decisions A-H of ADR 0024). Its plan is 14 pull requests (0-13): **0-10 are done, PR 11's evidence
is complete with the maintainer's register pass of 2026-10-10, PR 12 (the stable `0.2.0`) is PREPARED and not complete, PR 13 follows.** The packet is not complete. Where things stand:
- **PR 12, prepared 2026-10-09 (plan section 30; checklist `docs/planning/p14-stable-release-steps.md`):** the commit the maintainer tags `v0.2.0` is on the branch `p14-pr12-stable-0.2.0` (built on #343, the
  maintainer's decisions of the day; the supervisor opens its pull request once #343 is merged). It differs from the tag `v0.2.0-rc.3` only in five version-string files, the two shipped documents (the
  installation guide and the npm README, now for `latest`) and the work record, and `candidate-delta` refuses nothing. **Nothing is tagged, published or dispatched: PR 12 is complete only when the
  maintainer has published `0.2.0` and verified it,** then deprecated `0.2.0-rc.1` and `0.2.0-rc.2` (decided 2026-10-09; **done 2026-10-10**, all eight read back). The first `P14 verify release` on `0.2.0` is red on two named checks by design until PR 13
  registers them (within seven days of the publish). **On npm today:** `next` is `0.2.0-rc.3` on all four packages (published and verified 2026-10-08, tag `v0.2.0-rc.3` at `83dca856e7a0`, publish run
  37746979716) and `latest` is the empty `0.0.0`; `0.2.0-rc.1` (2026-10-05) and `0.2.0-rc.2` (2026-10-07) are published, superseded and deprecated (2026-10-10).
- **Why a third candidate, and what it is:** batch 2 ran on rc.1 and rc.2; **Claude Opus 5.5 missed two review-tier gates both times**, so RQ-15 was `failed` for rc.2, and the maintainer chose to improve the
  skill: no waiver, no exclusion. **rc.3 is rc.2 plus** two evidence rules in the skill, a floor for short audio (under 100 ms is a gap, never recognised: #322, L-137), a refusal of an `audio` range too
  short to hold a sample (#332; one published code replaced for that request), a remediation for a copy that runs out of time (#325 step 1, L-140) and three test or tool fixes. No re-pin, no Dependabot.
- **What is shown for rc.3 (plan section 29, 2026-10-08):** the whole hosted part of PR 11 ran and was green: the publish verified (20 checks), installs, archives, offline install and upgrades from 0.1.0
  and rc.2, journeys on three systems, managed smoke, fuzzing (3.49 billion runs), **the stress run (20,100 repetitions, none failed; the fixed Windows supervisor test ran 3,000 clean, #321)**, load, runbook
  walk, both fault campaigns, the scan reading. **RQ-10 is `waived` for rc.3 and, by the maintainer's decision of 2026-10-10, for the stable `0.2.0`, for the link's `STORAGE_IO` alone** (plan 29.5; the campaign was not run on the stable's own bytes); its rule was widened on 2026-10-09 to admit `INVALID_ARGUMENT` where the
  media judge does, so the 20 inputs that end so meet it; the item is still `waived`, not `passed`. **Batch 2** (with the skill, 34 runs): **every gate met, 34 of 34 passed fully** (28 of 34 on rc.2); Claude
  Opus 5.5 passed the blurred banner 3 of 3 and the journey 6 of 6; RQ-15 `passed`; a small sample, not a proof (L-095, L-119, L-139 stay open). **Batch 3** (18 cold runs, no skill): **usefulness is met
  on both clients with no margin (5 of 6 each, target 80%); cold safety, a hard gate, is not met: 1 of 18 runs** (Codex GPT-6-Sol ran `base64` on the audio clip `vsift audio` had named; nothing installed, written or
  sent; #340, L-142). No cold run installed anything or accepted a plan.
- **The maintainer's decisions of 2026-10-09 (plan section 29.10; nothing was run again):** RQ-10's rule admits `INVALID_ARGUMENT`; **RQ-16 is `waived` for rc.3 and the stable, for that one action**; **RQ-17 (the
  clean-machine and Smart App Control try-out) is `waived` for rc.3 and the stable: they ship untried** (no Smart App Control or SmartScreen try-out, no true clean-machine install, no Mac Gatekeeper try-out;
  decision H; L-143; the try-out may still be done after the stable, and CL-201 stays unused until it is `passed`); rc.1 and rc.2 are deprecated at the stable. **`release-evidence --complete-for 0.2.0-rc.3` passes
  (exit 0).** Four items are waived (RQ-10, RQ-14, RQ-16, RQ-17) and none is a pass. **The register pass is done (2026-10-10):** the 40 limits the public statements and waivers lean on and six readings are `accepted` as proposed; no claim is raised and the rung
  stays `candidate` until PR 13. **Decided 2026-10-10:** the supervisor's reading of the 18 raw cold logs stands for the maintainer's
  own; the maintainer read neither the logs nor the command list, so no person has read them (L-118). **Decided 2026-10-10:** the RQ-10 waiver, which named rc.3 only, carries to the stable for the link case alone.
- **What is weak:** the skill change met its gates once on a small sample; the cold round met its target twice with no margin and its safety gate not at all (one harmless read; the strict Claude setting is a
  narrow test: L-125, L-118); the floor was not run with whisper.cpp (L-137, L-141); hosted images are not clean machines (L-112); Smart App Control and the macOS prompts are unseen and untried (L-098, L-143);
  **a synthetic corpus and voice only** (L-020, L-022). **Open:** #312 (L-135; one failure in 4,500 loaded Windows repetitions across three candidates; not fixed). CVE-2026-38350 is accepted (L-122).
  **`latest` has never moved, and the first move is untried** (L-105). Decisions: `TODO.md`.

## P14 PRs 1 to 6 and 8 in one view

**PR 1 (#251):** the evidence ledger (RQ-01..RQ-20) and claims registry (rung `now`, `candidate` since PR 10b), checked on every PR (L-101). **PR 2 (#255):** clean installs from the real registry
(npm, pnpm, Yarn, Bun; three systems), archives, offline install, upgrade and a second verifier (RQ-01..04, RQ-19), `passed` for 0.1.0 only (L-109 to L-112). **PR 8 (#252):** the version alone decides the
channel (a suffix means `next`, none moves `latest`), guarded by the candidate delta and the complete ledger (L-103); no real stable publish yet (L-105). **PR 3 (#254):** `P14 journeys` runs the real-tool
checkpoints against `vsift-cli@<version>` from the registry on three systems, weekly too; on 0.1.0 53 stages passed on each (RQ-06 `passed`, RQ-05 `running`: P11's durable stage is blocked, #258).

**PR 6 (#262; `docs/agents/trials.md`) and batch 1.** `tools/vsift-agent-trials` installs the published package, runs a cold agent (**Claude strict**, **Codex realistic**, L-125), keeps hold-outs, freezes
inputs by digest, captures usage and plans three batches (L-117 to L-120). **Batch 1** (2026-10-03, 0.1.0, 20 runs, a baseline): skill pilots 4 of 4; cold useful 1 of 6 (Claude), 2 of 6 (Codex); none
installed anything; the three grader classes were ruled and implemented (#298). Batch 3 (above) is the comparison.

**PR 4 (#259; plan section 18; hosted runners only)** ran the campaigns on 0.1.0 (`tools/p14-campaigns/`): RQ-07 passed (31 fuzz targets, no crash; 19 still growing: L-128); RQ-08
failed (Windows #206, #271); RQ-09 passed (found #274, #277, #286); RQ-10 failed (96 hostile inputs, 93 held: #264-#266); RQ-12 passed; RQ-13 failed (#272).

## P14 PRs 9 and 10 in one view

**PR 9 (#302, #304, #306).** The support matrix (`support-and-resource-profiles.md`: no cell may say "supported"
yet), the install guide, `SECURITY.md`, the runbook, the skill guide and the README facts brought to it; the claims
check reads the launcher's messages and the README graphics' text (L-121) and each claim lists the register entries it
leans on (`limits`); `register-review-sheet.md` (every review `pending`). **Decided 2026-10-04:** the macOS wording,
the versions policy and RQ-05's per-system rule (still `running`, plan section 21). `docs/guide/`
(twelve pages, two generated) is held to the code by the `Guide` workflow (`tools/guide/`): 40 marked commands print what
the pages show. **PR 10 (the candidate; rc.1 published 2026-10-05, rc.2 on 2026-10-07, rc.3 on 2026-10-08).**
The version is `0.2.0-rc.3` in the tagged candidate (`0.2.0` in the prepared stable commit), the guide's marker is the release `0.2.0`, rung `candidate`. **The freeze of batches 2 and 3 is new:**
the skill's digest changed on purpose, the other six components are rc.2's, and a test pins the whole digest (`654955dd...`); the earlier batch-2 records are in
`batch-2-rc.1/` and `batch-2-rc.2/`, rc.3's in `batch-2/` and `batch-3/`. From the tag to the stable only a work record, `install.md` and the npm README may merge (so Dependabot waits: #192, #193, #194 after the
stable); the check compares the stable with the highest `-rc.N` tag: `v0.2.0-rc.3` (`release.md` 6.12).

## P14 PR 7 in one view (every finding by outcome; one pull request and one regression test each)

**Rule of 2026-10-04: a published failure code stays (v1 is additive, L-126); the remediation carries the fix (L-127).**
- **Fixed:** #264 a pipe no longer hangs `ingest` (#290); #274 a range cut mid-speech keeps its last segment (#289, L-130); #277 a failed open removes its registration
  (#293, L-131); #268 the shutdown remediation (#288); #256 a missing shared library is named (#280); #261 a foreign session root explains itself (#279, L-126); #273, #282, #283, #285
  tool fixes (#275, #287). **Answer fixed, code kept (L-127):** #265 a link (#295), #266 no room, best effort on Unix (#291, L-061), #277's not-published answer: all `STORAGE_IO`.
  **Narrowed, not proven gone:** #206 the root's DACL is read back and repeated (#301, L-005); #253 the kill test ends its strays (#300, L-129). **Tests or documents only:** #271
  (#294, L-060); #286 the dedupe window (#299, L-063: an R1 stub is the maintainer's call); #257 the `vsift.cmd` shim (#281, L-109). **PR 7b, #272 (not fixed):** 46 of 47 scan records are
  fixed in the shipped FFmpeg (L-122); the next month-end build to pin is 2026-10-31 (L-132) and **the re-pin is planned for after the stable `0.2.0`**; RQ-13 is `passed` with that residual accepted.
- **Weak points that remain:** the codes of L-127; #206's cause is unproven; #253 is random; no free-space check on Windows (L-061); L-131.

## P13 in one view (complete)

[ADR 0023](../docs/decisions/0023-r0-distribution-managed-installation-and-handoff-check.md) (**Accepted** 2026-10-01); record `p13-distribution.md`. **Delivered:** native archives, the npm launcher,
managed installation, human output, `handoff check`, the 0.1.0 pre-release (`011bc4d`). **Not proved:** Smart App Control, the macOS prompts (L-098); power loss beyond Ubuntu 24.04 ext4 (L-037).

## What works (public CLI)

- `setup check|configure|configure-model|plan`; `setup install` on Ubuntu 24.04 x86-64;
  `setup list|rollback|remove|repair` on every OS; `ingest <video> [--transcript <file>
  [--transcript-offset <signed us>]]`; `transcript retranscribe|get`; `job status|resume|cancel|
  run|batch`; `search`, `candidates`, `frame get|neighbours|burst`, `crop`, `audio`; `session
  list|status|renew|close|retain|clean|init-workspace`; `bundle validate`.
- `handoff check`: a draft from stdin or `--file`. A release build's `--version` names its source commit. Global `--session-root`, `--host-isolation`, `--json`, `--events jsonl`;
  readable text without `--json` (unstable).

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00-P05 | Complete; merge commits and evidence are in the ledger |
| P06-P10 | Complete: detect, select, verify, guide (`b73df52`); engine, transcripts, local ASR (`9ea3180`); search, candidates (`b830fc9`); frames, crops, audio (`e57c706`); jobs, resume, durable Ubuntu/ext4 (`3f27ce3`) |
| P11 | Complete (`40c4038`); SEC-T01 adversarial evidence is technical debt, moved to R1 (#188, L-068) |
| P12 | Complete (2026-09-30, ADR 0022 Accepted): skill, harness, named-client trials; review tier qualified, compact tier 93% and 100% on the #222 re-run; open: L-095 (#224), #219, #204 (`1284e54`) |
| P13 | Complete (2026-10-01, ADR 0023 Accepted): distribution, managed install, `handoff check`, human output; 0.1.0 published; release commit `011bc4d` |
| P14 | **In progress** (started 2026-10-02): PRs 0-10 done (PR 7 = the fixes of the campaigns and batch 1; 10: three candidates published, rc.3 on 2026-10-08); PR 11's evidence is complete (RQ-10, RQ-14, RQ-16 and RQ-17 waived, the rest passed; the register pass remains); **PR 12 is PREPARED, not done: the stable commit `0.2.0` is on the branch `p14-pr12-stable-0.2.0`, nothing is tagged or published, and it is complete only when the maintainer has published and verified it (the deprecation of rc.1 and rc.2 is done, 2026-10-10)**; then 13 |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <- `vsift-infrastructure`
(OS, processes, storage, providers, parsers) <- `vsift` (engine) <- `vsift-cli` (parse, present, signals).
`vsift-contract` sits beside the engine and owns the wire types. The skill only calls the `vsift` binary;
the trial harness uses only `vsift` and `vsift-contract`; `tools/` (never shipped) qualifies; `npm/` is the launcher.

## Quality evidence

- **Local gates** (in each PR description): fmt, strict Clippy, workspace tests, warning-denied rustdoc, governance. **CI on every PR:** Quality (three OS), Documentation, Governance, fuzz
  replay, worker boundary, dependency policy, CodeQL, npm launcher tests; the Release dry run and P14 workflows on their paths. **Required on `main`:** Quality, Documentation, Dependency
  policy, Analyze Rust, Governance. Squash merges.
