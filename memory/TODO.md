# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress: decisions A-H confirmed
2026-10-02.** The plan is 14 pull requests (0-13). **PRs 0 (#250), 1 (#251), 2 (#255), 3 (#254), 4 (#259),
5 (#276), 6 (#262), 7 (the fixes, below) and 8 (#252) are merged; agent-trial batch 1 ran on 2026-10-03.**
**PR 9 is merged (#302, #304, #306). PR 10, the candidate `0.2.0-rc.1`, is prepared and not published:
10a (#307, the skill's wording) and 10b with 10c (the cut and `release.md` 6.10) are open.** The packet is not
complete. Plan: `docs/planning/p14-qualification.md` (sections 15-23); ADR 0024 stays Proposed.
**Evidence:** `p14-evidence-ledger.json`, all **for 0.1.0 only** and stale for the candidate: `passed` RQ-01 to
RQ-04, RQ-06, RQ-07, RQ-09, RQ-12, RQ-19; **`failed`: RQ-08, RQ-10, RQ-13** (PR 7 fixes them; PR 11 re-runs them);
RQ-05 `running` (L-113; plan section 21); RQ-14 `waived`; 6 `planned`.
**Public text:** `public-claims.json`, rung `candidate` (CL-101 and CL-102 are in use; both need RQ-19, passed for
0.1.0 only, L-133); a claim above the rung fails while a register entry it leans on (`limits`) is pending.
**Decided 2026-10-02 (ADR 0024):** **A** R0 is `0.2.0` on `latest`. **B** `0.2.0-rc.N` under `next`, never announced.
**C** no signing unless try-outs show a block. **D** 84 agent runs in three batches, each on the go. **E** SEC-T01
narrowed. **F** "supported" per cell by fixed rules; managed install Ubuntu-only. **G** a claims ladder; nothing
announced before P14 completes. **H** a try-out blocks the stable only until observed. #246 waits.

**PR 7, every finding by outcome** (one pull request and one regression test each). **Maintainer rule
2026-10-04: a published failure code stays (v1 is additive); the remediation carries the fix (L-127).**
- **Fixed:** #264 a pipe no longer hangs `ingest` (#290); #274 a range cut mid-speech keeps its last segment
  (#289, L-130; L-124 closed); #277 a failed open removes its registration (#293, L-131); #268 the shutdown
  remediation (#288); #256 a missing library is named (#280); #261 a foreign session root explains itself
  (#279, L-126); #273, #282, #283, #285 tool fixes (#275, #287).
- **Answer fixed, code kept (L-127):** #265 a link says what happened, `STORAGE_IO` (#295); #266 no room is
  refused before the copy, `STORAGE_IO`, best effort on Unix (#291, L-061); #277's not-published answer.
- **Narrowed, not proven gone:** #206 the root's DACL is read back and repeated (#301; L-005); #253 the kill test
  ends its strays (#300; the Windows window is L-129). **Tests or documents only:** #271 (#294, L-060); #286 the
  dedupe window is stated (#299; an R1 stub is the maintainer's call, L-063); #257 the `vsift.cmd` shim (#281, L-109).
- **Not fixed yet:** #272 the FFmpeg snapshot: 46 of 47 records are fixed in the shipped build (#296, L-122); a pin
  must be a month-end build, the next is 2026-10-31 (L-132). RQ-13 stays `failed` until re-read.
- **Text and grader:** `vsift --help` carries a typical investigation (#297); the cold grader notes a path,
  `--session-root` and a system-folder listing, not a failure (#298; the freeze of batches 2 and 3 is committed).

**Earlier P14 PRs** (plan sections 15-19; `docs/development.md`; `docs/agents/trials.md`). **PR 4**
(`tools/p14-campaigns/`, hosted runners only): RQ-07 passed (31 fuzz targets, 19 still growing: L-128), RQ-09
and RQ-12 passed, RQ-08, RQ-10 and RQ-13 failed (the findings above). **PR 3:** `P14 journeys` runs the
real-tool checkpoints against `vsift-cli@<version>` from the registry on three systems, weekly too (L-114,
L-115); P11's durable stage cannot run hosted (#258, L-113). **PR 6 and batch 1:** the harness runs cold mode
(Claude strict, Codex realistic: L-125), hold-outs, `freeze` and usage capture; batch 1 (0.1.0, 20 runs, a
baseline): skill pilots 4 of 4, cold useful 1 of 6 (Claude) and 2 of 6 (Codex), nothing installed; the three
grader questions are ruled (#298). **README graphics:** redraw `roadmap.svg` with PRs 10 and 13 (L-121).
**0.1.0:** on npm under `next` (`latest` is an empty placeholder) and a GitHub pre-release; not announced (L-105).

## The remaining P14 pull requests (0-9 merged)

**10** is complete only when the maintainer has published the candidate and verified it: **10a** (#307) the skill's
wording; **10b and 10c** the bump and guide re-run, the changelog, the claims rung, the freeze of batches 2 and 3
(committed, with a test), the allowed lists of the candidate-to-stable check (a work record and `install.md` are now
allowed, `release.md` 6.8) and the maintainer's exact steps (`release.md` 6.10). **From the tag to the stable merge
nothing else** (no Dependabot, workflow or tool change). **11** its qualification (batches 2 and 3; `P14 journeys`;
re-run RQ-07 to RQ-10); **12** stable `0.2.0`; **13** ledger follow-up, P14 `complete`, handoff. **After the stable
tag, within seven days:** register the two stable checks in `STABLE_CHECKS` (`tools/p14-published/lib/verify.cjs`).

## What the maintainer owes, and when

- **Now:** **PR 10b:** confirm the widened allowed lists (ADR 0024's PR 10b note, decision 1), then **publish** by
  `release.md` 6.10 (merge, tag, dry run, publish, verify; the "up to date" rule on `main` back on first). **The skill:**
  read #307's three wording changes. **Dependabot:** #195 merged; #192, #193, #194 wait until after the stable.
  **The FFmpeg re-pin** (#272, L-132) is planned for after the stable. **Batches 2 and 3** run only on the go, on the
  candidate (`run-campaign.ps1 -Batch N -Client claude|codex -Version <rc>`; batch 3 needs `-AllowGraderChange`);
  needs Claude Code 2.1.284 and Docker.
- **PR 9:** one pass over `register-review-sheet.md` (thirty entries, seven later, nine readings: every review is
  `pending`), how RQ-05 closes for 0.1.0 (a `P07 local ASR` dispatch at its commit, or the weekly runs accepted),
  and whether `Guide` becomes a required check. **PRs 10 and 12:** each publish (`release.md` 6.3, 6.7). **PR 11:** the go for
  batches 2 and 3 (L-095); a Smart App Control try-out on the second Windows 11 machine; no macOS try-out.

## Open decisions and readings (maintainer)

- **R1 options:** a stub per ended request (L-063); close the Windows window (L-129, an ADR); a bounded wait
  for the initialization lock (L-131). **At v2:** revisit the codes L-127 keeps. **#204** Codex on Windows
  (L-076); the grader's `untrusted_listed` reading (F12-E01) and #219: settled before the trial freeze.
  **Readings** (none blocks): L-062, L-067, L-069, L-017, L-088, L-090, `tokio`, `durable_worker`, MSRV.

## Tracked issues and gates

- **Open:** #17 (P14), #219, #224 (L-095), #188 (SEC-T01, R1), #232, #246 (deferred), #258, #263, #272; #128
  (watch: not reproduced in 3,000 per system); flaky tests #253 (Windows kill test, mitigated) and #268 (macOS
  SIGTERM test): comment with the run link, rerun the job; #170-#178 (register); #159, #150, #147.
- **Opt-in real-tool paths** (`--ignored`) run a Cargo-built binary unless `VSIFT_E2E_BINARY` names an installed
  one (L-042); a failed run is a finding: issue first, then rerun. A stage that tests a fix newer than the published
  version skips below its first version only (L-115). **P14 tools:** `tools/p14-published/test/`, `tools/p14-campaigns/test/`.
  **Campaigns:** never on the maintainer's machine (L-056, L-057) except the agent batches; see `docs/development.md`.

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
