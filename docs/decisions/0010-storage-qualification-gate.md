# ADR 0010: Narrow storage qualification after the P03 feasibility gate

- Status: Proposed — requires maintainer design acceptance; not in force
- Date: 2026-09-11
- Would partially supersede: ADR 0006's cross-platform durable publication target
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

## Proposed decision

If three-platform crash qualification cannot be supplied for P03, keep
Windows/NTFS and macOS/APFS as desktop **ephemeral** storage qualification
targets. Permit future durable worker qualification only on Ubuntu 24.04/ext4,
conditional on a reviewed ordering and disposable OS/storage crash campaign.
Explicit durable requests on unqualified profiles must return a typed
unsupported-guarantee error before mutation; never downgrade to ephemeral.

This proposal does not approve even ephemeral support today. The complete P03
path, permissions, admission, integrity, reader/writer and process-crash tests
remain required before that implementation ships. Durable acknowledgements remain
disabled everywhere until the full qualification evidence exists.

Preserve explicit retention intent, source preservation, immutable generations,
stable OS lock anchors, private roots and disposable desktop defaults. Retained
exports on a profile without durable publication need a separately accepted
contract stating their actual guarantee; P05 must not silently promise durability.
No SQLite/catalogue, server host or P04+ feature is introduced.

## Alternatives and acceptance requirements

1. **Preferred if the environments are available:** retain ADR 0006 unchanged,
   supply owned disposable fault-test environments, and qualify the writable
   Windows directory barrier plus APFS/ext4 sequences. Investigate a vetted safe
   replacement wrapper if the crash campaign fails. Do not treat a successful
   ordinary rename or a generic file write-through flag as that proof.
2. Evaluate a proven embedded storage component scoped to an explicitly requested
   workspace. It must still solve contained artifact access, lifecycle and
   publication ordering; a metadata database alone does not make external files
   durable. This needs its own dependency, licence and scope review.

Maintainer acceptance must select the narrower profile above or an alternative,
identify an owned disposable crash-test environment, and update DEC-05/06/13,
the ledger, support profiles and affected contracts together. Until that explicit
acceptance, ADR 0006 remains authoritative, P03 remains in progress with a failed
qualification gate, and P04+ remain ineligible. Merging this proposal as a record
does not accept it or mark P03 complete.
