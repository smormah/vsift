# RQ-17 try-out sheet: Smart App Control and a clean Windows 11 machine

Status: **prepared 2026-10-05 (P14 PR 11b) and moved to the second candidate on 2026-10-06 (P14 PR 10 repeated); nothing on it has been done.** It is for the maintainer, on a second,
clean, wipeable Windows 11 machine (called **LOKI** below; the plan's unknowns table records it, 2026-10-02), with a
person at the console: every installer needs one. It is evidence item **RQ-17** of the
[evidence ledger](p14-evidence-ledger.json), against the published release candidate `0.2.0-rc.2`. The plan is
[`p14-qualification.md`](p14-qualification.md) sections 8, 10 and 14; the decisions are
[ADR 0024](../decisions/0024-r0-qualification-and-release-candidate.md) decisions C and H.

**Never print, type into a document, photograph or paste LOKI's sign-in details, and write no account name, e-mail address or
real machine name in any observation.** Say "LOKI". Crop screenshots to the window they show: a path such as
`C:\Users\<name>\...` carries the user name, so blank it. Nothing in this sheet needs a sign-in, an account or a token.

## What this decides, and what it does not

- **Decision H (accepted 2026-10-02): a try-out blocks the stable release only until an observation is recorded, whatever it
  shows.** A blocked file, a warning, a pass and "could not be done" are all observations. Untried hardware is stated, never
  hidden. Nothing in this sheet can fail it by showing a bad result.
- **Decision C's trigger.** If Smart App Control (or Gatekeeper, which this sheet cannot try) **blocks an npm-installed VSift on
  a default machine with no way through short of turning protection off**, you then choose, before the stable release,
  between a **documented limitation** (`install.md` section 4 already says Smart App Control may block it) and **signing**
  (certificate or signing service fees, key custody, a new secret in the release workflow, new lint rules, about a week).
  Signing changes the release workflow, which is code, so it means another candidate (`0.2.0-rc.3`) and the repeats that
  come with one. The sheet only collects the facts; it decides nothing.
- **What it cannot show.** Another machine's policy (AppLocker, App Control for Business, a corporate proxy), a Mac (no
  Mac is available, so the macOS Gatekeeper try-out ships as "untried", decision H and the plan), or that a user's recording
  works: the practice recording is synthetic.

## Ground rules

1. **Record before you change.** Do not turn Smart App Control, SmartScreen or Defender off, and do not change an execution
   policy, until the observation that needed it is written down. **Smart App Control cannot be turned back on without a clean
   reinstall of Windows**, so turning it off ends the try-out for that installation.
