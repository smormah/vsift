# ADR 0034: What changed on screen between two times

- Status: **Proposed**; accepted by the planner on 2026-11-03 with three changes and three additions,
  which this text includes (continuation, sections 6a and 6; the evidence room, section 6; the
  decoder's memory, section 3; a budget that splits a motion run, section 2; citing, section 5; the
  fuzz target, section 3). The planner ruled every choice at the end; none went to the maintainer.
- Date: 2026-11-03
- Refines: [ADR 0018](0018-visual-candidate-index-and-transcript-search.md) (the visual index and
  `candidates`) and [ADR 0019](0019-evidence-navigation.md) (exact frames and crops). It meets
  [ADR 0029](0029-time-anchoring.md) (clock time), [ADR 0035](0035-search-across-kept-sessions.md)
  (kept bundles) and [ADR 0037](0037-vsift-video-the-action.md) (`vsift <video>`), and changes none of
  them.
- Tracking: the 0.3.0 story W1. Register entry L-211.

## Context

A person or their agent asks "what changed on the screen between 12:30 and 14:10?". Today they page
`candidates`, then call `frame get` once or twice per candidate, and compare the images themselves.
The story asks for one bounded call that answers with the moments the screen changed in the range,
**where** on the screen each change is, and before and after evidence cut from the source's own
frames. It is evidence, not interpretation: VSift says "these pixels differ between these two exact
frames", never "a dialog opened".

What exists, checked on `main` at `2a59bdad`, 2026-11-03, by reading the files named:

- **The visual index** (`vsift-domain/src/visual.rs`, ADR 0018 decisions 9-14) cuts the video into
  fixed 60 s windows and samples each at 2 Hz as 128x72 grey, never kept. Each candidate with a
  `change_window` says the screen changed after `from_us` (the previous sample, an actual decoded
  frame's time) and at or before `representative_us` (the candidate's own frame). A window's first
  candidate is compared with a lead-in sample 0.5 s before it, so a change across a window boundary is
  seen. Three or more changing samples in a row are one `motion_start` followed by one
  `settled_after_motion`. A window keeps at most 32 candidates and counts the rest
  (`candidate_budget_exhausted`). Only the first four hours are indexed.
- **Frames** (`frame get`, ADR 0019) are FFmpeg's own 8-bit RGB PNGs at native size in displayed
  orientation, identified by their exact timestamp, reused on repeat, and stored as evidence. A session
  holds at most 384 evidence artifacts; a call that extracts N images takes N + 1 of them.
- **Crops** (`crop`) cut a rectangle in displayed pixels out of a stored frame, one FFmpeg run each.
- **Clock time** (ADR 0029): every `frame_evidence` and `visual_candidate` record already carries
  `clock_time` from the session's anchoring.
- **The infrastructure already walks FFmpeg's PNGs** (`ffmpeg_media/png.rs`, with `flate2` in the
  lock) but never decodes their pixels.
- **How busy real footage is** (intake note of 2026-10-27, run 40595722898): RM03, a 19-minute talk,
  has 576 screen changes (29.9 a minute) and 115 more dropped by the per-window budget; RM10 has 106 in
  3:21.

## Decision

### 1. A new command, `changes`

`vsift changes <session> --from <us> --to <us> [--limit 1..20] [--cursor <token>] [--crops]`
pages the screen changes whose after frame lies in the half-open range, 10 per page by default. It is
a new command because `candidates` is published as a shortlist that is never an image, and this
answer is evidence with images. The engine operation is `Engine::screen_changes`; the CLI only
presents it. Additive v1 only: a new command, new schemas, existing failure codes.

Times are position times in microseconds, as every other command takes them. A clock-time range
(`--from-clock`) would resolve through ADR 0029's existing anchoring and never a clock reader of its
own; it is not in slice 1.

### 2. What a change is

The changes of a range are read from the visual index, which a call without a cursor first extends as
`candidates` does (the same use case: missing windows in ascending order, at most 30 per call, the
rest `not_analyzed`; a call with a cursor never analyses, section 6a). No second analysis of the video
is made.

- A `visual_change` candidate is one change: **before** is the frame displayed at its
  `change_window.from_us`, **after** is the candidate's own frame.
- A `motion_start` candidate and the `settled_after_motion` candidate that ends its run are **one**
  change, `kind: "motion"`: before is the frame at the motion start's `change_window.from_us`, after
  the settled frame. A run still moving at its window's end ends at its last candidate, with
  `settled: false`. A page never splits the pair.
