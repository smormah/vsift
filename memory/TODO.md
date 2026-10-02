# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress: decisions A-H confirmed
2026-10-02.** The plan is 14 pull requests (0-13). **PRs 0 (#250), 1 (#251), 2 (#255, the
published-artifact qualification, run on 0.1.0) and 8 (#252, the release machinery) are merged.
PR 6, the agent-trial harness for the clean-install and cold-agent rounds, is done in this
change, awaiting review; PR 3 is being built in parallel.** The whole packet is not complete.
Plan: `docs/planning/p14-qualification.md` (section 15: PR 2's record); ADR 0024 stays Proposed.
**Evidence:** `docs/planning/p14-evidence-ledger.json`: RQ-01-RQ-04 and RQ-19 `passed` **for
0.1.0 only**, 15 `planned`. **Public text:** `docs/planning/public-claims.json` (rung `now`).

**Decided 2026-10-02 (ADR 0024, each as recommended):** **A** R0 ships as `0.2.0` on `latest`;
stable means no pre-release suffix. **B** a published `0.2.0-rc.N` under `next`, fixes only
after the cut, never announced. **C** no signing, unless try-outs show a block. **D** 84 agent
runs (34 skill, 30 cold, 8 pilots, 12 reserve) in three batches, each waiting for the maintainer's
go. **E** SEC-T01 by a reviewed adversarial fixture, narrowing as the fallback. **F** "supported"
per cell by fixed rules; managed install Ubuntu-only; Codex on Windows unsupported. **G** a claims
ladder checked against evidence; nothing announced before P14 completes. **H** a try-out blocks
the stable only until an observation is recorded. #246 (staged publishing) waits until after R1.

**PR 6 in one view** (ADR 0024's note; `docs/agents/trials.md`, "The P14 batches"): **nothing was
run, no model called.** `vsift-agent-trials install` installs `vsift-cli@<exact version>` from the
real registry into a fresh prefix; every trial record carries the proof (npm's fetch equals the
registry's integrity; the launcher's digest check; the version line). `prepare` takes the skill
from the package and plays the user (`setup configure`; `setup plan` and `install` on Ubuntu).
Cold mode: `C-01..C-03` (`cold/`), a workspace proved cold, safety as a hard gate, usefulness
apart, a gap report. Hold-outs `H-01`, `H-02` (frozen index); `freeze`; usage capture;
`campaign`, `summarize`, `run-campaign.ps1`; Codex targets `*-published`, with `libgomp1` (#256).
The Windows client reaches `vsift` only through Git Bash, never `cmd.exe` (#257); the grade
records the shell. Weak points: L-117..L-120. **Batch 1** (8 pilots, 12-run cold baseline, on
0.1.0) waits for the go.

**0.1.0 today:** on npm under `next` (`latest` is the empty `0.0.0` placeholder) and a GitHub
pre-release; not announced. The stable path (PR 8) has never run for real (L-105).

## The P14 pull requests (0, 1, 2, 8 merged; 6 in review)

**3** published-binary journeys on three systems; **4** long fuzz, stress, load, soak, malicious
media, runbook walk; **5** SEC-T01 or the narrowing; **6** trial harness; **7** fixes for what
2-6 find (#256, #257; the `vsift --help` section if the cold baseline shows gaps); **9** matrix,
documents, claims enforced, register sheet; **10** candidate `0.2.0-rc.1`; **11** its
qualification (batches 2 and 3); **12** stable `0.2.0`; **13** ledger follow-up, P14 `complete`,
handoff for the R0 trial. **Before PR 10:** settle the allowed-path lists in `candidate.rs`;
freeze the skill, grader, scenarios and settings (`freeze write`). **Before PR 12:** register
the two stable checks in `STABLE_CHECKS` (`tools/p14-published/lib/verify.cjs`).

## What the maintainer owes, and when

- **Now:** review PR 6 (`trials.md` first; the cold grader's strictness, ADR 0024's "decisions
  taken inside this ADR"). **The go for batch 1**, after PR 6 merges: edit a copy of
  `campaigns/campaign.example.json`, then `run-campaign.ps1 -Batch 1 -Client claude|codex
  -Version 0.1.0 -MaxRuns 4` (the pilots), read them, then the rest; needs Claude Code 2.1.284
  and Docker; cost unknown until the pilots record tokens. Optional (L-102): edit the v0.1.0
  release page ("Supported machines").
- **PR 5:** the SEC-T01 fixture, or the fallback. **PR 9:** one pass over the thirty register
  entries the claims lean on. **PRs 10 and 12:** each publish (`release.md` 6.3 and 6.7); the
  first stable publish is the first real `--tag latest`.
- **PR 11:** the go for batches 2 and 3 (they include the blurred-banner re-run, L-095); a Smart
  App Control try-out (fresh Windows 11 VM); a macOS 15 browser-download try-out.

## Open decisions and readings (maintainer)

- **#204** Codex on Windows (L-076; documented unsupported); the grader's `untrusted_listed`
  reading (F12-E01) and #219: both settled before the trial freeze.
- **Readings** (ADR 0021 and 0023 notes; none blocks): `KillMode=mixed` (L-069); batch limits
  and exit 6 (L-067); the engine's `tokio`; continuable failures, pruning, 192 KiB records;
  `durable_worker`; links (L-062); L-017, L-088, L-090; exits 126/127; MSRV.

## Tracked issues and gates

- **Open:** #17 (P14), #219, #224 (L-095), #188 (SEC-T01), #232 (root name with controls fails
  on Linux), #246 (deferred), #256, #257 (PR 7); #170-#178 (register); #159, #150, #147; #128,
  #206 (flaky); #204. (#205 is closed by PR 6: already fixed by #203.)
- **Opt-in real-tool paths** (`--ignored`) run a Cargo-built binary (L-042); the harness's
  `install_npm` runs the real npm against a loopback registry. **npm:** `node --test
  npm/test/launcher.test.cjs` when `npm/` changes. **P14 tools:** `tools/p14-published/test/`,
  workflows on hosted runners only. **Campaigns:** never on the maintainer's machine (L-056,
  L-057) except the agent batches; bump the three `UBUNTU_IMAGE_*` together.

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff on both
  transcript paths in named trials; P14 repeats it from a clean install.
- **The skill** calls the published CLI only; a new public command, flag, failure code or
  referenced field needs a skill update in the same change (`skill_contract`).
- **New public items** need their `CommandName`/`FailureCode::ALL`/`EventKind::ALL`/
  `EvidenceRecordType::ALL` entry, v1 schemas and a human renderer with a snapshot; v1 is
  additive only; a parser of untrusted input needs a fuzz target; npm packages have no scripts.
- **The release seam:** a change to `vsift-release`, `release.yml` or its lint needs a lint
  rule, a broken-copy mutation test and the shell check (`tools/vsift-release/tests/
  publish-steps.sh`); no workflow runs `npm dist-tag`; no test hard-codes the version.
- **Trials:** a change to the skill, grader, scenarios or settings after a batch's first counted
  run voids the batch (`freeze check`); a hold-out failure is a finding, never a grader edit;
  trial records never hold the check code.
- **Commits:** a change to the commit path reruns the crash campaign; never enable
  `fault-injection`, `durability-campaign` or `install-test-hooks` in a release.
