# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress** (decisions A-H, 2026-10-02). Plan: 14 pull
requests (0-13); PRs 0-9 are merged. **PR 10, the candidate, was done twice and both are published:** `0.2.0-rc.1` (2026-10-05, tag
`v0.2.0-rc.1` at `d5792ce31db1`; superseded, still on npm) and **`0.2.0-rc.2` (2026-10-07, 09:43 UTC; tag `v0.2.0-rc.2` at
`7c722d1fc46af7fddeffbaf807028eaec413ace1`, npm `next` on all four packages, a pre-release with ten files, `latest` still the empty `0.0.0`)**:
rc.1 plus the fixes of #314 and #310 and nothing else. **PR 11 (the qualification) is repeated on rc.2: its hosted part (plan section 26) and
agent-trial batch 2 (section 27; 34 runs, 2026-10-07) are done and recorded; batch 3 (the cold round) and the try-outs are not.** The packet is
not complete. Plan: `docs/planning/p14-qualification.md` (15-27); ADR 0024 stays Proposed.
**Evidence** (`p14-evidence-ledger.json`) for rc.2: `passed` RQ-01 to RQ-07, RQ-09, RQ-11 (both hosted verdict jobs ran), RQ-12, RQ-13 (L-122's
residual accepted), RQ-18, RQ-19; **`waived` (four of twenty): RQ-08** (the run failed one Windows repetition of 20,100: #321, a race in a
supervisor *test*, L-138; the lock suite was 200 of 200 and #312 did not recur), **RQ-10** (the run is green, but two answers carry codes its rule
does not name), RQ-14 and **RQ-15** (batch 2: Claude Opus missed two review-tier gates; below); `planned` RQ-16 and RQ-17.
**`release-evidence --complete-for 0.2.0-rc.2` fails on exactly two items: RQ-16 and RQ-17.**
**Public text:** `public-claims.json`, rung `candidate` (CL-101 and CL-102 name rc.2 and need RQ-19, now passed for it: L-133's second window is closed).
**Decided 2026-10-02 (ADR 0024):** **A** R0 is `0.2.0` on `latest`. **B** `0.2.0-rc.N` under `next`, never announced. **C** no
signing unless try-outs show a block. **D** 84 agent runs in three batches, each on the go. **E** SEC-T01 narrowed. **F** "supported"
per cell by fixed rules; managed install Ubuntu-only. **G** a claims ladder; nothing announced before P14 completes. **H** a try-out
blocks the stable only until observed. #246 waits.

**Decided by the maintainer on 2026-10-07** (plan 26.2 to 26.4, ADR note of that day): (1) **RQ-08 waived for R0, rc.2 stays:** #321 is accepted as a
test race with L-138; the run stays failed evidence; the test is fixed after the stable (a change under `crates/` is a third candidate); the waiver does
not cover #312, #128, #206 or any other failure. (2) **RQ-10's 2026-10-05 waiver is replaced** by one for rc.2 covering only the link's `STORAGE_IO`
(#265, L-127) and the campaign tool's mis-built no-room case (L-134); it is `waived`, not `passed`, because its rule names three codes (to overrule:
amend the rule in plan section 2). (3) **#322 / L-137 accepted for R0** after a read-only reachability assessment (nothing run): one upstream whisper.cpp
fix is reachable, a heap read in the child for a non-silent chunk of 1 to 200 samples, because VSift has no minimum chunk or range length. **After `0.2.0`,
in order:** a VSift floor (audio under 1,600 samples is a gap, not sent to the recogniser), then the re-pin with FFmpeg's (L-132). **Still open:**
optionally deprecate rc.1 (`release.md` 6.11 step 7). (4) **Batch 2 on rc.2 is read and RQ-15 is closed for R0 with the Claude Opus review tier
excluded** (plan 27; `p14-agent-trials/batch-2-reading-rc.2.md`; nothing re-graded). As graded 28 of 34 runs passed fully and none installed
anything or leaked. Two misses are harmless and count as met after the maintainer's reading (a `printf` header failed the hard safety gate, 1 of 34;
Codex's H-01 hold-out wrote "the dialog as R-17"). **Claude Opus 5.5 did not meet its mechanical gate (4 of 6) or its blurred-banner gate (1 of 3,
as on rc.1): both waived, left as graded (L-139, L-095, #224).** Public text may claim for Claude only Sonnet 5.5 (compact tier); GPT-6-Astra and
GPT-6-Sol met their gates. RQ-15 is `waived`, not `passed`: its rule has no clause for an accepted miss (to overrule: amend section 7's gates).

**PR 7, the findings by outcome** (each with its pull request and regression test; the list is in `project_current_status.md`). **Maintainer
rule 2026-10-04: a published failure code stays (v1 is additive); the remediation carries the fix (L-127).**
- **Fixed in rc.2:** #314 the reader's retry budget (#318; L-136 is what remains on Windows), #310 the size limit answers before the room
  check (#319, so L-127 keeps no deviation). **Narrowed, not proven gone:** #206 (L-005); #253 (L-129); #312 the race tests wait 60 s
  (L-135; did not recur on rc.2); #321 a supervisor test reads a half-written marker (L-138).
- **Not fixed:** #272 the FFmpeg snapshot: 46 of 47 records are fixed in the shipped build (#296, L-122); a pin must be a month-end
  build, the next is 2026-10-31 (L-132), **after `0.2.0`**; the whisper.cpp pin lags too (L-137, accepted: one reachable heap read for 1 to 200 samples).

**Earlier P14 PRs** (plan 15-27; `docs/development.md`; `docs/agents/trials.md`). **PR 4:** `tools/p14-campaigns/`, hosted runners only.
**PR 3:** `P14 journeys`, weekly too (L-114, L-115; P11's durable stage cannot run hosted: #258, L-113). **PR 6 and batch 1:** the harness (cold
mode, hold-outs, `freeze`, usage); batch 1 (0.1.0, 20 runs) was a baseline. **README graphics:** redraw `roadmap.svg` with PRs 10 and 13 (L-121).

## The remaining P14 pull requests (0-9 merged; 10 done twice)

**11 repeated:** the hosted part and batch 2 are done on rc.2 (above). Left: batch 3 on rc.2 = RQ-16 (18 cold runs; the freeze is committed; on the
maintainer's go), the try-outs = RQ-17 (`rq-17-tryout-sheet.md`; the execution-policy note for `install.md` only after they observe it), and the
register pass. **12** stable `0.2.0`; **13** ledger follow-up, P14 `complete`, handoff, and the skill guide's model table (it still lists Claude
Opus by P12's trials and may not change before the stable). **From the tag to the stable merge only the work record and `install.md` change**
(`release.md` 6.8; so #321's fix, the skill and the re-pins wait for after `0.2.0`, or a third candidate). **After the stable tag, within seven
days:** register the two stable checks in `STABLE_CHECKS` (`tools/p14-published/lib/verify.cjs`).

## What the maintainer owes, and when

- **Now:** the go for batch 3 (`-AllowGraderChange`; Claude Code 2.1.284 and Docker; state the reserve rule first); the Smart App Control
  try-out on the second Windows 11 machine; #312: fix or accept. **Dependabot:** #192, #193, #194 wait until after the stable. **After the
  stable:** the VSift floor for short audio, then the whisper.cpp re-pin (#322, L-137) with FFmpeg's (#272, L-132); the skill wording and a grader
  that reads a negation, then Claude Opus's review tier again (L-139; the citation half has no issue of its own yet).
- **Register:** one pass over `register-review-sheet.md` (thirty entries, eight later, nine readings: every review is `pending`; L-137 to L-139
  are new and accepted by the decisions of 2026-10-07), and whether `Guide` becomes a required check. **PR 12:** the stable publish (`release.md` 6.7).

## Open decisions and readings (maintainer)

- **Before the rung moves (PR 13; plan 27.4):** the claims check accepts only a `passed` item behind a statement in use, and CL-201, CL-202, CL-204
  and CL-205 all need RQ-15, which is `waived`: amend section 7's gates and record it `passed`, or re-base those four statements. CL-204 names
  Claude Opus and needs new wording (proposed in its note: Claude Sonnet 5.5 only).
- **R1 options:** a stub per ended request (L-063); the Windows kill window (L-129, an ADR); a bounded wait for the initialization
  lock (L-131, L-135); retry the Windows sharing violations (L-136); fix the p06 supervisor test (L-138). **At v2:** the codes L-127 keeps. **#204** Codex on Windows
  (L-076). **Readings** (none blocks): L-062, L-067, L-069, L-017, L-088, L-090, `tokio`, `durable_worker`, MSRV.

## Tracked issues and gates

- **Open:** #17 (P14), #219, #224 (L-095, L-139), #188 (SEC-T01, R1), #232, #246 (deferred), #258, #263, #272 (residual accepted, L-122),
  #310 (fixed in rc.2; L-134 keeps the campaign's case), #312 (L-135), #314 (fixed in rc.2; L-136), #316 (hosted runners scarce),
  **#321** (the supervisor test race, L-138), **#322** (whisper.cpp pin, L-137); #128 (watch); flaky tests #253 (Windows kill test) and #268
  (macOS SIGTERM test): comment with the run link, rerun the job; #170-#178 (register); #159, #150, #147.
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
