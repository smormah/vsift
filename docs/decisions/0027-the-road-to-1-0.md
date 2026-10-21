# ADR 0027: The road to 1.0: a 0.3.0 that matches and goes beyond the existing tool, then 1.0.0

- Status: **Accepted, 2026-10-21**, by the maintainer's decisions of that day, recorded in
  [`backlog-iterations.md`](../planning/backlog-iterations.md) ("The road to 1.0").
- Date: 2026-10-21
- Refines: [ADR 0011](0011-r1-industrial-capability-expansion.md) (R1's contents and order) and the
  delivery model of 2026-10-16 ([`delivery-governance.md`](../planning/delivery-governance.md), D1).
  ADR 0011 stays as it was accepted; this ADR moves part of what it assigned to R1 earlier and puts R1
  after `1.0.0`. It does not change R1's invariants, which still bind everything built here.
- Tracking: [#406](https://github.com/smormah/vsift/issues/406), [#407](https://github.com/smormah/vsift/issues/407),
  [#409](https://github.com/smormah/vsift/issues/409), [#474](https://github.com/smormah/vsift/issues/474),
  [#188](https://github.com/smormah/vsift/issues/188), [#176](https://github.com/smormah/vsift/issues/176),
  [#351](https://github.com/smormah/vsift/issues/351), [#272](https://github.com/smormah/vsift/issues/272).

## Context

Until 2026-10-21 the plan was: finish cycle 2 as `0.2.3`, prepare the launch in cycle 3, and start
R1 (packets P15 to P20, which add OCR, speaker labels, a managed catalogue, stitching and an
operated worker plane) only on the maintainer's word, with no date for `1.0.0`. Three facts made
that order wrong.

1. **VSift is not alone.** Other tools already let a coding agent investigate a video. The maintainer
   chose one of them as the reference (unnamed here: public documents name no other tools). Four capabilities that the
   maintainer wants VSift to match (on-screen text, speaker labels, links to online videos, and the
   time of day on results) are missing from VSift today. VSift's strengths (source-grounded
   evidence, exact frames and crops, honest coverage, a stable CLI contract, local-first operation)
   do not help a user whose first question needs one of the missing capabilities.
2. **People should be using VSift well before `1.0.0`.** The defects of `0.2.0` that mattered most were
   found by real use, not by the synthetic corpus ([L-022](../planning/known-limits.md#l-022)). A `1.0.0`
   that arrives together with its first public users would be a promise made blind. The word should go out with a
   release worth announcing, and `1.0.0` should follow some weeks of real use.
3. **R1 as written is too large to wait for.** Most of what users of the existing tools ask for (OCR,
   speakers, links) sits in P16 and in the online-sources issue #406, which ADR 0011 and the decision of
   2026-10-16 left inside R1, behind a catalogue, a worker plane and a P15 decision packet that none of
   those features needs.

## Decision

1. **`0.2.3` is cut at midnight on Friday 23 October 2026 (local time) with all of cycle 2**, the
   monthly round as planned, and published around 26 to 27 October. It does not contain the FFmpeg
   re-pin, because the build that the re-pin needs is the month-end build of 2026-10-31.
2. **The FFmpeg update ([#351](https://github.com/smormah/vsift/issues/351),
   [#272](https://github.com/smormah/vsift/issues/272)) ships as `0.2.4`, a hot fix from
   `release/0.2.x`** in early November, under the hot-fix procedure (the full three-system check,
   the publish, then the quick checks of the published bytes).
3. **`0.3.0` is a normal stable release**, cut around Friday 27 November and published around
   30 November or 1 December. Its scope is:
   - **Matching the reference tool:** OCR (on-screen text, searchable together with the transcript);
     speaker labels; online links (direct HTTPS video links, #407, and public pages through an optional
     yt-dlp component, #409); and **time anchoring**: every cited moment carries the position in the video, which anyone can check against the file, and the time it happened in the world, with the evidence for that claim. It has two stages, both in `0.3.0`. Stage 1 reads the
     recording's metadata and a user-supplied `--recorded-at`, handles the time zone honestly, and puts the source and the confidence on every
     time. Stage 2 adds anchors from clocks visible on screen (read by OCR), rollover precision to the second, cross-checks that report conflicts,
     and detection of cuts and pauses.
   - **Going beyond it:** "what changed on screen" between two times; search across kept sessions;
     and the language of each chunk (#474, moved back from R1).
   - **Not chosen:** an MCP server, which stays R2 (ADR 0011), and ready-made workflow prompts.
4. **The deadline rule:** a feature that is not qualified at the cut moves to `1.0.0`. A feature is
   built in slices that each stand on their own, so that a late slice leaves the earlier ones
   releasable.
5. **The word goes out with `0.3.0`, in early December.** The README and the launch checklist,
   which were cycle 3's items, are done in November. The rest of cycle 3 (the documentation sweep,
   the clean-machine try-out and the help text for agents without the skill) is done alongside `0.3.0`.
6. **`1.0.0` follows at the end of December or in the first week of January.** It requires
   [#188](https://github.com/smormah/vsift/issues/188) (RQ-14, adversarial containment of the strict
   worker), produced as denied-action probes that the maintainer reviews before they run, and the API
   stability promise of [#176](https://github.com/smormah/vsift/issues/176). **The stand-in provider and its
   harness are written by the maintainer by hand**, because agent sessions are stopped by an automated safety
   check when they author such code; for the same reason, the probes' techniques are not described in public
   documents (this ADR, the plan, the register or the changelog). Public text says only that the evidence exists and what it covers.
7. **R1 is reshaped and follows `1.0.0`.** P16's OCR and speaker labels, slices 1 and 3 of #406
   (#407 and #409) and #474 move into `0.3.0`. R1 keeps P15, P17 (stitching), P18 (the catalogue;
   `0.3.0`'s search across kept sessions is a first step towards it), P19, P20, #408 (HLS and DASH links),
   #410 (live platform streams), #107 (a live screen share), #368 (whole-recording questions), #147 (a
   faster-whisper adapter) and #174 (word times from the recogniser), with P16's other enrichments (voice
   activity, embeddings, visual tags).

### Rules that bind the `0.3.0` work

- **The reference tool is behaviour only.** Its README is read to see what it does. None of its code,
  text or tests is copied, VSift's designs come from VSift's own architecture, and no comparison
  table goes into the repository's public documents.
- **New engines are adapters**, like whisper.cpp and FFmpeg: external tools or models, pinned by URL,
  size and SHA-256 in the managed catalogue, installed by `setup plan` and `setup install`, and optional.
  The core path works when an engine is absent: the request is refused with a typed failure and a
  remediation. No Python runtime is added, and no engine is linked into VSift's crates when an external
  tool will do. A Rust crate dependency needs the review `AGENTS.md` describes, and `cargo deny`.
  **The choice of each engine is the maintainer's:** the evaluation reports the options with licence,
  size, platforms, accuracy and security.
- **Public JSON v1 changes only by addition:** new commands or new fields, each with its schema,
  contract tests, `cli-v1.md` and the generated reference.
- **The invariants of ADR 0011 hold.** Original media is authoritative; OCR text and speaker labels are
  derived evidence with provenance, nullable confidence and coverage gaps; a one-off investigation
  creates no cross-video record unless the user keeps the session; nothing is downloaded without
  HTTPS, pinned provenance and integrity checks; online media is untrusted input.
- **Evaluation runs on hosted runners** with the real-media set and generated fixtures, not on the
  maintainer's machine.
- **A skill change needs a new trial freeze**, and the batches run at the `0.3.0` cut, so each story
  makes its skill text changes in one place.
- **No `0.3.0` work reaches `main` before the `0.2.3` cut**, or it would ship half-built in `0.2.3`.
  Pull requests stay drafts until the supervisor opens `main`.

## Alternatives rejected

- **Keep the order: launch `0.2.3`, then R1 in full, then `1.0.0`.** Rejected: the features that
  decide a user's first impression would arrive behind a catalogue, a worker plane and a decision packet
  they do not need, with no date.
- **Announce `0.2.3`, or go to `1.0.0` without the new capabilities.** Rejected: VSift would meet its
  first public users without the capabilities it set out to match, and a stability promise would be made
  on a contract that is about to grow.
- **Build an MCP server for `0.3.0`.** Not chosen: the CLI and its JSON contracts stay the one public
  API, and an adapter needs its own threat review (R2).
- **Ship ready-made workflow prompts.** Not chosen for `0.3.0`.
- **Put the FFmpeg update into `0.2.3` by moving the cut.** Rejected: the build exists on 2026-10-31,
  the cut is 2026-10-23, and the hot fix route is the one the delivery model already has.
- **Another date for `0.3.0`.** The end of November puts the announcement in early December and leaves
  `1.0.0` a few weeks of real use later.

## Consequences

- `0.3.0` is a larger release than the earlier ones, with several new optional components. Its risk is
  schedule, not design, which is why the deadline rule exists: a slice that is not qualified waits for
  `1.0.0`, and the release is cut anyway.
- The capability list of `0.3.0` is a plan, not a claim. Public text about OCR, speakers, links and clock
  time waits for each feature's qualification (the claims registry's rules).
- [L-046](../planning/known-limits.md#l-046) (deliberate scope exclusions) is rewritten: OCR, speakers and
  links are no longer R1; live capture, HLS, DASH and scroll stitching are. [L-188](../planning/known-limits.md#l-188)
  (two languages in one recording) moves to `0.3.0`; [L-068](../planning/known-limits.md#l-068) (#188) and
  [L-043](../planning/known-limits.md#l-043) (#176) move to `1.0.0`; [L-132](../planning/known-limits.md#l-132)
  and [L-144](../planning/known-limits.md#l-144) (the FFmpeg re-pin) move to `0.2.4`.
- RQ-14 and RQ-17 are listed for the maintainer's decision at the `0.2.3` cut, as already planned; this
  ADR changes neither. RQ-14 is required for `1.0.0`.
- The R1 document, the delivery governance page, the planning README and both handoff files carry dated
  notes that point here. Where the earlier pages say "R1" for OCR, speakers or links, read this ADR.
