# Proposed architecture and public contracts

Status: accepted R0 design, implemented only through P05. Requirement IDs refer
to [the plan](README.md); later sections still describe future work unless
the [delivery ledger](delivery-ledger.json) marks their packet complete.

## 1. Ownership and module boundaries

Retain the four existing crates. Introduce named feature modules inside them as work
ships; do not pre-create dozens of empty crates or framework abstractions. The domain
uses standard-library value types and has no filesystem, runtime, serialization,
provider, logging, or network dependencies.

| Layer | Modules / concepts | Owned responsibilities |
| --- | --- | --- |
| Domain | source, timeline, evidence, session, job, policy | Validated IDs, ranges, lifecycle transitions, provenance, admission policy values |
| Application | setup, ingest, retrieve, export, recover, run_job | Orchestration, authorization-policy checks, operation outcomes, cancellation flow |
| Infrastructure | process, filesystem, runtime_registry, ffmpeg, whisper, serialization, telemetry | OS APIs, provider adapters, byte formats, physical storage, transport |
| CLI host | commands, config, output, composition | Parse and validate requests, compose adapters, select presentation, return exit status |
| Future hosts | worker service, MCP, index consumer | Adapt external requests to published use cases; never import infrastructure internals |

Application ports are deliberately narrow: `SourceReader`, `MediaProbe`,
`AudioExtractor`, `FrameExtractor`, `Transcriber`, `SessionStore`, `ArtifactStore`,
`JobStore`, `AdmissionController`, `RuntimeResolver`, `RuntimeInstaller`, `Clock`,
`OperationEvents`. Introduce each with its first consumer and conformance tests.
The infrastructure `ProcessSupervisor` is shared by provider adapters and installation
smoke tests. Domain and application never construct FFmpeg arguments.

Published operations return typed results. Expected errors preserve codes and
retryability; diagnostic causes stay in infrastructure. No public service locator,
catch-all JSON payload, stringly typed dispatch, or generic repository for everything.
Rust enums/newtypes implement the user's inward dependency and first-class error rules.

## 2. Identity, time, evidence and capability contracts

| Value | Contract |
| --- | --- |
| SessionId / JobId / ArtifactId / EvidenceId | Distinct opaque validated types; serialized IDs are not paths or access credentials |
| SourceId | Cryptographic digest of source bytes; paths, mtime and file size are locating hints, not sufficient identity |
| OperationKey | Digest of source identity, canonical parameters, schema/algorithm versions, provider/model identity, policy affecting results |
| MediaTime / TimeRange | Integer microseconds on normalized source presentation timeline, half-open [start,end); checked arithmetic; positive duration |
| StreamTime | Original stream index, rational time base, presentation timestamp and timeline offset; preserve conversion provenance |
| CropRect | Integer pixel coordinates in the displayed, orientation-correct source frame; positive size within bounds |
| TranscriptSegment | ID, start/end, text, optional speaker label and provider-specific confidence; language and alignment origin |
| VisualCandidate | ID, actual source time, signal reasons, thumbnail reference, dimensions, coverage window and score semantics |
| EvidenceArtifact | Digest, type, bytes, parent evidence IDs, source range/PTS, transform parameters, provider/model/algorithm versions |
| Coverage | Requested range, inspected/sampled ranges, interval and budget gaps, truncated flag and reasons |
| CapabilityReport | Requirement, provider identity, available/missing/incompatible/degraded, limitations, supported remediation |

Never fabricate a calibrated probability from an uncalibrated provider score. An
unknown confidence is null with its origin, not 1.0. Speaker labels are provider
labels, not verified human identities. Imported transcript timing records the chosen
offset; untimed text cannot support timestamp citations without explicit alignment.

A requested timestamp does not identify a mathematical frame number in variable-rate
video. Default frame policy: first displayed frame at or after the requested time;
report the actual PTS and delta, and reject if no frame meets the caller's tolerance.
Seek to an earlier keyframe then decode to the target as needed. Maintain edit-list,
rotation, nonzero-start and audio/video-offset fixture tests. Time normalization
must be implemented once and used by all providers.

