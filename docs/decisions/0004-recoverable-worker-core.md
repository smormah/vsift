# ADR 0004: Recoverable processing core for desktop and worker use

- Status: Proposed
- Date: 2026-09-09
- Extends: ADR 0002 and ADR 0003 without changing disposable desktop defaults

## Context

VSift must support repeated concurrent execution on desktops and within backend
workers. A future indexing service needs bounded processing, recovery and explicit
durability even before a database-backed catalogue exists. The current setup scaffold
does not provide those properties.

## Proposed decision

Build one Rust application core with a desktop CLI and a noninteractive single-host
job/batch interface. Coordinate cooperating processes through OS-backed locks and
bounded admission, not only in-process async primitives. Store immutable artifacts
and versioned session generations, with a qualified commit/recovery protocol.

Keep desktop sessions ephemeral by default. Require an explicit managed workspace
for durable worker recovery and retained outputs. A supervisor owns external queue
acknowledgement, tenant isolation, global quotas and remote storage. Local durable
state does not claim survival of disk loss or cluster-wide exactly-once execution.

Publish capabilities and effective resource/isolation/durability controls. Required
unsupported guarantees cause a typed rejection. Qualify platform implementations
before advertising them. See [contracts](../planning/architecture-and-contracts.md)
and [test gates](../planning/verification.md).

## Alternatives to evaluate in the feasibility packet

- A memory-only engine: simple but cannot recover work or coordinate separate calls.
- Mandatory global SQLite: unnecessary lifecycle coupling for disposable desktop use.
- A narrow file commit protocol: proposed, but requires native crash/locking evidence.
- A proven embedded persistence component scoped to explicit workspaces: reconsider
  if the file protocol cannot meet the required guarantees without becoming a database.
- A distributed service from the outset: deferred until queue, tenancy and storage
  requirements are concrete; keep integration contracts usable now.

## Consequences

The first release needs more than media wrappers: process supervision, state ownership,
resource admission and fault tests are prerequisites. Backend integrations reuse the
core without importing filesystem internals. The supported guarantee matrix can
differ across desktop and strict worker profiles. No network daemon or persistent
catalogue is introduced by this ADR.

Acceptance depends on the planning decision register and P00/P03 feasibility evidence.