2. **Re-read the Smart App Control state before and after each way in.** In evaluation mode Windows decides by itself, after
   some time and use, whether to turn it On or Off (Microsoft says it turns itself off on machines that look like developers').
   Installing Node.js is the kind of thing that can tip it. Write down every change and the time.
3. **One thing at a time, in a new terminal.** Open a new PowerShell window after installing anything that changes `PATH`.
4. **Use PowerShell, never `cmd.exe`, for anything with text you typed from a document** (L-109). The one exception below
   is a plain `--version` through the `.cmd` shim, and it is marked.
5. **Write what you saw, not what the sheet expects.** An expected line that does not appear is the point of the exercise.
6. LOKI needs Internet access to npm, GitHub and (Part 3) two more publishers. It sends only what those programs send
   (`npm`'s and `curl`'s own client identifiers and your IP address, as any browser would).

## What you need

| Item | Value |
| --- | --- |
| The release | `0.2.0-rc.2`, a pre-release: <https://github.com/smormah/vsift/releases/tag/v0.2.0-rc.2> |
| What `vsift --version` must print | `vsift 0.2.0-rc.2 (<the first 12 digits of the tag's commit, as the release page shows it>)` |
| The Windows archive | `vsift-0.2.0-rc.2-x86_64-pc-windows-msvc.tar.gz`; its size is on the release page, and its checksum is the line for it in the release's `SHA256SUMS` (neither is known before the publish; the first candidate's archive was 3,957,058 bytes) |
| Node.js | version 22 or later; the hosted runs used 22.23.3 with npm 10.9.9 (from <https://nodejs.org>; check the installer against the checksums on that site if you wish) |
| The npm command | `npm install --global vsift-cli@next` (always with `@next`: a plain `vsift-cli` is the empty `0.0.0` placeholder) |
| Part 3 only: the tools | below, with sizes and SHA-256 |
| Part 3 only: the practice recording | `F04-speech.mp4` (140 KB) and `F04-speech.srt`, links in [the guide's first investigation](../guide/first-investigation.md#2-get-the-practice-files) |
| A place to write | print this file, or copy it to a text editor on LOKI; hand the filled copy back (it is not committed as it is) |

## Part 0: Record the machine (about 10 minutes, nothing is installed)

Do this on a freshly installed or freshly reset LOKI, before anything else, and write it down.

```powershell
Get-ComputerInfo -Property OsName,OsVersion,OsBuildNumber,WindowsEditionId | Format-List
(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy').VerifiedAndReputablePolicyState
Get-ExecutionPolicy -List
Get-MpComputerStatus | Select-Object AMServiceEnabled,RealTimeProtectionEnabled,AntivirusSignatureLastUpdated | Format-List
```

The registry value is what the maintainer's own machine was read with on 2026-10-02 (`0` = Off); the community's reading of it
is `0` Off, `1` On, `2` Evaluation. **The Settings page is the authority, not the number:** Settings, Privacy & security,
Windows Security, App & browser control, **Smart App Control settings** (it says On, Evaluation or Off). Also read, on
the same page, **Reputation-based protection settings** (Check apps and files, SmartScreen for Microsoft Edge).

| Fact | What you read |
| --- | --- |
| Date and time (UTC), and who is at the console (a role, not a name) | |
| Windows edition, version and build; physical PC or virtual machine (which product) | |
| Smart App Control state, from Settings | On / Evaluation / Off |
| `VerifiedAndReputablePolicyState` | |
| SmartScreen: Check apps and files | On (Warn / Block) / Off |
| Defender real-time protection | |
| PowerShell execution policy (the `CurrentUser` and `LocalMachine` rows) | |
| Anything else installed (should be nothing but Windows' own and the browser) | |

**If Smart App Control reads Off here, parts 1 and 2 say nothing about it:** write that, and still do everything else (a clean
install and the guide are worth recording on their own). If it reads Evaluation or On, carry on.

## Part 1: A clean install through npm (also the first Smart App Control way in)

This is the path most people take, and the one a clean machine tests: nothing of ours is on LOKI, no Rust, no Git.

1. **Install Node.js** (the person at the console runs its installer). Open a **new** PowerShell and write down:
   `node --version`, `npm --version`, the time taken, any prompt Windows showed (User Account Control is expected for the
   installer), and the Smart App Control state again.

   | Observation | |
   | --- | --- |
   | Versions and anything unexpected | |
   | Smart App Control state after Node.js | |

2. **Install VSift** and time it:

   ```powershell
   npm install --global vsift-cli@next
   ```

   Expected: a few seconds, `added 2 packages` or similar, no warnings about scripts (no package runs one). Write down
   exactly what npm printed if it differs.

3. **Run it.**

   ```powershell
   vsift --version
   ```

   Expected: `vsift 0.2.0-rc.2 (<the first 12 digits of the tag's commit>)`.

   **If PowerShell says that running scripts is disabled** (about `vsift.ps1`): that is Windows' default execution policy on a
   client, and it also affects `npm` itself, so it may have stopped you earlier. Copy the message exactly. Then, **only after
   writing it down**, use one of these and write which one you used: (a) the **one marked exception**: `cmd /c vsift --version`
   (a plain `--version` through the `.cmd` shim, no text from anywhere else); (b) `npx vsift-cli@next --version`; (c) the
   normal user-level fix, `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`, which is a change to LOKI's settings and the
   maintainer's call. A user meets this on a default machine; `install.md` does not mention it yet.

   | Observation | |
   | --- | --- |
   | Exact output of `vsift --version` (or the message and what you did) | |
   | Was there a notification, a Windows Security toast, a SmartScreen window or a Smart App Control message? Exact text; which program it names. | |
   | Smart App Control state now | |

4. **If anything was blocked, read what Windows logged** (read-only), and write down the event ids and the file named:

   ```powershell
   Get-WinEvent -LogName 'Microsoft-Windows-CodeIntegrity/Operational' -MaxEvents 30 |
     Format-List TimeCreated, Id, LevelDisplayName, Message
   $exe = Join-Path (npm root --global) 'vsift-cli\node_modules\@vsift\win32-x64\vsift.exe'
   Get-AuthenticodeSignature $exe | Format-List Status, StatusMessage
   Get-Item $exe -Stream *
   ```

   Expected for the signature: `NotSigned` (VSift's executables are not signed, ADR 0023 decision C). `Get-Item -Stream *` shows only
   `:$DATA` when npm wrote the file (no download mark).

   | Observation | |
   | --- | --- |
   | Code Integrity events (ids, times, file) and the signature status | |

5. **What the program says on a machine with no tools** (this is the `setup` flow's first step):

   ```powershell
   vsift setup check
   $LASTEXITCODE
   vsift --help
   ```

   Expected (the hosted Windows runs gave exit 2): `Status: blocked`, a `[missing]` line each for FFmpeg and FFprobe and one for
   Whisper, each saying to install or locate the tool and that managed installation is not available for this target; `Local ASR model: not
   registered`. Write down whether the words were clear to someone who has never seen VSift, and anything that was not.

   | Observation | |
   | --- | --- |
   | `setup check` output and exit code | |
   | Anything unclear | |

6. **The registry's signatures** (a project folder, as `install.md` section 6 says):

   ```powershell
   mkdir C:\vsift-check; cd C:\vsift-check
   Set-Content package.json '{"private": true}'
   npm install vsift-cli@next
   npm audit signatures
   npx vsift --version
   ```

   Expected: the packages have verified registry signatures and verified attestations (the hosted run printed "2 packages with
   verified registry signatures, 2 with verified attestations"; the 0.1.0 install on a developer machine printed 4 and 4), and
   the version line of step 3.

   | Observation | |
   | --- | --- |
   | `npm audit signatures` output | |

## Part 2: The archive and downloads (the second and third Smart App Control ways in, and SmartScreen)

Use a **fresh folder** for each way, so one result cannot colour the next. Re-read the Smart App Control state between them.

**2a. A browser download** (this marks the file as coming from the Internet).

1. In Microsoft Edge, open the release page and download the Windows archive and `SHA256SUMS`. Write down any warning
   the browser or Windows showed on the download itself.
2. Check the checksum, as `install.md` section 3 step 2 says; the two hashes must be equal, case aside:

   ```powershell
   (Get-FileHash -Algorithm SHA256 .\vsift-0.2.0-rc.2-x86_64-pc-windows-msvc.tar.gz).Hash
   Select-String 'x86_64-pc-windows-msvc' .\SHA256SUMS
   ```

3. Look at the download mark, then extract with the **Windows** `tar` (not Git's):

   ```powershell
   Get-Item .\vsift-0.2.0-rc.2-x86_64-pc-windows-msvc.tar.gz -Stream *
   Get-Content .\vsift-0.2.0-rc.2-x86_64-pc-windows-msvc.tar.gz -Stream Zone.Identifier
   C:\Windows\System32\tar.exe -xzf .\vsift-0.2.0-rc.2-x86_64-pc-windows-msvc.tar.gz
   Get-Item .\vsift-0.2.0-rc.2-x86_64-pc-windows-msvc\vsift.exe -Stream *
   ```

   Expected: the archive has a `Zone.Identifier` stream (`ZoneId=3`). **Whether the extracted `vsift.exe` has one depends on
   the extractor** (`install.md` section 4 says so): write down what you see.
4. Run it, from PowerShell:

   ```powershell
   .\vsift-0.2.0-rc.2-x86_64-pc-windows-msvc\vsift.exe --version
   ```

   Expected if nothing blocks it: `vsift 0.2.0-rc.2 (<the first 12 digits of the tag's commit>)`. **If a window appears** ("Windows protected your PC", or a
   Smart App Control message): write the exact title and text, whether **More info** and **Run anyway** exist, and what
   happens when you use them; photograph or screenshot it. `install.md` section 4 says SmartScreen offers **More info**, then
   **Run anyway**, and that Smart App Control offers no way through; this is where that is seen for the first time.
5. Only if SmartScreen (not Smart App Control) stopped it: `Unblock-File .\...\vsift.exe`, run again, write down the result.

**2b. A command-line download** (no Internet mark is expected): in a new folder,

```powershell
curl.exe -L -O https://github.com/smormah/vsift/releases/download/v0.2.0-rc.2/vsift-0.2.0-rc.2-x86_64-pc-windows-msvc.tar.gz
Get-Item .\vsift-0.2.0-rc.2-x86_64-pc-windows-msvc.tar.gz -Stream *
C:\Windows\System32\tar.exe -xzf .\vsift-0.2.0-rc.2-x86_64-pc-windows-msvc.tar.gz
.\vsift-0.2.0-rc.2-x86_64-pc-windows-msvc\vsift.exe --version
```

Check the checksum as in 2a step 2 (the file is the same). Write down whether the run differed from 2a. If it did not,
the mark is not what blocks or allows the file, which is what the sentence "Installing through npm avoids the download mark"
in `install.md` section 4 leaves open for Smart App Control.

| Observation | 2a browser download | 2b command-line download |
| --- | --- | --- |
| Warning at the download | | |
| Checksum equal | | |
| Streams on the archive and on `vsift.exe` | | |
| What running it did (exact words; buttons; time) | | |
| Smart App Control state after | | |

## Part 3: A first investigation, and the `setup` flows

Only if Part 1 or 2 let `vsift.exe` run. **The tools below are unsigned third-party executables**, and Smart App Control, if it
is On, may block them as well: write down **which file** was blocked, `vsift.exe`, `ffmpeg.exe`, `ffprobe.exe` or
`whisper-cli.exe`. If a tool is blocked, stop Part 3 there and record that.

The three tools are the reviewed Windows builds the hosted runs used (`install.md` section 5.2: VSift never installs them on
Windows). Download each, check size and SHA-256 before you unpack it, and put them in `C:\vsift-tools\`:

| Tool | Where | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| FFmpeg and FFprobe (BtbN, LGPL, n9.0.1-11, 2026-08-31) | <https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-08-31-13-27/ffmpeg-n9.0.1-11-ge47273f4d9-win64-lgpl-9.0.zip> | 147,007,942 | `2484854ad6988d34560f4e6ea7a6ecb9dde0af7c229d2591815d056b04ec4f56` |
| whisper.cpp v1.9.2 (`whisper-cli.exe`) | <https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-x64.zip> | 8,194,445 | `49dcc16de826f20bd53d44f947a1ae49dfa81f86cad67a64d80820cb192d674a` |
| The `base` model, `ggml-base.bin` | <https://huggingface.co/ggerganov/whisper.cpp/resolve/80da2d8bfee42b0e836fc3a9890373e5defc00a6/ggml-base.bin> | 147,951,465 | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` |

**The `setup` flow** (each step's expected answer is from the guide; write down what you got):

```powershell
vsift setup configure ffmpeg --executable "C:\vsift-tools\ffmpeg\bin\ffmpeg.exe"
vsift setup configure ffprobe --executable "C:\vsift-tools\ffmpeg\bin\ffprobe.exe"
vsift setup check           # expected: Status: degraded (FFmpeg and FFprobe ok, Whisper missing)
vsift setup configure whisper --executable "C:\vsift-tools\whisper\whisper-cli.exe"
vsift setup configure-model --file "C:\vsift-tools\ggml-base.bin"
vsift setup check           # expected: Status: ready, and a Local ASR check line that passes
vsift setup plan --profile desktop   # not predicted on Windows: the guide says managed installation is Ubuntu-only; write the exact words
vsift setup list
```

(Adjust the folders to what the two archives unpack to: the point is that each path is absolute.)

**The first investigation**, the commands of [the guide's first investigation](../guide/first-investigation.md) steps 3 to 9
on `F04-speech.mp4` and `F04-speech.srt`, in a new folder. Identifiers and times differ every run; the shape and these
values do not:

| Step | Expected |
| --- | --- |
| `vsift ingest .\F04-speech.mp4 --transcript .\F04-speech.srt` | `Opened session ses_...`, `Transcript: revision 1 ... (imported_srt ... segments: 2)` |
| `vsift transcript get <session> --from 0 --to 14000000` | two segments: "Scroll to order 1017." (0.775 to 2.850 s) and the status change from queued to failed with the header fixed (2.975 to 6.950 s) |
| `vsift search <session> --query "order 1017"` | one hit, `match: phrase` |
| `vsift candidates <session> --from 0 --to 14000000` | three: `first_frame` at 0, `visual_change` at 7.0 s and at 10.0 s, 1440x900 |
| `vsift frame get <session> --at 10000000` | a PNG, 1440x900, that you open: order 1017 with status FAILED in red under a header that did not move |
| `vsift session close <session>` | `State: closed` |
| the listening route: `vsift ingest .\F04-speech.mp4`, then `vsift transcript retranscribe <session>` | `local_asr`, `Segments recognised now: 1`; seconds, silent until it ends |

Also write down the time of the two slow steps (the recognition and the first `frame get`), any message that mentioned a
path or a name of LOKI, and whether a person who has never seen VSift could have followed the guide from Part 1 to here
without the supervisor's help: the first place they would have stopped.

| Observation | |
| --- | --- |
| `setup check` after each configure (status words) | |
| `setup plan`, `setup list` (exact words) | |
| The investigation: each step matched / differed (what) | |
| Times; the first place a newcomer would stop | |

## Part 4: Take it away again (optional, about 5 minutes)

`install.md` section 8 on a real machine:

```powershell
npm uninstall --global vsift-cli
Get-ChildItem "$env:LOCALAPPDATA\vsift", "$env:LOCALAPPDATA\VSift-sessions" -ErrorAction SilentlyContinue
```

The two folders are VSift's own (configuration and sessions); deleting them removes every trace. Write down whether the guide's
table matched what was there, and that the practice files and the registered tools stayed.

## What to hand back, and what happens to it

Hand the filled copy (observations, the exact texts, cropped screenshots without names) to the supervisor. They are recorded as
**RQ-17**: one entry per way in (npm, browser archive, command-line archive) with the Smart App Control state **at the time**,
the exact text, and the date; `install.md` section 4 and `known-limits.md` L-098 are then rewritten from what was seen, not
from Microsoft's and Apple's documentation, and the plan's Windows row says what was seen. The outcomes:

| What you saw | What follows |
| --- | --- |
| Smart App Control was Off all along | Recorded as that: it says nothing about On. Say so in the matrix ("untried") and decide whether to find a machine where it is On |
| Smart App Control On or in evaluation and **nothing blocked** | Recorded; `install.md` section 4's "expects to be blocked" is rewritten to what was observed |
| Smart App Control **blocked** `vsift.exe` by every way in, with no way through short of turning it off | **Decision C's trigger.** Stop; tell the supervisor; you choose between the documented limitation and signing (and so a second candidate) |
| A SmartScreen warning with **Run anyway** | Matches `install.md`; recorded with the exact text |
| A SmartScreen or Defender stop with no way through, or a detection naming the file | Tell the supervisor at once; do not use a workaround; recorded exactly, and not read as a verdict on the file until it has been looked at |
| An execution-policy refusal of the npm `vsift.ps1` shim | A finding for `install.md` section 2 (a documentation change, which is allowed before the stable release) |
| Something not on this list | Recorded as seen |

**What this sheet does not cover:** macOS (no Mac; Gatekeeper ships as untried), another user's policy, a second Windows
machine with Smart App Control in the other state, the agent trials (their own checklist,
[`p14-batch-2-3-checklist.md`](p14-batch-2-3-checklist.md)) and the register pass.
