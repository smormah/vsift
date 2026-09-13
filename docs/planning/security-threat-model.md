# Security threat model and control plan

Status: proposed release requirements. Date: 2026-09-09.
Applies to the CLI, local state, runtime provisioning, native providers, agent handoff
and R0 worker deployment. Read [baseline findings](baseline-review.md) separately:
prospective controls below are not claims about current implementation.

## Assets, actors and trust boundaries

Assets: source integrity; confidential media/transcripts/images; local files outside
the state root; provider/model integrity; credentials in the host environment; CPU,
GPU, memory and disk availability; published packages; correctness of cited evidence.

Adversarial inputs include malformed or deliberately hostile media, hostile filenames,
forged manifests/bundles, model output, archives, network download responses, contributor
PRs, and instruction-bearing audio or screenshots. A local lower-privilege user can
attempt temporary-directory races. A compromised native provider may try to read host
files, access the network, fork or consume resources.

Desktop assumes the caller controls its OS account. Another process with the same
account and full filesystem authority can tamper with user-owned state; VSift must
avoid amplifying that authority, but does not provide hostile same-user isolation.
Server deployments handling mutually untrusted tenants require an external per-job
sandbox, caller authentication/authorization and separate storage scopes. Session IDs
alone are not tenant authorization.

Trust boundaries: request -> CLI; CLI -> application policy; application -> storage;
provider -> captured output; media -> decoder/model; download -> executable installation;
artifact -> agent; worker result -> supervisor; source/PR -> release pipeline.

## Attack and mitigation register

Priority P0 blocks exposing the affected operation in a release; P1 blocks the
corresponding supported deployment profile. Each row references the verification suite.

