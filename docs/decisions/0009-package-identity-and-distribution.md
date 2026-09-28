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

## 2026-09-28 note: launcher pattern and name checklist

The maintainer set the shape of the thin launcher and the list of names to hold before
release; both live in P13 of
[`implementation-work-packets.md`](../planning/implementation-work-packets.md)
("P13 launcher boundary" and "P13 name checklist"). The launcher is the `vsift` npm
package over per-platform packages declared as `optionalDependencies`, with no install
scripts in any package, so installation works the same under npm, pnpm, Yarn and Bun
and downloads nothing else. This refines the decision above and changes nothing in it:
no placeholder package, and a name counts as held only once the release publishes it.
