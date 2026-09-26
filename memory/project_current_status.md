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
- return the exact frame at a time or of a candidate, the frames around one, and bursts
  over up to 60 s, as full-resolution PNG files with requested and actual time;
- manage the session's lifetime and retention, and validate retained bundles;
- keep every folder it creates private to the user.

**P09 (evidence navigation) is in progress; the packet is not complete.** PR 1 (media
primitives and a security fix) is merged (`965617f`). PR 2, the evidence core in the
engine library, is merged (`4aecdaa`, #163). PR 3 is complete on branch
`p09/frame-commands` and awaits review: `frame get`,
`frame neighbours` and `frame burst` are public. PR 4 makes `crop` and `audio` public
and writes the qualification record. ADR 0019 is accepted with D1-D7. P00-P08 are
complete.

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
- **Evidence:** `frame_contract` (4 frozen examples), `frame_cli_contract` (reuse via a
  seeded record and preflight pass with stand-in tools that cannot run), opt-in
  `p09_evidence_e2e` (4 stages passed, Windows 11, FFmpeg 9.0, 154 s debug: all 29
  candidate frames at delta 0, reuse about 150 ms, cold frame about 1.8 s).

## P09 PR 2: the evidence core (engine only; merged `4aecdaa`)

- **Operations:** `Engine::frame_get` (a time with at-or-after or displayed-at and a
  tolerance, or a visual candidate's exact frame), `frame_neighbours` (1..20
  consecutive frames per side of an earlier frame item, typed side stops),
  `frame_burst` (1..100 even targets, default 12, over at most 60 s, deduplicated),
  `crop` (a rectangle of a frame or crop item, composed to source pixels, by FFmpeg
  re-decode) and `audio` (WAV 16 kHz mono, at most 30 s, clipped to the source).
- **Identities and reuse (V-08):** request keys (`opk_sha256_`) digest session,
  source, stream selector, operation, parameters, profile `p09-r0-v1` and the provider
  fingerprint; item identities (`evd_`) only what fixes the pixels or samples. Two
  requests naming one frame share one item and one file. A repeated request is
  answered from its complete record, every file re-verified, with no process and no
  write; another provider is a new key; a changed copy is `INTEGRITY_FAILURE`.
- **Lineage:** each extracting call commits its new files and one `evidence_record`
  (strict JSON, at most 256 KiB, schema `bundle-evidence-record.schema.json`) in one
  generation: request, selections with requested/actual time and delta, items, partial
  reason. No operation id or path. Files are delivered as verified absolute paths of the
  committed artifacts (D2).
- **Source check (D1):** after one full hash, evidence calls compare the copy's
  on-disk identity (recorded in the session manifest); a changed identity is hashed in
  full; items record `source_check`. Residual documented in ADR 0012.
- **Budgets:** 100 frames, 200 MP, 256 MiB, 120 s per call; 160 evidence artifacts per
  session (D4); typed partial results; `RESOURCE_LIMIT` before any process when full.
- **Storage and evidence:** kinds `audio_wav` and `evidence_record`; `bundle validate`
  checks records against their files; application, store, D1 and engine tests; opt-in
  real-FFmpeg engine tests; fuzz targets `evidence_record` and `crop_rect`.

## P09 PR 1 (merged `965617f`)

SEC-17 fix (diagnostics read only from the filter's own lines, numbered, time base
checked); integer-timestamp frame selection; `FfmpegMedia` listing, exact extraction,
crops and WAV clips; strict PNG walking; domain navigation rules; preflight profile 3.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`: disposable
  session (24 idle hours, at most 7 days); supplied SRT/WebVTT import never needs whisper.
- `transcript retranscribe <session> [--from --to]`, `transcript get ... [--events jsonl]`.
- `search <session> --query <text> [--from --to] [--limit] [--cursor] [--revision]`.
- `candidates <session> --from <us> --to <us> [--limit 1..100] [--cursor]`.
- `frame get/neighbours/burst` (P09 PR 3, above).
- `session list/status/renew/close/retain/clean` and `bundle validate` (which now also
  validates evidence records).
- Still `COMMAND_NOT_IMPLEMENTED`: audio, crop, job and setup
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
| P09 | In progress: PRs 1-2 merged; PR 3 complete on its branch; PR 4 remains |
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

- P09 PR 3 branch, Windows 11: fmt, strict Clippy, workspace tests, warning-denied
  rustdoc, governance, fuzz fmt/Clippy/replay pass; the opt-in `p09_evidence_e2e`
  passes with FFmpeg 9.0. Results go in the PR description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
