# Support matrix and resource profiles

Status: **matrix updated 2026-10-04 (P14 PR 9a; the macOS wording and the RQ-05 rule decided the same day, PR 9c)**; resource profiles from 2026-09-10, extended by
later packets. The matrix is [ADR 0024](../decisions/0024-r0-qualification-and-release-candidate.md)
decision F (confirmed by the maintainer on 2026-10-02) read against the evidence recorded in the
[evidence ledger](p14-evidence-ledger.json) and the [P14 plan](p14-qualification.md). Public
support begins only after P14 ([ADR 0005](../decisions/0005-r0-scope-and-qualification-profiles.md)):
until then every machine below is an **R0 target**, and the public documents say so.

## 1. What a cell means

A cell earns the word "supported" when, **for the R0 release's own bytes**, all four rules hold:

1. **Clean installs.** The packages install from the real npm registry on a scrubbed hosted
   image with npm, pnpm, Yarn and Bun (evidence item RQ-01).
2. **The archive.** The extracted native archive runs (RQ-02).
3. **The journeys.** The installed binary completes the supplied-transcript journey and the
   local-speech journey with that system's documented tools (RQ-05).
4. **The guide was walked.** The install guide's install, upgrade and uninstall steps were
   walked (RQ-04).

Four further rules come with it:

- **A named agent client counts only on the system it was trialled on.** A model that was not
  trialled is untested, whatever it can do.
- **Anything short of the rules is a qualification target.** A machine outside the table is
  unsupported and fails clearly: the npm launcher exits 127 and names the machines it is built for.
- **The word "supported" is a statement about evidence, not a service promise.** Nobody is on call,
  and every number was measured on a synthetic corpus and on hosted hardware unless the text says
  otherwise.
