#!/usr/bin/env bash
# Runs the shell of release.yml's publishing steps against stub `npm`, `gh`,
# `curl` and `sleep` commands, so the channel logic of the privileged job and
# the plan job's registry step are exercised end to end without touching any
# real service (P14 PR 8, docs/operations/release.md section 6.9).
#
#   bash tools/vsift-release/tests/publish-steps.sh .github/workflows/release.yml
#
# It extracts each step's script from the workflow itself, so what runs is the
# text that would run in the workflow. Needs GNU coreutils, `awk`, `openssl`
# and `base64 -w` (Linux, or Git Bash on Windows). The Rust test
# `publish_steps` runs it on Linux in CI; it exits non-zero on any failure.
set -uo pipefail

WORKFLOW="$1"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
BIN="${WORK}/bin"
mkdir -p "${BIN}"

# --- stubs -----------------------------------------------------------------
cat > "${BIN}/npm" <<'STUB'
#!/usr/bin/env bash
# State: ${STATE}/<package>.<tag> holds a version; ${STATE}/<package>@<version> exists once published.
state="${STATE}"
sanitize() { echo "${1//\//_}"; }
echo "npm $*" >> "${STATE}/calls.log"
case "$1" in
  --version) echo "11.19.0" ;;
  view)
    spec="$2"; field="${3:-}"
    case "${field}" in
      dist-tags.latest) f="${state}/$(sanitize "${spec}").latest"; [ -f "$f" ] && cat "$f" ;;
      dist-tags.next) f="${state}/$(sanitize "${spec}").next"; [ -f "$f" ] && cat "$f" ;;
      dist.integrity)
        f="${state}/$(sanitize "${spec}").integrity"
        if [ -f "$f" ]; then cat "$f"; else echo "npm error code E404" >&2; exit 1; fi ;;
    esac
    exit 0 ;;
  publish)
    file="$2"; shift 2
    tag=""
    while [ $# -gt 0 ]; do if [ "$1" = "--tag" ]; then tag="$2"; fi; shift; done
    base="$(basename "${file}" .tgz)"
    # base is like vsift-win32-x64-0.2.0 or vsift-cli-0.2.0-rc.1
    pkg=""; ver=""
    for p in vsift-darwin-arm64:@vsift/darwin-arm64 vsift-win32-x64:@vsift/win32-x64 vsift-linux-x64:@vsift/linux-x64 vsift-cli:vsift-cli; do
      stem="${p%%:*}"; name="${p#*:}"
      case "${base}" in "${stem}-"*) pkg="${name}"; ver="${base#"${stem}"-}" ;; esac
    done
    key="$(sanitize "${pkg}")"
    echo "stub publish ${pkg}@${ver} tag ${tag}" >> "${state}/published.log"
    echo "sha512-$(openssl dgst -sha512 -binary "${file}" | base64 -w 0)" > "${state}/${key}@${ver}.integrity"
    echo "${ver}" > "${state}/${key}.${tag}"
    exit 0 ;;
esac
STUB
chmod +x "${BIN}/npm"
cat > "${BIN}/sleep" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB
chmod +x "${BIN}/sleep"
cat > "${BIN}/gh" <<'STUB'
#!/usr/bin/env bash
echo "gh $*" >> "${STATE}/gh.log"
case "$1 $2" in
  "api repos/smormah/vsift/releases/latest") cat "${STATE}/github-latest" 2>/dev/null || true ;;
  "release create") ;;
  "release edit") if printf '%s\n' "$@" | grep -qx -- '--latest'; then echo "${3}" > "${STATE}/github-latest"; fi ;;
  "release view") ;;
esac
exit 0
STUB
chmod +x "${BIN}/gh"
cat > "${BIN}/curl" <<'STUB'
#!/usr/bin/env bash
# Records its arguments; writes a body to --output and prints the HTTP status
# to stdout (what --write-out does), as CURL_MODE says.
echo "curl $*" >> "${STATE}/curl.log"
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --output) out="$2"; shift ;;
    --write-out | --max-time | --retry | --proto) shift ;;
  esac
  shift
done
case "${CURL_MODE}" in
  ok) echo '{"dist-tags":{"latest":"0.0.0"},"versions":{}}' > "${out}"; printf '200' ;;
  notfound) echo '{"error":"Not found"}' > "${out}"; printf '404' ;;
  unreachable) printf '000'; exit 7 ;;
  silent-fail) exit 28 ;;
esac
STUB
chmod +x "${BIN}/curl"
export PATH="${BIN}:${PATH}"

