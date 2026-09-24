# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

Delivery was re-planned on 2026-09-23 ([ADR 0015](../docs/decisions/0015-r0-delivery-replan.md),
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md)). P00-P06 are
complete. P07 is in progress: increment 3a (local ASR core) is done, the packet is not.
Local checks: CI is the default Linux/macOS check; Docker only for platform code.

1. **Done:** 1a `vsift-contract` wire types (PR #126); 1b engine facade, thin CLI
   (PR #127); 2 supplied transcripts (PR #129); 2b media-tool preflight (PR #133); 2c
   JSONL evidence stream and bundle record schema (PR #134); speech fixtures (PRs
   #138-#140, #142, #143; `docs/planning/p07-speech-fixtures.md`).
2. **Done: increment 3a, local ASR core (this PR, branch `p07/asr-core`).** Not
   reachable from any command. Revision provenance generalised (import or local ASR
   run, per-segment origin), domain chunking/validation/silence/seam merge, the
   `transcribe_range` and `build_asr_revision` use cases, `FfmpegMedia::speech_pcm`,
   the whisper.cpp CLI adapter and `-ojf` parser, transcript record version 2. Also
   fixed: `setup check` no longer echoes whisper's loader line (it held a path).
3. **Next: increment 3b, pending maintainer decisions D1-D8** (raised with 3a):
   - `transcript retranscribe` as a superseding revision, calling
     `Engine::ensure_media_tools_verified` first; where the ASR work directory lives;
   - public schemas for local-ASR revisions and segments (and the v2 record schema);
   - whether superseded revisions need tombstone events in the evidence stream;
   - what an ASR run that heard no speech produces (today: no revision, typed error);
   - ADR 0017 recording the local-ASR contract; T-03..T-06 and the `p07_local_asr`
     E2E stage through the command, including a multi-chunk real-speech seam check.
4. **Then: increment 3c:** `setup check` reports `local_asr` functional verification
   (model and provider actually transcribe F01); a quantized model profile beside the
   pinned base; measure and set the default profile (ADR 0005 gates).
5. **Still owed by P07 before the packet closes:** `cargo-fuzz` targets for the SRT/VTT
   parsers (and now the whisper `-ojf` parser) as a scheduled nightly-toolchain CI job;
   the packet completion record in the ledger.

## Follow-ups (open issues before relying on them)

- A creator killed mid-provisioning leaves an unmarked root refused until removed.
- The evidence stream has no delete/tombstone events; retranscription (3b) must
  define them or document why not.
- Recognizer identity hashes the 148 MB model before and after every run. The opt-in
  run took about 46 s per short clip in a debug build and 8 s in release (hashing
  dominates debug). Consider caching the identity by size and modification time.

## Open decisions (maintainer)

- D1-D8 for increment 3b/3c (see the 3a pull request description).
- Whether to open a backlog issue for a faster-whisper adapter (proposed; awaiting
  the maintainer). whisper.cpp stays the default.
- Crate names are confirmed (`vsift` facade, `vsift-contract`); a crates.io
  availability check still precedes first publication.
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Supplied-transcript import needs real FFprobe, so import success paths are opt-in
  (`--ignored`). The local-ASR adapter test is opt-in too (`p07_local_asr`, needs
  `VSIFT_TEST_WHISPER_CLI`, `VSIFT_TEST_WHISPER_MODEL` and FFmpeg on `PATH`).
- whisper.cpp v1.9.2 `-ojf` output was valid UTF-8 for the F08 clip (which has no
  accented text) and for forced Japanese and Russian decodes. A token that splits a
  multi-byte character would make the file invalid UTF-8; the parser then fails the
  chunk (`unparseable_output`). No `-oj` fallback was needed.
- `bundle validate` now accepts record version 2 (strictly decoded); no command
  writes it and its schema is not published yet.
- #128: one local Windows run saw four `process_supervisor` tests fail (child exit
  status), then pass. Treat a recurrence as evidence and add it to #128.
- #144: on one throttled Windows runner the concurrent-preflight test exceeded the 5 s
  session-root provisioning wait (not a regression; 60/60 stress passes). Add recurrences.
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