- **Evidence belongs to a version and a commit.** A run of 0.1.0 shows what 0.1.0 did; the release
  candidate and the release each repeat it on their own bytes, or carry it only where nothing in its
  scope changed (the ledger's staleness rule, [`delivery-governance.md`](delivery-governance.md)).

The exact sentences each cell may use, and the evidence items that must be `passed` before it may
use them, are in the [public-claims registry](public-claims.json): statements CL-201 to CL-209 for the
release rung, none of them in use today. The Governance job refuses a controlled word outside a
registered statement, a statement above the current rung and a statement whose evidence is not
`passed` (`cargo run --locked -p vsift-governance -- public-claims`). That proves recorded evidence
and absent banned words, not that a sentence is true ([L-101](known-limits.md#l-101)).

## 2. The matrix today

Results are for the **published 0.1.0** on hosted runners and are dated in the
[P14 plan](p14-qualification.md) (sections 15, 17 and 18). They are the "before" picture: nothing
below is a result for the release candidate.

| Machine | Install paths | Tools | Shown for 0.1.0 | Still to show for the release's bytes |
| --- | --- | --- | --- | --- |
| **Windows 11 x64** (target: 25H2, `x86_64-pc-windows-msvc`, local NTFS) | npm, pnpm, Yarn, Bun; the archive | Your own FFmpeg, FFprobe and whisper.cpp; no managed install | Rules 1, 2 and 4 passed (RQ-01, RQ-02, RQ-04) and both journeys of rule 3 ran, on a hosted **Windows Server 2025** image with the repository's pinned tool builds. The P08, P09 and P11 numbers and the Claude Code trials (on a source-built binary) come from the maintainer's Windows 11 machine | The same on the candidate and the release; RQ-05 as a whole (see below); the clean-install agent round with Claude Code (RQ-15); the Smart App Control try-out (RQ-17) |
| **Ubuntu 24.04 x64** (`x86_64-unknown-linux-gnu`, glibc 2.35 or later, OpenSSL 3; local ext4 for durable sessions) | The same | Managed (`setup install`, or offline with `--artifact-dir`) or your own | Rules 1, 2 and 4 passed; the offline install with the real reviewed files (RQ-03) and the managed install from the publishers (RQ-06) passed; both journeys ran with the managed tools | RQ-05 as a whole; the two fault campaigns on the candidate (RQ-11); the clean-install agent round with Codex in a Linux container (RQ-15) |
| **macOS 15 on Apple silicon** (`aarch64-apple-darwin`, local APFS) | The same | Your own only; no managed install | Rules 1, 2 and 4 passed, and both journeys ran, on a hosted macOS 15 image with **Homebrew's** FFmpeg 9.0.1 and whisper-cpp 1.9.2, which VSift does not review ([L-114](known-limits.md#l-114)) | RQ-05 as a whole. No agent trial; the Gatekeeper try-out is untried (no Mac is available to the project) |
| **Worker host, Ubuntu 24.04 x64** (local ext4 with write barriers) | The same | The reviewed tools | The load ladder, a 100-request batch and a mixed soak (RQ-09) and the runbook walked step by step (RQ-12) passed for 0.1.0, in the hardened container and under systemd | A **qualification target**, not claimed: no evidence shows that the strict profile contains an exploited decoder or provider (RQ-14 waived, [L-068](known-limits.md#l-068)) |
| Another Linux on x64 | The same | Your own | Nothing | Not a target: the binary may run, and nobody has tried |
| Linux on Arm, Intel Macs, Windows on Arm, Windows 10, Alpine and other musl Linux | None | None | Nothing | Unsupported: the launcher says so and exits 127 |

**RQ-05, in one paragraph.** Its pass rule is per system (the maintainer's decision of 2026-10-04): on
each system every stage that can run there must pass; the P07 speech-recognition gates must hold on each
operating system; and P11's durable-workspace stage, which only the durable profile's one host (Ubuntu 24.04 with local
ext4 and write barriers) can run, must pass there and, everywhere else, must show the refusal (the durable
profile refused with `MISSING_CAPABILITY` and nothing created). For 0.1.0, 53 stages passed on all three
systems and that stage was `blocked` on all three: on Windows and macOS because the durable profile exists
only on Ubuntu, with the refusal it must give checked and holding; on the hosted Ubuntu runner because its
disk is mounted without write barriers ([#258](https://github.com/smormah/vsift/issues/258),
[L-113](known-limits.md#l-113)). What covers the Ubuntu case is recorded, not assumed: RQ-09 and RQ-12 ran
the published 0.1.0 with durable workspaces on an ext4 volume with write barriers (durable publication, a
100-request batch, 1,000 mixed requests with kills and redelivery, the runbook walked), but they do not re-run
that one stage's script, so they cover the durable path, not that stage check for check. The item stays
`running` for one other reason: the speech-recognition gates on Ubuntu and Windows are recorded only from the
weekly `P07 local ASR` workflow on source at other commits, not at the 0.1.0 commit, and the ledger counts
only the journeys' own runs as evidence for 0.1.0. That is a gap in what was recorded, not a failure. The
rule is no longer unsatisfiable. Even so, no cell may use its word until RQ-05 and the other items are
`passed` for the release candidate's own bytes: every item is stale for it.

**What the hosted evidence is not.** Hosted runners are shared virtual machines that carry a Rust
toolchain and other developer tools, so a "clean install" job proves there is no hidden dependency on them,
not that a person's machine works ([L-112](known-limits.md#l-112)). Windows 11 is not a hosted image. No macOS
machine has been used by a person. Windows Smart App Control and macOS Gatekeeper have never been seen
blocking or warning on a VSift file ([L-098](known-limits.md#l-098)); the install guide's section 4 is written from
the vendors' documentation. All evidence is for a synthetic corpus: real recordings are untried
([L-020](known-limits.md#l-020), [L-022](known-limits.md#l-022), [L-028](known-limits.md#l-028)).

## 3. The words each cell may use, by rung

The registry's three rungs ([ADR 0024](../decisions/0024-r0-qualification-and-release-candidate.md)
decision G) decide what a public document may say. The rung is `now` today; the maintainer moves it.

| Machine | `now` (0.1.0 pre-release, today) | `candidate` (0.2.0-rc.N) | `after_p14` (0.2.0 published, ledger complete) |
| --- | --- | --- | --- |
| Windows 11 x64 | R0 target | No cell gets the word; the document says "release candidate under qualification" | CL-201, which needs RQ-01, RQ-02, RQ-04, RQ-05, RQ-15 and RQ-17 passed; carries "Smart App Control untried" if the try-out was waived |
| Ubuntu 24.04 x64 | R0 target | The same | CL-202, which needs RQ-01 to RQ-06, RQ-11 and RQ-15; durable sessions are a separate statement, CL-207 (RQ-11) |
| macOS 15 on Apple silicon | R0 target | The same | CL-203, which needs RQ-01, RQ-02, RQ-04 and RQ-05; stays a qualification target if the hosted run does not pass. **The wording was accepted by the maintainer on 2026-10-04** as proposed (ADR 0024, PR 9a note) |
| Worker host | Qualification target | Qualification target | Not claimed (CL-208 stays unused: RQ-14 is waived, not passed) |

**Agent clients.** The skill was trialled with **Claude Code on Windows 11** (Claude Opus 5.5 in the review
tier, Claude Sonnet 5.5 in the compact tier) and **Codex in a Linux container** (GPT-6-Astra, GPT-6-Sol): the
results and their limits are in [the skill guide](../agents/skill.md). Codex on Windows is not supported
([#204](https://github.com/smormah/vsift/issues/204), [L-076](known-limits.md#l-076)); Claude Haiku 4.5 and
GPT-6-Luna are below the line ([L-082](known-limits.md#l-082), [L-084](known-limits.md#l-084)); the skill has
not been trialled on macOS; any other client or model is untested. The statements CL-204 and CL-205 name the
two clients and need RQ-15, the clean-install round. **That round ran on both release candidates (batch 2, 34
runs each; on `0.2.0-rc.2` on 2026-10-07), and RQ-15 is `waived`, not `passed`** (the maintainer's decision of
2026-10-07; [P14 plan](p14-qualification.md) section 27). GPT-6-Astra met the review tier's gates, and Claude
Sonnet 5.5 and GPT-6-Sol the compact tier's. **Claude Opus 5.5 did not meet two review-tier gates** (4 of 6
mechanically; 1 of 3 on the blurred banner) **and is excluded from what R0 claims**
([L-139](known-limits.md#l-139)). So for Claude Code a later document may name Claude Sonnet 5.5 only, CL-204
may not be used as worded, and no statement that needs RQ-15 (CL-201, CL-202, CL-204, CL-205) can be used
until the maintainer settles how a waived item backs it (plan section 27.4).

**Runtimes.** Installing through a package manager needs Node.js 22 or later, or Bun 1.2 or later, to run the
launcher; the native archive needs neither.

## 4. Not claimed, whatever the cell

No document may say, until the evidence exists: readiness for production use on real recordings; that the
worker's strict profile contains an exploited decoder or a malicious provider (R0 makes no such claim:
decision E, option 4, 2026-10-03); isolation between tenants; trust in the publisher of an unsigned
executable; durability beyond Ubuntu 24.04 with local ext4, or after the loss of a disk or a host;
managed installation beyond Ubuntu 24.04 x64; Codex on Windows; any model or client that was not trialled; a
citation validity of 100% for the compact tier. The registry's banned phrases (BAN-01 to BAN-09) hold these
mechanically in every scanned document.

## 5. Targets and filesystems

Windows and macOS worker use may be shown later; R0 makes no worker claim for them, and a durable
workspace is refused there with `MISSING_CAPABILITY`. Linux desktop use and other distributions may work
without any R0 evidence. Network filesystems are explicitly outside every claim. The release matrix records
exact operating-system updates, Rust target, FFmpeg and whisper.cpp builds, filesystem and host controls for
each run.

## Execution profiles

| Setting | `desktop-safe` | `worker-strict` |
| --- | --- | --- |
| Heavy-stage admission | 4 weight units per desktop root: a recognition takes its threads (at most 4 here), a visual window 2, a copy or evidence extraction 1 | The workspace's `--admission-slots` (1-64), set once at `session init-workspace` from measured CPU/RAM/GPU; a recognition takes up to 8 threads within it |
| Pending requests | At most 16 in a batch | `job batch`: a file of at most 1,000 lines, at most `--concurrency` (1-16, never above the capacity) running, the next line read only when one ends; the external queue owns the backlog |
| Source maximum | 4 hours and 20 GiB | Explicit job limit no larger than host maximum |
| Decoded frame | 16 megapixels | Same default; may be lowered by host |
| Candidate page | 20 default, 100 maximum | Same schema and hard maximum |
| Frame burst | 12 default, 100 maximum | Same plus total pixel/byte cap |
| Result JSON | 1 MiB | 1 MiB; larger data through artifacts/pages |
| Provider diagnostic capture | 64 KiB for each stream | Same hard cap |
| Probe structured output | 4 MiB | Same hard cap |
| Session/root temporary storage | 10 GiB / 20 GiB with reserve | Explicit reservation plus host quota |
| Session expiry | 24-hour idle, seven-day absolute | The workspace's retention (default 168 hours, 1 to 720) after opening or renewal, at most 720 hours in all |
| Shutdown target | Five-second graceful then five-second forced cleanup | `job run` and `job batch`: the first signal stops admitting and the next step and, after `--drain-timeout-ms` (default 0, at most 300 s), cancels the running ones at their next boundary; the same five-second provider budgets; a second signal escalates. A supervisor's stop timeout must cover the drain plus 10 s (runbook) |
| Worker request | Not applicable | One request of at most 64 KiB and 8 steps per `job run`; `deadline_ms` up to one day (per delivery); a step waits at most `--admission-wait-ms` (default and maximum 60 s) for admission; at most 16 `candidates` calls per step; at most 4,096 request records of at most 192 KiB per workspace |
| Durability | Ephemeral unless explicitly retained | A workspace created with `--durability durable` (Ubuntu 24.04 / ext4 only); its every session is `os_crash_durable` |
| Network/filesystem isolation | Report effective controls | `--host-isolation strict-linux`: attested cgroup v2 CPU, memory and PID limits, read-only root, loopback only, else `ISOLATION_UNAVAILABLE` before any work; limits reported as the host cgroup's. SEC-T01 for P11: non-adversarial evidence accepted by the maintainer (attestation and the CI container job's controls); adversarial containment evidence deferred as technical debt (L-068) |
| Free-space reserve | Not checked | 1 GiB beyond each source copy, checked on Unix; not checked on Windows |

Visual analysis (P08, `candidates`) in both profiles: 60 s windows, at most 30 per
call; per window at most 122 decoded 128x72 grey frames (about 1.1 MiB), 256 KiB of
FFmpeg diagnostics, two decoder threads, a 64 MiB decoder allocation cap and a 120 s
deadline, each holding two admission units (one before P11 PR 2); at most 32 candidates per window; a
session holds at most 64 visual-index records of at most 8 MiB (the four-hour source
bound at full budget is a 2.2 MB record). Measured on Windows 11 (Xeon E5-2698 v4,
FFmpeg 9.0): about 30 media seconds per second of analysis on 1440x900 video, and a
p95 warm page of about 100 ms for the largest index in an optimised build.

Evidence navigation (P09, ADR 0019) in both profiles: one call extracts at most 100
frames, 200 megapixels decoded and 256 MiB of PNG (or what the session's 10 GiB has
left), in provider runs of at most 8 frames, 64 MiB and 30 s, and runs for at most
120 s; a frame listing covers at most 60 s and 1,200 frames. A burst defaults to 12 and
allows 100 frames over at most 60 s; neighbours are 1 to 20 per side (default 1).
Audio clips are WAV, 16 kHz mono signed 16-bit, at most 30 s (about 0.9 MiB,
`audio_wav` at most 1 MiB). A session keeps at most 512 artifacts, a 128 KiB manifest
and 10 GiB, with a sub-budget of 384 evidence artifacts (images, clips and evidence
records of at most 256 KiB; ADR 0020 D-2 raised these from 256, 64 KiB and 160 in
P10 PR 2); a call without room for its record and one file is
`RESOURCE_LIMIT` before any provider runs, and one that runs out after extracting
something is `partial` (`session_evidence_budget`). Measured on Windows 11 (Xeon
E5-2698 v4, FFmpeg 9.0, release build, [qualification record](p09-evidence-navigation.md)):
a reused request takes about 120-200 ms through the binary; a cold 1280x720 frame about
1.5-2 s; on a 1.17 GB 1080p clip of about 39 Mbit/s a cold frame p95 4.1 s, a 12-frame
burst over 60 s 17.1 s, and the first call's full hash of the copy 10.9 s. Until P10
PR 1 every read validated the whole manifest chain, so warm calls grew by about 3.7 ms
per session generation (about 1 s at 256; issue #164); reads now stop at the writer's
chain checkpoint and warm reuse stays about 150 ms at 1,024 generations. Raising the
artifact cap to 512 (P10 PR 2) grows the manifest a read hashes to about 100 KiB at
most; measured with the evidence budget full (384 evidence artifacts, about 75 KiB per
manifest; Windows 11, release build, 20 warm reused `frame get` calls per point), p95
was 177 / 164 / 166 / 177 ms at 2 / 64 / 256 / 1,024 generations (slope 0.000 ms per
generation), against 144 / 163 / 144 / 138 ms for a session holding one frame
(`s11_warm_reuse_with_a_full_evidence_budget`).

Recoverable jobs (P10 PR 2, ADR 0020) in both profiles: a session keeps at most 64 jobs
(at the bound the oldest ended job no caller's operation id pins is pruned; if every
one is pinned a new job is `RESOURCE_LIMIT`), a job at most 16 attempts and 8
caller-supplied operation ids, a session at most 256 operation bindings, and a job at
most 1,024 chunk checkpoints of at most 256 KiB each (a chunk whose checkpoint would be
larger is simply not checkpointed). Job records and bindings are at most 64 KiB.
`BUSY` from admission, the writer or a moved generation is retried at most twice
with full-jitter backoff (200 ms doubling to 2 s); a live job answers other requests
with `BUSY` and a 2 s retry hint. Checkpoints are private files in the session and
count against no artifact budget; they are removed when the job commits, fails or is
cancelled, and with the session.

These values are admission ceilings, not throughput promises. Provider threads count
against weighted CPU admission. Source staging, model size and decoded outputs count
against disk/memory budgets. P04/P07/P14 may tighten a default based on measurements;
raising a hard limit requires security and capacity review plus an ADR update.

## Speech profile

The initial candidate is a pinned CPU whisper.cpp runtime with the multilingual `base`
model. P07 measures word error rate, critical identifiers/numbers, latency and memory
against F08 and the wider corpus. It becomes the default only if the recorded gates
pass. Imported transcripts avoid the model entirely. GPU and larger model profiles
remain optional, explicit and independently measured.

Reviewed profiles (P07 increment 3c, decision D6), chosen by the identity of the
registered model file:

| Profile | File | Gates on 4 threads | Measured 2026-09-25, Windows 11, Xeon E5-2698 v4 |
| --- | --- | --- | --- |
| `base` (**default**) | `ggml-base.bin`, 147,951,465 B | Enforced: WER <= 10% on clips without noise; every spoken critical term (noisy included) except known misses. Reported: RTF <= 0.5, peak `whisper-cli` memory <= 400 MiB. Noisy-speech WER not gated (#150) | WER 3.25% clean (met); no unexpected miss (met); RTF 0.388 (0.489 on a busier run); 338 MiB; F08 WER 61.5% (known limitation); load 316 ms |
| `base_q5_1` (optional) | `ggml-base-q5_1.bin`, 59,707,625 B | reported only | RTF 0.409; 250 MiB; WER 4.06% clean, 46.2% F08; load 177 ms |

`base` is the measured default (maintainer decision, 2026-09-25): speech with
background noise is gated on critical terms only, and a noise word-error gate
waits for the noisy-speech fixture set of issue #150 ([L-020](known-limits.md#l-020)). See the
[qualification record](p07-asr-qualification.md). A run's recognizer threads are the
machine's parallelism, at most 8, and count against CPU admission; the gates are
measured at 4.

## What each part of the evidence shows

P03's [filesystem feasibility record](p03-storage-feasibility.md) records native API
experiments and missing OS/storage crash evidence. [ADR 0010](../decisions/0010-storage-qualification-gate.md)
accepts ephemeral desktop evidence as the P03 target. PR #42 (`3eef9b7`)
shows process-crash-consistent internal publication through native
tests on each protected OS; it does not expose a user command or claim OS-crash
durability. Since P10 PR 4 the **durable profile** is shown on one system: Ubuntu 24.04
(`/etc/os-release`) with the session root on local ext4 mounts that keep write
barriers (`/proc/self/mountinfo`), and nothing else. There a durable session,
requested through the engine API (`IngestRequest::durability`; the command line
gains it with P11's durable workspace), acknowledges a generation only after the
protocol of [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)
has made it survive an OS crash or power loss, as the
[P10 durable-publication record](p10-durable-publication.md) shows. Every other
profile, and every ephemeral session, reports `process_crash_consistent`, and a
durable request there fails with `MISSING_CAPABILITY` before anything changes.
Storage that ignores flushes ([L-056](known-limits.md#l-056)), the loss of the disk or
host ([L-057](known-limits.md#l-057)) and an Ubuntu 24.04 user space on a kernel the
campaign did not run ([L-058](known-limits.md#l-058)) stay outside what was shown. P11
delivered the worker host and its [qualification record](p11-worker-host.md); the
adversarial half of SEC-T01 is not produced in R0 ([L-068](known-limits.md#l-068), moved to R1
by the maintainer on 2026-10-03), so R0 makes no claim that the worker's strict profile
contains an exploited decoder or a malicious provider.

What the release as a whole must show, each part with its evidence item in the
[P14 plan](p14-qualification.md):

- Fresh-machine installation without Rust, upgrade, rollback and uninstall (RQ-01 to RQ-04).
- All deterministic PR checks plus platform process/filesystem conformance tests.
- Crash injection and source-preservation proof on the named filesystem (RQ-11).
- Resource overload, cancellation, descendant cleanup and handle-leak tests (RQ-08, RQ-09).
- End-to-end fixture corpus accuracy, compact-agent evaluation, and the complete
  video-to-grounded-handoff lifecycle through named Codex and Claude Code clients
  (RQ-05, RQ-15, RQ-16).
- Release artifact provenance, inventory, notices and unresolved finding review (RQ-13, RQ-19).

Until the rules of section 1 are met for a machine, documentation says "qualification
target" for it, and no document claims readiness for production use.

P02 adds a PR check of the worker process boundary. It runs the
process contract inside a read-only, networkless Ubuntu 24.04 container with explicit
CPU, memory, swap and PID limits and no added capabilities. The test verifies that a
new process group remains inside the worker cgroup, induces observable CPU throttling,
reaches the PID ceiling and confirms that a bounded over-limit allocation cannot
complete successfully. That proves the supervisor's attestation and
fail-closed split; it is not the full P11 worker or P14 release evidence.

P04's internal desktop media profile is fixed in [ADR 0012](../decisions/0012-p04-source-media-profile.md)
and the [qualification record](p04-media-qualification.md): 20 GiB/600 s source
staging, four-hour/32-stream/16-megapixel probe, 15- or 30-second provider deadlines,
bounded stdout/stderr and a ten-second mono PCM maximum. The native Windows/NTFS
real-media run is development evidence only; the matrix above is where P14 states
what each system has shown, and the media-bound abuse runs are RQ-10 (whose result for 0.1.0
and the fixes that followed are in the plan, section 18).

## R1 profile planning


R1 adds an explicitly managed embedded-node profile and an industrial worker-plane
profile; it does not change desktop defaults. P15 must name their exact OS, filesystem,
catalogue, queue/object-store topology, isolation boundary and reference hardware before
implementation. Throughput, recovery time, recovery point and catalogue-size claims
remain unset until measured. See the
[R1 industrial capability expansion](r1-industrial-capability-expansion.md).
