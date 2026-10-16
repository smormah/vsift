# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now (2026-10-16)

**`0.2.1` is published, checked and the current stable release** (tag `v0.2.1` at `e9ab6e626733`; npm `latest` is `0.2.1`, `next` is `0.2.1-rc.3`; GitHub's latest release is `v0.2.1`). It fixes #353 (a sparse 30-second chunk failed a whole transcription), #359 (speech dropped silently at chunk seams; what the repair leaves is L-155 to L-160, issues #380 to #384) and #391 (`setup check` called a slow, healthy tool missing; the default deadline is now 30 seconds), and adds `--view brief`, coverage on `transcript get`, the missing-path remediations and the second skill text.
- **The publish (2026-10-15):** dry run 38756334727; publish run 38758081349. Attempt 1 published the four packages and then failed: its read-back of `latest` gave up after five minutes while one package still read `0.2.0` (#403); attempt 2, approved again, created the GitHub release. From outside: `latest` 0.2.1 on all four packages, the 28 earlier integrity values and 7 tag commits unchanged, `npm audit signatures` 4 and 4, 10 release files and 4 tarballs attested and equal to the plan, a plain install prints `vsift 0.2.1 (e9ab6e626733)`. `0.2.1-rc.1` and `0.2.1-rc.2` were deprecated the same day; `0.2.1-rc.3`, `0.2.0` and earlier are not.
- **Hosted checks of the stable's bytes:** `P14 verify release` 38764125726 (22 checks), `P13 managed smoke` 38764760524, `P14 published artifacts` from `0.2.0` (38765075608) and from `0.2.1-rc.3` (38765697122) and `P14 scan reading` 38777879949 passed. **`P14 journeys` 38764446399 failed on hosted Windows twice, in the worker's shutdown stage** (providers still running after the batch ended; then a cancelled recognition reported as `MISSING_CAPABILITY`); Ubuntu and macOS passed; `0.2.1-rc.3` passed the same stage. Issue #404, register entry L-167.
- **The post-publish records (section 11 of `docs/planning/release-0.2.1-stable-steps.md`) are written by the records change of 2026-10-16** (a pull request that was open and not merged when this was written): the ledger's `release_delta` and the entries for `0.2.1`, `release.md` 6.17 (6.15 and 6.16 marked records), the pages the stable commit could not change (README, `SECURITY.md`, the skill guide, the development guide, the installation guide, the guide's limits pages, the sentence in `trials.md`), the register (L-145, L-149 and L-150 deleted; L-161 rewritten; L-167 added) and these two files. Items 1 to 3 of section 11 are done; item 4 is the maintainer's choices, below.
- **One check does not pass:** `release-evidence --complete-for 0.2.1` names RQ-05 alone. The item is recorded `failed` (issues 258, 263, 404), because its rule (per system, every stage that can run passed) was not met on Windows and has no clause for an accepted finding. A waiver would pass the check, but it does not expire by itself and would let the next candidate through without a journeys run; leaving it `failed` makes the `0.2.2` candidate's check name RQ-05. This change records no waiver: that is the maintainer's decision.

## The maintainer's decisions of 2026-10-16

1. **#404 is a known issue of `0.2.1` (a finding accepted for `0.2.1`) and the first fix of `0.2.2`.**
2. **The order of the iterations:** A (close what `0.2.1` settled), D (the test review, a plan first), B (`0.2.2`, #404 and #257 first), C (the re-pins with #363's real-media set), then R1.
3. **Online sources (#406, slices #407 to #410: direct HTTPS links; HLS and DASH; platform pages through the optional yt-dlp; live platform streams) are planned inside R1,** starting from P15's decision packet. Not a `0.2.x` change.

## Next: the backlog map

**[`docs/planning/backlog-iterations.md`](../docs/planning/backlog-iterations.md)** maps every open issue to an iteration and says how each is tested.
1. **A:** close what `0.2.1` settled (#359, #365, #366, #367 in part, #369, #370, #371, #372, #373, #349, #17, #40; #368 by decision). An hour, no release.
2. **D:** the test review (flaky tests, harness, the strict cold setting #397, "our code or a dependency's quality"), as a plan first.
3. **B:** `0.2.2`, #404 and #257 first, then #396, #395, #394, #274, #334, #361, #362, #325, #266, #264, #265, #277, #232, #403 (the read-back window), #337.
4. **C:** the re-pins (FFmpeg 2026-10-31 build, whisper.cpp 1.9.5 or later: #351, #272, #322, #399) with #363's real-media set and the seam follow-ups #380 to #384.
5. **R1** (P15 to P20) when the maintainer says; P15's decision packet first, online sources among it.

**Testing cadence (the maintainer's rule of 2026-10-15):** the full round (all hosted evidence and agent batches 2 and 3) runs about once a week, at a big shift or when many changes have accumulated, never per fix; a fix ships on its own tests plus the quick checks of the published bytes; a skill change gets an uncounted smoke; checks of a dependency's quality run when a pin, model or call changes.

## Open for the maintainer

- **RQ-05 for `0.2.1`:** waive it (as RQ-07 was for #394), leave it `failed`, or add a clause to the plan; and whether the user-facing note about #404 (installation guide section 12, the README's status) stays as written.
- **Moving `next`** (L-108; it names `0.2.1-rc.3`, a pre-release of `latest`'s source) and **the Yarn first-day note** (an untagged `yarn add` inside its one-day gate was not tried).
- **#188** (SEC-T01, waived as RQ-14), **#176** (library API, MSRV, MCP adapter), **#246** (staged publishing), **#374** (the packaged desktop client check).
- **Choices to ratify or reverse:** #377's (the union guard, half a second, the stored member and what going back costs), the Reads change's, #392's (30 seconds, no deadline option on `setup plan` or `setup install`). **The folder answer that differs by platform:** a folder named as the source is `STORAGE_IO` on Windows and `INVALID_SOURCE` on Linux and macOS (align or keep).
- **Register reviews `pending`:** 95 of 149 entries, among them L-146 to L-148, L-151 to L-160, L-161 (rewritten, severity medium to low) and L-162 to L-167. **The next free register number is L-168.**

## What the next session reads first

`memory/project_current_status.md`, then `docs/planning/backlog-iterations.md`, `docs/planning/release-0.2.1.md` (the section "The evidence on 0.2.1 (recorded 2026-10-16)"), `docs/planning/known-limits.md` (L-146 onwards) and the evidence ledger. R1 starts with P15's decision packet, not before the maintainer says.

## Guardrails

- **The skill** calls the published CLI only; a new public item needs a skill update in the same change (`skill_contract`); its fuzz seed copy (`fuzz/seeds/handoff_check/SKILL.md`) must equal it byte for byte. **New public items** need their `*::ALL` entry, v1 schemas and a human renderer with a snapshot;
  v1 is additive only; a parser of untrusted input needs a fuzz target; npm packages have no scripts.
- **The release seam:** a change to `vsift-release`, `release.yml` or its lint needs a lint rule, a broken-copy mutation test and the shell check; no workflow runs `npm dist-tag`. After a candidate's tag, only the work record, the installation guide and the launcher's README may change until the stable is published (`release.md` 6.8). `P14 verify release` for a stable within seven days of its publish.
- **Trials:** a change to the skill, grader, scenarios or settings after a batch's first counted run voids the batch (`freeze check`); a hold-out failure is a finding, never a grader edit; a state file is never edited by hand without a disclosed decision (L-166).
- **Issues:** a failed run is a finding: issue first, then one re-run. No closing keywords ("Fix #n") in a squash title when the issue must stay open (a merge title closed #359 by accident on 2026-10-11).
- **Public text** uses "supported", "stable" and "qualified" only inside registered statements; every change to a quoted sentence updates `public-claims.json`.
