# VSift current status

As of 2026-09-26. Current-state document: rewrite it, don't append to it. Next
actions and open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives AI coding agents local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is
also an embeddable engine library (`vsift`) that the CLI, and later other hosts, use.
Today it can:
- check and register its dependencies and show a read-only setup plan, and report
  whether local speech recognition really works here (`setup check` `local_asr`);
- copy a video into a private, disposable session;
- import an existing SRT or WebVTT transcript with the video, aligned by an offset;
- transcribe the video's speech itself with whisper.cpp (`transcript retranscribe`);
- return timestamped transcript segments for a time range, as a page or a JSON Lines
  stream of keyed evidence records;
- search the transcript for words (`search`);
- list the moments where the screen changed (`candidates`);
- manage the session's lifetime and retention, and validate retained bundles;
- keep every folder it creates private to the user.

**P09 (evidence navigation) is in progress.** PR 1, the media primitives and a
security fix, is complete on branch `p09/media-primitives` and not yet merged. It adds
nothing a user can call yet: frames, crops and audio clips become commands in PRs 3
and 4, after the maintainer confirms ADR 0019's decisions D1-D7 and PR 2 builds the
evidence core. P00-P08 are complete.

## P09 PR 1: what it delivers (internal)

- **Security fix (SEC-17):** FFmpeg repeats a file's metadata in the diagnostics VSift
  reads frame and audio times from; two readers accepted any line containing the
  filter's name, so a crafted title could move `transcript retranscribe`'s segment
  times. Readers now take only lines the filter wrote, numbered without gaps, with the
  time base checked. Regression tests failed on the old code (ADR 0012 note).
- **Integer-timestamp selection:** `StreamTime::first_at_or_after`; the P04 `frame`
  call and all new calls select `pts` integers, never decimal seconds. All 29 recorded
  P08 candidates extract at delta 0.
- **Adapter (`FfmpegMedia`):** `list_frame_times` (60 s, 1,200 frames, 1 MiB
  diagnostics), `frames_at` (up to 8 exact frames, 64 MiB, rgb24 PNG), `crop_at`
  (FFmpeg crop after display rotation, pixel-exact on the rotated fixture), `wav_clip`
  (16 kHz mono, <= 30 s, header written in Rust); strict `parse_png_sequence`; new
  `MediaError` variants `InvalidFrameRequest`, `CropOutsideFrame`, `FrameNotFound`,
  `TimeBaseMismatch`.
- **Domain `evidence::navigation`:** at-or-after (default) and displayed-at selection
  with tolerance up to 10 s; neighbours 1..20 per side with typed stops; bursts of
  1..100 even targets over at most 60 s, deduplicated; `CropRect::parse` and `compose`.
- **Preflight profile 3:** the `frame` check also lists, extracts exactly and crops F01.
- **Evidence:** opt-in `p09_media_primitives` (V-01/V-06 at adapter level) passed with
  FFmpeg 9.0; always-run `p09_recorded_diagnostics` over real FFmpeg output; verifier
  v2 records independent `ffprobe` frame lists (no hash or truth changed).

## What works (public CLI, unchanged by P09 PR 1)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`: disposable
  session (24 idle hours, at most 7 days); supplied SRT/WebVTT import never needs whisper.
- `transcript retranscribe <session> [--from --to]`, `transcript get ... [--events jsonl]`.
- `search <session> --query <text> [--from --to] [--limit] [--cursor] [--revision]`.
- `candidates <session> --from <us> --to <us> [--limit 1..100] [--cursor]`: analysed
  on first use (30 minutes per call), warm pages without tools, honest coverage.
- `session list/status/renew/close/retain/clean` and `bundle validate`.
- Still `COMMAND_NOT_IMPLEMENTED`: frame, audio, crop, job and setup
  install/repair/list/rollback/remove.

## Visual candidates (P08)

- 128x72 grey samples at most every 0.5 s in fixed 60 s windows; a change is one block
  moving >= 6 or two >= 4; a candidate in every 10 s cell; typed gaps; index stores the
  displayed dimensions. Recall: every stable event of at least 1 s, 0 false changes,
  12.9 candidates/min; F04-E02, F05-E02, F12-E02 are corpus limitations (#159).
  Record: `docs/planning/p08-candidate-recall.md`.

## Transcript search and local ASR (P07, P08)

- Search normalises spelling (`2,048`→`2048`, `E-409`→`e409`, number words), phrase then
  all-terms tiers, coverage from provenance, on demand (p95 about 150 ms at 20,000).
- Local ASR: 30 s chunks with 5 s overlap, profiles `base` (default) and `base_q5_1`;
  base 3.25% clean WER, RTF 0.39. A `BoundSource` hashes the copy on open and before
  commit and compares on-disk identity per provider call (#148).

## Evidence stream, private folders and fuzzing

- `transcript get`, `search` and `candidates --events jsonl`: keyed evidence events.
- Every folder VSift creates is made private before use (SEC-18).
- Twelve `cargo-fuzz` targets, including `frame_showinfo`, `frame_listing` and
  `png_sequence`; weekly nightly run, per-PR seed replay.

## The engine library and contract

- **`crates/vsift`:** setup, session, transcript, retranscribe, search, candidates,
  `validate_bundle`, `verify_media_tools`, `identify_model`. One typed `EngineError`;
  `failure_code()` is the single public-code mapping. API 0.x.
- **`crates/vsift-contract`:** every v1 wire type and fixed prose; **`vsift-cli`** is thin.

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | Complete (2026-09-25, `9ea3180`): engine, transcripts, local ASR, fuzzing |
| P08 | Complete (2026-09-26, `b830fc9`): search, candidates, source binding |
| P09 | In progress: PR 1 of 4 complete on its branch; PRs 2-4 remain |
| P10–P12, P14 | Not started |
| P13 | Not started; now also delivers managed dependency installation |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present). `vsift-contract` sits beside the engine and depends on
domain and application only. `tools/vsift-governance` checks the ledger and these
files' sizes; `fuzz/` is the fuzz harness. The media adapter is
`crates/vsift-infrastructure/src/ffmpeg_media.rs` with submodules `evidence`,
`showinfo` and `png`. Largest file: `filesystem_session_store.rs`.

## Quality evidence

- P09 PR 1 branch, Windows 11: fmt, strict Clippy, workspace tests, warning-denied
  rustdoc, governance, fuzz fmt/Clippy/replay and the verifier's Python tests pass; the
  opt-in `p09_media_primitives`, `p04_media_e2e` and `p06_tool_verification` pass with
  FFmpeg 9.0. Results go in the PR description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. Qualification records are in `docs/planning/`; history in git,
  `CHANGELOG.md` and `docs/history/2026-09-09-to-23-delivery-log.md`.
