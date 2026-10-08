# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress** (decisions A-H, 2026-10-02). Plan: 14 pull
requests (0-13); PRs 0-10 are done. **PR 10 is complete a third time: `0.2.0-rc.3` was published and verified on 2026-10-08** (tag `v0.2.0-rc.3` at
`83dca856e7a00fc9a71c87baae99f0b1d401dd31`, publish run 37746979716; npm `next` is `0.2.0-rc.3` on all four packages and `latest` is the empty `0.0.0`). The first two
candidates are published and superseded, and stay on npm: `0.2.0-rc.1` (2026-10-05, `d5792ce31db1`) and `0.2.0-rc.2` (2026-10-07, `7c722d1fc46a`); none is deprecated. The packet is not
complete. Plan: `p14-qualification.md` (15-29); ADR 0024 stays Proposed.
**What rc.3 is:** rc.2 plus exactly (1) **two evidence rules in the agent skill** (an unreadable region proves nothing about its content in either direction; a claim states only what its own
citations show or say), the answer to batch 2's two missed gates (L-139, L-095); (2) #330: audio under 100 ms is a gap and never reaches the recogniser (#322, L-137), and `audio` of a range
of 31 microseconds or less is refused as `INVALID_ARGUMENT` (#332; one published code replaced for that one request); (3) #331: a copy that outruns the ten-minute limit says so, code kept
(#325 step 1, L-140); (4) #333: test and tool fixes only (#321's marker race, L-138; the campaign's no-room case, L-134; #327). No FFmpeg or whisper.cpp re-pin and no Dependabot.
**The agent-trial freeze is NEW on purpose** (`batch-2/` and `batch-3/freeze.json`): only the `skill` digest changed; whole-freeze digest `654955dd...`, pinned in `committed_freeze`. rc.2's batch-2
records are in `p14-agent-trials/batch-2-rc.2/`, rc.1's in `batch-2-rc.1/`. **Batch 2 and 3 have NOT run on rc.3.**
**PR 11, the hosted part, is repeated on rc.3 and recorded (plan section 29, `p14-scan-reading-2026-10-08.md`; work record only; no run failed, nothing re-run).** Every campaign was green: verify (20
checks), clean installs/archives/offline install and upgrades from 0.1.0 and rc.2 (the package carries the tag's skill byte for byte), journeys on 3 systems, managed smoke, fuzz (31 targets, 3.49
billion runs), **stress 25 jobs and 20,100 repetitions with none failed (the Windows supervisor test fixed in rc.3 ran 3,000 clean, #321)**, load, runbook walk, both fault campaigns, scan reading
(nothing new). **Evidence** (`p14-evidence-ledger.json`): `passed` for rc.3 RQ-01 to RQ-09, RQ-11, RQ-12, RQ-13, RQ-18, RQ-19; `waived` RQ-14 only; **`failed` RQ-10 against its own rule** (the run is
green, but the link's `STORAGE_IO` (#265) is outside the three codes the rule names, and no waiver was invented: **the maintainer decides**, plan 29.5); `failed` RQ-15 (rc.2's batch; not run on rc.3);
`planned` RQ-16, RQ-17. **`release-evidence --complete-for 0.2.0-rc.3 --commit 83dca856...` names 4 of 20: RQ-10, RQ-15, RQ-16, RQ-17.**
**Public text:** `public-claims.json`, rung `candidate`; the README and `install.md` name rc.3 as the candidate under qualification (L-133's third window closed with the publish).
**Decided 2026-10-02 (ADR 0024):** **A** R0 is `0.2.0` on `latest`. **B** `0.2.0-rc.N` under `next`, never announced. **C** no
signing unless try-outs show a block. **D** 84 agent runs in three batches, each on the go. **E** SEC-T01 narrowed. **F** "supported"
per cell by fixed rules; managed install Ubuntu-only. **G** a claims ladder; nothing announced before P14 completes. **H** a try-out
blocks the stable only until observed. #246 waits.

**Decided by the maintainer on 2026-10-07 and 2026-10-08:** batch 2 on rc.2: 28 of 34 runs passed fully; **Claude Opus 5.5 missed its mechanical gate (4 of 6) and its blurred-banner gate (1 of 3), as
on rc.1, so RQ-15 is `failed` for rc.2.** No waiver, no exclusion: **improve the skill and cut rc.3** (done); the re-pins stay after `0.2.0`. The waivers of RQ-08 and RQ-10 were about rc.2's runs:
RQ-08 is re-decided on the rc.3 run (`passed`, not carried over); RQ-10 is open.

**PR 7, the findings by outcome** (each with its pull request and regression test; the list is in `project_current_status.md`). **Maintainer
rule 2026-10-04: a published failure code stays (v1 is additive); the remediation carries the fix (L-127).**
- **Fixed in rc.2:** #314 the reader's retry budget (#318; L-136 remains on Windows), #310 the size limit answers before the room check (#319).
  **Fixed in rc.3:** #322 and #332 (#330), #325 step 1 (#331), and in tests or tools #321, #310's second finding and #327 (#333).
  **Narrowed, not proven gone:** #206 (L-005); #253 (L-129); #312 the race tests wait 60 s (L-135; has failed once in 4,500 loaded Windows repetitions over three candidates; not fixed).
- **Not fixed:** #272 the FFmpeg snapshot: 46 of 47 records are fixed in the shipped build (#296, L-122); a pin must be a month-end
  build, the next is 2026-10-31 (L-132), **after `0.2.0`**, with the whisper.cpp re-pin (L-137: the floor covers the one reachable read).

**Earlier P14 PRs** (plan 15-29; `docs/development.md`; `docs/agents/trials.md`). **PR 4:** `tools/p14-campaigns/`, hosted runners only.
**PR 3:** `P14 journeys`, weekly too (L-114, L-115; P11's durable stage cannot run hosted: #258, L-113). **PR 6 and batch 1:** the harness (cold
mode, hold-outs, `freeze`, usage); batch 1 (0.1.0, 20 runs) was a baseline. **README graphics:** redraw `roadmap.svg` with PRs 10 and 13 (L-121).

## The remaining P14 pull requests (0-10 done; 11 hosted part done three times)

**11, still to do on rc.3:** batch 2 (34 runs, the new freeze) and batch 3 = RQ-16 (18 cold runs, never run; `-AllowGraderChange`), each on the maintainer's go; the clean-machine try-out = RQ-17
(`rq-17-tryout-sheet.md`); the RQ-10 decision; then the register pass (plan 28.4, 29.7). **12** stable `0.2.0`; **13** ledger follow-up, P14 `complete`, handoff. **From a
candidate's tag to the stable merge only the work record and `install.md` change** (`release.md` 6.8); the stable is compared with the highest
candidate tag, `v0.2.0-rc.3`. **After the stable tag, within seven days:** register the two stable checks in `STABLE_CHECKS`.

## What the maintainer owes, and when

- **Now:** decide **RQ-10** (plan 29.5: waive it for the link's `STORAGE_IO` alone, or add RQ-13's "accepted by the maintainer with a register entry" clause to the rule and record it `passed`); the go for
  **batch 2** (separately batch 3: Claude Code 2.1.284 and Docker; state the reserve rule first), and the Smart App Control try-out on the second Windows 11 machine. **Close #321** (the fix has 3,000
  clean hosted repetitions). Optional: deprecate rc.1 and rc.2 (`release.md` 6.12 step 7), the upgrade from rc.1 (not run). #312: fix or accept. **Dependabot:** #192, #193, #194 wait until after the
  stable. **After the stable:** the whisper.cpp re-pin (#322, L-137) with FFmpeg's (#272, L-132).
- **Register:** one pass over `register-review-sheet.md` (thirty entries, eight later, nine readings: every review is `pending` but L-137 and L-138, accepted, and L-139, rejected = to be fixed, by the
  decisions of 2026-10-07); **L-138 now describes no live limit and L-134 is narrowed to what the campaign does not try**: delete or keep them in that pass; and whether `Guide` becomes a required check.

## Open decisions and readings (maintainer)

- **RQ-10** (above); **an issue for L-139's citation half** (#224 covers the blurred banner only; governance rule 14; the supervisor opens it). **If Claude Opus misses again on rc.3:** the claims check
  accepts only `passed` behind CL-201, CL-202, CL-204 and CL-205, which all need RQ-15; the skill change is a hypothesis, untested until batch 2 runs.
- **R1 options:** a stub per ended request (L-063); the Windows kill window (L-129, an ADR); a bounded wait for the initialization
  lock (L-131, L-135); retry the Windows sharing violations (L-136); #325's later steps (L-140); #334 (L-141). **At v2:** the codes L-127
  keeps. **#204** Codex on Windows (L-076). **Readings** (none blocks): L-062, L-067, L-069, L-017, L-088, L-090, `tokio`, `durable_worker`, MSRV.

## Tracked issues and gates

- **Open:** #17 (P14), #219, #224 (L-095, L-139), #188 (SEC-T01, R1), #232, #246 (deferred), #258, #263, #272 (residual accepted, L-122),
  #312 (L-135), #314 (fixed in rc.2; L-136), #316 (hosted runners scarce: queueing only on rc.3), **#321** (fixed; clean hosted run; close it, L-138),
  **#322** (the floor is in rc.3; the pin stays, L-137), #325 (step 1 done, L-140), #334 (L-141); #128 (watch); flaky tests #253 (Windows kill
  test) and #268 (macOS SIGTERM test): comment with the run link, rerun the job; #170-#178 (register); #159, #150, #147.
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
