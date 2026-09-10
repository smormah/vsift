# ADR 0007: Managed runtime provisioning

- Status: Accepted
- Date: 2026-09-10
- Resolves: DEC-07

## Context

FFmpeg and speech models are necessary for some workflows, but silent npm lifecycle
downloads, administrator requirements and mutable provider replacement would weaken
security and make running work irreproducible.

## Decision

Resolve providers in this order: explicitly configured approved path, immutable
VSift-managed version, then compatible executable discovered on PATH. Always report
the selected provenance and trust level. Managed components live in an application-
owned per-user directory and require no implicit elevation.

Installation uses a separate `setup plan` followed by `setup install` with an accepted
plan digest. Plans pin target, version, origin, integrity metadata, size, licence,
files and activation effects. Downloads are bounded, verified, staged, smoke-tested
and atomically activated. Jobs pin immutable component/model versions. Repair,
rollback and removal use the same transaction and do not delete user-managed files.

## Consequences

The npm package remains small and does not silently fetch large tools or models.
Offline and proxy-aware flows share the same verification policy. Redistribution of
each provider build/model requires separate licence and provenance review.
