# Changelog

All notable changes to VSift will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- P05 foreground disposable `ingest`, session list/status/renew/close/clean,
  explicit evidence-only or source-inclusive retain, and data-only bundle
  validation. Source and frame/audio artifacts use P03's private
  capability-scoped generations; a bounded index and held OS locks protect
  active or abandoned sessions during cleanup. Retained output reports
  process-crash-consistent publication under ADR 0013. The opt-in P05
  checkpoint runs real media through artifact publication, both export modes
  and source-preserving cleanup.
- P04 internal source snapshot and bounded FFprobe/FFmpeg media adapter with typed
  stream metadata, actual frame/audio timestamps, source identity and an opt-in
  real-media checkpoint. Project-owned synthetic fixtures include VFR, rotation,
  audio-track and malformed variants with independent provenance verification.
- Accepted ADR 0011 and scoped R1 as the managed industrial capability expansion:
  optional enrichment, source-grounded composition, explicit catalogue lifecycle,
  industrial worker growth and integrated qualification in P15-P20. R0 now has an
  explicit two-agent end-to-end release gate and MCP remains a later adapter.
- P03 native filesystem/lock feasibility experiments and a recorded OS/storage
  crash-qualification blocker. ADR 0010 now accepts ephemeral NTFS/APFS desktop
  qualification for P03 and defers strict Ubuntu/ext4 durable enablement to the
  P10/P11/P14 fault campaign.
- Began P03 implementation with typed durability requirements, qualified publication
  guarantees, non-wrapping storage generations, and an application gate that rejects
  unsupported durable requests before invoking the mutating session-store port.
- Added the first internal capability-scoped filesystem session-store adapter: it
  validates an existing owned root, serializes initialization with a stable OS lock,
  publishes an immutable checksummed generation zero, and verifies it before reuse.
  The adapter is not yet composed into a public command.
- Completed the internal P03 storage/coordination boundary in PR #42 with
  owned private-root provisioning, Unix owner/mode and Windows DACL validation,
  immutable root-wide weighted admission, shared/exclusive lifetime holds,
  generation-fenced publication, bounded integrity-chain recovery, and deterministic
  error/process-crash tests at every manifest and pointer boundary. Durable requests
  remain rejected before mutation and no session command is exposed. PR #43 also
  makes concurrent lock-contention tests wait against a bounded monotonic deadline
  instead of assuming a fixed number of scheduler yields.

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
