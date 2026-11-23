# Workflow: a recording to an evidence-backed bug report

If the vsift skill is loaded, its rules apply. If not, read `vsift --help` first, use only `vsift`
commands, and end with `vsift handoff check`.

**Outcome.** A bug report (Problem, Expected, Actual, Reproduction steps) whose every claim cites a
transcript segment, frame or crop of the recording, ending in one `vsift-handoff` block that `vsift
handoff check` accepts.

**Budget.** `compact` unless the user names another ([budgets.md](../references/budgets.md)): at most
30 tool calls, 6 images (1 per step), pages of 20. **Method:** search for the symptom, read windows
around it, look at the screen there. The report covers those stretches, not the whole recording.

**Before you start.** You need the video and the symptom in the user's words: an error text, a wrong
value, a step that fails. A transcript file (and its offset) helps. Ask only for what is missing.

## Steps

1. **Tools.** Run this once; FFmpeg and FFprobe must be `available`. If not, stop and report the
   missing tool as a `dependency` gap, quoting VSift's remediation:

```console
vsift setup check --json
```

2. **Open the recording.** Keep `session_id` (and `revision_id` with a transcript). With no transcript
   and `local_asr.verification.status` `verified`, transcribe once; with neither, go on with the screen
   and record a `transcript` gap.

```console
vsift ingest <video> --json
vsift ingest <video> --transcript <transcript> --transcript-offset <offset-us> --json
vsift job start <session> --json
```

3. **Find the symptom.** Search the user's own words first, one query at a time, then these cue words
   if the budget allows: `error`, `failed`, `fails`, `wrong`, `doesn't`, `expected`, `should`. A query
   is literal words (`fail` does not find `failed`).

```console
vsift search <session> --query "<text>" --view brief --limit <n> --json
```

4. **Read around each hit**, about 15 s either side, to find the steps before it and what was expected:

```console
vsift transcript get <session> --from <from-us> --to <to-us> --view brief --limit <n> --json
```

5. **Look at the screen** from about 5 s before each hit to 10 s after it: list the candidates, open the
   nearest frame, and crop it to read small text. Describe only what the pixels show.

```console
vsift candidates <session> --from <from-us> --to <to-us> --view brief --limit <n> --json
vsift frame get <session> --candidate <candidate> --json
vsift crop <session> <evidence> --rect <rect> --json
```

## The report

- Start the JSON from the skeleton, never from a template you fill in by search-and-replace:

```console
vsift handoff skeleton <session> --question "<question>" --json
```

- **Problem**, **Expected**, **Actual** and **Reproduction steps** are the claim sections `problem`,
  `expected`, `actual` and `reproduction`. A bug report uses no `category` and no `priority`.
- **Expected** says who expects it: the user (`reported`), the speaker or a visible label (cited). When
  nothing says it, write "not stated in the video" and make no claim for it.
- **Reproduction steps** are the steps the recording shows or says, in order, each with its time and
  citation. Never add a step from your own knowledge of the product.
- A stretch you did not read, where more of the bug could be, is a `transcript` gap with reason
  `not_inspected` and its `range`; a frame you did not open there is a `visual` gap.
- Check the whole message once with `vsift handoff check --json`, which reads it from standard input
  (with the skill, in a form of its REPORT state), fix what it reports, then send it.
