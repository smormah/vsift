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
| Page size | 20 | 50 | The `--limit` of `transcript get`, `search` and `candidates`. |
| Tool calls | 30 | 80 | Every command and every image you open. |
| Refinement depth | 2 | 4 | Refinements (neighbours, burst, crop, a new search term, a narrower window) in a row for one claim. |
| Wall time | 15 min | 30 min | From your first command; a transcription counts too. |
| Burst frames | 4 | 12 | The `--max-frames` of one `frame burst`. |

In the handoff, the limits are written as `images_per_step`, `images_total`,
`image_bytes`, `page_limit`, `tool_calls`, `refinement_depth`, `wall_time_s` and
`burst_frames` (bytes and seconds, not MiB and minutes).

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

## When a limit is reached

Stop at once, even mid-state. Do not start another command to "finish up". Write the
handoff with `status` `partial` (or `insufficient_evidence` if nothing is supported),
list the exhausted limits in `budget.exhausted`, add a gap with reason
`budget_exhausted` for what remains unchecked, and fill the resume card
([resume.md](resume.md)) so that another run can continue. Report `budget.used` in
every handoff, complete or not.
