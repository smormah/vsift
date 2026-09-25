# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

Delivery was re-planned on 2026-09-23 ([ADR 0015](../docs/decisions/0015-r0-delivery-replan.md),
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md)). P00-P06 are
complete. P07 is in progress: increments 1a-3b and the fuzz targets are done, the
packet is not. Local checks: CI is the default Linux/macOS check; Docker only for
platform code.

1. **Done:** 1a `vsift-contract` wire types (PR #126); 1b engine facade, thin CLI
   (PR #127); 2 supplied transcripts (PR #129); 2b media-tool preflight (PR #133); 2c
   JSONL evidence stream and bundle record schema (PR #134); speech fixtures (PRs
   #138-#140, #142, #143); 3a local ASR core (PR #145); `cargo-fuzz` targets (PR #146).
2. **In review: increment 3b (branch `p07/asr-retranscribe`).** `transcript
   retranscribe` through the engine under decisions D1-D8: both preflights, a private
   work directory in the session, complete spliced revisions with carried segments,
   `transcript get --revision`, widened v1 schemas and version-2 records, typed
   failures on existing codes, and the opt-in `p07_local_asr` checkpoint (passed
   locally). [ADR 0017](../docs/decisions/0017-local-asr-through-whisper-cpp.md) is
   **Proposed** and needs maintainer review before merge; it lists six decisions
   awaiting confirmation (empty-speech revision, audio-stream choice, retranscribe
   stream shape, no Ctrl-C trap, 1 s verification end bound, no range clamping).
3. **Next: increment 3c.** D4: `setup check` reports an additive `local_asr` object
   (runs the verification within 60 s if none is recorded). D6: the `base-q5_1`
   profile beside the pinned base, gates RTF <= 0.5, <= 400 MiB, WER <= 10% clean /
   25% F08, the measured default, `docs/planning/p07-asr-qualification.md`, and an
   opt-in CI workflow. T-04 accuracy and timing are measured there.
4. **Still owed by P07 before the packet closes:** the packet completion record in the
   ledger.

## Follow-ups (open issues before relying on them)

- A creator killed mid-provisioning leaves an unmarked root refused until removed.
- Every speech chunk rehashes the session's whole source copy before FFmpeg reads it
  (the P04 check-before-use rule): linear in source size per chunk. Long sources need
  a cheaper binding before the P14 load gates.
- The base model starts a segment that follows leading silence at its audio start
  (F09: 0.75 s, speech at 4.0 s). Measure in 3c; consider trimming leading silence.
- Ctrl-C is not trapped by the CLI (Tokio `signal` would be a dependency change).
- Model identity is hashed three times per run (about 0.3 s each in release); no
  cache added. Revisit if 3c's larger models make it dominate.

## Open decisions (maintainer)

- ADR 0017 review and its six recorded decisions (above).
- Local ASR, decided 2026-09-25: D1 request only through `transcript retranscribe`;
  D2 widen v1 schemas in place; D3 complete spliced revisions, older ones via `--revision`;
  D4 `setup check` `local_asr` object (3c); D5 refuse unpinned models; D6 `base-q5_1`
  and gates (3c); D7 no progress events yet; D8 no tombstones.
- faster-whisper adapter: backlog issue #147; whisper.cpp stays the default.
- Crate names are confirmed (`vsift` facade, `vsift-contract`); a crates.io
  availability check still precedes first publication.
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Supplied-transcript import and local ASR need real tools, so their success paths
  are opt-in (`--ignored`): `p07_transcript_e2e`, `p07_local_asr_e2e` (needs
  `VSIFT_TEST_WHISPER_CLI`, `VSIFT_TEST_WHISPER_MODEL`, FFmpeg on `PATH`; use
  `--release`), `engine_retranscribe`, `p07_local_asr` (infrastructure adapter test).
- whisper.cpp v1.9.2 `-ojf` output was valid UTF-8 (F08, forced Japanese/Russian); a
  split multi-byte token fails the chunk as `unparseable_output`; no `-oj` fallback.
- #128: process-supervisor tests failed intermittently on Windows under workspace
  load (again on 2026-09-25: `p04_preserves_invalid_bytes...`, `p03_caps_stdout...`),
  passing alone. Treat recurrences as evidence and add them to #128.
- #144: on one throttled Windows runner the concurrent-preflight test exceeded the 5 s
  session-root provisioning wait (not a regression). Add recurrences.
- The unchanged P06 checkpoint test and the stream contract tests use application
  and infrastructure types, so the CLI keeps both as development dependencies.
- FS-01: strict OS/storage-crash durability is unqualified. Durable requests fail
  closed until P10/P11/P14 run the Ubuntu/ext4 campaign (ADR 0010).
- Baseline findings B-01..B-11 close through their mapped packets.

## Parked: managed installation (now P13)

Resume order: smoke executor over the digest-bound policy; failure cleanup before
activation; guarded download/stage/smoke/publish; install, rollback and uninstall;
bounded version cleanup; kill and power-loss qualification; D-02..D-08 and the E2E
stage. The Ubuntu 24.04 x86-64 catalogue stops new plans on 2028-08-01.

## Guardrails

- R0 ships only when a coding agent goes from a local video to a grounded handoff
  through both the supplied-transcript and local-ASR paths, in named Codex and
  Claude Code trials (A-08/A-09).
- R1 packets P15..P20 (milestone 2, issues #25..#30) start only after P14.
- Live capture (#107, #108) waits for the finite-video journey.
- New features land in the engine (`crates/vsift`) once, never in a host. Any new
  media stage calls the preflight hook first. Wire sequencing lives in `vsift-contract`.
- A new public command, failure code, event kind or record type needs its
  `CommandName`, `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL`
  entry; conformance tests then require the v1 schemas to match.
