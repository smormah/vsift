# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P10 are complete. P11 (worker and batch host) is in progress: PR 1 is merged
(`0bcfac5`); PR 2 is complete on its branch `p11/workspace-admission`; the packet is
not.** ADR 0021 is accepted with maintainer decisions D1-D5 (2026-09-28).

1. **PR 2 (workspace, admission, attestation, contained inputs): done on its branch,
   awaiting review and merge.** `session init-workspace` with an immutable operator
   policy; `ingest --session-root <workspace>` inherits its durability (ADR 0020 D-3);
   weighted admission (visual window 2, copy/evidence 1, recognition its threads capped
   at min(parallelism, 8, capacity)), `AdmissionWait`, `Cancellation::child`;
   `--host-isolation strict-linux` with the kernel attestation; `InputRoot` contained
   inputs; the Unix free-space reserve; `controls.resource_limits` and
   `free_space_reserve`; fuzz target `host_attestation`.
2. **Next: PR 3** - `job run`, request records (`worker-requests/<bucket>/<op>.json`,
   replay / `BUSY` / continue / `IDEMPOTENCY_CONFLICT`), step idempotency, graceful
   shutdown (D4), the `request_record` fuzz target, `--input-root` and `--bundle-root`
   flags, the engine request path over `InputRoot`, `AdmissionWait` on every step and
   the `admission_waiting` lifecycle event, and a rerun of the crash campaign if the
   commit path changes.
3. **Then PR 4** - `job batch` (streaming reader, backpressure, D5 exit), SEC-T01 (the
   strict attestation in a container against a hostile fixture), the
   external-delivery simulation and the P11 qualification record.

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

## Open decisions (maintainer)

- **D2 reading (PR 2).** Implemented as: a workspace session expires the retention
  after opening or renewal, never beyond 720 h from opening. Confirm, or choose the
  retention itself as the hard limit.
- **Ephemeral workspace mode (PR 2).** Its sessions report `lifecycle.mode`
  `durable_worker` (the mode names the retention rules; `publication` says
  `process_crash_consistent`). Confirm or ask for a separate mode value.
- **Links in input paths (PR 2).** Every link on a request path is refused, even one
  inside the input root (stricter than ADR 0021 section 9; L-062).
- The D5 detail in ADR 0021's PR 1 notes (cancelled-without-shutdown ranking).
- MSRV policy before the library is first published; whether and when to cut 0.x
  pre-releases; whether a local MCP adapter is wanted after P12 (CLI and skill stay
  primary).

## Tracked issues

- #180 (job record and checkpoint fuzz targets, landed with PR 1): close it. #144
  (stabilised in P10 PR 3): close after a clean CI run on main.
- #170-#178: tracking issues for L-011, L-013, L-015, L-018, L-024, L-028, L-043,
  L-045 and L-042 (review pending).
- #159 motion fixtures (F04/F05/F12 E02); #150 noisy-speech fixtures before a noise
  WER gate; #147 faster-whisper adapter (backlog); #128 supervisor tests flaky on
  Windows under load.

## Other follow-ups

- **Known limits:** 60 entries to L-062 (review pending); PR 2 rewrote L-004, L-023,
  L-059 and added L-060 (no cross-process fairness), L-061 (free space), L-062 (links).
- **Campaign upkeep:** bump the three `UBUNTU_IMAGE_*` values together when Ubuntu
  retires release 20260911; raise `--max-ops` for layer C's job-free rounds.

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): `p07_transcript_e2e`,
  `p07_local_asr_e2e`, `p07_asr_qualification`, `p08_search_e2e`, `p08_candidates_e2e`,
  `p09_evidence_e2e`, `p10_recovery_e2e`, `engine_retranscribe`, `engine_jobs`,
  `p07_local_asr`, `p08_candidates_fixtures`, `p09_media_primitives`, `engine_evidence`,
  the S-11 measurements and the Windows console-interrupt test.
- Linux-only code (attestation reads, `fstatvfs`) is checked on Windows with
  `cargo clippy --target x86_64-unknown-linux-gnu` and an OpenSSL build-script override;
  it runs for real only on Linux CI and in PR 4's container.
- Never run the crash campaign's scripts on a machine whose disks matter. Durability
  ends at the disk (L-056, L-057); Windows/macOS durable requests fail closed.

## Guardrails

- R0 ships only when a coding agent goes from a local video to a grounded handoff through
  both the supplied-transcript and local-ASR paths, in named Codex and Claude Code trials.
- R1 packets P15..P20 start only after P14. Live capture (#107, #108) waits.
- New features land in the engine once, never in a host; the worker's public wire types
  are `WorkRequest` / `WorkResult`, never the application's `JobRequest`.
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas;
  a new parser of untrusted input needs a fuzz target with committed-seed provenance.
- Session commits go through `CommitHooks`/`Commit`; a change to the commit path reruns
  the crash campaign. Never enable `fault-injection` or `durability-campaign` in a release.
