# P03 storage feasibility record

Date: 2026-09-11. Predecessor: `25c3aad01fc9e2fc391c5c016295df5aff61fbdd`.
Issue: [#6](https://github.com/smormah/vsift/issues/6). Status: feasibility evidence
recorded; [ADR 0010](../decisions/0010-storage-qualification-gate.md) accepts the
narrower ephemeral desktop profile and permits P03 production implementation to
resume. P03 remains in progress; durable enablement remains gated to P10/P11/P14.

## Candidate dependency review

The standard library supplies safe file locks, synchronization and replacement,
but no portable directory-relative no-follow interface. The spike pins
`cap-std` and `cap-fs-ext` 4.0.3 as **development dependencies only** to evaluate
those missing operations. No domain/application/CLI dependency is added.

The Bytecode Alliance repository is active (not archived; latest push observed
2026-08-20). Both packages declare MIT OR Apache-2.0 OR Apache-2.0 WITH
LLVM-exception, compatible with the existing deny policy. They have no declared
MSRV; this repository's pinned Rust 1.98.1 and native CI are the compatibility
check. Default features select the standard synchronous interface. Transitive
platform wrappers include `cap-primitives`, `rustix` and Windows `winx`/
`windows-sys`; unsafe code stays in dependencies and requires candidate review,
not a VSift lint exception. Cargo.lock fixes the full graph; cargo deny and
dependency review are required before merging this investigation.

The lockfile adds 34 development-only packages, including multiple versions of
Windows support crates. No existing package version was upgraded. Initial
`cargo deny check` passed advisories, bans, licences and sources. Post-test review
found that cargo-deny 0.20.2 omits development-only licences and duplicate-version
checks by default, even though it gathers those packages for advisory/source
checks. P03 explicitly enables `licenses.include-dev` and
`bans.multiple-versions-include-dev` in deny.toml
so the candidate licence review is executable. This exposes the already-locked
P01 test dependency `borrow-or-share` 0.2.4 (through jsonschema/fluent-uri), whose
MIT-0 licence was previously outside the default check. Its packaged LICENSE was
reviewed against [SPDX MIT-0](https://spdx.org/licenses/MIT-0.html): the permissive
MIT grant without the attribution condition is compatible with this dual-licensed
project. MIT-0 is explicitly added to the allowed licence list on that basis;
no package exception or advisory ignore is added. Duplicate-version warnings
remain warnings under existing policy, including the candidate Windows wrappers
and io-lifetimes versions. Development dependency auditing is now stricter.

Review scope: directory opening, no-follow extension, relative rename, file flush,
and Windows directory share flags. This is not approval of the dependency as a
complete production storage boundary. Hard-link identity, private permissions,
bounded mutation/recovery and qualified durable publication remain adapter duties.

Source review of the 4.0.3 registry package found Windows `dir_options` and
`open_ambient_dir_impl` acquire read access and deny delete sharing to keep
directories stable. The relative rename implementation holds parents, derives
their paths and calls `std::fs::rename`; it exposes no write-through publication
option. This is not evidence of a containment vulnerability in cap-std.

## Evidence limits

The test spike uses only synthetic bytes inside exclusive test roots. It does not
consume or generate F01/F05/F06/F11 media; their independent manifest truth remains
unchanged. Tests map to prerequisites of S-01/S-02/S-03/S-07/S-12 and X-01/X-04/X-05,
not completion of those suites. Killing a child is process-crash evidence only.
Successful file/directory synchronization is API evidence, not an OS-crash or
power-loss qualification. NTFS/APFS/ext4 must be identified in each native run.

## Measured experiments and remaining gate

Local host: Windows 11 Pro 10.0.26200, x64, NTFS (both checkout and probe temp
directory), Rust 1.98.1. Ten experiments passed; the child fixture is
intentionally ignored by the ordinary test runner and explicitly launched by its
watchdog-bounded parent tests.

- `s07_probe_file_and_directory_flush`: file `sync_all` and replacement succeed;
  default read-only directory `sync_all` returns PermissionDenied, OS error 5.
- `s07_probe_windows_directory_write_access`: explicitly requesting write access
  with the safe standard-library Windows extension succeeds, including directory
  `sync_all`. This corrects the initial interpretation of the read-only-handle
  failure: it is not evidence that safe Windows durability is impossible. A real
  publication sequence must preserve directory identity and no-delete-sharing
  when integrating this handle with the capability boundary, then prove its
  crash semantics. The probe does not perform that integration.
- `s01_probe_relative_escape_denied`: absolute and parent-traversal reads fail;
  the synthetic outside sentinel remains identical.
- S-02 probes: no-follow does not exclude a hard-linked external inode; exclusive
  creation refuses replacement and preserves the sentinel. Windows held-directory
  rename is denied until handle release. Unix separately tests symlink rejection
  and parent substitution against an already-open handle.
- `s03_probe_open_handle_is_not_immutable_snapshot`: a same-size synthetic in-place
  write is visible through the original read handle. Source snapshot policy remains
  necessary; this experiment never opens real source media.
- S-12/X-05 probe: a distinct process holds an exclusive stable anchor despite idle
  time; a contender remains busy, then acquires that anchor after kill and reap.
- X-04 probes: independent shared handles exclude a writer until both release;
  an old reader observes old bytes after pointer replacement, a new reader new bytes.
- X-01 probe: kill after write, after file flush, and after pointer replacement;
  reopening sees the expected complete old/new synthetic pointer. This is three
  primitive boundaries, not the future complete generation/ack protocol.

All passing experiments are deliberately narrower than the full acceptance suites.
Still unimplemented/unqualified: production identity/ports/storage; weighted global
admission and policy transactions; every Windows reparse/junction/ADS/reserved-name
case; ACL/privacy validation; full source-change handling; generation hashes and
future-version validation; write/flush/rename fault injection, disk exhaustion and
cancellation; full concurrent stale-generation/idempotency races; owned cleanup;
OS/storage crash/restart acknowledgement tests. S-01..S-03/S-07/S-08/S-12 and
X-01..X-05 are **not complete**. SEC-07..SEC-11, SEC-18 and SEC-24 remain open.

The blocking evidence gap for a strict durable claim is the required disposable
OS/storage fault campaign.
No such harness is configured in the repository or this task. The local Windows
workstation must not be crashed for a probe, and ordinary GitHub hosted jobs do
not provide restart/recovery control over their underlying storage. An owned
disposable environment and reviewed publication/ack protocol are needed. The
accepted narrower profile uses ephemeral NTFS/APFS qualification for P03 and defers
Ubuntu/ext4 durable enablement to P10/P11/P14. This is not a conclusion that Windows
or macOS cannot implement safe durable storage in a later profile.

## Local validation and security review

`cargo fmt --all --check`, strict workspace/all-target/all-feature clippy,
`cargo test --workspace --locked` (92 passing, one internal child fixture ignored),
governance validation and warning-denied rustdoc passed. A separate post-test source/diff review checked dependency scope,
fixture ownership, exact nonrecursive cleanup, child watchdog/reaping, no source
media access, and the separation between API observation and qualification. It
corrected the initial overstatement of the Windows default-handle error and enabled
the missing development licence/duplicate checks with the review recorded above. No
production security finding is closed. There is no AI-trust checksum manifest or
update script in this repository; none was invented.

## Native CI evidence

Probe revision `a262fa9bbabb4bebc6ebde581204c4dbe0a8186d`, PR #24:
[CI run 34585520298](https://github.com/smormah/vsift/actions/runs/34585520298)
passed all three Quality jobs, documentation, governance and the strict-worker
regression job. [Security run 34585520299](https://github.com/smormah/vsift/actions/runs/34585520299)
passed the strengthened dependency policy and dependency review. Ten storage
experiments pass on each platform, with the internal child entry ignored by the
normal runner and invoked explicitly by parent tests. The original spike
`5f15c7730607570663d739b7a4dd70a03947e500` also passed its required checks in
CI run 34585026670, Security run 34585026624 and CodeQL run 34585026669.

| Host / filesystem | Primitive result | Qualification limit |
| --- | --- | --- |
| Local Windows 11 Pro 26200 / NTFS | Ten probes pass; default directory flush error 5, writable handle flush succeeds | No OS/storage fault campaign |
| Hosted Windows Server 2025 build 26100 / NTFS | Ten probes pass; writable directory flush succeeds | Not the Windows 11 target; no OS/storage fault campaign |
| Hosted macOS 26.6.2 build 25G83 arm64 / APFS | Ten probes pass; default and relative-readable directory sync succeed | Not the macOS 15 target; no OS/storage fault campaign |
| Hosted Ubuntu 24.04.5 LTS, kernel 6.17.0-1022-azure x64 / ext4 (`commit=30`) | Ten probes pass; default capability directory sync returns EBADF (9), relative-readable reopen sync succeeds | No OS/storage fault campaign |

API handle failures must be investigated before declaring a filesystem unsupported.
These results establish usable safe API candidates on each observed filesystem;
they do not establish the storage ordering contract after OS failure. No ordinary
hosted-runner success qualifies the exact ADR 0005 platform profiles. Local
inspection found no registered Hyper-V VM and no QEMU, VirtualBox or VMware CLI;
existing WSL distributions are not identified as disposable fault targets. No host
or user VM was restarted, crashed or reconfigured.

The post-test security review found no additional issue in this test-only diff.
R0 filesystem/admission/publication controls remain unimplemented; the passing spike
and strengthened dependency check must not be used to close their threats or advance
the ledger. ADR 0010 now permits the P03 adapter work only. P04 remains ineligible
until P03 implementation and its revised ephemeral-profile gates are complete.

## Primary-source basis

- [cap-std 4.0.3 source](https://github.com/bytecodealliance/cap-std/tree/v4.0.3),
  tag object `5cae39826c70e7da89cc821b825885e030d38f93`: inspected
  `cap-primitives/src/windows/fs/dir_utils.rs`, `rename_unchecked.rs`,
  `cap-primitives/src/fs/via_parent/rename.rs` and the public directory/extension APIs.
- [Microsoft FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)
  requires write access; volume flushing requires administrative privileges.
  Elevation/volume-wide flush is not a P03 workaround.
- [Microsoft MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
  documents write-through behavior separately from ordinary replacement. The spike
  does not qualify that API or a safe wrapper as a replacement sequence.
- [Linux fsync](https://man7.org/linux/man-pages/man2/fsync.2.html) distinguishes
  file data/metadata from persistence of the containing directory entry.
- [Apple fsync](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fsync.2.html)
  explains the stronger full-flush requirement for storage ordering. A successful
  portable `sync_all` result alone does not establish which barriers ran or qualify
  APFS failure recovery.
