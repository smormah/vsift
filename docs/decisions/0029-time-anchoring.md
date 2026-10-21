# ADR 0029: Time anchoring (clock times on timestamped results), stage 1

- Status: **Proposed, 2026-10-21.** The design was sent to the supervisor on 2026-10-21 and
  given the go the same day; the maintainer named the feature "time anchoring" and decided
  stage 1's scope and its split from stage 2 (below). It becomes Accepted with the merge of
  its pull request, after the `0.2.3` cut, for `0.3.0`.
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
