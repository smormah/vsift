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
*Amended 2026-09-28 by maintainer decision (PR 4 second-part notes): SEC-T01 is met for
P11 by non-adversarial evidence; the adversarial evidence is technical debt (L-068).*

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

## Implementation notes: PR 2 (2026-09-28)

PR 2 implements sections 3, 5a, 8 and 9 as groundwork for `job run` and
`job batch`, which still answer `COMMAND_NOT_IMPLEMENTED`.

- **Workspace (section 3, D1, D2).** `session init-workspace --durability
  durable|ephemeral --admission-slots N --retention-hours H` (`session.init-workspace`,
  `workspace-data`) needs an explicit absolute `--session-root` that is not the
  per-user cache and whose parent exists. The ownership marker gains an optional
  `workspace` member (`profile`, `durability`, `session_retention_seconds`; the
  capacity stays the marker's `admission_capacity`); a desktop root's marker is
  unchanged, and a build before P11 refuses a workspace marker (unknown member). The
  same policy again answers `already_initialized`; any other policy, or a desktop
  root, is `INVALID_ARGUMENT` with a fixed remediation (the mapping of
  `STATE_CONFLICT`: the request cannot succeed against the root as it stands). Every
  revalidation requires the marker to still hold the store's policy, so a policy
  changed underneath is `INTEGRITY_FAILURE`, never adopted. A durable policy is
  checked on the parent's filesystem before anything is created and on the new root
  before its marker is written: off Ubuntu 24.04 / ext4 it is `MISSING_CAPABILITY`
  with no directory left.
- **Retention (D2), as implemented.** A workspace session expires the workspace's
  retention after it opens or is renewed, and never beyond 720 hours from its opening
  (`SessionLifetimePolicy::Workspace`); the retention is recorded in the session's
  lifecycle (`workspace_retention_seconds`) so each session validates under its own
  rules. With the maximum retention a renewal therefore changes nothing. This reading
  of "renewable within it" is recorded for the maintainer's confirmation.
- **Lifecycle mode.** Every session of a workspace, ephemeral or durable, reports
  `lifecycle.mode` `durable_worker`: the mode names whose rules bound the session's
  life; how it publishes is `data.publication` (`process_crash_consistent` in an
  ephemeral workspace, `os_crash_durable` in a durable one).
- **CLI durable mode (ADR 0020 D-3).** `ingest --session-root <workspace>` opens a
  session with the workspace's durability: a durable workspace makes every session
  durable, and a durable engine request in an ephemeral workspace is refused
  (`WorkspaceNotDurable`, `INVALID_ARGUMENT`), never weakened.
- **Weighted admission (section 5a, X-07).** A visual window reserves 2 units, a copy,
  probe or evidence extraction 1, and a recognition its recognizer threads, now
  min(available parallelism, 8, the root's capacity); the capped count is in the run's
  provenance (known limit L-023). The recognition's reservation covers its chunk
  decoding (`FfmpegMedia::within_caller_admission`), because the decoding and the
  recognizer alternate and never run at once: a separate slot would count the job twice
  and could leave a one-unit root unable to recognise anything. Work heavier than the
  root's whole capacity fails `RESOURCE_LIMIT` before any tool is resolved or run.
  `AdmissionWait` (domain policy, application loop) is `Immediate` for interactive
  commands, which keep P10's two bounded retries, or `Bounded` (at most 60 s) with a
  full-jitter poll (50 ms doubling to 1 s, at least 10 ms, never past the budget) that
  reports its wait (`JobSummary::admission_wait`), tells the progress observer once
  (`admission_waiting`, for PR 3's lifecycle event) and ends in `BUSY` with
  `retry_after_ms` 2000, the job left resumable and not retried again. In PR 2 the
  bounded wait is wired into the recognition job; PR 3 applies it to its other steps.
  `Cancellation::child` gives each request a signal the process-wide one cancels but
  that cancels nothing else.
- **Strict Linux attestation (section 8).** The flag is the global
  `--host-isolation process-only|strict-linux` (the section's `--isolation`): with
  `strict-linux` the CLI attests the host before building its engine, and a host that
  does not attest answers `ISOLATION_UNAVAILABLE` (exit 2) before any work.
  `attest_strict_linux_host` reads, each bounded, `/proc/self/cgroup` (cgroup v2 only,
  at most 32 levels, no `..`), `cpu.max`, `memory.max` and `pids.max` of the cgroup and
  every ancestor (a limit on any level bounds the process), the root mount's own
  options (the fuzzed mountinfo parser) and `/proc/self/net/dev` (loopback only); a
  pure decision table names every gap. On an attested host the supervisor requires the
  strict boundary of every provider (`HostIsolation::minimum_requirement` is
  `StrictWorker`), so a provider never runs below it. The limits are reported, never
  set: job-result
  `controls.resource_limits` is `host_cgroup` or `not_enforced`. The real attestation
  runs in PR 4's SEC-T01 container job; here the parsers and the table are tested on
  fixture files everywhere and fuzzed (`host_attestation`, the 21st target).
- **Contained inputs (section 9).** `InputRoot` canonicalises the root once and holds
  it; a request path passes the relative grammar again, then is opened one component
  at a time following no link. The implementation is stricter than the section: a link
  anywhere on the path is refused, even one inside the root (known limit L-062), and
  the file must have one hard link. `SourceSnapshot::stage_contained` and
  `read_supplied_transcript_contained` read the opened file; the engine request path
  and the `--input-root` and `--bundle-root` flags come with `job run` in PR 3.
- **Free-space reserve.** Before a copy into a workspace on Unix, `fstatvfs` on the
  held root must show the source's size and 1 GiB free, else `RESOURCE_LIMIT`; Windows
  reports `not_enforced` (known limit L-061). Job-result `controls.free_space_reserve`
  says which.
- **Contract change.** `job-result.controls` gains the required `resource_limits` and
  `free_space_reserve`; no host emits a job result before PR 3, and the frozen examples
  are updated with it.

## Implementation notes: PR 3 (2026-09-28)

PR 3 implements sections 2, 4 and 6 for one request at a time: `job run`. `job batch`
still answers `COMMAND_NOT_IMPLEMENTED` (PR 4).

- **Command.** `vsift --session-root <workspace> job run --request <file> --input-root
  <dir> [--bundle-root <dir>] [--drain-timeout-ms 0..300000] [--admission-wait-ms
  0..60000] --json|--events jsonl` (defaults: drain 0, admission wait 60000; `0`
  reports contention at once). The file is read bounded (64 KiB + 1 byte) and decoded
  by `decode_work_request`; a refusal is its rejection's code and fixed remediation, and
  an unreadable file `INVALID_ARGUMENT`. Requests run only in a worker workspace (D1):
  a desktop root is `INVALID_ARGUMENT`. The `job-result` is the `data` of every
  outcome: `complete` and `partial` exit 0; `failed` and `cancelled` carry the error of
  the failure that ended the request (its typed remediation, the session in
  `affected_ids`, `retry_after_ms`) and exit with its class (D5: the failing step's).
  Every `BUSY` a request reports carries the 2 s hint.
- **Engine.** `Engine::run_work_request(WorkRequestRun) -> WorkOutcome` (`worker.rs`)
  never fails outright; `Engine::worker_readiness` gives the `started` event's
  readiness. The order is part of the contract: the workspace; a record with another
  digest (`IDEMPOTENCY_CONFLICT`) or with a result (replayed at once, before any input
  is opened, so a replay works after the inputs are gone); the durability
  (`workspace_not_durable`); a `retain` without an absolute existing bundle root; the
  source and sidecar opened inside the input root (a link is `path_outside_input_root`,
  a missing file `INVALID_ARGUMENT`, a hard-linked or special file `INVALID_SOURCE`);
  a session target that is not in the workspace; a shutdown already begun. Nothing is
  written before these pass.
- **Request records (section 4).** `worker-requests/<bucket>/<op>.json` (bucket: the
  first byte of SHA-256 of the operation id) with `<op>.lock` as the liveness authority,
  as for P10 jobs; a claimant that locked a lock file pruning removed notices by file
  identity and reports the request as held. A record holds the digest, the attempt,
  the timestamps, the session id and, while running, the canonical documents of the
  finished steps; once ended, only the canonical result and its SHA-256, checked on
  every read. Writes are staged, flushed and renamed; in a durable workspace the
  bucket is synchronised after the rename and every new directory into its parent.
  Readers use the retrying open. The step and result documents are the contract's
  (`StepResult::recorded_bytes` / `decode_recorded`, `WorkResult::…`): read back
  strictly, they must reproduce exactly the bytes they came from. The record bound is
  192 KiB, not 64: an ended record holds a result of up to 64 KiB escaped as JSON text
  (known limit L-063).
- **Ended, interrupted, busy.** A request has ended when it completed, completed with
  a stated gap, or failed with a code the retry policy classes as permanent; a failure
  classed transient or resumable (`BUSY`, `DEADLINE_EXCEEDED`, `STORAGE_IO`,
  `CANCELLED`) leaves it interrupted, like a cancellation, so a redelivery continues
  it (the domain's `admit_request` and `ends_request`). This extends section 4's table,
  which named only `failed`: replaying a `BUSY` for ever would defeat X-09. A request
  held by another process is `BUSY`; one held with another digest is a conflict even
  before it is recorded.
- **Pruning.** At 4,096 records the first record of a new operation id prunes the
  records of sessions that no longer exist (never a held one); with none to prune the
  request is `RESOURCE_LIMIT`. Section 4's "oldest ended first" is not implemented: an
  ended record whose session exists is kept, because its removal would let a
  redelivery run the work again (L-063).
- **Steps (section 2).** *Ingest:* the session id is recorded (the `request-accept`
  write) before the copy; the copy's admission is taken before the session is
  registered, so a busy root is retried without leaving a registration per try; the
  source and sidecar are opened again inside the input root for each try and staged
  through `ContainedSourceStore` (the application's open-session use case over a held,
  contained file). The public `IngestRequest` is unchanged: the engine's internal
  `PreparedIngest` carries the session id and the contained files. A continuation that
  finds the recorded session open adopts it (the ingest ended after activation but
  before its record said so); otherwise it records a new id and the abandoned
  registration is removed by normal cleanup, so one key opens one session.
  *Retranscribe:* `Engine::retranscribe` under `worker_step_operation_id` (application:
  `op_` and the first 32 hex digits of `sha256("vsift.job-step.v1\n" + op + "\n" +
  index)`), with the host's `AdmissionWait`; its own bounded admission wait is not
  retried again. *Candidates:* `Engine::candidates` until no `not_analyzed` or
  `deadline_exceeded` window is left, a call analyses nothing, or 16 calls
  (`MAX_CANDIDATE_CALLS`); a whole-source range is clipped by the operation; a stop
  mid-way commits what was analysed and cancels the step. *Retain:* into a hidden
  staging directory beside the bundle's name, renamed once it validates, with the
  manifest's SHA-256 as `bundle_sha256` (`BundleSummary::manifest_sha256`); a directory
  already under the name is accepted only when it validates as this session's bundle
  (same session, source, source inclusion and artifact count; the bundle records no
  generation), else `INVALID_ARGUMENT` (L-064). A retain already recorded is not
  validated again on a continuation (only `close` can follow it). *Close:* a closed
  session is success.
- **Retries and deadline (X-09).** `step_retry` (domain): only `BUSY` is retried, after
  the host's full-jitter `AdmissionWait` delays, within that wait and never leaving
  less than one second of the deadline (`MIN_STEP_BUDGET`); a step never starts with
  less. At the deadline the running step's signal is cancelled
  (`run_until_deadline`) and awaited, never dropped, so providers are reaped; the
  request ends `DEADLINE_EXCEEDED`, resumable. Deadlines and waits count per delivery
  (L-065).
- **Shutdown (section 6, D4).** `job run` registers a two-stage listener before any
  work: the first `SIGINT`/`SIGTERM` (Ctrl-C/Ctrl-Break) fires the run's `stop` (no
  further step starts) and, after `--drain-timeout-ms`, its cancellation (the running
  step stops at its next boundary: a recognition is left interrupted, candidates keep
  what they committed, a commit in progress completes); a second signal escalates. The
  request ends `cancelled` (resumable, exit 6); the stream says `draining` and
  `stopped` with reason `shutdown`. No member was added to the terminal event: the
  `stopped` lifecycle event carries the reason.
- **Events.** `--events jsonl`: `lifecycle started` (readiness), `request_admitted`,
  `progress` (`running_request` steps; a recognition's `recognising_speech` chunks, each
  with `request_operation_id`), `admission_waiting`, `draining`, the `result` event,
  `request_finished`, `stopped` (`end_of_input` or `shutdown`), then the terminal event
  carrying the `--json` response.
- **Unrecorded results.** When an ended request's result cannot be recorded
  (`WorkOutcome::unrecorded`) the work is committed and every step recorded, but the
  result is not acknowledged: `job run` answers `STORAGE_IO` (exit 7) with the result as
  data, and a redelivery records and returns it.
- **Result.** A request names its session only once the session exists; `publication`
  is the workspace's; `controls.free_space_reserve` is `enforced` for an ingest target
  on Unix, else `not_enforced`; a replay is the recorded result with `replayed: true`
  and nothing else changed. The contract formats the expiry itself
  (`LifecycleResponse::of_session`).
- **Fault points and fuzzing.** `request-accept`, `request-step` and `request-complete`
  (`FaultPoint::REQUEST`) pass after each record write; the engine's kill test stops a
  child at each and checks the rerun. The `request_record` target (22 targets) decodes a
  record, round-trips it and requires every recorded step and result to read back to
  exactly its bytes; its seeds are the example records pinned by
  `request_record_examples`.
- **Crash campaign.** The workload creates its root as a durable workspace, runs worker
  requests among its operations and acknowledges a request only once its result is
  recorded, with the operation id and result digest in the `ACK` line; the verifier
  holds each acknowledged record to its result and reads every record on disk. The data
  images grew to 3 GiB for the workspace's free-space reserve. Results are in
  [p10-durable-publication.md](../planning/p10-durable-publication.md) (P11 rerun).

## Implementation notes: PR 4, first part (2026-09-28)

PR 4 implements section 5 (`job batch`) with its tests. SEC-T01, the `p11_*` E2E
checkpoint, the operator runbook and the qualification record (section 10) are not
in this part (known limit L-038).

- **Command.** `vsift --session-root <workspace> job batch --requests <file>
  --input-root <dir> [--bundle-root <dir>] [--concurrency 1..16] [--admission-wait-ms
  0..60000] [--drain-timeout-ms 0..300000] --json|--events jsonl` (defaults:
  concurrency 1, admission wait 60000, drain 0). `--concurrency` above the workspace's
  capacity is `INVALID_ARGUMENT` before any work (`WorkerFailure::
  ConcurrencyExceedsCapacity`, `Engine::batch_readiness`).
- **Engine, not host.** The batch runs in the engine (`Engine::run_work_batch`,
  `batch.rs`), so every later host gets the same reader, isolation and summary; the CLI
  only presents. The engine reports `BatchEvent::Admitted` and `Finished` through a
  bounded Tokio channel the host supplies, sent with `send().await`, and builds each
  request's progress observer through a host factory (`BatchProgress`), where the CLI
  installs one `ProgressGate` per request.
- **Runtime check.** The future of `Engine::run_work_request` is `Send`, so each
  request runs as its own Tokio task (`JoinSet`) over `Arc<Engine>`; no thread per
  request is needed. The engine crate now depends on `tokio` directly (already a
  dependency of every layer below it).
- **Reader (infrastructure `batch_file.rs`).** The file is opened once and must be a
  regular file; its lines are counted through the same handle with a fixed buffer, the
  handle is rewound, and lines are then read one at a time, each held to 64 KiB plus
  one byte. **Refinement of section 5:** a file of more than 1,000 lines is refused
  whole before any work (`line_limit`, `not_started_from_line` 1, `RESOURCE_LIMIT`),
  rather than running its first 1,000 lines; a file that grows past the limit after
  the count still stops at line 1,001. A line over 64 KiB is refused alone
  (`request_too_large`), as section 5's isolation says; the whole batch is not failed
  for it (a deviation from the PR 4 brief, which suggested failing the batch). A file
  that cannot be opened or read is `input_error` (`STORAGE_IO`, exit 7).
- **Backpressure (X-08).** The next line is read only when fewer than `--concurrency`
  requests run; a shutdown while every slot is busy stops the reading at once. A host
  that stops reading events holds the batch back: the event channel fills and the
  batch stops reading. The CLI writes on the command's own task, so a paused stdout
  reader pauses the batch; progress is dropped and counted, never queued without bound.
- **Isolation.** Each line is decoded by `decode_batch_line`; a refused line (not
  decodable, over 64 KiB, or reusing an operation id an earlier line of the batch
  used: `duplicate_operation_id`) has no result: only its `request_finished`
  (`rejected`, its code and rejection) and its summary item. Every admitted request
  gets its own child cancellation, deadline, record and result. Blank lines are skipped
  but counted. Items are in the order lines ended.
- **Shutdown (D4).** The batch's `stop` stops the reading and each running request
  before its next step; `--drain-timeout-ms` later its cancellation stops their running
  steps. The stream says `draining` (`shutdown`), and `draining` (`drain_timeout`) when
  requests still run at the end of the drain. The summary is `shutdown` when the signal
  stopped the reading or a running request, with the first line not started; D5 makes
  that exit 6 (`CANCELLED`, fixed remediation). A stream that cannot be written stops
  the reading too (exit 7).
- **Outcome.** `complete` when every item completed, `partial` (with a batch warning)
  when some completed with a gap, else D5's failure with fixed remediation: the file's
  for `input_error`, the line limit's for `line_limit`, otherwise "at least one line
  failed". A request cancelled by `job cancel` that is the most severe line exits with
  its class's status, 6, like a shutdown; the termination reason tells them apart
  (L-067, for the maintainer to confirm).
- **Contention.** The requests of one batch share the workspace's admission and
  root-level locks, so with `--admission-wait-ms 0` one may answer `BUSY` because a
  sibling holds a unit for a moment (L-067); the default wait retries with jitter.

## Implementation notes: PR 4, second part (2026-09-28)

The second part of PR 4 completes section 10 and the packet's operator deliverables.

- **SEC-T01 (section 10), maintainer decision of 2026-09-28 (option 2).** SEC-T01 is
  satisfied for P11 by non-adversarial evidence: the strict-Linux attestation checks
  (section 8; the parsers and decision table on fixture files, the `host_attestation`
  and `mountinfo` fuzz targets, `ISOLATION_UNAVAILABLE` before any work off an attested
  host) and the controls the existing hardened `strict-worker-boundary` CI container job
  verifies. The adversarial evidence section 10 describes is technical debt, deferred
  for maintainer discussion and required before the R0 release (known limit L-068);
  this record's section 10 is amended accordingly, and nothing else in it changes.
  The PR 2 note that "the real attestation runs in PR 4's SEC-T01 container job" no
  longer holds: no container job was added, so `--host-isolation strict-linux`
  succeeding end to end on a real strict host is not yet exercised in CI (a residual
  of the qualification record; the attestation's positive path is covered on fixture
  files).
- **Fuzzing.** Target `job_batch_file` (23 targets): a whole batch file through the
  reader (`BatchLines`) under the production limits and under small ones, held to an
  independent split of the file at its line feeds, each line then decoded as `job
  batch` decodes it; seeds copy the frozen batch examples.
- **Single-host checkpoint.** `crates/vsift-cli/tests/p11_worker_e2e.rs` (opt-in, the
  P11 stage of the E2E spine): a mixed batch whose outputs are then searched, cited and
  validated against the frozen truth; the admission ladder at concurrency 1, 2 and 4,
  with the providers' sampled weight never above the capacity; a batch stopped by
  `SIGTERM` or a console Ctrl-Break, finished by `job resume` and redelivery, equal to
  an uninterrupted control and then replayed unchanged; and the durable workspace,
  required only on the qualified profile. Results are in the
  [P11 qualification record](../planning/p11-worker-host.md).
- **Operator runbook.** [docs/operations/worker-host.md](../operations/worker-host.md)
  (the packet's operator deliverables): supervisor invocation, acknowledgement order,
  duplicates, restart, cleanup, disk pressure, provider revocation, an isolated
  container deployment with the CI job's controls, a systemd example and the guarantee
  matrix. **Refinement:** the systemd example uses `KillMode=mixed`, not
  `control-group`: `control-group` signals the providers at the same moment as VSift,
  so a provider can exit abnormally before VSift's cancellation reaches it and fail its
  step instead of leaving it resumable; `mixed` signals VSift alone and `SIGKILL`s the
  whole cgroup only after `TimeoutStopSec` (at least the drain time plus 10 s).
- **Reading recorded for the runbook (L-069).** A permanent failure that describes the
  host (`MISSING_CAPABILITY`, the free-space reserve's `RESOURCE_LIMIT`) ends the
  request like any permanent failure (PR 3 notes), so the same operation id replays it
  after the host is fixed; the runbook tells supervisors to resubmit under a new id.

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