| ID | Abuse case | Required control | Verification |
| --- | --- | --- | --- |
| SEC-01 P0 | Shell/argument injection through path, query, filter or transcript | No shell; typed allowlisted options; separate path arguments; no raw filter/extra-args API | P-01, M-01, C-04 |
| SEC-02 P0 | PATH/current-directory executable hijack or DLL/library search pollution | Absolute approved provider path; private working directory; sanitized env; disclose BYO trust and verify managed identity | P-02, D-01 |
| SEC-03 P0 | Huge stdout/stderr, blocked pipes, malicious terminal escapes | Concurrent capped drains; output deadline; terminate excessive producer; escape control sequences before display | P-03, P-04, C-05 |
| SEC-04 P0 | Child spawns descendants, ignores cancellation or survives parent | Qualified process containment, escalation, reaping, inherited limit policy; no released slot before cleanup | P-05 through P-08 |
| SEC-05 P0 | Media decompression bomb / huge dimensions / infinite or corrupt streams | Byte, time, pixel, stream, artifact and disk limits; decoder isolation and watchdog | M-02/M-03, X-07 |
| SEC-06 P0 | Media references local secrets, URLs or cloud metadata | Restricted protocols/demuxers; reject playlists/external references for R0; no network and limited filesystem in strict worker sandbox | M-04, SEC-T01 |
| SEC-07 P0 | Path traversal or arbitrary overwrite/delete through artifact/bundle fields | Handle-relative contained storage; generated filenames; reject absolute paths, traversal, reparse/symlink/hard-link escapes | S-01/S-02, D-04 |
| SEC-08 P0 | Check-then-use race after canonicalization | Keep trusted directory handles; no-follow opens; revalidate object identity at mutation; deny unsupported guarantees | S-02/S-03 |
| SEC-09 P0 | Cleaner deletes active session, retained output or source | Ownership + exclusive lifetime claim + tombstone/quarantine; no source deletion; bounded scan and exact root scope | S-04/S-05/S-06 |
| SEC-10 P0 | Partial/corrupt output falsely reported complete | Validate before immutable publication; generation commit; hashes; typed integrity failure | S-07/S-08, X-01/X-02 |
| SEC-11 P0 | Concurrent writers or duplicate jobs publish conflicting evidence | Locks, operation identity, generation checks, conflict errors and idempotent publication | X-03 through X-06 |
| SEC-12 P0 | Malicious provider/model/runtime update | Pinned manifest, HTTPS, artifact integrity, publisher verification where available, no auto-update during work | D-02/D-03/D-05 |
| SEC-13 P0 | Zip Slip/tar traversal, decompression bomb, link entries or executable substitution | Bound compressed/extracted bytes and entry count; reject links/devices/traversal; stage privately; validate executable before atomic activation | D-04/D-06 |
| SEC-14 P0 | Download resume/redirect changes artifact; proxy credentials leak | Bind resumable transfer to artifact identity/validator; rehash whole result; strict redirect/TLS policy; redact credentials | D-03/D-07 |
| SEC-15 P0 | Runtime uninstall/rollback breaks active jobs | Immutable versions, job-held references, serialized activation and bounded GC; never remove BYO binaries | D-05/D-08 |
| SEC-16 P0 | Evidence contains commands to exfiltrate data or change code | Treat text and pixels as untrusted evidence; no media-driven policy/installs; skill limits authority to user's task | A-04, SEC-T02 |
| SEC-17 P0 | OCR/ASR/composite falsely asserts facts | Preserve actual source provenance, nullable confidence and gaps; refuse unsupported reconstruction; agent verifies source | T/V suites, A-03/A-05 |
| SEC-18 P0 | Sensitive content retained in caches, logs, errors or fixture repository | Explicit lifecycle, private permissions, no shared evidence cache, redacted telemetry; synthetic fixtures only | O-01/O-02, S-06 |
| SEC-19 P1 | Tenant reads another job's artifact by guessing ID | External authenticated scoped host; separate mounts/state; authorize every lookup, export and cleanup | SEC-T03; multi-tenant host deferred |
| SEC-20 P0 | Unlimited queue/retry concurrency or provider thread oversubscription | Finite queues, cross-process admission, provider thread caps, total deadlines and bounded retries | X-07/X-08/X-09 |
| SEC-21 P0 | Bundle import executes embedded instructions or parses unbounded records | Data-only schema; strict path/type/count/size checks; no deserialization that executes code; no remote references fetched | C-06/S-09 |
| SEC-22 P0 | CI/PR steals release secrets or replaces binary | Untrusted PR jobs without secrets; pinned actions; narrow permissions; trusted protected release workflow | R-SEC01/R-SEC02 |
| SEC-23 P0 | Correct checksums supplied by same compromised untrusted server | Trust anchor in reviewed source/release metadata; provenance/signature verification where supported; checksum alone not authenticity | D-02, R-SEC02 |
| SEC-24 P1 | OS crash/disk loss contradicts claimed durable completion | Qualify fsync/publication sequence; explicit durability level; fail on flush error; external durable storage for host-loss recovery | X-01/X-10 |
| SEC-25 P0 | Leaked secrets inherited by ffmpeg/ML subprocess | Allowlisted env/handles; stdin policy; no credential-bearing CLI args; restricted worker mounts/network | P-02, SEC-T01 |

### R1 managed and industrial expansion threats

These threats are scoped now so R1 cannot acquire persistence, models or remote worker
integration without their control cost. They are not claims that those capabilities
exist. The authoritative R1 boundary and test groups are in the
[industrial capability expansion](r1-industrial-capability-expansion.md).

