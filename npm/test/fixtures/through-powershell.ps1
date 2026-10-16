<#
.SYNOPSIS
Runs one command once per case, the way a PowerShell user or a PowerShell
script passes arguments, and prints what each run printed as JSON.

.DESCRIPTION
Used on Windows by npm/test/launcher.test.cjs to send the hostile cases of
tools/p14-published/lib/hostile.cjs through the shims npm installs (#257,
known limit L-109). The command, the arguments that come before each case and
the cases themselves arrive in the environment, each argument as Base64 of its
UTF-8 bytes, so nothing on the way in (pwsh's own command line) reads them as
syntax; see tools/p14-published/invoke.ps1 for the same reasoning. Each run
gets its arguments as an array through the call operator: PowerShell then
builds the command line itself, as it does for `vsift <arguments>` typed at
its prompt.

Before each case the marker file is removed; after it, the script records
whether it exists again, which means a command the case spelled has run.
#>

$ErrorActionPreference = 'Stop'

function Read-Arguments([string] $carried) {
    if ([string]::IsNullOrEmpty($carried)) {
        return @()
    }
    return @($carried.Split(',') | ForEach-Object { [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String($_)) })
}

$command = $env:VSIFT_TEST_COMMAND
$prefix = Read-Arguments $env:VSIFT_TEST_PREFIX
$cases = Read-Arguments $env:VSIFT_TEST_CASES
$marker = $env:VSIFT_TEST_MARKER
# The program under test sees none of this in its environment.
$env:VSIFT_TEST_COMMAND = $null
$env:VSIFT_TEST_PREFIX = $null
$env:VSIFT_TEST_CASES = $null
$env:VSIFT_TEST_MARKER = $null

$ErrorActionPreference = 'Continue'
$results = foreach ($case in $cases) {
    Remove-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
    $arguments = @($prefix) + @($case)
    $output = & $command @arguments 2>$null
    $status = $LASTEXITCODE
    [pscustomobject]@{
        stdout = (@($output) -join "`n")
        status = $status
        marker = (Test-Path -LiteralPath $marker)
    }
}
Remove-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue
ConvertTo-Json -InputObject @($results) -Compress -Depth 3