Lossless extraction, crops and deterministic transforms can have byte-level
reproduction guarantees when tool versions match. ASR and accelerated inference may
not be bit-identical across hardware: preserve lineage and outputs, and state that
distinction explicitly.

## 3. Public CLI contract

P01 publishes the namespace and common v1 boundary. Only `setup check` executes today;
every other row is reserved and returns `COMMAND_NOT_IMPLEMENTED` until its owning
packet ships. The exact limits, compatibility rules, schemas, and implementation map
are in the [v1 CLI contract](../contracts/cli-v1.md).

| Command | Purpose / constraints |
| --- | --- |
| `setup check [--profile ...] [--timeout-seconds ...] --json` | Read-only capability detection including existing BYO/managed state; no installation or mutation |
| `setup plan --profile ... --json` | Plan only missing or explicitly selected qualified components; versions, provenance, sizes, licences, permissions, exact actions and digest; typed manual guidance if no qualified install exists |
| `setup install --plan <file> --accept-plan <digest>` | Apply only that validated plan; revalidate expiry and current state; no silent elevation; typed manual fallback on failure |
| `setup repair ...` | Produce/apply a repair plan; same installation contract, no recursive arbitrary deletion |
| `setup list`, `setup remove`, `setup rollback`, `setup configure` | Managed versions and explicit off-PATH user-supplied executable/model registrations; live jobs pin immutable versions |
| `ingest <local-file> [--transcript ...] --json` | Foreground session preparation with checkpoints, explicit source/durability policy |
| `session list/status/close/renew` | Visible lifecycle and bounded storage reporting; close waits/rejects active work |
| `session retain <id> --output <dir> [--include-source]` | Explicit export; distinguish evidence-only and source-inclusive bundle |
| `session clean --expired [--dry-run]` | Bounded scan, claim and quarantine expired owned sessions; no arbitrary source deletion |
| `transcript get <session> --from ... --to ...` | Pageable timestamped text and alignment metadata |
| `transcript retranscribe <session> --from ... --to ...` | New transcript revision; preserve previous citations |
| `search <session> --query ...` | Literal/ranked transcript search; no raw regex or executable query input |
| `candidates <session> --from ... --to ... --limit ...` | Bounded ordered cards, thumbnails optional, stable continuation cursor |
| `frame get <session> --at ...`, `frame neighbours <evidence>` | Source-grounded frame and bounded adjacent states |
| `frame burst <session> --from ... --to ... --max-frames ...` | Finite count, dimensions and total-byte budget; actual timestamps |
| `crop <evidence> --rect ...`, `audio <session> --from ... --to ...` | Bounded source-derived image/audio artifact with lineage |
| `bundle validate <dir>` | Validate schema, contained paths, counts, sizes and hashes without executing embedded content |
| `job run --request <file>`, `job batch --requests <file>` | Versioned noninteractive worker inputs; explicit workspace, finite concurrency and admission |
| `job status/resume/cancel <id>` | Read-only status or explicit lifecycle transition; no implicit detached daemon |

Defer `compose` until R1 accuracy gates pass. `setup` alone displays help and performs
no installation. Unknown commands fail with documented usage errors. Default
noninteractive behavior never prompts indefinitely or changes system dependencies.
User-provided file/config/model content cannot grant installation authority. A
headless or agent call with missing permissions returns a typed reason and manual
recovery path rather than requesting elevation or hanging for a prompt. Only
reviewed component/target artifacts may appear in managed plans; unsupported
targets retain an explicit BYO path. See
[ADR 0014](../decisions/0014-progressive-dependency-setup.md).

### Output and errors

Ordinary `--json` writes exactly one bounded JSON result on stdout. Explicit
`--events jsonl` writes a sequence of versioned progress and terminal records instead;
there is one terminal result with an operation ID. Diagnostics go to stderr. A
broken output pipe triggers bounded cancellation and a documented I/O exit, not a
panic. Data already committed is discoverable by operation ID after a lost response.

