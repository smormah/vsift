# VSift current status

## Next

### P01 — Public command and JSON contracts

P01 / issue #4 is the earliest permitted packet. It defines typed command requests,
JSON envelopes, exit codes and contract tests before process, storage or media work.

## Complete

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
are unimplemented. This documentation commit/PR is the provenance for this entry;
add its merge hash when next updating the record.

### 2026-09-09 — Implemented scaffold

`5289a2b` introduced the Rust workspace, setup diagnostic precursor, eight tests,
docs and GitHub automation. `d7a459e` enabled the support documentation path.
PR #1 / `df85f70` renamed the public command to `vsift setup check`, with JSON operation
`setup.check`; reported required checks passed at that revision.

## Implemented versus planned

Only setup dependency probing is implemented. No ingestion, transcription, visual
extraction, session storage, managed installation, queue, index, durable job or npm
release exists yet. Inspection found unbounded pre-truncation process output, ambient
executable discovery, incomplete process-tree cleanup and insufficient contract tests.
See B-01..B-11; do not describe current code as production hardened.

## Next action

Execute only P01 / issue #4. Future sessions start with this file, TODO and the
delivery ledger, and must pass the Governance check before merge.
