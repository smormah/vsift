# VSift work record

Current-state handoff, rewritten in every change within the governance size limit.
History lives in git, `CHANGELOG.md`, the qualification records and `docs/history/`.

## Now

**P00-P12 are complete; P13 is in progress** (started 2026-09-30, plan accepted). Merged:
PR 0 (#226), the P12 debt fixes (#227, L-085), PR 1 (#228, L-071 closed), PRs 2a and 2b
(#229, #231; human output done, L-073 closed), PR 3 (#230), PR 4 (#234, `d43a518`,
`setup install`), PR 5 (#233, `handoff check`, L-086), PR 8 (#236, `772ead2`,
`release.yml`, the workflow lint) and PR 6 (#239, `02a8f76`, `setup list/rollback/
remove/repair`, cleanup and the stale-stage sweep, L-090). **PR 9 (npm packages and the
Verdaccio matrix) is done** on `p13-pr9-npm`: the `vsift` launcher and three `@vsift/…`
packages from the archives, qualified with npm, pnpm, Yarn and Bun on three OSes
(L-091 to L-094); PR 7 runs in parallel. The packet is not complete.

1. **Next P13 PRs** (`implementation-work-packets.md` "P13 scope and pull requests"):
   7 kill/power-loss tests of the managed store and the install E2E stage (in progress);
   10 `attest`/`publish` jobs (the lint's only `id-token` jobs), npm provenance and
   `dry_run`, publishing exactly the tarballs `npm-package` builds after `npm-qualify`;
   11 docs (`install.md`'s archive verification, SmartScreen/Gatekeeper; the record
   `p13-distribution.md`); 12 ledger. #222 is due.
2. **Maintainer-only, before PR 10's publish step:** the placeholder `vsift@0.0.0` (ADR
   0009 note); trusted publishers for `vsift` and the three `@vsift/…` packages; the
   `release` environment (maintainer as reviewer); tag ruleset; fork-PR approval; the
   first publish (2FA, or a short-lived environment token); the 0.x `next` pre-release.
3. **Technical debt before P14:** the compact tier's ≥90% target (L-085: 23 of 28, 82%,
   on `8ab976e`; fixes #218-#221 and #224 done; the re-run #222 pending; grader open
   points `untrusted_listed` and an `rg --files` exclude glob) and SEC-T01 (#188, L-068).

## Decided (maintainer, 2026-09-28/30)

- **P11 D1-D5** (ADR 0021); **P12** closed on its final round (ADR 0022 note).
- **P13 (ADR 0023 A-H, 2026-09-30):** launcher `vsift` over `@vsift/{win32-x64,
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
- **P13 PR 2b readings:** worker hosts render only their final result (L-017); the `\\?\`
  path note (`Copy-Item -LiteralPath`, or a root of ≤125 characters).
- **P13 PR 4 readings** (ADR 0023 note): digested plan intent vs observed state
  (`install_needed`, action `state`); debug builds resolve no publisher host; failed
  smoke `MISSING_CAPABILITY`; `BUSY` retry 30 s; `407` by text (L-088); rehash (L-087).
- **PR 5:** schema `handoff-check-data`, a `line` per pointer, skill forms without
  `--session`. **PR 8:** Windows `.tar.gz`, 12-digit commit, two lint rules, L-089;
  Release (now also the npm matrix) a required check?
- **PR 6** (ADR 0023 note): removal proves ownership, not integrity; `setup list`/`repair`
  `free`; `repair` drops `--profile`; pointer v2; sweep in every accepted install; L-090.
- **PR 9** (ADR 0023 note): launcher exits 126/127; the signal rules (POSIX relays
  `SIGINT` only when no standard stream is a terminal; Windows relays nothing, L-091);
  no SBOM in the platform packages; twelve more Release jobs per archive/npm PR; Yarn
  through a project install; only the minimum runtimes in the matrix (L-092).
- **Also:** MSRV; an MCP adapter.

## Tracked issues

- **Close:** #15, #14 (ledger), #180, #144 after a clean main, #210, #213. **Open:** #16,
  #218-#222, #232 (a session root name with controls fails `RootUnavailable` on Linux);
  #170-#178 (L-011, L-013, L-015, L-018, L-024, L-028, L-043, L-045, L-042); #159, #150
  fixtures; #147 faster-whisper; #128 flaky supervisor tests; #205, #206.

## Known issues and gates

- **Opt-in real-tool paths** (`--ignored`): the P07-P11 E2E tests, the `*_tools` engine
  tests, the Windows console-interrupt tests, the external-delivery simulation (L-042) and
  the P13 managed smoke and real install (Ubuntu). Linux-only code is linted in CI. The
  install tests need `install-test-hooks` (a workspace run enables it).
- **npm:** `node --test npm/test/launcher.test.cjs` when `npm/` changes; a workspace
  version bump also bumps `npm/vsift/package.json` and its three optional dependencies.
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
  renderer in `crates/vsift-cli/src/human/` with a snapshot. A parser of untrusted input
  needs a seeded fuzz target. No npm package has scripts or names a person.
- **Commits:** session commits go through `CommitHooks`/`Commit`. A change to the
  commit path, or to request-record writes, reruns the crash campaign. Never enable
  `fault-injection`, `durability-campaign` or `install-test-hooks` in a release (the
  workflow lint fails a `release.yml` that selects any). R1 starts only after P14.
