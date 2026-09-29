# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P11 are complete. P12 (agent skill) is in progress.** Merged: PRs 1, 2, 3a-3e
(3e #211, `261b50d`, where the counted campaigns ran: Opus 5.5/Haiku 4.5 in Claude
Code, GPT-6-Astra/GPT-6-Luna in Codex; raw logs local). **PR 3f** (branch
`p12-pr3f-orientation-slim-handoff`, in review) implements the maintainer's decisions
of 2026-09-29: orientation in the starting folder is housekeeping, and handoff v1
requires only what the agent alone knows (recorded values optional, checked if given).

1. **Review and merge PR 3f** (an increment, not the packet). Its table re-grades every
   counted run (`grade-3f.json` beside `grade.json`); the skill changed too.
2. **Then the approved re-run of the small models** (Haiku 4.5, GPT-6-Luna) on PR 3f's
   merge commit: Claude Code from a frozen checkout, Codex after rebuilding both images
   at that commit. Records go to `docs/planning/p12-agent-trials/`.
3. **Maintainer: review ADR 0022** (decisions 3, 4, 7 and the notes, including PR 3f's
   readings: `command -v` strict, `pixels_inspected` required, a scenario's gap `code`
   still required), the corpus truth amendment (F04-E05, F05-E04, F12-E03), L-075,
   L-078..L-081.
4. **Technical debt:** SEC-T01 adversarial evidence (#188, L-068), before P14.

## Found in P12 (for the maintainer)

- **Counted campaigns re-graded (PR 3f):** Opus 11/11 and Astra 11/11 pass both
  results; GPT-6-Luna mechanical 5 -> 20 of 31 (left: handoff shapes, 3 partial reports
  without a resume card, 2 `printf`); Haiku 3/28 (invents its own handoff shapes).
- **Codex images (L-075, decided):** the right check code proves image access; its
  image budgets are unmeasured (the rollout hides views in a code-mode `exec` call).
- **Still open (grader):** `untrusted_listed` takes only F12-E01 (0-8 s) though the
  on-screen instructions last to 12 s; the line-filter allowance (`| head` after a
  command) is still the harness's, not the skill's.
- **L-071 (P13):** no remediation for an unparsable line in JSON modes. **L-074
  (open):** SubRip import removes any `<letter...>` tag. **#210:** the `\\?\` retry.

## Decided (maintainer, 2026-09-28/29)

- **P11 D1-D5** (ADR 0021): explicit workspaces, workspace-set retention, request
  steps, stop-then-cancel shutdown with opt-in drain, the D5 batch exit.
- **SEC-T01 for P11:** non-adversarial evidence accepted; adversarial as debt (above).
- **P13 plan:** npm launcher over per-platform packages, no install scripts; all names
  held, no announcement before P14 (open items: P13 name checklist).
- **PR 3e (supervisor):** `--help` forms free, never piped; the `\\?\` retry stays
  (#210). **PR 3f (maintainer):** the two decisions above; `cd` elsewhere stays strict.

## Open decisions (maintainer)

- **ADR 0022** (above); **#204** (the product side of L-076: Codex on Windows).
- **P11 PR 4b readings to confirm:** the runbook's systemd example uses
  `KillMode=mixed`; a host-caused permanent failure replays under its operation id, so
  supervisors resubmit under a new id (L-069).
- **P11 PR 4a readings to confirm:** a file of more than 1,000 lines is refused whole;
  a line over 64 KiB is refused alone; a job-cancelled line that is the most severe
  exits 6, told apart by `termination_reason` (L-067); the engine depends on `tokio`.
- **P11 PR 2-3 readings** (ADR 0021 notes): continuable transient failures; pruning;
  192 KiB records; D2; `durable_worker` for ephemeral workspace sessions; input-path
  links refused (L-062). Also: MSRV and 0.x pre-releases; an MCP adapter after P12.

## Tracked issues

- #15 (P12) packet issue; #14 (P11) close with the ledger follow-up; #180 close;
  #144 close after a clean main. #170-#178 track L-011/013/015/018/024/028/043/045/042.
- #159 motion fixtures; #150 noisy-speech fixtures; #147 faster-whisper; #128 flaky
  Windows supervisor tests; #210 the `\\?\` image path.

## Other follow-ups

- **Known limits:** entries to L-081, review pending (P12 added L-071..L-081).
- **Not yet run as written:** the runbook's systemd unit and container example (L-038);
  `p11_durable_workspace` on Ubuntu 24.04 / ext4; `strict-linux` end to end in CI.
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
