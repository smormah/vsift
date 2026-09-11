# R0 qualification and resource profiles

Status: accepted starting targets under ADR 0005; public support begins only after P14.
Date: 2026-09-10.

## Qualification targets

| Profile | Target | Filesystem | Required qualification |
| --- | --- | --- | --- |
| Desktop Windows | Windows 11 25H2 x64, `x86_64-pc-windows-msvc` | Local NTFS | CLI, setup, sessions, native provider lifecycle, cancellation and crash recovery |
| Desktop macOS | macOS 15 arm64, `aarch64-apple-darwin` | Local APFS | Same desktop behavior; report available process/resource confinement |
| Strict worker Linux | Ubuntu 24.04 LTS x86-64, `x86_64-unknown-linux-gnu` | Local ext4 | Headless job/batch, cgroup-v2/container limits, durable recovery and shutdown |

Windows and macOS worker use may be qualified later, but R0 makes no strict-worker
claim for them. Linux desktop use and other distributions may work without an R0
guarantee. Network filesystems are explicitly unqualified. The release matrix records
exact OS updates, Rust target, FFmpeg/whisper.cpp build, filesystem and host controls.

## Execution profiles

| Setting | `desktop-safe` | `worker-strict` |
| --- | --- | --- |
| Heavy-stage admission | One weighted slot per state root | Finite host policy derived from measured CPU/RAM/GPU |
| Pending requests | At most 16 in a batch | Bounded by host policy; external queue owns backlog |
| Source maximum | 4 hours and 20 GiB | Explicit job limit no larger than host maximum |
| Decoded frame | 16 megapixels | Same default; may be lowered by host |
| Candidate page | 20 default, 100 maximum | Same schema and hard maximum |
| Frame burst | 12 default, 100 maximum | Same plus total pixel/byte cap |
| Result JSON | 1 MiB | 1 MiB; larger data through artifacts/pages |
| Provider diagnostic capture | 64 KiB for each stream | Same hard cap |
| Probe structured output | 4 MiB | Same hard cap |
| Session/root temporary storage | 10 GiB / 20 GiB with reserve | Explicit reservation plus host quota |
| Session expiry | 24-hour idle, seven-day absolute | Job/workspace policy, explicit and finite |
| Shutdown target | Five-second graceful then five-second forced cleanup | Host-configured deadline at least as strict |
| Durability | Ephemeral unless explicitly retained | Explicit durable workspace required |
| Network/filesystem isolation | Report effective controls | Required external container/cgroup policy; fail if requested controls absent |

These values are admission ceilings, not throughput promises. Provider threads count
against weighted CPU admission. Source staging, model size and decoded outputs count
against disk/memory budgets. P04/P07/P14 may tighten a default based on measurements;
raising a hard limit requires security and capacity review plus an ADR update.

## Speech profile

The initial candidate is a pinned CPU whisper.cpp runtime with the multilingual `base`
model. P07 measures word error rate, critical identifiers/numbers, latency and memory
against F08 and the wider corpus. It becomes the default only if the recorded gates
pass. Imported transcripts avoid the model entirely. GPU and larger model profiles
remain optional, explicit and independently qualified.

## Evidence required to claim support

- Fresh-machine installation without Rust, upgrade, rollback and uninstall.
- All deterministic PR checks plus platform process/filesystem conformance tests.
- Crash injection and source-preservation proof on the named filesystem.
- Resource overload, cancellation, descendant cleanup and handle-leak tests.
- End-to-end fixture corpus accuracy and named compact-agent evaluation.
- Release artifact provenance, inventory, notices and unresolved finding review.

Until those results exist, documentation must say “qualification target” rather than
“supported platform” or “production ready.”

P02 adds a PR qualification job for the strict-worker process boundary. It runs the
process contract inside a read-only, networkless Ubuntu 24.04 container with explicit
CPU, memory, swap and PID limits and no added capabilities. The test verifies that a
new process group remains inside the worker cgroup, induces observable CPU throttling,
reaches the PID ceiling and confirms that a bounded over-limit allocation cannot
complete successfully. That proves the supervisor's strict-mode attestation and
fail-closed split; it is not the full P11 worker or P14 release qualification.
