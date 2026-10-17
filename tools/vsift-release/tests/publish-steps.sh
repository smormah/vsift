#!/usr/bin/env bash
# Runs the shell of release.yml's publishing steps against stub `npm`, `gh`,
# `curl` and `sleep` commands, so the channel logic of the privileged job, the
# plan job's registry step and the guard job are exercised end to end without
# touching any real service (P14 PR 8, docs/operations/release.md section 6.9;
# the guard of #337 and governance rule 15, and the read-back window of #403,
# section 6.18).
#
#   bash tools/vsift-release/tests/publish-steps.sh .github/workflows/release.yml
#
# It extracts each step's script from the workflow itself, so what runs is the
# text that would run in the workflow. Needs GNU coreutils, `awk`, `openssl`
# and `base64 -w` (Linux, or Git Bash on Windows). With `jq` on the PATH (as on
# GitHub's Ubuntu runners) the stub `gh` applies the guard's own `--jq`
# filters to GitHub's JSON, as the real `gh` does; without it the stub uses the
# fixtures' expected filter output and says so. The Rust test `publish_steps`
# runs it on Linux in CI; it exits non-zero on any failure.
set -uo pipefail

WORKFLOW="$1"
# The workflow by absolute path, for the checks that run from another folder.
WORKFLOW_FILE="$(realpath "$1")"
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
# A tag read can lag: ${STATE}/<package>.<tag>.lag holds how many more reads
# return ${STATE}/<package>.<tag>.stale (an empty file is no answer) instead.
# ${STATE}/<package>.<tag>.hang makes the next read hang (once), as a stuck
# connection would; the step's `timeout` must cut it off.
case "$1" in
  --version) echo "11.19.0" ;;
  view)
    spec="$2"; field="${3:-}"
    case "${field}" in
      dist-tags.latest | dist-tags.next)
        f="${state}/$(sanitize "${spec}").${field#dist-tags.}"
        if [ -f "${f}.hang" ]; then
          rm -f "${f}.hang"
          echo "hung ${spec} ${field}" >> "${state}/hung.log"
          exec "$(command -pv sleep)" 600
        fi
        if [ -f "${f}.lag" ] && [ "$(cat "${f}.lag")" -gt 0 ]; then
          echo "$(( $(cat "${f}.lag") - 1 ))" > "${f}.lag"
          cat "${f}.stale" 2>/dev/null
        elif [ -f "$f" ]; then
          cat "$f"
        fi ;;
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
echo "sleep $*" >> "${STATE}/sleep.log"
exit 0
STUB
chmod +x "${BIN}/sleep"
write_gh_stub() {
cat > "${BIN}/gh" <<'STUB'
#!/usr/bin/env bash
# The releases the publish job reads and writes, and the GitHub API reads of the
# guard and the re-check. Each read answers only its exact path and query, from
# a fixture holding GitHub's JSON (${STATE}/<name>.json) and a `.filtered` twin
# holding what the script's --jq filter makes of it; a missing fixture answers
# as GitHub does on an error (its error body on stdout, a failure). The commit
# asked about must be ${STUB_COMMIT} and the branch head ${STUB_HEAD}, so a
# read of another commit, a branch's name instead of its head, or another
# query fails. GH_FAIL=runs or jobs makes that read fail as an unreachable API.
echo "gh $*" >> "${STATE}/gh.log"
answer() { # <fixture name> <filter>
  local file="${STATE}/$1.json"
  if [ ! -f "${file}" ]; then
    echo '{"message":"Not Found","status":"404"}'
    echo "gh: Not Found (HTTP 404)" >&2
    exit 1
  fi
  if command -v jq > /dev/null 2>&1; then
    jq -r "$2" "${file}" > "${STATE}/jq.out" || exit 1
    echo "${file}" >> "${FILTER_RAN}"
    cmp -s "${STATE}/jq.out" "${file}.filtered" || echo "${file}" >> "${FILTER_MISMATCH}"
    cat "${STATE}/jq.out"
  else
    cat "${file}.filtered"
  fi
}
unexpected() { echo "gh: unexpected read $1" >&2; echo "$1" >> "${STATE}/unexpected.log"; exit 1; }
if [ "$1" = api ]; then
  path="$2"; shift 2; filter=""
  while [ $# -gt 0 ]; do if [ "$1" = --jq ]; then filter="$2"; shift; fi; shift; done
  base="repos/smormah/vsift"
  case "${path}" in
    "${base}/releases/latest") cat "${STATE}/github-latest" 2>/dev/null || true ;;
    "${base}/actions/workflows/nightly.yml/runs?"*)
      [ "${path}" = "${base}/actions/workflows/nightly.yml/runs?head_sha=${STUB_COMMIT}&per_page=100" ] || unexpected "${path}"
      [ "${GH_FAIL:-}" != runs ] || { echo '{"message":"Service Unavailable"}'; echo "gh: Service Unavailable (HTTP 503)" >&2; exit 1; }
      answer runs "${filter}" ;;
    "${base}/actions/runs/"*)
      id="${path#"${base}/actions/runs/"}"; id="${id%%/*}"
      [ "${path}" = "${base}/actions/runs/${id}/jobs?per_page=100" ] || unexpected "${path}"
      [ "${GH_FAIL:-}" != jobs ] || { echo '{"message":"Service Unavailable"}'; echo "gh: Service Unavailable (HTTP 503)" >&2; exit 1; }
      answer "jobs-${id}" "${filter}" ;;
    "${base}/git/ref/heads/"*)
      [ "${path}" = "${base}/git/ref/heads/${STUB_BRANCH}" ] || unexpected "${path}"
      answer head "${filter}" ;;
    "${base}/compare/"*)
      [ "${path}" = "${base}/compare/${STUB_COMMIT}...${STUB_HEAD}?per_page=1" ] || unexpected "${path}"
      answer compare "${filter}" ;;
    "${base}/git/ref/tags/"*)
      [ "${path}" = "${base}/git/ref/tags/${STUB_TAG}" ] || unexpected "${path}"
      answer tag "${filter}" ;;
    "${base}/git/tags/"*)
      [ "${path}" = "${base}/git/tags/${STUB_TAG_OBJECT}" ] || unexpected "${path}"
      answer tag-object "${filter}" ;;
    "${base}/rules/branches/"*)
      [ "${path}" = "${base}/rules/branches/${STUB_BRANCH}?per_page=100" ] || unexpected "${path}"
      answer rules "${filter}" ;;
    *) unexpected "${path}" ;;
  esac
  exit 0
