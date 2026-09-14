# P06 provisioning source review — 2026-09-12

Status: **design/availability gate open; P06 remains planned**. This is a source
assessment, not installation or D-01..D-10 implementation evidence. Reviewed
protected-main predecessor: `777bc3e56788d43cd9a647dba54c287390c2998d`.

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

The verified-stream increment adds a bounded exact-size/SHA-256 check over an
injected byte source and unactivated sink. It is not exposed as managed setup
and does not authorize a user URL or checksum. The reviewed catalogue, HTTPS
transport, extraction and activation are separate pending P06 steps.

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
The full compiled-component source/notice review, catalogue and production
installer gates remain open; `setup plan` remains unavailable.

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
