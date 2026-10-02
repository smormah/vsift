#!/bin/bash
# In-container driver for P12's Codex trials (runbook: docs/agents/trials.md,
# "Codex trials in a Linux container"). codex-trial.ps1 starts one container
# per step and passes every value as its own argument; this script checks
# each value against a fixed pattern and hands it on as a single argument.
# Nothing here builds a command from a string.
#
# The steps run in separate containers so the agent never shares one with
# the repository:
#   prepare  (harness image)  writes the trial workspace from the corpus;
#   run      (agent image)    Codex and its agent, with only this trial's
#                             folder, the model and the sign-in mounted;
#   grade    (harness image)  after the agent has exited: grade, record, export.
set -euo pipefail
umask 077

readonly SRC=/opt/vsift/src
readonly VSIFT=/opt/vsift/bin/vsift
readonly HARNESS=/opt/vsift-trials/bin/vsift-agent-trials
readonly CODEX=/opt/codex/bin/codex
readonly FFMPEG=/opt/ffmpeg/bin/ffmpeg
readonly FFPROBE=/opt/ffmpeg/bin/ffprobe
readonly WHISPER=/opt/whisper.cpp/bin/whisper-cli
readonly MODEL=/opt/models/ggml-base.bin
# The reviewed ggml-base.bin (tools/p06_windows_candidate_smoke.py).
readonly MODEL_SIZE=147951465
readonly MODEL_SHA256=60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe
readonly TRIALS=/trials
readonly EXPORTS=/exports
# The published images (P14) carry the proof that VSift was installed from the
# real registry; the images built from the checkout (P12) do not.
readonly PUBLISHED_PROOF=/opt/vsift-published-proof.json
readonly PUBLISHED_VERSION_FILE=/opt/vsift-published/lib/node_modules/vsift-cli/package.json
readonly AUTH_SOURCE=/run/codex-auth/auth.json
readonly CODEX_HOME_DIR=/run/codex-home
# A trial directory relative to /trials: <trial-id> or debug-<name>/<trial-id>.
readonly TRIAL_PATTERN='^(debug-[a-z0-9][a-z0-9-]{0,40}/)?[a-z0-9][a-z0-9-]{0,80}$'
readonly MODEL_PATTERN='^[A-Za-z0-9._-]{1,64}$'

fail() {
  printf 'trial-driver: %s\n' "$1" >&2
  exit 2
}

usage() {
  cat <<'EOF'
trial-driver versions
    Tool versions and digests; checks the mounted model (a published image
    has no model of its own and prints the install proof instead). No network.
trial-driver sandbox-check
    Codex's Linux sandbox without a model: a write inside the workspace works,
    a write outside it and a network request (curl) fail. One plain HTTPS
    request outside the sandbox is the control.
trial-driver prepare --scenario <id> [--debug <name>] [--freeze <file>]
                                                          (harness image)
    Prepares a trial under /trials (or /trials/debug-<name>) and prints
    "trial <relative directory>". The scenario is looked up in scenarios/,
    cold/ and holdout/. In a published image the harness uses the installed
    package and runs setup plan and setup install for the managed tools; no
    model file is needed or checked.
trial-driver run --trial <relative directory> --model <model> [--phase <n>]
                 [--timeout-s <n>] [--debug-prompt <text>]  (agent image)
    Runs Codex once. The sign-in is copied to a tmpfs CODEX_HOME for the run
    and deleted afterwards.
trial-driver grade --trial <relative directory> [--phase <n>] (harness image)
    Grades, records (not a debug run) to /exports/records/ and exports the
    harness folder (raw logs) to /exports/trials/.
trial-driver regrade --trial <relative directory> --output <grade-name.json>
                     [--phase <n>]                           (harness image)
    Grades a finished trial again with this image's grader, from the raw
    logs only, into harness/phase-<n>/<grade-name.json> beside the original
    grade (never over it), and exports only that file.
EOF
}

require_match() {
  # $1 value, $2 pattern, $3 what
  [[ "$1" =~ $2 ]] || fail "invalid $3"
}

