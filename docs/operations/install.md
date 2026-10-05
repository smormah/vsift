# Installing VSift

Status: user guide, updated 2026-10-04 (P14 PR 9a; written by P13 PR 11 and PR 12 for the
published pre-release; [ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)).
**VSift 0.2.0-rc.1 is a release candidate under qualification.** It is available as the npm
packages under the dist-tag `next`, with npm provenance, and as native archives on a GitHub
pre-release, each carrying a Sigstore build-provenance attestation; `next` named the pre-release
0.1.0, published on 2026-10-01, until the candidate was published. It is not announced. It is not a stable
release, and no platform is "supported" yet (section 1). The evidence gathered so far, with its gaps, is recorded in the release evidence ledger
([`p14-evidence-ledger.json`](../planning/p14-evidence-ledger.json)). A plain `npm install vsift-cli`
installs the empty `0.0.0` placeholder, which stays `latest` until a stable release, so
always ask for `vsift-cli@next`. **What has been run against these steps.** In P13 they ran on
hosted runners against a local registry. Since 2026-10-02 (P14) the published 0.1.0 has also
been installed from the real registry with npm, pnpm, Yarn and Bun on hosted Windows, macOS and
Ubuntu runners, its three archives have been downloaded, checked, extracted and run, the offline
install of the managed tools has been run with the real files, the upgrade and uninstall steps
(sections 7 and 8) have been walked, and the same bytes have run the supplied-transcript and
local-speech journeys on all three systems. Those are results for 0.1.0 only (the release
candidate and the release repeat them on their own bytes), on hosted virtual machines, which
carry developer tools a clean machine lacks ([L-112](../planning/known-limits.md#l-112)). No
person has run them on a Mac, and nobody has seen Smart App Control or Gatekeeper react to a
VSift file (section 4). The records are the [P14 plan](../planning/p14-qualification.md)
(sections 15 to 18) and [`p13-distribution.md`](../planning/p13-distribution.md). Building from
source ([`development.md`](../development.md)) also works.

## 1. What is and is not supported

| Machine | Status today | What has been shown (0.1.0, hosted runners) | Notes |
| --- | --- | --- | --- |
| Windows 11 x64 | R0 target | Installs with all four package managers, the archive runs and both journeys ran, on a hosted Windows Server 2025 image; the project's Windows 11 development machine ran the agent trials | npm package `@vsift/win32-x64`. Bring your own FFmpeg, FFprobe and whisper.cpp (section 5.2). Smart App Control is untried (section 4) |
| macOS 15 on Apple silicon | R0 target | The same checks passed on a hosted macOS 15 image with Homebrew's tools, which VSift does not review ([L-114](../planning/known-limits.md#l-114)); no person has run VSift on a Mac | `@vsift/darwin-arm64`. Bring your own tools (section 5.2). Gatekeeper is untried (section 4) |
| Ubuntu 24.04 on x64 | R0 target, and the only machine where VSift installs its own tools | The same checks passed, with the managed tools installed by the binary itself, also offline from the real files | `@vsift/linux-x64`; needs glibc 2.35 or later and OpenSSL 3 (`libssl.so.3`), which Ubuntu 22.04 and 24.04 have. The managed whisper.cpp build also needs the OpenMP runtime `libgomp.so.1` (Ubuntu package `libgomp1`, which a minimal container image lacks; section 5.1) |
| Another Linux on x64 with glibc 2.35 or later and OpenSSL 3 | not a target | Nothing | The binary is built on Ubuntu 22.04 and may run; it has not been tested |
| Linux on Arm, Intel Macs, Windows on Arm, Windows 10, Alpine and other musl Linux | not supported | Nothing | The launcher says so and exits 127 (section 10) |

- **"R0 target" means a target VSift was built and tested for**, not yet a supported
  platform: the support matrix, [`support-and-resource-profiles.md`](../planning/support-and-resource-profiles.md),
  states the four rules a machine must meet before it earns the word "supported", what each
  machine has shown and what is still missing ([L-035](../planning/known-limits.md#l-035)).
  What was run where is in [`p13-distribution.md`](../planning/p13-distribution.md) and the P14
  plan.
- **Managed installation** (`vsift setup install`, section 5.1) is qualified on Ubuntu
  24.04 x64 only (ADR 0023 decision E). On Windows and macOS you install FFmpeg,
  FFprobe, whisper.cpp and the speech model yourself and tell VSift where they are
  (section 5.2).
- **A pre-release** has no stability promise beyond the versioned JSON contracts
  ([`cli-v1.md`](../contracts/cli-v1.md)). The readable text that commands print without
  `--json` is for people and may change. A stable release waits for the release
  qualification of P14, which is in progress.
- **Not signed.** The Windows and macOS executables carry no Authenticode signature and
  no Apple notarization. Section 4 says what that means for you and how to check a
  download instead.

To install through a package manager you need Node.js 22 or later, or Bun 1.2 or later,
to run the small launcher. Installing from a native archive needs neither.

## 2. Install with a package manager

The npm package is `vsift-cli`; the command it installs is `vsift`. The pre-release is
published under the dist-tag `next`, so always ask for `vsift-cli@next`. **Do not install
`vsift-cli` without `@next`:** `latest` stays the empty `0.0.0` placeholder until the first
stable release, and installs a package that has no command. Install globally, so the
`vsift` command is on your `PATH`:

```console
npm install --global vsift-cli@next
pnpm add --global vsift-cli@next
bun add --global vsift-cli@next
```

Yarn 4 has no global install; add VSift to a project and run it through Yarn:

```console
yarn add vsift-cli@next
yarn vsift --version
```

Recent Yarn 4 releases (4.18.1 checked) hold back any version published less than a day
ago (`npmMinimalAgeGate`, default one day), so for the first day after a release Yarn
reports that the versions of `vsift-cli` or `@vsift/...` "are quarantined". Wait a day, or
exempt VSift in the project's `.yarnrc.yml`:

```yaml
npmPreapprovedPackages:
  - vsift-cli
  - "@vsift/*"
```

Or run it once without installing:

```console
npx vsift-cli@next --version
pnpm dlx vsift-cli@next --version
yarn dlx --quiet --package vsift-cli@next vsift --version
bunx vsift-cli@next --version
```

**What is installed.** The `vsift-cli` package holds a small launcher (`bin/vsift.cjs` and
`lib/launcher.cjs`) and the agent skill (`skills/vsift/`, the folder to give Claude Code or
Codex). It lists the three platform packages as optional dependencies at its own exact
version; your package manager installs only the one whose `os` and `cpu` match your
machine. No package runs an install script, so installs work unchanged when scripts are
disabled (`--ignore-scripts`, Bun's default, pnpm's and Yarn's settings), and installing
downloads nothing else. After the install the command works offline. Media tools and the
speech model are separate (section 5).

**On Windows, do not let `cmd.exe` read text you did not write** ([L-109](../planning/known-limits.md#l-109),
issue #257). npm and pnpm create a `vsift.cmd` beside the `vsift.ps1` that PowerShell runs for
`vsift`, and `cmd.exe` re-reads the command line of a `.cmd` file: it expands `%NAME%`, can
drop a double quote, and acts on an unquoted `>`, `|` or `&` as its own syntax. On a hosted
Windows runner the argument `a;echo,x>marker`, started through `vsift.cmd`, created the file
`marker`: a command it spelled ran. VSift cannot change this: the `.cmd` file is generated by
npm and pnpm for every command a package installs, and the launcher behind it never sees the
original line.

- **Who is affected.** A program that passes `vsift` text it did not write (a search query
  taken from a transcript, a file name) **through `cmd.exe`**: Node's `child_process.exec` or
  `execSync`, Python's `subprocess` with `shell=True`, `cmd /c vsift ...`, a batch file; or a
  person who types such text into a `cmd.exe` prompt. Nothing else is: the `.cmd` is not used
  when you type `vsift` in PowerShell (it runs `vsift.ps1`), in Git Bash (the extensionless
  shell script) or under Bun (`vsift.exe`), and a hosted Windows runner gave each of those 14
  hostile file names (each had to open the file) and 11 hostile arguments (each had to run no
  command), with leading dashes, spaces, accents, `$()`, `;&^`, `%` and `--help` after `--`,
  and VSift answered exactly as when it is started directly.
- **Safe routes: an argument list, never a command string.** (1) PowerShell or Git Bash.
  (2) The native archive's `vsift.exe` (section 3) called directly: Node `execFile` or
  `spawn` without `shell`, Python `subprocess.run([...])` with a list, a process-creation call
  in any other language. (3) From an npm or pnpm install, the same without `cmd.exe`: run
  Node on the launcher, `execFile(process.execPath, [launcher, ...args])` with `launcher` =
  `<npm root --global>\vsift-cli\bin\vsift.cjs`, which is what the `.cmd` itself runs; or
  call the platform package's `vsift.exe` under `@vsift\win32-x64` (it skips the launcher's
  version and digest checks). A test (`npm/test/launcher.test.cjs`) sends the hostile
  arguments through that launcher route and requires them to arrive unchanged.
- **AI agents on Windows** should run `vsift` from PowerShell, Git Bash or the native
  `vsift.exe`, not through `cmd.exe`.

**Where the skill is.** Under the global package folder: `npm root --global` (or `pnpm
root --global`) names the folder that holds `vsift-cli/skills/vsift/`. Copy it as
[`../agents/skill.md`](../agents/skill.md) describes.

**Registries and lockfiles.** A registry mirror must serve both `vsift-cli` and the
`@vsift` scope. If an install from a lockfile made on another kind of machine leaves this
machine's platform package out (some package-manager versions record only the platform
they ran on), the launcher says so (exit 127); update the lockfile on a machine of each
kind, or reinstall without it.

## 3. Install from a native archive (no JavaScript runtime)

Each release on GitHub carries one archive per target, `SHA256SUMS`, and each target's
SBOM and notices file. The archive is `vsift-<version>-<target>.tar.gz` (a gzip-compressed
tar file on every system, Windows included; File Explorer opens it and `tar -xf` extracts
it) with `<target>` one of:

| Machine | `<target>` |
| --- | --- |
| Windows 11 x64 | `x86_64-pc-windows-msvc` |
| macOS 15, Apple silicon | `aarch64-apple-darwin` |
| Linux x64 | `x86_64-unknown-linux-gnu` |

It holds one folder with the `vsift` executable (`vsift.exe` on Windows), the licences,
`THIRD-PARTY-NOTICES`, a CycloneDX SBOM (`vsift.cdx.json`) and the agent skill
([`release.md`](release.md) section 2). The steps, in order of how much they prove:

1. **Download** the archive for your machine and `SHA256SUMS` from the release page.
2. **Check the checksum.** It finds a damaged or truncated download. It does not prove
   who made the file, because it comes from the same place as the file
   ([SEC-23](../planning/security-threat-model.md)). `SHA256SUMS` lists the three archives
   only; the SBOMs, notices and npm tarballs are covered by step 3.

   ```console
   grep 'x86_64-unknown-linux-gnu.tar.gz' SHA256SUMS | sha256sum --check       # Linux
   grep 'aarch64-apple-darwin.tar.gz' SHA256SUMS | shasum -a 256 --check       # macOS
   ```

   ```powershell
   (Get-FileHash -Algorithm SHA256 .\vsift-<version>-x86_64-pc-windows-msvc.tar.gz).Hash
   Select-String 'x86_64-pc-windows-msvc' .\SHA256SUMS   # the two hashes must be equal (case aside)
   ```

3. **Check the attestation.** This is the check that matters. Every release file and every
   npm tarball has a Sigstore build-provenance attestation made by this repository's
   Release workflow. With the [GitHub CLI](https://cli.github.com/) signed in to any
   GitHub account:

   ```console
   gh attestation verify vsift-<version>-<target>.tar.gz --repo smormah/vsift \
     --signer-workflow smormah/vsift/.github/workflows/release.yml \
     --source-ref refs/tags/v<version> --deny-self-hosted-runners
   ```

   A pass says this exact file was built by the Release workflow of `smormah/vsift` on a
   GitHub-hosted runner from the commit that tag `v<version>` names. It does not say the
   source is free of bugs, and it is not a claim about who the publisher is: there is no
   publisher signature (section 4).

4. **Extract and run** (Windows 11 has `tar` in a terminal):

   ```console
   tar -xzf vsift-<version>-<target>.tar.gz
   ./vsift-<version>-<target>/vsift --version
   ```

   Put the folder on your `PATH`, or copy the executable somewhere that is. The
   `--version` line ends in the first 12 digits of the commit it was built from, which must
   be the start of the commit the release tag names (`git ls-remote
   https://github.com/smormah/vsift 'refs/tags/v<version>^{}'`).

   **On Windows, use the `tar` that comes with Windows** (`C:\Windows\System32\tar.exe`).
   Git for Windows installs a GNU `tar` too, and when its tools come first on your `PATH` it
   reads `D:\...` as the name of another machine and stops with "Cannot connect to D". Call the
   Windows one by its full path, or give the GNU one `--force-local`. A hosted Windows runner
   met exactly this on 2026-10-02.

## 4. Windows and macOS: SmartScreen, Gatekeeper and what to check instead

The Windows and macOS executables are **not code-signed and not notarized**. That is a
recorded decision (ADR 0023 decision C), not an oversight: certificates and an Apple
developer account are recurring costs and key custody that this stage does not need, so
the trust signals are the Sigstore attestation and npm provenance of section 3 and
section 6, which anyone can verify. It also means **VSift makes no claim of publisher
trust**, and the operating systems treat an unsigned program from the internet with
suspicion. What follows is written from Microsoft's and Apple's documentation: VSift has
not yet watched these prompts appear for one of its own archives. (One `npx vsift
--version` of the published npm package ran on a Windows 11 machine with no block or
prompt; that machine has Smart App Control Off, so this says nothing about it.)

**Windows.**

- *SmartScreen.* A file downloaded in a browser is marked as coming from the internet.
  When Windows starts such a file that has no download reputation and no signature,
  Microsoft Defender SmartScreen can show a window titled "Windows protected your PC"
  saying it prevented an unrecognized app from starting. **More info** then **Run
  anyway** continues. It is not a verdict that the file is harmful: a new unsigned file
  has no reputation. If you have verified the file (section 3), you may run
  `Unblock-File .\vsift.exe` once instead. Whether the mark reaches the extracted
  executable depends on the tool that extracted the archive.
- *Smart App Control.* Windows 11 can run Smart App Control, which, when it is **On**,
  blocks unsigned programs that Microsoft's reputation service does not recognise
  ([Microsoft's description](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/overview);
  it does not tie this to a download mark). **Installing through npm may therefore not
  help with this.** Check Settings, Windows Security, App and browser control. VSift has not
  been run on a machine where it is known to be On, and expects to be blocked there
  ([L-098](../planning/known-limits.md#l-098)). Microsoft says Smart App Control can only be
  turned on by a clean install, so turning it off is a decision to take with care; it also
  says the feature turns itself off on machines that look like developers'.
- *Policies that allow only signed software* (AppLocker, App Control for Business) will
  refuse VSift for the same reason.

**macOS.**

- *Gatekeeper.* A file downloaded in a browser is quarantined. The first time macOS opens
  a quarantined program that is not notarized it stops it with a message that Apple cannot
  check it for malicious software (older versions: "cannot be opened because the developer
  cannot be verified"). Apple's guide for macOS 15 gives one way through: after the
  attempt open System Settings, Privacy & Security, scroll down, choose **Open Anyway** and
  confirm ([Apple's guide](https://support.apple.com/en-us/102445)). If you have verified
  the file (section 3), `xattr -d com.apple.quarantine ./vsift` removes the mark instead.

**Linux** shows no prompt.

**Installing through npm avoids the download mark** on both systems: a package manager
writes the executable without it, so neither SmartScreen's download check nor Gatekeeper
sees a quarantined file. Command-line downloaders such as `curl` and `gh release download`
generally do not add it either. Smart App Control and signed-software policies are not
described in terms of the mark, as above.

**What to check instead of a signature**, whichever way you installed:

| Check | What it proves | Command |
| --- | --- | --- |
| `SHA256SUMS` | The archive is the one listed on the release page; no proof of origin | section 3, step 2 |
| `gh attestation verify` | The file was built by this repository's Release workflow from the tagged commit | section 3, step 3 (also works on an npm tarball from `npm pack vsift-cli@next`) |
| `npm audit signatures` | The installed npm packages carry valid registry signatures and provenance attestations from that workflow | section 6 |
| The launcher's own check | The platform package is the one released with this launcher, and the executable's size and SHA-256 are the ones recorded when it was built | automatic on every run (section 10) |

## 5. Install FFmpeg, FFprobe, whisper.cpp and the speech model

VSift needs FFmpeg and FFprobe for any media work. whisper.cpp and its model are needed
only for local speech recognition: a transcript you already have (SRT or WebVTT, `vsift
ingest <video> --transcript <file>`) needs neither. Installing VSift, from npm or an
archive, downloads none of these. Nothing downloads them unless you run `vsift setup
install` yourself, and the VSift agent skill never runs it for you.

`vsift setup check` says what is missing and where each tool was found. Run it first and
after every change below.

### 5.1 Ubuntu 24.04 x64: let VSift install them

`vsift setup install` applies a plan you have read and accepted. It works per user, needs no
`sudo`, never elevates and never prompts.

```console
vsift setup check
vsift setup plan --profile desktop                 # read it: what, from where, how big
vsift setup plan --profile desktop --json > plan.json
vsift setup install --plan plan.json --accept-plan <the plan digest>
vsift setup check                                  # each tool now reports [managed version]
```

- **The plan** is read-only and lists, for each component still needed, the publisher, the
  exact address, the exact size, the SHA-256, the licence and notice link, and every file
  it will install. The three components are the FFmpeg and FFprobe build from BtbN
  FFmpeg-Builds (113 MB), the whisper.cpp v1.9.2 command-line build (9.5 MB) and the
  multilingual `base` model from the whisper.cpp model repository (148 MB): about 271 MB
  in three downloads. Keep at least 1 GB of disk free while installing (a cautious figure,
  not a measurement; the plan has the exact sizes). Licence information is disclosure, not
  legal clearance.
- **The digest** is the plan's own fingerprint: copy the `Plan digest:` line of the plan
  (`plan_digest` in the JSON). Giving it to `--accept-plan` is your acceptance of exactly
  that plan. If anything changed since you saved it (the reviewed catalogue, your
  configuration, what is installed in a way that changes the plan), the install refuses
  and tells you to run `setup plan --json` again.
- **What it does,** one component at a time: downloads from the publisher over HTTPS,
  checking the exact size and SHA-256 as the bytes arrive; unpacks only the reviewed files
  into a private stage; runs the tools in the stage (their version banners, a tiny media
  fixture and a tiny speech fixture) before it uses them; then activates the component
  with one atomic step. A component that fails never becomes active, and what was active
  before stays. Run the same command again and it continues from the first component that
  is not current. It restarts an interrupted download from byte zero: it never resumes.
- **A library a minimal image lacks (prerequisite: `libgomp1`).** The reviewed whisper.cpp build is linked
  against the OpenMP runtime, `libgomp.so.1` (Ubuntu package `libgomp1`). A hosted Ubuntu
  24.04 runner has it; the minimal `ubuntu:24.04` container image does not, and there
  `setup install` installs FFmpeg and then stops at the whisper.cpp component with
  `MISSING_CAPABILITY` (the banner check, `provider_failed`). The error names the library: the
  component carries `missing_shared_library: "libgomp.so.1"` and the remediation says to run
  `sudo apt-get install libgomp1` and then the same `setup install` again. (VSift takes the
  library's file name from the program's own error output only when it is a plain library
  name; for any other library the remediation names no package.) Install it before the first
  `setup install` on a container or other minimal image.
- **Where:** `~/.local/share/vsift/managed-v1` (or under `$XDG_DATA_HOME`). Nothing is
  written outside VSift's own folders.
- **Ctrl-C** cancels, discards the stage in progress and keeps what is already active. A
  second `setup install`, `rollback` or `remove` while one runs answers `BUSY` with a hint
  to retry in 30 seconds.
- **Offline or no direct access:** download the three files named by the plan's addresses
  on a machine that can (the file name is the last part of each address), copy them into a
  folder and run `vsift setup install --plan plan.json --accept-plan <digest>
  --artifact-dir /absolute/path/to/folder`. The files go through the same size and SHA-256
  check as a download; you supply only the folder, never an address or a checksum.
- **Managing what is installed.** VSift keeps each component's current and previous
  version, and never removes one that a running job holds.

  | Command | What it does |
  | --- | --- |
  | `vsift setup list` | Each component's selected and previous version, and whether each verifies. Read-only |
  | `vsift setup rollback <component> [--version <v>]` | Selects the previous (or a named) installed version, after it verifies |
  | `vsift setup remove <component> [--version <v>]` | Removes a version (not the selected one) or the whole component. `--stale-stages` removes what an interrupted install left |
  | `vsift setup repair` | Diagnoses a store an interruption left and prints the commands that fix it. It changes nothing: you run them |

  `<component>` is `ffmpeg_ffprobe`, `whisper_cli` or `whisper_model`. Every command
  re-checks the managed files against their SHA-256 when it uses them; commands that use
  the tools took 0.17 to 0.65 seconds in total on a hosted runner
  ([L-087](../planning/known-limits.md#l-087)).
- **Power loss.** On Ubuntu 24.04 with a local ext4 disk, a managed command that reported
  success survives a power loss, and whatever a power loss catches half done is detected
  and repaired. That is the only filesystem it was tested on
  ([`p13-distribution.md`](../planning/p13-distribution.md)).

If a download fails, section 9 lists each reason. VSift also falls back to section 5.2
whenever it cannot install safely.

### 5.2 Windows, macOS and other machines: bring your own

Install FFmpeg and FFprobe with whatever you normally use, and, for local speech
recognition, whisper.cpp's `whisper-cli` and a model. VSift does not review, pin or
redistribute these builds. whisper.cpp v1.9.2 publishes no macOS command-line build, so there you
build it from its source or take it from a package manager. Tools on your `PATH` are found
automatically; tools elsewhere are registered once, with absolute paths:

```console
vsift setup configure ffmpeg --executable /absolute/path/to/ffmpeg
vsift setup configure ffprobe --executable /absolute/path/to/ffprobe
vsift setup configure whisper --executable /absolute/path/to/whisper-cli
vsift setup configure-model --file /absolute/path/to/ggml-base.bin
vsift setup check
```

On Windows use paths such as `"C:\tools\ffmpeg\bin\ffmpeg.exe"`. The reviewed model is the
multilingual `base` model, `ggml-base.bin`, 147,951,465 bytes, SHA-256
`60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe`; `setup check` reports
a file with that size and hash as the pinned model, and a different model as a different
model. `setup check` reports `ready` when the tools answer; the first media operation that
needs them (today `ingest --transcript`) then runs a small built-in video through FFmpeg and
FFprobe once and refuses a pair that fails.
VSift never deletes a tool you registered.

## 6. Check what you installed

A short walk-through, in the order that shows the most for the least effort. Use a scratch
folder.

1. **The command runs.** `vsift --version` prints `vsift <version> (<12 hex digits>)`: the
   release build names its source commit (section 3 shows how to compare it with the tag).
   `vsift --help` lists every command.
2. **The npm packages are the released ones** (npm installs only), from a project folder
   so that `npm audit signatures` has something to read:

   ```console
   mkdir vsift-check && cd vsift-check
   echo '{"private": true}' > package.json    # PowerShell: Set-Content package.json '{"private": true}'
   npm install vsift-cli@next
   npm audit signatures
   npx vsift --version
   ```

   `npm audit signatures` should report the registry signatures and the provenance
   attestations as verified (for 0.1.0 on Windows it reported 4 packages with verified
   registry signatures and 4 with verified attestations); npmjs.com shows each version as
   built and signed on GitHub Actions.
3. **The archive is the released one** (archive installs): section 3, steps 2 and 3.
4. **Your setup is ready.** `vsift setup check` reports each tool, how it was found
   (`managed version`, your configured path, or `PATH`) and whether local speech
   recognition works here. After section 5.1, `vsift setup list` reports each component
   `verified`.
5. **A first run** on a short video you may share (a supplied transcript needs no speech
   recognition): `vsift ingest ./clip.mp4 --transcript ./clip.vtt`, then `vsift session
   list`. Sessions are disposable and expire after 24 idle hours; `vsift session close
   <session>` ends one now.

## 7. Upgrade

- **npm:** install again with the tag: `npm install --global vsift-cli@next` (likewise for
  pnpm and Bun; Yarn: `yarn add vsift-cli@next` again in the project). The launcher and the
  platform package move together, because the launcher pins the platform packages to its
  own exact version; the launcher refuses a platform package of another version (exit 126).
- **Archive:** extract the new archive over an empty folder and replace the old one, after
  the checks of section 3.
- **Managed tools** do not change with VSift. A newer VSift can carry a newer reviewed
  catalogue: run `vsift setup plan` again, read it, and accept it with `setup install`; the
  old versions stay as the previous version until the next install.
- **Sessions, configuration and managed tools** are kept. Going back to an older VSift is
  not supported: it may refuse a session a newer one wrote
  ([L-044](../planning/known-limits.md#l-044)).
- **What was run.** P14 ran the npm upgrade on hosted runners with sessions, a retained bundle and a
  registered configuration in place: the configuration came back byte for byte, an earlier session read
  as before and a new session worked. Only 0.1.0 is published, so the upgrade was 0.1.0 over 0.1.0 and
  0.1.0 to a locally built newer version, not a published newer release over 0.1.0
  ([L-111](../planning/known-limits.md#l-111)); only npm was upgraded, and no pnpm, Yarn or Bun upgrade
  was run.

## 8. Uninstall

P14 walked this section on hosted Windows, macOS and Ubuntu runners after the upgrade run above: the
managed tools removed, the package gone, VSift's own folders exactly the ones step 3 names, and
deleting them left no trace; the evidence bundle, the registered tools and the source videos stayed.

1. **Managed tools first, if you used them** (you need the `vsift` command for this):
   `vsift setup remove ffmpeg_ffprobe`, `vsift setup remove whisper_cli`, `vsift setup
   remove whisper_model`, then `vsift setup remove --stale-stages`. Content VSift cannot
   prove is its own (a link, an unknown file) is kept and `vsift setup repair` names it; delete
   the managed folder by hand in that case ([L-090](../planning/known-limits.md#l-090)).
2. **The package:**

   ```console
   npm uninstall --global vsift-cli
   pnpm remove --global vsift-cli
   bun remove --global vsift-cli
   yarn remove vsift-cli        # in the project
   ```

   This removes the launcher, the platform package and the command. Bun 1.2 leaves the
   platform package in its global folder, `~/.bun/install/global/node_modules/@vsift/`,
   where nothing runs it, and on Windows also a `vsift.exe` in Bun's `bin` folder that no
   longer starts vsift; delete them to tidy up. Package caches keep the downloaded
   tarballs until you clean them (`npm cache clean --force`, `pnpm store prune`, `yarn
   cache clean`, `bun pm cache rm`). An archive install is deleting its folder and its
   `PATH` entry.
3. **VSift's own folders**, if you want them gone:

   | What | Windows | macOS | Linux |
   | --- | --- | --- | --- |
   | Configuration (registered tool paths, and the record of the media-tool check) | `%LOCALAPPDATA%\vsift` | `~/Library/Application Support/vsift` | `$XDG_CONFIG_HOME/vsift` or `~/.config/vsift` |
   | Disposable sessions (private copies of videos you ingested) | `%LOCALAPPDATA%\VSift-sessions` | `~/Library/Caches/VSift-sessions` | `$XDG_CACHE_HOME/vsift-sessions` or `~/.cache/vsift-sessions` |
   | Managed tools | not installed | not installed | `$XDG_DATA_HOME/vsift/managed-v1` or `~/.local/share/vsift/managed-v1` (the `vsift` folder above it holds nothing else: delete it too) |

   `vsift session clean --expired` removes expired sessions first, if you prefer.
4. **What VSift never deletes:** your original videos and transcripts (it works on private
   copies), the evidence bundles you exported with `session retain` (they stay where you
   put them, outside automatic cleanup), and any tool you registered with `setup
   configure` (it can address only its own managed folder).

## 9. Proxies, firewalls and corporate environments

**npm side.** A registry mirror must serve `vsift-cli` and the `@vsift` scope. The
packages have no install scripts, so a policy that disables scripts changes nothing.
`npm audit signatures` needs the registry; its provenance check may also need Sigstore's
public services. A policy that allows only signed programs cannot run VSift (section 4).

**`setup install`** is the one command that downloads. It uses the system proxy settings
(on Linux the `HTTPS_PROXY` and `ALL_PROXY` variables are tested), never prints proxy
credentials, and talks only to the reviewed publishers over HTTPS: `github.com` and
`release-assets.githubusercontent.com` for FFmpeg and whisper.cpp, `huggingface.co` and
`us.aws.cdn.hf.co` for the model. It follows redirects only along that route. A failed
download is `DOWNLOAD_FAILED` (exit 7) with one reason and no address, header or credential in
the message:

| Reason | What happened | What to do |
| --- | --- | --- |
| `tls` | The server's certificate is not trusted here, for example a TLS-intercepting proxy | Add your organisation's CA to the system trust store, or use `--artifact-dir` |
| `redirect_policy` | A redirect left the reviewed route (another host or scheme, credentials in the address, more than three hops) | A proxy or captive portal, or a publisher that moved its files: use `--artifact-dir` or section 5.2; report it if it persists with no proxy |
| `http_status` | The server did not answer with one complete `200` (an asset removed, a refusal, a rate limit, a partial `206`) | Retry later, or `--artifact-dir` |
| `proxy_auth` | The proxy asked for credentials (`407`) | Give the proxy its credentials in the proxy address your environment uses, or `--artifact-dir` |
| `offline` | No connection, a dropped or stalled transfer, or the deadline | Check the network and run the same command again (it restarts that download) |
| `size` | The declared or received size is not the reviewed size | The file is not the reviewed file: do not retry blindly; use section 5.2 or report it |

Bytes of the reviewed size with the wrong SHA-256 are a different failure,
`INTEGRITY_FAILURE` (also exit 7): the file is discarded and never used. The reviewed
addresses and hosts are fixed in VSift; if a publisher withdraws a file or changes its
download host, the install fails safe until a new VSift release carries a new reviewed
catalogue ([L-099](../planning/known-limits.md#l-099)), and section 5.2 is the way
meanwhile.

## 10. What the launcher checks, and its exit codes 126 and 127

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
| 127 | No platform package for this machine: optional dependencies were omitted (`--omit=optional`, `--no-optional`), the lockfile was made without it, or the machine is not supported | Reinstall `vsift-cli` with optional dependencies included, on one of the machines in section 1 |
| 126 | The platform package is refused or cannot start: another version, an executable that does not match its recorded digest (damaged or replaced), a damaged launcher package, or a start error (on Linux, glibc or OpenSSL 3 missing; on Windows, an executable path of 260 characters or more) | Reinstall `vsift-cli`; on Windows with a very long install path, install in a folder with a shorter path ([L-094](../planning/known-limits.md#l-094)) |

The message names the package it expected, the version and the three machines the release is
built for (section 1). The
digest check finds a damaged or mismatched package; it is not a defence against someone
who can already write to the install folder ([L-093](../planning/known-limits.md#l-093)).

## 11. Ctrl-C and signals

Ctrl-C works as it does without the launcher: a long command cancels at its next step
and ends with `CANCELLED`, and a second Ctrl-C stops it faster. On Linux and macOS the
launcher also passes on `SIGTERM` and `SIGHUP`, and `SIGINT` when it is not running in a
terminal. A supervisor should signal the `vsift` process it started rather than its whole
process group, which would deliver the signal twice ([L-091](../planning/known-limits.md#l-091)).

## 12. Problems

**`INTEGRITY_FAILURE` right after `--session-root <folder>`.** If the folder already exists and
VSift did not create it (an empty folder you made with `mkdir`, say), VSift never adopts it and
leaves it untouched. The error's remediation says so: name a `--session-root` path that does not
exist yet (VSift creates it, private to you), or delete the folder yourself if it holds nothing
you need. An empty folder made a moment ago is refused after a wait of up to five seconds,
because it could be another VSift process still creating it.

Run `vsift setup check --json` and read [`SUPPORT.md`](../../SUPPORT.md) before opening an
issue; remove secrets, personal information and sensitive paths first. Security
concerns follow [`SECURITY.md`](../../SECURITY.md). Maintainers publishing a release use
[`release.md`](release.md).
