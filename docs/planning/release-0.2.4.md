# The hot fix 0.2.4: the plan

Status: **the plan for the hot fix `0.2.4` of `0.2.3` and the record of its evidence. Position on 2026-11-04: the fix (#571) is merged on `release/0.2.x` as `c2fb889e5f765c4bb9df79b6a5a4d469a2448973` and its forward-port (#573) on `main`; the maintainer gave the go on 2026-11-03 and accepted the FFmpeg build on 2026-11-04; the first candidate, `0.2.4-rc.1`, is being cut by the pull request "The release candidate 0.2.4-rc.1" into `release/0.2.x`. It is not tagged or published, and no evidence is recorded for it yet.** The maintainer's runbook for it is [`release.md`](../operations/release.md) 6.22. `0.2.3` stays npm `latest` until the stable `0.2.4` is published; nothing is announced.

## What this release is, and why

`0.2.3` was published on 2026-10-25 with the FFmpeg that `0.2.2` already pinned: BtbN's month-end build of 2026-08-31. FFmpeg has published memory-safety fixes since, in code VSift runs (the VP8, VP9, H.264, Opus and AAC decoders, the MP4 key atom and a Matroska duration overflow). The road to 1.0 ([ADR 0027](../decisions/0027-the-road-to-1-0.md)) therefore made the re-pin a **hot fix**, `0.2.4`, from `release/0.2.x`, outside the weekly window. **It is a patch release, not a work packet:** P00 to P14 stay complete, the claims rung stays `candidate`, and nothing here is a statement of support.

## Where things stand (2026-11-04, before the cut merges)

| Version or branch | State |
| --- | --- |
| `0.2.3` | The current release (npm `latest` on all four packages, GitHub's latest release `v0.2.3`, tag `v0.2.3` at `fda35b68a960`). Checked on 2026-11-04. |
| `0.2.3-rc.1` | npm `next` on all four packages (tag `v0.2.3-rc.1` at `09e40398894f`). Checked on 2026-11-04. |
| `release/0.2.x` | At `c2fb889e5f76` (the merge of #571) before the cut. The cut is a pull request into it; the branch is **not** deleted or made again (a hot fix is made on the branch of the version it fixes, `release.md` 6.18). |
| `0.2.4-rc.1` | **Being cut.** On npm it answers 404; there is no `v0.2.4*` tag. |
| `main` | Carries the same pin and records (#573, with the Windows catalogue) and goes on with `0.3.0`. |

## What is in it

**On `release/0.2.x` as #571, and in `0.2.4-rc.1`:**

- **The FFmpeg re-pin ([#272](https://github.com/smormah/vsift/issues/272), [L-132](known-limits.md#l-132)):** a new managed install on Ubuntu gets `ffmpeg-n9.0.2-39-g7adfbe40dc-linux64-lgpl-9.0.tar.xz` of `autobuild-2026-10-31-12-59` (136,027,168 bytes, catalogue revision `ubuntu-24.04-x86_64-2026-11-03-r4`), on the **9.0 line** (the maintainer's choice of 2026-11-03). A machine set up by `0.2.3` or earlier keeps its FFmpeg until `setup install` runs again; `setup check` says it is older than the reviewed one.
- **The build of 2026-08-31 is marked superseded, with the reason `security_fixes`.**
- **Two reviewed bounds raised, each only as far as that archive needs:** `MAX_XZ_ARCHIVE_BYTES` from 128 MiB to 130 MiB and the tar stream cap from 400,000,000 to 450,000,000.
- **The argument test** `crates/vsift-infrastructure/tests/ffmpeg_input_arguments.rs`: the forced `-protocol_whitelist file` and `-f mov` or `-f matroska` before `-i` on every FFmpeg and FFprobe command.
- **Three hostile cases** for `P14 malicious media`: `mp4-pssh-200000`, `mkv-fourcc-huffyuv` and `mkv-fourcc-g2m4`.
- **The Node.js pins of CI and the publish job ([#526](https://github.com/smormah/vsift/issues/526))** moved to the security releases of 2026-10-20; L-232 is deleted.
- **[L-227](known-limits.md#l-227):** five FFmpeg records of 2026-11-01 that the pinned build does not fix (two of their codecs can be named by a file VSift ingests).

**What the cut adds:** the version strings, this page, `release.md` 6.22, the changelog, the guide's version marker and generated reference pages, the ledger (RQ-08, RQ-15 and RQ-16 set to `planned`, below) and the work record. No code, dependency, workflow or skill change.

**Not in this release:**

- [#351](https://github.com/smormah/vsift/issues/351) and [#272](https://github.com/smormah/vsift/issues/272) stay open: upstream's fix for CVE-2026-107678 (an MP4 with very many `pssh` boxes) is on FFmpeg's development line only ([L-144](known-limits.md#l-144)).
- L-144 is re-checked at the November month-end build, for `0.3.0`.
- The Windows catalogue is `main`'s alone (#573): `0.2.4` re-pins Ubuntu only.
- Anything of `0.3.0`.

## The route: a hot fix

By `release.md` 6.18 ("A hot fix"): the fix is on the branch (#571), the full check on all three systems on the branch's head is the nightly dispatched on the branch (about 30 minutes), then a candidate and the stable release by 6.3 and 6.7, the publish on the maintainer's approval, and the quick checks of the published bytes. **There is no window, no long run and no agent batch before it**; the long runs run in the next window, and anything they find is an issue first. The exact steps are 6.22. The skill is unchanged and the agent-trial freeze is kept byte for byte, so no trial records are moved aside.

The runs that are made on `0.2.4-rc.1`, from the close-out of the fix's session:

1. **On the candidate's head, before the tag:** the nightly dispatched on `release/0.2.x`.
2. **Right after the publish (the quick checks of the published bytes):** `P14 verify release` (RQ-19), `P14 journeys` (RQ-05), `P13 managed smoke` (RQ-06) and `P14 published artifacts` from `0.2.3` and then from `0.2.3-rc.1` (RQ-01 to RQ-04).
3. **On the tag, and only these:** `P14 malicious media` (RQ-10, with its new `pssh`, Huffyuv and G2M4 cases), `P07 local ASR` and `P14 scan reading` with the dated reading by hand (RQ-13; the cycle's dependency change is FFmpeg `n9.0.1-11` to `n9.0.2-39` and the Node.js pins).
4. **After the stable is published:** the quick checks of the published bytes again (`P14 verify release` within seven days, RQ-19; `P14 journeys`; `P13 managed smoke`; `P14 published artifacts`).
5. **On `main`:** the scheduled `P07 local ASR` round of Monday 9 November is the first speech-recognition accuracy check of the new Windows bytes (#573).

## The evidence reading at the cut

`release-evidence --complete-for 0.2.4-rc.1 --commit c2fb889e5f765c4bb9df79b6a5a4d469a2448973` (run on 2026-11-04 in the cut's worktree, again after the ledger change below; **a reading, not a gate: it fails until the runs are recorded**) names **18 of the 20 items**: RQ-01 to RQ-07, RQ-09 to RQ-13, RQ-18 and RQ-19, which are `passed` for `0.2.3-rc.1` with 13 to 69 files of their scope changed since (the diff `v0.2.3-rc.1` to `c2fb889e` has 69 files outside the trial records; the Node.js pins touch 18 workflows, which most scopes contain), and **RQ-08, RQ-15 and RQ-16, which this cut sets to `planned`** (below). The first reading, before that change, named 15.

It does **not** name RQ-14 and RQ-17, which are `waived` and which the maintainer said to carry, with no version limit in the text, nor RQ-20 (the check itself).

**What the cut does to the ledger.** RQ-08, RQ-15 and RQ-16 were `waived` for `0.2.3` by the maintainer's decision of 2026-10-25, and each waiver's own text covers `0.2.3` and nothing later. A waived item names no version, so the check would have passed all three for `0.2.4` without anyone deciding ([L-179](known-limits.md#l-179)). As the cut of `0.2.3-rc.1` did for RQ-16, the cut sets each to `planned`, moves the decision into the item's `prior` as a record ("NOT COUNTED for 0.2.4-rc.1") and dates the item 2026-11-04. That carries out the maintainer's decision; it does not rewrite it. An item holds one status, so `--complete-for 0.2.3` now also names these three until the evidence for `0.2.4` replaces them.

For each named item: does the re-pin touch what it proves; what this plan does; who decides. "The re-pin touches it" is read from the item's scope in the ledger and from the header of the workflow that makes the run (read on 2026-11-04), not from a run. The plan does one of three things: **run on `0.2.4-rc.1`**, **run in the next window** or **carry**. **For an item whose scope the fix touches and that needs a long run, 6.18 says the stable release needs "that run or a recorded waiver by the maintainer", so "in the next window" is a waiver for `0.2.4`, not a delay: each such row reads "before the stable release 0.2.4: the run, or the maintainer's waiver for 0.2.4 (proposed: waive, and run in the next window)".** Every item that needs a long run and is not among the three of the route is for the maintainer: run or waive.

| Item | Does the re-pin touch it? | What the plan does | Who decides |
| --- | --- | --- | --- |
| RQ-01 clean installs, archives, offline install | Partly: the offline install of the managed tools now fetches the new FFmpeg archive; the four package managers' installs and VSift's native archives are not changed by it. 24 files of the scope changed (Cargo.lock, `release.yml`, `p14-published-artifacts.yml`, the crates). | **Run on `0.2.4-rc.1`:** `P14 published artifacts` from `0.2.3` (a quick check, 6.22 step 7). | Nobody: the coordinator dispatches. A failure is a finding for the maintainer. |
| RQ-02 archives and attestations | No (the archives are VSift's own); the scope's files changed for other reasons (version strings, workflows). | **Run on `0.2.4-rc.1`:** the same dispatch. | As RQ-01. |
| RQ-03 clean installs with four managers | Partly, as RQ-01 (21 files changed). | **Run on `0.2.4-rc.1`:** the same dispatch. | As RQ-01. |
| RQ-04 upgrades, uninstall and compatibility | Partly: the upgrade from `0.2.3` and `0.2.3-rc.1` moves a managed install, and `setup check` now reports the older FFmpeg as superseded (32 files). The record formats are not changed. | **Run on `0.2.4-rc.1`:** the upgrades inside `P14 published artifacts`, from `0.2.3` and then from `0.2.3-rc.1`. **`P14 compatibility` and `P14 local upgrade` are long runs: before the stable release 0.2.4, the run, or the maintainer's waiver for 0.2.4 (proposed: waive, and run in the next window).** | The maintainer: run or waive the two. |
| RQ-05 worker-host journeys | Yes, for the speech path: the journeys and the accuracy gates decode with the FFmpeg the run installs (24 files). | **Run on `0.2.4-rc.1`:** `P14 journeys` on the published candidate and `P07 local ASR` on the tag. | Nobody for the runs. |
| RQ-06 managed smoke | **Yes: it is the check of the new pin on the published bytes** (21 files). | **Run on `0.2.4-rc.1`:** `P13 managed smoke`. | Nobody. |
| RQ-07 fuzzing (3,600 s on each of 31 targets) | **Yes, and the most directly of the long runs: the two raised bounds (`MAX_XZ_ARCHIVE_BYTES` and the tar stream cap) sit inside the code that the `xz_tar_inventory` and `tar_inventory` targets fuzz** (22 files). | **Before the stable release 0.2.4: the run (`Fuzz` with `seconds=3600`), or the maintainer's waiver for 0.2.4 (proposed: waive, and run in the next window).** | The maintainer: run or waive. |
| RQ-08 stress | No: its workflow uses no FFmpeg; the scope's files changed for other reasons. `planned` by this cut; the waiver for `0.2.3` (#519, a known issue of that release) is in `prior`. | A long run the re-pin does not touch: **run in the next window, or waive.** | The maintainer. |
| RQ-09 load | Yes: the run installs the reviewed FFmpeg from the catalogue (its header says so) (20 files). | **Before the stable release 0.2.4: the run, or the maintainer's waiver for 0.2.4 (proposed: waive, and run in the next window).** | The maintainer: run or waive. |
| RQ-10 malicious media | **Yes: the archive bounds and the hostile cases are the change** (21 files). | **Run on `0.2.4-rc.1`:** `P14 malicious media`, the 96 inputs of `0.2.3-rc.1` plus the three new cases. | Nobody for the run. A finding is the maintainer's. |
| RQ-11 durability (`P13 managed power loss`, `P10 durability campaign`) | The power-loss run's managed workload installs the managed tools, now a larger FFmpeg archive; the durability campaign does not use FFmpeg (19 files, mostly the `setup` crates). | **Before the stable release 0.2.4: the run, or the maintainer's waiver for 0.2.4 (proposed: waive, and run in the next window).** | The maintainer: run or waive. |
| RQ-12 runbook walk | Yes: the walk installs the tools from their publishers as `P13 managed smoke` does (20 files). | **Before the stable release 0.2.4: the run, or the maintainer's waiver for 0.2.4 (proposed: waive, and run in the next window).** | The maintainer: run or waive. |
| RQ-13 scan reading | Yes: the dated reading is of the cycle's dependency change, FFmpeg and the Node.js pins (69 files). | **Run on `0.2.4-rc.1`:** `P14 scan reading` on the tag, and the dated reading by hand. | The maintainer accepts the reading. |
| RQ-15 agent-trial batch 2 | No: the skill is unchanged (nothing under `skills/vsift/` differs from `v0.2.3`) and the freeze of 2026-10-19 is kept. `planned` by this cut; the waiver for `0.2.3` is in `prior`. | A hot fix runs no agent batches (6.18), so **it can only be waived for 0.2.4.** | The maintainer: waive for 0.2.4 or not. |
| RQ-16 agent-trial batch 3 | No, as RQ-15. `planned` by this cut; the waiver for `0.2.3` is in `prior`. | As RQ-15: **it can only be waived for 0.2.4.** | The maintainer: waive for 0.2.4 or not. |
| RQ-18 claims and documents | The documents changed (the guide, the install guide, the register, the changelog; 13 files). Not the FFmpeg. | **Run on `0.2.4-rc.1`:** the Governance job of `CI` at the candidate's merge commit. | Nobody. |
| RQ-19 verify release | No; it reads the publish. The `0.2.3` publish-plan artifact has expired and was ruled non-blocking for #571; the `0.2.4-rc.1` publish makes a new one, and **RQ-19 for this candidate is read from that** (24 files). | **Run on `0.2.4-rc.1`:** `P14 verify release` right after the publish. | Nobody. |

**Standing waivers the check does not name, for the maintainer:**

| Item | State and what the re-pin does to it | Who decides |
| --- | --- | --- |
| RQ-14 strict worker's adversarial evidence | The standing waiver ([#188](https://github.com/smormah/vsift/issues/188), required for `1.0.0`). Untouched by the re-pin. The maintainer's word, "carry", has no version limit. | The maintainer. |
| RQ-17 try-outs (Smart App Control, clean machine, Mac) | The standing waiver, carried the same way. Untouched by the re-pin. | The maintainer. |

## What is known not fixed

- **#351 stays open.** A file with very many `pssh` boxes is not fixed by the 9.0 line (L-144); the hostile case `mp4-pssh-200000` shows how VSift's bounds answer it.
- **Five FFmpeg records of 2026-11-01** the pinned build does not fix (L-227).
- **A machine set up earlier keeps its FFmpeg** until `setup install` runs again (L-132).
- **The fix is shown by tests and hosted runs on the branch** (`P06 Ubuntu candidate smoke`, `P13 managed smoke` and `P07 local ASR` passed on the hot-fix branch), **not yet on the published bytes.**
- **Waiting for the maintainer, unchanged by this cut:** [#188](https://github.com/smormah/vsift/issues/188), [#176](https://github.com/smormah/vsift/issues/176), [#246](https://github.com/smormah/vsift/issues/246), [#374](https://github.com/smormah/vsift/issues/374).

## The evidence on 0.2.4-rc.1

Not yet recorded. The records pull request into `release/0.2.x` adds this section after the runs of the route, from what each run computed and never earlier (6.22 step 9).
