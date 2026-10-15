# Backlog iterations after 0.2.1 (written 2026-10-15)

Status: **a plan the maintainer asked for on 2026-10-15, not a decision.** It maps every open issue to an iteration and says
how each iteration is tested, so that a fix does not wait on a half-day evidence round. The maintainer decides the order;
a later change records each decision here.

## Where things stand

| Version | State |
| --- | --- |
| `0.2.0` | The first stable release (R0, packets P00 to P14), 2026-10-09. Superseded by `0.2.1`; not deprecated. |
| `0.2.1` | **The current stable release**, published 2026-10-15 (tag `v0.2.1` at `e9ab6e626733`; npm `latest`). It fixes #353 (a sparse chunk failed a whole transcription), #359 (speech dropped at chunk seams) and #391 (`setup check` called a slow tool missing), and adds `--view brief`, coverage on `transcript get`, the missing-path remediations and the second skill text. Verified from outside the same day. |
| `0.2.1-rc.1`, `0.2.1-rc.2` | Deprecated on npm (2026-10-15). |
| `0.2.1-rc.3` | The accepted candidate; npm `next`; not deprecated. |

The `0.2.x` line is R0 maintenance. R1 (packets P15 to P20, issues #25 to #30) is planned in
[`r1-industrial-capability-expansion.md`](r1-industrial-capability-expansion.md) and not started.

## How each kind of change is tested

The full round (all hosted evidence plus agent batches 2 and 3) costs half a day or more, millions of client tokens and
thousands of runner minutes. The maintainer decided on 2026-10-15 that it runs **about once a week, at a big shift, or when
many changes have accumulated, never per fix**. A critical fix ships on its own tests.

| Change | What runs | Cost |
| --- | --- | --- |
| Every pull request | CI (the required checks) | about 40 minutes of runner time, automatic |
| A fix or a hot fix | the regression tests in its pull request; after a publish, the five quick checks of the published bytes (`P14 verify release`, `P14 journeys`, `P13 managed smoke`, `P14 published artifacts` from the previous release and from the candidate) | about 90 minutes of runner time, no client tokens |
| A skill change | one uncounted skill smoke (about nine Claude runs on a local build) instead of the counted batches | about $6 and 15 minutes |
| The end of an iteration, at most weekly | the full round once: the rest of the hosted evidence, `Fuzz`, the dated scan reading, batches 2 and 3 | half a day or more, about $10 of Claude and the Codex allowance |
| A dependency re-pin | the checks of the dependency's quality: `P07 local ASR` accuracy, `P14 malicious media`, the dated scan reading | hours of runner time |

**Our code or a dependency's quality.** `P07 local ASR`'s word-error figure measures mostly whisper.cpp; `P14 malicious media`
measures FFmpeg's robustness as much as our isolation; the scan reading reads the pinned tools' records. These run when a
pin, a model or the way VSift calls a tool changes, not for every candidate. The test review (iteration D) writes this
rule into `delivery-governance.md` and the release runbook.

## Iteration A: close what 0.2.1 settled (an hour, no release)

Each issue is closed with its evidence or a note saying why it stays open.

- Fixed in code in `0.2.1`: #359 (#377; its limits are #380 to #384), #365 (#379; the skill uses it), #369 (#378; close after a passing Linux and macOS run, which CI gave), #372 (#378), #373 (#379).
- Skill wording in `0.2.1`: #366, #367 (the brief view and the five-minute rule; the first-call silence is not addressed: keep open or move to B), #370, #371, #349.
- Out of scope by the maintainer's decision of 2026-10-11: #368 (whole-recording questions): keep open for R1 or close as decided.
- #17 (P14, recorded complete 2026-10-10) and #40 (the R0 test spine).

## Iteration B: `0.2.2`, small fixes found during 0.2.1 (one candidate, about a week)

First, because they affect people on Windows:
- #404: on Windows the worker's shutdown can leave provider processes running, or report a cancelled recognition as `MISSING_CAPABILITY` (seen twice on the stable's journeys, 2026-10-15).
- #257 (security): the `vsift.cmd` shim npm and pnpm create lets cmd.exe re-read arguments.

Then:
- Advice and skill: #396 (the missing-tool advice says "locate", which led a cold agent to search folders), #395 ("search first" in the skill).
- Small defects: #394 (a saved setup plan's envelope members), #274 (the first five seconds of a clip fail as `MISSING_CAPABILITY`), #334 (a very short audio range), #362 and #361 (worker retranscribe coverage; checkpoints after a model swap), #325 and #266 (a slow copy; too little room), #264, #265 and #277 (a named pipe, a link, sessions left initializing), #232 (a root name with control characters).
- Release workflow: #403 (the read-back window), #337 (refuse a publish dispatched on a branch in seconds).

Testing: each fix with its regression test; one candidate; the quick checks; a skill smoke if the skill changes; one full round at the end of the iteration, if the maintainer wants the stable to carry new evidence.

## Iteration C: dependency re-pins (security)

- The FFmpeg build planned for 2026-10-31 and whisper.cpp 1.9.5 or later: #351, #272, #322, #399.
- The seam follow-ups that need real speech to measure: #381 (the half-second tolerance), #380, #382, #383, #384.
- #363 first: a small, freely licensed real-media test set with an opt-in campaign. The real defects of 0.2.0 (#353, #359, #391) were found by real recordings and a second machine, not by the synthetic corpus.
- Testing: this is the iteration that needs the dependency-quality checks and one full round.

## Iteration D: the test review (not a release)

- Flaky tests that cost re-runs: #128, #253, #268, #271, #312, #314, #285, #327.
- Runners and harness: #316, #263, #258, #282, #283; L-166 (a trial home's expired sign-in needed a hand edit of a state file).
- Trial design: #397 (the strict cold setting refuses compound commands, so it measures the setting), #336 (a claim's identifier its citation does not state), and the rule above about our code and a dependency's quality.
- Output: a short plan for the maintainer, then the changes to `delivery-governance.md`, the runbooks and the harness.

## Decisions waiting for the maintainer

- #188: SEC-T01 adversarial containment (high priority; waived as RQ-14 for `0.2.0` and `0.2.1`).
- #176: library API stability, MSRV, pre-releases and an MCP adapter.
- #246: staged npm publishing.
- #374: sessions made in a packaged desktop client (the maintainer's five-minute check).
- #204 and #206: Codex's Windows sandbox identity and root permissions on Windows.
- Documentation sweeps: #177, #286.

## Evaluations and the roadmap

- Evaluations: #170 to #175, #159, #150, #178.
- R1: P15 to P20 (#25 to #30). Later: a live screen share (#107), speakers (#108), a faster-whisper adapter (#147).

## Suggested order

1. A (close what 0.2.1 settled).
2. D's plan (the test review), so B and C are tested the new way.
3. B (`0.2.2`), #404 and #257 first.
4. C (re-pins), with #363's real-media set.
5. Decide R1.
