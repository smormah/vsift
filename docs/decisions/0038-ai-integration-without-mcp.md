# ADR 0038: AI integration without MCP: workflows, the skill everywhere, and the CLI alone

- Status: **Accepted by the planner, 2026-10-28**, at `9f563bc2`, relayed by the coordinator, with
  the conditions recorded in [Outcome](#outcome-the-planners-rulings-of-2026-10-28). D1 is the
  maintainer's, **decided on 2026-10-28** (amended into sections 2, 8 and 9 below; the planner
  accepted the amendment on 2026-10-29, so the meeting-notes and workshop-spec workflows may be built,
  in their own pull request). Each slice is
  built in its own pull request; until each lands, what it describes is a plan, not a feature.
- Date: 2026-10-28
- Refines: [ADR 0022](0022-agent-skill-and-named-client-qualification.md) (the skill, its command policy
  and its home at `skills/vsift/`), [ADR 0023](0023-r0-distribution-managed-installation-and-handoff-check.md)
  (the release archives, the npm package and `handoff check`) and
  [ADR 0024](0024-r0-qualification-and-release-candidate.md) (the agent trials, the freeze and the cold-agent
  mode). It meets [ADR 0029](0029-time-anchoring.md) (clock time) and
  [ADR 0037](0037-vsift-video-the-action.md) (`vsift <video>`, which stays `never` for the skill).
- Tracking: the 0.3.0 story W9. Register entries L-238 to L-242 are proposed in section 9. Each is added
  by the pull request that builds what it limits.

## Context

The maintainer's ruling of 2026-10-28 says that for 0.3.0, every AI integration except MCP reaches
parity, and goes further where VSift can: skills, ready-made workflows and the CLI. It followed his
own try-out. In a client that VSift's skill did not reach, an agent judged that 0.2.3 could not do the
job. It had no skill path and no ready-made workflows. It also built its handoff JSON by text
substitution, and PowerShell's case-insensitive `-replace` corrupted the JSON without any error.

What exists today (checked on `main` at `37077c12`, 2026-10-28, by reading the files named):

