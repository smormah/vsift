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
environment, with npm provenance under `next`, and only the qualified tarballs by
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
cargo run --locked -p vsift-governance -- release-evidence --complete-for 0.2.0-rc.1 [--commit <sha>]
cargo run --locked -p vsift-governance -- public-claims
```

The second form is the completeness check (evidence item RQ-20): it fails unless every item
the release needs is passed for that version and commit, carried forward because nothing in its
scope changed (it asks Git, so it needs history), waived by a recorded maintainer decision or
not applicable; for the stable version it also needs a recorded candidate-to-stable delta, which
P14 PR 8 builds. These checks prove that recorded evidence exists and that banned words are
absent; they do not fetch a run or judge a sentence (known limits L-101 and L-103). A pull
request that changes a quoted public sentence updates the registry with it; one that changes an
item's status updates the ledger.

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

## Drift response

If a change does not map to an R0 requirement, defer it. If an invariant conflicts
with a desired feature, stop and raise a scope ADR. If a predecessor or evidence is
missing, keep the packet planned/in progress. If implementation reveals an unsafe or
unreliable assumption, add a finding and regression target before continuing.

Automation cannot prove semantic correctness or prevent a maintainer from deliberately
changing its own controls. It makes drift visible and reviewable; protected review and
test evidence remain necessary.
