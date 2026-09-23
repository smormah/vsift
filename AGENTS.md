# VSift contributor instructions

These rules apply to every change in this repository, including changes made by AI coding agents.

## Session handoff and implementation planning

Before implementation, read `memory/TODO.md`, `memory/project_current_status.md`,
`docs/planning/delivery-ledger.json`, the relevant packet in
`docs/planning/implementation-work-packets.md`, and its linked contracts/tests.
Follow `docs/planning/delivery-governance.md`; do not treat a documented future
capability as implemented. Work only on an active packet whose predecessors are complete.

## Product boundaries

- VSift is local-first and provider-neutral.
- Original audiovisual media is authoritative; generated metadata is evidence assistance only.
- Desktop investigations are disposable by default. Persistence requires explicit user intent.
- Media engines and ML runtimes are adapters. Do not embed provider behaviour in the domain.
- The stable CLI and its versioned JSON contracts are public APIs.

## Architecture

- Dependencies point inward: `domain <- application <- infrastructure <- cli`.
- Crates expose published contracts only. Never import another crate's internal modules.
- Business concepts have one home in the domain. Orchestration belongs in application use cases.
- Infrastructure owns filesystem, process, network, database, and provider details.
- The CLI is a composition and presentation layer; it contains no business rules.
- Expected failures use typed enums and `Result`. Do not erase application errors into strings.
- Constructors receive dependencies explicitly. Do not use service locators or mutable global state.

## Rust standards

- Stable Rust only; the repository toolchain and MSRV are explicit.
- `unsafe` is forbidden in VSift crates. A future exception requires an accepted ADR and a narrowly isolated crate.
- Do not use `unwrap`, `expect`, `panic`, `todo`, or `unimplemented` in production or test code.
- Prefer owned, readable domain values over lifetime-heavy or prematurely zero-copy designs.
- Use exhaustive enums and newtypes instead of boolean switches and magic strings.
- Keep asynchronous code at I/O boundaries; domain logic remains synchronous and deterministic.
- External commands must use explicit executable and argument APIs. Never invoke a shell or concatenate command strings.
- Every public item has useful rustdoc. Explain why, not what, for non-obvious implementation decisions.

## Security and data handling

- Treat video paths, filenames, transcripts, OCR, model output, and tool output as untrusted input.
- Never interpolate untrusted input into a shell command.
- Validate and canonicalize filesystem boundaries before copy, move, retention, or cleanup operations.
- Automatic cleanup is restricted to positively identified VSift-owned temporary directories. Never delete source media.
- Downloads require HTTPS, pinned provenance, cryptographic integrity verification, bounded sizes, and safe extraction.
- Do not log secrets, full sensitive transcripts, or unnecessary absolute user paths.
- New dependencies require a clear need, compatible licence, active maintenance assessment, and `cargo deny` review.
- Dependency licence and duplicate-version review includes development-only feasibility candidates; preserve `licenses.include-dev` and `bans.multiple-versions-include-dev` in `deny.toml`.

## Quality gates

Before completing a change, run:

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -p vsift-governance -- check
```

Add tests at the lowest useful layer. Public CLI JSON changes require contract tests and documentation.

## Documentation is part of the change

Architecture, public behaviour, storage lifecycle, provider configuration, security assumptions, and contributor workflow changes must update the corresponding repository documentation and ADR in the same change.

Rewrite the two project memory files in the same change so they state the current
position in plain English: what works, what is in progress, open decisions and known
issues. They are current-state documents within the governance checker's size
limits, not logs. History lives in git, `CHANGELOG.md`, qualification records and
`docs/history/`. Say whether an increment or a whole packet is complete. Put
verification evidence in the pull request description rather than a follow-up pull
request. Findings close only with implementation and regression-test evidence.
Preserve accepted ADRs; record superseding decisions explicitly.

Do not leave unexplained TODO comments. Track deferred work in a GitHub issue and reference the issue from the code only when a local marker is necessary.