- `first_frame` and `periodic_coverage` candidates are not changes: nothing is compared with them.
- **A window whose budget dropped candidates never pairs.** Once a window over its 32 candidates has
  dropped any, a kept `settled_after_motion` may end a later run whose `motion_start` was dropped, and
  no rule could tell that pairing from a right one. So in such a window each kept motion candidate is a
  change of its own: a `motion_start` is a `motion` change with `motion: "start"` (after is its own,
  first moving frame; `settled: false`), and a `settled_after_motion` is a `motion` change with
  `motion: "end"` (before is the frame at its own `change_window.from_us`, the last moving sample;
  `settled: true`). The window's `candidate_budget_exhausted` gap counts what was dropped. A test covers
  both. (The first text of this section paired them and named the second case `run_start_dropped`;
  `motion: "end"` says the same, and the planner accepted it on 2026-11-03.)
- **Which changes a range holds:** those whose after frame lies in it. A change's before frame, and a
  motion run's start, may lie before `--from`.

Both frames are actual decoded frames that the index sampled, extracted exactly as `frame get` does
(the same extraction, profile `p09-r0-v1`, identities and reuse), so the before and after images of
an answer are ordinary `frame_evidence` an agent cites by `evidence_id`. Consecutive changes often
share a frame (the after of one is the before of the next), which is extracted and counted once.

### 3. How regions are found (profile `screen-change-regions-v1`)

Pure, deterministic Rust in the domain, over the pixels of the two stored PNGs themselves. No new
engine, no new crate, no second decode:

