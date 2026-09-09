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

