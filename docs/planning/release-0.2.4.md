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

**What the cut adds:** the version strings, this page, `release.md` 6.22, the changelog, the guide's version marker and generated reference pages, and the work record. No code, dependency, workflow or skill change.

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

`release-evidence --complete-for 0.2.4-rc.1 --commit c2fb889e5f765c4bb9df79b6a5a4d469a2448973` (run on 2026-11-04 in the cut's worktree; **a reading, not a gate: it fails until the runs are recorded**) names **15 of the 20 items**: RQ-01 to RQ-07, RQ-09 to RQ-13, RQ-18 and RQ-19. Each is `passed` for `0.2.3-rc.1`, with 13 to 69 files of its scope changed since (the diff `v0.2.3-rc.1` to `c2fb889e` has 69 files outside the trial records; the Node.js pins touch 18 workflows, which most scopes contain).

It does **not** name RQ-08, RQ-14, RQ-15, RQ-16 and RQ-17, which are `waived`, nor RQ-20 (the check itself). A waived item names no version, so the check passes it for any version ([L-179](known-limits.md#l-179)): **their silence is not a decision for `0.2.4`.**

For each named item: does the re-pin touch what it proves; what this plan does; who decides. "The re-pin touches it" is read from the item's scope in the ledger and from the header of the workflow that makes the run (read on 2026-11-04), not from a run. The plan does one of three things: **run on `0.2.4-rc.1`**, **run after the stable** (the next window, 6.18), or **carry** (the entry stays as it is). **Every item that needs a long run and is not among the three of the route is for the maintainer: run or waive.** The maintainer's recorded waiver or the run is needed before the stable (6.18: an item whose scope the fix touches and that needs a long run needs that run or a waiver), so "run after the stable" is the plan's proposal, not a way round that.

| Item | Does the re-pin touch it? | What the plan does | Who decides |
| --- | --- | --- | --- |
| RQ-01 clean installs, archives, offline install | Partly: the offline install of the managed tools now fetches the new FFmpeg archive; the four package managers' installs and VSift's native archives are not changed by it. 24 files of the scope changed (Cargo.lock, `release.yml`, `p14-published-artifacts.yml`, the crates). | **Run on `0.2.4-rc.1`:** `P14 published artifacts` from `0.2.3` (a quick check, 6.22 step 7). | Nobody: the coordinator dispatches. A failure is a finding for the maintainer. |
| RQ-02 archives and attestations | No (the archives are VSift's own); the scope's files changed for other reasons (version strings, workflows). | **Run on `0.2.4-rc.1`:** the same dispatch. | As RQ-01. |
| RQ-03 clean installs with four managers | Partly, as RQ-01 (21 files changed). | **Run on `0.2.4-rc.1`:** the same dispatch. | As RQ-01. |
| RQ-04 upgrades, uninstall and compatibility | Partly: the upgrade from `0.2.3` and `0.2.3-rc.1` moves a managed install, and `setup check` now reports the older FFmpeg as superseded (32 files). The record formats are not changed. | **Run on `0.2.4-rc.1`:** the upgrades inside `P14 published artifacts`, from `0.2.3` and then from `0.2.3-rc.1`. **`P14 compatibility` and `P14 local upgrade` are long runs: run after the stable (proposed).** | The maintainer: run or waive the two. |
| RQ-05 worker-host journeys | Yes, for the speech path: the journeys and the accuracy gates decode with the FFmpeg the run installs (24 files). | **Run on `0.2.4-rc.1`:** `P14 journeys` on the published candidate and `P07 local ASR` on the tag. | Nobody for the runs. |
| RQ-06 managed smoke | **Yes: it is the check of the new pin on the published bytes** (21 files). | **Run on `0.2.4-rc.1`:** `P13 managed smoke`. | Nobody. |
| RQ-07 fuzzing (3,600 s on each of 31 targets) | Yes: the `xz_tar_inventory` and `tar_inventory` targets read the inventory code whose two bounds were raised (22 files). | **Run after the stable (proposed):** `Fuzz` with `seconds=3600` in the next window. | The maintainer: run or waive. |
| RQ-09 load | Yes: the run installs the reviewed FFmpeg from the catalogue (its header says so) (20 files). | **Run after the stable (proposed).** | The maintainer: run or waive. |
| RQ-10 malicious media | **Yes: the archive bounds and the hostile cases are the change** (21 files). | **Run on `0.2.4-rc.1`:** `P14 malicious media`, the 96 inputs of `0.2.3-rc.1` plus the three new cases. | Nobody for the run. A finding is the maintainer's. |
| RQ-11 durability (`P13 managed power loss`, `P10 durability campaign`) | The power-loss run's managed workload installs the managed tools, now a larger FFmpeg archive; the durability campaign does not use FFmpeg (19 files, mostly the `setup` crates). | **Run after the stable (proposed).** | The maintainer: run or waive. |
| RQ-12 runbook walk | Yes: the walk installs the tools from their publishers as `P13 managed smoke` does (20 files). | **Run after the stable (proposed).** | The maintainer: run or waive. |
| RQ-13 scan reading | Yes: the dated reading is of the cycle's dependency change, FFmpeg and the Node.js pins (69 files). | **Run on `0.2.4-rc.1`:** `P14 scan reading` on the tag, and the dated reading by hand. | The maintainer accepts the reading. |
| RQ-18 claims and documents | The documents changed (the guide, the install guide, the register, the changelog; 13 files). Not the FFmpeg. | **Run on `0.2.4-rc.1`:** the Governance job of `CI` at the candidate's merge commit. | Nobody. |
| RQ-19 verify release | No; it reads the publish. The `0.2.3` publish-plan artifact has expired and was ruled non-blocking for #571; the `0.2.4-rc.1` publish makes a new one, and **RQ-19 for this candidate is read from that** (24 files). | **Run on `0.2.4-rc.1`:** `P14 verify release` right after the publish. | Nobody. |

**Waived entries the check does not name, for the maintainer to decide whether they carry to `0.2.4`:**

| Item | State and what the re-pin does to it | Who decides |
| --- | --- | --- |
| RQ-08 stress | `waived` for `0.2.3` (#519, a known issue). No FFmpeg in its workflow; the re-pin does not touch it. | The maintainer: carry, or run in the next window. |
| RQ-14 strict worker's adversarial evidence | The standing waiver ([#188](https://github.com/smormah/vsift/issues/188), required for `1.0.0`). Untouched by the re-pin. | The maintainer. |
| RQ-15, RQ-16 agent-trial batches 2 and 3 | `waived` for `0.2.3` ("the skill is unchanged and the batches run at the `0.3.0` cut"). The skill is unchanged here too and a hot fix runs no batches (6.18). | The maintainer: carry to `0.2.4`, or not. |
| RQ-17 try-outs (Smart App Control, clean machine, Mac) | The standing waiver. Untouched by the re-pin. | The maintainer. |

**This cut does not change the ledger.** Every status stays as it is: the 15 named items stay `passed` for `0.2.3-rc.1` and are named stale by the reading until the records pull request records `0.2.4-rc.1`'s runs; the five waived entries are not rewritten, because that would move the maintainer's decisions. The cut of `0.2.3-rc.1` set RQ-16 to `planned` so that the check would name it (L-179); whether to do the same for RQ-08, RQ-15 and RQ-16 is part of the maintainer's decision above.

## What is known not fixed

- **#351 stays open.** A file with very many `pssh` boxes is not fixed by the 9.0 line (L-144); the hostile case `mp4-pssh-200000` shows how VSift's bounds answer it.
- **Five FFmpeg records of 2026-11-01** the pinned build does not fix (L-227).
- **A machine set up earlier keeps its FFmpeg** until `setup install` runs again (L-132).
- **The fix is shown by tests and hosted runs on the branch** (`P06 Ubuntu candidate smoke`, `P13 managed smoke` and `P07 local ASR` passed on the hot-fix branch), **not yet on the published bytes.**
- **Waiting for the maintainer, unchanged by this cut:** [#188](https://github.com/smormah/vsift/issues/188), [#176](https://github.com/smormah/vsift/issues/176), [#246](https://github.com/smormah/vsift/issues/246), [#374](https://github.com/smormah/vsift/issues/374).

## The evidence on 0.2.4-rc.1

Not yet recorded. The records pull request into `release/0.2.x` adds this section after the runs of the route, from what each run computed and never earlier (6.22 step 9).
