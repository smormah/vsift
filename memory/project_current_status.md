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
- import an existing SRT or WebVTT transcript with the video, aligned by an explicit
  offset;
- transcribe the video's speech itself with whisper.cpp (`transcript retranscribe`),
  whole or one range, into a new revision that keeps earlier citations valid;
- return timestamped transcript segments for a time range, from any revision, as a page
  or as a JSON Lines stream of keyed evidence records;
- **search the transcript for words** (`search`, P08 PR 1, PR #156,
  ADR 0018 accepted): tolerant of spelling (`R-17` = "dialog r 17"), ranked,
  paged, and honest about which parts of the video no transcript covers;
- manage the session's lifetime and retention, and validate retained bundles;
- keep every folder it creates private to the user.

**P07 is complete** (merge `9ea3180`). **P08 (candidates and search) is in progress:**
PR 1 (transcript search) is complete on its branch; PR 2 (#148 source binding) runs in
parallel; PR 3 (visual index core) and PR 4 (`candidates`) are next. P08 is not complete
until `candidates` ships with its recall report.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
  `setup check` adds `local_asr` (model identity and profile, the local-ASR verification).
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`: disposable
  session (24 idle hours, at most 7 days); supplied SRT/WebVTT import never needs whisper.
- `transcript retranscribe <session> [--from --to]`: whisper.cpp v1.9.2 with a reviewed
  pinned model (`base` default, `base_q5_1`), preflights first, 30 s chunks, spliced
  revisions with carried segments (`carried_from`), `no_speech_recognised` recorded.
- `transcript get <session> --from --to [--limit] [--cursor] [--revision]`, and
  `--events jsonl` for the stream.
- `search <session> --query <text> [--from --to] [--limit 1..100] [--cursor]
  [--revision]` (PR 1): `items` are the matching segments as `transcript_segment`
  records with a parallel `hits` list (`phrase` or `all_terms`); `transcript_coverage`
  gives the basis (`supplied_transcript`, `local_asr`, `mixed`), scope
  `transcript_text`, and transcribed, untranscribed and no-speech ranges. An
  untranscribed part makes the result `partial` (exit 0) with the gaps in the envelope
  `coverage` (its first use). `--events jsonl` streams the records, then the hits.
- `session list/status/renew/close/retain/clean` and `bundle validate`.
- Still `COMMAND_NOT_IMPLEMENTED`: candidates, frame, audio, crop, job and setup
  install/repair/list/rollback/remove.

## Transcript search (P08 PR 1)

- **Domain (`search.rs`):** normalisation (lowercase; `2,048`→`2048`; hyphen joins
  `E-409`→`e409`; `10:32`→`10 32`; decimals by value; `zero`..`twenty` and tens as
  digits; other punctuation separates; no Unicode normalisation). Query ≤256 bytes,
  1..16 words, typed rejections `empty`, `too_long`, `too_many_terms`,
  `control_character`. Tier 1 phrase (words joined without spaces equal consecutive
  segment words joined), tier 2 all terms; rank (tier, ordinal); early-stopping scan.
  `SearchCoverage` derives coverage from provenance (chunk windows, inherited runs,
  supplied text taken as whole-source).
- **Application:** `page_search` with `CursorToken`/`QueryDigest` (`vsift.search.v1`,
  revision, range, normalised words; snapshot = revision number; key `<tier>-<ordinal>`).
- **Engine:** `Engine::search` validates the query before any read; reuses existing
  errors plus `EngineError::SearchQueryRejected` (maps to `INVALID_ARGUMENT`).
- **Contract:** `SearchData`, `SearchStreamData`, `SearchEvidenceStream`,
  `search_response`, `search_query_rejection_summary`, an `EvidenceStream` trait for
  hosts, `CoverageResponse` now built by the contract (`with_coverage` sets `partial`).
- **Computed on demand:** no index. S-11 (Windows 11, Xeon E5-2698 v4, release): a
  20,000-segment revision pages fully at limits 1/20/100 with p95 154/167/145 ms vs
  142 ms for a `transcript get` page of the same record (target 250 ms).
- **Opt-in checkpoint `p08_search_e2e`** (stage `p08_search_supplied`): F10 import,
  `R-17` and "dialog r 17" find F10-E01's segment; passed on Windows 11 with FFmpeg 9.0.

## Local ASR and qualification (P07)

- Domain: import or `AsrRun` provenance, inherited provenance for spliced revisions,
  `snap_to_segments`, 30 s chunks with 5 s overlap, -50 dBFS silence, seam merge.
- Profiles by identity: `base` (148 MB, default) and `base_q5_1` (60 MB). Gates for
  `base`: clean WER ≤10% and every spoken critical term except known misses. Measured
  (`docs/planning/p07-asr-qualification.md`): base 3.25% clean WER, RTF 0.39, 338 MiB;
  hosted run 36199691655 (after #153): Ubuntu RTF 0.244, Windows 0.264.

## Evidence stream and private folders

- `transcript get --events jsonl` and `search --events jsonl`: keyed evidence events,
  then one terminal event; no tombstones, consumers filter on `revision_id`.
- Every folder VSift creates is made private before use (SEC-18).

## Fuzzing

- Seven `cargo-fuzz` targets (SRT, WebVTT, whisper `-ojf`, records v1/v2, FFprobe
  metadata, cursors, and P08's `search_query`); weekly nightly run, per-PR seed replay.

## The engine library and contract

- **`crates/vsift`:** `Engine::new(EngineConfig, EnginePorts)`; setup, session,
  transcript, retranscribe, search, `validate_bundle`, `verify_media_tools`,
  `identify_model`. One typed `EngineError`; `failure_code()` is the single public-code
  mapping. API 0.x.
- **`crates/vsift-contract`:** every v1 wire type and fixed prose; **`vsift-cli`** is thin.

## What exists internally (not exposed by the CLI)

- **P02:** shell-free process supervision. **P03:** private storage roots, locks,
  admission, immutable generations, process-crash recovery (ephemeral profile; FS-01).
- **P04:** restricted FFprobe/FFmpeg metadata, frame, audio and speech PCM (the visual
  index in PR 3 will build on the frame primitives).
- **P06:** model identification. **Managed-installer foundations** (owned by P13).

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | Complete (2026-09-25, `9ea3180`): engine, transcripts, local ASR, fuzzing |
| P08 | In progress: PR 1 search complete on branch (ADR 0018 review pending); PR 2 in parallel; PR 3-4 next |
| P09–P12, P14 | Not started |
| P13 | Not started; now also delivers managed dependency installation |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present). `vsift-contract` sits beside the engine and depends on
domain and application only. `tools/vsift-governance` checks the ledger and these
files' sizes; `fuzz/` is the fuzz harness. Largest: `filesystem_session_store.rs`.

## Quality evidence

- P08 PR 1, Windows 11: fmt, strict Clippy, workspace tests, warning-denied rustdoc,
  governance, fuzz replay and fuzz Clippy pass; results are in the PR description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. Qualification records are in `docs/planning/`; history in git,
  `CHANGELOG.md` and `docs/history/2026-09-09-to-23-delivery-log.md`.
