# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P10 are complete. P11 (worker and batch host) is in progress: PRs 1 and 2 are
merged (`0bcfac5`, `6e89bdb`); PR 3 is complete on its branch `p11/job-run`; the
packet is not.** ADR 0021 is accepted with maintainer decisions D1-D5 (2026-09-28).

1. **PR 3 (`job run`, request records, shutdown): done on its branch, awaiting review
   and merge.** `job run --request --input-root [--bundle-root] [--drain-timeout-ms]
   [--admission-wait-ms]` in a worker workspace; request records in
   `worker-requests/` (replay, `IDEMPOTENCY_CONFLICT`, `BUSY`, continuation);
   step idempotency (session id recorded before the copy, derived operation id for
   the recognition, candidates until nothing is unanalysed, staged retain with its
   manifest digest, close); X-09 retries and deadline; the two-stage shutdown (D4);
   `lifecycle`/`progress`/`result` events; fault points `request-accept`,
   `request-step`, `request-complete`; fuzz target `request_record`; the opt-in
   external-delivery simulation; the crash campaign rerun with requests (see
   `docs/planning/p10-durable-publication.md`, P11 section).
2. **Next: PR 4** - `job batch` (streaming reader, backpressure, concurrency, D5
   exit), SEC-T01 (the strict attestation in a container against a hostile fixture),
   the `p11_*` single-host worker checkpoint of the E2E spine, the operator runbook
   (packet "P11 operator deliverables") and the P11 qualification record.

## Decided (maintainer, 2026-09-28)

- **D1** Workspaces are created explicitly by `session init-workspace` with operator
  policy. **D2** Workspace sessions live a finite, workspace-set time (default 168 h,
  max 720 h). **D3** Request steps: ingest (with transcript import), retranscribe,
  candidates, retain, close. **D4** The first shutdown signal stops admitting and
  cancels in-flight work at its next boundary; drain is opt-in via
  `--drain-timeout-ms` (at most 300 s). **D5** `job batch` exits 0 when every request
  is complete or partial, 6 on shutdown, else the worst class 7 > 1 > 5 > 3 > 2 > 4.

## Open decisions (maintainer)

- **PR 3 readings to confirm:** a request failed with a transient or resumable code
  (`BUSY`, `DEADLINE_EXCEEDED`, `STORAGE_IO`, `CANCELLED`) stays continuable rather
  than ended; pruning removes only records whose session is gone (not "oldest ended
  first"); the record bound is 192 KiB (a 64 KiB result escaped as JSON text); an
  existing bundle directory that is not the request's is `INVALID_ARGUMENT`; the
  job run shutdown reason is carried by `lifecycle stopped` (no terminal member);
  `--admission-wait-ms` defaults to 60 s; an unrecorded result answers `STORAGE_IO`.
- **From PR 2:** the D2 reading (retention after opening or renewal, never beyond
  720 h); ephemeral workspace sessions report `durable_worker`; every link on an input
  path is refused (L-062).
- The D5 detail in ADR 0021's PR 1 notes; MSRV and 0.x pre-releases; a local MCP
  adapter after P12 (CLI and skill stay primary).

## Tracked issues

- #180 (job record and checkpoint fuzz targets): close it. #144: close after a clean
  CI run on main.
- #170-#178: tracking issues for L-011, L-013, L-015, L-018, L-024, L-028, L-043,
  L-045 and L-042 (review pending).
- #159 motion fixtures; #150 noisy-speech fixtures; #147 faster-whisper adapter; #128
  supervisor tests flaky on Windows under load.

## Other follow-ups

- **Known limits:** 63 entries to L-065 (review pending); PR 3 rewrote L-010, L-038
  (now `job batch` only) and L-059, and added L-063 (request record cap), L-064 (retain
  staging directories) and L-065 (per-delivery deadlines).
- **Campaign upkeep:** bump the three `UBUNTU_IMAGE_*` values together when Ubuntu
  retires release 20260911; the data images are 3 GiB since PR 3 (the workspace's
  free-space reserve).

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): the P07-P10 E2E tests,
  `engine_retranscribe`, `engine_jobs`, the evidence and candidates fixtures, the S-11
  measurements, the Windows console-interrupt tests, `every_step_runs_with_real_tools`,
  `provider_output_never_reaches_output` and `repeated_external_delivery_commits_once`.
- Linux-only code is checked on Windows with `cargo clippy --target
  x86_64-unknown-linux-gnu` and an OpenSSL links override (the `openssl` cfg and
  `ossl*` cfgs); it runs for real only on Linux CI and in PR 4's container.
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
