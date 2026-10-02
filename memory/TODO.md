# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress: the maintainer started it
and confirmed decisions A-H on 2026-10-02.** The plan is 14 pull requests (0-13). **PRs 0
(#250), 1 (#251, the ledger and claims registry) and 8 (#252, the release machinery) are
merged. PR 2 (#255, the published-artifact qualification, run on 0.1.0) is done once it
merges; PR 3 is next.** The whole packet is not complete. Plan:
`docs/planning/p14-qualification.md` (section 15: PR 2's record); ADR 0024 stays Proposed.
**Evidence:** `docs/planning/p14-evidence-ledger.json`: RQ-01-RQ-04 and RQ-19 `passed` **for
0.1.0 only**, 15 `planned`. **Public text:** `docs/planning/public-claims.json` (rung `now`;
scans `release.md` and the release-notes templates too). The Governance job checks both.

**Decided 2026-10-02 (ADR 0024, each as recommended):** **A** R0 ships as `0.2.0` on `latest`
(1.0.0 later); stable means no pre-release suffix. **B** a published `0.2.0-rc.N` under `next`
(two planned), fixes only after the cut, never announced. **C** no signing, unless the try-outs
show a block with no way through. **D** 84 agent runs (34 with the skill, 30 cold-agent, 8
pilots, 12 reserve) in three batches, each waiting for the maintainer's go. **E** SEC-T01 by a
maintainer-reviewed adversarial fixture, narrowing the claim as the fallback. **F** "supported"
per cell by fixed rules; managed install Ubuntu-only; Codex on Windows documented unsupported.
**G** a claims ladder checked against evidence; nothing announced before P14 completes. **H** a
try-out blocks the stable only until an observation is recorded. Also: the cold-agent variant
is in P14; we try the published CLI ourselves at R0's completion (nothing installed without the
maintainer's word); #246 (staged publishing) waits until after R1 or the announcements.

**PR 8 in one view** (ADR 0024's note; `docs/operations/release.md` 6.7-6.9): the version alone
decides the channel: a suffix means `next` (a candidate is `-rc.N`), none means stable, which
moves `latest` on all four packages. A stable plan is guarded (candidate delta, candidate
published, `latest` forward-only, ledger complete for the candidate) and writes
`release-delta.json` for the ledger (copied by hand, L-103). No workflow may run `npm dist-tag`.
**Never run for real** (L-105). **Code is frozen at the candidate cut:** the lists in
`candidate.rs`, the release notes (Markdown templates, claims-checked), the workflow.

**0.1.0 today:** on npm under `next` (`latest` is the empty `0.0.0` placeholder) and as GitHub
pre-release `v0.1.0`; verified once on the maintainer's Windows 11 machine (Smart App Control
Off there: L-098). Not stable or supported; not announced.

## The P14 pull requests (0, 1, 8 done; 2 on merge)

**2** (done on merge) clean install from the real registry, archive, offline, upgrade, 0.1.0
compatibility, second verifier; **3** published-binary journeys on three systems; **4** long fuzz,
stress, load, soak, malicious media, runbook walk, scan reading; **5** SEC-T01 or the narrowing;
**6** trial harness, pilots, cold baseline; **7** fixes for what 2-6 find (#256, #257); **9**
matrix, documents, claims enforced, register sheet; **10** candidate `0.2.0-rc.1`; **11** its
qualification; **12** stable `0.2.0`; **13** ledger follow-up (the delta record, L-102's page), P14
`complete`, handoff for the R0 trial. **Before PR 10:** settle the allowed-path lists in
`candidate.rs`. **Before PR 12:** register the two stable checks in `STABLE_CHECKS`
(`tools/p14-published/lib/verify.cjs`); `P14 verify release` fails a stable version until then.

## What the maintainer owes, and when

- **Now:** review and merge PR 2; read its findings (#256 `libgomp1`, #257 Windows `.cmd` shim;
  documented, PR 7 decides). Optional (L-102): edit the v0.1.0 release page ("Supported machines").
- **PR 5:** the SEC-T01 fixture, or the fallback. **Before PR 6:** the go for agent batch 1.
  **PR 9:** one pass over the thirty register entries the claims lean on, and the readings.
- **PRs 10 and 12:** each publish (`release.md` 6.3 and 6.7, the preflight first); the first
  stable publish is the first real `--tag latest`.
- **PR 11:** the go for batches 2 and 3 (they include the blurred-banner re-run, L-095); a
  Smart App Control try-out on a fresh Windows 11 VM or another PC; a macOS 15 browser-download
  try-out (a Mac owned? unknown).

## Open decisions and readings (maintainer)

- **#204** Codex on Windows (L-076; documented unsupported); the grader's `untrusted_listed`
  reading (F12-E01) and #219: both settled before the trial freeze.
- **Readings** (ADR 0021 and 0023 notes; none blocks): `KillMode=mixed` (L-069); batch limits
  and exit 6 (L-067); the engine's `tokio`; continuable failures, pruning, 192 KiB records, D2;
  `durable_worker`; links (L-062); L-017, L-088, L-090; Windows `.tar.gz`; exits 126/127;
  `attest` without an approval; Release as a required check; MSRV; MCP.

## Tracked issues and gates

- **Open:** #17 (P14), #219, #224 (L-095), #188 (SEC-T01), #232 (a root name with controls
  fails on Linux), #246 (deferred), #256, #257 (PR 2 findings); #170-#178 (register); #159, #150
  fixtures; #147; #128 and #206 flaky tests; #204, #205.
- **Opt-in real-tool paths** (`--ignored`) run a Cargo-built binary, not an installed one
  (L-042): the P07-P11 E2E and `*_tools` tests, P13's real downloads.
- **npm:** `node --test npm/test/launcher.test.cjs` when `npm/` changes (a version bump: the
  files `release.md` 6.3 lists). **P14 tools:** tests in `tools/p14-published/test/`, workflows on
  hosted runners only. **Campaigns:** never on the maintainer's machine (L-056, L-057); bump the
  three `UBUNTU_IMAGE_*` together; trial records never hold the check code.

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff on both
  transcript paths in named trials; P14 repeats it from a clean install. R1 starts after P14.
- **The skill** orchestrates the published CLI only; a new public command, flag, failure code
  or referenced field needs a skill update in the same change (`skill_contract`).
- **New public items** need their `CommandName`/`FailureCode::ALL`/`EventKind::ALL`/
  `EvidenceRecordType::ALL` entry, v1 schemas and a human renderer with a snapshot; v1 is
  additive only since 0.1.0; a parser of untrusted input needs a seeded fuzz target; no npm
  package has scripts or names a person.
- **The release seam:** a change to `vsift-release`, `release.yml` or its lint needs a lint
  rule, a broken-copy mutation test and the shell check (`tools/vsift-release/tests/
  publish-steps.sh`); no workflow runs `npm dist-tag`; no test hard-codes the version.
- **Commits:** a change to the commit path reruns the crash campaign; never enable
  `fault-injection`, `durability-campaign` or `install-test-hooks` in a release.
