# ADR 0010: Narrow storage qualification after the P03 feasibility gate

- Status: Accepted
- Date: 2026-09-11
- Partially supersedes: ADR 0006's cross-platform durable publication target
- Tracking: [P03 / issue #6](https://github.com/smormah/vsift/issues/6)

## Context and measured finding

The [P03 feasibility record](../planning/p03-storage-feasibility.md) distinguishes
safe API availability from proven crash durability. On local Windows 11 build
26200 / NTFS, cap-std 4.0.3 file flush and pointer replacement work. Its default
read-only directory handle fails `sync_all` with PermissionDenied (OS error 5),
but an explicit writable directory handle opened through safe standard-library
APIs successfully synchronizes. The default-handle error is therefore **not** a
reason to reject Windows durability. Its rename path does not expose write-through
publication; whether a correctly ordered writable-directory flush supplies the
required barrier still needs native fault evidence.

No disposable OS/storage fault campaign in this task has established acknowledged
durable publication on NTFS, APFS or ext4. The available local host is the user's
Windows workstation; the current CI jobs are ordinary hosted process tests, not
an owned restartable storage-fault harness. API success and process-kill experiments
cannot substitute for that evidence. Production adapter expansion stops at this
unmet gate; safe cross-platform implementation has not been shown impossible.

## Decision

Keep Windows/NTFS and macOS/APFS as desktop **ephemeral** storage qualification
targets for R0. A retained bundle is explicitly preserved from VSift cleanup but
does not acquire a strict OS-crash durability claim merely because it is retained.
Its result reports the effective publication guarantee.

Qualify R0 durable worker acknowledgement only on Ubuntu 24.04/ext4, conditional on
a reviewed publication ordering and an owned disposable OS/storage crash campaign.
Until that campaign passes, durable operations remain disabled on every profile.
Explicit durable requests return a typed unsupported-guarantee error before mutation;
VSift never silently downgrades them to ephemeral.

P03 may now implement the safe storage/coordination boundary and qualify ephemeral
publication on the desktop targets. P03 completion still requires its complete path,
permissions, admission, integrity, reader/writer and process-crash suites. It exposes
the durable contract only as a fail-closed unsupported capability. P10, P11 and P14
own the later Ubuntu/ext4 OS/storage crash qualification and durable enablement; R0
cannot release the strict-worker profile before that evidence passes.

Preserve explicit retention intent, source preservation, immutable generations,
stable OS lock anchors, private roots and disposable desktop defaults. Retained
exports on a profile without durable publication need a separately accepted
contract stating their actual guarantee; P05 must not silently promise durability.
No SQLite/catalogue, server host or P04+ feature is introduced by this decision.

## Alternatives considered

1. Retain ADR 0006's three-platform durable target and block all later packets until
   three owned fault environments exist. Rejected because desktop ephemeral sessions
   do not require that guarantee and the block prevents the functional R0 journey.
   Windows/APFS durable profiles may still be added later through independent evidence.
2. Evaluate a proven embedded storage component scoped to an explicitly requested
   workspace. It must still solve contained artifact access, lifecycle and
   publication ordering; a metadata database alone does not make external files
   durable. Deferred unless the narrow file protocol fails its remaining P03 tests;
   adoption needs its own dependency, licence and scope review.

## Consequences

The product can advance toward a useful desktop R0 without making an unproved
durability promise. Ephemeral still means process-crash-consistent committed
generations, not in-memory or disposable correctness. Explicit retention controls
lifecycle, not disk-flush strength.

P03 remains in progress until its production implementation and mapped tests pass;
accepting this ADR is not completion evidence. The first eligible successor remains
P04 only after P03 is merged complete. Durable worker requirements R-09/R-10 remain
open through P10/P11/P14, and the release documentation must distinguish supported
desktop sessions from the not-yet-qualified strict worker profile.
