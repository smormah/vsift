# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P11 are complete. P12 (agent skill) is in progress.** Merged: PRs 1, 2, 3a-3h
(3h #216 `56f1e1f`: `display_text`). The final counted campaign ran on `56f1e1f`.
**PR 3i** (branch `p12-pr3i-final-fixes`, in review) is the last fix round before P12
closes: the maintainer's four decisions of 2026-09-30 (orientation probes, a redrawn
check image, resumed runs, number and time matching). An increment, not the packet.

1. **Review and merge PR 3i.** Its re-grade (`grade-3i.json`, table in the PR) shows
   only what the grader changes fix.
2. **Then re-run the compact tier** (Claude Sonnet 5.5 in Claude Code 2.1.284, GPT-6-Sol
   in Codex) on PR 3i's merge commit, images rebuilt there: the skill text (resume.md)
   and the check image changed, so the old runs cannot show their effect. Check the
   init event lists only `vsift` (`disableBundledSkills`). Records go to
   `docs/planning/p12-agent-trials/`.
3. **Maintainer: review ADR 0022** (decisions 3, 4, 7 and the notes, including PR 3f's
   to PR 3i's readings), the corpus truth amendment, L-075, L-078..L-084.
4. **Technical debt:** SEC-T01 adversarial evidence (#188, L-068), before P14.

## Found in P12 (for the maintainer)

- **Final campaign on `56f1e1f`** (full passes of 28; review tier of 11): Sonnet 5.5 25,
  GPT-6-Sol 15 (25 answers right), GPT-6-Luna 15, Opus 5.5 9, GPT-6-Astra 9. PR 3i's
  re-grade: Sol 21 (its 5 misread check codes remain), Luna 16, others unchanged.
- **Resume (A-02 phase 2, 9 of 9 resumed runs missed F02-E02):** the resumed agent took
  the card's `remaining` as its own budget (Codex saw 0 images left and opened none) and
  reported the earlier run's "queue depth 12" as unsupported or dropped it; some cards
  kept no frame or segment behind it. PR 3i: a new run has its own budget, repeats the
  image check and verifies each earlier finding with one command; the card may list
  `to_verify` (finding, evidence, window). Optional, not required: a decision.
- **Still strict after PR 3i:** an `rg --files` exclude glob with a separator
  (`!**/.git/**`), harmless since an exclude only narrows: a decision. Also open:
  `untrusted_listed` takes only F12-E01 (0-8 s); L-071 (P13); L-074.

## Decided (maintainer, 2026-09-28/30)

- **P11 D1-D5** (ADR 0021); SEC-T01 non-adversarial evidence accepted for P11.
- **P13 plan:** npm launcher over per-platform packages, no install scripts; names
  held, no announcement before P14 (P13 name checklist open); handoff validator (#213).
- **PR 3e-3h:** free unpiped `--help`; plain Windows paths when exact (#210); start-folder
  orientation is housekeeping; slim handoff v1; vocabulary; card only when work can
  continue; `display_text`. **Compact tier:** Sonnet 5.5 and GPT-6-Sol; GPT-6-Luna
  (L-084) and Haiku 4.5 (L-082) below the line.
- **PR 3i (2026-09-30):** `command -v`/`which <name>`, `ls -l` of named files, `true`
  are housekeeping; a new check image; resume guidance; number and time matching.

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
  Windows supervisor tests; #210 close (fixed by #215).

## Other follow-ups

- **Known limits:** entries to L-084, review pending (P12 added L-071..L-084).
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
  the same change (the `skill_contract` tests fail otherwise). The check image's code
  lives only in its pixels and, split, in the guard and the grader's image table.
- A new public command, failure code, event kind or record type needs its `CommandName`,
  `FailureCode::ALL`, `EventKind::ALL` or `EvidenceRecordType::ALL` entry and v1 schemas;
  a new parser of untrusted input needs a fuzz target with committed-seed provenance.
- Session commits go through `CommitHooks`/`Commit`; a change to the commit path (or to
  request-record writes) reruns the crash campaign. Never enable `fault-injection` or
  `durability-campaign` in a release. R1 packets P15..P20 start only after P14.
