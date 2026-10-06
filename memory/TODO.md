# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress** (decisions A-H, 2026-10-02). Plan: 14 pull
requests (0-13); PRs 0-9 are merged. **PR 10, the candidate, has been done twice.** The first, `0.2.0-rc.1`, was published on
2026-10-05 (tag `v0.2.0-rc.1` at `d5792ce31db1`, npm `next` on all four packages, a pre-release with ten files, `latest` still the
empty `0.0.0`); it stays published and is superseded. **The second, `0.2.0-rc.2`, is prepared (the change that wrote this file) and
READY TO TAG, NOT PUBLISHED: it is rc.1 plus the fixes of #314 (a Windows reader told "damaged", #318) and #310 (an over-limit source
answered `STORAGE_IO`, #319) and nothing else.** PR 10 is complete again only when the maintainer has published it and verified it
(`release.md` 6.11). **PR 11, the qualification, is to be repeated on rc.2**; what it recorded for rc.1 is history. The packet is not
complete. Plan: `docs/planning/p14-qualification.md` (15-25); ADR 0024 stays Proposed.
**Evidence** (`p14-evidence-ledger.json`) is still the first candidate's, and **stale for rc.2** wherever the crates or a version
string is in an item's scope: `release-evidence --complete-for 0.2.0-rc.2` fails on 17 of 20 items (plan 25.2). For rc.1: `passed`
RQ-01 to RQ-07, RQ-09, RQ-11, RQ-12, RQ-13 (L-122's residual accepted), RQ-18, RQ-19; `waived` RQ-10 (L-134; **its premise is gone and
it does not expire by itself**: re-run the media campaign and decide) and RQ-14; **`failed` RQ-08** (#312 open, L-135; #314 fixed, so
rc.2 is where it is tried); `planned` RQ-15 to RQ-17 (batch 2 ran on rc.1 on the maintainer's machine: records in
`p14-agent-trials/batch-2-rc.1/`, four readings open in `batch-2-reading.md`, nothing claimed).
**Public text:** `public-claims.json`, rung `candidate` (CL-101 and CL-102 name rc.2 in the README and `install.md`; both need RQ-19,
passed for rc.1 only until rc.2 is verified: L-133 reopened); a claim above the rung fails while a register entry it leans on is pending.
**Decided 2026-10-02 (ADR 0024):** **A** R0 is `0.2.0` on `latest`. **B** `0.2.0-rc.N` under `next`, never announced. **C** no
signing unless try-outs show a block. **D** 84 agent runs in three batches, each on the go. **E** SEC-T01 narrowed. **F** "supported"
per cell by fixed rules; managed install Ubuntu-only. **G** a claims ladder; nothing announced before P14 completes. **H** a try-out
blocks the stable only until observed. #246 waits.

