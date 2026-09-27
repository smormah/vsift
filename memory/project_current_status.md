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
  and, since P10 PR 2, continue an interrupted transcription where it stopped;
- return timestamped transcript segments for a time range, as a page or a JSON Lines
  stream of keyed evidence records;
- search the transcript for words (`search`);
- list the moments where the screen changed (`candidates`);
- return exact frames, neighbours, bursts, native-size crops and WAV clips with
  requested and actual time, lineage and reuse (P09);
- manage the session's lifetime and retention, and validate retained bundles;
- keep every folder it creates private to the user.

**P00-P09 are complete. P10 (recovery integration) is in progress:** of its four pull
requests, PR 1 (the commit path) is merged (`e2b14d9`) and PR 2 (jobs, keys and
checkpointed retranscription) is complete on branch `p10/jobs-checkpoints`, not yet
merged; PRs 3 and 4 have not started. ADR 0020 is accepted (2026-09-27, D-1..D-5 as
recommended). Every known limit is in `docs/planning/known-limits.md`.

## P10 PR 2: jobs and checkpointed retranscription (on its branch)

- **What a user sees:** running `transcript retranscribe` again after an interruption
  continues from the chunks already recognised (private checkpoints in the session)
  and commits exactly the revision an uninterrupted run gives. The result gains
  `data.job` (`job_id`, `resumed`, `chunks_reused`, `replayed`) and the envelope's
  `operation_id`; warnings `resumed_from_checkpoint`, `checkpoint_discarded`. A job
  running elsewhere is `BUSY` with the job in `affected_ids` and `retry_after_ms`
  2000. A renewal or another revision during the run is followed (retry against the
  new head, or re-splice when the widened range is unchanged; else `BUSY` as
  superseded). Ctrl-C is still not trapped (PR 3).
- **Engine:** `RetranscribeRequest.operation_id` (D-1; CLI flag in PR 3): same id and
  request replays the commit before any preflight or hash; same id, other request is
  `IDEMPOTENCY_CONFLICT` (exit 2, D-4). `Engine::job_status/job_resume/job_cancel` by
  job id through the root `job-index/`. New `EngineError` job variants with
  `affected_job()` and `retry_after_ms()`.
- **Domain:** `JobState` adds `Committing` and `Interrupted` (success only through
  committing; cancel during committing is too late; committing -> interrupted only
  when the chain shows no commit); `JobKind`; `RetryPolicy` (BUSY twice, full jitter
  200 ms / 2 s, deadline skip), `poisoned_chunk`; `ChunkCheckpoint`, `RecognitionKey`.
- **Application:** keys (request digest, recognition key without the base revision,
  `opk_sha256_` operation key, `job_` id, per-attempt commit operation id);
  `transcribe_range_checkpointed` (raw output stored after validation, reused only
  through the same validation and merge); `run_retranscription` (the ADR 0020 retry
  table), `reconcile`, `cancel_job`, `job_status`, `resumable_request`.
- **Storage:** `sessions/<ses>/jobs/<job>/{job.json, owner.lock, state.lock,
  chunks/<n>.json}`, `jobs/by-operation/<op>.json`, `job-index/<bucket>/<job>.json`;
  strict versioned records, epoch/attempt fence, owner lock as the only liveness
  authority; bounds 64 jobs, 16 attempts, 8 caller ids, 256 bindings, 1,024
  checkpoints of 256 KiB. Nine job fault points (`FaultPoint::JOB`). An unreferenced
  manifest above the head is replaced by the next publication (L-048 resolved).
- **Caps (D-2):** 512 artifacts, 384 evidence, 128 KiB manifests; full-budget warm
  reuse p95 164-177 ms from 2 to 1,024 generations (slope 0.000).
- **Tests:** X-01 kills at every job point and property resume; X-02 replay; X-03
  conflicts and 2/4/8 identical concurrent requests; X-04 2/4/8 retranscriptions with
  renewals (found and fixed a head/generation race); X-05 suspended owner; X-09 retry
  tables; S-08 records and checkpoints; opt-in real FFmpeg and whisper.cpp resumes.
- **Not compiled on Windows:** the Unix-only `a_sigstopped_owner_keeps_its_job`
  (opt-in) and the durable variants of the job paths (directory syncs).

## P10 still to do

- **PR 3:** `job status/resume/cancel` commands and cancellation events, the
  `--operation-id` flag, Ctrl-C/SIGTERM via Tokio `signal` (supersedes ADR 0017
  decision 4).
- **PR 4:** Ubuntu 24.04 / ext4 campaign (dm-log-writes >= 2,000 points, >= 300 QEMU
  kills, dm-flakey EIO, negative control), X-10, then flip `QUALIFIED_UBUNTU_EXT4`.

## P10 PR 1 (merged `e2b14d9`)

Store split into `filesystem_session_store/`; #164 resolved (reads stop at the writer's
`chain-verified.json`; warm reuse flat to 1,024 generations); durable publication
protocol implemented and disabled (`durable_profile`, `QUALIFIED_UBUNTU_EXT4 = false`);
eleven commit fault points with the S-07 kill test.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`: disposable
  session (24 idle hours, at most 7 days); supplied SRT/WebVTT import never needs whisper.
- `transcript retranscribe <session> [--from --to]` (resumable), `transcript get ...
  [--events jsonl]`.
- `search <session> --query <text> [--from --to] [--limit] [--cursor] [--revision]`.
- `candidates <session> --from <us> --to <us> [--limit 1..100] [--cursor]`.
- `frame get/neighbours/burst`, `crop`, `audio` (P09).
- `session list/status/renew/close/retain/clean` and `bundle validate` (records too).
- Still `COMMAND_NOT_IMPLEMENTED`: job and setup
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
| P10 | In progress (ledger `in_progress`): PR 1 merged, PR 2 complete on its branch; PRs 3-4 to do |
| P11, P12, P14 | Not started |
| P13 | Not started; also delivers managed installation and human-readable output |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present). `vsift-contract` sits beside the engine. Storage:
`filesystem_session_store/` (commit path in `commit.rs`/`publication.rs`, chain in
`chain.rs`, jobs in `jobs.rs`/`job_records.rs`), `durable_profile.rs`,
`fault_point.rs`, `retry_timer.rs`. Jobs: `vsift-application/src/job.rs` and
`job/run.rs`, `vsift/src/jobs.rs`.

## Quality evidence

- P10 PR 2 branch, Windows 11: fmt, strict Clippy (with and without features),
  workspace tests, warning-denied rustdoc, governance, fuzz fmt/Clippy/replay; opt-in
  `engine_retranscribe`, `engine_jobs` (FFmpeg 9.0, whisper.cpp v1.9.2) and the S-11
  measurements. Results go in the PR description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
