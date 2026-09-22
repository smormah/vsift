# ADR 0014: Detect, explicitly install, or guide dependency setup in R0

- Status: Accepted
- Date: 2026-09-13
- Refines: [ADR 0007](0007-managed-runtime-provisioning.md) and
  [ADR 0008](0008-cli-and-json-contract.md)
- Applies to: DEC-07, R-03, P06, A-01 and A-08/A-09

## Context

Most first-time users will not already have every media and speech dependency.
Others have some or all of them, including tools installed by scripts outside
`PATH`. A setup experience that only detects missing tools leaves the video
investigation unusable for a new user; an installer that ignores existing tools,
silently elevates or relies on unreviewed downloads is unsafe. Coding agents also
need rich, typed outcomes so they can explain the problem without treating a
request to inspect a video as permission to install software.

## Decision

R0 provides a progressive setup journey, for each capability actually needed:

1. `setup check` detects and validates preinstalled/configured providers and a
   selected local speech model. A canonical explicit path takes precedence over
   managed versions, then filtered `PATH`. Report compatibility, provenance and
   trust limitations; a successful `--version` or `--help` alone is insufficient.
   A supplied transcript does not require Whisper or model weights.
2. `setup plan` offers a non-mutating, reviewable plan for each missing or explicitly
   selected component/target pair with a qualified artifact. It identifies exact
   source, digest/signature policy, size, licence, destination, permissions and
   changes. Do not plan downloads for already-suitable user tools by default.
3. `setup install` applies only an unchanged, explicitly accepted plan. It uses
   the reviewed per-target trust anchor, bounded transfer and extraction, private
   staging, smoke tests and atomic activation in a user-owned location. It never
   runs during npm lifecycle hooks or ordinary video processing, never invokes an
   untrusted shell script or package manager, never silently elevates, and never
   replaces or removes user-managed files. Active jobs keep their selected version.
4. If managed installation is unavailable, unsafe, offline, denied by permissions
   or fails, return a typed, bounded manual path: which capability is missing,
   what failed, what the user can install or configure externally, and how to
   rerun validation. A user may run their own reviewed script/package manager or
   provide an existing absolute path. VSift does not execute that fallback for
   them or turn suggested commands into agent authority.

`setup check`, `setup plan` and failed operations must give a headless coding
agent machine-readable capability state and remediation. The agent may explain
options and ask the user to run an explicit setup step, but may not infer install
consent from a video-investigation request or from media content. Permission
failures must not prompt indefinitely or instruct the agent to retry with elevated
rights. Required media-stage preflight occurs before that stage mutates evidence.

R0 does not promise universal one-click installation. P06 must qualify at least
one complete managed-install target path and provide the manual/BYO fallback on
every named R0 target. Each component/target pair without a reviewed immutable
artifact must report a typed managed-unavailable reason; it must not invent a URL or claim
readiness. The [P06 source review](../planning/p06-provisioning-source-review.md)
records the current open provenance and licence gate. P07/P14 must still prove
the full local-ASR and supplied-transcript video journeys on the supported
profiles, regardless of how their user prepared the dependencies.

## Consequences

- Keep the managed installer and its SEC-12..SEC-15/SEC-23 controls in R0.
  Availability is per qualified target, not an all-platform assumption.
- D-01..D-10 must cover preinstalled, partial and off-PATH setups; explicit
  authorization; transfer, integrity, extraction and activation failures; denied
  rights; offline/unsupported targets; and useful manual fallback without side
  effects. The A-01/A-08/A-09 agent tests must exercise the same authority boundary.
- P06 remains planned until the source catalogue, implementation, regression tests
  and protected checks pass. This ADR records product behavior, not completion.

## 2026-09-13 clarification: direct-origin managed downloads

The managed path selects a reviewed, compatible artifact version and downloads it
on the user's machine from the publisher's HTTPS release origin. VSift does not
host, mirror or proxy provider binaries. A redirect needed by the publisher's
release service is part of the reviewed origin policy, not permission to follow
arbitrary hosts. The plan shows the exact version, origin, digest, compressed and
installed sizes, licence, notices/source location, destination and expiry or
replacement policy before separate user acceptance.

