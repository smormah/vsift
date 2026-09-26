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
- transcribe the video's speech itself with whisper.cpp (`transcript retranscribe`);
- return timestamped transcript segments for a time range, as a page or a JSON Lines
  stream of keyed evidence records;
- search the transcript for words (`search`);
- list the moments where the screen changed (`candidates`);
- return the exact frame at a time or of a candidate, the frames around one, bursts
  over up to 60 s and native-size crops as full-resolution PNG files, and WAV clips of
  up to 30 s, with requested and actual time, lineage and reuse;
- manage the session's lifetime and retention, and validate retained bundles;
- keep every folder it creates private to the user.

**P00-P09 are complete** (P09 merged `e57c706`, ADR 0019 accepted). **P10 (recovery
integration) is in progress:** of its four pull requests, PR 1 (the commit path) is
complete on branch `p10/commit-path` and not yet merged; PRs 2-4 have not started.
Every known limit, residual risk and deferral is in `docs/planning/known-limits.md`.

## P10 PR 1: the commit path (internal, on its branch)

Design and decisions: [ADR 0020](../docs/decisions/0020-recoverable-jobs-and-durable-publication.md)
(Proposed; D-1..D-5 await the maintainer). No public contract changes.

- **Store split:** `filesystem_session_store.rs` (5,200 lines) is now a directory
  module: `mod.rs` (types, stored shapes, shared I/O) and `root`, `initialization`,
  `commit`, `publication`, `chain`, `stored`, `lifecycle`, `reads`, `evidence`, `work`,
  `index`, `cleanup`, `bundle`, plus `tests.rs` and `p10_tests.rs`.
- **#164, incremental chain validation:** the writer keeps
  `sessions/<ses>/chain-verified.json` at the head it just committed (under the writer
  lock, staged and renamed, best effort); readers never write it. A read verifies the
  pointer, the head and every generation down to that checkpoint or the last head the
  same store instance verified. Missing or ahead of the head -> full walk; malformed or
  forged -> `INTEGRITY_FAILURE`; newer -> `UNSUPPORTED_SCHEMA`. Retain and cleanup
  still walk everything; artifacts are still re-hashed (INV-02). Warm reuse through the
  binary: p95 139 / 369 / 1,064 / 3,794 ms at 2 / 64 / 256 / 1,024 generations before;
  136-175 ms at 256 and 149-156 ms at 1,024 after (three runs), slope about 0 (target
  0.2 ms per generation met; 160 ms at 256 met in one of three runs, start-up jitter).
- **Durable protocol (disabled):** a session records `durability` at generation 0
  (absent = ephemeral; ephemeral manifests omit it) and every generation carries it;
  `publish_artifact`/`publish_evidence` pass it. Durable commits: create/write/flush
  each artifact, sync `artifacts/`; stage/flush/rename the manifest, sync
  `generations/`; stage/flush/rename the pointer, sync the session; then acknowledge;
  then the checkpoint. Initialization also syncs `sessions/` and the index bucket.
  Directories sync via `dir.open(".")?.sync_all()` (FS-01). fsyncgate: never re-flush a
  leftover file; accept an existing artifact only if the head lists it; a retry
  re-syncs before acknowledging. `durable_profile` claims `os_crash_durable` only on
  Linux, ext4 without `nobarrier`/`barrier=0` (bounded strict `mountinfo` parse) and
  `QUALIFIED_UBUNTU_EXT4`, which is `false`: every profile still fails closed.