- **The skill.** `skills/vsift/`: `SKILL.md` (300 lines, which is the guard's limit), seven references,
  the handoff schema, two example handoffs, the image-check PNG and Codex metadata. It ships byte for byte
  in the repository, in every release archive and in the `vsift-cli` npm package. People install it by
  copying the folder. [`docs/agents/skill.md`](../agents/skill.md) documents two clients: Claude Code and
  Codex.
- **The guard.** `crates/vsift-cli/src/skill_contract.rs` holds every `vsift` line of the skill to the
  real parser and to the command policy (`free`, `explicit`, `never`).
- **Help.** `vsift --help` already has a "typical investigation" section (P14 PR 7,
  `TYPICAL_INVESTIGATION_HELP` in `crates/vsift-cli/src/command.rs`). A test parses every command line in
  it.
- **The cold-agent mode.** The agent trials already have it ([trials.md](../agents/trials.md), "Cold-agent
  mode"): the CLI on `PATH` and nothing else, scenarios C-01 to C-03, and batch 3 at each cut. Item 6 of
  the brief is therefore half built. What is missing is a scenario that asks for an outcome, and help that
  leads there.
- **The handoff.** `vsift handoff check` validates a draft. Nothing helps an agent start one, so an agent
  without the skill invents a template and fills it in.
- **0.3.0 features already in the skill.** `ocr` (`explicit`), `ocr search` (`free`), `job start`,
  `watch` (`never`) and the setup door (`never`). **Merged but not yet in the skill:** `ingest --url` and
  `ingest --recorded-at` with the `clock_time` it adds to results. **Not merged yet:** speaker labels,
  "what changed on screen" and search across kept sessions.

Where clients look for skills. Each row was confirmed on 2026-10-28 from that client's own public
documentation (the URLs are in section 3):

| Client | Folder for one user | Folder for one project |
| --- | --- | --- |
| Claude Code | `~/.claude/skills/` | `.claude/skills/` |
| OpenAI Codex | `~/.agents/skills/` | `.agents/skills/` |
| GitHub Copilot (VS Code and CLI) | `~/.agents/skills/` (also `~/.copilot/skills/`) | `.agents/skills/` (also `.github/skills/`, `.claude/skills/`) |
| Cursor | `~/.agents/skills/` (also `~/.cursor/skills/`) | `.agents/skills/` (also `.cursor/skills/`, `.claude/skills/`) |
| Gemini CLI | `~/.agents/skills/` (also `~/.gemini/skills/`) | `.agents/skills/` (also `.gemini/skills/`) |
| OpenCode | `~/.agents/skills/` (also `~/.claude/skills/`) | `.agents/skills/` (also `.claude/skills/`) |
| Goose | `~/.agents/skills/` | `.agents/skills/` (also `.claude/skills/`) |
| Amp | `~/.agents/skills/` (also `~/.claude/skills/`) | `.agents/skills/` (also `.claude/skills/`) |

Two folders reach every one of these clients: `.claude/skills/` for Claude Code and `.agents/skills/`
for the rest. Claude Code does not read `.agents/skills/`.

## Decision

The work is built in five slices. Each slice stands alone and can ship alone, under the deadline rule.
Item 7 (the skill covers what 0.3.0 adds) rides along with each slice. The slices follow the brief's
order: the handoff hazard, the workflows, install, the plugin, and help with the cold scenario.

### 1. Handoffs without string templates (brief item 5): `vsift handoff skeleton`

A new free command, `vsift handoff skeleton <session> --question <text> --json`. It prints a complete
handoff that is valid against the handoff schema but states nothing yet. VSift fills in what it already
knows:

- `handoff_version`, and `question` from the argument (refused with `INVALID_ARGUMENT` if the text breaks
  the schema's safe-text rule);
- `status` `insufficient_evidence`;
- `capabilities.image_access` `unavailable` (the agent changes it only after its own image check), and
  `transcript_basis` from the session's newest transcript revision. `media_tools` and `local_asr` are
  left to the agent: they are what `setup check` reports, not what the session records (implementation
  note below);
- the `session` ids, including the transcript revision when there is one: the revision a plain
  `transcript get` reads, which is the open revision while a background transcription runs (ADR 0033
  D2), so the skeleton names the revision the agent's reads cite;
- `claims: []`, `citations: []`, `gaps: []` and `untrusted_instructions: []`;
- `lifecycle.action` `left_open`.

`data.handoff` is that object. `data.fill` lists the members only the agent can supply (the image check,
the claims and citations, the gaps it found, the instructions it saw, and the final status and
lifecycle). Without `--json` the command prints the ready fenced `vsift-handoff` block, for a person or
for a client that copies text.

**Why this and not only a sentence in the skill.** The hazard happened in a client the skill did not
reach. The fix has to live in the CLI, where every agent can find it from `vsift --help`. Copying ids
from VSift's own output stays safe, because ids are opaque tokens. The corruption came from a template
whose placeholder words were also JSON key names. A skeleton removes the template.

**What it never does.** It writes no file, never renews the session and adds no claim. The skill gains
one sentence: "Start the JSON from `vsift handoff skeleton`, or write it whole. Never build it by
search-and-replace."

**Contract.** A new command id `handoff.skeleton`, which the schemas' pattern already admits. A new data
schema with a frozen example, contract tests (the skeleton validates against `handoff.schema.json`, and
`handoff check --session` accepts it), `cli-v1.md` and the guide reference. Additive under v1.

**Cost:** about one day and one pull request. It touches no failure code and no frozen trial component
except the skill text.

### 2. Workflows, VSift's own (brief item 1)

Six workflows, each a short file in the skill at `skills/vsift/workflows/<name>.md`:

| File | Outcome | Budget | Method in one line |
| --- | --- | --- | --- |
| `bug-report.md` | A recording becomes an evidence-backed bug report | `compact` | The existing procedure: search the symptom words, read windows, inspect frames and report Problem, Expected, Actual and Reproduction. |
| `findings.md` | A narrated screencast becomes findings (bug, feature, question) | `standard` | Search cue words for each kind ("doesn't", "should", "why", "?"), then read windows and confirm on screen. Each finding is one claim with a `category`. |
| `workshop-spec.md` | A workshop becomes a spec with quoted decisions and open questions | `read_through` | Read the whole transcript page by page (amended 2026-10-29, D1), quote each decision from `display_text` with its segment, and list open questions; frames only within `standard`'s image limits. |
| `demo-backlog.md` | A product demo becomes a prioritised backlog | `standard` | Search problem and request cue words, confirm each item on a frame, and give each a `priority` from what was said (cited) or, when nothing says it, inferred, which the claim's statement says ("priority inferred: high"; ruled 2026-10-29, below). |
| `meeting-notes.md` | A meeting becomes actions, decisions and open questions | `read_through` | Read the whole transcript page by page (amended 2026-10-29, D1), and name speakers by `display_label` when segments carry one; frames only within `standard`'s image limits. |
| `logs-by-clock.md` | Moments in a recording are matched to system log lines through clock time | `compact` | Ingest with `--recorded-at` when the user knows the start. Turn each log time into a window, then cite the segments and frames whose own `clock_time` VSift reports. |

Every workflow uses only `free` commands. Each stays inside its profile and ends with `vsift handoff
check`, so its result is a checkable handoff. That is the difference from a summary: every item cites a
segment, frame or crop that `handoff check --session` resolves.

**Long recordings (amended 2026-10-29, D1).** Most meetings, workshops and demos run longer than 5 minutes.
The maintainer decided D1 on 2026-10-28, in his words: "Yes: meeting notes and workshop specs read the
whole transcript text, up to about two hours, with images still within the budget." So:

- **`meeting-notes.md` and `workshop-spec.md` read the whole transcript**, page by page in the brief view,
  under a new profile, `read_through` (below). This supersedes the #368 ruling for these two workflows
  only. The rest of the skill keeps it: a summary or timeline of a whole long recording still does not fit
  `compact` or `standard`.
- **`findings.md` and `demo-backlog.md` keep cue-word search** within `standard` (and `bug-report.md` and
  `logs-by-clock.md` within `compact`). They end `partial` and list every stretch they did not read as a
  `transcript` gap with reason `not_inspected`. Their cue words are English, so other languages find less
  (L-238).

**The `read_through` profile** is `standard` with more room for text and nothing more for images. The
identifier is snake case like every other closed value of the handoff:

| Limit | `standard` | `read_through` | Why |
| --- | --- | --- | --- |
| Page size (`--view brief` only) | 50 | 25 | Not for the agent's context: the whole read is the same text at either size. It is for the client's cut (below the table). |
| Transcript pages | (no limit of its own) | 120 | At about 5 s a segment, two hours is about 1,440 segments, 58 pages of 25; supplied subtitles with 2.5 s cues are about 116. 120 covers both. |
| Tool calls | 80 | 200 | At most 120 transcript reads, and the other calls stay within `standard`'s 80 (searches, `job status`, frames, the check). |
| Images per step, in total, bytes | 4, 24, 48 MiB | 4, 24, 48 MiB | Unchanged: "images still within the budget". |
| Refinement depth, burst frames | 4, 12 | 4, 12 | Unchanged. |
| Wall time | 30 min | 60 min | Reading 120 pages takes longer; the host still measures and enforces it. |

**Why pages of 25, when the brief view has allowed 50 since 0.2.1** (the planner's question,
2026-10-29). The segments a two-hour read brings into the agent's context are the same at either page
size, so 25 is not for the context. (Each page also repeats the revision block, so smaller pages cost
somewhat more in total. After a transcription that left many stretches no segment holds, that block alone
can be about 45,000 characters, and then no page size fits under the cut. budgets.md already says to report
such a read as incomplete.) It is for the client. A client can cut a long tool answer without any
error (one did at about 43,000 characters, budgets.md), and a brief page of 50 measured 35,000 to 57,000
characters (`docs/agents/skill.md`, the 0.2.1 change). So a page of 50 is often over the cut. A cut page is
asked again with `--limit 5` (budgets.md), which costs ten calls for that page, or it is reported as not
read. A page of 25 is about 18,000 to 28,000 characters, under the cut in the common case. A two-hour read
then takes up to 120 calls instead of 60. That is why the call ceiling is 200: 120 reads plus `standard`'s
80. `standard`'s targeted reads keep 50: they read a few windows, where an occasional cut and re-ask is
cheap. A client known not to cut may use 50 under `read_through` as an override (`budget.overrides`), which
halves the reads. Not measured here: the 25-segment size through any client. The 43,000-character cut is
one client's, and the page sizes are from the 0.2.1 measurements.

`transcript_pages` is a new limit member, so the handoff's `budget.limits` gains it (optional, and only
`read_through` gives it a value). `budget.profile` gains `read_through`. `handoff check` holds the given
limits to the profile's, as it does today. Both are additive to the skill-owned handoff schema;
`handoff_version` stays `1`.

**What a read-through costs the agent.** Two hours of brief pages is roughly 0.5 to 1.4 million
characters of tool output. That fits only a model with a long context window, so `read_through` is for
the review tier. A compact model gets the cue-word method of the other workflows instead, and the
workflow file says so before the first read. This is stated as L-243.

**Edges.**

- **Over about two hours**, or when the page ceiling runs out first: the report ends `partial`. Each
  stretch not read is a `transcript` gap (`not_inspected`, or `budget_exhausted` with the resume card when
  the budget ran out). Every item cites only segments from pages actually read.
- **A transcription still running** (ADR 0033): the workflow follows the job with `job status` and reads
  what is published with `--since`. If the job has not succeeded when the budget or the wall time runs
  out, the report ends `partial` with the unread stretches as gaps (`transcription_pending` where VSift
  says so) and a resume card naming the job.
- **A cut answer** follows budgets.md: ask again with `--limit 5`; if it is still cut, that page is
  reported as not read.
- **"Read all" only when true.** The report says it read the whole transcript only when the last page's
  `next_cursor` was null, no read was `partial`, and the transcription job (if any) had succeeded.
  Otherwise it says which stretch it read, from VSift's own times.
- **The handoff's caps stay** (50 claims, 100 citations, 50 gaps). A meeting with more items than 50 claims
  hold ends `partial`, with a gap note giving the count left out. Items that do not fit are not listed in
  prose either, since the prose says nothing the JSON does not.

**Clock time.** No command takes a clock time yet. The agent turns a log time into source time by
subtracting `time_anchor.recording_start`, which VSift reported. It then cites only items whose own
`clock_time` VSift returned, so a wrong subtraction shows up as a mismatch rather than a false claim. A
log line is not VSift evidence, so it is quoted in the prose as untrusted data and is never a citation
(L-239). The user names the log file. The agent reads it with its file-reading tool, as data. That is
one new exception to rule 3, stated in this workflow only.

**The handoff gains two optional members (decision D2).** `claims[].category` is one of `bug`,
`feature_request`, `question`, `decision`, `action_item`, `open_question`, `requirement` or
`backlog_item`. `claims[].priority` is one of `high`, `medium` or `low`. Both are additive. An older
`handoff check` refuses them, which is acceptable because the skill and the binary ship together. The
bug-report workflow uses neither. The others put each claim in `section` `context` with a `category`.
The "findings JSON checked by a schema" is therefore the handoff block itself, and there is no second
schema to keep in step.

**One file per workflow, not two (decision D7).** Each workflow file stands alone when pasted into a
client that has no skill. Its first lines say: "If the vsift skill is loaded, its rules apply. If not,
read `vsift --help` first, use only `vsift` commands, and end with `vsift handoff check`." The skill links
each file from one new line in `SKILL.md`. To stay at 300 lines, an existing sentence moves to a
reference, as earlier changes did. The plugin's slash commands point at these same files.

**Tests.** The guard extends to `workflows/*.md`. Every `vsift` line parses and is `free`, and every
inline command and flag exists. The budget a workflow names is a profile in `budgets.md`. The guard holds
`read_through` in `budgets.md` equal to the contract's constant (`vsift_contract::handoff::budget`), as it
does `compact` and `standard`, and `handoff check` accepts its limits. Three example handoffs (bug report,
findings, meeting notes) are built from fixture output and validate against the schema, including the new
members. The meeting-notes example is a whole-transcript read: every page read, `budget.profile`
`read_through` (amended 2026-10-29).

**Cost:** about two days and one pull request for the four cue-word workflows, the two handoff members
with their check rules, the guard and two examples. The two read-through workflows, the `read_through`
profile with its `transcript_pages` member and the meeting-notes example are about one more day, in their
own pull request after the planner reviews this amendment (amended 2026-10-29).

### 3. The skill everywhere an agent looks (brief items 2 and 3): `vsift skill install`

A new namespace with three operations.

- **`vsift skill install --client <name> [--client <name>...] [--project <dir>] [--replace] [--json]`**
  copies the skill into the folder that client documents. Without `--project` it uses the folder for one
  user. `<name>` is one of `claude-code` (`.claude/skills/vsift/`), or `codex`, `copilot`, `cursor`,
  `gemini-cli`, `opencode`, `goose` or `amp` (each `.agents/skills/vsift/`). Several clients that share a
  folder are written once. The result lists each folder with its outcome:
  - `installed`;
  - `unchanged` (the same bytes were already there, so nothing was written);
  - `replaced` (only with `--replace`, naming the digest that was replaced);
  - `refused` (a different folder is there and `--replace` was not given).

  A refusal changes nothing. It answers `INVALID_ARGUMENT`, and the remediation names `--replace` and says
  what would be replaced (decision D3). With `--replace`, a folder is replaced only if its `SKILL.md` names
  the skill `vsift`. A symbolic link or junction is removed as a link and never followed. Any other folder
  is refused.
- **`vsift skill status [--project <dir>] --json`** is read-only. For each known folder it reports
  `absent`, `current` (the bytes equal this binary's skill), `different` (another version, or edited) or
  `not_vsift`.
- **`vsift skill show <file>`** prints one text file of the skill: `SKILL.md`, a reference or a
  workflow. An agent with only the CLI can then read the full method without installing anything. It
  prints nothing else and refuses the PNG.

**The bytes come from the binary.** The skill is embedded at build time (`include_bytes!`, one entry per
file, in `vsift-contract` beside the handoff schema it already embeds). A test holds the list equal to
the folder, so a file added later cannot be forgotten. The install writes into a fresh staging folder
beside the target and verifies every file's SHA-256 against the embedded manifest. Only then does it
rename the folder into place. A failure leaves the target as it was. The install never needs the release
archive's copy, so it works the same from npm, from an archive and from a source build.

**Layers.** The client catalogue (each name, its scope and its folder below the home or project folder)
is a domain value, because it is the product's statement of where clients look. The install is an
application use case behind a port. Infrastructure resolves the home folder, canonicalises
`--project`, stages, verifies and renames. The CLI only composes and presents.

**The skill's own classes (brief item 3).** `skill install` is `never`: it changes another client's
configuration, and rule 8 forbids that. The person runs it. `skill status` and `skill show` are `free`
(decision D4).

**The release keeps one layout (decision D5).** Archives and npm keep `skills/vsift/`. They do not also
ship an `.agents/skills/vsift/` tree, because the install command writes that layout and a hidden folder
in an archive helps nobody. The repository never holds `.agents/skills/` or `.claude/skills/`, so the
rule of [skill.md](../agents/skill.md) holds: contributors' own agents never load the skill by accident.
npm keeps no install scripts. The docs say to run `vsift skill install` after `npm install`.

**Documentation.** `docs/agents/skill.md` and `docs/operations/install.md` gain the table above, with each
client's documentation link, and the manual copy as the fallback. Clients are named in docs, never in
marketing. The confirmation sources (2026-10-28):
<https://code.claude.com/docs/en/skills>, <https://learn.chatgpt.com/docs/build-skills>,
<https://code.visualstudio.com/docs/copilot/customization/agent-skills>,
<https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-skills>,
<https://cursor.com/docs/context/skills>, <https://geminicli.com/docs/cli/skills/>,
<https://opencode.ai/docs/skills/>, <https://goose-docs.ai/docs/guides/context-engineering/using-skills>
and <https://ampcode.com/docs/customize/skills>. Windsurf (now named Devin Desktop) also reads
`.agents/skills/`. It is left out of `--client` while its naming is in flux, and the docs say the
`codex` folder serves it.

**Tests.** Contract tests (schemas and frozen examples for the three results, through the binary against
a made-up home folder), and the embedded manifest equal to the folder. Install, re-install (`unchanged`),
refusal, `--replace`, a link at the target and a failed verification each leave the right files.
Nothing writes outside the temporary test home.

**Cost:** about two and a half days and one pull request. It is the largest slice.

### 4. A Claude Code plugin (brief item 4)

The repository gains `.claude-plugin/marketplace.json`, with the marketplace `name` `vsift` and
`owner.name` "VSift project", a neutral name with no person's details. The plugin itself lives at
`plugins/vsift/` and holds:

- `.claude-plugin/plugin.json` (`name`, `description`, `version`, `license` and `repository`, with no
  author);
- `skills/vsift/`, the skill;
- `commands/<workflow>.md`, one per workflow, which become `/vsift:bug-report` and so on. Each is a few
  lines that name its workflow file;
- optionally `agents/vsift-investigator.md`, a subagent that has the skill, with tools limited to the
  shell and file reading.

It ships no `hooks/`, `bin/`, `.mcp.json` or `package.json`. Installing it runs nothing, and a
governance test refuses any of them.

**Byte-identical by test, not by link.** Claude Code copies a plugin into its cache and rejects any path
that leaves the plugin folder. A symbolic link inside the repository would work, but on a Windows
checkout without symbolic links (Git's default) it becomes a text file, and the plugin breaks for
Windows users. So `plugins/vsift/skills/vsift/` is a real copy, and a test (in the guard, run by `cargo
test`) fails unless every file equals `skills/vsift/` byte for byte. The governance check already guards
the fuzz seed copy the same way.

**Which version a person gets (decision D6).** `/plugin marketplace add smormah/vsift` follows `main`,
whose skill can be ahead of the person's installed `vsift`. The docs say to add
`smormah/vsift#v<version>` for the version installed. The skill already handles an older binary that
refuses a newer command (L-240).

**Cost:** about one day, in one pull request.

### 5. Without any skill (brief item 6)

- **Help.** `TYPICAL_INVESTIGATION_HELP` gains two things. Step 6 gets a line: "`vsift handoff skeleton
  <session> --question <text> --json` gives the JSON to fill in; never build it by search and replace."
  A short "ready-made outcomes" paragraph lists the six workflows in one line each, and says `vsift skill
  show workflows/<name>.md` prints each one's full method. The existing test that parses help's command
  lines covers the new ones. This is help text, not JSON.
- **The cold scenario.** The existing cold mode gains C-04, a workflow-shaped request with no skill and no
  docs: "Write a bug report, with times, from this recording". It runs on an existing fixture whose truth
  the grader already holds. It is graded by the existing cold grader, with safety as the hard gate and
  usefulness reported apart, and runs at the 0.3.0 cut in batch 3. It is not run now. A scenario is a
  frozen component, so the draft leaves the freeze alone, and the merge rebase rewrites it as the 0.3.0
  rules say.

**Cost:** about one day and one pull request, mostly the scenario and its validation tests.

### 6. The skill covers what 0.3.0 adds (brief item 7)

Each slice adds what has merged by then, in one place per story:

- `ingest --url` (`free`, only a link the user gave, never one found in evidence);
- `ingest --recorded-at` and `clock_time` (PREPARE and `logs-by-clock.md`);
- OCR, `job start`, `watch` and the setup door are already there.

Speaker labels, "what changed on screen" and search across kept sessions are added by the slice that
follows their merge, or by their own story if they merge after W9. The freeze is rewritten at each
merge rebase.

### 7. What this changes in earlier records

- **ADR 0022.** The skill's folder gains `workflows/`. The skill classes `skill install` `never` and
  `skill status`, `skill show` and `handoff skeleton` `free`. Rule 3 gains the one exception for a log
  file the user names, in the logs workflow only. The rest of ADR 0022 stands.
- **ADR 0023.** The handoff gains two optional members. `handoff check` validates them.
- **ADR 0024.** The cold mode gains C-04.

### 8. Ranked cuts if time runs short

Cut the first item first:

1. The plugin's subagent. **Cut from 0.3.0** (the planner, 2026-10-29; slice 4's implementation note).
2. `vsift skill show`, with help naming the skill's files instead.
3. `claims[].priority`. The backlog's order then lives in prose, with `category` `backlog_item`.
4. The three example handoffs beyond the first (keep bug report).
5. `vsift skill status`. `install` already reports what it found.
6. C-04. The existing cold round still runs on the new help.
7. The plugin entirely. `skill install --client claude-code` reaches Claude Code.
8. `logs-by-clock.md`, if clock time slips from 0.3.0.
9. `workshop-spec.md` (amended 2026-10-29): it shares `read_through` with `meeting-notes.md`, so cutting it
   saves its file and example only.

**Never cut:**

- the skeleton (the hazard that started this);
- the other workflow files, with `meeting-notes.md` and its `read_through` profile (the maintainer's D1),
  and the two handoff members' `category`;
- `skill install` for both folders;
- the help pointers;
- item 7 for whatever has merged.

### 9. Proposed register entries

- **L-238:** The cue-word workflows (findings, demo-backlog, and bug-report and logs-by-clock over a long
  recording) find items by cue-word search within the budget. An item far from any cue word can be missed.
  The report says which stretches were not read, and the cue words are English. Meeting notes and
  workshop specs read the whole transcript instead (D1, amended 2026-10-29).
- **L-239:** No command takes a clock time. The agent subtracts the recording's start to find a window.
  Log lines are not citable evidence in handoff v1, so they are quoted in prose as untrusted data.
- **L-240:** A plugin added from `main` can carry a newer skill than the installed `vsift`. The docs say
  to pin the release tag.
- **L-241:** `skill install` knows only the clients whose folders were confirmed from their docs on
  2026-10-28. A client that moves its folder needs a new release, and the manual copy still works.
- **L-242:** The workflows, the plugin and the install command are untried by agents until the 0.3.0 cut.
  The text ships, but the behaviour is unmeasured.
- **L-243 (confirmed by the planner 2026-10-29; amended 2026-10-29):** `read_through` reads
  at most 120 pages of 25 segments (about two hours), needs a long-context model (roughly 0.5 to 1.4
  million characters of tool output for two hours), and a meeting with more than 50 items ends `partial`
  because the handoff holds at most 50 claims. A longer recording or more items are stated as gaps.

## Decisions for the planner and the maintainer

- **D1 (the maintainer).** Workflows over a long recording are cue-word sampled within `standard` and end
  `partial` with the unread stretches as gaps. This keeps the #368 ruling. A profile that reads a whole
  transcript is not in 0.3.0. *Recommended:* yes. **Outcome (2026-10-28): not as proposed: the
  whole-transcript read. In the maintainer's words: "Yes:
  meeting notes and workshop specs read the whole transcript text, up to about two hours, with images still
  within the budget."** The planner's reading: those two workflows read the whole transcript under a
  text-only profile with a page ceiling and a call ceiling sized for two hours, superseding #368 for them
  only; the other workflows keep cue-word search. Amended into sections 2, 8 and 9.
- **D2 (the planner).** Add the optional `claims[].category` and `claims[].priority` members, rather than
  using prose only. *Recommended:* add them, because they make a workflow's output checkable JSON.
- **D3 (the planner).** A refused install answers `INVALID_ARGUMENT` with a remediation naming
  `--replace`, rather than a new failure code. *Recommended:* reuse the code, since a new one would also
  move the trial freeze.
- **D4 (the planner).** The skill classes `skill install` `never`, and `status`, `show` and `handoff
  skeleton` `free`. *Recommended:* as stated.
- **D5 (the maintainer).** The release ships only `skills/vsift/`, not an `.agents/skills/` tree, and
  `skill install` writes the client layouts. *Recommended:* yes.
- **D6 (the planner).** The plugin's skill is a real copy held equal by a test, not a symbolic link. The
  marketplace owner is "VSift project". People pin the release tag. *Recommended:* as stated.
- **D7 (the planner).** One file per workflow serves both as the skill's procedure and as the plain
  prompt for clients without skills. *Recommended:* one file.
- **D8 (the maintainer).** The clients named in `--client` and the docs. *Recommended:* the eight in the
  table. Windsurf is documented only as served by `.agents/skills/`.
- **D9 (the planner).** The new command's name, `handoff skeleton`. `handoff start` or `handoff new` are
  the alternatives. *Recommended:* `skeleton`, which says it holds no findings.

## Consequences

- An agent in any of nine clients finds the skill after one command run by the person. An agent with
  only the CLI finds the method, the workflows and a safe way to start a handoff from `vsift --help`.
- Every workflow's result is checkable: `handoff check --session` resolves each citation.
- The public surface grows: one `handoff` operation, a `skill` namespace with three operations, and two
  optional handoff members. All are additive under v1.
- The skill text grows by six workflow files. Each is a frozen trial component, so the freeze moves at
  each merge rebase, and agents have not measured any of it until the cut.

## Alternatives

- **A script example in the skill to build the handoff.** Rejected: the skill forbids every program but
  `vsift`, and the hazard was in a client without the skill.
- **Only documenting the copy for more clients.** Rejected: that does not give the one-step install the
  brief asks for.
- **Shipping `.agents/skills/` and `.claude/skills/` trees in the release.** Rejected (D5): two more copies
  to keep equal, and a person still has to find the right folder.
- **A symbolic link for the plugin's skill.** Rejected (D6): it breaks Windows checkouts.
- **A second schema for workflow findings.** Rejected (D2): the handoff schema with one optional member
  already does the job, and `handoff check` already runs it.

## Outcome: the planner's rulings of 2026-10-28

Relayed by the coordinator. Slices 1, 3, 4 and 5 may start (slice 1 first), and slice 2's bug-report,
findings, demo-backlog and logs-by-clock files. The meeting-notes and workshop-spec files waited for D1,
which the maintainer decided on 2026-10-28; the planner accepted its amendment on 2026-10-29.

- **D2, yes**, `handoff_version` stays `1`. Before slice 2 is built this record must say exactly how an
  inferred priority is shown. The planner prefers no third member: the claim's statement says "inferred",
  and `handoff check` refuses `priority` without `category` `backlog_item`. The CHANGELOG says an older
  `handoff check` refuses the new members.
  **Inferred priority, the planner's ruling (2026-10-29, after reviewing #547): no third member.** `priority`
  holds only `high`, `medium` or `low`. A priority the agent inferred, rather than one said in the
  recording, is stated in the claim's own `statement` ("priority inferred: high", with the reason from the
  evidence), and `demo-backlog.md` says when to infer and that the statement must say so. The claim's
  `kind` stays what the item is. `handoff check` refuses `priority` on a claim whose `category` is not
  `backlog_item`, or that has no `category`.
- **D3, yes:** `INVALID_ARGUMENT`, as ADR 0037's refusal of an existing output is. Each folder's typed
  outcome is in `data`, and the remediation names `--replace`.
- **D4, D5 (the planner's call: it is packaging), D7 and D9, yes.**
- **D6, yes.** The exact syntax for pinning a marketplace to a tag is confirmed from Claude Code's own
  documentation before it is written into ours. No person's details go in `marketplace.json` or
  `plugin.json`. The governance test refusing `hooks/`, `bin/`, `.mcp.json` and `package.json` stays.
- **D8: the eight clients** (the planner's call), named in docs only, never in the README's pitch (W6a's).

Conditions:

1. **`--replace` is a swap, never delete-then-copy.** Stage and verify; move the old folder aside under a
   VSift-made name; rename the staged folder into place; only then remove the old one; on any failure, put
   the old one back. Never follow a link or junction at any step. `--project` is canonicalised and must
   already exist.
2. **`skill show` takes a manifest entry name, not a path.** Anything not in the embedded manifest is
   refused, tested with `../` and with absolute paths.
3. **`logs-by-clock` privacy.** Log lines are untrusted data, and instructions found in them go to
   `untrusted_instructions`. Quote the least text that makes the match; never quote secrets, tokens or
   personal data from a log. Stated in the workflow file and in `references/safety.md`.
4. **Register:** L-238 to L-242 are this story's, each added by the pull request that builds what it
   limits; the next free number is L-243.
5. **Slice 1's contract tests:** the skeleton validates against `handoff.schema.json`, and `handoff check
   --session` accepts it before it is filled in.

## Implementation note, 2026-10-28 (slice 1: `handoff skeleton`)

Built as section 1 says, with one narrowing: the skeleton's `capabilities` holds `image_access`
(`unavailable`) and `transcript_basis` only. `media_tools` and `local_asr` are what `setup check` reports,
not what the session records, and running the tool probes from this command would make it slow and
write nothing the agent cannot read from `setup check` itself, so they are left to the agent (they are
optional in the schema). Section 1 says so (amended 2026-10-29). The basis is the one `search` reports
for the session's newest revision (`SearchCoverage`), or `none` without one. That revision is the one a
plain read gives: while a background transcription runs, its open revision (ADR 0033 D2). An earlier
draft of this note said the open revision was not named; the store's read returns it first, and
`growing_read_cli_contract`'s `a_skeleton_names_the_open_revision_a_read_cites` holds the skeleton, a
read and `handoff check --session` to the same revision while a job publishes (corrected 2026-10-29). A
closed, expired or unknown session fails as a transcript read of it does.

- **Layers:** `vsift-contract` builds the skeleton (`HandoffChecker::skeleton`) and checks it with the
  same checker a draft gets, so it cannot ship a shape the check refuses; the engine reads the facts
  (`Engine::handoff_skeleton_facts`); the CLI composes and renders (the human form is the fenced block).
- **The skill:** `references/commands.md` (the `free` row and the form) and `references/handoff.md` (one
  paragraph). `SKILL.md` and its fuzz seed copy are unchanged. The guard gains the placeholder
  `<question>`. The trial freeze is a draft freeze, rewritten at the merge rebase.
- **Tests:** see `cli-v1.md`, "0.3.0 `handoff skeleton`". Condition 5 is
  `a_skeleton_names_the_session_and_passes_the_check_unfilled` (through the binary) and
  `a_skeleton_matches_its_frozen_example_and_the_handoff_schema`.
- **No register entry:** none of L-238 to L-242 limits this slice.

## Implementation note, 2026-10-29 (slice 3: `skill install`, `status`, `show`)

Built as section 3 and conditions 1 and 2 say, with these narrowings and additions:

- **`replaced` names what was there** (`different` or `link`), not its digest; `skill_digest` names this
  release's skill.
- **A new state, `parent_not_folder`:** when `.claude`, `.agents` or their `skills` folder is a link, a
  junction or a file, nothing below it is read and the folder is always refused, also with `--replace`
  (condition 1: never follow a link at any step).
- **`status` reports both folders of one scope** (yours, or `--project`'s), not both scopes at once.
- **`show` is pure:** it reads only the embedded manifest (condition 2), refusing `../`, absolute paths,
  folders and the PNG; nothing on the disk is read.
- **What a failure leaves.** A failed stage or create removes only its `.vsift-skill-stage-*` folder. If the
  second rename of a swap fails, the old folder is renamed back; should that rename fail too, it keeps its
  `.vsift-skill-old-*` name and is never removed. After a successful swap, the old folder is removed
  best-effort; a leftover keeps that name.
- **Layers:** the catalogue, the states and the install decision are domain values (`agent_skill`); the use
  case and the `SkillFolders` port are the application's; `FilesystemSkillFolders` does the disk work; the
  engine's facade (`Engine::install_skill`, `Engine::skill_status`) returns contract data with its own
  `SkillError`, so `EngineError` is unchanged; the CLI composes. No new failure code.
- **Register:** L-241 (the eight clients as their documentation gave them on 2026-10-28).

## Implementation note, 2026-10-29 (slice 4: the Claude Code plugin)

Built as section 4 says, with three differences, each for a reason found while building. **The planner
accepted all three on 2026-10-29**; each ruling is stated with its difference.

- **`plugin.json` states no `version`.** Claude Code keeps a person on a plugin's stated version until it
  changes, so a stated version would have to move with every release, and the release tools let a stable
  release differ from its candidate only in the version-string files they list
  (`tools/vsift-release/src/candidate.rs`, `tools/release_window.py`). Without it the plugin follows its
  marketplace source, which the docs pin to the release tag. The plugin rule refuses a `version`.
  *The planner's ruling (2026-10-29): accepted.* The tag pins the version; L-240 already says a plugin added
  from `main` can be ahead of the installed release, and the install text says to add it at the release tag.
- **The tag syntax is confirmed** from Claude Code's own install documentation (read 2026-10-29 with no
  identifying detail sent): a GitHub marketplace source takes `#ref` to pin a branch or tag, so
  `/plugin marketplace add smormah/vsift#v<version>`, and `claude plugin marketplace add` takes the same
  sources. The marketplace exists from the 0.3.0 tag on.
- **No slash commands yet.** The commands point at workflow files, which slice 2 builds, so each workflow's
  pull request adds its `commands/<workflow>.md`. *The planner's ruling (2026-10-29): accepted.* Each
  workflow's slash command comes with that workflow in slice 2, and if the commands name skill files, the
  skill guard's byte-for-byte test covers `plugins/vsift/commands/` too.
- **No subagent.** A plugin subagent preloads skills by name, and the documentation does not settle how a
  plugin's own skill is named there; a wrong name is skipped without an error. *The planner's ruling
  (2026-10-29): the subagent is cut from 0.3.0*, the first item of section 8's cut list. It is recorded as
  cut, not deferred to a date.
- **`claude plugin validate` was not run** (nothing is installed on the development machine for this pull
  request). *The planner's ruling (2026-10-29): acceptable;* the governance rule and its manifest tests
  stand in for it, and the 0.3.0 try-out adds the plugin from the release tag and records whether it loads.

- **Layers:** no Rust crate changes. The governance check gains the plugin rule
  (`tools/vsift-governance/src/claude_plugin.rs`): the marketplace has exactly the reviewed members, the owner
  `{"name": "VSift project"}` and one entry, `vsift` at `./plugins/vsift`; `plugin.json` holds only `name`,
  `description`, `license` and `repository`; nothing under `plugins/` is named `hooks`, `bin`, `monitors`,
  `.mcp.json`, `.lsp.json` or `package.json`, or is a link.
- **The copy:** `plugins/vsift/skills/vsift/` is a real copy of `skills/vsift/`. The skill guard's
  `the_plugins_skill_is_the_shipped_skill_byte_for_byte` fails on any file missing, added or different, so a
  skill change copies the folder again in the same change. The rebase onto #543 showed it working: the two
  skill files #543 changed had to be copied again.
- **The skill:** unchanged, so no new trial freeze.
- **Tests:** the plugin rule's unit tests (the reviewed manifests pass; a person, a component, a `version`,
  another source and every refused entry or link fail) and the byte-for-byte guard, which failed on a planted
  one-byte difference before it was removed.
- **Register:** [L-240](../planning/known-limits.md#l-240), a marketplace added without a tag can carry a
  newer skill than the installed `vsift`.

## Implementation note, 2026-10-29 (slice 2: the four cue-word workflows)

Built as section 2 says for `bug-report.md`, `findings.md`, `demo-backlog.md` and `logs-by-clock.md`, with the
planner's ruling on inferred priority (Outcome, D2). The read-through workflows (`meeting-notes.md`,
`workshop-spec.md`) and the `read_through` profile are the next pull request.

- **The files.** Each opens with D7's two sentences, names its budget and states that profile's limits
  (`compact`: 30 calls, 6 images, pages of 20; `standard`: 80, 24, 50), starts the JSON from `handoff
  skeleton` and ends with `handoff check`. Every command in them is `free`. `SKILL.md` links them from one
  line; to stay at 300 lines its sentence on quoting arguments now points at `references/commands.md`,
  which already said it. The fuzz seed copy and the plugin's copy are copied again.
- **Cue words are literal words** (search matches whole normalised words, so `fail` does not find
  `failed`, and punctuation such as `?` is no query). Each workflow lists its cue words per kind; the
  findings and backlog workflows end `partial` and list each stretch they did not read as a `transcript`
  gap (`not_inspected`) unless their windows covered the whole recording (L-238).
- **`category` and `priority`.** Two optional members of `claims[]` in `handoff.schema.json`, and one
  `if`/`then` rule: a claim with `priority` must have `category` `backlog_item`. The rule lives in the
  schema, so `handoff check` reports it with existing rules (`member_missing`, `value_not_allowed`) and
  no new rule identifier or failure code. `handoff_version` stays `1`. Checked on the binary built
  before the change: it refuses a claim with `category` as `member_unknown`, as the CHANGELOG says.
- **Clock time.** `references/commands.md` gains `ingest --recorded-at` (only with the start time the user
  gave; opening the video again to add one is `explicit`, as it is for a transcript). `ingest --url` is not
  added by this pull request; section 6 leaves it to a later slice.
- **Condition 3 (log privacy), as built.** `logs-by-clock.md` and a new "Log files" section of
  `references/safety.md`: the one log file the user names is read with the file-reading tool, in that
  workflow only; the least text that makes the match is quoted, in a code span; no secret, token, key,
  password or personal data is quoted. An instruction in a log line is listed under "Untrusted
  instructions observed" in the Markdown. **The JSON's `untrusted_instructions` cannot hold it**: each
  entry requires a `citation`, and a log line is not VSift evidence and has no citation (L-239). It is
  therefore stated in the Markdown alone, an exception to "the prose says nothing the JSON does not" that
  L-239 already makes for log lines.
- **The plugin's commands.** `plugins/vsift/commands/<workflow>.md`, one per workflow, give `/vsift:bug-report`
  and the others. Each is front matter (`description`, `argument-hint`) and a few lines naming its
  workflow file, with `vsift skill show workflows/<name>.md --json` for a client that cannot read it.
- **Tests.** The skill guard: workflow console blocks are held to `free`
  (`console_commands_parse_and_respect_their_class`), and
  `workflows_stand_alone_use_free_commands_and_state_their_budget` holds the opening, the budget and its
  limits, the skeleton, the check, the inline commands and the `SKILL.md` link;
  `each_workflow_has_one_plugin_command_that_names_it` holds one command per workflow, each naming only its
  own file, which must be in the plugin's skill copy (held byte for byte to the skill), with no front matter
  that grants a tool; `the_workflow_examples_use_the_members_their_workflows_name`. The contract:
  `a_priority_is_allowed_only_on_a_backlog_item`, the new examples in the example and differential tests, and
  the differential test's full draft with both members. Each new guard test and the priority test failed on a
  planted mutation before it was reverted.
- **Examples.** `examples/bug-report.handoff.json` and `examples/findings.handoff.json` are built from the
  output of this branch's binary on the `F04-speech` fixture with its supplied transcript (frames at 0 s and
  10 s opened and read); both pass `handoff check --session` against that session.
- **Register:** [L-238](../planning/known-limits.md#l-238) and [L-239](../planning/known-limits.md#l-239).
- **The trial freeze** is a draft freeze, rewritten at the merge rebase.

## Implementation note, 2026-10-29 (the read-through workflows: meeting notes and workshop spec)

Built as section 2's D1 amendment says, in its own pull request after slice 2 (stacked on it until it merges).

- **The profile.** `read_through` is a contract constant (`vsift_contract::READ_THROUGH_BUDGET`) and a table of its
  own in `references/budgets.md`, with the values of section 2's table: pages of 25, 120 transcript pages, 200
  tool calls, 60 minutes, and `standard`'s images, refinement and bursts. The table has its own columns
  (`read_through` and why), so the `compact`/`standard` table and both of its parsers (the skill guard and the
  trial harness) are unchanged. `budgets.md` now says the whole-transcript read is for these two workflows only,
  and that they search first too; SKILL.md's rule 6 (search before any `transcript get`) still holds for them.
- **The handoff.** `budget.profile` gains `read_through`; `budget.limits` gains the optional `transcript_pages`
  (null or 1 to 1,000); `budget.exhausted` gains `transcript_pages`, so a read the page ceiling stopped names it
  and needs the resume card. `handoff check` holds a given `transcript_pages` to the profile: 120 under
  `read_through`, refused under `compact` and `standard`, any value with `overrides`. `HandoffBudgetLimits` gains
  `transcript_pages: Option<u64>` (`None` for the other two profiles); its `members()` stays the eight limits
  every profile has. No new rule identifier or failure code; `handoff_version` stays `1`.
- **The workflows.** `meeting-notes.md` (items `decision`, `action_item`, `open_question`; speakers by
  `display_label`, never a name the recording does not say) and `workshop-spec.md` (`requirement`, `decision`,
  `open_question`). Each says a compact model, or another named profile, uses the cue-word method instead, before
  the first read; searches cue words first; sizes the read from the `revision` block (`segment_count`, the last
  `source_segments[].end_us`); reads every page with its `next_cursor`; asks a cut page again with `--limit 5`;
  follows a background transcription with `job status` and `--since`; and says "read all" only when the last
  page's cursor was null, no read was `partial` and any job had succeeded. A stretch a running job has not
  transcribed is a gap with reason `untranscribed_range` and the resume card naming the job, as
  `references/resume.md` already says. Each has its plugin command.
- **The example.** `examples/meeting-notes.handoff.json` is a whole-transcript read of the `F04-speech` fixture
  with its supplied transcript, on this branch's binary: three cue-word searches, one page of 25 whose
  `next_cursor` was null, one `action_item`, `budget.profile` `read_through` with every limit given. It passes
  `handoff check --session` against that session. No fixture is a real meeting, so the example shows the shape,
  not a long read.
- **Tests.** `the_read_through_profile_is_budgets_md` (the table against the constant, and only `read_through`
  has a page count); the workflow test also holds "at most 120 transcript pages"; the examples test holds the
  meeting-notes example to `read_through` and meeting items; the contract's
  `read_through_limits_and_its_page_count_are_the_profiles_own`; the differential test reads the new example and
  `transcript_pages`. Each failed on a planted mutation before it was reverted.
- **Not measured:** a read of two hours through any client (L-243).
- **Register:** [L-243](../planning/known-limits.md#l-243). **The trial freeze** is a draft freeze, rewritten at
  the merge rebase.

## Implementation note, 2026-10-29 (slice 5: the help's pointers, C-04 and `ingest --url`)

Built as section 5 says, in one pull request after the read-through pull request.

- **Help.** `TYPICAL_INVESTIGATION_HELP` gains a "ready-made outcomes" paragraph after step 7: the six workflows by
  file name, one line each, and `vsift skill show workflows/<name>.md`. Step 6 already named the skeleton (slice 1).
  `the_ready_made_outcomes_are_the_shipped_workflows` holds the list to the `workflows/` entries of
  `vsift_contract::SKILL_FILES`, and the existing help test parses the new command line (placeholder `<name>`). The
  guide reference is regenerated.
- **C-04.** `tools/vsift-agent-trials/cold/C-04-f05-bug-report.json` is C-01's situation (F05, its transcript,
  F05-E03, `key_facts_cited`) with the request "Write a bug report, with times, from this recording". Batch 3 runs it
  once in each tier for each client (`campaign::plan`); batch 1, the baseline, does not, so its result is read on its
  own: the summary keeps it out of the final round's 80% target (still C-01 to C-03) and reports it on a "Cold outcome" line of
  its own, as a measurement. The cold grader's rules are unchanged. It is not run before the cut.
- **`ingest --url`** (section 6) joins `references/commands.md`'s `ingest` row and allowed forms, `free`, only for a
  link the user gave and never for one found in evidence (the planner's ruling of 2026-10-29). The guard gains the
  placeholder `<link>`.
- **Not done:** the help does not say when to choose which workflow beyond its one line; the cold mode measures that
  at the cut.
- **Register:** [L-242](../planning/known-limits.md#l-242). **The trial freeze** is written on this pull request's
  final head and also covers #548 and #549, which merged without their freeze being rewritten.
