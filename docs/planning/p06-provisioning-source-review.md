# P06 provisioning source review — 2026-09-12

Status: **design/availability gate open; P06 remains planned**. This is a source
assessment, not installation or D-01..D-10 implementation evidence. Reviewed
protected-main predecessor: `777bc3e56788d43cd9a647dba54c287390c2998d`.

## Finding

ADR 0007 requires an immutable, reviewed trust anchor before a plan may authorize
download or activation. The repository has no selected artifact catalog for
FFmpeg/FFprobe, whisper.cpp CLI and multilingual `base` model: no exact per-target
URLs, archived digests, publisher identity, expected file inventory, binary licences,
or compatibility baseline. `setup check` still observes PATH only. The other setup
commands explicitly return `COMMAND_NOT_IMPLEMENTED`.

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

The safe interim user path is an explicit local provider installation followed by
`setup check`, with PATH provenance disclosed as unverified. This is existing P02
behavior, not P06 completion or a fresh-machine managed-install claim.
