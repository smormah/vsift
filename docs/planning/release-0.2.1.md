# The patch release 0.2.1: the plan

Status: a plan, written on 2026-10-10 with the cut of the first release candidate, `0.2.1-rc.1`. **The candidate is cut and is not tagged or published; `0.2.1` does not exist.**
Nothing here is evidence: the evidence ledger holds nothing for `0.2.1-rc.1`. It decides nothing for the maintainer: every decision it lists as open is open. The
runbook for the candidate is [`release.md`](../operations/release.md) section 6.14, the current position is in `memory/project_current_status.md`, and the register entries are
[L-145](known-limits.md#l-145) to [L-148](known-limits.md#l-148) and [L-142](known-limits.md#l-142).

## What this release is, and why

`0.2.0` was published on 2026-10-09 and is what `npm install vsift-cli` installs. The day after, the first real recording tried with it (a 34-minute screencast with long pauses while the speaker typed)
found a defect that the synthetic recordings of every earlier test could not: `transcript retranscribe` failed the whole run, kept nothing and said the speech recogniser was missing, when one 30-second stretch
held a few recognised pieces of text and one of them was rejected (#353, [L-145](known-limits.md#l-145), high). The maintainer decided on 2026-10-10 that a patch release carries the fix together with
everything else that was waiting. **It is a patch release, not a work packet.** P00 to P14 stay complete, `0.2.0` stays published and is not deprecated, and **`0.2.0` is still affected by L-145
until `0.2.1` is published**: the README and the installation guide say so and give the workarounds (a transcript file you supply, or ranges that avoid the failing stretch).

## What is in it

| Change | Where | What it does | What it does not do |
| --- | --- | --- | --- |
| Transcription: a stretch whose answer cannot be used is a recorded gap | #358 (#353) on `main` | the run finishes `partial` and lists what it did not transcribe; a search says "not transcribed", never "no speech"; one rejected piece in a stretch with fewer than four is tolerated; text already there is kept; when most of the recording cannot be transcribed the run still fails, says which reason, how many stretches and where, and the job is not left resumable | it is **not shown on the recording that found it** (commercial, cannot be committed): the tests use stand-in recordings; the thresholds are proposals, not measured on real recordings; a session that holds a gap cannot be read by `0.2.0` or earlier |
| The audio clip's text; a retry hint for `BUSY` | #357 (#340, #342) on `main` | `audio`'s help and readable result say an agent cannot listen to a clip; an evidence command's `BUSY` carries `retry_after_ms` 2000 and a remediation | the JSON result of `audio` is unchanged, so an agent that never reads the help is not told; no cold-scenario expectation was added |
| A flaky Windows test | #356 (#345) on `main` | the test retries the start race | the race is still in the harness's `run` (part of the frozen grader), so a real trial can meet it |
| The agent skill | this cut (#349, and the three above) | "video walkthrough"; a partial transcription's stretches are gaps with the reason `untranscribed_range`; a `BUSY` is retried once; an audio clip tells an agent nothing | **no trial has run on it and no scenario exercises the partial rule** |
| The version, a new trial freeze, the guide's marker, the runbook | this cut | `0.2.1-rc.1` in five files; a new freeze for the skill's new digest; the guide names `0.2.1`; `release.md` 6.14 | nothing here publishes anything |

**Not in `0.2.1`:** the FFmpeg and whisper.cpp re-pins ([#351](https://github.com/smormah/vsift/issues/351), CVE-2026-107678, [L-132](known-limits.md#l-132), [L-137](known-limits.md#l-137), [L-144](known-limits.md#l-144): the
maintainer accepted it for R0 and decided to fix it after the release; the next month-end build is 2026-10-31), [#337](https://github.com/smormah/vsift/issues/337) (a branch dispatch of the release workflow is refused only after the
builds), [#325](https://github.com/smormah/vsift/issues/325) (a slow copy fails after ten minutes, L-140), [#334](https://github.com/smormah/vsift/issues/334) (a few-millisecond `audio` range, L-141) and the small freely
licensed **real-media test set** ([#363](https://github.com/smormah/vsift/issues/363): pauses, typing, music, several speakers, and an opt-in campaign on it), which is what would have caught #353. The Dependabot pull requests stay open. The
register entries L-146 (a model swapped during a run), L-147 (a worker host's `retranscribe` step is `complete` with `coverage` null when chunks were unreadable; a schema change in a later release) and L-148 stay open and `pending` review.

## The evidence the candidate needs

The staleness rule of the evidence ledger (ADR 0024) lets an entry recorded for an earlier version count only if nothing in its item's scope changed since. Nothing was edited to make it pass or fail:
**`release-evidence --complete-for 0.2.1-rc.1` names 15 of the 20 items** (RQ-01 to RQ-09, RQ-11, RQ-12, RQ-13, RQ-15, RQ-18 and RQ-19: each is `passed` for `0.2.0` or `0.2.0-rc.3`, and about 55 to 250 files in its
scope changed since; the exact output is in the pull request that cut the candidate). **It does not name RQ-10, RQ-14, RQ-16 and RQ-17, which are `waived`, or RQ-20 (the check itself)**, and that is the danger:
a waiver does not expire by itself, so the check would pass those four for `0.2.1` without a single decision about them (next section).

What is run again, in the order of [`release.md`](../operations/release.md) 6.14 (everything hosted is read-only and spends hosted minutes; the second candidate's repeat cost about 3,650 job-minutes and the third's was of the same
order; the same parameters are used, so the figures compare):

| Item | What | Where and when | Why it matters for this release |
| --- | --- | --- | --- |
| RQ-19 | `P14 verify release`, `version=0.2.1-rc.1` | `main`, right after the publish | the first verification of a candidate published while `latest` is a stable release: it must read `next 0.2.1-rc.1, latest 0.2.0` and GitHub's latest release `v0.2.0` |
| RQ-01 to RQ-04 | `P14 published artifacts` with `from_version=0.2.0`, then `0.1.0`, one after another; `P14 compatibility` | `main`; the tag | clean installs, the archives, the offline install, the upgrades; **the installed package must carry the new skill** (byte for byte against the tag's `skills/vsift`) |
| RQ-05, RQ-06 | `P14 journeys`, `P13 managed smoke` (`published_version`), `P07 local ASR` | `main`; the tag for `P07` | the journeys on three systems; the transcription path changed, but its real-decode tests are opt-in (L-137, L-141) and no campaign has a recording with a long pause, so this shows ordinary recordings are unharmed, not that #353 is fixed |
| RQ-07 | `Fuzz`, 31 targets, 3,601 s each | the tag | the fix added optional stored members and a checkpoint kind, and the fuzz seeds cover them; the skill's seed copy follows the skill |
| RQ-08 | `P14 stress` | the tag | the stress suites run on the candidate's source; #312 and #128 are still not fixed |
| RQ-09, RQ-10, RQ-12 | `P14 load`, `P14 malicious media`, `P14 runbook walk` | `main`, `version=0.2.1-rc.1` | the program changed (the transcription path, the evidence commands' `BUSY` answer); the same parameters as before, so the figures compare |
| RQ-11 | `P13 managed power loss`, `P10 durability campaign` | the tag | the job checkpoint gained a kind |
| RQ-13 | `P14 scan reading` and a new dated reading | `main`, `version=0.2.1-rc.1` | a dated reading is repeated for any change; no dependency or shipped tool changed, so the findings are expected to be the 2026-10-10 reading's (CVE-2026-107678 among them, accepted) |
| RQ-15 | agent-trial batch 2 (34 runs), under the **new freeze** | the maintainer's explicit go, after the publish | the skill changed; the three models that met their gates did so with the old text, and the review tier's margin was small |
| RQ-16 | agent-trial batch 3 (18 runs, no skill), `-AllowGraderChange` | the maintainer's explicit go, separate from batch 2's | the cold agent reads `vsift --help`: `audio`'s help and the `BUSY` remediation are new |
| RQ-17 | the try-outs ([`rq-17-tryout-sheet.md`](rq-17-tryout-sheet.md)) | the maintainer | never done on any candidate; see the waivers |
| RQ-18 | the Governance job of the CI run at the candidate's commit | a push to `main` | it reads RQ-19's status, not its version ([L-133](known-limits.md#l-133)) |

**Agent batches 2 and 3 need the maintainer's explicit go and the maintainer's allowances.** The only measured costs are the third candidate's (batch 2 about $9 for Claude Code and 9 to 10 million input tokens for Codex; batch 3 about
$1.31 and 1.6 million). A scenario that exercises the partial-transcript rule would change a frozen component and need its own freeze and candidate: whether to add one, now or after `0.2.1`, is open.

## The waivers: each needs the maintainer's decision to carry to 0.2.1 (all open)

All four were decisions about `0.2.0-rc.3`'s evidence and were carried to `0.2.0`. None is a pass. The check does not ask, so each needs an explicit decision, and **this plan decides none of them**:

1. **RQ-10 (malicious media), the link case.** A symbolic link given as the video answers `STORAGE_IO` (#265, L-127), where the item's rule names three codes. Options the maintainer has: carry the waiver, run `P14 malicious media` on
   the candidate and waive again for the link alone, or change the rule (a change of a plan rule, not of evidence).
2. **RQ-14 (the strict worker).** The strict worker is not claimed to contain a hostile decoder (ADR 0024 decision E). Nothing in `0.2.1` touches isolation; whether the decision of 2026-10-03 carries to a new version
   without a restatement is the maintainer's.
3. **RQ-16 (the cold round), one action.** One cold agent read an audio clip file with `base64` (#340, L-142). `0.2.1` changes what `audio` says to a cold agent, and the batch has not been run on it: carry the waiver
   (the text fix is not a test of the text), or run batch 3 on the candidate and decide on its result.
4. **RQ-17 (the try-outs).** No Smart App Control, SmartScreen, clean-machine or Mac try-out was done, and `0.2.0` shipped untried (L-143). Whether `0.2.1` ships untried again, or the try-out ([L-098](known-limits.md#l-098); the sheet is
   written) happens first, is the maintainer's. The first real recording was tried on the maintainer's own machine on `0.2.0`; that use is never qualification evidence.

## The order of steps

1. **Merge the cut only when the supervisor can tag and publish at once** (repository pages such as the guide, the changelog and the skill guide name `0.2.1-rc.1` before it is on npm; 6.14 step 0.1 lists them).
2. **Tag, dry run, publish, approve, check from outside** (6.14 steps 1 to 4: the supervisor tags and dispatches, the maintainer approves the `release` environment).
3. **The verification and hosted checks of the published bytes** (6.14 step 5), then the source-built campaigns at the tag, one after another and staggered (runner shortage, #316, shows as queueing).
4. **Agent batches 2 and 3**, on the maintainer's go, under the committed freeze (`freeze check` must say `nothing frozen has changed`); a miss is a finding, never an edit to the grader, the skill or a scenario.
5. **Record the evidence in the ledger** in a work-record pull request, from what the runs and `SUMMARY.md` computed; open an issue for any failed run before re-running (governance rule 14); decide each waiver (above).
6. **The stable commit `0.2.1`**: the version text in the same five files, the launcher's README and the installation guide if they need it, and the work record, **and nothing else** (`release.md` 6.8; the check compares it with the highest `v0.2.1-rc.N` tag).
   The steps are a checklist modelled on [`p14-stable-release-steps.md`](p14-stable-release-steps.md), which is written for that release and is not written yet.
7. **Publish `0.2.1`** (the first dry run and publish of a stable version since `0.2.0`; `latest` moves from `0.2.0` to `0.2.1`), and **dispatch `P14 verify release` for it within seven days** (the delta record is kept seven days).
8. **After the publish:** the work-record change that closes L-145 and re-reads L-142, a look at the known-issue note about `0.2.0` in the README and the installation guide (it stays true for people still on `0.2.0`), and the maintainer's choices below.

## Open decisions

- Carry each of the four waivers, or not (above).
- Add a scenario for a partial transcription to the agent trials (a new freeze and candidate), now or after `0.2.1`.
- Confirm L-145's severity and whether plan section 12's and 31.4's wording is amended; confirm L-146 to L-148's severities (all `pending`).
- Whether to run the optional upgrade `0.2.0-rc.3` to `0.2.1-rc.1`, and whether to deprecate `0.2.1-rc.1` and `0.2.0-rc.3` when `0.2.1` is published (the earlier decision, for `0.2.0`, was to deprecate at the stable release and no sooner).
- Whether to move `next` to `0.2.1` after it is published (the workflow never does; [L-108](known-limits.md#l-108)); until then `next` names the candidate.
- Whether `#340`'s additive JSON hint (a field in `audio`'s result) is still wanted; the cold round may inform it.
- The claims rung stays `candidate`; nothing here raises it.

## What is weaker than it sounds

- **The fix is shown by stand-ins.** The recording that found #353 cannot be committed; the thresholds are proposals; the kept-text rule has no test over a real decode. A second real recording is the test that matters, and none is planned in this release
  (#363 is the way to have one).
- **The skill change is a hypothesis.** It is untried, the grader matches text and cannot read a negation, and the batches are small (three runs per scenario).
- **Every earlier result is for an earlier version.** The ledger's `passed` entries, the third candidate's batches and the scan reading are for `0.2.0` or `0.2.0-rc.3`; this candidate's program differs in 31 source files of `crates/`.
- **A session with a gap cannot be read by `0.2.0` or earlier** (it reads as damaged), so going back after using a gap session loses it.
- **6.14 has not been run.** Its commands are 6.12's with new numbers; what is new or untried is reading `latest` as `0.2.0` instead of the placeholder at every step, GitHub's latest release as `v0.2.0` (it was a 404), the sixteen-value integrity
  loop and the supervisor's and the maintainer's split.
- **The window of L-133 reopens for repository pages** (the guide's first page, the changelog, the skill guide, the trial documents, this plan and the runbook), which name the candidate between the merge and the publish; the README, the installation guide, the launcher's README and the security policy are worded to be true both ways.
