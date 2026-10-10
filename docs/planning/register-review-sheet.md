# Register review sheet: the known limits the public claims lean on

Status: **prepared 2026-10-04 by P14 PR 9a, for the maintainer to decide from in one pass.** This page
is a decision aid and decides nothing: every "proposed" below is a recommendation with its reason, and
nothing in the [register](known-limits.md) changes until the maintainer writes a review on the entry.
Three items the maintainer decided on 2026-10-04 (the macOS wording, the supported-versions policy and how
RQ-05 can pass) are marked below; **every register review is still `pending`.**

**Result, 2026-10-10: the maintainer made the pass, and every proposal below was taken as written.** The rows are left as they were prepared (some are out of date where they speak of
the candidates or of the rung `now`); the decisions are on the entries of the [register](known-limits.md), each with `accepted (2026-10-10, by the maintainer's register pass ...)`:

- **Part 1:** the 29 undecided entries are `accepted` (L-068 was already rescheduled to R1). The five rows that asked for a decision: **L-030** accepted for R0, the fix stays with #159; **L-058**
  accepted, no kernel series is pinned; **L-076** accepted as not supported in Codex's sandboxed mode on Windows for R0, the product question stays with #204 for R1; **L-095** accepted, the
  limit stands (the skill's rules met the gates once on the third candidate, a small sample); **L-098** accepted as untried and stated so, the try-out stays for after the release (L-143).
- **Part 2:** L-109, L-111, L-112, L-113, L-114 and L-115 are `accepted`. L-122 was accepted on 2026-10-05 and L-139 rejected (to be fixed) on 2026-10-07; neither changed.
- **Added since this sheet was written, reviewed in the same pass:** L-140 (the fix stays with #325), L-141 (#334), L-142 (the choice of a fix stays open under #340), L-143 and L-127 (the
  codes stay until v2) are `accepted`.
- **Part 3:** the readings of L-017, L-062, L-067, L-069, L-088 and L-090 are confirmed (their entries are `accepted`), and so are the two that are not register entries: the engine's direct
  dependency on `tokio`, and `lifecycle.mode` `durable_worker` for every session of a worker workspace. **The MSRV policy is left undecided:** it is needed before a crate is published, and R0
  publishes none.
- **Settled alongside:** CL-204's note and the support matrix's paragraph on agent clients now state the result of the repeat on the third candidate. **The `Guide` check is not made a
  required check on `main` for now:** it runs only when certain paths change, and a required check that does not run blocks a pull request; it would first need to run on every pull request.

Ten entries whose status was *open* are *accepted residual* now (L-022, L-028, L-030, L-042, L-043, L-067, L-076, L-113, L-141, L-142); the issues that track a fix stay open. No claim was
raised and the rung did not move: that is the ledger follow-up's (PR 13). 75 other entries of the register are still `pending`; no public statement leans on them.
Source: [P14 plan](p14-qualification.md) section 6 (the thirty entries) and section 10 ("Register and
readings"), [ADR 0024](../decisions/0024-r0-qualification-and-release-candidate.md) decision H.

## How to use it

1. Read a row, then open the entry (the ID links to it) only where the row says "decide" or you
   disagree with the proposal. Most rows ask for no more than "accepted".
2. Record a decision on the entry itself, as the register's
   [review workflow](known-limits.md#review-workflow) says: set its **Review** line to
   `accepted (YYYY-MM-DD)`, `rejected (YYYY-MM-DD): <reason>` or `rescheduled to <packet or issue>
   (YYYY-MM-DD)`, and update the summary table in the same change. A rejected entry must be fixed,
   so it needs an owner and an issue.
3. **What leaving an entry pending costs.** The claims that lean on it are not made
   ([P14 plan](p14-qualification.md) section 10), and since this change the Governance job holds
   that mechanically: each statement of the [claims registry](public-claims.json) lists the
   limits it leans on (`limits`), and a statement above the `now` rung that is in use fails
   `public-claims` while any of them is `pending` or `rejected`. Today the rung is `now` and no
   such statement is in use, so nothing fails; the pull request that moves the rung is where this
   sheet matters. An entry that is deleted must leave the lists it is in.
4. A reviewed entry is still a limit: **accepted** means "this stands as described and the public
   text says so", not "this is fine".

**Reading the columns.** *Leaned on by* names the public text and the registry statements whose
wording depends on the entry. *Proposed* is the author's recommendation, not a decision; *why, and what
changes if you decide otherwise* is the consequence for the public text. "Accept" is shorthand for
`accepted (<date>)` with the entry's status set to *accepted residual* where it said *open* or
*deferred*.

Three entries were out of date and were corrected in this change so that you review current text:
L-004 (it said decompression-bomb media had never been run, and RQ-10 has now run generated
variants), L-035 and L-038 (they described P14 as not yet having run the install steps and the
runbook).

## Part 1: the thirty entries of the plan

| ID | In one sentence | Leaned on by | Proposed | Why, and what changes if you decide otherwise |
| --- | --- | --- | --- | --- |
| [L-004](known-limits.md#l-004) | FFmpeg, FFprobe and whisper.cpp run with the user's own rights; on a desktop nothing sandboxes them. | README (the "not sandboxed" sentence links it); the matrix's list of what is not claimed; CL-201 to CL-203; CL-208 can never be used | **Accept** as a residual for R0; owner R1 | P14's malicious-media run (RQ-10) found 93 of 96 generated inputs inside their bounds and three CLI cases that PR 7 fixes. Containment is R1's work (with L-068). Rejecting it would mean R0 cannot ship a desktop profile without a sandbox |
| [L-007](known-limits.md#l-007) | A hostile recording can try to steer a model, and no tool makes every model immune. | README and skill guide trial sentences ("no run ... acted on injected text"); CL-204, CL-205 | **Accept** | Held by the trials' safety gate (zero out-of-policy actions). Re-run in batch 2 with the skill frozen |
| [L-008](known-limits.md#l-008) | Surviving an OS crash is shown on Ubuntu 24.04 with local ext4 and write barriers only; elsewhere a durable request is refused. | README ("durable sessions survive an OS crash on Ubuntu 24.04 with local ext4"); CL-207; the runbook; the matrix | **Accept** | The worker-profile half closes when RQ-11 (the two fault campaigns on the candidate) passes. CL-207 needs RQ-11 anyway |
| [L-020](known-limits.md#l-020) | On the one noisy clip the default model got 61.5% of words wrong; noisy speech is gated on critical terms only. | README ("measured on a synthetic corpus"); CL-209; the guide's limits page (9b) | **Accept** | The guide tells a reader that transcripts of noisy recordings can be substantially wrong. #150 (noisy fixtures) stays open |
| [L-021](known-limits.md#l-021) | Three reviewed misses of identifiers and numbers by the `base` model ("queued", "4407", "E-409"). | CL-209; the guide's limits page | **Accept** | Gates fail on any other miss; agents are told to confirm identifiers against the source |
| [L-022](known-limits.md#l-022) | No human voice, accent, crosstalk or long-recording speech evidence exists. | README ("every accuracy figure ... synthetic"); CL-209 | **Accept** for R0 (status open to accepted residual) | Real recordings are the post-R0 trial. If you reject it, R0 waits for licensed human speech fixtures (#150) |
| [L-028](known-limits.md#l-028) | Screen-change thresholds were calibrated on drawn video only. | README ("measured on a synthetic corpus"); CL-209 | **Accept** for R0 | The recall and false-change figures are worded as measured on the synthetic corpus. #175 (real recordings) stays open |
| [L-029](known-limits.md#l-029) | Sampling twice a second misses a change shorter than half a second, and coverage cannot report what sampling did not see. | README ("honest coverage" of what was analysed); the guide's limits page | **Accept** | The README claim is about analysed windows, which are reported; the guide says short flashes can be missed and a burst finds them |
| [L-030](known-limits.md#l-030) | The corpus draws no real motion (scroll, cursor, loading), so recall on such events is unmeasured. | README ("measured on a synthetic corpus"); the tutorial clip F04 contains the scroll event | **Decide**: accept for R0 and reschedule the fix (#159) to the post-R0 corpus work | The status is *open* (a defect). Accepting means no recall claim for scrolling or loading; the guide says so beside the tutorial |
| [L-035](known-limits.md#l-035) | Evidence on three systems exists for 0.1.0 only, on a synthetic corpus; no machine has met the matrix rules yet. | Every cell of the matrix; install.md section 1; README Platforms | **Accept** as the standing statement | True until the release. PR 13 deletes or rewrites it when the cells earn their words |
| [L-037](known-limits.md#l-037) | Managed install is shown on Ubuntu 24.04 x64 only, and its power-loss claim is for ext4 with stand-in versions. | README ("tested on Ubuntu 24.04 x64 only"); CL-004; CL-202 | **Accept** | RQ-06 and RQ-11 re-run on the candidate. A Windows or macOS managed install is not in R0 |
| [L-038](known-limits.md#l-038) | The worker host is a qualification target. | The runbook; the matrix; CL-208 (unusable) | **Accept** | Already consistent with decision E, option 4. The runbook was walked in RQ-12 (18 steps); the entry now says so |
| [L-042](known-limits.md#l-042) | Real-tool journeys run on demand and weekly, not on every pull request. | README Platforms ("on hosted test machines the published 0.1.0 has ... run"); the matrix | **Accept**; #178 stays open | The candidate and the release repeat the journeys on their own bytes |
| [L-043](known-limits.md#l-043) | The library API is 0.x and unstable; an MSRV policy and a local MCP adapter are undecided. | README ("stable, versioned JSON": the JSON, not the library); CL-002 | **Accept**; the two decisions are yours but block nothing in R0 | R0 publishes no crate. Decide the MSRV policy before any publication (R1) |
| [L-044](known-limits.md#l-044) | A newer VSift writes a session an older one may refuse; going back is not supported. | install.md section 7 | **Accept** | Sessions are disposable, and install.md says going back is not supported |
| [L-056](known-limits.md#l-056) | Durability is only as good as storage that honours flushes. | CL-207; the runbook | **Accept** | Say it in the runbook (it does) |
| [L-057](known-limits.md#l-057) | Losing the disk or the host loses the evidence; VSift keeps one local copy. | The runbook (restart and recovery); the matrix | **Accept** | Operators copy retained bundles to replicated storage |
| [L-058](known-limits.md#l-058) | The durable profile recognises Ubuntu 24.04 by `os-release` and the mount table, not by its kernel. | CL-207; the runbook | **Decide**: accept, and do not pin a kernel series | The entry's own reason: a pin refuses every security update. The campaigns run on the current hosted kernel weekly. Pinning would be a P14 change |
| [L-068](known-limits.md#l-068) | No evidence shows the strict profile contains an exploited decoder or a malicious provider. | The ban BAN-02; NC-902; the runbook; the matrix | **Already reviewed**: rescheduled to R1 (2026-10-03) | Nothing to decide. BAN-02 lifts only when RQ-14 is `passed`, and a waiver does not lift it |
| [L-072](known-limits.md#l-072) | Codex has no command allow list, so its trials are graded from its event stream, not blocked by settings. | The skill guide's "installed nothing" sentences for Codex; CL-205 | **Accept** | Held by the grader failing any attempt, run or not |
| [L-075](known-limits.md#l-075) | Codex's image views are not in its stream, so its image budgets are unmeasured. | CL-205; the skill guide | **Accept** | Held by the check code and the tool-call budget |
| [L-076](known-limits.md#l-076) | Codex's Windows sandbox cannot run VSift (its restricted token and VSift's private folders are incompatible). | BAN-07; README Platforms; the matrix; the skill guide | **Decide**: accept as documented unsupported for R0, and reschedule the product question (#204) to R1 or leave it | The status is *open* (a product defect). Accepting records "not supported in sandboxed mode"; the three options for a fix are in the entry |
| [L-082](known-limits.md#l-082) | Claude Haiku 4.5 does not follow the full investigation procedure (6 of 28 answers right). | The skill guide's model table; CL-204 | **Accept** | The tier is Claude Sonnet 5.5 and GPT-6-Sol |
| [L-083](known-limits.md#l-083) | Only `display_text` shows hidden characters; `text` and `original_text` stay raw by design. | The skill guide; the guide's concepts page | **Accept** | No counted report held a raw hidden character in the final rounds |
| [L-084](known-limits.md#l-084) | GPT-6-Luna is below the compact line (19 of 28 answers right). | The skill guide's model table; CL-205 | **Accept** | As L-082 |
| [L-095](known-limits.md#l-095) | Review-tier models stated blurred text as seen in pixels; the skill fix was re-measured in batch 2 on both candidates (2026-10-05 and 2026-10-07): GPT-6-Astra 3 of 3 twice, Claude Opus 5.5 1 of 3 twice. | The skill guide ("Not yet done"); CL-204 and CL-205 (review tier) | **There was a miss** (the row said "reopen on a miss"), and you decided it with RQ-15 on 2026-10-07: no waiver and no exclusion; the skill is improved and batch 2 is run again on a third candidate, `0.2.0-rc.3` ([L-139](known-limits.md#l-139)). The entry's own review is still yours and waits for that repeat | Gate: at least 2 of 3 per client with no claim of the blurred text as supported by pixels. Met by Codex twice, missed by Claude Opus twice; RQ-15 is `failed` for `0.2.0-rc.2` |
| [L-097](known-limits.md#l-097) | A publish that fails part-way leaves part of the release public until a re-run. | `release.md` (scanned) | **Accept** | The re-run path is written; it has not run against the real registry |
| [L-098](known-limits.md#l-098) | The Windows and macOS executables are unsigned; SmartScreen, Gatekeeper and Smart App Control are untried. | BAN-04; install.md section 4; README ("not code-signed or notarized"); the matrix | **Accept as deferred until RQ-17**, then decide (decision C's trigger) | Review after the try-outs: an observation, whatever it shows, is what the release needs |
| [L-099](known-limits.md#l-099) | Managed install depends on three publishers' files and redirect hosts. | README Quick start; CL-004; CL-202 | **Accept** | The weekly managed smoke shows drift; `--artifact-dir` and the bring-your-own route stay |
| [L-100](known-limits.md#l-100) | npm prints only `ENEEDAUTH` when a trusted publisher is wrong. | `release.md` (scanned) | **Accept** | Maintainer-only; the runbook's preflight covers it |

## Part 2: entries added since the plan that the claims also lean on

The plan chose the thirty before P14 PRs 2 to 7 added L-109 to L-132. The matrix and the install guide now
cite the seven below (and since 2026-10-07 an eighth, L-139, the last row), so a statement that leans on them is held by the same rule. Reading them in the same
pass costs a few minutes.

| ID | In one sentence | Leaned on by | Proposed | Why, and what changes if you decide otherwise |
| --- | --- | --- | --- | --- |
| [L-109](known-limits.md#l-109) | npm's and pnpm's `vsift.cmd` lets `cmd.exe` re-read a command line. | SECURITY.md (known issue); install.md section 2; the launcher's README | **Accept** (already an accepted residual) | VSift cannot change a file npm writes; the three safe routes are documented and pinned by a test |
| [L-111](known-limits.md#l-111) | The upgrade evidence has one published baseline, so 0.1.0 over 0.1.0 shows the procedure, not a newer release reading older data. | install.md section 7 | **Accept as deferred** | The candidate's run from 0.1.0 is the evidence; install.md already says what was run |
| [L-112](known-limits.md#l-112) | A hosted image with named programs hidden is not a clean machine. | The matrix ("what the hosted evidence is not"); install.md | **Accept** | Real machines are your try-outs (RQ-17) |
| [L-113](known-limits.md#l-113) | The durable stage of the worker checkpoint cannot run on a hosted runner, so RQ-05 could not be `passed` under its first rule. | The matrix (the RQ-05 paragraph); CL-201, CL-202, CL-203 all need RQ-05 | **Decided 2026-10-04 (the item below, option b);** accept the entry as a residual | The rule is per system now; see "The decision that blocked the cells" below. The entry's own review is still yours |
| [L-114](known-limits.md#l-114) | macOS is tried with Homebrew's FFmpeg and whisper.cpp, which VSift does not review. | The matrix; CL-203; install.md | **The wording was decided 2026-10-04** (CL-203 as registered); accept the entry | If the candidate's macOS runs fail, the cell stays a qualification target. The entry's own review is still yours |
| [L-115](known-limits.md#l-115) | The journeys run later tests against 0.1.0 and do not reach the launcher or the archives; a stage that asserts a fix newer than 0.1.0 is skipped below its first version, visibly. | The matrix | **Accept** | The candidate's tag carries the override, so its journeys use their own tests; the skip rule is in the development guide |
| [L-122](known-limits.md#l-122) | One FFmpeg record (CVE-2026-38350) is tied to its fix only by elimination. | No sentence of a public document today; RQ-13 cannot be `passed` until you decide (#272) | **Decide** (already on your list) | Accept the tie by elimination as the register entry, or ask upstream which report it describes. A refreshed build changes nothing (L-132) |
| [L-139](known-limits.md#l-139) | Claude Opus 5.5 did not meet two review-tier gates of batch 2 on either candidate (a negated statement about a blurred banner rated as supported by the frame; narration cited outside the truth window for a restated fact). | The matrix (the agent-clients paragraph); CL-204's note | **Already rejected by your decision of 2026-10-07 on RQ-15:** it is fixed in the skill and re-run on a third candidate, not waived and not excluded; nothing to decide in the pass until that repeat | CL-204 and the cells that need RQ-15 can be used only if Claude Opus 5.5 meets both gates on the third candidate (plan 27.5). If it misses again the question of this row comes back |

## The decision that blocked the cells (decided 2026-10-04: option b)

**Resolution.** The maintainer chose (b) on 2026-10-04: record that other evidence covers the stage, and word
RQ-05's pass rule per system. The rule is in the ledger and the plan (section 21). It is no longer unsatisfiable,
but RQ-05 stays `running` for 0.1.0 for a different reason: the P07 gates on Ubuntu and Windows are recorded only
as prior evidence, from weekly runs on source at other commits. The cell statements stay unusable until every item
they need is `passed` for the release candidate's own bytes, which no 0.1.0 result can do. The register reviews below
are untouched by this decision: all stay `pending`, and they remain the maintainer's to write.

The question as it was put to the maintainer follows, unchanged.

RQ-05's pass rule is "every stage passed". For 0.1.0 every stage that can run did, and P11's
`p11_durable_workspace` was blocked on every system (L-113): by design off Ubuntu, and on the hosted Ubuntu
runner because its disk is mounted without write barriers. Every cell statement (CL-201 to CL-203) requires
RQ-05, so none can be used while it is `running`. Three ways out, for the maintainer; the recommendation is (b):

- **(a)** Run the stage on the candidate inside the virtual machine the P10 campaign boots, with the
  installed executable. It keeps the rule as written and costs harness work in PR 11.
- **(b)** Record what covers the stage and word the pass rule per system. The published binary has already
  published durable worker requests on a real ext4 volume with write barriers (an image file mounted in the
  runner) with `os_crash_durable` publication: the load campaign (RQ-09) and the runbook walk (RQ-12) did it
  for 0.1.0 and rerun on the candidate. The durable stage then reads "Ubuntu only; covered by RQ-09 and
  RQ-12", and RQ-05 passes when the rest does. It changes a pass rule of the plan, so it is your decision.
- **(c)** Leave RQ-05 `running`, and make no cell statement. Not recommended: the cells are the point of
  the release.

## Part 3: the P11 and P13 readings

Choices the implementation pull requests made and flagged for the maintainer ("Operational readings" in
[`p11-worker-host.md`](p11-worker-host.md) and "Readings awaiting the maintainer" in
[`p13-distribution.md`](p13-distribution.md)); none blocks the release, and no public sentence depends on
one. Each row is a reading to confirm or overrule.

| Reading | What was chosen | Proposed |
| --- | --- | --- |
| [L-017](known-limits.md#l-017) | Human mode prints only the final result of long commands and of worker hosts; progress and lifecycle events exist only as JSON Lines. | **Confirm.** A progress line on stderr can come later without a contract change. The guide tells readers that `transcript retranscribe` is silent until it ends |
| [L-062](known-limits.md#l-062) | A worker request's input path may not go through any link (symbolic link, junction, hard link beyond one). | **Confirm.** The stricter rule removes every way out of the input root; operators copy or bind-mount |
| [L-067](known-limits.md#l-067) | Requests of one batch contend for the workspace's units and locks, and a request cancelled by `job cancel` makes the batch exit 6, like a shutdown. | **Confirm the exit reading**: the exit status is the failure class's (ADR 0008); a supervisor tells the cases apart by `termination_reason` |
| [L-069](known-limits.md#l-069) | A request that failed for good because of the host (a missing tool, a full disk) replays that failure; resubmit under a new operation id. | **Confirm.** A replay must never change what an acknowledged id means; the runbook's table says what to do |
| [L-088](known-limits.md#l-088) | A proxy's `407` inside an HTTPS tunnel is recognised by the end of a dependency's error text. | **Confirm.** A test on every system fails if the text changes; the failure is safe either way |
| [L-090](known-limits.md#l-090) | Content in the managed folder that VSift cannot prove its own is left for the user to delete. | **Confirm.** Deleting what cannot be proved risks deleting user data (AGENTS.md) |
| `tokio` | The engine crate depends on `tokio` directly, for the batch's task set and channels ([ADR 0021](../decisions/0021-worker-and-batch-host.md), PR 4 note). | **Confirm.** It was already a dependency of every layer below, so no new crate entered the lock file; `cargo deny` still reviews it |
| `durable_worker` | Every session of a worker workspace reports `lifecycle.mode` `durable_worker`, even when the workspace is ephemeral; `publication` says how it publishes. The related retention reading (a renewal at the maximum retention changes nothing) is in ADR 0021 (PR 2). | **Confirm.** The name is in the published v1 JSON, which is additive-only since 0.1.0, so renaming it would be a break |
| MSRV | There is no minimum-supported-Rust policy; the toolchain is the latest stable ([L-043](known-limits.md#l-043)). | **Decide before any crate is published.** R0 publishes none, so this blocks nothing |
