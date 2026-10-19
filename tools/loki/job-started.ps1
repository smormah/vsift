# LOKI's job-started hook (ADR 0025; docs/operations/loki-runner.md).
#
# The runner application runs this before every job, as the runner's own user,
# when ACTIONS_RUNNER_HOOK_JOB_STARTED names it in the runner's .env file. It hands
# the decision to job-guard.cjs, which must sit in the same folder; a non-zero exit
# fails the job before any of its steps runs. Keep both files outside the runner's
# application folder and work folder, writable only by an administrator.

#
# Node.js is named by its machine-wide install path, not looked up through PATH,
# which the runner's user could change.

$ErrorActionPreference = 'Stop'
& (Join-Path $env:ProgramFiles 'nodejs\node.exe') (Join-Path $PSScriptRoot 'job-guard.cjs')
exit $LASTEXITCODE
