# Delivery governance and anti-drift controls

The accepted goal, scope, decisions, invariants and packet dependency graph live in
`delivery-ledger.json`. The Rust governance checker runs locally and in CI:

```console
cargo run --locked -p vsift-governance -- check
```

It validates the exact P00-P14 and R-01-R-14 sets, accepted DEC-01-DEC-13 records,
source documents, fixture truth, bidirectional requirement mappings, predecessor
ordering, completion evidence and the size limits of the two handoff files. Since P13
PR 8 it also lints every GitHub workflow (pinned actions, no `pull_request_target`,
least-privilege `permissions`, `id-token` only for release attestation and publishing,
no untrusted expressions in `run` scripts, and no feature selection in release builds;
since P13 PR 10 also the release workflow's publishing rules: publishing only from a
dispatch of the release tag with `dry_run` cleared, in the protected `release`
environment, with npm provenance under `next` (or, since P14 PR 8, `latest` for a stable
version, held by channel rules, a stable plan's guards and a ban on moving a dist-tag in any
workflow), and only the qualified tarballs by
digest; see [`../operations/release.md`](../operations/release.md)). A packet marked complete requires a full merge
commit and nonempty verification record. The checker deliberately fixes the R0
objective; changing it requires an explicit reviewed code, ledger and ADR change.

Since P14 PR 1 the same `check` also validates two further records, so the Governance job
enforces them on every pull request: the
[release evidence ledger](p14-evidence-ledger.json) (what each `RQ-nn` evidence item of the
[P14 plan](p14-qualification.md) proves and where it stands: its schema, the item set
against the plan, every referenced requirement, threat, verification row and limit against
its owning document, the rules each status carries, and that every requirement, P14 threat,
`R-SEC03` and P14-owned limit is supported by some item) and the
[public-claims registry](public-claims.json) (the statements the public documents may make
about support, stability and qualification, each tied to its rung and to evidence that must
be `passed`, and the phrases that are never claimed). Two commands run them on their own:

```console
cargo run --locked -p vsift-governance -- release-evidence
cargo run --locked -p vsift-governance -- release-evidence --complete-for <version> [--commit <sha>]
cargo run --locked -p vsift-governance -- public-claims
```

