# The patch release 0.2.3: the plan

Status: **the plan for the patch release `0.2.3` (cycle 2 of [`backlog-iterations.md`](backlog-iterations.md)) and, later, the record of its evidence. Position on 2026-10-22: the first candidate, `0.2.3-rc.1`, is CUT by the pull request that writes this page, and it is NOT tagged, NOT published and has no evidence.** It is the second candidate made in the weekly window of [`release.md`](../operations/release.md) 6.18 and the second made on a release branch: the maintainer's runbook for it is `release.md` 6.21. `0.2.2` stays npm `latest` and `0.2.2-rc.1` stays `next` until the candidate is published; nothing is announced.

## What this release is, and why

`0.2.2` was published on 2026-10-18 (cycle 1). The maintainer's replan of 2026-10-21 ([ADR 0027](../decisions/0027-the-road-to-1-0.md), the first section of [`backlog-iterations.md`](backlog-iterations.md)) makes `0.2.3` **cycle 2, all of it**: the real-media test set, the follow-ups to the seams measured on real speech, the Windows private folders, the whisper.cpp re-pin and the dependency upkeep, cut at midnight on Friday 23 October (the maintainer's local time) and published about 26 to 27 October. **It is a patch release, not a work packet:** P00 to P14 stay complete, the claims rung stays `candidate`, and nothing here is a statement of support. **`0.3.0` work is not on `main`:** its pull requests stay drafts until the supervisor opens `main` after this cut.

## Where things stand (2026-10-22)

