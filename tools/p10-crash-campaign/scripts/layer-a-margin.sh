#!/usr/bin/env bash
# Reports how much of its timeout a layer A job used, so a shrinking margin is
# seen in a run that still passes and not discovered as a timeout.
#
# Layer A's positive run is about 99% replay, linear in the number of replay
# points (docs/planning/p10-durable-publication.md, "Layer A's time"). Its
# runtime therefore moves with the workload and with the hosted runner's speed,
# neither of which the campaign controls. A timed-out run proves nothing.
#
# Prints a Markdown line for the job's step summary and, above 60% of the
# timeout, a workflow `::warning::`. When the replay step did not succeed it says
# so plainly: an unfinished layer A is no evidence either way.
#
# Usage: layer-a-margin.sh <positive|negative> <job start, epoch seconds> <replay step outcome> <job timeout, minutes> [now, epoch seconds]
set -euo pipefail

mode=$1
started=$2
outcome=$3
timeout_minutes=$4
now=${5:-$(date +%s)}

case "$mode" in positive | negative) ;; *) echo "mode must be positive or negative" >&2; exit 2 ;; esac
case "$started" in '' | *[!0-9]*) echo "start must be epoch seconds" >&2; exit 2 ;; esac
case "$timeout_minutes" in '' | *[!0-9]* | 0) echo "timeout must be a positive number of minutes" >&2; exit 2 ;; esac
case "$now" in '' | *[!0-9]*) echo "now must be epoch seconds" >&2; exit 2 ;; esac

elapsed_minutes=$(( (now - started) / 60 ))
percent=$(( (now - started) * 100 / (timeout_minutes * 60) ))
# 60% of the timeout, in whole seconds, compared with the exact elapsed time.
warn_seconds=$(( timeout_minutes * 60 * 60 / 100 ))

if [ "$outcome" != success ]; then
  # A failed replay and one stopped by its step timeout both end here; the
  # elapsed time tells them apart (a stopped one sits at the step's limit).
  echo "- **Layer A ($mode) did not pass** (replay step: $outcome) after $elapsed_minutes of $timeout_minutes minutes ($percent%). If the replay was stopped by its timeout, layer A did not finish and proves nothing either way; otherwise the replay step's output says what failed."
  echo "::warning title=Layer A ($mode) did not pass::The replay step ended as $outcome after $elapsed_minutes of $timeout_minutes minutes ($percent%). A run stopped by its timeout proves nothing either way."
  exit 0
fi

echo "- Layer A ($mode) used $elapsed_minutes of $timeout_minutes minutes ($percent%)."
if [ $(( now - started )) -gt "$warn_seconds" ]; then
  echo "::warning title=Layer A ($mode) is using its timeout::$elapsed_minutes of $timeout_minutes minutes ($percent%), above 60%. See Layer A's time in docs/planning/p10-durable-publication.md: shard it if this happens twice."
fi
