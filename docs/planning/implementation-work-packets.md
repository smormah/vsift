# Implementation work packets

Status: accepted sequence. P00 completed in PR #18 (`924f6c5`); P01 / issue #4 is
next. Tests reference
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

## R0 packets

| Packet | Owning modules and concrete deliverable | Prerequisites | Tests / completion gate |
| --- | --- | --- | --- |
| P00 — Decisions and corpus | Docs, generated fixture definitions, benchmark manifest; accept/replace DEC-01..13; record supported targets and resource profiles | Maintainer review | Requirements map to tests; fixture truth is independent; unresolved decisions block affected packet only |
| P01 — Contract boundary | CLI command/output/config modules; schema files; domain IDs/ranges/errors; setup v1 compatibility; typed cancellation/error presentation | P00 | C-01..10; deterministic CLI tests replace environment-dependent assertions; schema examples all validate |
| P02 — Secure process execution | Infrastructure process supervisor and provider resolution boundary; OS adapters, bounded pipes, env/cwd, tree cleanup; clear effective-control report | P01 | P-01..08, C-05, B-01..03/B-05/B-08 closed; process-flood and descendant tests pass on every supported profile |
| P03 — Storage and coordination | Domain source/job/session IDs; application storage ports; safe filesystem adapter, stable OS locks, admission slots, generation commit and read holds | P01/P02 | S-01..03/S-07/S-08/S-12, X-01..05; crash/flush feasibility evidence reviewed before expanding adapter; no unsafe shortcut |
| P04 — Source and media primitives | Source binding/staging; FFprobe parsing; source timeline; FFmpeg audio/frame operations; provider conformance registry | P02/P03 | M-01..06, V-01; bounded operations with source identity, actual times and allowed protocol policy |
| P05 — Session lifecycle | Open/status/renew/close/clean, expiry, source-inclusive/evidence-only retain, bundle validation, private permissions | P03/P04 | S-04..11; no source deletion, active-session GC race, explicit persistence and restart semantics |
| P06 — Dependency setup | Setup check/plan/install/repair/configure/list/remove/rollback; pinned provider/model manifests; download transport/stager/activation | P02/P03/P04 | D-01..10; fresh-machine and offline BYO flows; no automatic install or elevation; B-04 closed |
| P07 — Transcription | SRT/VTT import and alignment, PCM chunks, whisper.cpp adapter, transcript revisions, bounded records | P04/P05/P06 | T-01..06; measured default model profile; imports avoid unnecessary ASR; chunk seams verified |
| P08 — Candidate/search index | Streaming visual signal extraction, periodic coverage, dedupe with time preservation, local transcript search, cursor paging | P04/P05/P07 | V-02..05, C-03, S-11; fixture recall report and honest gap metadata |
| P09 — Evidence navigation | Exact frames, neighbours, bursts, source audio ranges, native crops, artifact reuse and lineage | P04/P05/P08 | V-01/V-06..08; identical request reuses compatible evidence; requested/actual time and dimensions visible |
| P10 — Recovery integration | Stage checkpoints, operation-key handling, interrupted-job discovery/resume, cancellation/commit ordering and retry policy | P03/P05/P07/P08/P09 | X-01..06/X-09/X-10, S-07/S-08; fault campaign demonstrates no lost acknowledged durable evidence |
| P11 — Worker and batch host | Versioned JobRequest/Result; explicit durable workspace, finite batch reader, process-wide and cross-process admission, graceful shutdown, structured events | P02/P03/P10 | X-07..11, O-01..04, SEC-T01; strict Linux worker profile qualified, repeated external-delivery simulation passes |
| P12 — Agent skill | Generic procedure, model budgets, host image capability check, grounded QA template, checkpoint/resume instructions | P06..P11 | A-01..07; named compact-model trials meet agreed gates; no tool permission expansion; no embedded processing logic |
| P13 — Distribution | Native artifacts and thin npm launcher; architecture selection, notices, SBOM/provenance, signed release plan, upgrade/uninstall docs | P06/P11/P12 | Fresh OS install without Rust; offline/script-disabled recovery; signal/exit forwarding; R-SEC01/R-SEC02 |
| P14 — R0 qualification | Release evidence ledger, fuzz/race/fault/soak runs, findings triage, supported-profile matrix, operator/user docs and release candidate | P00..P13 | All R0 proof links; R-SEC03 and all release gates; public claims match measured support |

