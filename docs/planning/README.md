# VSift implementation blueprint

Status: proposed implementation baseline for maintainer review. Date: 2026-09-09.
Reviewed source revision: `df85f7065915145ab8b75b89756f0fa1e341a5f1`.
This is a design and delivery plan; its proposed capabilities are not implemented.

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
6. [Baseline review](baseline-review.md): observed gaps in the existing scaffold.

The source code describes what exists. Accepted ADRs describe existing architectural
decisions. This plan proposes additional contracts and release requirements; accepting
them requires resolving the decision register below before dependent implementation.
Existing ADRs remain intact. A reviewed design change gets a new ADR with links to
any decision it supersedes.

## Primary workflows

Desktop: a developer supplies a local QA recording and a question to their coding
assistant. The assistant checks prerequisites, opens a temporary session, imports or
creates a transcript, finds relevant spans, requests candidate frames, refines the
time range or crop, and cites the resulting evidence in its code investigation. It
closes the session or explicitly retains a bundle.

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
| R-15 | OCR, embeddings, VAD enrichment, small optional vision models | R1 | task-specific accuracy and resource evaluations |
| R-16 | Reliable scroll/pan composition with refusal and source-frame fallback | R1 | reconstruction ground truth |
| R-17 | Durable cross-video index, including optional SQLite adapter | R1/R2 | index consumer contract and rebuild tests |
| R-18 | Distributed queue service, tenancy, auth, quotas, remote object store | R2 | service threat model, failover and isolation tests |
| R-19 | Optional MCP adapter over published use cases | R1/R2 | adapter conformance tests |

R0 preserves scrolling sequences and supports detailed manual agent navigation. It
does not promise a stitched spreadsheet. OCR is useful but is not a condition for
the core image-reading workflow; a client with no image-reading capability must
report that limitation. R1 capabilities must not become mandatory runtime downloads.

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

The proposals below make implementation planning concrete. They need acceptance or
replacement at the planning checkpoint, not silent implementation as settled facts.

| ID | Proposed default / question | Must resolve before |
| --- | --- | --- |
| DEC-01 | R0 includes single-host worker execution and bounded batches; multi-tenant hosting remains external | P00 |
| DEC-02 | Windows x64 on a supported Windows release, macOS arm64, Linux glibc x64; exact minimum versions from clean-image qualification | P02/P13 |
| DEC-03 | CPU baseline whisper.cpp; propose multilingual base model after sample accuracy/latency evaluation; larger models optional | P06 |
| DEC-04 | Desktop idle TTL 24 hours, absolute lifetime 7 days; retained work never auto-expires | P03 |
| DEC-05 | Worker durable state must use an explicit workspace with a tested local filesystem | P03/P11 |
| DEC-06 | Immutable artifacts + versioned manifest commit; single metadata writer per session | P03 |
| DEC-07 | Dependency installs use approved pinned manifests in a per-user directory; no implicit elevation | P06 |
| DEC-08 | npm name ownership must be verified; scoped fallback if unavailable; published executable remains vsift | P13 |
| DEC-09 | Core namespace: setup, session, ingest, transcript, search, candidates, frame, audio, crop, bundle, job | P01 |
| DEC-10 | JSON v1 setup contract kept compatible; new operations use a documented v1 envelope | P01 |
| DEC-11 | Linux isolated worker qualification first; desktop confinement level is reported separately | P02/P11 |
| DEC-12 | Do not require speech diarization, OCR, embeddings, or stitching in R0 | P00 |
| DEC-13 | Validate native source-read and atomic-publish guarantees on each target; unsupported guarantees cause typed rejection | P03/P04 |

Existing decisions retained: Rust core, npm as distribution, CLI primary surface,
optional skill/MCP adapters, explicit persistence, source authority, and strict module
boundaries. No new paid inference provider is required.

## Completion and change control

The first iteration is complete only when all R0 requirements map to passing tests,
the qualification matrix is published, and each release gate in the verification
document has evidence. A green unit suite alone is insufficient.

Maintain requirement -> packet -> test -> result traceability. Add newly discovered
failure modes to the threat model and regression corpus in the same change. A
security review has finite scope; record residual risks, review date, versions, and
unsupported environments instead of claiming vulnerability-free software.

Planning checkpoint: review the decisions above and the packet sequence before
resuming feature implementation. This change adds planning records only.
