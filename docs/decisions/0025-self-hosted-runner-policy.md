# ADR 0025: The self-hosted runner policy (LOKI, the overnight test machine)

- Status: **Accepted** (2026-10-19, the maintainer's decision, given on pull request #472 after
  reading this record and the register entry L-186, which the maintainer accepted with it). The
  maintainer decided earlier the same day that LOKI gets a GitHub Actions runner with the label
  `loki`, used only by VSift's scheduled or manually started test workflows, never by pull
  requests and never by the release workflow; this record turns that decision into rules. Also
  decided on 2026-10-19: the RQ-17 try-out on LOKI comes before anything is installed for the runner
  (section 10).
- Date: 2026-10-19
- Applies to: every workflow under `.github/workflows/` (the governance workflow lint, rule 9),
  `.github/workflows/loki-overnight.yml`, `tools/loki/`, and the machine LOKI.
- Refines: [ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md) section 1
  (the workflow lint) and the test and release model of 2026-10-16
  ([`development.md`](../development.md), "The CI lanes").
- Runbook: [`operations/loki-runner.md`](../operations/loki-runner.md). Register:
  [L-186](../planning/known-limits.md#l-186) (the residual risk, and that the lane is not yet
  shown to work).

## Context

**The load has nowhere to go.** The campaigns that load a machine for hours (the race and stress
repetitions, the external-delivery simulation, the opt-in batch test of #277) run on hosted runners
when they run at all, and the reproductions that need a failure rate under load (#439, #443: about
one failure in a thousand loaded runs on Windows) were made on the maintainer's development
machine, which is shared with other projects and their own work. The cycle rules of 2026-10-18 stop
that: no load, stress or soak runs there. Hosted Windows runners have four CPUs and six-hour jobs,
and are not a real desktop.

**LOKI exists.** It is the maintainer's second, clean, wipeable Windows 11 machine, recorded for the
RQ-17 try-out ([`rq-17-tryout-sheet.md`](../planning/rq-17-tryout-sheet.md)). It is idle at night.

**The repository is public and personal.** GitHub restricts which workflows may use a self-hosted
runner only through runner groups, and runner groups exist only for organisations. On a personal
repository any job of any workflow that names the runner's labels is sent to it, including a
workflow a pull request adds or changes: GitHub runs a pull request's own copy of its workflows.
GitHub's own guidance is not to use self-hosted runners with public repositories for that reason.
Whatever restricts LOKI here must therefore be built: in the repository (checked when a change is
merged) and on LOKI (checked when a job arrives).

**The release path verifies against self-hosted runners.** Every `gh attestation verify` in the
release runbook passes `--deny-self-hosted-runners`, so an attestation made on LOKI would be
refused. The policy keeps it that way: nothing that publishes, signs or attests runs there.

## Decision

### 1. One self-hosted runner, named by its label

LOKI's runner carries the label `loki` (beside the runner application's defaults `self-hosted`,
`Windows` and `X64`). A job that runs there names `loki`; no other self-hosted runner is approved.
Every `runs-on` in the repository must be readable by the lint: a GitHub-hosted image label
(`ubuntu-24.04`, `windows-latest`, `macos-15`, ...), LOKI's labels, or a literal matrix of them. A
computed runner is refused, except for the one job listed in the lint with its reason (`ci.yml`'s
`quality`, whose systems `tools/ci_lanes.py` chooses from hosted labels). Runner groups are refused.

### 2. Allowed triggers: `schedule` and `workflow_dispatch` only

A job may target `loki` only in a workflow triggered solely by `schedule`, `workflow_dispatch` or
both. Never `pull_request`, `pull_request_target`, `push`, `workflow_run`, `issue_comment`,
`workflow_call`, `merge_group` or any other event: the lint allows the two and refuses the rest, so a
new event GitHub adds is refused too. A LOKI job cannot sit in a reusable workflow (its trigger is
`workflow_call`), so every LOKI job is written where its triggers are visible.

### 3. Never the release path

No LOKI job in `release.yml`. A workflow with a LOKI job holds no write scope at all except a
GitHub-hosted job's `issues: write` (the failure report): no `id-token`, `attestations`, `contents`,
`packages`, `pull-requests` or `actions` write, so it cannot attest, sign with an OIDC identity,
push, tag or release. It runs no job in a deployment environment. Publishing is already refused
outside the release workflow's `publish` job (lint rule 8).

