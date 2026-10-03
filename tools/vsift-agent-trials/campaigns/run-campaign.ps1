<#
.SYNOPSIS
    Runs one client's share of one P14 agent-trial batch: resumable, usage
    limit aware, stoppable, and writing only bounded records.

.DESCRIPTION
    THIS SCRIPT SPENDS THE MAINTAINER'S CLAUDE OR CODEX ALLOWANCE. It is never
    run by a test, by CI or by an agent. The maintainer starts each batch with
    an explicit go (docs/agents/trials.md, "The P14 batches"); until then
    nothing here has run.

    What it does, in order:
      1. reads the campaign configuration (a JSON file of local paths kept
         outside the repository: campaign.example.json is the template), and
         refuses a checkout with uncommitted changes, before it writes
         anything: the records name a committed state. The campaign's own
         output (docs/planning/p14-agent-trials) is not a change, so a state
         file written earlier, or by a run that was stopped, does not count.
         A dry run does the same check;
      2. builds the harness (cargo build --release) and checks the pinned
         client version;
      3. provides the published install: for Claude Code, `vsift-agent-trials
         install` (npm install --global vsift-cli@<exact version> into a fresh
         prefix under the neutral root, with the registry's integrity and the
         launcher's digest check recorded) or `verify-install` when it exists;
         for Codex, the clean-install images (codex-trial.ps1 build -Published);
      4. writes (first run) and checks (every run) the freeze: the skill, the
         grader, the scenarios, the settings and the corpus truth, by digest;
         a change after the batch started refuses to continue, because it
         voids the batch;
      5. asks `campaign next` for the next run of the batch's plan, runs it
         (prepare, run, grade, record), and tells `campaign mark` how it
         ended. A trial the client's own configuration report invalidated, a
         harness error and a usage-limited phase are not counted and the run
         stays next; three invalid or errored attempts block the run and stop
         the campaign for the maintainer to decide;
      6. when the client stops at its usage limit, waits until the reset time
         the client gave (or -WaitMinutes) and tries the same run again, up to
         -MaxWaitHours in all;
      7. writes summary.json and SUMMARY.md for the batch after every counted
         run, so a pause loses nothing.

    To pause: create the stop file (default <root>\STOP-CAMPAIGN). The script
    finishes the run it is in and stops; delete the file and start it again to
    resume. Nothing is lost: the state file and the records are the memory.

    Every command is started with an explicit executable and an argument
    array. No value read from a file, a scenario or a client's output is ever
    placed in a command string.

.PARAMETER Batch
    1: pilots and the cold baseline, against the published 0.1.0.
    2: the counted set with the skill, on the candidate.
    3: the cold final round, on the candidate.

.PARAMETER Client
    claude (Claude Code on this Windows machine) or codex (the Linux
    container on Docker Desktop).

.PARAMETER Version
    The exact published version under test (0.1.0, 0.2.0-rc.1): never a tag.

.EXAMPLE
    .\run-campaign.ps1 -Batch 1 -Client claude -Version 0.1.0 -Config C:\vsift-trials\campaign.json -DryRun
    .\run-campaign.ps1 -Batch 1 -Client claude -Version 0.1.0 -Config C:\vsift-trials\campaign.json
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateRange(1, 3)]
    [int] $Batch,

    [Parameter(Mandatory = $true)]
    [ValidateSet('claude', 'codex')]
    [string] $Client,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$')]
    [string] $Version,

    [Parameter(Mandatory = $true)]
    [string] $Config,

    # Where the batch's state, freeze, records and summary go (default:
    # docs/planning/p14-agent-trials/batch-<n> in this checkout).
    [string] $BatchDirectory,

    # The stop file (default <root>\STOP-CAMPAIGN).
    [string] $StopFile,

    # Stop after this many runs (0: no limit), for a first look.
    [ValidateRange(0, 200)]
    [int] $MaxRuns = 0,

    # Total time to wait for usage limits to lift before giving up.
    [ValidateRange(1, 72)]
    [int] $MaxWaitHours = 12,

    # How long to wait when the client does not say when its limit lifts.
    [ValidateRange(1, 600)]
    [int] $WaitMinutes = 30,

    # The cold final round must use the baseline's grader, cold scenarios,
    # settings and truth; this allows a change anyway (the baseline is then
    # not comparable, and the summary must say so).
    [switch] $AllowGraderChange,

    # The Claude Code settings of the cold runs: 'strict' (vsift alone, the
    # default and the only one for Claude Code on the maintainer's machine) or
    # 'realistic' (also ls, cat, head, tail, pwd, cd, wc, echo, sort). A rule
    # cannot confine those helpers to the workspace, so the realistic setting
    # lets the agent read any file the user can read: it is refused unless
    # -IsolatedMachine says this machine holds none of the user's own files.
    # Codex always runs the realistic setting, inside the Linux container.
    [ValidateSet('strict', 'realistic')]
    [string] $ColdVariant = 'strict',

    # States that this machine is isolated (a clean test machine, nothing of
    # the user's on it). The only thing that allows -ColdVariant realistic
    # for Claude Code.
    [switch] $IsolatedMachine,

    [switch] $NoBuild,

    # Print the plan and what would run; call no client, no npm and no docker.
    # The checkout is checked exactly as in a real run, so a dry run that
    # passes is not hiding a refusal.
    [switch] $DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandArgumentPassing = 'Standard'

