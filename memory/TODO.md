# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now (2026-10-10)

**P00-P14 are complete: the whole of R0's work packets, recorded 2026-10-10 by P14 PR 13b** (`docs/planning/p14-qualification.md` section 31; ADR 0024 is Accepted with it). Nothing is announced and no
platform is called "supported": the claims rung is still `candidate`. **Complete is a statement about the ledger and the checker, not about readiness** (four evidence items are waived, none is a pass; RQ-13 is `passed` with one accepted finding: CVE-2026-107678, an FFmpeg record reachable through the MP4 demuxer with no fix in the shipped build, accepted by the maintainer on 2026-10-10 and to be fixed after the release: L-144, #351).
- **The release `0.2.0` is published and checked.** Published 2026-10-09 (tag `v0.2.0` at `eeb2a22a46a8`, the merge of #344; Release run 37946261087): `latest` is `0.2.0` and `next` is `0.2.0-rc.3` on all four
  packages (until the candidate below is published). `0.2.0-rc.1` and `0.2.0-rc.2` are deprecated (2026-10-10); `0.2.0-rc.3`, `0.2.0` and `0.1.0` are not. `release.md` 6.13 records what the first move of `latest` did
  (trusted publishing accepted `--tag latest`, the release was marked latest, npm took about two and a half minutes: L-105). It is built from the third candidate's source (same program, other version number).
- **What was run on `0.2.0`'s own bytes (2026-10-09, all green):** `P14 published artifacts` from `0.1.0` and from `0.2.0-rc.3`, `P14 journeys` on three systems, `P13 managed smoke`, the CI and Guide runs at the
  stable commit; `P14 verify release` is RQ-19's evidence (its first run was red on the two stable checks not yet registered, by design; P14 PR 13a, #348, registered them). The ledger carries the third candidate's evidence
  for RQ-07 to RQ-12 and RQ-14 to RQ-17 by the recorded `release_delta`. **Four items are `waived`, none is a pass:** RQ-10 (a symbolic link answers `STORAGE_IO`), RQ-14 (the strict worker is not claimed to contain a
  hostile decoder), RQ-16 (one cold agent read an audio clip file) and RQ-17 (no Smart App Control, SmartScreen, clean-machine or Mac try-out: the release ships untried).
- **The neutral checkpoint for using the published CLI (plan section 12) was met when P14 was recorded complete, and the trial has started.** The maintainer installed `0.2.0` on their own machine on 2026-10-10,
  copied the skill, byte for byte, into their own agent client's folder and is testing real recordings; nothing is installed or configured on their machine by a session. The trial is never qualification evidence.
  **Its first real recording found #353 (L-145, high; below).** Section 12's condition is "no open high-severity limit blocks the investigate-a-video journey", and L-145 is one until `0.2.1` is published; the plan's
  own text (31.4 names L-068 as the only high entry) is not edited here.
- **The first real recording found a high-severity defect the day after the release (2026-10-10): #353, L-145. It is fixed on `main` and not released; `0.2.0` is still affected.** A recording whose 30-second chunk
  holds few recognised segments, one of them rejected, makes `transcript retranscribe` fail the whole run as `MISSING_CAPABILITY` and commit nothing; `job resume` fails the same way. Every accuracy figure so far came from a
  synthetic corpus (L-022). **P14 stays complete and `0.2.0` stays published and is not deprecated.** The workarounds stand (a supplied transcript, or ranges that avoid the failing window); the README and the install guide say so.
- **The LOKI try-out (RQ-17) is still available** (`docs/planning/rq-17-tryout-sheet.md`): it would give CL-201 its evidence, and until it is done the Windows row stays untried. `roadmap.svg` and the README's badge wait for the README design session (L-121).

## Next: the patch release 0.2.1 (the maintainer's decision of 2026-10-10): the first candidate is CUT, NOT TAGGED OR PUBLISHED

This is a patch release, not a work packet; the plan is `docs/planning/release-0.2.1.md`. **Increments done, release not done: nothing here is published, and the register entries stay open until the release.**
- **On `main` (merged 2026-10-10, each its own pull request with regression tests; issues closed):** #353 the transcription fix (#358: a chunk whose answer cannot be used is a recorded gap and the run is `partial`; the quarter rule applies from four
  segments; the failure says what failed; the job is not resumable then), #340 and #342 (#357: `audio`'s text, a `BUSY` retry hint), #345 (#356: a flaky test). **Not shown:** #353 on the real recording (commercial; stand-ins only); #340's JSON; #345's race in the harness.
- **In the cut of `0.2.1-rc.1` (one pull request, "P14-style cut: the release candidate 0.2.1-rc.1 ..."; not merged by the session that wrote it):** the version in the five files (each is `0.2.0`'s with the version text replaced); the skill change (#349
  wording, partial-transcript gaps, a `BUSY` retried once, the audio clip's text) and the fuzz seed copy that follows it; **a new agent-trial freeze** (skill `2f8686b7...`, whole `dbc4c22c...`, the other six digests the third candidate's; its records moved to
  `batch-2-rc.3/` and `batch-3-rc.3/`); the guide names `0.2.1`; `release.md` **6.14** (the supervisor tags and dispatches, the maintainer approves the `release` environment, `latest` stays `0.2.0`); tests for a candidate published while a
  stable version is `latest`; public text changed only where `next` naming `0.2.1-rc.1` would be false or misleading. **The audit found three places that assumed `latest` is the empty placeholder and would have been false for this candidate:** `install.md`
  section 7 (it told a person on a candidate that `latest` is the higher version), `release.md` 6.4's dist-tags comment, and the 404 expected from GitHub's latest release (it is `v0.2.0` now); the tools, the plan and the workflow read the registry's own `latest`.
- **Not done:** merge (only when the supervisor can tag and publish at once), tag, dry run, publish, the approval, the checks from outside; the hosted evidence at the candidate; **agent batches 2 and 3 under the new freeze (the maintainer's go and
  allowances, after the publish)**; the decisions below; the stable `0.2.1` (its checklist is not written) and its publish; the work-record change that closes L-145 and re-reads L-142 once that is published.
- **Meanwhile** `0.2.0` stays published, is not deprecated and **is still affected by L-145**; `latest` is `0.2.0`. Nothing is announced.

## Open for the maintainer

- **Carrying the four waivers to 0.2.1** (the completeness check names 15 of 20 items for the candidate and does **not** name the waived ones, so without a decision they carry silently): RQ-10's link case (#265), RQ-14, RQ-16's one action (#340; batch 3
  has not run on the new text) and RQ-17 (untried). **The optional later skill scenario for partial transcripts** (no scenario exercises the new rule; adding one needs its own freeze and candidate). **Whether to run the optional upgrade from `0.2.0-rc.3`**, to deprecate
  `0.2.1-rc.1` and `0.2.0-rc.3` at the stable release, and to move `next` afterwards (L-108).
- **L-145's review is `pending`:** confirm its severity (high by the rubric's second line, read for the investigate-a-video journey); say whether plan section 12's and 31.4's wording is amended. **L-146 (#361), L-147 (#362) and L-148 are `pending`:** confirm each severity.
- **RQ-05 on the release's own tag lacks** the Ubuntu and Windows ASR gates and the durable path (the third candidate's runs stand in `prior`): dispatch `P07 local ASR`, `P14 load`, `P14 runbook walk` at `v0.2.0` or accept (31.5 item 2); `P14 compatibility` was not dispatched at `v0.2.0` (item 3).
- **The claims rung stays `candidate`.** Moving it makes CL-201 to CL-209 stale unless each is used or deleted: six could be used on passed evidence (CL-202, 203, 204, 205, 207, 209), three cannot (CL-201 needs RQ-17, CL-206 RQ-16, CL-208 RQ-14: waived); that is yours (31.5 item 4).
- **Close #17** and **#321**; #312 fix or accept; **#340's additive JSON hint (option 2):** still wanted? **The register:** 79 of 133 entries are `pending`; L-139 stays rejected (#336); the MSRV policy is undecided; `Guide` is not a required check.

