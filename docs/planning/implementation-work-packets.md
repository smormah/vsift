# Implementation work packets

Status: accepted R0 sequence with scoped R1 packets, re-planned by
[ADR 0015](../decisions/0015-r0-delivery-replan.md) and
[ADR 0016](../decisions/0016-embeddable-engine-and-evidence-contract.md) on 2026-09-23.
P00-P12 are complete; P13 is in progress (started 2026-09-30, see "P13 scope and
pull requests"). P06 closed on detection, bring-your-own selection, verification and
guidance; managed installation moved to P13. Tests reference
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
| P10 — Recovery integration | Stage checkpoints, operation-key handling, interrupted-job discovery/resume, cancellation/commit ordering and retry policy; Ubuntu/ext4 durable publication qualification | P03/P05/P07/P08/P09 | X-01..06/X-09/X-10, S-07/S-08; owned OS/storage crash campaign demonstrates no lost acknowledged durable evidence before enablement. In progress in four PRs ([ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md), accepted 2026-09-27): commit path (incremental chain validation #164, durable protocol disabled, fault points; merged `e2b14d9`), jobs and checkpointed retranscribe (keys, chunk checkpoints, retry policy, exactly-once commit, engine job operations, `IDEMPOTENCY_CONFLICT`, caps D-2; merged `2ae55be`), public job surface and cancellation (`job status/resume/cancel`, `--operation-id`, trapped SIGINT/SIGTERM and Ctrl-C/Ctrl-Break; merged `8af331b`), Ubuntu 24.04/ext4 crash campaign and durable enablement (dm-log-writes power loss, QEMU kills, dm-flakey errors, negative control; engine-level durable ingest; [record](p10-durable-publication.md); PR 4) |
| P11 — Worker and batch host | Versioned JobRequest/Result; explicit durable workspace, finite batch reader, process-wide and cross-process admission, graceful shutdown, structured events | P02/P03/P10 | X-07..11, O-01..04, SEC-T01; strict Linux worker profile qualifies only after P10 durable evidence; repeated external-delivery simulation passes |
| P12 — Agent skill | Generic procedure, model budgets, host image capability check, complete local-video investigation, grounded QA template, checkpoint/resume instructions | P06..P11 | A-01..09; named Codex and Claude Code end-to-end trials plus compact-model gates; no tool permission expansion; no embedded processing logic. Closed 2026-09-30 by maintainer decision ([ADR 0022](../decisions/0022-agent-skill-and-named-client-qualification.md), Accepted; [qualification record](p12-agent-qualification.md)): the skill `skills/vsift/` and its CLI contract guard, the trial harness, and named-client trials on both clients. The review tier is qualified; the compact tier passed 82%, below the 90% target (L-085) |
| P13 — Distribution and managed installation | Native artifacts and thin npm launcher over per-platform optional packages with no install scripts (see "P13 launcher boundary"); package-name checklist held before release (see "P13 name checklist"); architecture selection, notices, SBOM/provenance, signed release plan, upgrade/uninstall docs. Managed dependency installation from ADR 0007/0014: accepted-plan transaction, direct download, staging, smoke before activation, atomic activation, `setup install/repair/list/rollback/remove`, bounded version cleanup, interruption/power-loss qualification, at least one qualified managed-install target. Human-readable terminal output for every command (ADR 0008; the readable terminal text of `cli-v1.md`), assigned 2026-09-26. Since 2026-09-30 also L-071's parse remediation and `vsift handoff check` (#213, R-13); scope, decisions and pull requests in "P13 scope and pull requests" ([ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md), Proposed). In progress | P06/P11/P12 | Fresh OS install without Rust; install and run through npm, pnpm, Yarn and Bun on every supported target; offline/script-disabled recovery; signal/exit forwarding; D-02..D-08; R-SEC01/R-SEC02; SEC-T02 over human output |
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

2026-09-28 (maintainer): the npm launcher uses per-platform packages, the pattern
esbuild and Biome use.

- One npm package per supported target holds that target's native binary and
  declares `os` and `cpu` (and `libc` if both glibc and musl Linux builds ship), so
  a package manager installs only the matching one. Their names are in the scope on
  the name checklist below.
- The `vsift` package contains only the launcher and its `bin` entry, and lists the
  platform packages as `optionalDependencies` pinned to its own exact version.
- No package has `preinstall`, `install` or `postinstall` scripts. Bun skips
  dependency lifecycle scripts by default, pnpm and Yarn can be set to, and many
  organisations disable them; a launcher that fetched its binary during install would
  install cleanly and then fail at first use. It also keeps ADR 0007/0014's rule that
  installing the package downloads nothing else.
- The launcher finds the installed platform package, checks that its version equals
  its own, and runs the binary through an explicit executable and argument list, never
  a shell. A missing, mismatched or unsupported platform fails with a readable message
  that names the expected package and the supported targets, not a stack trace.
- The launcher runs unchanged under Node.js and Bun (`bunx` honours its `node`
  shebang; `bunx --bun` runs it on Bun), so it uses only `node:` built-ins both
  support. P13 sets the minimum Node.js and Bun versions: Node.js 22 and Bun 1.2
  (ADR 0023, 2026-09-30).
- Qualification installs and runs the release candidate on every supported target
  with npm, pnpm, Yarn and Bun, both globally and one-shot (`npx`, `pnpm dlx`,
  `yarn dlx`, `bunx`), including with install scripts disabled, alongside the cases
  above. The native binaries stay downloadable from GitHub Releases for users with no
  JavaScript runtime.

### P13 name checklist

2026-09-28 (maintainer): every name VSift will be published under is held by the real
release before it is announced anywhere. Public promotion starts only after P14. An
availability observation is not a reservation (ADR 0009), and no placeholder package
is published, so a name is held only once the release workflow publishes to it.

2026-09-30 (maintainer, ADR 0023 decisions A and B): the names are chosen, crates.io
and native installers are not in R0, and the state column is updated.

| Channel | Name(s) | State on 2026-09-30 | Note |
| --- | --- | --- | --- |
| npm package | `vsift` | Not found by an anonymous `npm view` on 2026-09-10 and 2026-09-30; not held | ADR 0009; a maintainer-approved scoped name is the fallback |
| npm scope | `@<scope>`: to be confirmed by the maintainer; candidates `@vsift-cli`, then `@vsifthq`, then `@vsiftdev` | `@vsift` unavailable: npm refused the organisation name `vsift` on 2026-09-30 (organisation and user names share one namespace) | Holds the per-platform packages; the choice is recorded in an ADR 0009 note |
| npm platform packages | `@<scope>/win32-x64`, `@<scope>/darwin-arm64`, `@<scope>/linux-x64` | Not held; checked once the scope is chosen | One per R0 target (ADR 0023 decision D) |
| crates.io | `vsift`, `vsift-contract` | Not in R0 | Names confirmed in ADR 0016; no crate is published in R0 (ADR 0023 decision B) |
| crates.io | `vsift-domain`, `vsift-application`, `vsift-infrastructure` | Not in R0 | crates.io needs every dependency of a published crate published too |
| crates.io | `vsift-cli` | Not in R0 | Only if `cargo install` is offered; maintainer decision |
| GitHub | `smormah/vsift` repository and its Releases | Held | Hosts the native binaries; whether a `vsift` organisation is wanted is a maintainer decision |
| Native installers | winget, Scoop, Homebrew, Debian/Ubuntu package | Not in P13 | ADR 0001 says "appropriate native installation methods" without naming them; for R0 these are the GitHub Releases archives (ADR 0023), and each installer joins this table when chosen |

- P13 checks each name with that registry's own client (`npm view`, `cargo info`,
  `gh`), sending no personal contact details.
- Publishing identities use two-factor authentication and trusted publishing where the
  registry supports it; P13 verifies account and scope ownership before publication.
- A name found taken is a maintainer decision recorded in ADR 0009, never a silent
  rename.

### P13 scope and pull requests

2026-09-30 (maintainer): P13 started, and every recommendation of the P13 plan was
accepted. The decisions (A-H), the planned contract changes, the exclusions and the
maintainer-only actions are in
[ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
(Proposed). The ledger maps P13 to R-03, R-13 and R-14.

**Scope.**

1. Native artifacts and the release pipeline: `.github/workflows/release.yml` builds
   Windows x64 (MSVC, static C runtime), macOS 15 arm64 and x64 glibc Linux on Ubuntu
   22.04, with licences, `THIRD-PARTY-NOTICES`, a CycloneDX SBOM per target,
   `SHA256SUMS`, a same-runner reproducibility check, a commit suffix on `--version`,
   Sigstore build-provenance attestations, a protected `release` environment, npm
   trusted publishing and `dry_run` by default; plus a governance lint of workflows
   for R-SEC01.
2. The npm launcher (see "P13 launcher boundary"): a plain CommonJS `bin/vsift.cjs`
   over the three platform packages, qualified on a local Verdaccio registry.
3. Managed installation on Ubuntu 24.04 x64, in the P06 resume order, with `setup
   install/list/rollback/remove/repair`, `--artifact-dir`, bounded cleanup, a
   stale-stage sweep, kill and power-loss qualification, D-02..D-08 and
   `DOWNLOAD_FAILED`.
4. Human-readable output by default through a `TerminalText` builder, and the SEC-T02
   rerun over it (L-016's display, L-017, L-073).
5. L-071: a typed parse remediation that never echoes argument text.
6. `vsift handoff check` (#213), a `free` skill command.
7. Documentation: ADR 0023 and its notes, `cli-v1.md`, verification, threat model,
   known limits, `docs/operations/install.md` and `docs/operations/release.md`, the
   record `docs/planning/p13-distribution.md` and the P13 stage of the E2E spine.

**Pull requests.** Each is a coherent increment with its code, tests, documentation
and handoff updates. Nothing is published until PR 10's publish step runs with the
maintainer's approval at completion.

| PR | Content | Depends on |
| --- | --- | --- |
| 0 | ADR 0023 and the kickoff (ledger, traceability, issue #16, notes) | P12 complete |
| 1 | L-071 typed parse remediation | 0 |
| 2a | `crates/vsift-cli/src/human/`, the `TerminalText` builder and the first command renderers | 0 |
| 2b | The remaining renderers and the SEC-T02 human-output rerun | 2a |
| 3 | The production smoke executor and its failure cleanup | 0 |
| 4 | The guarded install transaction, managed lookup tier, `setup install`, `--artifact-dir`, `DOWNLOAD_FAILED` and the in-place contract values | 2a, 3 |
| 5 | `handoff check`, `vsift-contract::handoff`, the skill's two input forms | 2a and the P12 debt pull requests (skill, guard and grader overlap) |
| 6 | `setup list/rollback/remove/repair`, bounded cleanup and the stale-stage sweep | 4 |
| 7 | Kill and power-loss tests of the managed store, and the P13 install E2E stage | 6 |
| 8 | `release.yml` and the governance workflow lint | 0 |
| 9 | The npm packages and the Verdaccio matrix | 8 |
| 10 | Attestation and publish wiring | 8, 9; the maintainer's environment and trusted publishers |
| 11 | Documentation, the qualification record and completion | 1-10 |
| 12 | The ledger follow-up with the merge commit (governance rule 9) | 11 |

The compact tier's re-run (#222) follows PR 5 (decision F).

**Not in P13:** P14's release qualification, managed installation on Windows and
macOS, native installers, crates.io, the MCP adapter, SEC-T01 (L-068) and the P12
debt #218-#224.

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
