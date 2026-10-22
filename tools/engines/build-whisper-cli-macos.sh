#!/usr/bin/env bash
# Builds whisper.cpp's command line for macOS arm64 from a checked-out source
# tree into an output tree (ADR 0032 decision D). Run by engines.yml on a hosted
# macOS runner only: `build-whisper-cli-macos.sh <source> <build> <tree>`.
#
# One self-contained executable: static libraries (no dylib to place beside it),
# Metal with its shader library embedded (so no .metal file is looked up at run
# time), Accelerate for BLAS (a system framework), no OpenMP (Apple's compiler
# has none, so leaving it off keeps every machine the same), no SDL, no tests,
# no examples but the CLI. The deployment target is macOS 11, Rust's default for
# aarch64-apple-darwin, which VSift itself is built for. Paths of the build are
# mapped away, so the bytes do not name the runner's folders.
set -euo pipefail
# Archive members without their time stamps (see build-ffmpeg-macos.sh).
export ZERO_AR_DATE=1

source_dir="$1"
build_dir="$2"
tree="$3"

cmake -S "${source_dir}" -B "${build_dir}" \
  -G "Unix Makefiles" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_OSX_ARCHITECTURES=arm64 \
  -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 \
  "-DCMAKE_C_FLAGS=-ffile-prefix-map=${source_dir}=whisper.cpp -ffile-prefix-map=${build_dir}=build" \
  "-DCMAKE_CXX_FLAGS=-ffile-prefix-map=${source_dir}=whisper.cpp -ffile-prefix-map=${build_dir}=build" \
  "-DCMAKE_OBJC_FLAGS=-ffile-prefix-map=${source_dir}=whisper.cpp -ffile-prefix-map=${build_dir}=build" \
  -DBUILD_SHARED_LIBS=OFF \
  -DGGML_BACKEND_DL=OFF \
  -DGGML_NATIVE=OFF \
  -DGGML_METAL=ON \
  -DGGML_METAL_EMBED_LIBRARY=ON \
  -DGGML_BLAS=ON \
  -DGGML_OPENMP=OFF \
  -DWHISPER_SDL2=OFF \
  -DWHISPER_BUILD_TESTS=OFF \
  -DWHISPER_BUILD_SERVER=OFF \
  -DWHISPER_CURL=OFF
cmake --build "${build_dir}" --config Release --target whisper-cli --parallel 3

mkdir -p "${tree}"
cp "${build_dir}/bin/whisper-cli" "${tree}/whisper-cli"
chmod 0755 "${tree}/whisper-cli"
cp "${source_dir}/LICENSE" "${tree}/LICENSE"
chmod 0644 "${tree}/LICENSE"

# The executable must name no library outside the system's own.
otool -L "${tree}/whisper-cli" | tail -n +2 | while read -r library _; do
  case "${library}" in
    /usr/lib/* | /System/Library/*) ;;
    *)
      echo "whisper-cli links ${library}, outside the system libraries" >&2
      exit 1
      ;;
  esac
done