Published new-operation envelope shape:

```json
{
  "schema_version": "1",
  "command": "candidates",
  "operation_id": "op_0123456789abcdef",
  "status": "complete",
  "data": {"items": [], "next_cursor": null},
  "warnings": [],
  "error": null,
  "coverage": {"truncated": false, "gaps": [], "reasons": []},
  "lifecycle": {"mode": "ephemeral", "expires_at": "2026-09-10T12:00:00Z"}
}
```

The complete schema and valid fixtures are published under [`schemas/v1`](../../schemas/v1/README.md).
Keep the existing setup v1 fields (`command: setup.check`, readiness status,
dependencies) compatible. Schema versions govern payload shape; command versions,
bundle versions and algorithm versions have separate compatibility policies. Unknown
major versions are rejected; permitted additive fields are tested with old readers.

Errors carry code, safe message, retryable flag, optional retry-after, affected IDs,
and structured remediation containing executable/argument arrays plus required
authority. Never derive remediation commands from media text or provider stderr.

Preserve current setup success/degraded exit 0 and blocked exit 2. Published exits:
0 completed (warnings allowed); 1 unexpected internal failure; 2 usage/config or
missing/incompatible capability; 3 invalid/unsupported source; 4 retryable busy or
provider condition; 5 deadline/resource limit; 6 cancellation; 7 storage/integrity/I/O
failure. The JSON code distinguishes cases sharing an exit. Partially completed batch
results name each outcome and return nonzero if requested work failed. Parser failures
in explicit JSON mode must also have a documented machine-readable error.

Cursors encode version, session, snapshot generation, query hash and last item key;
validate size, scope and expiry. Never embed a raw path. HMAC is not required for a
local CLI with no authorization boundary; future remote hosts bind cursors to caller
scope and provide integrity protection. Stable pages operate on immutable generations.

## 4. Session lifecycle and privacy

P05 implements the ephemeral foreground subset here: 24-hour idle expiry with
a seven-day hard cap, held-lock close/cleanup coordination, a bounded root-local
session index, explicit evidence-only/source-inclusive bundles and data-only
validation. The exact shipped boundary and limitations are in the
[P05 qualification record](p05-session-qualification.md) and
[ADR 0013](../decisions/0013-retained-bundle-publication.md). P07+ preparing,
retrieval, worker persistence and managed catalogue behavior remain future
packets.

Proposed default ephemeral root: private per-user application cache, not the current
working directory. Root registration/configuration and runtime/model installations
are persistent and disclosed separately from media evidence. Session roots are
enumerable and include expiry, bytes, active operations and cleanup eligibility.

Idle TTL proposal: 24 hours, hard maximum seven days. Active command holds protect
work; heartbeat records help diagnosis, but a timestamp alone never authorizes
cleanup. A caller can renew within policy. Expired state is not physically erased
at an exact wall-clock instant without a running process: cleanup runs on subsequent
VSift activity or an explicitly scheduled host maintenance invocation. Report that
fact. No hidden daemon, silent scheduler registration or secure-erasure claim.

Opening returns a session, source binding and state; preparing adds stages; ready
permits retrieval; partial preserves verified successful stages; closing prevents
new holds; closed/expired becomes eligible for quarantine and deletion. Corrupt
ownership metadata fails closed and is reported for explicit inspection.

Retaining writes a new user-selected bundle, validates it, commits it and then marks
the export complete. Never move the authoritative source out of its original path.
`--include-source` copies a verified snapshot; evidence-only bundles disclose that
source re-extraction requires the original matching source. Exporting to an existing
unrelated directory fails; replacing an owned export requires a separate explicit
operation. Closing the temporary session does not delete retained output.

## 5. Source binding and filesystem implementation

