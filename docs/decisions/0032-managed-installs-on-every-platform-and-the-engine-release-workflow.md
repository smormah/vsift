# ADR 0032: Managed installs on Windows, macOS and Linux, optional components, and the engine release workflow

- Status: **Proposed, 2026-10-21.** The maintainer decided the story on 2026-10-21 (one-command setup on every
  supported system: `vsift setup plan`, then `vsift setup install`) and the decisions D1 to D6 below the same day;
  the design was approved by the supervisor that day. It becomes Accepted when its last pull request merges.
- Date: 2026-10-21
- Refines: [ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md) (decision E, managed
  installation on Ubuntu 24.04 x64 only; the transaction of section 3), [ADR 0014](0014-progressive-dependency-setup.md)
  (the 2026-09-13 direct-origin clarification), [ADR 0008](0008-cli-and-json-contract.md) (the v1 contract) and
  [ADR 0027](0027-the-road-to-1-0.md) (engines are optional pinned adapters). It supersedes ADR 0023's decision E
  and, for the engines named in decision D below, ADR 0014's rule that VSift hosts no provider binary; both ADRs
  stay as they were accepted, with a note pointing here.
- Tracking: the 0.3.0 managed-installs story; [#409](https://github.com/smormah/vsift/issues/409) (online links, whose
  setup half this ADR absorbs from draft #503); the OCR and speaker-label stories (ADR 0030 and ADR 0031, in
  review), which add their components on top of it.
- Register: [L-214](../planning/known-limits.md#l-214) (this pull request); L-215 and L-216 with the catalogue and
  the engine workflow.

## Context

Managed installation exists on one host profile, Ubuntu 24.04 x86-64 (ADR 0023 decision E). On Windows and macOS a
user must find FFmpeg, FFprobe and whisper.cpp alone, which is the step new users most often fail at and the one an
agent cannot do for them. 0.3.0 adds engines that are optional (on-screen text, speaker labels, online links), each
an external tool or model pinned in the managed catalogue. The installer could only read tar archives, installed
all of a plan's components as one chain that stopped at the first failure, and had no notion of a component a user
may leave out.

The publishers' own builds were read on 2026-10-21 from their release pages and APIs only (nothing was downloaded
to a development machine):

| System | FFmpeg (the build Linux pins, `n9.0.1-11-ge47273f4d9`) | whisper.cpp v1.9.5 |
| --- | --- | --- |
| Ubuntu 24.04 x64 | BtbN `autobuild-2026-08-31-13-27`, linux64 LGPL `.tar.xz` (pinned) | `b5454/whisper-bin-ubuntu-x64.tar.gz` (pinned) |
| Windows x64 | the same release's `win64-lgpl-9.0.zip`, 147,007,942 bytes, the publisher's digest and checksum file agreeing; already reviewed (`p06-windows-artifact-candidate.md`) | `b5454/whisper-bin-x64.zip`, 8,928,640 bytes, attested upstream; built with MSVC and, most likely, the dynamic C runtime and OpenMP |
| macOS arm64 | none suitable: BtbN builds no macOS; the macOS builders found are GPL, unversioned, unsigned or without a published digest | none: upstream publishes only a library framework for Apple systems |

## Decision

### A. Platforms (D6)

The managed targets are **Ubuntu 24.04 x86-64, Windows x86-64 and macOS arm64**, the three systems VSift itself is
published for. macOS x64 and Linux on arm64 get no catalogue: VSift ships no binary for them, GitHub's Intel macOS
runners end in 2027 and ONNX Runtime no longer builds for Intel macOS. Linux stays exactly Ubuntu 24.04 (the only
distribution the reviewed whisper.cpp build was qualified on). Every other host keeps the typed answer it has
today (`unavailable_target`, with the bring-your-own remediation).

### B. Archives

Two readers join the tar, gzip and XZ ones, each written as a parser of hostile input and fuzzed:

