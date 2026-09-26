# Implementation work packets

Status: accepted R0 sequence with scoped R1 packets, re-planned by
[ADR 0015](../decisions/0015-r0-delivery-replan.md) and
[ADR 0016](../decisions/0016-embeddable-engine-and-evidence-contract.md) on 2026-09-23.
P00-P08 are complete. P06 closes on detection, bring-your-own selection,
verification and guidance; managed installation moved to P13. P09's implementation
is complete across four pull requests (media primitives, the evidence core, the frame
commands, and `crop` and `audio` with the
[qualification record](p09-evidence-navigation.md)), pending their merges and the
ledger completion record; P10 follows. Tests reference
[verification](verification.md), and CI enforces the [delivery ledger](delivery-ledger.json).
Each packet becomes one or more focused issues/PRs before implementation. Splitting
a packet must preserve its contracts and acceptance gate; unrelated feature changes
must not be hidden in a hardening PR.

## Implementation protocol for smaller models

Every assigned issue contains: requirement IDs; accepted decision links; exact owning
modules and allowed dependency direction; typed request/result/error contracts; limits
and lifecycle rules; applicable threat IDs; required test IDs and fixtures; expected
commands/output examples; predecessor commit; documentation to update; explicit done
criteria. Include only the needed context, with links to the authoritative detail.

The implementer first reads the packet, root AGENTS.md, memory status and affected
contracts, then inspects actual code. It implements the smallest complete behavior,
runs the packet tests and required checks, updates records, and creates a reviewable
PR. It may not weaken a limit, broaden permissions, add arbitrary provider flags,
change retention, accept a new dependency or redefine a schema to make tests pass.
Resolve such changes through a design decision with a concrete alternative.

Review high-risk seams before and after implementation: process supervision,
filesystem containment, commit protocol, cleanup, installers, concurrency and releases.
Use a stronger model or qualified human for those reviews; tests remain authoritative.
The reviewer inspects failure paths and tests rather than trusting the implementer's
summary. New model size/version is qualified on an existing packet before relying on
it for a sensitive packet. This workflow does not assume any named model is infallible.

## Dependency sequence

```text
P00 decisions + fixtures
  -> P01 public contracts
  -> P02 process supervision
  -> P03 storage/locking feasibility and implementation
  -> P04 source/media primitives
  -> P05 session lifecycle and bundle export
  -> P06 setup provisioning
  -> P07 transcription
  -> P08 candidates and search
  -> P09 retrieval and source reinspection
  -> P10 recovery/idempotency qualification
  -> P11 worker/batch execution
  -> P12 agent skill and QA evaluation
  -> P13 native/npm distribution
  -> P14 integrated security/load/release qualification
```

The sequence is intentionally conservative for implementation handoffs. Some work
can later proceed concurrently when contracts are stable (for example transcription
and candidate extraction), using isolated branches and explicit integration ownership.
No concurrent work is required or authorized by this document itself.

Beginning with P04, each packet also extends the cumulative
[end-to-end test spine](e2e-test-spine.md). The harness starts as a deterministic,
opt-in mechanical journey and gains real components as their owning packets become
eligible. P12 attaches named agent-client trials; P14 performs release qualification.
This cross-packet test work does not authorize implementing a later packet early.

## R0 packets

