# ADR 0029: Time anchoring (clock times on timestamped results), stage 1

- Status: **Proposed, 2026-10-21.** The design was sent to the supervisor on 2026-10-21 and
  given the go the same day; the maintainer named the feature "time anchoring" and decided
  stage 1's scope and its split from stage 2 (below). It becomes Accepted with the merge of
  its pull request, after the `0.2.3` cut, for `0.3.0`. Stage 2 is the amendment of 2026-10-30
  at the end of this record, Proposed.
- Date: 2026-10-21
- Scope: `ingest`, `session status`, and every timestamped item of the v1 JSON contract
  (additive); the session lifecycle record (an optional member).
- Refines: [ADR 0008](0008-cli-and-json-contract.md) (an additive v1 change),
  [ADR 0012](0012-p04-source-media-profile.md) (the probe's fixed arguments name two more
  entries) and the CLI contract's promise that a plain `ingest` starts no FFmpeg.

## Context

Every VSift result locates evidence by its time into the video, in microseconds. A person
who wants to match a remark in a recording with a server log needs the real-world clock time
it was said at, and needs to know how far to trust it. Recordings often say when they were
made: phone and QuickTime recorders write a local-time tag with its zone
(`com.apple.quicktime.creationdate`), and MP4 and Matroska containers carry a UTC creation
time (`creation_time`, from the movie header or Matroska's `DateUTC`), which many tools set
to the time they encoded or remuxed the file and some devices to the end of the recording.
The operator often knows the start exactly. VSift read none of this before `0.3.0`.

## Decision

1. **Anchors.** A time anchor ties one position of the source timeline to one clock
   reading, from one named source with a fixed confidence (domain `TimeAnchor`,
   `TimeAnchors`). A session's clock times come from the first anchor of a ladder:
   - `operator` (confidence `exact`): `ingest --recorded-at <RFC 3339>`;
   - `quicktime_creationdate` (`high`): the recording app's local-time tag;
   - `container_creation_time` (`medium`): the container's UTC creation time.

   Every stage-1 anchor is at media time 0. The clock time of any position is the
   selected anchor's reading moved by the distance between the two positions.

2. **Zones are honest.** A reading is one of three kinds and the JSON says which:
   an instant with its local time and offset (`utc`, `local`, `utc_offset`); an instant
   whose local zone is unknown (`utc` only); or a local time whose zone is unknown
   (`local` only), which names no instant. `Z` and RFC 3339's `-00:00` give an instant with
   no local zone. Nothing guesses a zone.

3. **No modification time.** A file's modification time is not a source, not even as the
   weakest one. VSift reads the original file once, at ingest, and its modification time is
   the end of writing for a recording made on that machine, but the time of a copy, a
   download, a sync or an unpacking for most files a person is sent, and nothing in the
   file says which. A clock time with a "low" label that is hours or days wrong still sends
   a reader to the wrong log lines; an operator who knows the time states it with
   `--recorded-at`. An online provider's upload date (#409) is not a recording time either
   and is never an anchor.

4. **Reading the tags: the restricted probe, at ingest.** The existing FFprobe call
   ([ADR 0012](0012-p04-source-media-profile.md)) names
   `format_tags=creation_time,com.apple.quicktime.creationdate` in its `-show_entries`, and
   nothing else: asking for every format tag would let a hostile file's tens of megabytes of
   metadata fill the probe's 4 MiB bound (the hostile-media cases `mp4-metadata-32mib` and
   `mkv-tags-64mib`). Tags are untrusted text: at most 40 ASCII bytes, a strict grammar
   (`YYYY-MM-DDTHH:MM:SS[.fraction][Z|±HH:MM|±HHMM]`, no leap seconds, offsets within
   ±14:00 in whole minutes), a value that does not parse is left out and never fails the
   probe, and a reading earlier than 1995-01-01 or more than a day after the ingesting
   host's clock is dropped as no recording's time (the zero dates and epoch defaults of
   tools that wrote none).

   With `--transcript` the probe already runs, and its one run reads the tags too. A
   **plain** `ingest` now starts FFprobe once for the tags, but only when the media tools
   are set up and a recorded verification pass already covers them (`setup check` and every
   media command record one); it never starts a verification, creates nothing in the
   per-user state when none is recorded, and never fails because of the tags: no tools,
   unverified tools, a busy root or any probe failure only mean no container anchors. Only
   a cancelled probe cancels the ingest, as a cancelled copy does. This changes the CLI
   contract's sentence "Without `--transcript` it does not start FFmpeg"; the contract now
   says what starts and when.

   *Rejected alternative:* VSift's own reader of MP4 boxes and Matroska elements for the
   two tags. It would keep a plain ingest provider-free, but it is a second parser of
   hostile media inside VSift's trust boundary, against the rule that media engines are
   adapters (ADR 0003), and it could disagree with what FFmpeg reports.