**What the cut changed** (details: plan 25.1, ADR note of 2026-10-06): the version in five files, the pointers (README, `install.md`,
the guide's first page, the try-out sheet, the batch checklist), `release.md` 6.11 (6.10 is the rc.1 record), the freeze files rewritten
at `3f101e7ac69a` with the **same digests** (`committed_freeze` unchanged), batch-2's rc.1 records moved aside. The candidate-to-stable
check needed no change: it compares the stable release with the highest `-rc.N` tag, so with `v0.2.0-rc.2`.

**PR 7, every finding by outcome** (one pull request and one regression test each). **Maintainer rule
2026-10-04: a published failure code stays (v1 is additive); the remediation carries the fix (L-127).**
- **Fixed:** #264 a pipe no longer hangs `ingest` (#290); #274 a cut range keeps its last segment (#289, L-130); #277 a failed open
  removes its registration (#293, L-131); #268 (#288); #256 a missing library is named (#280); #261 a foreign session root explains
  itself (#279, L-126); #273, #282, #283, #285 tool fixes. **Since rc.1:** #314 the reader's retry budget (#318; L-136 is what remains on
  Windows), #310 the size limit answers before the room check (#319, so L-127 keeps no deviation).
- **Answer fixed, code kept (L-127):** #265 a link (#295), #266 no room (#291, L-061), #277's not-published answer.
- **Narrowed, not proven gone:** #206 the DACL read-back (#301; L-005); #253 the kill test (#300; L-129); #312 the race tests wait 60 s
  (L-135). **Tests or documents only:** #271 (#294, L-060); #286 (#299; an R1 stub is the maintainer's call, L-063); #257 (#281, L-109).
- **Not fixed:** #272 the FFmpeg snapshot: 46 of 47 records are fixed in the shipped build (#296, L-122); a pin must be a month-end
  build, the next is 2026-10-31 (L-132), **after `0.2.0`**.

**Earlier P14 PRs** (plan 15-24; `docs/development.md`; `docs/agents/trials.md`). **PR 4** (`tools/p14-campaigns/`, hosted runners only):
31 fuzz targets, load, malicious media, runbook walk, scan reading. **PR 3:** `P14 journeys` runs the real-tool checkpoints against
`vsift-cli@<version>` on three systems, weekly too (L-114, L-115; P11's durable stage cannot run hosted: #258, L-113). **PR 6 and batch 1:** the harness runs cold mode (Claude strict, Codex
realistic: L-125), hold-outs, `freeze` and usage capture; batch 1 (0.1.0, 20 runs) was a baseline. **README graphics:** redraw
`roadmap.svg` with PRs 10 and 13 (L-121). **0.1.0:** on npm under `next` (`latest` is an empty placeholder), a GitHub pre-release (L-105).

## The remaining P14 pull requests (0-9 merged; 10 prepared twice)

**10** publish rc.2 (the maintainer, `release.md` 6.11). **11 repeated** on rc.2 (plan 25.3): the hosted campaigns (`P14 verify release`,
`published artifacts` twice, `journeys`, `managed smoke` from `main` with the version; `stress`, `Fuzz`, `P10`, `power loss`, `compatibility`,
`P07` at the tag), the agent batches 2 and 3, the try-outs (`rq-17-tryout-sheet.md`; the execution-policy note for `install.md` only after
they observe it), the scan reading again, the ledger entries, the register pass. **12** stable `0.2.0`; **13** ledger follow-up, P14
`complete`, handoff. **From the tag to the stable merge only the work record and `install.md` change** (`release.md` 6.8: no Dependabot,
workflow or tool change). **After the stable tag, within seven days:** register the two stable checks in `STABLE_CHECKS`
(`tools/p14-published/lib/verify.cjs`).

## What the maintainer owes, and when

- **Now:** merge the cut **only when you can tag and publish at once**, then do `release.md` 6.11 (the "up to date" rule on `main` on
  first; the commit is the merge commit; dry run, read its plan, publish; four dispatches; optionally deprecate rc.1 after them).
  Then the go for batch 2 (again) and batch 3 (`-AllowGraderChange`; Claude Code 2.1.284 and Docker; state the reserve rule first; L-095),
  the Smart App Control try-out on the second Windows 11 machine, and #312: fix or accept after the repeat. **Dependabot:** #192, #193,
  #194 wait until after the stable. **The FFmpeg re-pin** (#272, L-132) is planned for after the stable.
- **Register:** one pass over `register-review-sheet.md` (thirty entries, seven later, nine readings: every review is `pending`), and
  whether `Guide` becomes a required check. **PR 12:** the stable publish (`release.md` 6.7).

## Open decisions and readings (maintainer)

- **R1 options:** a stub per ended request (L-063); the Windows kill window (L-129, an ADR); a bounded wait for the initialization
  lock (L-131, L-135); retry the Windows sharing violations (L-136). **At v2:** the codes L-127 keeps. **#204** Codex on Windows
  (L-076). **Readings** (none blocks): L-062, L-067, L-069, L-017, L-088, L-090, `tokio`, `durable_worker`, MSRV.

## Tracked issues and gates

- **Open:** #17 (P14), #219, #224 (L-095), #188 (SEC-T01, R1), #232, #246 (deferred), #258, #263, #272 (residual accepted, L-122),
  #310 (fixed in rc.2; L-134 keeps the campaign's case), #312 (L-135), #314 (fixed in rc.2; L-136), #316 (hosted runners scarce);
  #128 (watch); flaky tests #253 (Windows kill test) and #268 (macOS SIGTERM test): comment with the run link, rerun the job;
  #170-#178 (register); #159, #150, #147.
- **Opt-in real-tool paths** (`--ignored`) run a Cargo-built binary unless `VSIFT_E2E_BINARY` names an installed one (L-042); a failed
  run is a finding: issue first, then rerun. A stage that tests a fix newer than the published version skips below its first version
  only (L-115). **P14 tools:** `tools/p14-published/test/`, `tools/p14-campaigns/test/`. **Campaigns:** never on the maintainer's
  machine (L-056, L-057) except the agent batches; see `docs/development.md`.

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
