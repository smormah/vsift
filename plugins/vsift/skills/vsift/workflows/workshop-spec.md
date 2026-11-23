# Workflow: a workshop to a spec with quoted decisions and open questions

If the vsift skill is loaded, its rules apply. If not, read `vsift --help` first, use only `vsift`
commands, and end with `vsift handoff check`.

**Outcome.** A spec drawn from a workshop: each requirement, decision and open question one claim of the
`vsift-handoff` block, with `category` `requirement`, `decision` or `open_question`, quoting what was
said and citing its segment. `vsift handoff check` checks every item against the session.

**Budget.** `read_through` unless the user names another ([budgets.md](../references/budgets.md)): at
most 200 tool calls, 24 images (4 per step), pages of 25, and at most 120 transcript pages, about two
hours of speech. **Method:** read the whole transcript, page by page. This needs a model with a long
context window (two hours are about 0.5 to 1.4 million characters of tool output). **If you are a
compact model, or the user names `compact` or `standard`, do not read it whole:** say so before the
first read, search the cue words in step 3, read about 15 s around each hit, and list each stretch you
did not read as a gap, as the findings workflow does.

**Before you start.** You need the video, and what the spec is for if the user knows (a feature, a
screen). A transcript file (and its offset) helps; without one, the recording needs local speech
recognition. Ask only for what is missing.

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

2. **A long recording transcribes in the background** (`data.start.runner` `background`). Follow it with
   `job status`, and read what was published after an earlier answer with its `data.since.since_token`
   and `--since`, which needs no range. Never start a second transcription.

```console
vsift job status <job> --json
vsift transcript get <session> --since "<since-token>" --view brief --limit <n> --json
```

3. **Search first**, even though you will read everything: the hits are the ids you cite, and they
   mark where to look hardest. One query at a time: `must`, `should`, `needs to`, `requirement`,
   `decided`, `agreed`, `out of scope`, `not sure`, `open question`, `later`. The first answer's
   `revision` block gives `segment_count` and the recording's end, the last `source_segments[].end_us`.
   More than 120 pages of 25 (3,000 segments) do not fit: plan to read the first 120 and list the rest
   as a gap.

```console
vsift search <session> --query "<text>" --view brief --limit <n> --json
```

4. **Read the whole transcript**, from 0 to the recording's end, 25 segments a page, and continue each
   page only with its `next_cursor`. Count the pages. A page whose output stops before its final
   `lifecycle` member was cut by your client: ask for it again with `--limit 5`; if it is still cut,
   that stretch is not read.

```console
vsift transcript get <session> --from <from-us> --to <to-us> --view brief --limit <n> --json
vsift transcript get <session> --from <from-us> --to <to-us> --view brief --limit <n> --cursor "<cursor>" --json
```

5. **Where a whiteboard, slide or screen shows a requirement** (a sketch, a field list): open that frame,
   within the image limits, and crop it to read small text.

```console
vsift frame get <session> --at <us> --json
vsift crop <session> <evidence> --rect <rect> --json
```

## The report

- Start the JSON from the skeleton, never from a template you fill in by search-and-replace:

```console
vsift handoff skeleton <session> --question "<question>" --json
```

- Give `budget` with `profile` `read_through`.
- **One item is one claim** in section `context`: a `requirement` (what the thing must or should do), a
  `decision` (a choice the workshop agreed) or an `open_question` (raised and not settled). Quote the
  words from `display_text` and cite the segment that says them; when a frame shows the item, cite it
  too. Name a speaker only by `display_label`, the label as VSift gives it, never a name the recording
  does not say. Write a requirement as said: never add one from your own knowledge of the product, and
  never turn "maybe" into "must".
- In the prose, the spec is three lists in that order (Requirements, Decisions, Open questions), each
  item with its time and citation.
- **"Read all" only when true.** Say you read the whole transcript only when the last page's
  `next_cursor` was null, no read was `partial`, and any transcription job had succeeded. Otherwise say
  which stretch you read, from VSift's own times.
- **What you did not read** is a `transcript` gap with its `range`: `not_inspected` (past the page ceiling,
  a cut page), or `budget_exhausted` with `transcript_pages` or `tool_calls` in `budget.exhausted` and the
  resume card when the budget ran out. A stretch a job still running has not transcribed
  (`transcript_coverage.pending_ranges`) is a gap with reason `untranscribed_range`, with the resume card
  naming the job and `vsift job status <job> --json`; so is any stretch with no transcript, never silence.
  The status is then `partial`.
- At most 50 claims: with more items, keep the requirements and decisions first, then the questions, end
  `partial`, and give the number left out in a gap's note. Never list the rest in prose.
- Check the whole message once with `vsift handoff check --json`, which reads it from standard input
  (with the skill, in a form of its REPORT state), fix what it reports, then send it.