5. **Stored once.** The anchors are computed at ingest and stored as an optional member of
   the session's lifecycle record (`time_anchors`), written only when there is one, so a
   session without anchors is recorded byte for byte as before. Every later generation
   copies it. Decoding rebuilds each reading through the domain constructors, so a modified
   record is an integrity failure. A release before `0.3.0` reads a session that has the
   member as damaged: the trade-off known limit L-044 accepts for every additive lifecycle
   member. Retained bundles do not carry the anchors in stage 1 (L-210).

6. **The JSON (additive v1; absent when the session has no anchor).**
   - `ingest` and `session status` (and `renew`, `close`, `session list` items) gain
     `time_anchor`: `consistency`, the selected `source` and `confidence`,
     `recording_start`, and `anchors` (each with `source`, `confidence`, `media_time_us`
     and `time`). `consistency` is `single` (one anchor, or no two that name an instant),
     `consistent` (every two that name an instant agree within two seconds, and at least
     two do) or `conflicting` (two disagree; clock times still come from the ladder's
     first and a reader says they are disputed). Two anchors are compared only by their
     instants. A local time whose zone is unknown is never compared, with another one or
     with a zoned reading's local time (decision 9).
   - Every timestamped item gains `clock_time` (`source`, `confidence`, and `at` for a
     point, `start` and `end` for a range): transcript segments (full, brief and streamed),
     visual candidates (span and representative frame), frames and crops (the delivered
     frame) and audio clips (the planned range and the first decoded sample). Coverage
     ranges, settings and requests do not.
   - The member is named `clock_time`, not `wall_time`: the handoff schema already uses
     `wall_time_s` for an agent's elapsed budget.
   - [`time-anchor.schema.json`](../../schemas/v1/time-anchor.schema.json) defines both.

7. **Stage 2 slots in without changing stage 1.** Stage 2 (another story, built on OCR)
   adds anchors from clocks visible on screen, with their own `source` (for example
   `screen_clock`), a `media_time_us` other than 0 and their own confidence (a rollover
   gives second precision), as new entries of the same `anchors` list; cross-checks between
   sources that name an instant report through the same `consistency` member (decision 9);
   cut or pause detection from drift
   adds optional members (for example a per-anchor `drift_us`, a list of timeline segments
   and a per-item `clock_time.segment`) and may let `clock_time` come from the nearest
   anchor of the item's segment. Nothing in stage 1 assumes a single anchor or position 0:
   the domain orders any number of anchors (at most 16), and readers are told to treat an
   unknown `source` or `consistency` value as a reason to read the anchors.

8. **No skill change in stage 1.** The skill's handoff schema cannot carry clock times
   (`additionalProperties: false`); the agent reads them from the JSON. Adding them to the
   handoff and the skill's procedure is a trial-freeze change for the `0.3.0` cut, in one
   place, if the maintainer wants it.

