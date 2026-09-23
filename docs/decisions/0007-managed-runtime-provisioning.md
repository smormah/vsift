# ADR 0007: Managed runtime provisioning

- Status: Accepted; delivered by P13 per [ADR 0015](0015-r0-delivery-replan.md)
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

The later 2026-09-15 unqualified read-only plan note in ADR 0014 supersedes
that command-availability statement only for manual disposition. A managed
download plan still requires these catalogue, compatibility and activation
gates.

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

## 2026-09-15 implementation note: owned selected-file payload assembly

The previously separate exact-byte artifact stage and bounded archive readers
now compose within infrastructure. A rechecked artifact can populate a newly
created empty private `payload.pending` directory inside its positively marked
stage. Raw tar, gzip/tar and XZ/tar use the same reviewed archive limits,
aliases and exact selected-file digests as their standalone readers. The
archive's links, directories and permission bits are never materialized.
The selected flat regular files can be reopened only if the held payload
directory has the exact reviewed inventory and each requested file retains
its size/SHA-256, regular type and single-link identity. Unexpected content
or directory-name substitution blocks opening and cleanup; discard removes
only reviewed files and the held payload directory, leaving the artifact
available for explicit discard. Failed archive staging removes its created
files and the empty payload directory when ownership remains demonstrable.

A fresh pinned Ubuntu whisper.cpp archive passed an opt-in exact-byte import,
owned gzip/tar assembly, selected `whisper-cli` recheck and discard without
binary execution. This is still an unactivated payload: alias creation,
executable permissions, compatibility smoke, version activation, plan authority
and the reviewed catalogue remain separate gates. No public managed command
is enabled.

## 2026-09-15 implementation note: owned runtime layout preparation

An exact selected payload can now prepare a separate private
`runtime.pending` directory under its held artifact stage. The operation
checks a trusted flat-name and total-byte review before mutation, copies each
selected regular file with its original size/SHA-256, and creates only
explicit alias copies from selected files. It never materializes archive link
headers as filesystem links. Unix modes are set to owner-only read/write for
data and owner-only read/write/execute for the reviewed executable selection;
Windows continues to rely on the private directory ACL, with Windows runtime
compatibility still unqualified. Runtime open rechecks held directory identity,
the exact file set, regular/single-link type, mode where applicable, and each
requested digest. Discard removes only reviewed copies and leaves the original
payload and artifact for explicit later discard. A failed copy rolls back only
positively identified runtime files; suspicious extras block cleanup.

The pinned Ubuntu whisper.cpp archive passed opt-in owned import, gzip/tar
selected-file assembly, six required regular alias copies, executable-mode
preparation and whole-runtime recheck without binary execution. This narrows
D-04/D-06 preparation risk but does not accept the candidate, prove bounded
compatibility smoke, activate a version or enable managed installation.

## 2026-09-16 implementation note: hosted production-layout checkpoint

The credential-free, manually dispatched Ubuntu candidate workflow now first
downloads a fresh pinned whisper.cpp archive through the bounded candidate
transport and passes those bytes through the ignored Rust integration test for
the production owned-artifact, selected-payload and reviewed-runtime layout.
That checkpoint creates and rechecks the six selected files and six regular
alias copies, then explicitly discards every owned stage without running the
candidate. The workflow removes the downloaded archive even when the Rust check
fails. Its existing Python experiment subsequently downloads its own pinned
inputs and performs the model-backed candidate smoke on the disposable runner.

