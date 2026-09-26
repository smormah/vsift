# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

P00-P07 are complete (P07 merge `9ea3180`; re-plan in ADR 0015/0016). **P08
(candidates and search) is implemented across four pull requests; ADR 0018 was
accepted on 2026-09-26.** PR 1 (#156) is merged; #157, #158 and #160 (this change) merge
in that order. The packet completes with the ledger completion record. Local checks: CI is the default Linux/macOS
check.

1. **PR 1 (literal search, #156, merged `a2fadc1`)**; **PR 2 (bracketed source binding,
   #148, #157)** and **PR 3 (visual index core, #158)** are green in CI.
2. **PR 4, the `candidates` command (#160, this change; on PR 3)**. `Engine::candidates`, v1 command, `visual_candidate`
   record type, four schemas and four frozen examples, C-03/V-04 tests, the opt-in
   `p08_candidates_e2e` checkpoint (7 stages), ADR 0018 completed (Accepted),
   recall record `docs/planning/p08-candidate-recall.md`.
3. **Next (supervisor):** merge #157, #158 and #160 in order; write the P08 ledger
   completion record citing the merge commits and checkpoint reports. P09 starts only
   when the maintainer says so (governance rule 10).
4. **Decisions confirmed in ADR 0018 (2026-09-26):** index built inside `candidates`, 30
   windows per call, remainder `not_analyzed`; 2 Hz actual frames, a candidate at least
   every 10 s; recall gated on stable events of at least 1 s; no thumbnails; search
   streams existing `transcript_segment` records. PR 3 lowered the change thresholds
   (one block moving 6, or two moving 4) and gives `-ss`/`-t` in normalised time with a
   1 s margin. PR 4 clips a range past the video's end, stores the displayed dimensions
   in the index, and lets a host lower the per-call budget.

## Tracked issues

- #159: regenerate the motion fixtures so F04/F05/F12 E02 differ visibly (corpus
  limitations today; F04-E02 is the corpus's only scroll).
- #150: noisy-speech fixture set before any noise WER gate.
- #148: per-chunk source rehash; fixed by P08 PR 2 (#157), closes when it merges.
- #147: faster-whisper adapter (backlog); whisper.cpp stays the default.
- #128: process-supervisor tests fail intermittently on Windows under load; add recurrences.
- #144: a throttled Windows runner once exceeded the 5 s session-root provisioning wait.

## Other follow-ups

- Candidates: real screen recordings (heavier compression noise) are unmeasured; no
  denser on-demand pass for sub-0.5 s changes; the probe's duration is part of the index
  scope, so a tools upgrade that probes another duration makes an old index an
  integrity failure for that session.
- Search: no accent folding or Unicode normalisation; phrases do not cross segments;
  number compounds such as `thirty-two` are not converted.
- A creator killed mid-provisioning leaves an unmarked root refused until removed.
- F09: base starts a segment at its audio start after leading silence.
- Ctrl-C is not trapped; the model is hashed up to three times per run (no cache).

## Open decisions (maintainer)

- Crate names confirmed (`vsift`, `vsift-contract`); crates.io check precedes publication.
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): `p07_transcript_e2e`,
  `p07_local_asr_e2e`, `p07_asr_qualification`, `p08_search_e2e`, `p08_candidates_e2e`,
  `engine_retranscribe`, `p07_local_asr`, `p08_candidates_fixtures`, the S-11
  measurements (`engine_search`, `engine_candidates`, `--release`) and
  `source_binding::tests::a_real_multi_chunk_decode_hashes_the_copy_exactly_twice`.
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
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas.
