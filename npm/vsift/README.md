# vsift

`vsift` gives AI coding agents local, source-grounded access to the evidence in a
video: timestamped transcripts, search, the moments the screen changed, and exact
frames, crops and audio clips, all processed on your machine. This package installs
the `vsift` command and the agent skill that teaches Claude Code or Codex to use it.

```console
npm install --global vsift@next
vsift --version
vsift setup check
```

One-shot use works too: `npx vsift@next`, `pnpm dlx vsift@next`, `yarn dlx vsift@next`
or `bunx vsift@next`. Node.js 22 or later, or Bun 1.2 or later, runs the launcher.

## What gets installed

This package holds a small launcher (`bin/vsift.cjs`) and the agent skill
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
match its recorded digest or cannot be started). Reinstall `vsift` with optional
dependencies included; `vsift` itself never exits with either status.

The install guide, including the native archives for machines without a JavaScript
runtime and how to verify them, is
[`docs/operations/install.md`](https://github.com/smormah/vsift/blob/main/docs/operations/install.md).

## Licence

MIT OR Apache-2.0. Source: <https://github.com/smormah/vsift>.
