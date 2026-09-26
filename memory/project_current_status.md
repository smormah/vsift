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
- return the exact frame at a time or of a candidate, the frames around one, bursts
  over up to 60 s and native-size crops as full-resolution PNG files, and WAV clips of
  up to 30 s, with requested and actual time, lineage and reuse;
- manage the session's lifetime and retention, and validate retained bundles;
- keep every folder it creates private to the user.

**P09 is complete** (2026-09-27, merge `e57c706`, ledger record written; PRs #162, #163,
#165, #166; ADR 0019 accepted with D1-D7). P00-P09 are complete and the mechanical
checkpoint is met; P10 (recovery integration) is next and not started.

## P09 PR 4: `crop`, `audio` and qualification

- **Grammar:** `crop <ses> <evd> --rect x,y,w,h` (canonical decimals parsed by the
  domain rule against an unbounded frame, containment checked against the parent's
  record before any tool; parent a frame or crop) and `audio <ses> --from --to` (at
  most 30 s, clipped to the source with `range_clipped`).
- **Contract:** crops use the frame result (`operation` `crop`); audio has
  `audio_response`/`AudioEvidenceStream`, record type `audio_evidence`, schemas
  `audio-data`, `audio-stream-data`, `audio-evidence`; frozen `crop.json`, `audio.json`.
  Remediation per medium (30 s, range start, no audio, undecodable media).
- **Qualification:** `docs/planning/p09-evidence-navigation.md`. Release run of
  `p09_evidence_e2e` (Windows 11, Xeon E5-2698 v4, FFmpeg 9.0, whisper.cpp v1.9.2):
  eleven stages passed in 471 s, including the mechanical journey on both transcript
  paths (F03-speech: search, candidates, candidate frame, crop, audio, retained bundle;
  10.6 s supplied, 24.0 s local ASR). Crops pixel-equal to FFmpeg; audio starts 64 ms and 750 ms; malformed media
  `INVALID_SOURCE` with nothing committed; streams and a retained bundle validate. Perf
  (recorded): warm reuse p95 141 ms; on a 1.17 GB 1080p clip cold frame p95 4.1 s,
  12-frame burst 17.1 s, first full hash 10.9 s; warm cost grows about 3.7 ms per
  session generation (1 s at 256, #164).

## P09 PR 3: the frame commands (public)

- **Grammar (D6):** `frame get <ses> (--at <us> | --candidate <vcd>) [--select
  at-or-after|displayed-at] [--tolerance-us 0..10000000]` (default 1 s),
  `frame neighbours <ses> <evd> [--count 1..20]`, `frame burst <ses> --from --to
  [--max-frames 1..100]` (default 12). `--select`/`--tolerance-us` with `--candidate`,
  counts 0/21, frames 0/101 and a tolerance over 10 s are parse errors.
- **Contract:** `vsift-contract::navigation` presents the committed record
  (`frame_response`, `FrameEvidenceStream`); record type `frame_evidence` keyed by
  `evd_`; schemas `frame-data`, `frame-stream-data`, `frame-evidence`; `source_check` is
  per result so one key always carries one record; `files[]` carries the verified
  absolute path (D2; Windows `\\?\` form); a non-UTF-8 path is `STORAGE_IO`.
- **Failures** carry fixed remediation: each selection reason, burst over 60 s ->
  `candidates`, full session -> retain and reopen, unknown ids, wrong parent kind,
  missing FFmpeg/FFprobe (not Whisper). Partial results carry a per-reason warning.
- **Evidence:** `navigation_contract` (frozen examples), `evidence_cli_contract` (reuse
  via a seeded record and preflight pass with stand-in tools that cannot run), opt-in
  `p09_evidence_e2e` stages for V-01/V-07/V-08 (29 candidate frames at delta 0).

## P09 PRs 1-2: primitives and evidence core (engine)

- PR 1 (merged): SEC-17 fix, integer-timestamp frame selection, `FfmpegMedia` listing,
  exact extraction, crops and WAV clips, strict PNG walking, preflight profile 3.
- PR 2 (merged `4aecdaa`, #163): `Engine::frame_get/frame_neighbours/frame_burst/crop/audio`; request
  keys (`opk_sha256_`) and item identities (`evd_`, only what fixes the pixels, so
  requests naming one frame share one item and file); warm reuse re-verifies files
  and starts no process; one `evidence_record` per call (no path, no operation id);
  D1 identity-only source checks after one full hash; budgets 100 frames, 200 MP,
  256 MiB, 120 s per call, 160 evidence artifacts per session; `bundle validate`
  checks records against files; fuzz targets `evidence_record`, `crop_rect`.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`: disposable
  session (24 idle hours, at most 7 days); supplied SRT/WebVTT import never needs whisper.
- `transcript retranscribe <session> [--from --to]`, `transcript get ... [--events jsonl]`.
- `search <session> --query <text> [--from --to] [--limit] [--cursor] [--revision]`.
- `candidates <session> --from <us> --to <us> [--limit 1..100] [--cursor]`.
- `frame get/neighbours/burst`, `crop`, `audio` (P09, above).
- `session list/status/renew/close/retain/clean` and `bundle validate` (which now also
  validates evidence records).
- Still `COMMAND_NOT_IMPLEMENTED`: job and setup
  install/repair/list/rollback/remove. Human-readable terminal output is P13's.

## Visual candidates, search and local ASR (P07, P08)

- Candidates: 128x72 grey samples at most every 0.5 s in 60 s windows; a candidate in
  every 10 s cell; typed gaps; recall record `docs/planning/p08-candidate-recall.md`.
- Search normalises spelling and number words, phrase then all-terms tiers.
- Local ASR: 30 s chunks with 5 s overlap, profiles `base` (default) and `base_q5_1`;
  base 3.25% clean WER, RTF 0.39. Multi-call operations bind the copy with a
  `BoundSource` (#148).

## The engine library and contract

- **`crates/vsift`:** setup, session, transcript, retranscribe, search, candidates,
  evidence (frame get/neighbours/burst, crop, audio), `validate_bundle`,
  `verify_media_tools`, `identify_model`. One typed `EngineError`; `failure_code()` is
  the single public-code mapping. API 0.x.
- **`crates/vsift-contract`:** every v1 wire type and fixed prose; **`vsift-cli`** is thin.

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | Complete (2026-09-25, `9ea3180`): engine, transcripts, local ASR, fuzzing |
| P08 | Complete (2026-09-26, `b830fc9`): search, candidates, source binding |
| P09 | Complete (2026-09-27, `e57c706`): frames, neighbours, bursts, crops, audio, reuse, lineage |
| P10–P12, P14 | Not started |
| P13 | Not started; also delivers managed installation and human-readable output |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present). `vsift-contract` sits beside the engine. Evidence:
domain `evidence::{navigation, record}`, application `evidence` (identities, ports
`FrameExtractor`/`AudioExtractor`, use cases), infrastructure `evidence_record`,
`evidence_media` and the store's evidence methods, engine `evidence.rs`. Largest file:
`filesystem_session_store.rs`.

## Quality evidence

- P09 PR 3 and PR 4 branches, Windows 11: fmt, strict Clippy, workspace tests,
  warning-denied rustdoc, governance, fuzz fmt/Clippy/replay pass; the opt-in
  `p09_evidence_e2e` passes with FFmpeg 9.0. Results go in the PR description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
