# VSift work record

Current-state handoff, rewritten in every change; history: git, `CHANGELOG.md`, `docs/history/`.

## Now (2026-10-09)

**Note, 2026-10-10:** `0.2.0` was published on 2026-10-09 (tag `v0.2.0` at `eeb2a22a46a8`, the merge of #344; the GitHub release is not a pre-release). What this page says of PR 12 being "prepared", of nothing being published and of `latest` being `0.0.0` was written before the publish; PR 13 records the publish, its checks and the deprecation of rc.1 and rc.2 and rewrites it. The one change of 2026-10-10 here is the RQ-10 waiver.
**P00-P13 are complete. P14 (R0 qualification, #17) is in progress.** Its plan has 14 pull requests (0-13): 0-10 are done, 11's evidence is complete (the maintainer's register pass and reading of the cold logs remain), **12 (the stable `0.2.0`) is PREPARED and not complete**, 13 follows. Plan:
`p14-qualification.md` (sections 15-30); ADR 0024 stays Proposed until P14 completes.
- **PR 12 is prepared, not done.** The commit the maintainer will tag `v0.2.0` is on the branch `p14-pr12-stable-0.2.0`, built on #343 (the maintainer's decisions of 2026-10-09, not yet on `main`); the
  supervisor opens its pull request after #343 merges. It differs from the tag `v0.2.0-rc.3` only in: the version text of five files (`0.2.0-rc.3` becomes `0.2.0`), the two shipped documents
  (`docs/operations/install.md` and `npm/vsift-cli/README.md`, now written for `npm install vsift-cli` with no tag) and the work record (also the guide's first page and two stale non-claims removed from
  `public-claims.json`). `vsift-release candidate-delta` refuses nothing, `cargo check --locked` accepts the lockfile, and `release-evidence --complete-for 0.2.0-rc.3 --commit 83dca856...` passes.
  **Nothing is tagged, published or dispatched. PR 12 is complete only when the maintainer has published `0.2.0` and verified it.** The maintainer's checklist is
  `docs/planning/p14-stable-release-steps.md` (written as a planning page because `release.md` may not change before the tag).
- **What the maintainer does next, in this order:** (1) merge #343, then the PR 12 pull request, only when able to tag and publish at once (installation guide and npm README say `latest` is `0.2.0`
  before it is); (2) the checklist's preflight (the one setting only they can read: the four trusted publishers on npmjs.com, which `--tag latest` has never used), tag `v0.2.0`, dry run, publish with
  `dry_run` cleared, approve the `release` environment, check from outside; (3) **deprecate `0.2.0-rc.1` and `0.2.0-rc.2` on the four packages** (eight `npm deprecate` commands in the checklist; decided
  2026-10-09; the supervisor never runs them; rc.3 is not deprecated); (4) the hosted checks on `0.2.0` (`P14 verify release` is red on two named checks by design until they are registered);
  (5) **within seven days:** keep the publish run's `release-delta.json`, and have the two `STABLE_CHECKS` of `tools/p14-published/lib/verify.cjs` registered; (6) PR 13.
- **PR 13 (to do after the publish):** the ledger's own entries for `0.2.0` (`repeat` items RQ-01 to RQ-06, RQ-13, RQ-18, RQ-19), `release_delta` copied in, `release-evidence --complete-for 0.2.0 --commit
  <stable commit>` passing, the repository-only pages flipped, the claims rung to `after_p14`, P14 `complete` in the delivery ledger, ADR 0024 Accepted, the register swept, the neutral checkpoint for using the
  published CLI. **Pages the stable commit may not change, stale from the publish until PR 13:** root `README.md` (`@next` install block, status paragraph) and `roadmap.svg`, `docs/agents/skill.md` (line 43 and
  ~296), `docs/development.md` (the `--complete-for 0.2.0-rc.3` example), `docs/operations/release.md` (6.12 step 7), `SECURITY.md`. **Allowed but left alone:** `docs/guide/limits.md` (line 5), the support
  matrix (line 106), `delivery-governance.md` (line 38), `rq-17-tryout-sheet.md` (`@next` installs).
- **State of the packages:** `latest` is the empty `0.0.0` placeholder on all four npm packages; `next` is `0.2.0-rc.3` (tag `v0.2.0-rc.3` at `83dca856e7a0`, published 2026-10-08). `0.2.0-rc.1` (2026-10-05) and
  `0.2.0-rc.2` (2026-10-07) are published, superseded and not deprecated yet.
- **Evidence for `0.2.0-rc.3` is complete (2026-10-09):** `passed` RQ-01 to RQ-09, RQ-11 to RQ-13, RQ-15, RQ-18, RQ-19; `waived` RQ-14 (a mechanism), **RQ-10** (rc.3 and, decided 2026-10-10, the stable: a symbolic link answers `STORAGE_IO`,
  #265; its rule now admits `INVALID_ARGUMENT`), **RQ-16** (rc.3 and the stable: one cold agent read the clip `vsift audio` named; #340, L-142) and **RQ-17** (rc.3 and the stable: no Smart App Control, clean-machine
  or Gatekeeper try-out; L-143). A waiver is not a pass and the checker cannot see its limits; they live in the decisions' texts. Nothing has been run against `0.2.0`'s own bytes: that follows the publish.
- **Public text:** `public-claims.json` rung `candidate`; CL-201, CL-206 and the other later-rung claims stay unused. CL-204's note and the support matrix's agent-client paragraph still say "the repeat decides" (stale
  since batch 2). The stable's documents avoid "stable" and "supported" (registered statements only) and say what was not tried.
**Decided 2026-10-02 (ADR 0024):** **A** R0 is `0.2.0` on `latest`. **B** `0.2.0-rc.N` under `next`, never announced. **C** no signing unless try-outs show a block. **D** 84 agent runs in three batches.
**E** SEC-T01 narrowed. **F** "supported" per cell by fixed rules; managed install Ubuntu-only. **G** a claims ladder; nothing announced before P14 completes. **H** a try-out blocks the stable only until
observed (or waived: used for RQ-17). #246 waits. **2026-10-07/08:** improve the skill and cut rc.3 (done, batch 2 met every gate); the re-pins stay after `0.2.0`. **2026-10-09:** RQ-10's rule admits
`INVALID_ARGUMENT`; the RQ-16 waiver carries to the stable; rc.1 and rc.2 are deprecated at the stable; RQ-17 waived, the stable ships untried. **2026-10-10:** the RQ-10 waiver carries to the stable, for the link case only (plan 29.5).

## Open for the maintainer (none decided here)

- **Move `next` after the publish?** (L-108.) The workflow never does; both shipped documents are true either way. What Yarn 4 does with an untagged `yarn add vsift-cli` during its one-day hold is not known.
- **The register pass** over `register-review-sheet.md` (every review `pending` but L-137 and L-138 accepted, L-139 rejected = to be fixed; L-142 and L-143 are new). L-139 and L-095 record rc.3's result and stay
  open; L-138 describes no live limit and L-134 is narrowed. Also whether `Guide` becomes a required check, and CL-204's note and the matrix paragraph.
- **Confirm the reading of the 18 cold logs** (L-118): the maintainer is reading a generated command list; to be recorded when they confirm. **#340:** which option (help text only, an additive JSON hint, or
  accept and fix in `0.2.x`); a change to the CLI's text is a new candidate or a `0.2.x`. **Close #321** (3,000 clean hosted repetitions). #312: fix or accept.
- **After the stable:** the Dependabot pull requests (#192, #193, #194), the whisper.cpp re-pin (#322, L-137) with FFmpeg's (#272, L-132; next month-end build 2026-10-31), the RQ-17 try-out on LOKI and a Mac
  (`rq-17-tryout-sheet.md`; it would give CL-201 its evidence), `roadmap.svg` (L-121).
- **From the cold round (none filed):** `--limit` help says 1 to 100 where the budget is 50; `BUSY` with no hint for two `frame get` calls at once (L-131); the no-transcript answer is `INVALID_ARGUMENT` with a
  true remediation (L-127's family). **R1 options:** a stub per ended request (L-063); the Windows kill window (L-129); a bounded wait for the initialization lock (L-131, L-135); retry Windows sharing
  violations (L-136); #325's later steps (L-140); #334 (L-141). **At v2:** the codes L-127 keeps. **#204** Codex on Windows (L-076). **Readings:** L-062, L-067, L-069, L-017, L-088, L-090, `tokio`, `durable_worker`, MSRV.

## P14 PR 7, the findings by outcome (each with its pull request and regression test; details in `project_current_status.md`)

**Maintainer rule 2026-10-04: a published failure code stays (v1 is additive); the remediation carries the fix (L-127).**
- **Fixed in rc.2:** #314 (#318; L-136 remains on Windows), #310 (#319). **Fixed in rc.3:** #322 and #332 (#330), #325 step 1 (#331), tests or tools #321, #310's second finding, #327 (#333).
  **Narrowed, not proven gone:** #206 (L-005); #253 (L-129); #312 (L-135; one failure in 4,500 CPU-loaded Windows repetitions over three candidates; not fixed).
- **Not fixed:** #272 the FFmpeg snapshot: 46 of 47 records fixed in the shipped build (#296, L-122); a pin must be a month-end build (L-132), **after `0.2.0`**, with the whisper.cpp re-pin (L-137).

## Tracked issues and gates

- **Open:** #17 (P14), #219, #224 (L-095, L-139), #188 (SEC-T01, R1), #232, #246 (deferred), #258, #263, #272 (residual accepted, L-122), #312 (L-135), #314 (fixed in rc.2; L-136), #316 (hosted runners
  scarce), **#321** (fixed; the maintainer closes it, L-138), **#336** (L-139's citation half), **#322** (the floor is in rc.3; the pin stays, L-137), #325 (step 1 done, L-140), #334 (L-141), **#340** (L-142);
  #128 (watch); flaky tests #253 (Windows kill test) and #268 (macOS SIGTERM test): comment with the run link, rerun the job; #170-#178 (register); #159, #150, #147.
- **From the candidate's tag to the stable merge only the work record, `install.md` and the npm README may change** (`release.md` 6.8; the stable is compared with the highest candidate tag, `v0.2.0-rc.3`).
  Nothing else may merge: no Dependabot, workflow, tool or dependency change.
- **Opt-in real-tool paths** (`--ignored`) run a Cargo-built binary unless `VSIFT_E2E_BINARY` names an installed one (L-042); a failed run is a finding: issue first, then rerun. A stage that tests a fix newer
  than the published version skips below its first version only (L-115). **P14 tools:** `tools/p14-published/test/`, `tools/p14-campaigns/test/`. **Campaigns:** never on the maintainer's machine (L-056,
  L-057) except the agent batches; see `docs/development.md`.

## Guardrails

- **R0 ships** only when a coding agent goes from a local video to a grounded handoff on both transcript paths in named trials; P14 repeats it from a clean install. **The skill** calls the published CLI only;
  a new public item needs a skill update in the same change (`skill_contract`).
- **New public items** need their `*::ALL` entry, v1 schemas and a human renderer with a snapshot; v1 is additive only; a parser of untrusted input needs a fuzz target; npm packages have no scripts.
- **The release seam:** a change to `vsift-release`, `release.yml` or its lint needs a lint rule, a broken-copy mutation test and the shell check; no workflow runs `npm dist-tag`.
- **Trials:** a change to the skill, grader, scenarios or settings after a batch's first counted run voids the batch (`freeze check`); a hold-out failure is a finding, never a grader edit. **Commits:** the
  commit path reruns the crash campaign; no development feature in a release.
