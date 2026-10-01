# Installing VSift

Status: user guide, 2026-10-01 (P13 PR 11, updated by PR 12 for the published pre-release;
[ADR 0023](../decisions/0023-r0-distribution-managed-installation-and-handoff-check.md)).
**VSift 0.1.0 is a pre-release, published on 2026-10-01.** It is available as the npm
packages under the dist-tag `next`, with npm provenance, and as native archives on a GitHub
pre-release, each carrying a Sigstore build-provenance attestation. It is not a stable
release, and no platform is "supported" yet (section 1). A plain `npm install vsift-cli`
installs the empty `0.0.0` placeholder, which stays `latest` until a stable release, so
always ask for `vsift-cli@next`. The steps below were qualified on hosted runners against a
local registry; the one install from the real registry so far was npm on a Windows 11
development machine, and no coding agent has used the published package (the record is
[`p13-distribution.md`](../planning/p13-distribution.md), "First publish"). Building from
source ([`development.md`](../development.md)) also works.

## 1. What is and is not supported

| Machine | Status | Notes |
| --- | --- | --- |
| Windows 11 x64 | R0 target | npm package `@vsift/win32-x64`. Bring your own FFmpeg, FFprobe and whisper.cpp (section 5.2) |
| macOS 15 on Apple silicon | R0 target | `@vsift/darwin-arm64`. Bring your own tools (section 5.2). No media, speech or evidence check has run on macOS yet ([L-035](../planning/known-limits.md#l-035)) |
| Ubuntu 24.04 on x64 | R0 target, and the only machine where VSift installs its own tools | `@vsift/linux-x64`; needs glibc 2.35 or later and OpenSSL 3 (`libssl.so.3`), which Ubuntu 22.04 and 24.04 have |
| Another Linux on x64 with glibc 2.35 or later and OpenSSL 3 | not a target | The binary is built on Ubuntu 22.04 and may run; it has not been tested |
| Linux on Arm, Intel Macs, Windows on Arm, Windows 10, Alpine and other musl Linux | not supported | The launcher says so and exits 127 (section 10) |

- **"R0 target" means a target VSift was built and tested for**, not yet a supported
  platform: the release matrix that earns the word "supported" is the next packet's work
  ([L-035](../planning/known-limits.md#l-035)). What was run where is in
  [`p13-distribution.md`](../planning/p13-distribution.md).
- **Managed installation** (`vsift setup install`, section 5.1) is qualified on Ubuntu
  24.04 x64 only (ADR 0023 decision E). On Windows and macOS you install FFmpeg,
  FFprobe, whisper.cpp and the speech model yourself and tell VSift where they are
  (section 5.2).
- **A pre-release** has no stability promise beyond the versioned JSON contracts
  ([`cli-v1.md`](../contracts/cli-v1.md)). The readable text that commands print without
  `--json` is for people and may change. A stable release waits for the release
  qualification of the next packet.
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
prompt; that machine's Smart App Control state was not checked.)

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

## 8. Uninstall

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
   | Configuration (registered tool paths) | `%LOCALAPPDATA%\vsift` | `~/Library/Application Support/vsift` | `$XDG_CONFIG_HOME/vsift` or `~/.config/vsift` |
   | Disposable sessions (private copies of videos you ingested) | `%LOCALAPPDATA%\VSift-sessions` | `~/Library/Caches/VSift-sessions` | `$XDG_CACHE_HOME/vsift-sessions` or `~/.cache/vsift-sessions` |
   | Managed tools | not installed | not installed | `$XDG_DATA_HOME/vsift/managed-v1` or `~/.local/share/vsift/managed-v1` |

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
| 127 | No platform package for this machine: optional dependencies were omitted (`--omit=optional`, `--no-optional`), the lockfile was made without it, or the machine is not supported | Reinstall `vsift-cli` with optional dependencies included, on a supported machine |
| 126 | The platform package is refused or cannot start: another version, an executable that does not match its recorded digest (damaged or replaced), a damaged launcher package, or a start error (on Linux, glibc or OpenSSL 3 missing; on Windows, an executable path of 260 characters or more) | Reinstall `vsift-cli`; on Windows with a very long install path, install in a folder with a shorter path ([L-094](../planning/known-limits.md#l-094)) |

The message names the package it expected, the version and the supported targets. The
digest check finds a damaged or mismatched package; it is not a defence against someone
who can already write to the install folder ([L-093](../planning/known-limits.md#l-093)).

## 11. Ctrl-C and signals

Ctrl-C works as it does without the launcher: a long command cancels at its next step
and ends with `CANCELLED`, and a second Ctrl-C stops it faster. On Linux and macOS the
launcher also passes on `SIGTERM` and `SIGHUP`, and `SIGINT` when it is not running in a
terminal. A supervisor should signal the `vsift` process it started rather than its whole
process group, which would deliver the signal twice ([L-091](../planning/known-limits.md#l-091)).

## 12. Problems

Run `vsift setup check --json` and read [`SUPPORT.md`](../../SUPPORT.md) before opening an
issue; remove secrets, personal information and sensitive paths first. Security
concerns follow [`SECURITY.md`](../../SECURITY.md). Maintainers publishing a release use
[`release.md`](release.md).
