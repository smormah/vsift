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

**Version 0.2.4 is the hot fix of 0.2.3**, and the commands above, without a tag, install it: it
replaces 0.2.3 under the dist-tag `latest`, where 0.2.0 (2026-10-09) was the first release and 0.2.1
(2026-10-15), 0.2.2 (2026-10-18) and 0.2.3 (2026-10-25) the ones before this. It has not been announced. Yarn 4 holds back a version for a day after it is
published, so Yarn users get 0.2.4 a day after the publish (the installation guide has the setting that lets it through).

The dist-tag `next` is for release candidates and you do not need it: it names `0.2.4-rc.1`,
the release candidate that 0.2.4 was built from, or a later candidate or release. The older
candidates stay published; `0.2.0-rc.1`, `0.2.0-rc.2`, `0.2.1-rc.1`, `0.2.1-rc.2`, `0.2.1-rc.3` and `0.2.2-rc.1` are deprecated and superseded, as `0.2.4-rc.1` is once 0.2.4 is published.
The release candidates of later releases go to `next` when they are published and never to `latest`.

## What changed since 0.2.3

The changelog has the detail. In short: a new managed install on Ubuntu gets a newer, reviewed FFmpeg (9.0.2 plus 39
commits, a build of 2026-10-31, in place of the build of 2026-08-31), which carries the upstream memory-safety fixes
made since in code VSift runs (the VP8, VP9, H.264, Opus and AAC decoders, the MP4 key atom and a Matroska duration
overflow). A machine set up by 0.2.3 or earlier keeps its FFmpeg, and `vsift setup check` says it is older than the
reviewed one until you run `vsift setup plan` and `vsift setup install` again. **One flaw is not fixed:** an MP4 with
very many `pssh` boxes (CVE-2026-107678) can keep FFmpeg busy; the fix is on FFmpeg's development line only, and
VSift's time bound ends the work after about 15 seconds without fixing the flaw. No command or JSON field changed. The agent skill did not change.

## What was not tried

0.2.4 is built from the same source as the release candidate `0.2.4-rc.1` and differs from it
only in its version numbers, this README and the installation guide. That candidate was tried on hosted runners, on a
synthetic corpus and a synthetic voice; **only the checks that the new FFmpeg touches were run on it** (the install and upgrade checks, the managed install, the test with deliberately malformed and malicious files,
the local speech recognition accuracy set and the dependency scan), and nine other evidence items were waived by the maintainer for this release only, among them the stress, the
compatibility and the agent-trial runs, which run in the next window. The executables of 0.2.4 itself are checked after its publish. The accuracy set with noisy speech was not run on hosted macOS runners. The agent skill is unchanged, so its trials were not run again; in the earlier trials without the skill, a smaller model often stated the right facts without citing the
identifiers VSift returned for them (the help is to say how; the agent skill does). Two things were not tried before this release, and it ships without them:

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
line of a `.cmd` file (it expands `%NAME%` and acts on an unquoted `>`, `|` or `&`); no package can
leave that file out. Typing `vsift` in PowerShell or Git Bash is not affected (Yarn 1 writes no
PowerShell file, so there even PowerShell runs the `.cmd`). A program that passes `vsift` text it
did not write must not start `vsift.cmd`: neither through a shell (Node's `exec`, Python's
`shell=True`, `cmd /c`) nor with an argument list after a lookup through `PATHEXT` (Python's
`shutil.which("vsift")` finds the `.cmd`). Start the platform package's `vsift.exe`, or Node on
`bin/vsift.cjs`, by its full path, with an argument list and no shell.

The install guide, including the native archives for machines without a JavaScript
runtime and how to verify them, is
[`docs/operations/install.md`](https://github.com/smormah/vsift/blob/main/docs/operations/install.md).

## Licence

MIT OR Apache-2.0. Source: <https://github.com/smormah/vsift>.
