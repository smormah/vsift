# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now (2026-10-10)

**P00-P14 are complete: the whole of R0's work packets, recorded 2026-10-10 by P14 PR 13b** (`docs/planning/p14-qualification.md` section 31; ADR 0024 is Accepted with it). Nothing is announced and no
platform is called "supported": the claims rung is still `candidate`. **Complete is a statement about the ledger and the checker, not about readiness** (four evidence items are waived, none is a pass; RQ-13 is `passed` with one accepted finding: CVE-2026-107678, an FFmpeg record reachable through the MP4 demuxer with no fix in the shipped build, accepted by the maintainer on 2026-10-10 and to be fixed after the release: L-144, #351).
- **The release `0.2.0` is published and checked.** Published 2026-10-09 (tag `v0.2.0` at `eeb2a22a46a8`, the merge of #344; Release run 37946261087): `latest` is `0.2.0` and `next` is `0.2.0-rc.3` on all four
  packages. `0.2.0-rc.1` and `0.2.0-rc.2` are deprecated (2026-10-10); `0.2.0-rc.3`, `0.2.0` and `0.1.0` are not. `release.md` 6.13 records what the first move of `latest` did (trusted publishing accepted
  `--tag latest`, the release was marked latest, npm took about two and a half minutes: L-105). It is built from the third candidate's source (same program, other version number).
- **What was run on `0.2.0`'s own bytes (2026-10-09, all green):** `P14 published artifacts` from `0.1.0` and from `0.2.0-rc.3`, `P14 journeys` on three systems, `P13 managed smoke`, the CI and Guide runs at the
  stable commit. `P14 verify release` on `0.2.0` was red on exactly the two stable checks that were not yet registered (by design, run 37999880619); P14 PR 13a (#348) registered them and its green run is RQ-19's
  evidence in the ledger. The ledger carries the third candidate's evidence for RQ-07 to RQ-12 and RQ-14 to RQ-17 by the recorded `release_delta`. **Four items are `waived`, none is a pass:** RQ-10 (a symbolic
  link answers `STORAGE_IO`), RQ-14 (the strict worker is not claimed to contain a hostile decoder), RQ-16 (one cold agent read an audio clip file) and RQ-17 (no Smart App Control, SmartScreen, clean-machine or Mac
  try-out: the release ships untried).
- **The neutral checkpoint for using the published CLI (plan section 12) is met and raised.** The maintainer installed `0.2.0` on their own machine on 2026-10-10 and copied the skill, byte for byte, into their own
  agent client's folder; nothing is installed or configured on their machine by a session, and the notes of that use are reviewed in batches. The trial is never qualification evidence.
- **The LOKI try-out (RQ-17) is still available** (`docs/planning/rq-17-tryout-sheet.md`, being updated in another pull request): it would give CL-201 its evidence, and until it is done the Windows row stays untried.
- **The pages that named the candidate are corrected** (README, skill guide, development guide, release runbook, `SECURITY.md`, three sentences of `install.md`); `roadmap.svg` and the README's "status:
  pre-release" badge are not (L-121): they wait for the README design session.

## Open for the maintainer

- **RQ-05 on the release's own tag lacks** the Ubuntu and Windows ASR gates and the durable path (the third candidate's runs stand in `prior`; RQ-09 and RQ-12 carry): dispatch `P07 local ASR`, `P14 load`, `P14
  runbook walk` at `v0.2.0` or accept (31.5 item 2). **`P14 compatibility` was not dispatched at `v0.2.0`** (item 3).
- **The claims rung stays `candidate`.** Moving it to `after_p14` makes CL-201 to CL-209 stale unless each is used or deleted: six could be used on passed evidence (CL-202, 203, 204, 205, 207, 209) and three cannot
  (CL-201 needs RQ-17, CL-206 RQ-16, CL-208 RQ-14: waived). That is wording for the README, `install.md` and the matrix, which is yours (31.5 item 4). **Move `next`?** (L-108; still `0.2.0-rc.3`.)
- **Close #17** (the P14 issue) and **#321** (3,000 clean hosted repetitions); #312 fix or accept; #340 which option (help text, an additive JSON hint, or accept and fix in `0.2.x`).
- **The register:** 75 of 129 entries are still `pending` (no public statement leans on them); L-139 stays rejected (to be fixed, #336); the MSRV policy is undecided; `Guide` is not a required check.

## After the release: the work list (none of it blocks anything)

- **Findings and tests:** #340, #342 (`frame get` answers `BUSY` with no retry hint), #349 (the skill description's wording, in the next skill change), #345 (a flaky Windows trial-harness test), #337 (a branch
  dispatch of the release workflow is refused only after the builds), #325 (a slow copy fails after ten minutes: L-140), #334 (a few-millisecond `audio` range: L-141), #312, #309, the `release.md` tidy (6.10 to 6.12 are
  records of past publishes).
