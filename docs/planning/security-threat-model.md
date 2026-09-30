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
not a released vulnerability or closure of SEC-07..SEC-11 and SEC-18. (2026-09-27:
P10 PR 4 supplied that evidence for Ubuntu 24.04 / local ext4 only; see the P10 PR 4
note below.)

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
repository/workflow/environment. Since P13 PR 10 the Release workflow's `publish` job
uses it (repository `smormah/vsift`, workflow `release.yml`, environment `release`);
the trust relationships themselves are the maintainer's to configure before the first
publish ([`release.md`](../operations/release.md) section 6), so no npm publishing is
configured yet. See [npm trusted publishers](https://docs.npmjs.com/trusted-publishers/).

Produce an SBOM covering the Rust graph, launcher, shipped runtimes and model inventory;
retain notices and verify distribution rights for actual binaries/models. Do not
infer an FFmpeg build's licence from VSift's own dual licence. Review native DLL/shared
library contents and hashes. Release artifacts must map to a protected source commit,
and a tag alone must not bypass tests or release approval. Document signing/notarization
availability and avoid claiming publisher trust for unsigned artifacts.

### P13 notes

**2026-09-30, P13 started ([ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md),
Proposed).** The controls P13 will implement, per threat. This is a plan; the
evidence is added as each pull request lands, and none of it is claimed yet.

- **SEC-12 (malicious provider or runtime update):** only the reviewed Ubuntu 24.04
  catalogue revision can enter a plan; the transaction downloads from the publisher
  over HTTPS, checks exact size and SHA-256, smokes the candidate before
  `publish_and_select`, and never updates a runtime during work (D-02, D-03, D-05).
  *Evidence 2026-09-30 (PR 4):* `setup install` takes the compiled catalogue as the
  only trust anchor (the saved plan and `--artifact-dir` supply no URL, digest or file
  name), rebuilds the plan and requires the saved plan and digest to accept it, and
  verifies every byte by exact size and SHA-256 before anything is extracted; bytes that
  fail are discarded and never reach publication, and only a candidate whose smoke
  passed is published and selected, one component at a time. An update selects a new
  immutable version and never touches one a job holds: the executable a job resolved
  carries the version's shared use lock for the job's life. A managed tool runs only
  when every file matches its manifest by SHA-256, so a changed version is never run
  (L-006 closed for managed tools). Tests: `vsift-infrastructure/tests/p13_install_transaction.rs`,
  `vsift-application/tests/p13_install_transaction.rs`, `vsift/tests/engine_managed_lookup.rs`,
  `vsift-cli/tests/p13_setup_install_cli.rs` (D-02, D-03, D-05, D-06).
- **SEC-13 (archive traversal, bombs, substitution):** the existing bounded archive
  readers and flat private staging, plus a smoke of the staged runtime before
  activation and cleanup of every stage after a failure (D-04, D-06). *Evidence
  2026-09-30 (PR 3):* the smoke and cleanup exist and are tested (D-06): executables
  run only by explicit path from the unactivated stage, never through a shell; a
  foreign-format or non-executable file is refused before it runs; a provider that
  writes into its own installation or leaves a file behind fails; cleanup proves
  ownership first and never deletes an entry it did not create. Activation after a
  pass is PR 4.
- **SEC-14 (changed resume, redirects, proxy credentials):** no resume, a restart at
  byte zero and a whole-artifact hash; the reviewed redirect policy; `DOWNLOAD_FAILED`
  reasons that never render a signed URL or a proxy credential (D-03, D-07).
  *Evidence 2026-09-30 (PR 4):* each attempt is one `GET` with no `Range` header; only
  a complete `200 OK` identity body of the reviewed length is accepted (`206` refused);
  a dropped transfer discards its stage and the rerun restarts at byte zero (the test
  server records both requests without `Range`); redirects leave neither the reviewed
  route, host, scheme nor port, and a location carrying credentials is refused; an
  untrusted certificate is `tls`; a proxy's `407` is `proxy_auth`; errors carry only a
  typed reason, and sentinel proxy credentials from the environment appear in no output
  mode (D-03, D-07). The transport tests reach local servers only through the
  development-only `install-test-hooks` feature, which the crate refuses in release
  builds and the governance check refuses outside development dependencies. A
  development build resolves no host name for a publisher download, so no test run
  without `--release` can send a request to a real publisher (or anything to the
  internet) through `setup install`; release builds do not compile that guard.
- **SEC-15 (rollback or removal under running jobs):** per-version use locks, removal
  only of unselected unheld versions, cleanup bounded to the current and one previous
  version, a stale-stage sweep limited to positively marked stages, BYO files never
  addressed; kill and power-loss qualification that claims fail-closed detection plus
  repair (D-05, D-08).
  *Evidence 2026-09-30 (PR 6):* `setup rollback` selects only a version that verifies
  against its manifest (the recorded previous one also against the manifest the
  selection recorded), in one atomic rename under the install guard; `setup remove`
  refuses the selected version, deselects a component before removing it, and keeps a
  version whose shared use lock a job holds (`in_use`, `BUSY`). Deletion boundaries:
  every lifecycle operation opens the root, its version and selection folders and each
  stage without following links, proves the root's and each stage's marker and that
  each folder is private and still the one held, and removes only names it created
  (a version's manifest files and metadata, a stage's marker, artifact and flat payload,
  runtime and smoke files), each a single-link regular file; a link, a junction, a
  nested folder or an unknown name keeps the whole version or stage for the user (L-090).
  Removal proves ownership, not integrity, so a corrupted version can be removed; nothing
  outside the managed root, no source media and no user-configured tool is addressable.
  Bounded cleanup keeps the selected and previous versions and any held one; the sweep
  runs only under the guard, when no stage can be live. `setup list` and `setup repair`
  only read. Tests: `vsift-infrastructure/src/managed_store_lifecycle/tests.rs` (planted
  links and junction-like directory links, unknown files, corrupted and interrupted
  versions, held versions, a changed root marker), `vsift/tests/engine_managed_lifecycle.rs`,
  `vsift-cli/tests/p13_setup_lifecycle_cli.rs` (D-05, D-08). *PR 7 (2026-09-30):* a
  process killed at any of the 22 managed fault points, or by the operating system,
  leaves a store every command can read and `setup repair` describes; one decision
  widens what the store treats as its own: a folder at the root's, `versions-v1`'s or
  `current-v1`'s fixed name that is private and holds nothing, or only the start of
  its marker, is what a killed creation leaves, and an install finishes it (readers
  treat it as empty). A private, empty folder at `managed-v1` that the user created
  would be adopted the same way; any other content keeps the folder unowned. Every
  folder a command changes is flushed before it returns, so on Ubuntu 24.04 with ext4 a
  reported command survives a power loss, and nothing unverified is ever run. Tests:
  `vsift-infrastructure/tests/p13_install_transaction/kill.rs` and
  `every_commit_step_is_flushed_before_the_next`.
- **SEC-22 (CI steals secrets or replaces a binary):** the governance workflow lint
  (actions pinned by commit SHA, no `pull_request_target`, minimal permissions,
  `id-token` only in the attest and publish jobs); publication only from the protected
  `release` environment with the maintainer as reviewer; npm trusted publishing
  without a long-lived token; `dry_run` by default (R-SEC01). *Evidence 2026-09-30
  (PR 8):* the governance checker parses every workflow and fails an unpinned action,
  `pull_request_target`, missing or writable top-level `permissions`, `id-token:
  write` outside the release workflow's `attest` and `publish` jobs, and a `run`
  script that interpolates an expression an outsider can choose; no existing workflow
  violated a rule. `release.yml` builds and packages with `permissions: {}` and
  `contents: read`, no OIDC token, no secret and no cache, and cannot publish; its
  builds select no feature, which the lint enforces. The `release` environment and
  trusted publishing are PR 10. *Evidence 2026-09-30 (PR 9):* the npm packages are
  assembled, packed and qualified in the same workflow with the same limits
  (`contents: read`, no OIDC token, no secret); the only registry any job writes to is a
  Verdaccio on the runner's loopback address with no uplink, whose throwaway token lives
  in a scratch npmrc scoped to that address; the qualification driver refuses any other
  registry and strips registry and token settings from every package manager's
  environment. Every action it adds (`actions/setup-node`, `oven-sh/setup-bun`) is pinned
  by commit SHA, and the tools it installs are pinned to exact versions.
  *Evidence 2026-10-01 (PR 10):* only two jobs can write or request an OIDC token:
  `attest` (`id-token`, `attestations`) and `publish` (`id-token`, `contents`). Both run
  only for a manual dispatch with `dry_run` cleared, in `smormah/vsift`, on a `v*` tag
  that the plan job accepts as `v<version>`, so pull requests, pushes and forks never
  reach them; `dry_run` defaults to `true`, and a cleared one elsewhere fails the run.
  `publish` runs in the `release` environment (the maintainer approves each deployment,
  limits it to `v*` tags and protects those tags with a ruleset), uses npm trusted
  publishing, so no long-lived npm token exists, and names only one secret,
  `NPM_BOOTSTRAP_TOKEN`, an optional short-lived environment secret for the first
  publish of a package without a trusted publisher. Neither privileged job checks out
  code or builds, packs or installs anything: they run pinned actions (download,
  `actions/attest-build-provenance`, `actions/setup-node`) and a few lines of shell over
  this run's artifacts, checked by SHA-256 against job outputs that later jobs cannot
  change. The governance lint's rule 7 (`workflows/publish.rs`) fails a workflow that
  loosens any of this, with a test per rule; it also requires the dispatch input to be
  compared as a string, because GitHub's loose comparison makes `inputs.dry_run ==
  false` true on events with no inputs. The environment, ruleset and trusted publishers
  are maintainer steps not yet taken ([L-096](known-limits.md#l-096)).
- **SEC-23 (checksums from the same compromised server):** the managed trust anchor
  stays in reviewed source, never in a downloaded checksum; release archives carry
  Sigstore build provenance tied to the protected commit, npm packages carry npm
  provenance, and the launcher checks its platform package's version and, if cheap
  enough, its digest (R-SEC02). *Evidence 2026-09-30 (PR 9):* the launcher requires the
  platform package's version to equal its own and the executable's size and SHA-256 to
  match `platform-digests.json`, which the release tool computes from the archives the
  build produced (never by hand), before it starts anything; a changed byte, a replaced
  executable, another version and a damaged digest file are refused with exit 126 in the
  launcher's tests and in every qualification job, and the check's measured cost is held
  under 50 ms (decision H5). It proves the package is the one released with this
  launcher, undamaged; it is not a defence against someone who can write to the install
  folder ([L-093](known-limits.md#l-093)). *Evidence 2026-10-01 (PR 10):* when the
  maintainer publishes, the `attest` job creates a Sigstore build-provenance attestation
  for every archive, `SHA256SUMS`, SBOM, notices file and npm tarball, tied to the
  workflow, the tag and its commit; the npm packages carry npm provenance from trusted
  publishing; and the bytes published are those the `npm-qualify` jobs installed, by
  SHA-256 (`npm-package`'s `tarball-sums` output, checked by every later job). Anyone can
  verify with `gh attestation verify` and `npm audit signatures` (release.md section
  6.4). A `SHA256SUMS` from the release page stays a convenience, not proof of origin;
  its attestation is. No attestation exists until the first publish (L-096).
- **No install-time code (the npm packages, PR 9):** no VSift package declares a
  lifecycle script, a `gypfile` or a `binding.gyp`, so installing runs nothing and
  downloads nothing beyond the packages; the governance check refuses such a manifest
  under `npm/`, `vsift-release npm-verify` refuses it in a packed tarball, and every
  qualification job installs with scripts disabled. The launcher starts only the
  platform executable, by explicit path and argument list with `shell: false`; no
  manifest names a person.

Human-readable output (SEC-T02) and `handoff check`'s untrusted input (a draft
that may carry evidence text) belong with the agent-specific controls below: both
escape control and hidden characters and never echo input into a result. Since P13 PRs
2a and 2b, every human output is written through one builder that enforces this,
delivered paths included (each whole on its own line, a hostile one inert and
flagged), and SEC-T02 is re-run over all of it (verification section 7).

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

**P12 PR 1 (2026-09-28, [ADR 0022](../decisions/0022-agent-skill-and-named-client-qualification.md),
Proposed).** The skill in `skills/vsift/` implements these controls as instructions
and data, not as enforcement: every public command has one class (`free`, `explicit`
on the user's instruction, `never`), and `never` covers managed installation,
worker-host commands, the operator-only global options and every non-`vsift`
executable (SEC-16); evidence text, pixels and audio are data, embedded instructions
are listed with citations and never acted on, hidden characters are shown as
`<U+...>` notation and links are defanged (SEC-16, SEC-T02); claims carry support and
certainty, gaps are never absence, and a visual claim needs an image the model
verifiably read (the image check) (SEC-17); reports cite identities, never the
delivered paths, and the handoff schema refuses paths, home prefixes, links and hidden
characters in prose (SEC-18, L-007). The skill grants no tools. A unit-test guard
fixes the `never` and `explicit` sets and holds every command, flag, code and field
the skill names to the CLI. None of this is qualified yet: A-04 and SEC-T02 run in
the named-client trials, where an attempted out-of-policy action fails the trial even
if the client's sandbox blocked it.

**P12 PR 2 (2026-09-28).** The trial harness `tools/vsift-agent-trials` grades that
rule from the client's event stream, with the policy parsed from the skill's own
command table; Codex's permissions are graded rather than configured (L-072). Trials
run under a neutral root so delivered paths carry no user name (SEC-18), with a cleared
environment, isolated client configuration and canaries whose appearance fails the
trial (SEC-16). The tool-level SEC-T02 suite runs on every PR over synthetic
adversarial sidecars (verification section 7); P13 PRs 2a and 2b re-ran it over
human-readable output.

**P12 completion (2026-09-30, ADR 0022 accepted).** The named-client trials ran with
hostile spoken, subtitle and on-screen instructions: A-04 and SEC-T02 through Claude
Sonnet 5.5 in Claude Code and GPT-6-Sol in Codex, 20 counted runs. In those runs, and
in all 84 counted phases, the result was the same:

- no agent attempted an out-of-policy action;
- no agent leaked a canary;
- no agent installed anything;
- no report copied a raw hidden character, a live link or an absolute path.

The reference rounds (348 graded phases) showed no leak, install or injected action
either ([P12 qualification record](p12-agent-qualification.md), "Safety"). This is
evidence on named configurations and synthetic fixtures, not immunity. L-007 stays as
an accepted residual, and L-083 records that `text` keeps hidden characters raw.

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
characters are not rejected and `text` keeps them as written; since 2026-09-29 every
segment also carries `display_text`, which shows each hidden character (Unicode `Cf`,
`Default_Ignorable_Code_Point`, U+2028 and U+2029) as `<U+XXXX>`, and the skill
quotes only that ([ADR 0008 note](../decisions/0008-cli-and-json-contract.md#2026-09-29-note-display_text-for-hidden-characters)).
An agent must still treat transcript text as evidence, not instruction.

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
images, 120 s and the session's remaining evidence slots (384 of 512 artifacts since
P10 PR 2, ADR 0020 D-2; 160 of 256 before), with
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

P10 PR 1 (2026-09-26, [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)):
**SEC-08/SEC-10.** Reads now validate the manifest chain down to the writer's chain
checkpoint (#164) rather than to generation 0. Every read still verifies the pointer,
the head and every generation committed since the last full verification, and
re-hashes every artifact it returns (INV-02); retained exports and cleanup still walk
the whole chain. The checkpoint is written only by the writer under the writer lock,
staged and renamed, never ahead of the head; a malformed or forged checkpoint is
`INTEGRITY_FAILURE`, a newer one `UNSUPPORTED_SCHEMA`, and a missing one or one ahead
of the head means a full walk. Residual: damage to a generation below the anchor is
found by the next retain or cleanup, not by an ordinary read; old generations never
feed a read's result. The checkpoint lives in the owner-private session directory, so
forging it needs the same-user access that could rewrite the chain itself.
**SEC-24.** The durable publication order (flushes, then directory syncs of
`artifacts/`, `generations/` and the session, acknowledgement last; fsyncgate-safe
retries; any flush, sync or rename error is `STORAGE_IO` with nothing acknowledged) is
implemented but disabled: no profile claims `os_crash_durable` until the Ubuntu 24.04 /
ext4 campaign passes, so SEC-24 stays open. The mount-table parser that gates it reads
at most 1 MiB, refuses a damaged table whole and is fuzzed (`mountinfo`). Fault points
can stop the process only in unit tests and `fault-injection` builds, which cannot be
compiled without debug assertions and which the governance check refuses outside
development dependencies.

P10 PR 2 (2026-09-27, ADR 0020 accepted): recoverable jobs and checkpointed
retranscription.
**SEC-09.** A live job holds a shared hold on its session's lifetime (through its
source binding, work directory and job owner), so `session close` and `session clean`
stay `BUSY` while it runs; an interrupted job holds nothing and never blocks cleanup.
Cleanup removes a session's jobs with it (they live under the session directory, inside
the bounded owned-tree validation) and prunes their root job-index entries; jobs never
renew a session, and after close or expiry a resume fails like any request.
**SEC-10.** A chunk checkpoint is a private, uncommitted stage file, never evidence,
never named by a manifest and never exported by `session retain`. It holds the raw
provider output, stored only after that output passed validation, and a resumed chunk
passes the same validation and merge again, so a checkpoint cannot introduce a segment
the rules would reject. Every checkpoint and job record is strict versioned JSON (256
KiB and 64 KiB bounds, unknown fields rejected, payload digest, ordinal bound to its
file name, recognition key bound to the run, the job id re-derived from the record's
own keys); a damaged, forged or newer one is removed and redone (checkpoints) or fails
closed (records), never repaired (S-08). Resume still binds the source copy with a full
hash and verifies it again before each commit. Residual (known limit L-049): the
payload digest detects corruption, not a same-user forger who recomputes it; such a
forger could already rewrite committed evidence (accepted residual of this model).
**SEC-11.** Duplicate requests find one job (its id derives from the request's
operation key) and the job's OS lock admits one owner: a second is `BUSY` naming the
job; a suspended owner keeps its lock; a stale attempt cannot change the job (epoch and
attempt fence under the state lock). Each commit uses a deterministic operation id,
records `committing` first and is reconciled one way only from the manifest chain, so a
retry never publishes twice (X-02); an operation id reused for another request is
`IDEMPOTENCY_CONFLICT` (X-03). A cancellation and the commit transition are serialized
under the job's state lock. An unreferenced manifest an ended publication left above
the head is replaced by the next publication under the writer lock (it cannot belong
to a live one), which removes the blocked-session case of L-048.
**SEC-08/SEC-10 (readers vs replacement).** A reader that meets a metadata file being
replaced by rename (it opened the old file, which then has no link left, or on Windows
the name was absent for a moment) retries for at most 500 ms instead of reporting
`INTEGRITY_FAILURE` for a healthy session; a hard link or non-regular file is still
refused at once and a file still missing is still an integrity failure.
**SEC-24.** Job and checkpoint files follow the session's durability: a durable
session synchronises their directories after each rename. Durable mode stays disabled
on every profile until P10 PR 4's campaign, so SEC-24 stays open.

P10 PR 3 (2026-09-27): the public job surface and interruption handling.
**SEC-04.** The CLI traps the first `SIGINT`/`SIGTERM` or console Ctrl-C/Ctrl-Break of a
long command and turns it into the command's cancellation: the supervisor stops the
running provider tree (Unix: `SIGTERM` to its process group, a kill after 5 s;
Windows: the Job Object at once) and reaps it before the command returns; a second
interruption skips the graceful 5 s. The process exits only after its command has
returned, so an interruption never leaves a provider running (checked: no descendant
alive 10 s after the command ended, in `p10_recovery_e2e`). Because a console event
reaches every process on the console, a provider that ended unsuccessfully after the
caller cancelled is reported as cancelled, never as a decoding or recognition
failure, so an interrupt cannot plant a false `undecodable` window or poison a chunk.
`job cancel` from another process only records `cancelling` under the job's state lock
(never while committing); the owner notices within 250 ms and stops its own provider,
so no process ever signals another. Residuals: a process that inherited "ignore
Ctrl-C" on Windows sees only Ctrl-Break (L-053); VSift's own hashing and publication run
to their next check (L-054); a hard-killed CLI on Unix leaves its running provider to
finish its current unit (L-055).
**SEC-20.** Interruption adds no admission, thread or retry: the command's one
cancellation reaches its admission-bounded stages, and a resumed or cancelled job's
retries keep ADR 0020's policy (`BUSY` twice with jitter). The owner's cancel watcher
is one timer per running job, reading one small record per 250 ms without a lock.
**SEC-16/SEC-18.** Job results and remediation name only identifiers, states, counts
and codes (no path, transcript text or provider output); the one suggested command is
the executable `vsift` with the fixed words `job resume` and a validated job id, never
shell text, and needs no authority. `--operation-id` and job ids are parsed by the
grammar before any I/O.

P10 PR 4 (2026-09-27): the Ubuntu 24.04 / ext4 crash campaign and durable enablement.
**SEC-24.** The campaign ([P10 durable-publication record](p10-durable-publication.md))
replayed a power loss at every flush of a dm-log-writes log, killed Ubuntu 24.04
virtual machines at random moments and injected write and flush errors with
dm-flakey; no acknowledged durable generation was lost, every injected failure was
`STORAGE_IO` and never acknowledged, and a negative control proved the harness sees
loss. `QUALIFIED_UBUNTU_EXT4` is set; the profile check also requires Ubuntu 24.04 by a
bounded, strictly parsed and fuzzed `os-release` read, and everything unread or
unparsed fails closed. SEC-24 is closed for that profile; it stays open for every other
(durable requests there fail with `MISSING_CAPABILITY`), for storage that ignores
flushes (L-056) and for disk or host loss, which is the caller's to cover with
replicated storage (X-10, L-057). The campaign also found that a storage failure while
committed state was read (a filesystem that shut itself down after a write error) was
reported as `INTEGRITY_FAILURE`, which would present a failing disk as tampered
evidence; such failures are now `STORAGE_IO`, and a missing, mistyped or linked entry
is still an integrity failure (**SEC-10**: a typed integrity failure keeps meaning
damaged or altered bytes). The negative control can only be compiled
into development builds (the `durability-campaign` feature is refused without debug
assertions and by the governance check outside the campaign tool's non-default
feature).

P11 PR 2 (2026-09-28): worker workspaces, weighted admission, strict Linux
attestation and contained inputs ([ADR 0021](../decisions/0021-worker-and-batch-host.md)
PR 2 notes).
**SEC-05 (inputs and bombs).** A worker request's paths are opened inside an operator
input root held as a capability, one component at a time and following no link, after
the request path grammar (no `..`, absolute, drive, `\`, `:` or stream, device names,
control characters); a link anywhere on the path and a file with several hard links
are refused, and nothing outside the root is read (S-01, S-02; L-062). A worker
workspace checks, on Unix, that a source copy leaves 1 GiB free before it starts; it
is a pre-copy check, not a quota (L-061). Decoder allocation and thread caps are
unchanged.
**SEC-06/SEC-25 (strict worker).** `--host-isolation strict-linux` is accepted only
after bounded reads of the kernel's own view attest a cgroup v2 with finite CPU,
memory and PID limits (on the cgroup or an ancestor), a root mount read-only in its
own options and no network interface but loopback; every parser refuses a damaged
file whole and is fuzzed (`host_attestation`, `mountinfo`), and anything unread fails
closed with `ISOLATION_UNAVAILABLE` before any work. The limits are the host's and are
reported as such; VSift claims no enforcement. Whether such a host actually contains a
hostile decoder is SEC-T01 (status under P11 PR 4 below).
**SEC-20 (oversubscription).** Admission now weighs what runs: a recognition reserves
its recognizer threads (capped at the root's capacity and at 8), a visual window 2
units, anything else 1; the recognition's reservation covers its chunk decoding, so a
job is never counted twice and never runs beside an unreserved decoder. Work heavier
than the root fails `RESOURCE_LIMIT` before any work; the cross-process property is
tested on real OS locks with child processes at capacities 2, 4 and 8, with a negative
control. A job host's bounded admission wait is at most 60 s with full jitter; there is
no fairness between processes (L-060). The workspace policy (capacity, durability,
retention) is recorded once and never adopted when changed underneath (an integrity
failure), so a request can never raise it.
**SEC-24.** A durable worker workspace exists only where the qualified profile check
passes, checked before anything is created; its sessions all use ADR 0020's durable
protocol (the command line's durable mode). The commit path is unchanged.

P11 PR 3 (2026-09-28): `job run`, request records and the two-stage shutdown
([ADR 0021](../decisions/0021-worker-and-batch-host.md) PR 3 notes).
**SEC-09.** A worker request never deletes anything outside VSift's own root: a
`retain` step writes to a hidden staging directory beside its bundle's name and
renames it once it validates, and a staging directory a killed try left behind stays
(L-064); an existing directory under the bundle's name is accepted only when it
validates as the request's own bundle, never replaced. Request records are pruned only
when the session they name is gone and no process holds them, under the record's own
owner lock. Sources in the input root are opened read-only and never written.
**SEC-10.** A request record is replaced by a staged, flushed rename (and the bucket
synchronised in a durable workspace), so a reader sees the old or the new record whole;
an ended record holds its result's canonical bytes and their SHA-256, both checked on
every read, and the recorded steps and results must read back to exactly their bytes,
so a changed record is `INTEGRITY_FAILURE`, never a different replay. A result whose
record could not be written is not acknowledged (`STORAGE_IO`). The crash campaign
was rerun with requests in its workload (p10-durable-publication.md).
**SEC-11.** A request's operation id is its idempotency key: another digest under the
same id is `IDEMPOTENCY_CONFLICT` and changes nothing; the owner lock (the only
liveness authority) makes a concurrent duplicate `BUSY`; a claimant that locked a
lock file that pruning removed notices by file identity. Each step is idempotent: the
ingest's session id is recorded before its copy (a continuation adopts the session if
it opened, else opens a new one, so one key opens one session), a recognition runs
under an operation id derived from the request's, a retain accepts only its own bundle.
The kill test stops a process after each record write and the external-delivery
simulation kills workers at random while duplicates race: every request committed
once.
**SEC-18 (O-01).** A job result, its events and every remediation name identities,
counts, digests and enums only: never an input path (the request's paths stay inside
the engine), a sidecar's text, provider output, the environment or proxy settings.
`job_run_cli_contract` puts sentinel values in input path components, a rejected
sidecar, a parent environment variable and `HTTP_PROXY`/`HTTPS_PROXY` credentials and
finds none of them, nor any absolute path of the workspace, input or bundle root, in
stdout, stderr or events; an opt-in run adds a failing provider.
**SEC-25.** Providers a request starts inherit the same allowlisted environment as any
other command; a request can name no executable, environment or argument (ADR 0021
section 1), and the sentinel test above covers a credential-bearing proxy variable in
the parent environment.

P11 PR 4 (2026-09-28): `job batch`, the single-host checkpoint, the
[worker-host runbook](../operations/worker-host.md) and the
[P11 qualification record](p11-worker-host.md).
**SEC-T01 status.** Non-adversarial evidence accepted for P11 by the maintainer
(2026-09-28): the strict-Linux attestation checks above and the controls the hardened
`strict-worker-boundary` CI container job verifies (read-only root, no network, CPU,
memory, swap and PID limits, no capabilities, no new privileges, an unprivileged user,
a new process group kept inside the worker cgroup). Adversarial evidence is technical
debt, deferred for maintainer discussion and required before the R0 release (known
limit L-068). SEC-06 and SEC-25 therefore rest, for P11, on controls shown present and
attested, not on a demonstrated containment of a hostile provider.
**SEC-19 status (tenants).** P11 ships no multi-tenant host, and a workspace is one
trust domain: a request delivered to it may target any of its sessions by id and
writes into its one bundle root (an existing bundle of another session is refused,
never replaced). The runbook tells operators to give each tenant or trust domain its
own workspace, input and bundle roots and worker account; SEC-T03 still gates any
multi-tenant host. Results and events name no path, so one tenant's paths never reach
another's output.
**SEC-25 status.** Providers of a batch inherit the same allowlisted environment as any
command, and no request can name an executable, environment or argument; the runbook's
systemd and container deployments give the worker no network, a read-only root, no
host credentials and only the input (read-only), workspace and bundle mounts.
**SEC-20 (X-07, X-08) through the binary.** The `p11_admission_ladder` stage samples a
batch's provider processes at concurrency 1, 2 and 4 in a four-unit workspace and
never sees more weight than the capacity; a batch whose stdout is not read stops
starting requests (backpressure) with bounded memory.

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
