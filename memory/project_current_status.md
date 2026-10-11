# VSift current status

As of 2026-10-11. Current-state document: rewrite it, don't append to it. Next actions and
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
  retranscribe`), continue an interrupted transcription, and report, resume or cancel that work (**two known defects in the published `0.2.0`, both high: a real recording whose 30-second chunk has little speech
  and one rejected segment fails the whole run and keeps nothing, #353, L-145, fixed on `main` and not released; and, silently, speech can be missing where two overlapping 30-second pieces meet while the result says the whole range
  was transcribed, #359, L-149, fixed on `main` (#377), not released; both are planned for `0.2.1`**);
- return timestamped transcript segments, search them, list the moments where the screen changed,
  and return exact frames, neighbours, bursts, crops and WAV clips; **on `main`, in no
  published release (the Reads change): `--view brief` on `transcript get`, `search` and `candidates` (the same ids, times and text
  without what every item of a revision repeats, a quarter to two fifths of the size, about a fifth for a local transcription's
  segments; the default answer is unchanged) and a `transcript get` that states its coverage and is `partial` over a part
  nobody transcribed, as `search` is;**
- manage the session's lifetime and retention, validate retained bundles, stop a long command
  cleanly on Ctrl-C or `SIGTERM`, and keep every folder it creates private;
- run as a worker under an external supervisor (`session init-workspace`, `job run`, `job batch`),
  each step at most once per operation id; check an agent's draft report (`handoff check`); refuse
  to claim strict worker isolation unless the Linux kernel attests it;
- be installed without Rust: **`0.2.0` is published under npm's `latest` (2026-10-09), so `npm install
  --global vsift-cli` with no tag installs it**; the 0.1.0 pre-release and three release candidates stay
  published (`next` names `0.2.1-rc.1`, published 2026-10-10 and superseded, and leaves `latest` at `0.2.0`; `0.2.0-rc.1` and `0.2.0-rc.2` are deprecated). Archives on the GitHub release
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
- **The release `0.2.0` is published and verified.** Tag `v0.2.0` at `eeb2a22a46a8` (the merge of #344), Release run 37946261087, 2026-10-09; `latest` is `0.2.0` (and `next` was `0.2.0-rc.3` on all four npm packages until `0.2.1-rc.1` was published);
  the GitHub release is not a pre-release and is GitHub's latest. It is the third candidate's source with another version number (a program checked that), so the candidate's campaigns and trials carry to it. The
  first move of `latest` worked as designed (L-105). Nothing is announced.
- **Run on `0.2.0`'s own bytes (2026-10-09, all green):** clean installs with npm, pnpm, Yarn and Bun on Windows, macOS and Ubuntu (hosted), the three archives, the offline install, upgrades from `0.1.0` and from
  `0.2.0-rc.3`, the journeys on three systems (54 stages passed and 1 blocked as the per-system rule says), the managed smoke, and the Governance job. The credential-free `P14 verify release` was red on exactly its two
  unregistered stable checks (by design) and green after P14 PR 13a registered them (RQ-19).
- **Four of the twenty evidence items are `waived`, none is a pass:** RQ-10 (a symbolic link answers `STORAGE_IO`, #265), RQ-14 (the strict worker is not claimed to contain a hostile decoder, decision E), RQ-16 (one
  cold read of an audio clip, #340) and RQ-17 (the maintainer's Smart App Control, clean-machine and Mac try-outs were not done: the release ships untried, L-143). The checker cannot see their limits; the decisions' texts do.
- **Not done or not shown:** RQ-13 (the scan reading on `0.2.0`, complete, 2026-10-10) is `passed` with one new finding that the maintainer accepted for R0 and will have fixed after the release: CVE-2026-107678 (Medium: the MP4
  demuxer's `pssh` handling, no fix in the shipped FFmpeg snapshot, nothing run on a crafted file; L-144, #351); the Ubuntu and Windows ASR gates and the durable path were not repeated on the release's tag; the
  claims rung is still `candidate`, no platform is "supported", and 91 of the 145 register entries are `pending` review (no public statement leans on the pending ones).
- **The neutral checkpoint for using the published CLI (plan section 12) was met when P14 was recorded complete, and the trial has started.** The maintainer installed `0.2.0` on their own machine on 2026-10-10,
  copied the skill, byte for byte, into their own agent client's folder and is testing real recordings. That use never counts as qualification evidence. **Section 12's condition (no open high-severity limit
  blocks the investigate-a-video journey) is not true until `0.2.1` is published**, because L-145 and L-149 stay open until then; the plan's text, which names L-068 as the only high entry, is not edited by this change.
- **Real recordings found two high-severity transcription defects on the first day of testing (2026-10-10), and `0.2.0` has both.** (1) **#353, L-145 (loud):** a 34-minute screencast with long pauses failed `transcript retranscribe` as `MISSING_CAPABILITY` and kept
  nothing: one 30-second chunk with few recognised segments, one of them rejected, tripped the quarter rule, and the application had no outcome for an unusable chunk but failing the run. **Fixed on `main` (#358, `510637e`, after four independent review
  rounds), not released:** an unusable chunk is a recorded gap and the run answers `partial`, listing the stretch as not transcribed; the failure that remains (most answered chunks unusable) says which reason, how many chunks and where. Shown by
  tests on stand-in recordings, not on the recording that found it (it cannot be committed). (2) **#359, L-149 (silent):** merging the overlapping 30-second pieces can drop speech at a seam with no warning, and `search` and the result
  still say the whole range was transcribed (six holes listed on a 17:34 walkthrough, two of them checked against the segments' fields, and one missing word on the 34:36 lecture; nobody has listened to the audio). The cause is **inferred** from the
  source and timings (rule 2 of `merge_chunks` drops a later chunk's whole first segment when an earlier segment spans its midpoint); the raw recogniser output has not been seen. **The fix is on `main` (#377, below), not released.** The merge code is
  unchanged since `0.2.0`, so `0.2.1-rc.1` has it too. The README and the install guide say so and give the workarounds. Every accuracy figure before these came from a synthetic corpus with dense speech (L-022).
- **The patch release `0.2.1` (decided 2026-10-10, widened 2026-10-11). Its first candidate `0.2.1-rc.1` is PUBLISHED and SUPERSEDED; the second, `0.2.1-rc.2`, has the Answers and Reads changes on `main` and the rest not started; the release is not made.** `docs/planning/release-0.2.1.md` is the plan.
  **Published 2026-10-10** (tag `v0.2.1-rc.1` at `cb8cb3a`, the merge of #364; Release run 38084137607): `next` is `0.2.1-rc.1` on all four packages and `latest` is still `0.2.0` (`P14 verify release`, run 38086846526, passed). It carries #353's fix,
  #340 and #342 (`audio`'s help and readable result say an agent cannot listen to a clip, the JSON unchanged; the evidence commands' `BUSY` has a retry hint and a remediation), #345 (a flaky Windows test of the trial harness), the skill change of #349 with the
  partial-transcript, `BUSY` and audio-clip rules and **a new agent-trial freeze**, the guide's marker `0.2.1` and the runbook `release.md` 6.14 (the first for a candidate published while `latest` is a stable version). **Its hosted evidence round was
  stopped on purpose** (on the maintainer's decision of 2026-10-11 to put #359 into `0.2.1`, which makes rc.1's evidence obsolete, the supervisor session stopped the hosted-evidence agent and cancelled the campaign runs it had dispatched for rc.1): of 14 runs 6 finished green (`P14 verify release`, `P13 managed smoke`, `P14 compatibility`, `P14 scan reading`, `P14 runbook walk`, `P14 malicious media`) and 8 were cancelled (published artifacts, journeys, fuzz, stress, load, local ASR, managed
  power loss, durability); none is recorded in the ledger as evidence, and **agent batches 2 and 3 were not run on it.** **On 2026-10-11 the maintainer decided to put #359 into `0.2.1`, which changes the transcription code, so rc.2 is cut and rc.1 is superseded;** the
  larger findings ride along (#365 item size of `transcript get` and `search`, #366 `handoff check` of a long report on Windows, #367 slow and dense `candidates`, #368 a whole-recording question, #370 `frame get` `BUSY` during a transcription) and the small ones
  (#369, #371, #372, #373); #374 only if verified. **The designs of #365, #373 and the brief part of #367 were decided by the maintainer on 2026-10-11 and are built in the Reads change (below); #368 is out of scope for this line; the other designs are not decided.** The register has L-149 to L-154 for #359, #365, #366, #367, #368 and #370 (all `pending`). **rc.2 needs:** the changes with regression tests, a new skill and freeze, the hosted evidence once, agent batches 2 and 3 once
  (the maintainer's go and allowances), and the decisions on the four waivers (the version stays `0.2.1`: decided 2026-10-11). **Increments done:** the fix of #353 on `main`, the publish of rc.1; #359's fix (#377) and the Answers pull request (#378) are on `main` (below). **The Reads change is on `main` and in no release:** #365 and the brief part of #367
  (`--view full|brief`; the brief answer is projected from the full one, never leaves out a gap, keeps `carried_from` on a carried segment, and is refused with `--events jsonl`, whose records stay complete) and #373 (`transcript get` always states `transcript_coverage`, by `search`'s one function,
  computed in the application and given to a library host as `excerpt.coverage()`, and is `partial` over a gap; a complete read is the published `0.1.0` answer plus that member; a range retranscription after a first run with few or no segments leaves false gaps and loses its silence, L-033; and a read over a stretch where the recogniser wrote words that no segment holds, `local_asr.unplaced_ranges` of #377, is `partial` too and lists it, a fourth cause, pinned by a contract test).
  **Measured** by repeating the committed examples' items (and a search's hits) to a page of 50, compact JSON (method in `cli-v1.md`): transcript segments of a local transcription 51,618 to 9,883 characters (19.1%), with a carried segment 57,256 to 15,571 (27.2%), imported cues 35,511 to 9,251 (26.1%),
  search 36,702 to 11,567 (31.5%), candidates 31,980 to 12,718 (39.8%); the design's "under a quarter" is met only for the first, and the figures are for revisions with no `unplaced_ranges`: the brief view keeps the revision block whole, so a revision that lists many (up to 1,024, about 45 KB, L-159) is not a fraction of the full page. It is shown by contract and binary tests on stand-in sessions and examples; **no real recording was read, no client was run, and the frozen skill cannot use the option.**
  **#365 and #367 stay OPEN:** the skill change with its freeze and a real run for #365; the first-call silence over 120 s and the list's density, with the maintainer's counts, for #367. Not built, by decision: a `candidates` filter, progress events or parallel
  windows, page filling for `session list`. **Not done:** the rest of rc.2 (the skill change in one commit with a new freeze, the cut, runbook 6.15, agent batches 2 and 3) and the release. **P14 stays complete; `0.2.0` stays published and is not deprecated.**
  The fix of #353's own decisions left three limits (L-146, #361; L-147, #362; L-148).
- **"Answers", one of the changes of the second candidate (PR 2 of the design in #376), is on `main` (#378) and in no release.** **#369:** `ingest` of a path that names no file, a folder
  (Windows) or a file the account may not read keeps `STORAGE_IO`, its message and exit 7 and now says which in `remediation` (three fixed texts, no path; the path after `--transcript` too; any other failure to open a path is as it was; a link,
  #265, is untouched). Shown by unit tests of the classification, the contract and CLI tests, three frozen examples with human snapshots, and the binary on Windows. **The refused-file CLI test and the folder test for Linux and macOS run only on
  Unix and were compiled and linted for Linux, not run.** **A folder differs by platform and always did:** Windows answers `STORAGE_IO` (now with a text), Linux and macOS `INVALID_SOURCE` with none (unchanged; the contract text had said
  `INVALID_SOURCE` for both, and now says what each does). **#372:** a test through the binary shows that an id of a superseded revision still resolves in `transcript get --revision`, `handoff check --session` and a retained bundle; no behaviour
  changed, and the skill's two sentences wait for the cut's one skill commit.
- **#359's fix is merged (#377) after two rounds of independent review; it is one increment of the second candidate, it is not released, and the finding is not closed.** **What works** (shown on synthetic layouts and stand-ins, not on a real recording): `merge_chunks` has a guard between rules 2 and 3 that restores dropped segments, whole, until no stretch longer than 0.5 s of the union of all the chunks' segments is in no kept segment (a property over generated layouts against a frozen copy of the old merge, mutations of the guard caught); a revision lists the stretches where words that no segment holds may lie (`local_asr.unplaced_ranges`: an upper bound from a rejected segment's start to the next segment's start, cut to the chunk's window; no word, an empty range or nothing after it is only counted), `search` counts them as untranscribed and the run stays `complete`. **Known issues, registered as L-155 to L-160 (issues #380 to #384 for L-155 to L-159; none for L-160, which is a cost of the design and has no work item):** the guard judges by time, so it trades holes for repeats (in a reviewer's synthetic simulation, which is not in the repository, lost words fell by about 98 per cent and repeated words rose several-fold); up to half a second can still be missing, and the value is derived, not measured; the repeat rule looks at one pair per seam, can drop a copy that carries new time, and the guard can expose a segment to it that the old merge kept; some rejected kinds are only counted; a stretch is an upper bound and retranscribing it may give the same one (`ingest --transcript` is the sure remedy); one new optional record member means that a release before this one reads a revision holding it as damaged: if the newest revision holds it nothing of the session can be read by `0.2.0` or `0.2.1-rc.1`, and if only an older one does the newest reads normally and the older ones are refused by id, in `handoff check --session` and in `bundle validate`. **Open:** the check on a real recording (the opt-in stage `p07_local_asr_multi_chunk_seam` was not run), the listener's check in #359, the follow-ups #380 to #384, and the release. The README and install-guide notes for L-149 stay until the release.
- **What is weak:** **the `0.2.1` skill is untried, will change again, and no trial scenario exercises its partial-transcript rule or a whole-recording question**; `release.md` 6.14 ran once (rc.1) and what it showed is not recorded here; the third candidate's skill change met its gates once on a small sample; the cold round met its usefulness target twice with no margin and its safety gate not at all (one harmless read; no person has read the raw cold
  logs, L-118); hosted images are not clean machines (L-112); Smart App Control and the macOS prompts are unseen (L-098, L-143); **a synthetic corpus and voice only** (L-020, L-022), which the real recordings showed to matter (#353, #359; a small freely licensed real-media test set is #363); **the live testing is one person's, on two recordings, on Windows with one client, and was not repeated on rc.1**. CVE-2026-38350 (L-122), the
  whisper.cpp pin (L-137) and CVE-2026-107678 (L-144) are the maintainer's accepted residuals. Open defects: **#353 (L-145, high; fixed on `main`, not released), #359 (L-149, high; fixed on `main` by #377, not released)**, #365 to #374 (L-150 to L-154 and the small ones; #365, #367 and #373 built in part or whole on `main`), #312 (L-135; one failure in 4,500 loaded Windows repetitions), #351; #340 and #342 are fixed on `main`, not released; #369 is fixed and #372 pinned by a test in the Answers pull request (merged, #378).

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
| P14 | **Complete (2026-10-10, ADR 0024 Accepted): the R0 qualification; `0.2.0` published 2026-10-09 (`eeb2a22a46a8`); four items waived, one accepted FFmpeg finding (L-144, #351), the rung still `candidate`; two high findings came after it from real recordings (#353, L-145, fixed on `main`, not released; #359, L-149, fixed on `main` by #377, not released): `0.2.1-rc.1` is published and superseded, `0.2.1-rc.2` is planned** |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <- `vsift-infrastructure`
(OS, processes, storage, providers, parsers) <- `vsift` (engine) <- `vsift-cli` (parse, present, signals).
`vsift-contract` sits beside the engine and owns the wire types. The skill only calls the `vsift` binary;
the trial harness uses only `vsift` and `vsift-contract`; `tools/` (never shipped) qualifies; `npm/` is the launcher.

## Quality evidence

- **Local gates** (in each PR description): fmt, strict Clippy, workspace tests, warning-denied rustdoc, governance. **CI on every PR:** Quality (three OS), Documentation, Governance, fuzz
  replay, worker boundary, dependency policy, CodeQL, npm launcher tests; the Release dry run and P14 workflows on their paths. **Required on `main`:** Quality, Documentation, Dependency
  policy, Analyze Rust, Governance. Squash merges.
