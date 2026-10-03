# Incremental end-to-end test spine

Status: P04, P05 and P06 checkpoints, the P07 supplied-transcript and local-ASR
stages, the P08 search and visual-candidates stages, the P09 evidence-navigation
stages, the P10 recoverable run and the P11 single-host worker run are implemented.
P12's complete video-to-grounded-handoff run passed through named Claude Code and
Codex clients (2026-09-30, [P12 qualification record](p12-agent-qualification.md)).
The review tier is qualified; the compact tier met its 90% target on the re-run after P12's
fixes (#222, 2026-09-30: 93% and 100%). P13's installed-user and managed-dependency run is
implemented and has run mechanically: the managed-dependency stage passed on a hosted
Ubuntu 24.04 runner (2026-09-30, `install-e2e`, run 36793180858) and the installs of the
release packages with npm, pnpm, Yarn and Bun run in the Release workflow's twelve-job
matrix; the named-agent run from a clean install belongs to P14 (ADR 0023 decision H10).
The 0.1.0 pre-release was published on 2026-10-01 and installed once from the real
registry on Windows 11 with npm ([P13 record](p13-distribution.md), "First publish"); no
agent has used it.
Managed
installation moved from P06 to P13 under
[ADR 0015](../decisions/0015-r0-delivery-replan.md). Tracking issue: [#40](https://github.com/smormah/vsift/issues/40).

## Purpose

VSift must not wait until release qualification to discover that independently tested
components do not form a usable video investigation. Starting with P04, each packet
connects its production capability to one cumulative, opt-in test journey. The journey
grows with the product while the delivery ledger continues to control implementation
eligibility.

This framework is not evidence that an unimplemented stage works. A checkpoint reports
each stage as `passed`, `failed`, `blocked`, or `not_implemented`; it never replaces a
missing stage with a mock and calls the complete journey successful.

## Governing journeys

Two journeys converge on the same grounded evidence result:

- **A-08 / local ASR:** start with a synthetic local video and no transcript; inspect
  dependencies, extract audio, run the qualified local whisper.cpp adapter, search the
  timestamped transcript, refine visual evidence and validate every source reference.
- **A-09 / supplied transcript:** start with the same video and a valid supplied
  transcript; prove ASR is skipped, then exercise dependency degradation and unreadable
  visual handling without fabrication or silent installation.

The mechanical runner validates tool behavior and citations independently of model
prose. P12 adds bounded trials through named Codex and Claude Code clients; a mocked
agent cannot satisfy those trials.

## Incremental attachment points

| Packet | Required addition to the cumulative journey | First useful checkpoint |
| --- | --- | --- |
| P04 | Generate the approved synthetic media; bind its identity; run real FFprobe and bounded FFmpeg frame/audio operations | Video-to-timestamped-media artifacts |
| P05 | Open a disposable session, publish artifacts, and prove explicit retain/cleanup behavior | Repeatable session-scoped media run |
| P06 | Check preinstalled/partial/off-PATH tools, verify the selected FFmpeg/FFprobe against F01, and exercise typed manual recovery on denied/offline/unqualified paths without agent overreach | Fresh/BYO dependency run |
| P07 | Attach supplied-transcript and local-ASR paths to the same fixture truth | Video-to-timestamped-transcript run |
| P08 | Produce bounded visual candidates and transcript search results with coverage metadata | Video-to-searchable-candidates run |
| P09 | Refine exact frames, neighbours, crops and audio; validate requested versus actual timestamps and lineage | Complete mechanical video-to-evidence run |
| P10 | Kill and resume at stage/commit boundaries without accepting corrupt evidence | Recoverable mechanical run |
| P11 | Exercise finite batch, admission, cancellation and structured result behavior | Single-host worker run |
| P12 | Run A-01..A-09 through named Codex and Claude Code clients | Complete video-to-grounded-handoff run |
| P13 | Repeat from clean native/npm installation without a Rust toolchain, including one qualified explicit managed dependency install | Installed-user and managed-dependency run |
| P14 | Execute the supported release matrix and preserve reviewed evidence | R0 release qualification |

Every P04-P13 completion record must identify the attached stage and its checkpoint
evidence, or explicitly state why that packet has no applicable attachment. A unit or
component integration test remains necessary even when the cumulative journey passes.

## Execution model

The P04 source/media checkpoint remains:

```console
cargo test -p vsift-infrastructure --locked --test p04_media_e2e -- --ignored --nocapture
```

P05 adds a cumulative source/media-to-disposable-session checkpoint:

```console
cargo test -p vsift-infrastructure --locked --test p05_session_e2e -- --ignored --nocapture
```

It uses project-owned F01 media, real FFprobe/FFmpeg operations, committed
frame/audio artifacts, evidence-only and source-inclusive retained bundles,
explicit close/cleanup and source-preservation checks. It records a bounded
JSON report under `.vsift/e2e-runs/<run-id>/report.json` and leaves P06-P14
and the complete journey `not_implemented`, because it does not run them. The
P04 checkpoint still covers seven source/media scenarios. See the
[P04](p04-media-qualification.md) and [P05](p05-session-qualification.md)
qualification records.

P06 adds the dependency detect/select/verify/guide checkpoint:

```console
cargo test -p vsift-cli --locked --test p06_setup_e2e -- --ignored --nocapture
```

It needs FFmpeg and FFprobe on `PATH` and deliberately does not need whisper.cpp.
Setup journeys drive the compiled `vsift` binary with an isolated per-user
configuration base: tools found on `PATH`; off-`PATH` tools selected per call or
persisted; whisper missing (degraded, typed remedy naming the supplied-transcript
route); no media tools (blocked, headless single JSONL record); the host target's
plan (manual guidance with no actions or digest on unqualified targets, reviewable
but uninstallable actions on Ubuntu 24.04 x86-64); and a denied configuration write
(typed `STORAGE_IO`, record unchanged). The last journey reads the configured pair
back from storage and runs `FixtureMediaToolVerifier` on it, then shows that FFmpeg
standing in for FFprobe fails at the probe check. Missing FFmpeg/FFprobe makes the
affected journeys `blocked`, and the test fails unless every journey passed. It
writes `.vsift/e2e-runs/p06-<run-id>/report.json` and leaves P07-P14 and the complete
journey `not_implemented`. Offline behaviour is inferred rather than sandboxed: no
P06 command opens a network connection while `setup install` stays reserved (since P13 PR 4
it is implemented; the P13 stage below exercises it).

P07 adds the supplied-transcript stage of A-09:

```console
cargo test -p vsift-cli --locked --test p07_transcript_e2e -- --ignored --nocapture
```

It needs FFmpeg and FFprobe on `PATH`, registers them with `setup configure` in an
isolated per-user base and then runs every command with an empty `PATH`, so
whisper.cpp is absent and `setup check` reports it missing: the journey proves the
supplied-transcript path never needs local ASR. It imports the F10 video with
`fixtures/corpus/transcripts/F10.srt` and, separately, the equivalent `F10.vtt`, both
with the explicit `--transcript-offset 500000`, then cites F10-E01's frozen truth window
with `transcript get` and requires exactly one segment saying dialog R-17 on exactly
that window, with unknown confidence. A third journey imports with a wrong offset and
requires a typed `INVALID_ARGUMENT` naming `no_cues_within_source` and no open session.
Missing tools make the journeys `blocked`. It writes
`.vsift/e2e-runs/p07-<run-id>/report.json` and leaves P08-P14 and the complete journey
`not_implemented`; the local-ASR stage is the separate checkpoint below.

P07 increment 3b adds the local-ASR stage of A-08:

```console
VSIFT_TEST_WHISPER_CLI=<absolute whisper-cli path>
VSIFT_TEST_WHISPER_MODEL=<absolute ggml-base.bin path>
cargo test --release -p vsift-cli --locked --test p07_local_asr_e2e -- --ignored --nocapture
```

It runs on the P07 speech variants (`fixtures/corpus/generated/<id>-speech.*`), which
are generated from the frozen scripts by a test-only Kokoro workflow and verified
independently ([p07-speech-fixtures.md](p07-speech-fixtures.md)). Speech spans come
from `speech-provenance.json` and words from the frozen scripts in the manifest; no
expectation is taken from an ASR run. With an empty `PATH` and FFmpeg, FFprobe,
whisper.cpp and the model registered in an isolated per-user base, the stages are:
`p07_local_asr_setup`; `p07_local_asr_whole_file` (plain ingest of F05, then
`transcript retranscribe`, citing its speech window and words);
`p07_local_asr_bounded_revision` (a bounded rerun gives revision 2 with the earlier
segment carried, and revision 1 reads back unchanged with `--revision`);
`p07_local_asr_stream_and_bundle` (the `--events jsonl` stream, then `session retain`
and `bundle validate` with both records conforming to the bundle schema);
`p07_local_asr_f08_noise_spanish`; `p07_local_asr_f09_offset` (times anchored at the
0.75 s audio start); `p07_local_asr_multi_chunk_seam` (a two-chunk clip built at run
time from the speech utterances, every checked word heard exactly once);
`p07_local_asr_whisper_tripwire` (an F10 SubRip import succeeds with whisper
registered as a program that is not whisper); and `p07_local_asr_missing_model`
(typed `MISSING_CAPABILITY`, no revision). It prints `p07_local_asr: passed` when every
stage passed and writes `.vsift/e2e-runs/p07-local-asr-<run-id>/report.json`. A release
build is recommended because the 148 MB model is hashed three times per run; on
Windows 11 with the reviewed build the whole checkpoint took about 64 s (about 7.5 s
per short clip once verified). Missing tools or variables make every stage `blocked`.

P08 PR 1 adds the search stage of A-09, `p08_search_supplied`:

```console
cargo test -p vsift-cli --locked --test p08_search_e2e -- --ignored --nocapture
```

With an empty `PATH` and FFmpeg and FFprobe registered (whisper.cpp absent), it imports
F10 with `F10.srt` and `--transcript-offset 500000`, then searches for `R-17` and for the
spoken spelling `dialog r 17`. Both must find exactly the segment on F10-E01's frozen
truth window, as a phrase, with complete coverage whose basis is the supplied transcript;
the `--events jsonl` stream must carry the same record, and `transcript get` over the
truth window must cite it. It writes `.vsift/e2e-runs/p08-<run-id>/report.json` and
reports P09 and later stages `not_implemented`. On Windows 11 with FFmpeg 9.0
it passed on 2026-09-26 in about 4 s (each search about 90 ms through the binary).

P08 PR 4 adds the visual-candidates stages (V-02..V-05, S-11):

```console
cargo test --release -p vsift-cli --locked --test p08_candidates_e2e -- --ignored --nocapture
```

It needs FFmpeg and FFprobe on `PATH` (whisper.cpp only for the optional local-ASR
variant, through `VSIFT_TEST_WHISPER_CLI` and `VSIFT_TEST_WHISPER_MODEL`), registers
them in isolated per-user bases and runs the binary with an empty `PATH`. Stages:
`p08_candidates_fixtures` (F01-F10 and F12 through `candidates`, scored against the
manifest: every stable event of at least 1 s hit, no change candidate in F01 or F07,
a warm second call identical); `p08_lead_lag` (F03/F04/F05/F09 speech variants
imported with a SubRip cue written from the frozen script and speech placement; a
searched term's candidates within 10 s include one inside the event it names; and the
same after `transcript retranscribe` when whisper is set); `p08_candidates_budget` (a
3-minute clip built at run time, indexed by the engine library with a one-window
budget, continued from `not_analyzed` on each call, then read warm by the binary with
no tool); `p08_candidates_stream_and_bundle` (the JSONL stream, `session retain`,
`bundle validate`, the record conforming to its bundle schema);
`p08_candidates_malformed` (F11's damaged tail, a video truncated at run time, an
audio-only file); `p08_candidates_motion` (V-03 scroll-under-a-sticky-header and zoom
clips built at run time: a `settled_after_motion` candidate within 1 s of each stop and
a bounded count); `p08_candidates_s11` (a 30-minute session built at run time: cold
analysis and warm page times through the binary). Clips use FFmpeg's native MPEG-4
encoder, present in the pinned CI builds that omit libx264. It prints `p08_candidates:
passed` and writes `.vsift/e2e-runs/p08-candidates-<run-id>/report.json`. On Windows
11 with FFmpeg 9.0 and whisper.cpp v1.9.2 it passed on 2026-09-26 in 165 s; results
are in the [P08 candidate recall record](p08-candidate-recall.md).

P09 PR 3 adds the evidence-navigation checkpoint (V-01, V-07, V-08):

```console
cargo test -p vsift-cli --locked --test p09_evidence_e2e -- --ignored --nocapture
```

It needs FFmpeg and FFprobe on `PATH`, registers them in isolated per-user bases and
runs the binary with an empty `PATH`. Expectations come only from the frozen truth: the
independent `ffprobe` frame lists in `fixtures/corpus/generated/verification.json` and
FFmpeg's own decode of a fixture for pixel comparisons. Stages: `p09_frame_exact` (F01
at a keyframe, before one, between frames, the final frame, after it and at the end,
with both selection policies and a tight tolerance; F09's variable frame rate; the
rotated F01 variant at 720x1280, pixel-equal to FFmpeg's decode); `p09_candidate_frames`
(every candidate `candidates` returns for F01-F10 and F12, through
`frame get --candidate`, at delta 0 with its displayed dimensions);
`p09_neighbours_burst` (side stops at the start and end of the stream, 20 consecutive
neighbours each side, bursts of 0 and 101 frames refused by the grammar and of 1, 12
and 100 frames naming the truth's frames, a 60 s range clipped and 61 s refused with
the remediation to use `candidates`); and `p09_reuse` (repeats `reused` without a
write, two requests for one frame sharing one item and file, `full_hash` then
`identity` source checks).

P09 PR 4 completes the checkpoint (V-06, audio, malformed media, streams, bundles and
performance); run it with `--release` for the performance record:

```console
cargo test --release -p vsift-cli --locked --test p09_evidence_e2e -- --ignored --nocapture
```

Further stages: `p09_crop` (crops of the rotated variant equal to FFmpeg's decode pixel
for pixel, edges and one pixel past them, a crop of a crop in source pixels, F03's G18
cell green then red, a tiny glyph at native size); `p09_audio` (first-sample times 64 ms
and 750 ms, clipping at the end, no-audio, over-30 s and after-the-end refusals);
`p09_malformed` (F11's damaged audio, F11's truncated file and a clip cut short at run
time are `INVALID_SOURCE` with nothing committed); `p09_stream_and_bundle` (every
command's JSON Lines stream, then `session retain` and `bundle validate` with the
evidence records conforming to their bundle schema and holding no path); and `p09_perf`
(recorded, not gated: warm reuse, cold frames and a 12-frame burst on a 1080p clip of
about 1 GiB built at run time with the native MPEG-4 encoder and stream-copy loops,
128 MiB in a debug build or `VSIFT_TEST_P09_PERF_MB`, and the warm cost as the manifest
chain grows to 256 generations). It prints `p09_evidence: passed` and writes
`.vsift/e2e-runs/p09-<run-id>/report.json`.

The same checkpoint carries the **mechanical checkpoint** of the gates below:
`p09_mechanical_journey_supplied` and `p09_mechanical_journey_local_asr` each run one
continuous CLI journey over the F03 speech variant, one per transcript path (a SubRip
file written at run time from the frozen script and speech placement, or plain ingest
then `transcript retranscribe`, which needs `VSIFT_TEST_WHISPER_CLI` and
`VSIFT_TEST_WHISPER_MODEL` and is otherwise `blocked`): `search` for the critical term,
`candidates` within 10 s of the hit, `frame get --candidate` for a candidate inside the
critical event, `crop` of the changed cell, `audio` over the cited segment, then
`session retain` and `bundle validate`. Every citation is checked against the frozen
truth (segment inside the speech span, candidate and frame inside the event window,
crop region, size and colour, clip range and first sample) and the lineage (the frame's
request names the candidate, which the retained visual index holds; the crop's parent is
the frame). On Windows 11 with FFmpeg 9.0 and whisper.cpp v1.9.2 the release run passed
all eleven stages on 2026-09-26 in 471 s (the journeys 10.6 s and 24.0 s); the results
are in the [P09 qualification record](p09-evidence-navigation.md). The mechanical
checkpoint is met by these two stages.

P10 PR 3 adds the **recoverable mechanical run** (X-01..X-03, X-06, X-09 through the
binary):

```console
VSIFT_TEST_WHISPER_CLI=<abs> VSIFT_TEST_WHISPER_MODEL=<abs> \
  cargo test -p vsift-cli --locked --test p10_recovery_e2e -- --ignored --nocapture
```

It needs FFmpeg and FFprobe on `PATH` and whisper.cpp with its model, registers them in
isolated per-user bases and runs the binary with an empty `PATH`. Clips are built at run
time without re-encoding: F03's speech variant looped to 81 s (four 30 s chunks) and
F02 looped to 492 s (nine 60 s windows). Expectations come from the frozen truth (F03's
manifest entry and speech provenance) and from an uninterrupted control run. Stages:
`p10_local_asr_journey` (the P09 local-ASR journey on the looped clip: retranscribe as
the control run, `search`, `candidates`, `frame get --candidate`, `crop`, `audio`, then
`session retain` and `bundle validate` with every citation checked against the truth);
`p10_kill_and_resume` (a run killed once its first checkpoint exists is `interrupted`
in `job status`; the same command resumes it to the control segments; a committed
transcript artifact altered on disk is then `INTEGRITY_FAILURE`);
`p10_interrupt_and_job_resume` (Ctrl-C after the first checkpoint: `SIGINT` on Unix, a
console Ctrl-Break through `tools/send-console-ctrl.ps1` on Windows; `CANCELLED`, exit
6, the session and job named and `vsift job resume <job>` suggested, nothing committed,
no provider alive 10 s later; one checkpoint then altered, and `job resume` discards it
and commits the control segments); `p10_job_cancel_twice` (`job cancel` twice while a
run is live: the owner stops within the budget, the job ends `cancelled` without
checkpoints); `p10_operation_replay` (a run with `--operation-id` killed as its commit's
pointer moves is replayed by the same request without a new generation, and the same
id with another range is `IDEMPOTENCY_CONFLICT`; the fault-injection kill at the
pointer rename itself is in the storage kill test, because the release binary cannot
carry the `fault-injection` feature); and `p10_interrupt_candidates` (an interruption
halfway through a `candidates` call commits what it analysed and answers `partial`, and
a rerun completes to the control call's candidates). It prints `p10_recovery: passed`
and writes `.vsift/e2e-runs/p10-<run-id>/report.json`; results are recorded in the
[verification plan](verification.md) "P10 PR 3 evidence".

P11 PR 4 adds the **single-host worker run** (X-07, X-08, X-11, O-01, O-03, O-04
through the binary):

```console
VSIFT_TEST_WHISPER_CLI=<abs> VSIFT_TEST_WHISPER_MODEL=<abs> \
  cargo test --release -p vsift-cli --locked --test p11_worker_e2e -- --ignored --nocapture
```

It needs FFmpeg and FFprobe on `PATH` and whisper.cpp with its model, registers them in
isolated per-user bases and runs the binary with an empty `PATH` against ephemeral
worker workspaces, an input root and a bundle root. Clips are built at run time
without re-encoding. Stages: `p11_batch_mechanical` (one `job batch` of F03's speech
variant with recognition and candidates, F10 with its sidecar at +500 ms, F01 with a
closing retain, a malformed line and a path out of the input root: D5's exit 2, each
refused line reported alone; then `search`, `candidates`, `frame get --candidate` and
`bundle validate` on the batch's outputs, every citation checked against the frozen
truth and every bundle's manifest digest against the recorded one);
`p11_admission_ladder` (four candidate requests over a 180 s clip at concurrency 1, 2
and 4 in a four-unit workspace: the stream never has more requests in flight than the
concurrency, a sampler of the batch's provider processes never sees more weight than
the capacity, and each rung finds the same candidates); `p11_shutdown_and_redelivery`
(a batch with two long recognitions stopped mid-way by `SIGTERM` on Unix or a console
Ctrl-Break through `tools/send-console-ctrl.ps1` on Windows: exit 6 within 15 s, the
running requests `cancelled`, no provider left; `job resume` finishes one recognition,
redelivery continues the other, and the results equal an uninterrupted control; a
third delivery replays every line unchanged); and `p11_durable_workspace` (on Ubuntu
24.04 with ext4 a durable request runs and replays with `os_crash_durable`
publication; elsewhere the refusal is checked and the stage is `blocked`, required only
on that profile). It prints `p11_worker: passed` and writes
`.vsift/e2e-runs/p11-<run-id>/report.json`. On Windows 11 with FFmpeg 9.0 and
whisper.cpp v1.9.2 the release run passed on 2026-09-28 in 209 s; results are in the
[P11 qualification record](p11-worker-host.md). Two opt-in pieces from PR 3 remain
beside it: `every_step_runs_with_real_tools` (`job_run_cli_contract`) and the repeated
external-delivery simulation `repeated_external_delivery_commits_once`
(`external_delivery_stress`).

P12 PR 2 adds the **agent stage's machinery**: the trial harness
`tools/vsift-agent-trials` ([runbook](../agents/trials.md)) that prepares, runs, grades
and records A-01..A-09 and SEC-T02 through named Claude Code and Codex clients, and a
deterministic procedure checkpoint that is explicitly **not** an agent trial:

```console
VSIFT_P12_TRIAL_ROOT=<neutral root, for example C:\vsift-trials>
VSIFT_TEST_VSIFT_BIN=<absolute release vsift executable>
VSIFT_TEST_WHISPER_CLI=<abs> VSIFT_TEST_WHISPER_MODEL=<abs> \
  cargo test -p vsift-agent-trials --locked --test p12_skill_procedure_e2e -- --ignored --nocapture
```

It needs FFmpeg and FFprobe on `PATH` for the harness, which registers them (and
whisper.cpp with its model) in each trial's isolated per-user base; `vsift` runs with an
empty `PATH`. Stages: `p12_procedure_a08_local_asr` (the scenario
`A-08-f05-local-asr`: `setup check`, plain `ingest`, `transcript retranscribe` with an
operation id and `--events jsonl`, `search`, a bounded `transcript get`, `candidates`
around the span, `frame get --candidate`, `session retain`) and
`p12_procedure_a09_supplied` (the scenario `A-09-f05-supplied`: the same with `ingest
--transcript` and no local ASR). The walker has no image access, so it reports
`image_access` `unavailable` and cites frames with `pixels_inspected` false; the trace
and handoff are graded by the trial grader (mechanical and interpretation results). It
prints `p12_skill_procedure: passed` and writes `.vsift/e2e-runs/p12-<run-id>/report.json`.
On Windows 11 with FFmpeg 9.0 and whisper.cpp v1.9.2 it passed on 2026-09-28 in 45 s.
The named-client trials themselves, the "Complete video-to-grounded-handoff run", ran
in P12 PR 3's campaigns. Their attachment evidence is the
[P12 qualification record](p12-agent-qualification.md):

- **A-08 and A-09 (review tier):** passed mechanically in 11 of 11 trials on each
  client.
- **A-01..A-07 and SEC-T02 (compact tier):** 23 of 28 full passes on each client.

The trials spend client allowances, so they run on demand, not in CI. The SEC-T02
tool-level suite `sec_t02_adversarial_evidence` runs on every PR.

P13 PR 7 adds the **managed-dependency stage** of the "Installed-user and
managed-dependency run" (ADR 0023 §3 step 7):

```console
VSIFT_P13_INSTALL_E2E=1 cargo test --release --locked -p vsift-cli   --test p13_install_e2e -- --ignored --exact --nocapture
```

It runs only on Ubuntu 24.04 x86-64 and only as a release build (a development build
reaches no publisher); the manual workflow `P13 managed smoke`, job `install-e2e`, runs
it on a hosted runner and uploads its report. From a fresh per-user base with an empty
`PATH` and nothing configured, so no FFmpeg, whisper.cpp, model or Rust toolchain is
reachable: `p13_clean_host` (`setup check` not ready, `setup list` and `setup repair`
report no managed folder and create none); `p13_install_killed_in_download` (the
accepted `setup install` killed by `SIGKILL` once its first download holds 1 MiB, then
`setup repair` names the one abandoned stage with its command, every selection that
exists verifies and the guard is free); `p13_install_killed_in_smoke` (the rerun killed
while a compatibility smoke runs, the same checks); `p13_install_rerun_completes` (the
same accepted command completes and sweeps the abandoned stages, repair reports
healthy, `setup check` resolves every tool as `managed_version` and verifies local ASR);
`p13_managed_journey` (the A-08 journey on the managed tools alone: plain `ingest` of
F05's speech variant, `transcript retranscribe` cited against the speech window and its
words, `search`, `candidates`, `frame get --candidate`, `audio`, `session retain` and
`bundle validate`); and `p13_uninstall_and_reinstall` (`setup remove whisper_model`,
a retranscription then fails `MISSING_CAPABILITY`, and the same accepted plan
reinstalls only the model). It prints `p13_install: passed` and writes
`.vsift/e2e-runs/p13-<run-id>/report.json`. The kill and power-loss qualification of the
store itself is not this stage: it is the kill matrix in
`vsift-infrastructure/tests/p13_install_transaction/kill.rs` (every CI OS) and the
workflow `P13 managed power loss` ([verification](verification.md) "P13 PR 7
evidence"). **Result:** `p13_install: passed` in 47.7 s on `main` at `01656d6` ([run
36793180858](https://github.com/smormah/vsift/actions/runs/36793180858)); the same run's
other two jobs (`managed-install`, `managed-smoke`) passed as well. The stage runs the
release binary built on that runner, with an empty `PATH`, and the managed tools it
installs are the real reviewed artifacts downloaded from their publishers.

P13 PR 9 adds the **installed-user part** of the same run, in the Release workflow rather
than a test binary (the jobs `npm-package` and `npm-qualify`, [`release.md`](../operations/release.md)
section 5). It installs the packed release packages, not a build from source, and runs
them: on `windows-2025`, `macos-15` and `ubuntu-24.04`, each with npm, pnpm, Yarn and Bun
(twelve jobs), against a Verdaccio on the runner's loopback address with no uplink, with
install scripts disabled and under a folder whose name has spaces, accents and CJK
letters. Each job checks the global install (Yarn: a project install), `--version` naming
the commit, `setup check --json`, a path argument with spaces and Unicode through every
installed command, vsift's exit statuses through the launcher, standard input, the
launcher's cost (under 50 ms), signals with no orphan, a changed or replaced executable and
a mismatched platform package each refused readably, the one-shot runner (`npx`, `pnpm dlx`,
`yarn dlx`, `bunx`), optional dependencies omitted (exit 127, no stack trace), running with
the registry stopped and a clean uninstall. **Result:** all twelve jobs passed on `main` at
`951226f` (run 36786019996) and `57f03fe` (run 36797351652). The native archives are
checked by the `package` and `plan` jobs (reproducible, read back against their inputs, by
digest); their executables are the ones the matrix ran from the platform packages, but no
job extracts and runs an archive. **What this stage does not show:** a clean machine (the
hosted runners have a Rust toolchain on `PATH` that no step invokes), the real registry
on any system but one Windows 11 development machine with npm (the P13 record's "First
publish"), or an agent; those are the next packet's checkpoint. The same twelve jobs also
passed on the tag `v0.1.0` in the dry run and the published run (runs 36919612380 and
36931487439). How a user checks an install is
[`install.md`](../operations/install.md) section 6.

P14 PR 3 adds the **published-binary run** of the same checkpoints (RQ-05, RQ-06), because every
stage above ran a Cargo-built `vsift` (L-042). A binary override selects the executable under
test for all of them from one module, `crates/vsift-cli/tests/published_binary/mod.rs`
(`VSIFT_E2E_BINARY`, with the version and tag commit it must print; refused, never ignored,
otherwise; [`development.md`](../development.md) "Running a checkpoint against an installed
binary"), and the workflow `P14 journeys` runs `p06_setup_e2e`, `p07_transcript_e2e`,
`p07_local_asr_e2e`, `p08_search_e2e`, `p08_candidates_e2e`, `p09_evidence_e2e`,
`p10_recovery_e2e`, `p11_worker_e2e` and the new `p14_installed_binary_e2e` against
`vsift-cli@<version>` installed from the real npm registry, on Ubuntu 24.04 (tools installed by
the published binary's own `setup install`), Windows (the repository's pinned builds) and macOS
15 (Homebrew's tools, which are not reviewed artifacts). `p14_installed_binary_e2e` carries the
two cases no earlier checkpoint had: hostile file names through the real tools (SEC-01) and a
sentinel environment that must not reach a tool child (SEC-25). `P13 managed smoke` takes the
same override (`published_version`) so `p13_managed_install_real` and `p13_install_e2e` run the
published binary on Ubuntu 24.04, and both workflows run weekly. A stage that cannot run on a
system is reported (`blocked`, or listed under "Not run here, and why" in the job summary), never
skipped silently. Results and caveats: the [P14 plan](p14-qualification.md) section 17.

An opt-in Windows [candidate-only compatibility smoke](p06-windows-artifact-candidate.md)
has separately verified pinned third-party bytes and model-backed inference on
F01 tone audio. It is **not** the P06 stage, a P13 managed-install stage, a real-speech
transcription test or a substitute for the cumulative journey.

The following rules apply:

- It is opt-in during ordinary development and is run after substantial vertical
  increments. It is not an every-PR or mandatory hosted-CI job.
- Release qualification must run it; nightly or dedicated machines may run selected
  noninteractive profiles when their dependencies are available.
- It uses project-owned synthetic fixtures from `fixtures/corpus`; private meetings,
  production credentials and third-party media are forbidden.
- Runs are bounded by explicit time, output, process, memory and disk budgets. The
  process supervisor remains the only external-command boundary.
- Local reports go beneath ignored `.vsift/e2e-runs/<run-id>/`. Evidence promoted for
  review contains hashes, summaries and redacted diagnostics, not unrestricted model
  conversations or media.
- Missing dependencies produce `blocked` with typed remediation. The harness never
  downloads, installs, changes policy or expands agent permissions without explicit
  authorization.

## Evidence record

Each checkpoint record must include:

- repository revision and dirty-state indicator;
- scenario, fixture manifest version and media hashes;
- OS, architecture, filesystem and effective resource profile;
- VSift, FFmpeg/FFprobe, whisper.cpp, model and client versions where applicable;
- stage status, elapsed time, bounded diagnostics and artifact hashes;
- source-reference validation results and any known coverage gaps;
- explicit authorization record for any setup action; and
- overall `passed` only when every stage required at that checkpoint passed.

Model interpretation and mechanical correctness are separate results. A persuasive
answer cannot hide a missing artifact, invalid timestamp, failed stage or unauthorized
action.

## Development and release gates

- **Packet checkpoint:** the packet's new stage passes locally against the cumulative
  journey after its component tests pass.
- **Mechanical checkpoint:** after P09, both transcript paths reach validated source
  evidence without an agent or manual transcript/screenshot preparation. Met by
  `p09_mechanical_journey_supplied` and `p09_mechanical_journey_local_asr` in
  `p09_evidence_e2e` (2026-09-26).
- **Agent checkpoint:** after P12, A-08/A-09 pass through both named clients with fixed
  permissions, budgets and retained bounded trial records.
- **Distribution checkpoint:** after P13, the agent checkpoint begins from a clean
  supported-machine installation without Rust. *2026-09-30 (ADR 0023 decision H10):
  P13's stage proves the clean installation and one managed install mechanically; the
  named-agent run from a clean installation is P14's.* *2026-10-01: met mechanically as
  described above, on hosted runners and a local registry; then the 0.1.0 pre-release
  was published, and one install of `vsift-cli@next` from the real registry on a Windows 11
  development machine with npm is recorded in the [P13 record](p13-distribution.md); the
  clean-machine install with each package manager and the agent part are P14's.*
- **Release checkpoint:** P14 runs all supported profiles plus security, fault, load
  and release-integrity gates described in the verification specification. *2026-10-02:
  the plan (evidence items `RQ-01..RQ-20`, the published-binary journeys and the
  clean-install agent rounds), confirmed by the maintainer the same day, is
  [p14-qualification.md](p14-qualification.md); this bullet is otherwise unchanged.*

Failures become regression tests at the lowest useful layer. The end-to-end result
stays failed or blocked until the responsible production behavior and regression
evidence merge; fixture truth is never weakened to make a run pass.