- **Fault points:** `FaultPoint` (eleven, `artifact-install` .. `chain-checkpoint-write`)
  in `fault_point.rs`; `VSIFT_FAULT_POINT=<name>[:n]` exits with 91 in unit tests and
  `fault-injection` builds (feature off by default, `compile_error!` without debug
  assertions, governance check). Commits take `CommitHooks` (fault plan, in-process
  failure, test-only trace) and a `Commit` (the session's durability).
- **Tests:** S-07 kill at every point (ephemeral everywhere, durable on Unix), durable
  order trace on every platform, fsyncgate retries, S-08 checkpoint damage, the cache,
  the mountinfo parser; fuzz target `mountinfo`; the opt-in
  `s11_warm_reuse_as_the_manifest_chain_grows` measurement (`evidence_cli_contract`).
- **Not compiled on Windows:** the `cfg(target_os = "linux")` profile check
  (`os_crash_durable`, the constant) and the `cfg(unix)` directory sync and durable
  kill variant; CI's Ubuntu and macOS jobs compile and run them.

## P10 still to do

- **PR 2:** jobs keyed by `opk_sha256_` (job id from session + key), `ChunkCheckpoint`
  files under `sessions/<ses>/jobs/<job>/chunks`, `JobState` `Interrupted` and
  `Committing`, the OS lock as liveness authority, retry policy (BUSY auto-retry <= 2,
  full jitter 200 ms / 2 s; poison chunk after 3), checkpointed `transcript
  retranscribe`; needs D-1 and D-4.
- **PR 3:** public job surface and cancellation serialized with the commit; Ctrl-C and
  SIGTERM via Tokio `signal` (supersedes ADR 0017 decision 4).
- **PR 4:** Ubuntu 24.04 / ext4 campaign (dm-log-writes >= 2,000 points, >= 300 QEMU
  kills, dm-flakey EIO, negative control), X-10 host-loss boundary, then flip the
  constant; D-5 decides where it runs.

## P09: evidence navigation (complete)

- `frame get/neighbours/burst`, `crop`, `audio` with `--json`, `--events jsonl`
  (`frame_evidence`, `audio_evidence`) and `files[]` absolute paths (D2); request keys
  (`opk_sha256_`) and item identities (`evd_`); warm reuse starts no process; D1
  identity-only source checks after one full hash; per-call budgets; 160 evidence
  artifacts per session (D4, unchanged until D-2).
- Qualified in `docs/planning/p09-evidence-navigation.md`: `p09_evidence_e2e` eleven
  stages in 471 s (Windows 11, FFmpeg 9.0, whisper.cpp v1.9.2), including the mechanical
  journey on both transcript paths.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model`, the read-only `setup plan`.
- `ingest <video> [--transcript <file> [--transcript-offset <signed us>]]`: disposable
  session (24 idle hours, at most 7 days); supplied SRT/WebVTT import never needs whisper.
- `transcript retranscribe <session> [--from --to]`, `transcript get ... [--events jsonl]`.
- `search <session> --query <text> [--from --to] [--limit] [--cursor] [--revision]`.
- `candidates <session> --from <us> --to <us> [--limit 1..100] [--cursor]`.
- `frame get/neighbours/burst`, `crop`, `audio` (P09).
- `session list/status/renew/close/retain/clean` and `bundle validate` (records too).
- Still `COMMAND_NOT_IMPLEMENTED`: job and setup
  install/repair/list/rollback/remove. Human-readable terminal output is P13's.

## Earlier packets and the engine

- P07/P08: 30 s ASR chunks with 5 s overlap (`base` default, 3.25% clean WER), search
  tiers, 0.5 s visual sampling with a candidate per 10 s cell (`p08-candidate-recall.md`).
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
| P10 | In progress (ledger `in_progress`): PR 1 complete on its branch; PRs 2-4 to do |
| P11, P12, P14 | Not started |
| P13 | Not started; also delivers managed installation and human-readable output |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present). `vsift-contract` sits beside the engine. Storage:
`filesystem_session_store/` (commit path in `commit.rs`/`publication.rs`, chain in
`chain.rs`), `durable_profile.rs`, `fault_point.rs`. Largest files now:
`managed_artifact_store.rs` and `filesystem_session_store/tests.rs`.

## Quality evidence

- P10 PR 1 branch, Windows 11: fmt, strict Clippy (with and without features),
  754 workspace tests passing (51 opt-in ignored), warning-denied rustdoc, governance,
  fuzz fmt/Clippy/replay (15 targets). Results go in the PR description.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. History in git, `CHANGELOG.md` and `docs/history/`.
