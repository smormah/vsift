# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P09 are complete.** P09 (evidence navigation) closed on 2026-09-27 with merge
`e57c706` (PRs #162, #163, #165, #166; ADR 0019 accepted with D1-D7); its evidence is in
the ledger. The test spine's mechanical checkpoint is met.

1. **Public now:** `frame get <ses> (--at | --candidate) [--select] [--tolerance-us]`,
   `frame neighbours <ses> <evd> [--count 1..20]`, `frame burst <ses> --from --to
   [--max-frames 1..100]`, `crop <ses> <evd> --rect x,y,w,h`, `audio <ses> --from --to`
   (<= 30 s), each with `--json`, `--events jsonl` (`frame_evidence`,
   `audio_evidence` records) and `files[]` absolute paths (D2); schemas `frame-data`,
   `frame-stream-data`, `frame-evidence`, `audio-data`, `audio-stream-data`,
   `audio-evidence`; six frozen examples.
2. **Qualified:** `docs/planning/p09-evidence-navigation.md` - release run of
   `p09_evidence_e2e`, eleven stages passed in 471 s (Windows 11, FFmpeg 9.0,
   whisper.cpp v1.9.2): V-01 against the frozen frame lists, 29 candidate frames at
   delta 0, crops pixel-equal to FFmpeg, audio first-sample times, malformed media,
   streams and bundles, performance, and the test spine's mechanical checkpoint
   (`p09_mechanical_journey_supplied` 10.6 s, `p09_mechanical_journey_local_asr`
   24.0 s: search -> candidates -> candidate frame -> crop -> audio -> retained bundle
   on F03-speech, every citation checked against frozen truth).
3. **Next: P10 (recovery integration), not started.** Governance rule 10: the
   maintainer starts it. Read the P10 row of `docs/planning/implementation-work-packets.md`
   and X-01..X-06, X-09, X-10, S-07, S-08 first. #164 (incremental manifest-chain
   validation) and ADR 0019 D4 (raising the artifact caps) belong with P10's commit work.

## Tracked issues

- #159: regenerate the motion fixtures so F04/F05/F12 E02 differ visibly.
- #150: noisy-speech fixture set before any noise WER gate.
- #147: faster-whisper adapter (backlog); whisper.cpp stays the default.
- #128: process-supervisor tests fail intermittently on Windows under load.
- #144: a throttled Windows runner once exceeded the 5 s session-root provisioning wait.

## Other follow-ups

- Evidence: a burst over a range denser than one 1,200-frame listing (60 fps over more
  than 20 s) is rejected (`outside_listing`). Tiny text is measured on synthetic glyphs
  only. Neighbours list up to three windows (2, 10, 29 s).
- #164: every session read walks the whole manifest chain; warm evidence calls grow
  about 3.7 ms per generation (about 1 s at 256). Incremental validation (P10).
- Evidence records are read and decoded in full on every evidence call (at most 160
  records of 256 KiB); fine for R0, an index would help later.
- Candidates: real screen recordings are unmeasured; no denser pass for sub-0.5 s
  changes; the probe's duration is part of the index scope.
- Search: no accent folding or Unicode normalisation; phrases do not cross segments.
- A creator killed mid-provisioning leaves an unmarked root refused until removed.
- Ctrl-C is not trapped; the model is hashed up to three times per run (no cache).

## Open decisions (maintainer)

- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.
- Human-readable terminal output is assigned to P13 (decided 2026-09-26).

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): `p07_transcript_e2e`,
  `p07_local_asr_e2e`, `p07_asr_qualification`, `p08_search_e2e`, `p08_candidates_e2e`,
  `p09_evidence_e2e`,
  `engine_retranscribe`, `p07_local_asr`, `p08_candidates_fixtures`,
  `p09_media_primitives`, `engine_evidence`, the S-11 measurements (`engine_search`,
  `engine_candidates`, `--release`) and
  `source_binding::tests::a_real_multi_chunk_decode_hashes_the_copy_exactly_twice`.
- whisper.cpp `-ojf` output: a split multi-byte token fails the chunk.
- The CLI keeps application, infrastructure and domain as development dependencies for
  tests that seed session records; hosts depend only on `vsift` and `vsift-contract`.
- FS-01: strict OS/storage-crash durability is unqualified; durable requests fail closed
  until P10/P11/P14 run the Ubuntu/ext4 campaign (ADR 0010).
- A session written by this version records an optional `verified_source_identity`
  that older builds reject (sessions are disposable).

## Parked: managed installation (now P13)

Resume order: smoke executor over the digest-bound policy; failure cleanup before
activation; guarded download/stage/smoke/publish; install, rollback and uninstall;
bounded version cleanup; kill and power-loss qualification; D-02..D-08 and the E2E stage.

## Guardrails

- R0 ships only when a coding agent goes from a local video to a grounded handoff through
  both the supplied-transcript and local-ASR paths, in named Codex and Claude Code trials.
- R1 packets P15..P20 start only after P14. Live capture (#107, #108) waits.
- New features land in the engine once, never in a host. Any new media stage calls the
  preflight hook first; one that calls a provider more than once over a source takes a
  `BoundSource`. Read-only evidence calls use `BoundSource::open_for_evidence` (D1).
- Provider diagnostics are read only from lines that begin with the filter's own prefix.
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas.