1. **Pixels.** The infrastructure decodes each stored frame artifact, after checking it against its
   SHA-256: the PNG chunks are walked as today, `IDAT` is inflated with `flate2` (already a dependency)
   to exactly `height * (1 + 3 * width)` bytes, never more (a stream that ends early or runs on is
   refused), and the five PNG row filters are undone. Only what our own extraction writes is accepted
   (8-bit RGB, no interlace, the frame's displayed size).
   - **Size refused before inflating.** The header is read first; a frame over 16,000,000 pixels (the
     extraction's own `MAX_FRAME_PIXELS`) or a file over 64 MiB (`MAX_FRAME_BYTES`) is refused with
     `RESOURCE_LIMIT` before any inflate. A test covers the refusal.
   - **Peak memory.** The two frames of one change are compared one pair at a time, never a page's
     frames together: the two stored files (at most 64 MiB each) and two decoded RGB buffers of at most
     48 MB each (16 megapixels times 3), so at most about 230 MiB for a 16-megapixel pair and about
     15 MB for a 1920x1080 one. The 200-megapixel and 256 MiB bounds of section 6 are evidence bounds,
     not this.
   - **Fuzzing, in slice 1.** The decoder is a provider-output parser, so a fuzz target is published
     beside the PNG walker (ADR 0016 decision 6), seeded with a PNG extracted from a generated synthetic
     fixture, never a private recording.
2. **Changed pixels.** A pixel changed when any of its R, G and B differs by at least 32 levels.
3. **Changed tiles.** The frame is cut into 16x16 tiles of displayed pixels (smaller at the right and
   bottom edges). A tile changed when it holds at least 4 changed pixels, so isolated compression noise
   is not a change.
4. **Regions.** Changed tiles are joined when they touch, including diagonally, or are one unchanged
   tile apart, so a line of words is one region. Each region is the bounding rectangle of its tiles in
   displayed pixels, clipped to the frame: the same space `crop --rect` takes, so a region can be
   cropped as it stands.
5. **Each region** has `x`, `y`, `width`, `height` and `changed_pixels`. At most 8 regions per change,
   the largest by `changed_pixels`; `regions_left_out` counts the rest, and `changed_area` is the
   bounding rectangle of every changed tile with `changed_pixels` in all, so nothing is silently lost.
6. **No region.** When the index saw a change and the two frames differ nowhere at the threshold (a
   change below it, or slow drift, which the index measures against an earlier sample than the before
   frame), `regions` is empty and `no_region_at_threshold` is true. The change is still listed.
7. Frames of different displayed sizes (a stream whose size changes) are not compared:
   `regions` is empty and `comparison` is `dimensions_differ`.

The 32-level and 4-pixel values are first values, chosen on the synthetic corpus and measured on real
screen recordings in the hosted run (section 9). They are part of the profile: changing them is a new
profile, never a silent change.

### 4. Crops (slice 2, `--crops`)

With `--crops`, each of a change's two largest regions is cut out of the before frame and the after
frame through the existing `crop` (the source's own pixels at native size), so a change carries up to
four crop evidence ids. A call makes at most 16 crops; regions beyond that are listed with their
rectangle and no crop, and the result says so in `data.crops` (`made`, `regions_without_crop` and
`stop`), never in its status: a crop stop leaves the result `complete` or `partial` exactly as the
listing makes it (the note of 2026-11-04). Without `--crops` no crop is made. The request carries the choice as a two-value enum, never a
boolean (AGENTS.md; the planner, 2026-11-03); the command line keeps the flag.

### 5. The answer

`data` (`changes-data.schema.json`, example `changes.json`): `session_id`, the requested `range`,
`index` (as `candidates` reports it), `profile` (`screen-change-regions-v1` with its thresholds),
`coverage`, `changes`, `frames` (each distinct `frame_evidence` record of the page, once), `files` (as
the frame commands deliver them), `partial_reason`, `next_cursor`, and `resume` (section 6a: `null`,
or `from_us` and `reason`). Each change
(`screen-change.schema.json`, a published record):

- `change_id` (`chg_`, a digest of the session, source, stream, index profile, region profile and its
  candidate ids, so it is the same on every call);
- `kind` (`change` or `motion`), `motion` (`run`, `start` or `end`; null for a `change`), `settled`
  (motion only), `candidate_ids`, `change_window`;
- `before` and `after`: `evidence_id`, `time_us`, `clock_time` (from the frame evidence record);
- `comparison` (`compared`, or `dimensions_differ` when the two frames' displayed sizes differ). A
  change whose frames could not be taken (the evidence room, the deadline or a cancellation) is not
  listed: the page stops before it, `partial_reason` says why and `resume` names it (section 6a);
- `regions`, `regions_left_out`, `changed_area`, `no_region_at_threshold`;
- with `--crops`, each cropped region's `before_crop` and `after_crop` evidence ids.

`--events jsonl` streams the page's `frame_evidence` records, then one `screen_change` record per
change (key `change_id`), then the terminal event. A readable view prints one line per change (time,
regions as `x,y wxh`, the two frame files and their evidence ids).

**Citing.** The help, the guide page and the readable view tell the reader to cite a change by its
before and after `evidence_id`s, which `handoff check` knows, and never by `change_id`, which it does
not (section 10).

### 6. Bounds

- **Range:** any length inside the video; the page bounds the work, not the range. Analysis is
  `candidates`' (30 windows a call); frames and regions are bounded by the page.
- **Page:** 1 to 20 changes, 10 by default: at most 40 frames, within `frame get`'s 100 frames,
  200 megapixels, 256 MiB and the 120 s deadline, which apply to the call as they do to a burst.
- **Regions:** 8 per change. **Crops:** 16 per call, with `--crops` only. A crop takes two slots
  (its record and its image) unless it repeats one, so `--crops` adds up to 32 slots a call, and with
  crops on every page a session holds about 50 to 110 changes rather than 95 to 190 (the note of
  2026-11-04).
- **Session room.** What a page costs, checked in the code (`vsift/src/evidence.rs`,
  `EvidenceCall::known_media`, `find_reusable_record`) and as built (note of 2026-11-03 below): each
  frame is taken by one `frame get`. A request repeated exactly is reused and takes no slot; any other
  takes one slot for its evidence record and one for its image unless the session already holds that
  image. So a change takes 2 to 4 slots (2 when its before frame is the previous change's after
  frame), and a session's 384 slots hold about 95 to 190 changes with frames. A page stops at the last
  change whose frames fit, ends `partial` with `session_evidence_budget`, has no `next_cursor` (that
  session has no room for the next page), and gives `resume` (section 6a). If not even the first
  change fits, the call is `RESOURCE_LIMIT` with `frame get`'s remediation (retain the session, open a
  new one with `ingest`).
- **Four hours:** past the indexed four hours there are no changes, as there are no candidates, and
  coverage says so.

### 6a. Continuation

- **A call with a cursor never analyses**, as for `candidates`: only a call without one extends the
  index (at most 30 windows), then pages the analysed part.
- **The cursor** (`CursorToken`, as for `candidates`) is bound to the session and its expiry at issue,
  the source, the stream, the index profile, the region profile, the range, `--limit`, `--crops`, the
  state of every window the range touches (not analysed, undecodable, or its candidate identities), and
  the page's last change. Any other value, or a window in the range analysed since, is
  `INVALID_ARGUMENT`, never a silent restart.
- **`resume`** says where a fresh call (a new `--from`, no cursor) continues when this one cannot:
  `{from_us, reason}`. `from_us` is the after time of the first change not listed, or the start of the
  first unanalysed window when that comes first. `reason` is `not_analyzed` (the pages reached the end
  of the analysed part; a fresh call analyses from there), `session_evidence_budget` (a fresh call in a
  new session, after `ingest`), `deadline_exceeded` or `cancelled`. It is `null` when the range is
  answered to its end. A page with more changes after it in the analysed part has a `next_cursor`
  instead.
- **A test** pages a range over 30 windows to its end on a generated fixture, in the synthetic or fault
  suite (never real media or load): the pages, then `resume` with `not_analyzed`, then a fresh call
  from it, listing every change once.

**A recording as busy as RM03** (about 30 changes a minute): a 100-second question such as 12:30 to
14:10 is about 50 changes, five pages and 100 to 200 slots, inside one session. The whole of RM03 is
not: one session answers about 3 to 6 minutes of it, then stops with `session_evidence_budget` and
gives `resume` for a new session. The 115 changes RM03's index drops at its per-window budget are never
listed; coverage counts them in each window's `candidate_budget_exhausted` gap.

### 7. Coverage: what was not compared, and why

`coverage` carries the index's own gaps for the range (`not_analyzed`, `deadline_exceeded`,
`undecodable`, `no_decoded_frame`, and `candidate_budget_exhausted` with the number of changes it
dropped). A page that stopped early (section 6a) adds the rest of the searched range, from
`resume.from_us`, to the envelope's gaps under the stop's reason. Every listed change was compared, so
there is no count of uncompared changes. The envelope `coverage` is filled as for `candidates`; a
result with any gap or stop is `partial` with a fixed warning, exit 0.

### 8. What it never claims

- What a change means: no "dialog", "typed", "scrolled". Regions are rectangles of pixels that differ.
- That nothing else changed: differences below 32 levels, or in fewer than 4 pixels of a tile, are
  not reported; a cursor or one character is often below the index's own 128x72 grid and never
  becomes a change (ADR 0018's note of 2026-10-19: one character is seen 19% of the time).
- That nothing changed between samples: the index samples at 2 Hz, so something that came and went
  within half a second may be missing; a one-sample screen is listed as it is (`transient`).
- When exactly it changed: only that it was after `change_window.from_us` and at or before
  `change_window.to_us`.
- That regions are interface elements, windows or text.

### 9. Evidence on real media (hosted only)

The `P14 real media` campaign gains a `changes` step for RM01, RM04 and RM10, over the whole of each:
the number of changes, regions per change, region sizes, `no_region_at_threshold` cases, pages and
seconds. Scoring reads a change-time label file per recording when it exists (a labelled change is
found when a listed change's window holds its time) and reports recall and false changes; until the
maintainer has labelled RM01 and RM10 the report gives what was found and no accuracy figure. At most
two dispatches: one after slice 1 to check the thresholds on real compression noise, one on the final
head.

### 10. What slice 1 leaves out

- **On-screen text** that appeared or disappeared (`ocr`, ADR 0030): not in slice 1. A later slice
  could compare the OCR lines of the before and after frames when `ocr` has read both.
- **`vsift <video>`** (ADR 0037): not in slice 1. Its `screen-changes.txt` could later name regions;
  that is a change to `watch` with its own review.
- **Kept bundles** (ADR 0035): the before and after frames are ordinary evidence records, so `session
  retain` carries them with their clock times. Regions are not stored (they are recomputed from the
  frames on every call), so a kept bundle carries no regions, and `bundle search` does not search
  changes.
- **Citing a change:** `handoff check` knows no `chg_` id. An agent cites the before and after
  `evidence_id`s, which it already checks; a change citation is a later skill change.

### 11. Where the code goes

- **Domain:** pairing candidates into changes; the region rule over two RGB buffers; `ChangeId`;
  the profile's thresholds as typed constants.
- **Application:** the use case that extends the index, pages changes, extracts frames through the
  existing `FrameExtractor` and evidence commit, asks a new port for the stored frames' pixels, and
  builds the regions; a port `FramePixels`.
- **Infrastructure:** the PNG pixel decoder and the `FramePixels` adapter over the session store.
- **Engine, contract, CLI:** `Engine::screen_changes`, the wire types and schemas, the `changes`
  command with help, examples and a guide page.

No new dependency. Failures use existing codes only (`INVALID_ARGUMENT`, `INVALID_SOURCE`,
`MISSING_CAPABILITY`, `RESOURCE_LIMIT`, `INTEGRITY_FAILURE`, `UNSUPPORTED_SCHEMA`, `BUSY`,
`DEADLINE_EXCEEDED`, `CANCELLED`, with their published remediations), so the trial freeze is not
broken by a code.

### 12. Slices

1. **Slice 1:** the changes and their regions, with exact before and after frames, `--json`,
   `--events jsonl` and readable text, the pixel decoder and its fuzz target, continuation and
   `resume`, schemas, contract tests, `cli-v1.md`, guide reference, records.
2. **Slice 2:** `--crops`.
3. **The skill:** one line in `references/commands.md`, the story's only skill edit, under the freeze
   rules of `v030-rules.md` (a draft freeze labelled as such, rewritten at the merge rebase).

## Choices, as the planner ruled them (2026-11-03)

Every choice was accepted as proposed; none went to the maintainer.

1. A new command `changes`, not a mode of `candidates`. **Accepted.**
2. A motion run is one change (before the motion, after it settles), not two. **Accepted.**
3. Before is the frame at the change window's start (the tightest bracket), not the previous listed
   screen; slow drift may then show no region (`no_region_at_threshold`). **Accepted.**
4. Regions are computed from the stored PNGs by a new decoder in the infrastructure, not from a second
   FFmpeg decode as raw grey: comparing the cited images is the point. **Accepted**, with the bounded
   inflate, the SHA-256 checked first and the fuzz target (section 3).
5. The first thresholds: 32 levels, 4 pixels per 16x16 tile, one-tile bridging, 8 regions, as first
   values of a versioned profile. **Accepted.** The first hosted dispatch reports the noise floor on
   RM01, RM04 and RM10; if they must move, they move before the second dispatch and the pull request
   says so.
6. A busy recording stops at the session's evidence room rather than listing changes without frames.
   **Accepted**, with `resume` (section 6a).
7. `change_id` is published but not citable in `handoff check` in 0.3.0. **Accepted**, with the citing
   rule of section 5.
8. The skill mentions `changes` in one line. **Accepted** (section 12).

## Note of 2026-11-03: what the build found

- **One `frame get` per frame.** Slice 1 takes each frame through `frame get` (at the sample's own time,
  at or after with tolerance 0, or by candidate), so every image is an ordinary evidence record anyone
  can ask for again. A page-wide evidence operation would hold about twice as many changes per session,
  but it would add an operation to the stored evidence records and their published schema; it is not
  built. Section 6's room figures follow from it: about 95 to 190 changes per session, where the first
  text said 180 to 350.
- **`comparison` is `compared` or `dimensions_differ`.** A change whose frames were not taken is not
  listed (the page stops before it and `resume` names it), so no listed change is "not compared", and
  the first text's `changes_not_compared` count is not published (sections 5 and 7 now say so).
- **No pairing in a window with drops**, and `motion: "end"` instead of `run_start_dropped` (section 2).
- **Codec noise on the synthetic corpus.** On F04 (H.264), the status edit at 10 s is one 400x48
  region of 7,140 changed pixels, and six more regions of 9 to 25 changed pixels each sit around text
  elsewhere, where the re-encoded frames differ by 32 levels or more in a few pixels. They come after
  the real region, which is the largest. The first hosted dispatch measures them on RM01, RM04 and
  RM10; a higher `tile_pixels` (32 would remove every one seen on F04) is the candidate change before
  the second, as the planner ruled.

## Note of 2026-11-03: the first real-media run, and the thresholds stay

`P14 real media` run 41704516533 (hosted Ubuntu 24.04, mode `changes`, at `1de2c59f`) measured the
whole of three screen recordings in 396 s. A "small" region has fewer than 32 changed pixels:

| | Changes | A minute | Motion | Regions a change (p50, p90) | Changes with a small region | Small regions | No region |
| --- | --- | --- | --- | --- | --- | --- | --- |
| RM01 | 22 | 9.7 | 0 | 2, 8 | 36% | 19 | 0 |
| RM04 | 124 | 24.8 | 62 | 1, 3 | 13% | 19 | 2 |
| RM10 | 102 | 30.4 | 37 | 1, 4 | 19% | 33 | 0 |

Small regions are a minority and always come after the larger ones; only 3 of 248 changes reached the
8-region cap. Raising `tile_pixels` to 32 would also drop changes a person may want (a caret, one
character), which no measurement can tell from codec noise until RM01 and RM10 are labelled. So
`screen-change-regions-v1` keeps its first values; a later profile may move them with labels.

## Note of 2026-11-03: the second real-media run

Run 41706729159 (at `af0d5534`) listed exactly the same changes as the first, and added what a run
costs. Whole recordings in 478 s:

| | Calls | Sessions | Seconds | Seconds a call | Regions | Share of regions small |
| --- | --- | --- | --- | --- | --- | --- |
| RM01 | 2 | 1 | 49.7 | 24.9 | 74 | 26% |
| RM04 | 7 | 2 | 211.4 | 30.2 | 211 | 9% |
| RM10 | 6 | 2 | 217.3 | 36.2 | 207 | 16% |

RM04 (5 minutes, 124 changes) and RM10 (3:21, 102 changes) each filled one session's evidence room and
went on in a second from `resume`, as section 6a says. The thresholds stay.

## Note of 2026-11-04: crops, slice 2

The planner accepted the crops design on 2026-11-04 with one change (that a crop stop never makes the
result partial). As built:

- **When.** Crops are cut only after every change of the page has its frames and has been compared,
  so a page lists the same changes with or without `--crops`, and a crop never costs a listed change
  its frames.
- **Which.** Each listed change's first two regions (the largest), in page order; for each, the
  before crop, then the after crop, of the same rectangle, through `Engine::crop` with the frame's
  evidence item as parent. At most 16 crops a call (8 regions, about 4 changes with two regions each).
  No size floor: the second region is often a few pixels of codec noise (13 to 36% of changes on
  RM01, RM04 and RM10), and its crop is then close to empty; a floor would be a new threshold with no
  labels to weigh it.
- **The answer.** A listed region gains `before_crop` and `after_crop`, each present only when made.
  The crops are `frame_evidence` records of kind `crop`, listed in `frames` after the whole frames
  and delivered in `files` in the same order (at most 56 of each with `--crops`). `data.crops` is
  `{limit, made, regions_without_crop, stop}`, present only with `--crops`, so a call without it is
  slice 1's answer byte for byte. `made` counts the crop ids the regions name: one change's after
  frame is often the next change's before frame, so a region's after crop can be the same record as
  the next change's before crop, named twice and counted twice but listed once in `frames`. The
  16-crop bound counts names (the planner, on #574).
- **Stops.** `crop_limit` is the published bound and adds no warning. `session_evidence_budget`,
  `deadline_exceeded` and `cancelled` stop the cropping and add a fixed warning (run the same call
  again; after `session_evidence_budget`, a new session). None of them makes the result `partial`:
  `changes-data.schema.json` publishes that status is `partial` exactly when there is a gap or
  `partial_reason` is set, and `partial_reason` means the listing stopped. A crop made before a stop
  is kept even when its partner is not, because it is committed evidence.
- **The request** carries `ScreenChangeCrops` (`NoCrops`, `LargestRegions`); the cursor digest binds
  the same words slice 1 bound for its boolean (`no-crops`, `crops`), so a slice-1 cursor still
  continues. No new failure code, no skill change.
- **No hosted run** (the planner): crops reuse `crop`, already qualified, and their cost follows from
  slice 1's runs. A local opt-in engine test makes one call that reaches the 16-crop bound on a 12 s
  1280x720 video it generates with FFmpeg (synthetic, at a real screen's size, not a real recording),
  whose changes come in pairs half a second apart, so crops repeat: 16 named, 12 records listed
  (F04 has one change with regions, so it cannot reach the bound); its wall time and slots are in the
  pull request.

## Consequences

- One call answers "what changed between these times" with the frames to look at and where to look.
- Every image is an existing evidence record, so retention, `handoff check` and clock time work for
  it unchanged.
- Very busy footage is answered a page at a time and runs out of a session's evidence room; the answer
  says where it stopped.
