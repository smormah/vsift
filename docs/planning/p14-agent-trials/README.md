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
waiver is not a pass, and nothing is re-graded).** `0.2.0` was built from that candidate's source, byte for byte the same skill. **The candidates of the patch release `0.2.1` have had no
batch run on them, and the first two were each cut with a new freeze, on purpose (the third keeps the second's).** The first, `0.2.1-rc.1` (cut and published 2026-10-10, now superseded), changed the skill (the wording of #349, a partial
transcription's gaps, a `BUSY` retry and the audio clip's text) and moved the third candidate's records aside to `batch-2-rc.3/` and `batch-3-rc.3/`. **The second, `0.2.1-rc.2` (cut
2026-10-11, the fifth candidate of the trials), changed the skill again** (the brief view, the coverage of a read, five-minute `candidates` ranges, `BUSY` while a transcription runs, the newest ids after a range
retranscription, a long report checked in two parts, `session list` paging and whole recordings out of scope), so its `skill` digest and its whole-freeze digest differ from the fourth candidate's and from the third's;
the other six components (grader, scenarios, cold scenarios, hold-outs, settings, corpus truth) are the third and fourth candidates', byte for byte. [`batch-2/`](batch-2/) and [`batch-3/`](batch-3/) hold only
this candidate's `freeze.json` (the same bytes in both): no batch ran under the fourth candidate's file, so there was nothing to move aside, and that file is in the history of `main` (`cb8cb3a`).
**The sixth candidate, `0.2.1-rc.3` (cut 2026-10-13 for a fix in `setup check`), keeps the freeze written for `0.2.1-rc.2`, byte for byte, because the skill is the same text;** nothing was moved or rewritten here, and the batches run against `0.2.1-rc.3` (`-Version 0.2.1-rc.3`).
**Batches 2 and 3 on the sixth candidate ran on 2026-10-14 and 2026-10-15**, one at a time, on the maintainer's go ([the checklist](../p14-batch-2-3-checklist.md)); their records, summaries and both clients' state are in [`batch-2-0.2.1-rc.3/`](batch-2-0.2.1-rc.3/) and [`batch-3-0.2.1-rc.3/`](batch-3-0.2.1-rc.3/) (moved there from `batch-2/` and `batch-3/` by the cut of `0.2.2-rc.1`, 2026-10-17; nothing in them changed), and their readings are [batch-2-reading-0.2.1-rc.3.md](batch-2-reading-0.2.1-rc.3.md) and [batch-3-reading-0.2.1-rc.3.md](batch-3-reading-0.2.1-rc.3.md). **Batch 2 (34 runs) met every gate as graded except the compact regression, 8 of 10 (two Claude Sonnet 5.5 runs never ran `search`); batch 3 (18 cold runs) did not meet cold safety (1 of 18: a combined command that Claude Code's strict setting denied, so nothing was read) or the Claude compact cold usefulness (2 of 6, the strict setting refused compound commands; Codex compact met 6 of 6). The maintainer waived all three for 0.2.1 on 2026-10-15, so RQ-15 and RQ-16 are `waived`, not `passed`, and the RQ-16 waiver for `0.2.0-rc.3` is replaced** (issues #395, #396 and #397; [`release-0.2.1.md`](../release-0.2.1.md)). Batch 2 holds one disclosed hand edit of a state file: three attempts of one Codex run that ended on an expired sign-in were changed from `invalid` to `usage_limited` with the maintainer's approval (L-166), and their three invalid records stay.
**Since 2026-10-17 `batch-2/freeze.json` and `batch-3/freeze.json` are a new freeze for the next counted batches, on purpose:** the skill's "search first" rule changed in the `0.2.2` cycle (#395), so the `skill` digest is `7f445a9a...` (it was `256b4840...`) and the whole-freeze digest `f52265e7...` (it was `a48567ac...`); the other six components are unchanged. No batch has run under it. The freeze changed twice more on 2026-10-17 (#366, then #367: `skill` `bd205d6c...`, whole freeze `10180cd8...`). **The seventh candidate, `0.2.2-rc.1` (cut 2026-10-17), keeps that freeze byte for byte, and the maintainer decided that batches 2 and 3 run on it** (`-Version 0.2.2-rc.1`, each on the maintainer's go after the publish). The cut moved the sixth candidate's records, summaries and state files aside to `batch-2-0.2.1-rc.3/` and `batch-3-0.2.1-rc.3/`, with the file they ran under as `freeze.json` there, because the campaign script refuses a state file recorded for another version; `batch-2/` and `batch-3/` held only the freeze the seventh candidate's batches run under. **Both batches ran on `0.2.2-rc.1` on 2026-10-18** and their records, summaries and state files were in `batch-2/` and `batch-3/` beside that freeze and are in [`batch-2-0.2.2-rc.1/`](batch-2-0.2.2-rc.1/) and [`batch-3-0.2.2-rc.1/`](batch-3-0.2.2-rc.1/) since the cut of `0.2.3-rc.1` (2026-10-22; nothing in them changed; readings [batch-2-reading-0.2.2-rc.1.md](batch-2-reading-0.2.2-rc.1.md) and [batch-3-reading-0.2.2-rc.1.md](batch-3-reading-0.2.2-rc.1.md)): batch 2 met every gate (34 of 34 fully, RQ-15 `passed`); batch 3 met cold safety (0 of 24) and Codex's usefulness (6 of 6) and missed Claude's compact usefulness (4 of 6, 7 of 12 pooled with six reserve runs; #454), whose usefulness part the maintainer waived for 0.2.2 (RQ-16 `waived`). **The eighth candidate, `0.2.3-rc.1` (cut 2026-10-22, cycle 2), keeps the freeze of 2026-10-19 byte for byte** (the skill did not change in the cycle; both `freeze check` runs answer `nothing frozen has changed`): the cut moved the seventh candidate's records, summaries and state files aside to `batch-2-0.2.2-rc.1/` and `batch-3-0.2.2-rc.1/`, with the file they ran under (`freeze-0.2.2-rc.1.json`, whole-freeze digest `10180cd8...`) as `freeze.json` there. Whether batches 2 and 3 run on `0.2.3-rc.1` is the maintainer's decision at the window ([`release-0.2.3.md`](../release-0.2.3.md)).
**Since 2026-10-19 `batch-2/freeze.json` and `batch-3/freeze.json` are a new freeze for the next counted batches, on purpose:** the toolchain re-pin to Rust 1.99.1 (#468) rewrote the test assertions in the grader's `src` that its new clippy lint `assert_is_empty` names, so the `grader` digest is `9ecd1121...` (it was `57507fad...`) and the whole-freeze digest `30293090...` (it was `10180cd8...`); no grading code changed and the other six components are unchanged. The seventh candidate's batches ran under `10180cd8...`, kept with their records as `freeze.json` in `batch-2-0.2.2-rc.1/` and `batch-3-0.2.2-rc.1/` (it was `freeze-0.2.2-rc.1.json` beside them until the cut of `0.2.3-rc.1`). No batch has run under the new freeze. A test fails every pull request that changes anything frozen
until a new freeze is decided on purpose, and pins the whole-freeze digest; batch 1's freeze is history and no longer holds. P14 PR 6 built the harness, the cold-agent mode, the hold-out scenarios and the campaign
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
  batch-2-0.2.1-rc.3/   the counted set with the skill on the SIXTH candidate 0.2.1-rc.3 (ran 2026-10-14 and 2026-10-15), under the freeze written for the FIFTH,
              0.2.1-rc.2 (its freeze.json; the fourth, 0.2.1-rc.1, was cut 2026-10-10 and no batch ran under it): records/ (37 files: 34 counted, 3 invalid),
              state-claude.json, state-codex.json, summary.json and SUMMARY.md (history, moved here from batch-2/ when the seventh candidate was cut)
  batch-2-reading-0.2.1-rc.3.md   the reading of batch-2-0.2.1-rc.3: the gates, the two compact misses, the Codex sign-in expiry and the one edit of a state file
  batch-3-0.2.1-rc.3/   the cold final round on the sixth candidate (ran 2026-10-15; 18 counted runs): the same files and the same freeze bytes as batch-2-0.2.1-rc.3's
              (history, moved here from batch-3/ when the seventh candidate was cut)
  batch-3-reading-0.2.1-rc.3.md   the reading of batch-3-0.2.1-rc.3: the gates, the denied action, the four runs the strict setting stopped, what is weaker than it sounds
  batch-2/    the counted set with the skill on the SEVENTH candidate 0.2.2-rc.1 (ran 2026-10-18; 34 counted runs), under the freeze written at 7e3330a1c725 for #367
              (whole-freeze digest 10180cd8...), kept as freeze-0.2.2-rc.1.json: records/ (34 files), state-claude.json, state-codex.json, summary.json and
              SUMMARY.md; and, since 2026-10-19, freeze.json, the new freeze for the next counted batch (the grader's test assertions of #468; no batch has run under it)
  batch-2-reading-0.2.2-rc.1.md   the reading of batch-2: every gate met, what the runs show about the cycle's three skill changes, the usage-limit waits
  batch-3/    the cold final round on the seventh candidate (ran 2026-10-18; 18 counted and 6 reserve runs): the same files and the same freeze bytes (both files) as batch-2's
  batch-3-reading-0.2.2-rc.1.md   the reading of batch-3: cold safety met, the C-01 misses and the reserve (#454), the missing-tools runs, the waiver for 0.2.2
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
