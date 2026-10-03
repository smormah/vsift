# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress: decisions A-H confirmed
2026-10-02.** The plan is 14 pull requests (0-13). **PRs 0 (#250), 1 (#251), 2 (#255), 3 (#254, the
journeys), 5 (#276, SEC-T01 narrowed), 6 (#262, the agent-trial harness) and 8 (#252) are merged; PR 4
(#259, the robustness campaigns) is ready for review, not merged; PR 7 (fixes) is under way; agent-trial
batch 1 ran on 2026-10-03.** The packet is not complete. Plan: `docs/planning/p14-qualification.md`
(section 15: PR 2; 17: PR 3; 18: PR 4); ADR 0024 stays Proposed. **Evidence:** `p14-evidence-ledger.json`,
all **for 0.1.0 only**: `passed` RQ-01 to RQ-04, RQ-06, RQ-07, RQ-09, RQ-12, RQ-19; **`failed` with
issues: RQ-08, RQ-10, RQ-13**; RQ-05 `running`; RQ-14 `waived`; 6 `planned`. **Public text:**
`public-claims.json` (rung `now`).
**Decided 2026-10-02 (ADR 0024):** **A** R0 is `0.2.0` on `latest`. **B** `0.2.0-rc.N` under `next`, never
announced. **C** no signing unless try-outs show a block. **D** 84 agent runs in three batches, each on
the go. **E** SEC-T01 narrowed. **F** "supported" per cell by fixed rules; managed install Ubuntu-only.
**G** a claims ladder; nothing announced before P14 completes. **H** a try-out blocks the stable only
until observed. #246 waits.

**PR 4 in one view** (plan section 18; `tools/p14-campaigns/`; **hosted runners only, nothing
published, no product code changed**). **RQ-07 passed:** 31 fuzz targets x 3,601 s, 3.68 billion runs,
no crash; 19 still growing (L-128). **RQ-08 failed:** Windows reproduced #206 (7 of 1,500) and a
weighted-admission starvation (#271, 2 of 200); #128 did not reproduce; Ubuntu and macOS clean (L-123).
**RQ-09 passed:** ladder 1-8, 100-request batch, cancel, warm page p95 5 ms, a 1,000-request soak with
kills (and two longer ones, 18.3); found on the way #274 (a 5 s range of F02/F04/F05 fails as
`MISSING_CAPABILITY`, L-124), #277 and #286 (the dedupe window ends with the session; runbook corrected).
**RQ-10 failed:** 96 hostile inputs; #264 (a pipe hangs `ingest`), #265, #266 (L-127); `TRACKED` in
`hostile-media.cjs` lets a run with only those pass. **RQ-12 passed:** runbook walked, 18 steps, ten
divergences fixed. **RQ-13 failed:** #272, the FFmpeg snapshot lacks 17 fixes (L-122); the rest is clean.
**After the squash, re-point the ledger commits of RQ-07, 08, 09 and 12 to the merge commit.**

**PR 3 in one view** (`docs/development.md`): `VSIFT_E2E_BINARY` makes the real-tool checkpoints drive an
installed `vsift`; `P14 journeys` runs them against `vsift-cli@<version>` from the real registry on three
systems, weekly too (L-114, L-115). On 0.1.0 all passed (53 stages each); P11's durable stage cannot run
on a hosted runner (#258, L-113; PR 4 used an ext4 volume in a file); #263.

**PR 6 and batch 1 in one view** (`docs/agents/trials.md`; `docs/planning/p14-agent-trials/batch-1-reading.md`):
the harness installs the published package and runs cold mode (**Claude strict**; **Codex realistic**:
L-125), hold-outs, `freeze` and usage capture. **Batch 1 (0.1.0, 20 runs, a baseline):** skill pilots 4 of
4; cold useful 1 of 6 (Claude), 2 of 6 (Codex); none installed anything; safety "not met" on grader
classes to settle before batch 2 (#282).

**README graphics:** SVG text is unscanned (L-121); redraw `roadmap.svg` with PRs 10 and 13. **0.1.0:** on
npm under `next` (`latest` is an empty placeholder) and a GitHub pre-release; not announced (L-105).

## The P14 pull requests (0-3, 5, 6, 8 merged; 4 ready)

**7** fixes for what 2-6 find (#206, #271, #264-#266, #274, #277, #286; #256, #257, #261, #273 are
done; `vsift --help` gets a typical-investigation section: batch 1 shows the gap); **9** matrix,
documents, claims enforced, register sheet, the R0 user guide; **10** candidate `0.2.0-rc.1`; **11** its
qualification (batches 2 and 3; dispatch `P14 journeys`; re-run RQ-07 to RQ-10 after the fixes); **12**
stable `0.2.0`; **13** ledger follow-up, P14 `complete`, handoff. **Before PR 10:** settle the
allowed-path lists in `candidate.rs`; freeze the skill, grader, scenarios and settings (`freeze
write`). **Before PR 12:** register the two stable checks in `STABLE_CHECKS`
(`tools/p14-published/lib/verify.cjs`).

## What the maintainer owes, and when

- **Now:** **the decision on #272** (refresh the FFmpeg snapshot, read the 18 records with no fix
  reference, or accept a residual; RQ-13 stays failed until then). Read `batch-1-reading.md` and settle
  its three grader questions (a path in a report; `--session-root` for a cold agent; listing a system
  folder) **before batch 2**; a grader change needs a new freeze, once, before PR 10's cut. **Batches 2
  and 3** run only on the go, on the candidate (`run-campaign.ps1 -Batch N -Client claude|codex
  -Version <rc>`); needs Claude Code 2.1.284 and Docker.
- **PR 9:** one pass over the thirty register entries the claims lean on. **PRs 10 and 12:** each
  publish (`release.md` 6.3, 6.7). **PR 11:** the go for batches 2 and 3 (L-095); a Smart App Control
  try-out on the second Windows 11 machine; no macOS try-out.

## Open decisions and readings (maintainer)

- **#204** Codex on Windows (L-076; unsupported); the grader's `untrusted_listed` reading (F12-E01)
  and #219: settled before the trial freeze.
- **Readings** (none blocks): L-062, L-067, L-069, L-017, L-088, L-090, `tokio`, `durable_worker`, MSRV.

## Tracked issues and gates

- **Open:** #17 (P14), #219, #224 (L-095), #188 (SEC-T01, R1), #232, #246 (deferred), #258, #263
  (#256, #261, #273, #282, #283, #285 fixed; #257 documented); **from PR 4:** #206 (reproduced), #264-#266,
  #271, #272, #274, #277, #286; #128 (watch: not reproduced in 3,000 per system); flaky tests #253 (a
  managed-store kill test on Windows) and #268 (a macOS SIGTERM test): comment with the run link and
  rerun the job; #170-#178 (register); #159, #150, #147; #204.
- **Opt-in real-tool paths** (`--ignored`) run a Cargo-built binary unless `VSIFT_E2E_BINARY` names an
  installed one (L-042); a failed run is a finding: issue first, then rerun. **P14 tools:**
  `tools/p14-published/test/`, `tools/p14-campaigns/test/`. **Campaigns:** never on the maintainer's
  machine (L-056, L-057) except the agent batches; dispatch as `docs/development.md` says.

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff on both
  transcript paths in named trials; P14 repeats it from a clean install. **The skill** calls the
  published CLI only; a new public item needs a skill update in the same change (`skill_contract`).
- **New public items** need their `*::ALL` entry, v1 schemas and a human renderer with a snapshot; v1
  is additive only; a parser of untrusted input needs a fuzz target; npm packages have no scripts.
- **The release seam:** a change to `vsift-release`, `release.yml` or its lint needs a lint rule, a
  broken-copy mutation test and the shell check; no workflow runs `npm dist-tag`.
- **Trials:** a change to the skill, grader, scenarios or settings after a batch's first counted
  run voids the batch (`freeze check`); a hold-out failure is a finding, never a grader edit.
  **Commits:** the commit path reruns the crash campaign; no development feature in a release.