fi
case "$1 $2" in
  "release create") ;;
  "release edit") if printf '%s\n' "$@" | grep -qx -- '--latest'; then echo "${3}" > "${STATE}/github-latest"; fi ;;
  "release view") ;;
esac
exit 0
STUB
chmod +x "${BIN}/gh"
}
write_gh_stub
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
cat > "${BIN}/cargo" <<'STUB'
#!/usr/bin/env bash
# Stands in for `cargo run -p vsift-release -- candidate-delta ...` and
# `cargo run -p vsift-governance -- release-evidence ...`, as CANDIDATE_MODE
# and EVIDENCE_MODE say.
echo "cargo $*" >> "${STATE}/cargo.log"
case "$*" in
  *vsift-release*candidate-delta*)
    if [ "${CANDIDATE_MODE}" = found ]; then
      printf 'candidate-version=0.2.0-rc.1\ncandidate-commit=%s\n' "${CANDIDATE_SHA}"
    fi ;;
  *vsift-governance*release-evidence*)
    case "${EVIDENCE_MODE}" in
      complete) echo "VSift release evidence is complete for 0.2.0-rc.1 at ${CANDIDATE_SHA:0:12}." ;;
      incomplete) echo "docs/planning/p14-evidence-ledger.json: incomplete for 0.2.0-rc.1: RQ-01: is planned" >&2; exit 1 ;;
    esac ;;
esac
STUB
chmod +x "${BIN}/cargo"
export PATH="${BIN}:${PATH}"
export FILTER_RAN="${WORK}/filters-ran" FILTER_MISMATCH="${WORK}/filter-mismatch"

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
extract "Find the accepted release candidate" > "${WORK}/candidate.sh"
extract "Check the evidence ledger for the accepted candidate" > "${WORK}/evidence.sh"
extract "Refuse a wrong ref, or a candidate without a passed nightly, before anything is built" > "${WORK}/guard.sh"
extract "Re-check the guard's conditions after the approval, before anything is published" > "${WORK}/recheck.sh"
for script in record publish-next publish-latest verify-next verify-latest release-stable registry candidate evidence guard recheck; do
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
  TAGS_BEFORE=""
  for p in ${PACKAGES}; do
    key="${p//\//_}"
    [ "$2" != none ] && echo "$2" > "${STATE}/${key}.latest"
    [ "$3" != none ] && echo "$3" > "${STATE}/${key}.next"
    f="${p#@}"; f="${f//\//-}-${VERSION}.tgz"
    echo "tarball ${p}" > "${WORK}/run/npm-packages/${f}"
    TAGS_BEFORE="${TAGS_BEFORE:+${TAGS_BEFORE}$'\n'}${p} $2 $3"
  done
  # The plan job's output, as the publish job receives it (#403's recovery).
  export TAGS_BEFORE
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

echo "== the plan job names the accepted candidate and records the evidence check's answer"
CANDIDATE_SHA=1111111111111111111111111111111111111111; export CANDIDATE_SHA
setup 0.2.0 0.0.0 none
export SOURCE_COMMIT=2222222222222222222222222222222222222222 GITHUB_OUTPUT="${RUNNER_TEMP}/github-output"
: > "${GITHUB_OUTPUT}"
export CANDIDATE_MODE=found
check "candidate step (found)" 0 "$(run candidate)"
check "the candidate's version and commit reach GITHUB_OUTPUT" 0 "$([ "$(cat "${GITHUB_OUTPUT}")" = "$(printf 'candidate-version=0.2.0-rc.1\ncandidate-commit=%s' "${CANDIDATE_SHA}")" ]; echo $?)"
check "the candidate comparison is run for the commit under test" 0 "$(grep -q -- "candidate-delta --github-output --stable-commit ${SOURCE_COMMIT}" "${STATE}/cargo.log"; echo $?)"
: > "${GITHUB_OUTPUT}"
export CANDIDATE_MODE=none
check "candidate step (none)" 0 "$(run candidate)"
check "nothing reaches GITHUB_OUTPUT without a candidate" 0 "$([ ! -s "${GITHUB_OUTPUT}" ]; echo $?)"

