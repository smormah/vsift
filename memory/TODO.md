# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now (2026-10-11)

**P00-P14 are complete: the whole of R0's work packets, recorded 2026-10-10 by P14 PR 13b** (`docs/planning/p14-qualification.md` section 31; ADR 0024 is Accepted with it). Nothing is announced and no
platform is called "supported": the claims rung is still `candidate`. **Complete is a statement about the ledger and the checker, not about readiness** (four evidence items are waived, none is a pass; RQ-13 is `passed` with one accepted finding: CVE-2026-107678, an FFmpeg record reachable through the MP4 demuxer with no fix in the shipped build, accepted by the maintainer on 2026-10-10 and to be fixed after the release: L-144, #351).
- **`0.2.0` is published and checked, and it has TWO serious transcription defects found by real recordings.** Published 2026-10-09 (tag `v0.2.0` at `eeb2a22a46a8`, the merge of #344; Release run 37946261087): `latest` is `0.2.0`. `0.2.0-rc.1` and `0.2.0-rc.2`
  are deprecated (2026-10-10); `0.2.0-rc.3`, `0.2.0` and `0.1.0` are not. It is built from the third candidate's source (same program, other version number). **#353 / L-145 (high, loud):** a recording whose 30-second chunk has few recognised
  segments, one rejected, fails the whole `transcript retranscribe` and keeps nothing; **fixed on `main` (#358), not released.** **#359 / L-149 (high, silent):** merging the overlapping 30-second pieces can drop speech at a seam with no warning
  while `search` and the result say the whole range was transcribed (six places listed in a 17:34 recording, two of them checked, and one in a 34:36 one); the cause is inferred from the source and timings, nobody has listened to the audio; **the fix is in pull request #377, open (not on `main`, not released), and
  `0.2.0` and `0.2.1-rc.1` both have the defect.** The README and the install guide carry a known-issue note for each, with workarounds (a supplied transcript; ranges for #353; a check against the recording at the seams, every 25 seconds, for #359).
- **What was run on `0.2.0`'s own bytes (2026-10-09, all green):** `P14 published artifacts` from `0.1.0` and from `0.2.0-rc.3`, `P14 journeys` on three systems, `P13 managed smoke`, the CI and Guide runs at the
  stable commit, `P14 verify release` (RQ-19). The ledger carries the third candidate's evidence for RQ-07 to RQ-12 and RQ-14 to RQ-17 by the recorded `release_delta`. **Four items are `waived`, none is a pass:** RQ-10 (a symbolic link
  answers `STORAGE_IO`), RQ-14 (the strict worker is not claimed to contain a hostile decoder), RQ-16 (one cold agent read an audio clip file) and RQ-17 (no Smart App Control, SmartScreen, clean-machine or Mac try-out: the release ships untried).
- **The neutral checkpoint for using the published CLI (plan section 12) was met when P14 was recorded complete, and the trial has started.** The maintainer installed `0.2.0` on their own machine on 2026-10-10, copied the skill, byte for byte,
  into their own agent client's folder and is testing real recordings; nothing is installed or configured on their machine by a session; the trial is never qualification evidence. **Its first two real recordings found #353 and then, posted 2026-10-10,
  #359 and #365 to #374** (below). Section 12's condition is "no open high-severity limit blocks the investigate-a-video journey", and L-145 and L-149 are open high entries until `0.2.1` is published; the plan's text (31.4 names L-068 as the only high entry) is not edited.
  Every accuracy figure so far came from a synthetic corpus (L-022).
- **The LOKI try-out (RQ-17) is still available** (`docs/planning/rq-17-tryout-sheet.md`): it would give CL-201 its evidence. `roadmap.svg` and the README's badge wait for the README design session (L-121).

## Next: the patch release 0.2.1 (decided 2026-10-10, widened 2026-10-11): `0.2.1-rc.1` is PUBLISHED AND SUPERSEDED; `0.2.1-rc.2` is not started

Plan: `docs/planning/release-0.2.1.md`. **Increments done, release not done: nothing here is released as `0.2.1`, and the register entries stay open until it is.**
- **On `main` (merged 2026-10-10, each its own pull request with regression tests):** #353 the transcription fix (#358: a chunk whose answer cannot be used is a recorded gap and the run is `partial`; the failure says what failed), #340 and #342 (#357: `audio`'s text,
  a `BUSY` retry hint), #345 (#356: a flaky test). **Not shown:** #353 on the real recording (commercial; stand-ins only); #340's JSON; #345's race in the harness. Nothing for #359 (pull request #377 is open) or the findings below is merged.
- **Published 2026-10-10: `0.2.1-rc.1`** (tag `v0.2.1-rc.1` at `cb8cb3a`, the merge of #364; Release run 38084137607; npm `next` on all four packages, `latest` still `0.2.0`; `P14 verify release` run 38086846526 passed). It has the three fixes, the first new skill (a new
  trial freeze) and the runbook `release.md` 6.14, **and it has #359.** **Its hosted evidence round was stopped on purpose:** on the maintainer's decision of 2026-10-11 to put #359 into `0.2.1` (which changes the transcription code, so rc.1's evidence is obsolete), the supervisor session stopped the hosted-evidence agent and cancelled the hosted campaign runs it had dispatched for rc.1. Of 14 runs 6 finished green (history) and 8 were cancelled; none is recorded in the ledger as evidence; agent batches 2 and 3 were not run on it.
- **Decided 2026-10-11 by the maintainer:** #359 goes into `0.2.1`, which changes the transcription code, so **a second candidate `0.2.1-rc.2` is cut** and rc.1 is superseded. The larger findings ride along: #365 (transcript get/search item size), #366 (`handoff check`
  cannot take a long report on Windows), #367 (`candidates` slow and too dense), #368 (a whole-recording question does not fit the budgets and limits), #370 (`frame get` is `BUSY` during a transcription); and the small ones: #369, #371, #372, #373;
  #374 only if it can be verified (its effect is not verified; nothing is claimed). **The designs are NOT decided (a design pass is under way); #359's fix is pull request #377 (below).**
- **rc.2 needs:** the changes with regression tests; a new skill and a new agent-trial freeze; hosted evidence once, on rc.2 (nothing from rc.1 is reused); agent batches 2 and 3 once, on the maintainer's go and allowances; the decisions on the four waivers; the version-number question.
  **Not done:** all of that, then the stable `0.2.1` (its checklist is not written) and its publish, then the work-record change that deletes the entries that closed.
- **#359's fix, pull request #377 (open; round 1 of the independent review is answered in new commits and a second round follows):** `merge_chunks` has a guard that restores dropped segments until no stretch longer than 0.5 s of the union of all
  chunks' segments is in no kept segment, and a revision lists the stretches where words that no segment holds may lie (`local_asr.unplaced_ranges`, an upper bound; `search` counts them as untranscribed). **What is true, not just hoped:** it judges by time, so it trades holes for repeats
  (a reviewer's synthetic simulation: lost words 5,931 to 97, duplicated words 516 to 1,923); up to half a second can still be missing; rule 3 looks at one pair per seam and can still drop a copy that carries new time; rejected segments with no word, an empty range or nothing after them are only counted;
  a session holding the new record member cannot be read by `0.2.0` or `0.2.1-rc.1` (every revision, `handoff check --session` and `bundle validate` too). Registered as L-155 to L-160. **Not proven:** the cause is inferred, no real recording was used, the opt-in real-tool stage `p07_local_asr_multi_chunk_seam` was not run, the half second is derived for one configuration and not measured, and nobody has listened to the audio. The README and install-guide notes stay until the release.
- **"Answers" (PR 2 of the design in #376; not merged by the session that wrote it) is done in code, tests and documents: #369** (`ingest` of a path that names no file, a folder on Windows
  or a file the account may not read keeps `STORAGE_IO` and exit 7 and its remediation says which; the path after `--transcript` too) **and a test for #372** (an id of a superseded revision still resolves in `transcript get --revision`,
  `handoff check --session` and a retained bundle; no behaviour changed). **Not done there:** the skill's two sentences for #372 and the skill text for #369 (the cut), the register entries below, and every other pull request of the design.
- **Meanwhile** `0.2.0` stays published, is not deprecated and has both defects; `latest` is `0.2.0`, `next` is `0.2.1-rc.1`. Nothing is announced.

## Open for the maintainer

- **The version number (OPEN):** the larger changes add new options or members to the v1 contract (additive); is it `0.3.0` and not `0.2.1`? Every "planned for 0.2.1" (README, install guide, register, plan) follows the answer.
- **Carrying the four waivers to the next version** (the completeness check does not name waived items, so without a decision they carry silently): RQ-10's link case (#265), RQ-14, RQ-16's one action (#340; batch 3 has not run on the new text) and RQ-17 (untried).
  **Whether whole-recording questions are in scope** for this line (#368's first question). **Whether to deprecate `0.2.1-rc.1`** now or at the stable. **An optional later skill scenario** (partial transcripts, seam loss, whole-recording questions): its own freeze and candidate.
  Whether to run the optional upgrade from `0.2.0-rc.3`, and to move `next` afterwards (L-108).
- **Reviews `pending`:** L-145 (high by the rubric's second line, read for the investigate-a-video journey; plan section 12's and 31.4's wording) and **L-149 (high by the first line, read as "incomplete evidence presented as complete": the entry states the reading and
  that the known-issue note does not lower it; if the first line is read as not applying it is medium)**; L-146 (#361), L-147 (#362), L-148, L-150 (#365), L-151 (#366), L-152 (#367), L-153 (#368), L-154 (#370): confirm each severity; **L-155 to L-160 (added with #377: repeats, the half second, rule 3's containment, the kinds only counted, the upper bound and the retry, the rollback) are new and `pending`.** 91 of 145 entries are `pending`; L-139 stays rejected (#336).
- **Pull request #377's choices to confirm (#359):** a guard on the union and not on each segment, half a second as the tolerance, an empty range kept count-only and a reversed one taken from its start, the extension of rule 3 (L-155, L-157) left for later, and the new record member with its rollback cost (L-160). **Who does the listener's check of #359's holes** on the two recordings (they stay on the maintainer's machine), and **who verifies #374** (does an ordinary terminal's `session list` see sessions made in a packaged desktop client).
- **RQ-05 on the release's own tag lacks** the Ubuntu and Windows ASR gates and the durable path (the third candidate's runs stand in `prior`): dispatch `P07 local ASR`, `P14 load`, `P14 runbook walk` at `v0.2.0` or accept (31.5 item 2); `P14 compatibility` was not dispatched at `v0.2.0` (item 3).
- **The claims rung stays `candidate`.** Moving it makes CL-201 to CL-209 stale unless each is used or deleted: six could be used on passed evidence (CL-202, 203, 204, 205, 207, 209), three cannot (CL-201 needs RQ-17, CL-206 RQ-16, CL-208 RQ-14: waived); that is yours (31.5 item 4).
- **Close #17**; #312 fix or accept; **#340's additive JSON hint (option 2):** still wanted? The MSRV policy is undecided; `Guide` is not a required check.
- **From the Answers pull request:** (1) **a folder named as the source is `STORAGE_IO` on Windows and `INVALID_SOURCE` on Linux and macOS**; the contract text had said `INVALID_SOURCE` for both, and it now says what each does. Aligning Windows
  would change a published answer on that platform, so it was not done: align or keep? (2) **Register entries to write** (`known-limits.md`, not touched by that pull request; the next free number is L-155 after #375): the platform difference above;
  a locked file (a Windows sharing violation) and any other failure to open a path the system does not name plainly still answer `STORAGE_IO` with no remediation; a failed `ingest` still creates the session root (the engine provisions it before it opens
  the source; harmless, left as the design says); the refused-file test and the folder test for Linux and macOS were compiled and linted for Linux, not run. (3) #369 closes with the merge and a passing hosted run on Linux and macOS.

## Later: the work list (not in 0.2.1; none of it blocks anything)

- **Findings and tests:** #337 (a branch dispatch of the release workflow is refused only after the builds), #325 (a slow copy fails after ten minutes: L-140), #334 (a few-millisecond `audio` range: L-141), #312,
  #309, the `release.md` tidy (6.10 to 6.12 are records of past publishes), and #363 (a small freely licensed real-media test set, with an opt-in campaign on it: it would have caught #353 and #359).
- **Re-pins:** FFmpeg (a month-end build; the next is 2026-10-31, L-132) with whisper.cpp (#322, L-137); the Dependabot pull requests #192, #193 and #194. **#351 (CVE-2026-107678, L-144, accepted):** look for the upstream fix when the build is chosen or cherry-pick one, re-run the ancestry tool, and add a malformed-MP4 `pssh` case to the malicious-media campaign.
- **The README design session** (`roadmap.svg`, the badges and the graphics, within the claims ladder: L-121) and any promotion, which the maintainer starts (ADR 0024 decision G).
- **From the cold round (none filed):** the no-transcript answer is `INVALID_ARGUMENT` with a true remediation (L-127's family). **R1 options:** a stub per ended
  request (L-063); the Windows kill window (L-129); a bounded wait for the initialization lock (L-131, L-135); retrying Windows sharing violations (L-136); #325's later steps; #334 (L-141). **At v2:** the codes
  L-127 keeps. **#204** Codex on Windows (L-076). **#188** (SEC-T01) is R1 technical debt by decision E though its title still says before the R0 release.

## Tracked issues and gates

- **Fixed on `main`, closed on GitHub, not released:** #353 (L-145), #340 (L-142), #342, #345. **For 0.2.1 / rc.2, open:** #359 (L-149, high; the fix is pull request #377, open), #365 (L-150), #366 (L-151), #367 (L-152), #368 (L-153), #370 (L-154), **#369 (fixed) and #372 (its test is written) in the Answers pull request, not merged**, #371, #373, #349 (skill wording;
  in rc.1's skill, issue still open), **#374 (to be verified; no register entry)**. **Open, other:** #361 (L-146), #362 (L-147), #363, #351 (L-144), #17, #219, #224 (L-095, L-139), #188, #232, #246 (deferred), #258, #263, #272 (residual accepted, L-122),
  #312 (L-135), #314 (fixed in rc.2 of 0.2.0; L-136), #316 (hosted runners scarce), #336, #322 (the floor is in; the pin stays, L-137), #325, #334, #337; #128 (watch); flaky tests #253 (Windows kill test) and #268
  (macOS SIGTERM test): comment with the run link and rerun the job. #321 is closed.
- **A change after a candidate's tag may touch only the work record, the installation guide and the launcher's README** until the stable version is published (`release.md` 6.8; the stable commit is compared with the highest `-rc.N` tag of its own version).
  **`P14 verify release` for a stable version must be dispatched within seven days of its publish** (the delta record lives in the publish run's artifact; `release.md` 6.4 and 6.13).
- **Opt-in real-tool paths** (`--ignored`) run a Cargo-built binary unless `VSIFT_E2E_BINARY` names an installed one (L-042); a failed run is a finding: issue first, then rerun. **P14 tools:**
  `tools/p14-published/test/`, `tools/p14-campaigns/test/`. **Campaigns:** never on the maintainer's machine (L-056, L-057) except the agent batches; see `docs/development.md`.

## What the next session reads first

`memory/project_current_status.md`, then `docs/planning/release-0.2.1.md` (the plan: where things stand, scope, what rc.2 needs, the evidence, the four waivers, the order, the open decisions) and `docs/operations/release.md` 6.14 (the candidate's steps),
`docs/planning/known-limits.md` (L-145, L-149 to L-160 and L-142 for the 0.2.1 work; L-105, L-108, L-121, L-133 for the release and the README), `docs/planning/p14-stable-release-steps.md` (the model for the stable `0.2.1` checklist), and the delivery ledger.
R1 starts with P15's decision packet, not before the maintainer says.

## Guardrails

- **The skill** calls the published CLI only; a new public item needs a skill update in the same change (`skill_contract`); its fuzz seed copy (`fuzz/seeds/handoff_check/SKILL.md`) must equal it byte for byte. **New public items** need their `*::ALL` entry, v1 schemas and a human renderer with a snapshot;
  v1 is additive only; a parser of untrusted input needs a fuzz target; npm packages have no scripts.
- **The release seam:** a change to `vsift-release`, `release.yml` or its lint needs a lint rule, a broken-copy mutation test and the shell check; no workflow runs `npm dist-tag`. **Trials:** a change to the skill,
  grader, scenarios or settings after a batch's first counted run voids the batch (`freeze check`); a hold-out failure is a finding, never a grader edit. **Commits:** the commit path reruns the crash campaign; no
  development feature in a release. **Public text** uses "supported", "stable" and "qualified" only inside registered statements; every change to a quoted sentence updates `public-claims.json`.
