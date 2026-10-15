# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now (2026-10-15, evening)

**`0.2.1` is published and checked: it is the current stable release** (tag `v0.2.1` at `e9ab6e626733`; npm `latest` is `0.2.1`, `next` is `0.2.1-rc.3`; GitHub's latest release is `v0.2.1`).
- **What it fixes since `0.2.0`:** #353 (a sparse 30-second chunk failed a whole transcription), #359 (speech dropped silently at chunk seams; the repair leaves limits L-155 to L-160, issues #380 to #384), #391 (`setup check` called a slow, healthy tool missing; the default deadline is now 30 seconds; verified by the maintainer on a second Windows 11 machine). It adds `--view brief`, coverage on `transcript get`, the missing-path remediations (#369) and the second skill text.
- **Published and checked 2026-10-15:** dry run 38756334727; publish run 38758081349 (attempt 1 published the four packages but its npm read-back gave up after five minutes while one package's `latest` propagated, #403; attempt 2, approved again, finished the GitHub release). From outside: `latest` 0.2.1 on all four packages, the 28 earlier integrity values and 7 tag commits unchanged, `npm audit signatures` 4 and 4, 10 release files and 4 registry tarballs attested and equal to the plan, a plain install prints `vsift 0.2.1 (e9ab6e626733)`.
- **Deprecated 2026-10-15:** `0.2.1-rc.1` and `0.2.1-rc.2` on all four packages ("Superseded by 0.2.1. Install vsift-cli."); `0.2.1-rc.3`, `0.2.0` and earlier are not. #391 is closed.
- **Hosted checks of the stable's bytes:** `P14 verify release` 38764125726, `P13 managed smoke` 38764760524, `P14 published artifacts` from 0.2.0 (38765075608) and from 0.2.1-rc.3 (38765697122), `P14 scan reading` 38777879949: all passed. **`P14 journeys` 38764446399 failed on hosted Windows only, twice** (attempt 1: provider processes still running after the batch ended; attempt 2: a cancelled recognition reported as `MISSING_CAPABILITY`): #404, a race on the worker's shutdown path; `0.2.1-rc.3`'s run passed the same stage with the same code. The maintainer's decision is open (recommended: a finding for `0.2.1`, fixed in `0.2.2`).

## Next: the backlog map

**[`docs/planning/backlog-iterations.md`](../docs/planning/backlog-iterations.md)** maps every open issue to an iteration and says how each is tested. Suggested order, for the maintainer to decide:
1. **A:** close what `0.2.1` settled (#359, #365, #366, #367 in part, #369, #370, #371, #372, #373, #349, #17, #40; #368 by decision).
2. **D:** the test review (flaky tests, harness, the strict cold setting #397, "our code or a dependency's quality"), as a plan first.
3. **B:** `0.2.2`, #404 and #257 first, then #396, #395, #394, #274, #334, #361, #362, #325, #266, #264, #265, #277, #232, #403, #337.
4. **C:** the re-pins (FFmpeg 2026-10-31 build, whisper.cpp 1.9.5 or later: #351, #272, #322, #399) with #363's real-media set and the seam follow-ups #380 to #384.
5. R1 (P15 to P20) when the maintainer says.

**Testing cadence (the maintainer's rule of 2026-10-15):** the full round (all hosted evidence and agent batches 2 and 3) runs about once a week, at a big shift or when many changes have accumulated, never per fix; a fix ships on its own tests plus the quick checks of the published bytes; a skill change gets an uncounted smoke; checks of a dependency's quality run when a pin, model or call changes.

## The post-publish work record (due now)

`docs/planning/release-0.2.1-stable-steps.md` section 11: copy the publish run's `release-delta.json` into the ledger's `release_delta`; give RQ-01 to RQ-06, RQ-18 and RQ-19 their entries for `0.2.1` (journeys with #404); fold the steps page into `docs/operations/release.md` as 6.17 and mark 6.15 and 6.16 as records; restore the sentence about batches 2 and 3 in `docs/agents/trials.md`; delete the register entries closed by `0.2.1` (L-145, L-149, the done ones of L-150 to L-154) and rewrite L-161; rewrite `memory/project_current_status.md` for the published `0.2.1` (it still says "planned for 0.2.1" in places).

## Open for the maintainer

- **#404:** a finding for `0.2.1` and a fix in `0.2.2`, or investigate now.
- **The order of the iterations** above, and **#188** (SEC-T01, waived as RQ-14), **#176** (library API, MCP adapter, MSRV), **#246** (staged publishing), **#374** (the packaged desktop client check).
- **Reviews `pending`:** most register entries, among them L-149, L-155 to L-166. **Choices to ratify or reverse:** #377's (the union guard, half a second, the stored member and its rollback cost), the Reads change's, #392's (30 seconds, no deadline option on `setup plan` or `setup install`).
- **From 0.2.1's answers:** a folder named as the source is `STORAGE_IO` on Windows and `INVALID_SOURCE` on Linux and macOS (align or keep?). **The next free register number is L-167.**

## What the next session reads first

`memory/project_current_status.md`, then `docs/planning/backlog-iterations.md`, `docs/planning/release-0.2.1.md` (the 0.2.1 record and decisions), `docs/planning/release-0.2.1-stable-steps.md` (section 11, the post-publish work), `docs/planning/known-limits.md` (L-145 to L-166) and the delivery ledger. R1 starts with P15's decision packet, not before the maintainer says.

## Guardrails

- **The skill** calls the published CLI only; a new public item needs a skill update in the same change (`skill_contract`); its fuzz seed copy (`fuzz/seeds/handoff_check/SKILL.md`) must equal it byte for byte. **New public items** need their `*::ALL` entry, v1 schemas and a human renderer with a snapshot;
  v1 is additive only; a parser of untrusted input needs a fuzz target; npm packages have no scripts.
- **The release seam:** a change to `vsift-release`, `release.yml` or its lint needs a lint rule, a broken-copy mutation test and the shell check; no workflow runs `npm dist-tag`. After a candidate's tag, only the work record, the installation guide and the launcher's README may change until the stable is published (`release.md` 6.8). `P14 verify release` for a stable within seven days of its publish.
- **Trials:** a change to the skill, grader, scenarios or settings after a batch's first counted run voids the batch (`freeze check`); a hold-out failure is a finding, never a grader edit; a state file is never edited by hand without a disclosed decision (L-166).
- **Issues:** a failed run is a finding: issue first, then one re-run. No closing keywords ("Fix #n") in a squash title when the issue must stay open (a merge title closed #359 by accident on 2026-10-11).
- **Public text** uses "supported", "stable" and "qualified" only inside registered statements; every change to a quoted sentence updates `public-claims.json`.
