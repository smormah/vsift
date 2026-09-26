# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P09 are complete; P10 (recovery integration) is in progress.** P10 is planned as
four pull requests under [ADR 0020](../docs/decisions/0020-recoverable-jobs-and-durable-publication.md)
(Proposed). **PR 1 (commit path) is complete on branch `p10/commit-path`**, awaiting
review and CI (its Linux-only code is compiled only there). The packet is not complete.

1. **PR 1 delivered (internal, no public contract change):**
   - #164 resolved: reads validate the manifest chain only down to the writer's
     `chain-verified.json`; warm reuse p95 136-175 ms at 256 generations and
     149-156 ms at 1,024 (was 1,064 / 3,794 ms), slope about 0.
   - Durable publication protocol (flush, then sync `artifacts/`, `generations/`, the
     session; fsyncgate-safe retries), durability recorded per session, disabled:
     `durable_profile` needs Linux ext4 with barriers and `QUALIFIED_UBUNTU_EXT4`
     (`false`).
   - Eleven `FaultPoint`s (dev-only `fault-injection` feature), S-07 kill test at every
     point, durable order trace test, fuzz target `mountinfo` (15 targets).
2. **Next, in order:** PR 2 jobs, operation keys and checkpointed `transcript
   retranscribe` (ChunkCheckpoint, `Interrupted`/`Committing`, retry policy); PR 3
   public job surface and cancellation (Ctrl-C/SIGTERM via Tokio `signal`); PR 4 the
   Ubuntu 24.04 / ext4 crash campaign, then flip `QUALIFIED_UBUNTU_EXT4`.
3. **Pending maintainer decisions (ADR 0020):** D-1 `--operation-id` on `transcript
   retranscribe` only; D-2 caps 512 artifacts / 384 evidence / 128 KiB manifests
   (ADR 0019 D4 unchanged until then); D-3 durable mode via the engine API only in P10;
   D-4 `IDEMPOTENCY_CONFLICT` (exit 2); D-5 campaign on hosted ubuntu-24.04 with KVM,
   else a maintainer-owned KVM host. PR 2 needs D-1 and D-4.

## Tracked issues

- #164: resolved on the PR 1 branch; close when it merges. #13: the P10 packet.
- #159: regenerate the motion fixtures so F04/F05/F12 E02 differ visibly.
- #150: noisy-speech fixture set before any noise WER gate.
- #147: faster-whisper adapter (backlog); whisper.cpp stays the default.
- #128: process-supervisor tests fail intermittently on Windows under load.
- #144: a throttled Windows runner once exceeded the 5 s session-root provisioning wait.

## Other follow-ups

- **Known limits register:** `docs/planning/known-limits.md` (L-001..L-048, review
  pending); delete L-012 once PR 1 merges. Add every new limit in the same change.
- #164's p95 target (160 ms at 256) was met in one run of three (start-up jitter).
- Evidence: a burst over a range denser than one 1,200-frame listing (60 fps over more
  than 20 s) is rejected (`outside_listing`). Tiny text is measured on synthetic glyphs
  only. Neighbours list up to three windows (2, 10, 29 s).
- Evidence records are read and decoded in full on every evidence call (at most 160
  records of 256 KiB); fine for R0, an index would help later.
- Candidates: real screen recordings are unmeasured; no denser pass for sub-0.5 s
  changes; the probe's duration is part of the index scope.
- Search: no accent folding or Unicode normalisation; phrases do not cross segments.
- A creator killed mid-provisioning leaves an unmarked root; Ctrl-C is not trapped (PR 3).

## Open decisions (maintainer)

- ADR 0020 D-1..D-5 (above).
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): `p07_transcript_e2e`,
  `p07_local_asr_e2e`, `p07_asr_qualification`, `p08_search_e2e`, `p08_candidates_e2e`,
  `p09_evidence_e2e`, `engine_retranscribe`, `p07_local_asr`,
  `p08_candidates_fixtures`, `p09_media_primitives`, `engine_evidence`, the S-11
  measurements (`engine_search`, `engine_candidates`,
  `s11_warm_reuse_as_the_manifest_chain_grows`, `--release`) and
  `source_binding::tests::a_real_multi_chunk_decode_hashes_the_copy_exactly_twice`.
- whisper.cpp `-ojf` output: a split multi-byte token fails the chunk.
- The CLI keeps application, infrastructure and domain as development dependencies for
  tests that seed session records; hosts depend only on `vsift` and `vsift-contract`.
- FS-01: strict OS/storage-crash durability is unqualified; the protocol exists but
  durable requests fail closed until P10 PR 4's campaign passes (ADR 0010, ADR 0020).
- Sessions written by this version may hold `chain-verified.json` and
  `verified_source_identity`; older builds ignore the first and reject the second
  (sessions are disposable). A durable manifest's `durability` field is new too.

## Parked: managed installation (now P13)

Resume order: smoke executor over the digest-bound policy; failure cleanup before
activation; guarded download/stage/smoke/publish; install, rollback and uninstall;
bounded version cleanup; kill and power-loss qualification; D-02..D-08 and the E2E stage.

## Guardrails

- R0 ships only when a coding agent goes from a local video to a grounded handoff through
  both the supplied-transcript and local-ASR paths, in named Codex and Claude Code trials.
- R1 packets P15..P20 start only after P14. Live capture (#107, #108) waits.
- New features land in the engine once, never in a host; media stages call the
  preflight hook first and multi-call providers take a `BoundSource` (D1 for evidence).
- Session commits go through `CommitHooks`/`Commit`; a new commit boundary needs a
  `FaultPoint` and must be reached by the kill test. Never enable `fault-injection` in
  a release build.
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas.
