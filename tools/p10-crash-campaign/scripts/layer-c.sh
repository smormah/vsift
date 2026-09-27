#!/usr/bin/env bash
# Layer C of the P10 crash campaign (ADR 0020 section 7): write errors.
#
# A fresh ext4 filesystem sits on a device-mapper device whose table the
# harness switches, at a random moment while the workload commits, from
# `linear` to `flakey ... error_writes` (every write and flush fails with
# EIO; dm-dust is not available on the hosted runners). The first failure
# is usually an fsync (the data write-back or the journal commit), after
# which ext4 aborts its journal and later calls fail too. Each round:
#
# 1. mount, `verify` every acknowledgement so far;
# 2. run the workload, inject, let it stop after three failures in a row;
# 3. `assess` the round: every failure is STORAGE_IO, none happened before
#    the injection, and no operation started after it was acknowledged;
# 4. restore the table, unmount, and carry the round's acknowledgements on.
#
# Usage (as root): layer-c.sh <binary> <work dir> <rounds> <seed> <out dir>
set -euo pipefail

binary=$1
work=$2
rounds=$3
seed=$4
out=$5

case "$rounds" in '' | *[!0-9]*) echo "rounds must be a number" >&2; exit 2 ;; esac
case "$seed" in '' | *[!0-9]*) echo "seed must be a number" >&2; exit 2 ;; esac

device=vsift-flakey
data=$work/flakey.img
mountpoint=$work/mnt
acks=$out/acks.log
mkdir -p "$work" "$out" "$mountpoint" "$work/scratch"
rm -f "$data"
: > "$acks"
truncate -s 1G "$data"
mkfs.ext4 -q -F -b 4096 -E lazy_itable_init=0,lazy_journal_init=0 "$data"
dumpe2fs -h "$data" > "$out/dumpe2fs.txt" 2>/dev/null
loop=$(losetup --show -f "$data")
sectors=$(blockdev --getsz "$loop")
cleanup() {
  umount "$mountpoint" 2>/dev/null || true
  dmsetup remove "$device" 2>/dev/null || true
  losetup -d "$loop" 2>/dev/null || true
}
trap cleanup EXIT
dmsetup create "$device" --table "0 $sectors linear $loop 0"

switch_table() {
  dmsetup suspend --nolockfs "$device"
  dmsetup reload "$device" --table "$1"
  dmsetup resume "$device"
}

RANDOM=$seed
passed=0
injected_rounds=0
injected_failures=0
storage_io=0
fsck_nonzero=0
{
  echo "kernel=$(uname -r)"
  echo "os=$(. /etc/os-release && echo "$ID $VERSION_ID")"
  echo "targets=$(dmsetup targets | tr '\n' ';')"
} > "$out/environment.txt"

for round in $(seq 1 "$rounds"); do
  mount -t ext4 "/dev/mapper/$device" "$mountpoint"
  if [ "$round" = 1 ]; then
    echo "mount=$(findmnt -no FSTYPE,OPTIONS "$mountpoint")" >> "$out/environment.txt"
  fi
  verify_status=0
  "$binary" verify --root "$mountpoint/root" --acks "$acks" --prefix "VERIFY-C$round" \
    >> "$out/verify.log" || verify_status=$?
  if [ "$verify_status" != 0 ]; then
    echo "round $round: verification failed ($verify_status)" | tee -a "$out/rounds.log"
    exit 1
  fi
  log=$out/round-$round.log
  : > "$log"
  "$binary" workload --root "$mountpoint/root" --scratch "$work/scratch" \
    --ack-out "$log" --known-acks "$acks" --first-seq $(( round * 100000 )) \
    --max-ops 400 --stop-after-failures 3 --seed $(( seed + round )) &
  workload=$!
  # Inject 50-1550 ms into the round, while the workload is committing.
  delay=$(( 50 + RANDOM % 1500 ))
  sleep "$(printf '%d.%03d' $(( delay / 1000 )) $(( delay % 1000 )))"
  switch_table "0 $sectors flakey $loop 0 0 1 1 error_writes"
  echo "INJECT $(date +%s%N)" >> "$log"
  workload_status=0
  if ! timeout 300 tail --pid="$workload" -f /dev/null; then
    kill -9 "$workload" 2>/dev/null || true
  fi
  wait "$workload" || workload_status=$?
  umount "$mountpoint" || umount -l "$mountpoint"
  switch_table "0 $sectors linear $loop 0"
  # Recover the journal as the next mount would, then check the filesystem.
  mount -t ext4 "/dev/mapper/$device" "$mountpoint"
  umount "$mountpoint"
  fsck_status=0
  e2fsck -fn "/dev/mapper/$device" > "$out/fsck-$round.log" 2>&1 || fsck_status=$?
  assessment_status=0
  assessment=$("$binary" assess --log "$log") || assessment_status=$?
  echo "round $round delay_ms=$delay workload_status=$workload_status fsck=$fsck_status $assessment" \
    | tee -a "$out/rounds.log"
  if [ "$fsck_status" != 0 ]; then fsck_nonzero=$(( fsck_nonzero + 1 )); fi
  grep -a '^ACK ' "$log" >> "$acks" || true
  if [ "$assessment_status" != 0 ]; then
    echo "round $round failed its assessment" >&2
    exit 1
  fi
  failures=$(sed -n 's/.* injected_failures=\([0-9]*\).*/\1/p' <<< "$assessment")
  typed=$(sed -n 's/.* storage_io=\([0-9]*\).*/\1/p' <<< "$assessment")
  if [ "${failures:-0}" -gt 0 ]; then injected_rounds=$(( injected_rounds + 1 )); fi
  injected_failures=$(( injected_failures + ${failures:-0} ))
  storage_io=$(( storage_io + ${typed:-0} ))
  passed=$(( passed + 1 ))
done

# A last mount after the final restore: every acknowledgement must hold.
mount -t ext4 "/dev/mapper/$device" "$mountpoint"
final_status=0
"$binary" verify --root "$mountpoint/root" --acks "$acks" --prefix VERIFY-FINAL >> "$out/verify.log" \
  || final_status=$?
umount "$mountpoint"
e2fsck -fn "/dev/mapper/$device" > "$out/fsck-final.log" 2>&1 || final_status=$(( final_status + 10 ))

echo "RESULT rounds=$rounds passed=$passed injected_rounds=$injected_rounds \
injected_failures=$injected_failures storage_io=$storage_io acks=$(wc -l < "$acks") \
fsck_nonzero=$fsck_nonzero final_status=$final_status" | tee "$out/result.txt"
[ "$final_status" = 0 ] && [ "$fsck_nonzero" = 0 ] && [ "$injected_failures" -gt 0 ] \
  && [ "$injected_failures" = "$storage_io" ]
