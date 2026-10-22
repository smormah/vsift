#!/usr/bin/env bash
# Builds whisper.cpp's command line for Windows x64 from a checked-out source
# tree into an output tree (ADR 0032 decision D2). Run by engines.yml on a hosted
# Windows runner (Git Bash, MSVC through CMake's Visual Studio generator) only:
# `build-whisper-cli-windows.sh <source> <build> <tree>`.
#
# One self-contained executable that a clean Windows runs with nothing
# installed: the static C runtime (/MT, so no vcruntime140 or msvcp140) and no
# OpenMP (so no vcomp140; ggml's own thread pool runs the work), static
# libraries (no ggml or whisper DLL), no SDL. With one executable there is no
# loader choosing a CPU back end, so the CPU features are fixed: AVX2, FMA and
# F16C, which every processor Windows 11 accepts has; a processor without them
# fails the managed smoke (L-216). -Brepro (MSVC's /Brepro, written with a dash
# so Git Bash does not take it for a path) makes the compiler and linker leave
# out time stamps, so a second build compares byte for byte.
set -euo pipefail

source_dir="$1"
build_dir="$2"
tree="$3"

cmake -S "${source_dir}" -B "${build_dir}" \
  -A x64 \
  -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded \
  "-DCMAKE_C_FLAGS=-Brepro" \
  "-DCMAKE_CXX_FLAGS=-Brepro -EHsc" \
  "-DCMAKE_EXE_LINKER_FLAGS=-Brepro" \
  "-DCMAKE_STATIC_LINKER_FLAGS=-Brepro" \
  -DBUILD_SHARED_LIBS=OFF \
  -DGGML_BACKEND_DL=OFF \
  -DGGML_NATIVE=OFF \
  -DGGML_AVX=ON \
  -DGGML_AVX2=ON \
  -DGGML_FMA=ON \
  -DGGML_F16C=ON \
  -DGGML_OPENMP=OFF \
  -DWHISPER_SDL2=OFF \
  -DWHISPER_BUILD_TESTS=OFF \
  -DWHISPER_BUILD_SERVER=OFF \
  -DWHISPER_CURL=OFF
cmake --build "${build_dir}" --config Release --target whisper-cli --parallel 3

mkdir -p "${tree}"
cp "${build_dir}/bin/Release/whisper-cli.exe" "${tree}/whisper-cli.exe"
cp "${source_dir}/LICENSE" "${tree}/LICENSE"
