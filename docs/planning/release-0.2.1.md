# The patch release 0.2.1: the plan

Status: a plan, updated on 2026-10-13 with the fix of a finding from a second machine ([#391](https://github.com/smormah/vsift/issues/391), `setup check`), after the cut and publish of the second candidate. **`0.2.1` does not exist. Its first candidate, `0.2.1-rc.1`, was published on 2026-10-10 under npm's `next`
and is SUPERSEDED; its second, `0.2.1-rc.2`, was cut and PUBLISHED on 2026-10-11 (npm `next`) and has the defect of #391, as every published version has (`0.1.0`, `0.2.0` and its candidates, `0.2.1-rc.1`). The maintainer decided on 2026-10-13 to fix it in `0.2.1`, so a THIRD candidate, `0.2.1-rc.3`, will be cut once the fix is on `main`: it is not cut, and rc.2 is superseded by it
once it exists.** The sections below that speak of "the cut" and "this change" describe the cut of rc.2 as it was written on 2026-10-11 and are kept as written; the table "Where things stand" and the section "The third candidate" are the position now. The cut of rc.2 contains the fix of #359 (#377), the Answers change (#369 and #372, #378), the Reads change (#365's brief view, #373's coverage on `transcript get` and the brief view of #367's candidates,
#379), a second change to the agent skill with a new agent-trial freeze, and the version bump; the runbook for it is `release.md` 6.15. The maintainer decided on 2026-10-11 that the version stays `0.2.1`, that #368 is out of scope for this line, that `transcript get`
over a part nobody transcribed answers `partial` as `search` does, and that the brief view is built on `transcript get`, `search` and `candidates`; **the designs of the other larger findings (#366, #367 beyond the brief view, #370, #371) are not built: the skill answers them in words only.**
Nothing here is evidence: the evidence ledger holds nothing for any `0.2.1` candidate. It decides nothing for the maintainer: every decision it lists as open is open, and the designs of the larger changes
below are decided only where it says so. The runbook is [`release.md`](../operations/release.md) section 6.15 (6.14 is the record of the first candidate), the current position is in `memory/project_current_status.md`, and the register entries are
[L-145](known-limits.md#l-145) to [L-154](known-limits.md#l-154), [L-155](known-limits.md#l-155) to [L-160](known-limits.md#l-160) (the limits #359's fix leaves), [L-161](known-limits.md#l-161) (#391) and [L-142](known-limits.md#l-142).

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

## Where things stand (2026-10-13)

| Version | State |
| --- | --- |
| `0.2.0` | Published 2026-10-09; npm `latest`; not deprecated. Has L-145 and L-149 (both fixed in the published candidate `0.2.1-rc.2`, in no release that `latest` names) and L-161 (#391; fixed on `main`, in no candidate yet). |
| `0.2.1-rc.1` | **Published 2026-10-10** (tag `v0.2.1-rc.1` at `cb8cb3a`; Release run 38084137607; a GitHub pre-release; npm `next` on all four packages). `P14 verify release` (run 38086846526) passed: `next 0.2.1-rc.1, latest 0.2.0`, GitHub's latest release `v0.2.0`. It carries the fix of #353, the changes of #340, #342 and #345 and the first new skill, **and it has L-149 too** (`merge_chunks` is the same code as in `0.2.0`). **Superseded by rc.2; not deprecated.** |
| rc.1's hosted evidence | **Stopped on purpose, by the supervisor session, on the maintainer's decision of 2026-10-11 to put #359 into `0.2.1`:** that changes the transcription code, so the evidence gathered on rc.1 is obsolete. The supervisor session stopped the hosted-evidence agent and cancelled the hosted campaign runs it had dispatched for rc.1 (the 8 cancelled runs below); what had already finished stays as history. Of the 14 hosted runs started after the publish, 6 finished green (`P14 verify release`, `P13 managed smoke`, `P14 compatibility`, `P14 scan reading`, `P14 runbook walk`, `P14 malicious media`) and 8 were cancelled (`P14 published artifacts`, `P14 journeys`, `Fuzz`, `P14 stress`, `P14 load`, `P07 local ASR`, `P13 managed power loss`, `P10 durability campaign`). **None of them is recorded in the ledger as evidence for rc.1 or for `0.2.1`, and the cancelled ones show nothing.** Agent batches 2 and 3 were not run on rc.1 (nor on rc.2: below). |
| `0.2.1-rc.2` | **Published 2026-10-11** (tag `v0.2.1-rc.2` at `23ad4927`, the merge of #385; Release run 38135772111; a GitHub pre-release; npm `next` on all four packages). `P14 verify release` (run 38139064324) passed: `next 0.2.1-rc.2, latest 0.2.0`, GitHub's latest release `v0.2.0`. It carries the fix of #359 (#377), the Answers and Reads changes (#378, #379), the second skill and its freeze. **It has the defect of #391** ([L-161](known-limits.md#l-161)): the first `setup check` after a restart can report installed tools as `[timeout]` and advise installing them; the same code is in every published version (`0.1.0`, `0.2.0` and its candidates, `0.2.1-rc.1`; read from each tag). **To be superseded by rc.3 once that exists; not deprecated.** |
| rc.2's hosted checks | **Five hosted runs were made on rc.2's published bytes on 2026-10-11, all green, and that is all of rc.2's evidence that was spent:** `P14 verify release` (run 38139064324), `P14 journeys` (38139202117), `P13 managed smoke` (38139337654) and `P14 published artifacts` twice, from `0.2.0` (38139472845) and from `0.1.0` (38139753709). **None is recorded in the ledger, and none carries to rc.3**, whose `setup check` code differs. The source-built campaigns at the tag (`P14 compatibility`, `Fuzz`, `P14 stress`, `P07 local ASR`, `P13 managed power loss`, `P10 durability campaign`), the runs from `main` with the version (`P14 load`, `P14 malicious media`, `P14 runbook walk`, `P14 scan reading`) and agent batches 2 and 3 were not started for rc.2. |
| `main` | Has everything rc.2 has (it was at rc.2's commit when the fix was written) and, with the fix of #391, a 30-second default deadline for the dependency probes (it was 5), advice of its own for a tool whose probe timed out, and a `setup plan` in which such a probe causes no managed action. The version text is still `0.2.1-rc.2`: the fix does not bump it. |
| `0.2.1-rc.3` | **Not cut.** It will be `main` after the fix of #391 plus the version text; the skill is unchanged by the fix (the frozen components are byte for byte rc.2's, and both `freeze check` runs say `nothing frozen has changed`), so the agent-trial freeze of rc.2 stands unless the cut changes a frozen component. **Everything it needs is still to do: the cut, tag, dry run, publish, the hosted evidence once, agent batches 2 and 3 once (the maintainer's go), the four waivers' decisions, and the stable `0.2.1`.** Its runbook section is not written (`release.md` 6.15 is rc.2's). |

## What is in it

**Already on `main` and in rc.1:**

| Change | Where | What it does | What it does not do |
| --- | --- | --- | --- |
| Transcription: a stretch whose answer cannot be used is a recorded gap | #358 (#353) | the run finishes `partial` and lists what it did not transcribe; a search says "not transcribed", never "no speech"; one rejected piece in a stretch with fewer than four is tolerated; text already there is kept; when most of the recording cannot be transcribed the run still fails, says which reason, how many stretches and where, and the job is not left resumable | it is **not shown on the recording that found it** (commercial, cannot be committed): the tests use stand-in recordings; the thresholds are proposals; a session that holds a gap cannot be read by `0.2.0` or earlier; it does nothing for L-149 |
| The audio clip's text; a retry hint for `BUSY` | #357 (#340, #342) | `audio`'s help and readable result say an agent cannot listen to a clip; an evidence command's `BUSY` carries `retry_after_ms` 2000 and a remediation | the JSON result of `audio` is unchanged; no cold-scenario expectation was added |
| A flaky Windows test | #356 (#345) | the test retries the start race | the race is still in the harness's `run` (part of the frozen grader) |
| The agent skill, first change | the cut of rc.1 (#349 and the three above) | "video walkthrough"; a partial transcription's stretches are gaps with the reason `untranscribed_range`; a `BUSY` is retried once; an audio clip tells an agent nothing | no trial has run on it and no scenario exercises the partial rule |

**Decided on 2026-10-13 to go into `0.2.1` (rc.3): the finding from a second machine.** [#391](https://github.com/smormah/vsift/issues/391) ([L-161](known-limits.md#l-161), medium): on a second Windows 11 machine the first `vsift setup check` after a start reported an installed, healthy FFmpeg and FFprobe as `[timeout]`, called the setup `blocked` and advised installing them; the second run said `[ok]`. The three probes share one deadline, which was 5 seconds, and a timeout got the advice of a missing tool. The maintainer's decisions: fix it in `0.2.1`, raise the default, keep one shared deadline, no silent retry. **Built and on `main`:** the default is 30 seconds (the supervisor's proposal, confirmed by the merge); a timed-out tool is told what happened (started and no answer, or not started), under which deadline, and to run the check again or allow more time, in the readable output and in `remediation.next_step`, and is never told to install or locate the tool; in `setup plan` such a probe causes no managed action, and the tool's step says either that the plan proposes nothing because of it or, when the plan installs the paired FFmpeg and FFprobe build because the other tool is missing, that the build replaces it; after `--timeout-seconds 60` the advice no longer offers more time; the published `timed_out`, `blocked` and exit status are unchanged, and no schema changed. **Not shown:** the cold start itself (the tests use stand-in tools that never answer, and `--timeout-seconds 1`); anything on macOS or Linux; that 30 seconds is enough on a slow machine. **The skill is not changed** and does not name `timed_out`. The issue stays open until the released candidate has been run on the machine that showed it.

**Decided on 2026-10-11 to go into `0.2.1` (rc.2).** The design of #365, #373 and the brief part of #367 is decided and built (the Reads change); #368 is out of scope for this line; the others are for decision (the design of the second candidate is `release-0.2.1-findings-design.md`). What each issue proposed:

| Issue | Register | The finding | What the issue proposes (for decision) |
| --- | --- | --- | --- |
| [#359](https://github.com/smormah/vsift/issues/359) | [L-149](known-limits.md#l-149), high | speech at a chunk seam can be dropped with no warning and the coverage still says "transcribed" | keep a cut segment whose range reaches beyond its replacement, or decide ownership by time; report what merging and validation dropped as ranges with times; trim near-duplicates at seams. **The fix is on `main` (#377): a guard on the union of the chunks' segments (half a second) and a report of the stretches where words that no segment holds may lie, with the limits L-155 to L-160; the issue's third point (duplicates) is not done.** Needs an ADR 0017 note, domain and contract tests (the first row's timings and the mirror case), `cli-v1.md` and a listener's check on the two recordings |
| [#365](https://github.com/smormah/vsift/issues/365) | [L-150](known-limits.md#l-150) | each `transcript get` and `search` item repeats about 850 characters; a page of 50 is about 54,000 characters and an agent client lost items and the cursor | an additive opt-in compact form without the per-revision constants; a page size in the skill that fits; settle the `--limit` help (1 to 100 against a budget of 50) |
| [#366](https://github.com/smormah/vsift/issues/366) | [L-151](known-limits.md#l-151) | `handoff check` cannot take a long report on Windows (both skill forms use the command line) | a way to give the check a report that is not on the command line, or a check of the `vsift-handoff` block alone, or a stated size limit; measure the failing lengths first |
| [#367](https://github.com/smormah/vsift/issues/367) | [L-152](known-limits.md#l-152) | `candidates` on a long recording runs past a client's tool limit with no output and is too dense to page | a way to ask only for real screen changes; progress or an expected time; skill guidance for long recordings |
| [#368](https://github.com/smormah/vsift/issues/368) | [L-153](known-limits.md#l-153) | a whole-recording question (summary, timeline) does not fit the budgets or the report's limits | **decided 2026-10-11: out of scope for this line**; a description that says they are not, and a stated limit |
| [#370](https://github.com/smormah/vsift/issues/370) | [L-154](known-limits.md#l-154) | `frame get` is `BUSY` while a transcription of the session runs | frames can be extracted meanwhile, or the `BUSY` names the job and the skill says to ask after it |

**Smaller findings, also for rc.2 (no register entries: wording and skill-text findings, tracked by their issues):**
[#369](https://github.com/smormah/vsift/issues/369) (`ingest` of a path that names no file answers `STORAGE_IO` with an empty remediation: keep the code, add a remediation and a contract test, related to #265);
[#371](https://github.com/smormah/vsift/issues/371) (`session list` returns one session per page and the skill has no form to read the next page: add the form and the rule to continue until `next_cursor` is null);
[#372](https://github.com/smormah/vsift/issues/372) (the skill does not say that a range retranscription gives unchanged segments new ids);
[#373](https://github.com/smormah/vsift/issues/373) (the skill's step 3 asks for `coverage` and `transcript_coverage` that `transcript get` does not return: say it comes from `search`, or add the member).
**[#374](https://github.com/smormah/vsift/issues/374) is TO BE VERIFIED, and nothing is claimed about it:** sessions made by an agent in a packaged desktop client were stored under that client's own cache
folder, and whether an ordinary terminal's `session list` and `session clean` see them was not tried. Verify first (after an agent session in the desktop app, run `vsift session list --json` in an ordinary terminal and
compare); if it is confirmed, say it in the installation guide and the lifecycle text and let `setup check` or `session list` name the session root in use. It has no register entry until it is verified.

**What the cut of rc.2 contains of that list, finding by finding (2026-10-11):**

| Issue | In rc.2 | Not in rc.2 |
| --- | --- | --- |
| #359 | the fix (#377) and its limits L-155 to L-160 | the listener's check on the two recordings, a run on a real recording, the third point (duplicates) |
| #365 | `--view brief` on `transcript get`, `search` and `candidates` (#379); the skill uses it (pages of 50 only in the brief view on the standard profile) | one real run that pages with a cursor; the skill's use of it is untried, so the issue stays open |
| #366 | words only: the skill checks a report of about 5,500 characters or more in two parts (the text above the `vsift-handoff` block, then the block); no CLI change | a way to give the check a long report off the command line; the sizes are what the issue saw, not measured through the pinned client |
| #367 | the brief view of `candidates` (#379); the skill asks for one range of at most 5 minutes per call | a filter, progress events, parallel windows (not built, by decision); the first-call silence and the list's density, with the maintainer's counts on a real recording |
| #368 | out of scope for this line: the skill's description and a stated limit (a whole recording over 5 minutes is not summarised unless the user bounds the work) | any support for a whole-recording question |
| #369 | built (#378): three fixed remediation texts for `STORAGE_IO` | a locked file and any other failure to open a path stay with no remediation; the Unix cases ran on Linux only in the type check |
| #370 | words only: `frame get`, `frame burst` and `candidates` are not retried while a transcription runs; `session status`, `job status`, then a `visual` gap and the resume card | extracting frames meanwhile or a `BUSY` that names the job; no cause is named (L-154) |
| #371 | words only: the `--cursor` forms in the skill and the rule to continue until `next_cursor` is null | page filling for `session list` (not built, by decision) |
| #372 | a test (#378) that a superseded id still resolves; the skill says every segment of a range retranscription's revision has a new id | any behaviour change |
| #373 | built (#379): `transcript_coverage` always on `transcript get`, `partial` over a gap; the skill reads a partial read or search as usable and reports each stretch as a gap | the false gaps after a range retranscription (L-033) |
| #374 | nothing: it is to be verified | the verification, and any claim |

**Not in `0.2.1`:** the FFmpeg and whisper.cpp re-pins ([#351](https://github.com/smormah/vsift/issues/351), CVE-2026-107678, [L-132](known-limits.md#l-132), [L-137](known-limits.md#l-137), [L-144](known-limits.md#l-144): the
maintainer accepted it for R0 and decided to fix it after the release; the next month-end build is 2026-10-31), [#337](https://github.com/smormah/vsift/issues/337) (a branch dispatch of the release workflow is refused only after the
builds), [#325](https://github.com/smormah/vsift/issues/325) (a slow copy fails after ten minutes, L-140), [#334](https://github.com/smormah/vsift/issues/334) (a few-millisecond `audio` range, L-141) and the small freely
licensed **real-media test set** ([#363](https://github.com/smormah/vsift/issues/363): pauses, typing, music, several speakers, and an opt-in campaign on it), which is what would have caught #353 and #359. The Dependabot pull requests stay open. The
register entries L-146 (a model swapped during a run), L-147 (a worker host's `retranscribe` step is `complete` with `coverage` null when chunks were unreadable; a schema change in a later release) and L-148 stay open and `pending` review.

## The third candidate, `0.2.1-rc.3`: what it needs (2026-10-13)

1. **The fix of #391 on `main`**, with its regression tests, its register entry and its documents: done by the pull request that carries this text.
2. **The cut:** the version text `0.2.1-rc.3` in the five files, a runbook section for it (6.15 with the numbers changed, as 6.15 was 6.14's), and the pages that name the candidate (the guide's first page, the changelog, the skill guide, the trial documents, this plan, the register's L-108 and L-133). **To do; not part of the fix.**
3. **The freeze:** the fix changes no frozen component, so the committed freeze of rc.2 (`skill` `256b4840...`, whole freeze `a48567ac...`) is still what `freeze check` holds the tree to. Whether the cut writes a new freeze is decided at the cut; no batch ran under this one, so nothing would move aside.
4. **Tag, dry run, publish, approve, check from outside**, then **the hosted evidence, once, on rc.3** (the table below, with `0.2.1-rc.3` for the version): nothing from rc.1's or rc.2's runs carries over, and none of them was recorded. `P14 published artifacts` and `P14 journeys` run `setup check` on fresh hosted runners, which is the nearest thing to a cold start that is run; they do not show the fix, because nobody has seen them fail on it.
5. **Agent batches 2 and 3, once, on rc.3**, on the maintainer's explicit go and allowances.
6. **The check on the machine that showed #391**, by the maintainer, with the published rc.3: the first `setup check` after a start. It is the only evidence that closes the issue.
7. **The decisions the completeness check does not ask for:** the four waivers, and the deprecation of rc.1 and rc.2.

## What rc.2 needs (as written at its cut, 2026-10-11; rc.2 was then published, and what is left moves to rc.3)

1. **The changes above, each its own pull request with regression tests at the lowest useful layer.** Done for #359 (#377), #369 and #372 (#378) and #365, #367's brief view and #373 (#379); the skill's words for the rest are in the cut. A public contract
   change needs its schema, example, human renderer and `cli-v1.md` in the same pull request, and the register entry stays until the release.
2. **A new skill and a new agent-trial freeze: done in the cut.** The skill is one commit (the three commits of its branch, applied unchanged), its fuzz seed copy follows byte for byte, and `freeze write` gave `skill` `256b4840...` and the whole freeze `a48567ac...`; the other six digests are
   the third and fourth candidates' ([`docs/agents/trials.md`](../agents/trials.md), "The freeze (P14)"). The rc.1 freeze is not reused. **The skill may change again after the cut's pull request is opened** (an uncounted smoke of the text is being read); if it does, the freeze is written again on purpose
   and the cut is re-done before it is merged.
3. **The cut: done by this change:** the version text in the five files, the runbook (`release.md` 6.15), and the pages that name the candidate. 6.14 ran once; what that run showed about its steps is not recorded (see the last section).
4. **Hosted evidence, once, on rc.2.** The table below. Nothing from rc.1's runs carries over, and none of them was recorded. **To do.**
5. **Agent batches 2 and 3, once, on rc.2,** under the new freeze, on the maintainer's explicit go and allowances (below). **To do.**
6. **The decisions the check does not ask for:** the four waivers (below) and the deprecation of rc.1. The version number is decided (it stays `0.2.1`). **To do.**

## The evidence the candidate needs

**Read `0.2.1-rc.3` for the candidate: the table was written for rc.2, whose five hosted checks of 2026-10-11 are not recorded and do not carry over (above).**

The staleness rule of the evidence ledger (ADR 0024) lets an entry recorded for an earlier version count only if nothing in its item's scope changed since. For rc.1 the check named 15 of the 20 items
(RQ-01 to RQ-09, RQ-11, RQ-12, RQ-13, RQ-15, RQ-18 and RQ-19: each is `passed` for `0.2.0` or `0.2.0-rc.3`, and about 55 to 250 files in its scope changed since); **it did not name RQ-10, RQ-14, RQ-16 and RQ-17,
which are `waived`, or RQ-20 (the check itself)**, and that is the danger: a waiver does not expire by itself, so the check passes those four for `0.2.1` without a single decision about them (next section). The
list for rc.2 was recomputed at its cut (`release-evidence --complete-for 0.2.1-rc.2`, run as a reading and not as a gate): **the same 15 items, and the same waived ones not named**, with 90 to 130 files changed in the scope of each of them except RQ-13 and RQ-18, whose scopes include
the documents (over 300 and over 160). A candidate writes no `release_delta`; `vsift-release candidate-delta` cannot be run for one (it compares a stable commit with its accepted candidate and refuses a pre-release tree); the stable plan will compare the stable commit with `v0.2.1-rc.2`.

What is run, in the order of [`release.md`](../operations/release.md) 6.15 (everything hosted is read-only and spends hosted minutes; the repeat for `0.2.0`'s second candidate cost about 3,650 job-minutes and its third was of the
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

**Written at rc.2's cut and kept as written. Steps 1 to 3 were done for rc.2 (published 2026-10-11) and step 4 in part (five hosted checks, not recorded); the fix of #391 then joins step 1, and steps 2 to 6 are done again, once, for rc.3, with `0.2.1-rc.3` wherever a step names the candidate.**

1. **Write the changes** (above), #359 first (merged, #377), each merged on its own with its regression tests; **the work record and register are kept current as each merges.**
2. **Cut rc.2: done by this change, which is merged only when the supervisor can tag and publish at once** (repository pages such as the guide, the changelog and the skill guide name `0.2.1-rc.2` before it is on npm; 6.15 step 0.1 lists them).
   A public page that the stable commit may not change (the README is one) is corrected before the cut and not after it, because the stable commit is compared with the highest `v0.2.1-rc.N` tag; the known-issue notes about `0.2.0` and
   `0.2.1-rc.1` stay true as they are.
3. **Tag, dry run, publish, approve, check from outside** (6.15 steps 1 to 4: the supervisor tags and dispatches, the maintainer approves the `release` environment); the publish moves `next` from `0.2.1-rc.1` to `0.2.1-rc.2`.
4. **The verification and hosted checks of the published bytes** (6.15 step 5), then the source-built campaigns at the tag, one after another and staggered (runner shortage, #316, shows as queueing).
5. **Agent batches 2 and 3**, on the maintainer's go, under the committed freeze (`freeze check` must say `nothing frozen has changed`); a miss is a finding, never an edit to the grader, the skill or a scenario.
6. **Record the evidence in the ledger** in a work-record pull request, from what the runs and `SUMMARY.md` computed; open an issue for any failed run before re-running (governance rule 14); decide each waiver (above).
7. **The stable commit `0.2.1`**: the version text in the same five files, the launcher's README and the installation guide if they need it, and the work record, **and nothing else** (`release.md` 6.8; the check compares it with the highest `v0.2.1-rc.N` tag).
   The steps are a checklist modelled on [`p14-stable-release-steps.md`](p14-stable-release-steps.md), which is written for that release and is not written yet.
8. **Publish `0.2.1`** (the first dry run and publish of a stable version since `0.2.0`; `latest` moves from `0.2.0` to `0.2.1`), and **dispatch `P14 verify release` for it within seven days** (the delta record is kept seven days).
9. **After the publish:** the work-record change that deletes the register entries that now have implementation, regression tests and a release (L-145, L-149 and the ones among L-150 to L-154 that are done; L-155 to L-160 are the limits the fix of #359 leaves and stay open), re-reads L-142, a look at the
   known-issue notes about `0.2.0` and `0.2.1-rc.1` in the README and the installation guide (they stay true for people still on those versions), and the maintainer's choices below.

## Open decisions

- **The choices the fix of #391 made, confirmed by merging it or reversed:** the value 30 seconds (the maintainer chose "raise the default"; the figure is the supervisor's); the words of the advice; that a timed-out probe causes no managed action on any target and keeps the published disposition `manual_selection_required` (on Ubuntu a `PATH` tool that only timed out is no longer offered a managed build); that `setup plan` still probes twice, each time under its own deadline, when a managed tool is installed (up to about 60 seconds, and its two observations can disagree); that `setup plan` and `setup install` get no option for the deadline; that the local-ASR part of `setup check` still does not use the probes' results, so one answer can say `[timeout]` and `verified` ([L-161](known-limits.md#l-161) lists what the fix leaves).
- **Whether to deprecate `0.2.1-rc.2`** with rc.1, now or at the stable release.
- **The version number: decided on 2026-10-11, it stays `0.2.1`.** The larger changes add new options and members to the v1 contract (the `--view` option, a `transcript_coverage` member for `transcript get`; a way to give `handoff check` a
  report is not decided). They are additive, which the v1 policy allows; a change of number would have renamed the candidate (`0.3.0-rc.1`), the five version files, the runbook, the changelog, the
  trial documents, the guide's marker and every "planned for 0.2.1". It changes the status of one existing answer, a `transcript get` over a part nobody transcribed, which `search` already reported as `partial`.
- Carry each of the four waivers, or not (above).
- **What rc.2's skill says about whole-recording questions** (#368 is out of scope for this line, decided 2026-10-11): the description and a stated limit.
- Whether to deprecate `0.2.1-rc.1` on npm now (it has L-149 and the first skill's untried rules) or at the stable release, as `0.2.0-rc.1` and `-rc.2` were deprecated (the earlier decision, for `0.2.0`, was at the stable release and no sooner), and `0.2.0-rc.3` with it.
- Add a scenario for a partial transcription, a seam loss or a whole-recording question to the agent trials (a new freeze and candidate), now or after `0.2.1`.
- Confirm the severities of L-145 and L-149 (both high; L-149's reading of the rubric's first line is stated in its entry) and whether plan section 12's and 31.4's wording is amended; confirm L-146 to L-148, L-150 to L-154 and L-155 to L-160 (all `pending`).
- Who does the listener's check of the holes on the two recordings (the evidence L-149 needs; the recordings stay on the maintainer's machine), and who verifies #374.
- Whether to run the optional upgrades `0.2.1-rc.1` to `0.2.1-rc.2` and `0.2.0-rc.3` to `0.2.1-rc.2` (6.15 step 5), and whether to move `next` to `0.2.1` after it is published (the workflow never does; [L-108](known-limits.md#l-108)); until then `next` names the candidate.
- Whether `#340`'s additive JSON hint (a field in `audio`'s result) is still wanted; the cold round may inform it.
- The claims rung stays `candidate`; nothing here raises it.

## What is weaker than it sounds

- **The fix of #391 is shown by stand-ins, and the finding was seen once.** One machine, one start, one FFmpeg build; nobody reproduced it with the default deadline. The regression tests use tools that never answer, which shows what is said about a timeout and not that a cold start now passes; 30 seconds is a choice. A tool that truly hangs now holds `setup check` for 30 seconds, and with an unrecorded local-ASR check after it for about 95 seconds in the worst case (worked out from the code), nearer the two-minute limit of some agent clients; `setup plan` on a machine with a managed tool waits up to about 60. The independent review found one untrue step (a timed-out tool beside its missing pair was told the plan proposed nothing for it while the plan replaced it), which is corrected, and that `0.1.0` is affected too.
- **The fix of #353 is shown by stand-ins.** The recording that found it cannot be committed; the thresholds are proposals; the kept-text rule has no test over a real decode.
- **The fix of #359 is on `main` (#377) and its cause is still inferred.** It is shown by tests built from the issue's timings and by a reviewer's synthetic simulation, not on a real recording: the opt-in real-tool stage was not run, no listener's check
  has been done on the two recordings, which stay on the maintainer's machine, the half second is derived for one configuration and not measured, and the guard trades holes for repeats (L-155) and leaves L-156 to L-159. A second real recording is the test that matters, and the real-media test set that would repeat it (#363) is not in this release.
- **The live testing is the maintainer's, on two recordings, on Windows 11 with npm, driven by one client (Claude Code).** Every finding of 2026-10-10 is an observation of that setup; none was checked again on `0.2.1-rc.1`, and the figures
  (page sizes, minutes, tokens) are the agents' and the clients' as the issues report them.
- **The skill change is a hypothesis, and it may change once more.** Neither of its two changes has been run by a counted agent; no frozen scenario exercises most of the second (a whole-recording question, a report over 5,500 characters, a `BUSY` frame call during a transcription, `session list` paging,
  the new ids after a range retranscription, a partial read, an answer cut by the client), the grader matches text and cannot read a negation, and the batches are small (three runs per scenario). An uncounted smoke of the second change is being read, and if it changes the skill the freeze is written again.
- **Every earlier result is for an earlier version.** The ledger's `passed` entries, the third candidate's batches and the scan reading are for `0.2.0` or `0.2.0-rc.3`; rc.1's program differed from `0.2.0`'s in 31 source files of
  `crates/`, and rc.2's will differ in more.
- **A session with a gap cannot be read by `0.2.0` or earlier** (it reads as damaged), and **a revision holding the new record member `unplaced_ranges` (#359, L-160) is read as damaged by `0.2.0`, `0.2.1-rc.1` and earlier**: they read newest first, so if the newest revision of a session holds it nothing of the session can be read, and if only an older one does the newest reads normally and the older ones are refused (by id, in `handoff check --session` and in `bundle validate`); so going back after using either loses the session, or part of it.
- **6.14 ran once, for rc.1:** the Release run and the verification passed. What the run showed about the runbook (steps that were wrong or are missing) is not recorded in this plan, and 6.15 is 6.14 with the numbers changed (twenty earlier integrity values and five tag commits instead of sixteen and four) and one correction found while writing it (the title of the cut's pull request,
  which 6.14 named wrongly); ask the person who ran 6.14 before 6.15 is used.
- **The window of L-133 reopens for repository pages** (the guide's first page, the changelog, the skill guide, the trial documents, this plan and the runbook), which name the candidate between the merge of its cut and its
  publish; the README, the installation guide, the launcher's README and the security policy are worded to be true both ways.
