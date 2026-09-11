# VSift implementation blueprint

Status: accepted R0 implementation baseline with a scoped R1 expansion. Date: 2026-09-11.
Reviewed source revision: `df85f7065915145ab8b75b89756f0fa1e341a5f1`.
Its decisions are accepted through P00. P01 and P02 are complete. P03 is active:
its typed storage-guarantee contracts are merged and its first internal filesystem
session-store increment is in review. Public storage/session behavior and the complete
coordination/recovery proof remain unimplemented.

## Purpose and reading order

Deliver an agent-operated video investigation tool with a production-quality execution
core usable on desktops and inside server workers. Small implementation models should
receive a bounded work packet, explicit contracts, and independently checkable tests.
Model size is not a substitute for review, and instructions cannot guarantee the
accuracy of every model or discover every vulnerability.

Read these documents together:

1. [Scope and requirements](#requirements-and-release-boundary), below.
2. [Architecture and contracts](architecture-and-contracts.md): ownership, execution,
   state, files, command contracts, media semantics, and resource budgets.
3. [Threat model](security-threat-model.md): attack surfaces, controls, residual risks.
4. [Verification specification](verification.md): fixtures, test cases, failure injection,
   agent evaluation, benchmarks, and release gates.
5. [Implementation work packets](implementation-work-packets.md): ordered changes,
   dependencies, completion criteria, and review responsibilities.
6. [R1 industrial capability expansion](r1-industrial-capability-expansion.md):
   managed indexing, enrichment, reconstruction and operated worker growth.
7. [Baseline review](baseline-review.md): observed gaps in the existing scaffold.
8. [Delivery governance](delivery-governance.md), [traceability](traceability.md), and
   [qualification profiles](support-and-resource-profiles.md): enforceable scope controls.

The source code describes what exists. Accepted ADRs and the machine-checked delivery
ledger describe approved direction. Existing ADRs remain intact. A reviewed design
change gets a new ADR with links to any decision it supersedes.

## Primary workflows

Desktop: a developer supplies a local QA recording and a question to their coding
assistant. The assistant checks prerequisites, opens a temporary session, imports or
creates a transcript, finds relevant spans, requests candidate frames, refines the
time range or crop, and cites the resulting evidence in its code investigation. It
closes the session or explicitly retains a bundle.

That entire journey is the R0 product, not an architectural demonstration deferred
to R1. Release qualification runs it against real fixture videos through named OpenAI
Codex and Claude Code clients (or documented equivalent successors) with local shell
and image access. A general web chat without a local execution bridge is not claimed
to invoke the CLI. VSift itself remains independent of a hosted-model API.

Worker: a trusted supervisor stages an immutable input and invokes the same engine
non-interactively under a resource budget. VSift commits validated artifacts to an
explicit durable workspace and emits a versioned result. The supervisor transfers
the bundle to durable storage and acknowledges its external queue only after its
own storage commit. Duplicate deliveries can reuse a completed compatible job.

Indexing extension: a later consumer reads versioned bundle metadata and indexes
retained evidence. It does not need access to VSift's private workspace implementation.
Indexing, tenancy, remote queues, and databases are separate modules/hosts.

## Requirements and release boundary

R0 means the first public functional release, beyond the existing setup scaffold.
R1 means the next planned extension; R2 means a larger service integration.

| ID | Requirement | Release | Proof |
| --- | --- | --- | --- |
| R-01 | Local input, timestamped speech and visual evidence, repeated agent inspection | R0 | QA fixture end-to-end scenarios |
| R-02 | CLI first, versioned JSON, actionable typed failures, bounded pages | R0 | C test suite |
| R-03 | Setup checks, explicit installation plan/apply, BYO tools and models, offline use | R0 | D test suite and fresh-machine install |
| R-04 | Existing timestamped transcript or local whisper.cpp transcription | R0 | T test suite |
| R-05 | Candidates, source frame, neighbours, bursts, native-resolution crops, audio ranges | R0 | V test suite |
| R-06 | Source identity, actual timestamps, transformation lineage, explicit uncertainty | R0 | M/P/V suites |
| R-07 | Disposable sessions; explicit retained bundles; no hidden global media index | R0 | S and privacy tests |
| R-08 | Concurrent CLI processes and bounded parallel jobs on one host | R0 | X and load tests |
| R-09 | Cancellation, crashes, restart, idempotent work, atomic publication | R0 | P/S/X fault tests |
| R-10 | Explicit durable worker workspace and noninteractive job execution | R0 | worker acceptance and recovery tests |
| R-11 | CPU/memory/disk/process/output budgets with reported enforcement level | R0 | overload and sandbox tests |
| R-12 | Headless operation, structured logs, stable metrics/events for a supervisor | R0 | O and worker tests |
| R-13 | Agent skill, conservative context budget, evidence-backed QA handoff | R0 | A evaluation suite |
| R-14 | npm distribution and native binaries with provenance and supported-target tests | R0 | release qualification |
| R-15 | Optional bounded VAD/OCR/diarization/embedding/visual-tag enrichment | R1 | enrichment accuracy, resource, provenance and absence/failure evaluations |
| R-16 | Reliable scroll/pan composition with refusal and source-frame fallback | R1 | reconstruction ground truth and pixel provenance |
| R-17 | Explicit managed cross-video catalogue, including a qualified embedded adapter | R1 | lifecycle, migration, backup/restore, rebuild and privacy tests |
| R-18 | Industrial job control for repeated and horizontally scalable worker execution | R1 | lease/fencing, duplicate delivery, partition, host-loss and backpressure tests |
| R-19 | Production operations and security for qualified R1 deployment profiles | R1 | auth, observability, upgrade, disaster recovery, load/soak/chaos and runbooks |
| R-20 | Preserve the complete R0 workflow through two independent coding-agent clients | R1 | named Codex and Claude Code end-to-end regression evidence |

R0 preserves scrolling sequences and supports detailed manual agent navigation. It
does not promise a stitched spreadsheet. OCR is useful but is not a condition for
the core image-reading workflow; a client with no image-reading capability must
report that limitation. R1 capabilities must not become mandatory runtime downloads.
The detailed R1 boundary, P15-P20 graph and verification identifiers are defined in
the [industrial capability expansion](r1-industrial-capability-expansion.md). MCP and
public multi-tenant service ingress remain R2 concerns rather than displacing the
CLI/skill from the primary integration path.

### What fully functional means in R0

R0 is releasable only when a compatible coding agent can start with a local video and
finish with a timestamped transcript and source-grounded visual evidence without the
user manually extracting audio, transcribing it, or taking screenshots. Both the
provided-transcript and local-ASR paths must pass. Missing dependencies produce an
actionable setup plan and require explicit installation authority. Evidence references
remain resolvable after bounded paging/refinement, and session cleanup versus retention
is explicit. The agent may honestly report insufficient evidence; a scaffold-only,
transcript-only, or screenshot-only flow is not a functional R0 release.

### What server readiness means in R0

- Multiple OS processes may safely access one supported local state root. Work is
  coordinated through cross-process locks, not just an in-memory semaphore.
- A bounded batch invocation may run several independent jobs. Each job has isolated
  staging, typed outcomes, resource admission, and durable checkpoints when requested.
- Repeated delivery has at-least-once execution semantics with idempotent publication
  within the configured workspace. There is no cluster-wide exactly-once claim.
- R0 durable recovery assumes surviving, qualified local storage. Loss of a machine
  or disk requires the caller's replicated storage and queue; no local file format
  can provide that by itself.
- R0 is a worker executable/library core, not an HTTP server. Request traffic,
  distributed scheduling, tenant identity, and fleet autoscaling belong to the host.
  No RPS or throughput claim is valid until measured with a specified workload.
- A worker processes staged local assets and produces portable bundles. It does not
  place a shared JSON workspace on NFS/SMB or pretend that it is a distributed database.

These requirements preserve the original disposable desktop scope while giving the
processing engine a usable server contract from the start.

## Non-negotiable invariants

1. Source media is never modified or deleted by extraction, repair, rollback, or cleanup.
2. A reported completed artifact exists, validates, and has recorded provenance.
3. Readers see a committed generation; partial output never masquerades as complete.
4. Expiry does not authorize deleting a workspace actively held by a reader or writer.
5. No number of cooperating processes using one state root can bypass its admission limit.
6. Every loop, queue, provider output, retry, extraction request, and download is bounded.
7. Untrusted evidence cannot select executables, install dependencies, or change policy.
8. Optional enrichment failure preserves useful evidence and reports the missing coverage.
9. Durable acknowledgement follows the applicable storage commit, never precedes it.
10. Supported guarantees are reported explicitly; unsupported required guarantees fail closed.

## Decision register

P00 accepted these decisions on 2026-09-10. The linked ADRs are authoritative.

| ID | Accepted decision | ADR |
| --- | --- | --- |
| DEC-01 | R0 includes a bounded single-host worker core | [0004](../decisions/0004-recoverable-worker-core.md) |
| DEC-02 | Qualify Windows 11 25H2 x64, macOS 15 arm64 and Ubuntu 24.04 x64 targets | [0005](../decisions/0005-r0-scope-and-qualification-profiles.md) |
| DEC-03 | Evaluate CPU whisper.cpp multilingual base as the default candidate | [0005](../decisions/0005-r0-scope-and-qualification-profiles.md) |
| DEC-04 | Desktop idle TTL 24 hours and absolute lifetime seven days | [0005](../decisions/0005-r0-scope-and-qualification-profiles.md) |
| DEC-05 | Durable workers require an explicit qualified local workspace; R0 enablement is Ubuntu/ext4 only after P10/P11 qualification | [0010](../decisions/0010-storage-qualification-gate.md) |
| DEC-06 | Publish immutable artifacts through versioned manifest generations | [0006](../decisions/0006-workspace-publication-and-durability.md) |
| DEC-07 | Use explicit pinned per-user managed dependency plans | [0007](../decisions/0007-managed-runtime-provisioning.md) |
| DEC-08 | Keep executable `vsift`; recheck unscoped npm name before release | [0009](../decisions/0009-package-identity-and-distribution.md) |
| DEC-09 | Use the accepted R0 CLI namespace | [0008](../decisions/0008-cli-and-json-contract.md) |
| DEC-10 | Preserve setup v1 and use typed v1 envelopes for new operations | [0008](../decisions/0008-cli-and-json-contract.md) |
| DEC-11 | Qualify the strict Linux worker profile first | [0005](../decisions/0005-r0-scope-and-qualification-profiles.md) |
| DEC-12 | Keep diarization, OCR, embeddings and stitching outside R0 | [0005](../decisions/0005-r0-scope-and-qualification-profiles.md) |
| DEC-13 | Reject required filesystem/source guarantees that a target cannot prove; never downgrade durable requests | [0010](../decisions/0010-storage-qualification-gate.md) |

Existing decisions retained: Rust core, npm as distribution, CLI primary surface,
optional skill/MCP adapters, explicit persistence, source authority, and strict module
boundaries. No new paid inference provider is required.

[ADR 0011](../decisions/0011-r1-industrial-capability-expansion.md) accepts the release
boundary for the managed industrial expansion: R0 must already be functionally complete;
R1 owns P15-P20; MCP and public hostile multi-tenancy remain R2 by default. It does not
select the pending R1 backends/providers or resolve P03's storage qualification gate.

## Completion and change control

The first iteration is complete only when all R0 requirements map to passing tests,
the qualification matrix is published, and each release gate in the verification
document has evidence. A green unit suite alone is insufficient.

Maintain requirement -> packet -> test -> result traceability. Add newly discovered
failure modes to the threat model and regression corpus in the same change. A
security review has finite scope; record residual risks, review date, versions, and
unsupported environments instead of claiming vulnerability-free software.

P00 established decisions, fixture truth, traceability and anti-drift controls in
PR #18 (`924f6c5`); PR #20 (`3d7a7d2`) completed P01; and PR #22 (`4e9ef08`)
completed P02 through protected review. P03 feasibility evidence merged through
PR #24 (`cbc531e`). ADR 0010 accepts the narrower ephemeral desktop profile, so P03
production implementation may resume while strict durable enablement remains assigned
to P10/P11/P14. The evidence and limitations are recorded in the ledger and the
[P03 feasibility record](p03-storage-feasibility.md).
