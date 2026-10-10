# Implementation work packets

Status: accepted R0 sequence with scoped R1 packets, re-planned by
[ADR 0015](../decisions/0015-r0-delivery-replan.md) and
[ADR 0016](../decisions/0016-embeddable-engine-and-evidence-contract.md) on 2026-09-23.
P00-P13 are complete (P13 started 2026-09-30 and published the 0.1.0 pre-release on
2026-10-01, see "P13 scope and pull requests"); P14 is in progress (started by the
maintainer on 2026-10-02, see "P14 scope and pull requests"). P06 closed on detection, bring-your-own selection, verification and
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
| P12 — Agent skill | Generic procedure, model budgets, host image capability check, complete local-video investigation, grounded QA template, checkpoint/resume instructions | P06..P11 | A-01..09; named Codex and Claude Code end-to-end trials plus compact-model gates; no tool permission expansion; no embedded processing logic. Closed 2026-09-30 by maintainer decision ([ADR 0022](../decisions/0022-agent-skill-and-named-client-qualification.md), Accepted; [qualification record](p12-agent-qualification.md)): the skill `skills/vsift/` and its CLI contract guard, the trial harness, and named-client trials on both clients. The review tier is qualified; the compact tier passed 82%, below the 90% target (L-085). The compact re-run after the debt fixes (#222, 2026-09-30, on `a0bfb06`) met it: 93% and 100%; L-085 closed |
| P13 — Distribution and managed installation | Native artifacts and thin npm launcher over per-platform optional packages with no install scripts (see "P13 launcher boundary"); package-name checklist held before release (see "P13 name checklist"); architecture selection, notices, SBOM/provenance, signed release plan, upgrade/uninstall docs. Managed dependency installation from ADR 0007/0014: accepted-plan transaction, direct download, staging, smoke before activation, atomic activation, `setup install/repair/list/rollback/remove`, bounded version cleanup, interruption/power-loss qualification, at least one qualified managed-install target. Human-readable terminal output for every command (ADR 0008; the readable terminal text of `cli-v1.md`), assigned 2026-09-26. Since 2026-09-30 also L-071's parse remediation and `vsift handoff check` (#213, R-13); scope, decisions and pull requests in "P13 scope and pull requests" ([ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md), Accepted 2026-10-01). **Complete (2026-10-01):** pull requests 0-11 and the release-prep change are merged, the maintainer published the 0.1.0 pre-release under npm's `next` and on GitHub Releases, and the completion change (PR 12) records it ([qualification record](p13-distribution.md)) | P06/P11/P12 | Fresh OS install without Rust; install and run through npm, pnpm, Yarn and Bun on every supported target; offline/script-disabled recovery; signal/exit forwarding; D-02..D-08; R-SEC01/R-SEC02; SEC-T02 over human output |
| P14 — R0 qualification | Release evidence ledger, fuzz/race/fault/soak runs, findings triage, supported-profile matrix, operator/user docs and release candidate; the named-agent clean-install run includes a cold-agent variant (CLI on `PATH`, no skill, no docs; maintainer decision 2026-10-02). **In progress since 2026-10-02** (the maintainer confirmed decisions A-H and started it): scope and pull requests in "P14 scope and pull requests" below, decisions in [ADR 0024](../decisions/0024-r0-qualification-and-release-candidate.md) (Proposed until completion) | P00..P13 | All R0 proof links; R-SEC03 and all release gates; public claims match measured support |

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

2026-09-30 (P13 PR 9): implemented as the ADR 0023 PR 9 note records. The `vsift-cli`
package also carries the skill (decision H7) and `platform-digests.json` (H5) beside the
launcher; a launcher failure exits 127 (no platform package) or 126 (refused or cannot
start); the qualification is the Release workflow's `npm-qualify` matrix, with Yarn
through a project install because Yarn 4 has no global one (L-092).

### P13 name checklist

2026-09-28 (maintainer): every name VSift will be published under is held by the real
release before it is announced anywhere. Public promotion starts only after P14. An
availability observation is not a reservation (ADR 0009), and no placeholder package
is published, so a name is held only once the release workflow publishes to it.

2026-09-30 (maintainer, superseding the placeholder rule above for `vsift` only): the
scope is `@vsift` (the maintainer owns the organisation `vsift`; a brief same-day
choice of `@shongo` is withdrawn), and the maintainer
personally publishes a placeholder `vsift@0.0.0` to hold the launcher name
(ADR 0009 note, ADR 0023 decisions A and B amendments).

2026-09-30 (maintainer, later the same day): npm refused the unscoped `vsift` as too
similar to `sift` and `tsify`, so the launcher package is `vsift-cli`, held by the
maintainer's placeholder `vsift-cli@0.0.0`; the command stays `vsift` and the scope stays
`@vsift` (ADR 0009 note, ADR 0023 decision A amendment). Read "the `vsift` package" in the
launcher boundary above as `vsift-cli`.
2026-09-30 (maintainer, ADR 0023 decisions A and B): the names are chosen, crates.io
and native installers are not in R0, and the state column is updated.

| Channel | Name(s) | State (2026-09-30; updated 2026-10-01) | Note |
| --- | --- | --- | --- |
| npm package | `vsift` | **Refused by npm** (2026-09-30): E403 "Package name too similar to existing packages sift, tsify" on the maintainer's placeholder publish; never published. Anonymous `npm view` had found it free on 2026-09-10 and 2026-09-30 | Not used. ADR 0009 note "npm refused `vsift`": a not-found lookup is not availability |
| npm package (launcher) | `vsift-cli` | **Held** since 2026-09-30 by the maintainer's placeholder `vsift-cli@0.0.0` (`latest`); **0.1.0 published 2026-10-01 under `next`** | Installs the `vsift` command; ADR 0023 decision A amendment and ADR 0009 note of 2026-09-30 |
| npm scope | `@vsift` (maintainer, 2026-09-30) | Held: the maintainer owns the organisation `vsift`. The earlier "not available" came from a repeated submission after the first one had created it | Holds the per-platform packages; ADR 0009 2026-09-30 note |
| npm platform packages | `@vsift/win32-x64`, `@vsift/darwin-arm64`, `@vsift/linux-x64` | **Held** by `0.0.0` placeholders (the maintainer, 2026-10-01, `latest`); **0.1.0 published 2026-10-01 under `next`** by the Release workflow | One per R0 target (ADR 0023 decision D) |
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
- State read on 2026-10-01 before the publish (anonymous registry reads; nothing changed):
  `vsift-cli` had one version, `0.0.0`, as `latest`, with no command; the three platform
  packages were not found and were then held by `0.0.0` placeholders (path A, ADR 0009
  note of 2026-10-01). After the publish (read the same day, anonymously): all four
  packages have `latest` `0.0.0` and `next` `0.1.0`.

### P13 scope and pull requests

2026-09-30 (maintainer): P13 started, and every recommendation of the P13 plan was
accepted. The decisions (A-H), the planned contract changes, the exclusions and the
maintainer-only actions are in
[ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)
(Accepted 2026-10-01 at completion). The ledger maps P13 to R-03, R-13 and R-14 and marks
it complete.

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
and handoff updates. No pull request publishes anything: the publish step PR 10 wired
runs only when the maintainer dispatches it on a release tag and approves it, at
completion. State on 2026-10-01 (merge commits are in the
[qualification record](p13-distribution.md)):

| PR | Content | Depends on | State |
| --- | --- | --- | --- |
| 0 | ADR 0023 and the kickoff (ledger, traceability, issue #16, notes) | P12 complete | merged (#226) |
| 1 | L-071 typed parse remediation | 0 | merged (#228) |
| 2a | `crates/vsift-cli/src/human/`, the `TerminalText` builder and the first command renderers (setup, `ingest`, `session`, `transcript`, `search`, `bundle validate`, failures), with SEC-T02's rerun over them | 0 | merged (#229) |
| 2b | The remaining renderers (`candidates`, frames, `crop`, `audio`, `job`, the worker hosts) and their SEC-T02 rerun | 2a | merged (#231) |
| 3 | The production smoke executor and its failure cleanup | 0 | merged (#230) |
| 4 | The guarded install transaction, managed lookup tier, `setup install`, `--artifact-dir`, `DOWNLOAD_FAILED` and the in-place contract values | 2a, 3 | merged (#234) |
| 5 | `handoff check`, `vsift-contract::handoff`, the skill's two input forms | 2a and the P12 debt pull requests (skill, guard and grader overlap) | merged (#233) |
| 6 | `setup list/rollback/remove/repair`, bounded cleanup and the stale-stage sweep | 4 | merged (#239) |
| 7 | Kill and power-loss tests of the managed store, and the P13 install E2E stage | 6 | merged (#241, with the verifier fix #244) |
| 8 | `release.yml` and the governance workflow lint | 0 | merged (#236) |
| 9 | The npm packages and the Verdaccio matrix | 8 | merged (#240) |
| 10 | Attestation and publish wiring | 8, 9; the maintainer's environment and trusted publishers | merged (#243); the environment and publishers were made by the maintainer on 2026-10-01 |
| 11 | Documentation, the qualification record and completion preparation | 1-10 | merged (#245); release prep (#247) followed |
| 12 | The ledger follow-up and the first-publish record (governance rule 9) | 11, and the maintainer's first publish (done 2026-10-01) | this change; the ledger names `011bc4d`, the release commit |

The compact tier's re-run (#222) followed PR 5 (decision F): it ran on `a0bfb06` on
2026-09-30 and met the 90% target (P12 qualification record).

**Not in P13:** P14's release qualification, managed installation on Windows and
macOS, native installers, crates.io, the MCP adapter, SEC-T01 (L-068) and the P12
debt #218-#224.

### P14 scope and pull requests

2026-10-02 (maintainer): P14 started, and the maintainer confirmed all eight decisions,
A to H, exactly as recommended (version `0.2.0`, a published release candidate, no signing
unless the try-outs trigger it, the recommended 84-run trial plan, SEC-T01 by a reviewed
fixture with the narrowed claim as the fallback, the per-cell support matrix, the claims
ladder, and try-outs that block the stable only until an observation is recorded). The
ledger marks P14 `in_progress`. The decisions, the reasons and their costs are in
[ADR 0024](../decisions/0024-r0-qualification-and-release-candidate.md) (Proposed until
the packet completes, as ADR 0023 was); the
traceability tables, the evidence items (`RQ-01..RQ-20`), the campaign and trial budgets,
the matrix and the claims policy are in [p14-qualification.md](p14-qualification.md), which
becomes the qualification record when the packet completes. The ledger maps P14 to
R-01..R-14, SEC-01..SEC-25 and `ALL-R0`, `R-SEC03`; it adds no requirement and no ID the
checker knows.

**Scope.**

1. **The release evidence ledger:** `docs/planning/p14-evidence-ledger.json`, a
   `vsift-governance` subcommand that checks it for completeness against the candidate, and
   a public-claims registry the Governance job checks (the delivery ledger cannot hold
   per-pull-request evidence: it requires an empty `verification` until completion).
2. **Published-artifact qualification on hosted runners:** clean install from the real
   registry with npm, pnpm, Yarn and Bun on Windows, macOS and Ubuntu; the extracted native
   archive on each target; the offline `--artifact-dir` install with the real artifacts;
   upgrade from 0.1.0 and uninstall; a second, credential-free verification of each publish.
3. **The journeys on the published binary** on Ubuntu 24.04, Windows and macOS 15, and the
   managed-install drift run (L-042, L-035, L-099, #178).
4. **Fuzz, race, fault, load and soak runs:** long fuzzing with a parser gap review, stress
   on three systems, the load ladder to eight jobs, a 100-request batch, a mixed soak,
   malicious media in a disposable container, the fault campaigns re-run, the worker
   runbook walked.
5. **Findings triage and R-SEC03:** the scan reading, SEC-T01 (or its recorded narrowing),
   the open issues #128, #205, #206, #232.
6. **The named-agent rounds from a clean install**, with the skill (both clients, both
   transcript paths, hold-out scenarios, the blurred-banner re-run) and the cold-agent
   variant (no skill, no docs; 84 runs planned, see the plan's section 7).
7. **The supported-profile matrix, operator and user documents, and the claims check.** The user
   documents include the R0 user guide (a structure by task, a first-investigation tutorial,
   concepts, troubleshooting and a generated reference; see
   [user-guide-spec.md](user-guide-spec.md), maintainer decision 2026-10-02).
8. **The release candidate and the stable release:** release tooling for stable versions,
   `0.2.0-rc.N` qualified in full, the stable `0.2.0` published by the maintainer, and the
   completion record with the handoff for using the published CLI ourselves.

**Pull requests.** Each is a coherent increment with its code, tests, documentation and
handoff updates, and each PR description holds its verification evidence (no evidence-only
pull requests). No pull request publishes anything: the candidate and the stable are
published by the maintainer, by the procedure of `release.md` section 6. Effort is
agent-working days; wall time is calendar days including waits for hosted runs, the
maintainer and usage limits. Both are rough (plus or minus half) and not measured.

| PR | Content | Depends on | Effort / wall |
| --- | --- | --- | --- |
| 0 | ADR 0024 and the plan, the decisions marked confirmed, the ledger `in_progress`, the packet issue #17 synced, the two handoff files rewritten (#250) | P13 complete; the maintainer's confirmation (given 2026-10-02) | 0.5 d / 1 d; done once it merges |
| 1 | The evidence ledger and its completeness check, the claims registry and its check (seeded with today's claims), the `RQ-nn` and `A-10` rows in `verification.md` | 0 | 2-3 d / 4 d; done once it merges (#251: `p14-evidence-ledger.json`, `public-claims.json`, `release-evidence` and `public-claims` in `vsift-governance`; ADR 0024's PR 1 note) |
| 2 | Published-artifact qualification workflow: RQ-01 (clean install, four managers, three systems, real registry), RQ-02 (archive), RQ-03 (offline with real artifacts), RQ-04 (upgrade from 0.1.0, 0.1.0 JSON compatibility, uninstall), RQ-19 (second verifier); runs against 0.1.0, which is published | 1 | 4-5 d / 8 d; done once it merges (#255: four read-only workflows and `tools/p14-published/`, the frozen 0.1.0 examples and two compatibility tests; all green on 0.1.0; findings #256, #257; ADR 0024 note; plan section 15) |
| 3 | The journeys on the published binary: a binary override for the real-tool checkpoints, the `P14 journeys` workflow on Ubuntu 24.04 (managed tools), Windows (pinned) and macOS 15 (Homebrew) (RQ-05), RQ-06 and its weekly schedule | 1 | 5-6 d / 8 d; done once it merges (#254: the override in `crates/vsift-cli/tests/published_binary`, `p14_installed_binary_e2e`, `tools/p14_journeys.py`, the `P14 journeys` workflow, the published mode and weekly run of `P13 managed smoke`; ADR 0024's PR 3 note holds the first results) |
| 4 | Campaigns: the fuzz gap review, new targets and a raised duration cap (RQ-07); lock stress on Windows and the stress repetitions (RQ-08); ladder, 100-request batch and soak (RQ-09); malicious media (RQ-10); the runbook walk (RQ-12); the first scan reading (RQ-13); triage records | 1 | 5-7 d / 8 d (the long runs take about two days) |
| 5 | SEC-T01 (RQ-14): the adversarial fixture and its CI job, or the ADR amendment that narrows the claim, as the maintainer decides | 1; the maintainer's review | 3-5 d (fixture) or 1 d (narrowing) / 6 d |
| 6 | The trial harness: clean-install mode, cold-agent mode and scenarios, hold-out scenarios, usage capture, the Codex clean-install image, #205; pilots (8 runs) and the cold baseline against 0.1.0 (12 runs) | 1; the maintainer's go (batch 1) | 4-5 d / 8 d; the harness, the scenario sets, the campaign script and the runbook are built (#262, nothing run, no model called); batch 1 (the pilots and the baseline) waits for the maintainer's go ([`trials.md`](../agents/trials.md), "The P14 batches") |
| 7 | Fixes for what PRs 2-6 find, one finding per pull request with a regression test: the `vsift --help` "typical investigation" section if the baseline shows gaps, #232, #206 or #128 if reproduced, platform defects | findings | 3-8 d / 10 d (number of pull requests not known) |
| 8 | Release machinery: stable versions and `latest` in `vsift-release`, the Release workflow and the lint, the candidate rule, the candidate-to-stable delta check, release-notes wording, `release.md`; reviewed as a high-risk seam | 0 (decisions A-C) | 3-4 d / 7 d; built in [#252](https://github.com/smormah/vsift/pull/252), awaiting review (includes the evidence-ledger guard and the notes templates and `release.md` under the claims check) |
| 9 | The matrix, the documents and the claims: `support-and-resource-profiles.md`, the README for newcomers, `install.md`, `SECURITY.md`'s supported-versions table, the worker runbook, the skill guide, the register review sheet and the readings, the registry filled and enforced; the R0 user guide in `docs/guide/` ([spec](user-guide-spec.md)) with its two CI checks (the guide's examples against real runs, the generated reference up to date) | 1, 2-5, 8 | 5-7 d / 8 d; delivered as two pull requests: **9a** (the matrix, the documents, the claims and the register sheet) and **9b** (the guide and its two checks, built); PR 9 is complete only when both are merged |
| 10 | The release candidate `0.2.0-rc.1`: the version bump, `CHANGELOG.md`; the maintainer tags, runs the dry run, publishes and verifies (RQ-19) | 7, 8, 9 merged | 1 d / 2 d (the maintainer about an hour); prepared as **10a** (the skill's wording before the freeze, #307), **10b** (the cut: the allowed-path lists, the bump, the changelog, the claims rung, the agent-trial freeze) and **10c** (the maintainer's exact steps, `release.md` 6.10, delivered with 10b); PR 10 is complete only when the candidate is published and verified; **the first candidate `0.2.0-rc.1` was published on 2026-10-05 and a second, `0.2.0-rc.2` (the same plus the fixes of #314 and #310), was prepared in a repeat of PR 10 (2026-10-06, `release.md` 6.11) and published on 2026-10-07; a third, `0.2.0-rc.3` (the second plus the skill's two evidence rules and the fixes of #321, #322, #332 and #325's first step, with the campaign's corrected no-room case), was prepared in a second repeat (2026-10-08, `release.md` 6.12) and published on 2026-10-08 (publish run 37746979716, tag `v0.2.0-rc.3` at `83dca856e7a0`); the stable `0.2.0` is compared with it (PR 12)** |
| 11 | The candidate's qualification: every hosted workflow on rc.1, the counted agent rounds (batches 2 and 3), the maintainer's try-outs (RQ-17; **`waived` on 2026-10-09: the stable ships untried, plan 29.10, L-143**), the scan reading again, the ledger entries; findings fixed and a second candidate if needed. **Repeated on `0.2.0-rc.2`** for what its two fixes and its version touch (plan sections 25 to 27: RQ-15 `failed`), **and repeated in full on `0.2.0-rc.3`** (plan section 28). **The evidence of the repeat on `0.2.0-rc.3` is complete since 2026-10-09** (plan sections 29 and 29.10: RQ-10, RQ-14, RQ-16 and RQ-17 are `waived`, the rest `passed`, `release-evidence --complete-for 0.2.0-rc.3` passes); what remains of it is the maintainer's register pass and reading of the cold logs | 10 | 5-7 d / 12-15 d |
| 12 | The stable `0.2.0`: the version bump, the documents that ship inside the artifacts, the completeness check green on the candidate with the delta check; the maintainer publishes and verifies; the hosted qualification re-run on the stable bytes; **then the maintainer deprecates `0.2.0-rc.1` and `0.2.0-rc.2` on all four packages (decided 2026-10-09: at the stable, not before; the maintainer runs the npm commands, the supervisor never does; plan 29.10)**. **Done on 2026-10-10: all eight are deprecated and checked, `0.2.0-rc.3` and `0.2.0` are not** (a session typed the commands on the maintainer's instruction, under the maintainer's login and second factor). **Prepared on 2026-10-09 (plan section 30): the stable commit (the five version-string files, the two shipped documents and the work record; nothing else differs from `v0.2.0-rc.3`) is on the branch `p14-pr12-stable-0.2.0` and the maintainer's checklist is [`p14-stable-release-steps.md`](p14-stable-release-steps.md). PR 12 is complete only when the maintainer has tagged `v0.2.0`, published it and verified it; nothing has been tagged or published.** After the tag, within seven days: the two `STABLE_CHECKS` of `tools/p14-published/lib/verify.cjs`, and the pages the stable commit may not change (the root `README.md` and `roadmap.svg`, `docs/agents/skill.md`, `docs/development.md`, `docs/operations/release.md`, `SECURITY.md`) are updated in a follow-up | 11 | 1-2 d / 3 d (the maintainer about two hours) |
| 13 | The ledger follow-up (governance rule 9): the repository-only pages flip to the stable instructions, `p14-qualification.md` becomes the record, the delivery ledger gets P14 `complete` with the stable release commit and a verification summary, ADR 0024 Accepted, the register swept, the work record states R0 complete and the neutral checkpoint for using the published CLI | 12 and the maintainer's publish | 1 d / 1 d |

Totals: about 40-60 agent-days. Calendar time is about 7 weeks on the critical path if
pull requests 2, 3, 4, 6 and 8, which touch separate files, are built in parallel sessions
under one integrating owner (governance rule 2), and about 12 weeks if they are built one
after another; the plan does not require parallel work. A hosted run needs a published
version: PRs 2-6 qualify the published 0.1.0 while the code is built, and every later run
uses the candidate.

**Who runs what.**

| Who | What |
| --- | --- |
| CI (hosted runners) | Every qualification workflow (dispatched by the packet owner's session, or scheduled once stable), the Release workflow's builds and dry runs, the scan tools, the Governance checks |
| The packet owner's agent session | Code, tests, documents, pull requests; dispatching non-publishing workflows and reading their results; running the agent trials on the maintainer's machine after each go; the scan reading; the triage |
| The maintainer | The decisions; each trial-batch go; every publish (dispatch with `dry_run` cleared, the `release` approval, npm two-factor); the Smart App Control and Gatekeeper try-outs; the SEC-T01 fixture review; the register pass; merging; every announcement |

The crash and power-loss campaigns, malicious-media runs and any other destructive test run
on disposable hosted runners or virtual machines only, never on the maintainer's machine.
Every sub-agent brief that can touch a network repeats the personal-data rule, and no
workflow, header or record carries a personal detail.

**Not in P14:** R1 and P15 onward, staged npm publishing (#246, deferred by the maintainer
on 2026-10-02 to after R1 or the announcements), crates.io, the MCP adapter, native
installers, managed installation on Windows or macOS, a product fix for Codex on Windows
(#204), signing (unless decision C's trigger fires), real-recording accuracy work
(#150, #159, #173-#175), a multi-tenant host, any announcement, a documentation site and its tool, user-guide pages
for R1 and later (they ship with their own packets), and using the published CLI
ourselves (after P14; only its neutral handoff is P14's).

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
fixes add regression tests that fail on the previous behavior. A change that alters what a
user sees or does updates the matching page of the user guide in the same pull request
(`docs/guide/`, created by P14 PR 9b; see [user-guide-spec.md](user-guide-spec.md) and, for the two
checks that hold it to the code, [`development.md`](../development.md#the-user-guide-and-its-checks)).

Rewrite `memory/TODO.md` and `memory/project_current_status.md` in the same PR so
they describe the current state in plain English and stay within the size limits
the governance checker enforces. Put verification commands and results in the PR
description; do not open separate evidence-record PRs. Record the ledger merge
commit once, when a whole packet completes. Never invent a commit hash. Keep work
pending until the observable acceptance criteria pass. Review and merge through
existing protected-main checks. Do not disable checks to complete a packet.