export CANDIDATE_VERSION=0.2.0-rc.1 CANDIDATE_COMMIT="${CANDIDATE_SHA}" EVIDENCE_MODE=complete
setup 0.2.0 0.0.0 none
export GITHUB_OUTPUT="${RUNNER_TEMP}/github-output"
check "evidence step (complete)" 0 "$(run evidence)"
check "the check is run for the candidate at its commit" 0 "$(grep -q -- "release-evidence --complete-for ${CANDIDATE_VERSION} --commit ${CANDIDATE_SHA}" "${STATE}/cargo.log"; echo $?)"
check "status 0 and the complete message are recorded" 0 "$([ "$(cat "${RUNNER_TEMP}/evidence/status")" = 0 ] && grep -q 'is complete for 0.2.0-rc.1 at 111111111111' "${RUNNER_TEMP}/evidence/result.txt"; echo $?)"
export EVIDENCE_MODE=incomplete
setup 0.2.0 0.0.0 none
check "evidence step (incomplete) records the failure and does not fail the job" 0 "$(run evidence)"
check "status 1 and the finding are recorded" 0 "$([ "$(cat "${RUNNER_TEMP}/evidence/status")" = 1 ] && grep -q 'RQ-01: is planned' "${RUNNER_TEMP}/evidence/result.txt"; echo $?)"

echo "== the read-back waits for a slow registry, bounded, and fails at once on a wrong answer (#403)"
lag() { # <package> <tag> <reads> <stale value, or empty for no answer>
  local f="${STATE}/${1//\//_}.$2"
  echo "$3" > "${f}.lag"
  printf '%s' "$4" > "${f}.stale"
}
sleeps() { wc -l < "${STATE}/sleep.log" 2>/dev/null | tr -d ' ' || echo 0; }
setup 0.2.1 0.2.0 0.2.1-rc.3
run record > /dev/null
run publish-latest > /dev/null
: > "${STATE}/sleep.log"
lag @vsift/darwin-arm64 latest 25 0.2.0
check "one package's latest still reads its old value for 25 reads (12 minutes): the publish of 0.2.1 passes" 0 "$(run verify-latest)"
check "it waited (more than the ten reads of the old window)" 0 "$([ "$(sleeps)" -gt 10 ]; echo $?)"
setup 0.2.1 0.2.0 0.2.1-rc.3
run record > /dev/null
run publish-latest > /dev/null
lag vsift-cli latest 3 ""
check "no answer for three reads, then the version: passes" 0 "$(run verify-latest)"
setup 0.2.1 0.2.0 0.2.1-rc.3
run record > /dev/null
run publish-latest > /dev/null
: > "${STATE}/sleep.log"
lag @vsift/linux-x64 latest 100 0.2.0
check "a registry that never settles fails when the window ends" 1 "$(run verify-latest)"
check "after exactly 40 reads, 30 seconds apart (20 minutes)" 0 "$([ "$(sleeps)" = 40 ] && [ "$(sort -u "${STATE}/sleep.log")" = "sleep 30" ]; echo $?)"
check "and says the packages were published and how to finish" 0 "$(grep -q 'npm accepted every publication above' "${WORK}/out.log" && grep -q 'Re-run failed jobs' "${WORK}/out.log"; echo $?)"
setup 0.2.1 0.2.0 0.2.1-rc.3
run record > /dev/null
run publish-latest > /dev/null
: > "${STATE}/sleep.log"
lag @vsift/win32-x64 latest 100 0.1.9
check "latest reading neither the version nor its old value fails" 1 "$(run verify-latest)"
check "at once, without waiting" 0 "$([ "$(sleeps)" = 0 ]; echo $?)"
setup 0.2.1 0.2.0 0.2.1-rc.3
run record > /dev/null
run publish-latest > /dev/null
: > "${STATE}/sleep.log"
lag @vsift/darwin-arm64 latest 100 0.2.0
echo "0.2.1" > "${STATE}/vsift-cli.next"
check "next moving under a stable publish fails, even while latest is still on its way" 1 "$(run verify-latest)"
check "at once, without waiting" 0 "$([ "$(sleeps)" = 0 ]; echo $?)"
setup 0.2.2-rc.1 0.2.1 0.2.1-rc.3
run record > /dev/null
run publish-next > /dev/null
: > "${STATE}/sleep.log"
lag vsift-cli next 25 0.2.1-rc.3
check "a candidate's next still on its old value for 25 reads: passes" 0 "$(run verify-next)"
setup 0.2.2-rc.1 0.2.1 0.2.1-rc.3
run record > /dev/null
run publish-next > /dev/null
: > "${STATE}/sleep.log"
lag vsift-cli next 100 0.2.0-rc.1
check "next reading neither the version nor its old value fails" 1 "$(run verify-next)"
check "at once, without waiting" 0 "$([ "$(sleeps)" = 0 ]; echo $?)"
setup 0.2.2-rc.1 0.2.1 0.2.1-rc.3
run record > /dev/null
run publish-next > /dev/null
: > "${STATE}/sleep.log"
lag @vsift/linux-x64 next 100 0.2.1-rc.3
echo "0.2.2-rc.1" > "${STATE}/@vsift_linux-x64.latest"
check "latest moving under a pre-release fails, even while next is still on its way" 1 "$(run verify-next)"
check "at once, without waiting" 0 "$([ "$(sleeps)" = 0 ]; echo $?)"
check "the read-back asks npm past its own cache" 0 "$(grep -q 'dist-tags.next --prefer-online' "${STATE}/calls.log" && grep -q 'dist-tags.latest --prefer-online' "${STATE}/calls.log"; echo $?)"
setup 0.2.2-rc.1 0.2.1 0.2.1-rc.3
run record > /dev/null
run publish-next > /dev/null
lag @vsift/win32-x64 next 3 ""
check "a candidate's next with no answer for three reads, then the version: passes" 0 "$(run verify-next)"
setup 0.2.2-rc.1 0.2.1 0.2.1-rc.3
run record > /dev/null
run publish-next > /dev/null
: > "${STATE}/sleep.log"
lag vsift-cli next 100 0.2.1-rc.3
check "a candidate's next that never moves fails when the window ends" 1 "$(run verify-next)"
check "after exactly 40 reads, 30 seconds apart" 0 "$([ "$(sleeps)" = 40 ]; echo $?)"
setup 0.2.2-rc.1 0.2.1 0.2.1-rc.3
run record > /dev/null
run publish-next > /dev/null
lag vsift-cli next 100 ""
check "a candidate's next that never answers fails when the window ends" 1 "$(run verify-next)"
setup 0.2.2-rc.1 0.2.1 0.2.1-rc.3
run record > /dev/null
run publish-next > /dev/null
: > "${STATE}/@vsift_darwin-arm64.next.hang"
started_at="$(date +%s)"
check "a read that hangs is cut off and counted as no answer" 0 "$(run verify-next)"
check "after its 20 seconds, not after the hang" 0 "$([ -s "${STATE}/hung.log" ] && [ $(( $(date +%s) - started_at )) -lt 120 ]; echo $?)"
check "every read is cut off with timeout and retried at most once" 0 "$(grep -c -- 'dist-tags.next --prefer-online --fetch-retries=1' "${WORKFLOW_FILE}" | grep -qx 2 && grep -c 'timeout 20 npm view' "${WORKFLOW_FILE}" | grep -qx 4; echo $?)"

