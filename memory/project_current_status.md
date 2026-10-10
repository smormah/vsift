# VSift current status

As of 2026-10-10. Current-state document: rewrite it, don't append to it. Next actions and
open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives anyone, and their AI assistant, local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is also an
embeddable engine library (`vsift`) that the CLI, and later other hosts, use. Today it can:
- check and register its dependencies, show a read-only setup plan, report whether local speech
  recognition really works here (`setup check`), and on Ubuntu 24.04 x86-64 install the reviewed
  FFmpeg, whisper.cpp and model itself, then list, roll back, remove and repair them (the store
  survives kills and, on Ubuntu 24.04 with ext4, power loss);
- copy a video into a private, disposable session; import an SRT or WebVTT transcript with it,
  aligned by an offset; or transcribe the speech itself with whisper.cpp (`transcript
  retranscribe`), continue an interrupted transcription, and report, resume or cancel that work (**known defect in the published `0.2.0`, high: a real recording whose 30-second chunk has little speech
  and one rejected segment fails the whole run and keeps nothing, #353, L-145; fixed on `main`, ships in `0.2.1`, not released**);
- return timestamped transcript segments, search them, list the moments where the screen changed,
  and return exact frames, neighbours, bursts, crops and WAV clips;
- manage the session's lifetime and retention, validate retained bundles, stop a long command
  cleanly on Ctrl-C or `SIGTERM`, and keep every folder it creates private;
- run as a worker under an external supervisor (`session init-workspace`, `job run`, `job batch`),
  each step at most once per operation id; check an agent's draft report (`handoff check`); refuse
  to claim strict worker isolation unless the Linux kernel attests it;
- be installed without Rust: **`0.2.0` is published under npm's `latest` (2026-10-09), so `npm install
  --global vsift-cli` with no tag installs it**; the 0.1.0 pre-release and three release candidates stay
  published (`next` names `0.2.0-rc.3`; `0.2.0-rc.1` and `0.2.0-rc.2` are deprecated; a candidate of `0.2.1`, once
  published, moves `next` and leaves `latest` at `0.2.0`). Archives on the GitHub release
  carry a Sigstore attestation and npm carries provenance. The README shows 0.1.0's real output
  (graphics: L-121).

**The agent skill** (`skills/vsift/`) teaches Claude Code or Codex to run an investigation with
the CLI and write a cited report. P12's named-client trials qualified it:
- **Review tier:** Claude Opus 5.5 in Claude Code and GPT-6-Astra in Codex (a Linux container, L-076) each passed
  11 of 11 trials mechanically and 9 of 11 fully. **Compact tier:** Claude Sonnet 5.5 and GPT-6-Sol: 82% in P12;
  the re-run (#222) meets the 90% target, Sonnet 26 of 28 and Sol 28 of 28 (23 as run).
- **P14 refined it on clean installs of the candidates.** Claude Opus missed two review-tier gates on rc.1 and rc.2
  (L-139); the third candidate's skill has two more evidence rules and on 2026-10-08 all four models met every gate
  (34 of 34 runs; three runs per scenario, so a small sample, not a proof). **Without the skill (batch 3, rc.3): no cold
  agent installed anything or accepted a plan; one of 18 read an audio clip file with `base64`, which the hard gate
  counts, and RQ-16 is waived for that one action (#340, L-142).** `0.2.0` ships the same skill byte for byte.

## Where the project stands

**P00-P14 are complete (the whole of R0's work packets, recorded 2026-10-10).** What that means, exactly:
- **The release `0.2.0` is published and verified.** Tag `v0.2.0` at `eeb2a22a46a8` (the merge of #344), Release run 37946261087, 2026-10-09; `latest` is `0.2.0` and `next` is `0.2.0-rc.3` on all four npm packages;
  the GitHub release is not a pre-release and is GitHub's latest. It is the third candidate's source with another version number (a program checked that), so the candidate's campaigns and trials carry to it. The
  first move of `latest` worked as designed (L-105). Nothing is announced.
- **Run on `0.2.0`'s own bytes (2026-10-09, all green):** clean installs with npm, pnpm, Yarn and Bun on Windows, macOS and Ubuntu (hosted), the three archives, the offline install, upgrades from `0.1.0` and from
  `0.2.0-rc.3`, the journeys on three systems (54 stages passed and 1 blocked as the per-system rule says), the managed smoke, and the Governance job. The credential-free `P14 verify release` was red on exactly its two
  unregistered stable checks (by design) and green after P14 PR 13a registered them (RQ-19).
- **Four of the twenty evidence items are `waived`, none is a pass:** RQ-10 (a symbolic link answers `STORAGE_IO`, #265), RQ-14 (the strict worker is not claimed to contain a hostile decoder, decision E), RQ-16 (one
  cold read of an audio clip, #340) and RQ-17 (the maintainer's Smart App Control, clean-machine and Mac try-outs were not done: the release ships untried, L-143). The checker cannot see their limits; the decisions' texts do.
- **Not done or not shown:** RQ-13 (the scan reading on `0.2.0`, complete, 2026-10-10) is `passed` with one new finding that the maintainer accepted for R0 and will have fixed after the release: CVE-2026-107678 (Medium: the MP4
  demuxer's `pssh` handling, no fix in the shipped FFmpeg snapshot, nothing run on a crafted file; L-144, #351); the Ubuntu and Windows ASR gates and the durable path were not repeated on the release's tag; the
  claims rung is still `candidate`, no platform is "supported", and 79 of the 133 register entries are `pending` review (no public statement leans on the pending ones).
- **The neutral checkpoint for using the published CLI (plan section 12) was met when P14 was recorded complete, and the trial has started.** The maintainer installed `0.2.0` on their own machine on 2026-10-10,
  copied the skill, byte for byte, into their own agent client's folder and is testing real recordings. That use never counts as qualification evidence. **Section 12's condition (no open high-severity limit
  blocks the investigate-a-video journey) is not true until `0.2.1` is published**, because L-145 stays open until then; the plan's text, which names L-068 as the only high entry, is not edited by this change.
- **The first real recording found a high-severity defect on 2026-10-10: #353, L-145. It is fixed on `main` (#358, `510637e`, after four independent review rounds) and not released; `0.2.0` is still affected.**
  A 34-minute screencast with long pauses failed `transcript retranscribe` as `MISSING_CAPABILITY` and kept nothing: one 30-second chunk with few recognised segments, one of them rejected, tripped the quarter rule,
  and the application had no outcome for an unusable chunk but failing the run; `job resume` failed the same way, the job stayed `resumable`, and the error's remediation was #274's, which was wrong here. **On `main`
  an unusable chunk is a recorded gap and the run answers `partial`, listing the stretch as not transcribed; the quarter rule tolerates one rejected segment in a chunk of fewer than four; the failure that remains
  (most answered chunks unusable) says which reason, how many chunks and where, and the job is not resumable.** This is shown by tests on stand-in recordings, not on the recording that found it (it cannot be committed).
- **The patch release `0.2.1` (the maintainer's decision of 2026-10-10: it carries everything waiting). The code fixes are merged to `main`; the first candidate, `0.2.1-rc.1`, is CUT and NOT TAGGED OR PUBLISHED; the
  release is not made.** Merged to `main`: #353 (above), #340 and #342 (`audio`'s help and readable result say an agent cannot listen to a clip, which left the JSON as it was; the evidence commands' `BUSY` has a retry hint and a
  remediation) and #345 (a flaky Windows test of the trial harness, the harness itself unchanged). **The cut (one pull request, `docs/planning/release-0.2.1.md` is its plan) adds:** the version `0.2.1-rc.1` in the five files that hold
  it; the skill change of #349 with the partial-transcript, `BUSY` and audio-clip rules (applied unchanged) and the fuzz seed copy that follows it; **a new agent-trial freeze** (the skill's digest is new, the other six components are the
  third candidate's); the guide's marker `0.2.1`; the runbook `release.md` 6.14, which is the first for a candidate published while `latest` is a stable version (`0.2.0`, not the empty placeholder: three places assumed it and are
  corrected, `install.md` section 7, 6.4's comment and the 404 once expected from GitHub's latest release, now `v0.2.0`; the tools and the workflow read the registry as it is, and new tests hold that); and public text changed only where
  `next` naming the candidate would be false or misleading. **Increment done: the cut. Not done:** the merge and publish (the supervisor tags and dispatches, the maintainer approves the `release` environment), the hosted evidence at the
  tag, **the agent batches 2 and 3 under the new freeze (the maintainer's go and allowances)**, the decisions on the four waivers (the completeness check does not name waived items, so none is asked for by itself), the stable cut and its
  publish. **P14 stays complete; `0.2.0` stays published, is not deprecated and has the defect:** the README and the install guide say so and give the workarounds (a supplied transcript, or ranges that avoid the failing stretch).
  Every accuracy figure so far is from a synthetic corpus with dense speech (L-022); this is the first real one. The fix's own decisions left three limits for the register (L-146, #361; L-147, #362; L-148).
- **What is weak:** **the `0.2.1` skill is untried and no trial scenario exercises its new partial-transcript rule**; `release.md` 6.14 has not been run; the third candidate's skill change met its gates once on a small sample; the cold round met its usefulness target twice with no margin and its safety gate not at all (one harmless read; no person has read the raw cold
  logs, L-118); hosted images are not clean machines (L-112); Smart App Control and the macOS prompts are unseen (L-098, L-143); **a synthetic corpus and voice only** (L-020, L-022), which the first real recording showed to matter (#353; a small freely licensed real-media test set is #363). CVE-2026-38350 (L-122), the
  whisper.cpp pin (L-137) and CVE-2026-107678 (L-144) are the maintainer's accepted residuals. Open defects: **#353 (L-145, high: real recordings with pauses; fixed on `main`, not released)**, #312 (L-135; one failure in 4,500 loaded Windows repetitions), #351; #340 and #342 are fixed on `main`, not released.

## How P14 went, in one view (details: `docs/planning/p14-qualification.md`, sections 15 to 31)

- **PR 1 to 3 and 8 (the machinery):** the evidence ledger (RQ-01 to RQ-20) and the claims registry, checked on every PR (L-101); clean installs, archives, offline install and upgrade from the real registry; the
  journeys on the published binary; and a release path where the version alone decides the channel (a suffix means `next`, none moves `latest`).
- **PR 4 (the campaigns, on 0.1.0)** found what PR 7 fixed: fuzzing (31 targets), stress, load, malicious media, the worker runbook, the scan reading. **PR 7 (findings by outcome; one pull request and one regression
  test each; rule of 2026-10-04: a published failure code stays, the remediation carries the fix, L-127):** fixed #264, #265's answer, #266's answer, #274, #277, #268, #256, #261, #273, #282, #283, #285, #310,
  #314, #322, #325 step 1, #332; narrowed, not proven gone: #206, #253, #312; **not fixed: #272** (the FFmpeg snapshot: 46 of 47 records fixed in the shipped build, one tied by elimination, accepted; a re-pin must be a
  month-end build, L-132).
- **PR 6 (the trial harness) and the batches:** batch 1 (0.1.0) a baseline; batch 2 (with the skill) and batch 3 (cold) on the candidates; the freeze is by digest and a change voids a batch.
- **PR 9 and 10:** the support matrix (no cell may say "supported"), the install guide, the user guide with two CI checks, the register sheet; three candidates published (rc.1 2026-10-05, rc.2 2026-10-07, rc.3
  2026-10-08), each cut for a failed gate or a fix, never announced.
- **PR 11 (the candidates' evidence) and the maintainer's decisions:** every item `passed` or `waived` for rc.3; the decisions of 2026-10-08 to 2026-10-10 (RQ-10's rule widened and its waiver carried to the release,
  RQ-16's and RQ-17's waivers carried, deprecation at the stable, the register pass: 46 entries `accepted`, the cold logs' reading by the supervisor stands for the maintainer's).
- **PR 12 (the release, the maintainer's publish) and PR 13 (the ledger follow-up, 13a #348 and 13b):** the release commit, the publish, the deprecation, `0.2.0`'s own evidence, the ledger's `release_delta`, the
  pages that named the candidate, P14 `complete` and ADR 0024 Accepted. **Decided in 13b for the maintainer to confirm by merging:** the rung stays `candidate`, P14 `complete`, ADR 0024 Accepted, the claims-registry edits (plan 31.5); the maintainer decided in chat that CVE-2026-107678 is accepted and fixed after the release.

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
| P14 | **Complete (2026-10-10, ADR 0024 Accepted): the R0 qualification; `0.2.0` published 2026-10-09 (`eeb2a22a46a8`); four items waived, one accepted FFmpeg finding (L-144, #351), the rung still `candidate`; a high finding came after it (#353, L-145), fixed on `main` and not released: the first candidate of `0.2.1` is cut, not published** |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <- `vsift-infrastructure`
(OS, processes, storage, providers, parsers) <- `vsift` (engine) <- `vsift-cli` (parse, present, signals).
`vsift-contract` sits beside the engine and owns the wire types. The skill only calls the `vsift` binary;
the trial harness uses only `vsift` and `vsift-contract`; `tools/` (never shipped) qualifies; `npm/` is the launcher.

## Quality evidence

- **Local gates** (in each PR description): fmt, strict Clippy, workspace tests, warning-denied rustdoc, governance. **CI on every PR:** Quality (three OS), Documentation, Governance, fuzz
  replay, worker boundary, dependency policy, CodeQL, npm launcher tests; the Release dry run and P14 workflows on their paths. **Required on `main`:** Quality, Documentation, Dependency
  policy, Analyze Rust, Governance. Squash merges.
