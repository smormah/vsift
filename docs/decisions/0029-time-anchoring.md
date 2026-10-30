# ADR 0029: Time anchoring (clock times on timestamped results), stage 1

- Status: **Accepted.** Stage 1 on 2026-10-27, with the merge of its pull request (#501), and
  stage 2, the amendment of 2026-10-30 at the end of this record, with the merge of its own
  (#553), both for `0.3.0`. The design was sent to the supervisor on 2026-10-21 and given the go
  the same day; the maintainer named the feature "time anchoring" and decided stage 1's scope
  and its split from stage 2 (below).
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
     - *Note of 2026-10-30:* `0.3.0` takes an offset (`±HH:MM`), not an IANA name; see decision
       16 of the amendment below.
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
  *Amended 2026-10-30:* an open session's clock times now change only through a user's
  explicit `--accept-gap` or `--screen-zone` (decisions 13 and 16); `--recorded-at` still
  cannot be added later.
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

## Amendment of 2026-10-30: stage 2, clocks on screen

Status: **Accepted, 2026-10-30, by the planner, with its rulings written in below.** Stage 1's
decisions above stand, except where a decision below names what it changes.
Decision 9 binds everything here. The thresholds marked *tentative* are starting values, set by the
tests and the hosted evidence.

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
zones, so a dated face followed by one counts as having no stated zone), and lines scoring under
0.80. That bound only trims junk and is **no defence against a misread**. ADR 0030 calls the score
uncalibrated, and on compressed clocks wrong readings scored as high as right ones, 0.8993 to 0.9997
against a lowest right reading of 0.9057 (decision 22), so the model has to deal with them (decision
13). A fraction of a second is read and ignored: the unit stays 1 s.

*Step 2, the track.* Faces of one format class in one place (`TextRegion::same_place`) over
consecutive read frames become a **clock** only with at least three reads and at least one advance,
all at the rate of the video (decision 11). A value that never changes is `static` and a value that
falls is a countdown: neither is a clock, and each is only counted in the report. A log that scrolls
fails this test by design: a log line's timestamp is when something happened, not what time it is
now, so a scrolling line is not an anchor. A status line that stays in place and moves is.

*Counters.* An elapsed timer moves at the rate of the video too and looks like a time of day. A
moving face with seconds, no date and no AM or PM is a `counter` when its hour is `00`, or when its
offset (decision 11) lies in `(−duration, 0]`, because it then read zero during the recording, as a
meeting timer past its first hour (`01:03:22`) does. A counter is listed, and it gives no anchor, no
offset observation and no gap. The cost is that a real clock passing midnight during the recording
reads as a counter (L-213).

### 11. The model: an offset that is an interval

The **offset** of a clock at a moment is its reading minus the video's own time at that frame. A
clock that runs with the video keeps one offset. A reading on a frame at media time `t`, with a face
of unit `u` showing `r`, proves only that the offset lies in `[r − t, r + u − t)`. A minute clock read
every 10 s therefore already narrows the offset as reads accumulate, with no extra work.

- A **run** is a maximal sequence of reads whose intervals share a point, widened by 250 ms
  (*tentative*) for how late a screen paints its clock. Its offset interval is their intersection,
  so its width is the run's precision, and every statement about the clock carries that width.
- Time-of-day faces wrap at 24 hours: each read is unwrapped to the value nearest the run's prediction.
- A **rate** is judged only where it is proved. A single run always fits the video's rate, because its
  reads share an offset by definition, so a different rate shows as runs that creep apart by steps too
  small to be jumps. The rate is therefore judged across a stretch of consecutive runs with no jump or
  freeze between them. A step under the 2 s that decision 13 does not report stays inside the stretch,
  so its drift still counts: the steps add up over the stretch, and a chain of small steps cannot pass
  as neither a jump nor a rate. A rate is flagged only where 1.0 is proved to be impossible, when the
  drift over the stretch exceeds the face's unit plus the paint allowance, so a short stretch cannot
  accuse a clock. A clock of one run reports no rate. Two clocks over the same stretch with different
  flagged rates are a conflict (decision 14). A difference in their offsets is not (decision 9).

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
  midpoint that shows neither side's value becomes an excursion's read, so that bracket's search ends
  at the width it has (decision 13). The search moves to the next rollover, so one misread costs one
  bracket, and `stopped_brackets` reports each with its width, so a clock whose every rollover was
  stopped shows it. Each run searches its first and last usable rollover only, since one fixes the
  offset and two prove the rate. A jump's position is narrowed the same way. A session
  spends at most 256 such frames (*tentative*).
- One call reads in rounds, each seeing what the last read, and commits them all as **one** index
  revision, so a search does not spend the session's 64 revisions.
- A clock with seconds needs no search: one read is already within a second.
- Reads added this way extend `ocr search` hits and are not part of `ocr`'s coverage, which counts its
  own plan.

### 13. Cuts, pauses and freezes

Two consecutive runs of one place, each of at least three reads, differ in offset by a **jump**,
itself an interval. What it means depends on its sign and shape:

| Kind | Shape | Meaning | Changes `clock_time`? |
| --- | --- | --- | --- |
| `gap` | The offset rises | Time may be missing from the video: a cut, or a paused recording | Only when a person accepts it (below) |
| `backward` | The offset falls | A stretch repeated or moved, or a clock set back | No: reported as a conflict |
| `freeze` | A value holds past its unit, then the clock resumes | The screen stopped while the video ran, so video time is still right | No |
| (another clock) | The jump is within its width of a multiple of 15 minutes, and at least 30 minutes | Most likely another machine's clock in the same place | No: two clocks |

**An excursion comes first.** OCR can read a plausible wrong digit (`14:17` as `14:13`) with a high
score, and a stretch with matching runs either side has more than one cause: presenter A, then B with a
clock 30 s apart, then A again gives the same shape. So the rule does not claim the cause. It was first
reasoned from a misread filling a whole held minute; that was not observed (decision 22), and the rule
stands because a presenter's turn, and a misread of any length, still take this shape.

- A stretch whose offset departs and returns to the same interval is checked before any row below
  applies. **If it holds a run that qualifies as a clock (three reads and an advance), that is another
  clock's turn:** it is tracked and named as a clock of its own, and the clock it interrupted carries on
  with no gap and no backward jump for that turn, and never searches a bracket across it. So presenters
  who hand back keep their clocks, as decision 9 requires, and two machines 30 s apart taking turns are
  two clocks.
- Otherwise it is an **excursion**, reported in `clocks` with its span, its frames and its read count,
  offering both explanations, a misread or a turn too short to tell, without choosing.
- Its reads leave the track and are never dropped silently. Nothing about it is a gap, and it makes no
  segment. The rollover search never brackets across an excursion's reads.
- An offset that departs and does not return is not an excursion. It stays a detected gap, which
  changes no value (below), so a misread held long enough at the end of a recording reads as a detected
  gap (L-213).
- A misread of an hour digit lands on whole hours, so the last row below reports it as two clocks and
  changes nothing: `10:45` read as `20:45` is a whole 10 hours, which gives a second clock with no
  effect on anchors unless the user confirms a zone on it (L-213).

The last row is what decision 9 demands. Two presenters' taskbars in one place differ by whole hours,
and that must never read as a cut. The cost is that a real pause of exactly an hour is reported as
two clocks. A jump under 2 s (*tentative*) is not reported at all. The same rule means a change of
daylight saving during a recording is two clocks.

**A detected `gap` is reported; only a person applies it.** Decision 7 allows a correction, but this
design cannot tell a cut from a change of clock. Two people in one zone whose machines differ by 30 s,
sharing in turn in the same place, make a positive jump of 30 s. The quarter-hour rule does not catch
that, and with no other clock in view nothing contradicts it. A virtual machine's or remote desktop's
clock in the same place does the same, as does a machine correcting its own time. Applying such a
jump would make every later time silently wrong by a value VSift itself introduced. So a gap follows
decision 16's rule: an inference is reported, and only the user applies it.

- Gaps divide the timeline into **segments**. A gap has a stable id (`g1`, `g2`) and a state,
  `detected` or `accepted`.
- While `detected`, the gap changes no value. Each item after it carries `segment`, and the `clock_time`
  value stays what the anchor gives, so nobody reads a suspect time without being able to see why: the
  segment names the jump, its uncertainty and the clock that proved it.
- `vsift clocks <session> --accept-gap <id>`, repeatable, makes it `accepted`: `adjusted_by_us` takes
  effect, and the selected anchor's reading is moved by the accepted corrections between it and the
  item. Only what the person stated is applied, and an unknown id is `INVALID_ARGUMENT`.
- A gap that another clock contradicts (it spans the moment without jumping) is also a conflict, and
  it can still be accepted, because the person may know more.
- A session with no gap has one segment and answers exactly as it did.

So the logs-by-clock subtraction stays right unless a person accepted a gap.

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
| 2 | `quicktime_creationdate` | `high` | as stage 1 |
| 3 | `screen_clock_confirmed` (new) | `medium` | a clock whose zone the user confirmed: an instant |
| 4 | `container_creation_time` | `medium` | as stage 1 |
| 5 | `screen_timestamp` (new) | `medium` | a moving clock that states its zone: an instant |
| 6 | `screen_clock` (new) | `medium` | a dated clock with no zone: `local` only |

**No new confidence label.** `confidence` is a closed list of the three labels the maintainer
accepted, and decision 7 tells readers to tolerate an unknown `source` or `consistency`, not an unknown
`confidence`. All three new sources are `medium`; for `screen_clock`, `local` with `utc` null and the
source's name already say what is unknown. **The ladder is monotone:** confidence never rises as it
goes down.

The user confirmed a zone, not that the clock was right, so a confirmation does not displace the
recorder's tag, and a disagreement shows as `instants_disagree`. It does beat the container's time,
which is often the encode time (L-210). The two sources nobody confirmed rank below every stage-1
source, so running `clocks` never changes which anchor a session's clock times come from unless the
session had none. When `screen_clock` is selected, clock times are local with `utc` null, as a zoneless
`--recorded-at` gives. Selection stays per session, never per item, so one session's items never mix
two clock faces.

**A clock without a date is not an anchor.** Stage 1's `local` is a date and time, and a time of day
alone cannot be written as one, so a taskbar's `2:03 PM` adds nothing to `anchors` until it is
confirmed. It still gives rollovers, cuts, rates and offset observations. A date is never borrowed
silently.

`TimeAnchors` now allows one anchor per clock per segment for the screen sources, keeping the bound of
16: the most precise are kept and the report lists all of them. Stage 1's one-per-source rule stays
for its three sources. `consistency` compares anchors at their own positions, through accepted gaps
only, with each anchor's precision added to the two seconds. So an anchor after a detected gap that
disagrees with one before it is `conflicting`, which is what it is until a person says otherwise. A
local time is still never compared.

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

Zones are given as an offset, not an IANA name, for `0.3.0` (the form decision 9 gave only as an
example; L-213). An offset is what the observation already showed, it is exact for the moment
observed, and it needs no zone database in the binary. A name would add a dependency that must follow
rule changes.

### 17. The command: `vsift clocks`

```console
vsift clocks <session> [--max-frames 0..200] [--runner <path>] [--threads 1..8]
                       [--screen-zone <clock>=<±HH:MM>]... [--accept-gap <id>]... --json
```

- It works on the whole session, because an anchor and a segment are the session's, not a range's.
- It is opt-in and bounded, and it never runs `ocr`'s plan: with no index it answers `state: not_run`,
  `partial`, naming `ocr`. Absence is never a claim: "no clock found" speaks only for the frames read.
- It reads at most `--max-frames` refinement frames (default 40), then recomputes and commits.
  `--max-frames 0` reads nothing and needs no runner, so a confirmation never needs one.
- It reports each excursion (decision 13) with its span, frames and read count.
- `--screen-zone` and `--accept-gap` take only what the user stated, never an observation or a
  detected gap. The skill's command reference, which classes `clocks` as `explicit`, says so. They are
  the only two ways an open session's clock times change.
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
- `clock_segments`, at most 32 (*tentative*): the gaps of decision 13, each with its id, state,
  position, jump and proving clock. Gap ids stay stable as clock ids do: a recomputed gap takes the id
  of the entry whose position and jump it meets, and an accepted gap stays accepted.

They are stored there, not in an artifact, because every command that presents a `clock_time` reads
the lifecycle already and none should have to open a 48 MiB index. They are written by a new lifecycle
update under the session's writer lock, and decoding rebuilds them through the domain constructors,
so a modified record is an integrity failure. A worst-case lifecycle record (16 anchors, 8 clocks, 32
segments) is tested against the manifest's 128 KiB bound. Running `clocks` adds anchors and segments
but changes a clock time only where it fills an empty ladder; a session's clock times otherwise change
only through `--accept-gap` or `--screen-zone`. Its operator and tag anchors stay as they were opened.

### 19. The JSON, additive only

- `time_anchor.anchors[]` gains optional `clock_id` and `precision_us`, and `source` gains the three
  values above: the schema's description has told readers since stage 1 to treat an unknown source as
  a reason to read the anchors. `confidence` does not change.
- `time_anchor` gains optional `segments`: `segment`, `from_us`, `to_us`, and for each gap its
  `gap_id`, `state` (`detected`, `accepted`), `jump_us`, `uncertainty_us`, `clock_id` and
  `adjusted_by_us` (0 while `detected`). It is absent with one segment.
- `clock_time` gains optional `segment` and, when the source is a clock, `clock_id`. Both are absent
  when they say nothing, so a session without them answers byte for byte as before. `segment` on an
  item whose gap is `detected` is the sign that its time may be off by that gap.
- `time_anchor` stays absent while a session has no anchor, as in stage 1. A session whose only clock
  has no date therefore has none; `clocks` reports it.
- `session status` does not list the clocks. `clocks` with `--max-frames 0` does.

### 20. Bundles and worker requests (the maintainer's answer of 2026-10-23)

- The bundle manifest gains an optional `time_anchors` member, with the segments and the registry,
  written only when present and checked by `bundle validate`, so a retained session keeps its anchors.
  `BundleManifest` denies unknown fields, so `bundle validate` of a release before `0.3.0` refuses a
  bundle that carries the member; `cli-v1.md` says so, as for earlier members. A worst-case manifest is
  tested against its bound.
- Worker job requests gain an optional `recorded_at`, parsed as `ingest --recorded-at` is.

### 21. What this does not change

- **L-239.** No command takes a clock time, and stage 2 adds none. The logs-by-clock subtraction stays
  right unless a person accepted a gap, since a detected gap changes no value. Where one was accepted,
  `recording_start` alone no longer gives the window, and step 5 of the workflow, which cites only
  items whose own `clock_time` falls in the window, shows that as a mismatch rather than a false match.
- **The skill.** Only `clocks`' line in `references/commands.md` changes here. The handoff's clock
  citations and the workflow's segment wording are the one skill change the maintainer ruled, in a
  small pull request of their own after this one.
- **L-210** is rewritten: its cut bullet says that `clocks` detects cuts where a clock is in view, that
  a detected gap changes no value until a person accepts it, and that a cut with no clock in view is
  still silent; its "no command adds one later", worker and bundle clauses go.
- No clock is read from speech, and no modification time or upload date becomes a source.
- Clocks are not read in the background: `prepare` and `vsift <video>` do not run `clocks`.

### 22. Evidence first, then slices in one pull request

**Before slice 1:** one hosted dispatch, synthetic only, with the existing runner and no new code in
VSift, asked whether taskbar-sized clocks can be read at all; under two thirds parsing exactly at
1080p would have stopped the work. Run 41069543181, 2026-10-30, drew 12 to 14 px clocks in stand-in
fonts on light and dark bars, in 12-hour and 24-hour faces, with and without seconds:

| Case (H.264, `veryfast`) | Frames | Exact | Wrong | Not found |
| --- | --- | --- | --- | --- |
| Native 1080p, CRF 23 | 960 | 100% | 0% | 0% |
| 1440p scaled to 1080p, CRF 23 | 960 | 100% | 0% | 0% |
| Native 1080p, CRF 32 | 960 | 93.65% | 3.33% | 3.02% |

At CRF 32, right readings scored 0.9057 at the lowest and wrong ones 0.8993 to 0.9997, so the two
overlap and no bound separates them. The desktop was nearly still and the fonts were stand-ins, so
this shows that small text survives at this quality, not that a real taskbar reads well.

1. Zoneless clocks as their own sources: grammar, tracks, intervals, rollover search, `clocks`, the
   registry, the anchors, the schema and the contract tests.
2. Cuts and pauses: discontinuities, segments, `--accept-gap`, the conflict report.
3. Offset observations and `--screen-zone`.
4. Bundles and worker requests, as the last commit, cuttable.

A slice that does not qualify at the cut leaves the earlier ones standing.

**After slices 1 and 2:** the second dispatch, small and synthetic only, run 41111066004 at `9c95bff8`
(so before the search learned to pass over a stopped bracket).

*Held values.* 320 consecutive minutes, each held for 60 s of video at 12 px and CRF 32 and read four
times from 5 s after it changed: **1,280 exact, none wrong, none missing.** So whether a misread persists
across a held minute is **not answered**: there was no misread to follow.

*End to end.* The commit's `vsift ocr` and `vsift clocks` over four recordings of known clocks, at 14 px
and CRF 23: all four gave the clocks and gaps drawn. **Its precision of 250 ms and its measured cut of
419.8 s, ±0.45 s, are withdrawn, and so is every excursion it reported (1, 2, 1 and 1 on clocks drawn
without a fault):** the test's drawing was at fault. FFmpeg's `between()` includes both ends, so the frame
at each exact minute showed both values, and `ocr`'s 10 s grid reads those frames. Two reads either side
of a rollover give an offset as wide as the time between them plus the paint slack, so 250 ms is what two
readings on one frame give.

**After the planner's review:** the third dispatch, small and synthetic only, run 41115362147 at
`5a5a0144`, after the turn fix and with one value drawn on each frame.

*Held values.* 240 consecutive minutes, each held for 60 s of video at 12 px and CRF 32 and read at 0.1,
0.5, 1, 2 and 30 s after it changed: **1,200 exact, none wrong, none missing,** at each of the five times.
So misreads do not gather in the frames just after a change, on these recordings, which rules out the
fear that a rollover search reads the worst frames. Whether a misread persists is **still not answered**,
and the excursion rule stands regardless, because a presenter's turn takes the same shape. Together the
two runs held 560 values with no misread, which bounds a misread of a held value at about 0.5%, against
3.33% of reads in the first run. Reading soon after a change does not explain the difference on its own. Still not separated:
how often the screen changes (every 2 s against every 60 s), how many digits change, and the frame rate
(30 against 10).

*End to end.* The same two commands at 14 px and CRF 23 over five recordings of known clocks, now with a
presenter who hands back: **all five gave the clocks and gaps drawn, `complete` in one `clocks` call, with
no excursion and no stopped bracket on any clock.** The two presenters gave two clocks and no gap. Each
clock is precise to 550 ms, which is what the rollover search reaches on 10 fps video: a 300 ms bracket
plus the 250 ms paint slack. The cut of 420 s was measured as 420.0 s, ±0.55 s. A screen of static times
set 92 reads aside and made no clock.

### Limits (L-213)

A clock in a script the recogniser does not read, or a format outside the grammar, is not seen. A
12-hour clock without AM or PM is refused. A clock read only every 10 s can hide a short cut; a cut
with no clock in view is silent. A real pause of a whole number of quarter hours reads as two clocks.
A screen showing a video that shows a clock is reported as a clock. A real clock passing midnight
during the recording, with seconds and no date, reads as a counter. A timer that began before the
recording and is past its first hour reads as a clock, because it never read zero in view; it has no
date, so it gives no anchor. Zones are confirmed as an offset,
not an IANA name. A detected gap can be a change of machine rather than a cut, which is why it changes
nothing until a person accepts it. A misread held long enough at the end of a recording reads as a
detected gap, and an hour-digit misread reads as a second clock. An excursion's cause is not known.
Real clocks are untested: the evidence is synthetic, in stand-in fonts without ClearType. A screen
clock that was simply wrong gives a wrong reading: stage 2 reports how clocks move, not that any of
them is right.

### Verification planned

- Domain: grammar tables, and property tests for the model (decision 11), with generated clocks of
  known offset, rate, cuts and midnight crossings, and with a misread injected and held across several
  reads of one minute at each digit position: no gap, the offset unchanged, the excursion reported.
  Pure planner tests for the rollover search, including a misread midpoint, a resumed search and never
  bracketing across an excursion's reads.
- Application and engine: the stand-in reader, covering a search stopped and resumed, a concurrent
  commit, and `--max-frames 0` starting no runner.
- Store: every new member round-trips, refuses each modification and is absent when unused.
- Contract: schema conformance, examples, `time_anchor_cli_contract` extended, and a session that never
  ran `clocks` compared byte for byte.
- Hosted only, at most two dispatches, the first being decision 22's. The second, on `P14 real media`:
  synthetic recordings of known clocks generated on the runner (one clock, two clocks in different
  zones, a cut, a pause, a freeze, a midnight crossing, a meeting timer past its first hour, a chat full
  of static times, a scrolling log), and RM01, RM04 and RM10: no recording may gain a clock that a
  person cannot see.
