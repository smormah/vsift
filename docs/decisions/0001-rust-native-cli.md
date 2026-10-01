# ADR 0001: Rust native CLI with npm distribution

- Status: Accepted
- Date: 2026-09-09

## Context

VSift needs a portable CLI, safe process and filesystem handling, predictable resource use, and straightforward installation by AI coding assistants. npm is the preferred discovery and installation surface, but the media pipeline should not depend on an in-process JavaScript runtime.

## Decision

Implement VSift as a stable Rust application. Publish prebuilt native binaries for supported targets and provide an npm installer that selects and runs the correct binary. Also support appropriate native installation methods.

The public executable and npm package are both named `vsift`.

## Consequences

- Users do not need a Rust toolchain to run released binaries.
- Releases require a tested target matrix and artifact provenance.
- npm packaging is a distribution adapter rather than an application layer.
- Contributors use conventional Cargo commands and a pinned stable toolchain.


## 2026-10-01 note: names and installation methods as built (P13)

The decision stands: native binaries and an npm launcher that selects and runs the right
one. As built, the executable is `vsift` and the npm package that installs it is
**`vsift-cli`** (npm refused the unscoped `vsift`; [ADR 0009](0009-package-identity-and-distribution.md)
notes of 2026-09-30, [ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md)
decision A), over per-platform packages in the scope `@vsift`. The "appropriate native
installation methods" are, for R0, the archives on GitHub Releases; no installer (winget,
Scoop, Homebrew, a Debian package) is part of R0. The first release is a pre-release,
0.1.0, published on 2026-10-01 under the npm dist-tag `next` and as a GitHub pre-release
([`install.md`](../operations/install.md)); a stable release waits for P14.
