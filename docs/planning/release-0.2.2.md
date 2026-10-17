# The patch release 0.2.2: the plan

Status: **the plan for the patch release `0.2.2` (cycle 1 of [`backlog-iterations.md`](backlog-iterations.md)) and, later, the record of its evidence. Position on 2026-10-17: the first candidate, `0.2.2-rc.1`, is CUT by the pull request that writes this page, and it is NOT tagged, NOT published and has no evidence.** It is the first candidate made in the weekly window of [`release.md`](../operations/release.md) 6.18 and the first on a release branch: the maintainer's runbook for it is `release.md` 6.19. `0.2.1` stays npm `latest` and `0.2.1-rc.3` stays `next` until the candidate is published; nothing is announced.

## What this release is, and why

`0.2.1` was published on 2026-10-15. Its own hosted checks passed except one: the worker's shutdown stage of the journeys failed twice on hosted Windows ([#404](https://github.com/smormah/vsift/issues/404), [L-167](known-limits.md#l-167)), and the maintainer decided on 2026-10-16 that it is a known issue of `0.2.1` and the first fix of `0.2.2`. The same day the maintainer set the delivery model: four cycles, of which cycle 1 is `0.2.2`, "the fixes found during `0.2.1`, with the test review folded in" ([`delivery-governance.md`](delivery-governance.md), "Delivery cycles and the test and release model"). **It is a patch release, not a work packet:** P00 to P14 stay complete, the claims rung stays `candidate`, and nothing here is a statement of support.

## Where things stand (2026-10-17)

