# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

Delivery was re-planned on 2026-09-23 ([ADR 0015](../docs/decisions/0015-r0-delivery-replan.md),
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md)). **P00-P07 are
complete** (P07 merge `9ea3180`). **P08 (candidates and search) is in progress**; the ledger
marks it `in_progress`. P08 is delivered as four pull requests:

1. **PR 1, transcript search: merged (PR #156, `a2fadc1`;
   [ADR 0018](../docs/decisions/0018-visual-candidate-index-and-transcript-search.md)
   accepted 2026-09-26).** Remaining merges in order: #157, #158, #160. `vsift search` end to end: domain normalisation, tiers,
   ranking and coverage; application paging with query-bound cursors; `Engine::search`;
   contract types, schemas `search-data`/`search-stream-data` and frozen F10 examples;
   CLI; tests (C-03, S-11, contract, opt-in `p08_search_e2e`); fuzz target `search_query`.
2. **PR 2, #148 bracketed source binding (PR #157, this change):** one full hash when a
   multi-call operation opens the source, a cheap identity check before each provider
   call, a full hash before commit (ADR 0012 note). 869 MB, 24-chunk clip: 25.5 s vs 173.4 s.
3. **PR 3, visual index core (next):** 60 s pure windows, 2 Hz actual-frame sampling with a
   candidate at least every 10 s, merging with preserved time, stability, visual hash,
   batched visual-index records as a new session artifact kind, typed gap taxonomy.
4. **PR 4, `candidates` command (after PR 3):** analyses missing windows in range, at most
   30 minutes of media per call, remainder `not_analyzed`; recall report over `stable`
   events of at least 1 s; completes ADR 0018's visual half; closes P08.

#153 is fixed (PR #155, `10a251e`): the optimised Ubuntu whisper.cpp CPU backends are
pinned; hosted Ubuntu RTF 0.244, Windows 0.264.

## Decisions confirmed by the maintainer (ADR 0018, 2026-09-26)

1. Visual index built inside `candidates`, 30 minutes of media per call.
2. 2 Hz sampling with a candidate at least every 10 s.
3. Recall gate over `stable` events of at least 1 s; F04-E02/F05-E02 reported as corpus
   limitations; open an issue to regenerate the motion fixtures.
4. #148 fixed in P08 by bracketed source binding.
5. `search --events jsonl` streams existing `transcript_segment` records, then a terminal
   event with the hit list (implemented in PR 1).
6. No thumbnails in P08 (P09 frames).
Also confirmed, PR 1's additions: a third coverage basis `mixed` (local ASR spliced into
supplied text), and a supplied transcript taken to cover the whole source (unverified).

## Tracked issues

- #150: noisy-speech fixture set before any noise WER gate.
- #147: faster-whisper adapter (backlog); whisper.cpp stays the default.
- #128: process-supervisor tests fail intermittently on Windows under load; add recurrences.
- #144: a throttled Windows runner once exceeded the 5 s session-root provisioning wait.

## Other follow-ups

- Search: no accent folding or Unicode normalisation (needs a dependency); phrases do not
  cross segments; number compounds such as `thirty-two` are not converted. A spliced
  revision whose older run left no carried segment reports that run's range as
  untranscribed (understated, never overstated).
- A creator killed mid-provisioning leaves an unmarked root refused until removed.
- F09: base starts a segment at its audio start after leading silence.
- Ctrl-C is not trapped; the model is hashed up to three times per run (no cache).
- The local-ASR checkpoint's word checks are written for `base`.

## Open decisions (maintainer)

- Crate names confirmed (`vsift`, `vsift-contract`); crates.io check precedes publication.
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): `p07_transcript_e2e`,
  `p07_local_asr_e2e`, `p07_asr_qualification`, `p08_search_e2e`, `engine_retranscribe`,
  `p07_local_asr`, and the S-11 search measurement (`engine_search`, `--release`).
- whisper.cpp `-ojf` output: a split multi-byte token fails the chunk (`unparseable_output`).
- The CLI keeps application, infrastructure and (since P08) domain as development
  dependencies for tests that seed session records; hosts still depend only on `vsift`
  and `vsift-contract`.
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
  preflight hook first. Wire sequencing lives in `vsift-contract`.
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas.
