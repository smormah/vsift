#!/bin/bash
# Runs at every boot of the layer-B campaign guest (vsift-campaign.service).
#
# The host attaches four disks, found by their virtio serial numbers: the OS,
# the data disk under test (ext4, cache=none), a read-only disk holding
# "CYCLE <n> SEED <s> MODE <run|verify>" and every acknowledgement the host
# has seen so far, and this tools disk. Protocol lines go to the second serial
# port, which the host captures to a file; the kernel console stays on the
# first.
#
# Each boot recovers the data disk's journal (mount), checks it (`e2fsck
# -fn`), verifies every acknowledgement, then continues the workload until
# the host kills the machine.
set -u

channel=/dev/ttyS1
stty -F "$channel" raw -echo 115200 2>/dev/null
exec 3>>"$channel"
say() { printf '%s\n' "$*" >&3; }

data=/dev/disk/by-id/virtio-vsift-data
acks=/dev/disk/by-id/virtio-vsift-acks
tools=/opt/campaign

for _ in $(seq 1 100); do
  [ -b "$data" ] && [ -b "$acks" ] && break
  sleep 0.1
done
read -r _ cycle _ seed _ mode < <(head -c 4096 "$acks" | tr -d '\0' | head -n 1)
say "GUEST cycle=${cycle:-?} kernel=$(uname -r) os=$(. /etc/os-release && echo "$ID $VERSION_ID")"

finish() {
  say "CYCLE-DONE $1"
  sync
  systemctl poweroff --force
  exit 0
}

mkdir -p /data
mount -t ext4 "$data" /data || finish "mount-failed"
umount /data || finish "umount-failed"
e2fsck -fn "$data" > /run/fsck.log 2>&1
say "FSCK $?"
mount -t ext4 "$data" /data || finish "mount-failed"
say "MOUNT $(findmnt -no FSTYPE,OPTIONS /data)"
"$tools/vsift-crash-campaign" verify --root /data/root --acks "$acks" --prefix VERIFY \
  --out "$channel" > /dev/null
say "VERIFY-EXIT $?"
if [ "${mode:-}" != run ]; then
  umount /data
  finish "verified"
fi
mkdir -p /run/vsift-scratch
exec "$tools/vsift-crash-campaign" workload --root /data/root --scratch /run/vsift-scratch \
  --ack-out "$channel" --drain --known-acks "$acks" --first-seq $(( cycle * 1000000 )) \
  --seed "$seed" --recognizer-delay-ms 40
