# Incremental end-to-end test spine

Status: P04, P05 and P06 checkpoints, the P07 supplied-transcript and local-ASR
stages and the P08 search and visual-candidates stages are implemented; the complete
journey remains `not_implemented`. Managed
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
P06 command opens a network connection while `setup install` stays reserved.

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
  evidence without an agent or manual transcript/screenshot preparation.
- **Agent checkpoint:** after P12, A-08/A-09 pass through both named clients with fixed
  permissions, budgets and retained bounded trial records.
- **Distribution checkpoint:** after P13, the agent checkpoint begins from a clean
  supported-machine installation without Rust.
- **Release checkpoint:** P14 runs all supported profiles plus security, fault, load
  and release-integrity gates described in the verification specification.

Failures become regression tests at the lowest useful layer. The end-to-end result
stays failed or blocked until the responsible production behavior and regression
evidence merge; fixture truth is never weakened to make a run pass.
