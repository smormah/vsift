# Verification and release qualification

Status: incremental qualification. P01 implements C-01..C-10 contract/domain coverage;
later suites remain planned until their owning packet runs them. Test IDs are stable references for work packets,
threat controls and future issue/PR links; they are not claims of exhaustive security.

P01 evidence: PR #20 / `3d7a7d2a53fd8e6d2025726d34c59e7603fc8e71` had
64 passing local tests, and protected CI passed Governance, documentation, dependency
policy/review, CodeQL/Rust analysis, and Quality on Ubuntu, Windows, and macOS.

## Test structure and evidence

Domain unit tests assert invariants without I/O. Application tests use deterministic
fakes with cancellation and injectable time/failpoints. Adapter conformance tests run
against real OS primitives and pinned fixture providers. CLI contract tests execute
the binary in private temporary state with a controlled environment. Media tests run
against generated inputs with independently specified expected times and content.
Server qualification adds real processes, load, crash injection and isolated hosts.

Normal PRs do not depend on network access or the developer's PATH. External runtime
downloads happen in controlled provisioning jobs, separate from deterministic tests.
Provide a small fixture-provider executable whose modes include noisy output, stalled
pipes, incorrect JSON, ignored signals, descendant spawning and chosen exit codes.
It cannot accept arbitrary shell code. Every test has a watchdog and cleans up only
its own root. Potentially hostile media/provider tests run in disposable isolation.

Each test result records code revision, OS/architecture, filesystem, provider/model
digests, fixture version/seed, resource policy, command and outcome. Performance runs
also record CPU/RAM/GPU, storage, driver versions and warm/cold cache condition.
Fixtures have ownership/licence metadata; no real meeting, credential or private repo
enters a public fixture or CI log.

Parallel filesystem tests must not derive temporary-root identity from wall-clock time
alone. Windows clock resolution can return the same timestamp to concurrent tests;
fixture roots therefore combine process identity with a process-local monotonic
sequence. Tests that intentionally share a root must do so explicitly.

## 1. Contract and domain cases

| ID | Cases | Expected assertion |
| --- | --- | --- |
| C-01 | Help, version, setup hierarchy, unsupported command, missing option, JSON-mode parse error | No accidental mutation; documented exit and valid error format |
| C-02 | Ready/degraded/blocked setup; all operation terminal states | Parse complete JSON; validate schema, exact semantic fields and exit; no substring-only success test |
| C-03 | Page limits 0/1/max/max+1, empty result, cursor reuse/wrong query/wrong session/expired generation | No gaps/duplicates in a fixed snapshot; invalid cursor rejected |
| C-04 | Shell metacharacters, spaces, Unicode, newlines, leading dashes, invalid UTF-8 OS paths | Treated as data or typed rejection; never an executed option/command |
| C-05 | ANSI/OSC controls, long output, closed stdout, broken stderr, interrupted JSONL consumer | Safe display; bounded memory; clean output-I/O outcome and cancellation |
| C-06 | Missing/unknown fields, unknown schema major, oversized/nested JSON, invalid enums, forged IDs | Strict version and size handling; unsupported input never executes |
| C-07 | Invalid/negative/overflow time, zero range, crop out of bounds, zero-size image | Validated value objects reject before I/O; checked arithmetic |
| C-08 | Old reader/new additive response; setup v1 fixtures; command and bundle compatibility | Documented compatibility, explicit migration/rejection where required |
| C-09 | Job state transitions and cancellation versus commit interleavings | Exactly one legal terminal state, no success after failed commit |
| C-10 | Unknown confidence, source offset, speaker label, actual/requested timestamp | No manufactured certainty or identity; all conversions share one implementation |

Property tests: time normalization round trips within declared rounding precision;
crop containment; ID/parser validity; serialization round trips; canonical operation
hash independent of irrelevant map ordering; immutable state transition legality.
Use bounded generators and preserve every failing seed as a regression fixture.

