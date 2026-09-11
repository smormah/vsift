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

The P02 process boundary implements:

- canonical absolute executable selection with explicit provenance;
- exact argument arrays, null stdin, a canonical working directory and a cleared,
  explicitly allowlisted child environment;
- independent concurrent stdout/stderr drains capped at 64 KiB per stream;
- one operation deadline, caller cancellation, graceful/forced escalation and final reap;
- Windows Job Object or Unix process-group descendant lifecycle containment; and
- an effective-control report that distinguishes process containment from inherited
  strict Linux worker isolation.

The boundary is intended to support these additional capabilities as later adapters
are implemented and qualified:

- managed-runtime identity verification and compatibility policy;
- simpler cross-platform builds and licensing analysis;
- per-stage provider contracts and strict worker-host integration.

Direct native linking requires benchmark evidence and an accepted architecture decision record.

A child process or Unix process group alone does not provide filesystem/network
isolation or kernel CPU/memory/PID caps. Required strict-worker isolation therefore
fails with `ISOLATION_UNAVAILABLE` unless a trusted Linux host attests inherited
container/cgroup controls. Ambient `PATH` discovery remains bring-your-own,
unverified provenance until P06 adds managed identity and compatibility policy.
See the [baseline review](planning/baseline-review.md) and
[process contract](planning/architecture-and-contracts.md#7-multiprocessing-admission-and-cancellation).

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

This extension is recorded in [accepted ADR 0004](decisions/0004-recoverable-worker-core.md).
It is not an implemented durability or throughput guarantee.

## Public contracts

CLI commands expose two presentations of the same typed application result:

- concise human-readable output;
- an explicit JSON mode with a schema version.

JSON fields, error codes, exit codes, and evidence identifiers are public API. Changes require contract tests, documentation, and compatibility review.
The published [v1 CLI contract](contracts/cli-v1.md) and
[JSON schemas](../schemas/v1/README.md) define the P01 boundary. Only `setup check`
currently reaches an application use case; the remaining parsed namespace fails with
a typed not-implemented result until its owning packet ships.

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
