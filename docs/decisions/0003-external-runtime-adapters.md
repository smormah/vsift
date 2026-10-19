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

## Note — 2026-10-17: a provider's start runs off the runtime, and its deadline starts once it runs (#419)

On Windows `process-wrap` 10's Job Object wrapper creates each provider suspended, assigns it
to its job and resumes it by walking a `CreateToolhelp32Snapshot` of every thread on the
machine, one `Thread32Next` call per thread. On a Windows 11 desktop with about 17,500 threads
(release build) the snapshot took about 30 ms and the walk about 330 ms; six walks at once in
one process took 2.4 to 3.2 s each, because they wait for each other inside the process (six
in separate processes took about 0.4 s each). The start ran on the async runtime's thread and
inside the provider's deadline, so other tasks of that thread waited for it and a provider
given N seconds got N minus its start.

Two changes, neither of which needs a new crate, a new feature or `unsafe`:

- **The start runs on the runtime's blocking pool** (`tokio::task::spawn_blocking`). The
  start is not shorter, but timers, pipes and cancellations of other requests keep running. A
  start task that panics or is dropped with the runtime drops the child it made, which
  `KillOnDrop` and the kill-on-close job end; the supervisor reports `StartTaskFailed`.
- **A request's deadline is the provider's own running time**: it starts once the provider
  runs. A cancellation that arrives during the start is seen first when it returns, and the
  provider is ended through its job as before. A caller that shares one deadline among
  several providers in turn (`setup check`) gives each what is left of it when its turn
  comes, so a provider's start is outside its own time but is spent from the share of the
  providers after it, and the caller overruns its deadline by at most the start of the last
  provider it starts.
- **A cancellation during the start is answered at once** (`CancelledDuringStart`, added
  after the review): the run does not wait for a start that cannot be interrupted, and a
  task of the runtime kills and reaps the provider when the start returns. Until then the
  provider outlives its run; if the runtime shuts down first, the dropped child is ended by
  `KillOnDrop` and, on Windows, its kill-on-close job (L-170).

The Job Object, its kill-on-close limit, the suspended creation (L-129) and the console of
its own of the note above are unchanged.

**Not done: removing the walk.** It needs the provider's main thread, which neither `std` nor
`tokio` hands out, or a thread list of the one process (`PssCaptureSnapshot`, about 0.5 ms
alone and 1.5 ms six at once on the same machine), and either is a call only `unsafe` code
can make. That is a change in `process-wrap` upstream, or an accepted ADR for a narrowly
isolated crate of VSift's own; the maintainer decided on 2026-10-17 that this waits for R1, with nothing proposed upstream before then. `win-custody` 0.1.1, which
creates the child inside its job itself, was looked at and not adopted: one young release
from one maintainer, and its own process start without the `tokio` pipes and wait the
supervisor is built on. What remains is register entry L-170.

## Note — 2026-10-19: every provider runs in its own folder (#469)

**What went wrong.** Windows starts no process whose working directory is longer than 258
characters: `CreateProcessW` fails with `ERROR_DIRECTORY` (267), whatever `LongPathsEnabled`
says (measured on Windows 11 with it set, at 259 characters). The media adapter ran `FFmpeg`
and `FFprobe` in the folder of the private source copy, and whisper.cpp ran in its chunk
folder. Those folders lie deep inside the per-user folder (the media-tool check's workspace,
117 characters below `%LOCALAPPDATA%`) or the session root (56 characters below it for a
session's copy, 72 for its recognition work folder), so a `LOCALAPPDATA` of 142 characters or
more, or a session root of about 203, stopped every provider from starting. The start failure
became `MISSING_CAPABILITY`, with the advice to reinstall a working `FFmpeg`, while `setup
check`, which already ran the tools from their own folder, called them available.

**The decision.** Every provider (`FFmpeg`, `FFprobe` and whisper.cpp, in the media adapter,
the speech recognizer, `setup check`'s probe and the managed-install smoke) runs with its own
executable's folder as its working directory (`ProcessWorkingDirectory::of_executable`).
Everything a provider reads or writes is named by an absolute path, as before. Windows starts
a program only from a path shorter than `MAX_PATH` (L-094), so the folder of one it can start
is always a valid working directory, and `setup check` and the media commands now start the
tools the same way. `FFmpeg` 6 and later open an input path of any length, and whisper.cpp
reads the extended-length paths VSift gives it: the media-tool and speech checks pass in a
folder deeper than 300 characters with the gyan.dev `FFmpeg` 9.0 build on Windows 11 and, in
`P07 local ASR` run 39220885864, with the pinned BtbN `FFmpeg` and whisper.cpp v1.9.5 on
hosted Windows Server 2025 and Ubuntu 24.04. So nothing else has to be short; what an older
`FFmpeg` would do is L-193.

**SEC-02 is not weakened; the maintainer accepted the change on 2026-10-19** (the threat model's SEC-02 row records the decision). A working directory matters to library search, and on Windows the
program's own folder is searched before the working directory, so making them the same folder
adds no location; on Unix the working directory is not searched for libraries. The executable
is still named by its canonical absolute path, and the environment is still cleared. A
provider that writes into its working directory now writes into its own installation, which
the managed-install smoke's recheck refuses as before.

**The private-root check, too.** A private folder's DACL was read by the plain path
(`GetNamedSecurityInfoW`), which Windows refuses at `MAX_PATH` or more, so a per-user folder
whose own path passed 260 characters failed as unsafe storage. It is now read by the
canonical, extended-length path (`private_user_root::security_read_path`); what is checked is
unchanged (ADR 0026).

**Tests:** `crates/vsift-infrastructure/tests/long_provider_paths.rs` starts a stand-in
provider for a source copy and a chunk folder deeper than 300 characters (both failed with
`ERROR_DIRECTORY` before the change) and, opt-in, runs the media-tool and speech checks there
with the real tools; `a_root_deeper_than_max_path_is_created_and_validated` covers the DACL
read.
