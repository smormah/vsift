# Changelog

All notable changes to VSift will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Accepted ADR 0011 and scoped R1 as the managed industrial capability expansion:
  optional enrichment, source-grounded composition, explicit catalogue lifecycle,
  industrial worker growth and integrated qualification in P15-P20. R0 now has an
  explicit two-agent end-to-end release gate and MCP remains a later adapter.
- P03 native filesystem/lock feasibility experiments and a recorded OS/storage
  crash-qualification blocker. Production storage remains gated; proposed ADR 0010
  records a narrower qualification option without accepting it.

- Initial Rust workspace and architectural boundaries.
- Read-only `vsift setup check` runtime diagnostic with versioned JSON output.
- Contributor, security, governance, and automation foundations.
- Detailed proposed implementation blueprint, source baseline review, threat model,
  verification matrix and work packets for desktop and server-worker execution.
- Accepted R0 architecture decisions, qualification/resource profiles, synthetic
  fixture truth, GitHub packet backlog and CI-enforced anti-drift delivery ledger.
- Published the typed v1 R0 command namespace, JSON and JSONL terminal envelopes,
  stable errors/exits, configuration precedence, schemas, and compatibility examples.
- Added domain contracts for identifiers, source time/ranges, crops, paging cursors,
  confidence/provenance metadata, and legal job terminal transitions.
- Replaced environment-dependent CLI assertions with deterministic contract,
  compatibility, boundary, and property tests. Reserved operations fail explicitly
  without claiming their later implementation.
- Routed external setup probes through a shell-free process supervisor with canonical
  executable provenance, an allowlisted environment, bounded concurrent output,
  shared deadlines, caller cancellation, descendant cleanup, and truthful reporting
  of process containment versus strict worker isolation.
