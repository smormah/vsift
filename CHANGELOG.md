# Changelog

All notable changes to VSift will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The engine can now prove that the selected FFmpeg and FFprobe actually work.
  It runs a small reviewed test video, built into VSift, through the same
  metadata, frame and audio steps an investigation uses and checks each result
  against the video's known answers. It can also identify whether a registered
  Whisper model is the reviewed pinned model. Nothing new appears on the command
  line yet: a later step runs this automatically before the first media
  operation.

### Changed

- Delivery is re-planned by ADR 0015. Managed dependency installation
  (`setup install` and its repair, list, rollback and remove lifecycle) moves from
  P06 to P13 and remains an R0 release requirement. P06 now closes on detection,
  bring-your-own selection, verification of the selected tools and manual guidance.
  Behaviour is unchanged: `setup install` still returns `COMMAND_NOT_IMPLEMENTED`.
- ADR 0016 commits VSift to an embeddable engine library and a published evidence
  contract, starting at P07.
- Internal reorganisation with no behaviour change: the v1 JSON response types moved
  from the CLI into a new `vsift-contract` crate that every future host will share.
  Command output, exit codes and schemas are unchanged.
- Internal reorganisation with no behaviour change: VSift's engine is now a Rust
  library, the `vsift` crate, and the command-line tool is a thin layer over it.
  Future hosts such as a worker or a desktop app will use the same library. Command
  output, exit codes and schemas are unchanged; the library API is not yet stable.
- The governance check now keeps the two session handoff files to a current-state
  size. The earlier day-by-day log is archived in `docs/history/`.

### Fixed

