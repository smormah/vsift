# VSift current status

## Active

P05 issue #8 is in progress from protected main `8741f57dfc5b45c8cb4d3f1f7791d0f1d143c627`.
The implementation branch opens disposable source-bound sessions, commits
renew/close/artifacts through P03 generations, keeps a bounded root-local
registration index for list/cleanup, and validates explicit evidence-only or
source-inclusive retained bundles. Local cross-process and native
FFmpeg/FFprobe P05 checkpoint tests pass. Protected checks, merge and ledger
completion evidence are pending; P05 is not complete and P06 is not eligible.
Strict durable requests still fail before mutation.

P04 merged through protected PR #44 as `4fc859b3344bd47c254dd9da9cac72f5ad3d61d5`.
The ledger completion evidence is recorded in this follow-up. Issue #7 closes when
that record merges. P05 is next eligible, but its lifecycle has not started.

P03's implementation completed through protected PR #42 as
`3eef9b7ac3bcfe092d82137ccf2aa9aa084aca4f`; this evidence-only follow-up records
the ledger and handoff state, and issue #6 is closed. The filesystem `SessionStore`
remains internal and explicit durable requests still fail before mutation. No
storage/session command is exposed.

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

The incremental R0 E2E spine is tracked in `docs/planning/e2e-test-spine.md` and issue
#40. P04 bootstraps the executable real-media runner, P07 will attach both
transcript paths, P09 will complete the mechanical video-to-evidence journey, and P12
will add named Codex/Claude trials. It is opt-in at major checkpoints and mandatory at
release; the planning framework is not implementation evidence.

Spike commit: `5f15c7730607570663d739b7a4dd70a03947e500`, PR #24. Local Windows
checks pass (92 tests, fmt, strict clippy, governance and warning-denied rustdoc).
Post-test security review found cargo-deny's dev-only licence/duplicate checks
were off by default; they are now explicit, including a recorded MIT-0 review for
the already-locked P01 test dependency. No advisory or per-package exception added.
Follow-up `a262fa9bbabb4bebc6ebde581204c4dbe0a8186d` passed ten storage experiments
on each of NTFS, APFS and ext4 (CI run 34585520298); dependency policy/review passed
with development auditing enabled (run 34585520299). Safe writable/readable handle
probes resolve default-handle synchronization failures. Exact OS versions and the
remaining OS/storage crash gate are recorded. That gate is P10/P11/P14 work and does
not invalidate P03's completed ephemeral profile.

## Complete

### 2026-09-12 — P04 source and media primitives

PR #44 squash-merged as `4fc859b3344bd47c254dd9da9cac72f5ad3d61d5`. P04 adds
an internal held no-follow private source snapshot with SHA-256 identity and a
restricted, bounded FFprobe/FFmpeg adapter. Typed metadata includes selected stream
indexes, orientation, codec support and normalized timeline origin. Frame and audio
results preserve observed PTS and displayed dimensions. Project-owned F01-F10/F12
media and F11 malformed/parser variants carry same-build reproducible provenance;
an independent verifier checks 11 clips, seven malformed variants, timestamps and
selected pixels. The opt-in real-media checkpoint passed seven scenarios on native
FFmpeg 9.0 Windows/NTFS and reports P05-P14 plus the complete journey as
`not_implemented`. Local fmt, strict Clippy, workspace tests, warning-denied rustdoc,
governance and cargo-deny passed. Protected Quality passed on Ubuntu, Windows and
macOS; Governance, Documentation, strict-worker, dependency policy/review, CodeQL
and Rust analysis passed. ADR 0012 records desktop decoder isolation limits and
P11/P14 qualification ownership. No public media/session command was added.

### 2026-09-11 — P03 ephemeral storage and coordination

PR #42 squash-merged as `3eef9b7ac3bcfe092d82137ccf2aa9aa084aca4f`.
P03 now provides owned private-root provisioning, Unix owner/mode and Windows DACL
validation, immutable weighted admission, shared/exclusive cross-process lifetime
holds, writer fencing, monotonic immutable generations, a bounded SHA-256-linked
manifest chain, typed integrity/version/access/capacity failures and deterministic
recovery at every manifest/pointer write, flush and rename boundary. Same-operation
retry is idempotent, stale generations conflict, and unpublished attempts are ignored.

Local evidence is 128 passing tests, with three internal child entries intentionally
ignored by the ordinary runner and launched by watchdog-bounded parents, plus fmt,
strict Clippy, warning-denied rustdoc, governance and cargo-deny. Protected Quality
passed on Ubuntu, Windows and macOS; Governance, Documentation, strict-worker,
Dependency policy/review, CodeQL and Rust analysis passed. The completed profile is
process-crash-consistent ephemeral desktop storage only. Strict Ubuntu/ext4
OS/storage crash durability remains P10/P11/P14 work and durable requests remain
fail-closed.

PR #43's first Windows evidence run exposed that two lock-contention tests treated
100 scheduler yields as a timing budget. They now retry `Busy` against a five-second
monotonic deadline with 10 ms intervals. Both passed ten consecutive targeted runs,
then the full 128-test local suite and strict Clippy passed again.

### 2026-09-11 — P03 storage contract and initializer increments

PR #35 squash-merged as `9ee3c048e1460008cd4f6c3e16dc23f78115ad0d`; PR #36
squash-merged as `65fe00c3d43405a6ed5c8bda8ae50a2896e80d65`. These focused
increments establish the durability preflight, non-wrapping generations, typed storage
failures and the first internal capability-scoped generation-zero initializer. The
adapter uses held-directory relative operations, no-follow/single-link checks, stable
OS locking, bounded strict metadata and SHA-256 verification before idempotent reuse.

Local evidence is 105 passing tests plus fmt, strict Clippy, governance, warning-denied
rustdoc and dependency policy. PR #36 passed every protected job on Windows, macOS and
Ubuntu after a Unix-only needless-return lint was found and corrected. A later Windows
run exposed time-only fixture-root naming as intermittently non-unique under parallel
tests; PR #38 added a process-local monotonic discriminator and an identical-timestamp
regression test. The corrected suite passes 106 local tests and every protected job.
At that increment, P03 remained active: root provisioning/Windows ACL qualification,
arbitrary fault recovery, later generation publication, read/write holds and admission
were not yet complete. PR #42 subsequently completed them as recorded above.

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

Start P04 only through its governed packet workflow. Do not pull P05+ or P15+ behavior
into P04. The Ubuntu/ext4 OS/storage crash campaign remains mandatory in P10/P11/P14.
