# vsift-cli

`vsift` turns a video on your machine into evidence you can search and cite: timestamped
transcripts, the moments the screen changed, and exact frames, crops and audio clips, all
processed locally. Use it on its own, for example to transcribe and search a recorded meeting,
or let your AI assistant use it. This package (`vsift-cli`) installs the `vsift` command and the
agent skill that teaches Claude Code or Codex to use it.

```console
npm install --global vsift-cli@next
vsift --version
vsift setup check
```

One-shot use works too: `npx vsift-cli@next`, `pnpm dlx vsift-cli@next`,
`yarn dlx --package vsift-cli@next vsift` or `bunx vsift-cli@next`. Node.js 22 or later,
or Bun 1.2 or later, runs the launcher. **This version is a release candidate under qualification.**
It is published under the dist-tag `next` and is not announced; `latest` stays a `0.0.0` placeholder until the
first stable release.

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