9. **A clock face is not an instant (maintainer, 2026-10-21; binding on stage 2).** Screen
   shares in meetings span countries: the presenter's taskbar shows the presenter's own zone,
   the recording's tags show the recorder's machine, and logs may print UTC. Three clocks in
   one recording can disagree by whole hours and all be right.
   - **Absolute times come only from zone-aware sources:**
     - `--recorded-at` with an offset or `Z`;
     - the file's UTC tags;
     - on-screen timestamps that state their zone (`Z`, `+hh:mm`).
   - **Each on-screen clock without a zone is a source of its own,** read as a local time
     whose zone is unknown, and never converted to an instant silently.
     - When an absolute source exists too, stage 2 reports an **offset observation** between
       them, marked as an inference and never applied to clock times. For example: "this
       clock runs −05:00 from the recording's UTC start".
     - Only the user confirms a zone, with an option such as `--screen-zone <IANA zone>`
       naming the clock source it applies to. A confirmed clock then becomes zone-aware,
       with its own source and confidence.
   - **The zone-independent parts of stage 2 are its core value:**
     - cut and pause detection, from how each clock moves against video time;
     - rollover precision within each clock's own timeline;
     - several clocks tracked separately, where a difference in their rates is flagged and a
       difference in their offsets is not.
   - **Stage 1's JSON already fits:** every anchor keeps its own `source`, and `consistency`
     compares absolute anchors only, never a local time whose zone is unknown.
     - Stage 1's own sources can give such a time: a `--recorded-at` without an offset, or a
       recording app's tag without one.
     - Until the maintainer's decision, two such anchors, or one beside a zoned reading,
       were compared by their local times; that was corrected with this decision and pinned
       by the test `consistency_never_compares_a_local_time_whose_zone_is_unknown`.

## Consequences

- A plain ingest of a recording with tags, on a machine whose tools are verified, now runs
  one FFprobe (15 s deadline, one admission unit, the existing bounds). The first ingest
  after installing the tools reads no tags until a media command or `setup check` has
  recorded the verification; the docs say so.
- A session's clock times are fixed when it opens; `--recorded-at` cannot be added to an
  open session in stage 1 (a later `session` command could add an operator anchor).
- Worker job requests carry no `recorded_at` in stage 1; their ingests read the tags as a
  plain ingest does.

## The maintainer's answers of 2026-10-23

The four questions this pull request left open were answered at the merge rebase:

- **A plain `ingest` runs FFprobe once: accepted.** The change to the CLI contract's
  promise that a plain `ingest` starts no FFmpeg stands as decision 3 describes it (only
  with verified tools, never failing because of the tags).
- **The handoff cites clock times through one skill change at the `0.3.0` freeze, after
  stage 2.** Decision 8 stays: no skill change in stage 1. The agent reads clock times
  from the JSON until then.
