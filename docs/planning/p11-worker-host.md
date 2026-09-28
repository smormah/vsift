# P11 worker-host qualification record

Status: recorded 2026-09-28 on P11 PR 4's second part (branch
`p11/qualification-docs`, on `main` at `45c25d1`, which holds PRs 1-4a). Design:
[ADR 0021](../decisions/0021-worker-and-batch-host.md) (accepted, D1-D5). Verification
IDs: X-07..X-11, O-01..O-04, SEC-T01 and the packet's repeated external-delivery
simulation. Operator guidance: [worker-host runbook](../operations/worker-host.md).
The single-host checkpoint ran on Windows 11 Pro, Intel Xeon E5-2698 v4 (20 cores, 40
threads), FFmpeg and FFprobe 9.0 (gyan.dev full build), whisper.cpp v1.9.2 with
`ggml-base.bin`, release build.

## Result in plain English

A supervisor can run VSift as a worker: one versioned request at a time or a finite
file of them, each committed at most once however often it is delivered or its worker
killed, a bounded number at a time and weighted by what the work occupies, with events
it can act on and a shutdown that leaves every started request resumable. Every P11
requirement has mechanical evidence below, and the `p11_*` checkpoint passed with real
tools. **SEC-T01 is met for P11 by non-adversarial evidence, which the maintainer
accepted on 2026-09-28; the adversarial containment evidence is technical debt,
required before the R0 release (known limit [L-068](known-limits.md#l-068)).** The
strict Linux profile stays a qualification target until P14.

## Method

Evidence comes in five layers, each at the lowest layer that can show it:

1. **Contract, domain and store tests** on every PR, on Ubuntu, macOS and Windows:
   strict request decoding, the digest, the D5 rule, the record table, retries and
   deadlines, the store's records, reader limits and schemas.
2. **Engine and binary tests** on every PR: `engine_worker`, `engine_batch`,
   `job_run_cli_contract`, `job_batch_cli_contract`, `workspace_cli_contract`,
   `weighted_admission`, `contained_inputs`, with child processes, OS locks and real
   signals (`SIGTERM` on Unix in CI; the console Ctrl-Break tests opt-in on Windows).
3. **Stress repetitions** on Windows 11 in four parallel lanes (below), and the opt-in
   repeated external-delivery simulation through the binary.
4. **The P10 crash campaign rerun with worker requests** on Ubuntu 24.04 virtual
   machines (run [36379513017](https://github.com/smormah/vsift/actions/runs/36379513017)).
5. **The opt-in `p11_*` checkpoint** (`crates/vsift-cli/tests/p11_worker_e2e.rs`) with
   real FFmpeg and whisper.cpp, expectations from the frozen corpus truth and from an
   uninterrupted control, never from the results scored.

Fuzzing: `job_request`, `job_batch_line`, `job_record`, `chunk_checkpoint` (PR 1),
`host_attestation` (PR 2), `request_record` (PR 3) and `job_batch_file` (PR 4), 23
targets in all, replayed over committed seeds on every PR and fuzzed weekly.

## Evidence per requirement

| ID | Evidence |
| --- | --- |
| X-07 (weighted admission, strict host limits) | `weighted_admission_never_exceeds_root_capacity`: five child processes race the engine's real weights on OS locks at capacities 2, 4 and 8 against a shared ledger; **100 of 100** runs passed, and a negative control counting every weight as one fails every run. Work heavier than the root fails `RESOURCE_LIMIT` before any tool runs; the bounded, jittered admission wait ends in `BUSY` with a 2 s hint. Through the binary, `p11_admission_ladder` (below) never saw more weight than the capacity. Host limits: the strict attestation reads the inherited cgroup v2 CPU, memory and PID limits (SEC-T01 row) and the `strict-worker-boundary` CI job shows CPU throttling, the PID ceiling and a failed over-limit allocation inside a hardened container |
| X-08 (backpressure, slow reader) | `batch_reads_ahead_at_most_concurrency_lines` (at concurrency 1-3 exactly that many requests start while the next line waits unread), `a_host_that_stops_reading_holds_the_batch_back`, `a_paused_stdout_reader_bounds_memory_and_admission` (binary: fewer than half of 120 requests start while stdout is unread, resident memory under 256 MiB on Linux, all complete once read), `progress_is_dropped_and_counted_not_buffered`; the reader counts the file before any work and holds one bounded line at a time (`batch_file` tests, `job_batch_file` fuzz target) |
| X-09 (retry classification, budget, deadline) | Domain `busy_is_retried_within_the_wait_and_nothing_else_is`, `a_deadline_near_exhaustion_is_deadline_exceeded`, the retry-class table; engine `busy_admission_is_retried_within_the_wait`, `deadlines_and_permanent_failures_are_not_retried`; `a_deadline_cancels_the_step_and_waits_for_it`, `a_cancellation_ends_a_backoff`. Only `BUSY` is retried, with full jitter, never past the deadline |
| X-10 (worker/OS crash versus disk/host loss) | Durable workspaces through the CLI (PR 2) and the crash campaign rerun with requests, run 36379513017: layer A 11,043 replay points with 20 request acknowledgements, 0 lost, 0 damaged, the negative control losing 53 of 80; layer B 320 VM kills, 8,557 acknowledgements (517 requests), 0 lost; layer C 180 injected write and flush failures, all `STORAGE_IO`, none acknowledged ([P10 record](p10-durable-publication.md), "P11 rerun"). Disk or host loss is the operator's responsibility, stated in the runbook (L-057). `p11_durable_workspace` checks the refusal off the profile |
| X-11 (mixed batch) | `every_line_is_isolated_and_reported`, `a_mixed_batch_streams_the_contract`; opt-in `a_mixed_batch_reports_independent_outcomes` (an import completes, a malformed line is rejected, a recognition cancelled mid-run with `job cancel`, a stand-in recognizer failing a chunk, candidates on a truncated file `partial`, the batch outcome the most severe by an independent D5 ordering); `p11_batch_mechanical` (below) |
| O-01 (sensitive values) | Sentinel path components, a rejected sidecar's text, a parent environment variable and `HTTP_PROXY`/`HTTPS_PROXY` credentials never reach stdout, stderr or events of `job run` and `job batch` (`the_event_stream_follows_the_contract`, `a_rejected_sidecar_never_reaches_output`, `a_mixed_batch_streams_the_contract`, `events_are_bounded_by_the_lines`; opt-in `provider_output_never_reaches_output` with a failing FFprobe); every `p11_*` batch stream is checked for the host's absolute paths |
| O-02 (bounded labels, buffers, logs) | Every string member of the new event schemas is an enum or a bounded pattern; progress is at most one per second and 4,096 per request and dropped, never queued, for a slow reader; `events_are_bounded_by_the_lines` (property test: every event line under 64 KiB and schema-valid, a fixed number of events per line) |
| O-03 (ready, busy, unhealthy, missing) | `lifecycle_events_distinguish_ready_busy_unhealthy_missing`: `started` with readiness; busy is `admission_waiting` then `BUSY` (exit 4); a gone workspace is one terminal `STORAGE_IO` without `started`; a missing tool is `MISSING_CAPABILITY` on the request; strict isolation off a strict host is `ISOLATION_UNAVAILABLE` before any work. Stage timings (`elapsed_ms`), admission waits (`admission_wait_ms`) and termination reasons are in every result and summary |
| O-04 (graceful shutdown) | `sigterm_stops_a_request_resumably`, `sigterm_stops_a_batch_resumably` (Unix, CI), `ctrl_break_stops_a_request_resumably`, `ctrl_break_stops_a_batch_resumably` (Windows, opt-in, run here), with and without a drain time: admission stops, exit 6, `draining` then `stopped: shutdown`, and redelivery completes; engine `a_shutdown_stops_the_batch_and_leaves_it_resumable`, `a_stopped_request_continues_from_its_next_step`; `p11_shutdown_and_redelivery` (below) |
| Repeated external-delivery simulation | `repeated_external_delivery_commits_once` (opt-in): twenty operation ids delivered at least once through the binary, a third with a racing duplicate, half the workers killed after 0-400 ms, until all completed; every request then replays exactly its first result and the workspace holds one session per id. 24 runs with different seeds passed; the final 12 took 53 rounds, 253 kills, 60 `BUSY` duplicates and 204 replays. Kill points: `a_kill_at_every_request_fault_point_recovers`, `a_kill_mid_batch_then_a_rerun_matches_the_control` |
| SEC-T01 | Non-adversarial evidence, accepted for P11 by the maintainer (2026-09-28): see the next section. Adversarial evidence: technical debt (L-068) |

**Stress repetitions** (Windows 11, four parallel lanes): `weighted_admission` 100 of
100 (PR 2); `racing_initialisations_create_one_workspace` 200 of 200 (PR 2);
`engine_worker`, `job_run_cli_contract` and the store's `worker_request` tests 300 of
300, then 120 of 120 on the final code (PR 3); `engine_batch` 100 of 100 (PR 4a).

## SEC-T01

SEC-T01 asks that an isolated worker contain a hostile native provider. For P11 the
maintainer accepted non-adversarial evidence (decision of 2026-09-28, option 2):

- **Attestation checks.** `--host-isolation strict-linux` is accepted only when bounded
  reads of the kernel's own view attest a cgroup v2 with finite CPU, memory and PID
  limits (on the cgroup or an ancestor), a root mount read-only in its own options and
  no network interface but loopback; anything unread fails closed with
  `ISOLATION_UNAVAILABLE` before any work. Tests: `a_cgroup_v2_membership_is_one_unified_line`,
  `cgroup_limits_are_max_or_a_count`, `only_loopback_counts_as_no_network`,
  `the_root_mount_is_read_only_only_when_its_own_options_say_so`,
  `the_strict_decision_needs_every_control`, `this_host_is_attested_or_every_gap_is_named`,
  `strict_isolation_is_attested_before_any_work` and the O-03 test above; fuzz targets
  `host_attestation` and `mountinfo`.
- **Container controls.** The `strict-worker-boundary` CI job runs the process
  supervisor's qualification test (`p08_qualified_linux_host_reports_inherited_kernel_controls`)
  in an Ubuntu 24.04 container with `--network none`, `--read-only`, `--pids-limit 64`,
  `--memory 512m --memory-swap 512m`, `--cpus 1`, `--cap-drop ALL`,
  `--security-opt no-new-privileges` and an unprivileged user: it verifies the
  inherited limits, the read-only mount and the absent network, that a new process
  group stays in the worker cgroup, and bounded CPU, PID and memory pressure.

No adversarial provider fixture exists; the adversarial evidence is technical debt,
deferred for maintainer discussion and required before the R0 release
([L-068](known-limits.md#l-068)). The runbook's deployments apply the same controls
as the CI job.

## The single-host checkpoint (`p11_*`)

```console
VSIFT_TEST_WHISPER_CLI=<abs> VSIFT_TEST_WHISPER_MODEL=<abs> \
  cargo test --release -p vsift-cli --locked --test p11_worker_e2e -- --ignored --nocapture
```

Each stage has its own isolated per-user base with FFmpeg, FFprobe, whisper.cpp and
the model registered, an empty `PATH`, an ephemeral worker workspace, an input root
holding the corpus files and a bundle root. Result on 2026-09-28: **`p11_worker:
passed`** in 209 s (release build). An earlier debug-build run, whose shutdown batch
held one recognition instead of two, passed in 462 s.

| Stage | Result | What it showed |
| --- | --- | --- |
| `p11_batch_mechanical` | passed, 26 s | A batch of F03's speech variant (ingest, recognition, candidates, retain), F10 with `F10.srt` at +500 ms (ingest with import, candidates, retain), F01 (ingest, candidates, retain with source, close), a malformed line and `../F01.mp4`, at concurrency 3 in a 16-unit workspace: 23.0 s, exit 2 (D5: two refused lines), 3 complete and 2 rejected (`malformed_request`, `invalid_path`), each refused line without a result. Then `search 127.50` cited [3.0 s, 8.0 s), inside F03's speech span [0.5 s, 8.575 s) and over the term; the candidate at 4.0 s lies in F03-E02 [4 s, 9 s) and `frame get --candidate` returned it at delta 0; `search R-17` cited exactly F10-E01 [5 s, 9 s); F01's one candidate lies in F01-E01. Three bundles validated through the binary and the storage crate, each naming its request's session and source with the recorded artifact count, and each manifest's SHA-256 equal to the recorded `bundle_sha256` |
| `p11_admission_ladder` | passed, 65 s | Four candidate requests over a 180 s clip in a four-unit workspace. Concurrency 1: 29.9 s, 1 in flight, peak sampled weight 2. Concurrency 2: 16.2 s, 2 in flight, peak weight 4 (two windows). Concurrency 4: 17.0 s, **4 in flight but at most two windows (weight 4)**, 10.6 s of admission wait reported across the requests. Every rung found the same 48 candidates per request |
| `p11_shutdown_and_redelivery` | passed, 117 s | Two recognitions of an 81 s clip (lines 1 and 4), a visual request and an import, at concurrency 2: the control took 41.7 s. The interrupted copy received a console Ctrl-Break 16.5 s in (the first checkpoint written, line 4 admitted) and exited 6 in 2.0 s with `termination_reason` `shutdown`, lines 1 and 4 `cancelled`, lines 2 and 3 complete, and no provider alive afterwards. `job resume` finished line 1's job (21.8 s); the redelivered file (29.9 s) continued both stopped requests (attempt 2; line 4 from its recognition step) and completed all four; every line's statuses, counts, bundle transcript segments and bundle candidates equal the control's. A third delivery replayed every line unchanged in 0.2 s |
| `p11_durable_workspace` | blocked (platform) | Off Ubuntu 24.04 / ext4, `session init-workspace --durability durable` answered `MISSING_CAPABILITY` and created nothing, as required. Required only on the qualified profile, where it runs a durable request and its replay |

Timings include each run's local-ASR preflight and model hashing (the F03 recognition
step took 20.5 s for a 9 s clip in the release build). Reports are written to
`.vsift/e2e-runs/p11-<id>/report.json`.

## Residuals

- **SEC-T01 adversarial evidence** is technical debt (L-068); the strict profile is
  attested and its controls shown present, not shown to contain a hostile provider.
- **Not run as written:** the runbook's systemd unit and container example (L-038);
  `p11_durable_workspace` on the qualified profile. The strict-Linux attestation itself
  now runs inside the hardened `strict-worker-boundary` container in CI
  (`p11_strict_linux_attestation_holds_inside_the_hardened_container`), but a full
  `job run --host-isolation strict-linux` there is not yet exercised.
- **One machine for the checkpoint:** the `p11_*` stages ran on Windows 11; Linux and
  macOS run the contract, engine and binary tests in CI (L-035).
- **Sampling is a lower bound:** the ladder's sampler sees provider processes about
  every 50 ms, not reservations; the cross-process weight property itself is the
  `weighted_admission` race test.
- **Operational readings for the maintainer:** a host-caused permanent failure
  replays under its operation id (L-069); requests of one batch contend with each other
  and a job-cancelled line exits 6 (L-067); the runbook's `KillMode=mixed`.
- Earlier limits that still apply to workers: L-055 (a `SIGKILL` of VSift alone on
  Unix), L-057 (disk or host loss), L-060 (no fairness between processes), L-061 (the
  free-space reserve), L-062 (no links in input paths), L-063 (4,096 records), L-064
  (staging directories), L-065 (per-delivery deadlines), L-066 (1,000 lines).