# --- the cold setting --------------------------------------------------------
# Refused before anything is read or written. The strict Claude setting keeps
# the agent to vsift on the maintainer's machine; the realistic one is for an
# isolated machine only, and Codex has no strict variant to ask for.
if ($Client -eq 'claude' -and $ColdVariant -eq 'realistic' -and -not $IsolatedMachine) {
    throw 'The realistic cold setting for Claude Code lets the agent read any file the user can read (ls, cat and head cannot be confined to the workspace), so it runs only on an isolated machine. Say so with -IsolatedMachine, or keep the default -ColdVariant strict.'
}
if ($Client -eq 'codex' -and $PSBoundParameters.ContainsKey('ColdVariant') -and $ColdVariant -ne 'realistic') {
    throw 'The Codex cold run is always the realistic setting, inside the Linux container whose sandbox is its only restriction; there is no strict Codex variant. Leave -ColdVariant out.'
}
if ($IsolatedMachine -and -not ($Client -eq 'claude' -and $ColdVariant -eq 'realistic')) {
    throw '-IsolatedMachine only goes with -Client claude -ColdVariant realistic: it is the statement that allows the realistic Claude setting.'
}

$here = $PSScriptRoot
$repository = (Resolve-Path (Join-Path $here '..\..\..')).Path
$configuration = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
$root = $configuration.root
if (-not $BatchDirectory) { $BatchDirectory = Join-Path $repository "docs\planning\p14-agent-trials\batch-$Batch" }
if (-not $StopFile) { $StopFile = Join-Path $root 'STOP-CAMPAIGN' }
$records = Join-Path $BatchDirectory 'records'
$state = Join-Path $BatchDirectory "state-$Client.json"
$freezeFile = Join-Path $BatchDirectory 'freeze.json'
$harnessExe = Join-Path $repository 'target\release\vsift-agent-trials.exe'
$usageLimitExit = 75

function Write-Step([string] $Text) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $Text) }

# The neutral-root rule of the harness, checked early so a mistake costs nothing.
function Assert-NeutralPath([string] $Path, [string] $What) {
    $user = [Environment]::UserName
    if ($Path -like "*$user*" -or $Path -like "$env:USERPROFILE*" -or $Path -like "$env:TEMP*") {
        throw "$What must be a neutral path outside the user profile and the temporary folder, without the user name"
    }
}
Assert-NeutralPath $root 'The trial root'

function Invoke-Harness([string[]] $Arguments) {
    $output = & $harnessExe @Arguments
    return [pscustomobject]@{ Code = $LASTEXITCODE; Output = @($output) }
}

function Get-HeadCommit {
    $sha = (& git -C $repository rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $sha -notmatch '^[0-9a-f]{40}$') { throw 'git rev-parse HEAD failed' }
    return $sha
}

function Find-Scenario([string] $Id) {
    foreach ($folder in @('scenarios', 'cold', 'holdout')) {
        $path = Join-Path $repository "tools\vsift-agent-trials\$folder\$Id.json"
        if (Test-Path -LiteralPath $path) { return $path }
    }
    throw "No scenario file for $Id"
}

function Test-Stop {
    if (Test-Path -LiteralPath $StopFile) {
        Write-Step "The stop file exists ($StopFile): stopping after the current run. Delete it and start again to resume."
        return $true
    }
    return $false
}

# --- the checkout ------------------------------------------------------------
# The campaign's own output tree: the batch directories hold the state files,
# the freeze, the records and the summaries this script writes (#273). They are
# the result of the work, not the code under test, so they never count as an
# uncommitted change; anything else does. The check runs before anything is
# written and in a dry run too, so the dry run cannot hide a refusal.
$campaignOutput = 'docs/planning/p14-agent-trials'

function Test-PathUnder([string] $Path, [string] $Parent) {
    $prefix = $Parent.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    return $Path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
}

