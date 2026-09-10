# VSift current status

## Complete

### 2026-09-09 — Planning baseline prepared

Reviewed revision: `df85f7065915145ab8b75b89756f0fa1e341a5f1`.
Planning documents: [overview](../docs/planning/README.md),
[contracts](../docs/planning/architecture-and-contracts.md),
[security](../docs/planning/security-threat-model.md),
[tests](../docs/planning/verification.md),
[packets](../docs/planning/implementation-work-packets.md),
[current gaps](../docs/planning/baseline-review.md).

The proposal includes R0 single-host worker use, cross-process admission, checkpoints,
explicit durable state and bounded batch execution. Persistent cross-video indexing
remains optional and deferred. ADR 0004 is proposed, not accepted. These capabilities
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

Review and resolve the planning decision register with the maintainer, then execute
accepted packets with linked tests and reviewed contracts. Do not infer implementation
authorization solely from this proposal. Future sessions start with this file and TODO.
