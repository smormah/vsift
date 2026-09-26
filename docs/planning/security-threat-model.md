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
Since 2026-09-24 (#131) an opener that races the root's creator waits at most five
seconds for it, but only while the creator's provisioning lock is held or the root
is freshly created and still nearly empty; adoption always repeats the full owner,
privacy, marker, layout and no-link validation, so waiting never widens SEC-08.

Since 2026-09-24 private-root creation no longer trusts the parent's ACL (SEC-18).
A Windows directory inherits every inheritable ACE of its parent, and a real
profile's `%LOCALAPPDATA%` was found passing a sandbox group
(`(OI)(CI)(RX)`) and two `AppContainer` capability SIDs (`(OI)(CI)(F)`) to every new
child, so each root VSift created there failed its own DACL validation. Every
directory VSift creates as a private root (per-user configuration and its missing
ancestors, session root and its created parent, retained bundle, managed root) now
receives, before any content is written, a protected DACL (inheritance disabled)
granting full control only to the current user, `LocalSystem` and Administrators, the
principals the validator trusts; inherited and foreign entries are removed. The
change uses the existing `windows-acl` 0.3.0 dependency, whose DACL writes always set
`PROTECTED_DACL_SECURITY_INFORMATION`; VSift stays free of `unsafe`. It is path-based
while VSift holds a no-delete-share handle to the new directory, so the path cannot be
swapped, and the post-creation validation still runs. Unix keeps mode 0o700 at
creation (a umask can only clear bits), now also for missing ancestors of a private
root. An existing directory is never modified: when other accounts can access it the
operation fails with `STORAGE_IO` and fixed-prose remediation naming the folder kind,
never its path. Because a concurrent creator's new directory briefly carries its
inherited DACL, an opener that finds a directory which is empty, created within the
last 10 s and not yet private re-validates it with a short backoff (at most 2 s for the
configuration root; the session root's existing 5 s provisioning wait); content or age
ends the wait at once, and nothing observed while waiting is trusted. Residual: DACL replacement follows creation rather than being atomic
with it (that needs `CreateDirectoryW` security attributes, i.e. `unsafe` or a new
dependency), so a principal the parent already trusted could open a handle to the
still-empty directory in that window and keep it. Such a handle could list the names
of entries created later but not open them, because every later child inherits only
the private DACL.

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
bring-your-own provenance; P06 verifies selected tools and P13 owns managed identity
and version trust ([ADR 0015](../decisions/0015-r0-delivery-replan.md)). The strict
Linux CI profile supplies read-only filesystem, no-network, CPU, memory and PID
controls externally and exercises group escape plus bounded resource pressure, while
ordinary desktop probes make no such claim.

Since P07 (2026-09-24) the automatic media-tool preflight runs the P06 fixture
verification before the first media stage and records each pass in a per-user
verification record, so a planted or stale record could try to skip the check
(SEC-02). The record is an optimisation, never an authority: it lives in the private
per-user `VSift` directory (owner-only, no-follow and single-link checks), is strict,
versioned and at most 4 KiB, and any defect reads as "unverified" and is replaced. A
pass is keyed to a SHA-256 fingerprint of both canonical executable paths and their
on-disk identity, the reviewed compatibility policy and fixture digest, the adapter
profile, host isolation, verifier authority, verification profile and `VSift` version,
and ages out after seven days. Executable contents are not hashed; a same-user process
that rewrites a selected tool in place while preserving size and timestamps is outside
the desktop threat model above. Failures are never recorded, and the record holds
digests and times only, never paths or media (SEC-18). Lock-free readers that catch a
writer's rename (an opened record already unlinked, or a Windows delete-pending name)
retry a bounded number of times and otherwise read "unverified"; a record with more
than one link stays unsafe, and flushing before the rename is best effort because a
lost or torn record already reads "unverified" (issue #136). Automatic cleanup of
verification workspaces left by killed checks (issue #132) removes only directories
named exactly `vsift-tool-verification-<16 hex>` inside the private state directory
that are real directories, at least an hour old and not locked by a live check, with
no-follow removal.
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

Since 2026-09-26 (issue #148, SEC-08) an operation that calls a provider many times
over one session's source copy (local speech recognition and, since P08 PR 4, visual
sampling) binds the copy for the whole operation instead of rehashing it before every
call: one full SHA-256 verification when it is opened, an on-disk identity
comparison (size, modification time, device and file index, and the Unix
status-change time or the Windows creation time and attributes) before each provider
call, and a second full verification before anything derived from it is committed.
A replaced, resized or retimed copy fails before the provider reads it; any change
still present at the end fails the commit with a typed integrity failure. The
residual is ADR 0012's, unchanged: a same-user actor who changes the copy and
restores it (on Windows, including its modification time) before the closing
verification is not detected, just as one who changed it between a per-call hash and
the provider's read was not. Single-call operations keep the per-call rehash.

P07 increment 2 treats a supplied SRT/WebVTT sidecar as untrusted input: the same
no-follow local path policy as media, an 8 MiB file bound and per-line, per-cue and
cue-count bounds enforced while streaming (SEC-05); strict UTF-8 with control
characters, including C1 and Unicode line separators, rejected and output text
sanitized again at the contract boundary (SEC-03); unknown confidence kept
`null`, speaker labels only from explicit WebVTT voices and never guessed, and no cue
clamped or shifted to fit the source (SEC-17); and fixed-prose, typed remediation that
never echoes transcript text (SEC-16/SEC-18). The transcript lives only in the
disposable session and retained bundles; nothing is logged. Bidirectional-formatting
characters are not rejected and are shown as written; an agent must still treat
transcript text as evidence, not instruction.

P07 increment 3b (local ASR, ADR 0017) runs whisper.cpp only through `transcript
retranscribe`, with a closed argument list and no shell (SEC-01), a per-chunk deadline,
bounded and discarded stdout/stderr and a size-checked, no-follow read of its `-ojf`
file, whose model path and system information are never parsed into a value
(SEC-03/SEC-05). Only a model identified by SHA-256 as a reviewed pinned profile runs,
and the recognizer's identity is checked before and after every run, so a swapped
model fails the run instead of mixing outputs (SEC-12). Recognised text is untrusted
evidence: validated against its chunk's decoded audio, kept with uncalibrated
confidence and full provenance, and never promoted to instruction (SEC-16/SEC-17).
Chunk audio is user media, written only to a private work directory inside the
session, removed when the run ends and swept with the session; the run holds the
session so cleanup cannot remove files in use; remediation is fixed prose without
paths or text (SEC-18). A run holds one of the session root's admission slots for its
whole recognition and each chunk's decoding takes another, and it runs one whisper
process at a time with at most 8 threads (SEC-20). A superseded revision is never
rewritten or deleted, so an indexed citation cannot silently change (SEC-10/SEC-27).
Residual: the recognizer is a native process with the user's filesystem access, and
memory is bounded only by the operating system (an abnormal exit is reported as
`RESOURCE_LIMIT`).

P08 visual candidates (`candidates`, ADR 0018) decode user video only through
`FfmpegMedia::visual_samples`: one run per 60 s window with a closed argument list in
which only numbers and the private copy's path are filled in, the forced local
demuxer and `file` protocol, MOV external references disabled, `-xerror`, a 64 MiB
allocation cap, two threads, at most 122 frames of 128x72 grey pixels (about 1.1 MiB),
256 KiB of diagnostics and a 120 s deadline per window, and at most 30 windows per
call (SEC-05). A window FFmpeg rejects or whose output breaks a bound is recorded as
`undecodable` rather than retried: a video truncated mid-stream ends as such a gap,
not a failure or a false candidate, and F11's damaged tail, which damages only the
audio, is analysed completely because audio is never decoded (SEC-05/SEC-17). The frames are reduced to block
means and a 64-bit hash in memory and never written anywhere; the committed record
holds candidate times, reasons, change sizes and hashes, no pixels, lives only in the
disposable session and retained bundles, is bounded (8 MiB, 64 records) and is
decoded strictly with every analysis and identity rule re-derived on every read and
in `bundle validate`, so an edited record cannot claim coverage or changes the rules
would not produce (SEC-18/SEC-21). Every result states which parts of the range were
not analysed or could not be, and change sizes are published as uncalibrated
integers, never as confidence, so a gap is never presented as absence (SEC-17). Each
window's decode takes one of the session root's admission slots and a warm read runs
no provider (SEC-20). The copy is bound for the whole call as described above (SEC-08).
Residual: FFmpeg remains a native decoder with the user's filesystem access, as in
P04, and sampling at 2 Hz can miss a change shorter than 0.5 s or smaller than the
change rule; the result's coverage cannot report what sampling did not see.

SEC-17 finding, fixed 2026-09-26 in P09 PR 1 (ADR 0012 note of that date): the frame
and audio diagnostics readers accepted any line containing the filter's marker, and
FFmpeg echoes source metadata into the same output, so a crafted file could supply the
reported frame time and time base or the first audio sample time. On user media this
reached `transcript retranscribe`, whose chunk start times place transcript segments.
Every reader now accepts only lines that begin with the filter's own prefix, requires
frames numbered without gaps or repeats with strictly increasing timestamps (a single
extraction exactly one), and the frame paths fail closed unless the filter time base
equals the probed stream's. Regression tests fail on the earlier code; the fuzz targets
`frame_showinfo` and `frame_listing` check that indented copies of every line never
change a result. P09's evidence primitives (ADR 0019, not yet user-reachable) keep the
P04/P08 controls: one supervised run per call with a closed argument list, forced
demuxer and `file` protocol, `-xerror`, a 64 MiB allocation cap, two threads, a 30 s
deadline, at most 8 frames of at most 16 megapixels and 64 MiB of PNG, 1,200 listed
frames with 1 MiB of diagnostics, and WAV clips of at most 30 s; crop rectangles are
validated against the displayed frame before any I/O, and every PNG is walked chunk by
chunk with CRCs checked before it is accepted (SEC-05). Residual: a line that begins
with the filter's prefix (possible only through a log message that embeds an untrusted
string with a line break) and exactly continues the real numbering cannot be told apart
by text alone. For an extraction the count must still equal the images decoded; a
forged listing entry names a timestamp the exact extraction then does not find
(`FrameNotFound`), so it cannot become evidence.

P09 evidence core (PR 2, ADR 0019 accepted 2026-09-26; engine only, the commands
follow): every call runs the media-tool preflight and binds the source copy before
any provider runs, and evidence is committed only after a last identity comparison of
the copy (SEC-17/SEC-18). Per call at most 100 frames, 200 megapixels, 256 MiB of
images, 120 s and the session's remaining evidence slots (160 of 256 artifacts), with
typed partial results (SEC-05). Records are strict versioned JSON of at most 256 KiB
that never hold an operation id, a path or media bytes; decoding re-derives the
request key and every item identity, and `bundle validate` checks every item against
its file's kind, size, digest and PNG or WAV header and requires crop parents and
anchors to be present (SEC-07/SEC-17). Files reach the caller only as the absolute
path of the verified committed artifact inside the private session, valid while the
session exists (D2, SEC-07/SEC-18). The provider fingerprint in records and identities
is a SHA-256 digest; it binds the executables' canonical paths and identity but
reveals neither. Residual (D1, ADR 0012 note of 2026-09-26): after one full hash,
read-only evidence calls compare the copy's on-disk identity instead of hashing it, so
a same-user rewrite that restores every identity field between two calls is not seen
by the later call; on Unix the kernel-set status-change time prevents that, on Windows
a rewrite that restores the modification time is caught only by a later full hash. The
recorded identity lives in the private session manifest, writable only by the same
user, the actor this residual already assumes.

P09 delivery (PRs 3 and 4, 2026-09-26): `frame get`, `frame neighbours`,
`frame burst`, `crop` and `audio` are public. The grammar is closed and typed: opaque
`ses_`, `evd_` and `vcd_` identities, integer microseconds, counts and tolerances
bounded by the parser, and a crop rectangle of exactly four canonical unsigned
decimals with a positive size, parsed by the domain's own rule before any tool runs;
containment in the parent is checked against the session's committed record, never
the file (SEC-01/SEC-05). No request value reaches FFmpeg except the numbers the
adapter's closed argument list already takes. Errors and remediation are fixed prose
chosen by typed causes and never include a path, provider output or evidence text
(SEC-03/SEC-16). Results name local paths in exactly one place, `files[].path`, the
verified committed artifact inside the owner-private session (D2): the path is
re-hashed before it is returned, is valid only while the session exists and must not
be written to; a path that is not valid UTF-8 is `STORAGE_IO` rather than a lossy
string that could name another file (SEC-07/SEC-18). Evidence records and retained
bundles still hold no path, which the P09 checkpoint checks on a real retained
bundle. Damaged or cut-short media is `INVALID_SOURCE` with nothing committed.
Residual: an agent that copies a delivered path into later prose may leak the user's
session-root location to whoever reads that prose; the path is the user's own and the
default root is the per-user cache.

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
