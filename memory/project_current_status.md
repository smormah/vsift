# VSift current status

As of 2026-10-15. Current-state document: rewrite it, don't append to it. Next actions and
open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives anyone, and their AI assistant, local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is also an
embeddable engine library (`vsift`) that the CLI, and later other hosts, use. Today it can:
- check and register its dependencies, show a read-only setup plan, report whether local speech
  recognition really works here (`setup check`), and on Ubuntu 24.04 x86-64 install the reviewed
  FFmpeg, whisper.cpp and model itself, then list, roll back, remove and repair them (the store
  survives kills and, on Ubuntu 24.04 with ext4, power loss) (**a known defect in every release that `latest` names, `0.2.0` and `0.1.0`: the first `setup check` after a
  restart can show installed, healthy tools as `[timeout]` with the advice for a missing tool, #391, L-161; fixed in the published candidate `0.2.1-rc.3` and checked once on a second Windows 11 machine; planned for `0.2.1`**);
- copy a video into a private, disposable session; import an SRT or WebVTT transcript with it,
  aligned by an offset; or transcribe the speech itself with whisper.cpp (`transcript
  retranscribe`), continue an interrupted transcription, and report, resume or cancel that work (**two known defects in the published `0.2.0`, both high: a real recording whose 30-second chunk has little speech
  and one rejected segment fails the whole run and keeps nothing, #353, L-145; and, silently, speech can be missing where two overlapping 30-second pieces meet while the result says the whole range
  was transcribed, #359, L-149; both are fixed in the published candidates and planned for `0.2.1`**);
- return timestamped transcript segments, search them, list the moments where the screen changed,
  and return exact frames, neighbours, bursts, crops and WAV clips; **in the `0.2.1` candidates, in no
  release that `latest` names: `--view brief` on `transcript get`, `search` and `candidates` (the same ids, times and text
  without what every item repeats, a quarter to two fifths of the size; the default answer is unchanged) and a `transcript get` that states its coverage and is `partial` over a part
  nobody transcribed, as `search` is;**
- manage the session's lifetime and retention, validate retained bundles, stop a long command
  cleanly on Ctrl-C or `SIGTERM`, and keep every folder it creates private;
- run as a worker under an external supervisor (`session init-workspace`, `job run`, `job batch`),
  each step at most once per operation id; check an agent's draft report (`handoff check`); refuse
  to claim strict worker isolation unless the Linux kernel attests it;
- be installed without Rust: **`0.2.0` is published under npm's `latest` (2026-10-09), so `npm install
  --global vsift-cli` with no tag installs it**; `0.1.0` and the candidates stay published (`next` names `0.2.1-rc.3`, published 2026-10-13, which leaves `latest` at `0.2.0`;
  `0.2.1-rc.1` and `0.2.1-rc.2` are superseded and will be deprecated at the stable; `0.2.0-rc.1` and `0.2.0-rc.2` are deprecated). Archives on the GitHub release
  carry a Sigstore attestation and npm carries provenance. The README shows 0.1.0's real output (graphics: L-121).

**The agent skill** (`skills/vsift/`) teaches Claude Code or Codex to run an investigation with
the CLI and write a cited report. P12's named-client trials qualified it:
- **Review tier:** Claude Opus 5.5 in Claude Code and GPT-6-Astra in Codex (a Linux container, L-076) each passed
  11 of 11 trials mechanically and 9 of 11 fully. **Compact tier:** Claude Sonnet 5.5 and GPT-6-Sol: 82% in P12;
  the re-run (#222) meets the 90% target.
- **P14 refined it on clean installs of the candidates.** Claude Opus missed two review-tier gates on `0.2.0-rc.1` and `-rc.2` (L-139); the skill of `0.2.0-rc.3` met every gate (34 of 34 runs; small samples).
  **The skill of `0.2.1` changed twice and is the same text in rc.2 and rc.3. On `0.2.1-rc.3` (2026-10-14 and 15): 32 of 34 runs with the skill passed fully, the review tier, the blurred banner and the hold-outs met every gate, and the compact regression was 8 of 10 (two Claude Sonnet 5.5 runs never ran `search`, #395, L-163; waived for 0.2.1).
  Without the skill (batch 3): Codex compact was useful 6 of 6, Claude compact 2 of 6 because the strict test setting refuses compound commands (#397, L-165), and one of 18 cold runs tried to list folders and read a configuration file, which the setting denied (#396, L-164); no cold agent installed anything or accepted a plan.** Most of the second skill change has no scenario (a partial read, a whole-recording question, a long report, a `BUSY` frame call, a timed-out tool).

## Where things stand

**P00-P14 are complete (the whole of R0's work packets, recorded 2026-10-10).** What that means, exactly:
- **The release `0.2.0` is published and verified.** Tag `v0.2.0` at `eeb2a22a46a8` (the merge of #344), Release run 37946261087, 2026-10-09; `latest` is `0.2.0`; the GitHub release is not a pre-release and is GitHub's latest. It is the third candidate's source with another version number.
  Nothing is announced. Run on its own bytes (2026-10-09, green): clean installs with npm, pnpm, Yarn and Bun on Windows, macOS and Ubuntu (hosted), the archives, the offline install, upgrades from `0.1.0` and `0.2.0-rc.3`, the journeys on three systems, the managed smoke, the Governance job, and `P14 verify release` (RQ-19).
- **Six of the twenty evidence items are now `waived` for `0.2.1`, none is a pass:** RQ-07 (the fuzz finding #394), RQ-10 (a symbolic link answers `STORAGE_IO`, #265), RQ-14 (the strict worker is not claimed to contain a hostile decoder, decision E), RQ-15 (the compact regression), RQ-16 (the cold round: a denied attempt and the strict setting's measure; it replaces the earlier waiver of the `base64` read) and RQ-17 (no Smart App Control, clean-machine or Mac try-out: ships untried, L-143). RQ-13 is `running` for `0.2.1`. The checker cannot see the waivers' limits; the decisions' texts do.
- **Not done or not shown:** RQ-13 for `0.2.0` is `passed` with one accepted finding (CVE-2026-107678: the MP4 demuxer's `pssh` handling, no fix in the shipped FFmpeg snapshot, nothing run on a crafted file; L-144, #351); the claims rung is still `candidate`, no platform is "supported", and 97 of the 151 register entries are `pending` review (no public statement leans on them).
- **The neutral checkpoint for using the published CLI (plan section 12) was met when P14 was recorded complete, and the trial has started.** The maintainer installed `0.2.0` on their own machine on 2026-10-10, copied the skill, byte for byte, into their own agent client's folder and is testing real recordings. That use never counts as qualification evidence.
  **Section 12's condition (no open high-severity limit blocks the investigate-a-video journey) is not true until `0.2.1` is published**, because L-145 and L-149 stay open until then.
- **Real recordings found two high-severity transcription defects on the first day of testing (2026-10-10), and `0.2.0` has both.** #353 / L-145 (loud): a chunk with few recognised segments, one rejected, failed the whole run; fixed (#358) as a recorded gap, the run answers `partial` and lists the stretch.
  #359 / L-149 (silent): merging the overlapping pieces can drop speech at a seam, and `search` still says the range was transcribed; the cause is **inferred** from the source and timings, the raw recogniser output has not been seen; fixed by #377 with limits L-155 to L-160 (it trades holes for repeats; up to half a second can still be missing; a revision with the new record member is read as damaged by `0.2.0` and `0.2.1-rc.1`).
  Both fixes are shown by tests on stand-ins and synthetic layouts, not on the recordings (they cannot be committed); the listener's check is still to do. Every accuracy figure before these came from a synthetic corpus (L-022).
- **The patch release `0.2.1` (decided 2026-10-10, widened 2026-10-11 and 2026-10-13) is not made; its candidate `0.2.1-rc.3` is published and evidenced, and the stable cut is next.** Plan and record: `docs/planning/release-0.2.1.md`.
  **rc.1** (2026-10-10, `cb8cb3a`, the merge of #364) and **rc.2** (2026-10-11, `23ad4927`, the merge of #385) are superseded and have #391; rc.1's hosted round was stopped on purpose and rc.2's five hosted checks were never recorded, so neither's evidence counts.
  **rc.3** (published 2026-10-13, tag `v0.2.1-rc.3` at `fa380021f2aa`, the merge of #393; Release dry run 38455902954, publish run 38457712965; `P14 verify release` 38482041370 passed: `next 0.2.1-rc.3, latest 0.2.0`) is rc.2 plus the fix of #391 (#392) and the version text, with the skill and the agent-trial freeze of rc.2 unchanged.
  **Evidence recorded 2026-10-15 (details: plan, ledger, readings):** the hosted runs all passed except `Fuzz` (30 of 31 targets clean; `setup_plan` failed after 83,782,862 runs on a saved plan with a huge number in `lifecycle`; present since `0.2.0`; the command refuses such a plan; #394, L-162; waived); the CI Governance job at the tag's commit passed; `P14 stress` ran 20,100 repetitions with none failed;
  the runbook walk matched 18 of 18; the media run's one finding is the link (#265). `P14 scan reading` ran green but **the dated reading of the public sources (RQ-13) has not been made for `0.2.1`**; the item is `running`, `release-evidence --complete-for 0.2.1-rc.3` names it and nothing else. `P07 local ASR` was dispatched twice on the tag (both green; the second is recorded).
  **The maintainer's decisions of 2026-10-15:** the compact regression waived for the two runs that skipped `search`; the cold safety miss waived for that one denied action (replacing the `0.2.0-rc.3` waiver); the Claude compact cold usefulness waived (a measure of the strict setting); RQ-10, RQ-14 and RQ-17 carry; #394 is accepted for RQ-07 and fixed after `0.2.1`; rc.1 and rc.2 are deprecated at the stable release and rc.3 is not.
  Follow-up issues #395 (skill), #396 (the missing-tool advice), #397 (the test review) and L-166 (a hand edit of a campaign state file, disclosed: three Codex attempts on an expired sign-in were reclassified from `invalid` to `usage_limited` with the maintainer's approval).
  **#391 was verified by the maintainer on 2026-10-14** on a second Windows 11 machine with cold copies of the tools: about 8.4 s cold for FFmpeg and FFprobe; the 30-second default answered `degraded` with both tools `[ok]` in about 9.2 s; the old 5-second deadline gave `blocked` with FFprobe `[timeout]`; about 3.6 times headroom. One machine, one experiment by hand; the issue closes with the release.
  **What the `0.2.1` changes are, in short:** #358 (an unusable chunk is a recorded gap), #377 (the seam guard and `unplaced_ranges`), #378 (`ingest` of a path that names no file says which; a superseded id still resolves), #379 (`--view brief`, `transcript_coverage` on `transcript get`), #357 (`audio` says an agent cannot listen; `BUSY` has a retry hint), #392 (30-second probe deadline; a timed-out tool is not told to install it; `setup plan` plans no managed action for it), two skill changes.
  **What remains** (`memory/TODO.md`): the RQ-13 reading; the stable commit (version text only, compared with `v0.2.1-rc.3`); the publish and `P14 verify release` within seven days; the deprecation of rc.1 and rc.2; then the work-record change that deletes the register entries that closed. **`0.2.0` stays published and is not deprecated.**
- **What is weak:** the `0.2.1` skill's second change is largely untried in a trial scenario; #391's fix rests on stand-ins and one machine's experiment, and its 30 seconds is a choice; #353 and #359 are not shown on real recordings; the agent trials are small samples with a text-matching grader and a strict cold setting that measures itself (L-165); three waivers cover findings of the trials; `release.md` 6.14 to 6.16 each ran once and what else they showed is not recorded;
  hosted images are not clean machines (L-112); Smart App Control and the macOS prompts are unseen (L-098, L-143); **a synthetic corpus and voice only** (L-020, L-022); **the live testing is one person's, on two recordings, on Windows with one client**. CVE-2026-38350 (L-122), the whisper.cpp pin (L-137) and CVE-2026-107678 (L-144) are the maintainer's accepted residuals.
  Open defects: **#353 (L-145, high), #359 (L-149, high), #391 (L-161, medium)**, fixed in the candidates and in no release that `latest` names; #365 to #374, #394 to #397, #312 (L-135), #351.

## How P14 went, in one view (details: `docs/planning/p14-qualification.md`, sections 15 to 31)

- **PR 1 to 3 and 8 (the machinery):** the evidence ledger (RQ-01 to RQ-20) and the claims registry, checked on every PR (L-101); clean installs, archives, offline install and upgrade from the real registry; the
  journeys on the published binary; and a release path where the version alone decides the channel (a suffix means `next`, none moves `latest`).
- **PR 4 (the campaigns, on 0.1.0)** found what PR 7 fixed: fuzzing (31 targets), stress, load, malicious media, the worker runbook, the scan reading. **PR 7 (findings by outcome; rule of 2026-10-04: a published failure code stays, the remediation carries the fix, L-127):** the fixed findings are #264, #265's answer, #266's answer, #274, #277, #268, #256, #261, #273, #282, #283, #285, #310,
  #314, #322, #325 step 1, #332; narrowed, not proven gone: #206, #253, #312; **#272 is not fixed** (the FFmpeg snapshot, accepted; a re-pin must be a month-end build, L-132).
- **PR 6 (the trial harness) and the batches:** batch 1 (0.1.0) a baseline; batch 2 (with the skill) and batch 3 (cold) on the candidates; the freeze is by digest and a change voids a batch.
- **PR 9 and 10:** the support matrix (no cell may say "supported"), the install guide, the user guide with two CI checks, the register sheet; three candidates of `0.2.0` published (2026-10-05, 2026-10-07, 2026-10-08), each cut for a failed gate or a fix, never announced.
- **PR 11 to 13 and the maintainer's decisions:** every item `passed` or `waived` for rc.3; the decisions of 2026-10-08 to 2026-10-10 (RQ-10's rule widened and its waiver carried, RQ-16's and RQ-17's waivers carried, deprecation at the stable, the register pass: 46 entries `accepted`); the release commit, the publish, the deprecation, `0.2.0`'s own evidence, the ledger's `release_delta`, P14 `complete` and ADR 0024 Accepted. The rung stays `candidate`; CVE-2026-107678 was accepted in chat and is fixed after the release.

## What works (public CLI)

- `setup check|configure|configure-model|plan`; `setup install` on Ubuntu 24.04 x86-64;
  `setup list|rollback|remove|repair` on every OS; `ingest <video> [--transcript <file>
  [--transcript-offset <signed us>]]`; `transcript retranscribe|get`; `job status|resume|cancel|
  run|batch`; `search`, `candidates`, `frame get|neighbours|burst`, `crop`, `audio`; `session
  list|status|renew|close|retain|clean|init-workspace`; `bundle validate`.
- `handoff check`: a draft from stdin or `--file`. A release build's `--version` names its source commit. Global `--session-root`, `--host-isolation`, `--json`, `--events jsonl`;
  readable text without `--json` (unstable).

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00-P05 | Complete; merge commits and evidence are in the ledger |
| P06-P10 | Complete: detect, select, verify, guide (`b73df52`); engine, transcripts, local ASR (`9ea3180`); search, candidates (`b830fc9`); frames, crops, audio (`e57c706`); jobs, resume, durable Ubuntu/ext4 (`3f27ce3`) |
| P11 | Complete (`40c4038`); SEC-T01 adversarial evidence is technical debt, moved to R1 (#188, L-068) |
| P12 | Complete (2026-09-30, ADR 0022 Accepted): skill, harness, named-client trials; review tier qualified, compact tier 93% and 100% on the #222 re-run; open: L-095 (#224), #219, #204 (`1284e54`) |
| P13 | Complete (2026-10-01, ADR 0023 Accepted): distribution, managed install, `handoff check`, human output; 0.1.0 published; release commit `011bc4d` |
| P14 | **Complete (2026-10-10, ADR 0024 Accepted): the R0 qualification; `0.2.0` published 2026-10-09 (`eeb2a22a46a8`); the rung still `candidate`; three findings came after it (#353, #359 high from real recordings; #391 medium from a second machine), all fixed in the published candidate `0.2.1-rc.3`, whose evidence is recorded (six items waived, RQ-13 running); no `0.2.1` exists yet** |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <- `vsift-infrastructure`
(OS, processes, storage, providers, parsers) <- `vsift` (engine) <- `vsift-cli` (parse, present, signals).
`vsift-contract` sits beside the engine and owns the wire types. The skill only calls the `vsift` binary;
the trial harness uses only `vsift` and `vsift-contract`; `tools/` (never shipped) qualifies; `npm/` is the launcher.

## Quality evidence

- **Local gates** (in each PR description): fmt, strict Clippy, workspace tests, warning-denied rustdoc, governance. **CI on every PR:** Quality (three OS), Documentation, Governance, fuzz
  replay, worker boundary, dependency policy, CodeQL, npm launcher tests; the Release dry run and P14 workflows on their paths. **Required on `main`:** Quality, Documentation, Dependency
  policy, Analyze Rust, Governance. Squash merges.
