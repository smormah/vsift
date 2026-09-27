# P10 durable-publication qualification record (FS-01)

Status: measured 2026-09-27 on P10 PR 4 (branch `p10/durability-campaign`, from `main`
at `8af331b`). Design: [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)
section 7 (accepted, D-5: hosted `ubuntu-24.04` runners with KVM) under
[ADR 0010](../decisions/0010-storage-qualification-gate.md). Verification IDs: X-10,
S-07, SEC-24. Closes the P03 finding FS-01 for Ubuntu 24.04 on local ext4 only.

## Result in plain English

A durable session on Ubuntu 24.04 with a local ext4 filesystem keeps every generation
VSift acknowledged through a power loss at any flush, an OS crash at any moment and
write or flush errors. In each of two complete runs, one before and one after the
constant was set (together 22,078 simulated power losses, 640 killed virtual machines
and 324 injected I/O failures), no acknowledged operation was lost, no session was
left unreadable or unwritable, `e2fsck` found nothing, and every injected failure was
answered `STORAGE_IO` without an acknowledgement. The negative control, a build that
leaves out the synchronisations after the pointer rename, lost 54 of 80 acknowledged
operations in the same harness every time, so the harness sees loss when there is
loss. `QUALIFIED_UBUNTU_EXT4` is therefore set: a durable request (engine API only,
D-3) is honoured on that profile and still fails with `MISSING_CAPABILITY` everywhere
else. The campaign also found two defects, both fixed before the gating run: a storage
failure reading committed state was reported as `INTEGRITY_FAILURE`, and an ingest
reported the store's guarantee instead of its session's.

## What was qualified

