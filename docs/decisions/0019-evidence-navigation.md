# ADR 0019: Evidence navigation

- Status: Accepted (maintainer, 2026-09-26)
- Date: 2026-09-26
- Tracking: [P09 / issue #12](https://github.com/smormah/vsift/issues/12)
- Refines: [ADR 0008](0008-cli-and-json-contract.md) (the reserved `frame get`,
  `frame neighbours`, `frame burst`, `audio` and `crop` commands),
  [ADR 0012](0012-p04-source-media-profile.md) (frame selection, bounded extraction and
  the bracketed source binding), [ADR 0013](0013-retained-bundle-publication.md)
  (artifact and manifest bounds), [ADR 0016](0016-embeddable-engine-and-evidence-contract.md)
  (the evidence contract) and [ADR 0018](0018-visual-candidate-index-and-transcript-search.md)
  (a candidate's `representative_us` is what `frame get` extracts)
- Scope of this record: the media primitives and the pure navigation rules delivered by
  P09 PR 1 (not user-reachable), and the design the later P09 pull requests follow. The
  maintainer confirmed decisions D1-D7 on 2026-09-26 (below); PR 2 builds the evidence
  core on them (implementation note below).

## Context

P08 tells an agent *when* the screen changed; P09 must hand it the evidence itself: the
exact frame at a time, the frames around it, a burst over a stretch of time, a crop of a
region, and a short audio clip (V-01, V-06..V-08). Four questions had to be settled
before any public command: which frame a time names when no frame lies exactly there;
how a frame is extracted so that the time reported is the time of the pixels returned;
how a crop keeps its source lineage without a new image dependency; and what each call
may cost. P09 PR 1 also found and fixed a SEC-17 defect in the existing frame and audio
diagnostics readers (see the ADR 0012 note of 2026-09-26).

## Decision

### 1. List first, then extract exact timestamps

A frame request never guesses a frame from a frame rate or seeks by decimal seconds.
The adapter first **lists** the displayed frames of a bounded stretch of the stream
(`FfmpegMedia::list_frame_times`: one decode, `select` on integer timestamp bounds,
`showinfo` diagnostics, no image written), the pure domain rules choose frames from the
listing, and the adapter then **extracts exactly those timestamps**
(`select=eq(pts\,P)`). A listing covers a half-open range completely and says whether the
stream ends there (`ListingTail`); nothing outside it is assumed, so a request a listing
cannot decide is `outside_listing`, never a silent guess.

**Integer-timestamp selection.** `StreamTime::first_at_or_after` returns the smallest
stream timestamp whose normalized time (`to_media_time`, which truncates to whole
microseconds) is at or after a request. `FFmpeg` evaluates decimal `t` comparisons in
floating point, so a frame whose normalized time equals the request (for example a P08
candidate's `representative_us`) could be skipped or kept depending on rounding;
comparing integers cannot. The adapter fails closed (`TimeBaseMismatch`) unless the
filter's `config in time_base` equals the probed stream's time base. Measured with
`FFmpeg` 9.0: every one of the 29 recorded P08 candidates over F01-F10 and F12 is
extracted at delta 0 with the index's displayed dimensions.

### 2. Selection policies (domain `evidence::navigation`)

- **`at_or_after` (default):** the first frame whose time is at or after the request.
  The returned pixels were never on screen before the moment the agent named, so
  evidence is not taken from before it. F01 (20 fps): 1.025 s gives 1.05 s (delta
  25,000 us); 5.97 s is `after_final_frame`.
- **`displayed_at` (`--select displayed-at`):** the frame on screen at the request, the
  last frame at or before it. F01 5.97 s gives 5.95 s (delta -20,000 us).
- **Tolerance** 0..=10,000,000 us bounds the distance in the policy's direction
  (`no_frame_within_tolerance` beyond it). A request at or after the source's end is
  `at_or_after_end` for both policies. Every result keeps requested and actual time and
  the signed delta (ADR 0012).

### 3. Neighbours and bursts

- **Neighbours** are 1..=20 *consecutive displayed frames* on each side of an anchor
  frame, never time-spaced samples. A short side says why: `start_of_stream`,
  `end_of_stream` or `search_window` (the listing searched ended; more may exist).
- **Bursts** spread 1..=100 even time targets over a range of at most 60 s
  (`start + k * duration / count`), take the first frame at or after each target and
  before the range's end, and remove duplicates; the plan reports the target count and
  the number of distinct frames. A range past the source's end is clipped to it and
  says so. Even targets describe a stretch of time independently of the frame rate;
  deduplication stops a static or slow screen from returning one image many times.
- Properties prove plans are bounded, ordered, deduplicated, inside their range, and
  that neighbours are consecutive and stop only for a stated reason.

### 4. Crops by re-decoding in `FFmpeg`

A crop is `x,y,width,height` in orientation-correct displayed pixels
(`CropRect::parse`: canonical unsigned decimals only), validated against the displayed
frame before any I/O; `x + width = frame width` is accepted and one pixel more is
rejected. The adapter re-decodes the exact frame and crops inside `FFmpeg`
(`select=eq(pts,P),format=rgb24,crop=w:h:x:y:exact=1,showinfo`). `FFmpeg` applies display
rotation before these filters, so the rectangle is in displayed orientation: on the
rotated F01 variant (720x1280) every tested crop equals the same region of `FFmpeg`'s own
full decode pixel for pixel. A crop of a crop composes back to source coordinates
(`CropRect::compose`), so lineage always names source pixels. No image library is added
(D7).

### 5. Image and audio profiles

- **Images** are 8-bit RGB PNG exactly as `FFmpeg` encodes them from `format=rgb24`
  (native resolution; nothing is scaled, so no detail is invented or lost). The adapter
  walks the `image2pipe` stream strictly (`parse_png_sequence`): signature, `IHDR` first
  with the expected size, 8-bit RGB and no interlace, one run of `IDAT`, `IEND` last,
  every chunk CRC checked, no palette or other critical chunk, and exactly one image per
  `showinfo` frame line with the expected timestamp and size.
- **Audio** is a WAV clip of at most 30 s, 16 kHz mono signed 16-bit (the speech
  profile ASR already decodes), with the 44-byte header written by `VSift` (the WAV
  muxer writes unknown sizes to a pipe). The clip reports the first decoded sample's
  time: F01's audio-only variant starts at 64 ms (AAC priming), F09 at 750 ms (its
  offset).

### 6. Per-call budgets (SEC-05)

Every evidence run is one supervised `FFmpeg` process with a closed argument list (only
numbers and the private copy's path are filled in), the forced demuxer and `file`
protocol, MOV references disabled, `-xerror`, `-max_alloc` 64 MiB, two threads and a
30 s deadline. A listing covers at most 60 s and 1,200 frames (60 s at 20 fps; a denser
range is cut before the first unlisted frame and says more may follow) with 1 MiB of
diagnostics. An extraction returns at most 8 frames and 64 MiB, and no more frames than
fit that bound at the worst-case PNG size (`max_frames_per_run`: 8 up to 1080p, 2 at
4K, 1 at 16 MP); frames are at most 16 megapixels. A WAV clip is at most 30 s
(960,000 PCM bytes). The run seeks 5 s before the first frame and reads to 1 s after the
last; a timestamp that decodes no frame is `FrameNotFound`, never replaced. The
media-tool preflight (verification profile 3) exercises listing, exact extraction and a
crop on F01 under the existing `frame` check.

## Decisions D1-D7 (confirmed by the maintainer, 2026-09-26)

The maintainer accepted the recommended option of each decision, with these final
details. The alternatives considered are kept for the record.

- **D1 - source check per evidence call.** After one full SHA-256 of the session's
  source copy, an evidence call commits a verified on-disk identity of the copy with its
  evidence: a digest of its size, modification time, device and file index and the
  platform fields the media-tool fingerprint uses (Unix mode, owner and status-change
  time; Windows creation time and attributes). Later read-only evidence calls compare
  that identity only; when none is recorded or it differs, the copy is hashed in full,
  and bytes that differ from the committed source are `INTEGRITY_FAILURE` with nothing
  committed. Every item records `source_check` (`identity` or `full_hash`). This extends
  ADR 0012's accepted residual across calls (ADR 0012 note of 2026-09-26): on Windows a
  same-user rewrite that restores the modification time is caught only by a full hash.
  Mutating and committing operations elsewhere keep their existing policy.
  Alternatives: a full-hash bracket per call, or once per session per time window.
- **D2 - image delivery.** Images and clips are delivered only as the absolute path of
  the committed session artifact, in the result's `files[]`, valid while the session
  exists; evidence records and bundles never contain paths. Alternatives: `--output`
  export copies; inline base64.
- **D3 - selection.** `at_or_after` by default with `--select displayed-at`; bursts use
  even time targets, deduplicated, and report the distinct count; neighbours are
  consecutive frames with typed side stops.
- **D4 - artifact bounds.** 256 artifacts, a 64 KiB manifest and 10 GiB per session
  stay; evidence (frame and crop images, audio clips and evidence records) has a
  sub-budget of 160 artifacts (`MAX_EVIDENCE_ARTIFACTS`). Exhaustion is `RESOURCE_LIMIT`
  with the remediation to retain the session and open a new one. Alternative: raise the
  caps, which needs incremental chain validation (P10).
- **D5 - audio profile.** WAV, 16 kHz mono signed 16-bit little-endian, at most 30 s,
  artifact kind `audio_wav` (at most 1 MiB). Alternative: the source's rate and
  channels, at most 60 s.
- **D6 - command shapes.** `frame get <session> (--at <us> | --candidate <vcd>)
  [--select at-or-after|displayed-at] [--tolerance-us <us>]`, `frame neighbours
  <session> <evidence> [--count 1..20]`, `frame burst <session> --from <us> --to <us>
  [--max-frames 1..100, default 12]`, `crop <session> <evidence> --rect x,y,w,h` and
  `audio <session> --from <us> --to <us>`. Frames link to candidates through the
  candidate id in the request; `visual_candidate` records are unchanged. The grammar
  lands with the commands in PR 3 and PR 4; PR 2 adds only the engine request types.
- **D7 - crop implementation.** Crops by `FFmpeg` re-decode (`crop_at`); no new
  dependency. Alternative: an in-process `png` crate cropping the extracted frame.

Human-readable terminal output (the promise of ADR 0008 and the "readable terminal
text" of `docs/contracts/cli-v1.md`; most commands print pretty JSON today) is assigned
to P13, distribution and user documentation, by the same decision.

## Consequences

- PR 1 (this record, 2026-09-26) delivers the domain rules, the adapter calls, the
  hardened parsers, the preflight and fuzz targets; nothing is user-reachable.
- PR 2 (2026-09-26) delivers the evidence core on D1-D7: application use cases,
  session artifacts, reuse and lineage, and the engine operations (note below). PR 3
  makes `frame get`, `frame neighbours` and `frame burst` public; PR 4 `crop` and
  `audio`, with the P09 qualification record.
- An evidence call re-decodes the source every time; reuse of identical requests (V-08)
  is PR 2's artifact identity, not a cache in the adapter.
- Residuals: `FFmpeg` remains a native decoder with the user's filesystem access (ADR
  0012); a 60 fps source needs several listings for a 60 s burst; a seek that lands
  after a requested frame (a container start far from the probed origin) reports the
  frame as not found rather than extracting another.

## Alternatives

- **Seek by decimal time and take the next frame** (the P04 `frame` call): rejected for
  new calls; float comparison can miss an exact frame and the frame rate is not known
  for VFR sources.
- **List timestamps with `ffprobe -show_frames`:** it decodes the whole range without
  the bounded `select` and reports packet-level fields; the listing shares the
  extraction's decoder, filters and time base instead.
- **Crop in Rust with a new image crate:** a new decoding dependency for pixels
  `FFmpeg` already decodes (D7 alternative).

## 2026-09-26 implementation note: the evidence core (PR 2)

PR 2 builds the evidence core in the engine library (`Engine::frame_get`,
`frame_neighbours`, `frame_burst`, `crop`, `audio`); nothing is reachable from the CLI
until PR 3 and PR 4.

- **Identities.** A request key (`opk_sha256_...`) digests the session, source, stream
  selector (the selection rule, because keys are derived before any probe), operation,
  canonical parameters, adapter profile `p09-r0-v1` and the provider fingerprint (the
  media-tool preflight's fingerprint: executables' identity, adapter, reviewed policy,
  host isolation and verifying authority). An item identity (`evd_...`) digests only
  what fixes the pixels or samples: session, source, stream, profile, provider
  fingerprint, exact timestamp and time base, and the crop region (with its parent) or
  the clipped audio range. Requested times, policies, tolerances and candidates live in
  the request and its selections, so two requests that resolve to one frame share one
  item and one file. When the fingerprint cannot be computed, the item records
  `tool_fingerprint: null`, its identity also digests the file's SHA-256, and nothing
  is reused.
- **Reuse (V-08).** Before any provider runs, a call validates its request, reads the
  session, resolves the tools and runs the preflight (no process when a pass is on
  record), binds the copy (D1), reads the session's evidence records and derives its
  key. A complete record with the same key is returned as `reused` after every file it
  names is verified again (size and SHA-256, INV-02); nothing is written. A partial
  record is never reused. Another provider is another key; a changed copy is
  `INTEGRITY_FAILURE`.
- **Lineage.** Each extracting call commits one `evidence_record` artifact (strict
  versioned JSON, at most 256 KiB,
  [`bundle-evidence-record.schema.json`](../../schemas/v1/bundle-evidence-record.schema.json))
  with its new files in one generation: the request key and parameters, selections
  with requested and actual time, delta and role (`requested`; `before` and `after`
  with the side stops; `target` with its target time), the items and the partial
  reason. It never holds an operation id or a path. Decoding re-derives the key and
  every identity; `bundle validate` also checks the items against their files (ADR 0013
  note of 2026-09-26).
- **Budgets (SEC-05).** Per call: at most 100 frames, 200 megapixels decoded, 256 MiB
  of images (or what the session has left), the session's remaining evidence slots,
  30 s per provider run and 120 s per call. A budget, the deadline or a cancellation
  after something was extracted commits that, and the record carries the reason
  (`frame_budget`, `pixel_budget`, `byte_budget`, `session_evidence_budget`,
  `deadline_exceeded`, `cancelled`); with nothing extracted the call fails with its
  code. A session without room for a record and one file is `RESOURCE_LIMIT` before
  any provider runs.
- **Neighbours** list a 2 s window around the anchor first and widen to 10 s and 29 s
  only when a side stopped at the listing's edge; the nearest frames are extracted
  first, alternating sides, so a budget stop keeps the closest. A **burst** over a range
  denser than one 1,200-frame listing is rejected (`outside_listing`) for now.
- **Errors** use existing codes only: `INVALID_ARGUMENT` (no frame satisfies the
  request, unknown evidence or candidate, a parent of the wrong kind, a crop outside
  its parent, a range too long, a count or tolerance out of range, no audio or video
  stream), `RESOURCE_LIMIT` (the evidence budget), `INTEGRITY_FAILURE` (a changed copy,
  a candidate frame not at its time, a parent that no longer describes the stream) and
  the provider codes (`MISSING_CAPABILITY`, `INVALID_SOURCE`, `DEADLINE_EXCEEDED`,
  `BUSY`, `CANCELLED`).
- **Evidence:** application tests over fake extractors (selection, shared items, reuse
  rules, partial bursts, budgets), store tests (one-generation commits, kept and
  conflicting files, the sub-budget, bundle validation against missing, tampered and
  mis-kinded files, wrong image headers and missing crop parents), the D1 hash-count
  test, engine tests with stand-in tools (warm reuse starts no process; a replaced
  `FFmpeg` is a new provider while the old item stays readable; a modified copy is
  `INTEGRITY_FAILURE` with nothing committed; an exhausted budget fails before any
  process) and opt-in real-`FFmpeg` 9.0 engine tests against the PR 1 truth. Fuzz
  targets `evidence_record` and `crop_rect`.
