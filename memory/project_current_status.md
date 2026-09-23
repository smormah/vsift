# VSift current status

## Active

2026-09-23: Protected [PR #118](https://github.com/smormah/vsift/pull/118)
merged P06 compatibility-policy implementation `440f42d`, generated-artifact
bounds `799bfc7` and parking record `c5a85e1` as `3b2b9d5`. Full local gates
passed. Protected CI `35868919085`, dependency/security `35868919126` and Rust
analysis `35868918990` passed; the first macOS Quality attempt reproduced the
known unrelated P05 storage-lock `Busy` symptom and its unchanged failed-job
rerun passed. P06 is parked at the reviewed-policy boundary. The accepted Ubuntu
catalogue, strict saved-plan revalidation, exact staging/runtime primitives and
lifecycle primitives remain available, but no public install path is enabled.
Resume with the production bounded compatibility executor and cleanup evidence
before composing activation or exposing lifecycle commands. The ordered remaining
scope is recorded in `memory/TODO.md`; P06 remains planned.

2026-09-22: P06 reviewed compatibility-policy implementation `440f42d` and
generated-artifact bounds `799bfc7`
advances the accepted Ubuntu catalogue to revision
`ubuntu-24.04-x86_64-2026-09-22-r2`. It fixes the exact checked-in F01
size/SHA-256, expected FFmpeg/FFprobe build prefixes, 64-KiB per-stream and
transcript limits, 256-KiB generated-audio limit, 60-second media and 180-second
inference deadlines, and 16-kHz mono extraction.
The application rejects incomplete or out-of-policy catalogues without actions
or an acceptance digest. Every policy value is included in the canonical plan
digest, so a change cannot reuse an earlier acceptance. Focused application and
infrastructure tests pass. The production executor does not yet run the policy;
smoke result/failure cleanup, activation, public install/lifecycle, bounded GC,
power-loss qualification and D-01..D-10/P06 E2E remain open. P06 stays planned.

2026-09-22: Protected [PR #115](https://github.com/smormah/vsift/pull/115)
merged P06 accepted-plan revalidation implementation `39fb72d` and documentation
record `71bb29d` as `a7180d7` from protected predecessor `c982d84`.
[PR #116](https://github.com/smormah/vsift/pull/116) then merged shared strict
plan DTO refactor `cd6d00d` as `42df227`. A saved `setup plan --json` response
is now read under the one-MiB and depth-64 limits with
strict unknown-field rejection. The saved profile drives a fresh target,
catalogue, configuration, executable-probe, model and time evaluation. The
complete public data and separately supplied digest must still match before the
reserved install path can proceed. Malformed, stale or mismatched readable plans
fail before network or managed-root mutation. Full local formatting, strict
Clippy, workspace tests, warning-denied rustdoc and governance pass. Protected
Ubuntu, macOS, Windows, Documentation, Governance and strict-worker checks passed
for the implementation in CI `35790319080`, dependency/security `35790319171`
and Rust analysis `35790319082`; the DTO follow-up passed the same protected set
in CI `35791005761`, dependency/security `35791005738` and Rust analysis
`35791005741`. Even a
valid accepted plan still returns
`COMMAND_NOT_IMPLEMENTED`; compatibility smoke, the public install/lifecycle
transaction, bounded cleanup, power-loss qualification and D-01..D-10/P06 E2E
remain open. P06 stays planned.

2026-09-22: Protected [PR #113](https://github.com/smormah/vsift/pull/113)
merged P06 pinned raw-model staging implementation `2ba8867`, progress record
`9ac8aa1` and hosted evidence record `a4f4982` as `ed77829` from protected
predecessor `55ab879`. The change copies the accepted multilingual `base`
artifact into a private selected
payload and separate unactivated runtime. The source is rehashed before copy;
whole-artifact identity, portable filename, selected-file digest and runtime
inventory are checked. Invalid review fails before payload mutation. The
disposable Ubuntu workflow now runs a fresh pinned publisher-model download
through the production owned layout. Local fmt, strict Clippy, workspace
tests, warning-denied rustdoc, actionlint and governance passed. Fresh
[hosted Ubuntu run 35719747666](https://github.com/smormah/vsift/actions/runs/35719747666)
passed all three production owned layouts and the separate model-backed
candidate smoke. Protected Ubuntu, macOS, Windows, Documentation, Governance
and strict-worker checks passed in
[CI run 35720189321](https://github.com/smormah/vsift/actions/runs/35720189321),
[dependency/security run 35720189327](https://github.com/smormah/vsift/actions/runs/35720189327),
and [Rust analysis run 35720189342](https://github.com/smormah/vsift/actions/runs/35720189342).
The first Ubuntu Quality attempt reproduced the existing unrelated P05
cleanup-lock `Busy` symptom; its unchanged failed-job rerun passed. Production
compatibility smoke,
accepted-plan revalidation, public install/lifecycle, bounded cleanup,
power-loss and D-01..D-10/P06 E2E remain open. P06 stays planned.

2026-09-22: Protected [PR #111](https://github.com/smormah/vsift/pull/111)
merged P06 accepted-source staging bridge implementation `e4ba5bc`, progress
record `ec07c87`, hosted workflow `cb818ac` and evidence record `4854d91`
as `192e012` from protected predecessor `a989f17`. It
binds proposed Ubuntu actions back to exact current reviewed literals before
publisher transfer. The FFmpeg and whisper.cpp archive paths now derive
inventory, selected-file hashes, link headers, runtime aliases and executable
modes from that entry, rejecting mismatched whole-artifact or selected-payload
identity before unactivated preparation. The opt-in real-archive checks and
manually dispatched Ubuntu workflow use both archive bridges. Local fmt,
strict Clippy, workspace tests, warning-denied rustdoc and governance passed.
Fresh [hosted Ubuntu run 35668273600](https://github.com/smormah/vsift/actions/runs/35668273600)
passed both production owned archive layouts and the separate model-backed
candidate smoke. Initial protected macOS Quality in
[CI run 35668269011](https://github.com/smormah/vsift/actions/runs/35668269011)
reproduced the existing unrelated P05 cleanup-lock `Busy` symptom, then its
unchanged failed-job rerun passed; Ubuntu, Windows and other checks passed.
Updated-branch Ubuntu, macOS, Windows, Documentation, Governance and
strict-worker checks passed in [CI run 35718111229](https://github.com/smormah/vsift/actions/runs/35718111229);
[dependency/security run 35718111241](https://github.com/smormah/vsift/actions/runs/35718111241)
and [Rust analysis run 35718111230](https://github.com/smormah/vsift/actions/runs/35718111230)
passed.
Raw model staging, compatibility smoke, accepted-plan revalidation, public
installation/lifecycle, bounded GC, power-loss and D-01..D-10/P06 E2E remain
open. P06 stays planned.

2026-09-22: Protected [PR #109](https://github.com/smormah/vsift/pull/109)
merged P06 catalogue/plan implementation `b74de4d` and progress record
`9b97ff9` as `e533247` from protected predecessor `57cf7c2`. Reviewed source
now pins exactly one complete
managed candidate set for Ubuntu 24.04 x86-64: BtbN month-end FFmpeg/FFprobe,
whisper.cpp v1.9.2 CLI and the pinned multilingual `base` model. The catalogue
checks its own direct-origin routes, hashes, archive bounds, selected files,
link headers and regular alias inventory; a malformed entry fails closed.
`setup plan` is read-only and shows only needed actions, exact files and
publisher/licence/source/trust-limit disclosures. Its deterministic digest
binds target, catalogue revision, present probe and configured-model/selection
observations, and all actions; unsupported, expired or invalid entries yield no
digest. New plans stop on 2028-08-01. Full local fmt, strict Clippy, tests,
warning-denied rustdoc, governance and dependency policy passed (existing
duplicate-version warnings only). Protected Ubuntu, macOS, Windows,
Documentation, Governance and strict-worker checks passed in
[CI run 35665923881](https://github.com/smormah/vsift/actions/runs/35665923881);
[dependency/security run 35665923929](https://github.com/smormah/vsift/actions/runs/35665923929)
and [Rust analysis run 35665923796](https://github.com/smormah/vsift/actions/runs/35665923796)
passed. `setup install` still returns
`COMMAND_NOT_IMPLEMENTED`; production compatibility smoke, accepted-plan
revalidation, public lifecycle, bounded GC, power-loss and D-01..D-10/P06 E2E
remain open. P06 stays planned and issue #66 stays open.

2026-09-21: Protected [PR #105](https://github.com/smormah/vsift/pull/105)
merged P06 managed-version lifecycle implementation `ec6538b` and memory record
`fc3b51a` as `39211bd` from protected main `b32c82b`. It adds a guarded
rollback-selection primitive and removal fencing without exposing a public
command. Published-runtime capabilities hold shared private per-version OS locks.
An unselected version can be removed only after obtaining the exclusive lock; the
selected version returns `Selected` and any live holder returns `InUse`.
A tombstone blocks new openers, and deletion is restricted to the revalidated
manifest inventory and known metadata. Injected failures after payload, use-lock
and manifest deletion resume safely, as do tombstone-only and already-completed
retries. Unsafe substitutions fail closed. A native child-process regression
proves exclusion across processes and lock release after abrupt holder exit.
Full local formatting, strict workspace Clippy, tests, rustdoc and governance
passed; cargo-deny passed with the existing duplicate warnings. Protected CI
`35633903728`, dependency/security `35633903800` and Rust analysis `35633903719`
passed on the first attempt. Catalogue/plan/compatibility authority, power-loss
qualification, bounded GC and public install/rollback/uninstall orchestration
remain open. P06 stays planned and issue #66 stays open.

2026-09-16: Protected [PR #101](https://github.com/smormah/vsift/pull/101)
merged P06 immutable version publication implementation `285d3a1` and memory
record `4a16921` as `6eee0e0` from protected main `a492863`. The
provider-neutral infrastructure API requires the same managed root's
installation guard, records an exact private manifest for canonical
component/version identity, moves the prepared runtime into version storage,
fully reopens and rehashes it, and only then atomically replaces a current
pointer bound to the manifest SHA-256. Older versions remain readable, including
through a capability held across an update. Fault tests prove prior selection
before pointer replacement and idempotent retry both before and after commit;
identity conflicts, cross-root guards, linked metadata and invalid keys fail
closed. Windows candidate handles are explicitly closed before directory rename
and the destination is fully revalidated after reopen. Selected-runtime lookup
creates no absent storage. All local gates passed. Protected CI `35079430148`,
dependency/security `35079430100` and Rust analysis `35079430156` passed on the
first attempt. This is process-interruption ordering only; power-loss durability,
catalogue/plan/compatibility authority, rollback, uninstall, active-removal
fencing, bounded GC and public managed installation remain open. P06 stays
planned and issue #66 stays open.

2026-09-16: Protected [PR #99](https://github.com/smormah/vsift/pull/99)
merged P06 managed installation-guard implementation `3ff602f`, memory update
`0514737` and Unix import fix `2b5af29` as `e5b6c28` from protected main
`66cbc45`. The infrastructure-only root-wide OS lock is non-blocking for
headless use, classifies held-lock contention as `Busy`, and rejects linked or
non-private Unix lock files. Focused exclusion/release, hard-link/source-
preservation and Unix-mode tests plus all local gates pass. An initial local
full run's three unrelated Windows process-supervisor timing failures passed on
unchanged focused and complete reruns. Initial protected CI then found the
missing Unix permissions trait import; the corrected commit passed protected
CI `35074995313`, security `35074995207` and Rust analysis `35074995323` on all
targets. No version publication, accepted plan or public install is introduced,
so P06 remains planned. Docs-only record PR #100 first reproduced the existing
model-registration `Busy` on Ubuntu (job `104727292434`); the unchanged rerun
passed (job `104728936272`). [Issue #66](https://github.com/smormah/vsift/issues/66)
remains open.

2026-09-16: Protected [PR #97](https://github.com/smormah/vsift/pull/97)
merged the P06 hosted production-layout checkpoint `f9d2104` as `a405e33` from
protected main `8604370`. The opt-in Ubuntu 24.04 workflow now passes a fresh
bounded and SHA-256-verified whisper.cpp archive through the production Rust
owned-runtime test before its separate Python model-backed candidate experiment.
The Rust checkpoint performs exact import, selected payload assembly, regular
alias copies, executable-mode preparation, recheck and explicit discard without
running the binary; the archive is removed even if that check fails. All local
and protected gates passed in CI `35052887409`, security `35052887516` and Rust
analysis `35052887463`. Protected hosted run `35053264628` passed the production
layout in 3.48 seconds, then independently passed F01 inference in 23.03 seconds;
largest child peak RSS across the smoke was 291,688 KiB. No catalogue, accepted
plan, activation or public install is introduced; P06 stays planned.

2026-09-16: Protected [PR #95](https://github.com/smormah/vsift/pull/95)
merged P06 owned runtime layout implementation `15cec51` and memory update
`1023d24` as `218671d` from protected main `97f13d8`. It creates a fresh
private `runtime.pending` child from an exact selected payload, validates the
trusted byte/name/alias/executable review before mutation, copies only verified
regular files and aliases, sets Unix owner-only executable mode, and rechecks
held directory identity, exact file set, type/link count, modes and SHA-256.
Invalid reviews have no runtime effects; suspicious entries block open and
cleanup. A fresh pinned Ubuntu whisper.cpp archive passed six regular SONAME
alias copies and all twelve runtime-file rechecks without candidate execution.
Local fmt, strict Clippy, full workspace tests, warning-denied rustdoc,
governance and dependency policy passed. Protected Ubuntu and macOS Quality
first reproduced existing P05 lock `Busy` failures (jobs `104652588417` and
`104652588434`); unchanged reruns passed (jobs `104653707688` and
`104653707499`), together with Windows, Documentation, Governance and
strict-worker in [CI run 35051470387](https://github.com/smormah/vsift/actions/runs/35051470387).
[Dependency/security run 35051470403](https://github.com/smormah/vsift/actions/runs/35051470403)
and [Rust analysis run 35051470400](https://github.com/smormah/vsift/actions/runs/35051470400)
passed. No catalogue was accepted and no provider was smoke-tested or
activated. P06 remains planned; plan acceptance, source/notice closure,
compatibility, version transactions, lifecycle and D-01..D-10/E2E are pending.
[Issue #66](https://github.com/smormah/vsift/issues/66) remains open because
the intermittent lock holder is still unknown.

2026-09-15: Protected [PR #93](https://github.com/smormah/vsift/pull/93)
merged P06 configuration lock classification `2830a70` as `90bd4b8` from
protected main `664d43a`. It now separates true held-lock `BUSY` from OS lock I/O in
private BYO configuration writes. A cross-platform held-lock regression
preserves the prior record and confirms success after release; narrow context
on the sequential model-registration test will identify which write fails.
Local fmt, strict Clippy, full workspace tests, warning-denied rustdoc and
governance passed. Protected Ubuntu, macOS, Windows Quality, Documentation,
Governance and strict-worker passed in
[CI run 35018869099](https://github.com/smormah/vsift/actions/runs/35018869099),
dependency/security in [run 35018869101](https://github.com/smormah/vsift/actions/runs/35018869101)
and Rust analysis in [run 35018869095](https://github.com/smormah/vsift/actions/runs/35018869095).
Docs-only
PR #92 first Ubuntu Quality attempt reproduced the existing intermittent
`Busy` on model registration (run `35017216692`, job `104543565997`), and
unchanged rerun passed (job `104545464013`) before the protected merge
`664d43a`. [Issue #66](https://github.com/smormah/vsift/issues/66) remains
open because the exact lock holder is unproven. P06 stays planned; reviewed
layout, catalogue, compatibility smoke, installer and D/E2E are pending.

2026-09-15: Protected [PR #91](https://github.com/smormah/vsift/pull/91)
merged P06 unqualified read-only plan implementation `08f1a96` and headless
regressions `8a40634` as `1d79a8d` from protected main `ade30f6`. It now
composes the existing configured/PATH probe into
`setup plan --profile`. The v1 result gives per-tool manual BYO steps for
unavailable executables and labels responding tools probe-only. It has no
install actions or acceptance digest; `setup install` remains reserved and
the model/provider compatibility is unchecked. Frozen schema/example and
missing/off-PATH CLI tests and headless JSONL/corrupt-config tests, plus local
fmt, strict Clippy, full workspace tests, warning-denied rustdoc, governance
and dependency policy passed. Protected Ubuntu, macOS, Windows Quality,
Documentation, Governance and strict-worker passed in
[CI run 34989855963](https://github.com/smormah/vsift/actions/runs/34989855963),
dependency/security in [run 34989855845](https://github.com/smormah/vsift/actions/runs/34989855845)
and Rust analysis in [run 34989855862](https://github.com/smormah/vsift/actions/runs/34989855862).
Qualified source catalogue, plan acceptance,
compatibility smoke, alias/executable transition, atomic activation, lifecycle
and D-01..D-10/P06 E2E are still required. P06 stays planned.

2026-09-15: Protected [PR #89](https://github.com/smormah/vsift/pull/89)
merged P06 owned selected-file assembly `e0d2ada` with private initialization
correction `60eee40` as `c8d95dd` from protected main `d377430`. The private
marked artifact stage creates an empty private payload directory and passes a rechecked artifact
to bounded raw tar, gzip/tar or XZ/tar selected-file staging. Open rejects
changed, linked, extra or directory-substituted files; discard removes only
reviewed files and leaves the artifact for an explicit later discard. Fresh
pinned Ubuntu whisper.cpp archive bytes passed owned import, gzip/tar assembly,
`whisper-cli` recheck and disposal without executing a binary. Focused tests,
full workspace fmt/strict Clippy/tests, warning-denied rustdoc, governance
and dependency policy passed locally. Protected Ubuntu, macOS, Windows Quality,
Documentation, Governance and strict-worker checks passed in
[CI run 34985416473](https://github.com/smormah/vsift/actions/runs/34985416473),
dependency/security in [run 34985416294](https://github.com/smormah/vsift/actions/runs/34985416294),
and Rust analysis in [run 34985416283](https://github.com/smormah/vsift/actions/runs/34985416283).
Catalogue, plan acceptance, alias and executable preparation, smoke,
activation and full D/P06 E2E are still open; P06 stays planned.

2026-09-15: Protected [PR #87](https://github.com/smormah/vsift/pull/87)
merged P06 direct publisher transfer `c058756` as `0d36160` from protected
main `3a56ac2`. Infrastructure validates immutable
reviewed GitHub-release and Hugging Face model routes, admits only their
observed HTTPS publisher CDNs, uses system TLS with typed redacted failures,
and streams exact size/SHA-256 to the owned unactivated stage. No resume:
cancelled, failed or timed-out attempts discard owned staging and start from
zero next time. The pinned Ubuntu whisper.cpp asset passed opt-in direct
download/recheck without execution. Focused route, redirect, cancellation and
stream failure tests plus fmt, strict Clippy, full workspace tests,
warning-denied rustdoc, governance and dependency policy passed locally.
Protected Ubuntu, macOS and Windows Quality, Documentation, Governance and
strict-worker checks passed in [CI run 34981240230](https://github.com/smormah/vsift/actions/runs/34981240230),
dependency/security in [run 34981240514](https://github.com/smormah/vsift/actions/runs/34981240514),
and Rust analysis in [run 34981240137](https://github.com/smormah/vsift/actions/runs/34981240137).
Catalogue review, plan acceptance, assembly, compatibility smoke, activation
and full D/P06 E2E remain required; P06 stays planned.

2026-09-15: Protected [PR #85](https://github.com/smormah/vsift/pull/85)
merged P06 owned unactivated artifact staging `f6c3509` as `3ad5bf6` from
`eb56928`. An infrastructure-only managed-data store opens or creates a
private per-user root with an exact owner marker, rejects existing unmarked
storage, and imports exact reviewed bytes into a private marked stage. Each
open rechecks staged bytes; failed import discards only known owned files,
while unexpected entries block deletion. Full local gates and protected
Ubuntu, macOS, Windows Quality, Documentation, Governance, dependency and
CodeQL/Rust checks passed in [CI run 34953469291](https://github.com/smormah/vsift/actions/runs/34953469291),
[Security run 34953469302](https://github.com/smormah/vsift/actions/runs/34953469302)
and [CodeQL run 34953469319](https://github.com/smormah/vsift/actions/runs/34953469319).
No public managed command is enabled. The Ubuntu candidate's archive notice
does not enumerate all enabled compiled components, so catalogue acceptance
remains open. Direct HTTPS, plan authorization, assembly, smoke, version
transactions and complete D/P06 E2E qualification remain required. P06 stays
planned.

2026-09-15: Protected [PR #83](https://github.com/smormah/vsift/pull/83)
merged P06 contained selected-file staging `57b8113` as `b5d8769` from
protected main `e75d560`. The bounded archive adapters accept an empty private
directory capability and write only hash-verified reviewed regular files under
portable flat basenames. They do not materialize archive links, directories or
modes, and roll back created files on later archive/compression failure.
Focused raw-tar, gzip and XZ tests pass. Fresh pinned FFmpeg and whisper.cpp
archives passed contained staging without binary execution. Full workspace
tests, fmt, strict Clippy, warning-denied rustdoc, governance and
`cargo deny check` passed. Protected Ubuntu, macOS and Windows Quality,
dependency, docs, governance, strict-worker and CodeQL/Rust checks all passed.
Managed-root ownership, direct HTTPS, catalogue/plan acceptance, executable
permission transition, smoke and activation are absent. P06 stays planned;
P07/P08 remain ineligible.


2026-09-14: Protected [PR #81](https://github.com/smormah/vsift/pull/81)
merged P06 selected-file integrity `5454dcb` as `d9861a6`.
The existing bounded archive readers now hash exact reviewed regular-file bytes
without writing them and reject missing or mismatched selected files. Focused
and workspace tests pass. Fresh pinned Ubuntu opt-in archive tests passed
selected-file digest checks without binary execution. This is read-only
verification, not extraction or managed setup. Direct HTTPS transfer,
contained staging, catalogue/plan acceptance, smoke and atomic activation
are still absent. P06 stays planned; P07/P08 remain ineligible.
Local fmt, strict Clippy, workspace tests, rustdoc, governance and
`cargo deny check` passed. Protected Ubuntu, macOS and Windows Quality,
dependency, docs, governance, strict-worker and CodeQL/Rust checks passed
after an unchanged Ubuntu rerun. The first Ubuntu job returned the existing
intermittent `Busy` symptom in the P06 user-configuration unit test
(run `34897284569`, job `104154428124`); [issue #66](https://github.com/smormah/vsift/issues/66)
retains the lock investigation.

2026-09-14: Protected [PR #79](https://github.com/smormah/vsift/pull/79)
merged P06 read-only XZ/tar inventory `2a960ba` as `ad2a844`. It caps
compressed input and decoder block memory at 128 MiB each, rejects trailing
or concatenated streams and reuses the bounded tar policy. The exact pinned FFmpeg archive
passed opt-in size/SHA-256 and 73-entry inspection without execution. Local
fmt, strict Clippy, workspace tests, rustdoc, governance and `cargo deny check`
passed. Protected Ubuntu, macOS and Windows Quality, dependency, docs,
governance, strict-worker and CodeQL/Rust checks passed after an unchanged
Ubuntu rerun. The first Ubuntu job hit the open P05 `Busy` symptom in
[issue #66](https://github.com/smormah/vsift/issues/66). Total process/time bounds,
selected-file extraction, staging, managed plan and activation remain absent;
P06 is planned and P07/P08 ineligible.

2026-09-14: Protected [PR #77](https://github.com/smormah/vsift/pull/77)
merged P06 gzip tar inventory adapter `f553d98` and pinned archive regression
`c52df98` as `caf46ad`. The adapter is read-only and bounded
on compressed input and expanded tar stream. Pinned `flate2` 1.1.10 with a
pure-Rust backend passed dependency review and `cargo deny check`. Focused
decoder/adversarial tests, local fmt, strict Clippy, workspace tests, rustdoc
and governance passed. The exact publisher whisper.cpp archive passed opt-in
read-only inventory after pinned size/SHA-256 verification (`c52df98`), without
execution. Protected Ubuntu, macOS and Windows Quality, dependency, docs,
governance, strict-worker and CodeQL/Rust checks passed. XZ decoding,
selected-file extraction, staging, managed plan and activation remain absent;
P06 is planned and P07/P08 ineligible.

2026-09-14: Protected [PR #75](https://github.com/smormah/vsift/pull/75)
merged P06 raw-tar reader `01ba396` as `e0da4b8`. It inspects bounded,
sequential archive metadata without writing files. Focused
truncation, trailing-data, stream-budget, special-entry and PAX tests passed
(`c6fad01`). `tar` 0.4.46 dependency review, local fmt, strict Clippy,
workspace tests, rustdoc, governance and `cargo deny check` passed. Protected
Ubuntu, macOS and Windows Quality, dependency, docs, governance,
strict-worker and CodeQL/Rust checks passed after an unchanged Ubuntu rerun.
The first Ubuntu run hit the existing P05 `Busy` symptom in two tests;
[issue #66](https://github.com/smormah/vsift/issues/66) retains investigation.
No publisher decompression, selected-file extraction, staging, managed plan or
activation exists. P06 is
planned and P07/P08 ineligible.

2026-09-14: Protected [PR #72](https://github.com/smormah/vsift/pull/72)
merged Ubuntu candidate provenance as `8e299e4`. Protected
[PR #73](https://github.com/smormah/vsift/pull/73) merged archive inventory
policy `7e73b95` as `12749a9`; local fmt, strict Clippy, workspace tests,
governance and all required hosted checks passed. The policy validates bounded
entry metadata and exact reviewed link headers. It does not parse an archive,
write staging files or install anything. D-04, the catalogue and full P06
gates remain open; P07/P08 remain ineligible.

2026-09-14: Ubuntu P06 candidate provenance now links BtbN release build-scripts
commit `8267213e`, upstream FFmpeg commit `e47273f4` and whisper.cpp v1.9.2
tag commit `306c88f4`. A 2028-08-01 new-plan cutoff is proposed before the
month-end asset's two-year retention ends. These links support review but do
not prove binary signature or all compiled-source correspondence. Catalogue,
installer and D-01..D-10/P06 E2E remain open; P06 is planned.

2026-09-14: Protected [PR #68](https://github.com/smormah/vsift/pull/68)
merged as `78e005d` after all required checks passed; its first macOS Quality
attempt hit the existing P03 `Busy` flake tracked in
[issue #66](https://github.com/smormah/vsift/issues/66), then passed on an
unchanged-commit rerun. Opt-in
[hosted Ubuntu run 34851335044](https://github.com/smormah/vsift/actions/runs/34851335044)
passed pinned candidate hashes and F01 model-backed tone inference, captured
the FFmpeg build configuration/LGPL statement, and observed 23.05 seconds and
290,820 KiB largest child peak RSS across the smoke. This is not controlled
speech/resource or installer qualification. Catalogue, notices/source,
D-01..D-10 and P06 E2E remain open; P06 stays planned and P07/P08 ineligible.

2026-09-14: Protected [PR #67](https://github.com/smormah/vsift/pull/67)
merged the verified-stream primitive as `cda41cf` after all required checks
passed. P06 still has no managed URL/installer. The opt-in Ubuntu candidate
runner now prepares build configuration/licence and F01 resource observations;
these are unmeasured until the hosted workflow runs. Local archive and safe-
diagnostic tests pass. P06 remains planned and P07/P08 remain ineligible.

2026-09-14: P06 verified-stream primitive is underway from protected main
`a554586`: exact bounded bytes and SHA-256 are checked before any future
activation. Local fmt, strict Clippy and workspace tests pass. HTTPS source,
catalogue, extraction, activation, D-01..D-10 and P06 E2E remain open.
Protected PR #65 merged the source notice review as `a554586`; its first
Ubuntu Quality attempt failed with `Busy` in the existing shared-read-lock
test, then passed unchanged on rerun. [Issue #66](https://github.com/smormah/vsift/issues/66)
tracks the unexplained test failure. The ledger remains planned; P07/P08
remain ineligible.

2026-09-14: Protected [PR #64](https://github.com/smormah/vsift/pull/64)
merged model file registration as `15ed193`, with all supported-OS quality and
security/governance checks passed. A follow-up read-only Ubuntu candidate
review reconfirmed the exact FFmpeg archive hash and its LGPL v3 notice, plus
MIT publisher notices for whisper.cpp and the converted model. The candidate
is still not an accepted catalogue: full compiled-component source references,
plan-facing disclosure, production installer and D/E2E evidence remain open.
P06 stays planned; P07/P08 remain ineligible.

2026-09-14: Protected [PR #63](https://github.com/smormah/vsift/pull/63)
merged persistent BYO executable registration as `55c8a6d`; all Ubuntu,
macOS, Windows, documentation, governance, strict-worker, dependency and
security checks passed. The subsequent model selection increment is recorded
above. `setup check` still reports `local_asr_model: not_checked`. Model compatibility,
managed plans/install, D-01..D-10 and P06 E2E remain pending; the ledger stays
planned and P07/P08 remain ineligible.

2026-09-13: P06 persistent BYO executable registration was developed from
protected main `4c8dc36`. This increment selects private per-user canonical
paths and leaves model compatibility, managed installation and D/E2E gates open.
The ledger remains planned; P07/P08 remain ineligible.
Local fmt, strict Clippy, workspace tests, warning-denied rustdoc, governance,
cargo-deny and diff check passed; protected merge evidence is recorded above.

2026-09-13: Read-only P06 Ubuntu x64 candidate inventory from direct publisher
downloads is in `docs/planning/p06-ubuntu-artifact-candidate.md`. The verified
whisper.cpp archive contains eight symlinks; safe materialization and hosted
model-backed compatibility remain unproven. No managed catalogue or installer
has been accepted. P06 remains planned.
The opt-in hosted Ubuntu candidate runner is prepared with selected-file hashes,
non-link alias materialization and four passing local archive guardrails; its
hosted result remains pending.
The [hosted run 34782768288](https://github.com/smormah/vsift/actions/runs/34782768288)
subsequently passed on Ubuntu 24.04.5 x64 after protected PR #61 merged as
`a9ecd1b`. This is bounded tone-audio candidate evidence, not an accepted
catalogue, speech-accuracy finding or P06 E2E completion.

2026-09-13: P06 direct-origin clarification: managed downloads are intended to
come from the publisher's HTTPS release origin on the user's machine after
separate acceptance of a plan selecting reviewed pinned bytes. VSift does not
mirror or proxy provider binaries. Upgrades require a new catalogue review.
The exact build's notices/source, expiry, compatibility and installer evidence
are still pending; P06 remains planned and P07/P08 remain ineligible.

2026-09-13: The P05 issue #51 lock-lifetime fix merged through protected PR #58
as `9f86ff1`; supported OS, governance, dependency and security checks passed.
The [100-run Ubuntu stress](https://github.com/smormah/vsift/actions/runs/34756593985)
passed. The exact original CI lock holder remains unproven, but the root
initialization lock now has an explicit release and regressions assert immediate
availability. P06 retained-source PR #57 passed protected checks and merged as
`b7e88af`. Its [Windows Server 2025 smoke](https://github.com/smormah/vsift/actions/runs/34756838957)
passed pinned download integrity, F01 extraction and model-backed inference.
P06 remains planned: this test contains no speech, does not qualify Windows 11
or legal notices and does not implement the managed installer or D/E2E gates.
P07/P08 remain ineligible.

2026-09-13: The opt-in Windows candidate-smoke harness (`a0fe266`) merged
through protected PR #55 as `1c805c11519ec43ad91cc6beb18d8d39c40ee495`.
Hosted run 34737109736 passed exact archive/model hashes, selected ZIP file
integrity, FFmpeg/FFprobe F01 operations and whisper.cpp model-backed inference
on Windows Server 2025 x64. The harness is not called by the CLI and F01 is
tone-only. Real-speech accuracy, Windows 11, resource/containment,
legal/notice, managed installer and P06 E2E gates remain open. P06 stays
planned and P07/P08 remain ineligible. This original BtbN daily FFmpeg asset
had a 14-build retention window and was superseded for candidate testing by
the 2026-08-31 month-end asset; neither is an accepted installer source.

2026-09-13: A BtbN 2026-08-31 month-end Windows x64 LGPL archive was
downloaded for read-only inventory review. Its 147,007,942-byte SHA-256
`2484854ad6988d34560f4e6ea7a6ecb9dde0af7c229d2591815d056b04ec4f56`
matches the release asset API; selected FFmpeg/FFprobe/licence file hashes
are recorded in the P06 candidate document. PR #57 subsequently repointed the
opt-in runner to this retained candidate and passed hosted tone-audio smoke.
No binary was executed on the maintainer desktop, no managed installer is
available and P06 remains planned.

2026-09-13: The first P06 implementation increment merged via protected PR #53
as `0f0156bd62ac6777fcb1f412968df6eda977158e`; all required platform,
governance, documentation, dependency and security checks passed. A candidate
Windows x64 source set has exact observed FFmpeg/whisper archive hashes and a
pinned `base` model LFS pointer, but binary licence/notice and controlled smoke
tests remain before any accepted managed-install catalogue (`2256582`). No dependency was
installed. P06 stays planned in the ledger, so P07/P08 remain ineligible.

2026-09-13 P06 kickoff: `codex/p06-detection-guidance` starts from protected
main `3c3c93f236b8c34491295fb05a52a2b162e8b1ee` (implementation commit
`3a1da55`). It adds read-only
absolute executable selection for this check, structured manual/BYO remediation
and an explicit `executable_probe_only`/model-not-checked warning in v1 JSON.
These are partial P06 changes, not a claim of compatibility, persistent config,
managed installation or a complete local-ASR path. The source trust catalogue
and D-02..D-10/E2E gates remain open; P06 ledger status stays planned. Local
fmt, strict Clippy, workspace tests, warning-denied rustdoc, governance, diff
check and cargo-deny pass. Protected PR evidence and merge reference pending.

2026-09-13 R0 setup clarification: ADR 0014 requires detection of existing
tools, explicit reviewed plan/install for eligible missing dependencies, and
typed manual/BYO guidance on unavailable, denied, offline or failed installs.
The agent may explain but not silently install or elevate. Script-installed
off-PATH tools are a first-class selection case. PR #49's BYO-only rescope
closed without merge; the PR #48 source assessment was carried forward and
closed as superseded by PR #50. Its commits `b1c271d` and `6ab6ef3` merged
through protected checks as `52127e0e68171395fd8ec97f5604edeea5045b65`.
The immutable per-target catalogue gate remains unresolved;
issue #9 reflects the corrected journey. P06 remains planned and no new setup
functionality is implemented.

Protected Ubuntu Quality on docs-only PR #50 failed the existing P05
`registration_scan_and_abandoned_cleanup_respect_the_live_lock` test: the first
bucket scan returned `Busy` after registration. Issue #51 tracks reproduction
and lock-lifetime diagnosis; the test has not been changed. Windows/macOS
Quality and the other protected checks passed on the latest commit, including
Ubuntu. That green run does not establish root cause for the earlier failure.

P06 source review (2026-09-12) found no reviewed immutable per-target provider/model
catalog. Upstream FFmpeg supplies source only; whisper.cpp v1.9.4 has no release
assets and v1.9.2 has no macOS CLI archive. See
`docs/planning/p06-provisioning-source-review.md`. P06 remains planned, and no
managed-install or D-suite evidence is claimed. Issue #9 remains open.

No implementation packet is active. P05 merged through protected PR #46 as
`c3f9313f8df0871d17ab80ad5bb142be6421b36f`, its evidence follow-up merged,
and issue #8 closed. P06 is next eligible but remains planned at this source gate.
Strict durable requests still fail before mutation.

At the earlier P04 checkpoint, protected PR #44 merged as
`4fc859b3344bd47c254dd9da9cac72f5ad3d61d5`; it made P05 eligible.

P03's implementation completed through protected PR #42 as
`3eef9b7ac3bcfe092d82137ccf2aa9aa084aca4f`; this evidence-only follow-up records
the ledger and handoff state, and issue #6 is closed. The filesystem `SessionStore`
remains internal and explicit durable requests still fail before mutation. No
storage/session command is exposed.

The R1 managed industrial capability boundary is accepted in ADR 0011 and PR #31
(`0cfdb407f805282995f326ca93c99bc7170eda04`). It reserves P15-P20 for
contracts/corpus, enrichment, source-grounded composition, managed catalogue,
industrial worker plane and integrated qualification, with milestone 2 and issues
#25-#30. This is roadmap work only: P15+ implementation remains ineligible until P14
and P15's decision/fixture/ledger gate.
R0 issues #15 and #17 now include the named two-client, supplied-transcript and
local-ASR end-to-end release evidence required by A-08/A-09.

FS-01: OS/storage crash qualification is absent. The default NTFS read-only
directory handle fails synchronization (OS error 5); an explicit safe writable
directory handle succeeds. That resolves the API-access issue, not durability
qualification. Strict durable enablement remains gated, while ephemeral adapter work
may resume. The development-only native spike records primitive containment, lock and
process-kill observations, not P03 completion. Accepted ADR 0010 assigns strict
Ubuntu/ext4 durability proof and
enablement to P10/P11/P14; durable requests must fail before mutation until then.
No storage operation is exposed yet.

The incremental R0 E2E spine is tracked in `docs/planning/e2e-test-spine.md` and issue
#40. P04 bootstraps the executable real-media runner, P07 will attach both
transcript paths, P09 will complete the mechanical video-to-evidence journey, and P12
will add named Codex/Claude trials. It is opt-in at major checkpoints and mandatory at
release; the planning framework is not implementation evidence.

Spike commit: `5f15c7730607570663d739b7a4dd70a03947e500`, PR #24. Local Windows
checks pass (92 tests, fmt, strict clippy, governance and warning-denied rustdoc).
Post-test security review found cargo-deny's dev-only licence/duplicate checks
were off by default; they are now explicit, including a recorded MIT-0 review for
the already-locked P01 test dependency. No advisory or per-package exception added.
Follow-up `a262fa9bbabb4bebc6ebde581204c4dbe0a8186d` passed ten storage experiments
on each of NTFS, APFS and ext4 (CI run 34585520298); dependency policy/review passed
with development auditing enabled (run 34585520299). Safe writable/readable handle
probes resolve default-handle synchronization failures. Exact OS versions and the
remaining OS/storage crash gate are recorded. That gate is P10/P11/P14 work and does
not invalidate P03's completed ephemeral profile.

## Complete

### 2026-09-22 — P06 reviewed catalogue and plan increment

Implementation `b74de4d` accepts an exact Ubuntu 24.04 x86-64 source set for
read-only `setup plan`, with deterministic digest, expiry, complete installed
inventory and source/notice/trust-limit disclosure. Full local gates passed;
protected merge evidence remains to be recorded. This does not complete P06 or
make `setup install` available.

### 2026-09-13 — P05 lock follow-up and P06 retained-source review increments

Protected PR #58 merged the explicit root initialization lock release and
regressions as `9f86ff1`. Three-OS Quality and the
[100-run Ubuntu stress](https://github.com/smormah/vsift/actions/runs/34756593985)
passed. Protected PR #57 merged the retained month-end FFmpeg candidate
review as `b7e88af`; its
[Windows Server 2025 smoke](https://github.com/smormah/vsift/actions/runs/34756838957)
passed pinned hash checks and F01 FFprobe/FFmpeg/whisper.cpp compatibility.
Neither tone-audio inference nor source retention qualifies the installer or
completes P06. The exact historical lock holder remains unknown.

### 2026-09-12 — P05 session lifecycle and retained bundles

P05 opened disposable source-bound sessions, committed renew/close/artifacts
through P03 generations, added bounded root-local registration for
list/cleanup, and validated explicit evidence-only or source-inclusive
retained bundles. Implementation commit
`1238580a6a86a433019fb2ecabfeec3eef56ea43` plus fixture correction
`8e7232c86d1d324b8ccd7e8e5ba271d813544ee5` merged as
`c3f9313f8df0871d17ab80ad5bb142be6421b36f`. Local cross-process,
source-preservation, full workspace, warning-denied docs, governance and
dependency checks passed. The native Windows FFmpeg/FFprobe 9.0 P05 checkpoint
passed as development evidence. Protected Quality passed on Ubuntu, Windows and
macOS; Governance, Documentation, strict-worker, dependency policy/review,
CodeQL and Rust analysis passed. ADR 0013 retains process-crash-consistent
publication and the incomplete-export limit; P10/P11/P14 still own strict
OS/storage-crash qualification.

### 2026-09-12 — P04 source and media primitives

PR #44 squash-merged as `4fc859b3344bd47c254dd9da9cac72f5ad3d61d5`. P04 adds
an internal held no-follow private source snapshot with SHA-256 identity and a
restricted, bounded FFprobe/FFmpeg adapter. Typed metadata includes selected stream
indexes, orientation, codec support and normalized timeline origin. Frame and audio
results preserve observed PTS and displayed dimensions. Project-owned F01-F10/F12
media and F11 malformed/parser variants carry same-build reproducible provenance;
an independent verifier checks 11 clips, seven malformed variants, timestamps and
selected pixels. The opt-in real-media checkpoint passed seven scenarios on native
FFmpeg 9.0 Windows/NTFS and reports P05-P14 plus the complete journey as
`not_implemented`. Local fmt, strict Clippy, workspace tests, warning-denied rustdoc,
governance and cargo-deny passed. Protected Quality passed on Ubuntu, Windows and
macOS; Governance, Documentation, strict-worker, dependency policy/review, CodeQL
and Rust analysis passed. ADR 0012 records desktop decoder isolation limits and
P11/P14 qualification ownership. No public media/session command was added.

### 2026-09-11 — P03 ephemeral storage and coordination

PR #42 squash-merged as `3eef9b7ac3bcfe092d82137ccf2aa9aa084aca4f`.
P03 now provides owned private-root provisioning, Unix owner/mode and Windows DACL
validation, immutable weighted admission, shared/exclusive cross-process lifetime
holds, writer fencing, monotonic immutable generations, a bounded SHA-256-linked
manifest chain, typed integrity/version/access/capacity failures and deterministic
recovery at every manifest/pointer write, flush and rename boundary. Same-operation
retry is idempotent, stale generations conflict, and unpublished attempts are ignored.

Local evidence is 128 passing tests, with three internal child entries intentionally
ignored by the ordinary runner and launched by watchdog-bounded parents, plus fmt,
strict Clippy, warning-denied rustdoc, governance and cargo-deny. Protected Quality
passed on Ubuntu, Windows and macOS; Governance, Documentation, strict-worker,
Dependency policy/review, CodeQL and Rust analysis passed. The completed profile is
process-crash-consistent ephemeral desktop storage only. Strict Ubuntu/ext4
OS/storage crash durability remains P10/P11/P14 work and durable requests remain
fail-closed.

PR #43's first Windows evidence run exposed that two lock-contention tests treated
100 scheduler yields as a timing budget. They now retry `Busy` against a five-second
monotonic deadline with 10 ms intervals. Both passed ten consecutive targeted runs,
then the full 128-test local suite and strict Clippy passed again.

### 2026-09-11 — P03 storage contract and initializer increments

PR #35 squash-merged as `9ee3c048e1460008cd4f6c3e16dc23f78115ad0d`; PR #36
squash-merged as `65fe00c3d43405a6ed5c8bda8ae50a2896e80d65`. These focused
increments establish the durability preflight, non-wrapping generations, typed storage
failures and the first internal capability-scoped generation-zero initializer. The
adapter uses held-directory relative operations, no-follow/single-link checks, stable
OS locking, bounded strict metadata and SHA-256 verification before idempotent reuse.

Local evidence is 105 passing tests plus fmt, strict Clippy, governance, warning-denied
rustdoc and dependency policy. PR #36 passed every protected job on Windows, macOS and
Ubuntu after a Unix-only needless-return lint was found and corrected. A later Windows
run exposed time-only fixture-root naming as intermittently non-unique under parallel
tests; PR #38 added a process-local monotonic discriminator and an identical-timestamp
regression test. The corrected suite passes 106 local tests and every protected job.
At that increment, P03 remained active: root provisioning/Windows ACL qualification,
arbitrary fault recovery, later generation publication, read/write holds and admission
were not yet complete. PR #42 subsequently completed them as recorded above.

### 2026-09-11 — P03 storage qualification profile

PR #33 squash-merged as `c2b3829d77279a32b3487ab1170f820f1d68eeb5`.
ADR 0010 permits P03 to implement process-crash-consistent ephemeral publication on
Windows/NTFS and macOS/APFS. Retention may preserve an ephemeral workspace from VSift
cleanup, but does not upgrade its durability guarantee. Explicit durable requests must
fail before mutation until P10/P11/P14 complete the owned Ubuntu 24.04/ext4 OS/storage
crash campaign and enable the strict worker profile.

This is a qualification decision, not P03 completion. All protected checks passed;
local evidence was 92 passing tests plus fmt, strict Clippy, governance and diff checks.

### 2026-09-11 — R1 industrial capability scope

PR #31 squash-merged as `0cfdb407f805282995f326ca93c99bc7170eda04`.
ADR 0011 makes R1 the explicit managed industrial expansion while keeping R0 a fully
functional local-video investigation. R0 now requires supplied-transcript and local-ASR
paths through named Codex and Claude Code clients; a scaffold-only, transcript-only or
frame-only build cannot qualify. R1 requirements R-15..R-20 and packets P15..P20 own
optional enrichment, source-grounded reconstruction, explicit managed catalogue,
industrial worker growth and integrated production qualification. SQLite remains a
candidate single-node adapter; MCP and public hostile multi-tenancy remain R2.

Milestone 2 and issues #25-#30 preserve the future work. R0 issues #15/#17 carry the
new A-08/A-09 end-to-end gate. All PR checks passed across three OS quality jobs,
governance, documentation, strict-worker boundary, dependency policy/review, Rust
analysis and CodeQL. Local evidence was 92 passing tests plus fmt, strict Clippy,
governance, warning-denied rustdoc and diff checks.

### 2026-09-11 — P02 secure process execution

PR #22 squash-merged as `4e9ef08df1e53019df7645edb0e493628a3e401a`.
P02 added canonical absolute provider resolution with explicit provenance; exact argv,
null stdin, canonical cwd and an empty-by-default environment; independent capped
stdout/stderr drains; one operation deadline and sticky caller cancellation; graceful
and forced cleanup through Windows Job Objects or Unix process groups; and an
effective-control report that separates lifecycle containment from hard isolation.
Strict mode fails before spawn without a qualified Linux worker boundary.

P-01..P-08 and C-05 are covered by 82 passing local Windows tests and protected
Quality checks on Ubuntu, Windows and macOS. The strict Ubuntu container additionally
passed read-only/no-network, process-group escape, CPU throttling, PID ceiling and
memory-limit checks. Governance, documentation, dependency policy/review, CodeQL and
Rust analysis passed.

Only setup dependency probing uses the boundary today. Managed runtime identity and
compatibility remain P06; durable coordination is P03/P10/P11; the worker host is P11.
Desktop process containment is not represented as a filesystem, network or resource
sandbox.

### 2026-09-10 — P01 public command and JSON contracts

PR #20 squash-merged as `3d7a7d2a53fd8e6d2025726d34c59e7603fc8e71`.
P01 published typed IDs, time/ranges, crops, cursors, confidence and job transitions;
the full reserved R0 parser; bounded human/JSON/JSONL presentation; strict JSON input
limits; immutable configuration precedence; and four v1 schemas with compatibility
examples. C-01..C-10 are mapped to 64 passing local tests. Governance, documentation,
dependency policy/review, CodeQL/Rust analysis and Quality on Ubuntu, Windows and
macOS all passed.

Only `setup check` executes. Other parsed commands return `COMMAND_NOT_IMPLEMENTED`;
P01 does not claim process supervision, storage, media, provisioning, or worker
behavior. SEC-01/03/16/21 residual controls remain with their later owning packets.

### 2026-09-10 — P00 decisions, corpus truth and delivery governance

PR #18 merged as `924f6c52b018cf8f018568efd51a74cb6638b00e`. DEC-01..13
are accepted through ADRs 0004-0009. P00 / issue #3 established qualification and
resource profiles, F01-F12 declarative fixture truth, R0 traceability, milestone 1
with issues #3-#17, and an executable delivery ledger. The Governance job passed and
is now a required protected-main check alongside the existing quality/security gates.
All required PR checks passed; the local workspace had 13 passing tests.

No media runtime feature was implemented by P00. Fixture media generation remains P04.

### 2026-09-09 — Planning baseline prepared

Reviewed revision: `df85f7065915145ab8b75b89756f0fa1e341a5f1`.
Planning documents: [overview](../docs/planning/README.md),
[contracts](../docs/planning/architecture-and-contracts.md),
[security](../docs/planning/security-threat-model.md),
[tests](../docs/planning/verification.md),
[packets](../docs/planning/implementation-work-packets.md),
[current gaps](../docs/planning/baseline-review.md).

The accepted baseline includes R0 single-host worker use, cross-process admission,
checkpoints,
explicit durable state and bounded batch execution. Persistent cross-video indexing
remains optional and deferred. ADR 0004 is accepted. These capabilities
are unimplemented. The planning record merged through PR #2 as `e3f8569`.

### 2026-09-09 — Implemented scaffold

`5289a2b` introduced the Rust workspace, setup diagnostic precursor, eight tests,
docs and GitHub automation. `d7a459e` enabled the support documentation path.
PR #1 / `df85f70` renamed the public command to `vsift setup check`, with JSON operation
`setup.check`; reported required checks passed at that revision.

## Implemented versus planned

`setup check`, foreground source-bound `ingest`, session lifecycle commands and
`bundle validate` are executable. P04's restricted FFprobe/FFmpeg source and
frame/audio primitives are internal; the public transcription, search,
navigation, managed installation, queue, durable job and npm release remain
future packets. Ambient PATH remains a disclosed bring-your-own fallback;
managed identity/version trust is unimplemented. See the dated B-01..B-11
dispositions; do not describe current code as production hardened.

## Next action

Review the P06 provider/model source matrix and target-specific managed-install
boundary, then implement and qualify P06 under issue #9. Keep the ledger planned
and issue open until D-01..D-10, E2E and protected checks pass. The Ubuntu/ext4
OS/storage crash campaign remains mandatory in P10/P11/P14.
