# VSift current status

## Active

P03 / issue #6 is active at the storage feasibility gate on
`codex/p03-storage-feasibility`. The clean predecessor is protected-main P02 evidence
commit `25c3aad01fc9e2fc391c5c016295df5aff61fbdd`; P01/P02 are complete.
Adapter expansion is conditional on ADR 0006's native containment/lock/flush evidence.

FS-01: OS/storage crash qualification is absent. The default NTFS read-only
directory handle fails synchronization (OS error 5); an explicit safe writable
directory handle succeeds. That resolves the API-access issue, not durability
qualification. Production adapter expansion is gated. The development-only native
spike records primitive containment, lock and process-kill observations, not P03
completion. See `docs/planning/p03-storage-feasibility.md` and proposed ADR 0010.
Accepted decisions/profile targets remain unchanged; no storage operation is exposed.

Spike commit: `5f15c7730607570663d739b7a4dd70a03947e500`, PR #24. Local Windows
checks pass (92 tests, fmt, strict clippy, governance and warning-denied rustdoc).
Post-test security review found cargo-deny's dev-only licence/duplicate checks
were off by default; they are now explicit, including a recorded MIT-0 review for
the already-locked P01 test dependency. No advisory or per-package exception added.
Follow-up `a262fa9bbabb4bebc6ebde581204c4dbe0a8186d` passed ten storage experiments
on each of NTFS, APFS and ext4 (CI run 34585520298); dependency policy/review passed
with development auditing enabled (run 34585520299). Safe writable/readable handle
probes resolve default-handle synchronization failures. Exact OS versions and the
remaining OS/storage crash gate are recorded; P03 is not complete.

## Complete

### 2026-09-11 — P02 secure process execution

PR #22 squash-merged as `4e9ef08df1e53019df7645edb0e493628a3e401a`.
P02 added canonical absolute provider resolution with explicit provenance; exact argv,
null stdin, canonical cwd and an empty-by-default environment; independent capped
stdout/stderr drains; one operation deadline and sticky caller cancellation; graceful
and forced cleanup through Windows Job Objects or Unix process groups; and an
effective-control report that separates lifecycle containment from hard isolation.
Strict mode fails before spawn without a qualified Linux worker boundary.

P-01..P-08 and C-05 are covered by 82 passing local Windows tests and protected
Quality checks on Ubuntu, Windows and macOS. The strict Ubuntu container additionally
passed read-only/no-network, process-group escape, CPU throttling, PID ceiling and
memory-limit checks. Governance, documentation, dependency policy/review, CodeQL and
Rust analysis passed.

Only setup dependency probing uses the boundary today. Managed runtime identity and
compatibility remain P06; durable coordination is P03/P10/P11; the worker host is P11.
Desktop process containment is not represented as a filesystem, network or resource
sandbox.

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

Only setup dependency probing is executable, now through the P02 supervisor. No
ingestion, transcription, visual extraction, session storage, managed installation,
queue, index, durable job or npm release exists yet. Ambient PATH remains a disclosed
bring-your-own fallback; managed identity/version trust is unimplemented.
See the dated B-01..B-11 dispositions; do not describe current code as production
hardened.

## Next action

Resolve P03 / issue #6's failed feasibility gate through explicit design acceptance
and qualified crash evidence. P03 remains in progress; no successor is eligible.
Do not pull P04+ media or P05+ session behavior forward.
