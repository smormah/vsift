# R1 industrial capability expansion

Status: scoped expansion boundary; not authorized for implementation before P14 and
the P15 decision gate. Date: 2026-09-11.

## Outcome

R1 turns the R0 video-evidence engine into an explicitly managed, production-operated
capability for repeated ingestion and search. It adds durable catalogue lifecycle,
optional machine-assisted enrichment, source-grounded reconstruction, and a scalable
worker integration surface without changing the default disposable desktop workflow.

R1 is an expansion of a working product, not the point at which VSift first becomes
useful. R0 must already prove this complete user journey on a real local recording:

1. A coding agent with local command execution checks or plans dependencies.
2. The agent ingests a supplied video into a disposable session.
3. VSift imports a matching transcript or produces a timestamped local transcript.
4. The agent searches speech and navigates bounded visual candidates.
5. It retrieves exact frames, neighbours, bursts, crops, or audio ranges as needed.
6. It produces a grounded handoff with resolvable evidence references and honest gaps.
7. The session is closed and cleaned, or retained only through explicit user intent.

The R0 release candidate must run this journey through two independent coding-agent
clients: OpenAI Codex and Claude Code, or documented successor clients with equivalent
local shell and image-reading abilities. The ordinary ChatGPT or Claude web interface
cannot invoke a local executable without a local bridge; R0 does not pretend otherwise.
VSift remains provider-neutral and does not require its own hosted-model API key.

## Boundary between R0, R1 and R2

| Release | User-visible promise | Persistence and scale boundary |
| --- | --- | --- |
| R0 | Give a compatible coding agent a local video and obtain a timestamped transcript plus source-grounded visual evidence | Disposable desktop sessions by default; explicit retained bundles; bounded, recoverable single-host worker/batch execution |
| R1 | Manage and enrich retained video evidence repeatedly, search across videos, reconstruct supported scrolling views, and run an operated worker plane | Explicit managed catalogue; one qualified embedded node plus horizontally scalable worker integration; no hidden desktop persistence |
| R2 | Expose VSift as a broader service or ecosystem integration | Public network API, mutually untrusted multi-tenancy, fleet control plane, and optional MCP adapter after independent threat and conformance review |

“Industrial” means measured durability, bounded load, restart safety, operability and
clear ownership. It does not mean advertising arbitrary throughput, exactly-once
delivery, universal codec support, or hostile multi-tenant isolation without evidence.

## Invariants inherited from R0

R1 may add storage and models, but it may not weaken these rules:

1. Original audiovisual media remains authoritative and is never modified or deleted
   by extraction, indexing, reconstruction, rollback, repair, or catalogue cleanup.
2. Generated transcript, OCR, tags, embeddings, summaries and composites are derived
   evidence with versioned provenance, nullable confidence and coverage gaps.
3. A one-off desktop investigation creates no persistent cross-video record unless the
   caller explicitly selects a managed catalogue or retains and imports a bundle.
4. The CLI and versioned JSON contracts remain the canonical public automation API.
   Skills, services and future MCP adapters call the same application use cases.
5. Every queue, page, retry, scan, model request, record, artifact and resource use is
   bounded. Required unavailable guarantees fail before mutation and never downgrade.
6. A catalogue can be rebuilt from validated retained bundles and source bindings. The
   index is never the sole copy of authoritative evidence.
7. R1 profiles must continue to pass the complete R0 agent journey and its security
   regression suite.

## R1 requirements

