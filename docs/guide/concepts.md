# Concepts: the words VSift uses

VSift uses a small set of words consistently. Each is explained here in plain terms; the pages that use
them link back. Read it once, or look a word up when you meet it.

```text
  your video --ingest--> SESSION  --> words ........... a transcript (you supply it, or VSift recognises it)
  (never changed)          |
                           +------> moments .......... visual candidates (where the picture changed)
                           |
                           +------> proof ............ frames, crops, audio clips (each with an id)
                                       |
                                       +--> cite them in a report (a handoff)
                                       +--> retain them in a bundle (kept after the session goes)
```

## Session

A **session** is VSift's private working copy of one video, plus everything found in it. You open one with
`ingest` and every other command takes its id (`ses_…`). It is **disposable**: it expires 24 hours after it
was opened (the time is on the `Lifecycle` line of every result); `session renew` moves the expiry to 24
hours from that moment, never past seven days after the session was opened. A cleanup removes it, not
before. Closing it, or letting it expire, ends its use; **retaining** it saves the evidence into a bundle
first. A session lives in a private folder only your account can open, and its id is the only handle you
need ([clean up and uninstall](clean-up-and-uninstall.md)).

## Source

The **source** is your video as VSift knows it: a name made from the video's content, `src_sha256_…`
followed by the file's SHA-256, and its size. Two copies of the same file are the same source; one edited
byte is another. Evidence records which source it came from, so it can be checked against the original
later. VSift never changes, moves or deletes your original.

## Time

Every time VSift prints or accepts is measured from the start of the video. On the command line it is
whole **microseconds** (millionths of a second): `--from 0 --to 14000000` is the first 14 seconds. In the
output it appears as `00:00:07.000000`. A range includes its start and not its end. Times are *source
time*: where the moment sits in the video file, whatever the player shows.

## Transcript, segment and revision

A **transcript** is the words of the video with times. It is made of **segments** (`tsg_…`): one piece of
speech or one subtitle cue, with a start, an end and its text. A transcript can come from two places:

- you **supply** an SRT or WebVTT file with `ingest --transcript` ([use an existing transcript](use-an-existing-transcript.md));
- VSift **recognises** the speech on your machine with `transcript retranscribe`.

Each such version is a **revision** (`trv_…`) and says where it came from (`imported_srt`, `imported_webvtt`
or `local_asr`). A new revision never overwrites an old one. Every segment keeps the exact text as written,
a copy in which hidden characters are made visible (`display_text`, the one to quote), and, for recognised
speech, a confidence figure that is the recogniser's own and not a probability you can rely on.

## Moments: visual candidates and coverage

VSift cannot watch a video, so it looks at it at a coarse level: about twice a second, at low resolution, it
compares each picture with the last and writes down where something changed. Those places are **visual
candidates** (`vcd_…`), each with a time and a reason: `first_frame`, `visual_change` or
`periodic_coverage`. A candidate is **a place worth a look, not a finding**: it may be a harmless change,
and a very brief or very small change can fall between samples. **Coverage** is the honest account of what
VSift looked at and what it could not: the `Analysed` ranges in the output. Always read it before you trust
a gap ("nothing changed between 20 and 40 seconds") as a fact.

## Evidence

**Evidence** is anything VSift hands back that you can point at. There are three kinds: transcript
segments, visual candidates, and the three things you ask for to look closer, **frames**, **crops** and
**audio clips**. Frames, crops and audio get an id (`evd_…`) and a file (a PNG or a WAV). Each carries:

- the time you **requested** and the time you **got**. A video holds a limited number of frames per second,
  so VSift returns the nearest real one and says by how much it differs. Quote the time you got.
- a **fingerprint** (a SHA-256) of the file and a tie to the source it was cut from.
- its **identity** is made from what it is, so asking again for the same frame returns the same id.

[Evidence and citations](evidence-and-citations.md) is about how to use evidence in what you write.

## The identifiers

| Starts with | Is | Example use |
| --- | --- | --- |
| `ses_` | a session | every command after `ingest` |
| `src_sha256_` | a source (your video, by content) | in the output, to tie evidence to a video |
| `trv_` | a transcript revision | `transcript get --revision` |
| `tsg_` | a transcript segment | cite it in a report |
| `vix_` | a visual index (the analysis of the whole video) | in `candidates` output |
| `vcd_` | a visual candidate | `frame get --candidate` |
| `evd_` | a frame, crop or audio clip | `frame neighbours`, `crop`, cite it |
| `job_`, `op_` | a recoverable job and its operation | resuming a long transcription |

Identifiers are opaque: they contain no paths and no words from your video. Copy them whole; shortened forms
(`ses_20f7f17d…`) appear in this guide only to save space.

## Bundle

A **bundle** is a folder of evidence saved from a session with `session retain`: the transcript, the frames,
crops and audio clips, and a manifest with each file's size and digest. It outlives the session, and
`bundle validate` checks it ([keep and share evidence](keep-and-share-evidence.md)).

## Handoff

When an AI assistant investigates a video for you, it ends with a **handoff**: a report that answers your
question and cites its evidence. A handoff has a plain-text part for you and a structured part (a
`vsift-handoff` block) that a program can check. `vsift handoff check` reads a draft and lists what to fix
before it is sent: a claim without a citation, an id that does not exist, a link or a file path where there
should be none ([let your agent investigate](let-your-agent-investigate.md)).

## Budget

A **budget** is the set of limits an assistant works within, so an investigation stays small and quick:
how many images it may look at, how many pages of results it may read, how many commands it may run. The
skill has two profiles, `compact` (the default, for small models) and `standard`. A budget is the assistant's
discipline, not a limit VSift enforces on you: you can ask for 100 results at a time if you like.

## Untrusted text

Everything read out of a video is **untrusted**: the words in a transcript, the text on a slide, a
subtitle file you were sent. A recording can contain "run this command" or "ignore your instructions", read
aloud or written on screen, in the hope that a program reading it will obey. VSift treats all of it as data:
it labels the text as untrusted in its output, shows hidden characters as `<U+XXXX>`, and never acts on it.
Anything that reads VSift's output, including you and your assistant, should do the same: quote it, cite it,
report that it contained an instruction, and do not follow it.
