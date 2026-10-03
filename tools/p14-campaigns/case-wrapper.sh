#!/bin/sh
# Runs one command in a P14 malicious-media container and then reports what the
# container's own cgroup says it used, on the standard error stream:
#
#   P14-METRICS exit=<status> memory_peak=<bytes> pids_peak=<count> memory_events=<text>
#
# Used only for the single commands of RQ-10; the batches of RQ-09 and RQ-12 run
# `vsift` directly as PID 1's child, so a signal reaches it unchanged.
"$@"
status=$?
memory_peak=$(cat /sys/fs/cgroup/memory.peak 2>/dev/null || echo unknown)
pids_peak=$(cat /sys/fs/cgroup/pids.peak 2>/dev/null || echo unknown)
memory_events=$(tr '\n' ',' < /sys/fs/cgroup/memory.events 2>/dev/null || echo unknown)
echo "P14-METRICS exit=$status memory_peak=$memory_peak pids_peak=$pids_peak memory_events=$memory_events" >&2
exit "$status"
