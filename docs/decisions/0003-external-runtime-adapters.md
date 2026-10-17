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

## Note — 2026-10-16: a console of its own for each Windows provider (#404)

A console Ctrl-C or Ctrl-Break reaches every process attached to a console, so a provider
that shared VSift's console died of the interrupt that stopped VSift (whisper.cpp with
`STATUS_CONTROL_C_EXIT`, `FFmpeg` with exit code 255, measured on Windows 11), sometimes
before VSift had recorded its own cancellation, and the run then judged the death as the
provider's own failure. The Windows adapter now also starts every provider with
`CREATE_NO_WINDOW`, a console of its own without a window, beside the Job Object: the
interrupt reaches VSift alone, and a provider stops only when the supervisor kills its job.
`CREATE_NO_WINDOW` and not `DETACHED_PROCESS`, so a provider that starts console programs of
its own gives them its hidden console rather than a visible window each. Unix providers
already lead a process group of their own, which a terminal's interrupt does not reach. The
supervisor reports the control as `interrupt_isolation`.

This enables `process-wrap`'s `creation-flags` feature (its `CreationFlags` wrapper is the
only way to combine creation flags with its Job Object) and names the `windows` crate directly
for the flag's type. Both were already in the lock, at the same version (`windows` 0.62.2)
with the same feature (`Win32_System_Threading`, which the Job Object feature enables): no new
crate, and the only new code is that small wrapper. It costs one console host (`conhost.exe`) more per provider start: a recognition starts FFmpeg and whisper.cpp for each 30-second chunk, so an hour of audio starts about 240 more, about 5 to 15 s in all at the 20 to 60 ms per start measured on one Windows 11 desktop under load; and each console takes desktop heap, so very many at once (a service's non-interactive desktop has the least) can exhaust it, when a provider fails to start with `STATUS_DLL_INIT_FAILED` (`0xC0000142`), which `VSift` reports as a provider that ended abnormally; not seen, and not tried at a worker host's highest concurrency.
What the fix of #404 leaves is register entry L-167.
