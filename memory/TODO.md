# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P11 are complete. P12 (agent skill) is in progress.** PR 1 (#196, `7990fdf`)
merged the skill, guard and ADR 0022 (Proposed); PR 2 (#201) adds the trial harness,
21 scenarios, the SEC-T02 suite and the procedure checkpoint. Increments; no trial run.

1. **Maintainer: review ADR 0022** (decisions 3, 4, 7 and the PR 2 note), including
   the harness's reading allowances, which scenarios get five trials (all 21 x 5 x 2
   clients x 2 models is about 420 runs) and the F12-E02 window reading.
2. **Next P12 increment (PR 3): run the named-client trials** with the runbook
   `docs/agents/trials.md`. Maintainer: create `C:\vsift-trials`, sign each client in
   once into a trial home under it (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`), name the compact
   and review models. First one dry trial per client to confirm the stream formats and
   flags (L-075), then the counted trials, graded, reviewed and recorded under
   `docs/planning/p12-agent-trials/`.
3. **Technical debt:** SEC-T01 adversarial containment evidence (#188, L-068,
   `docs/planning/sec-t01-adversarial-handoff.md`), for maintainer discussion before P14.

## Found in P12 (for the maintainer)

- **L-070 (fixed 2026-09-28):** the `job resume` remediation for a closed or expired
  session now says to open a new session with `ingest`; the limit is removed.
- **#197 (fixed, #200 `98525dc`):** a kill during session registration left an empty
  index marker that failed every listing; the marker is now staged and renamed.
- **L-071 (deferred to P13):** a command line that does not parse answers `parse`
  with no remediation in JSON modes; on PowerShell an unquoted `--rect a,b,c,d` fails
  that way. The skill tells agents to quote it.
- **L-074 (P12 PR 2, open):** the SubRip importer removes any `<letter...>` tag, not
  only the `<i>/<b>/<u>/<font>` the contract lists; `original_text` keeps everything.
- **Plan vs code/truth:** `session renew` and `job cancel` are `explicit` as a whole
  (PR 1); the F05 error code is spoken at 6.5 s but shown from 9 s, and F12's code is
  drawn from the first frame, so the grader binds spoken facts by text (ADR 0022 note).

## Decided (maintainer, 2026-09-28)

- **P11 D1-D5** (ADR 0021): explicit workspaces, workspace-set retention, request
  steps, stop-then-cancel shutdown with opt-in drain, the D5 batch exit.
- **SEC-T01 for P11:** non-adversarial evidence accepted; adversarial as debt (above).
- **P13 plan:** npm launcher over per-platform packages, no install scripts, npm/pnpm/
  Yarn/Bun; all names held, no announcement before P14 (open items: P13 name checklist).

## Open decisions (maintainer)

- **ADR 0022** (above).
- **P11 PR 4b readings to confirm:** the runbook's systemd example uses
  `KillMode=mixed`; a host-caused permanent failure replays under its operation id, so
  supervisors resubmit under a new id (L-069).
- **P11 PR 4a readings to confirm:** a file of more than 1,000 lines is refused whole;
  a line over 64 KiB is refused alone; a job-cancelled line that is the most severe
  exits 6, told apart by `termination_reason` (L-067); the engine depends on `tokio`.
- **P11 PR 2-3 readings to confirm** (ADR 0021 notes): transient failures stay
  continuable; pruning only records of gone sessions; the 192 KiB record bound; the D2
  reading; ephemeral workspace sessions report `durable_worker`; every input-path link
  refused (L-062). Also: MSRV and 0.x pre-releases; a local MCP adapter after P12.

## Tracked issues

- #15 (P12): the packet issue. #14 (P11): close with the ledger follow-up. #180:
  close it. #144: close after a clean main.
- #170-#178: tracking issues for L-011, L-013, L-015, L-018, L-024, L-028, L-043,
  L-045 and L-042 (review pending).
- #159 motion fixtures; #150 noisy-speech fixtures; #147 faster-whisper adapter; #128
  supervisor tests flaky on Windows under load.

## Other follow-ups

- **Known limits:** to L-075 (review pending); P12 added L-071..L-075 (L-070 fixed).
- **Not yet run as written:** the runbook's systemd unit and container example (L-038);
  the `p11_durable_workspace` stage on Ubuntu 24.04 / ext4; a CI run of
  `--host-isolation strict-linux` succeeding end to end.
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
- The skill orchestrates the published CLI only: no processing logic, no tool grants.
  A new public command, flag, failure code or referenced field needs a skill update in
  the same change (the `skill_contract` tests fail otherwise).
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas;
  a new parser of untrusted input needs a fuzz target with committed-seed provenance.
- Session commits go through `CommitHooks`/`Commit`; a change to the commit path (or to
  request-record writes) reruns the crash campaign. Never enable `fault-injection` or
  `durability-campaign` in a release. R1 packets P15..P20 start only after P14.
