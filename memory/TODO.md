# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

Delivery was re-planned on 2026-09-23 ([ADR 0015](../docs/decisions/0015-r0-delivery-replan.md),
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md)). P00-P06 are
complete. P07 is in progress: increments 1a-3a and the fuzz targets are merged; 3b and
3c are done on their branches; the packet is not complete. Local checks: CI is the
default Linux/macOS check; Docker only for platform code.

1. **Done (merged):** 1a `vsift-contract` wire types (PR #126); 1b engine facade, thin CLI
   (PR #127); 2 supplied transcripts (PR #129); 2b media-tool preflight (PR #133); 2c
   JSONL evidence stream and bundle record schema (PR #134); speech fixtures (PRs
   #138-#140, #142, #143); 3a local ASR core (PR #145); `cargo-fuzz` targets (PR #146).
2. **In review: 3b (PR #149, branch `p07/asr-retranscribe`).** `transcript
   retranscribe` under D1-D8; [ADR 0017](../docs/decisions/0017-local-asr-through-whisper-cpp.md)
   is **Proposed** with six decisions awaiting confirmation.
3. **Done on branch `p07/asr-setup-profiles` (on top of 3b), not pushed: 3c**, whole
   increment, green locally:
   - D4: `setup check` `local_asr` object (model identity; verification recorded, ran
     now within its own 60 s budget, failed with typed check/reason, or not run).
     Legacy fields and exit status unchanged; real run passed for both profiles.
   - D6: reviewed `base_q5_1` profile (pinned at HF revision `5359861`, because the
     base revision `80da2d8` has no quantized files: **deviation to confirm**).
   - T-04: test-only scoring and the opt-in `p07_asr_qualification` test; record
     `docs/planning/p07-asr-qualification.md`.
   - Seam-merge fix: a sentence starting at a chunk's first sample was dropped
     (found with q5_1). Opt-in `P07 local ASR` workflow (Ubuntu, Windows), not run.
4. **Measured default outcome (maintainer decision needed):** `base` meets clean WER
   (3.25%), critical terms, RTF (0.388) and memory (338 MiB) but **fails F08 WER
   (61.5% vs 25%)**; `base_q5_1` also fails F08 (46.2%). Options are in the record;
   the qualification test fails on F08 until the gate or default is decided.
5. **Still owed by P07 before the packet closes:** merge of 3b and 3c, the default
   decision, a run of the `P07 local ASR` workflow (Linux evidence), then the packet
   completion record in the delivery ledger (supervisor writes it after merge).

## Follow-ups (open issues before relying on them)

- #148: every speech chunk rehashes the session's whole source copy before FFmpeg
  reads it (linear in source size per chunk); needs a cheaper binding before P14.
- #147: faster-whisper adapter (backlog). A creator killed mid-provisioning leaves an
  unmarked root refused until removed.
- F09: base starts a segment at its audio start after leading silence (0.75 s, not 4.0 s).
- Ctrl-C is not trapped (Tokio `signal` would be a new dependency). The model is hashed
  up to three times per run and twice per `setup check` (~0.3 s each); no cache.
- Accent and crosstalk are not in the speech corpus (T-04 gap); F08 (13 words) is
  the only noisy clip.

## Open decisions (maintainer)

- The local-ASR default under D6 (F08 gate not met): keep `base` with F08 as a known
  limit, qualify a larger profile, or add noisy fixtures first (see the record).
- The q5_1 pin revision (`5359861` instead of `80da2d8`, same repository).
- ADR 0017 review and its six recorded decisions.
- Crate names confirmed (`vsift`, `vsift-contract`); crates.io check precedes publication.
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): `p07_transcript_e2e`,
  `p07_local_asr_e2e` and `p07_asr_qualification` (need `VSIFT_TEST_WHISPER_CLI`,
  `VSIFT_TEST_WHISPER_MODEL`, optionally `VSIFT_TEST_WHISPER_MODEL_Q5_1`, FFmpeg on
  `PATH`; use `--release`), `engine_retranscribe`, `p07_local_asr`.
- whisper.cpp v1.9.2 `-ojf` output was valid UTF-8 (F08, forced Japanese/Russian); a
  split multi-byte token fails the chunk as `unparseable_output`; no `-oj` fallback.
- #128: process-supervisor tests fail intermittently on Windows under workspace load
  (passing alone). Treat recurrences as evidence and add them to #128.
- #144: on one throttled Windows runner the concurrent-preflight test exceeded the 5 s
  session-root provisioning wait (not a regression). Add recurrences.
- The P06 checkpoint test and the stream contract tests use application and
  infrastructure types, so the CLI keeps both as development dependencies.
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
