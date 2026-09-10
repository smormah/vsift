# VSift current status

## Active

### P01 — Public command and JSON contracts

P01 / issue #4 started from
`018db55ea1212e80d5e7942fef759d812b7dad59`. It defines typed command requests,
JSON envelopes, configuration precedence, exit codes and C-01..C-10 contract tests
before process, storage or media work. P02+ behavior remains out of scope.

Implementation on `feat/p01-public-contracts` now includes typed IDs, time/ranges,
crops, cursors, confidence and job transitions; the full reserved R0 parser; bounded
human/JSON/JSONL presentation; strict JSON input limits; immutable config precedence;
and four published schemas with compatibility examples. `setup check` is the only
executing operation. The local workspace passes 64 tests plus strict Clippy and
dependency advisory/licence/source checks before protected review.

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
executable discovery, incomplete process-tree cleanup and insufficient contract tests.
See B-01..B-11; do not describe current code as production hardened.

## Next action

Finish P01 through protected review and a record-only evidence update. Do not begin
P02 until the delivery ledger records P01 complete.
