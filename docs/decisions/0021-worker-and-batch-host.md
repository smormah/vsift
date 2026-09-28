# ADR 0021: Worker and batch host

- Status: Accepted (maintainer, 2026-09-28). Decisions D1-D5 below are confirmed as
  recommended.
- Date: 2026-09-28
- Tracking: [P11 / issue #14](https://github.com/smormah/vsift/issues/14),
  [#180](https://github.com/smormah/vsift/issues/180) (fuzz targets for the job
  record and chunk checkpoint)
- Refines: [ADR 0004](0004-recoverable-worker-core.md) (the worker core),
  [ADR 0006](0006-workspace-publication-and-durability.md) (durable workspaces),
  [ADR 0008](0008-cli-and-json-contract.md) (the reserved `job run` / `job batch` and
  the JSONL event mode), [ADR 0016](0016-embeddable-engine-and-evidence-contract.md)
  (decision 6, fuzz targets), [ADR 0017](0017-local-asr-through-whisper-cpp.md)
  (decision D7 of its section 6, "no progress events", superseded here) and
  [ADR 0020](0020-recoverable-jobs-and-durable-publication.md) (D-3, durable mode
  through the CLI)
- Scope of this record: the design of the whole P11 packet. PR 1 implements the
  contracts, events, progress and fuzz targets (sections 1 and 7); PR 2 the durable
  workspace, weighted admission, strict-Linux attestation and contained inputs
  (sections 3, 5a, 8 and 9); PR 3 `job run`, request records and graceful shutdown
  (sections 2, 4 and 6); PR 4 `job batch`, SEC-T01 and the qualification record
  (sections 5 and 10).

## Context

P10 made one long operation recoverable. A supervisor that runs `VSift` as a worker
needs more: one versioned request that names everything the work needs and nothing a
request must not decide (executables, environment, paths outside an operator's
roots, policy), one bounded result it can store and act on, a finite batch that never
needs the queue in memory, admission that respects the machine, a shutdown that stops
admitting and leaves work resumable, and events that let it tell busy from unhealthy
from missing capability (O-03) without reading free text. Architecture and contracts
sections 7, 8 and 10 set the rules; P11 turns them into a contract.

## Decision

### 1. Request and result (PR 1)

A **job request** (`job-request.schema.json`; Rust `vsift_contract::WorkRequest`,
decoded by `decode_work_request`) is strict and bounded: at most 64 KiB and 16 levels
of nesting, checked on the raw bytes; every member is required except `deadline_ms`;
unknown members are refused, and a newer `schema_version` is `UNSUPPORTED_SCHEMA`
even when it carries members this build does not know. It holds:

- `operation_id` (the `op_` grammar): the caller's idempotency key;
- `durability`: `durable` or `ephemeral`;
- `deadline_ms`: 1 to 86,400,000, or absent/`null` for the host's limit;
- `target`: `{"ingest": {"source": <path>, "transcript": {"path", "offset_us"} | null}}`
  or `{"session_id": "ses_..."}`;
- `steps`: at most 8 of `retranscribe {range | null}`, `candidates {range | null}`,
  `retain {bundle_name, include_source}` and `close {}`, run in order after the target.
  `retain` comes at most once and only `close` may follow it (so the bundle holds
  everything the request produced); `close` comes at most once and last; a session
  target needs a step.

Paths are relative to the operator's `--input-root`: at most 1,024 bytes and 32
components separated by `/`, and no empty, `.` or `..` component, `\`, `:`, control
character, character Windows forbids, name ending in a dot or a space, or Windows
device name (`CON`, `NUL`, `COM1`, `COM¹`, ...). The grammar is narrower than any one
platform's so one request means the same file everywhere and no spelling climbs out of
the root, picks an alternate data stream or aliases another name. Bundle names are
`[a-z0-9][a-z0-9_-]{0,63}`, one directory below `--bundle-root`.

The **request digest** is SHA-256 over `vsift.job-request.v1`, a line feed and the
canonical serialization of the decoded request (fixed member order, no whitespace,
every optional member written), without its operation id: the id is the key the digest
is bound to, and the same id with another digest is `IDEMPOTENCY_CONFLICT` (ADR 0020
D-4). Whitespace, member order and an omitted or `null` deadline do not change it.

A refusal is a typed `RequestRejection` with a fixed identifier, a failure code
(`UNSUPPORTED_SCHEMA` for a newer major, `INVALID_ARGUMENT` otherwise) and fixed-prose
remediation; it never echoes the input. Three rejections are decided by the host
against its workspace and input root (PRs 2-4): `path_outside_input_root`,
`duplicate_operation_id` (another line of the same batch) and `workspace_not_durable`.

A **job result** (`job-result.schema.json`; `WorkResult`) is at most 64 KiB, names no
path and carries no evidence text: `operation_id`, `request_digest`, a derived
`status` (`cancelled` when the request or a step was cancelled, `failed` when it or a
step failed, `partial` when a step left a stated gap, else `complete`), `replayed`,
`attempt`, `session_id`, `source_id`, `publication`, `lifecycle`, `steps` (the ingest
of an ingest target, then every requested step: `kind`, `status` including
`not_started`, `elapsed_ms`, `admission_wait_ms`, `job_id`, typed `outputs`,
`coverage`, `failure`), the request's `failure` (its code, retryability, hint, the
index of the step that ended it or the rejection that refused it) and `controls`
(`isolation`, `admission_capacity`, `concurrency`). After a failed or cancelled step
the rest are `not_started`; the contract refuses to present anything else.

`job-batch-data` (`JobBatchData`) summarises a batch: per-status counts, one item per
line processed (`line`, `operation_id`, `status` including `rejected`, `code`,
`rejection`), `not_started_from_line` and `termination_reason` (`end_of_input`,
`shutdown`, `line_limit`, `input_error`). Its outcome rule is D5. `workspace-data`
(`WorkspaceData`) is the result of `session init-workspace` (D1).

### 2. Step mapping and idempotency (PR 3)

Each step is one engine operation, idempotent by construction:

- **ingest:** the session id is allocated and recorded in the request record before
  the copy starts, so a retry after a crash finds the same session instead of copying
  twice; a supplied transcript is imported as `ingest --transcript` does.
- **retranscribe:** runs `Engine::retranscribe` under the derived operation id
  `op_` + 32 hex digits of SHA-256(`vsift.job-step.v1`, the request's operation id,
  the step index), so a retried request continues or replays the P10 job exactly once.
- **candidates:** calls `Engine::candidates` repeatedly until the range has no
  `not_analyzed` gap left (each call analyses at most 30 windows); what remains
  (`undecodable`, `no_decoded_frame`, a candidate budget) is reported as coverage and
  makes the step `partial`.
- **retain:** retains into `--bundle-root/<bundle_name>`; a retry that finds the bundle
  validates it and compares its manifest digest (`bundle_sha256`) with the recorded
  one, so a replaced or damaged bundle is `INTEGRITY_FAILURE`, never silently reused.
- **close:** closing a closed session is success.

### 3. Durable workspace (PR 2; D1, D2, ADR 0020 D-3)

A worker workspace is a session root created explicitly by `session init-workspace`
with an operator policy recorded in its marker: `profile: durable_workspace`,
`durability`, `admission_capacity` and `session_retention_seconds`. The policy is
immutable; a second initialisation with the same policy answers `already_initialized`,
another policy is refused. `durability: durable` is accepted only where OS-crash
durability is qualified (Ubuntu 24.04 / local ext4), so its sessions report
`os_crash_durable`. Its sessions live the workspace's retention (D2) and can be renewed
within it. `ingest --session-root <workspace>` opens a session with the workspace's
durability: that is the CLI's durable mode (ADR 0020 D-3).

### 4. Request records (PR 3)

Each request is recorded in `worker-requests/<bucket>/<op>.json` (the bucket is the
first two hex digits of the operation id's digest), with an `owner.lock` whose OS lock
is the liveness authority as for P10 jobs. A request with a known operation id is
answered from its record:

| Record | Same digest | Other digest |
| --- | --- | --- |
| ended (complete, partial, failed) | replay the recorded result (`replayed: true`) | `IDEMPOTENCY_CONFLICT` |
| live owner | `BUSY` with a retry hint | `IDEMPOTENCY_CONFLICT` |
| interrupted (owner gone) | continue from the first unfinished step | `IDEMPOTENCY_CONFLICT` |

A cancelled request is continued like an interrupted one. A workspace keeps at most
4,096 request records; the oldest ended ones are pruned first, and records expire with
the session they name. A `request_record` fuzz target lands with the record.

### 5. Finite streaming batch (PR 4)

`job batch --requests <file>` reads at most 1,000 lines of at most 64 KiB each, one at
a time: it never holds the file in memory. It admits at most `--concurrency` requests
at once (backpressure: the next line is read only when a slot frees) and isolates every
line: a malformed, refused or failed line is reported and the next one runs. Blank
lines are skipped but counted. Line 1,001 ends the batch with `line_limit`.

### 5a. Weighted admission (PR 2)

Admission weighs work by what it occupies: whisper.cpp takes its recognizer threads,
capped at min(available parallelism, 8, capacity); a visual `FFmpeg` pass 2; evidence
extraction and copying 1. A job command waiting for capacity waits with a bounded,
jittered backoff for at most 60 s (`AdmissionWait`) and then answers `BUSY`.
`--concurrency` is at most min(16, capacity). Requests of one batch are admitted in
line order (FIFO); there is no fairness between processes sharing a workspace, which
the external supervisor owns.

### 6. Graceful shutdown (PR 3; D4)

The first `SIGINT`/`SIGTERM` (or console Ctrl-C/Ctrl-Break) stops admission and
cancels in-flight work at its next boundary; every started request stays resumable
(its record is interrupted and its P10 job keeps its checkpoints). With
`--drain-timeout-ms` (at most 300,000) in-flight requests may instead finish until the
deadline, then are cancelled. A second signal escalates as in P10. The stream ends with
`lifecycle stopped`, then the terminal event; a stopped batch exits 6.

### 7. Events (PR 1)

Three event kinds join `evidence` and `terminal` (`EventKind::ALL`, with the ordinal
guard and a schema guard test):

- `progress` (`progress-event.schema.json`): `request_operation_id`, `job_id`, `stage`
  (`copying_source`, `recognising_speech`, `analysing_video`, `running_request`),
  `completed`, `total`, `unit` (`bytes`, `chunks`, `windows`, `steps`, fixed per stage)
  and `progress_dropped`. Advisory and bounded: at most one per second and 4,096 per
  request; an update inside the second replaces the held one, written with the next
  update or before the terminal event; when the reader is slow, progress is dropped
  and counted rather than blocking the work.
- `lifecycle` (`lifecycle-event.schema.json`): `started` (with `readiness`:
  publication, isolation, admission capacity, concurrency), `request_admitted`,
  `admission_waiting`, `request_finished` (status, code, rejection, dropped progress),
  `draining` and `stopped` (with a `reason`).
- `result` (`result-event.schema.json`): one request's `job-result`, with its `line`.

Every string member is an enum or a bounded pattern; no event carries a path, text or
provider output. Every line but the terminal one is at most 64 KiB; lifecycle, result
and terminal events are never dropped. A v1 reader skips unknown kinds but counts
their `sequence`, and the terminal event's `sequence` is the number of events before
it. `transcript retranscribe` and `job resume` emit chunk progress in `--events jsonl`
since PR 1; the evidence streams are unchanged.

### 8. Strict Linux attestation (PR 2)

`--isolation strict-linux` is accepted only when the host attests, read from the
kernel, that the process runs in a cgroup v2 with CPU, memory and PID limits, on a
read-only root filesystem, with no network interface but loopback; otherwise the
command answers `ISOLATION_UNAVAILABLE` before any work (P02's rule). The result's
`controls.isolation` states what was in force.

### 9. Contained inputs (PR 2)

`--input-root` is canonicalised once and opened as a `cap_std` directory; request
paths are opened through it, never joined as strings, and a link out of it is
`path_outside_input_root`. `--bundle-root` is used for output only.

### 10. Qualification (PR 4)

X-07..X-11, O-01..O-04 and SEC-T01 (an isolated hostile native fixture attempting
filesystem, network and credential access, fork pressure and output flooding) are
recorded in a P11 qualification record with the repeated external-delivery simulation.

## Decisions for maintainer confirmation

The five decisions were proposed with this record for the maintainer to confirm
before merge.

- **D1** Workspaces are created explicitly by `session init-workspace` with operator
  policy. *In plain English: nothing becomes a durable worker workspace by accident.*
- **D2** Durable-workspace sessions live a finite, workspace-set time (default 168 h,
  max 720 h, renewable within it). *Worker sessions are kept longer than desktop ones,
  but never forever.*
- **D3** v1 request steps are `ingest` (with supplied-transcript import),
  `retranscribe`, `candidates`, `retain` and `close`; not search, frame, crop or
  audio. *A request prepares and hands over evidence; interactive reading stays with
  the agent's own commands.*
- **D4** The first shutdown signal stops admitting and cancels in-flight work at its
  next boundary; draining is opt-in via `--drain-timeout-ms` (at most 300 s). *Stop
  quickly by default and keep the work resumable; wait only when asked.*
- **D5** `job batch` exits 0 when every request is complete or partial, 6 when a
  shutdown stopped the batch, else the most severe failure class in the order
  7 > 1 > 5 > 3 > 2 > 4. *One exit code a supervisor can act on, worst problem first.*

Confirmed: D1-D5 accepted by the maintainer on 2026-09-28.

## Implementation notes: PR 1 (2026-09-28)

PR 1 implements sections 1 and 7, the progress half of L-025 and the fuzz targets.
`job run` and `job batch` still answer `COMMAND_NOT_IMPLEMENTED`.

- **Contract.** `vsift-contract` gains `request` (`WorkRequest`, `decode_work_request`,
  `decode_batch_line`, `validate_steps`, `RelativeInputPath`, `BundleName`,
  `RequestRejection`), `work` (`WorkResult` from typed `WorkResultParts` and
  `StepResult`s), `batch` (`JobBatchData` and its D5 `outcome`), `workspace`
  (`WorkspaceData`) and `events` (`ProgressEventResponse`, `LifecycleEventResponse`,
  `ResultEventResponse`). The strict bounded decoder P01 froze in the CLI moved here as
  `decode_strict_json` with `JsonLimits`; the CLI keeps only the bounded file read for
  the saved setup plan. `sha2` (already a workspace dependency of the application) is
  now a dependency of the contract, for the request digest. `LifecycleResponse` gained
  `durable_worker` and `TerminalEventResponse::at_sequence` is public.
- **D5 detail.** A request cancelled without a shutdown (by `job cancel`) ranks between
  usage (2) and retryable (4); an input error counts as `STORAGE_IO` and a line limit
  as `RESOURCE_LIMIT`.
- **Schemas.** `job-request`, `job-result`, `job-batch-data`, `workspace-data`,
  `progress-event`, `lifecycle-event` and `result-event`, with frozen examples
  (`job-request.json`, `job-batch.requests.jsonl`, `job-run.json`,
  `job-run.replayed.json`, `job-run.partial.json`, `job-batch.json`,
  `job-batch.events.jsonl`, `workspace-init.json`,
  `transcript-retranscribe.events.jsonl`); `ingest-data.publication` admits
  `os_crash_durable` (a durable engine ingest already reported it since P10 PR 4).
- **Progress.** The application's `ProgressSink` port rides in `CheckpointScope` and
  `RetranscriptionPorts`: a checkpointed recognition reports 0 of its planned chunks
  once planned, then every chunk, reused or fresh. The engine's `ProgressObserver`
  (on `RetranscribeRequest` and `JobResumeRequest`) names the job; a replay reports
  nothing. The CLI serves the command's future and a bounded queue (16) on one task and
  writes through the new incremental `JsonLinesWriter` (each line flushed; lines but
  the terminal at most 64 KiB); `ProgressGate` applies the one-per-second and 4,096
  bounds. A failure after progress is the terminal event at the count of events before
  it.
- **Fuzzing (ADR 0016 decision 6).** New targets `job_request`, `job_batch_line`,
  `job_record` and `chunk_checkpoint` (the last two close the scope of #180). The job
  record and checkpoint codecs are public in `vsift-infrastructure`
  (`decode_job_record`, `encode_job_record`, `decode_chunk_checkpoint`,
  `encode_chunk_checkpoint`) for their targets; their seeds copy example records under
  `crates/vsift-infrastructure/tests/data/jobs/`, which `job_record_examples` pins to
  the encoder. `request_record` follows with PR 3.

## Consequences

- The worker request and result are public v1 contracts before any command uses them,
  so PR 2-4 build on a reviewed, fuzzed, schema-checked surface.
- `--events jsonl` of `transcript retranscribe` and `job resume` gains progress lines
  before the terminal event; readers that follow the v1 rule (skip unknown kinds,
  count their sequence) are unaffected. A terminal failure after progress no longer has
  `sequence` 0.
- A request can never name an executable, environment, shell text, callback URL,
  absolute path or policy override; operators own roots, isolation, capacity and
  retention.
- `--json` of `job batch` returns the summary only; per-request results are in the
  `result` events of `--events jsonl`, or replayed by operation id with `job run`.
- No fairness between processes sharing a workspace; an external supervisor owns
  scheduling across workers (architecture and contracts section 10).

## Alternatives

- **Reuse the application's `JobRequest` name on the wire.** It already names the P10
  job's request (`JobRequest::Retranscribe`); the public types are `WorkRequest` and
  `WorkResult`, with the schemas named `job-request` and `job-result`.
- **Accept a JSON request on stdin.** A file is bounded and re-readable for replay; a
  pipe is neither.
- **Treat any step list as a DAG.** A fixed order with `retain` and `close` last is
  enough for R0 and keeps idempotency per step simple.
- **Progress with a timer thread.** A held update is written with the next one or
  before the terminal event instead; recognition chunks take seconds, so the
  difference is small and there is no thread to stop.
