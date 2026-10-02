<#
.SYNOPSIS
Runs one command the way a PowerShell user types it, and exits with its status.
Used on Windows by the P14 published-artifact tools (lib/common.cjs), which start
`pwsh -NoProfile -NonInteractive -File invoke.ps1` with the command and its
arguments in the environment (P14_COMMAND, P14_ARGUMENT_COUNT and P14_ARGUMENTS).

.DESCRIPTION
Package managers and the commands they install are `.cmd` and `.ps1` shims
on Windows, which Node.js starts only through a shell. This script is that
shell step, with no text to interpret: PowerShell resolves the command as it
does for a user (a `.ps1` shim first, then `.cmd` or `.exe`) and runs it with
each argument exactly as given.

The arguments do not travel on pwsh's own command line. `pwsh -File script.ps1
--file=C:\x` splits a token that looks like `-name:value` at its colon before the
script sees it (it arrives as `--file=C` and `\x`), which is a property of that
command line and not of the program under test, and it would make every
`--option=<drive path>` case fail for a reason that has nothing to do with VSift.
Here each argument is carried as Base64 of its UTF-8 bytes, so that nothing
(a quote, a newline, a colon, a leading dash) can be read as syntax on the way in.

It is not the same step as npm/qualification/invoke.ps1 (P13 PR 9), which passes
the arguments on the command line; it is kept here so that this tool does not
depend on a path whose change would start the Release workflow.
#>

$ErrorActionPreference = 'Stop'
$command = $env:P14_COMMAND
$count = [int]$env:P14_ARGUMENT_COUNT
$rest = @()
if ($count -gt 0) {
    $rest = @($env:P14_ARGUMENTS.Split(',') | ForEach-Object { [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String($_)) })
}
# The program under test sees neither the command nor the arguments in its environment.
$env:P14_COMMAND = $null
$env:P14_ARGUMENT_COUNT = $null
$env:P14_ARGUMENTS = $null
& $command @rest
exit $LASTEXITCODE