function Assert-CleanCheckout {
    # A batch directory inside the checkout but outside the campaign's output
    # tree would be written by this script and then be read as a change, or
    # would have to be exempted and hide a real change.
    $batchFull = [IO.Path]::GetFullPath($BatchDirectory)
    $outputFull = [IO.Path]::GetFullPath((Join-Path $repository $campaignOutput))
    if ((Test-PathUnder $batchFull $repository) -and -not (Test-PathUnder $batchFull $outputFull)) {
        throw "-BatchDirectory is inside the checkout but not under ${campaignOutput}: use the default, or a folder outside the checkout"
    }
    $changes = @(& git -C $repository status --porcelain -- . ":(exclude)$campaignOutput")
    if ($LASTEXITCODE -ne 0) { throw 'git status failed: the checkout cannot be read, so it cannot be shown to be clean' }
    if ($changes.Count -gt 0) {
        $shown = ($changes | Select-Object -First 10) -join [Environment]::NewLine
        throw ("The checkout has uncommitted changes outside $campaignOutput (the campaign's own output): a batch runs at a committed state, which the records name. Commit or stash them first:" + [Environment]::NewLine + $shown)
    }
}
Assert-CleanCheckout

# --- the harness -------------------------------------------------------------
if (-not $NoBuild) {
    Write-Step 'Building the harness'
    & cargo build --release --locked -p vsift-agent-trials --manifest-path (Join-Path $repository 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
}
if (-not (Test-Path -LiteralPath $harnessExe)) { throw "No harness at $harnessExe (build it, or drop -NoBuild)" }

# --- the plan ----------------------------------------------------------------
New-Item -ItemType Directory -Force -Path $records | Out-Null
if (-not (Test-Path -LiteralPath $state)) {
    $created = Invoke-Harness @('campaign', 'init', '--batch', "$Batch", '--client', $Client, '--version', $Version, '--state', $state)
    if ($created.Code -ne 0) { throw "campaign init failed: $($created.Output)" }
    Write-Step $created.Output[0]
}
else {
    $recorded = (Get-Content -LiteralPath $state -Raw | ConvertFrom-Json).version
    if ($recorded -ne $Version) { throw "The state file is for version $recorded, not ${Version}: a batch is run against one version" }
}

if ($DryRun) {
    Write-Step 'Dry run: the plan below is what would run, in order. No client, npm or docker is called.'
    Write-Step ('Cold setting: ' + $(if ($Client -eq 'codex') { 'realistic (the container)' } else { $ColdVariant }))
    $plan = Get-Content -LiteralPath $state -Raw | ConvertFrom-Json
    $plan.runs | ForEach-Object {
        '{0,-8} {1,-52} {2,-18} {3}' -f $_.status, $_.run.run_id, $_.run.model, $_.run.scenario
    }
    exit 0
}

# --- the client and its published install ----------------------------------
$commit = Get-HeadCommit
if ($Client -eq 'claude') {
    $claude = $configuration.claudeExecutable
    $versionLine = (& $claude --version) -join ' '
    if ($versionLine -notlike "*$($configuration.claudeVersion)*") {
        throw "Claude Code is '$versionLine', the campaign pins $($configuration.claudeVersion): a changed client voids the comparison"
    }
    Write-Step "Claude Code: $versionLine"
    $env:CLAUDE_CODE_GIT_BASH_PATH = $configuration.gitBashPath
    $proof = Join-Path $root ".installs\$Version.proof.json"
    if (-not (Test-Path -LiteralPath $proof)) {
        Write-Step "Installing vsift-cli@$Version from the registry into a fresh prefix (scripts off; a cleared environment)"
        $installed = Invoke-Harness @(
            'install', '--version', $Version,
            '--prefix', (Join-Path $root ".installs\$Version"),
            '--node', $configuration.node, '--npm-cli', $configuration.npmCli,
            '--proof', $proof)
        if ($installed.Code -ne 0) { throw "install failed: $($installed.Output)" }
        Write-Step $installed.Output[0]
    }
    $verified = Invoke-Harness @('verify-install', '--proof', $proof)
    if ($verified.Code -ne 0) { throw "verify-install failed: $($verified.Output)" }
    Write-Step $verified.Output[0]
}
else {
    $codexScript = Join-Path $repository 'tools\vsift-agent-trials\containers\codex\codex-trial.ps1'
    $tag = "$($commit.Substring(0, 12))-$Version"
    & docker image inspect "vsift-codex-trials-agent-published:$tag" *> $null
    if ($LASTEXITCODE -ne 0) {
        Write-Step "Building the clean-install Codex images for vsift-cli@$Version (about 15 minutes; the build installs it from the registry)"
        & pwsh -NoProfile -File $codexScript build -Published -PublishedVersion $Version
        if ($LASTEXITCODE -ne 0) { throw 'codex image build failed' }
    }
    $seen = (& pwsh -NoProfile -File $codexScript versions -Published -PublishedVersion $Version) -join "`n"
    if ($seen -notlike "*$($configuration.codexVersion)*") { throw "The Codex image does not report $($configuration.codexVersion)" }
    Write-Step "Codex image tag $tag; $($configuration.codexVersion)"
}

# --- the freeze --------------------------------------------------------------
if (-not (Test-Path -LiteralPath $freezeFile)) {
    $written = Invoke-Harness @('freeze', 'write', '--repository', $repository, '--commit', $commit, '--output', $freezeFile)
    if ($written.Code -ne 0) { throw "freeze write failed: $($written.Output)" }
    Write-Step "Froze the skill, grader, scenarios, settings and truth at $($commit.Substring(0, 12)) ($($written.Output[0]))"
}
if ($Batch -eq 3 -and -not $AllowGraderChange) {
    $baseline = Join-Path (Split-Path $BatchDirectory -Parent) 'batch-1\freeze.json'
    if (Test-Path -LiteralPath $baseline) {
        $same = Invoke-Harness @('freeze', 'check', '--repository', $repository, '--file', $baseline, '--only', 'grader,cold,settings,truth')
        if ($same.Code -ne 0) { throw "The grader, the cold scenarios, the settings or the truth changed since the baseline (batch 1): $($same.Output) Use -AllowGraderChange only if the maintainer accepts that the baseline is no longer comparable." }
    }
}

# --- the loop ----------------------------------------------------------------
function Wait-ForUsageLimit([object] $ResetUnix) {
    $seconds = $WaitMinutes * 60
    if ($ResetUnix) {
        $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
        $seconds = [Math]::Max(60, [int64]$ResetUnix - $now + 120)
    }
    Write-Step ("Usage limit: waiting {0} minute(s); the stop file ends the wait" -f [Math]::Ceiling($seconds / 60))
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path -LiteralPath $StopFile) { return $false }
        Start-Sleep -Seconds 30
    }
    return $true
}

