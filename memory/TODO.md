# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

**P00-P12 are complete; P13 is in progress** (started 2026-09-30, plan accepted). Merged:
PR 0 (#226), the P12 debt fixes (#227, L-085), PR 1 (#228, L-071 closed), PR 2a (#229),
PR 3 (#230, `e22ee59`, smoke and cleanup), PR 2b (#231, `02df4eb`; human output done,
L-073 closed) and PR 5 (#233, `1a9d027`, `handoff check`, L-086). **PR 4 (`setup
install` and the managed lookup tier) is in review** on `p13-pr4-install` (#234). The
packet is not complete.

1. **Next P13 PRs** (`implementation-work-packets.md` "P13 scope and pull requests"):
   6 `setup list/rollback/remove/repair`, bounded cleanup and the stale-stage sweep
   (stages PR 3 and a killed PR 4 run retain); 7 kill/power-loss, E2E; 8 `release.yml`
   (package only `vsift`); 9 npm; 10 publish wiring; 11 docs; 12 ledger. #222 is due.
2. **After PR 4 merges (maintainer):** dispatch `P13 managed smoke` on `main` (both jobs,
   no credentials); record the run in D-02/D-03/D-06/D-07 and L-037, its `L-087` timing
   lines in L-087.
3. **Maintainer-only, before PR 10's publish step:** npm account with 2FA; **choose the
   platform-package scope** (`@vsift` refused; `@vsift-cli`, `@vsifthq` or `@vsiftdev`),
   create its organisation, record it in an ADR 0009 note; trusted publishers; the
   `release` environment (maintainer as reviewer); tag ruleset; fork-PR approval; the
   first publish (2FA, or a short-lived environment token); the 0.x `next` pre-release.
4. **Technical debt before P14:** the compact tier's ≥90% target (L-085: fixes for
   #218-#221 and #224 done; the re-run #222 pending) and SEC-T01's evidence (#188, L-068).

## P12 residuals (for the maintainer)

- **Compact tier 23 of 28 (82%)** each on `8ab976e`; fixes #218-#221, #224 await #222.
  Grader: A-02's period measured (#219; Sol 24 of 28); open: `untrusted_listed` takes
  only F12-E01, an `rg --files` exclude glob stays strict. Safety held in all 84 phases.

## Decided (maintainer, 2026-09-28/30)

- **P11 D1-D5** (ADR 0021); **P12** closed on its final round (ADR 0022 completion
  note: Codex in Linux, slim handoff, compact tier Sonnet 5.5 and GPT-6-Sol).
- **P13 (ADR 0023 A-H, 2026-09-30):** launcher `vsift` over `@<scope>/{win32-x64,
  darwin-arm64,linux-x64}` (scope open); nothing published during P13, then one 0.x
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
  (L-017); the `\\?\` path note (`Copy-Item -LiteralPath`, or a root of ≤125 characters).
- **P13 PR 4 readings** (ADR 0023 note): digested plan intent vs observed state
  (`install_needed`, action `state`); debug builds resolve no publisher host; failed
  smoke `MISSING_CAPABILITY`; `BUSY` retry 30 s; `407` by text (L-088); rehash (L-087).
- **P13 PR 5 readings:** schema `handoff-check-data` (repo naming), a `line` beside
  each pointer, skill forms without `--session`.
- **Also:** the npm platform-package scope (item 3 above); MSRV; an MCP adapter;
  ADR 0023's open detail (`setup list`/`repair` class).

## Tracked issues

- **Close:** #15, #14 (ledger), #180, #144 after a clean main, #210, #213 (PR 5 merged).
  **Open:** #16, #218-#222, #232 (a session root name with controls fails `RootUnavailable` on Linux).
- **#170-#178:** L-011, L-013, L-015, L-018, L-024, L-028, L-043, L-045 and L-042.
- **Others:** #159, #150 fixtures; #147 faster-whisper; #128 flaky supervisor tests; #205, #206.

## Known issues and gates

- **Opt-in real-tool paths** (`--ignored`): the P07-P11 E2E tests, the `*_tools`
  engine tests, the Windows console-interrupt tests, the external-delivery simulation
  (L-042) and the P13 managed smoke and real install (`P13 managed smoke`, Ubuntu).
  Linux-only code is linted in CI (locally `--target x86_64-unknown-linux-gnu` needs
  OpenSSL for Linux). The smoke tests' fake tools are the `vsift-smoke-fixture` bin;
  the install tests need `install-test-hooks` (a workspace run enables it).
- **Campaigns:** crash campaign never on disks that matter (L-056, L-057); bump the three
  `UBUNTU_IMAGE_*` together; trial records never hold the check code.

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
  `fault-injection`, `durability-campaign` or `install-test-hooks` in a release. R1
  (P15..P20) starts only after P14.
