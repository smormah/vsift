---
description: Match log lines to what a recording shows at the same clock time with vsift
argument-hint: <video> <log file> [recording start time]
---

Use the vsift skill of this plugin to produce log lines matched by clock time, following its workflow file
`workflows/logs-by-clock.md` step by step. Read the skill's `SKILL.md` and that workflow file first, with
your file-reading tool. If you cannot read them, `vsift skill show workflows/logs-by-clock.md --json` prints the
workflow of the installed `vsift`.

The recording and the request: $ARGUMENTS
