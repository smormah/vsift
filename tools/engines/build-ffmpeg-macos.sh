#!/usr/bin/env bash
# Builds an LGPL static FFmpeg and FFprobe for macOS arm64 from a checked-out
# source tree into an output tree (ADR 0032 decision D1). Run by engines.yml on a
# hosted macOS runner only: `build-ffmpeg-macos.sh <source> <build> <tree>`.
#
# The licence is LGPL 2.1 or later: no --enable-gpl, --enable-version3 or
# --enable-nonfree, and no external library but the system's (zlib, which the
# PNG encoder needs, and the VideoToolbox and AudioToolbox frameworks); every
# other library FFmpeg would detect is left out (--disable-autodetect), so the
# result does not depend on what a runner image happens to have. The libraries
# are linked statically into the two programs; nothing else is built (no
# ffplay, no documentation). The version banner is the description the Linux
# and Windows pins carry with VSift's recipe revision after it, written into the
# build's own copy of the source as FFmpeg's VERSION file (the published source
# archive is the upstream commit, untouched). Paths of the build are mapped
# away, so the bytes do not name the runner's folders.
set -euo pipefail
# Archive members without their time stamps, so the static libraries, and
# what the linker makes from them, are the same bytes on a second build.
export ZERO_AR_DATE=1

source_dir="$1"
build_dir="$2"
tree="$3"
: "${UPSTREAM_DESCRIBE:?the recipe names the upstream description}"
: "${ENGINE_VERSION:?the recipe names the engine version}"
revision="vsift-${ENGINE_VERSION##*-}"

work="${build_dir}/source"
rm -rf "${build_dir}"
mkdir -p "${build_dir}"
cp -R "${source_dir}" "${work}"
rm -rf "${work}/.git"
printf '%s\n' "${UPSTREAM_DESCRIBE}" > "${work}/VERSION"

prefix_map="-ffile-prefix-map=${work}=ffmpeg -ffile-prefix-map=${build_dir}=build"
(
  cd "${work}"
  ./configure \
    --prefix="${build_dir}/install" \
    --arch=arm64 \
    --cc=clang \
    --extra-version="${revision}" \
    --extra-cflags="-mmacosx-version-min=11.0 ${prefix_map}" \
    --extra-ldflags="-mmacosx-version-min=11.0" \
    --enable-static \
    --disable-shared \
    --disable-autodetect \
    --enable-zlib \
    --enable-videotoolbox \
    --enable-audiotoolbox \
    --disable-ffplay \
    --disable-doc \
    --disable-debug
  make -j3
  make install
)

mkdir -p "${tree}/bin"
for program in ffmpeg ffprobe; do
  cp "${build_dir}/install/bin/${program}" "${tree}/bin/${program}"
  chmod 0755 "${tree}/bin/${program}"
done
for notice in LICENSE.md COPYING.LGPLv2.1; do
  cp "${source_dir}/${notice}" "${tree}/${notice}"
  chmod 0644 "${tree}/${notice}"
done

# Neither program may name a library outside the system's own, and the banner
# must be the reviewed one.
for program in ffmpeg ffprobe; do
  otool -L "${tree}/bin/${program}" | tail -n +2 | while read -r library _; do
    case "${library}" in
      /usr/lib/* | /System/Library/*) ;;
      *)
        echo "${program} links ${library}, outside the system libraries" >&2
        exit 1
        ;;
    esac
  done
  # Whole outputs are read first: a reader that stops early would end the
  # program with SIGPIPE, which pipefail turns into a failure.
  version_output="$("${tree}/bin/${program}" -version)"
  first_line="$(printf '%s\n' "${version_output}" | sed -n 1p)"
  case "${first_line}" in
    "${program} version ${UPSTREAM_DESCRIBE}-${revision} "*) ;;
    *)
      echo "${program} reports '${first_line}', not version ${UPSTREAM_DESCRIBE}-${revision}" >&2
      exit 1
      ;;
  esac
  configuration="$("${tree}/bin/${program}" -hide_banner -buildconf)"
  case "${configuration}" in
    *--enable-gpl* | *--enable-nonfree* | *--enable-version3*)
      echo "${program} was configured outside LGPL 2.1 or later" >&2
      exit 1
      ;;
  esac
done
echo "Built FFmpeg ${UPSTREAM_DESCRIBE}-${revision} (LGPL) for macOS arm64."
