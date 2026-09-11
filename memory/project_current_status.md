# VSift current status

## Active

P03 / issue #6 is active on protected main
`b0d42e8cd6f051866f9a18da1d0d3e8a947b0f38`; P01/P02 are complete. PR #35
(`162ac7a`) is in review with the first production P03 contracts. It separates
requested durability from a qualified publication guarantee, prevents generation
wraparound and proves unsupported durability fails before the mutating port is called.
No filesystem adapter or storage command is exposed yet.

The R1 managed industrial capability boundary is accepted in ADR 0011 and PR #31
(`0cfdb407f805282995f326ca93c99bc7170eda04`). It reserves P15-P20 for
contracts/corpus, enrichment, source-grounded composition, managed catalogue,
industrial worker plane and integrated qualification, with milestone 2 and issues
#25-#30. This is roadmap work only: P15+ implementation remains ineligible until P14
and P15's decision/fixture/ledger gate.
R0 issues #15 and #17 now include the named two-client, supplied-transcript and
local-ASR end-to-end release evidence required by A-08/A-09.

FS-01: OS/storage crash qualification is absent. The default NTFS read-only
directory handle fails synchronization (OS error 5); an explicit safe writable
directory handle succeeds. That resolves the API-access issue, not durability
qualification. Strict durable enablement remains gated, while ephemeral adapter work
may resume. The development-only native spike records primitive containment, lock and
process-kill observations, not P03 completion. Accepted ADR 0010 assigns strict
Ubuntu/ext4 durability proof and
enablement to P10/P11/P14; durable requests must fail before mutation until then.
No storage operation is exposed yet.

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

### 2026-09-11 — P03 storage qualification profile

PR #33 squash-merged as `c2b3829d77279a32b3487ab1170f820f1d68eeb5`.
ADR 0010 permits P03 to implement process-crash-consistent ephemeral publication on
Windows/NTFS and macOS/APFS. Retention may preserve an ephemeral workspace from VSift
cleanup, but does not upgrade its durability guarantee. Explicit durable requests must
fail before mutation until P10/P11/P14 complete the owned Ubuntu 24.04/ext4 OS/storage
crash campaign and enable the strict worker profile.

This is a qualification decision, not P03 completion. All protected checks passed;
local evidence was 92 passing tests plus fmt, strict Clippy, governance and diff checks.

### 2026-09-11 — R1 industrial capability scope

PR #31 squash-merged as `0cfdb407f805282995f326ca93c99bc7170eda04`.
ADR 0011 makes R1 the explicit managed industrial expansion while keeping R0 a fully
functional local-video investigation. R0 now requires supplied-transcript and local-ASR
paths through named Codex and Claude Code clients; a scaffold-only, transcript-only or
frame-only build cannot qualify. R1 requirements R-15..R-20 and packets P15..P20 own
optional enrichment, source-grounded reconstruction, explicit managed catalogue,
industrial worker growth and integrated production qualification. SQLite remains a
candidate single-node adapter; MCP and public hostile multi-tenancy remain R2.

Milestone 2 and issues #25-#30 preserve the future work. R0 issues #15/#17 carry the
new A-08/A-09 end-to-end gate. All PR checks passed across three OS quality jobs,
governance, documentation, strict-worker boundary, dependency policy/review, Rust
analysis and CodeQL. Local evidence was 92 passing tests plus fmt, strict Clippy,
governance, warning-denied rustdoc and diff checks.

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

Merge the P03 storage-contract increment, then implement the capability-safe
filesystem `SessionStore`: ephemeral NTFS/APFS publication, safe containment and
stable locks, followed by admission. P03 remains in progress and P04 is ineligible
until the complete adapter and mapped tests merge. Do not pull P04+ or P15+ behavior
into P03. The Ubuntu/ext4 OS/storage crash campaign remains mandatory in P10/P11/P14.