echo "== the dist-tags before the publish come from the plan job, not from a read in the publish job (#403's recovery)"
setup 0.2.1 0.2.0 0.2.1-rc.3
check "record" 0 "$(run record)"
check "it reads nothing from npm" 0 "$(! grep -q 'dist-tags' "${STATE}/calls.log"; echo $?)"
check "and writes the plan's four lines in publication order" 0 "$([ "$(cat "${RUNNER_TEMP}/tags-before")" = "${TAGS_BEFORE}" ]; echo $?)"
run publish-latest > /dev/null
# A re-run of the failed publish job, after the four packages were published:
# the plan's output is kept, and a stale copy of the old latest is waited for.
: > "${RUNNER_TEMP}/tags-before"
run record > /dev/null
lag @vsift/darwin-arm64 latest 6 0.2.0
check "a re-run that reads a stale old latest waits instead of failing (the case of #403)" 0 "$(run verify-latest)"
setup 0.2.1 0.2.0 0.2.1-rc.3
TAGS_BEFORE="${TAGS_BEFORE/vsift-cli 0.2.0/vsift-cli unknown}"
check "a package the plan could not read stops the publish" 1 "$(run record)"
setup 0.2.1 0.2.0 0.2.1-rc.3
TAGS_BEFORE="$(printf '%s\n' "${TAGS_BEFORE}" | sed -n '1,3p')"
check "a missing package stops the publish" 1 "$(run record)"
setup 0.2.1 0.2.0 0.2.1-rc.3
TAGS_BEFORE="$(printf '%s\n' "${TAGS_BEFORE}" | sort -r)"
check "packages out of publication order stop the publish" 1 "$(run record)"
setup 0.2.1 0.2.0 0.2.1-rc.3
TAGS_BEFORE=""
check "no plan output stops the publish" 1 "$(run record)"