is_published() {
  [ -f "$PUBLISHED_PROOF" ]
}

check_model() {
  [ -f "$MODEL" ] || fail "the reviewed model is not mounted at $MODEL"
  [ "$(stat -c %s "$MODEL")" = "$MODEL_SIZE" ] || fail "the mounted model has the wrong size"
  echo "$MODEL_SHA256  $MODEL" | sha256sum --check --strict --quiet \
    || fail "the mounted model's SHA-256 is not the reviewed digest"
  echo "model: ggml-base.bin size and SHA-256 match the reviewed digest"
}

trial_directory() {
  # $1 relative directory; prints the absolute one
  require_match "$1" "$TRIAL_PATTERN" "trial directory"
  [ -f "$TRIALS/$1/harness/trial.json" ] || fail "no prepared trial at /trials/$1"
  printf '%s\n' "$TRIALS/$1"
}

# A fresh CODEX_HOME on the tmpfs, holding only a copy of the sign-in file.
auth_digest=""
setup_codex_home() {
  [ -f "$AUTH_SOURCE" ] || fail "the Codex sign-in file is not mounted"
  [ -d "$CODEX_HOME_DIR" ] || fail "$CODEX_HOME_DIR is not mounted (tmpfs)"
  find "$CODEX_HOME_DIR" -mindepth 1 -delete
  install -m 0600 "$AUTH_SOURCE" "$CODEX_HOME_DIR/auth.json"
  auth_digest=$(sha256sum "$CODEX_HOME_DIR/auth.json" | cut -d' ' -f1)
  trap cleanup_codex_home EXIT
}

cleanup_codex_home() {
  if [ -f "$CODEX_HOME_DIR/auth.json" ]; then
    local now
    now=$(sha256sum "$CODEX_HOME_DIR/auth.json" | cut -d' ' -f1)
    if [ "$now" != "$auth_digest" ]; then
      echo "note: Codex refreshed its sign-in during the run; the refreshed copy was discarded." >&2
      echo "      If a later run cannot sign in, sign in again into the Codex trial home." >&2
    fi
  fi
  find "$CODEX_HOME_DIR" -mindepth 1 -delete 2>/dev/null || true
}

cmd_versions() {
  echo "vsift commit: $VSIFT_COMMIT"
  if is_published; then
    # The published image: the install proof, the launcher and Codex.
    "$HARNESS" verify-install --proof "$PUBLISHED_PROOF"
    PATH=/opt/node/bin:/usr/bin:/bin /opt/vsift-published/bin/vsift --version
    "$CODEX" --version
    sha256sum "$HARNESS" "$CODEX" /opt/codex/codex-resources/bwrap
    if [ -e "$SRC" ]; then echo "repository: present (harness image)"; else echo "repository: absent (agent image)"; fi
    return 0
  fi
  "$VSIFT" --version
  "$CODEX" --version
  "$FFMPEG" -hide_banner -version | head -n 1
  "$FFPROBE" -hide_banner -version | head -n 1
  sha256sum "$VSIFT" "$HARNESS" "$CODEX" /opt/codex/codex-resources/bwrap "$FFMPEG" "$FFPROBE" "$WHISPER"
  if [ -e "$SRC" ]; then echo "repository: present (harness image)"; else echo "repository: absent (agent image)"; fi
  check_model
}

