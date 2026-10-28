# ADR 0038: AI integration without MCP: workflows, the skill everywhere, and the CLI alone

- Status: **Proposed, 2026-10-28.** Nothing it describes is built. The decisions in
  [Decisions for the planner and the maintainer](#decisions-for-the-planner-and-the-maintainer)
  are open. Each is marked with whose call it is.
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
  `media_tools`, `local_asr` and `transcript_basis` from the session's records;
- the `session` ids, including the transcript revision when there is one;
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
| `workshop-spec.md` | A workshop becomes a spec with quoted decisions and open questions | `standard` | Search decision and requirement cue words, quote each decision from `display_text` with its segment, and list open questions. |
| `demo-backlog.md` | A product demo becomes a prioritised backlog | `standard` | Search problem and request cue words, confirm each item on a frame, and give each a `priority` from what was said (cited) or `inferred`. |
| `meeting-notes.md` | A meeting becomes actions, decisions and open questions | `standard` | Search action, decision and question cue words, and name speakers by `display_label` when segments carry one. |
| `logs-by-clock.md` | Moments in a recording are matched to system log lines through clock time | `compact` | Ingest with `--recorded-at` when the user knows the start. Turn each log time into a window, then cite the segments and frames whose own `clock_time` VSift reports. |

Every workflow uses only `free` commands. Each stays inside its profile and ends with `vsift handoff
check`, so its result is a checkable handoff. That is the difference from a summary: every item cites a
segment, frame or crop that `handoff check --session` resolves.

**Long recordings.** Most meetings, workshops and demos run longer than 5 minutes. The maintainer's
ruling for #368 says a summary of a whole long recording does not fit the budgets. That ruling also lets
the agent start when the user has said what to hand back if the budget runs out. A workflow is that
statement. It finds items by cue-word search, ends `partial` and lists every stretch it did not read as a
`transcript` gap with reason `not_inspected`. It never claims to have read the whole meeting. This keeps
the #368 ruling unchanged (decision D1). The cue words are English, so other languages find less
(L-238).

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
inline command and flag exists. The budget a workflow names is a profile in `budgets.md`. Three example
handoffs (bug report, findings, meeting notes) are built from fixture output and validate against the
schema, including the new members.

**Cost:** about two days and one pull request (the six files, the two handoff members with their check
rules, the guard and three examples).

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

1. The plugin's subagent.
2. `vsift skill show`, with help naming the skill's files instead.
3. `claims[].priority`. The backlog's order then lives in prose, with `category` `backlog_item`.
4. The three example handoffs beyond the first (keep bug report).
5. `vsift skill status`. `install` already reports what it found.
6. C-04. The existing cold round still runs on the new help.
7. The plugin entirely. `skill install --client claude-code` reaches Claude Code.
8. `logs-by-clock.md`, if clock time slips from 0.3.0.

**Never cut:**

- the skeleton (the hazard that started this);
- the other five workflow files and the two handoff members' `category`;
- `skill install` for both folders;
- the help pointers;
- item 7 for whatever has merged.

### 9. Proposed register entries

- **L-238:** Workflows over a long recording find items by cue-word search within the budget. An item far
  from any cue word can be missed. The report says which stretches were not read, and the cue words are
  English.
- **L-239:** No command takes a clock time. The agent subtracts the recording's start to find a window.
  Log lines are not citable evidence in handoff v1, so they are quoted in prose as untrusted data.
- **L-240:** A plugin added from `main` can carry a newer skill than the installed `vsift`. The docs say
  to pin the release tag.
- **L-241:** `skill install` knows only the clients whose folders were confirmed from their docs on
  2026-10-28. A client that moves its folder needs a new release, and the manual copy still works.
- **L-242:** The workflows, the plugin and the install command are untried by agents until the 0.3.0 cut.
  The text ships, but the behaviour is unmeasured.

## Decisions for the planner and the maintainer

- **D1 (the maintainer).** Workflows over a long recording are cue-word sampled within `standard` and end
  `partial` with the unread stretches as gaps. This keeps the #368 ruling. A profile that reads a whole
  transcript is not in 0.3.0. *Recommended:* yes.
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
