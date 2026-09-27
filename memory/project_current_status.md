# VSift current status

As of 2026-09-27. Current-state document: rewrite it, don't append to it. Next
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
  that work by its job id (`job status/resume/cancel`);
- return timestamped transcript segments for a time range, as a page or a JSON Lines
  stream of keyed evidence records;
- search the transcript for words (`search`);
- list the moments where the screen changed (`candidates`);
- return exact frames, neighbours, bursts, native-size crops and WAV clips with
  requested and actual time, lineage and reuse (P09);
- manage the session's lifetime and retention, and validate retained bundles;
- stop a long command cleanly on Ctrl-C or `SIGTERM`, never leaving a helper process
  running;
- keep every folder it creates private to the user.

**P00-P09 are complete. P10 (recovery integration) is in progress:** of its four pull
requests, PR 1 (the commit path, `e2b14d9`) and PR 2 (jobs, keys and checkpointed
retranscription, `2ae55be`) are merged, and PR 3 (the public job surface and
interruption handling) is complete on branch `p10/job-surface`, not yet merged. PR 4
(the crash campaign) has not started. ADR 0020 is accepted (2026-09-27, D-1..D-5 as
recommended). Every known limit is in `docs/planning/known-limits.md`.

## P10 PR 3: job surface and interruptions (on its branch)

- **What a user sees:** `vsift job status <job>` (state, `live_owner`, `resumable` and
  why not, the operation id to retry with, requested range, progress in chunks,
  attempts, committed revision and generation, last failure code); `vsift job resume
  <job>` (the job afterwards and the retranscription); `vsift job cancel <job>`
  (interrupted: cancelled now and checkpoints removed; running: asked, the owner stops
  whisper within 250 ms; committing or succeeded: too late, warning
  `cancellation_too_late`; repeats change nothing). No session argument: the root's job
  index finds the job. `session status` adds `jobs` (16 newest) and `jobs_truncated`.
  `transcript retranscribe --operation-id op_...` (D-1): same id and request replays
  without a new generation (even without tools); another request is
  `IDEMPOTENCY_CONFLICT`. `job run/batch` stay `COMMAND_NOT_IMPLEMENTED` (P11).
- **Interruptions:** the first SIGINT/SIGTERM (Unix) or Ctrl-C/Ctrl-Break (Windows) of a
  long command (ingest, retranscribe, candidates, frame, crop, audio, job resume)
  cancels its one `Cancellation`; the second escalates (providers killed without the
  5 s graceful wait); the process exits only after its providers are reaped (SEC-04). A
  retranscription answers `CANCELLED` (exit 6) with `[session, job]` in `affected_ids`
  and a remediation command `vsift job resume <job>`; ingest stops its copy (64 KiB
  blocks) and removes the partial file; candidates and evidence commit what they
  finished. ADR 0017 decision 4 is superseded.
