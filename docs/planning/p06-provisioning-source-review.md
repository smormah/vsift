# P06 provisioning source review — 2026-09-12

Status: **Ubuntu 24.04 x86-64 catalogue accepted for read-only plans on
2026-09-21; managed installation and complete P06 qualification remain open**.
The sections below record the source investigation as it stood at their dated
checkpoints. They are not installation or D-01..D-10 implementation evidence. Reviewed
protected-main predecessor: `777bc3e56788d43cd9a647dba54c287390c2998d`.

## 2026-09-21 reviewed catalogue and plan authority

The application now receives a typed reviewed catalogue from infrastructure
source for exactly Ubuntu 24.04 x86-64. The three entries pin the
[BtbN month-end FFmpeg/FFprobe build](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-08-31-13-27),
[whisper.cpp v1.9.2 Ubuntu CLI](https://github.com/ggml-org/whisper.cpp/releases/tag/v1.9.2)
and [multilingual `base` model revision](https://huggingface.co/ggerganov/whisper.cpp/tree/80da2d8bfee42b0e836fc3a9890373e5defc00a6).
Their exact direct publisher URLs, whole-artifact and installed-file sizes and
SHA-256, archive bounds, selected paths, reviewed link headers, regular alias
copies and expected executable modes live in
`crates/vsift-infrastructure/src/managed_catalogue.rs`. Independent inventory
and hosted F01/model-backed observations are in the
[Ubuntu candidate record](p06-ubuntu-artifact-candidate.md). The catalogue
stops issuing new plans on **2028-08-01T00:00:00Z**, before the publisher's
month-end retention window ends. Any replacement requires reviewed source and
a new revision. An unsafe or withdrawn entry is revoked by removing or changing
it in reviewed source; installation must recompute the plan against current
source and reject an old digest. There is no live `latest` trust anchor or
VSift mirror.

The plan discloses the FFmpeg archive's verified LGPL version 3 text, the
publisher's static-build label, its pinned build-repository revision and the
limits of the compiled dependency/source inventory. The other entries disclose
their [MIT source notice](https://github.com/ggml-org/whisper.cpp/blob/v1.9.2/LICENSE)
and [pinned model card](https://huggingface.co/ggerganov/whisper.cpp/blob/80da2d8bfee42b0e836fc3a9890373e5defc00a6/README.md).
This is notice and provenance disclosure for a user-initiated direct download,
not a legal-clearance conclusion. The code does not redistribute those bytes.
Hosted tone-audio smoke is evidence of operations on one Ubuntu runner, not
real-speech accuracy, arbitrary desktop compatibility or production install.

`setup plan` now binds profile, exact target, catalogue revision, current
executable probe states and configured selections, model presence, and every
artifact/layout/disclosure field into a canonical SHA-256 digest. It emits
actions only for missing capabilities and a typed unavailable reason with no
digest on other targets, invalid entries or after expiry. Existing tools and
configured model files remain probe/presence-only. A plan is not authority to
mutate: `setup install` remains reserved until it can revalidate current state,
the accepted digest, compatibility and the complete installer transaction.
P06 stays planned; D-01..D-10, bounded cleanup, power-loss qualification and
the P06 E2E checkpoint remain open.

## 2026-09-22 accepted-source staging bridge

The infrastructure now rebinds each proposed Ubuntu action to its exact
current reviewed source entry before exposing a publisher transfer source.
Changing the action ID, version, URL, checksum, selected files or disclosure
removes that authority. The same accepted entry supplies whole-artifact
identity, archive inventory limits and link headers, selected-file digests,
regular runtime copies and executable modes to the private staging path.
Staging refuses a verified artifact with a different whole-artifact identity;
runtime preparation refuses a payload with a different selected inventory.
The opt-in pinned FFmpeg and whisper.cpp archive checks now use this bridge
for the owned payload and runtime path. The manually dispatched disposable
Ubuntu workflow runs both checks against fresh publisher bytes before its
separate model-backed candidate smoke. All three checkpoints passed in
[hosted run 35668273600](https://github.com/smormah/vsift/actions/runs/35668273600).
This is still an unactivated boundary. The raw model path is added in the
next dated checkpoint; production compatibility smoke, accepted-plan
revalidation, activation and cleanup qualification remain required before
`setup install` can run.

## 2026-09-22 pinned raw-model staging

The accepted multilingual `base` model now follows a reviewed raw-file path
through the same owned stage as the Ubuntu archives. The source artifact is
rehashed before copy; its flat runtime name and full size/SHA-256 must match
the accepted entry. A private selected payload and separate runtime copy are
rechecked without model execution or activation. Invalid names and a changed
whole-artifact identity fail before payload creation. The disposable Ubuntu
workflow downloads the exact pinned model from its publisher and runs the
production Rust owned-layout test before the separate candidate inference
smoke. Both stages passed in
[hosted Ubuntu run 35719747666](https://github.com/smormah/vsift/actions/runs/35719747666).
A successful byte/layout check does not prove production model compatibility,
speech accuracy, installer rollback or D-01..D-10 completion.

## Finding

ADR 0007 requires an immutable, reviewed trust anchor before a plan may authorize
download or activation. The repository has no selected artifact catalog for
FFmpeg/FFprobe, whisper.cpp CLI and multilingual `base` model: no exact per-target
URLs, archived digests, publisher identity, expected file inventory, binary licences,
or compatibility baseline. The first P06 increment supports per-call absolute
executable selection alongside filtered `PATH` and typed manual guidance, but
reports `executable_probe_only` and does not check the model. Managed setup
lifecycle commands still return `COMMAND_NOT_IMPLEMENTED`.

The later persistent-BYO increment implements `setup configure` for user-managed
executable paths in a private per-user record. The following increment adds
`setup configure-model` for a canonical nonempty user-managed model file in that
record. Registration does not read model bytes or change executable-only setup
readiness. Model-backed compatibility, reviewed managed source selection and
installation remain unimplemented.

The 2026-09-15 read-only plan increment changes `setup plan --profile` from a
generic reserved failure into a check-first diagnosis of configured or filtered
`PATH` executables. Its v1 result explicitly says
`managed_install: unavailable_unqualified`, with no actions or acceptance
digest, and gives typed user-managed/BYO next steps for unavailable tools.
Responding tools remain executable-probe-only candidates; the local ASR model
is not checked. `setup install` and the other managed lifecycle commands
remain reserved. No catalogue item, installation or compatibility qualification
is implied by this plan response; D-07/D-10 and P06 E2E remain open.

The verified-stream increment adds a bounded exact-size/SHA-256 check over an
injected byte source and unactivated sink. It is not exposed as managed setup
and does not authorize a user URL or checksum. The reviewed catalogue, HTTPS
transport, extraction and activation are separate pending P06 steps.

The following archive-inventory increment (`7e73b95`) adds a provider-neutral
infrastructure policy for a complete bounded entry index. It rejects portable
path escapes, case-insensitive duplicates, undeclared symbolic links, hard
links and special entries; a reviewed link header is inspected only, never
materialized. The policy has no archive parser, decompressor, filesystem write
or activation path. It therefore narrows D-04's extraction risk but does not
close D-04 or authorize the Ubuntu candidate for installation.

The next increment (`01ba396`, from protected main `ef95bba`) adds a read-only
raw-tar adapter over a bounded input stream. It reads each entry sequentially,
rejects unsupported extension/special headers, nonzero trailing content and
declared/whole-stream overages, then applies the same provider-neutral policy.
It does not unpack or write an entry, decompress the publisher's XZ/GZIP
archives, or authorize the Ubuntu candidate. D-04 remains open.

Dependency review: `tar` 0.4.46 is confined to `vsift-infrastructure`, pinned
with default `xattr` support disabled. The [maintainer repository](https://github.com/composefs/tar-rs)
was not archived and had a 2026-09-01 push at review; the crate declares
MIT OR Apache-2.0 and Rust 1.63 MSRV, below VSift's Rust 1.98 toolchain.
Version 0.4.46 is newer than the 0.4.45 fix for
[CVE-2026-33055](https://github.com/composefs/tar-rs/security/advisories/GHSA-gchp-q4r4-x4ff).
The new locked graph adds `tar` and `filetime`; `cargo deny check` passed
advisories, bans, licences and sources with existing duplicate-version warnings.
The [crate's security documentation](https://docs.rs/tar/0.4.46/tar/#security)
explicitly excludes concurrent destination mutation from its unpacking
guarantee. VSift does not call its unpack APIs; a later extractor must use
contained filesystem handles and verify selected regular-file bytes itself.

The next read-only increment decodes gzip through pinned `flate2` 1.1.10 in
`vsift-infrastructure`, with the pure-Rust backend and no C zlib backend. It
checks all gzip members, caps compressed input at a reviewed value not above
256 MiB, and feeds the existing 1 GiB-capped tar reader; a corrupt trailer or
additional decompressed content after the tar end marker is rejected. The
[crate documentation](https://docs.rs/flate2/1.1.10/flate2/) distinguishes
single-member decoding from `MultiGzDecoder`, which is why this adapter uses
the latter. `flate2` declares MIT OR Apache-2.0 and Rust 1.67 MSRV, below
VSift's Rust 1.98 toolchain; its [repository](https://github.com/rust-lang/flate2-rs)
was active and not archived at review (2026-09-06 push). The locked graph adds
`flate2`, `miniz_oxide`, `adler2`, `simd-adler32` and `crc32fast`;
`cargo deny check` passed advisories, bans, licences and sources with existing
duplicate-version warnings. This reader does not extract any bytes, qualify
the Ubuntu candidate, or close D-04.
The pinned whisper.cpp gzip asset also passed opt-in, read-only Rust inventory
inspection after independent exact-size/SHA-256 verification (`c52df98`);
all 44 entries and eight reviewed link headers matched. No binary was run.

The next read-only increment (`2a960ba`) adds XZ/tar inspection for the pinned
Ubuntu FFmpeg format. It uses `lzma-rust2` 0.20.1 with only `std` and `xz` in
production; its default `optimization` and encoder features are off. The
encoder is test-only. The crate declares Apache-2.0 and Rust 1.85 MSRV,
below VSift's Rust 1.98 toolchain. Its [maintainer repository](https://github.com/hasenbanck/lzma-rust2)
was not archived and was active at review (2026-09-14 push). The locked graph
adds `lzma-rust2` and `const-oid`; `cargo deny check` passed advisories, bans,
licences and sources with existing duplicate-version warnings. The
[documented `XzStream` API](https://docs.rs/lzma-rust2/0.20.1/lzma_rust2/struct.XzStream.html)
supports an explicit decode-memory limit, unlike the convenience reader.
VSift accepts one stream only, caps compressed bytes at 128 MiB, uses a 128 MiB
block-memory limit, and feeds the existing whole-tar cap. These are not a
total process-RSS or wall-clock bound; installation still needs resource and
deadline qualification. The exact FFmpeg archive passed opt-in Rust read-only
inventory after size/SHA-256 verification without running an executable.
There is still no selected-file extraction or activation, so D-04 and P06
remain open.

The next read-only increment (`5454dcb`) verifies exact path, regular-file
type, size and SHA-256 for a nonempty reviewed file selection while the bounded
tar reader consumes the whole archive. The gzip/tar and XZ/tar adapters use the
same check, and the pinned Ubuntu archive opt-in tests name the previously
observed selected-file digests. The check writes no file and does not qualify
contained extraction, a production catalogue, direct HTTPS transfer, a plan,
smoke validation or activation. D-04 and P06 remain open.
Both opt-in tests passed against fresh pinned publisher downloads after checking
whole-archive size and SHA-256, without running a binary.

The contained-staging increment (`57b8113`) adds no-follow, create-new writes
under an empty directory capability. Only the exact reviewed regular files are
written, under portable flat basenames and private modes; archive links,
directories and modes remain metadata only. Digest, later-entry, trailing gzip
member and trailing XZ content failures remove files created by the call, while
a nonempty staging directory is rejected without changing its contents. This
is an unactivated infrastructure primitive, not an application-owned managed
root, download, accepted catalogue, plan, compatibility check or installer.
D-04 is narrowed but remains open through lifecycle and adversarial E2E proof.
Fresh downloads of both pinned Ubuntu publisher archives matched their recorded
whole-archive identities and passed the contained-staging opt-in tests without
executing the staged files. FFmpeg staging took 106.30 seconds and whisper.cpp
staging took 2.47 seconds on the maintainer machine.

The [Windows x64 candidate investigation](p06-windows-artifact-candidate.md)
now records exact observed archive hashes, selected-file inventories and one
model LFS pointer. It is not an accepted catalogue: binary-specific licence
notices, real-speech/resource/desktop qualification, installer hardening and
the other named targets' fallback behavior remain open. Its opt-in Windows
Server 2025 tone-audio compatibility smoke passed in
[run 34737109736](https://github.com/smormah/vsift/actions/runs/34737109736);
that run did not establish transcript accuracy or Windows 11 support. BtbN's
[pinned retention policy](https://github.com/BtbN/FFmpeg-Builds/blob/847e5e1cacc2945ac46528d34d754bd36051680c/README.md#release-retention-policy)
keeps only the last 14 daily builds, so the selected 2026-09-09 asset is not
a dependable long-lived installer source. Its expiry/replacement path also
needs a reviewed decision before catalogue acceptance.

A [2026-08-31 month-end BtbN alternative](p06-windows-artifact-candidate.md#month-end-replacement-under-review)
has a verified archive and selected-file inventory. Its
[hosted tone-audio compatibility run](https://github.com/smormah/vsift/actions/runs/34756838957)
passed on Windows Server 2025. The publisher retains month-end builds for two
years, not indefinitely. Real-speech/resource/Windows 11 qualification,
binary notices and maintained replacement policy remain unqualified.

An [Ubuntu x64 candidate inventory](p06-ubuntu-artifact-candidate.md) now records
the month-end BtbN LGPL archive and upstream whisper.cpp v1.9.2 Ubuntu CLI
archive hashes and selected-file inventory. Its whisper archive contains eight
symlinks, so safe non-link extraction requires explicit review. It is not an
accepted catalogue or installer evidence.

The candidate's [hosted Ubuntu run 34782768288](https://github.com/smormah/vsift/actions/runs/34782768288)
subsequently passed pinned hashes, non-link selected-file layout, F01 media
operations and model-backed tone-audio inference. Resource, speech accuracy,
notices, production installer and D/E2E gates remain open. This does not promote
the Python qualification harness into the Rust installer.

A 2026-09-14 read-only recheck of the pinned Ubuntu FFmpeg archive confirmed
its `LICENSE.txt` is the LGPL version 3 text. The publisher labels the asset
LGPL/static. The selected whisper.cpp v1.9.2 notice and pinned model card state
MIT, with OpenAI Whisper's original notice also MIT. The exact build's full
compiled-component source/configuration references and plan-facing notice
disclosure still need review; these observed labels do not by themselves accept
the managed catalogue or settle legal status. See the
[Ubuntu candidate record](p06-ubuntu-artifact-candidate.md).

The opt-in [Ubuntu candidate run 34851335044](https://github.com/smormah/vsift/actions/runs/34851335044)
captured the pinned executable's build configuration and a line of its LGPL
runtime statement. F01 model-backed inference took 23.05 seconds; the largest
child-process peak RSS across the smoke was 290,820 KiB. This is tone-only,
single-run candidate evidence, not an isolated model resource/accuracy gate.
The candidate record now links the BtbN release's fixed build-scripts commit,
the upstream FFmpeg commit named by the binary, and the whisper.cpp tag commit.
It proposes a 2028-08-01 cutoff for new plans from the 2026-08-31 month-end
asset, subject to an earlier revocation or a reviewed replacement. These
source links and publisher checksums are supporting provenance, not a binary
signature or independent proof of all compiled dependency sources.
The full compiled-component source/notice review, catalogue and production
installer gates remain open; `setup plan` cannot offer a managed download.

An unactivated P06 managed-data root and exact-byte import boundary now exist
in infrastructure. A new root receives a fixed ownership marker; existing
unmarked or unsafe roots fail closed. Imported bytes are rechecked before
archive use and failed transfers discard only positively identified staging.
This does not accept the Ubuntu candidate into the catalogue, disclose a
plan, run a provider, or activate any version. The many components enabled in
the observed FFmpeg build configuration still require a bounded source/notice
assessment; the archive's LGPL text alone is insufficient to claim that its
entire compiled dependency set has been reviewed.

The direct-transfer infrastructure increment downloads a reviewed immutable
publisher URL to that owned unactivated stage with system TLS, an origin-specific
HTTPS CDN redirect rule, bounded deadlines and whole-artifact size/SHA-256.
Interrupted transfers discard and restart from zero; the verified offline
import applies the same byte policy. A pinned Ubuntu whisper.cpp release asset
passed an opt-in direct transfer test. This does not accept that asset into a
catalogue, qualify its runtime, or enable any setup command. Deterministic
TLS/proxy/drop fault campaigns, platform validation and activation remain open.

An owned payload-assembly increment now passes the rechecked artifact to the
bounded raw tar, gzip/tar or XZ/tar selected-file reader inside a fresh private
directory under that artifact stage. Only exact reviewed flat regular files
are written; unexpected, linked or changed payload files fail recheck and
block unsafe cleanup. A fresh pinned Ubuntu whisper.cpp archive passed the
opt-in import, owned assembly and `whisper-cli` recheck without execution.
The listed Ubuntu artifacts remain candidates, not an accepted catalogue or
plan. SONAME alias creation, executable modes, compatibility smoke, atomic
activation and full P06 qualification remain open.

The later owned-runtime-layout increment stages a **separate** fresh private
`runtime.pending` directory from the verified payload. It checks the reviewed
total copied-byte budget, portable/case-insensitive names, selected executable
list and alias-to-selected-source mapping before mutation; selected and alias
copies retain exact selected-file hashes. Unix executable mode is owner-only,
while the original selected payload remains private and unchanged. Reopen
rejects unexpected, linked, substituted, mode-changed or hash-changed runtime
files, and explicit discard removes only positively owned copies. A fresh
pinned Ubuntu whisper.cpp publisher archive passed the six regular SONAME
alias copies, `whisper-cli` mode preparation and runtime recheck without
execution. This is a layout candidate, not an accepted trust anchor, smoke
result, installed version or D-04/D-06 closure. Compatibility and source/
notice review, activation and full installer faults remain open.

The manually dispatched Ubuntu workflow now places a fresh bounded,
SHA-256-verified whisper.cpp download through that production Rust layout check
before running the separate Python model-backed candidate experiment. The Rust
checkpoint discards the owned runtime without execution; the Python checkpoint
independently obtains its pinned inputs. The first protected
[hosted run 35053264628](https://github.com/smormah/vsift/actions/runs/35053264628)
passed: the production Rust layout check completed and discarded its owned
stages before the independent F01 model-backed candidate experiment passed.
This composition does not create plan authority or catalogue acceptance.

The infrastructure also now has a root-wide, non-blocking managed installation
guard. A private single-link lock file serializes future version mutations and
returns typed `Busy` to concurrent headless callers; linked or incorrectly
permissioned lock files fail closed. This narrows D-05 concurrency risk but does
not publish a version or make any candidate catalogue-eligible.

The missing catalog is material because the listed sources do not form one
interchangeable upstream binary channel:

| Component | Verified source observation | Consequence |
| --- | --- | --- |
| FFmpeg/FFprobe | [FFmpeg downloads](https://ffmpeg.org/download.html) says the project supplies source code only and links independent Linux, Windows and macOS binary distributors. Its release PGP key authenticates source releases, not those compiled binaries. | Select, independently verify and review each build, included libraries and licence; an FFmpeg version string or source signature is not binary provenance. |
| whisper.cpp CLI | The [v1.9.4 release](https://github.com/ggml-org/whisper.cpp/releases/tag/v1.9.4) has no attached assets (GitHub release API observed 2026-09-12). The [v1.9.2 release](https://github.com/ggml-org/whisper.cpp/releases/tag/v1.9.2) includes Ubuntu x64 and Windows x64 CLI archives but no macOS CLI archive; its macOS XCFramework is a library, not the CLI required by the existing process adapter. | Do not claim a three-target managed CLI from either release. A pinned earlier asset, reviewed first-party build, or explicit BYO macOS limitation needs an accepted target decision. |
| Multilingual `base` model | ADR 0005 names it as a candidate, conditional on P07 accuracy/resource gates; it does not select an exact model artifact and digest. | A model downloaded using a provider script or user-supplied checksum cannot become the reviewed VSift trust anchor. Select an exact hosted artifact, licence and digest independently. |

## Required resolution before managed install

1. Record a per-target component matrix: exact origin and release, immutable
   archive and extracted-file SHA-256, bounded compressed/extracted sizes,
   file inventory and architecture, licence/notices, publisher evidence and
   revocation policy. Keep the trust anchor in reviewed source, never in a plan
   file, download response, media, transcript or model output.
2. Decide which of the ADR 0005 qualification targets receive managed CLI builds.
   For a target without a reviewed build, return typed `managed_unavailable`
   remediation and allow explicitly configured BYO/PATH resolution; do not invent
   a download URL, promote the XCFramework to a CLI, or report that target ready.
3. Confirm that selecting third-party FFmpeg builds and the chosen model respects
   their binary-specific licences and notices. [FFmpeg's legal guidance](https://ffmpeg.org/legal.html)
   makes build configuration relevant; the VSift crate licence does not settle it.
4. Then implement D-01..D-10 and the P06 E2E stage using that reviewed immutable
   matrix. Plans must fail closed when a component/target is absent. The prior
   active version must survive every failed transfer, extraction, smoke test and
   activation boundary. No implementation status or issue closure is warranted
   before those tests and protected checks pass.

ADR 0014 now makes manual/BYO guidance an R0 fallback whenever a qualified
managed install is unavailable or fails. The read-only executable probe is not
a completed setup journey. The safe interim path remains an explicit local
provider installation followed by `setup check`, with an absolute `--ffmpeg`,
`--ffprobe` or `--whisper` selection when the tool is off-PATH. The diagnostic
reports lookup route and its unverified scope, not managed identity,
compatibility or a fresh-machine install claim. P06 remains incomplete.

## 2026-09-13 direct-origin clarification

The accepted setup design downloads a pinned, qualified artifact directly from
its publisher to the user's machine after separate plan acceptance. VSift does
not mirror or proxy the provider binary. The reviewed catalogue, rather than a
live "latest" response or user-entered URL/checksum, selects the bytes. Upgrades
require a new review and pin; the month-end BtbN candidate's two-year retention
requires an expiry and replacement policy. Plans disclose the component licence,
notices/source location and trust limits so the user can make an informed choice.

This narrows the distribution posture but does not remove the binary-specific
licence/notice review in item 3 above. FFmpeg's published checklist primarily
addresses linking and distributing FFmpeg libraries; VSift invokes a separate
executable and does not bundle it with its own release. The applicable notices
for the exact selected build still need to be recorded without asserting that
direct download constitutes legal clearance. Windows Server tone-audio smoke
still does not qualify Windows 11 or real-speech behavior.

## 2026-09-16 publication status

Infrastructure can now publish a completely rechecked prepared runtime under an
immutable provider-neutral component/version identity and atomically replace its
hashed current pointer while retaining prior versions. Interrupted pointer
publication is idempotent, and conflicting identity reuse fails closed. This closes
no source-review item above: a caller still needs an accepted catalogue entry,
compatible smoke result and accepted plan before it may invoke that primitive.
Managed installation therefore remains unavailable.

## 2026-09-21 lifecycle status

Published runtimes now retain shared per-version OS locks. Under the root-wide
installation guard, infrastructure can revalidate and atomically reselect an older
immutable version, refuse removal of the selected or live-held version, and remove
only exact manifest-owned content after obtaining the exclusive lock. A private
tombstone prevents new openers and makes partial metadata deletion and a lost
success response retryable. A native child-process regression proves a held
version blocks removal and abrupt holder exit releases the OS lock. These
provider-neutral rollback and removal primitives do not select a source or
authorize their own use. The catalogue, target
compatibility, accepted plan, public commands and bounded garbage-collection policy
remain required, so managed installation remains unavailable.
