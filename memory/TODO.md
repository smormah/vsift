# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress: the maintainer started it
and confirmed decisions A-H on 2026-10-02.** The plan is 14 pull requests (0-13). **PR 0 (#250)
is merged. PR 8, the release machinery for a release candidate and the stable release (#252),
is done in this change, awaiting the maintainer's review; PR 1 (evidence ledger and claims
registry) is being built in parallel; the rest wait.** The whole packet is not complete. The
plan is `docs/planning/p14-qualification.md` (evidence RQ-01..RQ-20, traceability, budgets, the
matrix, the claims policy); the sequence is "P14 scope and pull requests" in
`docs/planning/implementation-work-packets.md`; ADR 0024 stays Proposed until P14 completes.

**Decided 2026-10-02 (ADR 0024, each as recommended):** **A** R0 ships as `0.2.0` on `latest`
(1.0.0 later); stable means no pre-release suffix. **B** a published `0.2.0-rc.N` under `next`
(two planned), fixes only after the cut, never announced. **C** no signing, unless the try-outs
show a block with no way through. **D** 84 agent runs (34 with the skill, 30 cold-agent, 8
pilots, 12 reserve) in three batches, each waiting for the maintainer's go. **E** SEC-T01 by a
maintainer-reviewed adversarial fixture, narrowing the claim as the fallback. **F** "supported"
per cell by fixed rules; managed install Ubuntu-only; Codex on Windows documented unsupported.
**G** a claims ladder checked against evidence; nothing announced before P14 completes. **H** a
try-out blocks the stable only until an observation is recorded. Also: the cold-agent variant
is in P14; we use the published CLI ourselves as a trial at R0's completion (nothing installed
without the maintainer's word); #246 (staged publishing) waits until after R1 or the announcements.

**PR 8 in one view** (ADR 0024's note; `docs/operations/release.md` 6.7-6.9): the version alone
decides the channel: a suffix means `next` (a candidate is `-rc.N`), none means stable, which
moves `latest` on all four packages. A stable plan states what moves and is guarded (candidate
delta, candidate published, `latest` forward-only; enforced on a publish and on a dispatch of
the tag even as a dry run); the publish job has a step per channel and reads the tags back; no
workflow may run `npm dist-tag`. 61 lint mutations; the shell runs against stubs in CI. **Never
run for real** (L-105); the evidence ledger is **not enforced** (L-106). **Code is frozen at
the candidate cut:** the lists in `candidate.rs`, the release notes, the workflow.

**0.1.0 today:** on npm under `next` (`latest` is the empty `0.0.0` placeholder) and as GitHub
pre-release `v0.1.0`; verified once on the maintainer's Windows 11 machine (Smart App Control
Off there, so that says nothing about it: L-098). Not stable or supported; not announced.

## The P14 pull requests (0 and 8 done, on merge)

**1** evidence ledger, claims registry; **2** clean install from the real registry, archive,
offline, upgrade, second verifier; **3** published-binary journeys on three systems; **4** long
fuzz, stress, load, soak, malicious media, runbook walk, scan reading; **5** SEC-T01 or the
narrowing; **6** trial harness, pilots, cold baseline; **7** fixes for what 2-6 find; **9**
matrix, documents, claims enforced, register sheet; **10** candidate `0.2.0-rc.1`; **11** its
qualification; **12** stable `0.2.0`; **13** ledger follow-up, P14 `complete`, handoff for the
R0 completion trial. **Before PR 10:** wire PR 1's completeness check into the stable plan
(L-106) and settle the allowed-path lists.

## What the maintainer owes, and when

- **Now:** review PR 8, a high-risk seam (ADR 0024's note first: the lists, the skill's
  exclusion, `0.1.0` stable by shape, `--tag latest` and `--latest` unexercised); close #16.
- **Before PR 6:** the go for agent batch 1. **PR 5:** the SEC-T01 fixture, or the fallback.
  **PR 9:** one pass over the thirty register entries the claims lean on, and the readings.
- **PRs 10 and 12:** each publish (`release.md` 6.3 and 6.7, the preflight first); the first
  stable publish is the first real `--tag latest`.
- **PR 11:** the go for batches 2 and 3 (they include the blurred-banner re-run, L-095); a
  Smart App Control try-out on a fresh Windows 11 VM or another PC; a macOS 15 browser-download
  try-out (a Mac owned? unknown). Also unknown: hosted-runner minutes.

## Open decisions and readings (maintainer)

- **#204** Codex on Windows (L-076; documented unsupported); the grader's `untrusted_listed`
  reading (F12-E01) and #219: both settled before the trial freeze.
- **Readings** (ADR 0021 and 0023 notes; none blocks): `KillMode=mixed` and resubmission
  (L-069); batch limits and exit 6 (L-067); the engine's `tokio`; continuable failures, pruning,
  192 KiB records, D2; `durable_worker`; links (L-062); L-017, L-088, L-090; Windows `.tar.gz`;
  exits 126/127; `attest` without an approval; Release as a required check; MSRV; MCP.

## Tracked issues and gates

- **Open:** #17 (P14), #16 (P13, to close), #219, #224 (L-095), #188 (SEC-T01), #232 (a root
  name with controls fails on Linux), #246 (deferred); #170-#178 (register); #159, #150
  fixtures; #147; #128 and #206 flaky tests; #204, #205.
- **Opt-in real-tool paths** (`--ignored`, never run by CI alone) run a Cargo-built binary,
  not an installed one (L-042): the P07-P11 E2E and `*_tools` tests, the Windows
  console-interrupt tests, P13's real downloads (dispatch-only workflows).
- **npm:** `node --test npm/test/launcher.test.cjs` when `npm/` changes; a version bump touches
  only the files `release.md` 6.3 lists. **Campaigns:** never on the maintainer's machine
  (L-056, L-057); bump the three `UBUNTU_IMAGE_*` together; trial records never hold the check code.

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff on both
  transcript paths in named Codex and Claude Code trials; P14 repeats it from a clean install.
  R1 starts only after P14.
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