Open only regular local files supported by the selected profile. Reject special
devices, FIFOs, network protocol input, ambiguous reserved Windows names and alternate
data streams. Input symlinks may be resolved once to a verified source where policy
allows; writable output, lock, staging and cleanup paths require stronger no-follow
handling. Never equate a string prefix or a preflight canonicalize with race-free
containment.

Choose one explicit source policy: bound existing source (desktop, detects changes)
or private staged snapshot (required for durable repeatability/worker execution).
Hash the opened byte stream. Where a provider cannot consume the same stable handle,
use a verified private snapshot for strict processing. Validate pre/post identity for
best-effort bound sources; explain that metadata checks do not defeat malicious
concurrent same-user modification. Copying and hashing belong to admission budgets.

Storage adapter must operate relative to trusted directory handles using vetted safe
APIs. Windows reparse points/junctions, Unix symlinks, hard links, path substitution,
case folding, long paths, permissions and concurrent renames need native tests. If
safe wrappers cannot provide a required guarantee, gate the capability; adding owned
unsafe code requires an ADR review rather than bypassing the repository rule.

## 6. Durable state without a mandatory database

Use bounded immutable artifact files, immutable per-stage record files, and a
versioned manifest generation. NDJSON records are streamed and checked; committed
files do not accept concurrent appends. A small per-session offset index can support
paging without repeatedly loading all transcripts. It is disposable/rebuildable and
does not index other videos.

Suggested physical layout (names internal, bundle schema public):

```text
private-state-root/
  ownership.json
  coordination/             # stable lock anchors, slot leases; no transcript text
  sessions/<id>/
    current.json            # committed manifest generation and digest
    generations/<n>.json
    records/<digest>.ndjson
    artifacts/<digest>.<type>
    attempts/<operation-id>/
```

All mutations go through a session transaction port. Artifacts are staged under a
unique attempt, bounded and validated, then installed as immutable content. Under
the metadata writer lock, publish a new manifest referencing installed objects and
atomically replace the commit pointer on the same filesystem. Read snapshots under
a lifetime hold. Retain prior committed generation until recovery validation and
reader release permit reclamation. Never replace the lock anchor itself.

The first P03 contract increment represents the requested durability separately from
the strongest publication guarantee qualified for an adapter. Application preflight
constructs the authorization value accepted by the mutating `SessionStore` port only
after the guarantee is satisfied. Storage generations are monotonic and fail on
numeric exhaustion rather than wrapping. This establishes the fail-closed seam; it
does not yet implement the filesystem transaction, artifact/job stores or admission.

The P03 filesystem adapter provisions or opens an explicit owned root at one ambient
authority boundary, validates bounded ownership/layout metadata plus Unix owner/mode
or a Windows DACL allowlist, and uses held directory capabilities thereafter. Stable
single-link lock anchors provide immutable root-wide weighted admission and shared or
exclusive lifetime coordination. Publication acquires admission, shared lifetime, then
the short writer lock; it never waits for a resource while holding the writer lock.

Authorized initialization installs generation zero with a same-filesystem directory
rename. Later publication fences on the expected generation, writes and synchronizes
an immutable manifest, renames it into the generation set, then writes, synchronizes
and atomically replaces the pointer. Recovery verifies a bounded SHA-256-linked chain,
ignores unpublished attempts and permits an identical operation to recover an already
committed result. Missing, multiply linked, malformed, stale, conflicting or future
metadata fails closed. This remains internal and is deliberately not composed into a
command before P04/P05 supply source binding and lifecycle behavior.

Durability modes:

- `ephemeral`: process-crash-consistent publication, best-effort OS cache persistence;
  an OS failure may lose recent work. Not durable job acknowledgement.
- `durable`: explicit workspace; flush written data and manifest, perform qualified
  atomic publication, and persist required metadata before success. Failure to
  satisfy platform guarantees returns an error; no silent downgrade.

