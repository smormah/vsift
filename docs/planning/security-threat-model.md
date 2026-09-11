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

## Process isolation profile

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

An install plan contains supported target, exact component versions, source URLs,
digest/signature metadata, sizes, permissions, licence notices, expected files and
activation changes. It has a canonical digest and expiry. User authorization applies
to that plan; content, host policy or state changes invalidate it. The CLI never
interprets an instruction inside a video as installation approval.

Keep binary/model installation separate from ordinary npm package installation.
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
