# VSift current status

## Active

P02 / issue #5 is active on `feat/p02-secure-process-supervisor`, based on
`cedbb63ad1ddc69bb1bbb405b424da67494ce990`. Work is limited to the secure process
supervisor and provider-resolution boundary: explicit invocation, trusted executable
selection, bounded streams, cancellation/deadline escalation, descendant containment
and honest effective-control reporting. P03+ storage/media behavior remains out of
scope.

## Complete

### 2026-09-10 — P01 public command and JSON contracts

PR #20 squash-merged as `3d7a7d2a53fd8e6d2025726d34c59e7603fc8e71`.
P01 published typed IDs, time/ranges, crops, cursors, confidence and job transitions;
the full reserved R0 parser; bounded human/JSON/JSONL presentation; strict JSON input
limits; immutable configuration precedence; and four v1 schemas with compatibility
examples. C-01..C-10 are mapped to 64 passing local tests. Governance, documentation,
dependency policy/review, CodeQL/Rust analysis and Quality on Ubuntu, Windows and
macOS all passed.

Only `setup check` executes. Other parsed commands return `COMMAND_NOT_IMPLEMENTED`;
P01 does not claim process supervision, storage, media, provisioning, or worker
behavior. SEC-01/03/16/21 residual controls remain with their later owning packets.

### 2026-09-10 — P00 decisions, corpus truth and delivery governance

PR #18 merged as `924f6c52b018cf8f018568efd51a74cb6638b00e`. DEC-01..13
are accepted through ADRs 0004-0009. P00 / issue #3 established qualification and
resource profiles, F01-F12 declarative fixture truth, R0 traceability, milestone 1
with issues #3-#17, and an executable delivery ledger. The Governance job passed and
is now a required protected-main check alongside the existing quality/security gates.
All required PR checks passed; the local workspace had 13 passing tests.

No media runtime feature was implemented by P00. Fixture media generation remains P04.

### 2026-09-09 — Planning baseline prepared

Reviewed revision: `df85f7065915145ab8b75b89756f0fa1e341a5f1`.
Planning documents: [overview](../docs/planning/README.md),
[contracts](../docs/planning/architecture-and-contracts.md),
[security](../docs/planning/security-threat-model.md),
[tests](../docs/planning/verification.md),
[packets](../docs/planning/implementation-work-packets.md),
[current gaps](../docs/planning/baseline-review.md).

The accepted baseline includes R0 single-host worker use, cross-process admission,
checkpoints,
explicit durable state and bounded batch execution. Persistent cross-video indexing
remains optional and deferred. ADR 0004 is accepted. These capabilities
are unimplemented. The planning record merged through PR #2 as `e3f8569`.

### 2026-09-09 — Implemented scaffold

`5289a2b` introduced the Rust workspace, setup diagnostic precursor, eight tests,
docs and GitHub automation. `d7a459e` enabled the support documentation path.
PR #1 / `df85f70` renamed the public command to `vsift setup check`, with JSON operation
`setup.check`; reported required checks passed at that revision.

## Implemented versus planned

Only setup dependency probing is implemented. No ingestion, transcription, visual
extraction, session storage, managed installation, queue, index, durable job or npm
release exists yet. Inspection found unbounded pre-truncation process output, ambient
executable discovery and incomplete process-tree cleanup.
See B-01..B-11; do not describe current code as production hardened.

## Next action

Complete P02 / issue #5 through protected review. Preserve P03+ storage/media behavior
as later work and record P-01..P-08 plus C-05 verification in the delivery ledger.
