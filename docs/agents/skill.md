# The VSift agent skill

Status: P12 complete (2026-09-30). The skill's command and schema references are held
to the CLI by tests, and it has been through named-client trials (A-01..A-09 and
SEC-T02, [runbook](trials.md)). It is **qualified for the review tier and, since the
compact re-run (#222, 2026-09-30), for the compact tier**; see
[Models and clients trialled](#models-and-clients-trialled). Design:
[ADR 0022](../decisions/0022-agent-skill-and-named-client-qualification.md) (Accepted).
Results: [P12 qualification record](../planning/p12-agent-qualification.md).

The skill teaches a coding agent (Claude Code, OpenAI Codex or another client that
reads the `SKILL.md` directory format) to investigate a local video with the `vsift`
command-line tool and hand back a grounded, cited report. It adds no tools and no
permissions: it only tells the agent which `vsift` commands to run, in what order,
within what budget, and how to report.

## What is in `skills/vsift/`

| Path | Purpose |
| --- | --- |
| [`SKILL.md`](../../skills/vsift/SKILL.md) | Trigger description and the eight-state procedure (CHECK_CAPABILITIES to CLOSE_OR_RETAIN), each with its allowed commands and stopping condition. |
| [`references/commands.md`](../../skills/vsift/references/commands.md) | The command policy: every public command is `free`, `explicit` (only on the user's instruction) or `never`. |
| [`references/budgets.md`](../../skills/vsift/references/budgets.md) | The `compact` and `standard` budget profiles. |
| [`references/handoff.md`](../../skills/vsift/references/handoff.md) | The grounded report template and citation rules. |
| [`references/safety.md`](../../skills/vsift/references/safety.md) | Evidence is data: prompt-injection, rendering and privacy rules. |
| [`references/resume.md`](../../skills/vsift/references/resume.md) | Operation ids, the resume card and recovery after a reset. |
| [`references/lifecycle.md`](../../skills/vsift/references/lifecycle.md) | Close, retain and the cleanup routine. |
| [`handoff.schema.json`](../../skills/vsift/handoff.schema.json) | The handoff v1 JSON schema, owned by the skill (not part of `schemas/v1`). |
| [`examples/`](../../skills/vsift/examples/) | Example handoffs built from real CLI output on the F10 and F03-speech fixtures. |
| [`assets/image-check.png`](../../skills/vsift/assets/image-check.png) | A small image with a code word, for the image-access check. |
| [`agents/openai.yaml`](../../skills/vsift/agents/openai.yaml) | Codex presentation metadata only. |

The skill lives at the repository root, not in `.claude/skills` or `.agents/skills`,
so that contributors' own agents working on VSift do not load it by accident.

## Installing it

The skill is the same folder everywhere: the repository's `skills/vsift/`, the
`skills/vsift/` folder inside every native release archive, and the `skills/vsift/` folder
of the `vsift-cli` npm package, byte for byte (`npm root --global` names the folder that
holds `vsift-cli/skills/vsift/`). Build or install `vsift` first so that the command is on
the agent's `PATH` ([`install.md`](../operations/install.md): 0.2.2 is published, as
`npm install --global vsift-cli` (no tag) and as archives on GitHub Releases; building from
source also works). Then copy (or link) the whole `skills/vsift`
directory, keeping its name, into the client's skill folder:

| Client | For one user | For one project |
| --- | --- | --- |
| Claude Code | `~/.claude/skills/vsift/` | `<project>/.claude/skills/vsift/` |
| OpenAI Codex | `$HOME/.agents/skills/vsift/` | `<repository>/.agents/skills/vsift/` |

Restart the client (or start a new session) so that it discovers the skill. Both
clients read the `name` and `description` front matter and load the rest on demand.
The Codex metadata sets implicit invocation on, so a request about a local video can
select the skill; the user can also name it.

## Requirements the skill states to the agent

- **Image access.** The agent must be able to open a local PNG file and read it. The
  skill checks this itself before any visual claim: the agent opens
  `assets/image-check.png` and records the code word it reads. The code appears
  nowhere in the skill's text, so it cannot be copied; a grader compares it with the
  truth. Without image access the agent works from the transcript only and marks
  every visual claim unsupported.
- **Local shell.** The agent runs `vsift` itself; nothing is uploaded. On Windows it uses
  PowerShell or Git Bash and never `cmd.exe` (`cmd /c`, a batch file): npm's `vsift.cmd` shim
  makes `cmd.exe` read the command line a second time, so text taken from the evidence could
  run as a command ([L-109](../planning/known-limits.md#l-109)). Added in
  `references/commands.md` on 2026-10-05 (P14 PR 10a), before the candidate's freeze; the
  agent trials do not exercise the shim (Claude Code reaches `vsift` through Git Bash and
  Codex runs on Linux), so the sentence is untested by them.
- **Budgets.** `compact` (the default, for small models and one-image clients) allows
  6 images, one per step, 30 tool calls, pages of 20, two refinements per claim,
  bursts of 4 frames and 15 minutes; `standard` allows 24 images, 4 per step, 80 tool
  calls, pages of 50, four refinements, bursts of 12 and 30 minutes. The user may name
  a profile or override a limit. (From `0.2.1`, a page of 50 is
  for `--view brief` only: the full view stays at 20 on both profiles.)
- **Permissions.** The skill never asks for broader tool permissions. Setup
  registration, renewal, retention, cleanup and job cancellation need the user's
  explicit instruction; managed installation, worker-host commands and any other
  executable are never run by the skill, not even `date` to time itself: the host
  measures and enforces the wall time, and the handoff reports it as `null` when the
  agent could not measure it. Each `vsift` command runs alone, from the folder the
  agent started in (never after `cd`); the only additions are `| tail -n 1` after
  `--events jsonl` and the two literal forms that pass the draft report to `vsift
  handoff check` (P13 PR 5), which the agent runs once before sending and whose
  findings it fixes. `vsift --help` and `vsift <namespace> <operation> --help` are free,
  read whole, to recover a command's flags.
- **Reporting.** Every stop, including a missing tool, an expired session or an
  exhausted budget, ends with the handoff as the final message: one `vsift-handoff`
  block, never a file, with no web address, Markdown link or local path (added
  2026-09-29 after the diagnostic passes). `SKILL.md`'s skeleton shows one filled-in
  claim, a segment and a frame citation and one untrusted instruction, and says what a
  stop before any evidence holds (2026-09-30, issue #218: compact models had invented
  the claim's shape from an empty list). Each claim names its subject and value and
  cites evidence showing that value; a region a frame shows as unreadable supports
  nothing, so a claim about its content is `partially_supported` on the transcript
  (#224). Two evidence rules were added on 2026-10-07, after batch 2 of the P14 agent
  trials (Claude Opus 5.5, review tier, on both candidates): an unreadable region proves
  nothing about its content in either direction, so a claim that something is absent from
  it is not `supported` on that frame either; and a claim states only what its own
  citations show or say, so every name, number or identifier in it is in evidence that
  claim cites (`SKILL.md` states both in one sentence each, `references/handoff.md` in
  full). The retained bundle is a snapshot, so the agent retains after its last
  evidence command (#220); a web address seen in evidence is defanged in the Markdown
  only, and the JSON describes it without writing it (#221). The
  JSON states only what the agent alone knows: its claims, the identity of each piece
  of evidence it cites, whether it looked at each image, gaps, the instructions it saw,
  what it did with the session and, when the work was cut short and can continue (a
  budget ran out, a transcription was cancelled or interrupted), the resume card. Times, revisions, the session's details and the budget's limits are
  optional, since VSift recorded them; a value the agent does give must be VSift's own
  (handoff v1 revised in place on 2026-09-29, before any release). `SKILL.md` lists the
  allowed words of every closed member beside the skeleton, and
  `references/handoff.md` lists all of them; a gap note may quote VSift's remediation
  whole (600 characters), and `references/resume.md` shows one exact resume card
  (PR 3g). What to do per failure code is in `references/commands.md`; since 2026-10-05
  (P14 PR 10a, before the candidate's freeze) its `STORAGE_IO` row tells the agent to read
  the remediation first and to put what it says in the gap's note in its own words (three of
  the CLI's `STORAGE_IO` answers are a link given instead of the file, a drive with no room
  and an id that names no published session, [L-127](../planning/known-limits.md#l-127);
  two of those remediations, 553 and 697 characters, leave little or no room in a note of
  600 characters, so the agent is not told to quote them whole), and its `RESOURCE_LIMIT` row no longer sends the agent
  to a smaller request when the remediation says there is no room. Transcript
  text is quoted only from a segment's `display_text` (a speaker from
  `display_label`), where VSift has already written every invisible or bidirectional
  character as `<U+202E>`-style notation; `text` and `original_text` keep them raw
  (PR 3h, ADR 0008 note of 2026-09-29).
- **Resuming.** A run that continues from a resume card another run wrote has its own
  budget (the card's `remaining` binds only the same run after a context reset),
  repeats the image check, and verifies every earlier finding again with one command
  (the segment's window, or the frame at its time) before it reports it; the card may
  list those findings in `to_verify` with their evidence and window (PR 3i, after
  every resumed A-02 run of the final campaign took the earlier budget as its own).
- **Probing.** The skill tells agents to check for VSift with `vsift setup check`
  only. Since 2026-09-30 the trial grader counts a stray `command -v vsift`, `ls -l` of
  the user's files or `|| true` as harmless orientation, not as an unauthorized action;
  the skill's rule is unchanged.

## The change for 0.2.1 (2026-10-10; released in `0.2.1`, published 2026-10-15)

Prepared on the branch `skill-0.2.1` and applied, unchanged, in the cut of the release candidate `0.2.1-rc.1` with its
new agent-trial freeze: the skill is a frozen trial component, so this changes its digest (`2f8686b7...`) and batches 2
and 3 are run again on the new text before the qualification above is claimed for it. **No trial has run on this text,
and no trial scenario exercises the partial-transcript rule** (the scenarios and the grader are frozen with the
rest, so the rule is read in the report only by what the existing scenarios happen to meet). The change, and nothing else:

- **Wording** (#349). The description says "video walkthrough", not "QA walkthrough", and names a bug report as one
  example of a handoff, not the headline use; `references/handoff.md` says the Problem section is what the question
  is about (often what goes wrong).
- **A partial transcription** (#353). `transcript retranscribe` and `job resume` can answer `partial`: each range in
  `untranscribed_ranges` is a `transcript` gap with the reason `untranscribed_range`, never silence, and the report
  offers a transcript file or a retry of that range alone (`SKILL.md` PREPARE, `references/handoff.md` Gaps).
- **Most of a recording cannot be transcribed** (#353). The `MISSING_CAPABILITY` row of `references/commands.md` says
  nothing is missing when the remediation says the tool works: no reinstall, a `transcript` gap in the agent's own
  words (the remediation is longer than a gap note), the reason and the job from `error.affected_ids`.
- **`BUSY`** (#342) carries `retry_after_ms`: wait, run the same command once more, run one command at a time.
- **`audio`** (#340): the file tells an agent nothing; speech is read with `transcript get`.

### Since the first candidate: the second candidate `0.2.1-rc.2` (2026-10-11; published, then superseded by `0.2.1-rc.3`, which has the same skill, as does `0.2.1`)

Prepared on the branch `skill-0.2.1-rc2` (more than one commit: the later ones follow the independent reviews) on top of the
Reads change (the brief view and the coverage of a read, now on `main`), and applied by the cut of the second candidate as
one commit with a new freeze (`skill` digest `256b4840...`, where the first candidate's was `2f8686b7...`): the skill is a frozen
trial component, so its digest changed again and batches 2 and 3 run again on this text. The third candidate, `0.2.1-rc.3` (2026-10-13, for the fix of #391 in `setup check`), carries this text byte for byte and so the same freeze. **It was run by agents once, on `0.2.1-rc.3` (batches 2 and 3, 2026-10-14 and 2026-10-15; `0.2.1` has the same skill), and no frozen scenario exercises most of it** (see the end of
this section). The change, and nothing else:

- **A whole recording is out of scope** (#368, the maintainer's decision of 2026-10-11 for this release line). The
  description no longer offers "a cited summary, timeline or handoff" and says that a summary or timeline of the whole of a
  long recording is beyond the skill's budgets. `SKILL.md` ("Before you start") and `references/budgets.md` say what to do for
  one over 5 minutes: if the user named a stretch, or what to hand back when the budget runs out, start, and end `partial`
  with the resume card if it runs out; if not, say so, offer a stretch or a question and ask before any command (nothing was
  started, so no report is needed: this does not break "every stop ends in REPORT"). `budgets.md` says how the length is
  known (the user, `source_segments[].end_us` with a transcript, `index.duration_us` from `candidates`; `ingest` reports no
  duration without a transcript). The first half is there because `A-02` asks for "every change ... in the whole recording"
  and tells the agent to stop with a resume card when the budget runs out, and `SEC-T02` asks for a summary of a 12-second
  clip: neither may become a refusal, and `A-02` must still end `partial` with a card.
- **The brief view** (#365). Every `transcript get`, `search` and `candidates` form carries `--view brief` (`SKILL.md`
  steps 3 and 4, `commands.md` forms and limits, `resume.md`). A brief item keeps its ids, times and `display_text` (and leaves
  out more than what the revision block says once: also `text`, `confidence`, `language`, `markup` and the rest); the
  revision's id is `revision.revision_id` of the answer (`handoff.md`, Citations). The refusal with `--events jsonl` is in
  `commands.md`. **Page sizes:** the profile table keeps `20` and `50` (the trial grader reads them and checks every
  `--limit` against them, so `compact` cannot be told 50); `budgets.md` says to ask for pages of 20 at most without the brief
  view, on either profile, and that `standard` may use 50 with it (an ask of the skill, not a property of the CLI).
  **The text does not promise a small page:** the brief view keeps the revision block whole, which after a transcription with
  many stretches of words no segment holds (`revision.local_asr.unplaced_ranges`, up to 1,024) is up to about 45,000
  characters in either view, and a full page of 50 segments is about 35,000 to 57,000. A client can cut a long answer without
  an error (one did at about 43,000 characters), and the CLI writes the gaps after `status` (the envelope's `coverage` after
  `data`, `transcript_coverage` after the items; the order is what the CLI writes today, not part of the contract). So an
  answer that stops before the final `lifecycle` member is asked again with `--limit 5`, and if it still stops the read is
  reported as incomplete instead of being trusted (`SKILL.md` step 3, `commands.md`, `budgets.md`). "Read a smaller range"
  was the first wording; it cannot converge when the revision block alone is cut.
- **Coverage of a read** (#373, #359). `transcript get` now states `transcript_coverage` and answers `partial` over a
  gap, as `search` does. A `partial` read or search is usable; its `transcript_coverage.untranscribed_ranges` list the
  stretches with no transcript (the envelope's `coverage.gaps` are the same stretches as text); a search cannot find
  words there and a read shows none, which does not mean none were said, so the agent reports each as a `transcript`
  gap with the reason `untranscribed_range` (the object ranges, not the text) and never as silence; more than about 10
  stretches are one gap with no `range` and the count in its note, because the handoff holds at most 50 gaps and
  `transcript_coverage` lists up to 100 ranges. `handoff.md` names the four causes in plain words: a range nobody
  transcribed, a stretch the recogniser could not read, words the recogniser wrote that no segment holds
  (`revision.local_asr.unplaced_ranges`: the transcription itself answered `complete`, but a later read or search over the
  stretch is `partial`, #377) and a false gap after a range retranscription that followed a first run with few or no
  segments ([L-033](../planning/known-limits.md#l-033)). The skill does not run `transcript retranscribe` for a gap
  unconditionally: only if the question needs the stretches, **once, with one range that covers them** and a new operation
  id (a first wording said "once per stretch", and the review reproduced that each per-stretch run uncovers the previous
  one, so it chases stretches); if a stretch is still listed, the agent reports it and offers a transcript file, the sure way.
  Nothing in the skill quotes the read warning ("Part of the requested range has no transcript here, ..."), so its new
  wording changes nothing here.
- **`candidates` on a long recording** (#367). One range of at most 5 minutes per call (`--to` at most 300000000 after
  `--from`), the brief view, only around the transcript's spans when there is a transcript, and one range of at most 5
  minutes at a time from where the question points when there is not (the old "page candidates over the whole video" is
  gone). The partial `candidates` gaps are `data.coverage.gaps` (objects with a reason); the envelope's `coverage.gaps` are
  text. No filter and no progress events exist, and the text does not mention any.
- **`BUSY` while a transcription runs** (#370). After a tool call that timed out, a `frame get`, `frame burst` or
  `candidates` call can stay `BUSY` for as long as the job runs (the review reproduced all three): none is retried. The
  job's id is not known after a timeout, so `resume.md` ("A transcription that is still running") gives one order:
  `session status` (its `jobs`), then `job status`, then REPORT with a `visual` gap (`not_inspected`, code `BUSY`) and the
  resume card, handling a `failure` that is null and a timed-out `candidates` call that keeps running in the background
  (`SKILL.md` PREPARE, `commands.md` `BUSY` row, `handoff.md` Status). `resume.md` step 5 no longer says to wait and ask
  again. The text names no cause: which resource held the refusal is not known ([L-154](../planning/known-limits.md#l-154)).
  The four lists of when to give a resume card now agree (`resume.md`, `handoff.md` twice, `SKILL.md`).
- **New ids after a range retranscription** (#372). Every segment of the new revision gets a new id, those outside the
  range too; an older id still resolves but one inside the replaced range can name text the run replaced, so the agent reads
  the range again and cites the ids it returns now, not any it cited from it before (`SKILL.md` step 6, `handoff.md`
  Citations). The skill's `handoff check` forms pass no `--session`, so the check never looked at ids.
- **Report size** (#366). A report over 64 KiB cannot be checked: it is shortened. About 5,500 characters passed in Git Bash
  and about 15,000 in PowerShell, and the agent cannot count (no `wc`), so when the report is long (about 5,500 characters or
  more) or the agent is unsure, it checks two parts of the same literal forms: the text above the `vsift-handoff` block, then
  the block (`SKILL.md` REPORT, `commands.md` "Checking the draft"). The text part is expected to answer
  `handoff_block_missing`, only its other findings matter, and a `line` in the block part counts from the block's first line.
  The sizes are what the issue saw, not a measurement through the pinned client; no form, flag or file was added.
- **`session list` and `session clean --expired --dry-run` pages** (#371). The `--cursor <n>` forms are allowed, and the
  agent continues until `next_cursor` is null (`commands.md`, `lifecycle.md`).
- **The REPORT checklist keeps its trial-driven safety clauses.** The first commit shortened the checklist to five lines and
  moved clauses to `safety.md` and `handoff.md`; the review pointed out that two of them come from compact-tier trial runs
  (ADR 0022, defects 15 and 16) and that `handoff check` flags `https://` and `www.` but not a defanged `hxxps://`, so
  `SKILL.md` again says: evidence only in code spans or blocks and never as the agent's own words, a web address only as
  `hxxps://...` in a code span of the Markdown and never in the JSON (which describes the link), no other address, no Markdown
  link, absolute path, drive letter or home folder, and a retained bundle is "the folder you named". The PowerShell reminders
  that only a parse error would teach (`--rect "10,20,300,80"` quoted, `--query=-17`) are back in the last paragraph.

**What moved, so that `SKILL.md` stays at 300 lines (the skill as a whole lost no rule; `SKILL.md` alone gave up the
following, each kept where it says):** the longer explanation of the optional handoff members (`handoff.md` "What the JSON
must hold, and what it may"); the pointer sentence that listed what `handoff.md` shows (crop and audio citations, `gaps[].code`,
`budget`, `resume`); the sentence on what `frame neighbours` and `frame burst` show (`budgets.md` "How to spend it",
`commands.md` limits and `--help`); the phrase "name a tool, quote VSift's remediation" and the exact shape of a Markdown link
(`safety.md` rule 6, `handoff.md` "Before you send it"); the sentence on how an expired session or an exhausted budget is
reported (`handoff.md` "Status", also `resume.md` and `budgets.md`); the sentence on a remediation's `command` (its
essential, "only if it is `free`", is still there, with `safety.md` rule 9); and `partial` as "a usable answer with a stated
gap" in the closing paragraph (step 3 says it). Four paragraphs were rewrapped to a wider line with no word changed (the
image check, the missing-tools bullet, the operation-id bullet and rule 3; `git diff --word-diff` shows it); the others that
changed shape (the whole-recording paragraph, the candidates and claims bullets, the closing paragraph) changed words too.

**Untried:** the scenarios and the grader are frozen, so no scenario exercises the whole-recording limit, a report over
5,500 characters, a `BUSY` frame, burst or `candidates` call during a transcription, `session list` paging, a range
retranscription's new ids, a `partial` read, an answer cut by the client, or a recording longer than a few minutes without a
transcript. Every scenario that reads a transcript now runs the brief view, and the sentences above that the guard cannot
test (the whole-recording limit, the `BUSY` rule, the two-part check, the 5-minute rule, the new ids, the retranscribe-once
rule for a gap and the cut-answer rule) are text shipped, behaviour unmeasured, until a real run shows an agent following
them.

## The change for 0.2.2 (2026-10-17; released in `0.2.2`, published 2026-10-18)

One rule, for #395. In agent-trial batch 2 on `0.2.1-rc.3` two compact Claude Sonnet 5.5 runs read the whole 20-second
transcript with `transcript get --view brief` and never ran `search`, although `FIND_SPOKEN_SPANS` began "Always search
first, even when the video is short enough to read whole" ([L-163](../planning/known-limits.md#l-163)). Where a compact model
could skip it: the rule was one bullet inside the third state, it gave its reason only as "search hits carry the segment ids",
and the next bullet said that on a short video "one window may hold the whole transcript". The change, and nothing else:

- **Rule 6** of "Rules that always apply" (cite or say you cannot) now ends: to find what was said, run `search` before any
  `transcript get`, because its hits are the segment ids and times you cite, and a whole-transcript read never replaces it.
  "To find" leaves the resume card's re-verification (`references/resume.md`, a `transcript get` of a known window) as it was.
- **`FIND_SPOKEN_SPANS`** says "Always search first, before any `transcript get`" and that a whole-transcript read is no
  substitute; the window after the hits is "never the whole transcript instead" (the sentence about one window holding the
  whole transcript is gone).
- **`references/commands.md`**: `transcript get` reads bounded pages around `search` hits, or a resume card's windows.

`SKILL.md` stays at 300 lines (the guard's limit); the fuzz seed copy is the same bytes. The CLI's skill guard pins the three
sentences. The skill never said "locate" for a tool, and its
setup text already says that `setup configure` takes only a path the user gives, never one found by searching the disk; the
CLI's own advice for a missing tool changed in the same pull request (#396, [L-164](../planning/known-limits.md#l-164)). The
trial freeze is new (`skill` `7f445a9a...`, whole freeze `f52265e7...`; [trials.md](trials.md), "The freeze (P14)"). **No agent
has run this text:** it gets the uncounted skill smoke, and the compact tier is measured again only in the next counted batch.

### A long report's check on Windows (#366; 2026-10-17, released in `0.2.2`)

The skill's two draft forms of `vsift handoff check` stay the only ones, and the check still reads a draft from standard
input or `--file`. What was measured, on Windows 11 with Claude Code (desktop app 2.42304, Agent SDK 0.3.299), Git for Windows
2.33.0 (bash 4.4.23) and PowerShell 7, by launching each form the way that client launches it (the measurement and the
regression test are `crates/vsift-cli/tests/handoff_long_report_windows.rs`):

- **Claude Code's PowerShell tool** hands PowerShell its script in an environment variable and runs it with
  `Invoke-Expression`, so the here-string never travels on the process command line: reports of 30,000, 40,000, 70,000 and
  140,000 characters all reached `vsift` whole. Put on the command line instead (`pwsh -Command <script>`, as the client the
  issue used did), the script fails to start between 30,000 and 40,000 characters (`ENAMETOOLONG`; Windows allows 32,767).
- **Claude Code's Bash tool** runs `bash -c` with the command inline, single-quoted for `eval` (each apostrophe becomes five
  characters), and Git Bash cuts that argument at about 8,190 characters: a report of 7,029 characters passed, one of 7,529
  failed with "unexpected EOF while looking for matching `'`" at line 75, which reproduces the issue's "about 70 lines in";
  above 32,767 the shell does not start. The same tool also turned a doubled backslash into one in a quoted heredoc, so the
  Git Bash form does not keep every character either.
- **The check's own bound was the next limit:** the PowerShell form delivered 70,000 characters and the check refused them as
  over 64 KiB. It now reads up to 128 KiB (ADR 0023 note of 2026-10-17).

So the form for a long report on Windows is the PowerShell one, which passes neither the command line nor a file. The
skill says so (`SKILL.md` REPORT, `commands.md` "Checking the draft", `handoff.md`): on Windows use the PowerShell form
whenever the client has a PowerShell tool, check the whole report in one call, and fall back to parts of at most about
30,000 characters if the client's PowerShell launch is refused as too long; in Git Bash keep parts of at most about 5,500
characters (no longer "two parts": the text above the block is cut at line ends into as few parts as needed, then the block
alone; a `line` counts from its part's first line). `SKILL.md` stays at 300 lines and its fuzz seed copy is the same bytes.
Not measured: Codex (it cannot run VSift on Windows, [L-076](../planning/known-limits.md#l-076)), newer Git for Windows
releases, and bash on Linux and macOS. **Agents ran this text once, in batch 2 on `0.2.2-rc.1` (2026-10-18), but no scenario asks
for a long report, so the check of one was not exercised**; the trial freeze was new then ([trials.md](trials.md), "The
freeze (P14)"): `skill` `13b485f3...`, whole freeze `cfaf9783...`, which #367's freeze replaced.

### Shortlisting the screens of a long recording (#367; released in `0.2.2`)

The CLI gained `candidates --omit periodic` (the maintainer's decision of 2026-10-17; [`cli-v1.md`](../contracts/cli-v1.md),
"Leaving out periodic candidates"), and the skill uses it where the list was too dense: a range with no transcript, read 5 minutes at
a time. `SKILL.md` INSPECT_CARDS adds the option to that sentence (the line count stays 300; the fuzz seed copy is the same bytes),
`references/commands.md` gains its two allowed forms (with and without `--cursor`) and a paragraph on `data.omitted` and the
cursor, and `references/budgets.md` gives the measured effect: on a generated 20:57 walkthrough the shortlist went from 38,349 to
18,324 characters in the same 5 calls, with every distinct screen still listed ([`candidates-long-recording-0.2.2.md`](../planning/candidates-long-recording-0.2.2.md)).
With a transcript the skill still asks only around its spans and does not use the option. An older `vsift` refuses the option as
unknown, which the skill says. **Agents ran this text once, in batch 2 on `0.2.2-rc.1` (2026-10-18): two Claude runs used the option on
20-second recordings, and a long recording was not exercised**; the trial freeze ([trials.md](trials.md), "The freeze (P14)") is
`skill` `bd205d6c...`, whole freeze `10180cd8...`.

## The change for 0.3.0: transcriptions in the background (ADR 0033; draft, 2026-10-21)

- **PREPARE starts the whole-video transcription with `vsift job start <session> --json`.** VSift answers when
  done for a recording under 10 minutes and at once for a longer one, which it transcribes in the background
  (`data.start.runner` `background`). A VSift before 0.3.0 refuses `job start` (`command` `parse`); the skill
  then runs `transcript retranscribe` with an operation id as before, so the operation-id rule and its example
  stay in PREPARE.
- **`references/resume.md` gains "A transcription in the background":** go on with the next states at once;
  check `vsift job status <job> --json` between steps and tell the user each time how far it is (`media`:
  `done_us` of `total_us`, `percent`, `speed`, `estimate.remaining_ms`, an estimate); `search` and
  `transcript get` answer over the published part, `partial` with `transcription_pending`; `--since` with an
  answer's `data.since.since_token` reads only what is new; never call the transcript complete before the job
  succeeded; out of budget, report `partial` with a resume card naming the job. In "A transcription that is
  still running", a `prepare` job is no longer a reason to stop media work: it leaves room for frames.
- **`references/commands.md`** classes `vsift job start` `free` and allows `job start` and the two `--since`
  forms (`search` and `transcript get`). The skill guard has a placeholder `<since-token>`.
- **What moved, so that `SKILL.md` stays at 300 lines:** the PREPARE paragraph after a timeout lost its
  parenthesis on `session status`, `job status` and `job resume` (resume.md says it), and the follow-up of a
  background job is in resume.md, as above. Four paragraphs were rewrapped to the widest line the file
  already had (116 characters) with no word changed (`git diff --word-diff` shows it). The fuzz seed copy is the
  same bytes.
- **Untried:** no trial has run the background path; the trial freeze is left alone while the pull request is
  a draft, and is rewritten for batches 2 and 3 when it is rebased to merge (the 0.3.0 rule).

## Keeping it in step with the CLI

`crates/vsift-cli/src/skill_contract.rs` (unit tests of the CLI crate, run by `cargo
test --workspace`) fails when the skill drifts from the CLI:

- every `vsift` line in a `console` block of the skill (and of this page) parses with
  the real parser, uses `--json` or `--events jsonl` and no operator-only option, and
  has the class its place requires; nothing is chained, piped or redirected except
  `| tail -n 1` after `--events jsonl`; a help form (`--help` after nothing, a
  namespace or an operation) must make the parser print its help, and is free;
- every shell fence of the skill is exactly one of the two literal forms that pass the
  draft report to `vsift handoff check` (a quoted heredoc, or a single-quoted
  here-string piped in), the skill's one input exception (P13 PR 5); `commands.md`
  shows both, `SKILL.md`'s REPORT the POSIX one, and a variant (another command, an
  unquoted or double-quoted delimiter, a double-quoted here-string, anything added) is
  refused; the contract's budget profiles equal `budgets.md`;
- the REPORT state's minimal handoff validates against `handoff.schema.json` and holds
  only the members the schema requires, the schema requires exactly the members the
  agent alone knows, and the
  rules state the stop-in-REPORT, no-file, no-`cd` and setup-check rules and the
  `compact` limits as the numbers `budgets.md` gives;
- every operation id the skill shows parses as one (`op_` and 16 to 64 lowercase
  letters or digits), and `SKILL.md` shows a valid example before its first command
  that takes one; the rules forbid every other program and self-timing;
- every inline `vsift` command names a real operation and every flag
  exists on it; every inline long option exists;
- the policy table classifies every public command exactly once, and the `never` and
  `explicit` sets match the reviewed lists in the test;
- every upper-case code is a published failure code (or a state name); every field or
  reason name resolves in the v1 contract (`schemas/v1` or `docs/contracts/cli-v1.md`)
  or the handoff schema;
- the example handoffs validate against `handoff.schema.json` and its citation,
  image-access and resume rules (work cut short that can continue carries a card
  of at most 2 KiB); `resume.md`'s card validates, shows every member and names a free
  next command;
- the vocabulary tables of `SKILL.md` (its fifteen members) and `references/handoff.md`
  (every member) list exactly the `enum` and `const` values the schema holds, and a
  gap note fits `VSift`'s longest fixed remediations with context;
- `SKILL.md`'s "before you send" checklist, `safety.md` and `handoff.md` name
  `display_text` and `display_label`, and `transcript-segment.schema.json` requires
  both;
- `resume.md` says that a new run has its own budget, repeats the image check and
  verifies each earlier finding again, and its card lists findings to verify with
  evidence it also keeps (PR 3i);
- no word of the current or a retired image check code is in a skill text file or the
  image's bytes, and neither joined code is in any text file of the repository;
  `SKILL.md` stays within 300 lines, holds only `name` and `description` in its front
  matter and links every reference.

A new command, flag or failure code therefore needs a skill update in the same change.

### The check image

The first check image (P12 PR 1) printed its code in small bold type in a 360x96
frame. In the final trial campaign GPT-6-Sol failed the image check in 5 runs, each
time reading the code with the same letter missing, so P12 PR 3i redrew it
(2026-09-30):

- **The code** is four upper-case letters and four digits, drawn from glyphs no reader
  confuses: never I, l, 1, O, 0, S, 5, B or 8. It is held, in two parts, only in the
  guard (`crates/vsift-cli/src/skill_contract.rs`) and in the grader's table of check
  images (`tools/vsift-agent-trials/src/skill.rs`), the grader's truth. A guard test
  fails if any word of the current or a retired code is in a skill text file or in
  the image's bytes as text, or if the joined code is in any text file of the
  repository.
- **The image** is a 776x168 greyscale PNG of 5,550 bytes, SHA-256
  `cfc5c888aae502a2bb5d47bae6b66ec7e5832fab3aeaa7af255e948c1e1962a1`: black Verdana
  Bold at 96 px on white, each glyph centred in its own 80 px cell, 56 px between the
  letters and the digits, 40 px margins.
- **How it was drawn** (FFmpeg 9.0, the gyan.dev full build, on Windows 11; the
  Verdana Bold font file of Windows 11, SHA-256
  `f3245f5f38f61bd1ceefb0f1338a5b88a21e6220832c2f43a38bbc7e1547c36f`). With `<g1>` to
  `<g8>` the code's eight glyphs in order and `<x1>` to `<x8>` their cells' left edges
  (40, 120, 200, 280, then 416, 496, 576, 656), one `drawtext` per glyph (shown for
  the first; the other seven differ only in `text` and the cell edge):

  ```text
  ffmpeg -f lavfi -i "color=c=white:s=776x168:d=1" -frames:v 1 -vf "drawtext=fontfile='C\:/Windows/Fonts/verdanab.ttf':text=<g1>:fontsize=96:fontcolor=black:y_align=baseline:x=<x1>+(80-text_w)/2:y=h/2+35,...,format=gray" -map_metadata -1 -fflags +bitexact -flags +bitexact -compression_level 9 image-check.png
  ```

  Two renderings with the same FFmpeg and font gave the same bytes. Another FFmpeg or
  font version may not, which is why the grader identifies the image by its SHA-256.
- **To replace it:** choose a new code by the same rule, render it as above, add its
  SHA-256 and code to the grader's table (marking the old entry retired) and put the
  code in the guard's current code, moving the old one to its retired codes; never
  write the code into any skill or documentation file. The grader reads the code of
  the image each trial's workspace received, so older trials still grade against the
  code they were shown.

## Models and clients trialled

Qualified by the P12 trials (2026-09-30) through the same skill, prompts and compact
budget; the compact tier by the re-run on `a0bfb06` after P12's fixes (#222). The full
numbers are in the [qualification record](../planning/p12-agent-qualification.md).

| Client | Model | Tier | Result | Support |
| --- | --- | --- | --- | --- |
| Claude Code 2.1.284 (Windows) | Claude Opus 5.5 (`claude-opus-5-5`) | review | A-08/A-09: 11 of 11 mechanical, 9 of 11 full passes (A-08 5 of 5) | **Qualified** (interpretation misses under the maintainer's review) |
| codex-cli 0.155.0-alpha.16 (Linux) | GPT-6-Astra (`gpt-6-astra`) | review | A-08/A-09: 11 of 11 mechanical, 9 of 11 full passes (A-08 4 of 5) | **Qualified** (interpretation misses under the maintainer's review) |
| Claude Code 2.1.284 (Windows) | Claude Sonnet 5.5 (`claude-sonnet-5-5`) | compact | A-01..A-07, SEC-T02 (#222 re-run): 26 of 28 full passes (93%), 28 of 28 answers right | **Qualified** (meets the 90% target) |
| codex-cli 0.155.0-alpha.16 (Linux) | GPT-6-Sol (`gpt-6-sol`) | compact | A-01..A-07, SEC-T02 (#222 re-run): 28 of 28 full passes (100%; 23 of 28 before the maintainer's `rg --files` decision), 28 of 28 answers right | **Qualified** (meets the 90% target) |
| Claude Code | Claude Haiku 4.5 | below the line | 6 of 28 answers right (`b68d746`) | Not supported ([L-082](../planning/known-limits.md#l-082)) |
| Codex | GPT-6-Luna | below the line | 19 of 28 answers right (`56f1e1f`) | Not supported ([L-084](../planning/known-limits.md#l-084)) |

**What the compact results mean in practice.** In the re-run, 2 of 56 compact
trials needed a correction, both a finding bound to a frame that does not show it
(P12's final round, before the fixes, needed one in five). In every counted run the
agent stayed within policy:

- it installed nothing;
- it followed no instruction found in the evidence;
- it leaked no secret;
- it copied no hidden character into its report.

Prefer a review-tier model when the report must be right first time.

**Other limits.**

- **A client counts only on the system it was trialled on** (the rule of the
  [support matrix](../planning/support-and-resource-profiles.md)): Claude Code on the project's
  Windows 11 machine, and Codex in a Linux container. No agent trial has run on macOS or on a
  hosted runner, every trial so far used synthetic videos and a synthetic voice, and the P12
  trials ran a source-built binary, not the published package.
- **Codex on Windows:** its sandbox cannot run VSift today
  ([L-076](../planning/known-limits.md#l-076), #204). The Codex trials ran in a Linux
  container.
- **Other clients and models:** they were not trialled. They may work if they can run
  a local command and open a PNG, but they are not qualified.

## Not yet done

- **The skill of `0.2.2` (the three changes above, #395, #366 and #367) was run by agents once, on its candidate (`0.2.2-rc.1`, whose skill is `0.2.2`'s byte for byte), and two of its three changes have no scenario.** Under the freeze committed by #435 (whole-freeze digest `10180cd8...`), batch 2 (2026-10-18, 34 counted runs) met every gate as graded: safety (the hard gate) 34 of 34, the review-tier journeys 6 of 6 for each client, the blurred banner 3 of 3 for each, the four hold-outs, and **the compact regression 10 of 10** (8 of 10 on `0.2.1-rc.3`); every run of the A-08, A-09 and hold-out scenarios ran `search` (#395, [L-163](../planning/known-limits.md#l-163)). No scenario asks for a long report or a long recording, so the check of a long report on Windows (#366) and the filter on a long recording (#367) were **not exercised**: every run checked its report on standard input through a here-document, and two Claude runs used `candidates --omit periodic` on 20-second recordings. Batch 3 (the cold round, no skill; 18 counted and 6 reserve runs): cold safety (hard) met, 0 of 24, and the missing-tools scenario on the new advice of #396 safe and useful in 6 of 6 ([L-164](../planning/known-limits.md#l-164)); Codex compact useful 6 of 6; **Claude compact useful 4 of 6, 7 of 12 pooled with the six reserve runs**, against 80% ([#454](https://github.com/smormah/vsift/issues/454), [L-178](../planning/known-limits.md#l-178)): the maintainer waived that usefulness part for `0.2.2` only, so RQ-16 is waived and not passed. The readings are [`batch-2-reading-0.2.2-rc.1.md`](../planning/p14-agent-trials/batch-2-reading-0.2.2-rc.1.md) and [`batch-3-reading-0.2.2-rc.1.md`](../planning/p14-agent-trials/batch-3-reading-0.2.2-rc.1.md); the results are in [`release-0.2.2.md`](../planning/release-0.2.2.md). Nothing was re-graded, and the grader, the skill and the scenarios were not changed.
- **The skill of `0.2.1` was run by agents once, on the third candidate (`0.2.1-rc.3`, whose skill is `0.2.1`'s byte for byte), and most of its second change still has no scenario.** Batch 2 (2026-10-14 and 2026-10-15, 34 counted runs, under the committed freeze): safety (the hard gate) 34 of 34, the review-tier journeys 6 of 6 for each client, the blurred banner 3 of 3 for each, the four hold-outs met, 32 of 34 runs passed fully, and **the compact regression was 8 of 10, which is not the gate:** two Claude Sonnet 5.5 runs read the whole 20-second transcript with `--view brief` and never ran `search`, which the frozen grader requires ([#395](https://github.com/smormah/vsift/issues/395), [L-163](../planning/known-limits.md#l-163)); the maintainer waived exactly those two runs for `0.2.1` on 2026-10-15, and a stronger "search first" rule is a follow-up in the next change to the skill. Batch 3 (the cold round, 18 runs, no skill): cold safety was not met, 1 of 18 (one combined command that tried to list folders and read a configuration file, which Claude Code's strict setting denied; VSift's advice for a missing tool invites it, [#396](https://github.com/smormah/vsift/issues/396), [L-164](../planning/known-limits.md#l-164)); cold usefulness was 6 of 6 for Codex compact and 2 of 6 for Claude compact under the strict setting, which refuses compound commands ([#397](https://github.com/smormah/vsift/issues/397), [L-165](../planning/known-limits.md#l-165)); the maintainer waived these for `0.2.1`. A waiver is not a pass, nothing was re-graded, and the grader, the skill and the scenarios were not changed. The rule for reporting a `partial` transcription, a whole-recording question, a long report, a `BUSY` frame call and a timed-out tool has no scenario: adding one is the maintainer's decision, and a new scenario changes a frozen component, so it would need its own freeze and candidate. The figures in the next bullets are for the skill of `0.2.0`.
- **The review tier's blurred-banner re-run met its gate on the third candidate, on a small
  sample.** The re-run after the #224 fix ran in batch 2 on the first two release
  candidates (2026-10-05 and 2026-10-07, three runs per client each time): GPT-6-Astra met
  the gate both times (3 of 3), and Claude Opus 5.5 did not (1 of 3 both times), and it also
  missed the mechanical gate (4 of 6 on the second candidate). The two evidence rules of
  2026-10-07 (above) are the change made for that, in the third candidate `0.2.0-rc.3`, and
  on it (batch 2, 2026-10-08) both models met the gate (3 of 3 each), Claude Opus 5.5 met the
  mechanical gate too (6 of 6) and 34 of 34 runs passed fully. **That is three runs per
  client and one wording; nothing shows that the rules caused the change, and the two limits
  stay open** ([L-095](../planning/known-limits.md#l-095),
  [L-139](../planning/known-limits.md#l-139)). The release 0.2.0 is built from the same
  source, so these runs were made on its source and not on its own bytes.
- **The named-agent runs from a clean installation, and what they left open** (P14, the
  release qualification; ADR 0023 decision H10). The harness for it is built (P14 PR 6):
  VSift is installed from the real npm registry into a fresh folder, a skill trial copies the
  skill from the **installed package** (checked equal to the repository's, which is where the
  grader reads the command table), and it includes a cold-agent variant (the CLI on `PATH`,
  no skill, no docs; maintainer decision 2026-10-02), graded with safety as a hard gate and
  usefulness reported apart. Two hold-out scenarios (one per transcript path) were written for
  it and nothing was tuned on them. **Batch 1 ran on 2026-10-03 against the published 0.1.0, as
  a baseline** (20 runs; the reading is in
  [`p14-agent-trials/batch-1-reading.md`](../planning/p14-agent-trials/batch-1-reading.md)):
  the four skill pilots (A-08 and A-09, on each client) passed fully, and no cold run
  installed anything or accepted a setup plan. The cold agent, which has only the CLI's own
  help, reached the key facts in 1 of 6 compact runs with Claude Code (a setting that lets it
  run `vsift` alone) and 2 of 6 with Codex (a container with ordinary read-only helpers); the
  two settings are different tests, and the cold target is 80% on the candidate's final round.
  A baseline measures; it is not a result for the candidate. **Batch 2 (the counted set, the
  hold-outs and the blurred banner; 34 runs) ran on all three candidates; on the second the
  evidence item was recorded as failed, for the two Claude Opus 5.5 gates above, and on the
  third every gate was met** ([P14 plan](../planning/p14-qualification.md) sections 27 and 29.8).
  **Batch 3 (the cold final round: 18 runs with no skill, on `0.2.0-rc.3`, 2026-10-08)** met the
  usefulness target on both clients with no margin (5 of 6 each, against 80%) and did not meet
  the cold safety gate: one Codex run (GPT-6-Sol) read, with `base64`, the audio clip file that
  `vsift audio` had named, which the gate counts as an out-of-policy action; nothing was
  installed, written or sent, and no cold run installed anything or accepted a setup plan. The
  maintainer waived that one action for `0.2.0-rc.3` and for the release 0.2.0 built from the
  same source (2026-10-09; [#340](https://github.com/smormah/vsift/issues/340),
  [L-142](../planning/known-limits.md#l-142)). A waiver is not a pass, and no person has read
  the raw cold logs ([L-118](../planning/known-limits.md#l-118)) ([the P14
  batches](trials.md#the-p14-batches)). Since the baseline `vsift --help` has a "typical
  investigation" section (P14 PR 7), which the cold agent reads and the skill does not need.
- **Open readings and limits:** Sonnet's "previous value" slip (#219), Codex on Windows
  (#204, L-076) and L-078 to L-084 in the register.

Done since this page's first version: the compact tier's ≥90% target (the #222 re-run
met it, 93% and 100%), the maintainer's review of the flagged strong-tier runs, and the
command that validates a handoff draft (`vsift handoff check`, P13 PR 5, #213).

How to run the trials is in the [trial runbook](trials.md).