- Locks are now always released explicitly instead of by closing their file
  (issue #66). On Linux and macOS a child process started by another thread
  briefly holds copies of every open file, so a lock released only by closing
  could stay held for a moment and make an immediate retry report `BUSY`. This
  caused the intermittent CI failures and would have affected a busy worker.
  It applies to session, registration, admission, root-initialization,
  configuration, managed-install and managed-version locks.
- Per-user dependency configuration now reports `BUSY` only when the OS says
  another handle holds its lock. Other lock acquisition failures surface as
  storage I/O; an intermittent hosted `BUSY` test symptom remains under review.
- Registration explicitly releases its short-lived root initialization lock
  before returning the long-lived marker hold, preventing a duplicated file
  descriptor from prolonging root contention during an immediate bucket scan.

### Added

- P06 now fixes the Ubuntu managed candidate's compatibility policy in the
  reviewed catalogue: the exact checked-in F01 fixture, expected FFmpeg build
  and FFprobe identities, 64-KiB per-stream and transcript limits, 256-KiB
  generated-audio limit, 60-second media deadline, 180-second inference
  deadline, and 16-kHz mono audio contract. Catalogue
  completeness and the accepted plan digest bind these values; an invalid or
  changed policy cannot reuse prior acceptance. Production smoke execution and
  activation remain pending.
- P06 can now strictly and boundedly decode a saved `setup plan --json`
  document, rebuild the plan from current target, catalogue, configuration,
  probes and time, require the entire presentation to remain unchanged, and
  verify the separately supplied acceptance digest. Malformed or stale readable
  plans fail before transfer or filesystem mutation. A valid plan still ends in
  `COMMAND_NOT_IMPLEMENTED`; compatibility smoke and the installer transaction
  remain pending.
- P06's pinned multilingual `base` model now uses the same accepted-action
  authority as the Ubuntu archives. Exact model bytes can be copied into a
  private unactivated payload and runtime with bounded size/SHA-256 rechecks;
  unsafe names or mismatched review fail before mutation. The disposable
  Ubuntu workflow exercises the path against fresh publisher bytes. Provider
  compatibility and managed activation remain pending.
- P06 accepted Ubuntu actions can now be rebound to exact reviewed publisher
  source and passed through the owned archive/payload/runtime preparation path
  using the catalogue inventory itself. Changed action fields or mismatched
  staged bytes fail closed. This remains unactivated; raw-model staging,
  compatibility smoke and the public installer are still pending.
- P06 now records an exact Ubuntu 24.04 x86-64 reviewed catalogue for the
  pinned month-end FFmpeg/FFprobe build, whisper.cpp v1.9.2 CLI and multilingual
  `base` model. `setup plan` emits only currently needed actions with direct
  publisher URLs, pinned bytes/hashes, archive and installed-file inventories,
  licence/source disclosures, trust limits, private destination, and a
  deterministic state-bound acceptance digest. It stops new plans on 2028-08-01
  and returns typed managed-unavailable guidance elsewhere. Installation and
  compatibility preflight remain unavailable; the plan makes no legal-clearance
  claim.
- P06 published runtimes now hold shared per-version OS locks. A guarded
  transaction can atomically select an older published version for rollback and
  remove only an unselected version after obtaining its exclusive lock. Selected
  or live-held versions remain intact; a private tombstone makes interrupted
  exact-file removal retryable through metadata deletion and a lost response.
  A native child-process test proves a live hold blocks removal and abrupt
  process exit releases it. Public install, rollback and uninstall commands
  remain unavailable.
- P06 can now publish a fully rechecked prepared runtime under a canonical
  component/version identity and atomically select it with a hashed pointer while
  holding the root installation guard. Published versions retain exact manifests,
  regular-file identity, private modes and SHA-256 checks; interrupted pointer
  replacement is retryable and prior versions remain readable. This infrastructure
  primitive carries no catalogue, compatibility or plan-acceptance authority.
- P06 now has a root-wide managed installation guard backed by a private,
  single-link OS-locked file. Concurrent writers receive typed `Busy` without
  waiting or retrying; linked or incorrectly permissioned lock files fail
  closed. The guard serializes future transactions but grants no install authority.
- The opt-in disposable Ubuntu P06 qualification workflow now sends a freshly
  bounded and SHA-256-verified whisper.cpp archive through the production Rust
  owned-runtime layout check before running the separate candidate compatibility
  smoke. It still grants no catalogue, plan, activation or install authority.
- P06 can now copy a verified payload into a fresh private, unactivated
  `runtime.pending` directory with only reviewed regular-file aliases and
  selected Unix owner-executable modes. Every copy is bounded and rechecked;
  failed preparation removes only its owned runtime files. A pinned Ubuntu
  whisper.cpp archive passed this layout stage without binary execution.
- P06 `setup plan --profile` now performs a read-only configured/PATH executable
  diagnosis. Until a per-target managed artifact is qualified, its v1 result
  reports an unavailable managed path, no install actions or acceptance digest,
  and typed manual BYO steps. `setup install` remains reserved.
- P06 now composes a verified managed artifact with bounded raw tar, gzip/tar
  or XZ/tar selected-file staging under a fresh private payload directory.
  Selected files are rechecked before use; changed, linked or unexpected files
  block opening and cleanup removes only the reviewed selection. The payload
  remains unactivated and managed installation remains unavailable.
- P06 can transfer an exact reviewed publisher artifact over direct HTTPS into
  the private unactivated stage. Immutable GitHub release and Hugging Face
  model routes admit only their reviewed CDN redirect, with bounded deadlines,
  cancellation, whole-artifact size/SHA-256 verification and no resume.
  Managed installation remains unavailable.
- P06 now has a positively marked private per-user managed root and one-artifact
  staging transaction. It verifies exact reviewed bytes on import and again
  before archive use, removes its own stage after failed import, and refuses
  unmarked roots or unexpected staging entries. This is an infrastructure
  boundary; managed installation remains unavailable.
- P06 bounded archive adapters can stage an exact reviewed regular-file
  selection into an empty private directory capability. Staging uses portable
  flat names, create-new/no-follow writes and private modes, ignores archive
  links/directories/modes, and removes files it created when any later archive
  or compression check fails. This infrastructure primitive does not activate
  managed installation.
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