| ID | Abuse case | Required control | Verification |
| --- | --- | --- | --- |
| SEC-26 P0 | One-off desktop evidence is silently enrolled in a persistent catalogue | Explicit catalogue selection and visible lifecycle; default ingest remains disposable | I-11, Q-01/Q-02 |
| SEC-27 P0 | Stale index, lost tombstone or partial erasure exposes deleted evidence | Transactional lifecycle events, bounded reconciliation, rebuild and erasure report | I-04..I-07, Q-09 |
| SEC-28 P0 | Poisoned OCR, tags or embeddings steer ranking or agent actions | Treat enrichment as untrusted hints; preserve source references and capability/coverage disclosure | E-01..E-08, Q-07 |
| SEC-29 P0 | Scroll/pan composition fabricates, duplicates or hides visible facts | Per-region source provenance, qualified thresholds, explicit gaps and refusal | RC-01..RC-08 |
| SEC-30 P0 | Queue replay, expired lease or stale worker publishes conflicting results | Request digest, attempt identity, fencing token and immutable idempotent commit | H-01..H-07 |
| SEC-31 P0 | Remote integration leaks credentials or reaches request-selected storage/URLs | Scoped workload identity, configured endpoints, sanitized transport and no request-selected credentials | H-06/H-09, Q-07 |
| SEC-32 P0 | Search cardinality, index growth or enrichment work exhausts service resources | Quotas, admission, bounded scans/pages/labels, cancellation and measured saturation | E-08, I-10, H-08, Q-03/Q-04 |
| SEC-33 P0 | Backup, restore or migration leaks or corrupts retained evidence | Scoped encrypted storage policy, integrity/version checks, atomic migration and recovery rehearsal | I-08/I-09/I-12, Q-06/Q-09 |
| SEC-34 P0 | Caller accesses another job or tenant by guessing an identifier | Authenticate and authorize every operation; separate storage scopes; opaque IDs are not authorization | H-09/H-10, SEC-T03 |
| SEC-35 P0 | Metrics, traces or operator logs disclose paths, transcript text or secrets | Allowlisted low-cardinality attributes, bounded exporters and sentinel redaction tests | H-08..H-12, Q-07/Q-09 |

## Process isolation profile

P03 qualification finding FS-01: the required OS/storage crash campaign is absent.
The Windows default read-only directory-flush error is resolved in an API probe
by requesting write access; that success does not qualify publication ordering.
ADR 0010 permits P03 production expansion only for ephemeral desktop publication and
requires durable requests to fail before mutation. See the
[evidence and unclosed controls](p03-storage-feasibility.md). Missing OS/storage crash
evidence remains a SEC-24 blocker for Ubuntu/ext4 durable enablement in P10/P11/P14,
not a released vulnerability or closure of SEC-07..SEC-11 and SEC-18.

PR #36 established the SEC-07/SEC-10 initialization seam. PR #42 (`3eef9b7`) adds
owned-root provisioning, Unix ownership/mode and Windows DACL validation,
handle-relative operations, no-follow/single-link checks, immutable weighted-admission
slots, shared/exclusive lifetime holds, writer fencing, bounded linked-manifest
integrity, future-version rejection and deterministic error/process-kill recovery at
every manifest/pointer write, flush and rename boundary. P04 source binding is
complete. P05 adds the public ephemeral lifecycle, private retained export,
bounded session index, held-lock cleanup and source-preservation regressions;
see its [qualification record](p05-session-qualification.md). P10/P11/P14 still own
durable stage acknowledgement and SEC-24's Ubuntu/ext4 OS/storage evidence.

No-shell execution addresses one injection route. It does not confine a vulnerable
decoder. Unix process groups aid termination; Windows Job Objects group processes;
neither should be presented as a complete filesystem/network security boundary.

Strict Linux worker qualification requires a non-root execution identity, read-only
verified runtime/model mounts, read-only staged source, one writable job root, no
host credentials, disabled network, process and memory limits, and an external
supervisor that terminates remaining work on worker failure. Configure seccomp and
capability removal where available; qualify required syscalls using the real providers.
Do not give the worker a container-engine socket or privileged mount.

Desktop profiles enumerate effective controls. If the caller requests strict
containment and the OS adapter cannot provide it, return ISOLATION_UNAVAILABLE.
Do not silently equate a memory estimate or timeout with a kernel-enforced cap.
Qualification evidence must cover nested Windows jobs and Linux container limits.
P02 reports Windows Job Object and Unix process-group lifecycle containment separately
from inherited hard isolation. Ambient provider discovery is explicitly unverified
bring-your-own provenance; P06 owns managed identity and version trust. The strict
Linux CI profile supplies read-only filesystem, no-network, CPU, memory and PID
controls externally and exercises group escape plus bounded resource pressure, while
ordinary desktop probes make no such claim.
Provider versions remain in the vulnerability inventory even though they run outside
the Rust process. Security fixes can revoke a managed version for new jobs, with a
documented handling policy for already running jobs.

## Installation and distribution policy