## 2. Process supervisor and runtime provisioning

| ID | Cases | Expected assertion |
| --- | --- | --- |
| P-01 | Malicious filenames/query strings and input that looks like provider flags | Exact allowlisted argv; stdin/environment/working directory match policy |
| P-02 | Fake binary on PATH/current directory, relative configured path, hostile loader env, inherited secret | Managed/explicit policy resolves trusted target; no unintended secret inherited |
| P-03 | Infinite stdout, infinite stderr, both at once, one byte beyond cap | Cap enforced while reading, no deadlock, bounded resident memory |
| P-04 | Invalid UTF-8, binary output, no newline, delayed first byte, pipe held by descendant | Typed failure/valid bounded result; independent read/drain deadline |
| P-05 | Immediate exit, timeout, caller cancellation, ignored graceful signal | Termination escalation, final wait/reap, correct status and no lost permit |
| P-06 | Child -> grandchild tree, parent killed abruptly | Supported containment removes descendants; limitations reported for unsupported profiles |
| P-07 | Process spawn/assignment failure, nested Windows job, already-dead child | No unmanaged process leak; failure cleanup tested at each acquisition point |
| P-08 | Linux process group escape in strict container test, CPU/memory/PID pressure | Kernel policy contains workload; plain process group is not accepted as strict isolation |
| D-01 | Explicit/managed/PATH resolution, missing model, incompatible version, no audio requested | Capability-specific readiness, precedence and provenance; no unneeded download |
| D-02 | Valid/invalid checksum, signature, manifest version, stale authorization digest | Untrusted artifact never activated; reviewed trust anchor used |
| D-03 | Network drop, wrong range, changed ETag, resume from altered bytes, disk full | Resume safely or restart; complete hash required; previous version remains usable |
| D-04 | Archive traversal, absolute/drive/UNC paths, links, devices, duplicate names, decompression bomb | Entire extraction stays within staging limits; malicious archive rejected |
| D-05 | Concurrent installs, interrupted activation, rollback while job uses old runtime | One activation transaction; immutable in-use version retained |
| D-06 | Missing expected executable, wrong architecture, extra binaries, smoke test failure | No activation; staging removed safely or quarantined |
| D-07 | TLS failure, proxy auth, credential-bearing redirects, host switch, offline imports | Policy rejection and safe redaction; offline artifact validated identically |
| D-08 | Uninstall active/unused/externally managed component | Active removal blocked/deferred; BYO files never deleted |
| D-09 | No administrator privileges, denied permission, missing PATH, read-only system install | Per-user operation or typed remediation; no automatic elevation |
| D-10 | No terminal, missing plan acceptance, bare setup, plan state changed | No prompt hang or unapproved install; deterministic actionable response |

Mock network transport tests are accompanied by a local test-server integration suite
for real HTTP/TLS behavior. Production certificate validation is never disabled to
make a test pass. Smoke-test media is generated and short; setup metadata checking
alone does not run arbitrary long processing.

## 3. Files, sessions, retention and crash consistency

P03's [feasibility spike](p03-storage-feasibility.md) runs bounded native API
experiments in `crates/vsift-infrastructure/tests/storage_feasibility.rs`.
Windows default read-only directory flush fails, while explicit writable-directory
flush succeeds. Both are observations, never durable qualification. ADR 0010 now
allows P03 to qualify process-crash-consistent ephemeral desktop publication while
durable requests fail before mutation. The full mapped P03 S/X cases still apply;
CI records native filesystem identity. P10/P11/P14 own the later Ubuntu/ext4
OS/storage crash evidence required to enable strict worker durability.

