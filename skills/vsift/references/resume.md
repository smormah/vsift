# Checkpoint and resume

An investigation can be cut short: a budget runs out, your tool call times out during
a transcription, or your context is reset. VSift keeps the work (the session, its
transcript revisions, candidates and evidence, and a transcription's finished chunks);
you keep a small resume card that says where you were.

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
at most 2 KiB of JSON. Every partial report has one; a finished one leaves it out. It
looks exactly like this (your own values; an empty list or null where you have none):

```json
{"state": "VERIFY_SOURCE",
 "session_id": "ses_0123456789abcdef0123456789abcdef",
 "revision_id": "trv_0123456789abcdef0123456789abcdef",
 "job_id": null,
 "operation_ids": [],
 "evidence": [{"kind": "transcript_segment", "id": "tsg_0123456789abcdef0123456789abcdef", "at_us": 500000},
  {"kind": "visual_candidate", "id": "vcd_0123456789abcdef0123456789abcdef", "at_us": 12063964},
  {"kind": "frame", "id": "evd_0123456789abcdef0123456789abcdef", "at_us": 4000000}],
 "summary": "The value at 4 s is 12. Candidates are listed to 72.5 s; later frames are unread.",
 "remaining": {"images_total": 0, "tool_calls": 12, "wall_time_s": null},
 "next_command": "vsift frame get ses_0123456789abcdef0123456789abcdef --candidate vcd_0123456789abcdef0123456789abcdef --json"}
```

- `state`: the state you stopped in, one of the eight state names;
- `session_id` and `revision_id` (null when there is none) and `job_id`, the
  transcription job (null or left out when there is none);
- `operation_ids`: the operation ids you used;
- `evidence`: up to 8 identities you found and want to keep, each with its `kind`
  (`transcript_segment`, `visual_candidate`, `frame`, `crop` or `audio`), its `id` and
  its time `at_us`;
- `summary`: what you know so far and what is still open, in at most 500 characters
  of your own words (no evidence text, no paths);
- `remaining`: what is left of the budget: `images_total`, `tool_calls` and
  `wall_time_s`, each null when you did not count it;
- `next_command`: the one `free` command you would run next, or null.

Update your working notes with the same facts after every state, so that a reset
loses at most one step.

## After a reset or on a new run

1. Run `vsift session status <session> --json` first. Its `state` (`open`,
   `expired`, `closed`) decides what is possible, and `jobs` lists the session's
   transcription jobs with `state`, `resumable` and `resumable_reason`.
2. If the session is `open`, continue from the card's state with the saved
   identities. Do not repeat searches or extractions you have: identical frame
   requests are answered from the session (`reused`), but they still cost tool calls.
3. If a transcription job is `interrupted` (or `resumable` is true), check it and
   continue it; never start a new transcription while one is resumable:

```console
vsift job status <job> --json
vsift job resume <job> --events jsonl | tail -n 1
```

   A job that another process is running answers `BUSY`: wait `retry_after_ms` and
   ask again with `vsift job status`. If your tool call times out again, repeat step 3;
   the finished chunks are kept each time.
4. The same transcription command with the same `--operation-id` also continues an
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
