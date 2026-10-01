# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P12 are complete; P13 is in progress.** Merged: PRs 0-10 (#226-#231, #233, #234,
#236, #239-#241, #243 `57f03fe`; P12 debt #227, #242); PR 10 wired `dry_run`, `plan`,
`attest`/`publish`, nothing published (L-096, L-097). PR 7: `install-e2e` passed (run
36793180858); `P13 managed power loss` (run 36793177930) found no damage but 53 "lost"
acks, each the in-flight command's newer selection, a verifier defect (ADR 0023 PR 7
addendum). **In review:** its fix, `p13-pr7-durability` (start marks, `managed::in_flight`);
then the maintainer re-dispatches the run on `main` (rule unchanged). Packet not complete.

1. **Next P13 PRs:** 11 docs (`install.md`'s verification walk-through,
   SmartScreen/Gatekeeper; the record `p13-distribution.md`, with the first publish's
   evidence); 12 ledger.
2. **Maintainer-only, in order** (release.md 6.2-6.4; `vsift-cli@0.0.0` stays `latest`):
   PR 7's power-loss re-run; fork-PR approval "all external contributors"; environment `release`
   (you as reviewer, self-review allowed, no admin bypass, tag rule `v*`); tag ruleset
   `v*`; first publish of the three `@vsift/…` names (path A placeholders with 2FA, or
   path B short-lived `NPM_BOOTSTRAP_TOKEN`); trusted publishers on all four (`smormah`,
   `vsift`, `release.yml`, `release`, "npm publish" ticked); disallow tokens; tag
   `v0.1.0`, dry run, dispatch with `dry_run` cleared, approve, verify.
3. **Debt before P14:** A-09 blurred, review tier (L-095, #224; maintainer runs it); SEC-T01
   (#188, L-068). #222 re-run met the compact target (Sonnet 26/28, Sol 28/28; L-085 closed).
   Open: grader reading `untrusted_listed` (F12-E01 only); #219.

## Decided (maintainer, 2026-09-28/30)

- **P11 D1-D5** (ADR 0021); **P12** closed on its final round (ADR 0022 note).
- **P13 (ADR 0023 A-H, 2026-09-30):** launcher `vsift-cli` (npm refused `vsift`; command `vsift`) over `@vsift/{win32-x64,
  darwin-arm64,linux-x64}`; only the `vsift-cli@0.0.0` placeholder in P13, then one 0.x
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
- **PR 7** (ADR 0023 note, reviewed): directory flushes; empty store folder adopted.
- **PR 9** (ADR 0023 note): exits 126/127; the signal rules (L-091); no SBOM in the
  platform packages; twelve more Release jobs per archive/npm PR; Yarn through a project
  install; only the minimum runtimes in the matrix (L-092).
- **PR 10** (release.md 6.6): path A (three more placeholders, an ADR 0009 note) or B;
  `attest` without an approval; npm staged publishing (not wired); release immutability;
  release notes; `SHA256SUMS` lists archives only. **Also:** MSRV; an MCP adapter.

## Tracked issues

- **Close:** #15, #14 (ledger), #180, #144 after a clean main, #210, #213. **Open:** #16,
  #219, #224, #232 (a session root name with controls fails `RootUnavailable` on Linux);
  #170-#178 (L-011, L-013, L-015, L-018, L-024, L-028, L-043, L-045, L-042); #159, #150
  fixtures; #147 faster-whisper; #128 flaky supervisor tests; #205, #206.

## Known issues and gates

- **Opt-in real-tool paths** (`--ignored`): the P07-P11 E2E tests, the `*_tools` engine
  tests, the Windows console-interrupt tests, the external-delivery simulation (L-042) and
  the P13 managed smoke, real install and E2E stage (Ubuntu). Linux-only code is linted
  in CI. The install and kill tests need `install-test-hooks` and `fault-injection`.
- **npm:** `node --test npm/test/launcher.test.cjs` when `npm/` changes; a workspace
  version bump also bumps `npm/vsift-cli/package.json` and its three optional dependencies.
- **Campaigns:** crash campaign (also `--store managed`) never on disks that matter
  (L-056, L-057); bump the three `UBUNTU_IMAGE_*` together; trial records never hold the
  check code. **Release:** only `plan`/`attest`/`publish` publish (lint rule 7).

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff,
  on both the supplied-transcript and local-ASR paths, in named Codex and Claude Code
  trials. P12's review tier met this; the compact tier met its target on the re-run (#222).
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
