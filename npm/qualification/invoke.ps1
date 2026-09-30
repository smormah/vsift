<#
.SYNOPSIS
Runs one command the way a PowerShell user types it, and exits with its status.
Used on Windows by the npm qualification driver (qualify.cjs), which starts
`pwsh -NoProfile -NonInteractive -File invoke.ps1 <command> <argument>...`.

.DESCRIPTION
Package managers and the commands they install are `.cmd` and `.ps1` shims
on Windows, which Node.js starts only through a shell. This script is that
shell step, with no text to interpret: the command and each argument arrive
as separate process arguments and are passed on with the call operator, so
nothing is parsed or expanded, and PowerShell resolves the command as it
does for a user (a `.ps1` shim first, then `.cmd` or `.exe`).
#>

$ErrorActionPreference = 'Stop'
$command = $args[0]
$rest = @($args | Select-Object -Skip 1)
& $command @rest
exit $LASTEXITCODE
