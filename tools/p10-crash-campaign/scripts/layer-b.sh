#!/usr/bin/env bash
# Layer B of the P10 crash campaign (ADR 0020 section 7): OS crash.
#
# An Ubuntu 24.04 guest (the pinned cloud image, its generic kernel) runs the
# durable workload on an ext4 data disk attached with cache=none, and the
# host kills QEMU with SIGKILL at a random moment: the guest kernel, its page
# cache and every I/O QEMU had not yet submitted vanish at once. The next
# boot recovers the journal, runs `e2fsck -fn`, verifies every
# acknowledgement the host received before the kill, and continues the
# workload on the same disk. After the last kill a final boot only verifies.
#
# Usage: layer-b.sh <binary> <cloud image> <work dir> <kills> <seed> <out dir>
# Needs /dev/kvm, qemu-system-x86, qemu-utils, cloud-image-utils, e2fsprogs.
set -euo pipefail

binary=$1
image=$2
work=$3
kills=$4
seed=$5
out=$6
here=$(cd "$(dirname "$0")" && pwd)
guest=$here/../guest

case "$kills" in '' | *[!0-9]*) echo "kills must be a number" >&2; exit 2 ;; esac
case "$seed" in '' | *[!0-9]*) echo "seed must be a number" >&2; exit 2 ;; esac

mkdir -p "$work" "$out"
tools_dir=$work/tools
rm -rf "$tools_dir"
mkdir -p "$tools_dir"
install -m 0755 "$binary" "$tools_dir/vsift-crash-campaign"
install -m 0755 "$guest/guest-cycle.sh" "$tools_dir/guest-cycle.sh"
install -m 0644 "$guest/vsift-campaign.service" "$tools_dir/vsift-campaign.service"
rm -f "$work/tools.img"
truncate -s 128M "$work/tools.img"
mkfs.ext4 -q -F -L vsift-tools -d "$tools_dir" "$work/tools.img"

qemu() {
  # $1: OS overlay, $2: console log, $3: campaign log, $4: acknowledgement disk,
  # further arguments are appended. Run in the background, the function's
  # subshell becomes QEMU itself (exec), so its PID is the one to kill.
  local os=$1 console=$2 campaign=$3 acks=$4
  shift 4
  exec qemu-system-x86_64 -enable-kvm -cpu host -smp 2 -m 2048 \
    -display none -monitor none -no-reboot -nic none \
    -serial "file:$console" -serial "file:$campaign" \
    -drive "if=none,id=os,file=$os,format=qcow2,cache=unsafe" \
    -device virtio-blk-pci,drive=os,bootindex=1 \
    -drive "if=none,id=data,file=$work/data.img,format=raw,cache=none,aio=native" \
    -device virtio-blk-pci,drive=data,serial=vsift-data \
    -drive "if=none,id=acks,file=$acks,format=raw,readonly=on" \
    -device virtio-blk-pci,drive=acks,serial=vsift-acks \
    -drive "if=none,id=tools,file=$work/tools.img,format=raw,readonly=on" \
    -device virtio-blk-pci,drive=tools,serial=vsift-tools \
    "$@"
}

# The data disk under test, and an empty acknowledgement disk for the
# preparation boot.
rm -f "$work/data.img"
truncate -s 3G "$work/data.img"
mkfs.ext4 -q -F -b 4096 -E lazy_itable_init=0,lazy_journal_init=0 "$work/data.img"
dumpe2fs -h "$work/data.img" > "$out/dumpe2fs.txt" 2>/dev/null
truncate -s 0 "$work/empty-acks.img"
truncate -s 1M "$work/empty-acks.img"

# One preparation boot: cloud-init installs the service, then powers off.
rm -f "$work/base.qcow2"
qemu-img create -q -f qcow2 -F qcow2 -b "$image" "$work/base.qcow2" 10G
printf 'instance-id: vsift-campaign\nlocal-hostname: vsift-campaign\n' > "$work/meta-data"
cloud-localds "$work/seed.img" "$guest/user-data" "$work/meta-data"
prepared=$(date +%s)
qemu "$work/base.qcow2" "$out/prepare-console.log" "$out/prepare-campaign.log" \
  "$work/empty-acks.img" \
  -drive "if=none,id=seed,file=$work/seed.img,format=raw,readonly=on" \
  -device virtio-blk-pci,drive=seed &
vm=$!
if ! timeout 900 tail --pid="$vm" -f /dev/null; then
  kill -9 "$vm" 2>/dev/null || true
fi
wait "$vm" 2>/dev/null || true
if ! grep -aq VSIFT-PREP-DONE "$out/prepare-console.log"; then
  echo "the preparation boot did not finish" >&2
  tail -n 50 "$out/prepare-console.log" >&2
  exit 1
fi
echo "prepare_seconds=$(( $(date +%s) - prepared ))" > "$out/environment.txt"