The platform adapter must demonstrate process-crash-consistent publication on the
qualified desktop NTFS/APFS profiles. Strict durable acknowledgement is enabled only
after the Ubuntu 24.04/ext4 publication ordering passes an owned disposable OS/storage
crash campaign in P10/P11/P14. Rust exposes synchronization and rename operations, but
portability does not itself establish identical crash guarantees. See
[ADR 0010](../decisions/0010-storage-qualification-gate.md),
[Rust File](https://doc.rust-lang.org/std/fs/struct.File.html) and
[rename](https://doc.rust-lang.org/std/fs/fn.rename.html).

Recovery chooses a verified committed generation, checks referenced objects, ignores
unpublished attempts, and returns interrupted stages to resumable state. Do not infer
success from an artifact filename. Corrupted committed evidence becomes an explicit
integrity failure; preserve remaining valid evidence and diagnostics. Resume verifies
source/config/provider identity before reusing anything.

Do not build a general transaction engine. R0 has single-writer metadata publication,
no cross-session transactions and no network filesystem guarantee. If the feasibility
spike cannot meet the invariant with this narrow design, stop and propose a proven
storage alternative with explicit lifecycle; the deferral of SQLite is not a reason
to ship an unreliable home-built database.

## 7. Multiprocessing, admission and cancellation

Distinguish async I/O, bounded CPU tasks, external decoder/model processes, and
concurrent invocations. Adding Tokio alone does not cover the latter three.

Single invocation: finite channels, cancellation tokens and fixed concurrency;
CPU-heavy owned work uses a bounded blocking pool. Provider thread counts are
explicit. A decoder with eight threads consumes eight CPU reservations, not one.

Across processes: all cooperating invocations for one root use stable OS lock-backed
admission slots. Slot reservations include CPU weight, model memory estimate, GPU
device token and expected disk usage. Root policy prevents an ordinary CLI flag from
raising global limits. Changing policy requires an idle/transactionally coordinated
root. Separate roots do not share those limits; server hosts use cgroups/Job Objects
or their scheduler for machine-wide enforcement.

Lock protocol: never wait for a resource while holding a metadata transaction lock.
Acquire job/admission ownership, then a session lifetime hold, then its short metadata
writer lock when publishing. GC claims exclusive session lifetime access before
closing/quarantining. It never waits indefinitely. Reads may share a lifetime hold;
two compatible extractions can stage in parallel and serialize only publication.
Conflicting state changes return BUSY/STATE_CONFLICT with bounded retry guidance.

OS locks are the authority for live local ownership. Heartbeats, PID and process start
identity help recovery but do not override held locks. Stable lock files are never
unlinked/recreated while references may exist. Generation tokens reject stale
attempt publication. Test suspended processes and PID reuse; do not steal a lease
just because a long CPU pause looks like death.

Process supervisor: trusted absolute executable; explicit args; stdin null unless a
bounded input is needed; safe working directory; allowlisted environment; concurrent
bounded pipe draining; operation and stage deadlines; graceful stop then forced
tree termination; reap and observe completion before releasing leases. The direct
child and descendants need OS containment. Tokio documents child drop/cleanup behavior
but it is not a sandbox: [Tokio process](https://docs.rs/tokio/latest/tokio/process/).

P02 implements this boundary with canonical executable and working-directory types,
an empty-by-default child environment, null stdin, independently capped concurrent
drains, a shared operation deadline, caller cancellation and an effective-control
report. Windows uses a Job Object assigned during suspended creation; Unix uses a
process group. The latter is lifecycle coordination rather than a security sandbox.
Required strict-worker execution is accepted only with a trusted Linux-host report of
inherited container/cgroup filesystem, network, CPU, memory and PID controls; otherwise
the boundary returns `ISOLATION_UNAVAILABLE` before spawn. Managed executable identity,
version compatibility and installation remain P06.

Windows: qualified Job Object lifecycle without breakaway; account for nested jobs
and process assignment races. Linux worker: inherited restricted cgroup/container
with CPU/memory/PID limits and an external supervisor; process groups assist normal
cancellation but alone do not prevent hostile child escape. macOS desktop: verified
process lifecycle and applicable limits, explicitly report missing hard confinement.
Required isolation profiles reject unsupported capabilities. See [Windows Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
and [Linux cgroup v2](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html).

Job states: pending -> admitted -> running -> committing -> succeeded/partial;
running -> cancelling -> cancelled; failed and interrupted are explicit. The commit
transition is serialized with cancellation so a terminal result cannot simultaneously
be cancelled and succeeded. Retrying creates a new attempt under the same logical
job and reuses only verified compatible stage outputs.

Retry only classified transient failures with capped exponential backoff and jitter
within an overall deadline (proposal: at most two retries). Do not retry malformed
media, signature mismatch, policy rejection, deterministic OOM or missing dependencies
without an explicit changed input/policy. Repeated failing provider health can suppress
new admissions for that provider within the host; persistent circuits require an
explicit policy and recovery probe. Shutdown stops admission, drains to deadline,
checkpoints committed work, cancels remaining children, and releases resources.

## 8. Proposed initial budgets

These are starting policies to qualify, not measured capacity claims. All are visible
in effective configuration, validated before work, and tested at boundaries.

| Resource | Desktop proposal | Worker policy |
| --- | --- | --- |
| Concurrent heavy stages | 1 per root | Explicit finite N from measured CPU/memory/GPU capacity |
| Admission queue | 16 requests per batch | Bounded; default fail fast when full; external queue owns backlog |
| Source duration / bytes | 4 hours / 20 GiB | Explicit limits per job; no unlimited default |
| Decoded frame | 16 megapixels maximum | Explicit qualified pixel limit |
| Candidate page | 20 default, 100 maximum | Same schema; configurable downward |
| Burst | 12 default, 100 hard maximum | Same; total bytes/pixels bounded as well |
| Stdout metadata result | 1 MiB maximum | Same; larger data paginated/file artifacts |
| Provider diagnostics | 64 KiB per stream | Hard capture limit; terminate abusive producer |
| ffprobe structured output | 4 MiB maximum | Stream/field/count limits; fail on excess |
| Temporary storage | 10 GiB session, 20 GiB root | Explicit reservation + host disk quota; maintain free-space reserve |
| Stage timeout | Profile-derived with an overall deadline | Caller deadline constrained by host maximum |
| Shutdown | 5 s graceful + 5 s forced cleanup target | Configured supervisor grace must cover this budget |

Long audio must be streamed/chunked: four hours of mono 16 kHz 16-bit PCM is roughly
461 MB before overhead. Never load it all into memory. Enforce output quotas during
writes; estimates and periodic disk checks alone are not hard containment against a
compromised decoder. Worker isolation requires host-enforced limits and reports them.

## 9. Media and retrieval pipeline

1. Resolve capability requirements from the request. An imported transcript avoids
   ASR/model requirements. Audio-less recordings still support visual inspection.
2. Bind/stage the source and probe explicitly selected streams. Bound probe duration,
   metadata size, stream counts and dimensions before deeper decoding.
3. Import SRT/WebVTT initially; explicit sidecar path or explicitly selected embedded
   text track. Do not recursively discover arbitrary files. Preserve original text
   and timing origin; reject or flag malformed alignment. Teams-specific parsing
   requires an adapter and fixtures, not guesses about export format.
4. If transcription is requested, extract known PCM in bounded chunks with overlap.
   whisper.cpp adapter validates output, offsets timestamps to source time and
   deterministically removes overlap duplicates. Preserve provider/model hashes and
   low-confidence/silence warnings. Re-transcription creates another revision.
5. Analyze a downscaled stream sequentially at a bounded sampling rate. Combine scene
   difference, regional change, stable post-scroll states and periodic coverage
   (proposal: 10-second checkpoints plus up to 2 Hz change analysis). Streaming
   processing may decode intervening frames; it need not save or send every frame.
6. Deduplicate similar candidates without dropping the timestamp sequence or mandatory
   coverage markers. A tiny UI change, transient tooltip or one-frame event can evade
   sparse sampling. Report coverage limits and offer an on-demand denser bounded pass.
7. Search transcript text and timestamps first; optional enrichment attaches separately
   versioned hints. The agent tests lead/lag windows and verifies image content.
8. Extract a native-resolution frame/burst/crop/audio range from the bound source,
   recording actual source time, orientation, dimensions, requested parameters and
   any resampling. Upscaling adds no source detail and is labeled as a transform.
9. Return only requested bounded evidence and stable references. A CLI path is useful
   only if the agent host can read images at that path; the skill checks that ability.

FFmpeg adapters own tested demuxer/protocol restrictions, explicit mapping and safe
output templates. No arbitrary filter strings or extra provider flags from evidence
or job requests. Protocol allowlists reduce unexpected I/O but file protocol access
still needs filesystem isolation. See [FFmpeg protocols](https://ffmpeg.org/ffmpeg-protocols.html)
and [FFmpeg command documentation](https://ffmpeg.org/ffmpeg.html). The selected speech
adapter must be qualified against pinned [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
releases and models; a working `--help` is insufficient.

## 10. Worker and indexing extension contracts

P04 implements the internal source/media edge under [ADR 0012](../decisions/0012-p04-source-media-profile.md):
a no-follow local source snapshot in a private P03 session, SHA-256 source identity,
typed stream selection and restricted FFprobe/FFmpeg operations. The media adapter
reports observed frame/audio PTS rather than inferring it from a request. It accepts
only local MP4/Matroska container bytes and the provider `file` protocol; MOV external
references are disabled. This is not yet a public CLI operation or a P05 session
lifecycle. See the [qualification record](p04-media-qualification.md) for limits and
test coverage.

`JobRequest` includes schema version, external correlation/idempotency key, immutable
source binding, requested stages, workspace/lifecycle, resource policy reference,
deadline and desired bundle location. It cannot specify arbitrary executable paths,
environment, shell text, remote callback URLs or elevated policy overrides.

`JobResult` includes logical job and attempt IDs, request digest, source identity,
terminal state, committed generation, bundle reference/digest, completed/missing
capabilities, timings, resource measurements and typed failure. Same key with different
request digest yields IDEMPOTENCY_CONFLICT. Deduplication is local to the explicitly
managed workspace and expires with its documented state lifecycle.

`job batch` streams a finite input list, admits bounded active work and emits per-job
results. It never needs the whole queue in RAM. Queue redelivery, cluster leases,
tenant fairness and dead-letter routing belong to the external supervisor. A result
printed on stdout is not an external queue acknowledgement or remote storage commit.

The retained bundle is the public index handoff: manifest version, source identity,
artifact/segment IDs, normalized timestamps, typed content, provenance, hashes and
capability/coverage flags. Future index consumers use upsert keys plus explicit
delete/tombstone events within their own transaction model. An index can be rebuilt
from retained bundles; it does not alter authoritative evidence. Remote adapters
must add scoped authorization, consistency and deletion semantics before release.

R1 makes this extension explicit through a `CataloguePort`, not through database
types in domain records. A possible SQLite adapter is limited to an explicitly created
single-node catalogue. It is neither a cross-process work queue nor a distributed
coordination strategy, and ordinary desktop ingestion must not create or update it.
Industrial delivery, remote artifact storage and catalogue persistence are separate
ports so each can be replaced and qualified without changing media or evidence logic.
See the [R1 industrial capability expansion](r1-industrial-capability-expansion.md).

Observability: correlated job/attempt/operation IDs, stage duration, admission wait,
bytes processed, queue depth, resource caps, retries, cancellations, provider versions
and failure codes. Export bounded structured events without transcript text, source
paths, credentials or unbounded labels. Telemetry is off-machine only by explicit
configuration. A worker host translates events into its metrics/tracing system.
