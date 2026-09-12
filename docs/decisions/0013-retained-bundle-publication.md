# ADR 0013: Explicit retained bundles on the ephemeral desktop profile

- Status: Accepted
- Date: 2026-09-12
- Tracking: [P05 / issue #8](https://github.com/smormah/vsift/issues/8)
- Refines: [ADR 0002](0002-disposable-investigation-sessions.md) and
  [ADR 0010](0010-storage-qualification-gate.md)

## Context

P05 makes disposable sessions and retained evidence visible. ADR 0010 qualifies
process-crash-consistent ephemeral publication, but does not qualify strict
OS/storage-crash durability. A retained directory therefore needs an explicit
portability and publication contract before the CLI can report success.

## Decision

`session retain` is explicit user intent to create a **new** private directory at
an absolute caller-selected path. VSift rejects an existing destination. The
directory is validated as owner-private before sensitive bytes are copied.
Immutable evidence artifacts are copied and SHA-256 checked. The
`--include-source` form also copies and checks the bound private source snapshot;
the evidence-only form records the source hash and size and states that matching
original media is required for later re-extraction. The original source is
neither moved nor deleted.

The bundle format is a bounded, data-only `bundle.json` plus generated
content-addressed artifact filenames and optional `source.media`. The manifest
is written last. An interruption may leave an incomplete private directory at
the selected path; `bundle validate` rejects it. VSift never reports that
directory as a completed bundle and never automatically deletes a caller-selected
export. A complete bundle is revalidated before `retain` returns success.
Replacement or repair of an existing export is a separate future operation.

The result reports `process_crash_consistent` publication, even for
source-inclusive exports. Retention exempts the directory from VSift's
disposable-session cleanup; it does **not** upgrade the storage durability
guarantee. Strict durable requests still fail before mutation on every profile.

## Consequences

- `session clean` may remove only a positively identified, exclusively claimed
  disposable session under the owned root. It cannot remove the source or a
  retained destination.
- Operators may inspect or remove an incomplete caller-selected export. No
  secure-erasure claim is made for either temporary or retained bytes.
- Bundle validation never executes embedded content, fetches references, or
  trusts unbounded filenames, counts, sizes, hashes, links, or future versions.
- P10/P11/P14 still own strict Ubuntu/ext4 OS/storage-crash qualification.
