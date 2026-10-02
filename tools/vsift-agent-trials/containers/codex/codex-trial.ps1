<#
.SYNOPSIS
    Builds the Codex trial images and runs Codex trials in them (P12; P14 adds
    the images that install VSift from the real npm registry).

.DESCRIPTION
    Operator wrapper for docs/agents/trials.md, "Codex trials in a Linux
    container". Every docker argument is an element of an array passed to
    docker directly: no command string is ever built, and each value the
    operator gives is checked against a fixed pattern first.

    A trial is three containers, so the agent never shares one with the
    repository or with another trial:
      1. prepare  (harness image: repository inside) writes the trial;
      2. run      (agent image: built tools only) runs Codex, with only this
                  trial's folder, the model and the sign-in mounted;
      3. grade    (harness image) grades, records and exports afterwards.

    Two families of images exist:
      * the P12 images (default): VSift and the harness are built from the
        checkout; the reviewed FFmpeg, whisper.cpp and the model are
        registered by prepare;
      * the P14 clean-install images (-Published -PublishedVersion <exact
        version>): VSift is installed from the REAL npm registry at build
        time and its proof (the registry's integrity, the launcher's digest
        check, the version line) goes into every trial record; prepare plays
        the user and runs `setup plan` and `setup install` for the managed
        tools; no model file is mounted; the agent image holds no FFmpeg, no
        whisper.cpp and no readable skill or README. Cold scenarios (C-*)
        and hold-outs (H-*) are found by their ids.

    No action here calls a model except trial, continue and debug, which are
    the maintainer's runs; this script is never run by a test or by CI.

    Actions:
      build          docker build of both images at the checkout's HEAD.
      versions       tool versions and digests in the agent image.
      sandbox-check  Codex's Linux sandbox without a model call.
      debug          one debug run (never a trial) with -Prompt, prepared from
                     -Scenario (default A-08-f05-local-asr).
      trial          one trial of -Scenario (its first phase). Its last line
                     of output is "trial-id <trial>", the folder name to pass
                     to continue -Trial and to find the exported records.
                     Exits 75 (after printing "usage-limit") when Codex
                     stopped at its usage limit: the phase is invalid, run
                     it again after the limit lifts.
      continue       phase -Phase of the prepared trial -Trial (A-02's second
                     phase gets only the first phase's resume card).
      regrade        grade a finished trial again, from its raw logs.

.EXAMPLE
    .\codex-trial.ps1 build
    .\codex-trial.ps1 sandbox-check
    .\codex-trial.ps1 trial -Scenario A-08-f05-local-asr -Model gpt-6-astra
    .\codex-trial.ps1 build -Published -PublishedVersion 0.1.0
    .\codex-trial.ps1 trial -Published -PublishedVersion 0.1.0 -Scenario C-03-f03-missing-tools -Model gpt-6-sol
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateSet('build', 'versions', 'sandbox-check', 'debug', 'trial', 'continue', 'regrade')]
    [string] $Action,

    # For regrade: the new grade's file name, beside the original grade.
    [ValidatePattern('^grade-[a-z0-9-]{1,32}\.json$')]
    [string] $Output,

    [ValidatePattern('^[A-Za-z0-9-]{1,64}$')]
    [string] $Scenario,

    [ValidatePattern('^[A-Za-z0-9._-]{1,64}$')]
    [string] $Model,

    [ValidatePattern('^[a-z0-9][a-z0-9-]{0,40}$')]
    [string] $Name,

    [ValidateLength(1, 4000)]
    [string] $Prompt,

    [ValidateRange(10, 7200)]
    [int] $TimeoutSeconds = 1500,

    # For continue: the trial folder prepare reported, and the phase.
    [ValidatePattern('^[a-z0-9][a-z0-9-]{0,80}$')]
    [string] $Trial,

    [ValidateRange(2, 9)]
    [int] $Phase = 2,

    # The image tag suffix; default: the short HEAD commit (and, for the
    # published images, the version).
    [ValidatePattern('^[A-Za-z0-9._-]{1,64}$')]
    [string] $Tag,

    # P14: use the clean-install images, which install VSift from the real
    # npm registry, and the exact version they install (never a tag).
    [switch] $Published,

    [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$')]
    [string] $PublishedVersion,

    # P14: a freeze file (vsift-agent-trials freeze write) that prepare checks
    # against the repository inside the harness image before it prepares.
    [string] $Freeze,

    # The Docker volume that holds the trial root (/trials). A volume, not a
    # Windows folder: VSift checks that its private folders are owned by the
    # user with mode 0700, which a Windows bind mount cannot represent.
    [ValidatePattern('^[a-z0-9][a-z0-9_.-]{0,63}$')]
    [string] $TrialVolume = 'vsift-codex-trials',

    # Where records and the harness folders (raw logs) come out (/exports).
    [string] $Exports = 'C:\vsift-trials\linux',

    # The signed-in Codex trial home; only its auth.json is mounted, and only
    # into the run container.
    [string] $ClientHome = 'C:\vsift-trials\.clients\codex',

    # The reviewed ggml-base.bin, mounted read-only and checked by digest
    # (the P12 images only; a published image installs its own model).
    [string] $ModelFile = 'C:\tools\whisper.cpp\models\ggml-base.bin'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandArgumentPassing = 'Standard'

$here = $PSScriptRoot
$repository = (Resolve-Path (Join-Path $here '..\..\..\..')).Path
$seccomp = Join-Path $here 'seccomp-userns.json'

# The exit status of the harness's `run` when the client stopped at its usage
# limit (EX_TEMPFAIL).
$usageLimitExit = 75

function Invoke-Docker([string[]] $Arguments) {
    & docker @Arguments
    if ($LASTEXITCODE -ne 0) { throw "docker exited with $LASTEXITCODE" }
}

function Get-Head {
    $sha = (& git -C $repository rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $sha -notmatch '^[0-9a-f]{40}$') { throw 'git rev-parse HEAD failed' }
    return $sha
}

function Assert-NeutralPath([string] $Path, [string] $What) {
    $user = [Environment]::UserName
    if ($Path -like "*$user*" -or $Path -like "$env:USERPROFILE*") {
        throw "$What must be a neutral path outside the user profile and without the user name"
    }
}

if ($Published -and -not $PublishedVersion) { throw '-Published needs -PublishedVersion (an exact version such as 0.1.0, never a tag)' }
if ($PublishedVersion -and -not $Published) { throw '-PublishedVersion needs -Published' }
if ($Freeze -and $Action -ne 'trial' -and $Action -ne 'debug') { throw '-Freeze is for trial and debug' }

if (-not $Tag) {
    $Tag = (Get-Head).Substring(0, 12)
    if ($Published) { $Tag = "$Tag-$PublishedVersion" }
}
if ($Published) {
    $agentImage = "vsift-codex-trials-agent-published:$Tag"
    $harnessImage = "vsift-codex-trials-harness-published:$Tag"
    $agentTarget = 'agent-published'
    $harnessTarget = 'harness-published'
}
else {
    $agentImage = "vsift-codex-trials-agent:$Tag"
    $harnessImage = "vsift-codex-trials-harness:$Tag"
    $agentTarget = 'agent'
    $harnessTarget = 'harness'
}

if ($Action -eq 'build') {
    $commit = Get-Head
    if (& git -C $repository status --porcelain) {
        Write-Warning 'The checkout has uncommitted changes: the images are not exactly HEAD.'
    }
    foreach ($target in @(@($agentTarget, $agentImage), @($harnessTarget, $harnessImage))) {
        $buildArguments = @(
            'build',
            '--file', (Join-Path $here 'Dockerfile'),
            '--build-arg', "VSIFT_COMMIT=$commit",
            '--target', $target[0],
            '--tag', $target[1]
        )
        # The published images install the exact version from the real npm
        # registry during the build; the build fails if it is not the published
        # package (the harness's `install` step checks the registry's
        # integrity, the launcher's digest and the version line).
        if ($Published) { $buildArguments += @('--build-arg', "VSIFT_PUBLISHED_VERSION=$PublishedVersion") }
        $buildArguments += $repository
        Invoke-Docker $buildArguments
    }
    foreach ($image in @($agentImage, $harnessImage)) {
        $id = (& docker image inspect --format '{{.Id}}' $image).Trim()
        Write-Output "$image $id"
    }
    Write-Output "vsift commit $commit"
    if ($Published) { Write-Output "vsift-cli $PublishedVersion installed from the registry" }
    exit 0
}

Assert-NeutralPath $Exports 'The exports folder'
Assert-NeutralPath $ClientHome 'The Codex client home'
New-Item -ItemType Directory -Force -Path $Exports | Out-Null
if (-not $Published -and -not (Test-Path -LiteralPath $ModelFile -PathType Leaf)) { throw "No model file at $ModelFile" }

# Options every container gets: unprivileged, no capabilities, read-only
# root, and the committed seccomp profile that lets Codex's bubblewrap
# create its user namespace (Docker's builtin profile refuses it).
$common = @(
    'run', '--rm', '--init',
    '--hostname', 'vsift-trials',
    '--user', '10001:10001',
    '--cap-drop', 'ALL',
    '--security-opt', 'no-new-privileges',
    '--security-opt', "seccomp=$seccomp",
    '--read-only',
    '--tmpfs', '/tmp:rw,nosuid,nodev,size=2g,mode=1777',
    '--pids-limit', '1024',
    '--memory', '8g'
)
# The reviewed model is mounted for the P12 images only: a published image's
# prepare installs the managed model with `setup install`.
if (-not $Published) {
    $common += @('--mount', "type=bind,source=$ModelFile,target=/opt/models/ggml-base.bin,readonly")
}

function Get-TrialMount([string] $Relative) {
    # Only this trial's folder, at the path prepare recorded.
    return @('--mount', "type=volume,source=$TrialVolume,target=/trials/$Relative,volume-subpath=$Relative")
}

function Invoke-Prepare([string[]] $DriverArguments) {
    $arguments = $common + @('--mount', "type=volume,source=$TrialVolume,target=/trials")
    if ($Freeze) {
        if (-not (Test-Path -LiteralPath $Freeze -PathType Leaf)) { throw "No freeze file at $Freeze" }
        $arguments += @('--mount', "type=bind,source=$Freeze,target=/run/freeze.json,readonly")
        $DriverArguments += @('--freeze', '/run/freeze.json')
    }
    $arguments += @($harnessImage, 'prepare') + $DriverArguments
    $output = & docker @arguments
    if ($LASTEXITCODE -ne 0) { throw "prepare failed (docker exited with $LASTEXITCODE)" }
    $output | ForEach-Object { Write-Host $_ }
    $line = @($output | Where-Object { $_ -match '^trial (\S+)$' })
    if ($line.Count -ne 1) { throw 'prepare did not report one trial' }
    $relative = $line[0].Substring(6)
    if ($relative -notmatch '^(debug-[a-z0-9][a-z0-9-]{0,40}/)?[a-z0-9][a-z0-9-]{0,80}$') { throw 'unexpected trial directory' }
    return $relative
}

function Invoke-Run([string] $Relative, [int] $RunPhase, [string[]] $Extra) {
    $auth = Join-Path $ClientHome 'auth.json'
    if (-not (Test-Path -LiteralPath $auth -PathType Leaf)) { throw "No Codex sign-in at $auth; run codex login with CODEX_HOME=$ClientHome" }
    $arguments = $common + @(
        '--tmpfs', '/run/codex-home:rw,nosuid,nodev,size=64m,mode=0700,uid=10001,gid=10001',
        '--mount', "type=bind,source=$auth,target=/run/codex-auth/auth.json,readonly"
    ) + (Get-TrialMount $Relative) + @(
        $agentImage, 'run', '--trial', $Relative, '--phase', "$RunPhase", '--model', $Model,
        '--timeout-s', "$TimeoutSeconds"
    ) + $Extra
    & docker @arguments
    if ($LASTEXITCODE -eq $usageLimitExit) {
        # The client stopped at its usage limit: the phase is invalid, never
        # counted. The campaign script waits and runs the scenario again.
        Write-Output 'usage-limit'
        exit $usageLimitExit
    }
    if ($LASTEXITCODE -ne 0) { throw "docker exited with $LASTEXITCODE" }
}

function Invoke-Grade([string] $Relative, [int] $GradePhase) {
    $arguments = $common + @('--mount', "type=bind,source=$Exports,target=/exports") +
        (Get-TrialMount $Relative) + @($harnessImage, 'grade', '--trial', $Relative, '--phase', "$GradePhase")
    Invoke-Docker $arguments
}

switch ($Action) {
    'versions' { Invoke-Docker ($common + @($agentImage, 'versions')) }
    'sandbox-check' {
        Invoke-Docker ($common + @('--mount', "type=volume,source=$TrialVolume,target=/trials", $agentImage, 'sandbox-check'))
    }
    'debug' {
        if (-not $Name -or -not $Model -or -not $Prompt) { throw 'debug needs -Name, -Model and -Prompt' }
        # A debug run may use another scenario's preparation (for example the
        # images-disabled one, to check that Codex cannot view an image).
        $debugScenario = if ($Scenario) { $Scenario } else { 'A-08-f05-local-asr' }
        $relative = Invoke-Prepare @('--scenario', $debugScenario, '--debug', $Name)
        Invoke-Run $relative 1 @('--debug-prompt', $Prompt)
        Invoke-Grade $relative 1
        Write-Output "trial-id $relative"
    }
    'trial' {
        if (-not $Scenario -or -not $Model) { throw 'trial needs -Scenario and -Model' }
        $relative = Invoke-Prepare @('--scenario', $Scenario)
        Invoke-Run $relative 1 @()
        Invoke-Grade $relative 1
        # The one machine-readable line an operator's loop captures.
        Write-Output "trial-id $relative"
    }
    'continue' {
        if (-not $Trial -or -not $Model) { throw 'continue needs -Trial and -Model' }
        Invoke-Run $Trial $Phase @()
        Invoke-Grade $Trial $Phase
        Write-Output "trial-id $Trial"
    }
    'regrade' {
        # Grading reads only the raw logs and records; no model is called.
        if (-not $Trial -or -not $Output) { throw 'regrade needs -Trial and -Output' }
        $gradePhase = if ($PSBoundParameters.ContainsKey('Phase')) { $Phase } else { 1 }
        $arguments = $common + @('--mount', "type=bind,source=$Exports,target=/exports") +
            (Get-TrialMount $Trial) + @($harnessImage, 'regrade', '--trial', $Trial, '--phase', "$gradePhase", '--output', $Output)
        Invoke-Docker $arguments
    }
}
