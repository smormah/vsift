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
- run one versioned worker request in a workspace (`job run`), each step at most once
  per operation id, however often the request is delivered or its worker killed;
- run a file of up to 1,000 such requests (`job batch`), a bounded number at a time,
  each line independently, reading the next line only when a request ends, with one
  summary and one exit status a supervisor can act on, and a shutdown that leaves
  every started request resumable;
- limit the work it runs at once by what that work weighs, across processes;
- refuse to claim strict worker isolation unless the Linux kernel attests it;
- keep every folder it creates private to the user.

**P00-P10 are complete.** **P11 (worker and batch host) is in progress: PRs 1-3 and
the first part of PR 4 (`job batch`, `45c25d1`) are merged; the rest of PR 4 is on
branch `p11/qualification`, done only in part (the batch-reader fuzz target). The
packet is not complete:** SEC-T01 (stopped pending a maintainer decision on its
hostile fixture), the `p11_*` E2E checkpoint, the operator runbook and the
qualification record remain (L-038). Every known limit is in
`docs/planning/known-limits.md`.

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
- **PR 3 (merged, `d64dfa1`):** `job run`, request records, two-stage shutdown.
- **PR 4, first part (merged, `45c25d1`):** `job batch` (below).
- **PR 4, rest (branch `p11/qualification`):** done: the `job_batch_file` fuzz
  target. Remaining: SEC-T01 container job (stopped pending a maintainer decision on
  its hostile fixture), `p11_*` E2E checkpoint, operator runbook, qualification
  record, final docs.

## P11 PR 4, first part: `job batch` (merged)

- **Command.** `vsift --session-root <workspace> job batch --requests <file>
  --input-root <dir> [--bundle-root <dir>] [--concurrency 1..16]
  [--admission-wait-ms 0..60000] [--drain-timeout-ms 0..300000] --json|--events jsonl`.
  Concurrency above the workspace's capacity is refused before any work.
- **Engine.** `Engine::run_work_batch` (`batch.rs`) and `Engine::batch_readiness`.
  Each request runs as its own Tokio task over `Arc<Engine>` (its future is `Send`);
  events go to the host through a bounded channel (`BatchEvent`), and each request's
  progress observer comes from a host factory (`BatchProgress`). The engine now depends
  on `tokio` directly.
- **Reader.** Infrastructure `batch_file.rs` (`BatchLines`, `BatchFile`): a regular
  file, counted through the same handle before any work (over 1,000 lines: refused
  whole, `line_limit`), then read one bounded line at a time, only when a slot frees.
  A line over 64 KiB is refused alone; so are malformed and duplicate-id lines.
- **CLI.** `crates/vsift-cli/src/batch.rs`: `started`, per line admitted / progress /
  result / finished in the order lines end, `draining` (`shutdown`, `drain_timeout`),
  `stopped`, terminal summary; D5 exit with fixed remediations. A paused stdout reader
  pauses the batch; progress is dropped and counted.
- **Evidence.** `verification.md` "P11 PR 4 evidence, first part": engine
  `engine_batch` (isolation, X-08 read-ahead, a host that stops reading, shutdown,
  two batches in one workspace, kill mid-batch at each request fault point), binary
  `job_batch_cli_contract` (events, limits, O-02 property test, O-03, X-08 paused
  stdout, O-04 by `SIGTERM` and opt-in Ctrl-Break), opt-in `engine_batch_tools`
  (X-11 with FFmpeg and a stand-in recognizer); 100 of 100 stress runs of
  `engine_batch` in four lanes on Windows 11.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`.
- `transcript retranscribe`, `transcript get`, `job status|resume|cancel|run|batch`,
  `search`, `candidates`, `frame get/neighbours/burst`, `crop`, `audio`.
- `session list/status/renew/close/retain/clean/init-workspace` and `bundle validate`.
- Global `--session-root`, `--host-isolation`, `--json`, `--events jsonl`.
- Still `COMMAND_NOT_IMPLEMENTED`: setup install/repair/list/rollback/remove.
  Human-readable terminal output is P13's.

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | Complete (2026-09-25, `9ea3180`): engine, transcripts, local ASR, fuzzing |
| P08 | Complete (2026-09-26, `b830fc9`): search, candidates, source binding |
| P09 | Complete (2026-09-27, `e57c706`): frames, neighbours, bursts, crops, audio, reuse, lineage |
| P10 | Complete (2026-09-28, `3f27ce3`): jobs, resume, cancellation, durable Ubuntu/ext4 |
| P11 | In progress: PRs 1-3 and PR 4's first part merged; SEC-T01, E2E, runbook, qualification remain |
| P12, P14 | Not started |
| P13 | Not started; also delivers managed installation and human-readable output |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and
owns the wire types. New in PR 4: engine `batch.rs`, infrastructure `batch_file.rs`,
CLI `batch.rs`; contract batch remediations and `JobBatchData` accessors. Storage:
`filesystem_session_store/` (jobs in `jobs.rs`, requests in `worker_requests.rs`).

## Quality evidence

- Branch `p11/qualification` (fuzz target): fuzz fmt, Clippy (all features) and replay
  (7 tests) green on Windows 11; the workspace gates run before review.
- P11 PR 4 first part (merged): fmt, strict Clippy (with and without features, on
  Windows and for `x86_64-unknown-linux-gnu`), 994 workspace tests, warning-denied
  rustdoc, governance, fuzz fmt/Clippy/replay on Windows 11; results go in the PR
  description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
