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

[ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md) (P13;
Proposed when written, Accepted 2026-10-01) records the maintainer's decisions: the unscoped `vsift` launcher over three
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
`@vsift` after all: see the next note); this
decision forbids a silent rename, so no package is published under a scope this ADR
does not name.

## 2026-09-30 note: the scope is `@vsift`, and `vsift` is held by a placeholder

**Scope: `@vsift`, owned by the maintainer.** The "not available" message above was
not a refusal. The maintainer's first submission created the organisation `vsift`, and
a repeated submission then reported the name as taken. The maintainer's npm account
lists the organisation `vsift` as its own (2026-09-30). The platform packages are
therefore the originally planned `@vsift/win32-x64`, `@vsift/darwin-arm64` and
`@vsift/linux-x64`.

*Correction history, same day:* before the maintainer saw that `vsift` was theirs,
they chose the company scope `@shongo` and created that organisation, and #235
recorded it with packages `@shongo/vsift-…`. The maintainer then chose `@vsift`
instead. It is the project's own name, so handing the project to another owner means
handing over one organisation and republishing nothing. `@shongo` stays a company
organisation and holds no VSift package. Nobody types a platform package name: people
install `vsift`, which pulls in the matching platform package, so the scope appears
only in lockfiles and `npm ls`.

**Placeholder. This supersedes the rule above** ("no placeholder package is published
during implementation", reaffirmed in the 2026-09-28 note). On 2026-09-30 the maintainer
decided to hold the unscoped `vsift` name now with a placeholder: version `0.0.0`,
holding only a README and `package.json`, with no code, no binaries and no install
scripts. The maintainer publishes it personally, with two-factor authentication.

*Why:* npm reserves a name only when something is published under it. The repository is
public, so the name is visible for the whole of P13. A publish also shows now, rather
than on release day, whether npm's similar-name check accepts `vsift` next to the
existing `sift` package. The scoped platform names need no placeholder: the `vsift`
organisation already owns them.

The rest of this ADR is unchanged. Nothing else is published during P13, and the first
real release is ADR 0023's 0.x pre-release under `next`. The placeholder stays the
`latest` dist-tag until a stable release replaces it, and its README says so.

## 2026-09-30 note: npm refused `vsift`; the launcher package is `vsift-cli`

**npm refused the unscoped name.** When the maintainer published the placeholder
`vsift@0.0.0` of the previous note, npm answered E403, "Package name too similar to
existing packages sift, tsify". That placeholder was never published, so the unscoped
`vsift` is not held and will not be used.

**The launcher package is `vsift-cli`, held by the maintainer's placeholder since
2026-09-30.** The maintainer published `vsift-cli@0.0.0` (README and `package.json` only;
no code, binaries or install scripts), and npm accepted it; an anonymous `npm view
vsift-cli` shows `0.0.0` as `latest`. It stays `latest` until a stable release replaces
it, and the 0.x pre-release goes under `next` (ADR 0023 decision B), so users install
`vsift-cli@next` until then. **The executable stays `vsift`**: the package's `bin` entry
is `vsift`, so people type `npm install --global vsift-cli` or `npx vsift-cli` and then
run `vsift`. **The scope stays `@vsift`**, with `@vsift/win32-x64`, `@vsift/darwin-arm64`
and `@vsift/linux-x64` unchanged. This supersedes the unscoped `vsift` launcher of ADR 0023
decision A and the `vsift` placeholder of the note above; ADR 0023 carries the amendment.

**Lesson.** An anonymous not-found from `npm view` (2026-09-10 and 2026-09-30) was not
availability: npm checks a new name's similarity to existing packages only when it is
published, so the only proof that a name can be used is a successful publish. Future name
decisions are settled by a placeholder publish, not by a lookup.

## 2026-10-01 note: names as built, and their state

The 2026-09-28 note's "the `vsift` npm package" and the Consequences' "no placeholder
package" are superseded by the two notes of 2026-09-30: the launcher package is
**`vsift-cli`** (command `vsift`), the platform packages are `@vsift/win32-x64`,
`@vsift/darwin-arm64` and `@vsift/linux-x64`, and the maintainer published the single
placeholder `vsift-cli@0.0.0`. Read-only checks on 2026-10-01 (anonymous registry reads;
no setting changed): `vsift-cli` has that one version, published 2026-09-30 at 21:59 UTC,
as `latest`, with no command; the three `@vsift/...` packages are not found, because the
0.x pre-release will first publish them.

**Decided 2026-10-01 (maintainer): path A.** [`release.md`](../operations/release.md)
section 6.2 step 5 offered two ways to make the three platform packages exist before
their trusted publishers could be configured. The maintainer chose path A: three more
`0.0.0` placeholders, published personally with two-factor authentication, which extends
the placeholder exception above from the launcher to the three platform packages. They
hold only a README and a `package.json` (no code, no binary, no scripts, no author) and
were published on 2026-10-01 (`@vsift/win32-x64` at 17:44 UTC, `@vsift/linux-x64` at
17:45, `@vsift/darwin-arm64` at 17:46). Path B, a short-lived token in the protected
`release` environment, was not used, so no npm token exists anywhere. The trade-off the
maintainer weighed: everything real is still published only by the Release workflow; the
manual placeholders exist because npm's trusted-publisher setting lives on a package's
own settings page and these packages did not yet exist (whether npm offers it before a
first publish was not confirmed). The trusted publishers were then configured on all four
packages with "npm publish" allowed; whether to move to staged publishing is
[issue #246](https://github.com/smormah/vsift/issues/246), deferred by the maintainer on
2026-10-02 until after R1 or the public announcements.

npm's own registry record of a package lists the publishing account (its `maintainers`
and `_npmUser` fields). VSift's manifests name nobody, and the governance check and
`npm-verify` enforce that.

## 2026-10-01 note: the pre-release is published

The notes above that say nothing real is published, or that the three `@vsift/...` packages
do not exist, describe the days before this one and are superseded. On 2026-10-01 the
Release workflow, started by the maintainer on the tag `v0.1.0` and approved in the protected
`release` environment, published `vsift-cli@0.1.0`, `@vsift/darwin-arm64@0.1.0`,
`@vsift/win32-x64@0.1.0` and `@vsift/linux-x64@0.1.0` under the dist-tag `next`, through
npm trusted publishing with npm provenance and no stored token, and created the GitHub
pre-release `v0.1.0` with its archives. `latest` is still the `0.0.0` placeholder on all
four packages, so users install `vsift-cli@next` until a stable release. The first publishing
attempt failed with `ENEEDAUTH` before anything was published, because (as the maintainer
reported) the trusted-publisher connections had not been completed; a second dispatch
after they were completed succeeded. The account is in
[the P13 record](../planning/p13-distribution.md) ("First publish") and in the completion
note of [ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md), which
is now Accepted. This decision's other parts are unchanged: the executable is `vsift`,
publishing is only through the protected release workflow with provenance, and no crate is
published to crates.io.

The Consequences' "P13 rechecks the registry and verifies account/namespace ownership
before publication" was done by the placeholders (the lesson stands: only a successful
publish proves a name), and "installation without Rust is a release qualification
requirement" is met for the npm path by P13's twelve-job matrix and one install from the
real registry, with the clean-machine install across package managers left to P14.