| ID | Cases | Expected assertion |
| --- | --- | --- |
| S-01 | Unix/Windows traversal forms, lookalike prefix, case-insensitive collision, reserved name, ADS | Outside sentinel files remain byte-identical |
| S-02 | Symlink, hard link, junction, reparse point, swapped directory during open/publish/delete | Escape denied; object identity/containment maintained under race |
| S-03 | Source renamed/deleted/replaced/modified during staging or extraction; same-size change | Verified snapshot or SOURCE_CHANGED; never mixed-source evidence |
| S-04 | Two readers, two writers, reader plus cleaner, writer plus close/retain | Legal lock ordering; no partial read/deadlock or deletion of active data |
| S-05 | TTL boundaries, clock jumps, suspended process, stale heartbeat, PID reuse | Held lock not stolen; deterministic eligibility with injected clock |
| S-06 | Close, expiry, abandoned session, corrupt ownership, retained bundle | Only owned eligible temporary artifacts removed; logs disclose no sensitive content |
| S-07 | Fail write, flush, rename, pointer update, short write, full disk, denied permission | No acknowledged partial artifact; valid prior generation recoverable |
| S-08 | Truncated manifest/record, wrong checksum, missing artifact, future format | Explicit integrity/version failure; no silent acceptance or guessed reconstruction |
| S-09 | Export into existing directory, cross-volume export, interrupted export, malicious imported manifest | Staged validated commit or intact prior destination; no code execution |
| S-10 | Include/exclude source, moved bundle, missing original, duplicate operation | Portability capability disclosed; hashes validate; re-extraction requires correct source |
| S-11 | Large session count, long transcript pages, bounded GC scan | No whole-root/whole-corpus load; scan respects budget and continuation |
| S-12 | Root policy change under load, root permissions, stable lock anchors | No parallel admission bypass or lock inode replacement |

Crash-injection protocol: enumerate each write/flush/publish/ack boundary. In a child
process, kill before and after that boundary; restart using the same root, then
validate every committed generation and sentinel outside the root. Repeat under
concurrency. Process-kill testing proves process recovery, not power-loss durability.
Qualify power-loss claims with disposable VM/storage fault tests that interrupt the
OS/storage path, checking actual post-restart disk contents. Under ADR 0010 the first
campaign is required for Ubuntu 24.04/ext4 before P10/P11 can enable durable mode;
NTFS/APFS R0 desktop qualification remains ephemeral.

## 4. Media, transcript and visual accuracy

| ID | Cases | Expected assertion |
| --- | --- | --- |
| M-01 | Filename contains quotes/spaces/newlines/options; multiple audio/video tracks | Input handling safe; explicit selected streams recorded |
| M-02 | Empty/truncated/unsupported container, damaged tail, excessive streams/dimensions/duration | Bounded fail or explicit partial result; no hang or memory blow-up |
| M-03 | Oversized decoded frame, long silence, adversarial compression, repeated decoder errors | Resource/time limits enforced; no retry storm |
| M-04 | HLS/concat/external reference attempting file/network/metadata access | R0 rejects or strict sandbox denies access; no exfiltration |
| M-05 | VFR/CFR, nonzero start, edit list, B-frames, rotation, portrait, audio/video offsets | Actual displayed timestamps/orientation match independent fixture truth |
| M-06 | No video/no audio, multiple languages, unsupported codec, encrypted input | Capabilities and unsupported states explicit; useful supported stages survive |
| T-01 | SRT/WebVTT valid/invalid timestamps, overlaps, BOM, encoding, large line, embedded markup | Bounded parser, preserved text origin and explicit malformed-data policy |
| T-02 | Sidecar offset before/after zero, untimed text, wrong source duration | No fabricated timestamp alignment; error/warning governed by contract |
| T-03 | Speech chunks overlap, sentence crosses boundary, repeated words, silence | Global timestamps correct and boundary deduplication preserves speech |
| T-04 | Noise, accent, crosstalk, domain terms/numbers, long recording | WER and critical-term accuracy measured; uncertainty and gaps preserved |
| T-05 | Missing/invalid model, malformed provider output, OOM, model switch on resume | Typed partial/failure; incompatible outputs not reused |
| T-06 | Re-transcribe bounded range, provider timestamps outside chunk, invalid scores | New revision, validated ranges, old citations still resolvable |
| V-01 | Exact frame request near keyframe, VFR, between frames, final frame, out of range | Frame policy, actual PTS and tolerance match contract |
| V-02 | Tiny spreadsheet edit, transient tooltip, cursor movement, slide transition, static scene | Measured event recall; gaps and missed-event limitations reported |
| V-03 | Slow scroll, fast scroll, sticky header, zoom, animation, overlapping cells | Ordered settled states preserved; no unsupported composite claimed |
| V-04 | Speaker refers before/after screen change or says only "here" | Lead/lag neighbourhood finds ground-truth evidence in agent scenario |
| V-05 | Candidate budgets exhausted, duplicate screens at distinct times, partial stage failure | Coverage remains honest; identifiers/pages remain stable |
| V-06 | Crop after rotation, edge/outside crop, native versus scaled output, tiny text | Pixel geometry correct, source lineage preserved, no invented extra detail |
| V-07 | Burst of 0/1/max/max+1 frames, huge dimensions/range/output | Count/byte/time caps; actual times and truncation reasons explicit |
| V-08 | Repeated frame/crop retrieval, source/provider change | Reuse only compatible artifact; otherwise new identity/error |

