# ADR 0020: Recoverable jobs and durable publication

- Status: Accepted (maintainer, 2026-09-27). Decisions D-1..D-5 below are confirmed as
  recommended.
- Date: 2026-09-26 (accepted 2026-09-27)
- Tracking: [P10 / issue #13](https://github.com/smormah/vsift/issues/13),
  [#164](https://github.com/smormah/vsift/issues/164)
- Refines: [ADR 0004](0004-recoverable-worker-core.md) (the recoverable core),
  [ADR 0006](0006-workspace-publication-and-durability.md) and
  [ADR 0010](0010-storage-qualification-gate.md) (the publication protocol and its
  qualification gate), [ADR 0017](0017-local-asr-through-whisper-cpp.md) (decision 4,
  Ctrl-C) and [ADR 0019](0019-evidence-navigation.md) (D4, the artifact caps)
- Scope of this record: the design of the whole P10 packet. PR 1 implements the
  commit path (sections 1-3); PR 2 the jobs, keys, checkpoints, retry policy and the
  engine-level job operations (sections 4-5, and D-2 and D-4); PR 3 the public job
  surface and signal handling (sections 5-6); PR 4 the crash campaign (section 7),
  whose evidence enabled durable publication on Ubuntu 24.04 / local ext4 only
  (2026-09-27); every other profile still fails durable requests closed.

## Context

P10 makes long work recoverable: a stage that finished survives an interruption, a
retried request finds the work it already did, cancellation and commit cannot race,
and a durable session keeps every acknowledged generation through an OS crash on the
one profile ADR 0010 allows to claim that, Ubuntu 24.04 on local ext4, once an owned
crash campaign has shown it.

Three facts shaped PR 1. Every session read walked the manifest chain to generation 0,
so a warm evidence request grew by about 3.6 ms per generation (1.06 s at 256, 3.8 s
at 1,024; #164). The P03 protocol flushed files but synchronised no directory, and a
capability directory handle on Linux cannot be flushed (`EBADF`, FS-01): `.` has to be
reopened with read access first. And after a failed `fsync`, Linux may drop the dirty
pages yet report a later `fsync` of the same file as successful ("fsyncgate"), so a
retry must not trust anything a failed attempt flushed.

## Decision

### 1. Incremental chain validation (#164, implemented in PR 1)

The session writer keeps `sessions/<ses>/chain-verified.json`
(`{schema_version, generation, manifest_sha256}`): the newest generation whose whole
chain it has verified. It advances the file under the writer lock after a successful
publication, to the head it has just committed, never beyond it; readers never write
it. The file is staged in `attempts/`, flushed and renamed over the old one, and
failing to write it never fails the commit, because it only shortens later reads.

A read verifies the pointer and the head as before, then walks down the chain only
until a generation whose digest equals both the link from the generation above it and
the anchor's digest. The anchor is the checkpoint or, when newer, the last head the
same store instance verified (adapter state, one entry, never shared between
instances or processes), so several reads in one command walk the chain once.

- A missing checkpoint, or one ahead of the head, means a full walk.
- A checkpoint that does not parse, has a non-canonical digest or disagrees with its
  generation is `INTEGRITY_FAILURE`; a newer schema is `UNSUPPORTED_SCHEMA`.
- Retained exports and cleanup still walk the whole chain (`bundle validate` reads no
  session chain).

**Guarantee.** Every read verifies the head and every generation committed since the
last full verification; committed artifacts are still re-hashed against the manifest
whenever they are read (INV-02). A generation below the anchor that changes is found by
the next full walk (retain or cleanup), not by an ordinary read. Measured through the
binary on Windows 11 (release): a warm reused `frame get` has p95 139 / 369 / 1,064 /
3,794 ms at 2 / 64 / 256 / 1,024 generations before, and the numbers recorded in the
[P09 record](../planning/p09-evidence-navigation.md) after.

### 2. Durable publication protocol (implemented in PR 1, disabled)

Durability becomes a session property. Generation 0 records `durability` in its
manifest, every later generation carries it unchanged (the chain walk checks this),
and a manifest without the field, as every session written before P10, is ephemeral;
an ephemeral manifest still omits it, so older builds keep reading ephemeral sessions.
The session's value, never the request's, chooses the protocol: publishing an
ephemeral session durably is a conflict, and a durable session commits durably even
for an ephemeral request. `publish_artifact` and `publish_evidence` pass the session's
value instead of a hard-coded ephemeral one.

For a durable session each commit runs, in this order, and acknowledges only at the end:

1. each content-addressed file is created new, written and flushed; then `artifacts/`
   is synchronised once (an activation also synchronises it for the source copy);
2. the manifest is staged in `attempts/`, written, flushed, renamed into
   `generations/`, and `generations/` is synchronised;
3. the pointer is staged, written, flushed, renamed to `current.json`, and the session
   directory is synchronised: the generation is committed and may be acknowledged;
4. the chain checkpoint follows (section 1).

Initialization flushes generation 0 and the pointer, synchronises `generations/` and
the attempt directory, renames it into `sessions/`, synchronises `sessions/` and the
session-index bucket and index, and only then acknowledges. A directory is synchronised
by reopening `.` relative to its capability handle with read access
(`dir.open(".")?.sync_all()`); where a platform cannot do that the call fails and
nothing is acknowledged.

**fsyncgate.** A durable commit never re-flushes what an earlier attempt left: a
leftover staged file is deleted and written again; an existing content-addressed
artifact is accepted only when the committed head lists it, otherwise its bytes are
staged again and renamed over it; an identical manifest a failed attempt already
renamed into `generations/` is staged again over itself; a leftover initialization
attempt is rebuilt, never adopted. A retry that finds its generation already committed
repeats the directory synchronisations before acknowledging. Any flush, sync or rename
error is `STORAGE_IO` with no acknowledgement. Directory metadata errors on ext4 abort
the journal and remount the filesystem read-only by default, so a later directory sync
fails rather than silently succeeding; the campaign (section 7) must confirm this.

**Enablement gate.** `durable_profile` decides what a root may claim when the store is
opened: `os_crash_durable` only when the build targets Linux, the root's `st_dev` maps
through a bounded (1 MiB), strict parse of `/proc/self/mountinfo` to ext4 mounts only,
none with `nobarrier` or `barrier=0`, and the constant `QUALIFIED_UBUNTU_EXT4` is set.
It is `false` until PR 4 records the campaign evidence (the Ubuntu release check lands
with it), so every profile still answers a durable request with
`MISSING_CAPABILITY` before any mutation. The parser is fuzzed (`mountinfo`).

### 3. Fault points (implemented in PR 1)

`FaultPoint` names every boundary of the commit path: `artifact-install`,
`artifact-directory-sync`, `manifest-write`, `manifest-flush`, `manifest-rename`,
`manifest-directory-sync`, `pointer-write`, `pointer-flush`, `pointer-rename`,
`pointer-directory-sync` and `chain-checkpoint-write`. They generalise P03's test-only
crash boundaries and are reached in both modes (in ephemeral mode a directory-sync
point marks where the sync would be). In unit tests and in builds with the
`fault-injection` feature, `VSIFT_FAULT_POINT=<name>[:<n>]` makes the process exit at
once (status 91, a marker on standard error, no unwinding) the n-th time one commit
reaches the point; the count lives in the commit, not in process-wide state. The
feature is off by default, the crate refuses to compile it without debug assertions,
and the governance check refuses it anywhere but a development dependency. A registry
test kills a child process at every point and checks S-07: the session reopens at its
last acknowledged generation or the new one, every listed file is whole, and the same
operation then completes. The durable order is checked on every platform by a
test-only operation recorder passed to the commit (not a global).

### 4. Jobs, operation keys and chunk checkpoints (PR 2)

A long operation runs as a job keyed by its request key (`opk_sha256_...`, as P09's
evidence keys), and the job id derives from the session and the key, so a retried
request finds its job. Stages checkpoint as `ChunkCheckpoint` files under
`sessions/<ses>/jobs/<job>/chunks/`: uncommitted private stage files holding the raw
provider output of one chunk, validated when read and redone when invalid. Only the
final commit publishes evidence; a checkpoint is never evidence. `JobState` gains
`Interrupted` and `Committing`. The OS lock on the job, not a heartbeat or timestamp,
is the liveness authority: a job whose lock can be taken is interrupted and resumable.
`transcript retranscribe` is the first checkpointed operation.

### 5. Cancellation and retry (PRs 2-3)

Cancellation is serialized with the commit under the job's lock: a cancel that wins
leaves no generation, one that loses reports the committed result, and a repeated
cancel is idempotent. Retry policy: `BUSY` is retried automatically at most twice with
full-jitter backoff (base 200 ms, cap 2 s); other failures go to the caller; integrity,
unsupported-schema, invalid-argument and missing-capability failures are never
retried; a chunk that fails three times is poisoned and ends the job with a typed
failure. The CLI traps Ctrl-C and SIGTERM through Tokio's `signal` feature and turns
them into cancellation, which supersedes ADR 0017 decision 4.

### 6. Public surface (PR 3)

A public job surface (`job status`, `job resume`, `job cancel`) and cancellation
events, with schemas and contract tests, subject to D-1, D-3 and D-4 below.

### 7. Crash campaign and host loss (PR 4)

Durable enablement needs an owned crash campaign on Ubuntu 24.04 / ext4: dm-log-writes
replay of at least 2,000 crash points, at least 300 QEMU kills during durable commits,
dm-flakey `EIO` injection on flush and write, and a mandatory negative control (a build
with one directory sync removed must lose an acknowledged generation, or the harness
cannot see what it claims to test). Passing flips `QUALIFIED_UBUNTU_EXT4`. X-10: local
durability survives a worker or OS crash on the qualified disk; losing the disk or
host is out of scope and must be covered by the caller's own replicated storage, which
the documentation states.

## Maintainer decisions (confirmed 2026-09-27)

- **D-1** A caller-supplied `--operation-id op_…` on `transcript retranscribe` only.
  The engine request carries an optional operation id from PR 2; the CLI flag lands in
  PR 3.
- **D-2** Caps of 512 artifacts / 384 evidence / 128 KiB, for generation manifests
  (and the retained bundle manifest that repeats their artifact list) only, now that
  #164 is measured (warm reads no longer grow with the chain). The 10 GiB bound is
  unchanged. Implemented in PR 2.
- **D-3** Durable mode requestable via the engine API only in P10; CLI via P11's
  durable workspace.
- **D-4** New failure code `IDEMPOTENCY_CONFLICT` (exit 2, not retryable) for the same
  operation id with a different request digest. Implemented in PR 2 (engine level).
- **D-5** Campaign on hosted `ubuntu-24.04` with KVM first, falling back to a
  maintainer-owned disposable KVM host (PR 4).

## Implementation notes: PR 2 (2026-09-27)

PR 2 implements sections 4 and 5 at the engine and storage level. Where it refines the
design above:

- **Keys.** The request digest (`vsift.retranscribe-request.v1`) is the session, the
  command and its canonical parameters (the range, or none). The *recognition key*
  (`vsift.recognition.v1`) is the session, source, audio stream, replaced range, chunk
  plan, decoding profile, recognizer identity and the local-ASR verification
  fingerprint (which binds the tools, isolation, verifier and `VSift` version); it
  deliberately leaves out the base revision, so checkpoints stay valid when only the
  base changes. The operation key (`opk_sha256_`, `vsift.retranscribe-op.v1`) is the
  recognition key and the base revision id; the job id is `job_` and 32 hex digits of
  `sha256("vsift.job.v1\n" + session + "\n" + opk)`, so an identical request after a
  crash finds the same job. Each commit uses a deterministic operation id derived from
  the job, its epoch and its attempt.
- **States.** Two edges are added to section 4's states: `Committing -> Interrupted`,
  taken only after recovery found in the manifest chain that the commit did not land,
  and `Running -> Failed` for a poisoned chunk or used-up attempts. A failed or
  cancelled job is restarted by the same request in a new *epoch* (attempts and
  failures reset); `job resume` refuses it.
- **Checkpoints.** `chunks/<ordinal:05>.json` v1 holds the raw provider output (token
  probabilities as their IEEE bits, so a decimal round trip cannot change a resumed
  revision), the decoded audio range and the outcome kind; a checkpoint is stored only
  after the output passed validation, and a resumed chunk passes the same validation
  and merge again, so the resumed revision is byte-identical. Reading one returns
  absent, found or unusable (removed, counted as `checkpoint_discarded`). Storing is an
  optimisation: a checkpoint over 256 KiB or one that cannot be written costs only the
  redo.
- **Retry and poison.** `BUSY` (admission, the writer, a generation moved by a renewal
  or another revision) is retried at most twice with full jitter (200 ms, doubling, cap
  2 s); every other failure ends the call with the job interrupted and resumable, and
  the job fails for good only on a poisoned chunk (three identical codes at the same
  chunk; cancellations and failures outside a chunk never count), after 16 attempts, or
  when a superseding revision changes the range.
- **Commit.** Under the job's state lock the job records `committing` with its commit
  operation id, the generation it expects to follow and the revision id, then
  publishes. A failure after the pointer moved is reconciled at once from the chain.
  A generation moved meanwhile is followed: with the same base the commit is retried
  against the new head; with a new newest revision the request is widened over it
  again, and the same range re-splices the recognised segments onto it (else the job
  fails as superseded and `BUSY` asks for a rerun). Reconciliation of a job whose owner
  is gone runs one way only: its commit operation above its recorded generation in the
  chain makes it `succeeded`.
- **Locks.** Every lock is taken without waiting, so no order can deadlock. The order
  used is the session's shared lifetime hold (the bound source copy, the work
  directory, the job owner), the job's `owner.lock`, then inside a publication
  admission, the lifetime hold again and the writer. `state.lock` is held only for a
  few file operations, with a bounded 500 ms retry to take it. A liveness probe takes a
  shared lock on `owner.lock` for an instant, so a racing acquisition may be told busy.
- **Operation ids.** A caller's id is bound to its job (`jobs/by-operation/<op>.json`,
  at most 8 per job and 256 per session) and pins the job; when the caller gives none,
  the commit's own id is bound after the commit as an alias that can replay the result
  and pins nothing. At 64 jobs the oldest ended, unowned, unpinned job is pruned. Ids
  are session-scoped and expire with the session.
- **Abandoned manifests (L-048).** A publication holds the writer lock from its
  manifest rename to its pointer rename, so a manifest found above the head under the
  writer lock is always one an ended publication left. It is now replaced by the next
  publication (staged again, as a durable retry already did) instead of refusing every
  other operation, so a crash between the two renames no longer blocks the session.
- **Readers meeting a replacement (found by the X-04 stress of PR #179).** Every
  metadata file a writer replaces by staging and renaming (the commit pointer, the
  chain checkpoint, job records, bindings and index entries) can be met mid-rename by
  a reader in another process. On every platform the reader can open the old file
  just before the rename and read its metadata just after, when it has no link left;
  on Windows the name can also be absent for a moment (a probe of 200,000 opens during
  continuous renames saw 701 of the first and 395 of the second). The single-link
  check reported the first as a damaged file, and every reader turned both into
  `INTEGRITY_FAILURE` for a healthy session: a latent P03 defect that PR 1's chain
  checkpoint and PR 2's job records made far more frequent. A file with no link left
  is now reported as no longer at its name (`NotFound`), and these files are opened
  with a bounded retry (a few immediate attempts, then 1 ms apart, at most 500 ms,
  also for access denied on Windows). What the reader then opens is a committed
  file, old or new; the old one is a consistent earlier snapshot. A hard-linked or
  non-regular file is still refused at once, and a file still missing after the
  budget is still an integrity failure. Generations are never replaced while a
  pointer names them, so manifest reads need no retry.
- **Racing creators of a lock anchor (found by the X-03 stress on macOS).** Eight
  identical requests create a job's `owner.lock` and `state.lock` at the same moment.
  On macOS one create-if-missing open occasionally reported `NotFound` (raw OS error
  2, from `jobs.rs` `create_or_open`, with 29 descriptors open of 10,240) although
  the job directory existed: a peer's concurrent creation of the same name, not
  damage. That open now uses the same bounded retry (at most 0.5 s), so the peer's
  file is opened once it appears; a directory that really vanished still fails
  after the budget.

## Implementation notes: PR 3 (2026-09-27)

PR 3 implements section 6 and the host half of section 5. Where it refines the design:

- **Public commands.** `job status <job>`, `job resume <job>` and `job cancel <job>` take
  no session: the job is found through the root's `job-index/`. `job-data.schema.json`
  reports the job's state, `live_owner`, `resumable` and `resumable_reason`
  (`interrupted`, `not_started`, `live_owner`, `succeeded`, `failed`, `cancelled`,
  `session_not_open`), the operation id a retry should carry, the requested range,
  progress (`chunks_total`, `chunks_checkpointed`), attempts, the committed revision
  and generation, and the last failure's code; never a path or text. `job resume`
  answers `job-resume-data.schema.json` (the job and the retranscription). `session
  status` adds its 16 newest jobs, read-only: `observed_state` reports what
  reconciliation would record without taking ownership. `job run` and `job batch` stay
  reserved for P11.
- **Planned chunks.** Job records written since PR 3 carry `planned_chunks` (optional,
  so PR 2 records still decode; PR 2 builds reject the new field, and sessions are
  disposable). It derives from the recognition key's range and plan, so it cannot
  disagree with the job's identity.
- **Resume rules.** `job resume` is `BUSY` for any live owner (also one that has only
  just created the job), `INVALID_ARGUMENT` with `JobNotResumable` for an ended job, and
  `INVALID_ARGUMENT` with the new `JobSessionNotOpen` (session and job in
  `affected_ids`, a renew-or-reopen remediation) for a closed or expired session, all
  before any tool is resolved.
- **Operation ids (D-1).** `transcript retranscribe --operation-id op_...` is parsed by
  the grammar before any I/O. A request that carries an id is now answered from its
  binding before tools are resolved (a committed id replays, another request with it
  is `IDEMPOTENCY_CONFLICT`, a live job is `BUSY`), so a retry works even where the
  tools have gone; a request without an id keeps ADR 0017's D5 order (the model is
  checked before any session read).
- **Job cancel reaching a running provider.** `job cancel` only records `cancelling`
  under the state lock (unless the job is committing: then it is too late). The owner
  reads its record every 250 ms (`JOB_CANCEL_POLL`) while it runs and fires its own
  cancellation, which makes the supervisor stop the provider it is running; the run
  then meets the request under the state lock and cancels the job (its checkpoints are
  removed). Before PR 3 a request was seen only between chunks.
- **Interruption of the command line (supersedes ADR 0017 decision 4).** The first
  `SIGINT`/`SIGTERM` (Unix) or console Ctrl-C/Ctrl-Break (Windows) during a long command
  (`ingest`, `transcript retranscribe`, `candidates`, `frame`, `crop`, `audio`, `job
  resume`) cancels the command's one `Cancellation`; a second escalates it
  (`Cancellation::escalate`): providers are killed without the 5 s graceful wait and
  reaped within the forced 5 s. The process only exits when the command has returned,
  so it never leaves a provider running (SEC-04). A retranscription stopped this way
  commits nothing, keeps its job `interrupted` and answers `CANCELLED` (exit 6) with the
  session and job in `affected_ids` and a remediation whose command is `vsift job
  resume <job>` (new `EngineError::JobInterrupted`). `ingest` checks the cancellation
  before every 64 KiB block of its copy and removes the partial copy; `candidates` and
  the evidence commands keep their partial-commit semantics. Short commands keep the
  default behaviour.
- **Failures caused by the interrupt itself.** A console event reaches every process on
  the console, so on Windows `FFmpeg` or whisper.cpp can die of the same Ctrl-C before
  the supervisor stops it. The supervisor reports a provider that ended unsuccessfully
  after cancellation was requested as `Cancelled` (`completed_termination`), and a
  retranscription records a chunk failure seen after its caller cancelled as the
  cancellation. Otherwise the interrupt would record false evidence gaps (an
  `undecodable` window) or count towards poisoning a chunk.
- **Windows Ctrl-C and the inherited ignore attribute.** Windows never tells a process
  that inherited "ignore Ctrl-C" (a child of a service, of some IDE and agent hosts, or
  of a process created in a new process group) about a Ctrl-C; Ctrl-Break is always
  delivered. Clearing the attribute needs `SetConsoleCtrlHandler(NULL, FALSE)`, i.e.
  `unsafe` platform code, which this ADR does not introduce. It is a known limit.
- **Session-root wait (#144).** `EnginePorts::with_session_root_wait` (at most 60 s)
  lets the concurrent-preflight test wait longer than the production five seconds, so
  a throttled runner no longer turns the documented `BUSY` into a test failure.

**Dependency review: Tokio's `signal` feature (2026-09-27).**

- *Purpose:* trap `SIGINT`/`SIGTERM` (Unix, `tokio::signal::unix`) and console
  Ctrl-C/Ctrl-Break (Windows, `tokio::signal::windows`) without `unsafe` code in VSift.
  The alternatives (the `ctrlc` or `signal-hook` crates directly, or our own FFI) add
  crates or need `unsafe`.
- *Lockfile impact:* none. The feature enables `signal-hook-registry` on Unix (already
  in `Cargo.lock` through the `process` feature) and more `windows-sys` features
  (`Win32_System_Console`) of a version already locked; `Cargo.lock` is byte-for-byte
  unchanged and no new crate is compiled on any target.
- *Licence:* unchanged: Tokio and `signal-hook-registry` are MIT (or MIT/Apache-2.0),
  `windows-sys` MIT OR Apache-2.0, all already allowed by `deny.toml`.
- *Maintenance:* the feature is part of Tokio itself (tokio-rs, actively maintained,
  1.x LTS releases), the version VSift already pins; `signal-hook-registry` is
  maintained by the `signal-hook` project and already reviewed with `process`.
- *Exposure:* the feature is enabled in the workspace dependency but used only by the
  CLI's `signal` module; the engine and libraries install no handler, so an embedding
  host keeps control of its own signals.

## Implementation notes: PR 4 (2026-09-27)

PR 4 implements section 7: the owned crash campaign, and the enablement it gates. The
method and every number are in the
[P10 durable-publication record](../planning/p10-durable-publication.md). Where it
refines the design:

- **Campaign.** `tools/p10-crash-campaign/` (a workspace tool, never published) and
  `.github/workflows/p10-durability-campaign.yml` (manual and weekly) on hosted
  `ubuntu-24.04` runners (D-5): layer A replays a dm-log-writes log at every flush and
  FUA write (our own reader of the kernel's version-1 format) and verifies every
  acknowledgement made before each point; layer B kills an Ubuntu 24.04 QEMU guest
  (the pinned cloud image, generic kernel, data disk `cache=none`) at random moments
  in four parallel shards; layer C swaps in a dm-flakey `error_writes` table (dm-dust
  is not available there). The acceptance numbers are those of section 7 (at least
  2,000 replay points and 300 kills with nothing lost, every injected failure
  `STORAGE_IO` and never acknowledged) plus a clean `e2fsck -fn` after every recovery
  and a new commit accepted by every recently acknowledged session after every
  replayed power loss.
- **Negative control (a finding).** Section 7 proposed removing one directory
  synchronisation. Removing the session directory's after the pointer rename lost
  nothing: on ext4 every file flush commits the whole running journal transaction, and
  the chain checkpoint's flush, still before the acknowledgement, made the pointer
  rename durable. The control therefore removes every synchronisation after the
  pointer rename (the session directory's and the checkpoint's flush); it then loses
  acknowledged generations (and leaves a zero-length checkpoint that makes the session
  unreadable), which the harness reports. The protocol itself is unchanged: the
  directory synchronisation is what guarantees the rename, on every filesystem, and
  the checkpoint's flush is an ext4 side effect the protocol does not rely on. The
  control lives behind a development-only `durability-campaign` feature of the
  infrastructure crate (refused without debug assertions and, by the governance check,
  anywhere but a development dependency and the campaign tool's non-default
  `campaign` feature) and the variable `VSIFT_CAMPAIGN_NEGATIVE_CONTROL=1`.
- **Enablement.** `QUALIFIED_UBUNTU_EXT4` is set after the gating run
  ([36340043451](https://github.com/smormah/vsift/actions/runs/36340043451): 11,037
  replay points, 320 kills and 180 injected failures, nothing lost, the control
  losing 54 of 80 acknowledgements) and confirmed by the same campaign on the
  release build with the constant set
  ([36347502530](https://github.com/smormah/vsift/actions/runs/36347502530)).
  `durable_profile` now also reads
  `/etc/os-release` (or `/usr/lib/os-release`; at most 64 KiB, parsed strictly by the
  public `classify_os_release`, fuzzed as `os_release`) and claims `os_crash_durable`
  only for `ID=ubuntu` and `VERSION_ID=24.04` together with ext4 mounts that keep write
  barriers; the decision is the public `qualifies` table, and anything unread or
  unparsed fails closed.
- **Storage failures are not damage (a finding).** Layer C's first full run met an
  evidence call answering `INTEGRITY_FAILURE` after the injected errors: once ext4
  shuts itself down on a write error, reads and opens of committed state fail with
  `EIO`, and the store turned every failure to open or read committed state into an
  integrity failure. A failing disk would then look like tampered evidence. The store
  now maps a storage failure there (`EIO`, a read-only or full filesystem, a stale
  handle, a busy resource, an interrupted call) to `STORAGE_IO`, in session reads and
  in the root's own layout check alike, and keeps an integrity failure for a missing,
  mistyped, linked or malformed entry.
- **Engine request (D-3).** `IngestRequest` gained `durability`; the command line keeps
  asking for ephemeral sessions until P11's durable workspace. An ingest now reports
  its session's own guarantee: before, it reported the store's strongest, which would
  have called an ephemeral session on a qualified root `os_crash_durable` once the
  constant was set.

## Note: D-3 fulfilled and admission weights (P11 PR 2, 2026-09-28)

- **D-3 is fulfilled.** The command line's durable mode is a durable worker workspace
  ([ADR 0021](0021-worker-and-batch-host.md) section 3): `session init-workspace
  --durability durable` on the qualified profile, then `ingest --session-root
  <workspace>`, whose sessions all publish with this record's durable protocol and
  report `os_crash_durable`. `IngestRequest::durability` is now the least a caller
  requires: a durable workspace raises an ephemeral request to durable, and a durable
  request in an ephemeral workspace is refused (`INVALID_ARGUMENT`), never weakened.
  The commit path itself is unchanged, so the crash campaign's evidence still applies.
- **Admission weights.** A recognition attempt (section 4) now reserves its recognizer
  threads of the root's capacity instead of one slot, and that reservation covers the
  chunk decoding between recognitions; the threads are capped at the root's capacity,
  so the recognizer identity, and with it the job and revision identity, can differ on
  a root of smaller capacity (known limit L-023). Contention keeps section 5's retry
  rule for interactive callers; a job host may instead wait a bounded, jittered time
  for admission (`AdmissionWait::Bounded`, at most 60 s), after which the attempt ends
  `BUSY` with the job interrupted and is not retried automatically.

## Note: index markers are published by rename (#197, 2026-09-28)

A session's registration, which comes before the initialization above, created its
index marker in place and then wrote it. A process killed between the two left an
empty marker that failed every later scan of its bucket, and its own cleanup, with
`INTEGRITY_FAILURE`. The marker is now written to `session-index/.registering.tmp`,
flushed and renamed into its bucket, under the root initialization lock every
registration holds; a leftover staged marker is replaced by the next registration.
A durable initialization still synchronises the bucket and the index before it
acknowledges, which now also makes the rename and the staged file's removal
durable. Fault points `registration-marker-create` and `registration-marker-rename`
(`FaultPoint::REGISTRATION`) stop a test child at both boundaries. The commit path is
unchanged.

## Note: the replaced-file retry's budget is counted from the first failed attempt (#314, 2026-10-06)

PR 2's bounded retry for files replaced by rename (above) measured its 500 ms from before the first
attempt. The release candidate's hosted stress campaign on Windows found one read in 140,721 answered
`INTEGRITY_FAILURE` while another thread published generations (none in 200 repetitions on Ubuntu or macOS);
repeating the test on a hosted runner with every CPU busy and diagnostics added reproduced it, one read in 600
repetitions, after a single attempt that took 4.4 s. An attempt that itself outlasts the budget (here a thread
that was not scheduled for seconds; an open call a scanner held would do the same) used it up: the reader had
opened the old pointer, the writer replaced it during the stall, the reader found the file with no link left
(`NotFound`), saw the clock run out and did not try again, and the read path reported the `NotFound` as damage
(a job record, as a job with no record).

The budget is now counted from the first failed attempt. The worst case for a file that really is missing is
the first attempt, then the budget, then the one attempt under way when the budget runs out, so a missing file
is reported after the same time as before when attempts are fast; an attempt that stalls across that deadline
can still end the wait, which needs a replacement and then a stall in the retry. The policy takes its clock
through a small seam (`RetryClock`), so the unit tests move a clock instead of waiting.

No published failure code, schema or field changed. A hard link, a non-regular file, a file still missing or
still refused with access denied (Windows) when the budget is spent, and an I/O error the read path does not
recognise as a failure of the storage are still `INTEGRITY_FAILURE`. That last case includes, on Windows, a file
another process holds without read sharing (os errors 32 and 33): it is answered at once and not waited for,
and a later release should retry it and then report with the same code. It is recorded as known limit L-136 and
was not seen in any campaign.

## Note: a job whose failure can only repeat ends at once, and a checkpoint can hold a verdict (#353, 2026-10-10)

Two additions to sections 4 and 5, proposed with [ADR 0017's note of the same date](0017-local-asr-through-whisper-cpp.md#2026-10-10-note-a-chunk-whose-answer-cannot-be-used-is-a-recorded-gap-not-a-failed-run-353).

- **A failure that a resume can only repeat fails the job at once.** When a run fails because the recogniser's answers were unusable for most of the chunks it answered,
  the job does not wait for the poison rule's third identical failure: the attempt records the failure with its first unusable chunk and the job ends as `failed`
  (checkpoints removed). `job status` then says `resumable: false` with `resumable_reason: failed`, `job resume` refuses with `INVALID_ARGUMENT`, and the same
  `transcript retranscribe` command starts the job anew in a new epoch. Every other failure is classified as before: the retry table, the poison rule and the attempt
  limit are unchanged. Before this, such a job stayed `resumable: true` and a resume could only fail the same way.
- **A chunk checkpoint can be an `unusable` verdict.** Besides `no_audio`, `silent` and `recognised`, a checkpoint can record that the recogniser answered and the
  domain's rules refused the answer as a whole (decoded range and reason, under the payload's digest). A resume reuses it and does not ask the recogniser again.
  The rule for a `recognised` checkpoint is unchanged (S-08): output the rules now reject is discarded and recognised again. A release before this one reads the new
  kind as an unusable checkpoint and redoes the chunk, so nothing breaks on a roll back.
- **A run checks the model's identity before it judges the answers.** A recogniser swapped while the run was in progress (the executable or the model file replaced) made the
  answers those of two models, so the run reports `model_changed`, a failure a retry can fix, and not that most answers were unusable, which ends the job. The order matters
  here and only here: `model_changed` is an ordinary retryable failure of the job, an unusable-answers failure is not.
- **What a supervisor sees when a worker step could not read part of a recording.** In `job run` and `job batch`, a `retranscribe` step whose run had unusable chunks is
  still `complete` with `coverage: null`, as is the request: its outputs name the revision, the generation and the chunks reused, and the worker schema has no member for
  gaps. This is deliberate for 0.2.1 (the worker schema is unchanged) and means a supervisor that reads only the request's result is not told that part of the recording was
  not transcribed. It can tell by reading the revision the step names, `transcript get --revision <revision_id>`, whose data carries the warning
  `provider_chunks_rejected` and `revision.local_asr.unusable_chunks`. The direct commands (`transcript retranscribe`, `job resume`) answer `partial`. A contract test pins the
  step's shape and stops compiling if a member for gaps is added, so the change is made on purpose in a later release, with the schema and these words.

## Consequences

- PR 1 changes no public contract. Warm reads no longer grow with the chain; every
  publication writes one more small file; an ephemeral commit's syscalls are
  unchanged apart from the checkpoint.
- A read no longer notices damage to a generation below its anchor; retain and
  cleanup still do. This is the stated guarantee, not a silent weakening.
- The durable protocol exists and is tested for order, fsyncgate handling and kills,
  but no profile can use it until the campaign passes. PR 1 left the artifact caps
  unchanged (ADR 0019 D4); PR 2 raises them (D-2).
- After a crash between a manifest and its pointer, any later publication replaces the
  unreferenced manifest (PR 2); the retry of the same operation still acknowledges an
  identical one.
- PR 2 raises the artifact caps (D-2); every retranscription is a job with private
  checkpoint files under the session (never in a manifest or bundle), and the public
  `transcript.retranscribe` data gains `job` and the envelope's `operation_id`.
- PR 3 adds the `job status`, `job resume` and `job cancel` commands, two schemas, the
  `jobs` of `session status`, `--operation-id` and trapped interruptions. A long
  command no longer ends at the first Ctrl-C but at its next boundary; a caller that
  needs it gone at once sends a second one.
- PR 4 enables durable sessions on Ubuntu 24.04 / local ext4 through the engine API
  (`IngestRequest::durability`); a host there gets `os_crash_durable` for a durable
  request, and every other profile still answers `MISSING_CAPABILITY`. Losing the disk
  or host stays the caller's responsibility (X-10). The campaign reruns weekly, so a
  kernel, image or store change that breaks durability is found; its pinned cloud
  image must be bumped when Ubuntu retires the dated release.

## Alternatives

- **Cache the verified chain only in memory.** Every CLI command is a new process, so
  it would not help the case #164 measured.
- **Keep one running digest over all manifests.** Changes the manifest format and
  still needs a trusted anchor; the checkpoint is additive.
- **Trust an existing file whose bytes read back correctly.** Reading back returns the
  page cache, which is exactly what fsyncgate makes untrustworthy.
- **A dependency for durable storage (an embedded database).** ADR 0010 alternative 2;
  it does not make external artifact files durable and needs its own review.
