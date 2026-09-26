# ADR 0019: Evidence navigation

- Status: Proposed
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
  decisions D1-D7 below need maintainer confirmation before PR 2 starts.

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

## Decisions for maintainer confirmation

- **D1 - source check per evidence call.** Recommended: persist a verified on-disk
  identity of the session's source copy after one full hash; later read-only evidence
  calls compare the identity only, and each evidence item records its `source_check`
  (`identity` or `full_hash`). Alternatives: a full-hash bracket per call (ADR 0012's
  bracketed binding around each call), or once per session per time window.
- **D2 - image delivery.** Recommended: images are delivered as the absolute path of the
  committed session artifact only, in the result's `files[]`. Alternatives: `--output`
  export copies; inline base64.
- **D3 - selection.** Recommended: `at_or_after` by default with `--select
  displayed-at`; bursts use even time targets, deduplicated; neighbours are consecutive
  frames.
- **D4 - artifact bounds.** Recommended: keep 256 artifacts and a 64 KiB manifest per
  session, with a P09 sub-budget of 160 evidence artifacts and `RESOURCE_LIMIT` beyond
  it. Alternative: raise the caps, which needs incremental chain validation (P10).
- **D5 - audio profile.** Recommended: WAV, 16 kHz mono, at most 30 s. Alternative: the
  source's rate and channels, at most 60 s.
- **D6 - command shapes.** Recommended: add `<session>` to `frame neighbours` and
  `crop`, add `--candidate <vcd_id>`, make `--max-frames` optional with default 12, and
  link frames to candidates rather than changing `visual_candidate` records.
- **D7 - crop implementation.** Recommended: crops by `FFmpeg` re-decode (this record).
  Alternative: an in-process `png` crate cropping the extracted frame.

## Consequences

- PR 1 (this record, 2026-09-26) delivers the domain rules, the adapter calls, the
  hardened parsers, the preflight and fuzz targets; nothing is user-reachable.
- PR 2 builds the evidence core (application use cases, session artifacts and reuse,
  engine operations) on D1-D7. PR 3 makes `frame get`, `frame neighbours` and `frame
  burst` public; PR 4 `crop` and `audio`, with the P09 qualification record.
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
