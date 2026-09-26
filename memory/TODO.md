# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P09 (evidence navigation) is in progress. PR 1 (media primitives and hardening) is
complete on branch `p09/media-primitives` and awaits review; the packet is not
complete.** P00-P08 are complete (P08 closed 2026-09-26, merge `b830fc9`).

1. **PR 1 delivered (not user-reachable):** the SEC-17 parser fix (frame and audio
   diagnostics now read only the filter's own lines, consistently numbered, time base
   checked); integer-timestamp frame selection; `FfmpegMedia` `list_frame_times`,
   `frames_at`, `crop_at`, `wav_clip`; `parse_png_sequence`; domain
   `evidence::navigation` (select, neighbours, bursts) and `CropRect::parse/compose`;
   preflight profile 3; fuzz targets `frame_showinfo`, `frame_listing`,
   `png_sequence`; verifier v2 frame-time lists; ADR 0019 (Proposed).
2. **Next: maintainer confirms D1-D7 in ADR 0019**, then PR 2.
   - D1 source check per evidence call (recommended: persisted verified identity after
     one full hash; items record `source_check` identity|full_hash).
   - D2 images as the committed-artifact path in `files[]` only.
   - D3 at-or-after default, `--select displayed-at`, even deduplicated bursts,
     consecutive neighbours.
   - D4 keep 256 artifacts / 64 KiB manifest, P09 sub-budget 160, `RESOURCE_LIMIT`.
   - D5 WAV 16 kHz mono <= 30 s. D6 `<session>` on neighbours/crop, `--candidate`,
     optional `--max-frames` default 12, frames linked to candidates. D7 FFmpeg crops.
3. **PR 2: evidence core** - application use cases, session artifacts, reuse and
   lineage (V-08), engine operations; pending D1-D7.
4. **PR 3:** `frame get`, `frame neighbours`, `frame burst` public (V-01, V-07).
5. **PR 4:** `crop` and `audio` public, P09 qualification record, then the ledger
   completion follow-up.

## Tracked issues

- #159: regenerate the motion fixtures so F04/F05/F12 E02 differ visibly (corpus
  limitations today; F04-E02 is the corpus's only scroll).
- #150: noisy-speech fixture set before any noise WER gate.
- #147: faster-whisper adapter (backlog); whisper.cpp stays the default.
- #128: process-supervisor tests fail intermittently on Windows under load; add recurrences.
- #144: a throttled Windows runner once exceeded the 5 s session-root provisioning wait.

## Other follow-ups

- Evidence: a 60 fps source needs several listings for a 60 s burst (listing cap 1,200
  frames); tiny-text crops (V-06) are unmeasured; the preflight adds three short FFmpeg
  runs to the first media operation per tool identity.
- Candidates: real screen recordings are unmeasured; no denser pass for sub-0.5 s
  changes; the probe's duration is part of the index scope.
- Search: no accent folding or Unicode normalisation; phrases do not cross segments;
  number compounds such as `thirty-two` are not converted.
- A creator killed mid-provisioning leaves an unmarked root refused until removed.
- F09: base starts a segment at its audio start after leading silence.
- Ctrl-C is not trapped; the model is hashed up to three times per run (no cache).

## Open decisions (maintainer)

- ADR 0019 D1-D7 (above) before P09 PR 2.
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): `p07_transcript_e2e`,
  `p07_local_asr_e2e`, `p07_asr_qualification`, `p08_search_e2e`, `p08_candidates_e2e`,
  `engine_retranscribe`, `p07_local_asr`, `p08_candidates_fixtures`,
  `p09_media_primitives`, the S-11 measurements (`engine_search`, `engine_candidates`,
  `--release`) and `source_binding::tests::a_real_multi_chunk_decode_hashes_the_copy_exactly_twice`.
- whisper.cpp `-ojf` output: a split multi-byte token fails the chunk (`unparseable_output`).
- The CLI keeps application, infrastructure and domain as development dependencies for
  tests that seed session records; hosts depend only on `vsift` and `vsift-contract`.
- FS-01: strict OS/storage-crash durability is unqualified; durable requests fail closed
  until P10/P11/P14 run the Ubuntu/ext4 campaign (ADR 0010).
- Baseline findings B-01..B-11 close through their mapped packets.

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
  `BoundSource`. Wire sequencing lives in `vsift-contract`.
- Provider diagnostics are read only from lines that begin with the filter's own prefix,
  with consistent numbering (ADR 0012 note of 2026-09-26).
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas.
