# ADR 0018: Visual-candidate index and transcript search

- Status: Proposed (maintainer review before merge of P08 PR 1)
- Date: 2026-09-26
- Tracking: [P08 / issue #11](https://github.com/smormah/vsift/issues/11)
- Refines: [ADR 0008](0008-cli-and-json-contract.md) (the reserved `search` and
  `candidates` commands and the envelope `coverage` member),
  [ADR 0016](0016-embeddable-engine-and-evidence-contract.md) (the evidence stream) and
  [ADR 0017](0017-local-asr-through-whisper-cpp.md) (revisions and citations)
- Scope of this record: the transcript-search half is decided in full and implemented
  by P08 PR 1. The visual-candidate half is the planned design, completed and
  amended by P08 PR 3 (visual index core) and PR 4 (`candidates` command).

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

## Decision: visual-candidate index (planned design, P08 PR 3 and PR 4)

- **Built inside `candidates`.** `candidates <session> --from --to` analyses the
  missing fixed 60 s windows of the requested range (pure windows, aligned to the
  source timeline), at most 30 minutes of media per call; the remainder is reported as
  `not_analyzed` coverage rather than silently skipped, and a later call continues it.
- **Sampling.** 2 Hz sampling of actual decoded frames, with a candidate at least every
  10 s, so a static screen is still represented.
- **Merging.** Adjacent similar samples merge with their time span preserved; each
  candidate carries stability `transient`, `in_motion` or `settled`; a screen repeated
  at different times stays separate candidates that share a visual hash.
- **Storage.** Batched visual-index records are a new session artifact kind, committed
  like transcript records and carried into retained bundles.
- **Coverage.** A typed gap taxonomy (not analysed, budget, decode failure, and so on)
  fills the envelope `coverage` the same way search does.
- **No thumbnails in P08.** Frame images are P09 (`frame get`).
- **Recall gate.** Measured over the manifest's `stable` events of at least 1 s; other
  events are reported, not gated. F04-E02 and F05-E02 are reported as corpus
  limitations, with an issue to regenerate the motion fixtures.

## Decisions for maintainer confirmation

1. The visual index is built inside `candidates`, at most 30 minutes of media per call,
   the rest reported as `not_analyzed`.
2. Sampling is 2 Hz of actual frames with a candidate at least every 10 s.
3. The recall gate covers `stable` events of at least 1 s; other events are reported;
   an issue tracks regenerating the motion fixtures (F04-E02, F05-E02).
4. Issue #148 (each speech chunk rehashes the whole source copy) is fixed in P08 by
   bracketed source binding (P08 PR 2).
5. `search --events jsonl` streams the existing `transcript_segment` records followed by
   a terminal event with the hit list; no new record type.
6. No thumbnails in P08; images belong to P09.

## Consequences

- Agents can find speech by what was said and are told plainly which parts of the video
  no transcript covers, so "no hit" is never mistaken for "not said".
- The public contract grows by one command, two data schemas and two frozen examples;
  no failure code, event kind or record type is added.
- Search cost grows with the revision size on every request. The bound (20,000
  segments) keeps it within the page target today; an index is a later, measured
  decision.
- Accent and Unicode-form differences are not folded, and cross-segment phrases are not
  found; both are documented limits.
