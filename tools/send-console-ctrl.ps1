<#
.SYNOPSIS
Sends a console Ctrl-C or Ctrl-Break to the console one process is attached
to. Opt-in test helper for VSift's P10 interruption tests on Windows.

.DESCRIPTION
Windows has no signal a test can send to one process: a console Ctrl-C or
Ctrl-Break reaches every process attached to a console. The VSift tests that
use this helper start `vsift` in a console of its own (CREATE_NO_WINDOW), so
the event reaches only `vsift` and the provider processes it started.

The helper detaches from its own console, attaches to the target's, handles
(and so survives) the event itself, sends it to the whole console (process
group 0), waits briefly and detaches again. It uses kernel32 through .NET
platform invoke, so VSift's own crates stay free of `unsafe` code.

A process that inherited the "ignore Ctrl-C" attribute (for example one
started from a service or from a host that ignores Ctrl-C itself) is never
told about a Ctrl-C by Windows; Ctrl-Break is always delivered.

It writes nothing once detached; the exit code says what happened:
0 sent, 2 could not attach to the target's console, 3 could not send,
4 could not install its own handler.

.PARAMETER ProcessId
The process whose console receives the event.

.PARAMETER Event
CtrlC (the default) or CtrlBreak.

.EXAMPLE
powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File tools/send-console-ctrl.ps1 -ProcessId 1234 -Event CtrlBreak
#>
param(
    [Parameter(Mandatory = $true)]
    [ValidateRange(1, [int]::MaxValue)]
    [int] $ProcessId,

    [ValidateSet('CtrlC', 'CtrlBreak')]
    [string] $Event = 'CtrlC'
)

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace VSiftTest
{
    public static class ConsoleControl
    {
        private delegate bool HandlerRoutine(uint controlType);

        // Kept in a static field so the delegate outlives every event.
        private static readonly HandlerRoutine Survive = controlType => true;

        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern bool FreeConsole();

        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern bool AttachConsole(uint processId);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool SetConsoleCtrlHandler(HandlerRoutine handler, bool add);

        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern bool GenerateConsoleCtrlEvent(uint controlEvent, uint processGroupId);

        public static bool SurviveEvents()
        {
            return SetConsoleCtrlHandler(Survive, true);
        }
    }
}
'@

# CTRL_C_EVENT is 0 and CTRL_BREAK_EVENT is 1.
$controlEvent = if ($Event -eq 'CtrlBreak') { [uint32]1 } else { [uint32]0 }

[void][VSiftTest.ConsoleControl]::FreeConsole()
if (-not [VSiftTest.ConsoleControl]::AttachConsole([uint32]$ProcessId)) {
    exit 2
}
if (-not [VSiftTest.ConsoleControl]::SurviveEvents()) {
    exit 4
}
$sent = [VSiftTest.ConsoleControl]::GenerateConsoleCtrlEvent($controlEvent, [uint32]0)
Start-Sleep -Milliseconds 300
[void][VSiftTest.ConsoleControl]::FreeConsole()
if (-not $sent) {
    exit 3
}
exit 0
