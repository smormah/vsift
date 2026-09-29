# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P11 are complete. P12 (agent skill) is in progress.** Merged: PRs 1, 2, 3a-3g
(3e #211 `261b50d`: the review tier's counted campaigns, Opus 5.5 and GPT-6-Astra;
3f #212 `b68d746`: the compact-tier runs, Sonnet 5.5, GPT-6-Luna, Haiku 4.5; raw logs
local; 3g #214 `f018e0d`: compact tier is Sonnet 5.5 and GPT-6-Luna, Haiku 4.5 below
the line (L-082), handoff vocabulary table and exact resume card; validator in P13,
#213). **#210 fix** (PR #215, branch `fix/210-plain-windows-paths`, in review): Windows
`files[].path` is plain `C:\...` when exact, `\\?\` otherwise (ADR 0019 note, L-016).

1. **Review and merge PR #215** (a standalone fix, not a packet increment).
2. **Then re-run the compact tier** (Sonnet 5.5 in Claude Code 2.1.284, GPT-6-Luna in
   Codex) on the latest `main`, images rebuilt there; check the init event lists only
   `vsift` (`disableBundledSkills`). Records go to `docs/planning/p12-agent-trials/`.
3. **Maintainer: decide the hidden-character proposal** (L-083; ADR 0022's PR 3g note:
   `text` renders bidi and zero-width characters as `<U+XXXX>`, `original_text` keeps
   them; a contract change, not implemented).
4. **Maintainer: review ADR 0022** (decisions 3, 4, 7 and the notes, including PR 3f's
   and PR 3g's readings: `observed` never `unsupported`, unused citations a warning,
   `remaining.images` accepted; the resume card only when work can continue, a
   supervisor decision), the corpus truth amendment, L-075, L-078..L-083.
5. **Technical debt:** SEC-T01 adversarial evidence (#188, L-068), before P14.

## Found in P12 (for the maintainer)

- **Compact tier on `b68d746`:** Sonnet 28/28 answers, 9/28 full; Luna 24, 11; Haiku 6, 2.
- **Hidden characters (L-083):** VSift's `text` keeps raw U+202E (and decodes WebVTT's
  `&#x202E;` into it); Sonnet copied one into 1 of 5 SEC-T02 reports, Haiku 4 of 5.
- **Still open (grader):** `untrusted_listed` takes only F12-E01 (0-8 s) though the
  instructions last to 12 s; the line-filter allowance is the harness's, not the skill's.
  L-071 (P13) and L-074 (SubRip `<letter...>` tags) stay open; #210 is in review.

## Decided (maintainer, 2026-09-28/29)

- **P11 D1-D5** (ADR 0021): explicit workspaces, workspace-set retention, request
  steps, stop-then-cancel shutdown with opt-in drain, the D5 batch exit.
- **SEC-T01 for P11:** non-adversarial evidence accepted; adversarial as debt (above).
- **P13 plan:** npm launcher over per-platform packages, no install scripts; all names
  held, no announcement before P14 (open items: P13 name checklist); a handoff
  validator command (#213).
- **PR 3e (supervisor):** `--help` forms free, never piped; the skill's `\\?\` retry
  stays. **#210 (maintainer):** plain Windows paths when exact, `\\?\` only when needed;
  the broad legacy reserved-name rule accepted. **PR 3f (maintainer):** orientation is
  housekeeping (`cd` elsewhere strict); handoff v1 states only what the agent knows.
  **PR 3g (maintainer):** the compact tier
  is Sonnet 5.5 and GPT-6-Luna; fix the vocabulary now, validator in P13. **PR 3g
  (supervisor):** resume card only when work can continue; keep `disableBundledSkills`.

## Open decisions (maintainer)

- **ADR 0022** (above); **#204** (the product side of L-076: Codex on Windows).
- **P11 readings to confirm** (ADR 0021 notes): PR 4b's systemd `KillMode=mixed` and
  resubmission under a new operation id (L-069); PR 4a's batch file and line limits and
  exit 6 for a job-cancelled line (L-067), the engine's `tokio`; PR 2-3's continuable
  transient failures, pruning, 192 KiB records, D2, `durable_worker` for ephemeral
  workspace sessions, input-path links refused (L-062). Also: MSRV, 0.x pre-releases,
  an MCP adapter after P12.

## Tracked issues

- #15 (P12) packet issue; #14 (P11) close with the ledger follow-up; #180 close;
  #144 close after a clean main. #170-#178 track L-011/013/015/018/024/028/043/045/042.
- #159 motion fixtures; #150 noisy-speech fixtures; #147 faster-whisper; #128 flaky
  Windows supervisor tests; #210 plain Windows image paths (PR #215).

## Other follow-ups

- **Known limits:** entries to L-083, review pending (P12 added L-071..L-083).
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