- **Engine and storage:** `EngineError::JobInterrupted` and `JobSessionNotOpen`,
  `affected_session()`; `JobStatusReport` gained resumability, planned chunks,
  committed generation, operation id and last failure; `Engine::session_jobs`
  (read-only, `observed_state`); `Engine::job_resume` returns `JobResumeReport` and
  refuses closed/expired sessions and any live owner; `Cancellation::escalate`;
  `IngestRequest.cancellation`; `EnginePorts::with_session_root_wait` (#144).
  `FilesystemSessionStore::watch_for_job_cancel` and `recorded_job_state`; job records
  gained optional `planned_chunks`. The supervisor reports a provider that failed after
  cancellation as cancelled (a console interrupt reaches every process on the console),
  and a retranscription records such a chunk failure as the cancellation.
- **Dependency:** Tokio's `signal` feature; `Cargo.lock` unchanged; review in ADR 0020.
- **Tests:** `job_cli_contract` (grammar, unknown job, succeeded/late cancel, live owner
  admitting and running = BUSY with retry hint, committing = too late, interrupted
  cancel removes checkpoints, closed session, IDEMPOTENCY_CONFLICT via the flag);
  `interrupt_cli_contract` (Unix SIGINT/SIGTERM on an ingest copy in CI; Windows
  Ctrl-Break opt-in through `tools/send-console-ctrl.ps1`); the watcher, escalation,
  termination-rule, staging-cancel and error-mapping unit tests; the job examples in
  `local_asr_contract`; opt-in `p10_recovery_e2e` (the recoverable mechanical run).
- **Windows finding:** a process that inherited "ignore Ctrl-C" (as this agent host's
  processes do) never sees Ctrl-C, only Ctrl-Break (L-053).

## P10 still to do

- **PR 4:** Ubuntu 24.04 / ext4 campaign (dm-log-writes >= 2,000 points, >= 300 QEMU
  kills, dm-flakey EIO, negative control), X-10, then flip `QUALIFIED_UBUNTU_EXT4`.

## P10 PRs 1-2 (merged)

- PR 1 (`e2b14d9`): store split into `filesystem_session_store/`; #164 resolved (reads
  stop at the writer's `chain-verified.json`); durable protocol implemented and disabled
  (`durable_profile`, `QUALIFIED_UBUNTU_EXT4 = false`); eleven commit fault points.
- PR 2 (`2ae55be`): `JobState` `Interrupted`/`Committing`, retry policy and poison rule;
  keys (request digest, recognition key, `opk_sha256_` operation key, `job_` id);
  checkpoints in `sessions/<ses>/jobs/<job>/chunks/`; exactly-once commit reconciled
  from the chain; `IDEMPOTENCY_CONFLICT` (exit 2, D-4); caps 512 / 384 / 128 KiB (D-2);
  readers retry a file being replaced (0.5 s) instead of reporting damage.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`: disposable
  session (24 idle hours, at most 7 days); supplied SRT/WebVTT import never needs whisper.
- `transcript retranscribe <session> [--from --to] [--operation-id]` (resumable),
  `transcript get ... [--events jsonl]`.
- `job status|resume|cancel <job>`.
- `search <session> --query <text> [--from --to] [--limit] [--cursor] [--revision]`.
- `candidates <session> --from <us> --to <us> [--limit 1..100] [--cursor]`.
- `frame get/neighbours/burst`, `crop`, `audio` (P09).
- `session list/status/renew/close/retain/clean` and `bundle validate` (records too).
- Still `COMMAND_NOT_IMPLEMENTED`: `job run/batch` and setup
  install/repair/list/rollback/remove. Human-readable terminal output is P13's.

## Earlier packets and the engine

- P07/P08: 30 s ASR chunks with 5 s overlap (`base` default, 3.25% clean WER), search
  tiers, 0.5 s visual sampling with a candidate per 10 s cell (`p08-candidate-recall.md`).
- P09: evidence navigation qualified in `docs/planning/p09-evidence-navigation.md`.
- **`crates/vsift`** is the engine (one typed `EngineError`, `failure_code()` the single
  public mapping, API 0.x); **`vsift-contract`** holds every v1 wire type; the CLI is thin.

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | Complete (2026-09-25, `9ea3180`): engine, transcripts, local ASR, fuzzing |
| P08 | Complete (2026-09-26, `b830fc9`): search, candidates, source binding |
| P09 | Complete (2026-09-27, `e57c706`): frames, neighbours, bursts, crops, audio, reuse, lineage |
| P10 | In progress (ledger `in_progress`): PRs 1-2 merged, PR 3 complete on its branch; PR 4 to do |
| P11, P12, P14 | Not started |
| P13 | Not started; also delivers managed installation and human-readable output |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine.
Storage: `filesystem_session_store/` (commit path in `commit.rs`/`publication.rs`, chain
in `chain.rs`, jobs in `jobs.rs`/`job_records.rs`), `durable_profile.rs`,
`fault_point.rs`, `retry_timer.rs`. Jobs: `vsift-application/src/job.rs` and
`job/run.rs`, `vsift/src/jobs.rs`; CLI `job.rs` and `signal.rs`.

## Quality evidence

- P10 PR 3 branch, Windows 11: fmt, strict Clippy (with and without features, and
  cross-target for Linux), workspace tests, warning-denied rustdoc, governance, fuzz
  fmt/Clippy/replay; opt-in `p10_recovery_e2e` and the Windows console-interrupt test
  with FFmpeg 9.0 and whisper.cpp v1.9.2. Results go in the PR description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