| ID | Requirement | Minimum proof |
| --- | --- | --- |
| R-15 | Optional bounded enrichment: VAD, OCR, diarization where supported, embeddings and coarse visual tags | Accuracy/resource/licence evaluation; typed partial coverage; core path works when every enrichment provider is absent |
| R-16 | Source-grounded scroll/pan composition with explicit refusal and frame fallback | Labelled reconstruction corpus; pixel/region lineage; no invented rows, cells or values |
| R-17 | Explicit managed cross-video catalogue with create/inspect/import/remove/rebuild/backup/restore lifecycle | Migration, rebuild, corruption, deletion, privacy and concurrent-reader/writer campaigns |
| R-18 | Industrial job control for repeated and horizontally scalable worker execution | Durable delivery simulation, leases/fencing, backpressure, host-loss/network-partition and duplicate-delivery tests |
| R-19 | Production operations and security for the qualified R1 deployment profiles | Authentication boundary, scoped storage, observability, upgrade/rollback, disaster recovery, load/soak/chaos and incident runbooks |
| R-20 | R0 functional parity remains intact through at least two independent coding-agent clients | Full fresh-video lifecycle on named Codex and Claude Code versions plus deterministic mechanical evidence validation |

R-20 measures tool use, citations and lifecycle behavior separately from whether a
model reaches the same subjective diagnosis as a human. A model may conclude that the
evidence is insufficient; it may not invent missing evidence or silently skip the
transcription and visual-inspection stages.

## Capability design

### Managed catalogue

The catalogue is an application port over validated retained bundles. Its public
contract owns source identity, bundle version, enrichment versions, timestamps,
capability/coverage flags, lifecycle state, and upsert/tombstone/rebuild events. It
does not expose database row IDs or allow adapters to become the domain model.

An embedded SQLite adapter is the preferred first R1 candidate for one explicitly
managed local node because it offers transactions, migrations, crash recovery and a
small operational footprint. It is not selected for distributed coordination and is
never opened merely because a user ran `vsift ingest`. The P15 decision packet must
compare it with a versioned file catalogue and a client/server database against:

- single-writer and concurrent-reader behavior;
- transaction and filesystem durability on qualified profiles;
- schema migration, online backup, restore and rebuild;
- corruption detection and repair boundaries;
- bounded query/scan behavior and index size;
- licence, maintenance, Rust safety and supply-chain posture.

Catalogue creation requires an explicit command/configuration and visible root. Its
status reports location in privacy-safe form, schema version, logical size, retention
policy, last verified backup and outstanding repair/reindex work. Removal is a scoped,
recoverable plan that distinguishes catalogue metadata, derived artifacts, retained
source copies and externally owned source media.

### Enrichment

Enrichment is a set of replaceable providers behind typed application ports. Every
record includes provider/model identity, input source or frame IDs, timestamp or image
region, transformation version, confidence origin, and coverage status. Model upgrades
create new revisions; they do not reinterpret an old record in place.

VAD narrows speech work but cannot remove audio from source access. OCR and tags aid
search but never promote text or labels to source truth. Embeddings are namespaced by
model and normalization version. Searches declare which capabilities participated so
a result from a partially enriched catalogue is not presented as exhaustive.

### Reconstruction

Composition operates on ordered source frames, motion/overlap estimates and stable
regions. A composite contains a provenance map back to contributing frame regions,
records masked/occluded areas, and refuses joins below a qualified threshold. Sticky
headers, cursors, animation, zoom, horizontal movement and repeated rows are explicit
test classes. Native source frames remain directly retrievable when composition fails.

### Industrial worker plane

R0 supplies a finite single-host job/batch contract. R1 adds adapters for durable
delivery and remote artifact storage without moving queue semantics into media code.
Application contracts cover request digest, attempt identity, lease and fencing token,
checkpoint compatibility, terminal outcome, bundle commit receipt, retry class and
dead-letter reason.

The first R1 reference deployment is a managed single-organization worker plane. It
may scale workers horizontally, but the chosen queue, catalogue and object store need
their own accepted ADR and qualification. Queue acknowledgement follows validated
remote bundle/catalogue commit. At-least-once delivery plus idempotent publication is
the default claim; exactly-once is not claimed. Public internet ingress and mutually
untrusted multi-tenancy remain R2 unless deliberately brought forward with SEC-T03.

### Operations

The operated profile publishes bounded structured events and metrics for admission,
queue age/depth, stage latency, retries, provider health, resource pressure, catalogue
lag, reconstruction refusal, storage errors and terminal outcomes. Labels exclude
transcript text, filenames, source paths, credentials and unbounded user values.

