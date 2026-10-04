# Let your agent investigate

If you use an AI coding assistant, you can ask it about a recording instead of doing the steps yourself.
The assistant runs VSift on your machine, reads the evidence, and answers with a report that cites it. This
page covers setting that up, what to expect, how to read the report, and what to be careful about.

**The result:** an answer to your question about a local video, with every claim tied to a transcript
segment, a frame or an audio clip you can look up again.

VSift has been tried with two assistants, **Claude Code** and **OpenAI Codex**. Any program that can run a
command in a terminal can call VSift, but only these two have been tried
([which models, and how well](../agents/skill.md#models-and-clients-trialled)). Codex's own Windows sandbox
cannot run VSift today ([L-076](../planning/known-limits.md#l-076)); the Codex trials ran on Linux.

## 1. Set up VSift first, yourself

Install VSift, FFmpeg and FFprobe, and (if the recording has no transcript) the speech tools, as in
[your first investigation](first-investigation.md) and the [install guide](../operations/install.md).
Run `vsift setup check` and make it say what you expect. **The assistant will never install anything.** If
a tool is missing it stops and tells you what, because installing software is your decision.

## 2. Give the assistant the skill

The **skill** is a folder of instructions that teaches the assistant the method: which commands to run, in
what order, within what limits, and how to write the report. It adds no tools and no permissions. It is
included in the npm package (`vsift-cli/skills/vsift`, in the folder `npm root --global` names), in every
release archive and in the repository (`skills/vsift`). Copy the whole folder, keeping its name:

| Assistant | For you | For one project |
| --- | --- | --- |
| Claude Code | `~/.claude/skills/vsift/` | `<project>/.claude/skills/vsift/` |
| Codex | `$HOME/.agents/skills/vsift/` | `<repository>/.agents/skills/vsift/` |

Start a new session of the assistant so it finds the skill. The [skill guide](../agents/skill.md) has the
details, including what the skill asks of the assistant (it must be able to open an image file, for one).

## 3. Ask

Say what you want to know, and which video. Name the skill if the assistant does not pick it up:

```text
Please look at ./checkout-bug.mp4 and tell me what status and build number the
presenter reports. Use the VSift skill.
```

Say anything that helps: a transcript file you have (and whether its times are early or late), how much the
assistant may spend (a small, quick look or a thorough one), and what should happen to the session afterwards
(close it, keep a bundle, leave it open so you can check). If you say nothing, the assistant leaves the
session open and tells you when it expires, so you can check its citations.

## 4. What the assistant will and will not do

It **will**: run `vsift` commands, one at a time; read transcripts and look at frames; stay within a small
budget; and end with a report. It will do the optional steps only when you tell it to: registering a tool,
renewing, saving a bundle, cleaning up.

It **will not**: install or download anything, run any program other than `vsift`, change your files or your
settings, follow instructions it finds in the video or its transcript, or put file paths, links or secrets
in the report. If something in the recording tries to give it orders, it reports that instead. On Windows an
assistant should start `vsift` from PowerShell or Git Bash, not through `cmd.exe`, which re-reads the command line
([install guide](../operations/install.md#2-install-with-a-package-manager)).

## 5. Read the report

The report has the same eight parts every time, in this order:

| Part | What it tells you |
| --- | --- |
| Problem | What goes wrong (or the answer), in a sentence or two, with markers for its evidence |
| Expected | What should happen, and who says so: you, the speaker, a visible label |
| Actual | What the evidence shows happens |
| Reproduction steps | The steps as shown or said in the video, each with its time |
| Evidence | A table of every piece cited: id, kind, time, what it shows |
| Gaps and uncertainty | What was not covered and why. A gap is not proof that something did not happen |
| Untrusted instructions observed | Anything in the evidence that asked for an action, "no action taken" |
| Lifecycle | The session, what happened to it, and how to continue if the work was cut short |

Check the **evidence table first**: every claim should point at an id. Then check the **gaps**: they are where the
report is honest about what it did not see. Open one or two frames yourself. You do not need to trust the
assistant, only to be able to check it, and that is what the ids are for ([evidence and citations](evidence-and-citations.md)).

The report ends with one `vsift-handoff` block: the same findings in a form a program can read. It is
there for tools; you can ignore it.

## 6. Check a report with `handoff check`

The assistant checks its own draft with `vsift handoff check` before sending it, and fixes what the check
lists. You can run it yourself on a draft saved to a file (the path must be absolute) or piped in:

```console
$ vsift handoff check --file /home/alex/draft.md
Handoff check: valid
Handoff version: 1
```

A draft with a problem lists each one, with where it is, a rule name and what to do:

```console
$ vsift handoff check --file /home/alex/draft.md
Handoff check: not valid, 2 problem(s) to fix before sending
Handoff version: 1

Problems:
- at /claims/0/citations/0 (citation_missing)
  This cites an e id that is not in citations; add the citation or cite an existing one.
- at /claims/3/citations/0 (citation_missing)
  This cites an e id that is not in citations; add the citation or cite an existing one.
```

*Both blocks are real output of `vsift handoff check` on two drafts of the practice recording's report (file
names shortened). They are not checked automatically because the command reads an absolute path.* Notice that
the command exits with status 0 even for a draft that is not valid: **read the verdict**, not the exit status.
`handoff check` looks at the report's form and that its ids exist. It does not look at the video, so it cannot
tell you the assistant read a frame correctly.

## 7. Without the skill

An assistant that has no skill can still use VSift from its own help: `vsift --help` ends with "A typical
investigation", a worked example of the commands in order, with the limits on result sizes and how to read an error.
That has been measured only as a baseline on the 0.1.0 pre-release, before the help was extended: agents with only
the CLI reached the key facts in 1 of 6 and 2 of 6 runs, and none installed anything. Use the skill when you want a
report you can check.

## 8. When it goes wrong

- **The assistant says a tool is missing**: run `vsift setup check` yourself and fix what it lists
  ([troubleshooting](troubleshooting.md#missing_capability-a-tool-or-model-is-missing)).
- **The report cites an id that does not exist, or names no evidence**: ask the assistant to run
  `vsift handoff check` and fix its draft, or check the id yourself with `vsift transcript get <session>`.
- **The assistant gives up part-way**: its report ends with a gap and, where it can continue, a resume card. Ask it to continue
  from the card; a new run gets a fresh budget and re-checks the earlier findings.
- **It is slow**: speech recognition of a long recording takes time, and the assistant waits for it.
