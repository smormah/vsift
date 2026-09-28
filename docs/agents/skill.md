# The VSift agent skill

Status: P12 increment (2026-09-28). The skill exists and its command and schema
references are held to the CLI by tests; it has **not** yet been qualified with named
agent clients (A-01..A-09, SEC-T02). Design: [ADR 0022](../decisions/0022-agent-skill-and-named-client-qualification.md) (Proposed).

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
  executable are never run by the skill.

## Keeping it in step with the CLI

`crates/vsift-cli/src/skill_contract.rs` (unit tests of the CLI crate, run by `cargo
test --workspace`) fails when the skill drifts from the CLI:

- every `vsift` line in a `console` block of the skill (and of this page) parses with
  the real parser, uses `--json` or `--events jsonl` and no operator-only option, and
  has the class its place requires;
- every inline `vsift` command names a real operation and every flag
  exists on it; every inline long option exists;
- the policy table classifies every public command exactly once, and the `never` and
  `explicit` sets match the reviewed lists in the test;
- every upper-case code is a published failure code (or a state name); every field or
  reason name resolves in the v1 contract (`schemas/v1` or `docs/contracts/cli-v1.md`)
  or the handoff schema;
- the example handoffs validate against `handoff.schema.json` and its citation,
  image-access and resume-size rules;
- the image check's code appears in no text file; `SKILL.md` stays within 300 lines,
  holds only `name` and `description` in its front matter and links every reference.

A new command, flag or failure code therefore needs a skill update in the same change.

The check image was generated once with FFmpeg's `drawtext` filter (white 360x96
background, black bold text, greyscale PNG, metadata stripped). To replace it, choose
a new code word, render it the same way, and update the code word held in the test;
never write the code into any skill or documentation file.

## Not yet done

Named-client trials (A-08/A-09 through Claude Code and Codex, the compact-model
gates, SEC-T02 adversarial evidence) are later P12 work; see ADR 0022 for the planned
protocol. Until they pass, the skill is a candidate, not a qualified integration.
