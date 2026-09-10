# Architecture

VSift is a local-first Rust application that converts technical video into evidence an AI coding agent can inspect progressively.

## Dependency rule

```text
vsift-cli ------------------------+
    |                             |
    v                             v
vsift-infrastructure ------> vsift-application ------> vsift-domain
```

- `vsift-domain` owns stable business concepts and invariants. It has no infrastructure dependencies.
- `vsift-application` owns use cases and the ports required from infrastructure.
- `vsift-infrastructure` implements process, filesystem, provider, and persistence ports.
- `vsift-cli` is the composition root and translates use-case results into human and versioned JSON output.

Dependencies may point only to the right in the diagram. Cross-crate access uses each crate's public root exports; implementation modules remain private.

## Runtime boundary

FFmpeg, FFprobe, `whisper.cpp`, OCR engines, and future local models are external providers. VSift invokes approved executables with explicit argument arrays and never through a command shell.

The boundary is intended to support the following capabilities as their adapters
are implemented and qualified:

- separate provider processes with explicitly configured resource and isolation controls;
- provider replacement without domain changes;
- bring-your-own and managed-runtime resolution;
- simpler cross-platform builds and licensing analysis;
- bounded execution, cancellation, and diagnostic capture.

Direct native linking requires benchmark evidence and an accepted architecture decision record.

A child process alone does not provide filesystem/network isolation, resource caps
or a complete descendant-cancellation guarantee. The current diagnostic scaffold
does not implement those controls. See the [baseline review](planning/baseline-review.md)
and [proposed process supervisor](planning/architecture-and-contracts.md#7-multiprocessing-admission-and-cancellation).

## Data lifecycle

The first product slice uses self-contained investigation workspaces rather than a permanent database:

```text
source video
    |
ephemeral session
    |-- manifest
    |-- transcript records
    |-- visual candidates
    `-- derived artifacts
          |
          +-- automatically removed
          `-- explicitly retained as a portable bundle
```

A future persistent catalogue may index retained bundles through an application port. It is not part of the default desktop lifecycle.

Expiry makes a temporary session eligible for cleanup. With no background process,
physical cleanup runs during a later invocation or explicit host maintenance, not
necessarily at the expiration instant.

## Proposed worker execution extension

The [implementation blueprint](planning/README.md) proposes single-host multiprocessing,
bounded admission, checkpoint recovery and explicit durable workspaces in the first
functional release. A server supervisor would stage sources and manage its own queue,
tenant authorization and durable remote result storage. Default desktop investigations
remain disposable; cross-video indexing remains an explicit later capability.

This extension is recorded in [proposed ADR 0004](decisions/0004-recoverable-worker-core.md).
It is not an implemented durability or throughput guarantee.

## Public contracts

CLI commands expose two presentations of the same typed application result:

- concise human-readable output;
- an explicit JSON mode with a schema version.

JSON fields, error codes, exit codes, and evidence identifiers are public API. Changes require contract tests, documentation, and compatibility review.

## Error model

Expected outcomes are represented by exhaustive enums and typed `Result` values. Examples include missing dependencies, changed source media, expired sessions, invalid time ranges, and insufficient evidence.

Unexpected failures are translated only at the outer CLI boundary. They must retain a causal chain for diagnostics without exposing sensitive data.

## Security boundaries

The primary trust boundaries are:

1. User-supplied paths and media.
2. External executable discovery and output.
3. Downloaded binaries and model files.
4. Generated transcripts, OCR, and model metadata.
5. Automatic cleanup of temporary evidence.

Each boundary requires validation, bounded resource use, typed failure, and tests before it is considered complete.
