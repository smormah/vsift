# Delivery cycles after 0.2.1 (the cycle map; decided 2026-10-16)

Status: **the maintainer's delivery model of 2026-10-16**, recorded here. It replaces the first version of this page
(written 2026-10-15, with iterations A to D), which the maintainer reshaped the next day: four larger cycles instead of small
iterations, with the test review folded into the first one. The stories inside each cycle are still a plan and are
re-ordered by the supervisor as findings arrive; the decisions below are not. The policy behind them (the standing
approvals, the test and release model) is in [`delivery-governance.md`](delivery-governance.md); the weekly release
window as an operating procedure is [`release.md`](../operations/release.md) 6.18.

## Where things stand

| Version | State |
| --- | --- |
| `0.2.0` | The first stable release (R0, packets P00 to P14), 2026-10-09. Superseded by `0.2.1`; not deprecated. |
| `0.2.1` | **The current stable release**, published 2026-10-15 (tag `v0.2.1` at `e9ab6e626733`; npm `latest`). It fixes #353 (a sparse chunk failed a whole transcription), #359 (speech dropped at chunk seams) and #391 (`setup check` called a slow tool missing), and adds `--view brief`, coverage on `transcript get`, the missing-path remediations and the second skill text. Verified from outside the same day. The hosted checks of its bytes passed except the journeys on hosted Windows (#404), **a known issue of `0.2.1` by the maintainer's decision of 2026-10-16**; the ledger records that item (RQ-05) as `failed`, with no waiver. |
| `0.2.1-rc.1`, `0.2.1-rc.2` | Deprecated on npm (2026-10-15): "Superseded by 0.2.1. Install vsift-cli." |
| `0.2.1-rc.3` | The accepted candidate; npm `next`; not deprecated. |
| `0.2.2` | **Cycle 1, started 2026-10-16.** Not cut. |

