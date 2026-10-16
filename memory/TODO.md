# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now (2026-10-16)

**`0.2.1` is stable and published** (tag `v0.2.1` at `e9ab6e626733`; npm `latest` is `0.2.1`, `next` is `0.2.1-rc.3`; GitHub's latest release is `v0.2.1`). It fixes #353 (a sparse 30-second chunk failed a whole transcription), #359 (speech dropped silently at chunk seams; what the repair leaves is L-155 to L-160, issues #380 to #384) and #391 (`setup check` called a slow, healthy tool missing; the default deadline is now 30 seconds), and adds `--view brief`, coverage on `transcript get`, the missing-path remediations and the second skill text. Its post-publish records are merged (#411): the ledger, `release.md` 6.17, the register, the pages the release could not change.
- **One evidence item is `failed` for `0.2.1`: RQ-05.** The published journeys failed twice on hosted Windows, in the worker's shutdown stage (#404, L-167); Ubuntu and macOS passed. The maintainer decided that it is a known issue of `0.2.1` and the first fix of `0.2.2`. No waiver is recorded, so `release-evidence --complete-for 0.2.1` names RQ-05 alone, and the `0.2.2` candidate's check will name it until the journeys are recorded again on that candidate.
- **The publish of 2026-10-15** needed a second attempt because the job's read-back of `latest` gave up after five minutes with the packages already published (#403); 6.17 has the steps and what the services showed. `0.2.1-rc.1` and `0.2.1-rc.2` are deprecated; `0.2.1-rc.3`, `0.2.0` and earlier are not.

## Cycle 1 (`0.2.2`) has started

The maintainer's delivery model of 2026-10-16 is in `docs/planning/delivery-governance.md` (the policy) and `docs/planning/backlog-iterations.md` (the cycle map, with every issue); the weekly window as a procedure is `docs/operations/release.md` 6.18.
- **Four cycles:** 1 = `0.2.2` (the fixes found during `0.2.1`, with the test review folded in); 2 = `0.2.3` (#363's real-media set, the FFmpeg and whisper.cpp re-pins, the seam follow-ups #380 to #384 measured on real speech, the evaluations); 3 = launch readiness (the README, the docs sweep, the clean-machine try-out RQ-17, help text for agents without the skill, a launch checklist); then R1 (P15 to P20), online sources (#406 to #410) among it.
- **Cycle 1 order:** #404 and #257 first; then #366, #367, #370, #371; then #396, #395, #394, #274, #334, #361, #362, #325, #266, #264, #265, #277, #232, #403, #337; and the test review (flaky tests, runners and harness, trial design incl. #397). Stories that do not overlap run in parallel.
- **Standing approvals inside a cycle:** the supervisor merges on green after review, files issues for findings, and closes issues whose own closing criteria are met, with the evidence in the comment. **Every `release` environment gate and every npm write stays with the maintainer.** Both memory files are rewritten in every pull request; parallel pull requests rebase before merge.
- **Iteration A is done (2026-10-16):** #365, #369, #372, #373, #349, #17 and #40 closed with evidence comments; #366, #367, #368, #370 and #371 have status comments and stay open; #359 stays open for the maintainer's own listening check on the two recordings.
- **In flight:** this change (the model, written down); a separate change to the CI workflows and `docs/development.md` (the gate and the nightly); the README is a separate session that starts early in this cycle.

## The test and release model, in one view (decided 2026-10-16; the machinery is not all built)

- **Pull-request gate:** only what the change can affect; target 5 minutes, hard limit 10; docs-only runs the document and governance checks; Windows and macOS only for platform-sensitive changes; a hot fix can ask for the full three-system run.
- **Nightly on `main`** when something merged: a failure opens an issue and stops development until fixed; **a candidate is cut only from a commit whose nightly passed** (to be enforced by the release workflow).
- **Weekly window:** cut at Friday or Saturday midnight; the long hosted runs follow, for what the week touched; publish on Monday or Tuesday on the maintainer's approval; a failure gets a targeted fix and a rerun of only the failing workflow.
- **Hot fix:** outside the window: the full ~30-minute three-system check, the publish, then the quick checks of the published bytes.
- **Monthly, or with a major change** (a skill rewrite, a re-pin): agent batches 2 and 3, the dependency-quality checks, the full durability campaign. Not weekly; an ordinary week's skill wording change gets the uncounted skill smoke (about $6). The weekly drift runs stay and open an issue on failure.
- **Two limits to remember:** the evidence ledger's scopes are coarse (`crates` is in most items' scope), so "limited to what the week touched" does not alone make the completeness check pass; and the freeze of `release.md` 6.8 still stops code merges between a candidate's tag and the stable commit.

## Open for the maintainer

- **#188** (SEC-T01, waived as RQ-14), **#176** (library API, MSRV, MCP adapter), **#246** (staged publishing), **#374** (the packaged desktop client check).
- **Moving `next`** (L-108; it names `0.2.1-rc.3`, a pre-release of `latest`'s source) and **the Yarn first-day note** (an untagged `yarn add` inside its one-day gate was not tried).
- **Choices to ratify or reverse:** #377's (the union guard, half a second, the stored member and what going back costs), the Reads change's, #392's (30 seconds, no deadline option on `setup plan` or `setup install`).
- **The folder answer that differs by platform:** a folder named as the source is `STORAGE_IO` on Windows and `INVALID_SOURCE` on Linux and macOS (align or keep).
- **RQ-05 is recorded `failed` for `0.2.1`** (no waiver; a waiver would not expire by itself). The user-facing note about #404 (installation guide section 12, the README's status) stays as written unless you say otherwise.
- **The window's details:** the time zone of "midnight", and whether a release branch should replace the freeze of 6.8 so development can continue during the window.
- **Register reviews `pending`:** 95 of 149 entries. **The next free register number is L-168.**

## What the next session reads first

`memory/project_current_status.md`, then `docs/planning/backlog-iterations.md`, `docs/planning/delivery-governance.md` (the model), `docs/operations/release.md` 6.18, `docs/planning/release-0.2.1.md` (the section "The evidence on 0.2.1 (recorded 2026-10-16)"), `docs/planning/known-limits.md` (L-146 onwards) and the evidence ledger. R1 starts with P15's decision packet, not before the maintainer says.

## Guardrails

- **The skill** calls the published CLI only; a new public item needs a skill update in the same change (`skill_contract`); its fuzz seed copy (`fuzz/seeds/handoff_check/SKILL.md`) must equal it byte for byte. **New public items** need their `*::ALL` entry, v1 schemas and a human renderer with a snapshot;
  v1 is additive only; a parser of untrusted input needs a fuzz target; npm packages have no scripts.
- **The release seam:** a change to `vsift-release`, `release.yml` or its lint needs a lint rule, a broken-copy mutation test and the shell check; no workflow runs `npm dist-tag`. After a candidate's tag, only the work record, the installation guide and the launcher's README may change until the stable release is published (`release.md` 6.8). `P14 verify release` for a stable release within seven days of its publish.
- **Trials:** a change to the skill, grader, scenarios or settings after a batch's first counted run voids the batch (`freeze check`); a hold-out failure is a finding, never a grader edit; a state file is never edited by hand without a disclosed decision (L-166).
- **Issues:** a failed run is a finding: issue first, then one re-run. No closing keywords ("Fix #n") in a squash title when the issue must stay open (a merge title closed #359 by accident on 2026-10-11).
- **Public text** uses "supported", "stable" and "qualified" only inside registered statements; every change to a quoted sentence updates `public-claims.json`.