"Latest" and "last stable" are discovery candidates, not installation trust
anchors. A newly published version cannot enter a plan until its exact bytes,
licence/notices, inventory and compatibility have been reviewed and pinned in a
new catalogue revision. The user may instead select an existing local executable
through the explicit BYO path; a user-supplied URL or checksum cannot authorize
managed installation. Licence disclosure tells the user what they are choosing;
it does not claim legal clearance or waive the source/notice review required by
ADR 0007. Direct-origin downloading does not itself make VSift a binary host.

## 2026-09-13 implementation note: persistent BYO selection

`setup configure` records only canonical absolute paths to user-managed FFmpeg,
FFprobe and whisper.cpp executables in a versioned, private per-user configuration
file. Registration does not execute the tool, validate a model, establish provider
compatibility or authorize managed installation. `setup check` resolves a per-call
path before the stored selection, then filtered `PATH`, and probes the selected
file afresh. Invalid/unknown configuration fails closed without silently falling
back to ambient tools. Model selection and managed-version precedence were still
open at this point.

## 2026-09-14 implementation note: model file selection

`setup configure-model --file <absolute-path>` records a canonical nonempty
user-managed model file in the same private per-user configuration. This is
registration only: it does not parse model contents, run inference, establish
compatibility or change `setup check`'s executable-only readiness. A later P06
preflight must revalidate the file before use. Managed-version precedence and
model-backed qualification remain open.

## 2026-09-15 implementation note: unqualified read-only plan

`setup plan --profile` now performs the configured/PATH executable probe and
returns a v1 read-only disposition. With no accepted managed catalogue it
reports `unavailable_unqualified`, no install actions and no acceptance digest,
plus manual BYO next steps for missing or unhealthy tools. A responding
executable is labelled probe-only; neither provider compatibility nor model
validity is inferred. `setup install` remains reserved. This narrows D-07/D-10
manual guidance but does not qualify a managed-install target or close P06.

## 2026-09-15 implementation note: configuration lock classification

The private BYO configuration writer now distinguishes the standard library's
`TryLockError::WouldBlock` from its I/O error variant. Only actual contention
is `Busy`; an OS lock failure is storage I/O. A held-lock regression checks
that configuration remains unchanged and succeeds after release. Narrow error
context on the sequential model-registration test will identify which write
fails if [issue #66](https://github.com/smormah/vsift/issues/66) recurs.
This classification does not identify the historical lock holder or close
that intermittent finding.

## 2026-09-21 implementation note: reviewed source and read-only plans

The first complete managed source set is now fixed in reviewed source for Ubuntu
24.04 x86-64 only. `setup plan` displays exact direct-origin artifact and layout
details, a time-bounded catalogue revision, licence and source references,
trust limits, and a deterministic digest bound to current observations.
`catalogue_accepted_install_pending` explicitly means the source is accepted
for reviewable planning while `setup install` remains unavailable. Other
targets, expired or invalid catalogue entries have typed unavailable outcomes
and manual/BYO guidance. This does not complete provider/model compatibility,
installation, D-01..D-10 or P06 E2E evidence.

## 2026-09-22 implementation note: accepted-plan revalidation gate

The reserved `setup install` path can now consume a readable bounded, strict
`setup plan --json` document. It derives the profile from that document, rebuilds
the plan from the current target, accepted catalogue, configuration, executable
probes, model observation and current time, requires the complete public plan to
match, then verifies the separately supplied acceptance digest. Unknown fields,
changed observations and digest mismatch fail before network or managed-storage
work. Even an accepted plan still returns `COMMAND_NOT_IMPLEMENTED`: production
compatibility smoke and the complete guarded transfer/publication transaction
remain prerequisites for enabling installation.

## 2026-09-22 implementation note: compatibility decision inputs

The current Ubuntu catalogue revision fixes one bounded compatibility policy:
the exact checked-in F01 fixture, expected FFmpeg build identity, 64-KiB
per-stream output capture, 60-second media deadline, 180-second inference
deadline, and 16-kHz mono extraction. Catalogue completeness and plan acceptance
both depend on these values. A changed policy therefore requires a fresh plan;
an invalid policy yields no managed actions or digest. Execution, result
recording, cleanup after smoke failure and activation remain later transaction
steps.