Readiness distinguishes unable-to-admit, dependency-degraded, storage-unavailable,
recovery-in-progress and healthy. Upgrade and rollback preserve readable prior bundle
and catalogue formats. Backup/restore, reindex, drain, capacity planning, incident
response and secure decommissioning receive executable runbooks before release.

## Work packets

The packet numbers P15-P20 are reserved for R1. Scoping may proceed before R0 is
finished, but implementation may not start until P14 is complete and P15 accepts the
remaining architectural choices.

| Packet | Deliverable | Dependencies | Gate |
| --- | --- | --- | --- |
| P15 — R1 contracts and qualification corpus | Accept storage/orchestration/provider ADRs; version catalogue/enrichment/job contracts; add reconstruction, index, queue and load truth | P14 for implementation | R-15..R-20 map bidirectionally to fixtures, threats and tests; no unresolved decision affects P16+ |
| P16 — Enrichment pipeline | Bounded VAD/OCR/diarization/embedding/tag providers and revisioned records | P15 | E-01..E-08; absence/failure preserves R0; model and licence inventory passes |
| P17 — Source-grounded composition | Scroll/pan detection, alignment, provenance masks, confidence and refusal | P15/P16 and R0 P09 | RC-01..RC-08; labelled corpus gates; source-frame fallback always available |
| P18 — Managed catalogue | Lifecycle commands/use cases, chosen embedded adapter, migrations, backup/restore, rebuild and deletion reconciliation | P15/P16 | I-01..I-12; explicit opt-in; corruption/concurrency/privacy campaigns pass |
| P19 — Industrial worker plane | Durable delivery and artifact-store ports/adapters, leases/fencing, recovery, admission, operational surfaces and reference deployment | P15/P18 and R0 P11 | H-01..H-12 plus SEC-T03 where applicable; duplicate/partition/host-loss/load tests pass |
| P20 — R1 qualification | Integrated compatibility, security, migration, disaster-recovery, load/soak/chaos and two-agent release evidence | P15..P19 | Q-01..Q-10; all R0 and R1 gates pass; public claims match measured profiles |

MCP is deliberately not a P15-P20 packet. It remains a thin optional R2 adapter over
published use cases and cannot displace the CLI/skill as the primary integration.

## Verification outline

### Enrichment (E)

- E-01..02: timestamp/region accuracy, malformed output, confidence and coverage gaps.
- E-03..04: provider absence, cancellation, timeout, OOM and incompatible revision.
- E-05..06: critical spreadsheet values/error codes and multilingual speech evaluated
  independently from the provider under test.
- E-07..08: model upgrade/reindex behavior, bounded resources, provenance and licences.

### Reconstruction (RC)

- RC-01..03: slow/fast scroll, vertical/horizontal pan, sticky headers and repeated rows.
- RC-04..05: zoom, animation, cursor/selection overlays and temporarily hidden regions.
- RC-06: every output region maps to source pixels; no unsupported interpolation.
- RC-07: uncertain or contradictory alignment refuses and exposes source-frame fallback.
- RC-08: output dimensions, frame count, memory, time and artifact bytes remain bounded.

### Index lifecycle (I)

- I-01..03: create/import/upsert, duplicate bundle, changed source and version conflict.
- I-04..05: explicit tombstone/delete, source disappearance and retained-source policy.
- I-06..07: rebuild equivalence, partial/corrupt bundle and interrupted reindex.
- I-08..09: forward/backward migration, interrupted migration, backup and restore.
- I-10: concurrent readers/writers and bounded pagination under catalogue growth.
- I-11: privacy-safe status/logging and no implicit desktop catalogue creation.
- I-12: damaged database/artifact mismatch fails safely and preserves repair evidence.

### Industrial host (H)

