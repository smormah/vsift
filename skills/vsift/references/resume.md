# Checkpoint and resume

An investigation can be cut short: a budget runs out, your tool call times out during
a transcription, or your context is reset. VSift keeps the work (the session, its
transcript revisions, candidates and evidence, and a transcription's finished chunks);
you keep a small resume card that says where you were and what another run must check
again.

## Operation ids

Every transcription you start carries an operation id you choose, so that repeating
the request continues or returns the same work instead of starting another one. The
grammar is `op_` followed by 16 to 64 lowercase letters or digits, nothing else: a
hyphen, an underscore or a capital letter after `op_` is refused before anything runs
(`INVALID_ARGUMENT`, `command` `parse`). Build it from your session: the letters
`retx`, then the first 24 characters after `ses_` in the session id, then a two-digit
counter (`01` for the whole-video transcription, `02`, `03` for later range checks),
all after `op_`. For the session `ses_0123456789abcdef0123456789abcdef`:

| Request | Operation id |
| --- | --- |
| The whole-video transcription | `op_retx0123456789abcdef0123456701` |
| The first range check | `op_retx0123456789abcdef0123456702` |

Record every id you use in the resume card. Reusing an id for a different request
fails with `IDEMPOTENCY_CONFLICT`; that means "choose the next counter", not "retry".

## The resume card

The handoff's `resume` object (see [../handoff.schema.json](../handoff.schema.json)),
at most 2 KiB of JSON. Give it when the work was cut short and can continue: a budget
limit ran out (`budget.exhausted`, a `budget_exhausted` gap), or a transcription was
cancelled or interrupted with its finished chunks kept (a `cancelled` gap or code
`CANCELLED`). Leave it out when the task finished, and when the report is partial only
because a capability is missing (no image access, no speech recognition, a missing
tool) or the session expired: another run cannot fix those by resuming. A card you do
give is checked: it must name your session and evidence VSift holds. It looks exactly
like this (your own values; an empty list or null where you have none):

```json
{"state": "VERIFY_SOURCE",
 "session_id": "ses_0123456789abcdef0123456789abcdef",
 "revision_id": "trv_0123456789abcdef0123456789abcdef",
 "job_id": null,
 "operation_ids": [],
 "evidence": [{"kind": "transcript_segment", "id": "tsg_0123456789abcdef0123456789abcdef", "at_us": 500000},
  {"kind": "visual_candidate", "id": "vcd_0123456789abcdef0123456789abcdef", "at_us": 12063964},
  {"kind": "frame", "id": "evd_0123456789abcdef0123456789abcdef", "at_us": 4000000}],
 "to_verify": [{"finding": "The value shown is 12.", "id": "evd_0123456789abcdef0123456789abcdef", "from_us": 4000000, "to_us": 8000000},
  {"finding": "The narrator says the value rises to twelve.", "id": "tsg_0123456789abcdef0123456789abcdef", "from_us": 500000, "to_us": 5850000}],
 "summary": "The value is 12 from 4 s to 8 s. Candidates are listed to 72.5 s; later frames are unread.",
 "remaining": {"images_total": 0, "tool_calls": 12, "wall_time_s": null},
 "next_command": "vsift frame get ses_0123456789abcdef0123456789abcdef --candidate vcd_0123456789abcdef0123456789abcdef --json"}
```

- `state`: the state you stopped in, one of the eight state names;
- `session_id` and `revision_id` (null when there is none) and `job_id`, the
  transcription job (null or left out when there is none);
- `operation_ids`: the operation ids you used;
- `evidence`: up to 8 identities you found and want to keep, each with its `kind`
  (`transcript_segment`, `visual_candidate`, `frame`, `crop` or `audio`), its `id` and
  its time `at_us`. Keep the transcript segment and the frame behind every finding
  you report, not only the candidate you would open next;
- `to_verify`: up to 4 findings you state as supported, most important first, each
  with the `finding` in at most 160 characters of your own words (no evidence text,
  no paths), the `id` of the transcript segment or frame that showed it, and the
  window it holds for, `from_us` to `to_us`: a segment's start and end, or for a frame
  its own time to where you know the state ends. Another run verifies each again
  before it reports it (below);
- `summary`: what you know so far and what is still open, in at most 500 characters
  of your own words (no evidence text, no paths);
- `remaining`: what is left of this run's budget: `images_total`, `tool_calls` and
  `wall_time_s`, each null when you did not count it. It binds only you, after a
  context reset in this same run; a new run has its own budget;
- `next_command`: the one `free` command you would run next, or null.

Update your working notes with the same facts after every state, so that a reset
loses at most one step.

## After a reset or on a new run

**Your budget.** After a context reset in the same run, you keep the budget you were
using: `remaining` is what is left of it. When the user starts a new run and gives you
a card that another run wrote, your budget is the profile the user names for this run,
counted from zero, whatever the card's `remaining` says: an earlier run's exhausted
budget is not yours.

1. Run `vsift session status <session> --json` first. Its `state` (`open`,
   `expired`, `closed`) decides what is possible, and `jobs` lists the session's
   transcription jobs with `state`, `resumable` and `resumable_reason`.
2. On a new run, do the image check of CHECK_CAPABILITIES again: image access, and
   the code you record, belong to this run. `vsift setup check` is not needed while
   the session is open.
3. **Verify every earlier finding again before you report it.** The card's findings
   are the earlier run's, not yours. For each `to_verify` entry (on a card without
   one, each finding of the `summary` you will report, with the evidence the card
   keeps for it), run one command:

