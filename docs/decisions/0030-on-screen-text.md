# ADR 0030: On-screen text (OCR)

- Status: Accepted (the maintainer, 2026-10-21). Decisions 1 to 6 are the maintainer's, after the
  engine evaluation of [`ocr-engine-evaluation.md`](../planning/ocr-engine-evaluation.md);
  decisions 7 to 12 were proposed by the OCR work and accepted by the maintainer the same day as
  recommended, the default period of 10 s included; decision 13 (`ocr search`) is the maintainer's
  choice of the same day, after the first hosted run showed that text read off a recording without a
  transcript could not be searched.
- Date: 2026-10-21
- Release: 0.3.0
- Refines: [ADR 0003](0003-external-runtime-adapters.md) (an engine is an external
  adapter), [ADR 0007](0007-managed-runtime-provisioning.md) (managed components),
  [ADR 0018](0018-visual-candidate-index-and-transcript-search.md) (search and the visual
  candidates) and [ADR 0019](0019-evidence-navigation.md) (exact frames)
- Register: L-202 to L-204 in [`known-limits.md`](../planning/known-limits.md)

## Context

A screen recording often says more on the screen than in speech: an invoice number, an error
dialog, a status line, a value that changes. Until 0.3.0 VSift could find only what was said;
an agent had to look at frames one by one to find what was shown. On-screen text must become
searchable evidence without breaking what makes VSift's evidence trustworthy: every claim cites
the exact frame and place it came from, coverage says what was not read, nothing runs without
the user asking, and an engine is an adapter, never a dependency of the domain.

The engine evaluation (step 1, hosted workflow only) compared Tesseract 5.5.3, PP-OCR (PaddleOCR's
models on ONNX Runtime) and ocrs on synthetic screens with exact truth and on real recordings
(RM01, RM04, RM10), all read from frames VSift itself extracted. PP-OCRv5 mobile with the Latin
recognizer read 99% of the synthetic lines exactly, against 74% for Tesseract and 92% for
Tesseract on frames enlarged twice, at about half a second a frame on four threads.

## Decisions

### 1. Engine: PP-OCRv5 mobile, Latin, on ONNX Runtime, through VSift's own runner (maintainer)

The engine is PP-OCRv5 mobile detection with the Latin-script recognizer, run on Microsoft's ONNX
Runtime by `vsift-ocr-runner`, a small Rust program in this workspace
(`crates/vsift-ocr-runner`). The supervisor runs it as an external tool; it is never linked into
the engine crates. `unsafe` stays forbidden in it. It reaches ONNX Runtime through the `ort`
wrapper crate only if that crate passes the dependency review of `AGENTS.md`; otherwise another
way is proposed. The runner takes image paths and writes bounded JSON Lines: per line a box,
text and confidence.

The runner's pipeline (detection resize, normalisation, the DB post-process, reading order, line
crops, recognition one line at a time and CTC decoding) is pure Rust over an `Inference` port, so all of it
is tested without the runtime; only the two model calls go through ONNX Runtime.

### 2. Publishing (maintainer)

VSift-built engine binaries are published on this repository's GitHub releases under their own
tags (for example `ocr-runner-v0.1.0`), built and attested by a protected workflow like
`release.yml`, and never marked latest. The engine release workflow and the cross-platform
managed-install system are owned by the infrastructure work; the OCR work supplies the runner
crate, its build recipe and the catalogue entries' data (runner, ONNX Runtime and the two models
as components).

### 3. Platforms (maintainer)

Managed installs on Windows, macOS and Linux. Until they exist, a user registers a runner with
`setup configure ocr-runner --executable <path>`, gives one with `ocr --runner`, or puts
`vsift-ocr-runner` on `PATH`.

### 4. Frames: the candidates plus one frame every N seconds (maintainer; N = 10 s, accepted with decision 8)

The plan of a range is the representative frame of every visual candidate in it and one frame
every `--period-us`. **The default period is 10 s** (1 s to 10 min): the same grid the visual
index guarantees a candidate on, so on a static screen it adds little, while a value that changes
inside a screen that looks the same to the candidate rule (a counter, a clock, a progress figure)
is still read at least every 10 s. The period is settable per call. Coverage counts every planned
read as read, failed, unresolved or pending.

### 5. Opt-in, explicit, bounded and resumable; Latin first (maintainer)

`ocr <session>` is a command of its own. Nothing runs it implicitly, and nothing else needs the
runner. A call reads at most `--max-frames` frames (1 to 200, default 40) within 300 s, commits
one index revision and stops; calling it again for the same range continues with what is
pending. Only Latin script is read; other scripts are later work (L-202).

