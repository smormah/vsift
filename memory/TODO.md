# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress: decisions A-H confirmed
2026-10-02.** The plan is 14 pull requests (0-13). **PRs 0 (#250), 1 (#251), 2 (#255), 6 (#262,
the agent-trial harness) and 8 (#252) are merged. PR 3 (#254, the journeys on the published
binary) is done in this change, awaiting review; PR 4 (#259) is a draft; PR 5 (#276) narrows
SEC-T01.** The packet is not complete. Plan: `docs/planning/p14-qualification.md` (section 15: PR
2's record; 17: PR 3's); ADR 0024 stays Proposed. **Evidence:** `p14-evidence-ledger.json`:
RQ-01 to RQ-04, RQ-06, RQ-19 `passed`, RQ-05 `running`, **all for 0.1.0 only**; RQ-14 `waived`
(2026-10-03); 12 `planned`. **Public text:** `public-claims.json` (rung `now`).

**Decided 2026-10-02 (ADR 0024, as recommended):** **A** R0 ships as `0.2.0` on `latest`. **B** a
published `0.2.0-rc.N` under `next`, never announced. **C** no signing unless try-outs show a
block. **D** 84 agent runs in three batches, each waiting for the maintainer's go. **E** SEC-T01
narrowed (option 4, 2026-10-03; no stand-in is authored for R0). **F** "supported" per cell by fixed
rules; managed install Ubuntu-only; Codex on Windows unsupported. **G** a claims ladder; nothing
announced before P14 completes. **H** a try-out blocks the stable only until observed. #246 waits.

**PR 3 in one view** (ADR 0024's note; `docs/development.md`): `VSIFT_E2E_BINARY` (+
`_EXPECTED_VERSION`, `_EXPECTED_COMMIT`) makes every real-tool checkpoint drive an installed
`vsift`; refused, never ignored, when wrong. `P14 journeys` runs them against `vsift-cli@<version>`
from the real registry on Ubuntu (managed tools), Windows (pinned) and macOS 15 (Homebrew, not
reviewed, L-114); `P13 managed smoke` takes `published_version`; both run weekly. **On 0.1.0:**
all three passed (53 stages each); P11's durable stage cannot run on a hosted runner (root mounted
`nobarrier`; #258, L-113). Weaker than it sounds: L-115 (later tests; no launcher or archives);
#263: one Windows run stalled (the driver, hardened).

**PR 6 in one view** (ADR 0024's note; `docs/agents/trials.md`, "The P14 batches"): **nothing was
run, no model called.** `vsift-agent-trials install` installs `vsift-cli@<exact version>` from the
real registry into a fresh prefix and proves it in every record; `prepare` plays the user; cold
mode `C-01..C-03` (**Claude strict**, vsift only: its pilots stalled; **Codex realistic**, the
container; a realistic Claude run needs an isolated machine, so the baseline compares only within a
client: L-125), hold-outs, `freeze`, usage capture, `campaign`, `summarize`, `run-campaign.ps1`;
Codex `libgomp1` (#256); Git Bash only (#257). Weak points: L-117..L-120. **Batch 1** pilots re-run.

**README graphics** (`docs/assets/readme/`): SVG text is unscanned (L-121), so hand-check it;
redraw `roadmap.svg` with PRs 10 and 13. **0.1.0 today:** on npm under `next` (`latest` is an empty
placeholder) and a GitHub pre-release; not announced. The stable path never ran (L-105).

## The P14 pull requests (0, 1, 2, 6, 8 merged; 3 on merge, 4 draft)

**4** campaigns, malicious media, runbook walk; **5** SEC-T01 narrowed (#276); **7** fixes for
what 2-6 find (#256, #257; `vsift --help` if the cold baseline shows gaps); **9** matrix,
documents, claims enforced, register sheet, the R0 user guide; **10** candidate `0.2.0-rc.1`;
**11** its qualification (batches 2 and 3; dispatch `P14 journeys` too); **12** stable `0.2.0`;
**13** ledger follow-up, P14 `complete`, handoff for the R0 trial. **Before PR 10:** settle the
allowed-path lists in `candidate.rs`; freeze the skill, grader, scenarios and settings (`freeze
write`). **Before PR 12:** register the two stable checks in `STABLE_CHECKS`
(`tools/p14-published/lib/verify.cjs`).

## What the maintainer owes, and when

- **Now:** review PR 3 (ADR 0024's note first: the weekly schedules, about an hour of hosted
  minutes a week; macOS judged with Homebrew's tools; RQ-05 left `running` for #258). **The go for
  batch 1:** edit a copy of `campaigns/campaign.example.json`, then `run-campaign.ps1 -Batch 1
  -Client claude|codex -Version 0.1.0 -MaxRuns 4` (the pilots), read them, then the rest; needs
  Claude Code 2.1.284 and Docker.
- **PR 9:** one pass over the thirty register
  entries the claims lean on. **PRs 10 and 12:** each publish (`release.md` 6.3 and 6.7). **PR
  11:** the go for batches 2 and 3 (the blurred-banner re-run, L-095); a Smart App Control
  try-out on the second, clean Windows 11 machine; no macOS try-out (no Mac).

## Open decisions and readings (maintainer)

- **#204** Codex on Windows (L-076; documented unsupported); the grader's `untrusted_listed`
  reading (F12-E01) and #219: both settled before the trial freeze.
- **Readings** (ADR 0021 and 0023 review lists; none blocks): L-062, L-067, L-069, L-017, L-088,
  L-090, `tokio`, `durable_worker`, exits 126/127, MSRV.

## Tracked issues and gates

- **Open:** #17 (P14), #219, #224 (L-095), #188 (SEC-T01, R1), #232, #246 (deferred), #256, #257
  (PR 7; its #261 and #273 are fixed), #258 (P11's durable stage on the published binary), #263 (a
  stalled Windows run); #170-#178 (register); #159, #150, #147; flaky tests #128, #206, #253 (a
  managed-store kill test on Windows), #268 (a macOS SIGTERM test); #204.
- **Opt-in real-tool paths** (`--ignored`) run a Cargo-built binary unless `VSIFT_E2E_BINARY`
  names an installed one; `P14 journeys` and `P13 managed smoke` run the published one weekly
  (L-042); a failed run is a finding: issue first, then rerun. **P14 tools:**
  `tools/p14-published/test/`, `tools/test_p14_journeys.py`. **Campaigns:** never on the
  maintainer's machine (L-056, L-057) except the agent batches.

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff on both
  transcript paths in named trials; P14 repeats it from a clean install. **The skill** calls the
  published CLI only; a new public command, flag, failure code or referenced field needs a skill
  update in the same change (`skill_contract`).
- **New public items** need their `CommandName`/`FailureCode::ALL`/`EventKind::ALL`/
  `EvidenceRecordType::ALL` entry, v1 schemas and a human renderer with a snapshot; v1 is
  additive only; a parser of untrusted input needs a fuzz target; npm packages have no scripts.
- **The release seam:** a change to `vsift-release`, `release.yml` or its lint needs a lint
  rule, a broken-copy mutation test and the shell check; no workflow runs `npm dist-tag`; no
  test hard-codes the version.
- **Trials:** a change to the skill, grader, scenarios or settings after a batch's first counted
  run voids the batch (`freeze check`); a hold-out failure is a finding, never a grader edit.
  **Commits:** the commit path reruns the crash campaign; no development feature in a release.