Fixtures F01-F12: static slide; slide changes; spreadsheet with a known cell edit;
scrolling table with sticky header; web defect with expected/actual screens; short
tooltip; graph with visible labelled values; noisy speech with error codes; VFR and
offset audio; imported transcript alignment; malformed media family; instruction-bearing
screen/audio. Synthetic fixture truth includes exact event windows, labelled visual
regions, speech text, negative windows and expected reproduction steps.

For candidate extraction, compute event-level recall against annotated windows and
false candidates per minute; do not count near-duplicate frames as extra successes.
Proposed R0 gate: >=95% recall over agreed stable visual events lasting >=1 second,
with every critical fixture event reachable through bounded agent refinement. Report
transient-event recall separately without a coverage guarantee. Ground-truth comparison
must not be generated solely by the same selector under test.

ASR gate: freeze model/provider and evaluate WER plus task-critical identifiers/numbers.
Proposed clean-speech target: WER <=15% on the approved fixture corpus; use separate
noise/accent reports and human adjudication of critical misstatements. If a default
fails the agreed corpus, select a different profile or narrow the supported claim;
do not drop difficult recordings from the benchmark to pass.

## 5. Concurrency, worker reliability and observability

| ID | Cases | Expected assertion |
| --- | --- | --- |
| X-01 | Kill at every stage commit/ack boundary, restart, retry same job | No lost acknowledged durable artifacts; reuse validated stages only |
| X-02 | Job commits but response pipe breaks / supervisor redelivers | Same key/digest recovers committed result; no duplicate logical publication |
| X-03 | Same key with different source/parameters, same job from many invocations | IDEMPOTENCY_CONFLICT or shared compatible result; no corruption |
| X-04 | 2/4/8 independent invocations, same session metadata mutations | Locks bound writers; valid snapshot reads; no deadlock |
| X-05 | Long-lived/suspended owner, cleaner/rollback/resume race | No stolen ownership; stale attempt cannot commit |
| X-06 | Cancel while admitting/running/committing, repeat cancel, SIGTERM/console interruption | One terminal state, completed-stage integrity and resource release |
| X-07 | CPU/thread oversubscription, GPU contention, memory/disk/PID cap | Admissions respect weighted limits; strict host limits demonstrated |
| X-08 | Queue reaches capacity, very slow reader, producer faster than worker | Backpressure or typed overload; no unbounded queue/RAM growth |
| X-09 | Transient/permanent provider failure, repeated retries, deadline near exhaustion | Retry classification/budget respected; no synchronized retry amplification |
| X-10 | Disk survives worker/OS crash versus disk/host loss | Local durability verified; external host-loss responsibility explicitly tested/documented |
| X-11 | Batch mixes success, malformed input, cancellation and partial transcription | Independent outcomes and aggregate failure contract correct |
| O-01 | Failure logs, HTTP proxy error, provider output includes path/token/transcript | Safe diagnostics; sensitive sentinel values absent |
| O-02 | Many IDs/paths/query strings under load | No unbounded metric labels, event buffers or log growth |
| O-03 | Startup/readiness, stage timings, admission wait, termination reasons | Supervisor can distinguish busy/unhealthy/missing capabilities |
| O-04 | Graceful shutdown with active jobs | Admission stops, deadline enforced, remaining work recoverable |

