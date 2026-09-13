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