| Packet | Owning modules and concrete deliverable | Prerequisites | Tests / completion gate |
| --- | --- | --- | --- |
| P00 — Decisions and corpus | Docs, generated fixture definitions, benchmark manifest; accept/replace DEC-01..13; record supported targets and resource profiles | Maintainer review | Requirements map to tests; fixture truth is independent; unresolved decisions block affected packet only |
| P01 — Contract boundary | CLI command/output/config modules; schema files; domain IDs/ranges/errors; setup v1 compatibility; typed cancellation/error presentation | P00 | C-01..10; deterministic CLI tests replace environment-dependent assertions; schema examples all validate |
| P02 — Secure process execution | Infrastructure process supervisor and provider resolution boundary; OS adapters, bounded pipes, env/cwd, tree cleanup; clear effective-control report | P01 | P-01..08, C-05, B-01..03/B-05/B-08 closed; process-flood and descendant tests pass on every supported profile |
| P03 — Storage and coordination | Domain source/job/session IDs; application storage ports; safe filesystem adapter, stable OS locks, admission slots, generation commit and read holds; durable mode fails closed | P01/P02 | S-01..03/S-07/S-08/S-12, X-01..05; ADR 0010 ephemeral desktop profile passes; no strict durable claim or unsafe shortcut |
| P04 — Source and media primitives | Source binding/staging; FFprobe parsing; source timeline; FFmpeg audio/frame operations; provider conformance registry | P02/P03 | M-01..06, V-01; bounded operations with source identity, actual times and allowed protocol policy |
| P05 — Session lifecycle | Open/status/renew/close/clean, expiry, source-inclusive/evidence-only retain, bundle validation, private permissions | P03/P04 | S-04..11; no source deletion, active-session GC race, explicit persistence and restart semantics |
| P06 — Dependency setup | Detect suitable existing tools; off-PATH BYO executable/model selection; read-only plans; bounded compatibility check of selected FFmpeg/FFprobe against F01 under the reviewed policy limits; model digest check or explicit unverified state; typed manual guidance. Managed installation moved to P13 (ADR 0015) | P02/P03/P04 | D-01, D-07 (unavailable-target guidance), D-09, D-10; preinstalled/partial/off-PATH/denied/offline/unqualified journeys on every named target; no automatic install or elevation; B-04 closed |
| P07 — Engine boundary and transcription | First increment: extract the embeddable engine facade and contract crate with no behaviour change (ADR 0016). Then SRT/VTT import and alignment, segment-identified PCM chunks, whisper.cpp adapter and its functional verification in `setup check`, transcript revisions, bounded records, published transcript schemas, SRT/VTT fuzz targets | P04/P05/P06 | Existing C-suite and schema tests unchanged by the refactor; T-01..06; measured default model profile; imports avoid unnecessary ASR; chunk seams verified |
| P08 — Candidate/search index | Streaming visual signal extraction, periodic coverage, dedupe with time preservation, local transcript search, cursor paging | P04/P05/P07 | V-02..05, C-03, S-11; fixture recall report and honest gap metadata. Implemented by PRs 1-4 (`search`, #148 source binding, visual index core, `candidates`); recall record [p08-candidate-recall.md](p08-candidate-recall.md) |
| P09 — Evidence navigation | Exact frames, neighbours, bursts, source audio ranges, native crops, artifact reuse and lineage | P04/P05/P08 | V-01/V-06..08; identical request reuses compatible evidence; requested/actual time and dimensions visible. Implemented by PRs 1-4 (media primitives, evidence core, `frame get/neighbours/burst`, `crop`/`audio`; [ADR 0019](../decisions/0019-evidence-navigation.md)); qualification record [p09-evidence-navigation.md](p09-evidence-navigation.md) |
| P10 — Recovery integration | Stage checkpoints, operation-key handling, interrupted-job discovery/resume, cancellation/commit ordering and retry policy; Ubuntu/ext4 durable publication qualification | P03/P05/P07/P08/P09 | X-01..06/X-09/X-10, S-07/S-08; owned OS/storage crash campaign demonstrates no lost acknowledged durable evidence before enablement |
| P11 — Worker and batch host | Versioned JobRequest/Result; explicit durable workspace, finite batch reader, process-wide and cross-process admission, graceful shutdown, structured events | P02/P03/P10 | X-07..11, O-01..04, SEC-T01; strict Linux worker profile qualifies only after P10 durable evidence; repeated external-delivery simulation passes |
| P12 — Agent skill | Generic procedure, model budgets, host image capability check, complete local-video investigation, grounded QA template, checkpoint/resume instructions | P06..P11 | A-01..09; named Codex and Claude Code end-to-end trials plus compact-model gates; no tool permission expansion; no embedded processing logic |
| P13 — Distribution and managed installation | Native artifacts and thin npm launcher; architecture selection, notices, SBOM/provenance, signed release plan, upgrade/uninstall docs. Managed dependency installation from ADR 0007/0014: accepted-plan transaction, direct download, staging, smoke before activation, atomic activation, `setup install/repair/list/rollback/remove`, bounded version cleanup, interruption/power-loss qualification, at least one qualified managed-install target. Human-readable terminal output for every command (ADR 0008; the readable terminal text of `cli-v1.md`), assigned 2026-09-26 | P06/P11/P12 | Fresh OS install without Rust; offline/script-disabled recovery; signal/exit forwarding; D-02..D-08; R-SEC01/R-SEC02 |
| P14 — R0 qualification | Release evidence ledger, fuzz/race/fault/soak runs, findings triage, supported-profile matrix, operator/user docs and release candidate | P00..P13 | All R0 proof links; R-SEC03 and all release gates; public claims match measured support |

### P00/P03 feasibility decisions

These are investigations with small throwaway/test prototypes, not permission to
build the whole system ahead of contracts. Establish safe cross-platform process
containment, handle-relative filesystem operations, atomic commit/flush ordering and
locks without owned unsafe code. Record supported behavior and limitations. If a
required guarantee cannot be demonstrated, propose a narrower supported profile or
a vetted storage/process dependency and obtain design acceptance before continuing.
ADR 0010 accepts the narrower profile: P03 qualifies ephemeral desktop publication
and rejects durable mode; P10/P11/P14 own Ubuntu/ext4 durable qualification.

### P01 configuration precedence

Define immutable effective config once per operation. Proposed precedence: validated
explicit flags -> explicitly selected config -> per-user config -> defaults, all
constrained by host policy. Do not auto-execute/load project-local configuration from
an untrusted working directory. Log only safe effective settings. Media text cannot
set config. Worker requests select approved provider/policy IDs rather than executable
paths. Define unknown-key/version rejection and secret handling before a config command.

### P06 provisioning details

Since [ADR 0015](../decisions/0015-r0-delivery-replan.md), P06 owns detection,
selection, verification and guidance; the download, extraction, activation,
repair/rollback and uninstall substeps below are delivered by P13 in the order
recorded at the 2026-09-23 parking checkpoint. The rules still apply unchanged.

Follow [ADR 0014](../decisions/0014-progressive-dependency-setup.md): check
first, install only missing/selected qualified components after separate explicit
plan acceptance, and always offer typed manual/BYO guidance if installation cannot
be completed. A script-installed tool outside `PATH` is a supported explicit-path
case. A provided transcript skips Whisper/model preflight. Headless agent calls
must not prompt, silently elevate, retry an unsafe download or treat evidence as
installation approval. A target with no reviewed artifact must be marked managed
installation unavailable, not given an invented URL. P06 must qualify at least
one complete managed-install target; every named R0 target must have a usable
manual/BYO path.

Split implementation into read-only resolution, plan generation, bounded verified
download, safe extraction, compatibility smoke test, activation, repair/rollback and
uninstall. Each substep has an independent failure fixture. Pin manifests and retain
provenance; active sessions reference immutable runtime/model versions. Updates must
not replace binaries under running jobs. Test resumable downloads with changed
content and failed activation. Package/model redistribution review precedes hosting
artifacts; do not promise one-click installation for an unqualified target.

### P11 operator deliverables

Provide an example supervisor invocation with explicit workspace, resource profile,
deadline, noninteractive policy and job request. Document external queue acknowledgement
ordering, duplicate request handling, restart, cleanup, disk pressure and provider
revocation. Supply a qualified isolated Linux deployment example; native Windows
desktop and optional server-worker support get their own guarantee matrix. Do not
start an HTTP listener or choose a cloud provider in this packet.

### P12 investigation procedure

Skill state machine: CHECK_CAPABILITIES -> PREPARE -> FIND_SPOKEN_SPANS -> INSPECT_CARDS
-> VERIFY_SOURCE -> REFINE_OR_STOP -> REPORT -> CLOSE_OR_RETAIN. Each state specifies
allowed CLI commands and a stopping condition. Set caller-configurable limits for
images, bytes, tool calls, time and refinement depth. Save evidence IDs and a bounded
working summary for resumption. Never assume the model can see a returned image path.

For weak/small models, start with one image and a bounded transcript window, select
lead/lag candidates from provided IDs, verify actual content, then refine. Stronger
models can request larger batches within the same budgets. Unsupported host/image
capability or insufficient evidence produces an honest partial report. The tool does
not autonomously edit the investigated codebase; the user's coding assistant owns
any separately authorized code changes.

### P13 launcher boundary

2026-09-26 (maintainer, with ADR 0019's decisions): human-readable terminal output
moves into P13. ADR 0008 and `docs/contracts/cli-v1.md` promise readable terminal text
without `--json`, while most commands print pretty JSON today; P13 delivers the
readable presentation for every command with its user documentation. The JSON
contracts are unchanged, and no requirement mapping changes.

Keep the launcher tiny and typed if TypeScript is used. It only selects the correct
native artifact, forwards arguments/signals/stdin/out/err and propagates status.
No application logic in npm scripts. Test spaces/Unicode, unsupported architecture,
optional dependencies omitted, offline execution, broken binary, Ctrl-C and clean
uninstall. The launcher and artifacts have matching versions and verified provenance.

## R1 industrial capability expansion

R1 is the managed, industrial expansion of the complete R0 product. Its authoritative
scope, invariants, test identifiers and open decisions are in
[`r1-industrial-capability-expansion.md`](r1-industrial-capability-expansion.md).
Scoping may continue now; implementation remains gated by P14 and the P15 decision
packet.

| Packet | Deliverable | Prerequisite and gate |
| --- | --- | --- |
| P15 — R1 contracts and qualification corpus | Accept catalogue/orchestration/provider decisions; version enrichment, catalogue and industrial job contracts; add independent reconstruction/index/queue/load truth | P14 before implementation; R-15..R-20 map bidirectionally to fixtures, threats and tests; unresolved architectural choices block P16+ |
| P16 — Enrichment pipeline | Optional bounded VAD/OCR/diarization/embedding/tag adapters with versioned records, timestamp/region mapping, confidence and model provenance | P15; E-01..E-08; critical text/number accuracy, absence/failure downgrade, resource/licence and upgrade/reindex gates |
| P17 — Source-grounded composition | Scroll/pan detection, overlap/motion estimation, stable-state alignment, provenance masks and refused uncertain joins | P15/P16 and R0 P09; RC-01..RC-08; sticky-header/zoom/repeated-row corpus; no invented pixels/cells; source-frame fallback |
| P18 — Managed catalogue | Explicit create/inspect/import/remove/rebuild/backup/restore lifecycle; chosen embedded adapter behind an application port | P15/P16; I-01..I-12; opt-in only, schema migration, corruption, stale source/tombstone/privacy and concurrent reader/writer gates |
| P19 — Industrial worker plane | Durable-delivery and artifact-store adapters; leases/fencing, recovery, admission, backpressure, operational surfaces and reference deployment | P15/P18 and R0 P11; H-01..H-12; host-loss/network-partition/duplicate delivery, auth scope and load gates |
| P20 — R1 qualification | Full R0 regression plus integrated security, migration, disaster recovery, load/soak/chaos and two-agent release evidence | P15..P19; Q-01..Q-10 and all R0/R1 gates; claims restricted to measured profiles |

SQLite is a possible embedded single-node catalogue adapter, not the domain contract,
a distributed coordination mechanism or default desktop state. MCP is no longer an
R1 packet: it remains a later optional adapter over the same published use cases.

## Definition of done for every implementation PR

The PR identifies packet/requirements; contains code, typed contracts, meaningful tests,
failure cases and matching docs; lists measured verification and remaining limitations;
includes no unrelated cleanup or silent privilege/persistence change. New dependencies
have reviewed maintenance/licence/target/security impact. Cross-platform behavior is
tested on affected targets. API/schema changes include compatibility fixtures. Security
fixes add regression tests that fail on the previous behavior.

Rewrite `memory/TODO.md` and `memory/project_current_status.md` in the same PR so
they describe the current state in plain English and stay within the size limits
the governance checker enforces. Put verification commands and results in the PR
description; do not open separate evidence-record PRs. Record the ledger merge
commit once, when a whole packet completes. Never invent a commit hash. Keep work
pending until the observable acceptance criteria pass. Review and merge through
existing protected-main checks. Do not disable checks to complete a packet.