| Version or branch | State |
| --- | --- |
| `0.2.2` | The current release (npm `latest`, GitHub's latest release, tag `v0.2.2` at `0dadc69df4f5`, published 2026-10-18). Has every defect listed under "What is in it" below. |
| `0.2.2-rc.1` | npm `next`; deprecated since 2026-10-18 ("Superseded by 0.2.2. Install vsift-cli."). |
| `main` | Carries `0.2.3-rc.1` from the merge of the pull request that cuts it (the version is a commit's, not a publish's). |
| `release/0.2.x` | **Exists, at `0dadc69df4f5` (the stable `0.2.2`), and is deleted and made again by the cut** (6.18 step 2.3, with the `Release window` check run first: its records are on `main`). |
| `0.2.3-rc.1` | **Cut, not tagged, not published.** The tag `v0.2.3-rc.1` is made on the new `release/0.2.x` at the same commit (6.21 step 4); the publish waits for the maintainer's approval of the `release` environment (6.21 step 6). |
| `0.2.3` | Not made. The target is Monday or Tuesday after the cut (6.18 step 5), about 26 to 27 October. |
| `0.2.4` | Not part of this window: a hot fix from `release/0.2.x` in early November (the FFmpeg update), after `0.2.3` is published. |

## What is in it

**Everything below is on `main` at the cut and in `0.2.3-rc.1`, and in no published version.** The `[0.2.3-rc.1]` section of the [changelog](../../CHANGELOG.md) has every change in detail; against the tag `v0.2.2` the cut's tree differs in 115 files under `crates/`, 8 under `schemas/v1/`, 70 under `tools/`, 23 under `.github/`, 2 under `fuzz/` and 55 under `fixtures/`; **nothing under `skills/vsift/` or `npm/` changed** (so the agent-trial freeze is unchanged in substance), and the lockfile gains one workspace crate (`vsift-windows-security`) and moves five third-party crates (below).

**The program (what ships):**

| Issue or story | Pull request | What it changes | What is left |
| --- | --- | --- | --- |
| [#469](https://github.com/smormah/vsift/issues/469) | #478 | on Windows providers run in their own folder, so a long per-user folder or session root no longer fails every media command as `MISSING_CAPABILITY` | an FFmpeg older than 6.0, not tried, may not open a long path ([L-193](known-limits.md#l-193)) |
| [#443](https://github.com/smormah/vsift/issues/443) with #206 | #473 | on Windows every private folder is created with its security descriptor, in one new crate allowed `unsafe` ([ADR 0026](../decisions/0026-windows-directories-created-with-their-security-descriptor.md)) | [L-175](known-limits.md#l-175) rewritten; a root already broken stays broken until its user follows the remediation |
| [#439](https://github.com/smormah/vsift/issues/439) | #470 | a managed activation on Windows tries a refused rename again for up to two seconds | the one failure was not traced to a step ([L-182](known-limits.md#l-182)) |
| [#322](https://github.com/smormah/vsift/issues/322), [#399](https://github.com/smormah/vsift/issues/399) | #463 | whisper.cpp v1.9.2 to v1.9.5 (build b5454), the Ubuntu managed install and `setup check`'s name for the reviewed build | a managed install made earlier keeps v1.9.2 until `setup install` runs again; the build release `b5454` is a GitHub pre-release whose retention upstream does not state ([L-181](known-limits.md#l-181)) |
| the story added with the re-pin | #471 | `setup check` says when a managed version is superseded by a newer reviewed pin; a hosted run upgrades a real `0.2.2` managed install | the hosted run needs the published candidate to be fully shown |
| [#380](https://github.com/smormah/vsift/issues/380), [#382](https://github.com/smormah/vsift/issues/382) | #482 | the repeat rule at seams compares every overlapping pair and no longer drops a copy that carries time | [L-155](known-limits.md#l-155) rewritten to the repeats that are left |
| [#383](https://github.com/smormah/vsift/issues/383), [#384](https://github.com/smormah/vsift/issues/384) | #479 | every rejected recogniser segment with a word is placed, and a search's warning fits the stretch | an unplaced stretch is an upper bound ([L-158](known-limits.md#l-158), [L-159](known-limits.md#l-159)) |
| [#485](https://github.com/smormah/vsift/issues/485) | #495 | a recogniser repetition no longer hides what the next chunk heard at a seam | other loops are left ([L-201](known-limits.md#l-201)) |
| dependency upkeep | #461, #462, #464, #468 | `tokio` 1.53.3, `process-wrap` 10.0.1, `lzma-rust2` 0.22.0, `jsonschema` 0.58.6 (`handoff check` reads `\s` as ECMA-262 does), Rust 1.99.1 and MSRV 1.99 (never 1.99.0) | the open Dependabot pull requests of these crates were replaced |

**Evaluations, the real-media set, tooling, CI and records (nothing that ships changes, but part of the candidate's tree):** the real-media set and its hosted campaign (#363: #460, #465, #476, #493, #497, #494), the noisy-speech set and its noise gate (#150, #492), the visual evaluations on real recordings (#173, #175: #477, #480, #496), the seam measurements on real speech (#174 in #483, #381 in #484), LOKI, the overnight test machine, with its self-hosted runner policy ([ADR 0025](../decisions/0025-self-hosted-runner-policy.md), #472), workflow action and image pins (#466, #467), the `Release window` check's fixes (#459, #488), the malicious-media run's memory reading (#481), the records after `0.2.2` (#457) and the road to 1.0 (#498, #507).

**Not in this release:** the FFmpeg re-pin ([#351](https://github.com/smormah/vsift/issues/351), [#272](https://github.com/smormah/vsift/issues/272)), which needs the month-end build of 2026-10-31 and goes to the hot fix `0.2.4` (ADR 0027), and everything of `0.3.0` (OCR, speaker labels, online links, time anchoring, the language of each chunk, search across kept sessions). **The skill did not change in the cycle.**

## What is known not fixed

- **The candidate's fixes are shown by tests and, for some, hosted runs on `main`, not on the published bytes.** #469 is shown by tests with long paths on Windows; #443 by an instrumented race test on one Windows 11 desktop and a loaded run; the seam changes by hosted runs on real speech (RM06, RM09, RM10) and unit tests; the whisper.cpp re-pin by the hosted upgrade run and the managed-install checks.
- **The real-media set is partial:** the hosted evidence covers the items in the test-media release at the time of each run ([L-180](known-limits.md#l-180)); every figure it gives measures a property on a handful of recordings, not a rate.
- **Findings filed in the cycle and not fixed:** [#206](https://github.com/smormah/vsift/issues/206) (the root's permissions, of which #443 was a state), [#397](https://github.com/smormah/vsift/issues/397) and [#336](https://github.com/smormah/vsift/issues/336) (trial design), and [#454](https://github.com/smormah/vsift/issues/454) ([L-178](known-limits.md#l-178): without the skill VSift's help does not lead a compact model to cite ids; cycle 3's story). The same binary recognises slightly differently on another CPU ([L-200](known-limits.md#l-200); the exact comparison waits for LOKI).
- **Waiting for the maintainer, unchanged by this cut:** [#188](https://github.com/smormah/vsift/issues/188) (RQ-14, required for `1.0.0`), [#176](https://github.com/smormah/vsift/issues/176), [#246](https://github.com/smormah/vsift/issues/246), [#374](https://github.com/smormah/vsift/issues/374), moving `next` ([L-108](known-limits.md#l-108)) and the folder answer that differs by platform.

## The window plan for 0.2.3-rc.1

The steps and their exact command lines are [`release.md`](../operations/release.md) 6.21. In order:

1. **The cut (midnight on Friday 23 October, the maintainer's local time):** the supervisor merges the cut's pull request to `main` before the night's nightly, waits for its `CI result` on the merge commit, dispatches the Nightly workflow on `main` while the merge commit is `main`'s head (or, if `main` has moved, on the new `release/0.2.x`) and waits for it to pass; then, **after the `Release window` check with `records=true`**, the maintainer deletes the old `release/0.2.x`, creates it at the merge commit and tags `v0.2.3-rc.1` there (annotated, `VSift 0.2.3-rc.1`).
2. **The dry run on the tag** (`guard` answers within a minute), then **the publish on the maintainer's approval** of the `release` environment. The publish moves `next` from `0.2.2-rc.1` to `0.2.3-rc.1` on all four packages; `latest` stays `0.2.2` and GitHub's latest release stays `v0.2.2`. The supervisor never approves the environment and runs no npm write.
3. **The quick checks of the published bytes,** right after the publish: `P14 verify release` (RQ-19), `P14 journeys` (RQ-05), `P13 managed smoke` (RQ-06) and `P14 published artifacts` from `0.2.2` and then from `0.2.2-rc.1` (RQ-01 to RQ-04), one after another.
4. **The long runs** (6.18 step 3). `crates/`, `Cargo.lock` and the workflows changed, so every item with them in scope needs a run on the tag. **This is the cycle that needs the monthly round** (a re-pin is a major change, [`backlog-iterations.md`](backlog-iterations.md) "How each kind of change is tested"): `P07 local ASR` (accuracy, the noisy-speech gate included), `P14 malicious media` (RQ-10), the dated `P14 scan reading` (RQ-13) and the full durability campaign (`P13 managed power loss` and `P10 durability campaign`, RQ-11), with `P14 compatibility`, `P14 local upgrade`, `Fuzz` (`seconds=3600`), `P14 stress`, `P14 load` and `P14 runbook walk`. All on the tag, staggered, hosted; **nothing runs on the maintainer's machine** (no load, no stress, no real-media transcription). `P14 real media` may be dispatched once on the tag as an advisory reading; it is not an evidence item. A failed run is a finding: an issue first, then a targeted fix on the release branch and a rerun of only that workflow (6.18 step 4).
5. **Agent-trial batches 2 and 3 are the maintainer's decision at the window.** The skill is unchanged, so the freeze of 2026-10-19 (`302930904439...`) is kept byte for byte and the cut moved the `0.2.2-rc.1` batches' records aside to `p14-agent-trials/batch-2-0.2.2-rc.1/` and `batch-3-0.2.2-rc.1/`. RQ-15 is `passed` for `0.2.2-rc.1` and stale for this candidate; RQ-16 is `planned` (L-179). Either run the batches on the published `0.2.3-rc.1` (`-Version 0.2.3-rc.1`, one at a time, on the maintainer's go and allowances, batch 3 with `-AllowGraderChange`; [`p14-batch-2-3-checklist.md`](p14-batch-2-3-checklist.md)) or decide a waiver for `0.2.3` (the maintainer's decision, recorded in the ledger, never the supervisor's).
6. **The records,** by a records pull request to `release/0.2.x` (the stable plan reads the ledger at the stable commit, 6.8), from what each run computed and never earlier; then the stable commit `0.2.3` on the branch (Monday or Tuesday is the target, 6.18 step 5), and after the publish the records change to `main`.

## The evidence items and their state (at the cut)

`vsift-governance release-evidence --complete-for 0.2.3-rc.1` is expected to fail until the runs above are recorded; at the cut it names 17 of the 20 items (the output is in the cut's pull request and in 6.21).

| Item | State in the ledger at the cut | What records it for `0.2.3-rc.1` |
| --- | --- | --- |
| RQ-01 to RQ-04 | `passed` for `0.2.2`, stale (118 to 129 files of their scopes changed) | `P14 published artifacts` twice, `P14 compatibility`, `P14 local upgrade` on the tag |
| RQ-05 | `passed` for `0.2.2`, stale | `P14 journeys` on the published candidate and `P07 local ASR` on the tag |
| RQ-06 | `passed` for `0.2.2`, stale | `P13 managed smoke` |
| RQ-07 to RQ-12 | `passed` for `0.2.2-rc.1`, stale | `Fuzz`; `P14 stress`; `P14 load`; `P14 malicious media` with `version=0.2.3-rc.1`; `P13 managed power loss` and `P10 durability campaign`; `P14 runbook walk` |
| RQ-13 | `passed` for `0.2.2`, stale | `P14 scan reading` and a dated reading by hand |
| RQ-14 | `waived` (the strict worker's adversarial evidence, a standing R0 waiver, ADR 0024 decision E; carried to `0.2.2` on 2026-10-18) | **a decision of the maintainer:** the completeness check passes a waived item for any version, so whether it carries to `0.2.3` must be decided, not inferred ([#188](https://github.com/smormah/vsift/issues/188) is required for `1.0.0`) |
| RQ-15 | `passed` for `0.2.2-rc.1`, stale | agent-trial batch 2 on the published candidate, or a decision |
| **RQ-16** | **`planned`** (the waiver for `0.2.2` moved into `prior`, as [L-179](known-limits.md#l-179) says) | agent-trial batch 3 on the published candidate, or a decision |
| RQ-17 | `waived` (no Smart App Control, clean-machine or Mac try-out; ships untried; carried to `0.2.2` on 2026-10-18) | **a decision of the maintainer**, as for RQ-14, or the try-outs ([`rq-17-tryout-sheet.md`](rq-17-tryout-sheet.md)) |
| RQ-18 | `passed` for `0.2.2`, stale | the Governance job of `CI` at the candidate's commit |
| RQ-19 | `passed` for `0.2.2`, stale | `P14 verify release` |
| RQ-20 | `planned` | the completeness check itself |

**Why only RQ-16 was set to `planned`.** A `waived` item names no version, so the check passes it for every version ([L-179](known-limits.md#l-179)). Of the three waived items, RQ-16's waiver (the Claude compact cold usefulness on `0.2.2-rc.1`, issue 454) was decided for `0.2.2` only and its run belongs to this window, so it is `planned`, with the waiver kept in `prior`. RQ-14 and RQ-17 are standing waivers that no run in the window can replace; they stay `waived` and are listed for the maintainer. **A cost of this choice:** an item holds one status, so `--complete-for 0.2.2` now also names RQ-16.

## Open decisions for this candidate

- **RQ-14 and RQ-17 for `0.2.3`:** carry the standing waivers or not (the check will not ask). RQ-14 is required for `1.0.0`.
- **RQ-15 and RQ-16:** run batches 2 and 3 on the published candidate (about the cost of the `0.2.2-rc.1` round), or waive for `0.2.3`.
- **Deprecation:** whether `0.2.2-rc.1` (`next`, already deprecated) stays `next` until the publish moves it, and whether `next` is moved by hand after `0.2.3` ([L-108](known-limits.md#l-108)).
- **The upgrade baselines:** `0.2.2` (the upgrade a person on `latest` does) and `0.2.2-rc.1` (a person on `next`) are planned; `0.2.1`, `0.2.0` and `0.1.0` are optional. A managed install made by `0.2.2` keeps whisper.cpp v1.9.2 until `setup install` runs again, which the upgrade run shows.

## What is weaker than it sounds

- **The coarse scopes:** `crates` is in the scope of most items, so "the long runs this week's changes touched" is nearly every long run; the window's cost is the cost of a full round for this candidate, and it is the monthly round besides.
- **The Windows fixes of the cycle (#469, #443, #439) are shown on one desktop and by tests;** hosted Windows is one run per workflow, not a rate.
- **The real-media evidence is a handful of recordings with hand-made labels** and, for the seams, one recogniser on one CPU model per hosted run ([L-200](known-limits.md#l-200)).
- **The release workflow's second use:** the `guard`, the re-check and the 20-minute read-back ran once against the real services (`0.2.2-rc.1` and `0.2.2`); this is the first window with an existing release branch to delete and make again.
- **L-133's window reopens for repository pages** that name the candidate before it is published (this page, the guide's first page, the changelog, the trial documents, the runbook and the register); the README, the installation guide, the launcher's README and the security policy are worded to be true both before and after the publish.