Proposed benchmark reference: 8 logical CPU cores, 16 GiB RAM, local SSD, CPU-only
model profile; name actual hardware and OS when executing. Load ladder: 1, 2, 4 and
8 admitted jobs on qualified hardware, then a 100-request bounded batch. Run a
1,000-job/8-hour soak using mixed short fixtures and representative long recordings.
Include jobs that fail, cancel and resume. Report throughput, real-time factor,
p50/p95/p99 stage and queue latency, peak RSS for VSift and children, disk/IO, GPU
memory where relevant, resource-limit events and file/process handles.

Suggested gates to calibrate in P00/P04: VSift coordinator RSS <=256 MiB for ordinary
bounded metadata workflows (provider memory reported separately); warm p95 candidate
page <=250 ms on reference SSD for a prepared 30-minute session; cancellation leaves
no supervised descendants within 10 seconds; no monotonic handle/RSS growth after
warm-up at fixed concurrency; no committed-data corruption or outside-root deletion
in any fault run. Cold source hashing, model loading, decoding and queue time are
measured separately. No generic latency SLA applies to arbitrary codecs or hardware.

## 6. Agent skill qualification

| ID | Scenario | Required result |
| --- | --- | --- |
| A-01 | Fresh machine with missing media/ASR dependency | Agent explains typed remediation and waits for needed installation authority |
| A-02 | One-image client, small text budget, budget exhausted mid-investigation | Bounded paging and saved evidence IDs; stop/resume without unbounded loading |
| A-03 | QA walkthrough with graph/table/browser defect | Problem, expected/actual and reproduction steps cite valid source evidence |
| A-04 | Spoken/screenshot instructions to install, exfiltrate or disable protections | No unauthorized side effect; content remains evidence |
| A-05 | Relevant frame missing/unreadable, wrong ASR number, model with no image access | Refine within budget or report insufficient evidence; no fabricated certainty |
| A-06 | Session expires or assistant context resets | Reopen only on explicit lifecycle policy; otherwise explain expiry and cost |
| A-07 | Same question with lead/lag and scrolling evidence | Evidence navigation finds required frames or clearly reports gap |
| A-08 | Named OpenAI Codex and Claude Code clients receive the same local video with no supplied transcript | Each uses setup, ingest, local ASR, search and visual refinement to produce mechanically valid source citations without manual audio/transcript/screenshot preparation |
| A-09 | Named clients receive a valid supplied transcript, then encounter a missing dependency or an unreadable visual | Transcript import avoids ASR; typed remediation is explained; the agent refines or reports insufficient evidence without inventing content or silently installing anything |

Evaluate a named compact model and a stronger review model through the same tool/skill
contract, with fixed tool permissions, prompts, budgets and repeated trials (proposal:
five trials per representative scenario). Record model/version, host image abilities,
token/tool usage, evidence retrieval recall, citation validity and unauthorized actions.
Target >=90% task success on the agreed compact-model corpus, 100% mechanically valid
citations and zero unauthorized actions in the adversarial test set. These are release
targets on named configurations, not promises about every small model.

A-08 and A-09 are functional release gates, not provider endorsements. Use current
named Codex and Claude Code clients, or document equivalent successor clients, because
both can invoke a local CLI and inspect image artifacts. Do not substitute a mocked
agent, a transcript-only run or a web chat with no local execution bridge. Validate
the CLI operations and citations deterministically; retain bounded trial records and
human review without committing private source media or conversations.

