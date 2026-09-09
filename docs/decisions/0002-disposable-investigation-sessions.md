# ADR 0002: Disposable investigation sessions

- Status: Accepted
- Date: 2026-09-09

## Context

Most desktop usage is expected to involve a recording inspected once and then forgotten. A hidden durable index would retain sensitive transcripts and images after the source video is deleted, without providing an adequate management surface.

## Decision

Create an application-owned ephemeral workspace for each investigation. The workspace remains available for a bounded multi-command agent session and then expires. The user may explicitly retain it as a portable evidence bundle.

A persistent cross-video catalogue is a later, opt-in adapter over retained bundles. It is not required by the first product slice.

## Consequences

- The initial session format uses readable versioned manifests and streaming records rather than a global database.
- Every response exposes lifecycle, expiry, size, and cleanup information.
- Automatic cleanup is strictly limited to positively identified VSift-owned temporary paths.
- Interruption recovery requires a bounded lease and later garbage collection.