The second form is the completeness check (evidence item RQ-20): it fails unless every item
the release needs is passed for that version and commit, carried forward because nothing in its
scope changed (it asks Git, so it needs history), waived by a recorded maintainer decision or
not applicable; for the stable version it also needs a recorded candidate-to-stable delta, which
P14 PR 8's `vsift-release candidate-delta` computes and the stable plan writes as `release-delta.json`
(the maintainer copies it into the ledger after the publish, L-103); the Release workflow runs this
check for the accepted candidate and refuses a stable plan when it fails (RQ-20). These checks prove that recorded evidence exists and that banned words are
absent; they do not fetch a run or judge a sentence (known limits L-101 and L-103). A pull
request that changes a quoted public sentence updates the registry with it; one that changes an
item's status updates the ledger. Since P14 PR 9a every claim also lists the known-limits entries
its wording leans on (`limits`), and a claim above the `now` rung that is in use fails
`public-claims` while any of those entries is `pending` or `rejected`: the maintainer's one-pass
review of the register ([`register-review-sheet.md`](register-review-sheet.md)) therefore gates the
release wording, and the registry scans every public document it lists (the worker runbook, the
launcher's messages and the support matrix since PR 9a, the user guide since PR 9b). The guide has
two checks of its own, in the read-only `Guide` workflow: its generated reference pages must equal
what `vsift --help` and `schemas/v1` give, and its marked examples must equal what the real binary
prints ([`development.md`](../development.md#the-user-guide-and-its-checks)).

The [R1 industrial capability expansion](r1-industrial-capability-expansion.md)
reserves R-15..R-20 and P15..P20 without activating them. P15 must create a separate
machine-readable R1 ledger and independent fixture truth before any P16+ implementation.
Do not broaden the R0 ledger to make a future feature appear eligible, and do not use
an R1 requirement to excuse a missing R0 end-to-end video investigation capability.
Future packet issues #25 through #30 are grouped under the
[R1 milestone](https://github.com/smormah/vsift/milestone/2); their existence does not
make a packet active or satisfy its P14/P15 prerequisites.

GitHub milestone `R0 — First functional release` contains issues #3 through #17.
Each implementation PR cites one packet issue, requirements, tests, threats/findings,
predecessors and exclusions. Protected main, CODEOWNERS, required CI/security checks
and the Governance check prevent an unattended task from merging around the ledger.

## Delivery cycles and the test and release model (decided 2026-10-16)

The maintainer decided on 2026-10-16 how work is grouped, who may approve what inside a cycle, and
what is run, and when, to test and release it. **The cycle map is
[`backlog-iterations.md`](backlog-iterations.md)** (stories and issue numbers); the weekly window as an
operating procedure is [`release.md`](../operations/release.md) 6.18. This section is the policy.
**It supersedes the rule of 2026-10-15 that "the full round" (all hosted evidence plus agent batches 2 and 3) runs
"about once a week, at a big shift, or when many changes have accumulated, never per fix"**, which was
written in the earlier backlog map and in the two handoff files. No accepted ADR states that cadence, so
none is rewritten; [ADR 0024](../decisions/0024-r0-qualification-and-release-candidate.md) carries a dated note
instead. **Status: decided, and partly not built.** The pull-request gate's path rules, the nightly run and the
nightly precondition of a candidate are separate changes (CI workflows, the release workflow); until they land
the rules below are followed by hand and the existing CI still runs its full set on every pull request.

### Delivery in cycles (D1 to D5)

- **D1. Work is delivered in four larger cycles instead of small iterations, each running as many stories in parallel as can be handled.** Cycle 1 is `0.2.2`
  (the fixes found during `0.2.1`, with the test review folded in); cycle 2 is `0.2.3` (the re-pins, the seam
  follow-ups measured on real speech, and the evaluations); cycle 3 is launch readiness; then R1 (P15 to P20).
  The stories of each cycle are in the cycle map. Parallel work needs non-overlapping files and explicit integration ownership (rule 2 below).
- **D2. Cycle 1 takes #366, #367, #370 and #371** (the skill-wording issues `0.2.1` left open), besides #404 and
  #257 first.
- **D3. Standing approvals inside a cycle.** The supervisor merges on green after review, files issues for
  findings, and closes an issue whose own closing criteria are met, with the evidence in the comment.
  **Every `release` environment gate and every npm write stays with the maintainer.** A decision that this
  document or an ADR reserves to the maintainer (a waiver, a claims rung, moving `next`) is not covered.
- **D4. The README is rebuilt as the front page by a separate design session during cycle 1.** It starts early in
  the cycle and does not wait for the fixes; it belongs to cycle 3's launch readiness.
- **D5. Records.** Both handoff files are still rewritten in every pull request, as `AGENTS.md` says.
  Parallel pull requests rebase before they merge, so the rewrite of one is never lost to the other.

Iteration A of the earlier map (closing what `0.2.1` settled) was done on 2026-10-16; the cycle map says how.

### The test and release model (D6)

| Stage | When | What runs | Rule |
| --- | --- | --- | --- |
| **Pull-request gate** | every pull request | only what the change can affect | Target 5 minutes, hard limit 10. A docs-only change runs the document and governance checks. A code change runs lint plus the unit and contract tests of the touched crates and their dependents, on Ubuntu. Windows and macOS run only for a platform-sensitive change. A hot fix can request the full three-system run. |
| **Nightly** | on `main`, when something merged | at least the full three-system check (what it adds is the CI change's design) | A failure opens an issue automatically and **must be fixed before further development (stop the line)**. **A release candidate may only be cut from a commit whose nightly passed.** |
| **Weekly release window** | the candidate is cut at Friday or Saturday midnight, in the maintainer's local time | the long hosted runs (2 to 3 hours), after the cut, limited to what the week's changes touched | The target is to release and publish on **Monday or Tuesday**. A failure in the window gets a targeted fix and a rerun of **only the failing workflow**, never the whole round, while development continues (see the constraint below). |
| **Hot fix** | outside the window | the full three-system check (about 30 minutes), then the publish, then the quick checks of the published bytes | The quick checks are `P14 verify release`, `P14 journeys`, `P13 managed smoke` and `P14 published artifacts` from the previous release and from the candidate. |
| **Monthly, or with a major change** (a skill rewrite, a re-pin) | about once a month, or when such a change lands | the full-cost items | Agent batches 2 and 3; the dependency-quality checks (`P07 local ASR` accuracy, `P14 malicious media`, the dated scan reading); the full durability campaign. **Not weekly.** An ordinary week's skill wording change gets the uncounted skill smoke (about nine Claude runs on a local build, about $6) instead. |
| **Weekly drift runs** | scheduled | the published binary, as today | They stay. A failure opens an issue by itself. |

**Why the split.** The full round costs half a day or more, millions of client tokens and thousands of runner minutes. Most of it
measures things a week's fixes did not touch, and a failure found late blocks a release that waited for it. The gate answers
the author quickly, the nightly finds what the gate chose not to run while the cause is one day old, and the window
spends the expensive runs once, on a commit that is already known to be green.

**Our code or a dependency's quality.** `P07 local ASR`'s word-error figure measures mostly whisper.cpp; `P14 malicious media`
measures FFmpeg's robustness as much as our isolation; the scan reading reads the pinned tools' records. They belong to the monthly
round and to a re-pin, a model change or a change to the way VSift calls a tool, not to every candidate.

**What the model does not change.**

- The four commands in `AGENTS.md` are still what an author runs before completing a change. The gate is the CI's
  shortcut, not the author's.
- Protected `main` keeps its required checks. A path-limited job must still report on every pull request; how it does is the
  CI change's design.
- The candidate rule ([`release.md`](../operations/release.md) 6.8, ADR 0024 decision B) and the stable plan's six guards hold. A
  hot fix still goes through a published candidate, because the stable plan needs one.
- **The evidence ledger's completeness check is not relaxed.** An item whose scope a change touched is `passed`, carried by the
  recorded delta, or waived by a recorded decision of the maintainer. The ledger's scopes are coarse (`crates` is in the
  scope of most items, RQ-01 to RQ-12 and RQ-14 to RQ-17), so "limited to what the week touched" does not by itself make the check pass: narrowing the scopes
  or deciding waivers is a change of its own, with the release-workflow change that enforces the nightly precondition.
- **The freeze between a candidate's tag and the stable commit (6.8) holds**, so "development continues" in the window means branches
  and draft pull requests, and merges of work record and documents that 6.8 allows; a change to code, a dependency or a workflow
  waits for the publish. Replacing this with a release branch would change the candidate check and ADR 0024 decision B, and is not decided here.

## Rules for implementation sessions

These apply to attended and unattended sessions alike, whether the implementer is a
person or an AI agent.

1. Read `AGENTS.md`, both memory files, the ledger, packet, linked contracts/tests and
   accepted ADRs before editing.
2. Work only on the earliest active packet whose predecessors are complete. One agent
   owns integration for that packet; parallel work requires non-overlapping files and
   explicit integration ownership.
3. Start from current protected `main`; record the predecessor commit. Never continue
   on a stale branch or silently absorb unrelated changes.
4. Keep each PR within the issue's observable outcome and exclusions. Put attractive
   unrelated ideas into a later issue without implementing them.
5. Do not relax security limits, tests, lint rules, branch protection, retention,
   permissions or typed errors to obtain a passing run.
6. A new dependency, network access, persistent data, executable source, public schema,
   platform claim or privilege requires its recorded review and relevant ADR update.
7. Treat failing tests as evidence. Fix the defect or revise the accepted design with
   an explicit ADR; never rewrite independent ground truth to match implementation.
8. Update code, tests, docs, changelog, ledger and handoff files in the same PR. Record
   actual limitations and residual risk. Verification commands and results belong in
   the PR description; do not open separate PRs that only record evidence.
9. Complete means merged through protected checks. Only when a whole packet completes,
   one small follow-up may set its ledger status with the merge commit the
   implementation PR could not know in advance.
10. Stop at packet completion or a documented blocker. Do not automatically advance
    into the next packet merely because time or model context remains.
11. R0 release evidence must include the complete video-to-grounded-handoff journey
    through the two named independent agent clients; component tests cannot substitute.
12. `memory/TODO.md` and `memory/project_current_status.md` describe the current state.
    Rewrite them rather than appending history, keep them within the governance
    checker's size limits, and leave history to git, the changelog, qualification
    records and `docs/history/`.
13. Report status in plain English. Say explicitly whether an increment or the whole
    packet is complete, and list what remains.
14. A failed required check is evidence. Link it to a tracked issue before re-running,
    and treat a tracked intermittent failure as priority work rather than routine noise.
15. A failed nightly stops the line (decided 2026-10-16): the issue it opens is fixed before further
    development, and a release candidate is cut only from a commit whose nightly passed. The model is
    the section "Delivery cycles and the test and release model" above.

## Drift response

If a change does not map to an R0 requirement, defer it. If an invariant conflicts
with a desired feature, stop and raise a scope ADR. If a predecessor or evidence is
missing, keep the packet planned/in progress. If implementation reveals an unsafe or
unreliable assumption, add a finding and regression target before continuing.

Automation cannot prove semantic correctness or prevent a maintainer from deliberately
changing its own controls. It makes drift visible and reviewable; protected review and
test evidence remain necessary.
