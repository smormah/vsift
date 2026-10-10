# Security policy

VSift processes untrusted media and invokes specialist native tools. Security reports are taken seriously.

VSift runs FFmpeg, FFprobe and whisper.cpp on local media, and on Ubuntu 24.04 `setup
install` downloads reviewed tool artifacts after you accept a plan (nothing else is
downloaded). Every external provider run goes through the bounded, shell-free process
supervisor of P02, with explicit executable provenance and descendant lifecycle
containment (a Windows Job Object, a Unix process group). Containment ends the
providers when a command is interrupted or fails. It does not cover two cases of a
host killed outright: on Windows, a kill (not an interruption) in the first instants of
a provider's start leaves that provider suspended, and it stays until it is ended or
the machine restarts ([L-129](docs/planning/known-limits.md#l-129)); on Unix, a running
provider finishes its current unit ([L-055](docs/planning/known-limits.md#l-055)). This does not make an
ambient executable trusted or turn desktop process containment into a filesystem,
network, CPU, memory or PID sandbox. The
[baseline review](docs/planning/baseline-review.md) records remaining hardening gaps;
the [threat model](docs/planning/security-threat-model.md) and
[verification plan](docs/planning/verification.md) define proposed release controls.
These documents do not certify that the planned mitigations have shipped.

## Supported versions

VSift 0.2.0, published on 2026-10-09 under npm's `latest` tag, is the version that receives security fixes. The 0.1.0 pre-release (2026-10-01) and the release candidates are published but are not a supported version: they receive none.

The table says which versions receive security fixes. It was a policy until `0.2.0` was published, and it is the practice from that day. It is about security fixes, not about platforms: which machines VSift has been shown to work on is in the [support matrix](docs/planning/support-and-resource-profiles.md).

| Version | What it is | Security fixes |
| --- | --- | --- |
| The newest `0.2.x` | The R0 release line, published under npm's `latest` tag (`0.2.0` is its only version so far) | Yes. A fix is released as a new patch version of the line (`0.2.1` and so on) with a security advisory. Only the newest patch of the line is fixed. |
| `0.2.0-rc.N` | The release candidates, published under `next` and never announced (`0.2.0-rc.1` and `0.2.0-rc.2` are deprecated on npm) | No. The release replaces them. |
| `0.1.0` | The first pre-release (2026-10-01) | No. Move to the newest release. |
| The default branch | Source for building from the repository | Fixes land here first. |

Fixes are made as soon as practical and without a promised delay (see [Reporting a vulnerability](#reporting-a-vulnerability)). Software the project does not control, such as FFmpeg, whisper.cpp and your operating system, is patched by its publishers; the [known-limits register](docs/planning/known-limits.md) records what is known about the builds VSift reviews.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability.

Use the repository's **Security** tab and select **Report a vulnerability** to submit a private GitHub security advisory. Include:

- affected revision or version;
- operating system and architecture;
- reproduction steps or a minimal synthetic fixture;
- potential impact;
- any suggested mitigation.

Do not send sensitive real-world recordings as evidence. Construct the smallest rights-safe reproduction possible.

The maintainers will acknowledge a complete report as soon as practical, assess severity, coordinate remediation and disclosure, and credit reporters who wish to be named.

## Security scope

High-priority areas include:

- command or argument injection;
- unsafe archive extraction or path traversal;
- deletion outside VSift-owned temporary storage;
- unverified binary or model downloads;
- sensitive transcript, path, or credential disclosure;
- denial of service through unbounded media or provider output;
- malicious media exploiting VSift's own parsing or orchestration.

Vulnerabilities in FFmpeg, Whisper, operating-system components, or other upstream software should also be reported to the relevant upstream project. Reports are still welcome when VSift can reduce exposure or improve isolation.

## Known issue: the Windows command file of npm and pnpm

On Windows, npm and pnpm write a `vsift.cmd` file for the `vsift` command, and `cmd.exe` reads the command line of a `.cmd` file a second time: it expands `%NAME%`, can drop a double quote and acts on an unquoted `>` or `|`. A program that builds a `cmd.exe` command line (Node's `exec`, Python's `subprocess` with `shell=True`, `cmd /c`) from text it did not write, and starts `vsift` through it, can have that text run. VSift cannot change a file that npm and pnpm generate. Start `vsift` from PowerShell or Git Bash, or run the native `vsift.exe` with an argument list and no shell; the PowerShell shim, Bun and the native archive are not affected. The details, and the same advice for programs and AI agents, are in the [install guide](docs/operations/install.md#2-install-with-a-package-manager) and [known limit L-109](docs/planning/known-limits.md#l-109).