```console
vsift transcript get <session> --from <from-us> --to <to-us> --limit <n> --json
vsift frame get <session> --at <us> --json
```

   For a transcript segment, read its `display_text` in the window and cite its
   segment id; it costs no image, so prefer it when it states the finding. For a
   frame, ask for it at its own time (its `from_us`, or the `at_us` the card keeps for
   it): VSift answers from the session (`reused`), and
   you open the image (one image of your budget) and cite its evidence id with
   `pixels_inspected` true. Never report an earlier finding as `unsupported`,
   `reported` or "per the card" because this run has not seen it: verify it with one
   command, or leave it out of the claims and record a `not_inspected` gap.
4. If the session is `open`, continue from the card's `state` and `next_command`
   with the saved identities. Do not repeat searches or extractions you have:
   identical frame requests are answered from the session (`reused`), but they still
   cost tool calls.
5. If a transcription job is `interrupted` (or `resumable` is true), check it and
   continue it; never start a new transcription while one is resumable:

```console
vsift job status <job> --json
vsift job resume <job> --events jsonl | tail -n 1
```

   A job that another process is running answers `BUSY`: wait `retry_after_ms` and
   ask again with `vsift job status`. If your tool call times out again, repeat this
   step; the finished chunks are kept each time.
6. The same transcription command with the same `--operation-id` also continues an
   interrupted job, and returns the committed revision (`replayed`) once it finished.

## If the session expired or was closed

Never open the video again silently. A new `ingest` copies and hashes the video again,
needs a new transcription (minutes of computation) unless the user supplied a
transcript, and gives new identities, so every earlier citation stays valid only for
the old session. Explain this cost to the user and follow the lifecycle policy they
stated ([lifecycle.md](lifecycle.md)):

- they said to reopen when needed: run `ingest` again and record the new session;
- otherwise: go to REPORT and write the handoff (status `partial` or
  `insufficient_evidence`) with a `lifecycle` gap whose reason is `session_expired`,
  and ask.

An expired or closed session cannot be renewed: `vsift session renew` only extends an
open session, and only when the user asked for it. A job whose session is closed or
expired cannot be resumed (`INVALID_ARGUMENT`); its remediation says the same, and the
way on is a new `ingest` under the user's lifecycle policy.