| Version or branch | State |
| --- | --- |
| `0.2.1` | The current release (npm `latest`, GitHub's latest release, tag `v0.2.1` at `e9ab6e626733`). Has every defect listed under "What is in it" below. |
| `0.2.1-rc.3` | npm `next`; not deprecated. |
| `main` | Carries `0.2.2-rc.1` from the merge of the pull request that cuts it (the version is a commit's, not a publish's). |
| `release/0.2.x` | **Does not exist yet.** The cut creates it at the merge commit of that pull request once the commit's `CI` has passed and its nightly has passed (6.19 steps 1 to 3). It is the first release branch of the project. |
| `0.2.2-rc.1` | **Cut, not tagged, not published.** The tag `v0.2.2-rc.1` is made on `release/0.2.x` at the same commit (6.19 step 4); the publish waits for the maintainer's approval of the `release` environment (6.19 step 6). |
| `0.2.2` | Not made. The target in the window is Monday or Tuesday after the cut, when the candidate's evidence is complete (6.18 step 5). |

## What is in it

**Everything below is on `main` at the cut (`f50ae6b7`, the merge of #452) and in `0.2.2-rc.1`, and in no published version.** The `[0.2.2-rc.1]` section of the [changelog](../../CHANGELOG.md) has every change in detail; against the tag `v0.2.1` the cut's tree differs in 125 files under `crates/` (63 of them source files), 18 under `schemas/v1/`, 4 under `skills/vsift/`, 22 under `tools/`, 17 under `.github/`, 8 under `fuzz/` and 5 under `npm/`, and the lockfile gains one crate (`fs4` 1.1.0, Windows only). Nothing under `fixtures/` changed.

**The program (what ships):**

| Issue | Pull request (merge) | What it changes | What is left |
| --- | --- | --- | --- |
| [#404](https://github.com/smormah/vsift/issues/404) | #432 (`28cf6f8`) | a command or step stopped between two provider calls answers `CANCELLED`; a worker step that fails after the host stopped it is recorded `cancelled`; Windows providers run in a console of their own; the P10 and P11 checkpoints tell processes apart by creation time | not yet run on hosted Windows; the journeys on this candidate are the evidence ([L-167](known-limits.md#l-167)) |
| [#362](https://github.com/smormah/vsift/issues/362) | #447 (`14c02a5f`) | a worker `retranscribe` step with unusable chunks is `partial` with the direct command's `coverage` | `0.2.1` reads such a record as damaged (a roll back only) |
| [#361](https://github.com/smormah/vsift/issues/361) | #442 (`e5c2774`) | the recogniser's identity is read after every answer, before it is checkpointed | a swap undone inside one chunk ([L-146](known-limits.md#l-146)) |
| [#334](https://github.com/smormah/vsift/issues/334) | #437 (`72b8b2d`) | a short `audio` range of a source without video is the range | the guard's rounding ([L-141](known-limits.md#l-141)) |
| [#394](https://github.com/smormah/vsift/issues/394) | #445 (`ee4039d9`) | a saved plan's `error`, `coverage` and `lifecycle` decode only as `null`; a fuzz seed of the crash's shape | the float-parsing choice ([L-176](known-limits.md#l-176)); [L-162](known-limits.md#l-162) until this candidate's fuzz run |
| [#266](https://github.com/smormah/vsift/issues/266) on Windows | #450 (`56a2eaaf`) | the room check and a workspace's 1 GiB reserve read the volume's free bytes on Windows (`fs4`) | [L-061](known-limits.md#l-061) |
| [#325](https://github.com/smormah/vsift/issues/325), first step | #430 (`6a0ebf5`) | a copy that clearly cannot fit its ten minutes is stopped early | the rest of #325 needs decisions ([L-140](known-limits.md#l-140)) |
| [#264](https://github.com/smormah/vsift/issues/264) | #427 (`1fdaa88`) | a named pipe, a device or a socket is refused without being opened, with a remediation | codes unchanged ([L-127](known-limits.md#l-127)) |
| [#232](https://github.com/smormah/vsift/issues/232) | #434 (`46005eb`) | a refused `--session-root` name says which rule it broke | code unchanged (`STORAGE_IO`) |
| [#370](https://github.com/smormah/vsift/issues/370) | #428 (`e2d8326`) | `frame get` and the evidence commands refused during a transcription name the job | the refusal stays, revisited in R1 ([L-154](known-limits.md#l-154)) |
| [#371](https://github.com/smormah/vsift/issues/371) | #425 (`92189b4`) | `session list` and `session clean` pages hold whole buckets up to 256 sessions | none recorded |
| [#366](https://github.com/smormah/vsift/issues/366) | #431 (`175b7cc`) | `handoff check` reads 128 KiB; the skill checks a long report whole from PowerShell on Windows and in parts in Git Bash | no agent has run it ([L-151](known-limits.md#l-151)) |
| [#367](https://github.com/smormah/vsift/issues/367) | #435 (`f0355b16`) | `candidates --omit periodic` (opt-in) and the skill's use of it | the first call's silence and the image budget ([L-152](known-limits.md#l-152)) |
| [#396](https://github.com/smormah/vsift/issues/396), [#395](https://github.com/smormah/vsift/issues/395) | #420 (`545db9f`) | the advice for a missing tool asks for no folder search; the skill's "search first" is one of the rules that always apply | no agent has run either ([L-164](known-limits.md#l-164), [L-163](known-limits.md#l-163), [L-171](known-limits.md#l-171)) |
| [#419](https://github.com/smormah/vsift/issues/419) | #446 (`59e84e8`) | a provider's start runs off the async runtime and its deadline starts once it runs | the start's cost itself, deferred to R1 ([L-170](known-limits.md#l-170)) |
| [#433](https://github.com/smormah/vsift/issues/433) | #452 (`f50ae6b7`) | a worker's `candidates` call stopped by busy admission is the step's `BUSY`, retried within the admission wait | none recorded |
| [#436](https://github.com/smormah/vsift/issues/436) | #449 (`824dbac`) | on macOS a refused kill of a provider group that had already ended is decided by the reap | [L-177](known-limits.md#l-177) |

**Documents, tests, CI and the release workflow (nothing that ships changes, but part of the candidate's tree):** #257 (the `vsift.cmd` shim documented, no package change can remove it; [L-109](known-limits.md#l-109) accepted; #415), #277 (when an abandoned registration is cleaned, #438), #274 (the job-path regression test and the load campaign's three clips, #426), #265 (RQ-10's rule and the campaign's judge, #448), the test review (#128 in #421, #253 in #441, #440 in #451, #312 in #444, #423 in #429), the CI lanes and the nightly (#414, #416), and the release workflow for the window: release branches, the `guard` job and the nightly precondition, a read-back of 20 minutes (#337 and #403 in #422, the scanning of window fixes and the `Release window` check in #424). **This is the first candidate built and published with that workflow.**

**Not in this release:** the FFmpeg and whisper.cpp re-pins ([#351](https://github.com/smormah/vsift/issues/351), [#272](https://github.com/smormah/vsift/issues/272), [#322](https://github.com/smormah/vsift/issues/322), [#399](https://github.com/smormah/vsift/issues/399)), the real-media test set ([#363](https://github.com/smormah/vsift/issues/363)) and the seam follow-ups [#380](https://github.com/smormah/vsift/issues/380) to [#384](https://github.com/smormah/vsift/issues/384): all cycle 2, `0.2.3`. No dependency update other than `fs4` (the open Dependabot pull requests stay open).

## What is known not fixed

- **The candidate's fixes are shown by tests, not on the published bytes.** #404's fix has not run on hosted Windows; #266 on Windows is shown by the store's and the engine's tests (the CLI-level no-room test stays Unix only); #436 was read from the code and is not reproduced (no macOS here).
- **Open issues that this candidate touches but does not close:** #257 (an accepted residual, [L-109](known-limits.md#l-109)); #359 (the listener's check on the two recordings is still to do; its limits are #380 to #384); #366, #395 and #396 (open until an agent run shows them, which batches 2 and 3 on this candidate can); #325 (the rest of its design); #370 (the refusal stays until R1); #419 (the start's cost, R1); #403 (the read-back is 20 minutes since #422; no publish has exercised it yet, and this candidate's is the first).
- **Findings filed during the cycle and not fixed:** [#443](https://github.com/smormah/vsift/issues/443) (a session root left without its user's access entry under a race on Windows, [L-175](known-limits.md#l-175)), [#439](https://github.com/smormah/vsift/issues/439) (a managed install's rerun after a kill failed once at activation under heavy load on Windows, cause not known), [#206](https://github.com/smormah/vsift/issues/206) (the root's permissions, of which #443 is a state), [#397](https://github.com/smormah/vsift/issues/397) and [#336](https://github.com/smormah/vsift/issues/336) (trial design), [#316](https://github.com/smormah/vsift/issues/316), [#263](https://github.com/smormah/vsift/issues/263) and [#258](https://github.com/smormah/vsift/issues/258) (hosted runners).
- **Waiting for the maintainer, unchanged by this cut:** [#188](https://github.com/smormah/vsift/issues/188) (RQ-14), [#176](https://github.com/smormah/vsift/issues/176), [#246](https://github.com/smormah/vsift/issues/246), [#374](https://github.com/smormah/vsift/issues/374), moving `next` ([L-108](known-limits.md#l-108)), [L-176](known-limits.md#l-176)'s choice, and the folder answer that differs by platform.
- **[#404](https://github.com/smormah/vsift/issues/404) is closed on GitHub, and it should not be yet.** It was closed on 2026-10-17 at 04:02 UTC when pull request #424 merged; the work record and L-167 say it stays open until `0.2.2` is published with the journeys passing on Windows. Reopening it is the supervisor's or the maintainer's (this change does not touch GitHub issues).

## The window plan for 0.2.2-rc.1

The steps and their exact command lines are [`release.md`](../operations/release.md) 6.19. In order:

1. **The cut (Saturday 2026-10-17, midnight in the maintainer's local time):** the supervisor merges the cut's pull request to `main`, waits for its `CI result` on the merge commit, dispatches the Nightly workflow on `main` while the merge commit is `main`'s head (or, if `main` has moved, on `release/0.2.x` once it exists) and waits for it to pass, then creates `release/0.2.x` at the merge commit and tags `v0.2.2-rc.1` there (annotated, `VSift 0.2.2-rc.1`, as every earlier tag).
2. **The dry run on the tag** (`guard` answers within a minute: the ref, the branch's rulesets and the nightly), then **the publish on the maintainer's approval** of the `release` environment. The publish moves `next` from `0.2.1-rc.3` to `0.2.2-rc.1` on all four packages; `latest` stays `0.2.1` and GitHub's latest release stays `v0.2.1`. The supervisor never approves the environment and runs no npm write.
3. **The quick checks of the published bytes,** right after the publish: `P14 verify release` (RQ-19), `P14 journeys` (RQ-05, the evidence that #404's fix holds on hosted Windows), `P13 managed smoke` (RQ-06) and `P14 published artifacts` from `0.2.1` and then from `0.2.1-rc.3` (RQ-01 to RQ-04), one after another.
4. **The long runs this candidate's changes touch** (6.18 step 3; the ledger's scopes decide, and `crates/`, `Cargo.lock` and the workflows changed, so every item with them in scope): `P14 compatibility` and `P14 local upgrade` (RQ-04), `P07 local ASR` (RQ-05's ASR gates), `Fuzz` with `seconds=3600` (RQ-07, which #394's fix and seed touch), `P14 stress` (RQ-08: #419, #436 and #433 change the process supervisor and the admission path), `P14 load` (RQ-09), **`P14 malicious media` (RQ-10, required at this candidate by plan section 32.2: the first run judged by the rule that admits the link's documented answer)**, `P13 managed power loss` and `P10 durability campaign` (RQ-11), `P14 runbook walk` (RQ-12) and `P14 scan reading` with a dated reading by hand (RQ-13: `fs4` is new and `process-wrap` gained a feature). All on the tag, staggered, with the parameters of the runs for `0.2.1-rc.3`; the Governance job of the `CI` run at the candidate's commit is RQ-18's. A failed run is a finding: an issue first, then a targeted fix on the release branch and a rerun of only that workflow (6.18 step 4).
5. **Agent-trial batches 2 and 3 on `0.2.2-rc.1`: the maintainer decided to run them.** They run after the publish, against the registry's `0.2.2-rc.1` (`-Version 0.2.2-rc.1`), each on the maintainer's own go and allowances, one at a time, under the freeze committed by #435 (`skill` `bd205d6c...`, whole freeze `10180cd88ec994dc48079f3aed46e5343a43b79264056e7b6af206c4b85f74c0`), which this cut keeps byte for byte ([`p14-batch-2-3-checklist.md`](p14-batch-2-3-checklist.md); batch 3 with `-AllowGraderChange`). The cut moved the `0.2.1-rc.3` batches' records aside to `p14-agent-trials/batch-2-0.2.1-rc.3/` and `batch-3-0.2.1-rc.3/`, so `batch-2/` and `batch-3/` hold only that freeze. It is the first counted run of the skill's three changes of the cycle (#395, #366, #367) and of the new missing-tool advice (#396).
6. **The records,** by a records pull request to `release/0.2.x` (the stable plan reads the ledger at the stable commit, 6.8), from what each run computed and never earlier; then the stable commit `0.2.2` on the branch (Monday or Tuesday is the target, 6.18 step 5), and after the publish the records change to `main`.

## The evidence items and their state (at the cut)

`vsift-governance release-evidence --complete-for 0.2.2-rc.1` is expected to fail until the runs above are recorded; at the cut it names 17 of the 20 items (the output is in the cut's pull request and in 6.19).

| Item | State in the ledger at the cut | What records it for `0.2.2-rc.1` |
| --- | --- | --- |
| RQ-01 to RQ-04 | `passed` for `0.2.1`, stale (128 to 153 files of their scopes changed) | `P14 published artifacts` twice, `P14 compatibility`, `P14 local upgrade` on the tag |
| RQ-05 | `failed` for `0.2.1` (#404) | `P14 journeys` on the published candidate and `P07 local ASR` on the tag |
| RQ-06 | `passed` for `0.2.1`, stale | `P13 managed smoke` |
| **RQ-07** | **`planned`** (the cut moved the waiver for #394 on `0.2.1` into `prior`) | `Fuzz`, 31 targets, `seconds=3600`, on the tag |
| RQ-08, RQ-09, RQ-11, RQ-12 | `passed` for `0.2.1-rc.3`, stale | `P14 stress`; `P14 load`; `P13 managed power loss` and `P10 durability campaign`; `P14 runbook walk` |
| **RQ-10** | **`planned`** (the waiver for the link ends with `0.2.1`, plan section 32.2; its history is in `prior`) | `P14 malicious media` with `version=0.2.2-rc.1`; `passed` if the link answers the documented `STORAGE_IO` with its remediation and nothing else is found |
| RQ-13 | `passed` for `0.2.1`, stale | `P14 scan reading` and a dated reading by hand |
| RQ-14 | `waived` (the strict worker's adversarial evidence, a standing R0 waiver, ADR 0024 decision E) | **a decision of the maintainer:** the completeness check passes a waived item for any version, so whether it carries to `0.2.2` must be decided, not inferred |
| **RQ-15**, **RQ-16** | **`planned`** (the waivers of 2026-10-15 for `0.2.1` are in `prior`) | agent-trial batches 2 and 3 on the published candidate |
| RQ-17 | `waived` (no Smart App Control, clean-machine or Mac try-out; ships untried) | **a decision of the maintainer**, as for RQ-14, or the try-outs ([`rq-17-tryout-sheet.md`](rq-17-tryout-sheet.md)) |
| RQ-18 | `passed` for `0.2.1`, stale | the Governance job of `CI` at the candidate's commit |
| RQ-19 | `passed` for `0.2.1`, stale | `P14 verify release` |
| RQ-20 | `planned` | the completeness check itself |

**Why four items were set to `planned` and the others were not.** A `passed` or `failed` item names its version and commit, and the completeness check sees for itself that it is stale for the candidate (it names those 13). A `waived` item names no version, so the check passes it for every version: the waivers for `0.2.1` of RQ-07 (#394), RQ-10 (the link), RQ-15 and RQ-16 (the trial misses) would have counted for `0.2.2-rc.1` silently, although each covers `0.2.1-rc.3` and the stable cut from it and nothing later (the plan for `0.2.1`, "The waivers and the maintainer's decisions of 2026-10-15"). Their runs are part of this window, so they are `planned`, with the waivers kept in `prior`. RQ-14 and RQ-17 are standing waivers that no run in the window can replace; they are left `waived` and listed for the maintainer's decision instead. **A cost of this choice:** an item holds one status, so `--complete-for 0.2.1` now also names the four `planned` items; the record of `0.2.1`'s waivers is in each item's `prior` and in [`release-0.2.1.md`](release-0.2.1.md).

## Open decisions for this candidate

- **RQ-14 and RQ-17 for `0.2.2`:** carry the standing waivers or not (the check will not ask).
- **RQ-05:** if the journeys pass on all three systems, the item is `passed` for the candidate and #404's entry closes with `0.2.2`'s publish; if Windows fails again, it is a finding of this candidate (an issue first).
- **Deprecation:** whether `0.2.1-rc.3` is deprecated at `0.2.2`'s publish (it is `next` until this candidate's publish moves it), and whether `next` is moved by hand after `0.2.2` ([L-108](known-limits.md#l-108)).
- **The upgrade baselines:** `0.2.1` (the upgrade a person on `latest` does) and `0.2.1-rc.3` (a person on `next`) are planned; `0.1.0` and `0.2.0` are optional.

## What is weaker than it sounds

- **The coarse scopes:** `crates` is in the scope of most items, so "the long runs this week's changes touched" is nearly every long run; the window's cost is not smaller than a full round's for this candidate.
- **#404 is the reason for the release and is shown only on one Windows 11 desktop and by stand-ins;** a hosted Windows pass of the journeys on this candidate is one run, not a rate.
- **The skill changed three times in the cycle and no agent has run any of it**; batches 2 and 3 are small samples (one to three runs per scenario and model) with a text-matching grader.
- **The release workflow's new jobs have not run against the real services**: the `guard`, the re-check after the approval and the 20-minute read-back were tested with a stub `gh` and stub registries only (181 shell checks), and the release-branch rulesets were applied on 2026-10-17 and have so far refused only a branch created before its commit's `CI` had finished (a test of that day); no release branch has existed yet.
- **L-133's window reopens for repository pages** that name the candidate before it is published (this page, the guide's first page, the changelog, the skill guide, the trial documents, the runbook and the register); the README, the installation guide, the launcher's README and the security policy are worded to be true both before and after the publish.
