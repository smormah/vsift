# ADR 0006: Workspace publication and durability

- Status: Accepted
- Date: 2026-09-10
- Resolves: DEC-05, DEC-06, DEC-13

## Context

Concurrent CLI invocations and worker retries need recoverable state without enrolling
every desktop recording in a permanent catalogue. JSON files used as mutable shared
state would not provide a safe transaction boundary.

## Decision

Use an explicit application-owned workspace containing immutable artifacts, immutable
stage records and versioned manifest generations. One metadata writer publishes a new
generation under an OS-backed lock after validating its referenced objects. Readers
hold a stable committed generation. Ephemeral mode provides process-crash-consistent
publication; durable mode additionally performs the qualified platform flush and
atomic-publication sequence before reporting success.

Desktop sessions remain ephemeral by default. Worker durability requires an explicit
workspace on a qualified local filesystem. Cross-process admission and lifetime locks
protect work and cleanup. Source reads and output publication must use platform-safe
handle-relative operations. If the required guarantees cannot be demonstrated on a
target, the operation returns a typed unsupported-guarantee error.

## Consequences

P03 begins with a feasibility gate for NTFS, APFS and ext4 locking/publication behavior.
Failure triggers a replacement ADR considering a narrower profile or proven embedded
storage. Network filesystems and host/disk loss are outside R0's local durability claim.
A future catalogue consumes retained bundles rather than sharing this internal layout.