- H-01..03: duplicate/out-of-order delivery, expired lease and stale fenced attempt.
- H-04..06: queue/storage outage, network partition and commit-before-ack ordering.
- H-07: host death during every stage and recovery on a different worker.
- H-08: bounded backpressure, admission fairness and retry jitter under overload.
- H-09: malicious job cannot select executable, policy, credential or arbitrary storage.
- H-10: scoped authorization prevents cross-job/tenant access where tenancy is enabled.
- H-11: rolling upgrade/rollback across compatible schema and provider revisions.
- H-12: drain/shutdown leaves acknowledged results valid and remaining work recoverable.

### Integrated qualification (Q)

- Q-01..02: the complete R0 lifecycle still passes through named Codex and Claude Code
  clients with and without a supplied transcript.
- Q-03: 1/2/4/8 worker load ladder, bounded 100-job batch and declared saturation point.
- Q-04: 1,000-job/eight-hour mixed-workload soak with stable handles, memory and disk.
- Q-05: storage, process, network and dependency fault campaign with no false completion.
- Q-06: catalogue backup/restore/rebuild and disaster-recovery time/data-loss evidence.
- Q-07: prompt-injection and poisoned-metadata corpus causes no unauthorized action.
- Q-08: dependency, SBOM, provenance, advisory and native-runtime review.
- Q-09: fresh deployment, upgrade, rollback, decommission and privacy erasure runbooks.
- Q-10: published capacity/support matrix contains only measured environments and loads.

Targets such as throughput, recovery time, recovery point and maximum catalogue size
are calibrated in P15 on named hardware and storage. No number is invented during
planning and no generic “enterprise scale” claim is permitted.

## Threat additions for R1

| ID | Threat | Required control and proof owner |
| --- | --- | --- |
| SEC-26 | Implicit or forgotten persistent collection | Explicit catalogue selection, visible lifecycle/status and I-11 regression |
| SEC-27 | Stale index, tombstone loss or incomplete privacy erasure | Transactional lifecycle events, reconciliation/rebuild and I-04..I-07 |
| SEC-28 | Poisoned OCR/tags/embeddings steer an agent or ranking | Untrusted labels, source verification, capability disclosure and Q-07 |
| SEC-29 | Reconstruction fabricates or duplicates visible facts | Pixel provenance, confidence/refusal and RC-01..RC-08 |
| SEC-30 | Queue replay or stale worker publishes conflicting results | Request digests, leases/fencing, immutable commits and H-01..H-07 |
| SEC-31 | Remote integration leaks credentials or accesses arbitrary locations | Scoped identities, fixed endpoints, no request-selected credentials/URLs and H-09 |
| SEC-32 | Search/index cardinality or model work exhausts resources | Admission, quotas, bounded pages/scans/labels and E/I/H load tests |
| SEC-33 | Backup, restore or migration exposes or corrupts sensitive evidence | Encrypted/scoped storage policy, integrity verification and I-08/I-09/Q-06 |
| SEC-34 | Cross-job or cross-tenant evidence disclosure | Authorization on every operation, storage partitioning and H-10/SEC-T03 |
| SEC-35 | Telemetry leaks transcript, filenames, paths or secrets | Allowlisted low-cardinality fields, redaction tests and Q-07/Q-09 |

## Explicit exclusions

R1 does not automatically acquire remote videos, scrape authenticated sites, ship a
public SaaS, train foundation models, promise perfect transcription, or make generated
metadata authoritative. It does not require every optional model or create a catalogue
during ordinary desktop ingestion. A public network API, hostile multi-tenancy and MCP
remain R2 decisions unless an accepted scope ADR moves them with their full security
and verification cost.

## Decisions still required at P15

1. Select and qualify the embedded catalogue adapter; SQLite is a candidate, not a
   foregone conclusion.
2. Select the R1 reference queue, remote artifact store and catalogue topology, or
   narrow P19 to published ports plus a single-host reference deployment.
3. Define source-retention, erasure, backup encryption and recovery objectives.
4. Select optional enrichment providers only after accuracy, licence, footprint and
   security evaluation.
5. Calibrate throughput, catalogue-size, recovery and resource targets on owned hosts.
6. Decide whether any mutually untrusted tenancy is included; if so, SEC-T03 and a
   dedicated service threat model become release-blocking.