### 4. Permissions and secrets

The LOKI job declares `permissions: { contents: read }` and nothing else. A workflow with a LOKI job
names no secret but `secrets.GITHUB_TOKEN`, and passes none to a reusable workflow; the LOKI job
itself names neither a secret nor `github.token`. Every checkout in a LOKI job sets
`persist-credentials: false`, so the job token is not written into the checkout's Git
configuration on a machine that keeps its folders. The runner still receives the job's own
short-lived, read-only token, as every runner does.

### 5. The runner account: a dedicated standard user, persistent runner, a clean-up step

The runner runs as a Windows service under a **dedicated local standard (non-administrator) user**,
`vsift-runner`, with its own folders outside every person's profile: `C:\vsift-runner\actions-runner`
(the application), `C:\vsift-runner\work` (the work folder) and `C:\vsift-runner\hooks` (the
job-started hook, writable only by an administrator). The user has no Microsoft account, no
administrator rights, no access to the maintainer's profile or media, and nothing but the
toolchains the workflow needs.

**Persistent, with a clean-up step, not ephemeral.** An ephemeral runner (`--ephemeral`, or a
just-in-time configuration) takes one job and unregisters; it needs a new registration for every
job, which means either the maintainer registering it by hand each night or a credential on LOKI
that can register runners (an administration token for the repository). That credential would be
the most valuable thing on the machine, and a job that reached it could register runners of its
own. A persistent runner needs no credential after its one registration (the runner's own
key, which can only take jobs). What it keeps between jobs is bounded instead:

- the checkout is cleaned by `actions/checkout` (`clean: true`) at the start of each run;
- every folder the tests create in the temporary directory is inside one folder of the run's own
  (`TEMP`, `TMP` and `TMPDIR` point there, `tools/loki/work-folder.cjs prepare`), and the last
  step removes it and the checkout's Cargo target folder;
- the runner application ends the job's leftover processes when the job ends;
- what stays is the runner user's Rust toolchain and Cargo registry cache and the Node.js tool
  cache: tools pinned by the repository, reused deliberately.

### 6. Clean-up of VSift-owned folders only

AGENTS.md's rule applies: automatic clean-up touches only positively identified VSift-owned
folders. `work-folder.cjs clean` removes the run's temporary folder only when it is a real folder
(not a link or junction), a direct child of the runner's temporary folder, named
`vsift-loki-<run id>` and holding the marker it wrote for that run; and the target folder only when
it is a real `target` folder holding Cargo's `CACHEDIR.TAG` inside a checkout of this repository. It
deletes nothing else, and names what the tests left behind (#423's names) as a warning.

### 7. No identity

The runner sends nothing personal. The machine's name, the runner's name and the runner user's name
appear in every job's public log (a public repository's logs and artifacts are public), so none of
them may be a person's name: the machine is `LOKI`, the runner `loki`, the user `vsift-runner`. The
workflow prints the operating system, the processors and the disk, never the user or a profile
path. Nothing in the workflow or the runbook asks for an email address, an account name or a
contact.

### 8. A run-time guard on LOKI itself

