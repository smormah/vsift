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
  surface and signal handling (sections 5-6); PR 4 the crash campaign (section 7).
  Durable publication stays disabled on every profile until PR 4.

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

## Alternatives

- **Cache the verified chain only in memory.** Every CLI command is a new process, so
  it would not help the case #164 measured.
- **Keep one running digest over all manifests.** Changes the manifest format and
  still needs a trusted anchor; the checkpoint is additive.
- **Trust an existing file whose bytes read back correctly.** Reading back returns the
  page cache, which is exactly what fsyncgate makes untrustworthy.
- **A dependency for durable storage (an embedded database).** ADR 0010 alternative 2;
  it does not make external artifact files durable and needs its own review.