### 6. Search covers on-screen text, additively (maintainer)

`search` finds on-screen text with its frame, time and region. The v1 contract changes only
additively: a new command, new schemas and an optional member of the search results.

### 7. The screen-text index (maintainer, as proposed)

Each `ocr` call that attempts a frame commits one immutable `screen_text_index_record` artifact:
a revision holding every frame of the one before it. A frame is identified exactly (stream, time
base, timestamp, time, displayed size), lists the planned reads that resolved to it, and has an
outcome: `read` with its lines (identity, upright region inside the frame, text, the engine's
uncalibrated score in basis points, and whether the runner cut anything), `image_rejected` or
`engine_failed`. Reads that resolve to no frame are kept as `unresolved`. Every identity is
derived from the session, source, stream, profile, engine build and frame, so a record is checked
again when it is read and in `bundle validate`. The record holds no pixels. Bounds: 4,096 frames,
256 lines a frame, 200,000 lines, 512 characters a line, 48 MiB a record, 64 records a session.

**One engine build per index.** The engine build is the runner's SHA-256 and version, the runtime
version and the two model digests. A call by another build starts a new index; text read by two
builds is never mixed, and only the newest index is searched. A frame a build failed on is not
read again by that build.

### 8. How a call reads (maintainer, as proposed)

