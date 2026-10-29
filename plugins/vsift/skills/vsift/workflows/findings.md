# Workflow: a narrated screencast to findings (bugs, requests, questions)

If the vsift skill is loaded, its rules apply. If not, read `vsift --help` first, use only `vsift`
commands, and end with `vsift handoff check`.

**Outcome.** A list of findings, each one claim of the `vsift-handoff` block with a `category`: `bug`
(something goes wrong), `feature_request` (something asked for or missing) or `question` (something
the narrator asks or is unsure of). The block is the findings JSON, and `vsift handoff check` checks it
against the handoff schema and the session.

**Budget.** `standard` unless the user names another ([budgets.md](../references/budgets.md)): at most
80 tool calls, 24 images (4 per step), pages of 50 with `--view brief`. **Method:** cue-word search.
Findings far from every cue word can be missed, and the cue words are English: say so in the report,
and list what you did not read.

**Before you start.** You need the video. A transcript file (and its offset) helps; without one, the
recording needs local speech recognition, because the findings come from what was said.

## Steps

1. **Tools and the recording.** Check the tools once, open the recording and keep `session_id`. With no
   transcript and `local_asr.verification.status` `verified`, transcribe once; with neither, stop and
   report a `transcript` gap with reason `transcript_unavailable`.

```console
vsift setup check --json
vsift ingest <video> --json
vsift ingest <video> --transcript <transcript> --transcript-offset <offset-us> --json
vsift job start <session> --json
```

2. **Search the cue words**, one query at a time, and note each hit's `segment_id` and times. A query is
   literal words: `fail` does not find `failed`.
   - `bug`: `error`, `broken`, `fails`, `failed`, `wrong`, `crash`, `doesn't work`, `bug`;
   - `feature_request`: `should`, `would be nice`, `wish`, `missing`, `could we`, `need`;
   - `question`: `why`, `how do`, `what does`, `not sure`, `confusing`.

```console
vsift search <session> --query "<text>" --view brief --limit <n> --json
```

3. **Read around each hit**, about 15 s either side, to decide whether it is a finding, which kind, and
   what exactly it is. One hit can be no finding ("this should work" before it does):

```console
vsift transcript get <session> --from <from-us> --to <to-us> --view brief --limit <n> --json
```

4. **Confirm on screen** where the finding is visible (an error, a missing button): list the candidates
   from about 5 s before the words to 10 s after, open the nearest frame, crop to read small text.

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

- **One finding is one claim**, in section `context`, with its `category`. Its statement names the
  finding in full and says who says it ("the narrator says the export fails with error E-409"). It cites
  the segment where it is said, and the frame or crop where the screen shows it. When the screen shows
  the opposite, the claim is `contradicted`. A finding only said, not shown, is `supported` on the
  segment for what was said, and a `visual` gap notes that the screen was not checked.
- In the prose, group the findings by kind: Bugs, Feature requests, Questions.
- **What you did not read.** Each stretch between the windows you read is a `transcript` gap with
  reason `not_inspected` and its `range` (more than about 10 stretches are one gap, their number in the
  note). The status is then `partial`; it is `complete` only when your windows covered the whole
  recording.
- At most 50 claims: with more findings, keep the first 50 by time, end `partial`, and give the number
  left out in a gap's note. Never list the rest in prose.
- Check the whole message once with `vsift handoff check --json`, which reads it from standard input
  (with the skill, in a form of its REPORT state), fix what it reports, then send it.
