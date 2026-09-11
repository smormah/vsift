# Delivery governance and anti-drift controls

The accepted goal, scope, decisions, invariants and packet dependency graph live in
`delivery-ledger.json`. The Rust governance checker runs locally and in CI:

```console
cargo run --locked -p vsift-governance -- check
```

It validates the exact P00-P14 and R-01-R-14 sets, accepted DEC-01-DEC-13 records,
source documents, fixture truth, bidirectional requirement mappings, predecessor
ordering and completion evidence. A packet marked complete requires a full merge
commit and nonempty verification record. The checker deliberately fixes the R0
objective; changing it requires an explicit reviewed code, ledger and ADR change.

The [R1 industrial capability expansion](r1-industrial-capability-expansion.md)
reserves R-15..R-20 and P15..P20 without activating them. P15 must create a separate
machine-readable R1 ledger and independent fixture truth before any P16+ implementation.
Do not broaden the R0 ledger to make a future feature appear eligible, and do not use
an R1 requirement to excuse a missing R0 end-to-end video investigation capability.

GitHub milestone `R0 — First functional release` contains issues #3 through #17.
Each implementation PR cites one packet issue, requirements, tests, threats/findings,
predecessors and exclusions. Protected main, CODEOWNERS, required CI/security checks
and the Governance check prevent an unattended task from merging around the ledger.

## Rules for unattended implementation

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
8. Update code, tests, docs, changelog, ledger and memory in the same PR. Record actual
   limitations and residual risk.
9. Complete means merged through protected checks. The following record update adds
   the merge hash if the implementation PR could not know it in advance.
10. Stop at packet completion or a documented blocker. Do not automatically advance
    into the next packet merely because time or model context remains.
11. R0 release evidence must include the complete video-to-grounded-handoff journey
    through the two named independent agent clients; component tests cannot substitute.

## Drift response

If a change does not map to an R0 requirement, defer it. If an invariant conflicts
with a desired feature, stop and raise a scope ADR. If a predecessor or evidence is
missing, keep the packet planned/in progress. If implementation reveals an unsafe or
unreliable assumption, add a finding and regression target before continuing.

Automation cannot prove semantic correctness or prevent a maintainer from deliberately
changing its own controls. It makes drift visible and reviewable; protected review and
test evidence remain necessary.
