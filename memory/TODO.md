# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P09 are complete; P10 (recovery integration) is in progress.** P10 is four pull
requests under [ADR 0020](../docs/decisions/0020-recoverable-jobs-and-durable-publication.md),
accepted by the maintainer on 2026-09-27 with D-1..D-5 as recommended. **PR 1 (commit
path) is merged (`e2b14d9`). PR 2 (jobs, keys and checkpointed retranscription) is
complete on branch `p10/jobs-checkpoints`**, awaiting review and CI (its Unix-only
test code is compiled only there). The packet is not complete.

1. **PR 2 delivered:** `JobState` `Interrupted`/`Committing`, the retry policy and poison
   rule (domain); request digest, recognition key, operation key, job id and commit
   operation ids, the `JobStore`/`ChunkCheckpoints`/`CommitLedger`/`RevisionStore` ports,
   `run_retranscription`, `CancelJob`, `JobStatusQuery`, reconcile (application); jobs,
   checkpoints, bindings and the root job index in session storage with nine job fault
   points (infrastructure); `Engine::retranscribe` as a job with an optional operation
   id, `Engine::job_status/job_resume/job_cancel`, `data.job`, the envelope
   `operation_id`, `IDEMPOTENCY_CONFLICT` (exit 2), `BUSY` with `affected_ids` and
   `retry_after_ms`; caps 512 / 384 / 128 KiB (D-2); L-048 resolved.
2. **Next, in order:** PR 3 the public `job status/resume/cancel` commands, the
   `--operation-id` flag on `transcript retranscribe` (D-1), cancellation events, and
   Ctrl-C/SIGTERM via Tokio's `signal` feature (supersedes ADR 0017 decision 4; needs
   the dependency review); PR 4 the Ubuntu 24.04 / ext4 crash campaign (D-5: hosted
   `ubuntu-24.04` with KVM first, else a maintainer-owned KVM host), then flip
   `QUALIFIED_UBUNTU_EXT4`.
3. D-3 stands: durable mode only through the engine API in P10, the CLI via P11.

## Tracked issues

- #164: resolved by PR 1 (merged); close it. #13: the P10 packet.
- #170-#178: tracking issues for L-011, L-013, L-015, L-018, L-024, L-028, L-043,
  L-045 and L-042 (opened 2026-09-27; review pending).
- #159: regenerate the motion fixtures so F04/F05/F12 E02 differ visibly.
- #150: noisy-speech fixture set before any noise WER gate.
- #147: faster-whisper adapter (backlog); whisper.cpp stays the default.
- #128: process-supervisor tests fail intermittently on Windows under load.
- #144: a throttled Windows runner once exceeded the 5 s session-root provisioning wait.

## Other follow-ups

- **Known limits:** `docs/planning/known-limits.md` (50 entries to L-052, without L-012
  and L-048; review pending). Add every new limit in the same change.
- PR 2 fixed two X-04 races: a head without a revision paired with a newer generation,
  and false `INTEGRITY_FAILURE` from readers meeting a rename-replace (L-052).
- Job retention: 64 jobs per session (oldest ended unpinned job pruned), 8 caller
  operation ids per job, 256 bindings, 16 attempts, 1,024 checkpoints of 256 KiB.
- Evidence: bursts denser than one 1,200-frame listing are rejected (`outside_listing`);
  tiny text measured on synthetic glyphs only; neighbours list up to three windows.
- Every evidence call decodes all evidence records (at most 384); an index later (L-013).
- Search: no accent folding or Unicode normalisation; phrases don't cross segments. A
  creator killed mid-provisioning leaves an unmarked root.

## Open decisions (maintainer)

- Tokio `signal` feature for PR 3 (dependency review).
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): `p07_transcript_e2e`,
  `p07_local_asr_e2e`, `p07_asr_qualification`, `p08_search_e2e`, `p08_candidates_e2e`,
  `p09_evidence_e2e`, `engine_retranscribe`, `engine_jobs` (FFmpeg; one test also
  whisper.cpp), `p07_local_asr`, `p08_candidates_fixtures`, `p09_media_primitives`,
  `engine_evidence`, the S-11 measurements (`engine_search`, `engine_candidates`,
  `s11_warm_reuse_*`, `--release`) and the source-binding multi-chunk decode.
- whisper.cpp `-ojf` output: a split multi-byte token fails the chunk.
- The CLI keeps application, infrastructure and domain as development dependencies for
  tests that seed session records; hosts depend only on `vsift` and `vsift-contract`.
- FS-01: strict OS/storage-crash durability is unqualified; durable requests fail closed
  until P10 PR 4's campaign passes (ADR 0010, ADR 0020).
- Sessions written by this version may hold `jobs/` (older builds ignore it) and
  manifests over 64 KiB (older builds reject them; sessions are disposable).

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
- Session commits go through `CommitHooks`/`Commit`; a new commit or job boundary needs a
  `FaultPoint` (`COMMIT` or `JOB`) reached by a kill test. Never enable `fault-injection`
  in a release build.
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas.
