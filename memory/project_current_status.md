# VSift current status

As of 2026-09-28. Current-state document: rewrite it, don't append to it. Next
actions and open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives AI coding agents local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is
also an embeddable engine library (`vsift`) that the CLI, and later other hosts, use.
Today it can:
- check and register its dependencies and show a read-only setup plan, and report
  whether local speech recognition really works here (`setup check` `local_asr`);
- copy a video into a private, disposable session;
- import an existing SRT or WebVTT transcript with the video, aligned by an offset;
- transcribe the video's speech itself with whisper.cpp (`transcript retranscribe`),
  continue an interrupted transcription where it stopped, and report, resume or cancel
  that work by its job id (`job status/resume/cancel`), with chunk progress;
- return timestamped transcript segments, search them, list the moments where the
  screen changed, and return exact frames, neighbours, bursts, crops and WAV clips;
- manage the session's lifetime and retention, and validate retained bundles;
- stop a long command cleanly on Ctrl-C or `SIGTERM`, never leaving a helper running;
- create a worker workspace with a fixed operator policy (`session init-workspace`);
  on Ubuntu 24.04 with local ext4 a durable workspace keeps every acknowledged result
  through an OS crash or power loss;
- run one versioned worker request in a workspace (`job run`): ingest from an operator
  input root, retranscribe, candidates, retain and close, each at most once per
  operation id, however often the request is delivered or its worker killed;
- limit the work it runs at once by what that work weighs, across processes;
- refuse to claim strict worker isolation unless the Linux kernel attests it;
- keep every folder it creates private to the user.

**P00-P10 are complete.** **P11 (worker and batch host) is in progress: PRs 1 and 2
are merged (`0bcfac5`, `6e89bdb`) and PR 3 is complete on its branch (`p11/job-run`),
not yet merged; the packet is not complete.** `job batch` still answers
`COMMAND_NOT_IMPLEMENTED`. Every known limit is in `docs/planning/known-limits.md`.

## P11 plan and decisions

[ADR 0021](../docs/decisions/0021-worker-and-batch-host.md) is accepted (maintainer,
2026-09-28) with decisions D1-D5: explicit `session init-workspace` with operator
policy (D1); worker sessions live a workspace-set time, default 168 h, at most 720 h
(D2); request steps ingest, retranscribe, candidates, retain, close (D3); the first
shutdown signal stops admitting and cancels at the next boundary, draining only with
`--drain-timeout-ms` up to 300 s (D4); `job batch` exits 0, 6 on shutdown, else the
worst failure class 7 > 1 > 5 > 3 > 2 > 4 (D5).

- **PR 1 (merged):** worker request/result, batch and workspace contracts, events.
- **PR 2 (merged):** workspace, weighted admission, strict attestation, contained
  inputs, free-space reserve.
- **PR 3 (done on its branch):** `job run` (below).
- **PR 4:** `job batch`, SEC-T01 container job, `p11_*` E2E checkpoint, operator
  runbook, qualification record.

## P11 PR 3: `job run` (branch)

- **Command.** `vsift --session-root <workspace> job run --request <file>
  --input-root <dir> [--bundle-root <dir>] [--drain-timeout-ms 0..300000]
  [--admission-wait-ms 0..60000] --json|--events jsonl`. Only in a worker workspace.
  The job result is the data of every outcome; a failed or cancelled request carries
  its error and exits with the failing step's class; every `BUSY` has a 2 s hint.
- **Engine.** `Engine::run_work_request` (`worker.rs`) and `worker_readiness`; the
  refusals that need no write run first (a recorded result is replayed before any
  input is opened). Steps map onto `ingest` (contained files through
  `ContainedSourceStore`, session id recorded before the copy, admission taken before
  registration), `retranscribe` (application `worker_step_operation_id`),
  `candidates` (at most 16 calls), a staged `retain` with its manifest digest, and
  `close`. `WorkerFailure` names the host's own failures.
- **Request records.** `worker-requests/<bucket>/<op>.json` with `<op>.lock`
  (infrastructure `worker_requests.rs`): staged, flushed, renamed, bucket-synced in a
  durable workspace; running records keep the finished steps' canonical documents,
  ended ones the result and its SHA-256 (contract `recorded_bytes`/`decode_recorded`,
  exact round trip). Domain `admit_request` (replay, conflict, busy, continue),
  `ends_request`, `step_retry` (X-09). At most 4,096 records; only those of gone
  sessions are pruned (L-063). An unrecorded result is `STORAGE_IO`.
- **Shutdown.** `signal::Shutdown`: first signal stops the next step, cancels the
  running one after the drain; second escalates; exit 6, resumable.
- **Evidence.** Engine, store and binary tests (replay/conflict/busy/continue, kill at
  each request fault point, X-09, O-01 sentinels, O-04 with `SIGTERM` and opt-in
  Ctrl-Break, contained inputs, schemas), opt-in real tools and external-delivery
  simulation, 300 of 300 stress runs, `request_record` fuzz target, crash campaign
  rerun (`docs/planning/verification.md` "P11 PR 3 evidence").

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`.
- `transcript retranscribe`, `transcript get`, `job status|resume|cancel|run`,
  `search`, `candidates`, `frame get/neighbours/burst`, `crop`, `audio`.
- `session list/status/renew/close/retain/clean/init-workspace` and `bundle validate`.
- Global `--session-root`, `--host-isolation`, `--json`, `--events jsonl`.
- Still `COMMAND_NOT_IMPLEMENTED`: `job batch` and setup
  install/repair/list/rollback/remove. Human-readable terminal output is P13's.

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | Complete (2026-09-25, `9ea3180`): engine, transcripts, local ASR, fuzzing |
| P08 | Complete (2026-09-26, `b830fc9`): search, candidates, source binding |
| P09 | Complete (2026-09-27, `e57c706`): frames, neighbours, bursts, crops, audio, reuse, lineage |
| P10 | Complete (2026-09-28, `3f27ce3`): jobs, resume, cancellation, durable Ubuntu/ext4 |
| P11 | In progress: PRs 1-2 merged, PR 3 complete on its branch; PR 4 to come |
| P12, P14 | Not started |
| P13 | Not started; also delivers managed installation and human-readable output |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and
owns the wire types. New in PR 3: domain `worker.rs`; application
`worker_step_operation_id`; contract `work/recorded.rs`, worker remediations and
`LifecycleResponse::of_session`; infrastructure `worker_requests.rs`,
`contained_source_store.rs`, `request_timing.rs`; engine `worker.rs`; CLI `worker.rs`
and `signal::Shutdown`. Storage: `filesystem_session_store/` (jobs in `jobs.rs`,
requests in `worker_requests.rs`).

## Quality evidence

- P11 PR 3 branch: fmt, strict Clippy (with and without features, on Windows and for
  `x86_64-unknown-linux-gnu`), workspace tests, warning-denied rustdoc, governance,
  fuzz fmt/Clippy/replay on Windows 11; results go in the PR description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
