# VSift current status

As of 2026-09-26. Current-state document: rewrite it, don't append to it. Next
actions and open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives AI coding agents local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is
also an embeddable engine library (`vsift`) that the CLI, and later other hosts, use.
On the P08 branches it can:
- check and register its dependencies and show a read-only setup plan, and report
  whether local speech recognition really works here (`setup check` `local_asr`);
- copy a video into a private, disposable session;
- import an existing SRT or WebVTT transcript with the video, aligned by an offset;
- transcribe the video's speech itself with whisper.cpp (`transcript retranscribe`);
- return timestamped transcript segments for a time range, as a page or a JSON Lines
  stream of keyed evidence records;
- **search the transcript for words** (`search`, P08 PR 1);
- **list the moments where the screen changed** (`candidates`, P08 PR 4), analysing
  the video on first use, at most 30 minutes per call, and saying plainly which parts
  are not analysed or could not be;
- manage the session's lifetime and retention, and validate retained bundles;
- keep every folder it creates private to the user.

**P07 is complete** (merge `9ea3180`). **P08's implementation is complete across four
pull requests** (ADR 0018 accepted 2026-09-26): PR 1 search (#156, merged), PR 2
bracketed source binding (#157), PR 3 visual index core (#158) and PR 4 `candidates`
(#160, this change). The packet closes when all four are merged and the ledger
completion record is written.

## What works (public CLI, on `p08/candidates`)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`: disposable
  session (24 idle hours, at most 7 days); supplied SRT/WebVTT import never needs whisper.
- `transcript retranscribe <session> [--from --to]`, `transcript get ... [--events jsonl]`.
- `search <session> --query <text> [--from --to] [--limit] [--cursor] [--revision]`:
  matching `transcript_segment` records with `hits` and `transcript_coverage`;
  untranscribed parts make it `partial` (exit 0) with envelope `coverage`.
- `candidates <session> --from <us> --to <us> [--limit 1..100] [--cursor]`:
  `visual_candidate` records in time order (actual frame time, span, change window,
  reasons, stability, uncalibrated change size, visual hash, displayed dimensions),
  `index` and `coverage` (analysed ranges and typed gaps); gaps make it `partial` (exit
  0). Analysed ranges and cursor calls are warm reads with no tool. `--events jsonl`.
- `session list/status/renew/close/retain/clean` and `bundle validate` (transcript and
  visual-index records, decoded strictly).
- Still `COMMAND_NOT_IMPLEMENTED`: frame, audio, crop, job and setup
  install/repair/list/rollback/remove.

## Visual candidates (P08 PR 3 core and PR 4 command)

- **Domain `visual`:** 128x72 grey samples at most every 0.5 s become 16x9 block means
  and a difference hash; fixed 60 s windows (never merged) with a 0.5 s lead-in; change
  = one block moving >= 6 or two >= 4; motion, transients, a candidate in every 10 s
  cell with a frame, at most 32 per window; typed gaps. The index carries the stream's
  displayed dimensions; `MediaDescription::visual_video_stream` picks the stream.
- **Application:** `extend_visual_index(_within)` (budget, clamped to 30),
  `merge_visual_extension` (lost commit race), `page_candidates` (cursor bound to the
  windows of its range), coverage helpers.
- **Engine:** `Engine::candidates`: warm read or tools, preflight, `BoundSource`, probe,
  analyse, full re-verify, commit (merge and retry up to 3 times), page.
  `EnginePorts::with_visual_window_budget`. Existing failure codes only.
- **Infrastructure:** `FfmpegMedia::visual_samples` (closed argv, 122 frames, 256 KiB
  diagnostics, 120 s; `-ss`/`-t` normalised with a 1 s margin), `visual_index_record`
  (8 MiB, 64 per session, strict decode, bundles validated), preflight profile 2.
- **Measured:** recorded and live gates hit every stable event of at least 1 s, F06's
  500 ms tooltip, 0 false changes, 12.9 candidates/min; F04-E02, F05-E02, F12-E02 are
  corpus limitations (#159). Warm pages p95 about 100 ms (engine, four-hour index).
  Record: `docs/planning/p08-candidate-recall.md`.

## Transcript search (P08 PR 1)

- Normalisation (lowercase, `2,048`→`2048`, `E-409`→`e409`, number words as digits),
  query ≤256 bytes and 1..16 words, phrase then all-terms tiers, coverage from
  provenance, computed on demand (S-11 p95 about 150 ms on 20,000 segments).

## Local ASR and source binding (P07, P08 PR 2)

- Import or `AsrRun` provenance, 30 s chunks with 5 s overlap, profiles `base`
  (default) and `base_q5_1`; base 3.25% clean WER, RTF 0.39.
- A `BoundSource` hashes on open and before commit and compares on-disk identity per
  provider call (#148); used by local ASR and visual sampling.

## Evidence stream, private folders and fuzzing

- `transcript get`, `search` and `candidates --events jsonl`: keyed evidence events
  (`transcript_segment`, `visual_candidate`), then one terminal event.
- Every folder VSift creates is made private before use (SEC-18).
- Nine `cargo-fuzz` targets, including `search_query`, `visual_samples` and
  `visual_index_record`; weekly nightly run, per-PR seed replay.

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
| P08 | Implementation complete in PRs 1-4 (ADR 0018 accepted); merging, then the ledger record |
| P09–P12, P14 | Not started |
| P13 | Not started; now also delivers managed dependency installation |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present). `vsift-contract` sits beside the engine and depends on
domain and application only. `tools/vsift-governance` checks the ledger and these
files' sizes; `fuzz/` is the fuzz harness. Largest: `filesystem_session_store.rs`.

## Quality evidence

- P08 PR 4 branch, Windows 11: fmt, strict Clippy, workspace tests, warning-denied
  rustdoc, governance, fuzz replay and fuzz Clippy pass; the opt-in
  `p08_candidates_e2e` (7 stages, with the local-ASR variant) and `engine_candidates`
  pass with FFmpeg 9.0. Results go in the PR description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. Qualification records are in `docs/planning/`; history in git,
  `CHANGELOG.md` and `docs/history/2026-09-09-to-23-delivery-log.md`.