function Invoke-ClaudeRun([object] $Run) {
    $scenario = Find-Scenario $Run.scenario
    $prepared = Invoke-Harness @(
        'prepare', '--root', $root, '--scenario', $scenario,
        '--install-proof', $proof, '--tools', 'registered', '--freeze', $freezeFile,
        '--cold-settings', $ColdVariant,
        '--vsift-commit', $commit,
        '--ffmpeg', $configuration.ffmpeg, '--ffprobe', $configuration.ffprobe,
        '--whisper', $configuration.whisper, '--model', $configuration.model,
        '--repository', $repository)
    if ($prepared.Code -ne 0) { return @{ Outcome = 'harness-error'; Note = 'prepare failed'; Trial = $null } }
    $trialDirectory = ($prepared.Output[0] -replace '^prepared ', '').Trim()
    $trialId = Split-Path $trialDirectory -Leaf
    $ran = Invoke-Harness @(
        'run', '--trial', $trialDirectory, '--client', 'claude', '--executable', $claude,
        '--model', $Run.model, '--client-home', $configuration.clientHome,
        '--path-dir', $configuration.gitBashUsr, '--pass-env', 'CLAUDE_CODE_GIT_BASH_PATH')
    if ($ran.Code -eq $usageLimitExit) {
        $reset = $null
        if ($ran.Output[0] -match 'reset=(\d+)') { $reset = $Matches[1] }
        return @{ Outcome = 'usage-limited'; Note = 'usage limit'; Trial = $trialId; Reset = $reset }
    }
    if ($ran.Code -ne 0) { return @{ Outcome = 'harness-error'; Note = 'run failed'; Trial = $trialId } }
    $graded = Invoke-Harness @('grade', '--trial', $trialDirectory, '--client-home', $configuration.clientHome)
    if ($graded.Code -ne 0) { return @{ Outcome = 'harness-error'; Note = 'grade failed'; Trial = $trialId } }
    $record = Join-Path $records "$trialId-claude-p1.json"
    $written = Invoke-Harness @('record', '--trial', $trialDirectory, '--output', $record, '--client-home', $configuration.clientHome)
    if ($written.Code -ne 0) { return @{ Outcome = 'harness-error'; Note = 'record failed'; Trial = $trialId } }
    Write-Step ($graded.Output -join ' ')
    $valid = (Get-Content -LiteralPath $record -Raw | ConvertFrom-Json).valid
    return @{ Outcome = $(if ($valid) { 'counted' } else { 'invalid' }); Note = $null; Trial = $trialId }
}

