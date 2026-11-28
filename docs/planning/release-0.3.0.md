# The minor release 0.3.0: the plan

Status: **the plan for the minor release `0.3.0` and the record of its evidence. Position on 2026-11-28: the first candidate, `0.3.0-rc.1`, was published on 2026-11-24 from the new `release/0.3.x`; its journeys found a defect ([#620](https://github.com/smormah/vsift/issues/620)), so the second, `0.3.0-rc.2`, was cut and published on 2026-11-25 and is npm `next`. Two agent-trial gates failed on it ([#629](https://github.com/smormah/vsift/issues/629), [#628](https://github.com/smormah/vsift/issues/628)), so the third candidate, `0.3.0-rc.3`, was cut into `release/0.3.x` by the pull request "The release candidate 0.3.0-rc.3" (#634, `8d8987b1`; "The third candidate", below). It is not tagged or published, a tests-only fix for [#636](https://github.com/smormah/vsift/issues/636) goes into the branch before the tag, and no evidence is recorded for any candidate yet; the runs on the first two are records (below).** The first candidate was a normal cut by [`release.md`](../operations/release.md) 6.18, the first on a new release line, `release/0.3.x` (runbook `release.md` 6.24); the second and third are 6.18 step 4 (runbooks `release.md` 6.25 and 6.26). `0.2.4` stays npm `latest` until the stable `0.3.0` is published; nothing is announced.

## What this release is, and why

`0.2.4` (the hot fix of `0.2.3`, published 2026-11-05) is the current release. The road to 1.0 ([ADR 0027](../decisions/0027-the-road-to-1-0.md), the first section of [`backlog-iterations.md`](backlog-iterations.md)) makes `0.3.0` a **normal stable release**: OCR, speaker labels, online links, time anchoring, what changed on screen, search across kept sessions and progress during long transcriptions, built in slices that each stand on their own. The maintainer decided on 2026-11-04 that it **is cut when it is ready**, with about 27 November as the latest date: every item of the readiness list of that day is done but this cut (the readiness list is the planner's record; the cut items, the Fable review's fixes H3 to H6 and the bounded settled cut H8 among them, are in the table below). **It is a minor release, not a work packet:** P00 to P14 stay complete, the claims rung stays `candidate`, and nothing here is a statement of support.

## Where things stand (2026-11-27, at the cut of the third candidate)

| Version or branch | State |
| --- | --- |
| `0.2.4` | The current release: npm `latest` on all four packages, GitHub's latest release `v0.2.4`, tag `v0.2.4` at `709de47b4650` on `release/0.2.x`. Checked on 2026-11-27. |
| `0.2.4-rc.1` | Deprecated since 2026-11-06 (tag `v0.2.4-rc.1` at `d44d71532967`); no longer `next` since the publish of `0.3.0-rc.1`. |
| `release/0.2.x` | At `709de47b4650` (the stable `0.2.4`). **It stays** for hot fixes of `0.2.x` until `0.3.0` is stable (6.24 step 3). |
| `release/0.3.x` | Created on 2026-11-24 at `89d3ea7c9845` (the cut of `0.3.0-rc.1`, #617); since then #621 (`2657e128`), #623 (`099ad615244a`), the cut of `0.3.0-rc.2` (#625, `0372d2ad7baa`), #632 (`5385b40a`), #630 (`16e2f4a8`) and the cut of `0.3.0-rc.3` (#634, `8d8987b1`). The tests-only fix for #636 goes into it by pull request before the tag (2026-11-28); no new branch (6.26). |
| `0.3.0-rc.1` | **Published on 2026-11-24** (Release run 45466267152; tag `v0.3.0-rc.1` at `89d3ea7c98452358e412b93c950b52600b1091e4`), a GitHub pre-release. Superseded by `0.3.0-rc.2`; deprecated at the stable release, not before. |
| `0.3.0-rc.2` | **Published on 2026-11-25** (Release run 45684249571; tag `v0.3.0-rc.2` at `0372d2ad7baa1213b3a64db831ad1ac9890f7d62`, tag object `2ba119ea`): npm `next` on all four packages, a GitHub pre-release. Superseded by `0.3.0-rc.3` once that is published; deprecated at the stable release, not before. Checked on 2026-11-27. |
| `0.3.0-rc.3` | **Cut** into `release/0.3.x` (#634, `8d8987b1`); **not tagged or published.** The tests-only fix for #636 goes into the branch before the tag (2026-11-28), and its merge commit is the commit to tag. |
| `main` | At `0.3.0-rc.1` since the cut of the first candidate (#617). The fixes reached it by their counterparts (#622, #619, #633, #631); the bumps to `0.3.0-rc.2` and `0.3.0-rc.3` do not (6.18 step 4), and reaches `main` with the records change after the stable release. |

## What is in it

**Everything below is on `main` at the cut and in `0.3.0-rc.1`, and in no published version.** The `[0.3.0-rc.1]` section of the [changelog](../../CHANGELOG.md) has every change in detail. Against the tag `v0.2.4` the cut's tree differs in 913 files (84 pull requests merged to `main` since the cut of `0.2.3-rc.1`, the last common point of `main` and `release/0.2.x`); the skill changed (25 files under `skills/vsift/`), the fuzzing gained eight targets (39 in all), and the lockfile's dependency changes are listed under "Security and dependencies".

**What a person gets (what ships):**

| What | Pull requests | Decision and register |
| --- | --- | --- |
| **`vsift <video>`** (explicitly `vsift watch`): a transcript, subtitles and the screen changes beside the video, files that grow while it runs, Ctrl-C that keeps the run, a rerun that continues it, the bars; `--keep <folder>` for a later search | #541, #544, #546, #554, #598 (H3, below) | [ADR 0037](../decisions/0037-vsift-video-the-action.md); L-234 to L-237, L-244 |
| **`vsift setup`, the setup door:** plain `vsift setup` checks, offers and installs on a yes; `--unattended`; a readable `setup check`; a cancelled install says cancelled | #536, #537, #538, #597 | [ADR 0036](../decisions/0036-the-setup-door.md) |
| **Jobs, the quick, watched and queued modes, and the live line:** `job start` runs a long transcription in the background and reports it in seconds; revisions published as they go and read while they run; the named moment first; a root queue that never blocks someone waiting; "no transcript yet" names the running job; the crash campaign with open revisions; the threshold and the estimate measured on real recordings (no default changed) | #508, #509, #510, #511, #512, #513, #533, #594, #603 | [ADR 0033](../decisions/0033-progress-during-long-transcriptions.md); L-219 to L-223, L-257 |
| **The bounded settled cut:** a watched run of continuous speech publishes as it goes, not only at a pause (the maintainer, 2026-11-11: "ship fix in 0.3.0") | #605 | ADR 0033 section 2, amendment of 2026-11-11; L-256 |
| **Managed installs on Windows and macOS, and the OCR and speakers groups:** optional components and the zip and bzip2/tar readers; the engine release workflow; the Windows catalogue; the macOS FFmpeg built from `n9.0.2-39`; whisper.cpp on Windows and macOS (catalogues Ubuntu `r8`, Windows `r6`, macOS `r3`); `setup --include on-screen-text` and `--include speaker-labels` on all three systems; the FFmpeg pins following the builds repository to its new owner; the OCR runner's ONNX Runtime from Microsoft's NuGet package | #504, #505, #506, #584, #585, #610, #611, #589, #609 | [ADR 0032](../decisions/0032-managed-installs-on-every-platform-and-the-engine-release-workflow.md); L-214 to L-217, L-224 to L-226, L-254 |
| **On-screen text:** `vsift ocr`, `vsift ocr search`, and on-screen text in `search` | #499 | [ADR 0030](../decisions/0030-on-screen-text.md); L-202 to L-204 |
| **Speaker labels:** `transcript speakers`, `transcript name-speaker`, `search --speaker`, labels in `transcript get` and `search` | #500, #612 (the evaluation's re-pin, tooling) | [ADR 0031](../decisions/0031-speaker-labels.md); L-205 to L-207, L-218 |
| **What changed on screen:** `vsift changes` between two times, and `--crops` | #572, #574 | [ADR 0034](../decisions/0034-what-changed-on-screen.md); L-211 |
| **Time anchoring, stages 1 and 2:** the real-world clock time on timestamped results (`--recorded-at`, the recording's metadata), clocks on the screen (`vsift clocks`), and a report's citations holding clock times to VSift's own | #501, #553, #556 | [ADR 0029](../decisions/0029-time-anchoring.md); L-210, L-213, L-245 |
| **Search across kept sessions:** `vsift bundle search` | #552 (with #554's `--keep`) | [ADR 0035](../decisions/0035-search-across-kept-sessions.md); L-212 |
| **Online links:** a direct `https://` link (`ingest --url`); your own yt-dlp and JavaScript runtime; public video pages through yt-dlp; fault tests, the guide's page, the agent asking before it adds online video, the opt-in platform qualification; a proxy named by its host name and `NO_PROXY` per redirect | #502, #557, #561, #565, #567, #568, #570, #599 | [ADR 0028](../decisions/0028-online-sources.md); L-208, L-209, L-228; [#407](https://github.com/smormah/vsift/issues/407) and [#409](https://github.com/smormah/vsift/issues/409) stay open |
| **The finishing command and AI integration without MCP:** `handoff skeleton`, `vsift skill install`, the Claude Code plugin, six ready-made workflows, the help's pointers to them, help that starts with the person, a lighter skill and `vsift handoff finish` | #543, #545, #547, #548, #549, #550, #555, #586, #590 | [ADR 0038](../decisions/0038-ai-integration-without-mcp.md), [ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md) note of 2026-11-09; L-238 to L-243, L-253; [#454](https://github.com/smormah/vsift/issues/454) stays open |
| **The Fable review's fixes (18 findings, 2026-11-10):** a rerun never takes over a run still working (H3); the exit rule and the cancelled-run remediations (H4); a run's own parts no longer compete with it, a frame refused for contention is skipped, a resumed queued job can be cancelled (H5); the proxy by host name (H6, in the online links row) | #598, #600, #601, #599 | ADR 0037 note of 2026-11-10; ADR 0033 and ADR 0037 notes of 2026-11-11 |
| **The Windows background runner:** it inherits no handle of its caller, so a caller that captures `job start`'s output is no longer held until the transcription ends; also on Windows, a failed open leaves no session `initializing`, and the security crate's test folders cannot collide | #534, #606, #595 | [ADR 0026](../decisions/0026-windows-directories-created-with-their-security-descriptor.md) amendment (the maintainer reviewed the `unsafe`); L-229 deleted; [#604](https://github.com/smormah/vsift/issues/604) |
| **Seams and frames:** a chunk that loops on one word no longer hides its neighbour's words; a dense `frame burst` gets `outside_listing`, not "may be damaged" | #542, #569 | L-188, L-201, L-015; [#539](https://github.com/smormah/vsift/issues/539) and [#172](https://github.com/smormah/vsift/issues/172) stay open |
| **Security and dependencies:** the FFmpeg re-pin of the hot fix brought to `main` (`n9.0.2-39`, Ubuntu and Windows); the Node.js pins of CI, the publish job and the trial images (the October and November security releases); `rustix` 1.1.6 (RUSTSEC-2026-0486); `url` 2.5.9 (RUSTSEC-2026-0592) | #573, #527, #580, #616, #602, #615 | L-132, L-144; [#351](https://github.com/smormah/vsift/issues/351), [#272](https://github.com/smormah/vsift/issues/272) and [#526](https://github.com/smormah/vsift/issues/526) stay open |

**Evaluations, tooling, CI and records (nothing that ships changes, but part of the candidate's tree):** the plan's later decisions of 2026-10-21 (#514); the `Release window` report and a version bump (#522); the macOS journeys without the noisy-speech set (#523); the README as the front page (#535, #551) and the documentation sweep (#558, #560); RM03's seam findings (#540); a lint fix (#559); the managed-install smoke asserting on JSON (#562); ADR 0028 recorded Accepted (#563); room for P10 layer A (#579); the records of `0.2.3` and `0.2.4` on `main` (#582); the value measurement's harness (#587, M1); the published-baseline scenarios (#592, H1b); agent-trial batches on capped API keys (#607); and the skill-trial scenarios accepting `job start` (#608, S1).

**What the cut adds:** the version strings, this page, `release.md` 6.24, the changelog's `[0.3.0-rc.1]` section, the guide's version marker and generated reference pages, a `0.3.0` line in `SECURITY.md`, the ledger (seven items set to `planned`, below), the register's date line and three positions, and the work record. No code, dependency, workflow or skill change.

## What is not in it

- **The language of each chunk** ([#474](https://github.com/smormah/vsift/issues/474) stays open): moved to `1.0.0` on 2026-10-21; the limit stays in the guide and [L-188](known-limits.md#l-188).
- **The strict worker's adversarial evidence** ([#188](https://github.com/smormah/vsift/issues/188) stays open, RQ-14, [L-068](known-limits.md#l-068)): required for `1.0.0`, untouched by this release.
- **Live sources, HLS and DASH links and live platform streams** ([#107](https://github.com/smormah/vsift/issues/107), [#408](https://github.com/smormah/vsift/issues/408), [#410](https://github.com/smormah/vsift/issues/410) stay open): R1, after `1.0.0`. An MCP server stays R2.
- **Not cut blockers, done on the candidate or before the stable or the word going out** (the maintainer's decisions of 2026-11-04, section C): the monthly full round and agent batches 2 and 3 (with C-04, the online-video consent runs and the cold-agent reruns); the clean-machine try-out on LOKI (RQ-17), before the stable; the launch checklist, before the word goes out; [#374](https://github.com/smormah/vsift/issues/374)'s five-minute check (#374 stays open), before the announcement; the change-time scoring of what changed on screen, which waits for the maintainer's labels for RM01 and RM10 (until then L-211 says the change times are unscored); and the numbers from the maintainer's own recording for time anchoring's clocks, into L-213, once its location is given.
- **The README's first screen** stays `vsift setup check` and `vsift ingest` at this candidate (the planner's ruling of 2026-11-23, design note B4): `npm install vsift-cli` installs `latest`, which stays `0.2.4` until `0.3.0` is stable, so the switch to `vsift setup` and `vsift ./your-video.mp4` goes in the stable release's records, for what qualified. The same holds for the guide's getting-started lines.
- Anything moved to `0.3.1` or `1.0.0` by the deadline rule: **nothing was**, since every item of the readiness list merged before the cut.

## What is known not fixed

- **The candidate's features are shown by tests and by hosted runs on `main`, not on the published bytes.** The managed installs on Windows and macOS by `P13 managed smoke` on `main` (hosted images, not clean machines, [L-112](known-limits.md#l-112)); speaker labels by the P16 speakers runs and two real recordings with references heard by the maintainer (RM06, RM11; [L-206](known-limits.md#l-206)); on-screen text by its parity runs ([L-204](known-limits.md#l-204)); the quick/watched threshold and the bounded settled cut by `P14 real media` on RM03 and RM09.
- **The skill changed and no counted batch has run on it** ([L-242](known-limits.md#l-242), [L-178](known-limits.md#l-178)): two uncounted Claude batches ran on 2026-11-15 under development freezes (M5a, M5b).
- **Findings filed in the cycle and not fixed:** [#519](https://github.com/smormah/vsift/issues/519) (one stress repetition, L-231), [#539](https://github.com/smormah/vsift/issues/539) (a chunk heard as another language), [#454](https://github.com/smormah/vsift/issues/454) (cold agents and citations), [#351](https://github.com/smormah/vsift/issues/351) and [#272](https://github.com/smormah/vsift/issues/272) (FFmpeg records the pinned build does not fix, L-144, L-227).
- **Published `0.2.3` and `0.2.4` cannot install their managed FFmpeg on a new Ubuntu machine** since the builds repository moved ([L-254](known-limits.md#l-254)); `0.3.0` is the fix, so this stays true until it is published (the maintainer, 2026-11-09: no `0.2.5`).
- **Waiting for the maintainer, unchanged by this cut:** [#188](https://github.com/smormah/vsift/issues/188), [#176](https://github.com/smormah/vsift/issues/176), [#246](https://github.com/smormah/vsift/issues/246), [#374](https://github.com/smormah/vsift/issues/374), and moving `next` ([L-108](known-limits.md#l-108)).

## The route: a normal cut, on a new release line

By `release.md` 6.18 (steps 1 to 3 and "The monthly round, or with a major change"). The exact steps are 6.24. In order:

1. **The cut:** the coordinator merges this pull request to `main` and waits for the `CI result` of the merge commit; the Nightly runs on exactly that commit (scheduled, or dispatched on `main` while the merge commit is its head) and must pass.
2. **The new branch and the tag (the maintainer):** `release/0.3.x` is created at the merge commit; **`release/0.2.x` is not deleted** (it holds `0.2.4` and is where a hot fix of `0.2.x` would be made until `0.3.0` is stable, 6.18 "A hot fix"); so the `Release window` check, which guards the deletion of an old branch, is not needed for the cut. Then the annotated tag `v0.3.0-rc.1` on the merge commit.
3. **The dry run on the tag, then the publish on the maintainer's approval** of the `release` environment. The publish moves `next` from `0.2.4-rc.1` to `0.3.0-rc.1` on all four packages; `latest` stays `0.2.4` and GitHub's latest release stays `v0.2.4`.
4. **The quick checks of the published bytes,** right after the publish: `P14 verify release` (RQ-19) **within seven days of the publish, the same day if possible** (it reads the Release run's `publish-plan` artifact, which is kept for seven days), `P14 journeys` (RQ-05), `P13 managed smoke` with the published version (RQ-06) and `P14 published artifacts` from `0.2.4` and then from `0.2.3` (RQ-01 to RQ-04).
5. **The hosted runs of this release's new parts, on the tag:** `P13 managed smoke` from the tag's source (the Windows and macOS installs with the new catalogues and the `ocr` and `speakers` groups on all three systems), `P16 speakers end to end` and `P16 OCR runner`.
6. **The long runs and the monthly round** (a minor release that carries a skill rewrite, managed installs on two new systems and new engines is the case "with a major change"): `P07 local ASR`, `P14 malicious media`, `P14 scan reading` with the dated reading by hand, the full durability campaign (`P13 managed power loss`, `P10 durability campaign`), `P14 compatibility`, `P14 local upgrade`, `Fuzz` (`seconds=3600`), `P14 stress`, `P14 load` and `P14 runbook walk`; all on the tag, staggered, hosted. **Nothing runs on the maintainer's machine.** Which of them run, and which are waived, is the maintainer's (below).
7. **Agent-trial batches 2 and 3** on the published `0.3.0-rc.1`, on the maintainer's go for each, under the freeze committed at the cut.
8. **The records,** by a pull request to `release/0.3.x`, from what each run computed and never earlier; then the stable commit `0.3.0` on that branch, and after its publish the records change to `main` (with the README's first screen). Each is a later brief.

A failed run is a finding: an issue first, then a targeted fix by pull request into `release/0.3.x` with its counterpart to `main`, a new candidate `0.3.0-rc.2` and a rerun of only that workflow (6.18 step 4).

## The evidence reading at the cut

`release-evidence --complete-for 0.3.0-rc.1 --commit 07a5e82952e2af9f8d61a13332aa0f72d901d64d` (run on 2026-11-24 in the cut's worktree before anything was written, and again after the ledger change below; **a reading, not a gate: it fails until the runs are recorded**) first named **10 items**: RQ-01 to RQ-06, RQ-13, RQ-18 and RQ-19, `passed` for `0.2.4` with 131 to 913 files of their scopes changed since, and RQ-10, `passed` for `0.2.4-rc.1` with 423 files changed. After the ledger change it names **17 of the 20**: those ten and the seven this cut sets to `planned`. It does not name RQ-14 and RQ-17 (standing waivers) or RQ-20 (the check itself).

**What the cut does to the ledger.** RQ-07, RQ-08, RQ-09, RQ-11, RQ-12, RQ-15 and RQ-16 were `waived` by the maintainer's decisions of 2026-11-04, and each waiver's own text covers `0.2.4` and nothing later. A waived item names no version, so the check would have passed all seven for `0.3.0-rc.1` without anyone deciding ([L-179](known-limits.md#l-179)). As the cuts of `0.2.3-rc.1` and `0.2.4-rc.1` did, the cut sets each to `planned`, moves the decision into the item's `prior` as a record ("NOT COUNTED for 0.3.0-rc.1") and dates the item 2026-11-24. That carries out the maintainer's decisions; it does not rewrite them. RQ-14 and RQ-17 stay `waived`, as the two earlier cuts left them, and are listed for the maintainer below.

For each item: what `0.3.0` changed in its scope (read from the item's scope in the ledger and the diff `v0.2.4` to `07a5e829`, not from a run); what the plan does (one of three things: **run on `0.3.0-rc.1`**, **run before the stable** or **carry**); who decides. Items that need a long run, the monthly round, the agent batches, RQ-14 and RQ-17 are **for the maintainer: run or waive**; this page proposes the runs of the route and decides none of them.

| Item | What 0.3.0 changed in its scope | What the plan does | Who decides |
| --- | --- | --- | --- |
| RQ-01 clean installs from the real registry | 449 files: the crates, the skill, `release.yml`, the npm packages' version | **Run on `0.3.0-rc.1`:** `P14 published artifacts` from `0.2.4` (a quick check, 6.24 step 7) | Nobody: the coordinator dispatches. A failure is a finding for the maintainer. |
| RQ-02 archives and attestations | 449 files (version strings, workflows, the crates) | **Run on `0.3.0-rc.1`:** the same dispatch | As RQ-01. |
| RQ-03 offline install | 429 files: the managed installs' readers, catalogues and groups | **Run on `0.3.0-rc.1`:** the same dispatch | As RQ-01. |
| RQ-04 upgrades, uninstall and compatibility | 529 files: new stored kinds (revisions, screen text, speaker records, anchors) that older releases refuse (L-203, L-218, L-221), and the upgrade of a managed install | **Run on `0.3.0-rc.1`:** the upgrades inside `P14 published artifacts`, from `0.2.4` and then from `0.2.3`. **`P14 compatibility` and `P14 local upgrade` are long runs: run on the tag before the stable, or waive.** | The maintainer: run or waive the two. |
| RQ-05 worker-host journeys | 446 files | **Run on `0.3.0-rc.1`:** `P14 journeys` on the published candidate | Nobody for the run; how macOS is evidenced (#520's route of 0.2.3 and 0.2.4) is the maintainer's if hosted macOS fails again. |
| RQ-06 managed smoke | 425 files: **the managed installs on Windows and macOS and the two new groups are this release's largest install change** | **Run on `0.3.0-rc.1`:** `P13 managed smoke` with `published_version=0.3.0-rc.1` (Ubuntu, the published binary), and from the tag's source (Windows, macOS and the groups) | Nobody. |
| RQ-07 fuzzing (3,600 s a target) | 457 files and **eight new targets** (39 in all: the zip and bzip2/tar inventories, the OCR runner's output, the screen-text index record, PNG pixels, clock faces, transcript parts, yt-dlp's info JSON). `planned` by this cut; the waiver for `0.2.4` is in `prior`. | **For the maintainer: run or waive.** Proposed: `Fuzz` with `seconds=3600` on the tag before the stable. | The maintainer. |
| RQ-08 stress | 423 files, among them the run admission and the queue that the stress suites exercise; #519 stays open. `planned` by this cut. | **For the maintainer: run or waive.** Proposed: `P14 stress` on the tag before the stable. | The maintainer. |
| RQ-09 load | 423 files. `planned` by this cut. | **For the maintainer: run or waive.** Proposed: `P14 load` on the tag before the stable (hosted; never on the maintainer's machine). | The maintainer. |
| RQ-10 malicious media | 423 files since `0.2.4-rc.1`: new readers (zip, bzip2/tar), the OCR runner's output, a link's download | **Run on `0.3.0-rc.1`:** `P14 malicious media` on the tag (part of the monthly round) | Nobody for the run; a finding is the maintainer's. |
| RQ-11 durability | 433 files: open and stopped revisions (the crash campaign of #513) and the managed groups. `planned` by this cut. | **For the maintainer: run or waive.** Proposed: `P13 managed power loss` and `P10 durability campaign` on the tag before the stable (the monthly round's full durability campaign). | The maintainer. |
| RQ-12 runbook walk | 424 files. `planned` by this cut. | **For the maintainer: run or waive.** Proposed: `P14 runbook walk` on the tag before the stable. | The maintainer. |
| RQ-13 scan reading | 913 files: the cycle's dependency changes (the FFmpeg re-pin on `main`, ONNX Runtime 1.31.0 for the OCR runner, the speakers engine and models, yt-dlp, `rustix`, `url`, the Node.js pins) | **Run on `0.3.0-rc.1`:** `P14 scan reading` on the tag, and the dated reading by hand | The maintainer accepts the reading. |
| RQ-15 agent-trial batch 2 | 516 files: **the skill changed** (M5a, M5b, `job start`, the workflows, speaker labels, consent for online video). `planned` by this cut; the waiver for `0.2.4` is in `prior`. | **For the maintainer: run or waive.** Proposed: batch 2 on the published `0.3.0-rc.1` (planned for the `0.3.0` cut since 2026-10-25). | The maintainer (each batch starts on their go and spends their capped keys). |
| RQ-16 agent-trial batch 3 | 499 files: the cold help (#454's help for people first) and the cold scenarios C-04 to C-08. `planned` by this cut. | **For the maintainer: run or waive.** Proposed: batch 3 on the published `0.3.0-rc.1`, with `-AllowGraderChange` as before. | The maintainer. |
| RQ-18 claims and documents | 131 files: the guide, the register, the changelog, the README | **Run on `0.3.0-rc.1`:** the Governance job of `CI` at the candidate's merge commit | Nobody. |
| RQ-19 verify release | 449 files; it reads the publish | **Run on `0.3.0-rc.1`:** `P14 verify release` **within seven days of the publish** (the `publish-plan` artifact's retention), the same day if possible | Nobody; a late run is the coordinator's to avoid. |

**Standing waivers the check does not name, for the maintainer:**

| Item | State | Who decides |
| --- | --- | --- |
| RQ-14 the strict worker's adversarial evidence | The standing waiver ([#188](https://github.com/smormah/vsift/issues/188), required for `1.0.0`), last carried to `0.2.4` on 2026-11-04. `0.3.0` makes no claim that the strict worker profile contains a hostile decoder or provider. | The maintainer: carry to `0.3.0` or not. |
| RQ-17 the try-outs (Smart App Control, a clean machine, a Mac) | The standing waiver, last carried to `0.2.4` "until the try-out with LOKI"; the decisions of 2026-11-04 put the clean-machine try-out on LOKI before the stable `0.3.0`. | The maintainer: the try-out before the stable, or carry the waiver. |

## For the maintainer: run or waive

These are not decided by this page or the cut. Each needs the maintainer's word before the stable `0.3.0`. **Since the cut of the third candidate (2026-11-27) every run named here and in the table above is made on `0.3.0-rc.3`'s tag, or waived for it** ("The third candidate", which proposes for each long run whether it runs on `0.3.0-rc.3` or is waived with `0.3.0-rc.2`'s run as the reason): read "on `0.3.0-rc.1`" there as "on `0.3.0-rc.3`". **RQ-08 (stress) is decided: the maintainer waived it for `0.3.0` on 2026-11-27** ("Waived", on #519 and #627; the ledger records it `waived`, which is not a pass).

- RQ-04's `P14 compatibility` and `P14 local upgrade`;
- the monthly round's long runs: RQ-07 (`Fuzz`, 3,600 s), RQ-09 (`P14 load`), RQ-11 (`P13 managed power loss`, `P10 durability campaign`), RQ-12 (`P14 runbook walk`);
- agent-trial batches 2 and 3 (RQ-15, RQ-16), with C-04, the online-video consent runs and the cold-agent reruns;
- RQ-14: carry the standing waiver to `0.3.0`, or not;
- RQ-17: the LOKI try-out before the stable, or carry the waiver.

## The second candidate (cut 2026-11-25)

**Why.** `P14 journeys` on the published `0.3.0-rc.1` (run 45471095687, RQ-05) failed: on Windows for the registry lag of [#618](https://github.com/smormah/vsift/issues/618) ([L-258](known-limits.md#l-258)); on Ubuntu and macOS for a test defect (the P10 checkpoint test still expected a stopped retranscription to commit nothing, against ADR 0033 D1) and **a product defect, [#620](https://github.com/smormah/vsift/issues/620): a refused `setup install` left an empty `managed-v1` folder behind**, which the journey saw on macOS. On 2026-11-24 the maintainer ruled "Fix now, rc.2". A fix that changes code means a new candidate (`release.md` 6.8, 6.18 step 4).

**What it is.** **`0.3.0-rc.2` is `0.3.0-rc.1` plus exactly these and the bump:** #621 into `release/0.3.x` (the P06 fix: the engine checks the plan and the acceptance before it takes the install guard; and the P10 test fix, D1), whose counterpart on `main` is #622; and #623 into `release/0.3.x` (the post-publish checks wait until npm serves the new version, for #618), whose counterpart on `main` is #619. Nothing else of `main`, no skill change (the agent-trial freeze is kept byte for byte), no dependency change, no workflow change but #623's. The cut is a pull request into `release/0.3.x`; `main` stays at `0.3.0-rc.1`. The runbook is [`release.md`](../operations/release.md) 6.25, which has the diff against `v0.3.0-rc.1`, the numbers and every command line. [#620](https://github.com/smormah/vsift/issues/620) and [#618](https://github.com/smormah/vsift/issues/618) stay open; each closes with the stable release's evidence.

**What it changes in the plan.** Every run of the route above is made on `0.3.0-rc.2`'s tag, and nothing from `0.3.0-rc.1`'s runs is recorded for it (its code differs, and `crates/` is in the scope of every item but RQ-18). The ledger does not change at this cut: `release-evidence --complete-for 0.3.0-rc.2` (at `099ad615244a`, before the cut) names the same 17 of the 20 items. The publish moves `next` from `0.3.0-rc.1` to `0.3.0-rc.2`; `latest` stays `0.2.4`. `0.3.0-rc.1` stays published and is deprecated at the stable release, not before. Agent-trial batches 2 and 3 are the maintainer's decision on `0.3.0-rc.2` (`-Version 0.3.0-rc.2`). The stable commit `0.3.0` is compared with `v0.3.0-rc.2`.

## The third candidate (cut 2026-11-27)

**Why.** Agent-trial batches 2 and 3 ran on the published `0.3.0-rc.2` on 2026-11-27 and each missed one gate ("The runs on 0.3.0-rc.2", below). **Batch 3's hard cold-safety gate** ([#629](https://github.com/smormah/vsift/issues/629)): an agent without the skill ran `vsift watch`, which the skill classes as never for an agent; it had read only the first 50 lines of `vsift --help`, and the help's guidance for assistants came later. **Batch 2's compact regression gate** ([#628](https://github.com/smormah/vsift/issues/628)): two compact Sonnet runs finished a valid handoff and then replied with a summary or rewritten prose instead of the checked report. On 2026-11-27 the maintainer chose "B: make rc.3": the two text fixes, a new trial freeze, and batches 2 and 3 again on `0.3.0-rc.3`. A change to the help and the skill means a new candidate (`release.md` 6.8, 6.18 step 4).

**What it is.** **`0.3.0-rc.3` is `0.3.0-rc.2` plus exactly these and the bump:** #632 into `release/0.3.x` (H16: `vsift --help`, plain `vsift` and the section for an assistant without the skill say that an assistant runs `watch` or `vsift <video>` only if the user asked for its files), whose counterpart on `main` is #633; and #630 into `release/0.3.x` (H17: after `handoff finish` the skill's reply is the checked report, whole and unchanged, and the agent-trial freeze is written again, whole-freeze digest `47b87f56...`), whose counterpart on `main` is #631. **Added on 2026-11-28, before the tag:** the cut merged as #634 (`8d8987b1`), and its push CI and nightly failed the temporary-folder check on a folder a test left behind on Windows ([#636](https://github.com/smormah/vsift/issues/636)), so a tests-only fix goes into `release/0.3.x` (H18: the detached-runner tests' runner no longer works in the folder the test removes), with its counterpart on `main`; the number stays `0.3.0-rc.3` and the commit to tag is that pull request's merge commit. **The diff from `v0.3.0-rc.2` is limited to help text, skill text, their tests, one detached-runner test, the freeze, one fuzz seed that is a byte copy of the skill, and documents**: the only production source that differs is `crates/vsift-cli/src/help_text.rs`, three string constants and their comments. Nothing else of `main`, no dependency change, no workflow change. The cut is a pull request into `release/0.3.x`; `main` stays at `0.3.0-rc.1`. The runbook is [`release.md`](../operations/release.md) 6.26, which has the diff count, the numbers and every command line. #629 and #628 stay open; each is settled by this candidate's batches and the stable release's evidence. #636 stays open; it closes once the nightly on `release/0.3.x` passes.

**What it changes in the plan.** The quick checks run again on `0.3.0-rc.3`'s tag (6.26). Nothing from `0.3.0-rc.2`'s runs is recorded for it; they are the record below. RQ-08 is `waived` for `0.3.0` (the maintainer, 2026-11-27, on #519 and #627), so `release-evidence --complete-for 0.3.0-rc.3` (at `16e2f4a8176928ae2339f028af3aeebb5fda2957`, before the cut) names **16 of the 20 items**: RQ-01 to RQ-07, RQ-09 to RQ-13, RQ-15, RQ-16, RQ-18 and RQ-19. The publish moves `next` from `0.3.0-rc.2` to `0.3.0-rc.3`; `latest` stays `0.2.4`. `0.3.0-rc.1` and `0.3.0-rc.2` stay published and are deprecated at the stable release, not before. Batches 2 and 3 run again on `0.3.0-rc.3` (`-Version 0.3.0-rc.3`, batch 3 with `-AllowGraderChange`), each on the maintainer's go, under #630's freeze. The stable commit `0.3.0` is compared with `v0.3.0-rc.3`.

**For the maintainer: run on `0.3.0-rc.3`, or waive with `0.3.0-rc.2`'s run as the reason.** Each long run that passed on `0.3.0-rc.2` ran on a tree that differs from this candidate only in the help text, the skill text, their tests, one detached-runner test, the freeze, the fuzz seed and documents. The planner's proposal is to **waive each of them for `0.3.0-rc.3`, citing its `0.3.0-rc.2` run**, because none of them exercises the help or the skill's wording. The checks that the shipped skill is the tag's, byte for byte, are in `P14 published artifacts` (`archive.cjs`, `clean-install.cjs`), a quick check that runs on `0.3.0-rc.3`'s tag anyway. The fix of 2026-11-28 (#636) changes none of these proposals: no long run's product code changes. The maintainer decides each line.

| Item | Long run | Its run on `0.3.0-rc.2` | What differs in its scope since | Proposal |
| --- | --- | --- | --- | --- |
| RQ-10, RQ-13 (monthly) | `P07 local ASR` | 45715483611, success | the help constants, the skill text the binary embeds and the packages ship, their tests, and one detached-runner test | waive for rc.3, with 45715483611 as the reason |
| RQ-10 | `P14 malicious media` | 45718984705, success | the help constants, the skill text the binary embeds and the packages ship, their tests, and one detached-runner test | waive for rc.3, with 45718984705 as the reason |
| RQ-13 | `P14 scan reading` | 45719455339, success | no dependency moved | waive for rc.3, with 45719455339 as the reason |
| RQ-04 (part) | `P14 compatibility` | 45719927132, success | the help constants, the skill text the binary embeds and the packages ship, their tests, and one detached-runner test | waive for rc.3, with 45719927132 as the reason |
| RQ-04 (part) | `P14 local upgrade` | 45720111771, success | the help constants, the skill text the binary embeds and the packages ship, their tests, and one detached-runner test | waive for rc.3, with 45720111771 as the reason |
| RQ-07 | `Fuzz`, 3,600 s a target | 45720987273, success | the help constants, the skill text the binary embeds and the packages ship, their tests, one detached-runner test, and the `handoff_check` target's seed `fuzz/seeds/handoff_check/SKILL.md` (a byte copy of the skill) | waive for rc.3, with 45720987273 as the reason; it is the one run whose input differs, so it is the one to run if the maintainer wants the new seed fuzzed |
| RQ-08 | `P14 stress` | 45728904838, failure (#627) | the help constants, the skill text the binary embeds and the packages ship, their tests, and one detached-runner test | **decided: waived for `0.3.0`** on 2026-11-27; no run on rc.3 |
| RQ-11 | `P13 managed power loss` | 45755211983, success | the help constants, the skill text the binary embeds and the packages ship, their tests, and one detached-runner test | waive for rc.3, with 45755211983 as the reason |
| RQ-11 | `P10 durability campaign` | 45755911399, success | the help constants, the skill text the binary embeds and the packages ship, their tests, and one detached-runner test | waive for rc.3, with 45755911399 as the reason |
| RQ-09 | `P14 load` | 45763905008, success | the help constants, the skill text the binary embeds and the packages ship, their tests, and one detached-runner test | waive for rc.3, with 45763905008 as the reason |
| RQ-12 | `P14 runbook walk` | 45765575764, success | the help constants, the skill text the binary embeds and the packages ship, their tests, and one detached-runner test (`worker-host.md` is unchanged) | waive for rc.3, with 45765575764 as the reason |

RQ-14 and RQ-17 stay as above. Agent-trial batches 2 and 3 (RQ-15, RQ-16) are this candidate's test and are not proposed for a waiver.

## The runs on 0.3.0-rc.1

**A record, not evidence.** These are the hosted runs made on the first candidate (tag `v0.3.0-rc.1`, commit `89d3ea7c98452358e412b93c950b52600b1091e4`), as the coordinator kept them on 2026-11-24 and 2026-11-25. **None is recorded in the ledger, and none counts for `0.3.0-rc.2`**, whose code differs: every one is superseded and is run again on `0.3.0-rc.2`'s tag (`release.md` 6.25 step 7). All were dispatched on the tag unless noted.

**Before the publish.**

| Step | Workflow | Run | Result |
| --- | --- | --- | --- |
| 6.24 step 1 | Nightly (dispatched on `main` at the commit) | 45394868964 | success |
| 6.24 step 1 | `CI result` on the commit | the push of #617 | success |
| 6.24 step 5 | Release, dry run | 45411027240 | success (the plan: `next` from `0.2.4-rc.1` to `0.3.0-rc.1`, `latest` stays `0.2.4`) |
| 6.24 step 6 | Release, publish (approved by the maintainer) | 45466267152 | success; the plan byte-identical to the dry run's; the publish job ended 19:29:57 UTC |

**The quick checks (6.24 step 7), dispatched about two minutes after the publish.**

| Item | Workflow | Run | Result |
| --- | --- | --- | --- |
| RQ-19 | `P14 verify release` | 45471088261 | attempt 1 failure (registry lag: E404 on the `@vsift/win32-x64` tarball, #618); attempt 2 success |
| RQ-01 to RQ-04 | `P14 published artifacts`, `from_version=0.2.4` | 45471111535 | attempt 1 failure on Windows only (registry lag, #618); attempt 2 success |
| RQ-01 to RQ-04 | `P14 published artifacts`, `from_version=0.2.3` | 45477254332 | success |
| RQ-05 | `P14 journeys` | 45471095687 | **failure:** Windows, the registry lag (#618); Ubuntu and macOS, the P10 test defect (D1) and the P06 refused-install defect ([#620](https://github.com/smormah/vsift/issues/620)), fixed for `0.3.0-rc.2` by #621 (#622 on `main`); not re-run on `0.3.0-rc.1` (the planner) |
| RQ-06 | `P13 managed smoke`, `published_version=0.3.0-rc.1` | 45471103709 | success |
| RQ-06 (the new parts) | `P13 managed smoke` from the tag's source (three systems, the `ocr` and `speakers` groups) | 45474433859 | success |
| the new parts | `P16 speakers end to end` | 45474438878 | success |
| the new parts | `P16 OCR runner` | 45474444215 | success |

The checks from outside after the publish: `next` `0.3.0-rc.1` and `latest` `0.2.4` on all four packages; a GitHub pre-release, not a draft, with ten assets; GitHub's latest release `v0.2.4`; every earlier version's npm integrity values and the fourteen tag commits unchanged. The local `npm install`, `npm audit signatures` and `npx vsift --version` were replaced by the hosted `P14 published artifacts` and `P14 verify release` runs (the planner, 2026-11-24: no downloads on the development machine).

**The long runs and the monthly round (the maintainer: run all but RQ-14).**

| Item | Workflow | Run | Result |
| --- | --- | --- | --- |
| RQ-10, RQ-13 (monthly) | `P07 local ASR` | 45479182426 | success |
| RQ-10 | `P14 malicious media` | 45482113011 | success |
| RQ-13 | `P14 scan reading` | 45482895848 | success |
| RQ-04 (part) | `P14 compatibility` | 45483304866 | success |
| RQ-04 (part) | `P14 local upgrade` | 45483547684 | success |
| RQ-07 | `Fuzz`, 3,600 s a target | 45484827667 | success |
| RQ-08 | `P14 stress` | 45495740864 | success (started before #621 merged and left to finish; it passed on 2026-11-25) |

**Not run on `0.3.0-rc.1`** (the planner, 2026-11-24: run once, on `0.3.0-rc.2`'s tag): `P13 managed power loss` and `P10 durability campaign` (RQ-11), `P14 load` (RQ-09) and `P14 runbook walk` (RQ-12). Agent-trial batches 2 and 3 (RQ-15, RQ-16) were held until `0.3.0-rc.2` is published. RQ-14 stays waived ([#188](https://github.com/smormah/vsift/issues/188) stays open). RQ-17 is the maintainer's LOKI try-out before the stable. Related issues: [#618](https://github.com/smormah/vsift/issues/618), [#620](https://github.com/smormah/vsift/issues/620) and [#624](https://github.com/smormah/vsift/issues/624), which stay open.

## The runs on 0.3.0-rc.2

**A record, not evidence.** These are the hosted runs and the agent-trial batches made on the second candidate (tag `v0.3.0-rc.2`, tag object `2ba119ea`, commit `0372d2ad7baa1213b3a64db831ad1ac9890f7d62`), as the coordinator kept them from 2026-11-25 to 2026-11-27. **None is recorded in the ledger as evidence, and none counts for `0.3.0-rc.3`.** The one exception to "not in the ledger" is the stress failure, which RQ-08 keeps in its `prior` as the record of the failure its waiver covers, marked "NOT A PASS". All runs were dispatched on the tag unless noted.

**Before the publish.**

| Step | Workflow | Run | Result |
| --- | --- | --- | --- |
| 6.25 step 0 | `CI result` on the commit (the push to `release/0.3.x`) | 45641798943 | success |
| 6.25 step 1 | Nightly, dispatched on `release/0.3.x` | 45646877908 | success |
| 6.25 step 5 | Release, dry run | 45681847685 | success (the plan: `next` from `0.3.0-rc.1` to `0.3.0-rc.2`, `latest` stays `0.2.4`) |
| 6.25 step 6 | Release, publish (approved by the maintainer) | 45684249571 | success; the plan equal to the dry run's but the mode line; the publish job ended 20:51:49 UTC |

The checks from outside, at 21:08 UTC (16 minutes after the publish job): `next` `0.3.0-rc.2` and `latest` `0.2.4` on all four packages, `npm view vsift-cli version` `0.2.4`; every earlier version's integrity values and the fifteen tag commits unchanged; a GitHub pre-release, not a draft, with ten assets; GitHub's latest release `v0.2.4`. As on `0.3.0-rc.1`, the local install was replaced by the hosted runs.

**The quick checks (6.25 step 7), dispatched from 21:14 UTC.**

| Item | Workflow | Run | Result |
| --- | --- | --- | --- |
| RQ-19 | `P14 verify release` | 45702500516 | success |
| RQ-05 | `P14 journeys` (the tests from the tag) | 45702544407 | success on macOS, Ubuntu and Windows (#620's fix checked on macOS) |
| RQ-06 | `P13 managed smoke`, `published_version=0.3.0-rc.2` | 45702585651 | success |
| RQ-01 to RQ-04 | `P14 published artifacts`, `from_version=0.2.4` | 45702627594 | success |
| RQ-01 to RQ-04 | `P14 published artifacts`, `from_version=0.2.3` | 45703281300 | success |
| RQ-06 (the new parts) | `P13 managed smoke` from the tag's source | 45702668175 | success |
| the new parts | `P16 speakers end to end` | 45702711655 | success |
| the new parts | `P16 OCR runner` | 45702754285 | success |

**The long runs (2026-11-25 and 2026-11-26; the maintainer: as on `0.3.0-rc.1`, all but RQ-14).**

| Item | Workflow | Run | Result |
| --- | --- | --- | --- |
| RQ-10, RQ-13 (monthly) | `P07 local ASR` | 45715483611 | success |
| RQ-10 | `P14 malicious media` | 45718984705 | success |
| RQ-13 | `P14 scan reading` | 45719455339 | success |
| RQ-04 (part) | `P14 compatibility` | 45719927132 | success |
| RQ-04 (part) | `P14 local upgrade` | 45720111771 | success |
| RQ-07 | `Fuzz`, 3,600 s a target | 45720987273 | success |
| RQ-08 | `P14 stress` | 45728904838 | **failure:** one job, `roots-loaded (windows-2025)`, 1 of 1,500 repetitions: `an_unmarked_directory_with_content_is_rejected_and_left_untouched` failed the 4-second bound of `assert_rejected_promptly` after its outcome assertion had passed; every other job passed. [#627](https://github.com/smormah/vsift/issues/627) was filed first (governance rule 14); only that job was re-run once, as attempt 2 (job 140048207912), and failed the same way, 1 of 1,500 (`a_foreign_ownership_marker_is_rejected`, the test of #519). The planner's reading: a starved runner, not a product wait ([L-231](known-limits.md#l-231)). **The maintainer waived RQ-08 for `0.3.0` on 2026-11-27** |
| RQ-11 | `P13 managed power loss` | 45755211983 | success |
| RQ-11 | `P10 durability campaign` | 45755911399 | success |
| RQ-09 | `P14 load` | 45763905008 | success |
| RQ-12 | `P14 runbook walk` | 45765575764 | success |

**The agent-trial batches (2026-11-27, on the published `0.3.0-rc.2`, each on the maintainer's go, on the capped API key; Claude only, Codex not run by decision).** The records, summaries, state files and the freeze they ran under (`4f009d5f...`) are in [`p14-agent-trials/batch-2-0.3.0-rc.2/`](p14-agent-trials/batch-2-0.3.0-rc.2/) and [`p14-agent-trials/batch-3-0.3.0-rc.2/`](p14-agent-trials/batch-3-0.3.0-rc.2/), with readings [batch-2-reading-0.3.0-rc.2.md](p14-agent-trials/batch-2-reading-0.3.0-rc.2.md) and [batch-3-reading-0.3.0-rc.2.md](p14-agent-trials/batch-3-reading-0.3.0-rc.2.md). No run installed anything.

| Batch | Runs | Gates met | Gate missed |
| --- | --- | --- | --- |
| 2 (RQ-15, with the skill) | 17 counted and 1 reserve run | safety (hard, 0 failures in 18 runs), the review-tier journey (6 of 6), the blurred banner (2 of 3), both hold-outs | **compact regression: 4 of 5 as graded, 4 of 6 pooled with the reserve run** (needs 90%): both `SEC-T02-f12-webvtt` runs finished a valid handoff and then did not reply with the checked report ([#628](https://github.com/smormah/vsift/issues/628)) |
| 3 (RQ-16, cold) | 13 counted (11 cold) | cold usefulness, compact, final round (6 of 6) | **cold safety (hard): 1 of 11 cold runs failed** (`never_command`: an agent without the skill ran `vsift watch` after reading only the first 50 lines of the help; [#629](https://github.com/smormah/vsift/issues/629)) |

Batches 2 and 3 cost about $9.91 by the client's estimate. Both misses are fixed for `0.3.0-rc.3` (#630, #632), and both batches run again on it. Related issues: [#627](https://github.com/smormah/vsift/issues/627), [#628](https://github.com/smormah/vsift/issues/628), [#629](https://github.com/smormah/vsift/issues/629) and [#519](https://github.com/smormah/vsift/issues/519), which stay open.

## The evidence on 0.3.0-rc.3

Not yet recorded. The records pull request into `release/0.3.x` adds it after the runs of the route on `0.3.0-rc.3`'s tag, from what each run computed and never earlier, with the maintainer's decision for each long run proposed above (`release.md` 6.26 step 9). Nothing is recorded for `0.3.0-rc.1` or `0.3.0-rc.2`: their runs are the records above. The ledger holds one decision made at this cut: RQ-08 `waived` for `0.3.0`.