## Later: the work list (not in 0.2.1; none of it blocks anything)

- **Findings and tests:** #337 (a branch dispatch of the release workflow is refused only after the builds), #325 (a slow copy fails after ten minutes: L-140), #334 (a few-millisecond `audio` range: L-141), #312,
  #309, the `release.md` tidy (6.10 to 6.12 are records of past publishes), and #363 (a small freely licensed real-media test set, with an opt-in campaign on it).
- **Re-pins:** FFmpeg (a month-end build; the next is 2026-10-31, L-132) with whisper.cpp (#322, L-137); the Dependabot pull requests #192, #193 and #194. **#351 (CVE-2026-107678, L-144, accepted):** look for the upstream fix when the build is chosen or cherry-pick one, re-run the ancestry tool, and add a malformed-MP4 `pssh` case to the malicious-media campaign.
- **The README design session** (`roadmap.svg`, the badges and the graphics, within the claims ladder: L-121) and any promotion, which the maintainer starts (ADR 0024 decision G).
- **From the cold round (none filed):** `--limit` help says 1 to 100 where the budget is 50; the no-transcript answer is `INVALID_ARGUMENT` with a true remediation (L-127's family). **R1 options:** a stub per ended
  request (L-063); the Windows kill window (L-129); a bounded wait for the initialization lock (L-131, L-135); retrying Windows sharing violations (L-136); #325's later steps; #334 (L-141). **At v2:** the codes
  L-127 keeps. **#204** Codex on Windows (L-076). **#188** (SEC-T01) is R1 technical debt by decision E though its title still says before the R0 release.

## Tracked issues and gates

- **Fixed on `main`, closed on GitHub, not released:** #353 (L-145, high; open in the register until `0.2.1` is published), #340 (L-142), #342, #345. **Open, in 0.2.1 (the cut carries it):** #349. **Open, other:** #361 (L-146), #362 (L-147), #363, #351 (L-144), #17, #219, #224 (L-095, L-139), #188, #232, #246 (deferred), #258, #263, #272 (residual accepted, L-122),
  #312 (L-135), #314 (fixed in rc.2; L-136), #316 (hosted runners scarce), #321, #336, #322 (the floor is in; the pin stays, L-137), #325, #334, #337; #128 (watch); flaky tests #253 (Windows kill test) and #268
  (macOS SIGTERM test): comment with the run link and rerun the job.
- **A change after a candidate's tag may touch only the work record, the installation guide and the launcher's README** until the stable version is published (`release.md` 6.8; the stable commit is compared with the highest `-rc.N` tag of its own version).
  **`P14 verify release` for a stable version must be dispatched within seven days of its publish** (the delta record lives in the publish run's artifact; `release.md` 6.4 and 6.13).
- **Opt-in real-tool paths** (`--ignored`) run a Cargo-built binary unless `VSIFT_E2E_BINARY` names an installed one (L-042); a failed run is a finding: issue first, then rerun. **P14 tools:**
  `tools/p14-published/test/`, `tools/p14-campaigns/test/`. **Campaigns:** never on the maintainer's machine (L-056, L-057) except the agent batches; see `docs/development.md`.

## What the next session reads first

`memory/project_current_status.md`, then `docs/planning/release-0.2.1.md` (the plan: scope, evidence, the four waivers, the order, the open decisions) and `docs/operations/release.md` 6.14 (the candidate's steps),
`docs/planning/known-limits.md` (L-145 to L-148 and L-142 for the 0.2.1 work; L-105, L-108, L-121, L-133 for the release and the README), `docs/planning/p14-stable-release-steps.md` (the model for the stable `0.2.1` checklist), and the delivery ledger.
R1 starts with P15's decision packet, not before the maintainer says.

## Guardrails

- **The skill** calls the published CLI only; a new public item needs a skill update in the same change (`skill_contract`); its fuzz seed copy (`fuzz/seeds/handoff_check/SKILL.md`) must equal it byte for byte. **New public items** need their `*::ALL` entry, v1 schemas and a human renderer with a snapshot;
  v1 is additive only; a parser of untrusted input needs a fuzz target; npm packages have no scripts.
- **The release seam:** a change to `vsift-release`, `release.yml` or its lint needs a lint rule, a broken-copy mutation test and the shell check; no workflow runs `npm dist-tag`. **Trials:** a change to the skill,
  grader, scenarios or settings after a batch's first counted run voids the batch (`freeze check`); a hold-out failure is a finding, never a grader edit. **Commits:** the commit path reruns the crash campaign; no
  development feature in a release. **Public text** uses "supported", "stable" and "qualified" only inside registered statements; every change to a quoted sentence updates `public-claims.json`.
