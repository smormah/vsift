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

This boundary provides:

- crash and resource isolation;
- provider replacement without domain changes;
- bring-your-own and managed-runtime resolution;
- simpler cross-platform builds and licensing analysis;
- bounded execution, cancellation, and diagnostic capture.

Direct native linking requires benchmark evidence and an accepted architecture decision record.

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

