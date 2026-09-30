# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P12 are complete; P13 is in progress** (started 2026-09-30, plan accepted). PR 0,
the kickoff (#226, `dbc60f7`: ADR 0023 Proposed, ledger `in_progress` with R-13), is
merged; no P13 code yet. The P12 debt fixes (L-085) are on `p12-debt-skill-fixes`.

1. **Next P13 PRs** (`implementation-work-packets.md` "P13 scope and pull requests"):
   1 L-071; 2a/2b human output; 3 smoke executor; 4 install; 5 `handoff check` (after
   the P12 debt); 6 lifecycle; 7 kill/power-loss, E2E; 8 `release.yml`; 9 npm; 10
   publish wiring; 11 docs; 12 ledger. #222 re-runs after PR 5.
2. **Maintainer-only, before PR 10's publish step:** npm account with 2FA; **choose the
   platform-package scope** (`@vsift` refused by npm; `@vsift-cli`, `@vsifthq` or
   `@vsiftdev`), create its organisation and record it in an ADR 0009 note; trusted publishers; the GitHub `release` environment with
   the maintainer as reviewer; the tag ruleset; fork-PR workflow approval; the first
   publish (personally with 2FA, or a short-lived token only in the environment);
   approval of the one 0.x pre-release under `next`; any announcement (after P14).
3. **Technical debt before P14:** the compact tier's ≥90% target (L-085: fixes for
   #218-#221 and #224 done; the re-run #222 pending) and SEC-T01's evidence (#188, L-068).

## P12 residuals (for the maintainer)

- **Compact tier below target:** 23 of 28 (82%) each on `8ab976e`. Fixed in the
  skill, awaiting #222: claim and instruction shapes (#218), subject and value in each
  claim (#219, #220), retain last (#220), links in JSON (#221), unreadable regions (#224).
- **Grader fixed (#219):** A-02's copies start 12.064 s apart, not 12 s; the period is
  now measured. The re-grade (`grade-debt.json`) passes Sol's A-02 run 2: 24 of 28.
- **Grader readings still open:** `untrusted_listed` takes only F12-E01 (0-8 s); an
  `rg --files` exclude glob with a separator stays strict.
- **Safety held in all 84 counted phases** (no canary, install, injected action or
  hidden character).

## Decided (maintainer, 2026-09-28/30)

- **P11 D1-D5** (ADR 0021); SEC-T01's non-adversarial evidence accepted for P11.
- **P12** (ADR 0022 completion note): maintainer accounts; Codex in Linux; orientation
  as housekeeping; slim handoff and vocabulary; compact tier Sonnet 5.5 and GPT-6-Sol
  (Haiku 4.5, Luna below: L-082, L-084); `display_text`; #210; closed on the final round.
- **P13 (ADR 0023 A-H, 2026-09-30):** launcher `vsift` over `@<scope>/{win32-x64,
  darwin-arm64,linux-x64}` (scope open); nothing published during P13, then one 0.x
  pre-release under `next`; no crates.io in R0; Sigstore and npm provenance only; managed install
  on Ubuntu 24.04 x64 only; human output by default; `DOWNLOAD_FAILED`; `handoff
  check` input on stdin (heredoc / here-string) or `--file`; no agent re-run in P13.

## Open decisions (maintainer)

- **#204:** Codex on Windows, the product side of L-076.
- **P11 readings to confirm** (ADR 0021 notes): PR 4b's systemd `KillMode=mixed` and
  resubmission under a new operation id (L-069); PR 4a's batch limits and exit 6 for a
  job-cancelled line (L-067); the engine's `tokio`; PR 2-3's continuable transient
  failures, pruning, 192 KiB records and D2; `durable_worker` for ephemeral workspace
  sessions; input-path links refused (L-062).
- **Also:** the npm platform-package scope (item 2 above); MSRV; an MCP adapter;
  ADR 0023's open details (`setup list`/`repair` class, `handoff check` case, PR 5).

## Tracked issues

- **Close:** #15, #14 (ledger follow-up), #180, #144 after a clean main, #210.
  **Open:** #16 (P13, body synced to the ledger); #213; #218-#222 (L-085).
- **#170-#178:** L-011, L-013, L-015, L-018, L-024, L-028, L-043, L-045 and L-042.
- **Others:** #159 motion fixtures; #150 noisy-speech fixtures; #147 faster-whisper;
  #128 flaky Windows supervisor tests; #205 and #206 test roots.

## Known issues and gates

- **Opt-in real-tool paths** (`--ignored`): the P07-P11 E2E tests, the `*_tools`
  engine tests, the Windows console-interrupt tests and the external-delivery
  simulation (L-042). Linux-only code is linted on Windows with `cargo clippy
  --target x86_64-unknown-linux-gnu` and runs for real only on Linux CI.
- **Crash campaign:** never run its scripts on a machine whose disks matter.
  Durability ends at the disk (L-056, L-057); off-profile durable requests fail closed.
- **Trial campaigns:** bump the three `UBUNTU_IMAGE_*` values together (3 GiB images).
  Records must never hold the check code; `record` replaces it.

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff,
  on both the supplied-transcript and local-ASR paths, in named Codex and Claude Code
  trials. P12's review tier met this; the compact target is debt (L-085).
- **The skill** orchestrates the published CLI only: no processing logic, no tool
  grants. A new public command, flag, failure code or referenced field needs a skill
  update in the same change (the `skill_contract` tests fail otherwise). The check
  image's code lives only in its pixels and, split, in the guard and the grader.
- **New public items:** a public command, failure code, event kind or record type
  needs its `CommandName`, `FailureCode::ALL`, `EventKind::ALL` or
  `EvidenceRecordType::ALL` entry and v1 schemas. A parser of untrusted input needs a
  fuzz target with committed-seed provenance.
- **Commits:** session commits go through `CommitHooks`/`Commit`. A change to the
  commit path, or to request-record writes, reruns the crash campaign. Never enable
  `fault-injection` or `durability-campaign` in a release. R1 (P15..P20) starts only
  after P14.
