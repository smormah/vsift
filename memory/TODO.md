# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P12 are complete; P13 is in progress** (started 2026-09-30, plan accepted). Merged:
PR 0 (#226), P12 debt (#227), PR 1 (#228, L-071 closed), PR 2a (#229), PR 3 (#230,
`e22ee59`, smoke and cleanup) and PR 2b (#231, `02df4eb`, human output done, L-073
closed). **PR 5 (`handoff check`, #213) is in review**; PR 4 is in progress. The packet
is not complete.

1. **Next P13 PRs** (`implementation-work-packets.md` "P13 scope and pull requests"):
   4 install (transaction over PR 3's `smoke_before_activation`, then
   `publish_and_select`; decides what `setup install` reports of the smoke's typed
   failures); 6 lifecycle and stale-stage sweep (must handle stages PR 3 retains); 7
   kill/power-loss, E2E; 8 `release.yml` (package only `vsift`); 9 npm; 10 publish
   wiring; 11 docs; 12 ledger. #222 re-runs once PR 5 is merged (the maintainer
   starts it; no model was called).
2. **Maintainer-only, before PR 10's publish step:** scope `@shongo` chosen and its
   organisation created (2026-09-30); the placeholder `vsift@0.0.0` (ADR 0009 note,
   published by the maintainer); trusted publishers; the
   `release` environment (maintainer as reviewer); tag ruleset; fork-PR approval; the
   first publish (2FA, or a short-lived environment token); the 0.x `next` pre-release.
3. **Technical debt before P14:** the compact tier's ≥90% target (L-085: fixes for
   #218-#221 and #224 done; the re-run #222 pending) and SEC-T01's evidence (#188, L-068).

## P12 residuals (for the maintainer)

- **Compact tier below target:** 23 of 28 (82%) each on `8ab976e`. Fixed in the
  skill, awaiting #222: claim and instruction shapes (#218), subject and value in each
  claim (#219, #220), retain last (#220), links in JSON (#221), unreadable regions (#224).
- **Grader:** A-02's period is measured (#219; Sol re-grades to 24 of 28). Open readings:
  `untrusted_listed` takes only F12-E01; an `rg --files` exclude glob with `/` is strict.
- **Safety held in all 84 counted phases** (no canary, install, injection, hidden char).

## Decided (maintainer, 2026-09-28/30)

- **P11 D1-D5** (ADR 0021); SEC-T01's non-adversarial evidence accepted for P11.
- **P12** (ADR 0022 completion note): maintainer accounts; Codex in Linux; orientation
  as housekeeping; slim handoff and vocabulary; compact tier Sonnet 5.5 and GPT-6-Sol
  (Haiku 4.5, Luna below: L-082, L-084); `display_text`; #210; closed on the final round.
- **P13 (ADR 0023 A-H, 2026-09-30):** launcher `vsift` over `@shongo/vsift-{win32-x64,
  darwin-arm64,linux-x64}`; only the `vsift@0.0.0` placeholder during P13, then one 0.x
  pre-release under `next`; no crates.io in R0; Sigstore and npm provenance only; managed
  install on Ubuntu 24.04 x64 only; human output by default; `DOWNLOAD_FAILED`; `handoff
  check` input on stdin (heredoc / here-string) or `--file`; no agent re-run in P13.

## Open decisions (maintainer)

- **#204:** Codex on Windows, the product side of L-076.
- **P11 readings to confirm** (ADR 0021 notes): PR 4b's systemd `KillMode=mixed` and
  resubmission under a new operation id (L-069); PR 4a's batch limits and exit 6 for a
  job-cancelled line (L-067); the engine's `tokio`; PR 2-3's continuable transient
  failures, pruning, 192 KiB records and D2; `durable_worker` for ephemeral workspace
  sessions; input-path links refused (L-062).
- **P13 PR 2b readings** (ADR 0023 note): worker hosts render only their final result
  in human mode, events stay JSON Lines (L-017 now that residual); the `\\?\` path note
  (PowerShell `Copy-Item -LiteralPath`, or a root of at most 125 characters).
- **Also:** MSRV; an MCP adapter;
  `setup list`/`repair` class (ADR 0023). PR 5 readings: schema `handoff-check-data`
  (repo naming), a `line` beside each pointer, skill forms without `--session`.

## Tracked issues

- **Close:** #15, #14 (ledger), #180, #144 after a clean main, #210; #213 with PR 5. **Open:** #16, #218-#222.
- **#170-#178:** L-011, L-013, L-015, L-018, L-024, L-028, L-043, L-045 and L-042.
- **Others:** #159, #150 fixtures; #147 faster-whisper; #128 flaky supervisor tests; #205, #206.

## Known issues and gates

- **Opt-in real-tool paths** (`--ignored`): the P07-P11 E2E tests, the `*_tools`
  engine tests, the Windows console-interrupt tests, the external-delivery simulation
  (L-042) and the P13 managed smoke (`P13 managed smoke` workflow, Ubuntu only).
  Linux-only code is linted in CI (locally `--target x86_64-unknown-linux-gnu` needs
  OpenSSL for Linux). The smoke tests' fake tools are the `vsift-smoke-fixture` bin.
- **Crash campaign:** never on a machine whose disks matter (L-056, L-057).
- **Trial campaigns:** bump the three `UBUNTU_IMAGE_*` values together; records never
  hold the check code (`record` replaces it).

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff,
  on both the supplied-transcript and local-ASR paths, in named Codex and Claude Code
  trials. P12's review tier met this; the compact target is debt (L-085).
- **The skill** orchestrates the published CLI only: no processing logic, no tool
  grants. A new public command, flag, failure code or referenced field needs a skill
  update in the same change (`skill_contract` fails otherwise); its one input exception is
  the two `handoff check` forms. The check code lives only in its pixels, guard and grader.
- **New public items:** a public command, failure code, event kind or record type
  needs its `CommandName`, `FailureCode::ALL`, `EventKind::ALL` or
  `EvidenceRecordType::ALL` entry, v1 schemas and, for a command that completes, a
  renderer in `crates/vsift-cli/src/human/` (`render_value`'s match is exhaustive) with
  a snapshot. A parser of untrusted input needs a seeded fuzz target.
- **Commits:** session commits go through `CommitHooks`/`Commit`. A change to the
  commit path, or to request-record writes, reruns the crash campaign. Never enable
  `fault-injection` or `durability-campaign` in a release. R1 (P15..P20) starts only
  after P14.