echo "== the guard: which refs may run each mode, and the nightly precondition (#337, governance rule 15)"
write_gh_stub
COMMIT=3333333333333333333333333333333333333333
HEAD_COMMIT=5555555555555555555555555555555555555555
TAG_OBJECT=6666666666666666666666666666666666666666
fixture() { # <name> <json> <what the script's filter makes of it>
  printf '%s\n' "$2" > "${STATE}/$1.json"
  printf '%s' "$3" > "${STATE}/$1.json.filtered"
}
on_branch() { # <comparison status>: the branch head exists and compares so
  fixture head "{\"ref\":\"refs/heads/${STUB_BRANCH}\",\"object\":{\"type\":\"commit\",\"sha\":\"${HEAD_COMMIT}\"}}" "${HEAD_COMMIT}"$'\n'
  fixture compare "{\"status\":\"$1\",\"ahead_by\":0}" "$1"$'\n'
}
no_branch() { rm -f "${STATE}/head.json" "${STATE}/compare.json"; }
tag_at() { # <commit>: a lightweight tag naming it
  rm -f "${STATE}/tag-object.json"
  fixture tag "{\"ref\":\"refs/tags/${STUB_TAG}\",\"object\":{\"type\":\"commit\",\"sha\":\"$1\"}}" "commit $1"$'\n'
}
annotated_tag_at() { # <commit>: an annotated tag object naming it
  fixture tag "{\"ref\":\"refs/tags/${STUB_TAG}\",\"object\":{\"type\":\"tag\",\"sha\":\"${TAG_OBJECT}\"}}" "tag ${TAG_OBJECT}"$'\n'
  fixture tag-object "{\"sha\":\"${TAG_OBJECT}\",\"object\":{\"type\":\"commit\",\"sha\":\"$1\"}}" "commit $1"$'\n'
}
rules() { # <rule type>...: the rules GitHub says apply to the release branch
  local json="" text="" rule
  for rule in "$@"; do
    json="${json:+${json},}{\"type\":\"${rule}\",\"ruleset_id\":1}"
    text="${text}${rule}"$'\n'
  done
  fixture rules "[${json}]" "${text}"
}
ALL_RULES=(creation deletion non_fast_forward pull_request required_status_checks)
guard_setup() { # <version> <event> <ref> <dry_run input> [repository]
  export STATE="${WORK}/state"; rm -rf "${STATE}" "${WORK}/run"; mkdir -p "${STATE}" "${WORK}/run"
  : > "${STATE}/gh.log"
  printf '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "%s"\nedition = "2024"\n\n[workspace.dependencies]\nversion = "9.9.9"\n' "$1" > "${WORK}/run/Cargo.toml"
  export EVENT_NAME="$2" GIT_REF="$3" DRY_RUN_INPUT="$4" REPOSITORY="${5:-smormah/vsift}" SOURCE_COMMIT="${COMMIT}" GH_TOKEN=x
  export GITHUB_STEP_SUMMARY="${STATE}/summary.md"
  local core="${1%%-*}"
  export STUB_COMMIT="${COMMIT}" STUB_HEAD="${HEAD_COMMIT}" STUB_BRANCH="release/${core%.*}.x" STUB_TAG="v$1" STUB_TAG_OBJECT="${TAG_OBJECT}"
  unset GH_FAIL
  RUNS_JSON=""; RUNS_TEXT=""
  fixture runs '{"total_count":0,"workflow_runs":[]}' ""
  tag_at "${COMMIT}"
  rules "${ALL_RULES[@]}"
  cd "${WORK}/run"
}
nightly_run() { # <id> <started> <event> <status> <conclusion> <job conclusion:job name>...
  local id="$1" started="$2" event="$3" status="$4" conclusion="$5" jobs="" text="" job
  shift 5
  RUNS_JSON="${RUNS_JSON:+${RUNS_JSON},}{\"id\":${id},\"event\":\"${event}\",\"status\":\"${status}\",\"conclusion\":${conclusion},\"run_started_at\":\"${started}\",\"head_sha\":\"${COMMIT}\"}"
  case "${event}" in
    schedule | workflow_dispatch) RUNS_TEXT="${RUNS_TEXT}${started} ${id} ${status} ${conclusion//\"/}"$'\n' ;;
  esac
  fixture runs "{\"total_count\":0,\"workflow_runs\":[${RUNS_JSON}]}" "${RUNS_TEXT}"
  for job in "$@"; do
    jobs="${jobs:+${jobs},}{\"name\":\"${job#*:}\",\"status\":\"completed\",\"conclusion\":\"${job%%:*}\"}"
    text="${text}${job%%:*}"$'\t'"${job#*:}"$'\n'
  done
  fixture "jobs-${id}" "{\"total_count\":0,\"jobs\":[${jobs}]}" "${text}"
}
passed_run() { nightly_run "$1" "$2" "$3" completed '"success"' "${PASSED_JOBS[@]}"; }
PASSED_JOBS=("success:Anything new on main" "success:Quality (ubuntu-latest)" "success:Quality (windows-latest)" "success:Quality (macos-latest)" "success:Governance" "skipped:Report a failure")
GATED_OUT=("success:Anything new on main" "skipped:Quality (ubuntu-latest)" "skipped:Quality (windows-latest)" "skipped:Quality (macos-latest)" "skipped:Governance" "skipped:Report a failure")
FAILED_JOBS=("success:Anything new on main" "failure:Quality (ubuntu-latest)" "success:Quality (windows-latest)" "success:Quality (macos-latest)" "success:Governance" "success:Report a failure")
api_calls() { grep -c '^gh api' "${STATE}/gh.log" || true; }
refused_with() { grep -q -- "$1" "${WORK}/out.log"; echo $?; }
no_unexpected_read() { [ ! -s "${STATE}/unexpected.log" ]; echo $?; }

