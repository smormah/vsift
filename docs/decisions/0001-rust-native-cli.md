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