The lint checks what is merged, not what runs: a pull request's copy of a workflow runs before
review. LOKI therefore has a **job-started hook** (`ACTIONS_RUNNER_HOOK_JOB_STARTED`, set in the
runner's `.env`): `tools/loki/job-started.ps1` runs `tools/loki/job-guard.cjs`, which refuses,
before any step runs, every job that is not `smormah/vsift`'s `loki-overnight.yml` started by
`schedule` or `workflow_dispatch` on a branch. The maintainer copies the two files from a reviewed
commit to `C:\vsift-runner\hooks`; a change in the repository does not change them. A new LOKI
workflow therefore needs both a merged change and the maintainer updating LOKI's copy.

The repository setting **Actions, Fork pull request workflows from outside collaborators: "Require
approval for all outside collaborators"** is the third layer: no outsider's workflow runs at all
until the maintainer approves it, and the maintainer never approves one that changes
`.github/workflows/` without reading it.

### 9. Pausing and removing

- **Pause for a night or a week:** stop the service (`Stop-Service` on the runner's service, as
  an administrator). Queued jobs wait up to a day and are then cancelled; the report job comments
  on the open `loki-overnight-failure` issue. **For longer**, also disable the workflow
  (`gh workflow disable loki-overnight.yml`) so no run queues.
- **Remove:** stop and uninstall the service (`config.cmd remove` with a removal token the
  maintainer creates in the repository settings), delete the runner in Settings, Actions, Runners if
  it is still listed, delete `C:\vsift-runner`, and delete the `vsift-runner` user and its profile.

The runbook has the exact commands.

### 10. The RQ-17 try-out comes first (decided 2026-10-19)

LOKI is also the clean machine of the RQ-17 try-out (Smart App Control as a new user meets it).
Setting up the runner installs the build tools, Node.js and Rust, after which Windows may switch
Smart App Control off for good. **The maintainer decided on 2026-10-19 that the try-out comes
first:** it is done on the clean LOKI with the published `0.2.2`, before anything is installed for
the runner ([runbook](../operations/loki-runner.md) section 0;
[`rq-17-tryout-sheet.md`](../planning/rq-17-tryout-sheet.md) points to the runbook as what follows
it). Once the runner is set up, LOKI is not a clean machine: a later try-out needs a reset of LOKI
(the runner removed first and set up again after) or another clean Windows 11 machine.

## Residual risks

Stated plainly; [L-186](../planning/known-limits.md#l-186) carries them for review.

1. **A merged workflow change reaches LOKI.** Anyone who can merge to `main` (the maintainer) can
   change `loki-overnight.yml` or the code it runs, within the lint's rules, and that code runs on
   LOKI as `vsift-runner`. The lint, the review and the hook bound the triggers and the token, not
   what the merged code does. A malicious or mistaken merged change could read what
   `vsift-runner` can read, use LOKI's network, and change the runner user's own files, including the
   runner application's `.env` (the service must be able to write its own folder to update itself),
   so it could switch the hook off for later jobs.
2. **A dispatch needs only write access.** `workflow_dispatch` can be started by anyone with write
   access to the repository, on any branch they can push, and runs that branch's copy of the
   workflow and code. Today that is the maintainer alone; a collaborator with write access would
   be able to run code on LOKI.
3. **A pull request's copy of a workflow.** Outsiders' runs wait for approval and the hook refuses
   their event; both must hold. If the approval setting is relaxed and the hook is missing or
   broken (for example after the runner's `.env` is lost), a fork's pull request can add a workflow
   that names `loki` and it would run.
4. **The runner application updates itself** from GitHub's releases while it runs. That is
   GitHub's code with the same trust as the first download, but it is not verified by this
   repository after the first install.
5. **The machine is on the maintainer's network.** A job that runs on LOKI can reach whatever
   LOKI can reach. The runbook asks for LOKI to hold nothing the runner user can read except the
   runner's folders and its toolchains, and for the maintainer's private media to be unreadable by
   that user.
6. **LOKI stops being a clean machine.** Installing the build tools, Rust and Node.js makes LOKI a
   developer machine; Windows may then switch Smart App Control off by itself, and it cannot be
   turned back on without reinstalling Windows. The RQ-17 try-out needs Smart App Control as a new
   user meets it. Decided 2026-10-19: the try-out comes first (section 10), and a later try-out
   needs a reset of LOKI or another clean machine
   ([L-186](../planning/known-limits.md#l-186)).

## Consequences

- The governance workflow lint has a rule 9 (`tools/vsift-governance/src/workflows/runners.rs`) with
  a broken-copy test for each of its parts; every workflow in the repository passes it.
- `loki-overnight.yml` is the overnight lane: the Windows stress suites every night at 02:07 UK time
  (both GMT and BST, from two UTC schedules), and by dispatch any suite, `repro` (one named test,
  many runs, several copies at once, with every CPU kept busy) and the slot of the real-media
  campaign. A failure opens or updates one `loki-overnight-failure` issue; it is a finding, not a
  stop-the-line event like the nightly's.
- `tools/p14-campaigns/stress.cjs` gains the `batch-277` and `repro` suites and the `--until budget`
  mode. The `P14 stress` workflow runs the LOKI tools' tests on a pull request that changes them,
  since the overnight workflow never runs on one.
- Nothing runs until the maintainer has set LOKI up (the runbook) and registered the runner; the lane
  is unproven until its first nights.
- The hosted lanes are unchanged: the pull-request gate, the nightly and the release window do not
  depend on LOKI, and LOKI's result is never release evidence on its own (it is a Windows desktop,
  not a clean qualification host, and its runs are of the checked-out source).