cmd_sandbox_check() {
  local root="$TRIALS/debug-sandbox-check"
  mkdir -p "$root"
  local work
  work=$(mktemp -d "$root/run-XXXXXX")
  mkdir -p "$work/workspace" "$work/codex-home" "$work/tmp"
  echo "control, outside the sandbox:"
  if curl --silent --show-error --max-time 15 --output /dev/null https://example.com/; then
    echo "  curl https://example.com/: reachable (the container has network)"
  else
    echo "  curl https://example.com/: FAILED outside the sandbox; the check below proves nothing"
  fi
  echo "inside Codex's sandbox (workspace-write, network off):"
  # A fixed probe script; the work directory reaches it as "$1".
  local probe='
echo "  uid inside: $(id -u)"
if echo ok > "$1/workspace/inside.txt"; then echo "  write inside the workspace: allowed"; else echo "  write inside the workspace: DENIED"; fi
if echo no > "$1/outside.txt" 2>/dev/null; then echo "  write outside the writable roots: ALLOWED (bad)"; else echo "  write outside the writable roots: denied"; fi
if curl --silent --show-error --max-time 15 --output /dev/null https://example.com/; then echo "  curl https://example.com/: SUCCEEDED (bad)"; else echo "  curl https://example.com/: failed (exit $?)"; fi
'
  HOME="$work/workspace" TMPDIR="$work/tmp" CODEX_HOME="$work/codex-home" \
    "$CODEX" sandbox -P ':workspace' -C "$work/workspace" \
      -c 'sandbox_workspace_write.network_access=false' \
      -c 'sandbox_workspace_write.exclude_tmpdir_env_var=true' \
      -c 'sandbox_workspace_write.exclude_slash_tmp=true' \
      -- /bin/bash -c "$probe" probe "$work"
  rm -rf "$work"
}

# The scenario file for an id: the tuning, cold or hold-out folder.
scenario_file() {
  local folder
  for folder in scenarios cold holdout; do
    if [ -f "$SRC/tools/vsift-agent-trials/$folder/$1.json" ]; then
      printf '%s\n' "$SRC/tools/vsift-agent-trials/$folder/$1.json"
      return 0
    fi
  done
  return 1
}

