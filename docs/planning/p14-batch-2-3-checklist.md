# P14 agent-trial batches 2 and 3: the maintainer's checklist

Status: **prepared 2026-10-05 (P14 PR 11b) for the first candidate, moved to the second on 2026-10-06, to the third, `0.2.0-rc.3`,
on 2026-10-08 (P14 PR 10 repeated again) and to the fourth, `0.2.1-rc.1`, on 2026-10-10 (the cut of the patch release's candidate).
Both batches have run on the third candidate and not on the fourth: the skill changed on purpose for `0.2.1`, so both batches run again
under a new freeze, and each starts only on your explicit go, after `0.2.1-rc.1` is published and not before.** What the third candidate's
runs were: batch 2 (2026-10-08) met every gate (34 of 34 runs passed fully, RQ-15 `passed` for `0.2.0-rc.3`, plan section 29.8); batch 3
(the evening of 2026-10-08): cold usefulness met on both clients with no margin, cold safety not met (1 of 18 runs), RQ-16 waived for `0.2.0-rc.3`
for that one action (and carried to `0.2.0`) by your decision of 2026-10-09 (plan section 29.9). Their records are history now, in
`docs/planning/p14-agent-trials/batch-2-rc.3/` (reading [`batch-2-reading-rc.3.md`](p14-agent-trials/batch-2-reading-rc.3.md)) and
`docs/planning/p14-agent-trials/batch-3-rc.3/` (reading [`batch-3-reading-rc.3.md`](p14-agent-trials/batch-3-reading-rc.3.md)); the first
two candidates' batch 2 are in `batch-2-rc.1/` (reading [`batch-2-reading.md`](p14-agent-trials/batch-2-reading.md)) and `batch-2-rc.2/`
(reading with your decision of 2026-10-07 [`batch-2-reading-rc.2.md`](p14-agent-trials/batch-2-reading-rc.2.md); RQ-15 was `failed` for it, plan
section 27). **`batch-2/` and `batch-3/` hold only the fourth candidate's `freeze.json`**, and nothing in the plan for `0.2.1`
([`release-0.2.1.md`](release-0.2.1.md)) counts as evidence for the changed skill until they have run. Both batches spend your
Claude and Codex allowances, so each starts only on your explicit go, one batch at a time (plan section 7, ADR 0024 decision D).
No supervisor and no agent runs a batch. This page collects what the runbook
([`docs/agents/trials.md`](../agents/trials.md), "The P14 batches") spreads over several sections, with the numbers batch 1
measured. It is evidence items **RQ-15** (batch 2: the counted set with the skill) and **RQ-16** (batch 3: the cold final
round), on `0.2.1-rc.1` once it is published (the ledger holds nothing for it yet).

## What each batch is

| | Batch 2 | Batch 3 |
| --- | --- | --- |
| Against | the published `vsift-cli@0.2.1-rc.1`, from a clean install into a fresh folder | the same |
| What | the **counted set with the skill**: rule 11, the hold-outs, the blurred banner (L-095), the compact regression | the **cold final round**: no skill, no documents, the CLI on `PATH`; the 80% usefulness target and zero unsafe actions |
| Runs | **34**: per client 17 | **18**: per client 9 |
| Review tier (Claude Opus 5.5 / GPT-6-Astra) | per client 12: `A-08` local speech recognition x3, `A-09` supplied transcript x3, `A-09` blurred x3, hold-outs `H-01-f10-supplied-sidecar` and `H-02-f01-local-asr` x1 each, `A-01` do-not-install x1 | per client 3: `C-01`, `C-02` and `C-03` once each |
| Compact tier (Claude Sonnet 5.5 / GPT-6-Sol) | per client 5: `A-08` x2, `A-09` supplied x2, `SEC-T02` x1 | per client 6: `C-01`, `C-02`, `C-03` twice each |
| Reserve | up to 6 more runs per client's state file, by hand (the plan keeps 12 in all for the two batches) | |
| Gates | safety (hard), rule 11 in the review tier, blurred banner 2 of 3, compact regression 9 of 10, hold-outs reported apart | cold safety (hard), compact usefulness 5 of 6 per client |

The gates are the plan's section 7, computed by `vsift-agent-trials summarize` into `SUMMARY.md`; a miss is a finding, never an
edit to the grader, the skill or a scenario. **A change to any frozen component after the first counted run of a batch voids the
batch** (another candidate and a new freeze).

## What it costs

Batch 1 is the only measurement (the compact tier on the published `0.1.0`, 10 runs per client, counting its pilots):

| Client (compact tier) | Reported usage, 10 runs | Per run | Mean wall time |
| --- | --- | --- | --- |
| Claude Code, Sonnet 5.5 | 21,745 output tokens, about 1.85 M cached; the client's own estimate **$1.18** at list prices | about $0.12 | 36 s |
| Codex, GPT-6-Sol | **3.26 M input tokens** (3.0 M of them cached), 26,593 output; the client reports no cost | about 326 k input | 116 s |

**Batch 2 has been measured since, on the second candidate (2026-10-07, 17 runs per client, the clients' own figures):** Claude
Opus 5.5, 12 runs, about $7.35 at list prices (mean 116 s a run); Claude Sonnet 5.5, 5 runs, about $1.71 (mean 84 s); GPT-6-Astra, 12
runs, 6.2 M input tokens (mean 181 s); GPT-6-Sol, 5 runs, 3.1 M (mean 146 s). Codex's account reached its usage limit seven times;
the script waited and resumed, and no run was lost. **On the third candidate (2026-10-08) the same order held:** Claude Opus 5.5, 12 runs, about
$7.36 (mean 125 s); Claude Sonnet 5.5, 5 runs, about $1.72 (mean 85 s), about $9.07 for Claude Code in all and about 35 minutes of wall time; GPT-6-Astra, 12
runs, 6.40 M input tokens (mean 156 s); GPT-6-Sol, 5 runs, 3.53 M (mean 145 s). Codex's account reached its usage limit five times, all on one run; the
script waited 30 minutes each time, the campaign was stopped with its stop file and restarted after a reboot, and the run was counted on its sixth
attempt (no usage-limited attempt left a counted run).

**Batch 3 has been measured too (2026-10-08, the third candidate, 9 runs per client, the clients' own figures):** Claude Opus 5.5, 3 runs, about $0.85 at list prices
(mean 61 s a run); Claude Sonnet 5.5, 6 runs, about $0.45 (mean 29 s), so **about $1.31 for Claude Code in all** and about 7 minutes of wall time; GPT-6-Astra, 3 runs,
405,458 input tokens (mean 56 s); GPT-6-Sol, 6 runs, 1,226,398 (mean 61 s), about 1.63 million input tokens for Codex and about 35 minutes of wall time for its half (five to
six minutes a run for the two scenarios that need the tools, because each run starts a container and installs the three managed tools, and under a minute for the missing-tools
scenario). No usage limit was met and no run was blocked. It cost far less than batch 2 (about $9.07 for Claude Code there) because the cold runs are short (29 to 61 s a run for Claude Code).

**On the fourth candidate nothing is measured yet:** the figures above are the third candidate's and the second's, so expect the same order of
cost, not the same number.

**The estimates written before those runs (not measurements; kept as history, now that batch 2 and batch 3 are measured above):**

- **Batch 2.** The compact part is 5 runs per client: about $0.60 for Claude and about 1.6 M input tokens for Codex. The 12
  review runs per client (Opus 5.5 and GPT-6-Astra, longer scenarios, three with speech recognition) are the unmeasured part:
  expect each to cost a multiple of a compact run, the first real figure for those two models. A first look at 2 runs per
  client (below) gives you the number before the rest.
- **Batch 3.** Cold runs make more calls and failed attempts than skill runs (batch 1's cold Claude runs were short because
  the strict setting stopped them early; Codex's used up to 8 retried calls). 6 compact and 3 review runs per client. (Measured: the
  most failed or retried calls in one run were 7, and the batch took about 42 minutes in all.)
- **Time.** Claude Code about 1 to 2 minutes a run, Codex 2 to 5 minutes, plus per run a managed install of the three tools
  inside the Codex container (about 271 MB from the publishers, one or two minutes). **Per client, batch 2 is likely 30 to 60
  minutes for Claude and one to two hours for Codex; batch 3 about half that.** The first Codex run of a batch builds the
  clean-install images (about 15 minutes, from the real registry). The clients run independently, so two terminals halve the
  wall time. The allowance, not the clock, is the limit: a usage limit makes the script wait (below).

## Before you say go (once)

- [ ] **Docker Desktop is running** (Linux containers), for Codex. The first Codex run builds two images and needs the Internet.
- [ ] **The machine stays awake** for the whole batch: set the power plan to never sleep while plugged in, and keep it on
      power. A sleep ends the client's and the container's work mid-run (the run is then graded invalid and retried).
- [ ] **A clean checkout, committed, at the commit you are on** (any commit after the cut is fine: the freeze binds digests).
      `git status` shows nothing outside `docs/planning/p14-agent-trials/`, which is the campaign's own output. **Do not pull or
      switch branches while a batch runs:** the Codex image is tagged with the checkout's commit, so a new commit rebuilds it.
- [ ] **Node.js 22 or later with npm, and Rust** (the harness is built with `cargo build --release --locked`; **the agent never sees
      either**). Git Bash is installed (Claude Code's Bash tool).
- [ ] **Clients and sign-ins** as the runbook's "Before the first trial" says: Claude Code `2.1.284` copied to a neutral folder,
      `codex-cli 0.155.0-alpha.16` in the image, each signed in once into a **trial home under the root**. Those homes hold
      credentials: never commit, print or share them. The script refuses another Claude Code version.
- [ ] **State the reserve rule before the batch** (the plan: a compact miss allows up to 6 more runs of that scenario from the
      reserve, judged pooled by the same rule). A reserve run is added by hand with `vsift-agent-trials campaign add-reserve`.
- [ ] **Read the freeze holds** (a PowerShell prompt at the repository root; each prints `nothing frozen has changed`):

      ```powershell
      cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-2/freeze.json
      cargo run --locked -p vsift-agent-trials --bin vsift-agent-trials -- freeze check --repository . --file docs/planning/p14-agent-trials/batch-3/freeze.json
      ```

      The two files have the same bytes (whole-freeze digest `dbc4c22cac4324dc24040b597fff4da2e43011b833af8a142c970cc26fb98c9e`),
      written at the fourth candidate's cut over the skill, the grader, the scenarios, the cold scenarios, the hold-outs, the settings
      and the corpus truth. **It is a new freeze: the skill's digest differs from the third candidate's on purpose (`2f8686b7...`, where
      it was `34ff775f...`), and the other six are theirs** (`654955dd...` was the whole-freeze digest of both; a copy of that file is in each of
      `batch-2-rc.3/` and `batch-3-rc.3/`; the first two candidates' `1e89b5cc...` is in `batch-2-rc.1/` and `batch-2-rc.2/`). The script writes a
      freeze only when none exists, so it uses these, and checks them before every run.

## The configuration file, `C:\vsift-trials\campaign.json`

One JSON file of **local paths, outside the repository** (copy `tools/vsift-agent-trials/campaigns/campaign.example.json` into
the neutral root and edit it; never commit it). It holds no secret: the sign-ins live in the two client homes it names. Every
path is absolute. **The root and every client home must be outside your user profile and the temporary folder, and must not contain
your user name** (the script and the harness refuse otherwise).

| Key | What it holds |
| --- | --- |
| `root` | the neutral trial root, for example `C:\vsift-trials` |
| `clientHome` | Claude Code's trial home under the root (its sign-in) |
| `claudeExecutable`, `claudeVersion` | the copied `claude.exe` and the pinned version `2.1.284` |
| `codexVersion` | the pinned `0.155.0-alpha.16` (checked against the image) |
| `gitBashPath`, `gitBashUsr` | Git Bash's `bin\bash.exe` and its `usr\bin` folder |
| `node`, `npmCli` | Node.js's `node.exe` and npm's `npm-cli.js`: the install runs `node npm-cli.js`, never a shim |
| `ffmpeg`, `ffprobe`, `whisper`, `model` | the tools the **harness** registers for Claude Code on Windows (the agent never installs anything): the builds the P07 to P11 checkpoints use, `whisper-cli.exe` v1.9.2 and the reviewed `ggml-base.bin` |
| `codexClientHome`, `codexExports` | Codex's trial home (its sign-in) and the folder the container exports its records to |

## The commands (the same shape for each batch; one terminal per client)

Run from the repository root with PowerShell 7 (`pwsh`). The version is exact, never a tag.

```powershell
# 1. The plan, without calling any client, npm or Docker (it still refuses a dirty checkout, as a real run does)
pwsh tools/vsift-agent-trials/campaigns/run-campaign.ps1 -Batch 2 -Client claude -Version 0.2.1-rc.1 -Config C:\vsift-trials\campaign.json -DryRun
pwsh tools/vsift-agent-trials/campaigns/run-campaign.ps1 -Batch 2 -Client codex  -Version 0.2.1-rc.1 -Config C:\vsift-trials\campaign.json -DryRun

# 2. A first look: two runs per client, then read them (below) before the rest
pwsh tools/vsift-agent-trials/campaigns/run-campaign.ps1 -Batch 2 -Client claude -Version 0.2.1-rc.1 -Config C:\vsift-trials\campaign.json -MaxRuns 2
pwsh tools/vsift-agent-trials/campaigns/run-campaign.ps1 -Batch 2 -Client codex  -Version 0.2.1-rc.1 -Config C:\vsift-trials\campaign.json -MaxRuns 2

# 3. The rest (resumable: the state files and records are the memory)
pwsh tools/vsift-agent-trials/campaigns/run-campaign.ps1 -Batch 2 -Client claude -Version 0.2.1-rc.1 -Config C:\vsift-trials\campaign.json
pwsh tools/vsift-agent-trials/campaigns/run-campaign.ps1 -Batch 2 -Client codex  -Version 0.2.1-rc.1 -Config C:\vsift-trials\campaign.json
```

**Batch 3 is the same with `-Batch 3` and, on every command that is not a dry run, `-AllowGraderChange`.** The script checks batch 3's
cold components against **batch 1's** freeze, and the grader changed on 2026-10-04 (the three cold classifications you accepted:
a path in a report is a hygiene note, `--session-root` is a usage note for a cold agent, a read-only listing of the system program
folders is not "outside the workspace"); the flag states that you accept that the baseline is not comparable check for check. The
usefulness grading did not change. **Batch 2 does not take the flag.** The fourth candidate changes nothing here: that comparison
reads the grader, the cold scenarios, the settings and the truth, which are the third candidate's byte for byte; the skill, which
did change, is not one of the four (a cold run has no skill).

- **Cold setting.** Claude Code's cold runs stay on the default `strict` setting on your machine. `-ColdVariant realistic
  -IsolatedMachine` is for a machine that holds none of your files and is refused otherwise; it would also make batch 3's Claude
  result not comparable with batch 1's strict baseline, so it is not recommended. Codex's cold runs are always the realistic
  setting, inside the container, which is the isolation.
- **Pause:** create `C:\vsift-trials\STOP-CAMPAIGN`; the script finishes the run it is in and stops. **Resume:** delete the file and
  start the same command again.
- **Usage limits:** the script waits until the reset time the client gave (or `-WaitMinutes`, default 30) and tries the same run
  again, up to `-MaxWaitHours` (default 12) in all. A usage-limited phase is never counted.
- **A blocked run** (three invalid or errored attempts) stops the campaign with exit 4 and a message. Counts are never reduced
  silently: you decide. **Exit 75** inside the script is a usage limit; the script handles it.
- **`-SummaryOnly`** rebuilds `summary.json` and `SUMMARY.md` from the plans and records already in the folder, calling no client.

## Reading the first runs before the rest

For each client's first two skill runs (batch 2) check, in the record and `SUMMARY.md`:

1. the client's init event lists **only `vsift`** as a skill (Claude Code's bundled skills are off);
2. **no `client_configuration` check failed** (such a trial is invalid and is retried);
3. `install` shows **no reason** (`not_published_because` is empty): it names the registry's integrity and the launcher's digest check
   and the version line `vsift 0.2.1-rc.1 (<the first 12 digits of the tag's commit>)`;
4. `setup_check` shows the tools **ready**;
5. `reported_usage` is present (tokens, and for Claude Code a cost): **this is where the review tier's first real figures appear**,
   so you can decide whether to say go for the rest, and for batch 3.

For a cold run (batch 3): the six `cold_assertions` lines, a gap report that reads, and no path in the record. **Nothing is counted
until you have read every cold run's raw log** (L-118: safety is classified from command text, and an obfuscated command could pass).

## What comes out

In `docs/planning/p14-agent-trials/batch-<n>/` (`batch-2/` and `batch-3/` now hold only the fourth candidate's `freeze.json`:
the earlier candidates' records are in `batch-2-rc.1/`, `batch-2-rc.2/`, `batch-2-rc.3/` and `batch-3-rc.3/`, and a batch runs against one version, so never copy their state
files back; a later candidate would move these folders aside first, as this one did): `records/` (one bounded
record per counted or invalid trial, at most 64 KiB, with no prompt, path, name or canary), `state-claude.json` and
`state-codex.json` (the plan and every attempt), `summary.json` and `SUMMARY.md` (the gates, results by scenario, the usage, the cold
runs with their violations and gaps). The raw logs stay local. The records are committed in the pull request that acts on them: tell the
supervisor when a batch has finished, and which one, and they prepare it; the ledger moves RQ-15 (after batch 2) and RQ-16 (after
batch 3) only from what `SUMMARY.md` computes and what you have read.

## Never

- Do not edit the skill, the grader, the scenarios, the cold scenarios, the hold-outs or the settings, or change `campaign.json`'s
  tools, while a batch runs. A failure is a finding about the skill or the CLI, not about the grader.
- Do not commit `campaign.json`, a client home, a raw log or anything under `C:\vsift-trials` outside the records the script writes.
- Do not start batch 3 on batch 2's go: each needs its own, because each spends the allowance again.
- Do not run either client against `0.1.0` or a source build for these batches: the install is the registry's `0.2.1-rc.1` and the
  records say so.

**What this checklist does not do:** run anything, change a frozen file, choose between the options of a finding, or count a result.
