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
  that work by its job id (`job status/resume/cancel`); with `--events jsonl` a
  transcription now reports its progress chunk by chunk;
- return timestamped transcript segments for a time range, as a page or a JSON Lines
  stream of keyed evidence records;
- search the transcript for words (`search`);
- list the moments where the screen changed (`candidates`);
- return exact frames, neighbours, bursts, native-size crops and WAV clips with
  requested and actual time, lineage and reuse (P09);
- manage the session's lifetime and retention, and validate retained bundles;
- stop a long command cleanly on Ctrl-C or `SIGTERM`, never leaving a helper process
  running;
- for a host that embeds the engine on Ubuntu 24.04 with local ext4, keep a durable
  session's every acknowledged result through an OS crash or power loss;
- keep every folder it creates private to the user.

**P00-P10 are complete.** **P11 (worker and batch host) is in progress: PR 1 of four is
complete on its branch (`p11/contracts-events`), not yet merged; the packet is not
complete.** A supervisor cannot run `job run` or `job batch` yet: they still answer
`COMMAND_NOT_IMPLEMENTED`. Every known limit is in `docs/planning/known-limits.md`.

## P11 plan and decisions

[ADR 0021](../docs/decisions/0021-worker-and-batch-host.md) is accepted (maintainer,
2026-09-28) with decisions D1-D5 as recommended: explicit `session init-workspace`
with operator policy (D1); worker sessions live a workspace-set time, default 168 h,
at most 720 h (D2); request steps are ingest, retranscribe, candidates, retain and
close (D3); the first shutdown signal stops admitting and cancels at the next
boundary, draining only with `--drain-timeout-ms` up to 300 s (D4); `job batch` exits
0, 6 on shutdown, else the worst failure class 7 > 1 > 5 > 3 > 2 > 4 (D5).

- **PR 1 (done on its branch):** contracts, events, progress, fuzzing (below).
- **PR 2 (next):** durable workspace and CLI durable mode, weighted admission,
  strict-Linux attestation, contained input and bundle roots.
- **PR 3:** `job run`, request records and replay, step idempotency, graceful shutdown.
- **PR 4:** `job batch`, SEC-T01, external-delivery simulation, qualification record.

## P11 PR 1: contracts, events, progress and fuzzing (branch)

- **Request.** `vsift-contract::decode_work_request` / `decode_batch_line`: strict,
  at most 64 KiB and 16 levels, unknown members refused, a newer major
  `UNSUPPORTED_SCHEMA`. Operation id, durability, optional deadline, an ingest target
  (a path relative to the operator's input root, optional supplied transcript) or a
  session, and up to 8 steps with order rules (retain once and only close after it;
  close once and last). A strict relative-path grammar (no absolute, `..`, `\`, `:`,
  control characters, trailing dot or space, Windows device names). Canonical
  SHA-256 digest (`vsift.job-request.v1`, without the operation id). Typed
  `RequestRejection` with fixed remediation. The P01 strict decoder moved from the CLI
  into the contract (`decode_strict_json`).
- **Result.** `WorkResult` (`job-result`): derived status, replay flag, steps with
  typed outputs, coverage and failures, request failure, controls; at most 64 KiB, no
  path, no text. `JobBatchData` (`job-batch-data`) with the D5 outcome rule;
  `WorkspaceData` (`workspace-data`). Rust names avoid the application's `JobRequest`.
- **Events.** `EventKind` gains `progress`, `lifecycle`, `result` (schemas, ordinal
  and schema guards; only progress droppable). New incremental `JsonLinesWriter` in
  the CLI (each line flushed; non-terminal lines at most 64 KiB).
- **Progress.** Application port `ProgressSink` (in `CheckpointScope` and
  `RetranscriptionPorts`), engine `ProgressObserver` on `RetranscribeRequest` and
  `JobResumeRequest`, CLI `ProgressGate` (one per second, 4,096 per request, a 16-slot
  queue that drops and counts). Evidence streams unchanged. Supersedes ADR 0017 D7.
- **Fuzzing.** Targets `job_request`, `job_batch_line`, `job_record`,
  `chunk_checkpoint` (20 in all); the record codecs are public in infrastructure, and
  their seeds copy pinned example records in `tests/data/jobs/`.
- **Schemas and examples.** Seven new schemas, nine new examples;
  `ingest-data.publication` admits `os_crash_durable`.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`: disposable
  session (24 idle hours, at most 7 days); supplied SRT/WebVTT import never needs whisper.
- `transcript retranscribe <session> [--from --to] [--operation-id]` (resumable; chunk
  progress with `--events jsonl`), `transcript get ... [--events jsonl]`.
- `job status|resume|cancel <job>` (`resume` reports progress like retranscribe).
- `search`, `candidates`, `frame get/neighbours/burst`, `crop`, `audio`.
- `session list/status/renew/close/retain/clean` and `bundle validate`.
- Still `COMMAND_NOT_IMPLEMENTED`: `job run/batch` and setup
  install/repair/list/rollback/remove. Human-readable terminal output is P13's.

## P10 in brief (complete 2026-09-28, `3f27ce3`)

Recoverable retranscribe jobs with chunk checkpoints and exactly-once commit,
`IDEMPOTENCY_CONFLICT`, `job status/resume/cancel`, Ctrl-C/SIGTERM cancellation, and
durable publication qualified on Ubuntu 24.04 / ext4 by an owned crash campaign
(`docs/planning/p10-durable-publication.md`; ADR 0020). Durable sessions are
engine-only until P11 PR 2's workspace (L-059).

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | Complete (2026-09-25, `9ea3180`): engine, transcripts, local ASR, fuzzing |
| P08 | Complete (2026-09-26, `b830fc9`): search, candidates, source binding |
| P09 | Complete (2026-09-27, `e57c706`): frames, neighbours, bursts, crops, audio, reuse, lineage |
| P10 | Complete (2026-09-28, `3f27ce3`): jobs, resume, cancellation, durable Ubuntu/ext4 |
| P11 | In progress: PR 1 of 4 complete on its branch; PRs 2-4 to come |
| P12, P14 | Not started |
| P13 | Not started; also delivers managed installation and human-readable output |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and
now also owns the worker request/result, batch and workspace types and the event
kinds (`request`, `work`, `batch`, `workspace`, `events`, `input`). Progress: domain
`ProgressStage`/`ProgressUpdate`, application `ProgressSink`, engine
`ProgressObserver`, CLI `progress.rs` and `output::JsonLinesWriter`. Storage:
`filesystem_session_store/` (jobs in `jobs.rs`/`job_records.rs`).

## Quality evidence

- P11 PR 1 branch: fmt, strict Clippy (with and without features), workspace tests,
  warning-denied rustdoc, governance, and fuzz fmt/Clippy/replay on Windows 11. Results
  go in the PR description; evidence rows in `docs/planning/verification.md`.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
