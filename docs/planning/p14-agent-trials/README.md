# P14 agent trial records

**Status: no trial has run.** P14 PR 6 built the harness, the cold-agent mode, the hold-out
scenarios and the campaign scripts, and ran nothing: every batch spends the maintainer's Claude
and Codex allowances and starts on the maintainer's explicit go. This folder is where the
batches' bounded records land. Until a batch runs it holds only this page.

The plan is [p14-qualification.md](../p14-qualification.md) section 7; the runbook, with what the
maintainer does for each batch and what it costs, is
[`docs/agents/trials.md`](../../agents/trials.md) ("The P14 batches"); the decisions are
[ADR 0024](../../decisions/0024-r0-qualification-and-release-candidate.md) decision D and its PR 6
note.

## Layout

```text
p14-agent-trials/
  batch-1/    pilots and the cold baseline, against the published 0.1.0
  batch-2/    the counted set with the skill, on the candidate
  batch-3/    the cold final round, on the candidate
    records/<trial>-<client>-p1.json   one bounded record (at most 64 KiB) per counted or invalid trial
    state-claude.json, state-codex.json  the plan and every attempt of each client (run identifiers,
                                         trial identifiers, outcomes; no path, prompt or name)
    freeze.json                        the digests of the skill, grader, scenarios, settings and
                                       corpus truth the batch ran under
    summary.json, SUMMARY.md           the gates of the plan, results by scenario and the usage the
                                       clients reported, computed from the state and the records
```

## What a record holds

The same as P12's records (the calls with their verdicts, both results, the handoff, the
digests of the scenario, skill, settings and fixtures, the SHA-256 of the raw logs, which stay
local), plus, since PR 6:

- `mode` (`skill` or `cold`), `holdout`, `skill_source` and `tools_source`;
- `install`: what proves the published install was used (the version, the registry and the
  integrity of the launcher and native packages as npm installed them and as the registry
  reports them, the launcher's digest check, the version line `vsift --version` printed through the
  launcher, the Node.js and npm versions);
- `setup_check`: what `setup check` reported after the harness provided the tools and before
  the agent started;
- `freeze_sha256`: the digest of the freeze file the trial was prepared under;
- `reported_usage`: tokens and the client's own cost estimate, as the client reported them;
- `usage_limit`: set when the client stopped at its usage limit (the trial is then invalid);
- `cold`, for a cold trial: the safety gate and its violations by kind, usefulness, the gap
  report (failed and retried calls with their typed error codes) and the calls that were
  off-method but not unsafe; and `cold_assertions`, what `prepare` proved about the workspace.

A record never holds a prompt beyond the arguments P12's records already held, a tool's output,
the check image's code, a canary value, a user name or a local path (the neutral tokens
`<workspace>`, `<home>`, `<install>` and the others replace them, and a test reads every file in
this folder for what should not be there).
