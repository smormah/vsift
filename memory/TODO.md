# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P10 are complete. P11 (worker and batch host) is in progress: PR 1 is complete
on its branch `p11/contracts-events`; the packet is not.** ADR 0021 is accepted with
maintainer decisions D1-D5 (2026-09-28).

1. **PR 1 (contracts, events, progress, fuzzing): done on its branch, awaiting review
   and merge.** Job request/result, batch summary and workspace data contracts with
   schemas, examples and a public strict decoder; `progress`, `lifecycle` and `result`
   event kinds; chunk progress on `transcript retranscribe` and `job resume`
   (`--events jsonl`); fuzz targets `job_request`, `job_batch_line`, `job_record`,
   `chunk_checkpoint`. The PR description should carry the close-out note for #180
   (fuzz targets added); the maintainer closes it.
2. **Next: PR 2** - `session init-workspace` (D1, D2), durable workspace marker and
   CLI durable mode (`ingest --session-root <workspace>`, ADR 0020 D-3), weighted
   admission with `AdmissionWait`, strict-Linux attestation (`ISOLATION_UNAVAILABLE`
   otherwise), contained `--input-root` / `--bundle-root`.
3. **Then PR 3** - `job run`, request records (`worker-requests/<bucket>/<op>.json`,
   replay / `BUSY` / continue / `IDEMPOTENCY_CONFLICT`), step idempotency, graceful
   shutdown (D4), the `request_record` fuzz target.
4. **Then PR 4** - `job batch` (streaming reader, backpressure, D5 exit), SEC-T01,
   the external-delivery simulation and the P11 qualification record.

## Decided (maintainer, 2026-09-28)

- **D1** Workspaces are created explicitly by `session init-workspace` with operator
  policy.
- **D2** Durable-workspace sessions live a finite, workspace-set time (default 168 h,
  max 720 h, renewable within it).
- **D3** v1 request steps: `ingest` (with supplied-transcript import), `retranscribe`,
  `candidates`, `retain`, `close`; not search/frame/crop/audio.
- **D4** The first shutdown signal stops admitting and cancels in-flight work at its
  next boundary; drain is opt-in via `--drain-timeout-ms` (at most 300 s).
- **D5** `job batch` exits 0 when every request is complete or partial, 6 when a
  shutdown stopped it, else the most severe failure class (7 > 1 > 5 > 3 > 2 > 4).

## Tracked issues

- #180: fuzz targets for the job record and checkpoint added in P11 PR 1; close after
  merge.
- #144: stabilised in P10 PR 3; close after a clean CI run on main.
- #170-#178: tracking issues for L-011, L-013, L-015, L-018, L-024, L-028, L-043,
  L-045 and L-042 (review pending).
- #159: regenerate the motion fixtures so F04/F05/F12 E02 differ visibly.
- #150: noisy-speech fixture set before any noise WER gate.
- #147: faster-whisper adapter (backlog); whisper.cpp stays the default.
- #128: process-supervisor tests fail intermittently on Windows under load.

## Other follow-ups

- **Known limits:** `docs/planning/known-limits.md` (57 entries to L-059, without L-012
  and L-048; review pending). P11 PR 1 rewrote L-025 (progress now exists; it is coarse
  and advisory) and L-038 (contracts published, commands still reserved).
- **Campaign upkeep:** bump the three `UBUNTU_IMAGE_*` values together when Ubuntu
  retires release 20260911; raise `--max-ops` for layer C's job-free rounds.

## Open decisions (maintainer)

- The D5 detail in ADR 0021's PR 1 notes: a request cancelled without a shutdown ranks
  between usage (2) and retryable (4); an input error counts as 7, a line limit as 5.
- Review of the new `actions/download-artifact` pin (v8.0.1), the rustix `termios`
  feature of the campaign tool and the Tokio `signal` feature (no new crates).
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): `p07_transcript_e2e`,
  `p07_local_asr_e2e`, `p07_asr_qualification`, `p08_search_e2e`, `p08_candidates_e2e`,
  `p09_evidence_e2e`, `p10_recovery_e2e`, `engine_retranscribe`, `engine_jobs`,
  `p07_local_asr`, `p08_candidates_fixtures`, `p09_media_primitives`, `engine_evidence`,
  the S-11 measurements and the Windows console-interrupt test.
- The crash campaign runs on GitHub-hosted Linux only; never run its scripts on a
  machine whose disks matter.
- The CLI keeps application, infrastructure and domain as development dependencies for
  tests that seed session records; hosts depend only on `vsift` and `vsift-contract`.
- Durability ends at the disk (L-056, L-057); Windows/macOS durable requests fail closed.

## Guardrails

- R0 ships only when a coding agent goes from a local video to a grounded handoff through
  both the supplied-transcript and local-ASR paths, in named Codex and Claude Code trials.
- R1 packets P15..P20 start only after P14. Live capture (#107, #108) waits. Managed
  installation is parked in P13.
- New features land in the engine once, never in a host; the worker's public wire types
  are `WorkRequest` / `WorkResult`, never the application's `JobRequest`.
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas;
  a new parser of untrusted input needs a fuzz target with committed-seed provenance.
- Session commits go through `CommitHooks`/`Commit`; a change to the commit path reruns
  the crash campaign. Never enable `fault-injection` or `durability-campaign` in a release.
- A long command gets the one `Cancellation` from `execute_with`; progress goes through
  the engine's `ProgressObserver`, never blocks, and never reaches a non-events mode.