ADR 0014 makes the setup sequence explicit: diagnose first; propose a reviewed,
component/target-specific plan only for missing or selected tools; apply only after
separate acceptance; provide typed manual/BYO guidance whenever the managed path
is unavailable, offline, denied or fails. The agent may explain that guidance but
must not infer install authority from a video-inspection request, run a suggested
script or retry with elevated rights. A missing reviewed artifact is a managed-
unavailable outcome, never a reason to use an arbitrary URL. At least one complete
managed-install target must be qualified before R0 claims this capability; every
named R0 target must retain a tested manual/BYO path.

An install plan contains supported target, exact component versions, source URLs,
digest/signature metadata, sizes, permissions, licence notices, expected files and
activation changes. It has a canonical digest and expiry. User authorization applies
to that plan; content, host policy or state changes invalidate it. The CLI never
interprets an instruction inside a video as installation approval.

Keep binary/model installation separate from ordinary npm package installation.
No npm lifecycle hook downloads providers or models; `setup install` is a separate
explicit operation.
Use native platform packages selected by a small reviewed launcher; avoid silently
fetching model weights in npm lifecycle scripts. Users need a Rust toolchain only
when building from source. Qualify package-manager configurations that omit optional
dependencies or disable scripts and provide an explicit recovery path.

Use short-lived release credentials and trusted publishing where supported. npm's
OIDC publishing can attach provenance; configure its trust relationship for the exact
repository/workflow/environment. This is a proposed release control, not currently
configured npm publishing. See [npm trusted publishers](https://docs.npmjs.com/trusted-publishers/).

Produce an SBOM covering the Rust graph, launcher, shipped runtimes and model inventory;
retain notices and verify distribution rights for actual binaries/models. Do not
infer an FFmpeg build's licence from VSift's own dual licence. Review native DLL/shared
library contents and hashes. Release artifacts must map to a protected source commit,
and a tag alone must not bypass tests or release approval. Document signing/notarization
availability and avoid claiming publisher trust for unsigned artifacts.

## Agent-specific controls

The tool cannot make every downstream model immune to prompt injection. Evidence
fields are labeled untrusted, with stable IDs separate from instructions and safe
rendering for text. Screenshots can carry hidden/visible instructions too. The skill
must keep the original user objective, retrieve only evidence, and never execute a
command, reveal a secret, install software, or contact a URL because a recording says
to do so. Its permissions must remain narrow even if the model follows hostile text.

Qualification uses malicious spoken instructions, spreadsheet cells and screenshots
asking the agent to exfiltrate or weaken protections. A refusal to obey is required;
one successful defense does not prove universal safety. See [OWASP prompt injection prevention](https://cheatsheetseries.owasp.org/cheatsheets/LLM_Prompt_Injection_Prevention_Cheat_Sheet.html).

## Residual risks and response

P04 applies SEC-05/SEC-06/SEC-17 controls at the internal media edge: held no-follow
source staging, byte-identified snapshots, forced local demuxers/protocol, disabled
MOV external references, bounded probe/extraction and actual presentation timestamps.
The provider remains a native process with filesystem access in the desktop profile;
these controls do not constitute a filesystem sandbox or a whole-process memory cap.
P11/P14 retain strict decoder isolation and malicious-media release qualification.
See [ADR 0012](../decisions/0012-p04-source-media-profile.md) and the
[P04 qualification record](p04-media-qualification.md).

- Rust memory safety does not prevent logic errors or vulnerabilities in native tools.
- Provider supply-chain compromise, OS compromise and hostile same-user code remain
  risks beyond the CLI's own permission boundary.
- Sampling may omit evidence, transcription may be wrong and agent reasoning may fail.
- Ephemeral deletion does not erase backups, snapshots, SSD remnants or files copied
  elsewhere. State exactly when cleanup runs and what it owns.
- Strict durable acknowledgement is only as strong as the qualified filesystem,
  OS and storage hardware. A local bundle is not a replicated backup.

Release gate: no unmitigated critical/high finding in an exposed supported path.
Lower-severity exceptions require an owner, rationale, compensating control, test and
expiry. Keep private vulnerabilities in GitHub private advisories; public tasks
describe neutral hardening work without exploit details. Re-review this model when
adding a protocol, parser, backend, provider, credential, privilege or retention mode.