The order is part of the contract: validate; resolve the runner (no runner is
`MISSING_CAPABILITY` before any work); list the range's candidates through `candidates`, which
analyses missing windows as it does for itself; plan; prepare the media tools and the session's
copy as the evidence commands do; resolve each pending read to the first displayed frame at or
after its time (a candidate's exactly to its own frame); extract the frames as PNG into the
session's private work folder; run the runner in batches of at most 8 images; compare the copy's
identity a last time; commit, merging onto a concurrent call's revision at most three times. A
call that stopped after reading something keeps it and is `partial`; one that read nothing and
stopped fails and commits nothing.

The runner is run with a closed argument list, an empty environment, its own folder as its
directory (the frames are handed over by absolute path in the session's private work folder), the
threads it was given (taken from the session root's admission capacity) and the supervisor's
deadline and kill. Its output is at most 4 MiB, checked strictly: protocol 1, one `image` record
per image given, lines inside their image. Anything else is the runner's failure, never data.

### 9. The `ocr` result (maintainer, as proposed)

`screen-text-data.schema.json`: the range read, the newest index (identity, revision, engine,
counts), the frames this call attempted, why it stopped, and the range's coverage (scope
`screen_text`: planned, read, failed, unresolved, pending, the period and whether the candidates
are all known), with the visual index's gaps. The envelope `coverage` is `truncated` and the
status `partial` with a fixed warning exactly while a read is pending or failed or the candidates
are not all known; its reasons are `frames_pending`, `frames_failed` and the candidate gap reasons.

### 10. Search results (maintainer, as proposed)

A search first page carries `screen_text`: the query's occurrences in the newest index within the
searched range, matched by the transcript's normalisation and rule, at most the page's limit, with
the total. A line read on consecutive read frames with the same words in the same place (at least
half of the smaller region overlapping) is one occurrence, cited by its first frame and region,
with the time of the last such frame and the number of frames. `state` is `not_run`, `partial` or
`complete`, with the same coverage block.

The member is on the first page only and never changes the search's status or envelope coverage.
Two reasons: the transcript's cursor, status and coverage are published and pinned since 0.1.0,
and an interleaved ranking of spoken and shown text would need a cursor over two indexes that can
change independently. The page limit bounds the hits, and `total_hits` says when there are more;
`--from` and `--to` narrow them.

### 11. Skill and help (maintainer, as proposed)

The skill's command reference names `ocr` in the `explicit` class: an agent runs it when the user
asks for text on the screen, not on its own. This is the one skill change, so the 0.3.0 cut needs a
new trial freeze.

### 12. Older releases (maintainer, as proposed)

A release before 0.3.0 does not know the new artifact kind and refuses a session or bundle that
holds one (`UNSUPPORTED_SCHEMA` or `INTEGRITY_FAILURE` from its strict reader); a session without
on-screen text is unchanged. This is recorded as L-203 rather than avoided, because silently
skipping an unknown artifact would break the strict-bundle rule.

### 13. `ocr search`: on-screen text is searched with or without a transcript (maintainer)

`search` refuses a session without a transcript (since P08), and its `screen_text` member rides on
its first page, so text `ocr` read off a silent recording could not be found at all. Making search's
transcript members optional would not be additive. So `vsift ocr search <session> --query <text>
[--from --to] [--limit] [--cursor]` (`ocr-search-data.schema.json`) pages every on-screen
occurrence of one session, with or without a transcript, by the same matching and grouping as
decision 10, with `total_hits`, the `state` and coverage block, and a cursor of its own bound to the
session, the index revision (a new `ocr` revision ends it), the range, the normalised query and the
session's expiry. Its envelope is `partial` with a fixed warning while the state is not `complete`
(reasons `not_run`, or `frames_pending`, `frames_failed`, `candidates_incomplete`). `search` and its
first-page member stay as they are. The use case (`page_screen_text` in `vsift-application`) reads one
session's index and nothing else, so the search across kept sessions planned for 0.3.0's second wave
calls it once per session.

**Note of 2026-10-30 (ADR 0035).** The search across kept sessions does not call `page_screen_text`
once per session, as this decision expected: that use case pages with a cursor of its own for one
session, while a search across bundles ranks every hit together under one cursor. It calls the domain
function beneath it, `screen_text_occurrences`, so matching and grouping stay in one place.

## Note of 2026-10-21: a call bounded to analysed windows (for ADR 0033)

ADR 0033's background `prepare` job runs OCR as one of its streams after its visual stream has
analysed the windows, and must never start visual analysis from OCR. The engine's `ocr` operation
takes `ScreenTextWindows`: `AnalyseMissing` (what `vsift ocr` does, the default) or `AnalysedOnly`,
which plans only the candidates and periodic reads inside windows the visual index has recorded
(`plan_analysed_windows` in `vsift-application`, per session) and fails with `WindowsNotAnalysed`
(`INVALID_ARGUMENT`) while no window is. The periodic reads of the other windows are counted apart
in the coverage block's `awaiting_analysis`, which keeps coverage incomplete (envelope reason
`frames_awaiting_analysis`); they join the plan once their windows are analysed. Calls stay bounded
(40 frames by default) and resumable.

## Note of 2026-10-21: one line at a time, and the parity measured (decision 1)

The reference pipeline reads lines in batches of six, each padded to the widest; the hosted
characterisation (L-204) showed that this padding, like cutting a slightly rotated rectangle instead
of an upright one, changes 5 to 7% of real-recording lines in the reference itself. The runner now
reads each line alone, so a line's text never depends on its neighbours; on identical crops it writes
the reference recogniser's text for 98% of real-recording lines, it is about twice as fast, and its
synthetic accuracy is unchanged.

## Note of 2026-10-21: the `ort` dependency review (decision 1)

The coordinator gave the download go for `ort` and `ort-sys` 2.0.0-rc.13 and `libloading` 0.9.0 on
2026-10-21 (rc.15 was refused: under two weeks old). The review under `AGENTS.md`:

- **Need:** the runner must call ONNX Runtime's C API; writing that binding ourselves would need
  `unsafe`, which this crate forbids. `ort` holds the raw calls; the runner uses only its safe API
  (`init_from`, the session builder, `run`, `try_extract_tensor`, the model metadata). The runner
  keeps `#![forbid(unsafe_code)]`, and the governance check holds it.
- **Features:** `default-features = false, features = ["load-dynamic"]`: no `download-binaries`
  (the build never downloads ONNX Runtime), no TLS, no `ndarray`, no `tracing`; the library is
  loaded at run time from the runner's own `onnxruntime` folder by absolute path. Compiled graph:
  `ort`, `ort-sys` (no build-time dependency enabled), `libloading` 0.9.0 and the already-locked
  `smallvec`, `cfg-if` and `windows-link`. Only `vsift-ocr-runner` depends on them; no engine crate.
- **Licences:** `ort` and `ort-sys` MIT OR Apache-2.0; `libloading` ISC. `cargo deny check`
  (advisories from the local database of 2026-10-20, bans, licences, sources) passes.
- **Lock-only entries:** `ort`'s `std` feature names `ndarray?/std`, so Cargo records `ndarray`
  0.17.2, `matrixmultiply` 0.3.11, `rawpointer` 0.2.1, `portable-atomic` 1.15.0 and
  `portable-atomic-util` 0.2.8 in `Cargo.lock` although nothing enables them (`cargo tree` shows
  none compiled). All are MIT OR Apache-2.0 and pass `cargo deny`.
- **Maintenance:** `ort` (pykeio) released rc.13 on 2026-07-28, rc.14 and rc.15 since; about 9
  million downloads in the last 90 days. `libloading` (nagisa) 0.9.0 of 2025-11-05, a foundation
  crate of the ecosystem. rc.13 needs ONNX Runtime 1.17 or newer and accepts 1.31.0.
- **What `ort` can still do that we guard:** it panics if its API is used before a library is
  loaded, so the runner loads the library first and turns every refusal into a typed failure; a
  panic that remains ends the runner's process, which VSift reports as the runner's failure.

## Note of 2026-10-22: the review's fixes (RV-2), before the first merge

A focused review of the draft found one high and four medium defects; each is fixed with a test
that fails without it. Protocol 1 has not shipped, so it is changed in place, not versioned.

- **Decision 1, a bounded detection map.** A frame of an extreme aspect ratio (128 × 125,000
  passes the 16-megapixel bound) was enlarged to a map of about 500 million pixels. The
  enlargement is now capped so the map stays within `max_side`² (4 M pixels) before its sides are
  rounded, and a map over 8 Mi pixels is refused as `image_invalid` before anything is reserved.
  The cap was preferred over refusing such frames or capping the longer side, because it leaves
  the measured pipeline's map unchanged for every aspect ratio up to about 7.4 to 1.
- **Decision 7, one build per index proven at every read.** `describe` and every `read` hash the
  runtime library and both models before loading them; `describe` names the library's digest, and
  `read`'s greeting names the runner version, the runtime version and the three digests. Before
  each batch VSift hashes the executable again; a batch whose executable or greeting is not the
  build `describe` reported is refused as `build_changed` (`INTEGRITY_FAILURE` when nothing was
  read) and nothing of it is kept; batches before it, each proven by its own greeting, are kept.
- **Decision 5, the 300 s bound holds.** Every media and runner process of a call gets the lesser
  of its own deadline and the time the call has left, and none starts once nothing is left; the
  call then stops as `deadline_exceeded`. Only the final commit follows the bound.
- **Decision 7, a full index costs nothing.** Before anything is read, the frame budget is clipped
  to the room the index has left: its 4,096 frames, and its 200,000 lines counted as if each new
  frame held its 256. With no room the call stops as `index_full` (additive in the result) without
  reading a frame, and a session that already holds 64 revisions fails with `RESOURCE_LIMIT`
  before the runner is started. The plan-size ceiling this leaves is recorded in L-202.
- **Decision 10, search never fails because of on-screen text.** The first page reads the
  screen-text index first, and the visual index only when there is a screen-text index. An index
  that cannot be read is named in the member (`unavailable`: which index, and `integrity_failure`,
  `unsupported_version` or `unreadable`), with the new state `unavailable` when it is the
  screen-text index; the search's status, coverage and transcript hits are unchanged.
- **Decision 8, the wording.** The runner's directory is its own folder, as the code always did;
  the decision now says so.
- **The managed store's flat folder (the planner's decision of 2026-10-22).** The runner finds the
  runtime and the models in `onnxruntime/` and `models/`, or flat beside its executable, as the
  managed store keeps a component's files; every basename is unique. The digests are of the files
  the run loads, in either layout.

## Consequences

- An agent can find what was shown as it finds what was said, and cite the exact frame, time and
  region; it can see from coverage what has not been read.
- VSift ships and maintains a small native runner and depends on ONNX Runtime's C library at run
  time, outside the engine. The runner's pre- and post-processing must match the reference
  pipeline's; that parity is measured in the hosted workflow before the runner is published.
- Text that appears and goes between two planned frames is not read, and only Latin script is read
  (L-202); a shorter period reads more at a higher cost.
- Until the runner passes its hosted parity run it is not published, and until the managed install
  exists a user registers a runner they built (L-204).

## Alternatives considered

- **Tesseract** as an external tool: simpler to ship, but it read small interface text clearly
  worse in the evaluation, even on enlarged frames.
- **ocrs**: pure Rust, but it gives no confidence, reads Latin letters without diacritics and its
  models are CC BY-SA 4.0 from mutable URLs.
- **Linking ONNX Runtime into `vsift`**: rejected by decision 1; a native library crash would end
  the supervisor, and the engine stays provider-neutral.
- **Reading every frame or a fixed fine grid**: unbounded cost on long recordings for little gain
  over candidates plus a period.
- **Interleaving on-screen hits with transcript hits in one ranked page**: see decision 10.
