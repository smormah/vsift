# P09 evidence-navigation qualification record

Status: measured 2026-09-26 on P09 PR 4 (`p09/crop-audio`, which builds on PR 3
`p09/frame-commands` and PR 2 `p09/evidence-core`; PR 1 is merged as `965617f`),
Windows 11 Pro, Intel Xeon E5-2698 v4 (20 cores, 40 threads, 2.2 GHz), 64 GiB RAM,
FFmpeg and FFprobe 9.0 (gyan.dev full build), release build. Design:
[ADR 0019](../decisions/0019-evidence-navigation.md) (accepted, D1-D7). Verification
IDs: V-01, V-06, V-07, V-08.

## Method

The opt-in checkpoint `crates/vsift-cli/tests/p09_evidence_e2e.rs` drives the
compiled `vsift` binary exactly as a headless agent would: isolated per-user bases, an
empty `PATH`, and FFmpeg and FFprobe registered with `setup configure`.

```console
cargo test --release -p vsift-cli --locked --test p09_evidence_e2e -- --ignored --nocapture
```

Expectations come only from frozen truth, never from the results being scored:

- the independent `ffprobe` frame lists the fixture verifier recorded in
  [`fixtures/corpus/generated/verification.json`](../../fixtures/corpus/generated/verification.json)
  (F01, F09, F01-rotation-90);
- the corpus manifest (durations, resolutions, audio modes, F03's G18 cell);
- FFmpeg's own decode of the fixture (`select=eq(pts,P)`, `rgb24`) as the pixel
  reference for every whole-frame and crop comparison; and
- the P08 candidates `candidates` returns for the fixtures (29 over F01-F10 and F12,
  the recorded P08 set).

The component evidence below it runs everywhere without FFmpeg:
`crates/vsift-contract/tests/navigation_contract.rs` (the six frozen examples, schema
rejections), `crates/vsift-cli/tests/evidence_cli_contract.rs` (grammar, limits,
failures with remediation, and reuse through the binary with stand-in tools that
cannot run), and the PR 1 and PR 2 domain, application, store and engine tests listed
in [verification](verification.md).

The run passed on 2026-09-26: `p09_evidence: passed`, all nine stages, 435 s. In a
debug build the performance stage uses a 128 MiB clip, because debug hashing is too slow
for a gigabyte within the per-call deadline.

## V-01: exact frames (`p09_frame_exact`, `p09_candidate_frames`)

| Request | Truth | Result |
| --- | --- | --- |
| F01 `--at 2000000` (keyframe) | 2,000,000 | 2,000,000, delta 0 |
| F01 `--at 1950000` (just before a keyframe) | 1,950,000 | 1,950,000, delta 0 |
| F01 `--at 1025000` (between frames) | 1,050,000 | 1,050,000, delta 25,000 |
| F01 `--at 1025000 --select displayed-at` | 1,000,000 | 1,000,000, delta -25,000 |
| F01 `--at 5950000` (final frame) | 5,950,000 | 5,950,000, delta 0 |
| F01 `--at 5970000` | no frame at or after | `INVALID_ARGUMENT`, `after_final_frame` remediation |
| F01 `--at 5970000 --select displayed-at` | 5,950,000 | 5,950,000, delta -20,000 |
| F01 `--at 6000000` (the end), both policies | no frame | `INVALID_ARGUMENT`, nothing committed |
| F01 `--at 1025000 --tolerance-us 10000` | next frame 25 ms away | `INVALID_ARGUMENT` |
| F09 (VFR, 2 s origin) `--at 3200000` | 3,250,000 / displayed-at 3,150,000 | both exact |
| F01-rotation-90 `--at 2000000` | displayed 720x1280 | 720x1280, pixel-equal to FFmpeg's decode |

Every frame is 1280x720 on F01 (native size, 8-bit RGB PNG) with its `pts` and time
base. All 29 candidates of F01-F10 and F12 come back through `frame get --candidate`
at delta 0 with the candidate's displayed dimensions (1, 4, 2, 3, 3, 3, 1, 2, 4, 4 and
2 per fixture).

## V-06: crops (`p09_crop`)

- On the rotated F01 variant (displayed 720x1280), the crops 100,200,300,150; the whole
  frame 0,0,720,1280; the bottom-right pixel 719,1279,1,1; and the right edge
  420,1000,300,280 each equal FFmpeg's own decode of that displayed region pixel for
  pixel; 421,1000,300,280 (one pixel past the edge) is `INVALID_ARGUMENT` with the
  rectangle remediation before any tool runs.
- A crop of the crop 100,200,300,150 at 10,20,50,40 records `frame_x` 110 and
  `frame_y` 220 (source pixels) and equals the source region 110,220,50,40; the
  repeated outer crop is `reused`.
- F03's G18 cell 850,420,280,70 has mean RGB (97, 178, 119) at 3.9 s and
  (203, 92, 88) at 4.0 s: green, then red, as the manifest's event says.
- Tiny text: the first letter of F01's title (a 5x7 glyph drawn at scale 4, the crop
  35,20,20,28) comes back as a 20x28 image equal to the source region, with 240 of its
  560 pixels in the glyph's colour. Nothing is scaled, so a crop never shows detail the
  source did not have; an agent that needs a larger view must scale the image itself.

## Audio (`p09_audio`)