: > "$work/acks.txt"
RANDOM=$seed
cycle_failures=0
kills_during_operation=0
boot_seconds_total=0
started=$(date +%s)
: > "$out/cycles.log"

for cycle in $(seq 1 $(( kills + 1 ))); do
  mode=run
  if [ "$cycle" = $(( kills + 1 )) ]; then mode=verify; fi
  directory=$work/cycle
  rm -rf "$directory"
  mkdir -p "$directory"
  printf 'CYCLE %d SEED %d MODE %s\n' "$cycle" $(( seed * 1000 + cycle )) "$mode" > "$directory/acks.img"
  cat "$work/acks.txt" >> "$directory/acks.img"
  truncate -s 64M "$directory/acks.img"
  qemu-img create -q -f qcow2 -F qcow2 -b "$work/base.qcow2" "$directory/os.qcow2"
  booted=$(date +%s)
  qemu "$directory/os.qcow2" "$directory/console.log" "$directory/campaign.log" "$directory/acks.img" &
  vm=$!
  state=timeout
  for _ in $(seq 1 3000); do
    if grep -aq '^WORKLOAD-START' "$directory/campaign.log" 2>/dev/null; then state=running; break; fi
    if grep -aq '^CYCLE-DONE' "$directory/campaign.log" 2>/dev/null; then state=done; break; fi
    if ! kill -0 "$vm" 2>/dev/null; then state=exited; break; fi
    sleep 0.1
  done
  boot=$(( $(date +%s) - booted ))
  boot_seconds_total=$(( boot_seconds_total + boot ))
  delay=0
  if [ "$state" = running ]; then
    # Kill 0.2-12 s into the workload: nearly always inside an operation.
    delay=$(( 200 + (RANDOM * 32768 + RANDOM) % 11800 ))
    sleep "$(printf '%d.%03d' $(( delay / 1000 )) $(( delay % 1000 )))"
    kill -9 "$vm"
  elif [ "$state" = done ]; then
    timeout 120 tail --pid="$vm" -f /dev/null || kill -9 "$vm"
  else
    kill -9 "$vm" 2>/dev/null || true
  fi
  wait "$vm" 2>/dev/null || true
  fsck=$(sed -n 's/^FSCK \([0-9]*\).*/\1/p' "$directory/campaign.log" | tr -d '\r' | head -n 1)
  verify=$(grep -a '^VERIFY \(OK\|FAIL\)' "$directory/campaign.log" | tr -d '\r' | head -n 1 || true)
  # Only complete lines count: a line the kill cut short was never
  # acknowledged.
  lines=$directory/campaign.log
  if [ -s "$lines" ] && [ -n "$(tail -c 1 "$lines")" ]; then
    head -n -1 "$lines" > "$directory/complete.log"
  else
    cp "$lines" "$directory/complete.log"
  fi
  acked=$(grep -ac '^ACK ' "$directory/complete.log" || true)
  grep -a '^ACK ' "$directory/complete.log" | tr -d '\r' >> "$work/acks.txt" || true
  last=$(grep -a '^\(START\|ACK\|FAIL\) ' "$directory/complete.log" | tail -n 1 | cut -d' ' -f1 || true)
  if [ "$last" = START ]; then kills_during_operation=$(( kills_during_operation + 1 )); fi
  failed_ops=$(grep -ac '^FAIL ' "$directory/complete.log" || true)
  ok=yes
  if [ "$fsck" != 0 ] || [[ "$verify" != "VERIFY OK"* ]] || [ "$failed_ops" != 0 ]; then ok=no; fi
  if [ "$mode" = run ] && [ "$state" != running ]; then ok=no; fi
  if [ "$mode" = verify ] && [ "$state" != done ]; then ok=no; fi
  echo "cycle=$cycle mode=$mode state=$state boot_s=$boot kill_ms=$delay fsck=${fsck:-none} acked=$acked failed=$failed_ops last=${last:-none} ok=$ok ${verify:-no-verify}" \
    | tee -a "$out/cycles.log"
  if [ "$ok" != yes ]; then
    cycle_failures=$(( cycle_failures + 1 ))
    mkdir -p "$out/failed-cycle-$cycle"
    cp "$directory/console.log" "$directory/campaign.log" "$out/failed-cycle-$cycle/" || true
    grep -a '^VERIFY' "$directory/campaign.log" | head -n 50 >&2 || true
  fi
  if [ "$cycle" = 1 ]; then
    grep -a '^\(GUEST\|MOUNT\) ' "$directory/campaign.log" | tr -d '\r' >> "$out/environment.txt" || true
  fi
done

cp "$work/acks.txt" "$out/acks.log"
echo "RESULT kills=$kills cycles=$(( kills + 1 )) cycle_failures=$cycle_failures \
kills_during_operation=$kills_during_operation acks=$(wc -l < "$work/acks.txt") \
boot_seconds_total=$boot_seconds_total seconds=$(( $(date +%s) - started ))" | tee "$out/result.txt"
[ "$cycle_failures" = 0 ]
