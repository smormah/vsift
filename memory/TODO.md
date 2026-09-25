# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

Delivery was re-planned on 2026-09-23 ([ADR 0015](../docs/decisions/0015-r0-delivery-replan.md),
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md)). **P00-P07 are
complete.** P07 (engine boundary and transcription) closed on 2026-09-25 with merge
`9ea3180`; its evidence is in the ledger. Local checks: CI is the default Linux/macOS
check; Docker only for platform code.

1. **P07 delivered:** `vsift-contract` and the `vsift` engine facade; supplied SRT/WebVTT
   import; automatic media-tool preflight; JSONL evidence stream; Kokoro speech
   fixtures (test-only); local ASR through whisper.cpp v1.9.2 (`transcript
   retranscribe`, spliced revisions, `transcript get --revision`,
   [ADR 0017](../docs/decisions/0017-local-asr-through-whisper-cpp.md)); `setup check`
   `local_asr` reporting; `base` (default) and `base_q5_1` profiles; T-04 scoring;
   weekly `Fuzz` and `P07 local ASR` workflows. Qualification: workflow run
   36175016465 passed on Ubuntu 24.04 and Windows 2025 (base clean WER 3.25%).
2. **Next: P08 (candidate/search index), not started.** Governance rule 10: the
   maintainer starts the next packet. Before implementation read the P08 row of
   `docs/planning/implementation-work-packets.md` and V-02..V-05, C-03, S-11.
3. **Candidate small fix before or alongside P08:** #153 (pin the optimised Ubuntu
   whisper.cpp CPU backends; Linux RTF 3.2 today vs 0.28 on Windows).

## Tracked issues

- #153: the reviewed Ubuntu whisper.cpp pin keeps only the generic x64 CPU backend,
  so Linux ASR runs about 11x slower (RTF 3.245) than Windows (0.284).
- #150: noisy-speech fixture set (with accent and crosstalk) before any noise WER
  gate; F08 (13 words) is the only noisy clip today.
- #148: every speech chunk rehashes the session's whole source copy before FFmpeg
  reads it (linear in source size per chunk); needs a cheaper binding before P14.
- #147: faster-whisper adapter (backlog); whisper.cpp stays the default.
- #128: process-supervisor tests fail intermittently on Windows under workspace load
  (passing alone). Add recurrences as evidence.
- #144: on one throttled Windows runner the concurrent-preflight test exceeded the 5 s
  session-root provisioning wait (not a regression). Add recurrences.

## Other follow-ups

- A creator killed mid-provisioning leaves an unmarked root refused until removed.
- F09: base starts a segment at its audio start after leading silence (0.75 s, not
  4.0 s); consider trimming leading silence.
- Ctrl-C is not trapped (Tokio `signal` would be a new dependency). The model is hashed
  up to three times per run and twice per `setup check` (~0.3 s each); no cache.
- The local-ASR checkpoint's word checks are written for `base`; `base_q5_1` hears
  F05's "invoice" as "in voice", so the workflow measures q5_1 only in T-04.

## Open decisions (maintainer)

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
