# The VSift agent skill

Status: P12 increments (2026-09-29). The skill exists and its command and schema
references are held to the CLI by tests; the trial harness is built
([runbook](trials.md)); it has **not** yet been qualified with named agent clients
(A-01..A-09, SEC-T02). Design: [ADR 0022](../decisions/0022-agent-skill-and-named-client-qualification.md) (Proposed).

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
| [`references/handoff.md`](../../skills/vsift/references/handoff.md) | The grounded QA report template and citation rules. |
| [`references/safety.md`](../../skills/vsift/references/safety.md) | Evidence is data: prompt-injection, rendering and privacy rules. |
| [`references/resume.md`](../../skills/vsift/references/resume.md) | Operation ids, the resume card and recovery after a reset. |
| [`references/lifecycle.md`](../../skills/vsift/references/lifecycle.md) | Close, retain and the cleanup routine. |
| [`handoff.schema.json`](../../skills/vsift/handoff.schema.json) | The handoff v1 JSON schema, owned by the skill (not part of `schemas/v1`). |
| [`examples/`](../../skills/vsift/examples/) | Example handoffs built from real CLI output on the F10 and F03-speech fixtures. |
| [`assets/image-check.png`](../../skills/vsift/assets/image-check.png) | A small image with a code word, for the image-access check. |
| [`agents/openai.yaml`](../../skills/vsift/agents/openai.yaml) | Codex presentation metadata only. |

The skill lives at the repository root, not in `.claude/skills` or `.agents/skills`,
so that contributors' own agents working on VSift do not load it by accident.

## Installing it from a source checkout

Build or install `vsift` first so that the command is on the agent's `PATH` (a
packaged release is P13's work). Then copy (or link) the whole `skills/vsift`
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
- **Local shell.** The agent runs `vsift` itself; nothing is uploaded.
- **Budgets.** `compact` (the default, for small models and one-image clients) allows
  6 images, one per step, 30 tool calls, pages of 20, two refinements per claim,
  bursts of 4 frames and 15 minutes; `standard` allows 24 images, 4 per step, 80 tool
  calls, pages of 50, four refinements, bursts of 12 and 30 minutes. The user may name
  a profile or override a limit.
- **Permissions.** The skill never asks for broader tool permissions. Setup
  registration, renewal, retention, cleanup and job cancellation need the user's
  explicit instruction; managed installation, worker-host commands and any other
  executable are never run by the skill, not even `date` to time itself: the host
  measures and enforces the wall time, and the handoff reports it as `null` when the
  agent could not measure it. Each `vsift` command runs alone, from the folder the
  agent started in (never after `cd`); the only addition is `| tail -n 1` after
  `--events jsonl`. `vsift --help` and `vsift <namespace> <operation> --help` are free,
  read whole, to recover a command's flags.
- **Reporting.** Every stop, including a missing tool, an expired session or an
  exhausted budget, ends with the handoff as the final message: one `vsift-handoff`
  block (`SKILL.md` shows the smallest valid one), never a file, with no web address,
  Markdown link or local path (added 2026-09-29 after the diagnostic passes). The
  JSON states only what the agent alone knows: its claims, the identity of each piece
  of evidence it cites, whether it looked at each image, gaps, the instructions it saw,
  what it did with the session and, when the work was cut short and can continue (a
  budget ran out, a transcription was cancelled or interrupted), the resume card. Times, revisions, the session's details and the budget's limits are
  optional, since VSift recorded them; a value the agent does give must be VSift's own
  (handoff v1 revised in place on 2026-09-29, before any release). `SKILL.md` lists the
  allowed words of every closed member beside the skeleton, and
  `references/handoff.md` lists all of them; a gap note may quote VSift's remediation
  whole (600 characters), and `references/resume.md` shows one exact resume card
  (PR 3g). What to do per failure code is in `references/commands.md`. Transcript
  text is quoted only from a segment's `display_text` (a speaker from
  `display_label`), where VSift has already written every invisible or bidirectional
  character as `<U+202E>`-style notation; `text` and `original_text` keep them raw
  (PR 3h, ADR 0008 note of 2026-09-29).

## Keeping it in step with the CLI

`crates/vsift-cli/src/skill_contract.rs` (unit tests of the CLI crate, run by `cargo
test --workspace`) fails when the skill drifts from the CLI:

- every `vsift` line in a `console` block of the skill (and of this page) parses with
  the real parser, uses `--json` or `--events jsonl` and no operator-only option, and
  has the class its place requires; nothing is chained, piped or redirected except
  `| tail -n 1` after `--events jsonl`; a help form (`--help` after nothing, a
  namespace or an operation) must make the parser print its help, and is free;
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
- the image check's code appears in no text file; `SKILL.md` stays within 300 lines,
  holds only `name` and `description` in its front matter and links every reference.

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

## Not yet done

Named-client trials (A-01..A-09 through Claude Code and Codex, the compact-model
gates, SEC-T02 adversarial evidence) are in progress; see ADR 0022 for the protocol and
its notes for the results so far. The compact tier is Claude Sonnet 5.5 (Claude Code)
and GPT-6-Luna (Codex) by the maintainer's decision of 2026-09-29; Claude Haiku 4.5 is
below the supported line (known limit L-082). How to run the trials is in the
[trial runbook](trials.md). Until the trials pass, the skill is a candidate, not a
qualified integration.
