# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress: the maintainer started it
and confirmed decisions A-H on 2026-10-02.** The plan is 14 pull requests (0-13). **PR 0 (#250)
is merged; PR 1 (this change: the evidence ledger, the claims registry and their checks) is done
once it merges; PR 2 is next; PR 8 is being built in parallel.** The packet is not complete.
Plan: `docs/planning/p14-qualification.md`; sequence: "P14 scope and pull requests" in
`docs/planning/implementation-work-packets.md`; ADR 0024 stays Proposed until P14 completes.
**Where evidence stands:** `docs/planning/p14-evidence-ledger.json` (20 items, all `planned`, none
passed; earlier material kept as `prior`). **What public text may say:**
`docs/planning/public-claims.json` (rung `now`). The Governance job checks both; any status change
or quoted-sentence change updates them (`docs/development.md`).

**Decided 2026-10-02 (ADR 0024, each as recommended):**
- **A** R0 ships as `0.2.0` on `latest` (1.0.0 later, once real recordings have been tried);
  stable means no pre-release suffix. **B** a published `0.2.0-rc.N` under `next` (two
  planned), fixes only after the cut, never announced. **C** no signing, unless the try-outs
  show a block with no way through. **D** the 84-run agent plan: 34 with the skill, 30
  cold-agent, 8 pilots, 12 reserve, in three batches that each wait for the maintainer's go.
- **E** SEC-T01 by a maintainer-reviewed adversarial fixture, narrowing the claim as the
  fallback. **F** "supported" per cell by fixed rules; managed install stays Ubuntu-only;
  Codex on Windows documented unsupported. **G** a claims ladder checked against evidence;
  nothing is announced before P14 completes. **H** a try-out blocks the stable only until an
  observation is recorded.
- **Also:** the cold-agent variant (CLI on `PATH`, no skill, no docs) is in P14; we use the
  published CLI ourselves at the completion of R0 (raised once; nothing is installed without
  the maintainer's word); #246 (staged publishing) waits until after R1 or the announcements.

**0.1.0 today:** on npm under `next` (`latest` is the empty `0.0.0` placeholder) and as GitHub
pre-release `v0.1.0`; verified once (`npm audit signatures`, `gh attestation verify`) on the
maintainer's Windows 11 machine with npm. Not stable or supported; nothing is announced. Smart
App Control is **Off** there (read 2026-10-02), so that run says nothing about it (L-098).

## The P14 pull requests (0 is done; 1 on merge)

**1** (done on merge) evidence ledger and claims registry, with their checks; **2** clean install
from the real registry, archive, offline install, upgrade, second verifier; **3** published-binary
journeys on Ubuntu, Windows and macOS, managed-install drift run; **4** long fuzz, stress, load,
soak, malicious media, runbook walk, scan reading; **5** SEC-T01 fixture or the narrowing
amendment; **6** trial harness (clean-install, cold-agent), pilots, cold baseline against 0.1.0;
**7** fixes for what 2-6 find (maybe a `vsift --help` "typical investigation" section); **8**
release machinery for stable and `latest` (a high-risk seam; its delta check fills the ledger's
`release_delta`, L-103); **9** matrix, documents, claims enforced, register review sheet; **10**
candidate `0.2.0-rc.1`; **11** candidate qualification (workflows, counted agent rounds,
try-outs, fixes); **12** stable `0.2.0`; **13** ledger follow-up, P14 `complete`, handoff for the
R0 completion trial.

## What the maintainer owes, and when

- **Now:** review and merge PR 1. Known gap (L-102): the published v0.1.0 release notes say
  "Supported machines"; PR 8 rewords the template, the live page stays unless you edit it.
- **Before PR 6:** the go for agent batch 1 (pilots and the cold baseline, on the
  maintainer's accounts). **PR 5:** review the SEC-T01 fixture, or choose the fallback.
- **PR 9:** one pass over the thirty register entries the claims lean on, and the readings.
- **PRs 10 and 12:** each publish (tag, dry run, dispatch, approve, verify; about an hour).
- **PR 11:** the go for batches 2 and 3 (they include the blurred-banner re-run, L-095); a
  Smart App Control try-out on a fresh Windows 11 VM or another PC (Windows Sandbox: unknown);
  a macOS 15 browser-download try-out (a Mac: unknown, asked). Unknown too: free hosted minutes.

## Open decisions and readings (maintainer)

- **#204** Codex on Windows (L-076; P14 documents it as unsupported); the grader's
  `untrusted_listed` reading (F12-E01 only) and #219: both settled before the trial freeze.
- **P11 readings** (ADR 0021 notes): `KillMode=mixed` and resubmission (L-069); batch
  limits and exit 6 (L-067); the engine's `tokio`; continuable failures, pruning, 192 KiB
  records, D2; `durable_worker`; links (L-062). **P13 readings** (ADR 0023 notes; none
  blocks): L-017, `MISSING_CAPABILITY` for a failed smoke, L-088, L-090, Windows `.tar.gz`,
  exits 126/127, `attest` without an approval, Release as a required check; MSRV; MCP.

## Tracked issues and gates

- **Open:** #17 (P14), #219, #224 (L-095), #188 (SEC-T01), #232 (a root name with controls
  fails on Linux), #246 (deferred); #170-#178 (register); #159, #150 fixtures; #147; #128 and
  #206 flaky tests; #204, #205.
- **Opt-in real-tool paths** (`--ignored`, never run by CI alone) run a Cargo-built binary,
  not an installed one (L-042): the P07-P11 E2E tests, the `*_tools` engine tests, the Windows
  console-interrupt tests, P13's real downloads (dispatch-only workflows).
- **npm:** `node --test npm/test/launcher.test.cjs` when `npm/` changes; a version bump also
  bumps `npm/vsift-cli/package.json` and its three optional dependencies.
- **Releasing:** `docs/operations/release.md` section 6; only `plan`/`attest`/`publish` publish.
- **Campaigns:** never on the maintainer's machine (L-056, L-057); bump the three
  `UBUNTU_IMAGE_*` together; trial records never hold the check code.

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff on both
  transcript paths in named Codex and Claude Code trials; P14 repeats it from a clean install.
- **The skill** orchestrates the published CLI only; a new public command, flag, failure code
  or referenced field needs a skill update in the same change (`skill_contract`).
- **New public items** need their `CommandName`/`FailureCode::ALL`/`EventKind::ALL`/
  `EvidenceRecordType::ALL` entry, v1 schemas and a human renderer with a snapshot; v1 is
  additive only since 0.1.0; a parser of untrusted input needs a seeded fuzz target; no npm
  package has scripts or names a person.
- **Commits:** a change to the commit path reruns the crash campaign; never enable
  `fault-injection`, `durability-campaign` or `install-test-hooks` in a release. R1 starts
  only after P14.