cmd_prepare() {
  local scenario="" debug="" freeze=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --scenario) scenario=${2-}; shift 2 ;;
      --debug) debug=${2-}; shift 2 ;;
      --freeze) freeze=${2-}; shift 2 ;;
      *) fail "unknown argument" ;;
    esac
  done
  require_match "$scenario" '^[A-Za-z0-9-]{1,64}$' "scenario"
  local scenario_path
  scenario_path=$(scenario_file "$scenario") || fail "no such scenario (or not the harness image)"
  local root="$TRIALS"
  if [ -n "$debug" ]; then
    require_match "$debug" '^[a-z0-9][a-z0-9-]{0,40}$' "debug name"
    root="$TRIALS/debug-$debug"
  fi
  local freeze_arguments=()
  if [ -n "$freeze" ]; then
    [ "$freeze" = /run/freeze.json ] && [ -f "$freeze" ] || fail "the freeze file is not mounted at /run/freeze.json"
    freeze_arguments=(--freeze "$freeze")
  fi
  local out
  if is_published; then
    # Clean-install mode: the harness plays the user's part (setup plan, then
    # setup install with the plan's digest). FFmpeg here only builds clips.
    out=$("$HARNESS" prepare --root "$root" \
      --scenario "$scenario_path" \
      --install-proof "$PUBLISHED_PROOF" --tools managed \
      --vsift-commit "$VSIFT_COMMIT" \
      --ffmpeg "$FFMPEG" --ffprobe "$FFPROBE" \
      --repository "$SRC" "${freeze_arguments[@]}")
  else
    check_model
    out=$("$HARNESS" prepare --root "$root" \
      --scenario "$scenario_path" \
      --vsift "$VSIFT" --vsift-commit "$VSIFT_COMMIT" \
      --ffmpeg "$FFMPEG" --ffprobe "$FFPROBE" --whisper "$WHISPER" --model "$MODEL" \
      --repository "$SRC" "${freeze_arguments[@]}")
  fi
  local trial=${out#prepared }
  [[ "$trial" == "$root"/* && -d "$trial" ]] || fail "prepare did not report a trial under $root"
  printf 'trial %s\n' "${trial#"$TRIALS"/}"
}

cmd_run() {
  local relative="" model="" timeout=1500 phase=1 debug_prompt="" has_debug=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --trial) relative=${2-}; shift 2 ;;
      --phase) phase=${2-}; shift 2 ;;
      --model) model=${2-}; shift 2 ;;
      --timeout-s) timeout=${2-}; shift 2 ;;
      --debug-prompt) debug_prompt=${2-}; has_debug=1; shift 2 ;;
      *) fail "unknown argument" ;;
    esac
  done
  [ ! -e "$SRC" ] || fail "run refuses the harness image: the agent must not see the repository"
  local trial
  trial=$(trial_directory "$relative")
  require_match "$model" "$MODEL_PATTERN" "model"
  require_match "$timeout" '^[0-9]{2,5}$' "timeout"
  require_match "$phase" '^[1-9]$' "phase"
  if [ "$has_debug" = 1 ]; then
    [ -n "$debug_prompt" ] && [ ${#debug_prompt} -le 4000 ] || fail "the prompt must be 1-4000 characters"
    [[ "$relative" == debug-* ]] || fail "a debug prompt needs a trial prepared with --debug"
  fi
  is_published || check_model
  setup_codex_home
  local arguments=(run --trial "$trial" --phase "$phase" --client codex --executable "$CODEX"
    --model "$model" --client-home "$CODEX_HOME_DIR" --timeout-s "$timeout")
  if [ "$has_debug" = 1 ]; then
    arguments+=(--debug-prompt "$debug_prompt")
  fi
  "$HARNESS" "${arguments[@]}"
}

cmd_grade() {
  local relative="" phase=1
  while [ $# -gt 0 ]; do
    case "$1" in
      --trial) relative=${2-}; shift 2 ;;
      --phase) phase=${2-}; shift 2 ;;
      *) fail "unknown argument" ;;
    esac
  done
  [ -e "$SRC" ] || fail "grade needs the harness image"
  local trial
  trial=$(trial_directory "$relative")
  require_match "$phase" '^[1-9]$' "phase"
  "$HARNESS" grade --trial "$trial" --phase "$phase" || true
  local id
  id=$(basename "$trial")
  if [[ "$relative" != debug-* ]]; then
    local suffix=""
    [ "$phase" = 1 ] || suffix="-phase-$phase"
    mkdir -p "$EXPORTS/records"
    "$HARNESS" record --trial "$trial" --phase "$phase" \
      --output "$EXPORTS/records/$id$suffix-codex.json" \
      --client-home "$CODEX_HOME_DIR" || true
  fi
  mkdir -p "$EXPORTS/trials/$id"
  cp -R "$trial/harness" "$EXPORTS/trials/$id/"
  echo "exported /exports/trials/$id/harness"
}

cmd_regrade() {
  local relative="" phase=1 output=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --trial) relative=${2-}; shift 2 ;;
      --phase) phase=${2-}; shift 2 ;;
      --output) output=${2-}; shift 2 ;;
      *) fail "unknown argument" ;;
    esac
  done
  [ -e "$SRC" ] || fail "regrade needs the harness image"
  local trial
  trial=$(trial_directory "$relative")
  require_match "$phase" '^[1-9]$' "phase"
  require_match "$output" '^grade-[a-z0-9-]{1,32}\.json$' "grade file name"
  "$HARNESS" grade --trial "$trial" --phase "$phase" --output "$output" || true
  local id
  id=$(basename "$trial")
  mkdir -p "$EXPORTS/trials/$id/harness/phase-$phase"
  cp "$trial/harness/phase-$phase/$output" "$EXPORTS/trials/$id/harness/phase-$phase/$output"
  echo "exported /exports/trials/$id/harness/phase-$phase/$output"
}

[ -n "${VSIFT_COMMIT:-}" ] || fail "VSIFT_COMMIT is not set in the image"
command=${1-help}
[ $# -gt 0 ] && shift
case "$command" in
  versions) cmd_versions ;;
  sandbox-check) cmd_sandbox_check ;;
  prepare) cmd_prepare "$@" ;;
  run) cmd_run "$@" ;;
  grade) cmd_grade "$@" ;;
  regrade) cmd_regrade "$@" ;;
  help|--help|-h) usage ;;
  *) usage; exit 2 ;;
esac
