# ADR 0007: Managed runtime provisioning

- Status: Accepted
- Date: 2026-09-10
- Resolves: DEC-07

## Context

FFmpeg and speech models are necessary for some workflows, but silent npm lifecycle
downloads, administrator requirements and mutable provider replacement would weaken
security and make running work irreproducible.

## Decision

Resolve providers in this order: explicitly configured approved path, immutable
VSift-managed version, then compatible executable discovered on PATH. Always report
the selected provenance and trust level. Managed components live in an application-
owned per-user directory and require no implicit elevation.

Installation uses a separate `setup plan` followed by `setup install` with an accepted
plan digest. Plans pin target, version, origin, integrity metadata, size, licence,
files and activation effects. Downloads are bounded, verified, staged, smoke-tested
and atomically activated. Jobs pin immutable component/model versions. Repair,
rollback and removal use the same transaction and do not delete user-managed files.

## Consequences

The npm package remains small and does not silently fetch large tools or models.
Offline and proxy-aware flows share the same verification policy. Redistribution of
each provider build/model requires separate licence and provenance review.

## 2026-09-14 implementation note: verified transfer primitive

P06 now has a typed size/SHA-256 requirement and a bounded streaming verifier.
It accepts only a nonempty artifact of at most 1 GiB and exactly the reviewed
size and SHA-256. The destination is unactivated staging and must be discarded
on every error. This is not a URL fetcher, source trust catalogue, archive
extractor, compatibility smoke test or activation transaction. The HTTPS
adapter must enforce a transfer deadline and redirect/proxy policy; only a
reviewed catalogue may supply the expected digest. The default `setup plan`
and `setup install` remain unavailable pending the remaining P06 gates.

## 2026-09-14 implementation note: bounded tar inventory reader

P06 now reads raw tar headers through a bounded, sequential, no-write adapter
and applies the archive-inventory policy before any selected-file extraction.
The reader rejects unsupported extension/special headers, checks exact reviewed
link metadata, drains entry content rather than seeking past truncation, and
rejects nonzero trailing data or an oversized uncompressed tar stream. It does
not decompress publisher archives, extract files, stage runtimes or activate
them. The managed plan/install commands remain unavailable until those steps
and their failure tests are complete.

## 2026-09-14 implementation note: gzip tar inventory

The read-only P06 path now decodes all gzip members through a bounded
compressed-input reader before the tar inventory check. The expanded tar has
its independent whole-stream limit; invalid gzip trailers and hidden content
after the tar end marker fail closed. This covers an inspection format used by
the Ubuntu whisper.cpp candidate, not publisher archive qualification,
selected-file extraction, installation or activation. XZ decoding had not yet
been added at this point.

## 2026-09-14 implementation note: XZ tar inventory

P06 now also reads one XZ stream through a pure-Rust decoder with a reviewed
128 MiB compressed-input ceiling and a 128 MiB decoder block-memory limit,
then applies the bounded tar inventory policy. Concatenated streams, trailing
bytes, truncation and excess dictionary requirements fail closed. The pinned
Ubuntu FFmpeg archive passed read-only inventory on its verified bytes without
executing the included programs. This is not a total process-RSS or time bound;
resource qualification, selected-file extraction and managed activation remain
pending.

## 2026-09-14 implementation note: selected-file integrity

The bounded tar reader now also accepts a nonempty reviewed list of exact
regular-file paths, sizes and SHA-256 digests. It validates the complete archive
inventory and hashes the selected file bytes while reading raw tar, gzip/tar or
XZ/tar without writing to disk. A missing, renamed, non-regular or changed file
fails closed. This does not stage or install a runtime. Direct HTTPS transport,
reviewed plan acceptance, contained staging, compatibility smoke and atomic
activation remain separate P06 gates; the default managed setup commands remain
unavailable.

## 2026-09-15 implementation note: contained selected-file staging

P06 now has an infrastructure-only staging operation for raw tar, gzip/tar and
XZ/tar inputs. A caller must supply a newly created empty private directory
capability. The operation writes only reviewed regular files, flattened to
portable basenames, with create-new/no-follow semantics and private file modes;
archive directories, links and permission bits are never materialized. It
checks the complete bounded archive and selected hashes, and removes every file
created by the call if a later digest, inventory, decompression or trailing-data
check fails. The directory remains unactivated and caller-owned.

This increment has no managed root lifecycle, direct HTTPS transport, plan
acceptance, executable permission transition, compatibility smoke or atomic
activation. Those gates remain required before `setup plan` or `setup install`
can become available.

## 2026-09-15 implementation note: owned unactivated artifact staging

The per-user managed-data root now has a private directory check and an exact
VSift ownership marker. Only a newly created root may receive that marker;
an existing unmarked or altered directory fails closed. Each imported artifact
uses a fresh private, marked stage and a create-new/no-follow regular file.
The exact reviewed size and SHA-256 are checked while copying and again when
the staged file is opened for archive inspection. Failed copies discard only
the two known stage files and their directory. Unexpected content or a changed
marker blocks cleanup rather than risking deletion outside the owned stage.

The same bounded import can receive a trusted offline byte source. This does
not make a user-entered checksum or URL a trust anchor. The reviewed catalogue,
direct HTTPS policy, selected-file assembly, installation lock, smoke and atomic
activation still gate all public managed commands. No version is active yet.

## 2026-09-15 implementation note: direct reviewed publisher transfer

The infrastructure transport now accepts an immutable GitHub release-asset or
Hugging Face model-revision route supplied by reviewed source, fetches it from
the publisher on the user's machine over HTTPS, and permits only that origin's
observed release CDN redirect route. It rejects downgrade, other hosts,
credential-bearing URLs, unexpected encoding/range/status and size metadata.
Connect, stalled-read and total deadlines bound the operation; cancellation
discards its positively identified unactivated stage. There is no resume:
an interrupted attempt is discarded and a later attempt starts at byte zero.
Every completed stage checks the reviewed whole-artifact size and SHA-256 and
is rechecked before archive inspection. Errors report typed reasons without
rendering signed CDN URLs or proxy credentials.

The transport uses pinned maintained `reqwest` with the native TLS backend:
Windows and macOS use their system TLS facilities and Linux uses OpenSSL.
The alternate bundled root-certificate backend introduced a dependency
licence rejected by the existing `cargo deny` policy; native TLS passed that
policy without relaxing it. This requires platform and proxy failure testing
on the named targets. The exact Ubuntu whisper.cpp archive passed an opt-in
direct publisher download into owned staging without execution. Catalogue
approval, plan acceptance, selected-file assembly, compatibility smoke and
atomic activation still gate public managed commands. No version is active.
