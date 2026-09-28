# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P11 are complete. P12 (agent skill) is in progress.** Merged increments: PR 1
(#196, skill, guard, ADR 0022 Proposed) and PR 2 (#201, `9d2f60e`, harness, 21
scenarios, SEC-T02 suite). PR 3a (branch `p12-pr3a-dryrun-fixes`) fixes what the first
dry trials showed. The maintainer approved the named-client trials (~80 counted runs:
Claude Code `claude-opus-5-5`/`claude-haiku-4-5-20251001`, Codex
`gpt-6-astra`/`gpt-6-luna`); no counted trial has run.

1. **Maintainer: decide how Codex trials run (L-076).** On Windows Codex's unelevated
   sandbox cannot run VSift (its private session root refuses the sandbox SID:
   `STORAGE_IO`/`INTEGRITY_FAILURE`) and does not enforce the network. Options: WSL or
   Ubuntu; `danger-full-access` graded only; the elevated sandbox (admin setup, untested).
2. **Maintainer: re-run the Claude Code dry trial on PR 3a** (runbook
   `docs/agents/trials.md`): no trust warning, not `INVALID TRIAL`, `search` run.
   Then the counted trials, graded and recorded under `docs/planning/p12-agent-trials/`.
3. **Maintainer: review ADR 0022** (decisions 3, 4, 7, the PR 2 and dry-trial notes):
   reading allowances, which scenarios get five trials, the F12-E02 window reading.
4. **Technical debt:** SEC-T01 adversarial containment evidence (#188, L-068), for
   maintainer discussion before P14.

## Found in P12 (for the maintainer)

- **Dry trials (PR 3a, fixed):** Claude Code ignored an untrusted workspace's allow
  rules (the harness now trusts each workspace; one settings source; a client's own
  "ignored" report makes the trial invalid); Codex rejected every command without a
  Windows sandbox mode (now `unelevated`); Opus never ran `search` (skill says search
  first, guarded). Claude Code runs read-only commands like `echo` under `dontAsk`;
  the grader fails them.
- **L-071 (P13):** no remediation for an unparsable line in JSON modes (quote `--rect`
  on PowerShell). **L-074 (open):** SubRip import removes any `<letter...>` tag.
- **Plan vs code/truth:** `session renew` and `job cancel` are `explicit` as a whole;
  F05's code is spoken at 6.5 s but shown from 9 s, F12's is drawn from the first
  frame, so the grader binds spoken facts by text (ADR 0022 note).

## Decided (maintainer, 2026-09-28)

- **P11 D1-D5** (ADR 0021): explicit workspaces, workspace-set retention, request
  steps, stop-then-cancel shutdown with opt-in drain, the D5 batch exit.
- **SEC-T01 for P11:** non-adversarial evidence accepted; adversarial as debt (above).
- **P13 plan:** npm launcher over per-platform packages, no install scripts, npm/pnpm/
  Yarn/Bun; all names held, no announcement before P14 (open items: P13 name checklist).

## Open decisions (maintainer)

- **ADR 0022** and **L-076** (above).
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

- #15 (P12) packet issue; #14 (P11) close with the ledger follow-up; #180 close;
  #144 close after a clean main. #170-#178 track L-011/013/015/018/024/028/043/045/042.
- #159 motion fixtures; #150 noisy-speech fixtures; #147 faster-whisper; #128 flaky
  Windows supervisor tests.

## Other follow-ups

- **Known limits:** entries to L-076, review pending (P12 added L-071..L-076).
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
