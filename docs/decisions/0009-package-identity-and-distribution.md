# ADR 0009: Package identity and distribution

- Status: Accepted
- Date: 2026-09-10
- Resolves: DEC-08

## Context

The native executable and npm discovery name should be memorable and consistent, but
registry availability can change and package publication has supply-chain consequences.

## Decision

The executable remains `vsift`. Use the unscoped npm name `vsift` if it can be reserved
and configured securely at release time; the registry returned not-found when checked
on 2026-09-10. This observation is not ownership or a reservation. If unavailable,
use a maintainer-approved scoped package while keeping the executable name.

Distribution uses prebuilt native artifacts and a thin launcher. Publishing requires
the protected release workflow, short-lived trusted publishing where supported,
provenance, SBOM/notices, target selection tests and an explicit release approval.

## Consequences

P13 rechecks the registry and verifies account/namespace ownership before publication.
No placeholder package is published during implementation. Installation without Rust
is a release qualification requirement.