| Request | Truth | Result |
| --- | --- | --- |
| F01-audio-only 0-1 s | first sample at 64 ms (AAC priming) | `actual_start_us` 64,000; 16,000 samples, 16 kHz mono 16-bit |
| F09 0-2 s | audio starts at 750 ms | `actual_start_us` 750,000; 32,000 samples |
| F01-audio-only 5-8 s | audio ends at 6 s | clipped to 5-6 s, `range_clipped: true` |
| F10 (no audio) 0-1 s | no audio stream | `INVALID_ARGUMENT`, no-audio remediation |
| 0-30.000001 s | over 30 s | `INVALID_ARGUMENT`, 30 s remediation |
| F01-audio-only 7-8 s | starts after the end | `INVALID_ARGUMENT`, range-start remediation |

## V-07: bounds (`p09_neighbours_burst`, contract tests)

- Neighbours of F01's first frame (count 2) are 50 and 100 ms with
  `before_stop: start_of_stream`; of the final frame (count 3) the three before it with
  `after_stop: end_of_stream`; count 20 around 3 s gives exactly the 20 consecutive
  frames on each side.
- `--max-frames 0` and `101` are parse errors; bursts of 1, 12 and 100 targets over F01
  name exactly the truth's frames (100 distinct frames, targets 60 ms apart on the
  50 ms grid, 9.2 s); a 60 s range over the 6 s video is clipped
  (`clipped_at_end_of_stream`); 61 s is `INVALID_ARGUMENT` with the remediation to find
  moments with `candidates`.
- Contract: a partial burst carries `partial_reason` and its warning
  (`frame-burst.partial.json`); a session without room is `RESOURCE_LIMIT` with the
  retain-and-reopen remediation before any process; counts 0/21 and tolerances over
  10 s are parse errors.

## V-08: reuse (`p09_reuse`, contract tests)

- A repeated `frame get` returns the same result with `reused: true` in 120-126 ms
  through the binary and writes nothing; 1.04 s names the same item and file as
  1.025 s (both 1.05 s) under a new record (one more artifact, no new image).
- The first call checks the copy with `full_hash`, later calls by `identity` (D1).
- With stand-in tools that cannot run, an identical request is reused with its
  verified file, and every other request fails at the provider
  (`evidence_cli_contract`); a replaced FFmpeg is a new provider and a changed copy is
  `INTEGRITY_FAILURE` (engine tests, PR 2).

## Damaged media, streams and bundles

- `p09_malformed`: F11's damaged audio (`audio`), F11's truncated file (`frame get`),
  and F05 cut to 60 % at run time (`frame get` at 18 s, a burst across the cut) are all
  `INVALID_SOURCE` with nothing committed; the damaged file's video and the cut file's
  first second still extract.
- `p09_stream_and_bundle`: `--events jsonl` for `frame get` (2 events), `frame
  neighbours` (5), `frame burst` (5), `crop` (2) and `audio` (2) conform to their
  schemas with contiguous sequences and matching counts; `session retain` then `bundle
  validate` pass with 5 evidence records, 9 images and 1 clip, every record conforms
  to `bundle-evidence-record.schema.json` and none holds a path.

## Performance (`p09_perf`, recorded, not gated)

| Measurement | Result |
| --- | --- |
| Warm reuse on F01, 20 calls through the binary | p95 142 ms (target 250 ms: met) |
| Cold `frame get` on F01 (after the first call's preflight) | 1.5-2.0 s |
| First evidence call of a base (runs the media-tool preflight) | about 6 s |
| 1080p clip: 1.17 GB, 240 s, about 39 Mbit/s MPEG-4, built at run time | built in 3.3 s by stream copy |
| `ingest` of that clip (copy and hash) | 19.5 s |
| First `frame get` (one full SHA-256 of the copy, D1) | 10.3 s |
| Cold `frame get`, 10 times across the clip | p95 4.1 s (3.3-4.1 s) |
| 12-frame burst over 60 s of it | 15.8 s |
| Warm reuse on it | p95 196 ms |
| Warm reuse at 3 / 64 / 128 / 256 session generations | p95 235 / 335 / 636 / 1,042 ms |

The clip is F07 (1920x1080) with temporal noise so it does not compress, 30 s encoded
with FFmpeg's native MPEG-4 encoder and looped by stream copy to about 1 GiB; it is a
worst case for decoding, far denser than a screen recording.

**Manifest-chain walk.** Every session read validates the whole manifest chain, so a
warm call costs about 3.7 ms more per generation: over the 250 ms target from about 60
generations. An evidence session reaches at most about 160 evidence commits (the D4
sub-budget), where a warm call would take about 0.6 s. Incremental chain validation
(P10, the D4 alternative) removes the growth; until then the cost is bounded by the
evidence and artifact budgets.

## Residuals and known limits

- **D1 source check:** after one full hash, evidence calls compare the copy's on-disk
  identity; on Windows a same-user rewrite that restores the modification time between
  two calls is caught only by a later full hash (ADR 0012 note of 2026-09-26).
- **60 fps listing cap:** a frame listing covers at most 1,200 frames, so a burst over
  more than 20 s of 60 fps video is refused (`outside_listing`); neighbours search at
  most 29 s either side (`search_window`).
- **Tiny text:** measured on synthetic 5x7 glyphs at scale 4 only; real screen text
  with anti-aliasing, subpixel rendering and compression is not in the corpus, and
  crops are never upscaled.
- **Paths:** delivered paths are absolute and, on Windows, in the extended-length form
  `\\?\C:\...` the engine returns; they are valid only while the session exists.
- **Manifest chain:** warm cost grows linearly with session generations (above).
- **Performance** was measured on one Windows machine; Ubuntu and macOS runs of the
  opt-in checkpoint have not been recorded.