function Invoke-CodexRun([object] $Run) {
    $lines = & pwsh -NoProfile -File $codexScript trial -Published -PublishedVersion $Version `
        -Scenario $Run.scenario -Model $Run.model -Freeze $freezeFile `
        -ClientHome $configuration.codexClientHome -Exports $configuration.codexExports
    $code = $LASTEXITCODE
    if ($code -eq $usageLimitExit) { return @{ Outcome = 'usage-limited'; Note = 'usage limit'; Trial = $null; Reset = $null } }
    if ($code -ne 0) { return @{ Outcome = 'harness-error'; Note = "the container script exited $code"; Trial = $null } }
    $idLine = @($lines | Where-Object { $_ -match '^trial-id (\S+)$' })
    if ($idLine.Count -ne 1) { return @{ Outcome = 'harness-error'; Note = 'no trial id'; Trial = $null } }
    $trialId = ($idLine[0] -replace '^trial-id ', '').Trim()
    $source = Join-Path $configuration.codexExports "records\$trialId-codex.json"
    if (-not (Test-Path -LiteralPath $source)) { return @{ Outcome = 'harness-error'; Note = 'no record exported'; Trial = $trialId } }
    $record = Join-Path $records "$trialId-codex-p1.json"
    Copy-Item -LiteralPath $source -Destination $record
    $valid = (Get-Content -LiteralPath $record -Raw | ConvertFrom-Json).valid
    return @{ Outcome = $(if ($valid) { 'counted' } else { 'invalid' }); Note = $null; Trial = $trialId }
}

function Update-Summary {
    $summary = Invoke-Harness @(
        'summarize', '--state', $state, '--records', $records,
        '--output-json', (Join-Path $BatchDirectory 'summary.json'),
        '--output-markdown', (Join-Path $BatchDirectory 'SUMMARY.md'))
    if ($summary.Code -ne 0) { Write-Warning "summarize failed: $($summary.Output)" }
}

$ran = 0
$waitedSeconds = 0
while ($true) {
    if (Test-Stop) { break }
    if ($MaxRuns -gt 0 -and $ran -ge $MaxRuns) { Write-Step "Reached -MaxRuns $MaxRuns"; break }
    # Nothing frozen may change while a batch runs.
    $held = Invoke-Harness @('freeze', 'check', '--repository', $repository, '--file', $freezeFile)
    if ($held.Code -ne 0) { throw "The freeze no longer holds, which voids the batch: $($held.Output)" }

    $next = Invoke-Harness @('campaign', 'next', '--state', $state)
    if ($next.Code -eq 3) { Write-Step $next.Output[0]; break }
    if ($next.Code -eq 4) { Write-Warning $next.Output[0]; exit 4 }
    if ($next.Code -ne 0) { throw "campaign next failed: $($next.Output)" }
    $run = $next.Output[0] | ConvertFrom-Json
    Write-Step "Run $($run.run_id): $($run.scenario) with $($run.model)"

    $result = if ($Client -eq 'claude') { Invoke-ClaudeRun $run } else { Invoke-CodexRun $run }

    $outcome = switch ($result.Outcome) {
        'counted' { 'counted' }
        'invalid' { 'invalid' }
        'usage-limited' { 'usage-limited' }
        default { 'harness-error' }
    }
    $markArguments = @('campaign', 'mark', '--state', $state, '--run', $run.run_id, '--outcome', $outcome)
    if ($result.Trial) { $markArguments += @('--trial', $result.Trial) }
    if ($result.Note) { $markArguments += @('--note', $result.Note) }
    $marked = Invoke-Harness $markArguments
    Write-Step $marked.Output[0]

    if ($outcome -eq 'usage-limited') {
        $started = Get-Date
        if (-not (Wait-ForUsageLimit $result.Reset)) { break }
        $waitedSeconds += ((Get-Date) - $started).TotalSeconds
        if ($waitedSeconds -gt ($MaxWaitHours * 3600)) { Write-Warning "Waited more than $MaxWaitHours hour(s) in all; stopping. Start again when the allowance is back."; break }
        continue
    }
    if ($outcome -eq 'counted') {
        $ran += 1
        Update-Summary
    }
}

Update-Summary
Write-Step ((Invoke-Harness @('campaign', 'status', '--state', $state)).Output -join ' ')
Write-Step "Records: $records. Summary: $(Join-Path $BatchDirectory 'SUMMARY.md')."
