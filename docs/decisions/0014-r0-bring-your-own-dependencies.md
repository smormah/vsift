# ADR 0014: Bring-your-own dependency readiness for R0

- Status: Accepted
- Date: 2026-09-13
- Supersedes: [ADR 0007](0007-managed-runtime-provisioning.md) for R0
- Revises: DEC-07, R-03 and P06

## Context

R0 originally required VSift to download, stage and activate managed FFmpeg and
whisper.cpp builds. This made a verified cross-platform binary/model catalogue and
its supply-chain policy a prerequisite for the first useful video investigation.
The primary desktop workflow instead starts with tools the user already installed,
possibly through their own scripts and outside `PATH`. A supplied transcript also
does not need a speech engine. Bundling or downloading providers is unnecessary for
that workflow and adds a separate installation authority and attack surface.

## Decision

R0 is bring-your-own (BYO) only. VSift does not package, download, install, update,
repair, remove or execute package-manager commands for media runtimes or models.
Its npm/native distribution contains VSift itself, not FFmpeg, FFprobe, whisper.cpp
or model weights. No ordinary operation, setup command, agent skill or media-derived
content may trigger an installation or network fetch.

P06 extends `setup check` with capability-specific diagnosis and explicit
selection of user-installed executables and a local speech model. A canonical
absolute configured path takes precedence over filtered `PATH` discovery. Desktop
discovery is disclosed as user-managed, not supply-chain verified. A strict worker
requires host-approved executable/model identities on read-only mounts and fails
closed when that policy is unavailable. Check version/target and bounded real
operation compatibility; do not equate a successful `--version` or `--help` with
readiness. Detect missing, incompatible, unhealthy and untrusted capabilities
honestly, without treating the earlier `setup check` scaffold as this completed work.

Every provider-backed media operation preflights only its required capabilities
before that stage mutates session evidence:
FFmpeg/FFprobe for media processing, and a compatible local whisper.cpp executable
plus selected model only for ASR. A usable supplied transcript avoids the ASR
requirement. Headless and agent calls return a typed, bounded, actionable failure
with the missing component and manual setup guidance; they never hang for a prompt.
Remediation is data, not permission for the assistant to run an installer. A user
may install tools with their preferred scripts or package manager and rerun the
check. VSift never changes or deletes user-managed provider files.

The already-published v1 parser reserves installer-related setup verbs. They
continue to return `COMMAND_NOT_IMPLEMENTED`; they are not R0 acceptance
criteria, and no installer semantics should be inferred from the
reservation. `setup configure` is the P06 BYO-selection surface. Any future managed
installer needs its own accepted ADR, threat review, qualification catalogue and
explicit release scope. ADR 0007 remains its historical proposal, not R0 authority.

## Consequences and gates

- P06's D-01..D-10 suite now proves BYO resolution, compatibility, model presence,
  no-network/no-mutation behavior, safe path handling, TOCTOU disclosure, and
  actionable agent-facing failures. The installer-specific tests and SEC-13..SEC-15
  are deferred, not considered passed.
- B-04 closes only after tested compatibility and model-aware preflight are
  implemented, not from a PATH lookup or version string alone.
- R0 remains a full local-video-to-evidence release: missing dependencies must be
  understandable and resolvable by the user, while installed dependencies must
  pass the supplied-transcript and local-ASR end-to-end journeys.
- This decision changes planning and acceptance only. It does not itself implement
  P06 or establish support for any local installation.