guard_setup 0.2.2-rc.1 pull_request refs/pull/7/merge ""
check "a pull request passes the guard" 0 "$(run guard)"
check "without reading anything" 0 "$([ "$(api_calls)" = 0 ]; echo $?)"
guard_setup 0.2.2-rc.1 push refs/heads/release/0.2.x ""
check "a push to a release branch passes the guard" 0 "$(run guard)"
guard_setup 0.2.2-rc.1 workflow_dispatch refs/heads/main true
check "a dry run on main passes" 0 "$(run guard)"
check "without reading anything" 0 "$([ "$(api_calls)" = 0 ]; echo $?)"
guard_setup 0.2.2-rc.1 workflow_dispatch refs/heads/feat/release-branches-and-window true
check "a dry run on another branch is refused" 1 "$(run guard)"
check "naming the refs a dry run may use" 0 "$(refused_with 'only on main, release/0.2.x or the tag v0.2.2-rc.1')"
check "and the summary says it was refused before anything was built" 0 "$(grep -q 'refused before anything is built' "${GITHUB_STEP_SUMMARY}"; echo $?)"
guard_setup 0.2.2-rc.1 workflow_dispatch refs/heads/main false
check "a publish dispatched on main is refused (#337)" 1 "$(run guard)"
check "with the remedy: dispatch it on the tag" 0 "$(refused_with 'Dispatch it on the tag, or leave dry_run set')"
check "without reading anything" 0 "$([ "$(api_calls)" = 0 ]; echo $?)"
guard_setup 0.2.2-rc.1 workflow_dispatch refs/heads/release/0.2.x false
check "a publish dispatched on the release branch is refused" 1 "$(run guard)"
guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.1 false
check "a publish dispatched on another version's tag is refused" 1 "$(run guard)"
guard_setup 0.2.2-rc.1 workflow_dispatch refs/heads/release/0.3.x true
check "a dry run on another line's release branch is refused" 1 "$(run guard)"
guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false someone/vsift
check "a publish in a fork is refused" 1 "$(run guard)"
guard_setup 0.2.2-rc.1 workflow_dispatch refs/heads/main maybe
check "a dry_run input that is neither true nor false is refused" 1 "$(run guard)"

guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false
on_branch ahead
passed_run 101 2026-10-17T02:41:00Z schedule
check "a candidate on the release branch whose nightly passed may publish" 0 "$(run guard)"
check "every read was of exactly this commit, the branch head's own commit and this tag" 0 "$(no_unexpected_read)"
check "the Nightly runs read are those of exactly this commit, in any status" 0 "$(grep -q "nightly.yml/runs?head_sha=${COMMIT}&per_page=100" "${STATE}/gh.log"; echo $?)"
check "the commit is compared with the branch head's commit, not with its name (a tag of that name cannot win)" 0 "$(grep -q "compare/${COMMIT}...${HEAD_COMMIT}" "${STATE}/gh.log" && ! grep -q 'compare/[^ ]*release/' "${STATE}/gh.log"; echo $?)"
check "the summary names the run that passed" 0 "$(grep -q 'Nightly run 101' "${GITHUB_STEP_SUMMARY}"; echo $?)"
on_branch identical
check "the tag at the head of the release branch may publish" 0 "$(run guard)"
DRY_RUN_INPUT=true
check "the dry run on the tag (the rehearsal) passes too" 0 "$(run guard)"

guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false
on_branch ahead
passed_run 101 2026-10-17T02:41:00Z schedule
annotated_tag_at "${COMMIT}"
check "an annotated tag is peeled to its commit" 0 "$(run guard)"
annotated_tag_at 4444444444444444444444444444444444444444
check "an annotated tag that names another commit is refused" 1 "$(run guard)"
tag_at 4444444444444444444444444444444444444444
check "a tag moved after the dispatch (a re-run of an old run) is refused" 1 "$(run guard)"
check "naming both commits" 0 "$(refused_with "names 4444444444444444444444444444444444444444 now, not ${COMMIT}")"
rm -f "${STATE}/tag.json"
check "a tag GitHub cannot read is refused" 1 "$(run guard)"
fixture tag '{"object":{"type":"commit","sha":"not-a-commit"}}' "commit not-a-commit"$'\n'
check "a tag whose answer is not a commit is refused" 1 "$(run guard)"

guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false
passed_run 101 2026-10-17T02:41:00Z schedule
on_branch diverged
check "a candidate tag that is not on the release branch is refused" 1 "$(run guard)"
on_branch behind
check "a candidate tag ahead of the release branch is refused" 1 "$(run guard)"
no_branch
check "a candidate tag when the release branch does not exist is refused" 1 "$(run guard)"
check "naming the branch" 0 "$(refused_with 'release/0.2.x does not exist')"
on_branch ahead
rm -f "${STATE}/compare.json"
check "a comparison GitHub cannot answer refuses the candidate" 1 "$(run guard)"

guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false
on_branch ahead
passed_run 101 2026-10-17T02:41:00Z schedule
rules creation deletion non_fast_forward required_status_checks
check "a release branch whose rulesets lack a pull-request rule refuses the candidate" 1 "$(run guard)"
check "naming the missing rule" 0 "$(refused_with 'have no pull_request rule')"
rules
check "a release branch with no ruleset at all refuses the candidate" 1 "$(run guard)"
rm -f "${STATE}/rules.json"
check "rules GitHub cannot read refuse the candidate" 1 "$(run guard)"

guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false
on_branch ahead
check "a candidate no nightly has tested is refused" 1 "$(run guard)"
check "saying so and how to run one" 0 "$(grep -q 'no Nightly run has tested' "${WORK}/out.log" && grep -q 'gh workflow run nightly.yml' "${WORK}/out.log"; echo $?)"
DRY_RUN_INPUT=true
check "and so is its rehearsal on the tag" 1 "$(run guard)"

guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false
on_branch ahead
nightly_run 101 2026-10-17T02:41:00Z schedule completed '"failure"' "${FAILED_JOBS[@]}"
check "a candidate whose nightly failed is refused" 1 "$(run guard)"
check "naming the job that failed" 0 "$(refused_with 'Quality (ubuntu-latest) (failure)')"
nightly_run 102 2026-10-18T02:41:00Z schedule completed '"success"' "${GATED_OUT[@]}"
check "a later scheduled run that ran nothing (green on GitHub) does not make it pass" 1 "$(run guard)"
passed_run 103 2026-10-18T09:00:00Z workflow_dispatch
check "a later dispatched run that passed does" 0 "$(run guard)"
nightly_run 104 2026-10-18T11:00:00Z workflow_dispatch completed '"failure"' "${FAILED_JOBS[@]}"
check "and a still later one that failed refuses it again (the latest decides)" 1 "$(run guard)"

guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false
on_branch ahead
passed_run 101 2026-10-17T02:41:00Z schedule
nightly_run 102 2026-10-17T09:00:00Z workflow_dispatch in_progress null
check "a passed run with a re-run still in progress refuses the candidate" 1 "$(run guard)"
check "saying the run is still in progress" 0 "$(refused_with 'Nightly run 102 of')"
check "and to wait for it" 0 "$(refused_with 'is still in_progress')"

guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false
on_branch ahead
nightly_run 101 2026-10-17T02:41:00Z schedule completed '"success"' "success:Anything new on main" "success:Quality (ubuntu-latest)" "skipped:Quality (windows-latest)" "success:Governance" "skipped:Report a failure"
check "a green run with a skipped check is not a pass" 1 "$(run guard)"
check "naming the skipped job" 0 "$(refused_with 'Quality (windows-latest) (skipped)')"

guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false
on_branch ahead
passed_run 101 2026-10-17T02:41:00Z push
check "a branch experiment's push-triggered run is not the nightly" 1 "$(run guard)"

guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false
on_branch ahead
passed_run 101 2026-10-17T02:41:00Z schedule
export GH_FAIL=runs
check "Nightly runs that cannot be read refuse the candidate" 1 "$(run guard)"
export GH_FAIL=jobs
check "a run's jobs that cannot be read refuse the candidate" 1 "$(run guard)"
unset GH_FAIL

guard_setup 0.2.2 workflow_dispatch refs/tags/v0.2.2 false
on_branch ahead
check "a stable version on the protected release branch is not asked for a nightly of its own" 0 "$(run guard)"
check "and reads no Nightly run" 0 "$(! grep -q 'nightly.yml' "${STATE}/gh.log"; echo $?)"
no_branch
check "but must be on the release branch" 1 "$(run guard)"
on_branch ahead
rules creation deletion
check "and the branch must be protected" 1 "$(run guard)"
rules "${ALL_RULES[@]}"
tag_at 4444444444444444444444444444444444444444
check "and its tag must still name the run's commit" 1 "$(run guard)"

guard_setup 0.2.2-rc.2 workflow_dispatch refs/heads/release/0.2.x true
check "a dry run on the release branch without a nightly goes on" 0 "$(run guard)"
check "and its summary says a candidate from this commit would be refused" 0 "$(grep -q 'would be refused' "${GITHUB_STEP_SUMMARY}"; echo $?)"
passed_run 105 2026-10-19T10:00:00Z workflow_dispatch
: > "${GITHUB_STEP_SUMMARY}"
check "a dry run on the release branch after a dispatched nightly passed" 0 "$(run guard)"
check "says the precondition would be met" 0 "$(grep -q 'would meet the nightly precondition' "${GITHUB_STEP_SUMMARY}"; echo $?)"

guard_setup 0.2.2-rc.1 workflow_dispatch refs/tags/v0.2.2-rc.1 false
on_branch ahead
passed_run 101 2026-10-17T02:41:00Z schedule
export STUB_COMMIT=7777777777777777777777777777777777777777
check "the stub answers only reads of exactly the run's commit (a read of another commit fails)" 1 "$(run guard)"
check "and records the read it did not expect" 0 "$([ -s "${STATE}/unexpected.log" ]; echo $?)"

