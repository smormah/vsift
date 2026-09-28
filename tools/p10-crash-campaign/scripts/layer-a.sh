#!/usr/bin/env bash
# Layer A of the P10 crash campaign (ADR 0020 section 7): power loss at every
# flush.
#
# A fresh ext4 filesystem sits on a dm-log-writes device, which records every
# write, flush and FUA write on a separate log device. The workload runs
# durable operations on it and marks the log after every acknowledgement. The
# replay then rebuilds the device after every flush and FUA entry of the log,
# mounts it (ext4 replays its journal as after a power loss), verifies every
# acknowledgement made before that point and runs `e2fsck -fn`.
#
# Usage (as root): layer-a.sh <binary> <work dir> <operations> <seed> <positive|negative> <out dir>
#
# In the negative control the binary must be a `campaign` build and
# VSIFT_CAMPAIGN_NEGATIVE_CONTROL=1 removes the session directory
# synchronisation after every pointer rename; the replay must then find lost
# acknowledgements, or the campaign cannot see what it claims to test.
set -euo pipefail

binary=$1
work=$2
operations=$3
seed=$4
mode=$5
out=$6

case "$operations" in '' | *[!0-9]*) echo "operations must be a number" >&2; exit 2 ;; esac
case "$seed" in '' | *[!0-9]*) echo "seed must be a number" >&2; exit 2 ;; esac
case "$mode" in positive | negative) ;; *) echo "mode must be positive or negative" >&2; exit 2 ;; esac

device=vsift-logwrites-$mode
data=$work/data.img
log=$work/log.img
base=$work/base.img
mountpoint=$work/mnt

mkdir -p "$work" "$out" "$mountpoint" "$work/scratch"
rm -f "$data" "$log" "$base"
truncate -s 3G "$data"
truncate -s 8G "$log"
# Ubuntu 24.04's mke2fs defaults, 4 KiB blocks, and every inode table and the
# journal written now, so no lazy initialisation runs while the log records.
mkfs.ext4 -q -F -b 4096 -E lazy_itable_init=0,lazy_journal_init=0 "$data"
dumpe2fs -h "$data" > "$out/dumpe2fs.txt" 2>/dev/null
cp --sparse=always "$data" "$base"

data_device=$(losetup --show -f "$data")
log_device=$(losetup --show -f "$log")
cleanup() {
  umount "$mountpoint" 2>/dev/null || true
  dmsetup remove "$device" 2>/dev/null || true
  losetup -d "$data_device" "$log_device" 2>/dev/null || true
}
trap cleanup EXIT

sectors=$(blockdev --getsz "$data_device")
dmsetup create "$device" --table "0 $sectors log-writes $data_device $log_device"
mount -t ext4 "/dev/mapper/$device" "$mountpoint"
{
  echo "kernel=$(uname -r)"
  echo "os=$(. /etc/os-release && echo "$ID $VERSION_ID")"
  echo "mount=$(findmnt -no FSTYPE,OPTIONS "$mountpoint")"
  echo "targets=$(dmsetup targets | tr '\n' ';')"
} > "$out/environment.txt"

started=$(date +%s)
if [ "$mode" = negative ]; then
  export VSIFT_CAMPAIGN_NEGATIVE_CONTROL=1
fi
"$binary" workload --root "$mountpoint/root" --scratch "$work/scratch" \
  --ack-out "$out/acks.log" --mark-device "$device" \
  --max-ops "$operations" --seed "$seed" --recognizer-delay-ms 0
unset VSIFT_CAMPAIGN_NEGATIVE_CONTROL
workload_seconds=$(( $(date +%s) - started ))

umount "$mountpoint"
dmsetup remove "$device"
losetup -d "$data_device" "$log_device"
trap - EXIT
echo "log_allocated_bytes=$(du -B1 "$log" | cut -f1)" >> "$out/environment.txt"

started=$(date +%s)
status=0
"$binary" replay --log "$log" --base "$base" --work "$work" --acks "$out/acks.log" \
  --mount-point "$mountpoint" --report "$out/replay-report.txt" || status=$?
replay_seconds=$(( $(date +%s) - started ))
summary=$(grep '^SUMMARY ' "$out/replay-report.txt" || true)
echo "RESULT mode=$mode status=$status workload_seconds=$workload_seconds replay_seconds=$replay_seconds $summary" | tee "$out/result.txt"

case "$mode:$status" in
  positive:0) exit 0 ;;
  negative:1)
    # The control passes only if acknowledgements were lost.
    lost=$(sed -n 's/.* lost_acks=\([0-9]*\).*/\1/p' <<< "$summary")
    if [ "${lost:-0}" -gt 0 ]; then exit 0; fi
    echo "negative control lost no acknowledgement: the harness cannot detect loss" >&2
    exit 1 ;;
  negative:0)
    echo "negative control lost no acknowledgement: the harness cannot detect loss" >&2
    exit 1 ;;
  *) exit 1 ;;
esac
