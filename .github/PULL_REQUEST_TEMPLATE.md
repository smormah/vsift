## Purpose

<!-- What problem does this change solve? -->

## Delivery traceability

- Packet: <!-- P00-P20, or N/A with a concrete reason -->
- Requirements: <!-- R-01 etc. -->
- Tests: <!-- C-01 etc. -->
- Threats/findings: <!-- SEC-01 / B-01 etc., or None after review -->
- Issue: <!-- Fixes #... -->

<!-- Confirm that the packet's predecessors are complete in delivery-ledger.json. -->

## Design

<!-- Explain the approach and important alternatives considered. -->

### Scope control

<!-- List the packet outcome and explicit exclusions. Explain any ledger/ADR change. -->

## Security and privacy

<!-- Describe effects on paths, processes, downloads, media, transcripts, retention, or logs. Write "None" only after reviewing these boundaries. -->

## Verification

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --workspace --locked`
- [ ] New or changed behaviour has appropriate tests.
- [ ] Public JSON and exit-code changes have contract tests.
- [ ] `cargo run --locked -p vsift-governance -- check`
- [ ] The delivery ledger records accurate status and completion evidence.

## Documentation

- [ ] README and contributor documentation are current.
- [ ] Architecture documentation or an ADR was updated when required.
- [ ] `CHANGELOG.md` describes the user-visible change.
- [ ] `memory/TODO.md` and `memory/project_current_status.md` are current.