# --- extract a step's script from the workflow -----------------------------
extract() {
  awk -v name="      - name: $1" '
    $0 == name { found = 1; next }
    found && $0 == "        run: |" { inrun = 1; next }
    found && inrun { if ($0 ~ /^          / || $0 == "") { sub(/^          /, ""); print } else { exit } }
  ' "${WORKFLOW}"
}
extract "Record the dist-tags before publishing" > "${WORK}/record.sh"
extract "Publish the platform packages, then vsift-cli, under next (pre-release)" > "${WORK}/publish-next.sh"
extract "Publish the platform packages, then vsift-cli, under latest (stable)" > "${WORK}/publish-latest.sh"
extract "Require next to be this version and latest to be untouched" > "${WORK}/verify-next.sh"
extract "Require latest to be this version and next to be untouched" > "${WORK}/verify-latest.sh"
extract "Create the GitHub release (stable) and mark it latest" > "${WORK}/release-stable.sh"
extract "Read npm's public metadata of the four packages" > "${WORK}/registry.sh"
for script in record publish-next publish-latest verify-next verify-latest release-stable registry; do
  [ -s "${WORK}/${script}.sh" ] || { echo "FAIL: could not extract ${script}"; exit 1; }
done

PASS=0; FAIL=0
check() { # name, expected exit (0 or nonzero), actual
  local name="$1" want="$2" got="$3"
  if { [ "${want}" = 0 ] && [ "${got}" = 0 ]; } || { [ "${want}" != 0 ] && [ "${got}" != 0 ]; }; then
    PASS=$((PASS + 1)); echo "ok   - ${name}"
  else
    FAIL=$((FAIL + 1)); echo "FAIL - ${name} (wanted ${want}, exit ${got})"
  fi
}

PACKAGES="@vsift/darwin-arm64 @vsift/win32-x64 @vsift/linux-x64 vsift-cli"
setup() { # version, latest-before, next-before
  export STATE="${WORK}/state"; rm -rf "${STATE}" "${WORK}/run"; mkdir -p "${STATE}" "${WORK}/run/npm-packages" "${WORK}/run/tmp"
  : > "${STATE}/calls.log"
  export RUNNER_TEMP="${WORK}/run/tmp"
  export VERSION="$1" TAG="v$1" GH_TOKEN=x NPM_BOOTSTRAP_TOKEN=""
  for p in ${PACKAGES}; do
    key="${p//\//_}"
    [ "$2" != none ] && echo "$2" > "${STATE}/${key}.latest"
    [ "$3" != none ] && echo "$3" > "${STATE}/${key}.next"
    f="${p#@}"; f="${f//\//-}-${VERSION}.tgz"
    echo "tarball ${p}" > "${WORK}/run/npm-packages/${f}"
  done
  cd "${WORK}/run"
}
run() { bash -e -o pipefail "${WORK}/$1.sh" > "${WORK}/out.log" 2>&1; echo $?; }
tag_of() { cat "${STATE}/${1//\//_}.$2" 2>/dev/null || echo none; }

echo "== pre-release 0.2.0-rc.1: next moves, latest does not"
setup 0.2.0-rc.1 0.0.0 0.1.0
check "record" 0 "$(run record)"
check "pre-release publishes" 0 "$(run publish-next)"
check "next is the new version" 0 "$([ "$(tag_of vsift-cli next)" = 0.2.0-rc.1 ]; echo $?)"
check "latest untouched" 0 "$([ "$(tag_of vsift-cli latest)" = 0.0.0 ]; echo $?)"
check "four packages published" 0 "$([ "$(wc -l < "${STATE}/published.log")" = 4 ]; echo $?)"
check "platform packages first, launcher last" 0 "$([ "$(tail -n 1 "${STATE}/published.log")" = "stub publish vsift-cli@0.2.0-rc.1 tag next" ]; echo $?)"
check "verification passes" 0 "$(run verify-next)"
check "stable verification of a pre-release fails" 1 "$(run verify-latest)"
echo "0.2.0-rc.1" > "${STATE}/vsift-cli.latest"
check "a pre-release that moved latest fails its verification" 1 "$(run verify-next)"

echo "== a stable version given to the pre-release step is refused before anything is published"
setup 0.2.0 0.0.0 0.2.0-rc.1
run record > /dev/null
check "stable version in the next step" 1 "$(run publish-next)"
check "nothing published" 0 "$([ ! -s "${STATE}/published.log" ]; echo $?)"

echo "== a pre-release given to the stable step is refused before anything is published"
setup 0.2.0-rc.1 0.0.0 0.1.0
run record > /dev/null
check "pre-release version in the latest step" 1 "$(run publish-latest)"
check "nothing published" 0 "$([ ! -s "${STATE}/published.log" ]; echo $?)"

echo "== stable 0.2.0: latest moves on all four, next is untouched"
setup 0.2.0 0.0.0 0.2.0-rc.1
check "record" 0 "$(run record)"
check "stable publishes" 0 "$(run publish-latest)"
check "latest is the new version" 0 "$([ "$(tag_of vsift-cli latest)" = 0.2.0 ] && [ "$(tag_of @vsift/win32-x64 latest)" = 0.2.0 ]; echo $?)"
check "next untouched" 0 "$([ "$(tag_of vsift-cli next)" = 0.2.0-rc.1 ]; echo $?)"
check "every publish used --tag latest" 0 "$([ "$(grep -c -- '--tag latest' "${STATE}/calls.log")" = 4 ] && ! grep -q -- '--tag next' "${STATE}/calls.log"; echo $?)"
check "stable verification passes" 0 "$(run verify-latest)"
check "pre-release verification of a stable fails" 1 "$(run verify-next)"
check "stable release is created and marked latest" 0 "$(run release-stable)"
check "gh edited with --latest" 0 "$(grep -q -- 'release edit v0.2.0 --repo smormah/vsift --draft=false --latest' "${STATE}/gh.log"; echo $?)"
check "gh create without --prerelease or --latest" 0 "$(grep 'release create' "${STATE}/gh.log" | grep -qv -e '--prerelease' -e '--latest'; echo $?)"