- **Profile:** Ubuntu 24.04 (`/etc/os-release` `ID=ubuntu`, `VERSION_ID=24.04`) with the
  session root on an ext4 filesystem whose every mount keeps write barriers. The
  filesystems under test were made by Ubuntu 24.04's `mke2fs` 1.47 with its default
  features (`has_journal ext_attr resize_inode dir_index filetype extent 64bit flex_bg
  sparse_super large_file huge_file dir_nlink extra_isize metadata_csum`), 4 KiB blocks
  and 256-byte inodes, and mounted with the defaults (`rw,relatime`, `data=ordered`,
  barriers on, superblock error behaviour `continue`). Kernels exercised:
  6.17.0-1022-azure (the hosted runner, layers A and C) and 6.8.0-139-generic (the
  pinned cloud image's generic kernel, layer B).
- **Code:** the durable protocol of ADR 0020 section 2 (flushes, then synchronisation of
  `artifacts/`, `generations/` and the session directory; acknowledgement last;
  fsyncgate-safe retries) and the durable initialisation, through the public engine
  (`Engine::ingest` with `DurabilityRequirement::Durable`), the store's evidence and
  lifecycle commits, and checkpointed retranscription jobs.
- **Builds:** Rust 1.98.1. The runs before the constant was set used a `campaign`
  build (release optimisation with debug assertions, `--features campaign`), whose
  only differences were that it claimed the profile without the constant and could
  run the negative control; the confirmation run after the constant was set used the
  plain release build for layers A, B and C (the negative control always needs the
  `campaign` build).

## Method

Everything runs on GitHub-hosted `ubuntu-24.04` runners (4 vCPUs, 15 GiB, KVM) from
`.github/workflows/p10-durability-campaign.yml` (manual, and weekly on Sundays), with
the harness in `tools/p10-crash-campaign/`: one Rust binary
(`vsift-crash-campaign`: `workload`, `verify`, `replay`, `assess`) and the scripts
around it.

**Workload.** The workload opens durable sessions through the engine, commits frames
and their evidence records (stand-in media, the real P09 evidence use case), runs
checkpointed retranscription jobs (stand-in recognizer, the real P10 job use case,
three chunks per job) and renews sessions, mixing them at random from a recorded seed.
After every success it writes `ACK <seq> <time> <kind> <session> <generation>
<manifest-sha256> <artifact digests> <revision>` to its acknowledgement channel before
it starts the next operation (for layer A it also writes the dm-log-writes mark
`ack-<seq>`; for layer B the channel is a serial port it drains with `tcdrain`, so the
host holds every acknowledgement before the next operation begins).

**Verifier.** Given the acknowledgements made before a crash, `verify` checks the
session root two independent ways. On disk, from the documented layout alone: the
commit pointer names a manifest with the recorded digest, every generation links to
the digest of the one below down to generation 0, generation 0 is durable, and every
artifact the head lists and the source copy hash to their entries. Through the store:
the session opens, its evidence records decode, each acknowledged revision is found
and every job reads (a job whose owner died is reconciled). An acknowledgement is
**lost** when its session is missing or unreadable, the head is below it, its
generation's manifest changed, or something it committed is gone; anything else wrong
in any session, acknowledged or not, is **damage**.

### Layer A: power loss at every flush (dm-log-writes)

A fresh 1 GiB ext4 filesystem (`mkfs.ext4 -b 4096 -E lazy_itable_init=0,lazy_journal_init=0`)
on a `log-writes` device-mapper target records every write, flush and FUA write of
the workload's run in completion order. A write is logged only when a later flush or
FUA write reaches the device, so the log lists writes in the order they became
durable. The replay (our own reader of the kernel's version-1 log format, unit-tested)
rebuilds the device from its image at mkfs time up to **every flush and FUA entry**,
and before any entry; at each point it mounts a copy (ext4 replays its journal as after
a power loss), verifies every acknowledgement whose mark lies before the next point
(the device then held no more than this point's prefix), commits one renewal to each
of the four most recently acknowledged sessions (they must stay writable), unmounts
and runs `e2fsck -fn`, which must report a clean filesystem.

The **negative control** runs the same harness with a `campaign` build and
`VSIFT_CAMPAIGN_NEGATIVE_CONTROL=1`, which removes every synchronisation after the
pointer rename (the session directory's and the chain checkpoint's flush). It must
lose acknowledgements. The first control removed only the session-directory
synchronisation, as ADR 0020 section 7 proposed, and lost nothing: on ext4 any file
flush commits the whole running journal transaction, so the chain checkpoint's flush
made the pointer rename durable before the acknowledgement. The control was widened to
everything after the rename, which is what the directory synchronisation must
guarantee on its own.

### Layer B: OS crash (QEMU SIGKILL)

The pinned Ubuntu 24.04 cloud image (release 20260911,
`ubuntu-24.04-server-cloudimg-amd64.img`, SHA-256
`612b2c0cc1bc413a6cb8c38fd611794caf0f2b436c50013d8b3794db12ad7354`, 625,256,960 bytes,
downloaded over HTTPS with a size bound and verified before use) boots under KVM
(QEMU 8.2.2, 2 vCPUs, 2 GiB) with a 1 GiB ext4 data disk attached `cache=none,aio=native`
(host `O_DIRECT`: a write the guest's flush completed is in the host's kernel, a write
QEMU had not submitted is lost). One preparation boot with cloud-init installs the
campaign service; every cycle then boots a fresh overlay of that image, recovers the
data disk's journal (mount), runs `e2fsck -fn`, verifies every acknowledgement the
host has captured so far (they reach the guest on a read-only disk), continues the
workload on the same disk (checkpointed retranscription at 40 ms per chunk) and is
killed with `SIGKILL` 0.2 to 12 s after the workload starts. A final boot only
verifies. Four shards of 80 kills run in parallel on separate runners.

### Layer C: write and flush errors (dm-flakey)

dm-dust is not available on the hosted kernel, so a `linear` table is swapped at a
random moment (50 to 1,550 ms into a round) for `flakey ... 0 0 1 1 error_writes`:
every write and flush fails with `EIO`. The first failure is usually a flush (a data
write-back or a journal commit), after which ext4 aborts its journal and later calls
fail with `EIO` or `EROFS`. Each round verifies the acknowledgements so far, runs the
workload until three operations in a row fail (from the confirmation run on, every
second round leaves out retranscription jobs: their many flushes otherwise make them
the operation a randomly timed error almost always hits first), and `assess`es the
round: every failure
must be `STORAGE_IO` (the engine's public mapping), none may happen before the swap
began, and no operation that started after the failing table was live may be
acknowledged. The table is then restored, the journal recovered and `e2fsck -fn`
must be clean.

## Results

All runs are on GitHub-hosted `ubuntu-24.04` runners (4 vCPUs, 15 GiB RAM), on
2026-09-27. Acceptance (ADR 0020 section 7, checked by `scripts/acceptance.sh` in the
workflow's last job): layer A at least 2,000 replay points with no lost
acknowledgement, no damage and a clean `e2fsck`; the negative control at least one
lost acknowledgement; layer B at least 300 kills with every cycle recovered, clean and
verified; layer C every injected failure `STORAGE_IO` and none acknowledged.

**Gating run (before the constant was set):**
[36340043451](https://github.com/smormah/vsift/actions/runs/36340043451), commit
`7e8b141`, `campaign` build. All acceptance criteria met.

| Layer | Result | Time |
| --- | --- | --- |
| A, power loss | 400 operations, 400 acknowledgements; 51,290 log entries; **11,037 replay points**, each mounted, verified, probed with a new commit and checked: 0 lost acknowledgements, 0 damaged points, 0 `e2fsck` findings, 0 mount failures | workload 59 s, replay 2,261 s |
| A, negative control | 80 operations; 2,200 replay points: **54 of 80 acknowledgements lost** at 596 points (head behind the acknowledged generation, or a session left unreadable by an unflushed checkpoint at 550 points); `e2fsck` clean | 233 s |
| B, OS crash | **320 kills** in 4 shards of 80 (319 inside an operation; kill 0.3 to 12.0 s after the workload started, median 5.8 s); 324 boots each recovered, `e2fsck`-clean and verified; 10,653 acknowledgements, 0 lost, 0 damaged, 0 failed operations; each final boot verified 139 to 190 sessions (2,489 to 3,029 generations, 689 to 826 jobs) | 52 to 61 min a shard |
| C, write errors | 60 rounds, all injected: **180 injected failures, 180 `STORAGE_IO`**, 0 before the swap, 0 acknowledged after it; 403 acknowledgements carried through every later round; `e2fsck` clean after every recovery and at the end | 24 min |

**Confirmation run (after the constant was set, plain release build):**
[36347502530](https://github.com/smormah/vsift/actions/runs/36347502530), commit
`2b8455e` (the constant set; the code of this pull request), plain release build for
layers A, B and C. All acceptance criteria met.

| Layer | Result | Time |
| --- | --- | --- |
| A, power loss | 400 operations and acknowledgements; 51,020 log entries (213 MB of log); **11,041 replay points**: 0 lost, 0 damaged, 0 `e2fsck` findings, 0 mount failures | workload 60 s, replay 2,610 s |
| A, negative control | 2,180 replay points: **54 of 80 acknowledgements lost** at 592 points (546 with an unreadable session) | 302 s |
| B, OS crash | **320 kills** (317 inside an operation), 324 recovered, clean and verified boots; 10,599 acknowledgements, 0 lost, 0 damaged, 0 failed operations; each final boot verified 146 to 178 sessions (2,592 to 2,967 generations, 716 to 795 jobs) | 55 to 58 min a shard |
| C, write errors | 60 rounds, alternating the full mix and a job-free mix: the failing table went live during operations in 48 rounds (in 12 job-free rounds the workload finished its 400 operations first); **144 injected failures, 144 `STORAGE_IO`**, 0 before the swap, 0 acknowledged after it; the first failure of a round hit a retranscription 30 times, evidence 15, an ingest 2 and a renewal once; 9,639 acknowledgements carried through; `e2fsck` clean throughout | 19 min |

**Earlier runs and what they found.**

- [36337576649](https://github.com/smormah/vsift/actions/runs/36337576649) (`c78d588`,
  small sizes): the first negative control, which removed only the session
  directory's synchronisation, lost nothing in 2,364 replay points: too weak for ext4
  (see ADR 0020 PR 4 notes). It was widened to every synchronisation after the
  pointer rename. The layer B script also left the killed
  QEMU running (fixed).
- [36338196373](https://github.com/smormah/vsift/actions/runs/36338196373) (`d0f5926`,
  small sizes): the widened control lost 54 of 80 acknowledgements; every other layer
  passed at small sizes.
- [36338992442](https://github.com/smormah/vsift/actions/runs/36338992442) (`dd923f7`,
  full sizes): layers A (11,037 points), B (320 kills, 10,561 acknowledgements) and the
  negative control passed; layer C failed in round 14: after the injected errors an
  evidence call answered `INTEGRITY_FAILURE`. ext4 had shut itself down, so reads of
  committed state failed with `EIO`, and the store reported every failure to read
  committed state as damage. Fixed in `7e8b141` (storage failures there are now
  `STORAGE_IO`), which the gating run then qualified.

## What this does not cover (residuals)

- **Storage that ignores flushes.** The campaign proves VSift issues the right flushes
  in the right order and that ext4 on a well-behaved block device keeps them. A drive,
  controller, RAID layer or hypervisor that acknowledges a flush it did not make
  durable (a volatile write cache without power-loss protection, `cache=unsafe`, a
  virtual disk that drops flushes) breaks every filesystem's guarantees, VSift's
  included. That is the operator's responsibility ([L-056](known-limits.md#l-056)).
- **Losing the disk or the host (X-10).** Local durability survives a worker or OS
  crash and power loss of the machine; it does not survive losing the disk, the
  filesystem or the host. A caller that needs that must keep its evidence on its own
  replicated storage (for example by retaining bundles onto replicated storage and
  copying them off the host); VSift offers no replication
  ([L-057](known-limits.md#l-057)).
- **Other filesystems and mounts.** Network filesystems (NFS, SMB, FUSE), overlay and
  tmpfs roots, other local filesystems (XFS, Btrfs, ZFS) and ext4 mounted with
  `nobarrier` or `barrier=0` are not qualified; a durable request there fails with
  `MISSING_CAPABILITY` before anything is changed. `data=journal` and `data=writeback`
  mounts were not exercised (the qualified mounts use the default `data=ordered`).
- **Other operating systems.** Windows/NTFS and macOS/APFS durability stay
  unqualified (ADR 0010); every durable request there fails closed
  ([L-008](known-limits.md#l-008)).
- **Kernels and distributions.** The gate identifies the distribution from
  `os-release`, not the kernel: an Ubuntu 24.04 user space in a container on another
  kernel passes it. The weekly run re-qualifies the hosted kernel and the pinned image
  ([L-058](known-limits.md#l-058)).
- **Retained bundles** keep process-crash consistency even for a durable session: the
  export does not run the durable protocol.
- **Crash during root provisioning.** A crash while a brand-new session root is
  created is outside the campaign (every run's root was provisioned before its first
  acknowledgement); nothing is acknowledged before the root is complete.

## Reproducing

```console
# Harness unit tests (any platform)
cargo test --locked -p vsift-crash-campaign

# The campaign (GitHub Actions, workflow_dispatch; sizes are inputs)
gh workflow run p10-durability-campaign.yml
```

On a disposable Ubuntu 24.04 machine with KVM, the scripts run directly (as root for
layers A and C): `tools/p10-crash-campaign/scripts/layer-a.sh <binary> <work> <ops>
<seed> positive|negative <out>`, `layer-b.sh <binary> <image> <work> <kills> <seed>
<out>`, `layer-c.sh <binary> <work> <rounds> <seed> <out>` and `acceptance.sh
<evidence>`. They create loop devices, device-mapper targets and virtual machines;
never run them on a machine whose disks matter.
