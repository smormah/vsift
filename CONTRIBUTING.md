# Contributing to VSift

Thank you for helping make technical video evidence accessible to AI coding agents.

## Before starting

- Search existing issues and pull requests.
- Open an issue before substantial behavioural or architectural work.
- Read the [delivery governance](docs/planning/delivery-governance.md) and
  [delivery ledger](docs/planning/delivery-ledger.json). Implementation work must
  belong to the earliest active packet whose predecessors are complete.
- Never attach confidential recordings, transcripts, credentials, or proprietary screenshots.
- Report suspected vulnerabilities privately according to [SECURITY.md](SECURITY.md).

## Development workflow

1. Fork and clone the repository.
2. Create a focused branch from `main`.
3. Make one coherent change with tests and documentation.
4. Run the required checks in [Development](docs/development.md), including
   `cargo run --locked -p vsift-governance -- check`.
5. Open a pull request using the repository template.

Pull requests should explain the problem, the chosen design, security and privacy effects, test evidence, and documentation changes.

## Design expectations

- Preserve the dependency direction documented in [Architecture](docs/architecture.md).
- Keep external tools behind application ports.
- Use typed errors for expected outcomes.
- Keep one-off evidence disposable unless the user explicitly retains it.
- Treat media, paths, transcripts, and provider output as untrusted.
- Add an architecture decision record before changing a governing constraint.
- Never mark a packet complete without a merged commit and recorded verification evidence.

## Contribution licence

Unless explicitly stated otherwise, any contribution intentionally submitted for inclusion in VSift is licensed under `MIT OR Apache-2.0`, without additional terms or conditions.

## Review

Maintainers may request changes for correctness, security, maintainability, compatibility, test coverage, or documentation. Approval is not a guarantee of immediate release.
