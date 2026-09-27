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
- for a host that embeds the engine on Ubuntu 24.04 with local ext4, keep a durable
  session's every acknowledged result through an OS crash or power loss;
- keep every folder it creates private to the user.

**P00-P09 are complete. P10 (recovery integration) is implemented across all four of
its pull requests:** PR 1 (the commit path, `e2b14d9`), PR 2 (jobs, keys and
checkpointed retranscription, `2ae55be`) and PR 3 (the public job surface and
interruption handling, `8af331b`) are merged; **PR 4 (the Ubuntu 24.04 / ext4 crash
campaign and durable enablement) is complete on branch `p10/durability-campaign`**, not
yet reviewed or merged. The packet is complete when PR 4 merges and the ledger
completion record is written. Every known limit is in `docs/planning/known-limits.md`.

## P10 PR 4: crash campaign and durable enablement (on its branch)

- **What a host sees:** `IngestRequest::durability` (engine API only, ADR 0020 D-3).
  On Ubuntu 24.04 with the session root on ext4 mounts that keep write barriers a
  durable request is honoured and reported `os_crash_durable`; everywhere else it fails
  with `MISSING_CAPABILITY` before anything changes. The CLI still opens ephemeral
  sessions; an ingest now reports its session's own guarantee (it used to report the
  store's strongest).
- **Gate:** `QUALIFIED_UBUNTU_EXT4 = true`; `durable_profile` also reads `os-release`
  (64 KiB bound, strict `classify_os_release`, fuzz target `os_release`) and decides by
  the public `qualifies` table; anything unread or unparsed fails closed.
- **Campaign:** `tools/p10-crash-campaign/` (workspace tool `vsift-crash-campaign`:
  `workload`, `verify`, `replay`, `assess`; scripts; guest service) and
  `.github/workflows/p10-durability-campaign.yml` (manual, weekly; hosted
  `ubuntu-24.04` with KVM). Layer A: dm-log-writes replay at every flush and FUA write
  with verification, a write probe and `e2fsck -fn` at each point. Layer B: SIGKILL of
  the pinned Ubuntu 24.04 cloud image (release 20260911, SHA-256 pinned) with a
  `cache=none` data disk, four shards. Layer C: dm-flakey `error_writes` swapped in at
  random. The negative control (`durability-campaign` feature,
  `VSIFT_CAMPAIGN_NEGATIVE_CONTROL=1`) removes every sync after the pointer rename and
  must lose acknowledgements. Gating run 36340043451 (before the flip) and
  confirmation run 36347502530 (release build, flipped) each met every criterion:
  ~11,040 replay points, 320 kills, 144-180 injected failures, nothing lost; the
  control lost 54 of 80. Details: `docs/planning/p10-durable-publication.md`.
- **Findings fixed:** (1) removing only the session-directory sync hid behind ext4's
  journal (the checkpoint's flush commits the rename), so the control removes every
  sync after the rename; (2) a storage failure reading committed state (ext4 shut down
  after a write error: `EIO`) was `INTEGRITY_FAILURE`; now `STORAGE_IO`
  (`map_committed_io`, also the root's layout check); (3) the ingest guarantee report.
- **Governance:** the checker allows `durability-campaign` only in development
  dependencies and the unpublished campaign tool's non-default `campaign` feature; the
  feature refuses release builds. New action pin: `actions/download-artifact` v8.0.1.
- **Residuals:** storage ignoring flushes (L-056), disk or host loss (L-057, X-10),
  Ubuntu recognised by `os-release` (L-058), engine-only durable requests (L-059),
  Windows/macOS unqualified (L-008).

## P10 PRs 1-3 (merged)

- PR 1 (`e2b14d9`): store split into `filesystem_session_store/`; #164 resolved (reads
  stop at the writer's `chain-verified.json`); durable protocol (then disabled); eleven
  commit fault points.
- PR 2 (`2ae55be`): `JobState` `Interrupted`/`Committing`, retry policy and poison rule;
  keys (request digest, recognition key, `opk_sha256_` operation key, `job_` id);
  checkpoints in `sessions/<ses>/jobs/<job>/chunks/`; exactly-once commit reconciled
  from the chain; `IDEMPOTENCY_CONFLICT` (exit 2, D-4); caps 512 / 384 / 128 KiB (D-2).
- PR 3 (`8af331b`): `job status/resume/cancel <job>`, `session status` `jobs`,
  `--operation-id` (D-1); first SIGINT/SIGTERM or Ctrl-C/Ctrl-Break cancels a long
  command, the second escalates; `job cancel` reaches a running owner within 250 ms;
  Tokio `signal` feature; Windows inherited "ignore Ctrl-C" is L-053.

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
| P10 | Implemented (ledger `in_progress`): PRs 1-3 merged, PR 4 complete on its branch; completion record after merge |
| P11, P12, P14 | Not started (P11 is next once P10 is recorded complete) |
| P13 | Not started; also delivers managed installation and human-readable output |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present, signals). `vsift-contract` sits beside the engine.
Storage: `filesystem_session_store/` (commit path in `commit.rs`/`publication.rs`, chain
in `chain.rs`, jobs in `jobs.rs`/`job_records.rs`), `durable_profile.rs`,
`fault_point.rs`, `retry_timer.rs`. Jobs: `vsift-application/src/job.rs` and
`job/run.rs`, `vsift/src/jobs.rs`; CLI `job.rs` and `signal.rs`. Campaign:
`tools/p10-crash-campaign/`.

## Quality evidence

- P10 PR 4 branch: fmt, strict Clippy (with and without features), workspace tests,
  warning-denied rustdoc, governance, fuzz fmt/Clippy/replay and cargo deny on Windows
  11; the Linux build, Clippy and tests plus the whole campaign on hosted
  `ubuntu-24.04`. Results and run links go in the PR description and the record.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
