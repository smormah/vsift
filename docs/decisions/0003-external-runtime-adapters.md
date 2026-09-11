# ADR 0003: External specialist runtime adapters

- Status: Accepted
- Date: 2026-09-09

## Context

FFmpeg and Whisper already provide mature media and transcription capabilities. Direct native linking would couple VSift to provider ABIs, complicate distribution, and expand the impact of provider failures.

## Decision

Invoke specialist runtimes as child processes behind application ports. Use explicit executable paths and argument arrays without a command shell. Bound execution time and captured output.

The initial adapters are FFmpeg, FFprobe, and a `whisper.cpp`-compatible CLI.

## Consequences

- Providers can be user-managed or installed into a VSift-managed runtime.
- Process failures and timeouts become typed results.
- Direct library integration remains possible if later benchmarks justify the additional coupling.

## P02 implementation note — 2026-09-10

The process adapter uses `process-wrap` 10 through its safe Tokio API. Only its Job
Object, process-group, kill-on-drop and Tokio features are enabled. The crate is
actively published under `MIT OR Apache-2.0`, declares Rust 1.87 (below VSift's 1.98
baseline), and adds Rust-only platform dependencies rather than an external native
build toolchain. `cargo deny check` accepts its advisories, licences and sources.

Windows providers are assigned to a Job Object during suspended creation. Unix
providers lead a process group. Both adapters enforce lifecycle cleanup, but a Unix
process group is not a security sandbox: filesystem, network, CPU, memory and PID
limits must be inherited from the qualified Linux worker host and reported separately.
The supervisor fails closed when strict isolation is required but unavailable.

Setup-time ambient discovery ignores relative, empty and current-directory `PATH`
entries, resolves a canonical absolute regular file, clears the child environment and
records ambient provenance. This is a bring-your-own diagnostic path, not verified
managed identity; checksums, version compatibility and managed precedence remain P06.
