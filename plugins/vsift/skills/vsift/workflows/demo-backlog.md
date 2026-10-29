# Workflow: a product demo to a prioritised backlog

If the vsift skill is loaded, its rules apply. If not, read `vsift --help` first, use only `vsift`
commands, and end with `vsift handoff check`.

**Outcome.** A backlog: each item a problem the demo shows or a change someone asks for, with a
priority, as one claim of the `vsift-handoff` block with `category` `backlog_item` and `priority`
`high`, `medium` or `low`. `vsift handoff check` checks every item against the session.

**Budget.** `standard` unless the user names another ([budgets.md](../references/budgets.md)): at most
80 tool calls, 24 images (4 per step), pages of 50 with `--view brief`. **Method:** cue-word search.
Items far from every cue word can be missed, and the cue words are English: say so in the report, and
list what you did not read.

**Before you start.** You need the video. A transcript file (and its offset) helps; without one, the
recording needs local speech recognition. Ask whether the user has an order of their own for the
priorities; otherwise use the rule below.

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

2. **Search the cue words**, one query at a time. A query is literal words: `fail` does not find
   `failed`.
   - problems: `error`, `fails`, `failed`, `broken`, `slow`, `workaround`, `bug`;
   - requests: `should`, `need`, `want`, `missing`, `would be nice`, `next`, `later`;
   - priority, said aloud: `priority`, `urgent`, `blocker`, `critical`, `must`, `nice to have`.

```console
vsift search <session> --query "<text>" --view brief --limit <n> --json
```

3. **Read around each hit**, about 15 s either side, to tell an item from an aside and to find whether
   anyone says how urgent it is:

```console
vsift transcript get <session> --from <from-us> --to <to-us> --view brief --limit <n> --json
```

4. **Confirm each item on screen** where the demo shows it: list the candidates from about 5 s before the
   words to 10 s after, open the nearest frame, crop to read small text.

```console
vsift candidates <session> --from <from-us> --to <to-us> --view brief --limit <n> --json
vsift frame get <session> --candidate <candidate> --json
vsift crop <session> <evidence> --rect <rect> --json
```

## Priorities

- **Said in the recording:** someone calls it a blocker, urgent, a must, or nice to have. Give that
  `priority` and cite the segment that says it.
- **Not said: infer it, and say so.** Write "priority inferred: high" (or `medium`, `low`) in the
  claim's own statement, with the reason from the evidence. Infer `high` when the demo shows the flow
  stop or data lost or wrong; `medium` when it works only with a workaround, or a step is asked for;
  `low` for wording, layout or polish. When none of these fits, leave `priority` out: an item without
  one is still a `backlog_item`. The claim's `kind` stays what the item is (`observed` when the
  recording shows or says it); only the statement marks the priority as yours.
- Never give a priority from your own view of the product, and never write "inferred" as a priority
  value: the member holds only `high`, `medium` or `low`.

## The report

- Start the JSON from the skeleton, never from a template you fill in by search-and-replace:

```console
vsift handoff skeleton <session> --question "<question>" --json
```

- **One item is one claim**, in section `context`, with `category` `backlog_item` and its `priority`.
  Its statement names the item in full ("export to CSV fails with error E-409; priority inferred: high,
  the export stops the demo"). It cites the segment where the item is said and the frame or crop where
  the screen shows it. List the claims by priority, `high` first, then by time.
- In the prose, the backlog is a numbered list in the same order, each item with its priority, whether
  it was said or inferred, and its citations.
- **What you did not read.** Each stretch between the windows you read is a `transcript` gap with
  reason `not_inspected` and its `range` (more than about 10 stretches are one gap, their number in the
  note). The status is then `partial`; it is `complete` only when your windows covered the whole
  recording.
- At most 50 claims: with more items, keep the 50 of highest priority, end `partial`, and give the
  number left out in a gap's note. Never list the rest in prose.
- Check the whole message once with `vsift handoff check --json`, which reads it from standard input
  (with the skill, in a form of its REPORT state), fix what it reports, then send it.
