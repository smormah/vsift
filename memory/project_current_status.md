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
  that work by its job id (`job status/resume/cancel`), with chunk progress in
  `--events jsonl`;
- return timestamped transcript segments, search them, list the moments where the
  screen changed, and return exact frames, neighbours, bursts, crops and WAV clips;
- manage the session's lifetime and retention, and validate retained bundles;
- stop a long command cleanly on Ctrl-C or `SIGTERM`, never leaving a helper running;
- create a worker workspace with a fixed operator policy (`session init-workspace`);
  on Ubuntu 24.04 with local ext4 a durable workspace keeps every acknowledged result
  of its sessions through an OS crash or power loss, now reachable from the CLI;
- limit the work it runs at once by what that work weighs, across processes;
- refuse to claim strict worker isolation unless the Linux kernel attests it;
- keep every folder it creates private to the user.

**P00-P10 are complete.** **P11 (worker and batch host) is in progress: PR 1 is merged
(`0bcfac5`) and PR 2 is complete on its branch (`p11/workspace-admission`), not yet
merged; the packet is not complete.** A supervisor cannot run `job run` or `job
batch` yet: they still answer `COMMAND_NOT_IMPLEMENTED`. Every known limit is in
`docs/planning/known-limits.md`.

## P11 plan and decisions

[ADR 0021](../docs/decisions/0021-worker-and-batch-host.md) is accepted (maintainer,
2026-09-28) with decisions D1-D5: explicit `session init-workspace` with operator
policy (D1); worker sessions live a workspace-set time, default 168 h, at most 720 h
(D2); request steps ingest, retranscribe, candidates, retain, close (D3); the first
shutdown signal stops admitting and cancels at the next boundary, draining only with
`--drain-timeout-ms` up to 300 s (D4); `job batch` exits 0, 6 on shutdown, else the
worst failure class 7 > 1 > 5 > 3 > 2 > 4 (D5).

- **PR 1 (merged, `0bcfac5`):** worker request/result, batch and workspace contracts,
  `progress`/`lifecycle`/`result` events, chunk progress, fuzz targets.
- **PR 2 (done on its branch):** workspace, admission, attestation, contained inputs
  (below).
- **PR 3 (next):** `job run`, request records and replay, step idempotency, graceful
  shutdown, `--input-root`/`--bundle-root`, `request_record` fuzz target.
- **PR 4:** `job batch`, SEC-T01 container job, external-delivery simulation,
  qualification record.

## P11 PR 2: workspace, admission, attestation, contained inputs (branch)

- **Workspace.** `vsift --session-root <abs> session init-workspace --durability
  durable|ephemeral --admission-slots 1..64 [--retention-hours 1..720]`
  (`session.init-workspace`, `workspace-data`). The policy is recorded in the root's
  ownership marker (`workspace` member) and is immutable: the same policy again is
  `already_initialized`, another policy or a desktop root `INVALID_ARGUMENT`, a marker
  changed underneath `INTEGRITY_FAILURE`. Durable only where `durable_profile`
  qualifies (checked on the parent and the new root), else `MISSING_CAPABILITY` and
  nothing created. Domain `WorkspacePolicy`, `WorkspaceRetention`,
  `SessionLifetimePolicy`; engine `Engine::init_workspace`.
- **Workspace sessions.** Expire the retention after opening or renewal, never beyond
  720 h from opening (recorded in the lifecycle as `workspace_retention_seconds`);
  report `lifecycle.mode` `durable_worker`; `ingest --session-root <workspace>`
  inherits the durability (ADR 0020 D-3; `os_crash_durable` in a durable workspace).
- **Admission (X-07).** Weights: visual window 2, copy/probe/evidence 1, recognition
  its threads = min(parallelism, 8, capacity) (in provenance; L-023). The recognition
  permit covers its chunk decoding (`FfmpegMedia::within_caller_admission`). Heavier
  than the root: `RESOURCE_LIMIT` before work. `AdmissionWait` (domain policy,
  application loop in the retranscription job): `Immediate` for the CLI, `Bounded`
  (<= 60 s, full jitter) for job hosts, ending `BUSY` with `retry_after_ms` 2000;
  `JobSummary::admission_wait`, `ProgressObserver::with_admission_waiting`.
  `Cancellation::child` for per-request signals.
- **Attestation.** `attest_strict_linux_host` (infrastructure `host_attestation.rs`):
  bounded reads of `/proc/self/cgroup`, `cpu.max`/`memory.max`/`pids.max` on every
  cgroup level, the root mount (`classify_root_mount`) and `/proc/self/net/dev`; pure
  `decide_strict_linux`. Engine `attest_host_isolation`; CLI global
  `--host-isolation process-only|strict-linux`, `ISOLATION_UNAVAILABLE` (exit 2)
  before any work.
- **Contained inputs.** `InputRoot::open_file`: the relative grammar, then one
  component at a time with no link followed (any link refused, L-062) and a single
  hard link; `SourceSnapshot::stage_contained`, `read_supplied_transcript_contained`.
- **Free space and controls.** `ensure_free_space` (Unix `fstatvfs`, 1 GiB reserve;
  Windows not enforced, L-061); `IngestOutcome::free_space`; job-result
  `controls.resource_limits` and `controls.free_space_reserve`.
- **Fuzzing.** `host_attestation` (21 targets); `mountinfo` checks the root mount too.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`: disposable
  session (24 idle hours, at most 7 days; a workspace's own retention in a workspace).
- `transcript retranscribe <session> [--from --to] [--operation-id]` (resumable; chunk
  progress with `--events jsonl`), `transcript get ... [--events jsonl]`.
- `job status|resume|cancel <job>`; `search`, `candidates`, `frame
  get/neighbours/burst`, `crop`, `audio`.
- `session list/status/renew/close/retain/clean/init-workspace` and `bundle validate`.
- Global `--session-root`, `--host-isolation`, `--json`, `--events jsonl`.
- Still `COMMAND_NOT_IMPLEMENTED`: `job run/batch` and setup
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
| P11 | In progress: PR 1 merged, PR 2 complete on its branch; PRs 3-4 to come |
| P12, P14 | Not started |
| P13 | Not started; also delivers managed installation and human-readable output |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and
owns the wire types (worker request/result, batch, workspace, events). New in PR 2:
domain `admission.rs` and the lifetime/workspace policy in `session.rs`; application
`admit_waiting` in `job/run.rs`; infrastructure `host_attestation.rs`, `input_root.rs`,
the workspace marker in `filesystem_session_store/root.rs`; engine `workspace.rs`,
`isolation.rs`. Storage: `filesystem_session_store/` (jobs in `jobs.rs`).

## Quality evidence

- P11 PR 2 branch: fmt, strict Clippy (with and without features, and for
  `x86_64-unknown-linux-gnu` from Windows), workspace tests, warning-denied rustdoc,
  governance, fuzz fmt/Clippy/replay on Windows 11; the X-07 cross-process test ran
  100 of 100 in four lanes. Results go in the PR description; evidence rows in
  `docs/planning/verification.md`.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
