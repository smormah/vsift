# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now

**P00-P13 are complete. P14 (R0 qualification, #17) is in progress: decisions A-H confirmed
2026-10-02.** The plan is 14 pull requests (0-13). **PRs 0 (#250), 1 (#251), 2 (#255), 3 (#254,
the journeys on the published binary), 5 (#276, SEC-T01 narrowed), 6 (#262, the agent-trial
harness) and 8 (#252) are merged; PR 4 (#259) is a draft; PR 7 (fixes) is under way; agent-trial
batch 1 ran on 2026-10-03.** The packet is not complete. Plan: `docs/planning/p14-qualification.md` (section 15: PR
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

**PR 6 and batch 1 in one view** (ADR 0024's note; `docs/agents/trials.md`; reading:
`docs/planning/p14-agent-trials/batch-1-reading.md`): `vsift-agent-trials install` installs
`vsift-cli@<exact version>` from the real registry and proves it in every record; cold mode
`C-01..C-03` (**Claude strict**; **Codex realistic**, the container: L-125), hold-outs, `freeze`,
usage capture, `campaign`, `summarize`, `run-campaign.ps1`. **Batch 1 (0.1.0, 20 runs, a baseline,
nothing qualified):** skill pilots 4 of 4; cold useful 1 of 6 (Claude), 2 of 6 (Codex); no cold run
installed anything; safety "not met" on grader classes to settle before batch 2 (#282).

**README graphics** (`docs/assets/readme/`): SVG text is unscanned (L-121), so hand-check it;
redraw `roadmap.svg` with PRs 10 and 13. **0.1.0 today:** on npm under `next` (`latest` is an empty
placeholder) and a GitHub pre-release; not announced. The stable path never ran (L-105).

## The P14 pull requests (0, 1, 2, 3, 5, 6, 8 merged; 4 draft)

**4** campaigns, malicious media, runbook walk; **5** SEC-T01 narrowed (#276); **7** fixes for
what 2-6 find (#256, #257; `vsift --help` gets a typical-investigation section: batch 1 shows the gap); **9** matrix,
documents, claims enforced, register sheet, the R0 user guide; **10** candidate `0.2.0-rc.1`;
**11** its qualification (batches 2 and 3; dispatch `P14 journeys` too); **12** stable `0.2.0`;
**13** ledger follow-up, P14 `complete`, handoff for the R0 trial. **Before PR 10:** settle the
allowed-path lists in `candidate.rs`; freeze the skill, grader, scenarios and settings (`freeze
write`). **Before PR 12:** register the two stable checks in `STABLE_CHECKS`
(`tools/p14-published/lib/verify.cjs`).

## What the maintainer owes, and when

- **Now:** read `batch-1-reading.md` and settle its three grader questions (a path in a report;
  `--session-root` for a cold agent; listing a system folder) **before batch 2**; a grader change
  needs a new freeze, once, before PR 10's candidate cut. **Batches 2 and 3** run only on the
  maintainer's go, on the candidate (`run-campaign.ps1 -Batch N -Client claude|codex -Version
  <rc>`); needs Claude Code 2.1.284 and Docker.
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

- **Open:** #17 (P14), #219, #224 (L-095), #188 (SEC-T01, R1), #232, #246 (deferred), #257
  (PR 7; #256, #261, #273 fixed; #257 documented, L-109 accepted), #258 (P11's durable stage on the published binary), #263 (a
  stalled run), #282, #283 (trial script, records); #170-#178 (register); #159, #150, #147; flaky tests #128, #206, #253 (a
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
