# P14 agent trial records

**Status: batch 1 (the baseline on the published 0.1.0) ran on 2026-10-03. Batch 2 (the counted
set with the skill, 34 runs) has run on all three candidates; the first two runs are history: on the first, `0.2.0-rc.1`, on 2026-10-05 (its records are in
[`batch-2-rc.1/`](batch-2-rc.1/) and its reading, whose four cases were never decided, is [batch-2-reading.md](batch-2-reading.md)), and on the
second, `0.2.0-rc.2`, on 2026-10-07 (its records are in [`batch-2-rc.2/`](batch-2-rc.2/); its reading, with the maintainer's decision of the same day, is
[batch-2-reading-rc.2.md](batch-2-reading-rc.2.md): Claude Opus 5.5's review tier missed two gates, the evidence item RQ-15 was `failed` for that
candidate, nothing is re-graded, and the maintainer decided to improve the skill and cut a third candidate, `0.2.0-rc.3`). The third candidate was
cut on 2026-10-08, and batch 2 ran on it the same day: its 34 records, summary and both clients' state are in [`batch-2-rc.3/`](batch-2-rc.3/), its reading is
[batch-2-reading-rc.3.md](batch-2-reading-rc.3.md), **every gate is met as graded (34 of 34 runs passed fully), and RQ-15 is `passed` for `0.2.0-rc.3`**
(a small sample, not a proof: the reading says what is weaker than it sounds). Batch 3 (the cold final round, RQ-16, 18 runs with no skill) ran on the same
candidate on 2026-10-08: its 18 records, summary and both clients' state are in [`batch-3-rc.3/`](batch-3-rc.3/) and its reading is
[batch-3-reading-rc.3.md](batch-3-reading-rc.3.md). Cold usefulness is met on both clients with no margin (5 of 6 each), the hard cold safety gate is not met (1 of 18 runs: a `base64` of
the audio clip that `vsift audio` had named), and the maintainer decided on 2026-10-09 to waive RQ-16 for `0.2.0-rc.3` only, for that one action (issue #340, register entry L-142; a
waiver is not a pass, and nothing is re-graded).** `0.2.0` was built from that candidate's source, byte for byte the same skill. **The candidate of the patch release `0.2.1`,
`0.2.1-rc.1`, was cut on 2026-10-10 with a new freeze, on purpose, and no batch has run on it: [`batch-2/`](batch-2/) and [`batch-3/`](batch-3/) hold only its
`freeze.json` (the same bytes in both), and the third candidate's records moved aside to `batch-2-rc.3/` and `batch-3-rc.3/`.** The skill changed (the wording of #349, a partial
transcription's gaps, a `BUSY` retry and the audio clip's text), so the `skill` digest and the whole-freeze digest differ from the third candidate's; the other six components
(grader, scenarios, cold scenarios, hold-outs, settings, corpus truth) are theirs, byte for byte. **Batches 2 and 3 on the fourth candidate start only on the maintainer's explicit go
and after the candidate is published**, one at a time ([the checklist](../p14-batch-2-3-checklist.md)); until they have run, RQ-15 and RQ-16 say nothing about the changed skill.
A test fails every pull request that changes anything frozen
until another candidate and a new freeze are decided on purpose, and pins the whole-freeze digest; batch 1's freeze is history and no longer holds. P14 PR 6 built the harness, the cold-agent mode, the hold-out scenarios and the campaign
scripts; every batch spends the maintainer's Claude and Codex allowances and starts on the
maintainer's explicit go. This folder is where the batches' bounded records land. The reading of
batch 1 is [batch-1-reading.md](batch-1-reading.md); it qualifies nothing, it is the "before"
picture for the final round.

The maintainer's checklist for batches 2 and 3 (what runs, the commands, the configuration file, the
cost) is [`../p14-batch-2-3-checklist.md`](../p14-batch-2-3-checklist.md).
The plan is [p14-qualification.md](../p14-qualification.md) section 7; the runbook, with what the
maintainer does for each batch and what it costs, is
[`docs/agents/trials.md`](../../agents/trials.md) ("The P14 batches"); the decisions are
[ADR 0024](../../decisions/0024-r0-qualification-and-release-candidate.md) decision D and its PR 6
note.

## Layout

```text
p14-agent-trials/
  batch-1/    pilots and the cold baseline, against the published 0.1.0 (ran 2026-10-03)
  batch-1-strict-first-attempt/   the first four Claude runs (same strict settings; kept, not counted twice)
  batch-1-reading.md              the maintainer-side reading of batch 1
  batch-2-rc.1/   the counted set with the skill, on the FIRST candidate 0.2.0-rc.1 (ran 2026-10-05; history,
                  moved here when the second candidate was cut; its freeze.json is a copy of the file as it was)
  batch-2-reading.md              the maintainer-side reading of batch-2-rc.1
  batch-2-rc.2/   the counted set with the skill, on the SECOND candidate 0.2.0-rc.2 (ran 2026-10-07; history,
                  moved here when the third candidate was cut; its freeze.json is a copy of the file as it was)
  batch-2-reading-rc.2.md         the reading of batch-2-rc.2 and the maintainer's decision of 2026-10-07
  batch-2-rc.3/   the counted set with the skill, on the THIRD candidate 0.2.0-rc.3 (ran 2026-10-08: records/, state-claude.json,
                  state-codex.json, summary.json, SUMMARY.md and freeze.json; history, moved here when the fourth candidate was cut;
                  its freeze.json is a copy of the file as it was)
  batch-2-reading-rc.3.md         the reading of batch-2-rc.3 (the third candidate): the gates against the earlier candidates, the usage-limit
                                  check, what the Claude Opus reports did differently, and what is weaker than it sounds
  batch-3-rc.3/   the cold final round (no skill), on the THIRD candidate 0.2.0-rc.3 (ran 2026-10-08, 18 counted runs; history, moved here
                  when the fourth candidate was cut; its freeze.json is a copy of the file as it was)
  batch-3-reading-rc.3.md         the reading of batch-3-rc.3: the gates against the 0.1.0 baseline, the cold finding and the maintainer's decision of
                                  2026-10-09, the read of the raw logs, what the cold agents struggled with, and what is weaker than it sounds
  batch-2/    the FOURTH candidate 0.2.1-rc.1 (cut 2026-10-10): holds only freeze.json, the file the candidate was cut with; the batch has
              not run (it starts on the maintainer's go, after the candidate is published, and writes its records, state files and summary here)
  batch-3/    the same for the cold final round: only freeze.json (the same bytes as batch-2's), no batch run yet
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
