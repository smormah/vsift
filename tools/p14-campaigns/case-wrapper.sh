#!/bin/sh
# Runs one command in a P14 malicious-media container and then reports what the
# container's own cgroup says it used, on the standard error stream:
#
#   P14-METRICS exit=<status> memory_peak=<bytes> pids_peak=<count> own_peak=<bytes> anon_peak=<bytes> file_end=<bytes> memory_events=<text>
#
# `memory_peak` is the cgroup's high-water mark. It counts the page cache of
# every file the command reads or writes in the container, which the kernel
# reclaims when the limit is reached, so a command that copies a large file
# peaks at the limit without needing that memory (L-194). The other two say
# what the command itself held:
#
# - `own_peak`: the command's own resident high-water mark (`VmHWM` of the
#   process the wrapper starts, `vsift` itself), read every 50 ms while it runs;
#   it only rises, so the last reading misses at most the last 50 ms;
# - `anon_peak`: the largest anonymous memory of the whole cgroup (`anon` in
#   `memory.stat`: VSift and every provider it starts) seen at those readings,
#   a sample that can miss a short spike;
# - `file_end`: the page cache still charged to the cgroup when the command
#   has ended (`file` in `memory.stat`).
#
# The readings run beside the command: at most one `sleep` at a time, which
# `pids_peak` counts. Used only for the single commands of RQ-10; the batches
# of RQ-09 and RQ-12 run `vsift` directly as PID 1's child, so a signal reaches
# it unchanged.

own_peak=0
anon_peak=0

# The value of the line `<key> <number>` (memory.stat) or `<key>: <number> kB`
# (/proc status) in a file, or nothing.
field() {
  while read -r name value unit; do
    if [ "$name" = "$2" ]; then
      echo "$value"
      return 0
    fi
  done < "$1"
}

reading() {
  hwm=$(field "/proc/$child/status" 'VmHWM:' 2>/dev/null)
  if [ -n "$hwm" ] && [ "$((hwm * 1024))" -gt "$own_peak" ]; then own_peak=$((hwm * 1024)); fi
  anon=$(field /sys/fs/cgroup/memory.stat anon 2>/dev/null)
  if [ -n "$anon" ] && [ "$anon" -gt "$anon_peak" ]; then anon_peak=$anon; fi
}

# The command runs as a background job of this shell, so its standard input is
# /dev/null (a container started without -i has none anyway). The readings stop
# when it has ended: its status file is gone, or it is a zombie waiting to be
# reaped by `wait`.
"$@" &
child=$!
while state=$(field "/proc/$child/status" 'State:' 2>/dev/null) && [ -n "$state" ] && [ "$state" != Z ]; do
  reading
  sleep 0.05
done
wait "$child"
status=$?

memory_peak=$(cat /sys/fs/cgroup/memory.peak 2>/dev/null || echo unknown)
pids_peak=$(cat /sys/fs/cgroup/pids.peak 2>/dev/null || echo unknown)
file_end=$(field /sys/fs/cgroup/memory.stat file 2>/dev/null)
memory_events=$(tr '\n' ',' < /sys/fs/cgroup/memory.events 2>/dev/null || echo unknown)
echo "P14-METRICS exit=$status memory_peak=$memory_peak pids_peak=$pids_peak own_peak=$own_peak anon_peak=$anon_peak file_end=${file_end:-unknown} memory_events=$memory_events" >&2
exit "$status"
