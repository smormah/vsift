# ADR 0011: R1 is the managed industrial capability expansion

- Status: Accepted
- Date: 2026-09-11
- Extends: ADR 0002, ADR 0004 and ADR 0005
- Does not resolve: the P15 catalogue/orchestration/provider choices

## Context

The initial plan named enrichment, reconstruction and indexing as later capabilities,
but did not give the second product slice a coherent release outcome or enforceable
delivery boundary. That ambiguity could either pull industrial persistence into R0 or
allow the core video-investigation journey to be deferred until after R0.

VSift needs both: a fully useful first release for a developer and a deliberate path
to operated, high-volume indexing. One-off desktop use must not leave an unmanaged
database behind, while server deployments need explicit durability, lifecycle,
recovery and scale qualification.

## Decision

R0 remains a complete agent-operated local-video investigation. Its release gate
includes dependency handling, ingest, supplied-transcript or local-ASR processing,
speech search, visual candidate refinement, source frame/audio retrieval, grounded
handoff and explicit cleanup/retention. Qualify the workflow through two independent
coding-agent clients with local shell and image access: OpenAI Codex and Claude Code,
or documented equivalent successors.

R1 is the managed industrial capability expansion described in
[`r1-industrial-capability-expansion.md`](../planning/r1-industrial-capability-expansion.md).
Reserve P15-P20 for R1 contracts/corpus, enrichment, source-grounded composition,
managed catalogue, industrial worker plane and integrated qualification.

Desktop ingestion remains disposable unless the caller explicitly selects a managed
catalogue or imports a retained bundle. SQLite is only a candidate embedded adapter
behind an application port. It is not selected here, is not a distributed queue, and
must not become mandatory desktop state. R1 may support horizontal workers through
separately qualified delivery and artifact-store adapters; public internet ingress,
mutually untrusted multi-tenancy and MCP remain R2 decisions by default.

R1 planning can continue while R0 is built. Runtime implementation may not begin
before P14 and the P15 decision/fixture/ledger gate. Every R1 profile must retain the
R0 functional and security regression suite.

## Consequences

- R0 cannot ship as only a scaffold, transcription wrapper or frame extractor.
- Industrial durability and scale are preserved as owned requirements rather than an
  informal backlog, but they do not silently enlarge active R0 packets.
- P15 must create the machine-readable R1 ledger, independent fixture truth, accepted
  backend/provider ADRs and calibrated performance/recovery targets.
- The primary integration remains CLI plus agent skill. Optional transports reuse the
  same application contracts and cannot fork business logic.
- This ADR changes release ownership only. It does not claim R0 or R1 implementation,
  accept a storage backend, or supply the missing P03 crash evidence.
