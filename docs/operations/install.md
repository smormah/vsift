# Installing VSift

Status: user guide, 2026-09-30 (P13 PR 9,
[ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)).
**Nothing is published yet.** The npm packages described here are built and qualified
by the Release workflow and will first be published as a 0.x pre-release under the npm
dist-tag `next`, with the native archives on GitHub Releases, when P13 completes and the
maintainer approves it. Until then `npm install vsift` finds only a placeholder
(`vsift@0.0.0`, no code) and VSift is built from source
([`development.md`](../development.md)). P13 PR 11 completes this guide (verifying an
archive against its attestation, and what Windows SmartScreen and macOS Gatekeeper do
with an unsigned download).

## 1. What you need

| Your machine | npm package that carries `vsift` | Notes |
| --- | --- | --- |
| Windows 11 x64 | `@vsift/win32-x64` | |
| macOS 15 on Apple silicon | `@vsift/darwin-arm64` | |
| Linux x64 with glibc 2.35 or later | `@vsift/linux-x64` | needs OpenSSL 3 (`libssl.so.3`); Ubuntu 22.04 and 24.04 have both |

To install through a package manager you need Node.js 22 or later, or Bun 1.2 or later,
to run the small launcher. Other machines (Linux on Arm, Intel Macs, Windows on Arm,
Alpine and other musl Linux) are not supported in R0.

## 2. Install with a package manager

Install globally, so the `vsift` command is on your `PATH`:

```console
npm install --global vsift@next
pnpm add --global vsift@next
bun add --global vsift@next
```

Yarn 4 has no global install; add VSift to a project and run it through Yarn:

```console
yarn add vsift@next
yarn vsift --version
```

Recent Yarn 4 releases (4.18.1 checked) hold back any version published less than a day
ago (`npmMinimalAgeGate`, default one day), so for the first day after a release Yarn
reports that the versions of `vsift` or `@vsift/…` "are quarantined". Wait a day, or
exempt VSift in the project's `.yarnrc.yml`:

```yaml
npmPreapprovedPackages:
  - vsift
  - "@vsift/*"
```

Or run it once without installing:

```console
npx vsift@next --version
pnpm dlx vsift@next --version
yarn dlx --quiet --package vsift@next vsift --version
bunx vsift@next --version
```

Then check your setup:

```console
vsift --version        # vsift 0.1.0 (<the first 12 digits of the source commit>)
vsift setup check
```

**What is installed.** The `vsift` package holds a small launcher (`bin/vsift.cjs` and `lib/launcher.cjs`)
and the agent skill (`skills/vsift/`, the folder to give Claude Code or Codex). It lists
the three platform packages as optional dependencies at its own exact version; your
package manager installs only the one whose `os` and `cpu` match your machine. No
package runs an install script, so installs work unchanged when scripts are disabled
(`--ignore-scripts`, Bun's default, pnpm's and Yarn's settings), and installing downloads
nothing else. After the install the command works offline. Media tools and the speech
model are separate: `vsift setup check` says what is missing, and on Ubuntu 24.04
`vsift setup install` can install them ([`cli-v1.md`](../contracts/cli-v1.md)).

**Where the skill is.** Under the global package folder: `npm root --global` (or `pnpm
root --global`) names the folder that holds `vsift/skills/vsift/`.

**Corporate registries.** A registry mirror must serve both `vsift` and the `@vsift`
scope. **Lockfiles.** If an install from a lockfile made on another kind of machine
leaves this machine's platform package out (some package-manager versions record only
the platform they ran on), the launcher says so (exit 127); update the lockfile on a
machine of each kind, or reinstall without it.

## 3. What the launcher checks, and what its failures mean

Each time it runs, the launcher finds the platform package for this machine, checks that
it has the launcher's own version and that the executable's size and SHA-256 are those
recorded when the release was built, and then runs it with your arguments, standard
input, output and error. It exits with vsift's status; if vsift is ended by a signal, so
is the launcher. The checks take 5 to 11 ms on the hosted CI machines (about 40 ms on
a workstation without SHA extensions).

If the launcher cannot run vsift it prints one message on stderr, starting with
`vsift (npm launcher):`, and exits with a status vsift itself never uses:

| Exit | Meaning | What to do |
| ---: | --- | --- |
| 127 | No platform package for this machine: optional dependencies were omitted (`--omit=optional`, `--no-optional`), the lockfile was made without it, or the machine is not supported | Reinstall `vsift` with optional dependencies included, on a supported machine |
| 126 | The platform package is refused or cannot start: another version, an executable that does not match its recorded digest (damaged or replaced), a damaged launcher package, or a start error (on Linux, glibc or OpenSSL 3 missing; on Windows, an executable path of 260 characters or more) | Reinstall `vsift`; on Windows with a very long install path, install in a folder with a shorter path |

The message names the package it expected, the version and the supported targets.

## 4. Ctrl-C and signals

Ctrl-C works as it does without the launcher: a long command cancels at its next step
and ends with `CANCELLED`, and a second Ctrl-C stops it faster. On Linux and macOS the
launcher also passes on `SIGTERM` and `SIGHUP`, and `SIGINT` when it is not running in a
terminal. A supervisor should signal the `vsift` process it started rather than its whole
process group, which would deliver the signal twice ([L-091](../planning/known-limits.md#l-091)).

## 5. Uninstall

```console
npm uninstall --global vsift
pnpm remove --global vsift
bun remove --global vsift
yarn remove vsift            # in the project
```

This removes the launcher, the platform package and the command (Bun 1.2 leaves the
platform package in its global folder, `~/.bun/install/global/node_modules/@vsift/`,
where nothing runs it, and on Windows also a `vsift.exe` in Bun's `bin` folder that no
longer starts vsift; delete them to tidy up). Package caches keep
the downloaded tarballs until you clean them (`npm cache clean --force`, `pnpm store
prune`, `yarn cache clean`, `bun pm cache rm`). VSift's own data (sessions, the
configuration, managed tools) is not touched; `vsift setup list` and `vsift setup remove`
manage the tools, and sessions are disposable by default.

## 6. Without a JavaScript runtime

The native archives on GitHub Releases (`vsift-<version>-<target>.tar.gz`, with
`SHA256SUMS`) hold the same executable, the licences, `THIRD-PARTY-NOTICES`, a CycloneDX
SBOM and the skill ([`release.md`](release.md) section 2). Extract one and run
`vsift --version`. Verification against the release's build-provenance attestation, and
the prompts an unsigned download triggers on Windows and macOS, are documented here from
P13 PR 10 and 11.
