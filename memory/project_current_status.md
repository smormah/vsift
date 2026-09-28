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
- run as a worker under an external supervisor: create a worker workspace with a fixed
  operator policy (`session init-workspace`), run one versioned request (`job run`) or
  a file of up to 1,000 (`job batch`), each step at most once per operation id however
  often it is delivered or its worker killed, a bounded number at a time, weighted by
  what the work occupies, with a shutdown that leaves every started request resumable;
- on Ubuntu 24.04 with local ext4, keep every acknowledged result of a durable
  workspace through an OS crash or power loss;
- refuse to claim strict worker isolation unless the Linux kernel attests it;
- keep every folder it creates private to the user.

**P00-P11 are complete.** P11 (worker and batch host) closed on 2026-09-28 with merge
`40c4038` (PRs #184-#187, #189; ADR 0021 accepted with D1-D5). SEC-T01 is met for P11
by non-adversarial evidence (maintainer decision, 2026-09-28): the real strict-Linux
attestation passes inside the hardened CI container. Its adversarial containment
evidence is technical debt required before the R0 release (#188, L-068,
`docs/planning/sec-t01-adversarial-handoff.md`). P12 (agent skill) is next.

## P11 in one view

[ADR 0021](../docs/decisions/0021-worker-and-batch-host.md) is accepted with maintainer
decisions D1-D5 (workspace by explicit init; workspace-set retention; steps ingest,
retranscribe, candidates, retain, close; stop-then-cancel shutdown with opt-in drain;
the D5 batch exit).

- **PR 1 (merged, `0bcfac5`):** request/result, batch and workspace contracts, events,
  progress, fuzz targets.
- **PR 2 (merged, `6e89bdb`):** workspace, weighted admission, strict attestation,
  contained inputs, free-space reserve.
- **PR 3 (merged, `d64dfa1`):** `job run`, request records, two-stage shutdown, crash
  campaign rerun with requests (run 36379513017).
- **PR 4a (merged, `45c25d1`):** `job batch` (engine `batch.rs`, reader
  `batch_file.rs`, CLI `batch.rs`).
- **PR 4b (branch, local):** fuzz target `job_batch_file` (23 targets); the opt-in
  single-host checkpoint `crates/vsift-cli/tests/p11_worker_e2e.rs`; the operator
  runbook `docs/operations/worker-host.md`; the qualification record
  `docs/planning/p11-worker-host.md`; SEC-T01 status, known limits L-068/L-069 and the
  final docs. No production code changed.

## The single-host checkpoint (`p11_*`)

Opt-in, real FFmpeg 9.0 and whisper.cpp v1.9.2 (ggml base), Windows 11:
`p11_batch_mechanical` (a mixed F03/F10/F01 batch plus two refused lines, then
`search`, `candidates`, `frame get --candidate` and `bundle validate` on its outputs,
cited against the frozen truth), `p11_admission_ladder` (concurrency 1, 2, 4 in a
four-unit workspace; the sampled provider weight never above 4, two windows at most
with four requests in flight), `p11_shutdown_and_redelivery` (Ctrl-Break mid-batch,
exit 6 in about 1 s; `job resume` and redelivery; results equal to an uninterrupted
control; a third delivery replays unchanged) all passed; `p11_durable_workspace`
reports `blocked` off Ubuntu 24.04 / ext4 (the refusal is checked) and is required
only there. Timings are in the qualification record.

## Runbook decisions worth knowing

- Acknowledge a message only after its result is recorded; redeliver on `BUSY`,
  `CANCELLED`, `DEADLINE_EXCEEDED`, `STORAGE_IO` or no result; dead-letter permanent
  failures. Derive the operation id from the message's key; route redeliveries to the
  same workspace; one workspace per trust domain.
- The systemd example uses `KillMode=mixed` and `TimeoutStopSec` of at least the drain
  plus 10 s (for the maintainer to confirm, TODO). A host-caused permanent failure
  replays under its id; resubmit under a new one (L-069).

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
| P11 | Complete (2026-09-28, `40c4038`); SEC-T01 adversarial evidence is technical debt (#188, L-068) |
| P12, P14 | Not started |
| P13 | Not started; also delivers managed installation and human-readable output |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine and
owns the wire types. The worker lives in the engine (`worker.rs`, `batch.rs`); the CLI
only presents (`worker.rs`, `batch.rs`). Storage: `filesystem_session_store/` (jobs in
`jobs.rs`, requests in `worker_requests.rs`), batch reader `batch_file.rs`.

## Quality evidence

- PR 4b gates on Windows 11 (fmt, strict Clippy with and without features and for
  `x86_64-unknown-linux-gnu`, workspace tests, warning-denied rustdoc, governance,
  fuzz fmt/Clippy/replay) go in the pull request description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
