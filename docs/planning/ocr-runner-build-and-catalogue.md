# OCR runner: build recipe and catalogue data (0.3.0, ADR 0030)

Status (2026-10-21, 0.3.0 development): **input for the engine release workflow and the
cross-platform managed install, which the infrastructure work owns (ADR 0030 decision 2).
Nothing here is published yet.** The runner and its ONNX Runtime backend are in the tree; its
parity with the reference pipeline is measured by the hosted workflow `P16 OCR runner`
([L-204](known-limits.md#l-204)), which also stages this folder on Linux, Windows and macOS and
writes its inventory (every file's size and SHA-256) as `inventory-<platform>.json`. The models are
republished unchanged with the runner release (the coordinator's decision of 2026-10-21).

## 1. What a runner install is

One folder, with nothing outside it and nothing read from the environment
(`crates/vsift-ocr-runner/src/cli.rs`, `Layout::beside`):

| Path in the folder | What | From |
| --- | --- | --- |
| `vsift-ocr-runner` (`vsift-ocr-runner.exe` on Windows) | The runner | Built by the engine release workflow from this repository |
| `onnxruntime/` | The ONNX Runtime shared library (`libonnxruntime.so.1.31.0` and its `libonnxruntime.so.1` link on Linux, `libonnxruntime.1.31.0.dylib` on macOS, `onnxruntime.dll` on Windows) | Microsoft's release archive, `lib/` |
| `models/ch_PP-OCRv5_det_mobile.onnx` | PP-OCRv5 mobile text detector | Section 4 |
| `models/latin_PP-OCRv5_rec_mobile.onnx` | PP-OCRv5 mobile Latin recognizer; its dictionary is read from the model's own `character` metadata | Section 4 |
| `LICENSES/` | The runner's licence, ONNX Runtime's `LICENSE` and `ThirdPartyNotices.txt`, PaddleOCR's Apache-2.0 licence for the models | The three sources |

The runner also accepts the same files **flat beside the executable** (the managed store keeps a
component's files in one folder): for each file it looks in its subfolder first, then for the same
name next to the executable (`Layout::beside`, tests
`files_are_found_in_their_subfolders_first_then_beside_the_executable` and
`describe_works_from_the_managed_stores_flat_folder`).

`vsift ocr` runs the folder's executable, which it is given by `--runner`, by
`setup configure ocr-runner --executable`, or (later) by the managed install. The library file
names inside `onnxruntime/` are those of Microsoft's 1.31.0 archives; the release workflow's
inventory confirms them before any entry is written.

## 2. Build recipe

On each target (hosted Ubuntu 24.04 x64, macOS 15 arm64, Windows Server 2025 x64), from the
release tag `ocr-runner-v<version>`:

```console
cargo build --release --locked -p vsift-ocr-runner
```

- The runner is a workspace member and uses the workspace's toolchain, lints and `Cargo.lock`;
  it depends on no VSift crate and no VSift crate depends on it.
- `cargo test --locked -p vsift-ocr-runner` runs its pipeline tests without the runtime.
- The release job then assembles the folder of section 1, smokes it (`describe` must answer
  protocol 1 with the two model digests of section 4 and runtime version `1.31.0`; `read` on a
  rendered frame must give the line the frame shows), archives it as `.tar.xz` (Linux, macOS) or
  `.tar.gz` (the catalogue's formats; no zip), and attests it as `release.yml` does. The release
  is never marked latest.

## 3. ONNX Runtime 1.31.0 (Microsoft's GitHub release `v1.31.0`)

| Target | Asset | Bytes | SHA-256 (GitHub's asset digest) |
| --- | --- | ---: | --- |
| Ubuntu 24.04 x64 | `onnxruntime-linux-x64-1.31.0.tgz` | 11,408,003 | `cc5c72baf5ae5c8238a6841f0897227be2d02826b9cf98eaf02fdefaeeb03a57` |
| macOS 15 arm64 | `onnxruntime-osx-arm64-1.31.0.tgz` | 36,549,009 | `1031b36dd3bdaa8d976a6130d9cf2a05e9be320b93ae804f8427b91dc3432afc` |
| Windows x64 | `onnxruntime-win-x64-1.31.0.zip` | 82,887,349 | `3958d8a44984160692c1b4ffd0de3cb48f8c39f9d34bb31e433db06e30b82195` |

Source: `https://github.com/microsoft/onnxruntime/releases/download/v1.31.0/<asset>`. Licence:
MIT, with `ThirdPartyNotices.txt`. The Windows asset is a zip, which the managed catalogue does not
accept; the release workflow therefore carries the library inside the runner's own archive on every
target rather than as a component of its own (proposed; the infrastructure work decides).

## 4. The models

The evaluation's files (RapidOCR's ModelScope list at `v3.10.0`, the SHA-256 values RapidOCR
publishes):

| File | Bytes | SHA-256 | Source |
| --- | ---: | --- | --- |
| `ch_PP-OCRv5_det_mobile.onnx` | 4,819,576 | `4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae` | `https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/det/ch_PP-OCRv5_det_mobile.onnx` |
| `latin_PP-OCRv5_rec_mobile.onnx` | 7,904,513 | `b20bd37c168a570f583afbc8cd7925603890efbcdc000a59e22c269d160b5f5a` | `https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/latin_PP-OCRv5_rec_mobile.onnx` |

Licence: Apache-2.0 (PaddleOCR's models, converted to ONNX by RapidOCR). The managed catalogue
accepts GitHub release and Hugging Face revision sources only, so **proposed**: the engine release
workflow fetches the two files by exact size and SHA-256 and publishes them unchanged with the
runner release (inside the runner's archive, or as assets of the same `ocr-runner-v<version>`
release), with PaddleOCR's licence and a notice naming the source. The catalogue then pins only
GitHub release URLs. These digests are also what `describe` reports, so a session's index names the
exact models that read it.

## 5. Catalogue entry data

The managed components a runner install becomes (the field names are those of the catalogue's
accepted artifacts):

| Field | Runner (with its runtime and models) |
| --- | --- |
| `component` | `ocr_runner` (proposed) |
| `version` | `0.1.0` |
| `publisher` | this repository's releases |
| `source_url` | `https://github.com/smormah/vsift/releases/download/ocr-runner-v0.1.0/<archive>` |
| `integrity` | the archive's size and SHA-256, from the release workflow's output |
| `format` | `TarXz` (Linux, macOS); `TarGz` (Windows) |
| `selected_files` | the folder of section 1, each with its size and SHA-256 from the release inventory |
| `licence` | the runner's licence, MIT (ONNX Runtime) and Apache-2.0 (models), with their notice files |
| `trust_limit` | the runner reads untrusted pixels; it runs only under VSift's supervisor, as an external tool |

The per-file sizes and digests come from the first hosted release run; none can be written before
it, and none is guessed here.

## 6. What VSift checks at run time

The runtime's version in `describe` is named by the library's SHA-256 (`KNOWN_RUNTIMES` in
`crates/vsift-ocr-runner/src/onnx.rs`): ONNX Runtime 1.31.0's build information says
`git-branch=HEAD`, and its version string is reachable only through raw C calls. A pinned library is
`1.31.0`; any other library is `sha256-<16 hex digits>`, a different engine build. Linux x64's
library is `469849977cb60744ea160927e1a3a0643fd4c55c0aff743251c6d293402e334e` (29,345,384 bytes),
Windows x64's `0c42dc4e278f0968fdf9ebf1b1e54a74c657a9942e70803e453c91993451cf4c` (17,253,176 bytes)
and macOS arm64's `2a50a63e7f41f8d4d551d3ee47248df3864f6540109fce8ad520ca3b51af4b70`
(44,830,288 bytes).

## 7. The staged folder, per platform (run 39604405934)

Every file but the runner itself is the same bytes on every release of runner 0.1.0; the runner's
digest is that run's build and changes with the release build. The release workflow writes its own
inventory the same way (`tools/p16_ocr_runner.py stage`).

| File | Ubuntu 24.04 x64 | Windows x64 | macOS 15 arm64 |
| --- | --- | --- | --- |
| `models/ch_PP-OCRv5_det_mobile.onnx` | 4,819,576 bytes, `4d97c44a...78ae` | same | same |
| `models/latin_PP-OCRv5_rec_mobile.onnx` | 7,904,513 bytes, `b20bd37c...5f5a` | same | same |
| `onnxruntime/<library>` | `libonnxruntime.so.1.31.0`, 29,345,384 bytes, `46984997...334e` | `onnxruntime.dll`, 17,253,176 bytes, `0c42dc4e...cf4c` | `libonnxruntime.1.31.0.dylib`, 44,830,288 bytes, `2a50a63e...4b70` |
| `LICENSES/onnxruntime-LICENSE` | 1,073 bytes, `2f07c727...922c` | 1,094 bytes, `c250d627...b674` | as Ubuntu |
| `LICENSES/onnxruntime-ThirdPartyNotices.txt` | 338,088 bytes, `143764b9...3de3` | 344,457 bytes, `c53a7650...6622` | as Ubuntu |
| `LICENSES/models-NOTICE.txt` | 489 bytes, `8d8a53d6...1745` | same | same |
| the runner | 633,768 bytes (this build) | 491,520 bytes (this build) | 613,136 bytes (this build) |

The full digests are in the run's log and its `inventory-<platform>.json` artifacts.

Whatever installed the runner, `vsift ocr` runs `describe` first and records the runner's own
SHA-256 and version, the runtime version and the two model digests as the engine build of the
index it writes (ADR 0030 decision 7). A different build starts a new index.