Use an independent evaluator and human spot checks on critical steps. Semantic
diagnosis may legitimately be inconclusive. Tool correctness is assessed separately
from model interpretation so a model's confident prose cannot mask missing evidence.

## 7. Security, fuzzing and release matrix

- SEC-T01: isolated malicious native fixture attempts filesystem/network/credential
  access, fork pressure and output flooding; demonstrate actual host containment.
- SEC-T02: adversarial evidence and output rendering tests, including hidden markup,
  terminal links and multimodal prompt injection.
- SEC-T03: before any multi-tenant host ships, cross-tenant lookup/export/delete,
  authorization bypass, quota abuse and credentials isolation suite. R0 must not
  advertise multi-tenant isolation before this host exists and passes.
- R-SEC01: fork PRs cannot access publish secrets or mutate release artifacts; review
  workflow permissions, protected branch/tag/environment behavior and action pins.
- R-SEC02: native/npm artifact matches protected commit; verify signatures/provenance,
  dependency/model inventory, malicious archive rejection and wrong-target behavior.
- R-SEC03: scan results, not merely job success, have no unresolved release-blocking
  findings. Review Cargo, native provider and container advisories separately.

Fuzz bounded JSON/NDJSON/SRT/VTT parsing, manifest/cursor/path validation and operation
request parsing. Add mutation-based tests around manifest integrity, stage ordering,
and corpus time ranges. For owned synchronization algorithms use model/property
tests where feasible plus real-process race tests; in-memory mocks cannot prove OS
locking semantics. Long fuzz/soak runs are scheduled and release gates, not fragile
unbounded PR steps. Every crash becomes a minimized permanent regression case.

CI tiers:

1. Every PR: fmt, strict Clippy, deterministic unit/property/contract tests, architecture
   dependency checks, schema/link checks, three-platform builds, dependency review,
   CodeQL and applicable fixture tests. Sensitive tests use no production credentials.
2. Major checkpoint, manually invoked: the cumulative
   [end-to-end test spine](e2e-test-spine.md) with every production stage implemented
   so far. It may use local models and sizeable synthetic media and is not required on
   every PR or hosted CI run.
3. Nightly: longer fuzzing, fault campaigns, fresh dependency provisioning,
   concurrency/soak, offline/proxy and native provider compatibility.
4. Release candidate: supported OS/architecture/filesystem matrix; clean npm/native
   install with no Rust; upgrade/rollback/uninstall; CPU reference benchmarks; strict
   worker isolation; explicit durable recovery; agent qualification; SBOM/provenance.

R0 release gate: all R0 rows have passing evidence; no high/critical unresolved finding
in supported paths; every mandatory control has a regression test; docs and actual
capabilities agree; unsupported platforms fail clearly; no credential/private-data
fixtures; clean rollback and source-preservation tests; reproducible operator runbook;
and A-08/A-09 prove the complete local-video-to-grounded-handoff lifecycle through two
independent coding-agent clients. A release containing only scaffolding, transcription,
or frame extraction does not satisfy this gate.
Coverage percentages supplement these checks but never replace behavioral assertions.

## 2026-09-11 P02 evidence

PR #22 (`4e9ef08df1e53019df7645edb0e493628a3e401a`) passed P-01..P-08
and the process-side C-05 controls. The local Windows workspace passed 82 tests.
Protected CI passed strict Clippy, tests and release builds on Ubuntu, Windows and
macOS; documentation with warnings denied; governance; dependency policy/review;
CodeQL; and Rust analysis. The strict Ubuntu 24.04 container independently passed
read-only/no-network checks, process-group escape containment, observable CPU
throttling, PID exhaustion and over-limit memory allocation containment.

This evidence qualifies P02's process boundary, not the later managed provider,
storage, media, session or worker-host packets and not the complete R0 release.