echo "== the stable release step fails if GitHub's latest is not the tag"
setup 0.2.0 0.0.0 0.2.0-rc.1
echo "v0.1.0" > "${STATE}/github-latest"
cat > "${BIN}/gh" <<'STUB'
#!/usr/bin/env bash
echo "gh $*" >> "${STATE}/gh.log"
case "$1 $2" in
  "api repos/smormah/vsift/releases/latest") cat "${STATE}/github-latest" 2>/dev/null || true ;;
esac
exit 0
STUB
check "latest release not moved" 1 "$(run release-stable)"

echo "== latest ahead of the version, or a pre-release as latest, refuses the stable"
setup 0.2.0 0.3.0 0.2.0-rc.1
run record > /dev/null
check "latest ahead" 1 "$(run publish-latest)"
check "nothing published" 0 "$([ ! -s "${STATE}/published.log" ]; echo $?)"
setup 0.2.0 0.2.0-rc.1 0.2.0-rc.1
run record > /dev/null
check "a pre-release as latest" 1 "$(run publish-latest)"
setup 0.2.0 none none
run record > /dev/null
check "no latest at all" 1 "$(run publish-latest)"
setup 0.10.0 0.9.0 none
run record > /dev/null
check "0.9.0 is below 0.10.0 (version order, not text order)" 0 "$(run publish-latest)"
setup 0.9.0 0.10.0 none
run record > /dev/null
check "0.10.0 is above 0.9.0" 1 "$(run publish-latest)"

echo "== a re-run after a partial stable publish completes it"
setup 0.2.0 0.0.0 0.2.0-rc.1
run record > /dev/null
# the first two platform packages were published already
for p in @vsift/darwin-arm64 @vsift/win32-x64; do
  key="${p//\//_}"; f="${p#@}"; f="${f//\//-}-${VERSION}.tgz"
  echo "sha512-$(openssl dgst -sha512 -binary "npm-packages/${f}" | base64 -w 0)" > "${STATE}/${key}@${VERSION}.integrity"
  echo "${VERSION}" > "${STATE}/${key}.latest"
done
: > "${RUNNER_TEMP}/tags-before"
run record > /dev/null
check "re-run publishes the rest" 0 "$(run publish-latest)"
check "only the two missing packages were published" 0 "$([ "$(wc -l < "${STATE}/published.log")" = 2 ]; echo $?)"
check "all four are latest" 0 "$(run verify-latest)"

echo "== a version already on npm with other bytes stops the publish"
setup 0.2.0 0.0.0 none
run record > /dev/null
echo "sha512-other" > "${STATE}/vsift-cli@0.2.0.integrity"
check "other bytes" 1 "$(run publish-latest)"

echo "== a stable that moved latest and next both fails verification"
setup 0.2.0 0.0.0 0.2.0-rc.1
run record > /dev/null
run publish-latest > /dev/null
echo "0.2.0" > "${STATE}/vsift-cli.next"
check "next moved" 1 "$(run verify-latest)"

echo "== the plan job's registry step records one status per package and never fails the job"
for mode_and_status in ok:200 notfound:404 unreachable:000 silent-fail:000; do
  mode="${mode_and_status%%:*}"; want="${mode_and_status#*:}"
  setup 0.2.0 0.0.0 none
  export CURL_MODE="${mode}"
  check "registry step (${mode})" 0 "$(run registry)"
  all_ok=0
  for stem in vsift-cli vsift-darwin-arm64 vsift-linux-x64 vsift-win32-x64; do
    [ "$(cat "${RUNNER_TEMP}/registry/${stem}.status" 2>/dev/null)" = "${want}" ] || all_ok=1
  done
  check "four statuses are ${want} (${mode})" 0 "${all_ok}"
done
setup 0.2.0 0.0.0 none
export CURL_MODE=ok
run registry > /dev/null
check "the scoped package is requested with an encoded slash" 0 "$(grep -q 'https://registry.npmjs.org/@vsift%2Fwin32-x64' "${STATE}/curl.log"; echo $?)"
check "the launcher is requested by name" 0 "$(grep -q 'https://registry.npmjs.org/vsift-cli' "${STATE}/curl.log"; echo $?)"
check "no request carries a credential, header or body" 0 "$(! grep -q -e '-H' -e '--header' -e '--data' -e '-X' -e '--user' "${STATE}/curl.log"; echo $?)"

echo
echo "passed ${PASS}, failed ${FAIL}"
[ "${FAIL}" = 0 ]
