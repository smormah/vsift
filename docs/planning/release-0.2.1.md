# The patch release 0.2.1: the plan

Status: a plan, rewritten on 2026-10-11 after the maintainer's live testing on real recordings. **`0.2.1` does not exist. Its first candidate, `0.2.1-rc.1`, was published on 2026-10-10 under npm's `next`
and is SUPERSEDED; a second candidate, `0.2.1-rc.2`, is to be cut. Nothing new for it is merged yet: the fix of #359 is pull request #377 (open) and the designs of the larger changes are not decided.**
Nothing here is evidence: the evidence ledger holds nothing for any `0.2.1` candidate. It decides nothing for the maintainer: every decision it lists as open is open, and the designs of the larger changes
below are not decided. The runbook is [`release.md`](../operations/release.md) section 6.14, the current position is in `memory/project_current_status.md`, and the register entries are
[L-145](known-limits.md#l-145) to [L-154](known-limits.md#l-154), [L-155](known-limits.md#l-155) to [L-160](known-limits.md#l-160) (the limits #359's fix leaves) and [L-142](known-limits.md#l-142).

## What this release is, and why

`0.2.0` was published on 2026-10-09 and is what `npm install vsift-cli` installs. The first real recording tried with it, the day after, found a defect that the synthetic recordings of every earlier test could not:
`transcript retranscribe` failed the whole run, kept nothing and said the speech recogniser was missing ([#353](https://github.com/smormah/vsift/issues/353), [L-145](known-limits.md#l-145), high; **loud**, and fixed
on `main`). The maintainer decided on 2026-10-10 that a patch release carries that fix with everything else that was waiting, and the first candidate was cut and published the same evening.

The maintainer's live testing on two real recordings (a 34:36 lecture and a 17:34 spoken walkthrough, on `0.2.0`; findings posted 2026-10-10) then found a **second serious transcription defect, silent**: merging the
overlapping 30-second pieces can drop speech at a seam while the result still says the whole range was transcribed ([#359](https://github.com/smormah/vsift/issues/359), [L-149](known-limits.md#l-149), high; the
cause is inferred from the source and timings, not confirmed with raw recogniser output, and nobody has listened to the audio yet), and a set of findings about how an agent uses the CLI and the skill on a
recording that is more than a few minutes long. **On 2026-10-11 the maintainer decided to put #359 into `0.2.1`, which changes the transcription code, so a second candidate will be cut, and that the larger findings
ride along.** **It is a patch release, not a work packet.** P00 to P14 stay complete, `0.2.0` stays published and is not deprecated, and **`0.2.0` is affected by both [L-145](known-limits.md#l-145) and
[L-149](known-limits.md#l-149) until `0.2.1` is published**: the README and the installation guide carry a known-issue note for each, with the workarounds.

## Where things stand (2026-10-11)

| Version | State |
| --- | --- |
| `0.2.0` | Published 2026-10-09; npm `latest`; not deprecated. Has L-145 (fixed on `main`, not released) and L-149 (the fix is pull request #377, open). |
| `0.2.1-rc.1` | **Published 2026-10-10** (tag `v0.2.1-rc.1` at `cb8cb3a`; Release run 38084137607; a GitHub pre-release; npm `next` on all four packages). `P14 verify release` (run 38086846526) passed: `next 0.2.1-rc.1, latest 0.2.0`, GitHub's latest release `v0.2.0`. It carries the fix of #353, the changes of #340, #342 and #345 and the first new skill, **and it has L-149 too** (`merge_chunks` is the same code as in `0.2.0`). **Superseded by rc.2; not deprecated.** |
| rc.1's hosted evidence | **Stopped on purpose, by the supervisor session, on the maintainer's decision of 2026-10-11 to put #359 into `0.2.1`:** that changes the transcription code, so the evidence gathered on rc.1 is obsolete. The supervisor session stopped the hosted-evidence agent and cancelled the hosted campaign runs it had dispatched for rc.1 (the 8 cancelled runs below); what had already finished stays as history. Of the 14 hosted runs started after the publish, 6 finished green (`P14 verify release`, `P13 managed smoke`, `P14 compatibility`, `P14 scan reading`, `P14 runbook walk`, `P14 malicious media`) and 8 were cancelled (`P14 published artifacts`, `P14 journeys`, `Fuzz`, `P14 stress`, `P14 load`, `P07 local ASR`, `P13 managed power loss`, `P10 durability campaign`). **None of them is recorded in the ledger as evidence for rc.1 or for `0.2.1`, and the cancelled ones show nothing.** Agent batches 2 and 3 were not run on rc.1; they are run once, on rc.2. |
| `main` | Has everything rc.1 has (it was rc.1's commit, `cb8cb3a`, when this was written) and nothing for the findings below; #359's fix is pull request #377, open. |
| `0.2.1-rc.2` | Planned, not started. |

## What is in it

**Already on `main` and in rc.1:**

| Change | Where | What it does | What it does not do |
| --- | --- | --- | --- |
| Transcription: a stretch whose answer cannot be used is a recorded gap | #358 (#353) | the run finishes `partial` and lists what it did not transcribe; a search says "not transcribed", never "no speech"; one rejected piece in a stretch with fewer than four is tolerated; text already there is kept; when most of the recording cannot be transcribed the run still fails, says which reason, how many stretches and where, and the job is not left resumable | it is **not shown on the recording that found it** (commercial, cannot be committed): the tests use stand-in recordings; the thresholds are proposals; a session that holds a gap cannot be read by `0.2.0` or earlier; it does nothing for L-149 |
| The audio clip's text; a retry hint for `BUSY` | #357 (#340, #342) | `audio`'s help and readable result say an agent cannot listen to a clip; an evidence command's `BUSY` carries `retry_after_ms` 2000 and a remediation | the JSON result of `audio` is unchanged; no cold-scenario expectation was added |
| A flaky Windows test | #356 (#345) | the test retries the start race | the race is still in the harness's `run` (part of the frozen grader) |
| The agent skill, first change | the cut of rc.1 (#349 and the three above) | "video walkthrough"; a partial transcription's stretches are gaps with the reason `untranscribed_range`; a `BUSY` is retried once; an audio clip tells an agent nothing | no trial has run on it and no scenario exercises the partial rule |

**Decided on 2026-10-11 to go into `0.2.1` (rc.2); the design of each is NOT decided** (a design pass is under way; the issues' proposals are for decision):

| Issue | Register | The finding | What the issue proposes (for decision) |
| --- | --- | --- | --- |
| [#359](https://github.com/smormah/vsift/issues/359) | [L-149](known-limits.md#l-149), high | speech at a chunk seam can be dropped with no warning and the coverage still says "transcribed" | keep a cut segment whose range reaches beyond its replacement, or decide ownership by time; report what merging and validation dropped as ranges with times; trim near-duplicates at seams. **The fix is pull request #377 (open, not merged): a guard on the union of the chunks' segments (half a second) and a report of the stretches where words that no segment holds may lie, with the limits L-155 to L-160; the issue's third point (duplicates) is not done.** Needs an ADR 0017 note, domain and contract tests (the first row's timings and the mirror case), `cli-v1.md` and a listener's check on the two recordings |
| [#365](https://github.com/smormah/vsift/issues/365) | [L-150](known-limits.md#l-150) | each `transcript get` and `search` item repeats about 850 characters; a page of 50 is about 54,000 characters and an agent client lost items and the cursor | an additive opt-in compact form without the per-revision constants; a page size in the skill that fits; settle the `--limit` help (1 to 100 against a budget of 50) |
| [#366](https://github.com/smormah/vsift/issues/366) | [L-151](known-limits.md#l-151) | `handoff check` cannot take a long report on Windows (both skill forms use the command line) | a way to give the check a report that is not on the command line, or a check of the `vsift-handoff` block alone, or a stated size limit; measure the failing lengths first |
| [#367](https://github.com/smormah/vsift/issues/367) | [L-152](known-limits.md#l-152) | `candidates` on a long recording runs past a client's tool limit with no output and is too dense to page | a way to ask only for real screen changes; progress or an expected time; skill guidance for long recordings |
| [#368](https://github.com/smormah/vsift/issues/368) | [L-153](known-limits.md#l-153) | a whole-recording question (summary, timeline) does not fit the budgets or the report's limits | first decide whether whole-recording questions are in scope for this line; then a procedure, a profile and schema limits that fit, or a description that says they are not |
| [#370](https://github.com/smormah/vsift/issues/370) | [L-154](known-limits.md#l-154) | `frame get` is `BUSY` while a transcription of the session runs | frames can be extracted meanwhile, or the `BUSY` names the job and the skill says to ask after it |

**Smaller findings, also for rc.2 (no register entries: wording and skill-text findings, tracked by their issues):**
[#369](https://github.com/smormah/vsift/issues/369) (`ingest` of a path that names no file answers `STORAGE_IO` with an empty remediation: keep the code, add a remediation and a contract test, related to #265);
[#371](https://github.com/smormah/vsift/issues/371) (`session list` returns one session per page and the skill has no form to read the next page: add the form and the rule to continue until `next_cursor` is null);
[#372](https://github.com/smormah/vsift/issues/372) (the skill does not say that a range retranscription gives unchanged segments new ids);
[#373](https://github.com/smormah/vsift/issues/373) (the skill's step 3 asks for `coverage` and `transcript_coverage` that `transcript get` does not return: say it comes from `search`, or add the member).
**[#374](https://github.com/smormah/vsift/issues/374) is TO BE VERIFIED, and nothing is claimed about it:** sessions made by an agent in a packaged desktop client were stored under that client's own cache
folder, and whether an ordinary terminal's `session list` and `session clean` see them was not tried. Verify first (after an agent session in the desktop app, run `vsift session list --json` in an ordinary terminal and
compare); if it is confirmed, say it in the installation guide and the lifecycle text and let `setup check` or `session list` name the session root in use. It has no register entry until it is verified.

**Not in `0.2.1`:** the FFmpeg and whisper.cpp re-pins ([#351](https://github.com/smormah/vsift/issues/351), CVE-2026-107678, [L-132](known-limits.md#l-132), [L-137](known-limits.md#l-137), [L-144](known-limits.md#l-144): the
maintainer accepted it for R0 and decided to fix it after the release; the next month-end build is 2026-10-31), [#337](https://github.com/smormah/vsift/issues/337) (a branch dispatch of the release workflow is refused only after the
builds), [#325](https://github.com/smormah/vsift/issues/325) (a slow copy fails after ten minutes, L-140), [#334](https://github.com/smormah/vsift/issues/334) (a few-millisecond `audio` range, L-141) and the small freely
licensed **real-media test set** ([#363](https://github.com/smormah/vsift/issues/363): pauses, typing, music, several speakers, and an opt-in campaign on it), which is what would have caught #353 and #359. The Dependabot pull requests stay open. The
register entries L-146 (a model swapped during a run), L-147 (a worker host's `retranscribe` step is `complete` with `coverage` null when chunks were unreadable; a schema change in a later release) and L-148 stay open and `pending` review.

## What rc.2 needs

1. **The changes above, each its own pull request with regression tests at the lowest useful layer.** #359 first (its fix is pull request #377, open); the larger designs after the design pass; the small ones. A public contract
   change needs its schema, example, human renderer and `cli-v1.md` in the same pull request, and the register entry stays until the release.
2. **A new skill and a new agent-trial freeze.** The skill changes again (at least #371 to #373, and probably #365 to #368 and #370), so its digest changes, the freeze is cut again and its fuzz seed copy follows byte for byte
   ([`docs/agents/trials.md`](../agents/trials.md), "The freeze (P14)"). The rc.1 freeze is not reused.
3. **The cut:** the version text in the five files, the runbook (`release.md` 6.14; see the last section for what is not known about how it ran), and the pages that name the candidate.
4. **Hosted evidence, once, on rc.2.** The table below. Nothing from rc.1's runs carries over, and none of them was recorded.
5. **Agent batches 2 and 3, once, on rc.2,** under the new freeze, on the maintainer's explicit go and allowances (below).
6. **The decisions the check does not ask for:** the four waivers (below) and the version number (open decisions).

## The evidence the candidate needs

The staleness rule of the evidence ledger (ADR 0024) lets an entry recorded for an earlier version count only if nothing in its item's scope changed since. For rc.1 the check named 15 of the 20 items
(RQ-01 to RQ-09, RQ-11, RQ-12, RQ-13, RQ-15, RQ-18 and RQ-19: each is `passed` for `0.2.0` or `0.2.0-rc.3`, and about 55 to 250 files in its scope changed since); **it did not name RQ-10, RQ-14, RQ-16 and RQ-17,
which are `waived`, or RQ-20 (the check itself)**, and that is the danger: a waiver does not expire by itself, so the check passes those four for `0.2.1` without a single decision about them (next section). The
list for rc.2 is recomputed at its cut; expect the same shape, with more files changed.

What is run, in the order of [`release.md`](../operations/release.md) 6.14 (everything hosted is read-only and spends hosted minutes; the repeat for `0.2.0`'s second candidate cost about 3,650 job-minutes and its third was of the
same order; the same parameters are used, so the figures compare):

| Item | What | Where and when | Why it matters for this release |
| --- | --- | --- | --- |
| RQ-19 | `P14 verify release`, `version=0.2.1-rc.2` | `main`, right after the publish | it must read `next 0.2.1-rc.2, latest 0.2.0` and GitHub's latest release `v0.2.0` (rc.1's read the same way) |
| RQ-01 to RQ-04 | `P14 published artifacts` with `from_version=0.2.0`, then `0.1.0`, one after another; `P14 compatibility` | `main`; the tag | clean installs, the archives, the offline install, the upgrades; **the installed package must carry the new skill** (byte for byte against the tag's `skills/vsift`). rc.1's run of this was cancelled |
| RQ-05, RQ-06 | `P14 journeys`, `P13 managed smoke` (`published_version`), `P07 local ASR` | `main`; the tag for `P07` | the journeys on three systems; the transcription path changes again; its real-decode tests are opt-in (L-137, L-141) and no campaign has a recording with a long pause or a long first segment, so this shows ordinary recordings are unharmed, not that #353 or #359 is fixed |
| RQ-07 | `Fuzz`, 31 targets, 3,601 s each | the tag | the fix added optional stored members and a checkpoint kind, and a coverage change would add more; the seeds follow the skill |
| RQ-08 | `P14 stress` | the tag | the stress suites run on the candidate's source; #312 and #128 are still not fixed |
| RQ-09, RQ-10, RQ-12 | `P14 load`, `P14 malicious media`, `P14 runbook walk` | `main`, `version=0.2.1-rc.2` | the program changed; the same parameters as before, so the figures compare |
| RQ-11 | `P13 managed power loss`, `P10 durability campaign` | the tag | the job checkpoint gained a kind |
| RQ-13 | `P14 scan reading` and a new dated reading | `main`, `version=0.2.1-rc.2` | a dated reading is repeated for any change; no dependency or shipped tool is expected to change, so the findings are expected to be the 2026-10-10 reading's (CVE-2026-107678 among them, accepted) |
| RQ-15 | agent-trial batch 2 (34 runs), under the **new freeze** | the maintainer's explicit go, after the publish | the skill changed twice; the three models that met their gates did so with the third candidate's text, and the review tier's margin was small |
| RQ-16 | agent-trial batch 3 (18 runs, no skill), `-AllowGraderChange` | the maintainer's explicit go, separate from batch 2's | the cold agent reads `vsift --help`: `audio`'s help, the `BUSY` remediation and any new option are new |
| RQ-17 | the try-outs ([`rq-17-tryout-sheet.md`](rq-17-tryout-sheet.md)) | the maintainer | never done on any candidate; see the waivers |
| RQ-18 | the Governance job of the CI run at the candidate's commit | a push to `main` | it reads RQ-19's status, not its version ([L-133](known-limits.md#l-133)) |

**Agent batches 2 and 3 need the maintainer's explicit go and the maintainer's allowances.** The only measured costs are the third candidate's of `0.2.0` (batch 2 about $9 for Claude Code and 9 to 10 million input tokens for Codex; batch 3 about
$1.31 and 1.6 million). **A scenario that exercises the partial-transcript rule, a seam loss or a whole-recording question would change a frozen component and need its own freeze and candidate:** whether to add one, now or after `0.2.1`, is open.

## The waivers: each needs the maintainer's decision to carry to 0.2.1 (all open)

All four were decisions about `0.2.0-rc.3`'s evidence and were carried to `0.2.0`. None is a pass. The check does not ask, so each needs an explicit decision, and **this plan decides none of them. The stopped round of rc.1 decided none of them either:**

1. **RQ-10 (malicious media), the link case.** A symbolic link given as the video answers `STORAGE_IO` (#265, L-127), where the item's rule names three codes. Options the maintainer has: carry the waiver, run `P14 malicious media` on
   the candidate and waive again for the link alone (a `P14 malicious media` run started after rc.1's publish finished green and is not recorded), or change the rule (a change of a plan rule, not of evidence).
2. **RQ-14 (the strict worker).** The strict worker is not claimed to contain a hostile decoder (ADR 0024 decision E). Nothing in `0.2.1` touches isolation; whether the decision of 2026-10-03 carries to a new version
   without a restatement is the maintainer's.
3. **RQ-16 (the cold round), one action.** One cold agent read an audio clip file with `base64` (#340, L-142). `0.2.1` changes what `audio` says to a cold agent, and the batch has not been run on it: carry the waiver
   (the text fix is not a test of the text), or run batch 3 on the candidate and decide on its result.
4. **RQ-17 (the try-outs).** No Smart App Control, SmartScreen, clean-machine or Mac try-out was done, and `0.2.0` shipped untried (L-143). Whether `0.2.1` ships untried again, or the try-out ([L-098](known-limits.md#l-098); the sheet is
   written) happens first, is the maintainer's. The live testing on the maintainer's own machine is use, never qualification evidence.

## The order of steps

1. **Write the changes** (above), #359 first (pull request #377, open), each merged on its own with its regression tests; **the work record and register are kept current as each merges.**
2. **Cut rc.2** and **merge the cut only when the supervisor can tag and publish at once** (repository pages such as the guide, the changelog and the skill guide name `0.2.1-rc.2` before it is on npm; 6.14 step 0.1 lists them).
   A public page that the stable commit may not change (the README is one) is corrected before the cut and not after it, because the stable commit is compared with the highest `v0.2.1-rc.N` tag; the known-issue notes about `0.2.0` and
   `0.2.1-rc.1` stay true as they are.
3. **Tag, dry run, publish, approve, check from outside** (6.14 steps 1 to 4: the supervisor tags and dispatches, the maintainer approves the `release` environment); the publish moves `next` from `0.2.1-rc.1` to `0.2.1-rc.2`.
4. **The verification and hosted checks of the published bytes** (6.14 step 5), then the source-built campaigns at the tag, one after another and staggered (runner shortage, #316, shows as queueing).
5. **Agent batches 2 and 3**, on the maintainer's go, under the committed freeze (`freeze check` must say `nothing frozen has changed`); a miss is a finding, never an edit to the grader, the skill or a scenario.
6. **Record the evidence in the ledger** in a work-record pull request, from what the runs and `SUMMARY.md` computed; open an issue for any failed run before re-running (governance rule 14); decide each waiver (above).
7. **The stable commit `0.2.1`**: the version text in the same five files, the launcher's README and the installation guide if they need it, and the work record, **and nothing else** (`release.md` 6.8; the check compares it with the highest `v0.2.1-rc.N` tag).
   The steps are a checklist modelled on [`p14-stable-release-steps.md`](p14-stable-release-steps.md), which is written for that release and is not written yet.
8. **Publish `0.2.1`** (the first dry run and publish of a stable version since `0.2.0`; `latest` moves from `0.2.0` to `0.2.1`), and **dispatch `P14 verify release` for it within seven days** (the delta record is kept seven days).
9. **After the publish:** the work-record change that deletes the register entries that now have implementation, regression tests and a release (L-145, L-149 and the ones among L-150 to L-154 that are done; L-155 to L-160 are the limits the fix of #359 leaves and stay open), re-reads L-142, a look at the
   known-issue notes about `0.2.0` and `0.2.1-rc.1` in the README and the installation guide (they stay true for people still on those versions), and the maintainer's choices below.

## Open decisions

- **The version number (OPEN, nothing decided).** The larger changes may add new options or members to the v1 contract (a compact form of `transcript get` and `search`, a filter for `candidates`, a `transcript_coverage`
  member for `transcript get`, a way to give `handoff check` a report). They are additive, which the v1 policy allows, but they are more than a patch: **the maintainer may prefer `0.3.0` to `0.2.1`.** A change of number renames
  the candidate (`0.3.0-rc.1`, not `0.2.1-rc.2`), the five version files, the runbook, the changelog, the trial documents and the guide's marker, and every "planned for 0.2.1" in the README, the install guide, the register and this plan.
  The note in the README and the install guide says "0.2.1" today because that is what the maintainer decided on 2026-10-11.
- Carry each of the four waivers, or not (above).
- **Whether whole-recording questions are in scope for this release line** (the first question of #368), and what rc.2's skill says either way.
- Whether to deprecate `0.2.1-rc.1` on npm now (it has L-149 and the first skill's untried rules) or at the stable release, as `0.2.0-rc.1` and `-rc.2` were deprecated (the earlier decision, for `0.2.0`, was at the stable release and no sooner), and `0.2.0-rc.3` with it.
- Add a scenario for a partial transcription, a seam loss or a whole-recording question to the agent trials (a new freeze and candidate), now or after `0.2.1`.
- Confirm the severities of L-145 and L-149 (both high; L-149's reading of the rubric's first line is stated in its entry) and whether plan section 12's and 31.4's wording is amended; confirm L-146 to L-148, L-150 to L-154 and L-155 to L-160 (all `pending`).
- Who does the listener's check of the holes on the two recordings (the evidence L-149 needs; the recordings stay on the maintainer's machine), and who verifies #374.
- Whether to run the optional upgrade `0.2.0-rc.3` to `0.2.1-rc.2`, and whether to move `next` to `0.2.1` after it is published (the workflow never does; [L-108](known-limits.md#l-108)); until then `next` names the candidate.
- Whether `#340`'s additive JSON hint (a field in `audio`'s result) is still wanted; the cold round may inform it.
- The claims rung stays `candidate`; nothing here raises it.

## What is weaker than it sounds

- **The fix of #353 is shown by stand-ins.** The recording that found it cannot be committed; the thresholds are proposals; the kept-text rule has no test over a real decode.
- **The fix of #359 exists (pull request #377, open) and its cause is still inferred.** It is shown by tests built from the issue's timings and by a reviewer's synthetic simulation, not on a real recording: the opt-in real-tool stage was not run, no listener's check
  has been done on the two recordings, which stay on the maintainer's machine, the half second is derived for one configuration and not measured, and the guard trades holes for repeats (L-155) and leaves L-156 to L-159. A second real recording is the test that matters, and the real-media test set that would repeat it (#363) is not in this release.
- **The live testing is the maintainer's, on two recordings, on Windows 11 with npm, driven by one client (Claude Code).** Every finding of 2026-10-10 is an observation of that setup; none was checked again on `0.2.1-rc.1`, and the figures
  (page sizes, minutes, tokens) are the agents' and the clients' as the issues report them.
- **The skill change is a hypothesis, and it changes again.** The first change is untried, the grader matches text and cannot read a negation, and the batches are small (three runs per scenario).
- **Every earlier result is for an earlier version.** The ledger's `passed` entries, the third candidate's batches and the scan reading are for `0.2.0` or `0.2.0-rc.3`; rc.1's program differed from `0.2.0`'s in 31 source files of
  `crates/`, and rc.2's will differ in more.
- **A session with a gap cannot be read by `0.2.0` or earlier** (it reads as damaged), and **a session holding the new record member `unplaced_ranges` (#359, L-160) cannot be read by `0.2.0`, `0.2.1-rc.1` or earlier** (every revision of such a session fails as damaged, as do `handoff check --session` and `bundle validate`), so going back after using either loses the session.
- **6.14 ran once, for rc.1:** the Release run and the verification passed. What the run showed about the runbook (steps that were wrong or are missing) is not recorded in this plan; ask the person who ran it before rc.2.
- **The window of L-133 reopens for repository pages** (the guide's first page, the changelog, the skill guide, the trial documents, this plan and the runbook), which name the candidate between the merge of its cut and its
  publish; the README, the installation guide, the launcher's README and the security policy are worded to be true both ways.