The `0.2.x` line is R0 maintenance. R1 (packets P15 to P20, issues #25 to #30) is planned in
[`r1-industrial-capability-expansion.md`](r1-industrial-capability-expansion.md) and not started.

## The decisions of 2026-10-16

**Delivery in cycles (D1 to D5).**

- **D1. Four cycles.** Work is delivered in four larger cycles instead of small iterations: cycle 1 is `0.2.2`, cycle 2 is
  `0.2.3`, cycle 3 is launch readiness, and then R1 (P15 to P20). Their stories are below.
- **D2. Cycle 1 also takes #366, #367, #370 and #371** (the skill-wording issues that `0.2.1` left open), after #404 and #257.
- **D3. Standing approvals inside a cycle.** The supervisor merges on green after review, files issues for findings, and closes
  issues whose own closing criteria are met, with the evidence in the comment. Every `release` environment gate and every npm
  write stays with the maintainer.
- **D4. Stories inside a cycle run in parallel where they do not overlap.**
- **D5. Records.** Both memory files are still rewritten in every pull request, as `AGENTS.md` says. Parallel pull requests
  rebase before they merge.

**The test and release model (D6)** is written out in [`delivery-governance.md`](delivery-governance.md) and summarised in
"How each kind of change is tested" below: a short pull-request gate (target 5 minutes, hard limit 10); a nightly run on `main`
whose failure stops the line and without whose pass no candidate is cut; a weekly release window (the candidate is cut at Friday or
Saturday midnight, the long hosted runs follow, the target is to publish on Monday or Tuesday); hot fixes outside the window; and the
full-cost items monthly or with a major change.

**The earlier decisions that still stand:** online sources are planned inside R1, starting from P15's decision packet
(2026-10-16, below); #404 is a known issue of `0.2.1` and the first fix of `0.2.2`; #368 (whole-recording questions) is out of scope
for the `0.2.x` line (2026-10-11).

**What the earlier map's names became.** Other pages (the known-limits register, `release-0.2.1.md`, ADR 0024 and the release runbook)
still say "iteration A", "B", "C" or "D". Read them as follows.

| The earlier name | Now |
| --- | --- |
| Iteration A (close what `0.2.1` settled) | Done on 2026-10-16 (below) |
| Iteration B (`0.2.2`, the small fixes) | Cycle 1, `0.2.2` |
| Iteration D (the test review) | Folded into cycle 1; the policy part is written (`delivery-governance.md`), its other items are cycle 1 stories |
| Iteration C (the re-pins) | Cycle 2, `0.2.3` |
| "The full round about once a week" (rule of 2026-10-15) | Superseded by D6: the full-cost items run monthly or with a major change, the long hosted runs run once a week in the window and only for what the week touched |

## Iteration A: done on 2026-10-16

The issues that `0.2.1` settled were closed with an evidence comment, or given a status comment and left open.

- **Closed with an evidence comment:** #365, #369, #372, #373, #349, #17 and #40.
- **Status comment, stay open:** #366, #367, #368, #370 and #371. `0.2.1` shipped the skill's words only for them; #366, #367, #370 and #371
  are cycle 1 stories (D2) and #368 is R1.
- **#359 stays open for the maintainer's own listening check** on the two recordings that found it (the fix is shown on stand-ins and
  synthetic layouts only; its limits are #380 to #384, in cycle 2).

The register entries of #353 (L-145), #359 (L-149) and #365 (L-150) were deleted when `0.2.1` was published (2026-10-16), because each
now has its implementation, regression tests and a release. The entries for #366, #367, #368 and #370 stay.

## Cycle 1: `0.2.2`, the fixes found during 0.2.1, with the test review folded in

One candidate, cut in a weekly window (below). Stories that do not overlap run in parallel (D4).

**First:**
- #404: on Windows the worker's shutdown can leave provider processes running, or report a cancelled recognition as
  `MISSING_CAPABILITY` (seen twice on the stable's journeys, 2026-10-15). A known issue of `0.2.1`, decided 2026-10-16
  ([L-167](known-limits.md#l-167)); the ledger's RQ-05 is `failed` for `0.2.1` and is recorded again on the `0.2.2` candidate.
- #257 (security): the `vsift.cmd` shim npm and pnpm create lets cmd.exe re-read arguments.

**Also in cycle 1 (D2):** #366, #367, #370 and #371, the skill-wording issues. The first-call silence of #367 is not addressed by
`0.2.1`'s words.

**The fixes found during `0.2.1`:**
- Advice and skill: #396 (the missing-tool advice says "locate", which led a cold agent to search folders), #395 ("search first" in the skill).
- Small defects: #394 (a saved setup plan's envelope members), #274 (the first five seconds of a clip fail as `MISSING_CAPABILITY`), #334 (a very
  short audio range), #362 and #361 (worker retranscribe coverage; checkpoints after a model swap), #325 and #266 (a slow copy; too little room),
  #264, #265 and #277 (a named pipe, a link, sessions left initializing), #232 (a root name with control characters).
- Release workflow: #403 (the read-back window), #337 (refuse a publish dispatched on a branch in seconds).

**The test review (the earlier iteration D):**
- Flaky tests that cost re-runs: #128, #253, #268, #271, #312, #314, #285, #327.
- Runners and harness: #316, #263, #258, #282, #283; L-166 (a trial home's expired sign-in needed a hand edit of a state file).
- Trial design: #397 (the strict cold setting refuses compound commands, so it measures the setting), #336 (a claim's identifier its citation does not state).
- The rule about our code or a dependency's quality, and the cadence, are written (`delivery-governance.md`, `release.md` 6.18). What is
  still to build: the pull-request gate's path rules, the nightly with its issue-on-failure, and the release workflow's nightly precondition (separate changes).

**Testing:** each fix with its regression test; the gate on each pull request; the nightly on `main`; the candidate cut from a green
nightly; the long hosted runs for what the cycle touched; an uncounted skill smoke if the skill's wording changes.

## Cycle 2: `0.2.3`, real media, the re-pins and the follow-ups measured on real speech

- **#363 first:** a small, freely licensed real-media test set with an opt-in campaign. The real defects of `0.2.0` (#353, #359, #391) were found by
  real recordings and a second machine, not by the synthetic corpus.
- **The re-pins:** the FFmpeg build planned for 2026-10-31 and whisper.cpp 1.9.5 or later: #351, #272, #322, #399.
- **The seam follow-ups, measured on real speech:** #380 to #384 (#381 is the half-second tolerance).
- **Evaluations:** #173, #174, #175, #150 and #159.

**Testing:** this is the cycle that needs the monthly round (a re-pin is a major change): the dependency-quality checks (`P07 local ASR` accuracy,
`P14 malicious media`, the dated scan reading), agent batches 2 and 3 if the skill changes, and the full durability campaign.

## Cycle 3: launch readiness

- **The README as the front page.** It starts early, in a separate session during cycle 1.
- **The documentation sweep:** #177, #286, #374, #219, #224, #246.
- **The clean-machine try-out (RQ-17).**
- **Help text for agents without the skill** (the cold-agent variant must be usable from `--help` alone).
- **A launch checklist:** names held on every chosen channel, the claims rung.

## R1 (P15 to P20)

Starts with P15's decision packet and not before the maintainer says. Planned inside it:
- P15 to P20 (#25 to #30).
- **Online sources (decided 2026-10-16, from P15's decision packet; not a `0.2.x` change):** [#406](https://github.com/smormah/vsift/issues/406)
  (investigate videos from online links with source-grounded evidence) with four slices: [#407](https://github.com/smormah/vsift/issues/407) direct HTTPS
  video links, built in VSift; [#408](https://github.com/smormah/vsift/issues/408) HLS and DASH links; [#409](https://github.com/smormah/vsift/issues/409)
  platform pages through the optional yt-dlp component; [#410](https://github.com/smormah/vsift/issues/410) live platform streams through yt-dlp, on #107's live design.
- A live screen share (#107), speakers (#108), whole-recording questions (#368), a faster-whisper adapter (#147).
- #176 (library API stability, MSRV, pre-releases and an MCP adapter), #188 (SEC-T01 adversarial containment), #204 and #206 (Codex's Windows sandbox identity and root
  permissions on Windows).
- Evaluations: #170, #171, #172 and #178.

## How each kind of change is tested

The full round (all hosted evidence plus agent batches 2 and 3) costs half a day or more, millions of client tokens and thousands of runner minutes. The model
of 2026-10-16 spreads it: what a change can affect runs at once, the rest runs where it is cheapest to find. The policy is in
[`delivery-governance.md`](delivery-governance.md); the window is [`release.md`](../operations/release.md) 6.18. **Status: decided; the gate's path rules,
the nightly and the nightly precondition are separate changes, and until they land the existing CI runs its full set on every pull request.**

| Change or stage | What runs | Cost |
| --- | --- | --- |
| Every pull request | The gate: only what the change can affect. Docs-only: the document and governance checks. Code: lint plus the unit and contract tests of the touched crates and their dependents on Ubuntu; Windows and macOS only for a platform-sensitive change. Target 5 minutes, hard limit 10. A hot fix can request the full three-system run. | minutes of runner time |
| `main`, when something merged | The nightly. A failure opens an issue and stops development until it is fixed; a candidate is cut only from a commit whose nightly passed. | automatic |
| The weekly window | The candidate is cut at Friday or Saturday midnight; the long hosted runs (2 to 3 hours) follow, limited to what the week's changes touched; publish on Monday or Tuesday, on the maintainer's approval. A failure gets a targeted fix and a rerun of only the failing workflow. | hours of runner time, no client tokens |
| A hot fix | Outside the window: the full three-system check (about 30 minutes), the publish, then the quick checks of the published bytes (`P14 verify release`, `P14 journeys`, `P13 managed smoke`, `P14 published artifacts` from the previous release and from the candidate). | about 30 minutes before, about 90 minutes after |
| A skill wording change in an ordinary week | One uncounted skill smoke (about nine Claude runs on a local build) instead of the counted batches. | about $6 and 15 minutes |
| Monthly, or with a major change (a skill rewrite, a re-pin) | The full-cost items: agent batches 2 and 3; the dependency-quality checks (`P07 local ASR` accuracy, `P14 malicious media`, the dated scan reading); the full durability campaign. Not weekly. | half a day or more, about $10 of Claude and the Codex allowance |
| The weekly drift runs | As today, against the published binary; a failure opens an issue by itself. | automatic |

**Our code or a dependency's quality.** `P07 local ASR`'s word-error figure measures mostly whisper.cpp; `P14 malicious media` measures FFmpeg's robustness as
much as our isolation; the scan reading reads the pinned tools' records. These run when a pin, a model or the way VSift calls a tool changes, and in the monthly
round, not for every candidate.

**Two limits of the model, written down because they bite in the window.** The evidence ledger's completeness check is not relaxed: its item scopes are coarse
(`crates` is in the scope of most items, RQ-01 to RQ-12 and RQ-14 to RQ-17), so a week that touches `crates` still needs those items passed, carried or waived. And the candidate rule (`release.md` 6.8)
still freezes code, dependencies and workflows from the candidate's tag to the stable commit, so merges of that kind wait for the publish.

## Decisions waiting for the maintainer

- #188 (SEC-T01 adversarial containment; high priority; waived as RQ-14 for `0.2.0` and `0.2.1`), #176 (library API stability, MSRV, pre-releases and an MCP adapter),
  #246 (staged npm publishing) and #374 (sessions made in a packaged desktop client; the maintainer's five-minute check).
- Whether to move `next` ([L-108](known-limits.md#l-108)) and the first-day note for Yarn users.
- Ratifying or reversing the choices of #377, of the Reads change and of #392.
- The folder answer that differs by platform (`STORAGE_IO` on Windows, `INVALID_SOURCE` on Linux and macOS).
- RQ-05 stays `failed` for `0.2.1` ([`release-0.2.1.md`](release-0.2.1.md)); the maintainer may still waive it, knowing a waiver does not expire by itself.
- The time zone of "midnight" when the cut is scheduled, and whether a release branch should replace the freeze of 6.8 so that development continues in the window.
