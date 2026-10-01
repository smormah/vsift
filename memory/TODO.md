# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P12 are complete. P13 is not.** PRs 0-11 are done: 0-10 merged (#226, #228-#231, #233,
#234, #236, #239-#241, #243, #244 `6de55da`; P12 debt #227, #238, #242); PR 11 (#245) is the user guide
`docs/operations/install.md`, the record `docs/planning/p13-distribution.md` and the closing
sweep. **Nothing is published** except the `vsift-cli@0.0.0` placeholder; the attest and
publish jobs have never run (L-096). The power-loss campaign passed on `main` (run 36829198545);
`P13 managed smoke` with `install-e2e` passed (run 36793180858). The ledger still says
`in_progress`: only PR 12 may change it, after a real publish.

**What remains, in order:**

1. **The maintainer's release steps** (`docs/operations/release.md` section 6; read `install.md`
   first). 6.2: fork-PR approval "all external contributors"; environment `release` (you as
   reviewer, self-review allowed, no admin bypass, tag rule `v*`); tag ruleset `v*`; first
   publish of the three `@vsift/...` names (path A placeholders with 2FA, an ADR 0009 note
   needed; or path B, short-lived `NPM_BOOTSTRAP_TOKEN`); trusted publishers on all four
   (`smormah`, `vsift`, `release.yml`, `release`, "npm publish" ticked); disallow tokens.
   6.3: version, CHANGELOG release section, tag `v0.1.0`, dry run, dispatch with `dry_run`
   cleared, approve. 6.4: verify (`npm audit signatures`, `gh attestation verify`).
   State read 2026-10-01: none of 6.2 is done; the three `@vsift/...` are not on the registry.
2. **The publish** (`vsift-cli@next`; `latest` stays `0.0.0`).
3. **PR 12** (small, after the publish): the record's "First publish" section; ADR 0023
   Accepted; ledger P13 `complete` with PR 11's merge commit; L-036 and L-096 closed or
   rewritten; the register's packet-close sweep. Then P14 (not started; governance rule 10).

**Debt before P14:** A-09 blurred, review tier (L-095, #224; the maintainer runs it); SEC-T01
(#188, L-068); L-098's try-out on Windows 11 with Smart App Control On and a macOS 15
browser download. Open: grader reading `untrusted_listed` (F12-E01 only); #219.

## Decided (maintainer, 2026-09-28/30)

- **P11 D1-D5** (ADR 0021); **P12** closed on its final round (ADR 0022 note); the compact
  re-run #222 met the target (Sonnet 26/28, Sol 28/28; L-085 closed).
- **P13 (ADR 0023 A-H, 2026-09-30):** launcher `vsift-cli` (npm refused `vsift`; command
  `vsift`) over `@vsift/{win32-x64,darwin-arm64,linux-x64}`; only the `0.0.0` placeholder in
  P13, then one 0.x pre-release under `next`; no crates.io; Sigstore and npm provenance only;
  managed install on Ubuntu 24.04 x64 only; human output by default; `DOWNLOAD_FAILED`;
  `handoff check` input on stdin or `--file`; no agent re-run in P13.

## Open decisions (maintainer)

- **#204:** Codex on Windows, the product side of L-076.
- **P11 readings** (ADR 0021 notes): `KillMode=mixed` and resubmission (L-069); batch limits and
  exit 6 for a job-cancelled line (L-067); the engine's `tokio`; continuable failures, pruning,
  192 KiB records, D2; `durable_worker` for ephemeral workspaces; input-path links (L-062).
- **P13 readings** (each in its ADR 0023 note; none blocks completion): PR 2b worker hosts
  render only their result (L-017), the `\\?\` note; PR 4 plan intent vs observed state,
  failed smoke `MISSING_CAPABILITY`, `407` by text (L-088), rehash (L-087); PR 5 schema
  `handoff-check-data`; PR 6 removal proves ownership, `list`/`repair` `free`, L-090; PR 7
  directory flushes, an empty store folder adopted; PR 8 Windows `.tar.gz`, 12-digit commit,
  L-089; PR 9 exits 126/127, signal rules (L-091), no SBOM in platform packages, matrix
  coverage (L-092); PR 10 path A or B, `attest` without an approval, npm staged publishing
  (not wired), release immutability, release notes (their npm-avoids-prompts sentence may
  not hold for Smart App Control, L-098), `SHA256SUMS` lists archives only, Release as a
  required check (main requires Quality, Documentation, dependency policy and review, Rust
  analysis, Governance). **Also:** MSRV; an MCP adapter.

## Tracked issues

- **Close:** #213 (delivered by PR 5; still open on GitHub, read 2026-10-01). **Open:** #16,
  #219, #224, #232 (a session root name with controls fails `RootUnavailable` on Linux);
  #170-#178 (L-011, L-013, L-015, L-018, L-024, L-028, L-043, L-045, L-042); #159, #150
  fixtures; #147 faster-whisper; #128 flaky supervisor tests; #205, #206.

## Known issues and gates

- **Opt-in real-tool paths** (`--ignored`, never run by CI alone): the P07-P11 E2E tests, the
  `*_tools` engine tests, the Windows console-interrupt tests, the external-delivery
  simulation (L-042) and P13's real downloads (`P13 managed smoke`, `P13 managed power loss`,
  dispatch only). The install and kill tests need `install-test-hooks` and `fault-injection`.
- **npm:** `node --test npm/test/launcher.test.cjs` when `npm/` changes; a workspace
  version bump also bumps `npm/vsift-cli/package.json` and its three optional dependencies.
- **Campaigns:** crash campaign (also `--store managed`) never on disks that matter
  (L-056, L-057); bump the three `UBUNTU_IMAGE_*` together; trial records never hold the
  check code. **Release:** only `plan`/`attest`/`publish` publish (lint rule 7).

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff,
  on both the supplied-transcript and local-ASR paths, in named Codex and Claude Code
  trials. P12 qualified both tiers; a run from a clean install is P14's (decision H10).
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
