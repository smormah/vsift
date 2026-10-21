#!/usr/bin/env bash
# Builds the OCR runner folder for one target from this repository's source
# into an output tree (ADR 0030, ADR 0032 decision D). Run by engines.yml on a
# hosted runner only: `build-ocr-runner.sh <repository> <build> <tree>`, with
# TARGET set to linux-x64, windows-x64 or macos-arm64.
#
# The runner is built as VSift itself is (release.yml): `--release --locked`,
# no feature, the static C runtime and -Brepro on Windows (MSVC's /Brepro,
# written with a dash so Git Bash does not take it for a path), Linux on Ubuntu
# 22.04 so the binary runs on newer glibc. The folder around it is assembled by
# the OCR story's own stage tool (`tools/p16_ocr_runner.py stage`), which fetches
# Microsoft's ONNX Runtime 1.31.0 archive and the two PP-OCRv5 models by their
# pinned sizes and SHA-256 and writes them unchanged, so the pins have one home.
# The runner's own licences join them, and the runner must then describe
# itself with the pinned runtime and models.
set -euo pipefail

repository="$1"
build_dir="$2"
tree="$3"
: "${TARGET:?the leg names its target}"
: "${CRATE:?the recipe names the crate}"
python="$4"

case "${TARGET}" in
  linux-x64)
    rust_target=x86_64-unknown-linux-gnu
    platform=linux-x64
    executable=vsift-ocr-runner
    rustflags=""
    ;;
  windows-x64)
    rust_target=x86_64-pc-windows-msvc
    platform=win-x64
    executable=vsift-ocr-runner.exe
    rustflags="-C target-feature=+crt-static -C link-arg=-Brepro"
    ;;
  macos-arm64)
    rust_target=aarch64-apple-darwin
    platform=osx-arm64
    executable=vsift-ocr-runner
    rustflags=""
    ;;
  *)
    echo "the OCR runner has no target '${TARGET}'" >&2
    exit 1
    ;;
esac

mkdir -p "${build_dir}"
(
  cd "${repository}"
  CARGO_TARGET_DIR="${build_dir}/cargo" RUSTFLAGS="${rustflags}" \
    cargo build --release --locked -p "${CRATE}" --bin "${CRATE}" --target "${rust_target}"
)
built="${build_dir}/cargo/${rust_target}/release/${executable}"

"${python}" "${repository}/tools/p16_ocr_runner.py" stage --out "${tree}" --runner "${built}" \
  --platform "${platform}" --inventory "${build_dir}/stage-inventory.json"
for licence in LICENSE-MIT LICENSE-APACHE; do
  cp "${repository}/${licence}" "${tree}/LICENSES/vsift-ocr-runner-${licence}"
done

# The runner must start and describe itself with the pinned runtime and models.
"${python}" "${repository}/tools/p16_ocr_runner.py" describe --runner "${tree}/${executable}"
