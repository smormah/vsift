# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P09 are complete. P10 (recovery integration) is implemented across its four
pull requests; the packet is complete once PR 4 merges and the ledger completion
record is written** (the supervisor writes it). PR 1 (`e2b14d9`), PR 2 (`2ae55be`) and
PR 3 (`8af331b`) are merged. **PR 4 (the Ubuntu 24.04 / ext4 crash campaign and
durable enablement) is complete on branch `p10/durability-campaign`**, awaiting review
and CI; no pull request is open yet.

1. **PR 4 delivered:** `tools/p10-crash-campaign/` and the `P10 durability campaign`
   workflow (manual and weekly): layer A dm-log-writes power loss at every flush,
   layer B QEMU kills of the pinned Ubuntu 24.04 image, layer C dm-flakey write
   errors, and the negative control (must lose acknowledgements). Evidence and run
   links: `docs/planning/p10-durable-publication.md`. `QUALIFIED_UBUNTU_EXT4` is set;
   the gate also checks Ubuntu 24.04 by `os-release`. `IngestRequest::durability`
   (engine only, D-3). Two defects found and fixed: storage failures reading committed
   state were `INTEGRITY_FAILURE` (now `STORAGE_IO`); an ingest reported the store's
   guarantee instead of its session's.
2. **Next:** review and merge PR 4; the ledger completion record for P10; close #13
   and #164. Then P11 (worker and batch host) is the earliest eligible packet.
3. D-3 stands: durable mode only through the engine API; the CLI via P11.

## Tracked issues

- #13: the P10 packet (close with the completion record). #164 resolved by PR 1.
- #144: stabilised in PR 3; close after a clean CI run on main.
- #170-#178: tracking issues for L-011, L-013, L-015, L-018, L-024, L-028, L-043,
  L-045 and L-042 (review pending).
- #159: regenerate the motion fixtures so F04/F05/F12 E02 differ visibly.
- #150: noisy-speech fixture set before any noise WER gate.
- #147: faster-whisper adapter (backlog); whisper.cpp stays the default.
- #128: process-supervisor tests fail intermittently on Windows under load.

## Other follow-ups

- **Known limits:** `docs/planning/known-limits.md` (57 entries to L-059, without L-012
  and L-048; review pending). PR 4 rewrote L-008 (durability qualified only on Ubuntu
  24.04 / ext4) and L-010, and added L-056 (storage that ignores flushes), L-057 (disk
  or host loss, X-10), L-058 (Ubuntu recognised by `os-release`, not kernel) and L-059
  (durable only through the engine).
- **Campaign upkeep:** when Ubuntu retires cloud-image release 20260911, bump the three
  `UBUNTU_IMAGE_*` values in the workflow together (see `docs/development.md`).
- Job retention: 64 jobs per session, 8 operation ids per job, 256 bindings, 16
  attempts, 1,024 checkpoints of 256 KiB. Evidence calls decode every record (L-013).

## Open decisions (maintainer)

- Review of the new `actions/download-artifact` pin (v8.0.1) and the rustix `termios`
  feature used only by the campaign tool (no new crate; `Cargo.lock` adds only the
  tool itself).
- Review of the Tokio `signal` feature (recorded in ADR 0020 PR 3 notes; no new crate).
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): `p07_transcript_e2e`,
  `p07_local_asr_e2e`, `p07_asr_qualification`, `p08_search_e2e`, `p08_candidates_e2e`,
  `p09_evidence_e2e`, `p10_recovery_e2e`, `engine_retranscribe`, `engine_jobs`,
  `p07_local_asr`, `p08_candidates_fixtures`, `p09_media_primitives`, `engine_evidence`,
  the S-11 measurements and the Windows console-interrupt test.
- The crash campaign runs on GitHub-hosted Linux only; its scripts create loop and
  device-mapper devices and VMs: never run them on a machine whose disks matter.
- whisper.cpp `-ojf` output: a split multi-byte token fails the chunk.
- The CLI keeps application, infrastructure and domain as development dependencies for
  tests that seed session records; hosts depend only on `vsift` and `vsift-contract`.
- Durability ends at the disk: storage that ignores flushes (L-056) and disk
  or host loss (L-057) are outside it; Windows/macOS durable requests fail closed.

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
  or `durability-campaign` in a release build.
- A change to the commit path, a directory synchronisation or the durable profile must
  rerun the crash campaign (all three layers and the negative control) before merge.
- Map a storage failure reading committed state with `map_committed_io`, never straight
  to `IntegrityFailure`.
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas.
- A long command gets the one `Cancellation` from `execute_with`; a new provider stage
  must honour it at its boundaries (see the CLI `signal` module).
