# Running VSift as a worker host

Status: operator runbook for the P11 worker host
([ADR 0021](../decisions/0021-worker-and-batch-host.md)), 2026-09-28. The Linux strict
worker profile is a **qualification target**, not a supported platform: public support
begins after P14 ([support profiles](../planning/support-and-resource-profiles.md)).
For P11 the strict profile's isolation (SEC-T01) rests on non-adversarial evidence the
maintainer accepted: the kernel attestation and the hardened container controls of the
CI job; adversarial containment evidence is technical debt
([known limit L-068](../planning/known-limits.md#l-068)), moved to R1 by the maintainer's
decision of 2026-10-03. **R0 makes no claim that the strict profile contains a hostile
decoder or provider:** the worker host is a qualification target, and the limits are the
host's. What was tested, and how, is in the
[P11 qualification record](../planning/p11-worker-host.md).

VSift has no daemon, no HTTP listener and no queue. A worker host is an external
supervisor (your queue consumer, a systemd unit, a container orchestrator) that
writes job requests to files and runs `vsift job run` or `vsift job batch` on them,
one process per invocation. VSift owns the work inside one workspace: admission,
idempotency, recovery and shutdown. The supervisor owns the queue, acknowledgements,
routing, retries across hosts, scheduling across workspaces and replication.

## 1. Layout

| Path (example) | Owner and mode | Purpose |
| --- | --- | --- |
| `/var/lib/vsift/home` | `vsift`, 0700 | the worker account's per-user VSift configuration (registered tools and model) |
| `/var/lib/vsift/workspace` | `vsift`, 0700, created by VSift | the worker workspace: sessions, jobs, request records |
| `/srv/vsift/inputs` | readable by `vsift`, never written | the operator input root; request paths are relative to it |
| `/srv/vsift/bundles` | `vsift`, writable | the bundle root; `retain` steps write `<bundle_name>/` here |
| `/var/lib/vsift/queue`, `/var/lib/vsift/results` | `vsift` | request files written by the supervisor and the event streams it reads back |

Create the account and the folders before anything else (the P14 walk of 2026-10-02 made
them this way; the account's uid is the one section 10's `--user` names, so the volumes
are writable from the container too):

```console
sudo useradd --system --uid 10001 --user-group --home-dir /var/lib/vsift/home \
  --no-create-home --shell /usr/sbin/nologin vsift
sudo install -d -o vsift -g vsift -m 0700 /var/lib/vsift /var/lib/vsift/home /srv/vsift/bundles
sudo install -d -o vsift -g vsift /var/lib/vsift/queue /var/lib/vsift/results
sudo install -d -m 0755 /srv/vsift/inputs
```

Rules VSift enforces: the workspace and the per-user folder must be private to the
worker account (else `STORAGE_IO` before use); every request path is opened inside the
input root one name at a time following no link, so inputs must be plain files with
one hard link, not symlinks (L-062); a bundle directory VSift creates is owner-private,
so readers of bundles run as the worker account or copy them. For a durable workspace
put `/var/lib/vsift` on local ext4 (Ubuntu 24.04) with write barriers; network
filesystems are unqualified.

A workspace is **one trust domain**. Any request delivered to it may target any of its
sessions by id and writes into its one bundle root, so give each tenant or trust
domain its own workspace, input root, bundle root and worker account. VSift has no
multi-tenant host (SEC-19, SEC-T03).

## 2. One-time setup

The tools come first. On Ubuntu 24.04 x64 the published `vsift` installs the reviewed
FFmpeg, FFprobe, whisper.cpp and model for the worker account itself
([`install.md`](install.md) section 5.1: `setup plan`, then `setup install --plan ...
--accept-plan <digest>`, run with the worker's `HOME`; a minimal image also needs
`libgomp1`); then the lines below register them by path. The paths in the example
(`/usr/bin/ffmpeg`, `/opt/whisper.cpp/...`) are an operator's own tools: use the
reviewed files' real paths if you registered the managed ones. The P14 walk did exactly
that.

```console
# As the worker account, with its own per-user base.
export HOME=/var/lib/vsift/home XDG_CONFIG_HOME=/var/lib/vsift/home/.config
vsift setup configure ffmpeg  --executable /usr/bin/ffmpeg  --json
vsift setup configure ffprobe --executable /usr/bin/ffprobe --json
vsift setup configure whisper --executable /opt/whisper.cpp/bin/whisper-cli --json
vsift setup configure-model --file /opt/whisper.cpp/models/ggml-base.bin --json
vsift setup check --json
vsift --session-root /var/lib/vsift/workspace session init-workspace \
  --durability durable --admission-slots 8 --retention-hours 168 --json
```

- **Durability.** `durable` is accepted only on Ubuntu 24.04 with the workspace on
  local ext4 **mounted with write barriers**; anywhere else it fails with
  `MISSING_CAPABILITY` and creates nothing. A hosted CI runner's disk is ext4 mounted
  `nobarrier`, so `durable` is refused there (the P14 walk met this on 2026-10-02): look
  at the mount options (`findmnt --target /var/lib/vsift`) before relying on it. On a
  test host without such a disk, an ext4 file system in a file has barriers: `truncate
  --size 24G <image>`, `mkfs.ext4 -F <image>`, `mount -o loop,rw,relatime <image>
  /var/lib/vsift` (the P14 campaigns do this; it is a test aid, not a production
  layout). Use `ephemeral` elsewhere: sessions then survive a crash or kill of VSift, not
  an OS crash or power loss.
- **Admission slots.** The workspace's capacity in weight units, fixed at creation: a
  visual window's FFmpeg pass weighs 2, a copy, probe or evidence extraction 1, a
  speech recognition its recognizer threads (the machine's parallelism, capped at 8 and
  at the capacity). Size it from the CPUs the worker's cgroup grants: with
  `CPUQuota=800%` use 8. A request whose work is heavier than the whole capacity fails
  `RESOURCE_LIMIT` before any tool runs. `--concurrency` of a batch may not exceed it.
- **Retention.** Sessions live the retention after they open or are renewed, at most
  720 hours in all. It is **not** the window in which a redelivered request is recognised
  as a duplicate: that lasts while the session exists or while the workspace holds fewer
  than 4,096 request records, whichever is longer (section 5).
- The policy is immutable: running the same `init-workspace` again answers
  `already_initialized`; another policy is refused. To change it, drain the host and
  create a new workspace.

## 3. Supervisor invocation

A request names only relative paths, steps and an operation id: never an executable,
environment, absolute path or policy. Example `request.json`:

```json
{"schema_version":"1","operation_id":"op_3f9c2a7e5d1b4c6a8e0f2d4b6a8c0e1f","durability":"durable","deadline_ms":3600000,"target":{"ingest":{"source":"incoming/2026-09-28/walkthrough.mp4","transcript":null}},"steps":[{"retranscribe":{"range":null}},{"candidates":{"range":null}},{"retain":{"bundle_name":"walkthrough-3f9c2a7e","include_source":false}},{"close":{}}]}
```

One request per message, noninteractive, with the explicit workspace, roots, isolation,
admission wait and drain time. `--host-isolation strict-linux` is accepted only where
the kernel attests the limits (the unit or the container below): in a plain shell the
same command is refused before any work with `ISOLATION_UNAVAILABLE` (exit 2), and that
refusal is the only event it writes:

```console
vsift --host-isolation strict-linux --session-root /var/lib/vsift/workspace \
  job run --request /var/lib/vsift/queue/3f9c2a7e.json \
  --input-root /srv/vsift/inputs --bundle-root /srv/vsift/bundles \
  --admission-wait-ms 60000 --drain-timeout-ms 30000 --events jsonl </dev/null
```

Or a file of up to 1,000 such lines (a longer file is refused whole, L-066), at most
`--concurrency` at once:

```console
vsift --host-isolation strict-linux --session-root /var/lib/vsift/workspace \
  job batch --requests /var/lib/vsift/queue/batch-0042.jsonl \
  --input-root /srv/vsift/inputs --bundle-root /srv/vsift/bundles \
  --concurrency 4 --admission-wait-ms 60000 --drain-timeout-ms 30000 --events jsonl </dev/null
```

- **Deadline.** `deadline_ms` bounds one delivery (at most one day); a step never
  starts with less than a second left and a running step is cancelled at the deadline
  (`DEADLINE_EXCEEDED`, resumable). The deadline, admission wait and attempt count
  restart with each delivery (L-065), so bound total effort in the supervisor.
- **Noninteractive.** VSift never prompts, installs or downloads. `--events jsonl`
  writes one bounded JSON line per event (at most 64 KiB, no paths, no transcript
  text); read stdout continuously, because a reader that stops reading holds the batch
  back (backpressure). `--json` prints only the final result.
- **Readiness.** The first event is `lifecycle` `started` with `readiness`
  (`publication`, `isolation`, `admission_capacity`, `concurrency`). A failure before
  it (no workspace, isolation not attested, a bad flag) is the only event: treat the
  host as not ready and do not acknowledge anything.
- **Exit status.** `job run`: 0 for `complete` or `partial`, else the failing step's
  class. `job batch` (maintainer decision D5): 0 when every line is complete or
  partial, 6 on shutdown, else the worst class 7 > 1 > 5 > 3 > 2 > 4. Acknowledge per
  request from the results (section 4), not from the batch's exit status.

### Example systemd unit (Ubuntu 24.04)

The supervisor writes the batch file and the unit runs it; the unit has no network,
so the supervisor that talks to the queue runs outside it. This is an example to
adapt, not a qualified artifact: confirm on the host that the batch's `started` event
says `"isolation":"strict_linux"` before taking work.

```ini
# /etc/systemd/system/vsift-batch@.service  (start: systemctl start vsift-batch@0042)
[Unit]
Description=VSift job batch %i
RequiresMountsFor=/var/lib/vsift /srv/vsift

[Service]
Type=exec
User=vsift
Group=vsift
Environment=HOME=/var/lib/vsift/home XDG_CONFIG_HOME=/var/lib/vsift/home/.config XDG_CACHE_HOME=/var/lib/vsift/home/.cache
ExecStart=/opt/vsift/bin/vsift --host-isolation strict-linux \
  --session-root /var/lib/vsift/workspace job batch \
  --requests /var/lib/vsift/queue/batch-%i.jsonl \
  --input-root /srv/vsift/inputs --bundle-root /srv/vsift/bundles \
  --concurrency 4 --admission-wait-ms 60000 --drain-timeout-ms 30000 --events jsonl
StandardInput=null
StandardOutput=truncate:/var/lib/vsift/results/batch-%i.events.jsonl
StandardError=journal

# Stop: SIGTERM to vsift only; it stops admitting, lets running steps drain for
# 30 s, cancels them, and gives each provider 5 s graceful + 5 s forced. Whatever
# is left after TimeoutStopSec (>= drain + 10 s, plus margin) is SIGKILLed.
KillMode=mixed
KillSignal=SIGTERM
SendSIGKILL=yes
TimeoutStopSec=45

# cgroup v2 limits: the strict attestation requires finite CPU, memory and PIDs.
CPUQuota=800%
MemoryMax=12G
MemorySwapMax=0
TasksMax=256

# Read-only root, no network but loopback, no privileges.
ProtectSystem=strict
ReadWritePaths=/var/lib/vsift /srv/vsift/bundles
ReadOnlyPaths=/srv/vsift/inputs
PrivateNetwork=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectHome=yes
NoNewPrivileges=yes
CapabilityBoundingSet=
AmbientCapabilities=
RestrictSUIDSGID=yes
LockPersonality=yes
```

**Why `KillMode=mixed`, not `control-group`.** With `control-group`, systemd sends
`SIGTERM` to every process of the unit at once, so FFmpeg and whisper.cpp are signalled
at the same moment as VSift and can exit abnormally before VSift's own cancellation
reaches them: a step may then fail with a provider error instead of being cancelled
resumably. `mixed` sends `SIGTERM` to VSift alone and `SIGKILL` to the whole cgroup
only when `TimeoutStopSec` expires, so the graceful path runs first and nothing
outlives the unit either way. VSift reaps its providers before it exits on every
graceful path; on Unix a `SIGKILL` of VSift alone leaves a running provider to finish
its current unit (L-055), which the cgroup kill removes.

`TimeoutStopSec` must cover `--drain-timeout-ms` plus 10 s (the providers' graceful and
forced budgets); a commit in progress always completes first. Do not set `Restart=`:
the supervisor redelivers instead (section 6). The unit exits non-zero for any batch
that is not wholly complete or partial; read the events file for per-request results.
A `systemctl stop` while requests run ends with exit status 6 (systemd labels it
`NOTCONFIGURED`; it is VSift's shutdown code), the unit `failed`, and an events file that
ends with `lifecycle` `stopped` and the terminal event; `systemctl start` of the same
unit redelivers the file. Walked on a hosted runner (P14, RQ-12): the stop took the full
30 s drain because a recognition was running, inside `TimeoutStopSec`.

## 4. Queue acknowledgement

Acknowledge a message only after VSift has **recorded** its result: a result is
recorded exactly when it is returned from `job run`'s `data`, or a `job batch`
`result` event, **and** it has ended. Then every later delivery of the same request
replays exactly that result (`replayed: true`) without running anything, even when the
inputs are gone.

| What the supervisor sees for a request | Recorded? | Action |
| --- | --- | --- |
| `status` `complete` or `partial` (fresh or `replayed`) | yes | acknowledge; `partial` names its gaps in each step's `coverage` |
| `failed` with `INVALID_ARGUMENT`, `INVALID_SOURCE`, `RESOURCE_LIMIT`, `INTEGRITY_FAILURE`, `UNSUPPORTED_SCHEMA`, `INTERNAL` | yes | acknowledge to a dead-letter queue for a person |
| `failed` with `MISSING_CAPABILITY` | yes | acknowledge to dead letter **and** mark the host unhealthy: a tool or model is missing or changed; after fixing the host, resubmit under a **new** operation id (L-069) |
| `failed` with `IDEMPOTENCY_CONFLICT` | the other request is | a producer bug (one id, two requests): dead letter, never retry |
| `BUSY` (exit 4, `retry_after_ms` 2000) | no | redeliver the same bytes after the hint: another process holds the request or the capacity |
| `cancelled` or `failed` with `CANCELLED`, `DEADLINE_EXCEEDED`, `STORAGE_IO` | no | redeliver the same bytes (backoff); the request continues from its first unfinished step |
| `job run` `STORAGE_IO` (exit 7) **with** a result as `data` | no (the work is committed, its result is not) | redeliver; the next delivery records and returns it |
| a batch line `request_finished` `rejected` (no result) | no | the line is not a valid request: dead letter |
| no result at all (the process was killed, the host crashed, a failure before `started`, `ISOLATION_UNAVAILABLE`) | no | do not acknowledge; fix the host if it said why, then redeliver |

A shutdown that stops a request ends it as `cancelled` with `CANCELLED` and, in `job run`'s
terminal error, a remediation that says to deliver the same request again, whether the signal
arrived between two steps or while one was running (until P14 PR 7 a step cancelled while it
ran reported `CANCELLED` with an empty remediation, #268). A supervisor acts on the code and
the status, never on that text.

In a batch, act on each `result` event as it arrives; lines the batch never started
(`not_started_from_line`) have no result and are simply redelivered. The events file of
a stopped batch ends with `lifecycle` `stopped` and the terminal event; a file without
them belongs to a batch that was killed.

## 5. Duplicates and operation ids

The operation id is the idempotency key. Derive it deterministically from the
message's own idempotency key (for example `op_` and the first 32 hex digits of a
SHA-256 of it), never from the delivery, so every redelivery carries the same id.

- The same id with the same request (spacing, member order and an omitted deadline do
  not matter): replayed when ended, `BUSY` while another process runs it, continued
  when interrupted. One id opens at most one session.
- The same id with another request: `IDEMPOTENCY_CONFLICT`, nothing changed.
- Duplicates are recognised **within one workspace** only. Route every delivery of a
  message to the same host and workspace (sticky routing); a duplicate delivered to
  another workspace runs again there.
- A workspace keeps at most 4,096 request records; when it is full, records of sessions
  that no longer exist are pruned first, else a new request is `RESOURCE_LIMIT`
  (L-063). A record outlives its session until it is pruned, and **it is pruned as soon as
  the table is full and its session is gone**: a session that was closed and then removed by
  `session clean` takes its record with it, however short its life. So the dedupe window
  lasts while the session exists, or while the table is not full, and not for the retention:
  a duplicate delivered later runs again, and the same id with another request is accepted
  (the P14 soak met both, #286). **Acknowledge a message before the session can be
  cleaned**, and do not rely on a replay or an `IDEMPOTENCY_CONFLICT` for an old id.

## 6. Restart and recovery

After a crash of VSift, the worker or the host, redeliver every unacknowledged
message: interrupted requests continue from their first unfinished step, a speech
recognition resumes from its chunk checkpoints, and ended requests replay. The owner
lock of a request is an OS lock released when its process dies, so a `BUSY` after a
restart means another live process holds it. In a durable workspace on the qualified
profile every acknowledged result survives an OS crash or power loss (the P10 crash
campaign, rerun with worker requests); an ephemeral workspace survives process crashes
only. Losing the disk or the host loses the workspace (L-057): copy retained bundles to
replicated storage if they must outlive the host.

A stopped batch (exit 6) leaves every started request resumable; its interrupted
speech recognitions can also be finished by id with `vsift --session-root <ws> job
resume <job>` (the job id is in the step's `job_id`), after which the redelivered
request replays that job's commit.

## 7. Cleanup

- Run `vsift --session-root /var/lib/vsift/workspace session clean --expired --json`
  periodically (a systemd timer), paging with `--cursor` until `next_cursor` is null (a
  cursor is a hash-bucket number, 0 to 255, so a whole pass is up to 256 calls: P14 walked
  24 pages on a small workspace). It
  removes closed, expired and abandoned sessions of the workspace; it never touches the
  input root or the bundle root. `session clean` of a session with active work answers
  `BUSY`.
- A `retain` step killed mid-copy leaves a hidden staging directory
  `<bundle-root>/.<bundle_name>.<op>.retaining` (L-064); VSift never deletes anything
  in the bundle root. Remove such directories only when no request with that operation
  id is running (for example with the worker stopped).
- Delete a request file only after its result was acknowledged; delete inputs after
  every request naming them has ended.

## 8. Disk pressure

On Unix a workspace checks, before each source copy, that the source's size and a
1 GiB reserve are free on its filesystem, else the ingest fails `RESOURCE_LIMIT`, which
ends the request and is replayed (so resubmit under a new id once space is freed). It is
a pre-copy check, not a quota (L-061), and Windows does not check. A write that fails
for lack of space during a step is `STORAGE_IO`, which leaves the request resumable.
Keep the workspace on its own filesystem or quota, alert well above the reserve, and
clean expired sessions. The per-session and per-root budgets (10 GiB per session, 512
artifacts) still apply ([support profiles](../planning/support-and-resource-profiles.md)).

## 9. Provider change and revocation

Tools are registered per worker account (`setup configure`, `setup configure-model`)
and resolved anew by every command; there is no long-lived process to restart. To
revoke or replace FFmpeg, FFprobe, whisper.cpp or the model:

1. Stop admitting: stop the supervisor and send `SIGTERM` to running VSift processes
   (or stop their units). Started requests stay resumable.
2. Change the registration (`setup configure ... --executable <new>`), or remove it
   (a missing tool). A changed executable is verified again against the pinned fixtures
   before its first media stage; only reviewed pinned model profiles run.
3. Restart the supervisor and redeliver.

What VSift does meanwhile: a recognition checks the recognizer's identity before and
after its run, so output of two models is never mixed; an executable or model changed
during a run fails that step with `MISSING_CAPABILITY`. A request needing a tool that
is no longer registered fails `MISSING_CAPABILITY` and ends, and that failure is what
later deliveries replay: resubmit it under a new operation id after the host is fixed
(L-069). Results already recorded are not re-derived with the new provider; a
recording that must be redone with it is a new request.

## 10. Isolated Linux deployment (container)

The same hardening the repository's `strict-worker-boundary` CI job applies to the
process supervisor's qualification (read-only root, no network, CPU, memory, swap and
PID limits, all capabilities dropped, no new privileges, an unprivileged user), as a
worker invocation:

```console
docker run --rm --init \
  --network none --read-only --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  --cpus 8 --memory 12g --memory-swap 12g --pids-limit 256 \
  --cap-drop ALL --security-opt no-new-privileges --user 10001:10001 \
  --env HOME=/var/lib/vsift/home --env XDG_CONFIG_HOME=/var/lib/vsift/home/.config \
  --volume /srv/vsift/inputs:/srv/vsift/inputs:ro \
  --volume /var/lib/vsift:/var/lib/vsift:rw \
  --volume /srv/vsift/bundles:/srv/vsift/bundles:rw \
  --stop-timeout 45 \
  vsift-worker@sha256:<pinned digest> \
  vsift --host-isolation strict-linux --session-root /var/lib/vsift/workspace \
    job batch --requests /var/lib/vsift/queue/batch-0042.jsonl \
    --input-root /srv/vsift/inputs --bundle-root /srv/vsift/bundles \
    --concurrency 4 --drain-timeout-ms 30000 --events jsonl
```

- The image holds VSift, the pinned FFmpeg, FFprobe and whisper.cpp builds and the
  model on its read-only root; the per-user registration under `/var/lib/vsift/home`
  names those paths. Pin the image by digest. The runbook does not ship an image;
  [`tools/p14-campaigns/worker.Dockerfile`](../../tools/p14-campaigns/worker.Dockerfile)
  is the one the P14 campaigns built and ran (the pinned Ubuntu 24.04 base, `libgomp1`,
  the CA bundle, an account of uid 10001, the published executable; the tools came from
  `setup install` into the mounted `/var/lib/vsift/home`, so they are in a volume rather
  than on the root). `--cpus` cannot exceed the host's CPUs (the example's 8 needs an
  8-CPU host; the walk used 4 on a 4-CPU runner), and the volumes' owner must be uid
  10001 (section 1).
- `docker stop` sends `SIGTERM` to the container's first process and `SIGKILL` after
  `--stop-timeout`; with `--init` the signal reaches VSift, so the same drain rule
  applies (at least drain + 10 s).
- The container's cgroup provides the CPU, memory and PID limits the strict
  attestation reads; a durable workspace additionally needs the `/var/lib/vsift`
  volume on local ext4 of an Ubuntu 24.04 host (the profile check reads the container's
  `os-release`, so the image must be Ubuntu 24.04 too, L-058).
- Do not mount a container-engine socket, host credentials or writable host paths
  beyond the three volumes; the worker needs no network.

## 11. Guarantee matrix

What each profile guarantees today. "Tested" means on the named machine in the
qualification record; nothing here is supported before P14.

| Guarantee | Linux strict worker (Ubuntu 24.04, ext4) | Linux, other | Windows 11 desktop (native) | macOS desktop |
| --- | --- | --- | --- | --- |
| `job run`, `job batch`, replay, continuation | yes (CI tests on Ubuntu) | yes, unqualified | yes (all P11 tests and the `p11_*` checkpoint) | yes (CI tests) |
| Durable workspace (`os_crash_durable`) | yes: P10 crash campaign, rerun with requests | no: `MISSING_CAPABILITY` | no: `MISSING_CAPABILITY` | no: `MISSING_CAPABILITY` |
| Survives a VSift crash or kill (`process_crash_consistent`) | yes | yes | yes | yes |
| `--host-isolation strict-linux` | accepted only when the kernel attests cgroup v2 limits, a read-only root and loopback only; adversarial containment evidence deferred as technical debt (L-068) | same attestation | `ISOLATION_UNAVAILABLE` | `ISOLATION_UNAVAILABLE` |
| Provider tree stopped when VSift dies | graceful paths reap; a `SIGKILL` of VSift alone leaves the current unit running (L-055), the cgroup kill removes it | same | yes: the Job Object kills the tree | as Linux (process groups) |
| Graceful shutdown signal | `SIGTERM`/`SIGINT` | same | console Ctrl-Break (or Ctrl-C unless ignored, L-053); a Windows service stop sends no console event | `SIGTERM`/`SIGINT` |
| Free-space reserve before a copy | yes (1 GiB) | yes | not checked (L-061) | yes |
| Weighted admission across processes | yes | yes | yes (tested) | yes |
| Network and filesystem confinement of providers | the host's (attested, not enforced by VSift) | the host's | none: providers run with the user's rights (L-004) | none (L-004) |

On Windows a worker host is a desktop-grade profile: use an ephemeral workspace, a
supervisor that owns a console for VSift so it can deliver Ctrl-Break, and no claim of
isolation or OS-crash durability.
