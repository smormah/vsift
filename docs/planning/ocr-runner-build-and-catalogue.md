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

Whatever installed the runner, `vsift ocr` runs `describe` first and records the runner's own
SHA-256 and version, the runtime version and the two model digests as the engine build of the
index it writes (ADR 0030 decision 7). A different build starts a new index.
