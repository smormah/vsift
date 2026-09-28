# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P10 are complete. P11 (worker and batch host) is implemented; it completes when
its last pull request merges.** PRs 1-3 and PR 4's first part (`job batch`) are merged
(`0bcfac5`, `6e89bdb`, `d64dfa1`, `45c25d1`). PR 4's second part is on branch
`p11/qualification-docs` (local commits, not pushed): the `job_batch_file` fuzz target,
the `p11_*` single-host checkpoint, the operator runbook, the qualification record and
the final docs. ADR 0021 is accepted with maintainer decisions D1-D5 (2026-09-28).

1. **Review and merge PR 4b** (supervisor): push `p11/qualification-docs`, open the pull
   request with the gate results and the checkpoint timings from the qualification
   record, merge through protected checks.
2. **Then** one small follow-up sets P11 `complete` in the ledger with the merge commit
   and its verification record (delivery governance rule 9). Stop at packet completion;
   P12 starts only on the maintainer's word.

## Technical debt

- **SEC-T01 adversarial containment evidence** (known limit L-068, high): deferred by
  the maintainer on 2026-09-28 (option 2); P11 rests on non-adversarial evidence
  (attestation checks and the `strict-worker-boundary` container controls). Must be
  resolved before the R0 release (P14). Handoff document:
  `docs/planning/sec-t01-adversarial-handoff.md` (written by the supervisor); GitHub
  issue to be opened and linked in L-068.

## Decided (maintainer, 2026-09-28)

- **D1** Workspaces are created explicitly by `session init-workspace` with operator
  policy. **D2** Workspace sessions live a finite, workspace-set time (default 168 h,
  max 720 h). **D3** Request steps: ingest (with transcript import), retranscribe,
  candidates, retain, close. **D4** The first shutdown signal stops admitting and
  cancels in-flight work at its next boundary; drain is opt-in via
  `--drain-timeout-ms` (at most 300 s). **D5** `job batch` exits 0 when every request
  is complete or partial, 6 on shutdown, else the worst class 7 > 1 > 5 > 3 > 2 > 4.
- **SEC-T01 for P11:** non-adversarial evidence accepted; adversarial evidence is
  technical debt (above).

## Open decisions (maintainer)

- **PR 4b readings to confirm:** the runbook's systemd example uses `KillMode=mixed`,
  not the `control-group` of the brief (control-group signals providers together with
  VSift, so a step can fail instead of stopping resumably; ADR 0021 PR 4 second-part
  notes); a host-caused permanent failure (`MISSING_CAPABILITY`, the free-space
  reserve's `RESOURCE_LIMIT`) replays under its operation id, so supervisors resubmit
  under a new id (L-069).
- **PR 4a readings to confirm:** a file of more than 1,000 lines is refused whole
  before any work; a line over 64 KiB is refused alone; a job-cancelled line that is
  the most severe exits 6 like a shutdown, told apart by `termination_reason` (L-067);
  the engine depends on `tokio` directly.
- **PR 2-3 readings to confirm** (ADR 0021 notes): transient failures stay continuable;
  pruning only records of gone sessions; the 192 KiB record bound; the D2 reading;
  ephemeral workspace sessions report `durable_worker`; every input-path link refused
  (L-062). Also: MSRV and 0.x pre-releases; a local MCP adapter after P12.

## Tracked issues

- #14 (P11): close with the ledger follow-up. #180 (job record/checkpoint fuzz
  targets): close it. #144: close after a clean main.
- #170-#178: tracking issues for L-011, L-013, L-015, L-018, L-024, L-028, L-043,
  L-045 and L-042 (review pending).
- #159 motion fixtures; #150 noisy-speech fixtures; #147 faster-whisper adapter; #128
  supervisor tests flaky on Windows under load.

## Other follow-ups

- **Known limits:** 67 entries to L-069 (review pending). PR 4b rewrote L-004, L-010,
  L-038, L-055 and L-057 and added L-068 (SEC-T01 technical debt) and L-069.
- **Not yet run as written:** the runbook's systemd unit and container example (L-038);
  the `p11_durable_workspace` stage on Ubuntu 24.04 / ext4 (it reports `blocked`
  elsewhere); a CI run of `--host-isolation strict-linux` succeeding end to end.
- **Campaign upkeep:** bump the three `UBUNTU_IMAGE_*` values together (3 GiB images).

## Known issues and gates

- Real-tool success paths are opt-in (`--ignored`): the P07-P11 E2E tests, the
  `*_tools` engine tests, the Windows console-interrupt tests and the external-delivery
  simulation (L-042). Linux-only code is linted on Windows with `cargo clippy --target
  x86_64-unknown-linux-gnu` and runs for real only on Linux CI.
- Never run the crash campaign's scripts on a machine whose disks matter. Durability
  ends at the disk (L-056, L-057); Windows/macOS durable requests fail closed.

## Guardrails

- R0 ships only when a coding agent goes from a local video to a grounded handoff through
  both the supplied-transcript and local-ASR paths, in named Codex and Claude Code trials.
- R1 packets P15..P20 start only after P14. Live capture (#107, #108) waits.
- New features land in the engine once, never in a host; the worker's public wire types
  are `WorkRequest` / `WorkResult`, never the application's `JobRequest`.
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas;
  a new parser of untrusted input needs a fuzz target with committed-seed provenance.
- Session commits go through `CommitHooks`/`Commit`; a change to the commit path (or to
  request-record writes) reruns the crash campaign. Never enable `fault-injection` or
  `durability-campaign` in a release.