### P00/P03 feasibility decisions

These are investigations with small throwaway/test prototypes, not permission to
build the whole system ahead of contracts. Establish safe cross-platform process
containment, handle-relative filesystem operations, atomic commit/flush ordering and
locks without owned unsafe code. Record supported behavior and limitations. If a
required guarantee cannot be demonstrated, propose a narrower supported profile or
a vetted storage/process dependency and obtain design acceptance before continuing.

### P01 configuration precedence

Define immutable effective config once per operation. Proposed precedence: validated
explicit flags -> explicitly selected config -> per-user config -> defaults, all
constrained by host policy. Do not auto-execute/load project-local configuration from
an untrusted working directory. Log only safe effective settings. Media text cannot
set config. Worker requests select approved provider/policy IDs rather than executable
paths. Define unknown-key/version rejection and secret handling before a config command.

### P06 provisioning details

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

Keep the launcher tiny and typed if TypeScript is used. It only selects the correct
native artifact, forwards arguments/signals/stdin/out/err and propagates status.
No application logic in npm scripts. Test spaces/Unicode, unsupported architecture,
optional dependencies omitted, offline execution, broken binary, Ctrl-C and clean
uninstall. The launcher and artifacts have matching versions and verified provenance.

## R1/R2 extension plan

| Packet | Deliverable | Prerequisite and gate |
| --- | --- | --- |
| P15 — OCR/VAD | Optional provider adapters with bounded results, timestamp/region mapping and model provenance | P14; critical number/text accuracy, no OCR-required core path, model resource/licence tests |
| P16 — Embeddings and small vision tags | Versioned enrichment records, opt-in model setup, per-session semantic retrieval | P14/P15; retrieval recall/cost baseline, downgrade behavior, model upgrade/reindex policy |
| P17 — Scroll composition | Geometric overlap/motion estimation, stable-state alignment, provenance masks, refused uncertain joins | P09/P15; labelled scrolling corpus with sticky headers/zoom; no invented pixels/cells; source-frame fallback |
| P18 — Persistent index | Explicit managed catalogue lifecycle, typed ingest/upsert/delete/rebuild contract; optional SQLite adapter | P14; schema migrations, backup/restore, stale source/tombstone/privacy policy, concurrent reader/writer tests |
| P19 — Distributed host | Durable queue/object store, tenant auth, scoped quotas, leases/fencing, dead-letter handling, remote recovery | P11/P18; service threat model, host-loss/network-partition/duplicate delivery tests, tenant isolation |
| P20 — MCP adapter | Tool discovery and image delivery over the same use cases | P14; transport authentication/security and CLI-equivalent behavior; no forked business logic |

No automatic extension is authorized by listing it. Each extension has its own scope
decision and fixtures. SQLite is a possible local catalogue backend, not an implicit
choice for a distributed service or default desktop state.

## Definition of done for every implementation PR

The PR identifies packet/requirements; contains code, typed contracts, meaningful tests,
failure cases and matching docs; lists measured verification and remaining limitations;
includes no unrelated cleanup or silent privilege/persistence change. New dependencies
have reviewed maintenance/licence/target/security impact. Cross-platform behavior is
tested on affected targets. API/schema changes include compatibility fixtures. Security
fixes add regression tests that fail on the previous behavior.

Update `memory/TODO.md` and `memory/project_current_status.md` with state and predecessor
or merged commit references; add the final merge hash in the next record update if it
cannot be known before merge. Never invent a commit hash. Keep work pending until the
observable acceptance criteria pass. Review and merge through existing protected-main
checks. Do not disable checks to complete a packet.
