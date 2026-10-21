# Engine releases

VSift builds a few engines itself, where no publisher offers a build the managed installer can use
([ADR 0032](../decisions/0032-managed-installs-on-every-platform-and-the-engine-release-workflow.md) sections D
and E). Each is built from a pinned upstream commit by the `Engines` workflow (`.github/workflows/engines.yml`),
published on this repository's releases under an engine tag, and then pinned by the managed catalogue by size and
SHA-256 like any publisher's file. An engine release is not a VSift release: VSift's own releases keep their
`v<version>` tags and are the only ones ever marked latest.

| Engine | Why VSift builds it | Upstream | Licence | Runner |
| --- | --- | --- | --- | --- |
| `whisper-cli-macos` | whisper.cpp publishes no macOS command-line build | ggml-org/whisper.cpp v1.9.5 (`d1be6fde11ac`) | MIT | `macos-15` |
| `whisper-cli-windows` | upstream's Windows build needs the Microsoft C runtime and OpenMP (hosted inventory, run 39604340709) | the same commit | MIT | `windows-2025` |
| `ffmpeg-macos` | no publisher offers an LGPL static macOS build of the pinned version (the maintainer's decision D1) | FFmpeg `e47273f4d922` (`n9.0.1-11-ge47273f4d9`) | LGPL 2.1 or later | `macos-15` |
| `ocr-runner` | VSift's own runner (ADR 0030), with ONNX Runtime 1.31.0's library and the two PP-OCRv5 models beside it, which ModelScope alone publishes | this repository's `crates/vsift-ocr-runner` at the tagged commit; Microsoft's v1.31.0 archives and RapidOCR's ModelScope list v3.10.0, each by pinned size and SHA-256 (`tools/p16_ocr_runner.py stage`) | MIT or Apache-2.0 (runner), MIT (runtime), Apache-2.0 (models) | `ubuntu-22.04`, `windows-2025`, `macos-15` |

An engine with several targets (the OCR runner) builds one leg per target; the plan merges the legs into one
release.

## What a build does

`tools/engines/build-engine.sh <engine>`, on a GitHub-hosted runner only:

1. reads `tools/engines/recipes/<engine>.env` (the engine's version `<upstream>-r<n>`, the upstream repository,
   commit and tag, and the licence);
2. fetches exactly that upstream commit and refuses any other; when the recipe names a tag, the tag must name the
   same commit upstream;
3. builds twice with `tools/engines/build-<engine>.sh` and compares the two builds file by file, byte for byte;
4. packages the first deterministically (`tools/engines/package_engine.py`: one root folder, path order, owner 0,
   the upstream commit's time, mode 0755 or 0644, gzip without a name) and checks that a second package of the
   second build is the same bytes;
5. writes `dist/`: `<engine>-<version>.tar.gz`, its `.inventory.json` (its size and SHA-256, entry count,
   expanded bytes, tar stream size and every file's size and SHA-256: what the catalogue pins), its
   `.build-info.json` (upstream commit, recipe digest, runner image) and, for an LGPL engine, the corresponding
   source `<engine>-<version>-source.tar.gz` (the upstream commit's tree, untouched), with `SHA256SUMS`.

Each build script checks what it made: the executables link nothing but the system's libraries; FFmpeg reports the
reviewed version banner (`n9.0.1-11-ge47273f4d9-vsift-r<n>`) and was configured without `--enable-gpl`,
`--enable-version3` or `--enable-nonfree`.

## Cutting an engine release

1. **Change the recipe** (a new upstream commit, or the same one with a new revision `-r<n>`) by pull request. The
   pull request runs the `Engines` workflow as a dry run of every engine: each builds twice, is compared and is
   packaged, and nothing is published. Merge it to `main`.
2. **The maintainer creates the tag** `<engine>-v<version>` (for example `whisper-cli-macos-v1.9.5-r1`) on that
   commit of `main`. An engine release is made once: a tag that has a release already is refused, so a change is a
   new revision.
3. **Dispatch a dry run on the tag:** `gh workflow run engines.yml --repo smormah/vsift --ref <tag> -f
   engine=<engine>`. Read the plan in the run's summary (the files and their digests).
4. **Dispatch the publish:** the same with `-f dry_run=false`. The guard refuses it unless the ref is the engine's
   tag, the tag still names the run's commit, the commit is on `main` and the tag has no release. After the build
   and the plan, `attest` attests every file with build provenance, and `publish` waits for the maintainer's
   approval of the `release` environment. It then creates the release as a draft on the tag with exactly the
   planned files, publishes it with `--latest=false`, and reads the assets back by digest.
5. **Pin it in the catalogue** by pull request: the asset's URL
   (`https://github.com/smormah/vsift/releases/download/<tag>/<engine>-<version>.tar.gz`), size and SHA-256, and
   the selected files from the published `.inventory.json`, then the hosted managed smoke on the engine's system.

Anyone can check a file: `gh attestation verify <file> --repo smormah/vsift`.

## Maintainer-only settings

- **A tag ruleset for the engine tags** (`whisper-cli-macos-v*`, `whisper-cli-windows-v*`, `ffmpeg-macos-v*`, and
  each new engine's pattern) restricting creation, update and deletion to the maintainer, as the `v*` tags are.
- **The `release` environment's allowed deployment refs** must include the engine tag patterns, beside `v*`; its
  required reviewer stays the maintainer.

## The rules the workflow is held to

The governance workflow lint's rule 10 (`tools/vsift-governance/src/workflows/engines.rs`), with a test that
refuses a broken copy of the real workflow for each rule: triggers only `pull_request` and `workflow_dispatch` with
`engine` and `dry_run` (default `true`); the guard first, never skipped; `attest` and `publish` only under every
publish condition; write scopes only in `attest` (`id-token`, `attestations`) and `publish` (`contents`); `publish`
alone in the `release` environment and never cancelled; `gh release` only to create a draft with `--verify-tag`,
`--draft` and `--latest=false`, publish it with `--draft=false --latest=false` and view it; no upload, `--clobber`,
deletion, tag or push; `gh api` only reads; the privileged jobs download only this run's engine artifact and
compare it with the plan's sums; no secret. Rule 9 keeps every job on GitHub-hosted runners.
