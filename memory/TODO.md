# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

Delivery was re-planned on 2026-09-23 ([ADR 0015](../docs/decisions/0015-r0-delivery-replan.md),
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md)). P00-P06 are
complete. P07 is in progress: five increments are done, the packet is not.
Local checks: CI is the default Linux/macOS check; Docker only for platform code.

1. **Done:** 1a `vsift-contract` wire types (PR #126, `00e707f`); 1b `vsift` engine
   facade, thin CLI (PR #127, `0e5a0cd`); 2 supplied transcripts (PR #129,
   `4aa4ae0`); 2b automatic media-tool preflight (PR #133, `27f9416`); 2c JSONL
   evidence stream and bundle transcript-record schema (PR #134, `08d9830`).
2. **Speech fixtures for local ASR (in progress, test-only).** The maintainer chose
   Kokoro-82M, pinned, for test clips only. Generator, verifier and the
   `p07-speech-fixtures.yml` workflow merged (PR #138, `44613df`); F09 speaks at 1.3x
   to fit its frozen window (PR #139, `3305400`). Run 36049056343 passed; whisper.cpp
   review found F08's English "AB" spoken as "ob". PR #140 adds a reviewed
   pronunciation hint. Next: re-run the workflow, repeat the whisper review, commit
   the clips with provenance in one PR citing the run, and pin the spaCy wheel hash.
3. **Increment 3 (next): local ASR** with whisper.cpp v1.9.2 (official build), the
   maintainer's choice:
   - whisper.cpp adapter over the process supervisor, output validated and offset to
     source time; imported and ASR revisions share `TranscriptRevision`;
   - bounded PCM chunking with overlap and deterministic duplicate removal at seams;
   - call `Engine::ensure_media_tools_verified` before its first media stage;
   - whisper functional verification reported by `setup check`;
   - a lightweight (quantized) model option alongside the pinned base model;
   - `transcript retranscribe` (still `COMMAND_NOT_IMPLEMENTED`) as a new revision;
     decide then whether superseded revisions need tombstone events in the stream;
   - T-03..T-06 and the `p07_local_asr` E2E stage.
4. **Still owed by P07 before the packet closes:**
   - `cargo-fuzz` targets for the SRT/VTT parsers (ADR 0016 decision 6) as a scheduled
     nightly-toolchain CI job (maintainer approved); `proptest` covers them now;
   - the packet completion record in the ledger once increment 3 merges.
5. **Fixed outside the packet:** #131 racing first uses converge on one session root
   (PR #135, `a5d799a`); #136 verification-record reader classification and bounded
   lock retries, #132 stale verification workspaces swept (PR #137, `617d631`).
   Private folders (this change): every folder VSift creates is made private itself,
   so a Windows profile whose `%LOCALAPPDATA%` passes other accounts' entries no
   longer breaks `setup configure`; an existing non-private folder fails `STORAGE_IO`
   with remediation naming the folder kind.

## Follow-ups (open issues before relying on them)

- A creator killed mid-provisioning leaves an unmarked root refused until removed.
- The evidence stream has no delete/tombstone events; the first operation that
  supersedes evidence (retranscription) must define them or document why not.

## Open decisions (maintainer)

- Whether to open a backlog issue for a faster-whisper adapter (proposed; awaiting
  the maintainer). whisper.cpp stays the default.
- Crate names are confirmed (`vsift` facade, `vsift-contract`); a crates.io
  availability check still precedes first publication.
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Supplied-transcript import needs real FFprobe, so import success paths are opt-in
  (`--ignored`). The JSONL stream and bundle checks run everywhere: their tests commit
  a transcript session straight through the session store.
- #128: one local Windows run saw four `process_supervisor` tests fail (child exit
  status), then pass. Treat a recurrence as evidence and add it to #128.
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
