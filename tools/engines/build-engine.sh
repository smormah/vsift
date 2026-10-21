#!/usr/bin/env bash
# Builds one VSift-made engine twice from its pinned upstream source, compares
# the two builds byte for byte and packages the first deterministically into
# `dist/` (ADR 0032 section E). Run by engines.yml's build job on a hosted runner
# only: `build-engine.sh <engine>`, from the repository root.
#
# The upstream source is fetched by its commit, not by a branch, and must be
# exactly that commit; when the recipe names a tag, the tag must name the same
# commit upstream. Nothing else is fetched: the build scripts use the runner's
# own compiler and system libraries.
#
# dist/ then holds:
#   <stem>.tar.gz              the engine (package_engine.py)
#   <stem>.inventory.json      its size, digest, bounds and every file's digest
#   <stem>.build-info.json     where it came from and what built it
#   <stem>-source.tar.gz       the corresponding source, for an LGPL engine
#   SHA256SUMS                 the digest of each of the above
set -euo pipefail

engine="$1"
case "${engine}" in
  whisper-cli-macos | whisper-cli-windows | ffmpeg-macos) ;;
  *)
    echo "unknown engine '${engine}'" >&2
    exit 1
    ;;
esac
recipe="tools/engines/recipes/${engine}.env"
set -a
# shellcheck source=/dev/null
. "${recipe}"
set +a
[ "${ENGINE}" = "${engine}" ] || { echo "${recipe} names ${ENGINE}, not ${engine}" >&2; exit 1; }
[[ "${UPSTREAM_COMMIT}" =~ ^[0-9a-f]{40}$ ]] || { echo "the recipe's commit is not a full SHA" >&2; exit 1; }
[[ "${ENGINE_VERSION}" =~ ^[0-9][0-9A-Za-z.]*-r[0-9]+$ ]] || { echo "the recipe's version '${ENGINE_VERSION}' is not <upstream>-r<n>" >&2; exit 1; }

# Git Bash on a Windows runner: a POSIX form of the runner's temporary folder,
# and its real Python (a `python3` there can be the Store's stand-in).
case "$(uname -s)" in
  MINGW* | MSYS*)
    temp_root="$(cygpath -u "${RUNNER_TEMP:?}")"
    python="$(command -v python)"
    ;;
  *)
    temp_root="${RUNNER_TEMP:?}"
    python="$(command -v python3)"
    ;;
esac
work="${temp_root}/vsift-engine"
source_dir="${work}/source"
stem="${ENGINE}-${ENGINE_VERSION}"
rm -rf "${work}" dist
mkdir -p "${work}" dist

git init --quiet "${source_dir}"
git -C "${source_dir}" fetch --quiet --depth 1 "${UPSTREAM_REPOSITORY}" "${UPSTREAM_COMMIT}"
git -C "${source_dir}" -c advice.detachedHead=false checkout --quiet FETCH_HEAD
fetched="$(git -C "${source_dir}" rev-parse HEAD)"
[ "${fetched}" = "${UPSTREAM_COMMIT}" ] || { echo "fetched ${fetched}, not ${UPSTREAM_COMMIT}" >&2; exit 1; }
if [ -n "${UPSTREAM_TAG}" ]; then
  peeled="$(git ls-remote "${UPSTREAM_REPOSITORY}" "refs/tags/${UPSTREAM_TAG}^{}" | cut -f1)"
  [ -n "${peeled}" ] || peeled="$(git ls-remote "${UPSTREAM_REPOSITORY}" "refs/tags/${UPSTREAM_TAG}" | cut -f1)"
  [ "${peeled}" = "${UPSTREAM_COMMIT}" ] || { echo "upstream ${UPSTREAM_TAG} names ${peeled:-nothing}, not ${UPSTREAM_COMMIT}" >&2; exit 1; }
fi
# The upstream commit's own time is every archive entry's time.
mtime="$(git -C "${source_dir}" log -1 --format=%ct)"

for round in 1 2; do
  rm -rf "${work}/build"
  bash "tools/engines/build-${engine}.sh" "${source_dir}" "${work}/build" "${work}/tree-${round}"
done

# The two builds must be the same bytes, file for file.
(cd "${work}/tree-1" && find . -type f | LC_ALL=C sort) > "${work}/files-1"
(cd "${work}/tree-2" && find . -type f | LC_ALL=C sort) > "${work}/files-2"
cmp "${work}/files-1" "${work}/files-2" || { echo "the two builds hold different files" >&2; exit 1; }
while read -r file; do
  cmp "${work}/tree-1/${file}" "${work}/tree-2/${file}" || { echo "the two builds of ${file} differ" >&2; exit 1; }
done < "${work}/files-1"

"${python}" tools/engines/package_engine.py --tree "${work}/tree-1" --root "${stem}" --mtime "${mtime}" \
  --archive "dist/${stem}.tar.gz" --inventory "dist/${stem}.inventory.json"
"${python}" tools/engines/package_engine.py --tree "${work}/tree-2" --root "${stem}" --mtime "${mtime}" \
  --archive "${work}/second.tar.gz" --inventory "${work}/second.inventory.json"
cmp "dist/${stem}.tar.gz" "${work}/second.tar.gz" || { echo "the two packages differ" >&2; exit 1; }

case "${LICENCE}" in
  LGPL*)
    # The corresponding source of an LGPL engine is published beside it: the
    # upstream commit's tree exactly (no VERSION file, no build output).
    git -C "${source_dir}" archive --format=tar --prefix="${stem}-source/" HEAD | gzip -n -9 > "dist/${stem}-source.tar.gz"
    ;;
esac

ENGINE_STEM="${stem}" ENGINE_MTIME="${mtime}" "${python}" - "${recipe}" > "dist/${stem}.build-info.json" <<'PYTHON'
import hashlib, json, os, platform, sys
recipe = open(sys.argv[1], "rb").read()
print(json.dumps({
    "engine": os.environ["ENGINE"],
    "version": os.environ["ENGINE_VERSION"],
    "target": os.environ["TARGET"],
    "licence": os.environ["LICENCE"],
    "upstream_repository": os.environ["UPSTREAM_REPOSITORY"],
    "upstream_tag": os.environ.get("UPSTREAM_TAG") or None,
    "upstream_commit": os.environ["UPSTREAM_COMMIT"],
    "source_date_epoch": int(os.environ["ENGINE_MTIME"]),
    "recipe_sha256": hashlib.sha256(recipe).hexdigest(),
    "vsift_commit": os.environ.get("GITHUB_SHA"),
    "runner_image": os.environ.get("ImageOS"),
    "runner_image_version": os.environ.get("ImageVersion"),
    "machine": platform.machine(),
    "built_twice_and_compared": True,
}, indent=2, sort_keys=True))
PYTHON

"${python}" - dist <<'PYTHON'
import hashlib, os, sys
folder = sys.argv[1]
lines = []
for name in sorted(os.listdir(folder)):
    with open(os.path.join(folder, name), "rb") as handle:
        lines.append(f"{hashlib.sha256(handle.read()).hexdigest()}  {name}\n")
with open(os.path.join(folder, "SHA256SUMS"), "w", encoding="ascii", newline="\n") as sums:
    sums.writelines(lines)
PYTHON
cat dist/SHA256SUMS