echo "== the publish job re-checks the guard's conditions after the approval (decision of 2026-10-17)"
recheck_setup() { # <version> <ref> <dry_run input> [repository]
  guard_setup "$1" workflow_dispatch "$2" "$3" "${4:-smormah/vsift}"
  rm -f "${WORK}/run/Cargo.toml"
  export VERSION="$1"
}
recheck_setup 0.2.2-rc.1 refs/tags/v0.2.2-rc.1 false
on_branch ahead
passed_run 101 2026-10-17T02:41:00Z schedule
check "a candidate that still qualifies after the approval is published" 0 "$(run recheck)"
check "every read was of exactly this commit, the branch head's own commit and this tag" 0 "$(no_unexpected_read)"
check "the re-check reads the commit the tag names now" 0 "$(grep -q 'git/ref/tags/v0.2.2-rc.1' "${STATE}/gh.log"; echo $?)"
check "and the rules of the release branch" 0 "$(grep -q 'rules/branches/release/0.2.x' "${STATE}/gh.log"; echo $?)"
check "and needs no checkout (no Cargo.toml in the publish job)" 0 "$([ ! -e Cargo.toml ]; echo $?)"
nightly_run 102 2026-10-18T02:41:00Z schedule completed '"success"' "${GATED_OUT[@]}"
check "a later scheduled run that ran nothing changes nothing" 0 "$(run recheck)"
nightly_run 103 2026-10-18T09:00:00Z workflow_dispatch in_progress null
check "a nightly re-run still in progress during the approval refuses the publish" 1 "$(run recheck)"
nightly_run 103 2026-10-18T09:00:00Z workflow_dispatch completed '"failure"' "${FAILED_JOBS[@]}"
check "a nightly re-run that failed during the wait refuses the publish" 1 "$(run recheck)"
check "saying it was refused after the approval" 0 "$(grep -q 'Refused after the approval, before anything is published' "${WORK}/out.log" && grep -q 'nothing is published' "${GITHUB_STEP_SUMMARY}"; echo $?)"

recheck_setup 0.2.2-rc.1 refs/tags/v0.2.2-rc.1 false
on_branch ahead
passed_run 101 2026-10-17T02:41:00Z schedule
tag_at 4444444444444444444444444444444444444444
check "a tag moved to another commit during the wait refuses the publish" 1 "$(run recheck)"
check "naming both commits" 0 "$(grep -q "names 4444444444444444444444444444444444444444 now, not ${COMMIT}" "${WORK}/out.log"; echo $?)"
annotated_tag_at "${COMMIT}"
check "an annotated tag of the run's commit is peeled and published" 0 "$(run recheck)"
rm -f "${STATE}/tag.json"
check "a tag GitHub cannot resolve refuses the publish" 1 "$(run recheck)"
tag_at "${COMMIT}"
on_branch diverged
check "a release branch that no longer holds the commit refuses the publish" 1 "$(run recheck)"
no_branch
check "a release branch deleted during the wait refuses the publish" 1 "$(run recheck)"
on_branch ahead
rules creation deletion non_fast_forward pull_request
check "a release branch whose required checks were removed during the wait refuses the publish" 1 "$(run recheck)"

recheck_setup 0.2.2 refs/tags/v0.2.2 false
on_branch ahead
check "a stable version on its protected release branch passes without a nightly of its own" 0 "$(run recheck)"
check "and reads no Nightly run" 0 "$(! grep -q 'nightly.yml' "${STATE}/gh.log"; echo $?)"

recheck_setup 0.2.2-rc.1 refs/heads/main false
on_branch ahead
passed_run 101 2026-10-17T02:41:00Z schedule
check "a publish on a branch is refused again" 1 "$(run recheck)"
recheck_setup 0.2.2-rc.1 refs/tags/v0.2.2-rc.1 true
check "a run with dry_run set is refused" 1 "$(run recheck)"
recheck_setup 0.2.2-rc.1 refs/tags/v0.2.2-rc.1 false someone/vsift
check "a fork is refused" 1 "$(run recheck)"
recheck_setup 0.2.2-rc.1 refs/tags/v0.2.2-rc.2 false
check "a ref that is not the planned version's tag is refused" 1 "$(run recheck)"
check "the refusals of the ref read nothing" 0 "$([ "$(api_calls)" = 0 ]; echo $?)"

if command -v jq > /dev/null 2>&1; then
  check "the guard's --jq filters ran on GitHub's JSON" 0 "$([ -s "${FILTER_RAN}" ]; echo $?)"
  check "and gave what the fixtures expect" 0 "$([ ! -s "${FILTER_MISMATCH}" ]; echo $?)"
else
  echo "note - jq is not installed: the guard's --jq filters were not run (CI's Linux runners run them)"
fi
echo
echo "passed ${PASS}, failed ${FAIL}"
[ "${FAIL}" = 0 ]
