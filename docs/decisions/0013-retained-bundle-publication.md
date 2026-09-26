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

## 2026-09-24 implementation note: transcript records are validated as data

P07 adds the artifact kind `transcript_record`. Its content is now published as
`schemas/v1/bundle-transcript-record.schema.json`. Before this change,
`bundle validate` checked only its size and SHA-256 against `bundle.json`. That
proves the file is the one the manifest names, but both could have been rewritten
together.

`bundle validate` now also reads every transcript record (bounded by the 24 MiB
kind limit) and decodes it with the same strict rules as a session read:
- unknown fields, values and formats are rejected;
- every domain import invariant is re-checked, such as each segment lying at its
  cue timing plus the offset;
- the record must name the bundle's source.

A non-conforming record fails with `INTEGRITY_FAILURE`, and a newer record version
with `UNSUPPORTED_SCHEMA`. This remains data-only validation: nothing is executed or
fetched. Bundles that `session retain` produced are unaffected, because their records
already satisfy these rules. `session retain` itself revalidates the finished bundle,
so it applies the same check.

## 2026-09-26 implementation note: visual-index records are validated as data

P08 adds the artifact kind `visual_index_record`: one revision of a session's
visual-candidate index, at most 8 MiB and at most 64 per session (a 65th is
`RESOURCE_LIMIT`). Each revision holds every window of the one before it, so a session
read uses only the newest record, and a bundle carries all of them.

As for transcript records, `bundle validate` (and therefore `session retain`) decodes
every visual-index record strictly, not only its size and SHA-256:
- unknown fields, values, versions and formats are rejected;
- every window's range must follow the fixed 60 s grid of the recorded duration;
- candidate spans, sample counts, the change policy and the 10 s coverage rule are
  re-checked;
- every candidate and revision identity is derived again from the session, source,
  stream, profile, window and time, and the record must name the bundle's session and
  source.

A non-conforming record fails with `INTEGRITY_FAILURE`, a newer record version with
`UNSUPPORTED_SCHEMA`. Records hold candidates only, never pixels. The public schema for
the record is published with the `candidates` command (P08 PR 4); until then no
command writes one.

## 2026-09-26 implementation note: evidence records and media are validated as data (P09)

P09 PR 2 adds two artifact kinds: `audio_wav` (a 16 kHz mono 16-bit WAV clip, at most
1 MiB) and `evidence_record` (one evidence call's lineage, at most 256 KiB); frame and
crop images keep `frame_png`. Evidence (images, audio and records) has a sub-budget of
160 of a session's 256 artifacts (ADR 0019 D4); the 64 KiB manifest and 10 GiB bounds
are unchanged, and a manifest that would exceed 64 KiB is now refused before it is
written. One evidence call commits its new files and its record in one generation; a
file the session already holds with the same name, kind, size and digest is kept, and
the same name with another kind is an integrity failure. The session manifest may also
record `verified_source_identity`, the digest of ADR 0019 D1; it is not copied into
bundles. The field is optional, so older manifests stay readable; a session written by
this version is not readable by an older build (sessions are disposable).

`bundle validate` (and therefore `session retain`) decodes every evidence record
strictly, as for the other records:
- unknown fields, values, versions and formats are rejected, and the request key and
  every item identity are derived again;
- each item's file must be in the bundle with the item's kind, size and digest;
- an image's PNG header must state the item's size as 8-bit RGB without interlace, and
  a clip's WAV header 16 kHz mono 16-bit PCM and its data size;
- every crop parent and neighbours anchor must be an item of the bundle, and items that
  share an identity must agree on everything but their source check.

A non-conforming record fails with `INTEGRITY_FAILURE`, a newer record version with
`UNSUPPORTED_SCHEMA`.
