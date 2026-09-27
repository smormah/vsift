# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P09 are complete; P10 (recovery integration) is in progress.** P10 is four pull
requests under [ADR 0020](../docs/decisions/0020-recoverable-jobs-and-durable-publication.md),
accepted by the maintainer on 2026-09-27 with D-1..D-5 as recommended. PR 1 (commit
path, `e2b14d9`) and PR 2 (jobs and checkpointed retranscription, `2ae55be`) are merged.
**PR 3 (the public job surface and interruption handling) is complete on branch
`p10/job-surface`**, awaiting review and CI (its Unix-only code was cross-checked with
Clippy for Linux on Windows, not run). The packet is not complete.

1. **PR 3 delivered:** `job status/resume/cancel <job>` (no session; found through the
   root job index) with `job-data` and `job-resume-data` schemas; `session status`
   `jobs[]` (16) and `jobs_truncated`; `transcript retranscribe --operation-id` (D-1);
   the CLI `signal` module (first SIGINT/SIGTERM or Ctrl-C/Ctrl-Break cancels long
   commands, the second escalates); `job cancel` reaches a running owner through a
   250 ms watcher; Ctrl-C during retranscribe is `CANCELLED` naming session and job
   with a `vsift job resume <job>` command; ingest copy cancellation; #144 stabilised.
2. **Next:** PR 4, the Ubuntu 24.04 / ext4 crash campaign (D-5: hosted `ubuntu-24.04`
   with KVM first, else a maintainer-owned disposable KVM host), X-10, then flip
   `QUALIFIED_UBUNTU_EXT4` and close the packet.
3. D-3 stands: durable mode only through the engine API in P10, the CLI via P11.

## Tracked issues

- #13: the P10 packet. #164 resolved by PR 1: close it.
- #144: stabilised in PR 3 (injectable root wait); close after a clean CI run.
- #170-#178: tracking issues for L-011, L-013, L-015, L-018, L-024, L-028, L-043,
  L-045 and L-042 (review pending).
- #159: regenerate the motion fixtures so F04/F05/F12 E02 differ visibly.
- #150: noisy-speech fixture set before any noise WER gate.
- #147: faster-whisper adapter (backlog); whisper.cpp stays the default.
- #128: process-supervisor tests fail intermittently on Windows under load.

## Other follow-ups

- **Known limits:** `docs/planning/known-limits.md` (53 entries to L-055, without L-012
  and L-048; review pending). PR 3 added L-053 (Windows inherited "ignore Ctrl-C"),
  L-054 (a second interrupt cannot cut VSift's own hashing short) and L-055 (a
  hard-killed Unix CLI's provider finishes its unit); rewrote L-010, L-025, L-038, L-041.
- Job retention: 64 jobs per session, 8 caller operation ids per job, 256 bindings,
  16 attempts, 1,024 checkpoints of 256 KiB.
- Evidence calls decode every evidence record (at most 384); an index later (L-013).
- Search: no accent folding or Unicode normalisation; phrases don't cross segments.

## Open decisions (maintainer)

- Review of the Tokio `signal` feature (recorded in ADR 0020 PR 3 notes; no new crate).
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): `p07_transcript_e2e`,
  `p07_local_asr_e2e`, `p07_asr_qualification`, `p08_search_e2e`, `p08_candidates_e2e`,
  `p09_evidence_e2e`, `p10_recovery_e2e`, `engine_retranscribe`, `engine_jobs`,
  `p07_local_asr`, `p08_candidates_fixtures`, `p09_media_primitives`, `engine_evidence`,
  the S-11 measurements and the Windows console-interrupt test
  (`interrupt_cli_contract`, via `tools/send-console-ctrl.ps1`).
- whisper.cpp `-ojf` output: a split multi-byte token fails the chunk.
- The CLI keeps application, infrastructure and domain as development dependencies for
  tests that seed session records; hosts depend only on `vsift` and `vsift-contract`.
- FS-01: strict OS/storage-crash durability is unqualified; durable requests fail closed
  until P10 PR 4's campaign passes (ADR 0010, ADR 0020).
- Sessions written by this version may hold job records with `planned_chunks`, which
  PR 2 builds reject, and manifests over 64 KiB (older builds reject them; disposable).

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
- A long command gets the one `Cancellation` from `execute_with`; a new provider stage
  must honour it at its boundaries (see the CLI `signal` module).
