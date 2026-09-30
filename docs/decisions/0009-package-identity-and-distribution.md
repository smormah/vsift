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

## 2026-09-30 note: P13 names, publication and trust signals

[ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md) (P13,
Proposed) records the maintainer's decisions: the unscoped `vsift` launcher over three
per-platform packages in one npm scope (`@<scope>/win32-x64`, `@<scope>/darwin-arm64`,
`@<scope>/linux-x64`); nothing published during P13's pull requests, then one quiet 0.x
pre-release under the dist-tag `next` at P13 completion; no crates.io publication in
R0; Sigstore and npm provenance as the only trust signals, without Authenticode or
notarization. The decision above is unchanged: an anonymous `npm view` found `vsift`
free on 2026-09-30, which is still not a reservation.

**The preferred scope `@vsift` is unavailable** (2026-09-30): npm refused the
organisation name `vsift`, because organisation names share the user-name namespace.
The fallback scope is the maintainer's choice, in this order of preference:
`@vsift-cli`, `@vsifthq`, `@vsiftdev`. It will be recorded here once chosen (it is
`@shongo`: see the next note); this
decision forbids a silent rename, so no package is published under a scope this ADR
does not name.

## 2026-09-30 note: the scope is `@shongo`, and `vsift` is held by a placeholder

**Scope.** The maintainer chose the company scope `@shongo` and created the npm
organisation `shongo` on 2026-09-30. Because the scope is a company's rather than the
project's, every package in it names the project, so the platform packages are
`@shongo/vsift-win32-x64`, `@shongo/vsift-darwin-arm64` and `@shongo/vsift-linux-x64`
(Sentry's `@sentry/cli-<platform>` packages follow the same pattern). Nobody types these
names: people install `vsift`, which pulls in the matching platform package, so the
scope appears only in lockfiles and `npm ls`. A scope cannot be renamed. If VSift later
moves to another owner, the platform packages are republished under a new scope and the
launcher's lookup changes. Users keep typing `vsift`.

**Placeholder. This supersedes the rule above** ("no placeholder package is published
during implementation", reaffirmed in the 2026-09-28 note). On 2026-09-30 the maintainer
decided to hold the unscoped `vsift` name now with a placeholder: version `0.0.0`,
holding only a README and `package.json`, with no code, no binaries and no install
scripts. The maintainer publishes it personally, with two-factor authentication.

*Why:* npm reserves a name only when something is published under it. The repository is
public, so the name is visible for the whole of P13. A publish also shows now, rather
than on release day, whether npm's similar-name check accepts `vsift` next to the
existing `sift` package. The scoped platform names need no placeholder: the `shongo`
organisation already owns them.

The rest of this ADR is unchanged. Nothing else is published during P13, and the first
real release is ADR 0023's 0.x pre-release under `next`. The placeholder stays the
`latest` dist-tag until a stable release replaces it, and its README says so.
