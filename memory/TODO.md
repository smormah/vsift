# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress** (decisions A-H, 2026-10-02). Plan: 14 pull
requests (0-13); PRs 0-10 are done. **PR 10 is complete a third time: `0.2.0-rc.3` was published and verified on 2026-10-08** (tag `v0.2.0-rc.3` at
`83dca856e7a00fc9a71c87baae99f0b1d401dd31`, publish run 37746979716; npm `next` is `0.2.0-rc.3` on all four packages and `latest` is the empty `0.0.0`). The first two
candidates are published and superseded, and stay on npm: `0.2.0-rc.1` (2026-10-05, `d5792ce31db1`) and `0.2.0-rc.2` (2026-10-07, `7c722d1fc46a`); none is deprecated (they are deprecated
at the stable, decided 2026-10-09). The packet is not complete. Plan: `p14-qualification.md` (15-29); ADR 0024 stays Proposed.
**What rc.3 is:** rc.2 plus exactly (1) **two evidence rules in the agent skill** (an unreadable region proves nothing about its content; a claim states only what its own citations show), the answer to rc.2's
two missed gates (L-139, L-095); (2) #330: audio under 100 ms is a gap, never recognised (#322, L-137), and `audio` of 31 microseconds or less is refused as `INVALID_ARGUMENT` (#332); (3) #331: a copy that
outruns the ten-minute limit says so (#325 step 1, L-140); (4) #333: test and tool fixes only (#321, L-138; L-134; #327). No re-pin, no Dependabot. The agent-trial freeze is new on purpose (whole-freeze digest `654955dd...`, skill digest `34ff775f...`).
**PR 11 is repeated a third time on rc.3 and its evidence is complete (2026-10-09): `release-evidence --complete-for 0.2.0-rc.3 --commit 83dca856...` passes, exit 0, nothing named.** What is left of
it is the maintainer's and is not evidence: the register pass and their own reading of the cold logs. Main moved on by four work-record pull requests after the tag: #338 (the hosted part), #339
(batch 2), #341 (batch 3) and the one that records the maintainer's decisions of 2026-10-09 (plan section 29.10; this change).
- **Hosted part, recorded (plan section 29):** every campaign green (verify 20 checks, clean installs, archives, offline install, upgrades from 0.1.0 and rc.2, journeys on 3 systems, managed
  smoke, fuzz 31 targets / 3.49 billion runs, stress 25 jobs / 20,100 repetitions with none failed, load, runbook walk, both fault campaigns, scan reading).
- **Batch 2 on rc.3, recorded (2026-10-08, section 29.8):** with the skill, 34 runs, **every gate met, 34 of 34 passed fully** (28 of 34 on rc.2); Claude Opus 5.5 passed the blurred banner 3 of 3 and
  the journey 6 of 6; RQ-15 `passed`. A small sample, not a proof (3 blurred runs per client, 1 run per hold-out, a text-matching grader, one skill wording). About $9.07 for Claude Code.
- **Batch 3 on rc.3, recorded (run 2026-10-08, decided 2026-10-09, section 29.9):** 18 cold runs (no skill, no documents; Claude Code 9 under the strict setting, Codex 9 under the realistic one).
  **Cold usefulness is met on both clients with no margin: 5 of 6 compact runs each (83%, target 80%; the baseline on 0.1.0 was 1 of 6 and 2 of 6). Cold safety, a hard gate, is NOT met: 1 of 18 runs**
  (Codex GPT-6-Sol ran `base64` on the audio clip `vsift audio` had named, inside the container; nothing installed, written or sent; the report was correct; #340, L-142). No cold run installed
  anything or accepted a plan. A waiver is not a pass. About $1.31 for Claude Code, 1.63 M input tokens for Codex, 42 minutes.
- **The maintainer's decisions of 2026-10-09 (section 29.10; work record only, nothing was run again):** (1) **RQ-10's pass rule admits `INVALID_ARGUMENT`** exactly where the media judge does
  (`FOLLOW_UP_CODES` in `tools/p14-campaigns/lib/hostile-judge.cjs` for a follow-up call on an accepted source; the file-name, folder, link and pipe cases' own `codes`): the 20 inputs of the run
  that end so meet the rule; RQ-10 stays `waived` for rc.3 (the link's `STORAGE_IO`) and is not `passed`. (2) **The RQ-16 waiver carries to the stable `0.2.0`** (same bytes), for the same one action only (a `base64` read of the clip VSift named, in the container, on the synthetic corpus); no other action or candidate.
  (3) **`0.2.0-rc.1` and `0.2.0-rc.2` are deprecated at the stable**, not before: a step PR 12 must include; the maintainer runs the npm commands, the supervisor never does. (4) **RQ-17 is `waived`
  for rc.3 and the stable: they ship untried** (no Smart App Control or SmartScreen try-out, no true clean-machine install, no Mac Gatekeeper try-out; decision H; L-143).
- **Not done:** the register pass; the maintainer's own reading of the raw cold logs (they are reading a generated command list of the 18 runs; to be recorded when they confirm, L-118).
**Evidence** (`p14-evidence-ledger.json`): `passed` for rc.3 RQ-01 to RQ-09, RQ-11, RQ-12, RQ-13, RQ-15, RQ-18, RQ-19; `waived` RQ-14 (a mechanism), **RQ-10** (rc.3 only, the link's `STORAGE_IO` #265;
plan 29.5), **RQ-16** (rc.3 and the stable, one action) and **RQ-17** (rc.3 and the stable, untried); RQ-20 is the check. A waived item is complete for any version, so the checker cannot see these
limits: they live in the decisions' texts, and a new candidate decides again. **Public text:** `public-claims.json`, rung `candidate`; the README and `install.md` name rc.3 as the candidate under
qualification. CL-202, CL-204, CL-205 have every evidence item they name `passed` but sit at `after_p14`, unused, and lean on register entries still `pending`; **CL-201 requires RQ-17 `passed` and
CL-206 requires RQ-16 `passed`: both stay unused.** **CL-204's note and the support matrix's agent-client paragraph still say "the repeat decides" (stale since batch 2); not touched.**
**Decided 2026-10-02 (ADR 0024):** **A** R0 is `0.2.0` on `latest`. **B** `0.2.0-rc.N` under `next`, never announced. **C** no signing unless try-outs show a block. **D** 84 agent runs in three batches,
each on the go. **E** SEC-T01 narrowed. **F** "supported" per cell by fixed rules; managed install Ubuntu-only. **G** a claims ladder; nothing announced before P14 completes. **H** a try-out blocks the
stable only until observed (or waived: used for RQ-17). #246 waits. **Earlier P14 PRs** are summarised in `project_current_status.md`.
**Decided 2026-10-07 and 2026-10-08:** batch 2 on rc.2 failed RQ-15: no waiver, no exclusion, **improve the skill and cut rc.3** (done; batch 2 on it met every gate). The re-pins stay after `0.2.0`.
RQ-08 `passed` on rc.3's run; RQ-10 waived for rc.3 (the link case only; the rc.1 upgrade stays skipped).

**PR 7, the findings by outcome** (each with its pull request and regression test; the list is in `project_current_status.md`). **Maintainer
rule 2026-10-04: a published failure code stays (v1 is additive); the remediation carries the fix (L-127).**
- **Fixed in rc.2:** #314 the reader's retry budget (#318; L-136 remains on Windows), #310 the size limit answers before the room check (#319).
  **Fixed in rc.3:** #322 and #332 (#330), #325 step 1 (#331), and in tests or tools #321, #310's second finding and #327 (#333).
  **Narrowed, not proven gone:** #206 (L-005); #253 (L-129); #312 the race tests wait 60 s (L-135; has failed once in 4,500 loaded Windows repetitions over three candidates; not fixed).
- **Not fixed:** #272 the FFmpeg snapshot: 46 of 47 records are fixed in the shipped build (#296, L-122); a pin must be a month-end
  build, the next is 2026-10-31 (L-132), **after `0.2.0`**, with the whisper.cpp re-pin (L-137: the floor covers the one reachable read).

## The remaining P14 pull requests (0-10 done; 11's evidence is complete; 12 and 13 to do)

**11, still to do (the maintainer's):** the register pass (plan 28.4, 29.7, 29.9). **12** stable `0.2.0` **including the deprecation of `0.2.0-rc.1` and `0.2.0-rc.2` after the publish is verified**
(the maintainer runs `npm deprecate` on the four packages; `release.md` 6.12 step 7 is superseded; PR 12 adds the step to the stable's runbook); **13** ledger follow-up, P14 `complete`, handoff.
**From a candidate's tag to the stable merge only the work record and `install.md` change** (`release.md` 6.8); the stable is compared with the highest
candidate tag, `v0.2.0-rc.3`. **After the stable tag, within seven days:** register the two stable checks in `STABLE_CHECKS`.

## What the maintainer owes, and when

- **Now:** the register pass over `register-review-sheet.md` (thirty entries, eight later, nine readings: every review is `pending` but L-137 and L-138, accepted, and L-139, rejected = to be fixed;
  **L-142 and L-143 are new, pending**). **L-139 and L-095 record the rc.3 result and stay open; L-138 describes no live limit and L-134 is narrowed**; delete or keep them; and whether `Guide` becomes a
  required check. Confirm the reading of the 18 cold logs when done (L-118). **#340:** which option (help text only, an additive JSON hint, or accept and fix in `0.2.x`); any change to the CLI's text
  needs a new candidate before the stable. **Close #321** (3,000 clean hosted repetitions). #312: fix or accept. **Dependabot:** #192, #193, #194 wait until after the stable.
- **At the stable (PR 12):** the publish session, then the deprecation of rc.1 and rc.2. **After the stable:** the whisper.cpp re-pin (#322, L-137) with FFmpeg's (#272, L-132); the RQ-17 try-out may
  still be done (LOKI, `rq-17-tryout-sheet.md`) and recorded, which gives CL-201 its evidence. **README graphics:** redraw `roadmap.svg` with PRs 10 and 13 (L-121).

## Open decisions and readings (maintainer)

- **The claims wording** (CL-204's note, the matrix paragraph) waits for the register pass. L-139's citation half is #336 (#224 covers the blurred banner).
- **From the cold round (reading `batch-3-reading-rc.3.md`, none filed):** the help says `--limit` takes 1 to 100 where the budget is 50 (one miss in 9 Codex runs); `BUSY` with no hint for two `frame get` calls
  at the same moment on one session (not diagnosed; related to L-131); the no-transcript answer is `INVALID_ARGUMENT` with a true remediation (documented; L-127's family); Claude Code's strict setting
  accepted and refused other forms than its file was read to say (L-125).
- **R1 options:** a stub per ended request (L-063); the Windows kill window (L-129, an ADR); a bounded wait for the initialization
  lock (L-131, L-135); retry the Windows sharing violations (L-136); #325's later steps (L-140); #334 (L-141). **At v2:** the codes L-127
  keeps. **#204** Codex on Windows (L-076). **Readings** (none blocks): L-062, L-067, L-069, L-017, L-088, L-090, `tokio`, `durable_worker`, MSRV.

## Tracked issues and gates

- **Open:** #17 (P14), #219, #224 (L-095, L-139), #188 (SEC-T01, R1), #232, #246 (deferred), #258, #263, #272 (residual accepted, L-122),
  #312 (L-135), #314 (fixed in rc.2; L-136), #316 (hosted runners scarce), **#321** (fixed; the maintainer closes it, L-138), **#336** (L-139's citation half),
  **#322** (the floor is in rc.3; the pin stays, L-137), #325 (step 1 done, L-140), #334 (L-141), **#340** (the audio clip a cold agent cannot use, L-142); #128 (watch); flaky tests #253 (Windows kill
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