- **Re-pins:** FFmpeg (a month-end build; the next is 2026-10-31, L-132) with whisper.cpp (#322, L-137); the Dependabot pull requests #192, #193 and #194. **#351 (CVE-2026-107678, L-144, accepted):** look for the upstream fix when the build is chosen or cherry-pick one, re-run the ancestry tool, and add a malformed-MP4 `pssh` case to the malicious-media campaign.
- **The README design session** (`roadmap.svg`, the badges and the graphics, within the claims ladder: L-121) and any promotion, which the maintainer starts (ADR 0024 decision G).
- **From the cold round (none filed):** `--limit` help says 1 to 100 where the budget is 50; the no-transcript answer is `INVALID_ARGUMENT` with a true remediation (L-127's family). **R1 options:** a stub per ended
  request (L-063); the Windows kill window (L-129); a bounded wait for the initialization lock (L-131, L-135); retrying Windows sharing violations (L-136); #325's later steps; #334 (L-141). **At v2:** the codes
  L-127 keeps. **#204** Codex on Windows (L-076). **#188** (SEC-T01) is R1 technical debt by decision E though its title still says before the R0 release.

## Tracked issues and gates

- **Open:** #351 (L-144), #17, #219, #224 (L-095, L-139), #188, #232, #246 (deferred), #258, #263, #272 (residual accepted, L-122), #312 (L-135), #314 (fixed in rc.2; L-136), #316 (hosted runners scarce), #321, #336, #322 (the floor
  is in; the pin stays, L-137), #325, #334, #340, #342, #345, #337, #349; #128 (watch); flaky tests #253 (Windows kill test) and #268 (macOS SIGTERM test): comment with the run link and rerun the job.
- **A change after the stable release may touch any file**, but a new version is a new candidate: the code, the skill and the shipped documents are frozen at a candidate's cut (`release.md` 6.8 binds the next stable
  commit, which is compared with the highest `-rc.N` tag of its own version). **`P14 verify release` for a stable version must be dispatched within seven days of its publish** (the delta record lives in the publish run's
  artifact; `release.md` 6.4 and 6.13).
- **Opt-in real-tool paths** (`--ignored`) run a Cargo-built binary unless `VSIFT_E2E_BINARY` names an installed one (L-042); a failed run is a finding: issue first, then rerun. **P14 tools:**
  `tools/p14-published/test/`, `tools/p14-campaigns/test/`. **Campaigns:** never on the maintainer's machine (L-056, L-057) except the agent batches; see `docs/development.md`.

## What the next session reads first

`memory/project_current_status.md`, then `docs/planning/p14-qualification.md` section 31 (what P14 recorded and what is open), `docs/planning/known-limits.md` (L-105, L-121, L-133 for the release and the README),
`docs/operations/release.md` 6.13 and `docs/planning/p14-stable-release-steps.md` (the model for the next stable version), and the delivery ledger. R1 starts with P15's decision packet, not before the maintainer says.

## Guardrails

- **The skill** calls the published CLI only; a new public item needs a skill update in the same change (`skill_contract`). **New public items** need their `*::ALL` entry, v1 schemas and a human renderer with a snapshot;
  v1 is additive only; a parser of untrusted input needs a fuzz target; npm packages have no scripts.
- **The release seam:** a change to `vsift-release`, `release.yml` or its lint needs a lint rule, a broken-copy mutation test and the shell check; no workflow runs `npm dist-tag`. **Trials:** a change to the skill,
  grader, scenarios or settings after a batch's first counted run voids the batch (`freeze check`); a hold-out failure is a finding, never a grader edit. **Commits:** the commit path reruns the crash campaign; no
  development feature in a release. **Public text** uses "supported", "stable" and "qualified" only inside registered statements; every change to a quoted sentence updates `public-claims.json`.
