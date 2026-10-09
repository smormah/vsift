# vsift-cli

`vsift` turns a video on your machine into evidence you can search and cite: timestamped
transcripts, the moments the screen changed, and exact frames, crops and audio clips, all
processed locally. Use it on its own, for example to transcribe and search a recorded meeting,
or let your AI assistant use it. This package (`vsift-cli`) installs the `vsift` command and the
agent skill that teaches Claude Code or Codex to use it.

```console
npm install --global vsift-cli
vsift --version
vsift setup check
```

One-shot use works too: `npx vsift-cli`, `pnpm dlx vsift-cli`,
`yarn dlx --package vsift-cli vsift` or `bunx vsift-cli`. Node.js 22 or later,
or Bun 1.2 or later, runs the launcher.

## Which version you get

**Version 0.2.0 is the first release published under the dist-tag `latest`**, so the commands
above, without a tag, install it. Until then `latest` was an empty `0.0.0` placeholder, a package
with no command. It has not been announced. Yarn 4 holds back a version for a day after it is
published, so Yarn users get 0.2.0 a day after the publish (the installation guide has the
setting that lets it through).

The dist-tag `next` is for release candidates and you do not need it: it names `0.2.0-rc.3`,
the release candidate that 0.2.0 was built from, or a later candidate or release. The older
candidates `0.2.0-rc.1` and `0.2.0-rc.2` stay published and are superseded.

## What was not tried

0.2.0 is built from the same source as the release candidate `0.2.0-rc.3` and differs from it
only in its version numbers, this README and the installation guide. That candidate was tried on hosted runners and in
trials with the agent skill, on a synthetic corpus and a synthetic voice; nothing has been run
on a real recording. Two things were not tried before this release, and it ships without them:

- **Windows Smart App Control, SmartScreen and macOS Gatekeeper** have not been seen reacting
  to VSift, and no machine without a developer's tools has installed this package. The
  executables are not signed or notarized, and Smart App Control, where it is turned on, can
  block an unsigned program however it was installed, npm included.
- **A Mac.** VSift has not been run on a Mac by the people who build it; the macOS checks ran on hosted runners with
  Homebrew's tools, which VSift does not review.

The installation guide (linked below) and the register of known limits say what else is missing.

## What gets installed

This package holds a small launcher (`bin/vsift.cjs`, which runs `lib/launcher.cjs`) and the agent skill
(`skills/vsift/`). The native executable comes in one of three platform packages,
which your package manager picks by operating system and processor:

| Package | Runs on |
| --- | --- |
| `@vsift/win32-x64` | Windows x64 |
| `@vsift/darwin-arm64` | macOS on Apple silicon |
| `@vsift/linux-x64` | Linux x64 with glibc 2.35 or later and OpenSSL 3 |

No package has an install script, and installing downloads nothing else. The
launcher checks that the platform package has its own version and that the
executable's SHA-256 is the one recorded when the release was built, then runs it
with your arguments and standard streams, relays Ctrl-C and termination signals, and
exits with its status.

If the launcher cannot run the executable it says why on stderr and exits with 127
(no platform package for this machine, for example because optional dependencies were
omitted) or 126 (the platform package has another version, or the executable does not
match its recorded digest or cannot be started). Reinstall `vsift-cli` with optional
dependencies included; `vsift` itself never exits with either status.

On Windows, npm and pnpm also write a `vsift.cmd` file, and `cmd.exe` re-reads the command
line of a `.cmd` file (it expands `%NAME%` and acts on an unquoted `>` or `|`). Typing `vsift` in
PowerShell or Git Bash is not affected. A program that passes `vsift` text it did not write should
not go through `cmd.exe` (Node's `exec`, Python's `shell=True`, `cmd /c`): start the platform
package's `vsift.exe`, or Node on `bin/vsift.cjs`, with an argument list and no shell.

The install guide, including the native archives for machines without a JavaScript
runtime and how to verify them, is
[`docs/operations/install.md`](https://github.com/smormah/vsift/blob/main/docs/operations/install.md).

## Licence

MIT OR Apache-2.0. Source: <https://github.com/smormah/vsift>.