- **A bounded zip reader** (`bounded_zip_inventory.rs`) for the Windows publishers' zips (FFmpeg, whisper.cpp,
  ONNX Runtime, Deno). Its policy is narrower than the format so that every reader of the same bytes sees the
  same files: entries tile the file from offset 0 with nothing between them, the central directory follows and
  an end record with no comment ends the file; each local header agrees byte for byte with its central record;
  stored or deflated only, unencrypted, one disk, no Zip64 (its markers and its extra block are refused), no
  comments; regular files and empty directories only (a Unix mode of a link, a device or a FIFO is refused); the
  paths, duplicates and bounds go through the provider-neutral inventory policy the tar readers use, which refuses
  `\`, `:`, `.`, `..` and duplicates under ASCII case folding. Only the selected files are expanded, each stopped
  at its declared size and accepted only when its size, CRC-32, SHA-256 and compressed length equal the review.
  It replaces the one-entry reader of draft #503.
- **A bzip2/tar reader** (`bzip2_tar_inventory.rs`) for sherpa-onnx, which publishes every prebuilt archive as
  `.tar.bz2` (D5). One stream only; a second stream or any trailing byte is refused. The decoder is the `bzip2`
  crate over `libbz2-rs-sys`, a Rust port of libbzip2 with no C build, maintained with bzip2 itself by the
  Trifecta Tech Foundation. Its licence, `bzip2-1.0.6` (permissive, BSD-style), is allowed for that crate alone in
  `deny.toml`, **pending the maintainer's approval at review**.

The plan's `format` gains `zip` and `tar_bz2`. The fuzz harness gains `zip_inventory` and `bzip2_tar_inventory`
(33 targets).

### C. Optional components (D4)

- **An optional component is a named group of managed components** that a user adds with `setup plan --include
  <name>` (repeatable): `online_links` (yt-dlp and the Deno runtime it needs, Ubuntu only, from #409 and ADR 0028),
  and, with their stories, `ocr` and `speakers`. A group, not a single component, because OCR alone needs three
  components (its runner, ONNX Runtime and its models), each with its own publisher and version. Each group's
  prose (what it adds, the one-sentence summary an agent relays, a terms notice when one applies, where it is
  offered) lives in one place, `vsift-contract`'s `optional_component.rs`.
- **The plan** lists every optional group of the release (the desktop profile; a worker plan lists only what it
  includes) in the additive `optional_components` array: `included`, `not_included` with the command that adds it,
  or `unavailable` on this host; and `operator_decision_required`, the instruction to whoever runs setup to ask the
  operator before accepting. An included group's actions are nested in its entry, so the top-level `actions` stay
  the core's. A group joins the digest only when included, so a plan without `--include` is digested as before.
- **The transaction is split** (`install_managed_plan`). The core installs first, exactly as before: the first
  failure stops it and every optional action is `blocked`. Then each included group installs as a part of its
  own, its pending components staged and smoked together; a group's failure stops only that group. When the core
  is installed and a group is not, the result is **`partial`, exit 0**, with the warning
  `optional_component_not_installed` and, in the additive `optional_groups` array, the managed component that
  failed, its typed reason and failure code and fixed advice that says local investigation is not affected.
- **`setup check`** gains the additive `optional_components` array: every group with `installed` (and the
  versions), `not_installed` (with the command that plans it, on the desktop profile) or `unavailable`. It never
  changes the local-video readiness.
- **New components appear only behind `--include`** (D3): a plan, an install or a check that does not include an
  optional group names no new component.

### D. VSift-built engines (D1, D2)

Where no publisher offers a suitable build, VSift builds one from a pinned upstream source in its own engine release
workflow (section E) and publishes it on this repository's releases; the catalogue pins the asset by size and SHA-256
like any other, through the reviewed GitHub release route.

- **whisper.cpp's command-line build for macOS arm64**, from tag v1.9.5 (`d1be6fde11ac`): static, Metal embedded,
  OpenMP off, no SDL. Upstream's licence is MIT; the archive carries it.
- **An LGPL static FFmpeg for macOS arm64 (D1, the maintainer's decision of 2026-10-21)**, from FFmpeg commit
  `e47273f4d9227152dcbf543cebaf9e2430ddbcc4`, the commit the Linux and Windows pins are built from, configured
  without `--enable-gpl`, `--enable-version3` or `--enable-nonfree`. VSift then distributes an LGPL binary, so the
  release carries the licence text and the exact corresponding source (the source archive of that commit and the
  build recipe) beside the binary, for as long as the binary is published. **This supersedes ADR 0014's
  direct-origin rule for these engines**: VSift does host these binaries. It does not change the rule for any
  binary a publisher builds; VSift still mirrors nothing.
- **whisper.cpp for Windows (D2, the supervisor's decision):** a hosted inventory of `b5454/whisper-bin-x64.zip`
  first reads what its executables import. If they import the Microsoft C runtime or OpenMP DLLs, which a clean
  Windows need not have, VSift builds its own (a static C runtime, OpenMP off, the CPU-variant back ends kept) and
  reruns P07's accuracy check on it: one-command setup needs no prerequisite.
- **The OCR runner** (its story) and the PP-OCRv5 models, whose only pinned source is ModelScope: the models are
  republished unchanged with the runner's release, checked by digest, under their Apache-2.0 notice.

### E. The engine release workflow

`.github/workflows/engines.yml` builds a VSift-made engine from a tagged commit on `main` and publishes it under an
engine tag (`whisper-cli-macos-v1.9.5-r1`, `ffmpeg-macos-v…`, `ocr-runner-v…`). It is held to the release
workflow's standard by its own lint rules, each with broken-copy mutation tests: dispatch only, `dry_run` by
default; a guard that the tag still names the run's commit and that the commit is on `main`; builds on hosted
runners only (no `loki`, no self-hosted label), each built twice and compared; build provenance attested in an
`attest` job, the only one with `id-token`; publishing only in a `publish` job in the protected `release`
environment, after the maintainer's approval, as a draft on the existing tag and then published, **never marked
latest** (`--latest=false` on both steps), never overwriting an asset. Details are in its pull request.

### F. Proof

`P13 managed smoke` runs on all three systems with real downloads on the hosted runners: `setup plan`, `setup
install`, `setup check`, a transcription and a frame.

### G. The v1 contract (D3)

The rule written into [`cli-v1.md`](../contracts/cli-v1.md): **the catalogue-content fields may gain values within
v1**: a plan action's `format`, `component` and `id`, the names and components of optional components, and the
maximum number of actions an optional component carries. Readers must not treat them as closed lists, and skip a
value they do not know. Every other v1 rule stands. The members this ADR adds are additive and written only where
they have something to say: `optional_components` and `operator_decision_required` in a plan; `optional_components`
in `setup check`; `core`, `optional_components` and `optional_groups` in `setup install` only when the plan
includes an optional group.

## Consequences

- One command sets up VSift on all three systems once the catalogue lands; every optional engine reuses the
  same install, lookup, smoke and removal path, and adds only its pins, its prose and its smoke.
- An optional engine can fail to install (a proxy, an unsupported CPU) without a user losing the core.
- VSift now publishes two kinds of binary it did not build from its own source, and the LGPL obligations of one of
  them; the engine workflow's rules and the release environment's approval are the controls.
- The zip reader refuses archives some writers produce (Zip64, comments, backslash separators): a publisher that
  changes its writer makes a re-pin fail at review rather than at a user's install
  ([L-214](../planning/known-limits.md#l-214)).

## Maintainer-only actions

- Approve the `bzip2-1.0.6` licence exception for `libbz2-rs-sys` (slice 1).
- Approve `engines.yml` and its lint, and every engine publish (slice 2).
- Add a tag ruleset for the engine tags (`*-v*` other than `v*`), and add those tag patterns to the `release`
  environment's allowed deployment refs (slice 2).

## Implementation note, 2026-10-21 (slice 1: the archive readers and optional components)

The zip and bzip2/tar readers, their fuzz targets and seeds; `ManagedArtifactFormat::Zip` and `TarBz2`; optional
components as groups (`OptionalComponent`, `--include`, the plan's `optional_components`, the split transaction,
`partial`, `setup check`'s `optional_components`); and the `online_links` group with draft #503's yt-dlp 2026.08.19
and Deno 2.9.7 pins on Ubuntu (catalogue revision `ubuntu-24.04-x86_64-2026-10-21-r4`, hosted review run
39535463204). Reused from #503: the plan and install shapes, the core-then-optional transaction, the pins and their
hosted review; replaced: its one-entry zip reader (by the general one) and its online-video-only members (by the
group-generic `optional_groups` and `optional_components`).

## Implementation note, 2026-10-21 (slice 2: the engine release workflow)

- **D2 measured.** The hosted inventory of `b5454/whisper-bin-x64.zip` (run 39604340709, `tools/p06_archive_inventory.py`,
  which now reads what each PE file imports) shows `whisper-cli.exe`, `whisper.dll` and the ggml DLLs importing
  `msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll` and, for ggml, `vcomp140.dll`: the Visual C++
  redistributable a clean Windows need not have. The same run shows FFmpeg's Windows build importing only the
  universal C runtime and system DLLs, and both zips using only what the zip reader accepts (deflate, with stored in
  FFmpeg's; no Zip64, comments or backslash names). So VSift builds whisper.cpp for Windows too: one executable with
  the static C runtime, no OpenMP and the AVX2, FMA and F16C instructions every processor Windows 11 accepts has;
  P07's accuracy check is rerun on it before the catalogue pins it.
- **The workflow** (`.github/workflows/engines.yml`, runbook [`engines.md`](../operations/engines.md)) has the jobs
  `guard`, `build` (a literal matrix of three legs on `macos-15`, `windows-2025` and `macos-15`; a dispatch builds only
  the chosen engine's leg), `plan`, `attest` and `publish`. Recipes and build scripts are in `tools/engines/`; each
  build fetches the pinned commit, builds twice, compares byte for byte and packages deterministically with an
  inventory the catalogue pins from.
- **Rule 10 of the workflow lint** (`workflows/engines.rs`) holds it to the release workflow's standard, with 55
  broken copies of the real file each refused (28 at first, 27 added by the review of 2026-10-22 below); rules 4 and 9 now name the engine workflow too (its `attest` job is
  the only other one with `id-token`, and it has no LOKI job).
- **The OCR runner** (the coordinator's request of 2026-10-21) is a fourth engine with three targets: VSift's own
  crate built as VSift is, its folder assembled by the OCR story's stage tool (the runtime library and the models
  by pinned size and SHA-256, republished unchanged), and `describe` checked. Open: the managed store keeps a
  component's files flat, and the runner looks in `onnxruntime/` and `models/`; the recommendation is that the
  runner also accepts them beside itself.
- **Dry runs:** `Engines` run 39610758455 built every upstream engine twice, byte for byte the same (the first run,
  39607212953, found the macOS FFprobe builds different until `ZERO_AR_DATE=1`).
- **Nothing is published by this slice.** The first publishes need the maintainer's tag ruleset for the engine tags,
  the engine tag patterns in the `release` environment's allowed refs, a tag and an approval each.

## Review note, 2026-10-22 (RV-3 on the engine release workflow)

A read-only review found the workflow sound and the lint short of what the maintainer is asked to rely on. Changes
in the same pull request; the design is as above.

- **The lint holds the tag guard (finding 16).** The guard's checks (the ref is the engine's tag and never `v*`, the
  tag still names the run's commit, the commit is on `main`, no release, no draft) had been shell text only, so
  deleting any left every rule green. Each is now a required whole command in the guard's script, in front of the
  line that sets `mode=publish`; the accepted engine names are exactly the dispatch input's options and none starts
  with `v`; `publish` repeats the ref, tag and draft checks after the approval; and `ENGINE_TAG`, `GIT_REF`,
  `SOURCE_COMMIT` and `REPOSITORY` are bound only to the guard's output or the `github` context fields, never
  `github.ref_name`. Fifteen broken copies pin it.
- **A draft is a release (10).** GitHub answers 404 to a tag lookup for a draft, so the guard lists the releases as
  well and refuses with "a draft exists; delete it first"; `publish` checks again with its write token. Whether a
  read-only job token lists drafts is not something the review could confirm, which is why the second check
  exists.
- **The plan names the targets (12).** The guard reads the recipe's `TARGETS` into an output; `plan` fails unless
  exactly those targets' sums files are present; and the lint compares the build matrix with the recipes.
- **Spellings (13, 14).** `gh` with global flags before `release`, `-XDELETE`, `-fkey=value` and `-Fkey=value` are
  refused; `secrets` is refused as a word in any form.
- **One publish group for each engine tag (15):** `engines-publish-${{ needs.guard.outputs.tag }}`, still never
  cancelled, so a queued publish of one engine is not replaced by another's.
- **Accepted, not changed (11).** `attest` signs the planned files before the maintainer's approval at `publish`. A
  rejected publish leaves a provenance statement for files that were never released: it says what was built, not
  that it was released, and the catalogue pins only a release's assets. Moving `attest` into the `release`
  environment would mean two approvals for each publish, so it stays where it is.
