# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P10 are complete. P11 (worker and batch host) is in progress: PRs 1-3 are
merged (`0bcfac5`, `6e89bdb`, `d64dfa1`); PR 4 is done only in part on its branch
`p11/job-batch`; the packet is not complete.** ADR 0021 is accepted with maintainer
decisions D1-D5 (2026-09-28).

1. **PR 4, first part (`job batch`): done on its branch, awaiting review.** Engine
   `run_work_batch` (reader counted then streamed, one Tokio task per request,
   bounded event channel), CLI `job batch` with events, summary and the D5 exit,
   batch shutdown; engine, binary and opt-in real-tool tests (X-08, X-11, O-02,
   O-03, O-04, shared workspace, kill mid-batch). See ADR 0021 "PR 4, first part".
2. **Still to do in PR 4 (L-038):**
   - SEC-T01: the strict profile in the hardened CI container against a hostile
     provider fixture. Stopped: needs a maintainer decision on how that fixture is
     built and reviewed before any work resumes.
   - The `p11_*` single-host checkpoint of the E2E spine
     (`crates/vsift-cli/tests/p11_worker_e2e.rs`).
   - The operator runbook `docs/operations/worker-host.md` (packet "P11 operator
     deliverables") and the qualification record `docs/planning/p11-worker-host.md`.
   - A fuzz target for the batch file's line reader (`BatchLines`, generic over
     `Read + Seek` for this), with committed seeds.
   - Final rows for X-08, X-11, O-02..O-04 and SEC-T01 in `verification.md`,
     `e2e-test-spine.md`, the support profiles' worker-strict column, the threat
     model's SEC-T01/SEC-19/SEC-25 status, architecture-and-contracts and README.

## Decided (maintainer, 2026-09-28)

- **D1** Workspaces are created explicitly by `session init-workspace` with operator
  policy. **D2** Workspace sessions live a finite, workspace-set time (default 168 h,
  max 720 h). **D3** Request steps: ingest (with transcript import), retranscribe,
  candidates, retain, close. **D4** The first shutdown signal stops admitting and
  cancels in-flight work at its next boundary; drain is opt-in via
  `--drain-timeout-ms` (at most 300 s). **D5** `job batch` exits 0 when every request
  is complete or partial, 6 on shutdown, else the worst class 7 > 1 > 5 > 3 > 2 > 4.

## Open decisions (maintainer)

- **PR 4 readings to confirm:** a file of more than 1,000 lines is refused whole
  before any work (the ADR said line 1,001 ends the batch); a line over 64 KiB is
  refused alone rather than failing the batch; a line cancelled by `job cancel` that
  is the most severe exits 6 like a shutdown, told apart by `termination_reason`
  (L-067); the engine depends on `tokio` directly.
- **SEC-T01 approach:** how the hostile provider fixture is built, reviewed and run.
- **PR 3 readings to confirm:** transient or resumable failures stay continuable;
  pruning removes only records of gone sessions; the 192 KiB record bound; a foreign
  bundle directory is `INVALID_ARGUMENT`; `lifecycle stopped` carries the job run
  shutdown reason; `--admission-wait-ms` defaults to 60 s; an unrecorded result is
  `STORAGE_IO`.
- **From PR 2:** the D2 reading; ephemeral workspace sessions report
  `durable_worker`; every link on an input path is refused (L-062). Also: MSRV and
  0.x pre-releases; a local MCP adapter after P12.

## Tracked issues

- #180 (job record/checkpoint fuzz targets): close it. #144: close after a clean main.
- #170-#178: tracking issues for L-011, L-013, L-015, L-018, L-024, L-028, L-043,
  L-045 and L-042 (review pending).
- #159 motion fixtures; #150 noisy-speech fixtures; #147 faster-whisper adapter; #128
  supervisor tests flaky on Windows under load.

## Other follow-ups

- **Known limits:** 65 entries to L-067 (review pending); PR 4 rewrote L-038 (what
  remains of P11) and added L-066 (1,000-line batch file) and L-067 (contention
  inside a batch; exit of a job-cancelled line).
- **Campaign upkeep:** bump the three `UBUNTU_IMAGE_*` values together (3 GiB images).

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): the P07-P10 E2E tests,
  `engine_retranscribe`, `engine_jobs`, `engine_batch_tools`, the evidence and
  candidates fixtures, the S-11 measurements, the Windows console-interrupt tests,
  `every_step_runs_with_real_tools`, `provider_output_never_reaches_output` and
  `repeated_external_delivery_commits_once`.
- Linux-only code is checked on Windows with `cargo clippy --target
  x86_64-unknown-linux-gnu` and an OpenSSL links override (the `openssl` cfg and
  `ossl*` cfgs up to `ossl340`); it runs for real only on Linux CI.
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
- Session commits go through `CommitHooks`/`Commit`; a change to the commit path (or to
  request-record writes) reruns the crash campaign. Never enable `fault-injection` or
  `durability-campaign` in a release.