- **Anchors in bundles and worker requests come with stage 2.** Retained bundles keep
  carrying none and worker job requests keep carrying no `recorded_at` until then;
  [L-210](../planning/known-limits.md#l-210) stays open for that.
- **The confidence labels `exact`, `high` and `medium` are accepted** as they are.

## Verification

- Domain: the grammar, offsets, leap years, round trips through Unix time, the ladder, the
  plausibility window, consistency (absolute anchors only, decision 9), shifting across
  midnight (`time_anchor` tests).
- Probe parsing of the two tags, recorder tag first, and an unreadable tag left out
  (`ffmpeg_media` tests); the fuzz target `ffprobe_metadata` with a new seed
  (`recorder-tags.json`) and an invariant on the tags.
- The stored form round-trips and refuses every modification
  (`filesystem_session_store::time_anchors`); a session without anchors writes no member,
  and a renewal keeps them (`time_anchor_store`).
- The use case stores the ladder, a failed probe opens without container anchors, a
  cancelled probe cancels (`vsift-application` `session` tests); a plain ingest without a
  recorded pass never runs the preflight (`engine_media_tool_preflight`).
- The contract: `clock` tests, the examples `ingest.time-anchor.json` and
  `transcript-get.clock-time.json`, candidates, frames and audio with clock times; through
  the binary, `time_anchor_cli_contract` (`--recorded-at` in each zone form, refusals,
  status, transcript and search full, brief and streamed, a session without anchors).
- Real tags: the opt-in `time_anchor_real_tags`, run by the hosted `P07 local ASR`
  workflow, makes an MP4, a MOV with the recorder tag, a Matroska file and a 1990 MP4 with
  the pinned FFmpeg and reads them through the binary.

## Amendment of 2026-10-30: stage 2, clocks on screen (Proposed)

Status: **Proposed, 2026-10-30.** Sent for review before building. Stage 1's decisions above stand,
except where a decision below names what it changes. Decision 9 binds everything here. The
thresholds marked *tentative* are starting values, set by the tests and the hosted evidence.

### Why the shape

A clock on a screen says two kinds of thing. *What time it is* depends on a zone nobody stated.
*How it moves against the video* depends on no zone at all, and that is where stage 2 gets its value.
So the model is built on one zone-free quantity, and the zone is added last and only by the user.

### 10. What counts as a clock: it must move

A time on a screen is usually not the present: a chat message's "10:32", a calendar grid, a file
list, a line of a log. Text alone cannot tell these from a clock, so **recognition has two steps**,
and only the second can make a clock.

*Step 1, the face (pure, in `vsift-domain`).* One OCR line, untrusted, at most 64 bytes, is read by a
closed grammar. Nothing is repaired: `O` is not read as `0`.

| Face | Examples | Unit | Zone | Date |
| --- | --- | --- | --- | --- |
| 24-hour | `14:03`, `14:03:12` | 60 s, 1 s | none | none |
| 12-hour | `2:03 PM`, `2:03:12 pm`, `2:03 p.m.` | 60 s, 1 s | none | none |
| ISO date and time | `2026-10-21 14:03:12`, `2026-10-21T14:03` | 60 s, 1 s | none | yes |
| With a month name | `21 Oct 2026 14:03`, `Oct 21, 2026 2:03 PM` | 60 s, 1 s | none | yes |
| States its zone | any dated face followed by `Z`, `UTC`, `GMT`, `+00:00`, `-0500` | 60 s, 1 s | yes | yes |

Refused: a line with two or more time tokens (`9:00 - 10:00`, `1:23 / 4:56`), a token touching a
letter, digit, `/` or a further `:` or `.` group (`16:9`, `10:30:00.0`, `1.2.3:45`), a one-digit hour
without AM or PM (`1:23` is a duration far more often than a time), numeric dates with slashes or
dots (`10/21/2026` is two different days), zone abbreviations (`IST` and `CST` each name several
zones, so a dated face followed by one counts as having no stated zone), and lines read with the
engine's score under 0.80 (*tentative*). A fraction of a second is read and ignored: the unit stays 1 s.

*Step 2, the track.* Faces of one format class in one place (`TextRegion::same_place`) over
consecutive read frames become a **clock** only with at least three reads and at least one advance,
all at the rate of the video (decision 11). A value that never changes is `static` and a value that
falls is a countdown: neither is a clock, and each is only counted in the report. A log that scrolls
fails this test by design: a log line's timestamp is when something happened, not what time it is
now, so a scrolling line is not an anchor. A status line that stays in place and moves is.

*Counters.* An elapsed timer moves at the rate of the video too and looks like a time of day. A
face with seconds, no date, no AM or PM and hour `00` is a `counter`: it is listed, and it gives no
anchor, no offset observation and no change to a clock time.

### 11. The model: an offset that is an interval

The **offset** of a clock at a moment is its reading minus the video's own time at that frame. A
clock that runs with the video keeps one offset. A reading on a frame at media time `t`, with a face
of unit `u` showing `r`, proves only that the offset lies in `[r − t, r + u − t)`. A minute clock read
every 10 s therefore already narrows the offset as reads accumulate, with no extra work.

- A **run** is a maximal sequence of reads whose intervals share a point, widened by 250 ms
  (*tentative*) for how late a screen paints its clock. Its offset interval is their intersection,
  so its width is the run's precision, and every statement about the clock carries that width.
- Time-of-day faces wrap at 24 hours: each read is unwrapped to the value nearest the run's prediction.
- A **rate** is judged only where it is proved. From a run's first and last reads, the slope lies in
  `[(Δr − u) / Δt, (Δr + u) / Δt]`. A rate is flagged only when 1.0 lies outside that range, so a
  short run cannot accuse a clock. Two clocks over the same stretch with different flagged rates are a
  conflict (decision 14). A difference in their offsets is not (decision 9).

This is the domain's clock model, pure and deterministic, with its rules under a version
(`CLOCK_RULES_VERSION`). Its properties are tested directly: reordering reads changes nothing, a read
consistent with a run never widens its interval, and a clock generated with a known offset is always
inside the interval reported.

### 12. Rollover precision, read through the existing index

Where two reads of a run differ by one unit, the change happened between them, and reading the frames
between narrows the offset to the frame where it changed. **The search keeps no state of its own.**

- The next time to read is a pure function of the clocks in the newest index: the midpoint of the
  narrowest useful bracket. The reads go through the existing `read_screen_text_within` as ordinary
  periodic `ReadTarget`s and are committed as an ordinary revision of the screen-text index.
- So it is bounded and resumable exactly like `ocr`: a call that stops loses nothing, a read is never
  repeated by the same engine build, the 40-frame default, 200-frame limit and 300 s deadline hold,
  the index's bounds hold, and a new engine build starts again, because its index is a new one.
- A search stops at a bracket of 500 ms (*tentative*), which from a 10 s grid takes five reads. A
  midpoint that shows neither side's value is treated as unreadable, and the frame after it is tried
  once. Each run searches its first and last rollover only, since one fixes the offset and two prove
  the rate. A discontinuity's position is narrowed the same way. A session spends at most 256 such
  frames (*tentative*).
- A clock with seconds needs no search: one read is already within a second.
- Reads added this way extend `ocr search` hits and are not part of `ocr`'s coverage, which counts its
  own plan.

### 13. Cuts, pauses and freezes

Two consecutive runs of one place, each of at least three reads, differ in offset by a **jump**,
itself an interval. What it means depends on its sign and shape:

| Kind | Shape | Meaning | Changes `clock_time`? |
| --- | --- | --- | --- |
| `gap` | The offset rises | Time is missing from the video: a cut, or a paused recording | Yes (below) |
| `backward` | The offset falls | A stretch repeated or moved, or a clock set back | No: reported as a conflict |
| `freeze` | A value holds past its unit, then the clock resumes | The screen stopped while the video ran, so video time is still right | No |
| (another clock) | The jump is within its width of a multiple of 15 minutes, and at least 30 minutes | Most likely another machine's clock in the same place | No: two clocks |

The last row is what decision 9 demands. Two presenters' taskbars in one place differ by whole hours,
and that must never read as a cut. The cost is that a real pause of exactly an hour is reported as
two clocks. A jump under 2 s (*tentative*) is not reported at all. The same rule means a change of
daylight saving during a recording is two clocks.

**A `gap` corrects clock times.** It is independent of any zone, and without it every clock time after
a cut is wrong by the length of the cut (L-210). Decision 7 allows this. The correction is applied
when the two runs qualify and no other clock spans the same moment without jumping. If one does, the
jump is a conflict and nothing is corrected. Corrections divide the timeline into **segments**, the
selected anchor's reading is moved by the corrections between it and the item, and each item says
which segment it is in. Nothing is hidden: the segment names its jump, its uncertainty and the clock
that proved it. A session without a qualifying gap has one segment and answers exactly as it did.

### 14. The conflict report

`clocks` lists what disagrees and never resolves it:

- `instants_disagree`: two anchors that name an instant differ by more than two seconds plus their
  precision. This is the existing `consistency: conflicting`, now with the pair named.
- `rates_differ`: two clocks over one stretch with different flagged rates.
- `jump_not_shared`: one clock jumps where another spans the moment without jumping.
- `backward`: decision 13's case.

### 15. Sources, confidence and the ladder

Three sources are added, after which `AnchorSource`'s order is the ladder:

| Order | Source | Confidence | Reading |
| --- | --- | --- | --- |
| 1 | `operator` | `exact` | as stage 1 |
| 2 | `screen_clock_confirmed` (new) | `medium` | a clock whose zone the user confirmed: an instant |
| 3 | `quicktime_creationdate` | `high` | as stage 1 |
| 4 | `container_creation_time` | `medium` | as stage 1 |
| 5 | `screen_timestamp` (new) | `medium` | a moving clock that states its zone: an instant |
| 6 | `screen_clock` (new) | `low` (new) | a dated clock with no zone: `local` only |

A confirmation ranks second because it is the user's explicit statement, as `--recorded-at` is. The
two sources that nobody confirmed rank below every stage-1 source, so running `clocks` never changes
which anchor a session's clock times come from unless the session had none. When `screen_clock` is
selected, clock times are local with `utc` null, as a zoneless `--recorded-at` gives. Selection stays
per session, never per item, so one session's items never mix two clock faces.

**A clock without a date is not an anchor.** Stage 1's `local` is a date and time, and a time of day
alone cannot be written as one, so a taskbar's `2:03 PM` adds nothing to `anchors` until it is
confirmed. It still gives rollovers, cuts, rates and offset observations. A date is never borrowed
silently.

`TimeAnchors` now allows one anchor per clock per segment for the screen sources, keeping the bound of
16: the most precise are kept and the report lists all of them. Stage 1's one-per-source rule stays
for its three sources. `consistency` compares anchors at their own positions, through the segments,
with each anchor's precision added to the two seconds. A local time is still never compared.

### 16. Offset observations and the user's confirmation

Where a clock and an anchor that names an instant both exist, `clocks` reports an **offset
observation**: the difference between them, the UTC offsets it fits (the nearest multiple of 15
minutes within ±14:00, within the clock's precision plus 30 s *tentative*; a time of day fits modulo
24 hours, so it can fit two, such as `+10:00` and `-14:00`), and the anchor it was measured against.
It is always marked `inference` and is **never applied to a clock time**. A difference that fits no
offset is reported too, because it usually means the anchor is not the recording's start.

Only the user confirms, with `vsift clocks <session> --screen-zone <clock>=<±HH:MM>`, repeatable. The
clock then gives a `screen_clock_confirmed` anchor. A clock with no date takes its date from the
absolute anchor nearest in time, the date on which the two readings are closest, so any remaining
difference shows as `instants_disagree`. With no absolute anchor it cannot be confirmed:
`INVALID_ARGUMENT`, whose remediation names `--recorded-at`. Confirming a `counter` or an unknown
clock is `INVALID_ARGUMENT`.

Zones are given as an offset, not an IANA name, for this release. An offset is what the observation
already showed, it is exact for the moment observed, and it needs no zone database in the binary. A
name would add a dependency that must follow rule changes.

### 17. The command: `vsift clocks`

```console
vsift clocks <session> [--from <us> --to <us>] [--max-frames 0..200] [--runner <path>]
                       [--threads 1..8] [--screen-zone <clock>=<±HH:MM>]... --json
```

- It is opt-in and bounded, and it never runs `ocr`'s plan: with no index it answers `state: not_run`,
  `partial`, naming `ocr`. Absence is never a claim: "no clock found" speaks only for the frames read.
- It reads at most `--max-frames` refinement frames (default 40), then recomputes and commits.
  `--max-frames 0` reads nothing and needs no runner, so a confirmation never needs one.
- Its order follows `ocr`'s (ADR 0030 decision 8). The commit merges onto a concurrent writer's work at
  most three times.
- `state` is `complete` when no refinement is pending and every planned `ocr` read of the range is
  done; otherwise it is `partial`, with reasons and the fixed warning pattern of `ocr`.
- The result is `clocks-data.schema.json`: each clock with its id, kind, format, reads, precision,
  offset interval, rate, zone state and observations; the discontinuities and conflicts; the counts of
  lines refused by reason; and, for every claim, the exact `frame` and `region` that prove it,
  including the frames either side of a rollover, so a person can check it with `frame get`.
- On-screen text is untrusted: a clock's reading is parsed into numbers and never echoed as free text.

### 18. Storage

The clocks are **derived**: the newest screen-text index and the rules give them again, so the full
report is not stored. Three things are, in the lifecycle record, each written only when present, so a
session that never runs `clocks` is recorded byte for byte as before:

- `time_anchors` gains the screen anchors, each with its clock id and precision.
- `screen_clocks`, at most 8 (*tentative*): for each clock its id, format class, offset interval and
  any confirmed offset. This registry keeps ids stable when later `ocr` calls read earlier frames: a
  recomputed clock takes the id of the entry whose interval it meets, in `c1`, `c2` order.
- `clock_segments`, at most 32 (*tentative*): the corrections of decision 13.

They are stored there, not in an artifact, because every command that presents a `clock_time` reads
the lifecycle already and none should have to open a 48 MiB index. They are written by a new lifecycle
update under the session's writer lock, and decoding rebuilds them through the domain constructors,
so a modified record is an integrity failure. This replaces the consequence "a session's clock times
are fixed when it opens" for screen sources only. A session's operator and tag anchors still are.

### 19. The JSON, additive only

- `time_anchor.anchors[]` gains optional `clock_id` and `precision_us`; `source` gains the three
  values above, and `confidence` gains `low`. The schema's description has told readers since stage 1
  to treat an unknown value as a reason to read the anchors.
- `time_anchor` gains optional `segments`: `segment`, `from_us`, `to_us`, `adjusted_by_us`,
  `uncertainty_us`, `clock_id`. It is absent with one segment.
- `clock_time` gains optional `segment` and, when the source is a clock, `clock_id`. Both are absent
  when they say nothing, so a session without them answers byte for byte as before.
- `time_anchor` stays absent while a session has no anchor, as in stage 1. A session whose only clock
  has no date therefore has none; `clocks` reports it.
- `session status` does not list the clocks. `clocks` with `--max-frames 0` does.

### 20. Bundles and worker requests (the maintainer's answer of 2026-10-23)

- The bundle manifest gains an optional `time_anchors` member, with the segments and the registry,
  written only when present and checked by `bundle validate`, so a retained session keeps its anchors.
- Worker job requests gain an optional `recorded_at`, parsed as `ingest --recorded-at` is.

### 21. What this does not change

- **L-239.** No command takes a clock time, and stage 2 adds none. Its subtraction gets harder across a
  gap: `recording_start` is no longer enough after a cut, so a window computed from it lands off by the
  jump. Step 5 of the logs-by-clock workflow, which cites only items whose own `clock_time` falls in
  the window, shows that as a mismatch rather than a false match. The workflow's wording about
  `time_anchor.segments` belongs to the one skill change at the `0.3.0` freeze.
- **L-210** is rewritten: its cut bullet becomes what decision 13 covers and what it does not (a cut
  with no clock in view is still silent), and its "no command adds one later", worker and bundle
  clauses go.
- No clock is read from speech, and no modification time or upload date becomes a source.
- Clocks are not read in the background: `prepare` and `vsift <video>` do not run `clocks`.

### 22. Slices, in one pull request

1. Zoneless clocks as their own sources: grammar, tracks, intervals, rollover search, `clocks`, the
   registry, the anchors, the schema and the contract tests.
2. Cuts and pauses: discontinuities, segments, the corrected `clock_time`, the conflict report.
3. Offset observations and `--screen-zone`.
4. Bundles and worker requests.

A slice that does not qualify at the cut leaves the earlier ones standing.

### Limits (L-213)

A clock in a script the recogniser does not read, or a format outside the grammar, is not seen. A
12-hour clock without AM or PM is refused. A clock read only every 10 s can hide a short cut; a cut
with no clock in view is silent. A real pause of a whole number of quarter hours reads as two clocks.
A screen showing a video that shows a clock is reported as a clock. A screen clock that was simply
wrong gives a wrong reading: stage 2 reports how clocks move, not that any of them is right.

### Verification planned

- Domain: grammar tables, and property tests for the model (decision 11), with generated clocks of
  known offset, rate, cuts and midnight crossings. Pure planner tests for the rollover search,
  including a misread midpoint and a resumed search.
- Application and engine: the stand-in reader, covering a search stopped and resumed, a concurrent
  commit, and `--max-frames 0` starting no runner.
- Store: every new member round-trips, refuses each modification and is absent when unused.
- Contract: schema conformance, examples, `time_anchor_cli_contract` extended, and a session that never
  ran `clocks` compared byte for byte.
- Hosted only, on `P14 real media`, at most two dispatches: synthetic recordings of known clocks
  generated on the runner (one clock, two clocks in different zones, a cut, a pause, a freeze, a
  midnight crossing, a chat full of static times, a scrolling log), and RM01, RM04 and RM10 as the
  negative control: no recording may gain a clock that a person cannot see.