Keeping preparation and execution as separate checkpoints makes the evidence
boundary explicit: one proves that fresh publisher bytes satisfy the production
layout primitive on the target host, while the other observes candidate runtime
compatibility. Neither checkpoint is an installer transaction or catalogue
acceptance. Protected [run 35053264628](https://github.com/smormah/vsift/actions/runs/35053264628)
passed both checkpoints on Ubuntu 24.04: the Rust production-layout test
completed in 3.48 seconds, and the independent F01 model-backed candidate smoke
completed afterward. This narrows target-host layout uncertainty but does not
join preparation and execution into an install transaction.

## 2026-09-16 implementation note: installation transaction guard

The managed root now exposes a root-wide, non-blocking installation guard. Its
lock file is opened without following links inside the positively marked private
root and must remain a single-link regular file; Unix additionally requires mode
`0600`. A held OS lock returns typed `Busy`, while other lock failures remain I/O.
Dropping the guard releases the lock, allowing a later transaction to proceed.
Tests cover exclusion/release, external hard-link rejection with source
preservation, and Unix permission rejection.

This guard is a concurrency prerequisite for D-05. It carries no plan digest,
provider selection, compatibility result or user authorization, and therefore
cannot activate a runtime by itself. Immutable version publication, current
pointer replacement, crash recovery, rollback and active-job retention remain
separate required gates.

## 2026-09-16 implementation note: immutable version publication

A prepared runtime can now be published beneath the marked private managed root
using a bounded canonical component/version identity while the caller holds that
root's installation guard. Publication writes a private immutable-version manifest
covering the exact file names, sizes, SHA-256 digests and executable/data modes,
moves the candidate into the version store, then reopens and rehashes the complete
published inventory before atomically replacing the component's hashed current
pointer. A guard from another root, linked metadata, unexpected files and reuse of
an identity for a different manifest fail closed. Looking up an absent selection is
read-only and does not create managed storage.

The previous pointer remains selected when publication stops before replacement;
the already published version is reusable on retry. If replacement commits but the
caller observes a later failure, retry is also idempotent. Older versions remain
present and a held published-directory capability continues to open its reviewed
files after a newer version is selected. These tests establish process-interruption
ordering only; no power-loss durability claim is added.

This is an infrastructure capability, not install authority. It does not prove that
a candidate passed compatibility smoke, bind an accepted plan digest, authorize a
catalogue entry, expose `setup install`, implement rollback/uninstall/garbage
collection, or prevent removal while another process holds a version. Those gates
remain required before managed installation is available.

## 2026-09-21 implementation note: rollback selection and removal fencing

Every opened published runtime now retains a shared OS lock on a private,
single-link per-version lock file. While holding the same root's installation
guard, infrastructure can revalidate an existing immutable version and atomically
select it as the component's current version. This supplies the rollback selection
primitive without downloading, republishing or changing the version contents.

Removal checks the current pointer first and returns `Selected` without mutation
when the requested version is active. For another version it requires an exclusive
use lock, returning `InUse` while any published-runtime capability remains live.
After exclusivity is established, a private tombstone fences new openers. Deletion
is limited to the manifest's exact reviewed files and known metadata; unknown,
linked or changed content fails closed. The lock handle closes before its file is
removed for Windows compatibility. Retries safely resume after partial payload,
lock-file or manifest deletion, after only the tombstone remains, and after the
version directory was already removed.

A native child-process regression opens an unselected version in one process,
proves the guarded remover receives `InUse` in another, terminates the holder,
and then removes the version successfully. This checks both cross-process
exclusion and OS lock release after abrupt process exit on each CI platform.

This implements provider-neutral lifecycle mechanics only. No source catalogue,
compatibility decision, accepted plan, public rollback/uninstall command or bounded
garbage-collection policy invokes them yet. Power-loss durability and an
independent process-crash-at-every-deletion-boundary campaign remain open, so
managed installation remains unavailable.

## 2026-09-21 implementation note: catalogue and plan

A single Ubuntu 24.04 x86-64 catalogue revision now pins direct publisher
artifacts, selected payload/runtime inventories, exact integrity and bounded
archive metadata, the 2028-08-01 stop-new-plans date, and notice/source
disclosures. Application planning binds that reviewed data and current probe,
model presence and configured selections into a canonical SHA-256 digest.
Unsupported, expired or invalid catalogue states produce no managed action or
digest. This supplies reviewable plan authority but does not permit activation:
the CLI still reserves `setup install` until compatibility and accepted-plan
revalidation are wired into the transaction.

## 2026-09-22 implementation note: plan revalidation without mutation

The CLI now has the accepted-plan gate needed immediately before a future
transaction. A saved JSON plan is read with fixed byte/nesting limits and strict
unknown-field rejection, compared in full with a newly evaluated current plan,
and authorized only when `--accept-plan` matches that current digest. This
detects catalogue expiry, target, configuration, probe, model and presentation
changes without trusting the saved document as authority. The valid path still
returns `COMMAND_NOT_IMPLEMENTED` and invokes no transfer, staging, compatibility
smoke, publication or activation primitive.

## 2026-09-22 implementation note: digest-bound compatibility policy

The accepted Ubuntu catalogue now carries the exact F01 identity, expected
FFmpeg and FFprobe build lines, bounded stream/generated-file output and process
deadlines, and 16-kHz mono audio
contract that a production smoke executor must enforce. Catalogue validation
fails closed when those limits are absent or outside the reviewed bounds. The
canonical plan digest includes every policy value, making a policy revision a
new acceptance decision even when artifact identities are unchanged. This note
accepts the policy only; no candidate is executed or activated yet.
