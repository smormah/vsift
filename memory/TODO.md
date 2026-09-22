# VSift project work record

## Current checkpoint

2026-09-22: P06 accepted-plan revalidation implementation `39fb72d` adds the
authority gate immediately before a future install transaction. A saved
`setup plan --json` response is read under the one-MiB and depth-64 limits with
strict unknown-field rejection. Its profile drives a fresh target/catalogue/
configuration/probe/model/time evaluation; the complete public plan and the
separately supplied acceptance digest must both match. Malformed, stale or
mismatched readable inputs fail before network or managed-root mutation. Full
local formatting, strict Clippy, workspace tests, warning-denied rustdoc and
governance pass. A valid plan still
returns `COMMAND_NOT_IMPLEMENTED`; production compatibility smoke, public
install/lifecycle, bounded cleanup, power-loss qualification and D-01..D-10/P06
E2E remain open. P06 stays planned.

2026-09-22: Protected [PR #113](https://github.com/smormah/vsift/pull/113)
merged P06 pinned raw-model staging implementation `2ba8867`, progress record
`9ac8aa1` and hosted evidence record `a4f4982` as `ed77829` from protected
predecessor `55ab879`. The change copies the accepted multilingual `base`
artifact into a private selected
payload and separate unactivated runtime. Exact whole-file identity, portable
name, selected-file digest and runtime inventory are checked; invalid review
fails before payload mutation. The disposable Ubuntu workflow now includes a
fresh publisher-model download and Rust owned-layout check. Local fmt, strict
Clippy, workspace tests, warning-denied rustdoc, actionlint and governance
passed. Fresh [hosted Ubuntu run 35719747666](https://github.com/smormah/vsift/actions/runs/35719747666)
passed all three production owned layouts and the separate model-backed
candidate smoke. Protected Ubuntu, macOS, Windows, Documentation, Governance
and strict-worker checks passed in
[CI run 35720189321](https://github.com/smormah/vsift/actions/runs/35720189321),
[dependency/security run 35720189327](https://github.com/smormah/vsift/actions/runs/35720189327),
and [Rust analysis run 35720189342](https://github.com/smormah/vsift/actions/runs/35720189342).
The first Ubuntu Quality attempt reproduced the existing unrelated P05
cleanup-lock `Busy` symptom; its unchanged failed-job rerun passed. Model
compatibility smoke in the production path, accepted-plan revalidation,
public install/lifecycle, bounded cleanup, power-loss and D-01..D-10/P06 E2E
remain open. P06 stays planned.

2026-09-22: Protected [PR #111](https://github.com/smormah/vsift/pull/111)
merged P06 accepted-source staging bridge implementation `e4ba5bc`, progress
record `ec07c87`, hosted workflow `cb818ac` and evidence record `4854d91`
as `192e012` from protected predecessor `a989f17`. It
rechecks every planned Ubuntu action against the complete reviewed literal
before exposing direct-publisher transfer authority. The accepted entry now
drives bounded archive staging and regular-file runtime assembly for the
FFmpeg and whisper.cpp archives; whole-artifact and selected-payload identity
must match before each step. The pinned-archive opt-in tests and manually
dispatched Ubuntu workflow exercise both archive bridges; full local fmt,
strict Clippy, workspace tests, warning-denied rustdoc and governance passed.
Fresh [hosted Ubuntu run 35668273600](https://github.com/smormah/vsift/actions/runs/35668273600)
passed both production owned archive layouts and the separate model-backed
candidate smoke. Initial protected macOS Quality in
[CI run 35668269011](https://github.com/smormah/vsift/actions/runs/35668269011)
reproduced the existing unrelated P05 cleanup-lock `Busy` symptom, then its
unchanged failed-job rerun passed; Ubuntu, Windows and the other checks passed.
Updated-branch Ubuntu, macOS, Windows, Documentation, Governance and
strict-worker checks passed in [CI run 35718111229](https://github.com/smormah/vsift/actions/runs/35718111229);
[dependency/security run 35718111241](https://github.com/smormah/vsift/actions/runs/35718111241)
and [Rust analysis run 35718111230](https://github.com/smormah/vsift/actions/runs/35718111230)
passed. The raw model staging
path, compatibility smoke, accepted-plan revalidation, public
install/lifecycle, bounded cleanup,
power-loss and D-01..D-10/P06 E2E remain open. P06 stays planned.

2026-09-22: Protected [PR #109](https://github.com/smormah/vsift/pull/109)
merged P06 reviewed Ubuntu 24.04 x86-64 catalogue and deterministic
read-only planning implementation `b74de4d` and progress record `9b97ff9`
as `e533247` from protected predecessor `57cf7c2`. The three direct-origin
entries pin the retained FFmpeg/FFprobe archive, whisper.cpp CLI and multilingual
`base` model with complete selected/runtime inventory, source/notice/trust-limit
disclosures and a 2028-08-01 stop-new-plans cutoff. `setup plan` emits only
missing-component actions and a SHA-256 digest bound to target, catalogue,
probe/model/selection observations and exact actions. Unsupported, expired and
invalid catalogues provide typed no-action/no-digest manual guidance. Existing
tools and model files remain probe/presence-only. `setup install` is still
reserved; this is catalogue/plan authority, not a managed-install completion.
Full local fmt, strict Clippy, workspace tests, warning-denied rustdoc and
governance passed; `cargo deny check` passed with existing duplicate-version
warnings. Protected Ubuntu, macOS, Windows, Documentation, Governance and
strict-worker checks passed in [CI run 35665923881](https://github.com/smormah/vsift/actions/runs/35665923881);
[dependency/security run 35665923929](https://github.com/smormah/vsift/actions/runs/35665923929)
and [Rust analysis run 35665923796](https://github.com/smormah/vsift/actions/runs/35665923796)
passed. P06 stays planned, and issue #66 remains open.

2026-09-21: Protected [PR #105](https://github.com/smormah/vsift/pull/105)
merged P06 managed-version lifecycle implementation `ec6538b` and progress
record `fc3b51a` as `39211bd` from protected predecessor `b32c82b`. It adds
provider-neutral rollback selection and exact removal fencing. A caller holding
the same managed root's installation guard can revalidate and atomically select
an older immutable published version. Every opened published runtime now retains
a shared private per-version OS lock; removal refuses the selected version,
returns typed `InUse` while any process holds the version, and proceeds only with
the exclusive lock. A private tombstone fences new openers while exact
manifest-owned files are deleted. Retries recover after payload, use-lock and
manifest deletion, a tombstone-only directory, and an already completed removal;
substituted directory entries fail closed and are preserved. A native
child-process regression proves removal exclusion and lock release after abrupt
holder exit. Full local fmt, strict Clippy, workspace tests, rustdoc and governance
passed; cargo-deny passed with the existing duplicate-version warnings. Protected
Ubuntu, macOS, Windows, Documentation, Governance and strict-worker checks passed
on the first attempt in [CI run 35633903728](https://github.com/smormah/vsift/actions/runs/35633903728);
[dependency/security run 35633903800](https://github.com/smormah/vsift/actions/runs/35633903800)
and [Rust analysis run 35633903719](https://github.com/smormah/vsift/actions/runs/35633903719)
also passed. This adds no source catalogue, target compatibility or accepted-plan
authority, power-loss guarantee, bounded GC, or public install, rollback or
uninstall command. P06 remains planned; issue #66 remains open.

2026-09-16: Protected [PR #101](https://github.com/smormah/vsift/pull/101)
merged P06 immutable managed-version publication implementation `285d3a1` and
progress record `4a16921` as `6eee0e0` from protected predecessor `a492863`.
A prepared runtime can publish only while holding the same root's installation
guard, under bounded canonical provider-neutral component/version keys.
Publication writes a private exact file/size/SHA-256/mode manifest, closes the
Windows candidate directory handle before its handle-relative rename, then
reopens and rehashes the complete published inventory before atomically
replacing a manifest-digest-bound current pointer. Absent selection lookup is
read-only. Tests cover prior-version retention, a held old-version capability
across update, interruption before and after pointer replacement with
idempotent retry, cross-root guard rejection, linked manifest rejection with
external-file preservation, identity conflict without selection change, and
invalid keys. Local gates and protected Ubuntu, macOS, Windows, Documentation,
Governance and strict-worker passed in [CI run 35079430148](https://github.com/smormah/vsift/actions/runs/35079430148);
[dependency/security run 35079430100](https://github.com/smormah/vsift/actions/runs/35079430100)
and [Rust analysis run 35079430156](https://github.com/smormah/vsift/actions/runs/35079430156)
passed. This adds no power-loss guarantee, catalogue or plan authority,
compatibility smoke, rollback/uninstall/GC, removal fencing or public install
command. P06 remains planned; issue #66 remains open.

2026-09-16: Protected [PR #99](https://github.com/smormah/vsift/pull/99)
merged P06 managed installation-guard implementation `3ff602f`, progress record
`0514737` and Unix import fix `2b5af29` as `e5b6c28` from protected main
`66cbc45`. It adds a root-wide, non-blocking OS lock inside the positively marked
private managed root. The lock must be a single-link regular file and Unix mode
`0600`; another holder returns typed `Busy`, while other lock failures remain
I/O. Tests prove exclusion/release, external-hard-link rejection with source
preservation, and Unix mode rejection. Local gates passed after an initial full
run's three unrelated Windows process-supervisor timing failures passed on
unchanged focused and complete reruns. The first protected CI attempt exposed a
missing Unix permission-extension import; no behavior changed, and corrected
Ubuntu, macOS, Windows Quality, Documentation, Governance and strict-worker
passed in [CI run 35074995313](https://github.com/smormah/vsift/actions/runs/35074995313).
[Dependency/security run 35074995207](https://github.com/smormah/vsift/actions/runs/35074995207)
and [Rust analysis run 35074995323](https://github.com/smormah/vsift/actions/runs/35074995323)
passed. The guard narrows D-05 transaction concurrency risk but provides no plan
authority, publication, rollback or install command. The docs-only protected
record PR #100 first reproduced the existing model-registration `Busy` symptom
in Ubuntu job `104727292434`; its unchanged rerun passed in job `104728936272`.
[Issue #66](https://github.com/smormah/vsift/issues/66) remains open. P06 remains
planned.

2026-09-16: Protected [PR #97](https://github.com/smormah/vsift/pull/97)
merged P06 hosted production-layout checkpoint `f9d2104` as `a405e33` from
protected main `8604370`. The credential-free, manually dispatched Ubuntu 24.04
workflow uses the bounded candidate downloader for fresh pinned whisper.cpp
bytes, passes them through the production Rust owned-artifact/payload/runtime/
discard integration check without execution, and removes the archive on success
or failure. The existing Python experiment then independently downloads pinned
inputs and performs model-backed candidate smoke. Actionlint, five Python
guardrail tests and all local gates passed. Protected Ubuntu, macOS and Windows
Quality, Documentation, Governance and strict-worker passed in
[CI run 35052887409](https://github.com/smormah/vsift/actions/runs/35052887409);
[dependency/security run 35052887516](https://github.com/smormah/vsift/actions/runs/35052887516)
and [Rust analysis run 35052887463](https://github.com/smormah/vsift/actions/runs/35052887463)
passed. Protected [hosted run 35053264628](https://github.com/smormah/vsift/actions/runs/35053264628)
then passed the production layout in 3.48 seconds, cleaned it, and passed the
independent F01 candidate smoke in 23.03 seconds with 291,688 KiB largest-child
peak RSS across the whole smoke. This joins two evidence checkpoints; it does
not accept a catalogue, authorize a plan, activate a version or enable
`setup install`. P06 remains planned.

2026-09-16: Protected [PR #95](https://github.com/smormah/vsift/pull/95)
merged P06 owned runtime layout preparation `15cec51` and its memory update
`1023d24` as `218671d` from protected predecessor `97f13d8`. It copies an exact
reviewed payload into a separate fresh private `runtime.pending` directory.
The reviewed total-copy budget, flat portable/case-insensitive filenames,
selected executables and alias sources are validated before mutation; selected
files and regular alias copies retain their pinned size/SHA-256. Unix selected
executables receive owner-only mode; runtime open/recheck rejects unexpected,
linked, changed, mode-altered or directory-substituted files. Discard removes
only positively owned runtime copies while leaving the original payload and
artifact for explicit disposal. A fresh pinned 9,497,583-byte Ubuntu
whisper.cpp publisher archive passed opt-in owned assembly, six SONAME regular
alias copies, `whisper-cli` mode preparation, all twelve runtime rechecks and
ordered discard without binary execution. Local fmt, strict Clippy, full
workspace tests, warning-denied rustdoc, governance and `cargo deny check`
passed. Protected Ubuntu and macOS Quality first reproduced two existing P05
lock `Busy` failures (jobs `104652588417` and `104652588434`); their unchanged
reruns passed (jobs `104653707688` and `104653707499`), as did Windows,
Documentation, Governance and strict-worker in
[CI run 35051470387](https://github.com/smormah/vsift/actions/runs/35051470387).
[Dependency/security run 35051470403](https://github.com/smormah/vsift/actions/runs/35051470403)
and [Rust analysis run 35051470400](https://github.com/smormah/vsift/actions/runs/35051470400)
passed. [Issue #66](https://github.com/smormah/vsift/issues/66) remains open;
reruns do not identify the lock holder. Catalogue acceptance, compiled-component
source/notice closure for FFmpeg, model/compatibility smoke, version activation,
plan authority, repair/rollback/uninstall and D-01..D-10/P06 E2E remain open.
P06 stays planned.

2026-09-15: Protected [PR #93](https://github.com/smormah/vsift/pull/93)
merged P06 configuration lock classification `2830a70` as `90bd4b8` from
protected predecessor `664d43a`. It maps only `TryLockError::WouldBlock` to
retryable `BUSY`;
an OS lock error now surfaces as storage I/O. A held-lock regression proves
the user-managed record stays unchanged while the lock is held and updates
after release. The sequential model-registration test now names each write in
its error context. Local fmt, strict Clippy, full workspace tests,
warning-denied rustdoc and governance passed. Protected Ubuntu, macOS, Windows
Quality, Documentation, Governance and strict-worker passed in
[CI run 35018869099](https://github.com/smormah/vsift/actions/runs/35018869099);
[dependency/security run 35018869101](https://github.com/smormah/vsift/actions/runs/35018869101)
and [Rust analysis run 35018869095](https://github.com/smormah/vsift/actions/runs/35018869095)
passed. The docs-only PR #92 first Ubuntu Quality job failed the same
intermittent model-registration `Busy` test (run `35017216692`, job
`104543565997`), then its unchanged failed-job rerun passed (job
`104545464013`) and merged as `664d43a`. The exact holder remains unknown;
[issue #66](https://github.com/smormah/vsift/issues/66) stays open. P06 remains
planned, with reviewed layout, source catalogue, installer and D-01..D-10/E2E
still open.

2026-09-15: Protected [PR #91](https://github.com/smormah/vsift/pull/91)
merged P06 read-only unqualified plan implementation `08f1a96` and headless
regressions `8a40634` as `1d79a8d` from protected predecessor `ade30f6`.
`setup plan --profile` now diagnoses
configured or filtered-`PATH` executables and return a complete v1 manual
disposition. No accepted catalogue means `managed_install` is
`unavailable_unqualified`, actions are empty and the acceptance digest is null;
`setup install` remains reserved. Frozen schema/example and CLI contract tests
cover missing tools, an off-PATH configured tool, headless JSONL and corrupt
BYO configuration. Local fmt, strict Clippy,
full workspace tests, warning-denied rustdoc, governance and `cargo deny check`
passed. Protected Ubuntu, macOS, Windows Quality, Documentation, Governance
and strict-worker passed in
[CI run 34989855963](https://github.com/smormah/vsift/actions/runs/34989855963);
[dependency/security run 34989855845](https://github.com/smormah/vsift/actions/runs/34989855845)
and [Rust analysis run 34989855862](https://github.com/smormah/vsift/actions/runs/34989855862)
passed. The exact
FFmpeg compiled-component source/notice review, accepted catalogue,
compatibility/model verification, executable preparation, activation,
installer fault campaigns and D-01..D-10/P06 E2E remain open. P06 stays planned.

2026-09-15: Protected [PR #89](https://github.com/smormah/vsift/pull/89)
merged P06 owned selected-file payload assembly `e0d2ada` with private
initialization correction `60eee40` as `c8d95dd` from protected predecessor
`d377430`. It composes exact-byte artifacts with bounded raw tar, gzip/tar
and XZ/tar readers under a fresh private payload directory.
Exact reviewed flat files are rechecked on open; changed, linked, extra or
replaced-directory content fails closed, and explicit discard removes only
the reviewed selection. A fresh pinned Ubuntu whisper.cpp publisher archive
passed exact-byte import, owned gzip/tar assembly, `whisper-cli` recheck and
discard without execution. Focused storage/substitution tests, full local fmt,
strict Clippy and workspace tests, warning-denied rustdoc, governance and
`cargo deny check` passed. Protected Ubuntu, macOS, Windows Quality,
Documentation, Governance and strict-worker checks passed in
[CI run 34985416473](https://github.com/smormah/vsift/actions/runs/34985416473);
[dependency/security run 34985416294](https://github.com/smormah/vsift/actions/runs/34985416294)
and [Rust analysis run 34985416283](https://github.com/smormah/vsift/actions/runs/34985416283)
passed. The exact Ubuntu FFmpeg candidate still needs compiled-component
source/notice review before catalogue acceptance. Plan authority, alias and
executable preparation, smoke, activation, lifecycle and D-01..D-10/P06 E2E
remain open. P06 stays planned.

2026-09-15: Protected [PR #87](https://github.com/smormah/vsift/pull/87)
merged P06 direct publisher transport implementation `c058756` as `0d36160`
from protected predecessor `3a56ac2`. It adds immutable reviewed GitHub-release
and Hugging Face model routes, origin-specific HTTPS CDN redirects, system TLS,
deadlines and cancellation. It writes only to the existing private unactivated
stage, verifies exact size/SHA-256, and discards interruptions without resume.
The pinned 9,497,583-byte Ubuntu whisper.cpp asset passed an opt-in fresh
publisher download without execution. Focused route/redirect/cancellation and
streaming short/excess/digest/cleanup tests, local fmt, strict Clippy,
workspace tests, warning-denied rustdoc, governance and `cargo deny check`
passed. Protected Ubuntu, macOS and Windows Quality, Documentation,
Governance and strict-worker checks passed in [CI run 34981240230](https://github.com/smormah/vsift/actions/runs/34981240230);
[dependency/security run 34981240514](https://github.com/smormah/vsift/actions/runs/34981240514)
and [Rust analysis run 34981240137](https://github.com/smormah/vsift/actions/runs/34981240137)
passed. Catalogue acceptance remains blocked by
the exact FFmpeg build's compiled-component source/notice review; setup plan,
selected-file assembly, smoke, activation, D-01..D-10 and P06 E2E remain open.
P06 stays planned.

2026-09-15: Protected [PR #85](https://github.com/smormah/vsift/pull/85)
merged P06 owned unactivated artifact staging `f6c3509` as `3ad5bf6`
from predecessor `eb56928`. The private per-user managed-data root and each
fresh stage use exact ownership markers; import and later open verify the
reviewed size/SHA-256, failed transfers discard only known stage files, and
unowned or unexpectedly changed storage is preserved and rejected. Focused
Windows tests cover valid import/discard, changed source/staged bytes,
unowned roots and unexpected content; Ubuntu CI also passed a Unix stage-name
substitution regression. Local fmt, strict Clippy, full workspace tests,
warning-denied rustdoc, governance and dependency policy passed. Protected
[CI run 34953469291](https://github.com/smormah/vsift/actions/runs/34953469291)
passed Ubuntu, macOS and Windows Quality, Documentation, Governance and
strict-worker checks; [Security run 34953469302](https://github.com/smormah/vsift/actions/runs/34953469302)
and [CodeQL run 34953469319](https://github.com/smormah/vsift/actions/runs/34953469319)
passed. The Ubuntu candidate catalogue is still unaccepted because the exact
FFmpeg build's compiled-source/notice disclosure remains open. Direct HTTPS,
plan acceptance, archive assembly in the owned root, smoke, activation and
D-01..D-10/P06 E2E remain open. P06 stays planned.

2026-09-15: Protected [PR #83](https://github.com/smormah/vsift/pull/83)
merged P06 contained selected-file staging `57b8113` as `b5d8769` from
predecessor `e75d560`. Raw tar, gzip/tar and XZ/tar readers can now
write only reviewed regular files under portable flat basenames through an
empty private directory capability. Writes are create-new/no-follow with
private modes; archive directories, links and modes are ignored. Focused tests
cover hash and later-inventory failure cleanup, hidden gzip/XZ content,
nonempty-directory preservation, basename collisions and Windows reserved
names. Fresh pinned FFmpeg and whisper.cpp archives passed contained staging
without binary execution. Full workspace tests, fmt, strict Clippy,
warning-denied rustdoc, governance and `cargo deny check` passed. Protected
Ubuntu, macOS and Windows Quality, dependency, docs, governance, strict-worker
and CodeQL/Rust checks all passed on the reviewed commit. This creates no
managed root and performs no download, plan acceptance, smoke or activation;
P06 remains planned.


2026-09-14: Protected [PR #81](https://github.com/smormah/vsift/pull/81)
merged P06 selected-file integrity `5454dcb` as `d9861a6`.
The bounded raw-tar, gzip/tar and XZ/tar readers now verify a nonempty reviewed
selection of exact regular-file paths, sizes and SHA-256 while still validating
the whole inventory. Focused tests and workspace tests pass. Pinned publisher
archive opt-in tests now include the selected digests; their new assertions
passed fresh pinned-asset runs without binary execution. Direct HTTPS,
contained staging, managed catalogue/plan acceptance, smoke and activation
remain open. P06 stays planned.
Local fmt, strict Clippy, workspace tests, warning-denied rustdoc, governance
and `cargo deny check` passed. Protected Ubuntu, macOS and Windows Quality,
dependency, docs, governance, strict-worker and CodeQL/Rust checks passed after
an unchanged Ubuntu rerun. The first Ubuntu job hit the known intermittent
`Busy` failure in `model_registration_preserves_executables_and_replaces_only_the_model`
(run `34897284569`, job `104154428124`); the lock cause remains open in
[issue #66](https://github.com/smormah/vsift/issues/66).

2026-09-14: Protected [PR #79](https://github.com/smormah/vsift/pull/79)
merged P06 XZ/tar inspection `2a960ba` as `ad2a844`. It adds one-stream,
read-only decode under a 128 MiB compressed-input ceiling and 128 MiB decoder
block-memory limit before the existing bounded tar policy. The exact publisher Ubuntu
FFmpeg archive passed opt-in size/SHA-256 and 73-entry Rust inventory without
binary execution. Local fmt, strict Clippy, workspace tests, warning-denied
rustdoc, governance and `cargo deny check` passed. Protected Ubuntu, macOS and
Windows Quality, dependency, docs, governance, strict-worker and CodeQL/Rust
checks passed after an unchanged Ubuntu rerun. The first Ubuntu job repeated
P05 `Busy` in the hard-link cleanup test, recorded in
[issue #66](https://github.com/smormah/vsift/issues/66). Total process/time
qualification, selected-file extraction, contained staging, catalogue and
activation remain open; P06 stays planned.

2026-09-14: Protected [PR #77](https://github.com/smormah/vsift/pull/77)
merged P06 gzip tar inspection `f553d98` and pinned archive regression
`c52df98` as `caf46ad`. It reads all gzip members through
the pinned pure-Rust `flate2` 1.1.10 backend under a 256 MiB maximum compressed
input limit, then applies the existing bounded tar inventory policy. Focused
tests cover valid, truncated, corrupt, oversized and hidden-second-member
streams. The exact publisher whisper.cpp archive passed opt-in read-only
inventory after size/SHA-256 verification (`c52df98`), without execution.
Local fmt, strict Clippy, workspace tests, warning-denied rustdoc,
governance and `cargo deny check` passed. Protected Ubuntu, macOS and Windows
Quality, dependency, docs, governance, strict-worker and CodeQL/Rust checks
passed.
XZ decoding, selected-file extraction, staging, catalogue and activation remain
open; P06 stays planned.

2026-09-14: Protected [PR #75](https://github.com/smormah/vsift/pull/75)
merged P06 raw-tar inventory reader `01ba396` and PAX regression `c6fad01`
as `e0da4b8`. It adds bounded sequential metadata inspection and rejects
truncation, trailing content and special headers before any future extraction.
The reviewed
`tar` 0.4.46 dependency is infrastructure-only. Local fmt, strict Clippy,
workspace tests, warning-denied rustdoc, governance and `cargo deny check`
passed. Protected Quality on Ubuntu, macOS and Windows, dependency, docs,
governance, strict-worker and CodeQL/Rust checks passed. The first Ubuntu run
hit the existing P05 `Busy` symptom in two lifecycle tests; unchanged rerun
passed and [issue #66](https://github.com/smormah/vsift/issues/66) records it.
Publisher XZ/GZIP decompression, selected-file extraction, contained staging,
catalogue and activation remain open; P06 stays planned.

2026-09-14: After protected [PR #72](https://github.com/smormah/vsift/pull/72)
merged the fixed Ubuntu provenance references as `8e299e4`, protected
[PR #73](https://github.com/smormah/vsift/pull/73) merged the P06 archive-
inventory policy (`7e73b95`, merge `12749a9`). It added portable path,
duplicate, reviewed-link and expansion-budget validation over untrusted entry
metadata. Local fmt, strict Clippy, workspace tests and governance passed;
Ubuntu, macOS, Windows Quality, Documentation, Governance, dependency,
strict-worker and CodeQL/Rust checks passed. It has no parser, extraction,
contained staging or activation; D-04 and P06 remain open.

2026-09-14: P06 Ubuntu candidate provenance review identified BtbN's retained
release-tag build-scripts commit `8267213e`, upstream FFmpeg commit `e47273f4`
from the binary version, and whisper.cpp tag commit `306c88f4`. The candidate
record links source/build inputs and proposes 2028-08-01 as the last date for
new plans before the publisher's two-year retention window. This does not
constitute a binary signature, complete compiled-source proof or an accepted
managed catalogue. P06 remains planned.

2026-09-14: Protected [PR #68](https://github.com/smormah/vsift/pull/68)
merged the Ubuntu diagnostic runner as `78e005d`. Its first macOS Quality
attempt hit the existing intermittent P03 `Busy` failure in
[issue #66](https://github.com/smormah/vsift/issues/66); the unchanged-commit
rerun and all other required checks passed. Opt-in
[hosted run 34851335044](https://github.com/smormah/vsift/actions/runs/34851335044)
passed pinned archive/model verification and F01 model-backed tone inference.
It observed 23.05 seconds and 290,820 KiB largest child peak RSS across the
smoke. Build configuration and an LGPL statement were captured. Full binary
notice/source review, model resource/accuracy qualification, catalogue,
installer and D-01..D-10/P06 E2E remain open; P06 stays planned.

2026-09-14: The P06 verified-stream primitive merged through protected
[PR #67](https://github.com/smormah/vsift/pull/67) as `cda41cf`; all supported
OS quality, documentation, governance, strict-worker, dependency and security
checks passed. On `codex/p06-ubuntu-resource-evidence`, the opt-in hosted
candidate runner is being extended to print the pinned FFmpeg configuration/
licence statement and F01 elapsed/RSS observations. Local archive/diagnostic
tests pass. Hosted execution and catalogue assessment are pending; P06 is planned.

2026-09-14: P06 verified-stream work on `codex/p06-verified-stream` adds a typed
reviewed size/SHA-256 requirement and bounded transfer over injected streams.
It does not fetch or activate a dependency. Local fmt, strict Clippy and full
workspace tests pass; protected checks and merge evidence are pending. The
source-notice record merged through protected [PR #65](https://github.com/smormah/vsift/pull/65)
as `a554586` after all required checks passed on a rerun. Its first Ubuntu
attempt failed an existing P03 shared-lifetime-lock test with `Busy`; the
unchanged-commit rerun passed. [Issue #66](https://github.com/smormah/vsift/issues/66)
tracks that unexplained intermittent failure. P06 remains planned.

2026-09-14: `setup configure-model` merged through protected
[PR #64](https://github.com/smormah/vsift/pull/64) as `15ed193`; Ubuntu, macOS,
Windows, documentation, governance, strict-worker, dependency and security
checks passed. P06 remains planned. A read-only source review on
`codex/p06-ubuntu-source-review` reconfirmed the pinned FFmpeg archive hash and
identified its LGPL v3 `LICENSE.txt` text without executing it. The selected
whisper.cpp and pinned model repositories declare MIT; full build source and
notice disclosure remain open before managed plan acceptance.

2026-09-14: P06 persistent BYO executable registration merged through protected
[PR #63](https://github.com/smormah/vsift/pull/63) as `55c8a6d`. Ubuntu,
macOS and Windows Quality, Documentation, Governance, strict-worker,
dependency policy/review and CodeQL/Rust analysis passed. The Unix extension
import correction is included. P06 remained planned. The subsequent
`codex/p06-model-selection` increment recorded a user-selected nonempty model
file without loading it; its protected merge is recorded above. Model-backed
compatibility, managed installation, D-01..D-10 and P06 E2E remain open.

2026-09-13: P06 persistent BYO executable registration was implemented
from protected main `4c8dc36` on `codex/p06-persistent-byo`. It stores canonical
per-user FFmpeg/FFprobe/whisper.cpp paths without executing them; `setup check`
probes selected paths with per-call precedence. At that increment, model
persistence, compatibility, managed installation, D-01..D-10 and P06 E2E
remained open. Local fmt, strict Clippy, workspace tests, warning-denied
rustdoc, governance, cargo-deny and diff check passed. Protected checks and
merge are recorded above.

2026-09-13: A read-only Ubuntu x64 P06 candidate review independently verified
the publisher archive hashes and selected files for the retained month-end
BtbN FFmpeg build and upstream whisper.cpp v1.9.2 CLI. The whisper archive
contains eight links; no binary ran on the maintainer desktop. Hosted
compatibility, exact link-safe layout, notices and production installer gates
remain open. P06 is still planned.
An opt-in credential-free Ubuntu 24.04 candidate smoke and four local archive
guardrail tests are prepared; hosted execution and review are pending.
The [hosted Ubuntu run 34782768288](https://github.com/smormah/vsift/actions/runs/34782768288)
passed on Ubuntu 24.04.5 x64 with pinned archive/model hashes, non-link
selected-file layout, F01 media operations and model-backed tone inference.
It does not qualify speech accuracy, resource use or the production installer.

2026-09-13: The maintainer clarified P06's intended download posture: an
explicitly accepted plan fetches a reviewed pinned provider artifact directly
from its publisher on the user's machine, not through a VSift mirror/proxy.
"Latest" is only a candidate for later catalogue review, never an unreviewed
runtime choice. ADR 0014 and the source review now record this; licence/notice,
expiry, compatibility and installer gates remain open, and P06 stays planned.

2026-09-13: P05 issue #51's lock-lifetime fix merged through protected PR #58 as
`9f86ff1` after the three-OS quality and security checks passed. The opt-in
[100-run Ubuntu stress](https://github.com/smormah/vsift/actions/runs/34756593985)
passed. The precise original CI lock holder was not captured; the explicit
root-lock release and regressions mitigate the observed `scan 0: Busy`.
P06 retained-source PR #57 then passed protected checks and merged as `b7e88af`.
Its [hosted Windows candidate smoke](https://github.com/smormah/vsift/actions/runs/34756838957)
passed exact hashes, FFprobe/FFmpeg F01 operations and whisper.cpp model loading
on Windows Server 2025. This is not installer, real-speech, Windows 11 or P06
E2E qualification. P06 remains planned and P07/P08 remain ineligible.

2026-09-13: Protected PR #55 merged the opt-in disposable Windows candidate
smoke (`a0fe266`, merge `1c805c1`). Hosted
[run 34737109736](https://github.com/smormah/vsift/actions/runs/34737109736)
passed archive/model hashes, selected-file verification, FFmpeg/FFprobe F01
operations and whisper.cpp model-backed inference on Windows Server 2025.
It is not an accepted installer catalogue, real-speech transcript-accuracy
test, Windows 11 qualification or P06 E2E completion.

2026-09-13: P06 read-only detection/guidance increment merged through protected
[PR #53](https://github.com/smormah/vsift/pull/53) as
`0f0156bd62ac6777fcb1f412968df6eda977158e`. Windows, macOS and Ubuntu
Quality, Governance, Documentation, strict worker, dependency policy/review,
CodeQL and Rust analysis passed. A Windows x64 artifact candidate review now
records checked FFmpeg/whisper archive hashes and a pinned multilingual `base`
model LFS pointer, but remains unqualified pending licence/notice and isolated
compatibility gates (candidate record `2256582`). No installer ran and P06 is
not complete.

2026-09-13: P06 has begun on `codex/p06-detection-guidance` from protected main
`3c3c93f236b8c34491295fb05a52a2b162e8b1ee` (implementation commit
`3a1da55`). The first read-only increment
adds per-call absolute FFmpeg/FFprobe/Whisper selection, truthful executable-only
scope and typed manual/BYO remediation. It does not persist selections, validate
provider/model compatibility or authorize downloads. P06 remains planned in the
delivery ledger until its complete D-suite, one reviewed managed target and E2E
checkpoint merge. Local fmt, strict Clippy, workspace tests, warning-denied
rustdoc, governance, diff check and cargo-deny passed; protected PR evidence
and merge reference to follow.

2026-09-13: The maintainer confirmed the R0 setup journey in ADR 0014:
detect existing/partly installed components first, explicitly plan/install
reviewed missing ones, and always provide typed manual/BYO guidance if managed
installation is unavailable, denied or fails. Script-installed off-PATH tools
must be selectable. Agents need rich remediation but cannot infer install
authority from video inspection. The BYO-only proposal PR #49 closed unmerged;
P06 source-review PR #48 was carried forward and closed as superseded by PR #50.
The source assessment and clarification commits `b1c271d` and `6ab6ef3`
merged through protected PR #50 as
`52127e0e68171395fd8ec97f5604edeea5045b65`. P06 remains planned, with
its immutable provider/model catalogue still a gate for each managed target.
This changes acceptance, not runtime behavior.

2026-09-12: P06 source review started from protected main
`777bc3e56788d43cd9a647dba54c287390c2998d`; P05's evidence follow-up is
merged and issue #8 is closed. A reviewed cross-target provider/model artifact
catalog is absent. P06 implementation is gated by the source and licence decision
recorded in `docs/planning/p06-provisioning-source-review.md`.

2026-09-12: P05 implementation from protected main
`8741f57dfc5b45c8cb4d3f1f7791d0f1d143c627` merged through protected PR #46 as
`c3f9313f8df0871d17ab80ad5bb142be6421b36f`. The ledger now records
completion and the passed three-OS quality, governance, documentation,
security and local checkpoint evidence. Its evidence follow-up has since merged
and issue #8 is closed. P06 is the next eligible packet and remains planned.

2026-09-12: P04 source/media primitives completed through protected PR #44 as
`4fc859b3344bd47c254dd9da9cac72f5ad3d61d5`. This evidence-only follow-up
records its completed ledger status and final test evidence. Issue #7 closed after
that record merged, making P05 eligible at this checkpoint.

2026-09-11: P03's implementation completed through protected PR #42 as
`3eef9b7ac3bcfe092d82137ccf2aa9aa084aca4f`. Its private provisioning and ACL
checks, weighted admission, lifetime holds, fenced generations and process-crash
recovery remain internal; no storage/session command is exposed. This evidence-only
follow-up records the completed ledger state; issue #6 is closed. P04 is now the next
eligible packet but remains planned.

## Pending

- P06 next critical path after `b74de4d`: wire install to the reviewed catalogue
  and fresh accepted-plan digest under the installation guard; run exact target
  compatibility smoke before atomic publication. Implement public
  install/repair/list/rollback/remove, bounded owned-stage/tombstone/version
  cleanup, power-loss and interruption qualification, and D-01..D-10/P06 E2E.
  Do not infer model/provider compatibility from a version probe or model file
  presence. Keep other targets on typed manual/BYO guidance.
- P06 target gate: the exact Ubuntu 24.04 x86-64 catalogue is accepted for
  read-only plans, but no complete managed-install target is yet qualified.
  The source matrix does not justify a three-target installer. Complete the
  Ubuntu installer and test typed manual/BYO fallback, permission denial and
  off-PATH selection on every named R0 target. See
  `docs/planning/p06-provisioning-source-review.md`. P06 remains planned;
  D-01..D-10, E2E, ledger completion and issue #9 closure remain pending.
- P06 candidate smoke follow-up: measure peak resources and real-speech
  transcription on an owned fixture once P07 supplies speech, then qualify
  Windows 11 desktop separately. Tone-only runner evidence accepted only a
  disclosed source catalogue for planning; it does not qualify the complete
  installer or E2E gate. No legal-clearance claim is made.
- P06 source availability: the reviewed 2026-09-09 BtbN FFmpeg daily asset is
  subject to its 14-build retention policy; its successful hosted smoke does
  not make it a durable installer source. The 2026-08-31 month-end alternative
  (PR #57, `b7e88af`) is now the reviewed planning anchor with a 2028-08-01
  stop-new-plans cutoff. Replace or revoke it through a reviewed catalogue
  revision before retention expiry; never resolve live `latest`.
- P06 next increments: validate real-speech behavior, resource use and model
  selection rather than accepting tone-audio inference alone; compose accepted
  plans with direct download, exact layout and smoke before activation; then
  implement public repair/list/rollback/remove and native three-OS failure/E2E
  evidence. Do not mark probe-only success as ASR readiness or let an agent
  infer installation approval.
- FS-01: OS/storage crash qualification is missing. The default cap-std NTFS
  read-only directory handle fails synchronization; a safe writable-directory
  handle succeeds. Do not misreport this as Windows durability being impossible.
  ADR 0010 accepts the narrower desktop profile; do not treat this as durable evidence.
  Supply an owned disposable Ubuntu/ext4 fault environment before P10/P11 durable
  enablement and keep explicit durable requests fail-closed until then.
- Resolve baseline findings B-01..B-11 through their mapped implementation packets.
- P03 spike implementation: `5f15c7730607570663d739b7a4dd70a03947e500`, merged
  through PR #24 as `cbc531e80761078354be0b9942c52f00ddac05b0`.
  Local Windows validation: 92 tests, fmt, strict clippy, governance and rustdoc pass.
  Security review enabled cargo-deny's development licence/duplicate checks and
  recorded the existing borrow-or-share MIT-0 licence review. Follow-up
  `a262fa9bbabb4bebc6ebde581204c4dbe0a8186d` passed ten storage experiments each
  on NTFS, APFS and ext4 in CI run 34585520298; strengthened dependency checks pass
  in run 34585520299. Exact environments and the unresolved OS/storage crash gate
  are recorded in the P03 feasibility document. P03's narrower ephemeral profile is
  complete; this durable qualification remains P10/P11/P14 work.
- Preserve the complete R0 local-video-to-grounded-handoff journey. It must pass with
  supplied-transcript and local-ASR paths through named Codex and Claude Code clients;
  do not defer product usefulness to R1.
- Track the cumulative opt-in E2E spine in `docs/planning/e2e-test-spine.md` and issue
  #40. P04 and P05 have real-media checkpoints, with P06-P14 and the complete
  journey explicitly `not_implemented`. Every later packet must attach its production
  stage or explicitly record why none applies; P12/P14 cannot substitute for missing
  earlier integration.
- Preserve managed indexing, optional enrichment, source-grounded reconstruction,
  industrial worker growth and integrated qualification as R1 packets P15..P20 under
  ADR 0011 and `docs/planning/r1-industrial-capability-expansion.md`. SQLite remains
  only a candidate explicit single-node adapter. Do not start P15 implementation before
  P14 or use R1 scope to expand the active P03 packet. GitHub milestone 2 contains
  future issues #25 through #30.

## Completed

- 2026-09-22: P06 reviewed-source and plan increment `b74de4d` pins the
  Ubuntu 24.04 x86-64 direct-origin catalogue, exact archive/runtime inventory,
  expiry and disclosures, and emits state-bound deterministic read-only plans.
  Unsupported, expired and invalid catalogue states have no managed actions or
  acceptance digest. Full local fmt, strict Clippy, workspace tests, rustdoc,
  governance and dependency policy passed. Protected merge evidence pending;
  P06 itself remains planned.
- 2026-09-13: P05 registration-lock follow-up #51 merged through protected
  PR #58 as `9f86ff1`, with explicit root-lock release, duplicate-handle
  regression, three-OS Quality and
  [100-run Ubuntu stress](https://github.com/smormah/vsift/actions/runs/34756593985).
  The original holder remains unproven. P06 retained Windows source review
  PR #57 merged as `b7e88af`; the
  [hosted candidate smoke](https://github.com/smormah/vsift/actions/runs/34756838957)
  passed hash verification, F01 media extraction and model-backed inference.
  These are qualification increments, not P06 completion.

- 2026-09-13: ADR 0014's detect → explicit install → manual guide R0 setup
  contract and the P06 provider/model source assessment merged through protected
  PR #50 as `52127e0e68171395fd8ec97f5604edeea5045b65` (branch commits
  `b1c271d`, `6ab6ef3`). Local fmt, strict Clippy, workspace tests, rustdoc,
  governance and diff checks passed. Latest protected Quality passed on Windows,
  macOS and Ubuntu; Governance, Documentation, strict-worker, dependency
  policy/review, CodeQL and Rust analysis passed. An earlier Ubuntu attempt
  failed the existing P05 registration scan with `Busy`; issue #51 preserves
  that unresolved investigation. P06 is not implemented by this record.

- 2026-09-12: P05 implementation commit
  `1238580a6a86a433019fb2ecabfeec3eef56ea43` and fixture correction
  `8e7232c86d1d324b8ccd7e8e5ba271d813544ee5` merged through protected
  PR #46 as `c3f9313f8df0871d17ab80ad5bb142be6421b36f`.
  Foreground ephemeral ingestion, typed lifecycle, bounded list/cleanup,
  source-safe retention and data-only bundle validation passed local gates,
  native FFmpeg/FFprobe P05 checkpoint, three-OS Quality, Governance,
  Documentation, strict-worker, dependency policy/review, CodeQL and Rust
  analysis. The bundle publication limit is recorded in ADR 0013.

- 2026-09-12: P04 merged through protected PR #44 as
  `4fc859b3344bd47c254dd9da9cac72f5ad3d61d5`. Internal no-follow source
  snapshots, typed restricted FFprobe/FFmpeg metadata/frame/audio operations,
  deterministic project-owned fixtures, independent pixel/timestamp verification and
  the opt-in seven-scenario real-media checkpoint landed. Local fmt, strict Clippy,
  workspace tests, warning-denied rustdoc, governance and cargo-deny passed. The
  generator reproduced the same provenance SHA-256 on a second same-build run; the
  verifier checked 11 source clips and seven malformed variants. Protected Quality
  passed on Ubuntu, Windows and macOS; Governance, Documentation, strict-worker,
  dependency policy/review, CodeQL and Rust analysis passed. Native FFmpeg 9.0 on
  Windows/NTFS is development evidence, not a release support claim. ADR 0012
  records the remaining desktop decoder isolation limits; P05-P14 and the complete
  E2E journey remain `not_implemented` at this checkpoint.

- 2026-09-11: P03 completed in protected PR #42 as
  `3eef9b7ac3bcfe092d82137ccf2aa9aa084aca4f`. It added owned private-root
  provisioning, Unix owner/mode and Windows DACL validation, immutable root-wide
  weighted admission, cross-process shared/exclusive lifetime coordination, fenced
  immutable generation publication, bounded linked-manifest verification and recovery
  at every manifest/pointer write, flush and rename boundary. Durable initialization
  and publication fail before mutation. The local workspace passed 128 tests (three
  child-process entries intentionally ignored and launched by parent tests), fmt,
  strict Clippy, rustdoc, governance and cargo-deny. All protected three-OS quality,
  governance/documentation, strict-worker, dependency and CodeQL/Rust checks passed.
  The adapter remains internal, strict OS/storage crash durability remains assigned to
  P10/P11/P14, and P04 remains planned until separately started.
  PR #43's first Windows run also exposed a timing-only concurrency-test weakness;
  its bounded monotonic retry fix passed ten consecutive targeted runs and the full
  local gate set before protected checks were rerun.

- 2026-09-11: P03 implementation increments PR #35
  (`9ee3c048e1460008cd4f6c3e16dc23f78115ad0d`) and PR #36
  (`65fe00c3d43405a6ed5c8bda8ae50a2896e80d65`) established typed storage
  guarantees plus the internal capability-scoped generation-zero initializer. The
  second increment passed 105 local tests and all protected cross-platform/security
  checks after its Unix lint regression was corrected. A later Windows run exposed
  time-only temporary-root naming as intermittently non-unique under parallel tests;
  PR #38 (`3fd29247bb241e4aa6f5561e410a06ed5d1cf839`) added a process-local
  sequence and a same-timestamp regression test. The corrected suite passes 106 local
  tests and every protected job. These increments do not complete P03 or expose a
  storage/session command.
- 2026-09-11: the narrower P03 storage qualification profile merged through PR #33
  as `c2b3829d77279a32b3487ab1170f820f1d68eeb5`. ADR 0010 permits
  process-crash-consistent ephemeral desktop publication, keeps explicit durable
  requests fail-closed, and assigns strict Ubuntu/ext4 durability qualification to
  P10/P11/P14. P03 implementation remains pending.
- 2026-09-11: the R1 managed industrial capability expansion merged through PR #31
  as `0cfdb407f805282995f326ca93c99bc7170eda04`. ADR 0011 preserves a complete R0
  video-to-grounded-handoff release gate and reserves R1 requirements R-15..R-20,
  packets P15..P20, threats SEC-26..SEC-35 and E/RC/I/H/Q tests. Milestone 2 and
  issues #25-#30 hold future R1 work; R0 issues #15/#17 carry the two-agent gate.
- 2026-09-09: foundation and GitHub governance published in `5289a2b`; support channel
  update in `d7a459e`.
- 2026-09-09: setup command rename merged through PR #1 as `df85f70`.
- 2026-09-09: source review of `df85f70` and comprehensive implementation proposal
  merged through PR #2 as `e3f8569`. It adds no runtime implementation.
- 2026-09-10: P00 merged through PR #18 as `924f6c5`. It accepted DEC-01..13,
  established milestone 1 and issues #3-#17, froze F01-F12 declarative fixture truth,
  and added a machine-validated ledger plus a required protected-branch Governance
  check. All PR checks passed; the workspace had 13 passing tests.
- 2026-09-10: P01 merged through PR #20 as `3d7a7d2`. It published the typed
  R0 CLI/config/JSON boundary, four v1 schemas, domain value contracts, stable
  errors/exits, bounded presentation and C-01..C-10 coverage. All protected checks
  passed across Ubuntu, Windows and macOS; the local workspace had 64 passing tests.
- 2026-09-11: P02 merged through PR #22 as `4e9ef08`. It added canonical provider
  resolution, shell-free process supervision, bounded concurrent pipes, one operation
  deadline, sticky cancellation, Windows Job Object/Unix process-group cleanup, and
  honest strict-isolation reporting. P-01..P-08 and C-05 passed across protected
  three-OS and strict Linux checks; the local Windows workspace had 82 passing tests.
