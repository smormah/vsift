# ADR 0018: Visual-candidate index and transcript search

- Status: Accepted (maintainer, 2026-09-26)
- Date: 2026-09-26
- Tracking: [P08 / issue #11](https://github.com/smormah/vsift/issues/11)
- Refines: [ADR 0008](0008-cli-and-json-contract.md) (the reserved `search` and
  `candidates` commands and the envelope `coverage` member),
  [ADR 0016](0016-embeddable-engine-and-evidence-contract.md) (the evidence stream) and
  [ADR 0017](0017-local-asr-through-whisper-cpp.md) (revisions and citations)
- Scope of this record: the transcript-search half is implemented by P08 PR 1, and
  the visual-candidate half by P08 PR 3 (visual index core) and PR 4 (`candidates`
  command), with the two PR 3 deviations from the first design recorded in decision
  10. P08 PR 2 (bracketed source binding, issue #148) is recorded in the ADR 0012 note
  of 2026-09-26.

## Context

P08 turns a session's evidence into something an agent can find: speech by what was
said, and screens by when they changed (V-02..V-05, C-03, S-11). Two questions had to be
settled before code. How is text searched, and what does a search result honestly
promise about what it could not see? And how are visual candidates produced and
indexed without a whole-video decode on every request? The answers constrain the public
contract (`search`, `candidates`, the envelope `coverage`), the evidence stream, and
the session artifacts, so they are recorded here.

## Decision: transcript search

### 1. Command and request

- `search <session> --query <text> [--from <us> --to <us>] [--limit 1..100] [--cursor
  <token>] [--revision <trv_id>]`. The query is literal data: never a pattern, regular
  expression or command. `--from/--to` go together (half-open, segments intersecting the
  range are searched); `--revision` searches an older revision, the newest by default.
- The engine operation is `Engine::search`; the CLI only presents it. Search never runs
  a provider and writes nothing.

### 2. Normalisation (domain, one implementation)

Query and segment text are normalised identically into words: lowercase; `2,048` is
`2048`; a hyphen between letters or digits joins them (`E-409` is `e409`); a colon
between digits separates numbers (`10:32` is `10 32`); a decimal point between digits
is kept and a decimal compares by value (`125.00` is `125`); `zero`..`twenty` and the
tens are digits; every other non-alphanumeric character separates words. No Unicode
normalisation or accent folding: it would need Unicode tables the standard library does
not have, that is a new dependency; precomposed and decomposed spellings therefore
differ. The T-04 scorer in the infrastructure tests stays an independent oracle and
does not call this code.

A query is at most 256 bytes and 1..16 words; control characters (including tab and
newline) or nothing left after normalisation are rejected with typed reasons `empty`,
`too_long`, `too_many_terms`, `control_character`.

### 3. Matching and ranking

Within one segment: **phrase** (tier 1) when the query words joined without spaces
equal a run of consecutive segment words joined without spaces (`AB 731` finds
`AB-731`, `dialog r 17` finds `Dialog R-17`, `407` never finds `4407`); otherwise
**all terms** (tier 2) when every query word is a segment word. A phrase that runs from
one segment into the next is not found (documented limit). Ranking is tier, then
segment start, then ordinal; ordinals follow start order, so the rank is (tier,
ordinal).

### 4. Computed on demand, no persisted index

Search reads the immutable revision and matches it on every request. Measured (S-11,
Windows 11, Xeon E5-2698 v4, optimised build): a 20,000-segment revision, the import
bound, pages completely at limits 1, 20 and 100 with p95 page times of 154, 167 and
145 ms, against 142 ms for a `transcript get` page of the same record. Reading and
verifying the stored record dominates; matching adds about 10-25 ms. That is inside the
250 ms warm-page target, so no index is persisted. An index would be new persistent
state with its own integrity and lifecycle rules; it is reconsidered only if a
measurement misses the target.

### 5. Cursors

`page_search` reuses the `CursorToken`/`QueryDigest` pattern of `transcript get`: the
digest covers `vsift.search.v1`, the revision identity, the range (or none) and the
normalised words, so two spellings that normalise alike are one search; the snapshot is
the revision number; the last-item key is `<tier>-<ordinal>`; the expiry is the session
expiry at issue. Any other query, range, session or revision, an expired cursor or a
forged key is `INVALID_ARGUMENT`, never a silent restart. The page size may change
between pages.

### 6. Result and evidence stream

`--json` returns `items` (the matching segments as the published `transcript_segment`
records, exactly as `transcript get` returns them) and a parallel `hits` list
(`segment_id`, `match`). `--events jsonl` streams those same records as evidence events
in rank order, then one terminal event whose data carries `hits`, `record_count`,
coverage and the cursor. Search creates no new record type: an indexer upserts records
it may already hold, and a later search never contradicts a transcript read.

### 7. Honest coverage

Every result states, in `data.transcript_coverage`, its `basis`, `scope:
"transcript_text"` (on-screen text is never searched), the `searched_range` (the
request clipped to the source) and its `transcribed_ranges`, `untranscribed_ranges`
and `no_speech_ranges` (each at most 100, `ranges_truncated` otherwise):

- `supplied_transcript`: a supplied file is taken to cover the whole source; its
  completeness is not verified.
- `local_asr`: every chunk window a run examined is covered (transcribed, silent or
  without audio); silent and audio-less windows no transcribed window overlaps are no
  speech. A spliced revision adds, outside its replaced range, the coverage of each
  revision whose segments it carries. A run whose segments were all replaced is no
  longer recorded in the revision, so its ranges count as untranscribed: coverage can be
  understated, never overstated.
- `mixed`: local ASR spliced into supplied text (added beyond the two bases first
  proposed, because such a revision is neither).

The envelope `coverage` (frozen in v1) is `truncated: true` exactly when part of the
searched range is untranscribed, `gaps` lists those ranges as `<from_us>-<to_us>`
(merged, at most 100) and `reasons` holds distinct identifiers (`untranscribed_range`,
and `gap_list_truncated` beyond 100 gaps). Such a result has status `partial`, a fixed
warning, and exit 0, as the v1 exit table promises for a supported partial result.

### 8. Failures

Existing codes only. `INVALID_ARGUMENT` for a rejected query (fixed remediation naming
the reason), an empty range, an unknown revision (the existing revision remediation), a
session without a transcript (the existing remediation naming `transcript
retranscribe` and `ingest --transcript`), a closed or expired session, and a rejected
cursor; page sizes 0 and 101 and a lone `--from`/`--to` are parse errors, like
`transcript get`. A stored record that fails verification is `INTEGRITY_FAILURE` or
`UNSUPPORTED_SCHEMA` as for every read.

## Decision: visual-candidate index (as built by P08 PR 3 and PR 4)

### 9. Command and request

- `candidates <session> --from <us> --to <us> [--limit 1..100] [--cursor <token>]`
  pages the candidates whose representative time lies in the half-open range, 20 per
  page by default. A range that runs past the end of the video is clipped to it (the
  clipped range is reported as `coverage.searched_range`); one that starts at or after
  the end is `INVALID_ARGUMENT`. The engine operation is `Engine::candidates`; the CLI
  only presents it.
- **Built inside `candidates`.** A call first analyses the missing fixed 60 s windows
  of its range in ascending order, at most 30 (30 minutes of media) per call; the rest
  is reported as `not_analyzed` coverage and a later call continues it. A host may lower
  the budget (`EnginePorts::with_visual_window_budget`, clamped to 30).
- **Warm reads.** When every window of the range is already recorded, or the call
  carries a cursor, the page is read from the newest committed index without resolving
  a tool, running a provider or writing anything. A cursor call never analyses, so the
  pages it continues cannot change under it.
- **Order of steps on a cold call:** validate; read the newest index (the session must
  be open); resolve `FFmpeg`/`FFprobe` and run the media-tool preflight (profile 2,
  which includes the `visual_sampling` check); bind the committed copy (hashed once,
  compared by on-disk identity before every provider call, ADR 0012 note of
  2026-09-26); probe; select the first video stream whose codec the adapter decodes;
  analyse; verify the copy's full hash again; commit one new index revision; page.

### 10. Sampling and analysis (profile `r0-visual-v1`)

- Each window is decoded as one bounded `FFmpeg` run with a closed argument list: a
  `select` expression keeps actual decoded frames at least 0.5 s apart (2 Hz) from a
  lead-in 0.5 s before the window to its end, each frame keeps its own timestamp
  (`-fps_mode passthrough`), and is downscaled to 128x72 grey; `showinfo` reports the
  times, which are parsed and normalised, never inferred from the request. At most
  122 frames, 1.1 MiB of pixels, 256 KiB of diagnostics and 120 s per window.
- **Deviation from the first design (PR 3):** the input `-ss`/`-t` are given in
  normalised time with a 1 s margin (seek 1 s before the lead-in, read 1 s past the
  window end), not as the exact window in raw stream time: `FFmpeg` interprets an input
  `-ss` relative to the container's start time, which need not equal the probed origin
  (the earliest stream start, 2 s for F09), and `-t` counts from the seek point. The
  exact bounds are left to the `select` expression, which compares raw stream
  timestamps; the margin costs little decoding.
- A sample keeps only its 16x9 grid of 8x8 block means and a 64-bit difference hash.
- **Change rule (deviation, PR 3):** a block counts as changed when its mean moves by
  at least 4 grey levels; a change of screen is two such blocks, or any single block
  moving by at least 6. The proposal (two blocks at 6, or one at 12) missed F04's
  order 1001-to-1017 edit, which moves one block by 6-7. Unchanged consecutive samples
  of the synthetic corpus differ by at most 1 level. Real recordings with heavier
  compression noise are not measured.
- Each sample is compared with the previous sample and with the first sample of the
  current screen state, so slow drift is caught. Unchanged samples merge into one
  candidate whose `span` runs to the next candidate or the window end, with its
  `sample_count`. A screen seen in one sample only is `transient`; three or more
  consecutive changing samples are one `motion_start` (`in_motion`) followed by
  `settled_after_motion`. Every 10 s cell with a decoded frame has a candidate
  (`periodic_coverage` when nothing changed in it), so a static screen is represented.
  A screen that reappears later is a separate candidate with the same `visual_hash`.
- At most 32 candidates per window: the first and the coverage candidates are always
  kept, then the largest changes; the window records how many were dropped
  (`candidate_budget_exhausted`).
- Windows never merge. A candidate's identity derives from the session, source,
  stream, profile, window ordinal and representative time, so it never changes when
  later windows are analysed, and an analysed window is never analysed again.

### 11. Storage and commits

- Each call that records anything commits one `visual_index_record` artifact: the
  whole new revision (every earlier window unchanged plus the new ones, the stream's
  displayed dimensions and the probed duration), strict versioned JSON of at most
  8 MiB, at most 64 per session; only the newest is read. Retained bundles carry every
  record and `bundle validate` re-derives the window grid, spans, sample counts, change
  rule, cell coverage and identities (ADR 0013 note of 2026-09-26). The shape is
  published as `bundle-visual-index-record.schema.json`.
- Concurrent calls: a commit that finds the generation moved re-reads the newest
  revision and merges its own recorded windows onto it; windows are pure functions of
  the source, stream and profile, so a window both calls analysed is identical and the
  union is safe. At most three attempts, then `BUSY`.
- A window the provider rejects is recorded once as `undecodable` and never decoded
  again. A deadline, busy admission or cancellation stops the call, leaves that window
  and the rest unrecorded and retryable, and still commits what was analysed before.

### 12. Result, coverage, cursors and stream

- `data` (`candidates-data.schema.json`): `session_id`, the requested `range`, `index`
  (`index_id`, revision `number`, `profile`, `duration_us`, and the fixed
  `window_us`, `sample_interval_us`, `coverage_interval_us`), `coverage`
  (`searched_range`, merged `analyzed` ranges, typed `gaps` with `reason` and
  `dropped_candidates`, `ranges_truncated`), `items` and `next_cursor`.
- Each item is a published `visual_candidate` evidence record
  (`visual-candidate.schema.json`): `candidate_id` (`vcd_`), `source_id`,
  `source_segment_id`, `stream_index`, `window`, `representative_us` (the actual
  decoded frame's time, which P09 `frame get` extracts), `span`, `change_window`
  (the change happened after its `from_us` and at or before its `to_us`), `reasons`,
  `stability`, `change` (`changed_blocks`, `max_block_delta`: uncalibrated integers
  for ordering, never a confidence), `visual_hash`, `sample_count`,
  `displayed_dimensions` and `analysis` (`r0-visual-v1`, 128x72). **No thumbnails in
  P08**; images are P09.
- Gap taxonomy: `not_analyzed`, `deadline_exceeded` (the window this call timed out
  on), `undecodable`, `no_decoded_frame` (a 10 s cell of an analysed window without a
  frame), `candidate_budget_exhausted`. The envelope `coverage` is filled as for
  search: `truncated` exactly when the range has a gap, `gaps` merged across reasons as
  `<from_us>-<to_us>` (at most 100), `reasons` the distinct gap identifiers in taxonomy
  order (plus `gap_list_truncated`); such a result is `partial` with a fixed warning
  and exits 0.
- Cursors reuse `CursorToken`: the digest covers `vsift.candidates.v1`, the source,
  stream, profile, range and the state of every window the range touches (not
  analysed, undecodable, or its candidate identities); the last-item key is
  `<window>-<representative_us>`; the expiry is the session expiry at issue. Analysing
  windows outside the range leaves a cursor valid; any change inside it rejects the
  cursor (`INVALID_ARGUMENT`) rather than skipping or repeating a candidate.
- `--events jsonl`: one `visual_candidate` evidence event per item (key
  `candidate_id`), then one terminal event whose data is the page without items plus
  `record_count` (`candidates-stream-data.schema.json`).

### 13. Failures

Existing codes only. `INVALID_ARGUMENT`: an empty or reversed range, a range starting
at or after the video's end, a rejected cursor, a cursor for a session without an
index (fixed remediation), a video without a video stream (fixed remediation), a
closed or expired session; `--limit 0`/`101` and a missing `--from`/`--to` are parse
errors. `INVALID_SOURCE`: no decodable video stream, or a probe that rejects the copy.
`MISSING_CAPABILITY`: `FFmpeg`/`FFprobe` missing (only when windows must be analysed;
fixed remediation), a failed preflight, or a provider that cannot run on a window.
`DEADLINE_EXCEEDED`, `BUSY`, `CANCELLED`: only when the call analysed nothing new and
nothing of the range was indexed before; otherwise the result is partial with gaps.
`RESOURCE_LIMIT`: a record over 8 MiB or a 65th index record. `INTEGRITY_FAILURE` /
`UNSUPPORTED_SCHEMA`: a stored record that fails verification, a copy that changed
during the call, or an index that no longer describes the probed source.

### 14. Measured recall and cost

Recorded `FFmpeg` 9.0 samples of F01-F10 and F12 (always-run gate) and the same through
the binary (opt-in `p08_candidates_e2e`): every `stable` event of at least 1 s is hit;
F06's 500 ms tooltip is hit as `transient`; static F01 and F07 have no change
candidate; F04-E02, F05-E02 and F12-E02 are corpus limitations drawn with the pixels
of the state before them (issue #159), reported (F12-E02 is gated and hit by periodic
coverage); 0 false changes; 12.9 candidates per minute; median timestamp error 0.
Analysis ran at 29 media seconds per second on 1440x900 20 fps windows and 67 on a
30-minute 640x360 10 fps session (30 windows in 26.9 s). Warm pages take p95 146-152
ms through the binary on that session, and 101-103 ms in the engine on the largest
index a session can hold (four hours, every window at its 32-candidate budget, a
2.2 MB record). Details, per-event
tables and the V-03 motion clips are in the
[P08 candidate recall record](../planning/p08-candidate-recall.md).

## Decisions for maintainer confirmation

The maintainer confirmed every decision below on 2026-09-26, together with those
added by P08 PR 3 and PR 4.

1. The visual index is built inside `candidates`, at most 30 minutes of media per call,
   the rest reported as `not_analyzed`.
2. Sampling is 2 Hz of actual frames with a candidate at least every 10 s.
3. The recall gate covers `stable` events of at least 1 s; other events are reported;
   F04-E02, F05-E02 and F12-E02 are corpus limitations and issue #159 tracks
   regenerating the motion fixtures.
4. Issue #148 (each speech chunk rehashes the whole source copy) is fixed in P08 by
   bracketed source binding (P08 PR 2).
5. `search --events jsonl` streams the existing `transcript_segment` records followed by
   a terminal event with the hit list; no new record type.
6. No thumbnails in P08; images belong to P09.

Also confirm the additions made while building: search's third coverage basis
`mixed` and a supplied transcript taken to cover the whole source (PR 1); the lowered
change thresholds and the normalised, margined `-ss`/`-t` (PR 3, decision 10); and
for `candidates` (PR 4) a range past the video's end clipped rather than rejected, the
index carrying the stream's displayed dimensions for warm reads, a host-lowerable
per-call window budget, and a commit race merged at most three times before `BUSY`.

## Consequences

- Agents can find speech by what was said and are told plainly which parts of the video
  no transcript covers, so "no hit" is never mistaken for "not said".
- The public contract grows by two commands (`search`, `candidates`), one evidence
  record type (`visual_candidate`), the data schemas `search-data`,
  `search-stream-data`, `candidates-data`, `candidates-stream-data`,
  `visual-candidate` and the storage schema `bundle-visual-index-record`, with
  frozen examples; no failure code or event kind is added.
- A visual index is new persistent session state: at most 64 records of at most 8 MiB,
  strictly validated on every read and in bundles, removed with the session.
- Candidates are an agent's shortlist, not evidence: sampling at 2 Hz can miss a
  change shorter than 0.5 s or one smaller than the change rule, and every result says
  which parts of its range were not analysed or could not be.
- Search cost grows with the revision size on every request. The bound (20,000
  segments) keeps it within the page target today; an index is a later, measured
  decision.
- Accent and Unicode-form differences are not folded, and cross-segment phrases are not
  found; both are documented limits.
