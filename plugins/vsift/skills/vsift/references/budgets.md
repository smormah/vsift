# Budgets

A budget keeps an investigation bounded for the model and the machine. Use the
profile the user names; otherwise `compact`, which suits small models and one-image
clients. The user may also override any single limit ("up to 10 images"); record
that in the handoff (`budget.overrides`).

| Limit | `compact` | `standard` | What counts |
| --- | --- | --- | --- |
| Images per step | 1 | 4 | Images you open before you next reason about them. |
| Images in total | 6 | 24 | Every image you open or try to open, including the image check. |
| Image bytes in total | 12 MiB | 48 MiB | The sum of `image.bytes` of the images you open. |
| Page size | 20 | 50 | The `--limit` of `transcript get`, `search` and `candidates`: 50 only with `--view brief` (below). |
| Tool calls | 30 | 80 | Every command and every image you open. |
| Refinement depth | 2 | 4 | Refinements (neighbours, burst, crop, a new search term, a narrower window) in a row for one claim. |
| Wall time | 15 min | 30 min | From your first command; a transcription counts too. The host measures and enforces it, not you (below). |
| Burst frames | 4 | 12 | The `--max-frames` of one `frame burst`. |

In the handoff, the limits are written as `images_per_step`, `images_total`,
`image_bytes`, `page_limit`, `tool_calls`, `refinement_depth`, `wall_time_s` and
`burst_frames` (bytes and seconds, not MiB and minutes).

## Wall time: the host keeps it, not you

The wall-time limit is measured and enforced by the host that runs you (your client
or its operator), which stops you when it runs out. You cannot measure it:

- VSift returns no current time. The session's one clock time in its results,
  `lifecycle.expires_at`, is set when the session opens (24 hours later) and moves
  only with `session renew`, so it never tells you how long you have been working.
- You run nothing but `vsift` commands, so you never run `date` or any other program
  to read the time, before, after or chained to a command ([commands.md](commands.md)).

So report `resume.remaining.wall_time_s`, and `budget.used.wall_time_s` if you give
`budget.used` at all, as `null` (not measured), unless your client itself shows you
the elapsed time. Add
`wall_time_s` to `budget.exhausted` only when the host told you the time ran out.

## How to spend it

- Small models: one image and one transcript window at a time. Choose lead and lag
  candidates from the identities VSift returned, look at one, then decide the next.
- Prefer a crop to a second full frame when you need to read text: a crop is smaller
  and shows the text at native size.
- Prefer `frame neighbours` with a small `--count` to a burst when you need only the
  moment before and after.
- A frame whose `image.bytes` would exceed the remaining image bytes is not opened;
  crop it first or report the gap.
- Never page through the whole transcript or all candidates "to be safe"; search
  first and read windows around hits.
- Ask for `--view brief` on each `transcript get`, `search` and `candidates`: each item is
  smaller, with the same ids, times, `display_text`, coverage and gaps. Ask for pages of 20
  at most without it, on either profile; with it, `standard` may use 50 (these are what you
  ask, not what the CLI allows). The brief view is not always a small page: it keeps the
  revision block whole, and after a transcription that left many stretches of words no
  segment holds (`revision.local_asr.unplaced_ranges`) that block alone is up to about
  45,000 characters, in either view.
- A client can cut a long answer without any error: one cut a page at about 43,000 characters
  and lost items and the `next_cursor`. The CLI writes `status` before `data` and the gaps
  after it (the envelope's `coverage` after `data`, `transcript_coverage` after the items), so
  a cut answer can say `partial` and show no gap. If the output stops before the final
  `lifecycle` member (the last one), ask again with `--limit 5`; if it still stops, report the
  read as incomplete instead of trusting it. A smaller range does not help when the revision
  block alone is that large.
- Ask `candidates` for one range of at most 5 minutes per call (`--to` at most 300000000
  after `--from`), and with a transcript only around its spans, not across the video. Without
  a transcript add `--omit periodic`: on a generated 21-minute walkthrough it halved what the
  candidates cost (38,349 to 18,324 characters) and still listed every distinct screen.
- **A summary or timeline of a whole recording over 5 minutes does not fit these profiles.**
  It needs every transcript segment and more screens than the image budget holds, and can
  need more claims and citations than `handoff.schema.json` allows (50 and 100). If the user
  named a stretch, or what to hand back when the budget runs out, start, spend the budget on
  it and, if it runs out, end `partial` with the resume card. If not, say so, offer a stretch
  or a question and ask before any command; no report is needed for that, since nothing was
  started ([SKILL.md](../SKILL.md)). Judge the length by what the user says or, once a
  session is open, by `source_segments[].end_us` (with a transcript) or `index.duration_us`
  (the first `candidates` call); `ingest` reports no duration without a transcript.

## When a limit is reached

Stop at once, even mid-state. Do not start another command to "finish up". Write the
handoff with `status` `partial` (or `insufficient_evidence` if nothing is supported),
list the exhausted limits in `budget.exhausted`, add a gap with reason
`budget_exhausted` for what remains unchecked, and fill the resume card
([resume.md](resume.md)) so that another run can continue. `budget.used` is
optional in every handoff: the host counts your tool calls and images itself.
