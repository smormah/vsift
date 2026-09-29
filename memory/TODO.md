# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P11 are complete. P12 (agent skill) is in progress.** Merged increments: PR 1
(#196), PR 2 (#201), PR 3a (#203), PR 3c (#207, `9791f70`) and PR 3d (#208, `ed07c0d`,
skill-folder listings). PR 3b (branch `p12-pr3b-codex-container`, in review) runs the
Codex trials in a Linux container. The maintainer approved ~80 counted runs (Claude
Code `claude-opus-5-5`/`claude-haiku-4-5-20251001`; Codex `gpt-6-astra`/`gpt-6-luna`).

1. **Claude Code counted trials (39 runs):** the third dry trial passed cleanly on
   `9791f70`; the first counted run then listed the skill's `examples/` with `Glob`
   and the campaign was stopped. It restarts from zero after PR 3d, from a frozen
   checkout, with records under `docs/planning/p12-agent-trials/`.
2. **Codex trials in a Linux container (decision 2026-09-28, L-076, #204):** PR 3b.
   Its dry A-08 trial passed all but `image_check`: codex-cli 0.155's stream has no
   event for a viewed image. **Maintainer: decide how to grade Codex's images (L-075)**
   and review L-078..L-080 before the Codex counted trials.
3. **Maintainer: review ADR 0022** (decisions 3, 4, 7 and the notes) and the corpus
   truth amendment (persistent events F04-E05, F05-E04, F12-E03; corpus README).
4. **Technical debt:** SEC-T01 adversarial containment evidence (#188, L-068), for
   maintainer discussion before P14.

## Found in P12 (for the maintainer)

- **Codex container (PR 3b):** bubblewrap needs user namespaces (seccomp, L-078) and
  existing writable roots; VSift refuses a session root it did not provision.
- **Second Claude dry trial (PR 3c, fixed):** it chained `date` to time its budget
  (now forbidden; the host keeps the wall time, L-077), first sent an invalid
  `--operation-id`, and a correct frame citation of `INVOICE 4407` at 9 s failed
  because the key placed it only in 0-5 s (truth amended, grader unchanged).
- **Still open (grader):** `untrusted_listed` takes only F12-E01 (0-8 s) though the
  on-screen instructions last to 12 s; a frame before 8 s binds `install` (speech only).
- **L-071 (P13):** no remediation for an unparsable line in JSON modes (quote `--rect`
  on PowerShell). **L-074 (open):** SubRip import removes any `<letter...>` tag.
- **Plan vs code/truth:** `session renew` and `job cancel` are `explicit` as a whole;
  F05's code is spoken at 6.5 s, shown from 9 s: spoken facts bind by text.
## Decided (maintainer, 2026-09-28)

- **P11 D1-D5** (ADR 0021): explicit workspaces, workspace-set retention, request
  steps, stop-then-cancel shutdown with opt-in drain, the D5 batch exit.
- **SEC-T01 for P11:** non-adversarial evidence accepted; adversarial as debt (above).
- **P13 plan:** npm launcher over per-platform packages, no install scripts, npm/pnpm/
  Yarn/Bun; all names held, no announcement before P14 (open items: P13 name checklist).

## Open decisions (maintainer)

- **ADR 0022** (above); **#204** (the product side of L-076: Codex on Windows).
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

- **Known limits:** entries to L-080, review pending (P12 added L-071..L-080).
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
