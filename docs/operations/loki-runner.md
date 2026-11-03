# LOKI, the overnight test machine: setting up its runner

This is the maintainer's runbook for turning LOKI, the clean Windows 11 test machine, into the
machine that runs VSift's overnight load, stress and reproduction runs, through a GitHub Actions
self-hosted runner with the label `loki`. The policy and its reasons are
[ADR 0025](../decisions/0025-self-hosted-runner-policy.md); the workflow is
`.github/workflows/loki-overnight.yml`; what it leaves at risk is
[L-186](../planning/known-limits.md#l-186).

Everything here is done by the maintainer, at LOKI's console, as an administrator unless a step
says otherwise. Nothing here is done by a session or an agent, and no token, password or account
name is ever pasted into a document, an issue, a chat or a command line that is kept in history.

**Never write a person's name into anything on this machine that a job can see.** A public
repository's job logs are public: they show the machine's name, the runner's name and paths with the
runner user's name in them.

## 0. The first step: the RQ-17 try-out (decided 2026-10-19)

LOKI was set aside for the RQ-17 try-out ([`rq-17-tryout-sheet.md`](../planning/rq-17-tryout-sheet.md)):
Smart App Control on a clean Windows 11, as a new user meets it. **Installing the build tools,
Node.js and Rust makes LOKI look like a developer's machine, and Windows may then switch Smart App
Control off by itself; it cannot be switched back on without reinstalling Windows.**

**Decided by the maintainer on 2026-10-19: the try-out comes first.** Do the RQ-17 try-out on the
clean LOKI with the published `0.2.2`, following the sheet, and hand back the filled sheet before
anything below is installed. Only then go on to section 1.

Once the runner is set up, LOKI is no longer a clean machine: **a later try-out (a new release, or
RQ-17 again) needs a reset of LOKI** (remove the runner first, section 11, then reset Windows and set
the runner up again afterwards) **or another clean Windows 11 machine.**

## 1. The machine's name

```powershell
$env:COMPUTERNAME
```

It must not be a person's name. If it is, rename it (`Rename-Computer -NewName LOKI`, then restart).

## 2. Prerequisites (as an administrator)

Install each machine-wide, from its publisher, and check its version after a new PowerShell window
opens. Use `winget` or the publisher's own installer; both check the publisher's signature.

| Tool | Why | Command | Check |
| --- | --- | --- | --- |
| Git for Windows | `actions/checkout` needs Git | `winget install --exact --id Git.Git` | `git --version` |
| PowerShell 7 | the workflow's steps run in `pwsh` | `winget install --exact --id Microsoft.PowerShell` | `pwsh --version` |
| Node.js 22 (LTS), machine-wide in `C:\Program Files\nodejs` | the job-started hook runs `C:\Program Files\nodejs\node.exe`; the workflow itself pins its own Node.js 22.23.4 | `winget install --exact --id OpenJS.NodeJS.22` | `& 'C:\Program Files\nodejs\node.exe' --version` prints `v22.` and more |
| Visual Studio 2022 Build Tools with the C++ workload | Rust's linker and the Windows SDK on Windows | `winget install --exact --id Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive --wait"` | `Test-Path 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC'` |
| Python 3 (optional) | none of today's suites; the Python tools under `tools/` if a later suite runs one | `winget install --exact --id Python.Python.3.12` | `py --version` |

**Not installed:** FFmpeg, whisper.cpp and the speech model. None of today's suites need them, and a
suite that does stages its own pinned copies inside its run, verified by size and SHA-256, so LOKI
never holds an unpinned copy on its `PATH`.

**Rust is installed for the runner's user, not machine-wide** (step 5): the toolchain of
`rust-toolchain.toml` (1.98.1 today) is installed by the workflow on its first night and reused.

## 3. The runner's user

A dedicated local **standard** user (not an administrator), with no Microsoft account. Its name is
public in every job's log, so it is `vsift-runner`.

```powershell
$password = Read-Host -AsSecureString 'A new password for vsift-runner'
New-LocalUser -Name 'vsift-runner' -Password $password -PasswordNeverExpires -AccountNeverExpires `
  -Description 'GitHub Actions runner for VSift (ADR 0025)'
Add-LocalGroupMember -Group 'Users' -Member 'vsift-runner'
```

Keep the password in your password manager; the runner's service needs it in step 7.

## 4. The runner's folders

The runner's application, its work folder and its hook live under one folder outside every
person's profile. Only administrators and the system can change the top folder and the hook; the
runner's user can change only its application folder (the runner updates itself) and its work
folder.

```powershell
New-Item -ItemType Directory -Force 'C:\vsift-runner\actions-runner', 'C:\vsift-runner\work', 'C:\vsift-runner\hooks'
icacls 'C:\vsift-runner' /inheritance:r /grant:r 'Administrators:(OI)(CI)F' 'SYSTEM:(OI)(CI)F' 'vsift-runner:(RX)'
icacls 'C:\vsift-runner\actions-runner' /grant 'vsift-runner:(OI)(CI)M'
icacls 'C:\vsift-runner\work' /grant 'vsift-runner:(OI)(CI)M'
icacls 'C:\vsift-runner\hooks' /grant 'vsift-runner:(OI)(CI)RX'
```

## 5. Rust for the runner's user

Open a PowerShell window **as `vsift-runner`** (`runas /user:vsift-runner pwsh`, then type its
password), and in it:

```powershell
cd $env:TEMP
Invoke-WebRequest -Uri 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe' -OutFile rustup-init.exe
Invoke-WebRequest -Uri 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe.sha256' -OutFile rustup-init.exe.sha256
(Get-FileHash rustup-init.exe -Algorithm SHA256).Hash.ToLower()
Get-Content rustup-init.exe.sha256
```

The two hashes must be equal (the second file's first word). Then:

```powershell
.\rustup-init.exe -y --profile minimal --default-toolchain none
Remove-Item rustup-init.exe, rustup-init.exe.sha256
```

Close that window. Rust is in `C:\Users\vsift-runner\.cargo` and `.rustup`.

## 6. The runner application, with its checksum verified

Back in an administrator's PowerShell 7. Find the newest release of the runner at
<https://github.com/actions/runner/releases> and set `$version` to it (`2.338.0`, published
2026-10-06, was the newest on 2026-10-19). Each release's notes list a "SHA-256 Checksums" line for
`actions-runner-win-x64-<version>.zip`, and GitHub shows each file's digest beside it.

```powershell
cd 'C:\vsift-runner\actions-runner'
$version = '2.338.0'
$zip = "actions-runner-win-x64-$version.zip"
Invoke-WebRequest -Uri "https://github.com/actions/runner/releases/download/v$version/$zip" -OutFile $zip
(Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
(Get-Item $zip).Length
```

Compare the hash with the release notes' line for `win-x64` **and** with the digest GitHub shows for
the file. For `2.338.0` both are
`f48e0750a21812bca5f82de5f7f5aeae71abee647fab5a582f1742d07eba455f`, and the size is 104,063,355
bytes. If either differs, stop: delete the file and do not go on. If they match:

```powershell
Expand-Archive -Path $zip -DestinationPath .
Remove-Item $zip
```

## 7. Register the runner with the `loki` label and run it as a service

1. On GitHub, in the repository: **Settings, Actions, Runners, New self-hosted runner**, choose
   **Windows** and **x64**. The page shows a registration token (valid for an hour) in its
   `config.cmd` line. You need only the token; the page's download steps are done already.
2. In the administrator's PowerShell, in `C:\vsift-runner\actions-runner`:

   ```powershell
   .\config.cmd --url https://github.com/smormah/vsift --name loki --labels loki `
     --work 'C:\vsift-runner\work' --runasservice --windowslogonaccount '.\vsift-runner'
   ```

   Paste the token when it asks for it, and type `vsift-runner`'s password when it asks for the
   service account's password, so neither ends up in your command history. Accept the default
   runner group. The runner's labels are then `self-hosted`, `Windows`, `X64` and `loki`.
3. The service is `actions.runner.smormah-vsift.loki`, running as `vsift-runner`:

   ```powershell
   Get-Service 'actions.runner.*' | Select-Object Name, Status, StartType
   (Get-CimInstance Win32_Service -Filter "Name like 'actions.runner.%'").StartName
   ```

   The second line must print `.\vsift-runner`. In the repository's **Settings, Actions, Runners**,
   `loki` shows as **Idle**.

## 8. The job-started hook and the runner's paths

The hook refuses, before any step runs, every job that is not `loki-overnight.yml` of
`smormah/vsift` started by its schedule or by a dispatch on a branch (ADR 0025 section 8).

1. Copy `tools/loki/job-guard.cjs` and `tools/loki/job-started.ps1` **from a reviewed commit** of
   `main` (the merge of the change that added them, or later) into `C:\vsift-runner\hooks`. For
   example, open each file on GitHub at that commit, choose **Raw**, and save it there. Check both
   are there and nothing else:

   ```powershell
   Get-ChildItem 'C:\vsift-runner\hooks'
   & 'C:\Program Files\nodejs\node.exe' 'C:\vsift-runner\hooks\job-guard.cjs'
   ```

   The second line must print `LOKI job guard: REFUSED: ...` (it has no job to allow).
2. Edit `C:\vsift-runner\actions-runner\.env` (Notepad, as administrator) and add the line:

   ```text
   ACTIONS_RUNNER_HOOK_JOB_STARTED=C:\vsift-runner\hooks\job-started.ps1
   ```

3. Open `C:\vsift-runner\actions-runner\.path`. It is the `PATH` the runner gives every job, taken
   from the window `config.cmd` ran in. It must name Git, PowerShell 7, Node.js and
   `C:\Users\vsift-runner\.cargo\bin`; add whichever is missing, separated by `;`.
4. Restart the service: `Restart-Service 'actions.runner.smormah-vsift.loki'`.

## 9. The repository settings to check

In the repository's **Settings, Actions, General**:

- **Fork pull request workflows from outside collaborators** (also shown as **Approval for running
  fork pull request workflows from contributors**): choose **Require approval for all outside
  collaborators** (or **all external contributors**). Never approve a run of a pull request that
  changes `.github/workflows/` before reading the change.
- **Workflow permissions:** "Read repository contents and packages permissions". Every workflow
  declares its own scopes anyway.

## 10. The first run

From any machine with `gh` signed in as the maintainer:

```console
gh workflow run loki-overnight.yml --repo smormah/vsift --ref main -f suites=locks -f minutes=10
```

In the run: the **Set up runner** step of `Overnight suites (LOKI)` prints `LOKI job guard: allowed`;
the first night installs the Rust toolchain and builds the tests (expect 20 to 40 minutes); the job
summary lists `locks` with its repetitions; the artifact `loki-overnight-1` holds `summary.md` and
`locks/result.json`. After it, `C:\vsift-runner\work\_temp` has no `vsift-loki-*` folder left.

Tell the supervisor the run's link and anything that failed. From then on the nightly set runs every
night at 02:07 UK time. Every dispatch command, including the `repro` suite for #439 and #443, is in
the comment at the top of the workflow and in [`development.md`](../development.md), "The overnight
lane (LOKI)".

## 11. Pausing and removing

- **Pause for a night or a week:** `Stop-Service 'actions.runner.smormah-vsift.loki'`. A scheduled
  run then queues and is cancelled after a day; its report comments on the open
  `loki-overnight-failure` issue. Start it again with `Start-Service`.
- **Pause for longer:** also `gh workflow disable loki-overnight.yml --repo smormah/vsift`, so
  nothing queues (`gh workflow enable` to resume).
- **Remove:** in **Settings, Actions, Runners**, open `loki`, choose **Remove** and copy the removal
  token it shows; in the administrator's PowerShell in `C:\vsift-runner\actions-runner` run
  `.\config.cmd remove` and paste the token when asked (it stops and deletes the service and
  unregisters the runner). Then delete `C:\vsift-runner`, and the user and its profile:
  `Remove-LocalUser -Name 'vsift-runner'`, then **Settings, Accounts, Other users** or **System,
  Advanced system settings, User profiles** to delete its profile folder. If the runner is still
  listed on GitHub, remove it there with **Force remove**.

## 12. Later: the maintainer's private media on LOKI (a note, not a design)

The maintainer may later keep private recordings on LOKI for their own local testing. That set stays
local and **out of CI**: no workflow reads it, and nothing in the overnight lane is designed to.
When it happens:

- keep it outside `C:\vsift-runner` and outside `vsift-runner`'s profile, on a folder whose access
  list names only the maintainer's own account (and administrators), with **an explicit deny for
  `vsift-runner`** so a later change of inheritance cannot open it;
- check it from the runner user's side: in a `runas /user:vsift-runner pwsh` window,
  `Get-ChildItem <that folder>` must fail with access denied;
- never add a workflow input, an environment variable or a suite that names that folder. The public
  real-media set (#363) is the one that CI uses.
