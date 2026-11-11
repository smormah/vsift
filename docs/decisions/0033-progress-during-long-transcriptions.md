# ADR 0033: Progress during long transcriptions: published as it goes, started in the background, read while it runs

- Status: **Accepted, 2026-10-21.** The design was approved by the coordinator on 2026-10-21; decisions
  D1, D2 and D4 and the three modes (section 6) are the maintainer's of that day, D3 and D5 to D8 the
  coordinator's on the maintainer's behalf. The implementation lands in five pull requests (section 11);
  until each lands, what it describes is a plan, not a feature.
- Date: 2026-10-21
- Refines: [ADR 0017](0017-local-asr-through-whisper-cpp.md) (a revision appears only when a run ends),
  [ADR 0020](0020-recoverable-jobs-and-durable-publication.md) (jobs, checkpoints and the commit),
  [ADR 0021](0021-worker-and-batch-host.md) (progress events, admission, the worker's steps) and
  [ADR 0018](0018-visual-candidate-index-and-transcript-search.md) (the visual index and search).
  Their text stays as accepted; where this record changes what they say, it says so in section 10.
- Tracking: the 0.3.0 story "never wait in the dark" (supervisor's brief of 2026-10-21);
  [L-154](../planning/known-limits.md#l-154); the register entries L-219 to L-221; live sources are
  [#107](https://github.com/smormah/vsift/issues/107) and [#410](https://github.com/smormah/vsift/issues/410) (R1).

## Context

On 2026-10-21 the maintainer watched a competing tool give no feedback at all for a 1.5-hour
transcription. VSift was no better: the skill runs `transcript retranscribe --events jsonl | tail -n 1`,
which throws away every progress line, and an agent's shell call returns only when the command ends.
Neither a person nor an agent learns anything until a long transcription is done, and nobody can use
the part that is already done.

What VSift already had:

- **Recoverable jobs (P10).** `job status`, `job resume` and `job cancel` work by job id; a job keeps one
  checkpoint per chunk (the recogniser's raw answer), and an owner lock is the authority on whether a
  process runs it. `job status` already reports the planned chunks and the checkpoints.
- **Chunk progress events (P11)** in `--events jsonl`, counted in chunks, at most one a second.
- **A revision is published once, at the end,** from the merge of all of a run's chunks
  (`merge_chunks`). Nothing a run did is readable before it ends, and a cancelled or failed run
  commits nothing.
- **The visual index is already published in pieces:** `candidates` analyses 60 s windows, at most 30 a
  call, commits each call as an index revision and reports the rest `not_analyzed`.
- **Admission (L-154):** a recognition reserves its whole thread count of the root's capacity, so on a
  four-unit desktop root `frame get` and `candidates` answer `BUSY` until it ends.
- **Storage bounds** that any design must respect: 512 artifacts and 4,096 generations per session, at
  most 64 visual-index records, and a manifest whose artifact list only grows.

The maintainer set three further constraints on 2026-10-21: design **streaming-first**, so that a live
source (a screen share, a live stream; R1) becomes a new source on top of this and not a redesign;
widen it to the **whole investigation pipeline** (audio, visual windows and on-screen text under one
status); and choose the behaviour by **who is waiting** (section 6).

## Decision

### 1. A growing revision: chunks are published as they finish

A local speech-recognition run **opens** its revision when it starts. Everything that names the
revision is known then: the session, the revision number, the source segment, the run's settings, the
planned range and the revision it supersedes. While it runs, the run **publishes** the settled part of
its merge (section 2) as **parts** of that revision, through the session's ordinary manifest chain, so
a published segment is committed evidence: it can be cited at once, and the sealed revision holds it
with the same identity, time and text. When the run ends it **seals** the revision: the final record is
the one `build_asr_revision` makes today, byte for byte, and the generation that seals it retires the
parts.

- **Identity.** Segment identities follow from the revision and the segment's ordinal in time order, as
  today. Publishing only a prefix of the time order keeps every published ordinal final. This is why a
  revision is never published in islands (section 4 handles "the important part first" without them).
- **Parts.** A part artifact holds the segments added since the previous part, so the bytes written grow
  with the transcript, not with its square. A piece is durable as soon as it finishes (its chunk
  checkpoint); it is published at a bounded cadence (D7): at most every 10 s of wall time and at most 64
  parts a revision, the cadence growing for a long run so that the bound holds. **The cadence is a
  parameter of the run, not a constant** (section 8). Retiring artifacts is new to the manifest and is
  allowed only for the parts of the revision that the same generation seals; it is fuzzed and covered by
  the crash campaign.
- **One open revision a session.** Another retranscription or a transcript import while a revision is
  open answers `BUSY` naming the job (the existing `TranscriptionRunning` answer).
- **Ending early (D1, the maintainer).** A published segment can never be retracted, so a run that is
  cancelled, or fails in a way that cannot be resumed, seals its revision as **`stopped`**: the published
  part stays, and the part it did not reach keeps the earlier revision's text, as the rule that a run must
  not delete what it could not read already does for unusable chunks (`EarlierTextRule`). A chunk not
  reached is recorded with a new chunk outcome, `not_reached`. An **interrupted** run (killed, a reboot)
  stays open, and `job resume` continues it. This is one code path for the foreground and the background:
  a foreground `transcript retranscribe` that is cancelled now leaves a `stopped` revision rather than
  nothing. That is changed behaviour against `0.2.x`, recorded in the changelog and `cli-v1.md`.
- **Resume checks what it published.** A resumed run merges again from its checkpoints and checks that the
  settled part equals what was already published. Each open revision records the version of the merge
  rules that published it; a VSift with other merge rules that resumes it seals it `stopped` at what was
  published and continues in a successor revision. Published text is never rewritten.

### 2. What is settled at a seam whose second chunk is not done

`settled_merge` (domain, `vsift_domain::settled_merge`) runs `merge_chunks` unchanged over the finished
chunks and keeps the segments that no unfinished chunk can change.

- **Why the merge is local.** Every rule of the merge relates a segment to segments that overlap it in
  time, or to the rest of its own chunk: rules 1 and 2 look at the segment's own time; the guard closes a
  hole inside one stretch of the union of all validated segments, with dropped segments overlapping the
  hole; rule 3 compares overlapping segments of consecutive chunks and keeps a copy that would leave a
  hole in its stretch. A stretch of that union (overlapping or touching segments joined) is decided by
  what lies in it.
- **The cut.** No segment of the first unfinished chunk, or of any later one, starts before that chunk's
  window start less the **early-audio bound** (`MAX_EARLY_AUDIO_MICROS`, 5 s for a recorded file: a
  decoder's first sample may lie before the window it was asked for, at a frame boundary; a run stops
  publishing at a chunk whose audio began earlier, and its seal is checked against what it published).
  Call that time the reach. Every stretch of the finished chunks' union that
  ends before the reach is final, and the last finished chunk's right edge and core lie past it. The
  **settled time** is the start of the first stretch that ends at or after the reach, or the reach itself.
  The settled part is every merged segment that ends at or before the settled time, and it is a prefix of
  the whole run's merge.
- **So, at a seam whose second chunk is not done:** the first chunk's segments in the overlap, and the
  whole stretch they belong to, are held back. With the R0 plan the settled time trails the finished audio
  by about 10 s (the overlap and the early-audio bound) and more when a stretch of speech runs on; on
  real speech the published text should trail the recognised audio by about 10 to 40 s.
- **The claim is pinned by the test, not by this argument** (the coordinator's priority). The property
  `every_cut_of_a_run_settles_a_prefix_of_its_merge` cuts generated runs of up to 24 chunks after every
  chunk (the full matrix of cut points), for the R0 plan and for windows of 10, 5 and 2 s, with audio
  decoded up to the bound early, rejected segments of every kind, and words that make the repeat rule act,
  and requires at each cut: the settled segments are exactly the first segments of the whole merge, byte
  for byte; the whole merge has exactly those ending by the settled time and no other starting before it;
  the settled unplaced stretches are the merge's; each cut settles at least what the one before did; the
  last cut settles everything. Every existing merge test also runs the matrix through its helper. A sweep
  shows the matrix is not vacuous for each plan (cuts that settle part of a run, and cuts the stretch rule
  holds back). With the stretch rule removed, or the early-audio bound set to zero, the tests fail. A
  counterexample makes the cut more conservative; the merge rules are never changed to suit it.
- **What is not part of the claim:** a revision's summary members (its language, its warning counts,
  `unplaced_ranges` beyond the settled time) describe what is published so far while the revision is open,
  and are final at the seal.

**Amendment, 2026-11-11: a bounded cut inside a long stretch (L-256; H8, the maintainer's decision of 11 Nov to fix it
in 0.3.0).** W8 measured that the stretch rule holds back the whole of a continuous talk: RM03's stretch of touching
segments ran from 0 to 767 s, so its first part came 215 s into the transcription with 12:48 at once. The settled time is
now the later of two cuts; the merge rules do not change, and `MERGE_RULES_VERSION` stays 2.

- **The stretch cut** is the rule above, unchanged.
- **The bounded cut.** Let `λ` be the reach or the last finished window's end less the edge margin (0.5 s), whichever is
  earlier. `T` is the latest point **before** `λ` that no finished segment crosses (segments may touch there) and where a
  segment that rules 1 and 2 keep ends. The bounded cut is the latest point that no finished segment crosses **more than**
  the tolerance (0.5 s) before `T`.
- **Why it is a prefix (sketch; `settled.rs` has it in full).** Rules 1 and 2 can change, once later chunks are finished,
  only segments that end after `λ` (a midpoint in the last chunk's core past its next seam, the last chunk's right-edge
  cut, a replacement spanning a later segment's midpoint), so every segment that starts before `T` keeps its decision.
  The guard closes a hole with segments ordered by how much of the whole hole they cover, and nothing bounds a hole's
  length, so **no fixed distance before the reach is safe**; but no hole contains `T` (a kept segment ends there), holes
  before it are closed from segments that end by it and holes after it from segments that start after it. Rule 3 drops a
  copy only where no hole longer than the tolerance opens, and after the guard every hole is within it, so it looks at
  most the tolerance past the copy's end, which is before `T`. The repeat and loop rules act through rules 1 and 2. Each
  cut only moves forward as chunks finish, so the settled part only grows. Every equality is held back (a segment ending
  exactly at `λ`, a cut exactly the tolerance before `T`).
- **The bound is one of structure, not of time.** On speech of ordinary segments the settled time trails the finished
  audio by the reach's distance from it (10 s with R0) and about two segments; a single segment as long as a window still
  holds its stretch back. No time bound enters the contract.
- **Two margins are redundant, kept on purpose (the planner's ruling of 2026-11-11).** (a) **The edge term of `λ`:** every
  segment a later chunk can change ends after the reach itself, not only after `λ`: rule 1 needs a midpoint at or after the
  middle of the last seam, which is after the next window's start; rule 2's cover needs a neighbour segment that starts half a
  second after that window's start and spans the midpoint; a replacement spans the midpoint of a segment that starts at or
  after the reach. So `λ` could be the reach; the term costs at most 60 ms, with a bound under half a second. (b) **Rule 3's
  margin:** the kept segment that ends at `T` starts at or after the bounded cut (no finished segment crosses the cut), so it
  is still there when rule 3 judges a settled segment, which sorts before it; that segment's hole test then stops before `T`,
  and a cut anywhere strictly before `T` would do. The margin costs about one segment of trail. Neither is load-bearing; both
  stay, because a segment of trail is cheap and published text that differs from the seal is not. The two structural
  conditions (no finished segment crosses `T`; a segment kept by rules 1 and 2 ends there) are what the proof uses: the
  loosened sweep below (4 plans, 2 bounds, 300 runs each) with either one dropped found no counterexample, and they stay.
- **The tests.** The property `every_cut_of_a_run_settles_a_prefix_of_its_merge` and its sweep are unchanged and pass.
  `every_cut_of_a_continuous_run_settles_a_prefix_of_its_merge` adds runs of continuous speech (segments of 1 to 6 s
  touching or with gaps under half a second, over every plan, bound, kind of rejected segment and word kind), with
  segment edges snapped onto the reach, the tight reach, the edge margin and the tolerance before another edge; with R0
  it also requires that from the second chunk the settled part is not empty and trails the finished audio by at most
  22.5 s (for those runs only). A sweep shows many cuts settle inside a stretch, for every plan and bound. With the cut
  allowed a second past the reach, the same sweep finds counterexamples, so the matrix catches a wrong cut. Hand-built
  runs pin each boundary (a segment ending exactly at the reach, exactly at the edge margin, a later segment starting
  exactly at the reach, an edge exactly the tolerance before `T`, touched and crossed points). The seal stays the safety
  net: `a_resume_over_parts_published_under_other_merge_rules_fails_loudly` shows that a run whose seal does not begin with
  its published parts fails as an integrity failure and keeps what was published.
- **Measured (hosted run 43084227851 on the fix's branch, `real-media-set.md` 14.9).** RM03's first readable part came
  20.1 s into the transcription (215 s before), with 24 parts (5 before); its 5- and 10-minute prefixes, which published
  nothing before the seal, had their first parts 22 and 18 s in. The published text trailed the recognised audio by 20 s at
  the median on RM03 and its prefixes, 43 to 48 s at the 90th percentile and 61 s at most, the moment before a part is due;
  RM09 30 s, 55 s and one sample of 110 s. Quick and watched sealed the same revisions, segment for segment.
  [L-256](../planning/known-limits.md#l-256) keeps what is left, low.
- **Earlier revisions.** A revision published under the stretch cut alone is a prefix of the same merge, so a run resumed
  by this version continues it; a run resumed by an earlier build over parts this one published settles less and waits
  until it has caught up. `0.3.0` is unpublished, so no released revision is affected.

### 3. Readers: `transcript get`, `search`, `--since`, coverage

All of this is additive to JSON v1.

- **Plain reads show the newest revision, even while it is open (D2, the maintainer),** marked `partial`
  with what is pending. A live source would never have a sealed revision, so this is the only default that
  works for streaming. The previous complete revision stays readable with `--revision`.
- **Coverage of an open revision.** The part not yet published is listed in `untranscribed_ranges` (so a
  reader of `0.2.x` sees a gap, which is honest) and also in a new `pending_ranges`. When an earlier
  sealed revision still holds text there, a new `pending_covered_by` names it and the ranges, so an agent
  can read them with `--revision`. The envelope is `partial` with the new coverage reason
  `transcription_pending`; an open revision's answer is never `complete`.
- **The revision block** gains `state` (`open`, `sealed` or `stopped`) and `published_until_us`.
- **"Since" reads.** `transcript get --since <token>` and `search --since <token>` answer what was
  published after the token, in time order, with the next `since_token`. A token is bound to the
  session, one revision and its watermark (how many of its segments the reader has been given), and always
  reads that revision; once it is sealed or stopped the answer says so and names any successor. `--cursor`
  still pages inside one since query. A plain read of an open revision gives its token only on the answer
  that ends the read (no `next_cursor`), so a reader who follows it never skips a page (review RV-1,
  2026-10-22).

### 4. The important part first

`job start ... --first <from_us>:<to_us>` (D5) runs as one job in two phases. It widens the window to the
whole run's chunk grid (chunk k is the window starting k times the stride after the run's start),
recognises those chunks first and seals them at once as an ordinary range revision N, citable within
about one window's processing time. Then it runs the whole source in time order as the growing revision
N+1, reusing those chunks' raw answers, so no audio is recognised twice and N+1 is the revision a run
without `--first` would make: its seams are the chunk-join merge's, not range splices. Until N+1's
settled time passes the window, a read of the window names revision N in `pending_covered_by`. The skill
tells agents: when the user names a moment, start with that window.

### 5. One background job for the whole pipeline

- **`vsift job start <session> [--transcript] [--candidates] [--screen-text] [--first <from_us>:<to_us>]
  [--mode quick|watched|queued] --json` (D3)** starts a job of a new kind, `prepare`, and returns at once
  (in the watched and queued modes) with its `job_id`. A `prepare` job tracks one sub-job a stream; the
  audio stream is an ordinary retranscribe job, so its checkpoints, resume and the worker's step mapping
  do not change. It is resumed and cancelled like any job.
- **Order (D6): visual windows first, then audio, then on-screen text when asked.** Visual analysis runs
  at about 29 to 67 media seconds a second, so 90 minutes take a few minutes; after it, `candidates` over
  the session is a warm read (no tool, no admission) for the whole transcription. Visual windows are
  published through the existing index revisions, a batch about every ten windows or 15 s, which keeps a
  four-hour video within about 24 of the 64 index records. On-screen text follows the analysed windows:
  a finished window's frames can be read without waiting for the whole video. That needs, from the OCR
  work (draft #499, coordinated through the supervisor), an `ocr` engine call bounded to windows already
  analysed, and a pending reason of its own for windows not yet analysed.
- **The runner.** In the watched and queued modes `job start` records the job and starts the same binary,
  by its explicit path, as `job resume <job> --json`, with the caller's global options, its standard
  streams connected to nothing and its working folder the executable's own (#469). No shell is used. On
  Unix it gets a process group of its own (`process_group(0)`), so a terminal's Ctrl-C and a kill of the
  caller's group do not reach it. On Windows it starts with `CREATE_NO_WINDOW` (a hidden console of its
  own, which its providers share, rather than `DETACHED_PROCESS`, for the reason the process supervisor
  gives) and `CREATE_NEW_PROCESS_GROUP`, and with `CREATE_BREAKAWAY_FROM_JOB` when the caller's job object
  allows it; the answer says which (`detachment`). `job start` waits up to 10 s for the runner to hold the job's owner lock; if it does
  not, it kills the child, leaves the job resumable and says to run `job resume` in the foreground.
- **No orphans, with bounds.** A watcher reads the job's cancel request every 0.25 s and cancels the run,
  and the process supervisor kills a running provider on cancellation, so `job cancel` stops the runner in
  about a second, not at the next chunk. `session close` asks every live job of the session to cancel,
  waits up to 10 s, then closes (**the maintainer's decision of 2026-10-21**, a changed behaviour: until 0.2.x a
  close while a transcription ran answered `BUSY`); a runner that missed it fails its next publication, sees the session
  closed and exits. The runner stops itself at the session's expiry. Killed outright, the job is
  interrupted and resumable; on Windows its providers end with its job object, on Unix a provider can
  outlive it until its chunk ends (L-220).
- **Status in seconds (D8).** `job status` gains, per stream: its state, stage, `done_us`,
  `published_until_us`, `total_us` (null when the end is not known), the elapsed time and the speed in
  media seconds a wall second; and `percent` and an **estimate** of the time left only when the end is
  known. The estimate is the media still to do divided by the cumulative measured speed of the fresh
  chunks of the current attempt, admission waits left out (chunks reused on a resume do not count); it is
  labelled an estimate with its basis, is null until at least two fresh chunks and 20 s have run, and is
  optimistic over silence. The runner writes these figures to a small record in the job's folder, at most
  once a second. Progress events gain the same members. `job status` also reports the job's **mode**.
- **Admission (D4, the maintainer): a background recognition leaves one unit free** (its threads are the
  root's capacity less one, when that is at least two), so `frame get`, crops and audio clips work while
  it runs. On a four-unit root the recognition is up to about a quarter slower, and since the thread
  count is part of a run's provenance, a background revision is not the same revision as a foreground
  one. L-154 is revisited accordingly.
- **The live line.** When the output is readable text (not `--json` or `--events`) and standard error is
  a terminal, the CLI shows one line on standard error that it updates at most once a second and clears
  before the result ("Transcribing 34:10 of 90:00 (38%), 2.1x real time, about 26 min left (estimate)").
  It never appears in JSON or JSON Lines output.

### 6. Three modes, chosen by who is waiting (the maintainer)

The three modes are parameters of the one engine path, never separate code paths. They are chosen
automatically, and `--mode quick|watched|queued` overrides the choice.

- **quick:** the default for a short recording (under about 10 minutes; the threshold is set from
  measurement in the pull request that adds the modes). The call runs in the foreground and answers when
  done, as today: no runner, no polling, no parts; the revision is sealed in one generation. Its overhead
  against today's path is measured and must be nil within noise.
- **watched:** the default for a longer recording; sections 1 to 5 as written: the background runner, the
  growing revision, the status with its estimate, one admission unit left free.
- **queued:** nobody is waiting, or several recordings were handed over with no rush. Jobs wait in a
  queue of the session root and run one after another at full capacity, with no spare unit, publishing
  only at the seal. **Fairness:** interactive commands and watched jobs are admitted ahead of queued work
  at the next chunk boundary, so a queue never blocks someone waiting. The queue reuses R0's worker ideas
  (idempotent steps, records, resume) for ordinary sessions, and survives a restart. **For 0.3.0:** first
  in, first out, one job at a time a root, a status a job; priorities between many queued jobs wait for
  1.0.
- **The skill** picks quick or watched by the recording's length, and queued when the user hands over
  several recordings or says there is no rush.

**Note, 2026-11-11: the threshold and the estimate measured on real recordings (readiness item 5; W8).** The hosted
`P14 real media` run 43036639892 (mode `progress`, commit `89abe00d`, 46 minutes; AMD EPYC 9V74, 4 vCPUs, `base`,
whisper.cpp with 4 threads for quick and 3 for watched) ran `job start --mode quick` and `--mode watched` on fresh sessions
of RM03 (19:16), of its first 5, 10 and 15 minutes (cut on the runner by stream copy) and of RM09 (28:02). Both modes seal
after the visual stream, which both run first (9 to 28 s on the 1080p talk, 6 s on the small briefing).

| Media | Quick, to the seal | Watched, to the seal | Watched costs | First readable part (watched) |
| --- | --- | --- | --- | --- |
| RM03, first 5 min | 97.9 s | 103.5 s | +5.7% | none before the seal |
| RM03, first 10 min | 180.2 s | 183.8 s | +2.0% | none before the seal |
| RM03, first 15 min | 264.1 s | 268.3 s | +1.6% | 238 s (12:48 of the talk, at once) |
| RM03, 19:16 | 336.0 s | 343.9 s | +2.4% | 244 s (12:48 at once) |
| RM09, 28:02 | 397.4 s | 405.3 s | +2.0% | 27 s (24 parts in all) |

- **The two modes cost about the same, and the revisions agree.** The real-time factor is 0.29 to 0.33 on the talk and 0.24 on
  the briefing; a watched run is 1.6 to 5.7 percent slower for its one thread less. For every one of the five pairs the
  sealed revisions are the same segment for segment and word for word (272 and 369 segments for the two whole items), so ADR
  section 9's comparison holds on real speech despite the different thread counts.
- **There is no cliff at ten minutes.** On the talk the wall time is about 16 s plus 0.275 s a media second (the briefing,
  with a small picture and a faster recogniser, is quicker: 0.24): a recording just under ten minutes keeps the caller waiting
  about three minutes in quick mode, one of five minutes about 1:40.
- **Proposal: (a), keep ten minutes, and change nothing before the cut.** The cost of either side of the line is small (a
  few percent of wall time, the same revision), so the number matters less than the waiting; a rule by expected wall time
  (c: quick while the call is expected to answer within about two minutes, which is about six minutes of the talk here) is the
  better rule in principle, but it needs the speed of the machine before the first run, which the call does not have, and it
  would change the help text, the skill and the contract. It is a candidate after `0.3.0` and the maintainer's to decide.
  The measurement also shows that the watched mode's first part is not what the threshold choice would buy on continuous
  speech: see [L-256](../planning/known-limits.md#l-256), which is a decision of its own.
- **The estimate's error** is in [L-257](../planning/known-limits.md#l-257): +46 to +62 percent at 10 percent done on the
  talk, -13 and -23 percent at 10 and 25 percent on the briefing; within 10 percent from 50 percent done on runs of 15
  minutes or more; first shown 44 to 48 s into the transcription of the talk and 20 s into the briefing's. A known limit, no fix.
- **The claim of section 2 is not met on continuous speech:** the published text trailed the recognised audio by up to about
  12 minutes on RM03, not 10 to 40 s ([L-256](../planning/known-limits.md#l-256)); it did meet it on RM09. The maintainer
  decided the same day to fix it in `0.3.0`: the bounded cut of section 2's amendment of 2026-11-11.

### 7. What the skill says

One place, the PREPARE state of `SKILL.md` (with the lines of `references/commands.md` that the skill
contract requires): for a recording longer than about 10 minutes, start with `job start` (with `--first`
when the user named a moment), check `job status` and report progress to the user at each check, and work
on the published part with `search`, `transcript get` and `--since`, never calling it complete. A new
trial freeze follows the 0.3.0 rule (`v030-rules.md`): left alone while the pull request is a draft.

### 8. What a live source would still need

0.3.0 reads recorded files. The target scenario the maintainer set on 2026-10-21 is **a live football
match: the agent responds within seconds when a goal is scored, then analyses the play before it.**
**VSift does not analyse or detect events; the agent does.** VSift's job is to give the agent fresh,
grounded evidence fast enough to judge. Future work, not built in 0.3.0:

- **Latency:** second-level, so that an agent can respond within a few seconds of the moment.
- **Fresh evidence within seconds:** frames on change or at a period, transcript text as it is
  recognised, on-screen text, each with its time and position; **everything searchable as it arrives**,
  over the recent past.
- **Neutral shortlists only,** as `candidates` gives today ("the screen changed a lot", "loudness rose"),
  as pointers, never judgements such as "goal".
- **A bounded rolling look-back buffer** of raw media, disposable by default, that the agent can pull from
  after it judges a moment relevant.
- **Agent-driven look-back priority:** the agent asks for a window first, which is the live form of
  `--first`.
- **Push delivery** of new evidence (a long-running `vsift watch` streaming JSON lines, or R2's
  interfaces), so the agent watches rather than polls.
- **An unbounded input:** a chunk plan with no known end (planned incrementally; the plan is a fixed
  stride, so the next window is always known), back-pressure from capture to recognition and analysis,
  and dropping input under overload, stated in coverage.
- **Compaction of parts mid-stream** (0.3.0 retires parts only at the seal), because a stream never seals
  and the artifact and generation bounds would be reached.
- **Video too:** frames arriving with no known end, for the visual and on-screen-text streams.
- **A long-lived host** rather than a detached runner per job, and clock anchoring of live time
  (time anchoring, ADR 0029, in review).
- **The rights and terms** of broadcast content stay the user's.

**What was checked in the 0.3.0 design so that none of it blocks the scenario:**

- **The settled-time rule works with small chunks:** the matrix of section 2 runs with windows of 10, 5
  and 2 s (shorter than the early-audio bound) and holds.
- **The early-audio bound is a parameter of `settled_merge`, not a constant.** It sets the least delay
  between audio being heard and its text being settled (5 s for a recorded file, whatever the chunk
  size). A live source with a decoder bounded by construction passes its own bound; the matrix also runs
  with a 40 ms bound where the input allows it.
- **The publication cadence is a parameter of the run, not a constant** (section 1).
- **Status is in seconds of media and a speed;** percent and the estimate are added only when the end is
  known (section 5), so a status with no known end is still meaningful.
- **Known-end assumptions that remain, named:** `plan_chunks` needs the range's end, and a run plans at
  most 1,024 chunks; the visual index is bounded to four hours; parts are retired only at the seal. Each is
  local to its module and is listed above as live-source work.

### 9. JSON v1, records and tests

- **Contract, additive only:** the command `job.start` and its data schema; new members of the job data,
  the revision block, `transcript_coverage`, progress events and the `since` reads; the coverage reason
  `transcription_pending`; the chunk outcome `not_reached`. **No new failure code.** Each addition has
  its schema, contract tests, `cli-v1.md` and the guide's reference.
- **Tests, deterministic,** with the stand-in recogniser the existing tests use (one that waits on a gate
  between chunks for reads in mid-run): the merge matrix (domain); growing publication, the seal, a cancel
  sealing `stopped`, resume against what was published and the merge-version hand-over (application);
  parts, retirement, chain validation, the crash points and a fuzz target for the part record
  (infrastructure); through the binary on all three systems: a background start that outlives its caller,
  the status, reads in mid-run, a hard kill of the runner, `session close` stopping it, and no process
  left; the modes' threshold, the fairness rule and a queue surviving a restart.
- **Real recordings** only on the hosted `P14 real media` workflow, with RM03 (19 minutes) or RM09
  (28 minutes): a background run's sealed revision against a foreground run's, and the estimate's error.
  Nothing long runs on a development machine.

### 10. What this changes in earlier records

- ADR 0017 and 0020: "a failure or cancellation discards all work" and "a cancel that wins leaves no
  generation" no longer hold for a run that published parts: it is sealed `stopped` (D1). A run that
  published nothing (a quick run, a queued run, a run stopped before its first part) still leaves nothing.
- ADR 0021 section 5a: a background (watched) recognition takes one unit less than the root's capacity
  (D4); interactive and quick runs are unchanged.
- ADR 0018: reads default to the newest revision, including an open one (D2).

### 11. Delivery

1. **This record and `settled_merge`** with its matrix (no behaviour changes).
2. **The growing revision,** its storage and the readers: coverage, the revision block, `--since`.
3. **The background start,** status, cancellation and `session close`, the live line, the quick and
   watched modes, and the skill.
4. **The visual stream** in the `prepare` job.
5. **The queued mode** (if it does not fit in the third).

The on-screen-text stream follows #499 and its bounded call; `candidates --since`, visual publication per
window rather than per batch, compaction mid-stream and the chunk cache across jobs are staged for 1.0 or
R1.

## Implementation notes: the second pull request (2026-10-21)

The second pull request implements sections 1 to 3: the growing revision, its storage and its readers.
The background start, the status, the modes and the skill follow in the third.

- **Domain.** `AsrChunkOutcome::Pending` and `NotReached`; `TranscriptRevision::new_open` and
  `new_stopped` beside `new`, each holding its state to its own chunks (`InvalidGrowth` otherwise),
  `RevisionState` and `published_until`; `EarlierTextRule::stopped_at`; `SearchCoverage::pending` and the
  cut of an open or stopped revision (its run's reading counts only before the time it is published
  until; its segments cover themselves); `MERGE_RULES_VERSION`, held to a digest of what the merge makes
  of a fixed sweep (`the_merge_rules_version_names_what_the_merge_makes`); `MAX_TRANSCRIPT_PARTS` (64).
- **Application.** `RunObserver` and `RunSoFar` (what a run has after each chunk; `open` settles it),
  `build_open_asr_revision` (the sealed revision's assembly, cut at the first segment a later part could
  precede or change: a segment of the run past the settled time, or earlier text inside the replaced range
  that reaches past it), `build_stopped_asr_revision`, the job's `Publication` (`AtTheSeal` or
  `AsItGoes` with an interval on an injected `Stopwatch`) and its `Publisher`, which publishes only while
  every finished chunk is checkpointed and its audio began within the bound, and whose seal is checked
  against what it published (`PublishedDiffers` otherwise, sealed stopped). `seal_stopped` on a
  cancellation or a failure that ends the job; `job cancel` of a job no process runs seals it too.
  The since reads (`since_token`, `page_since`) are scoped cursors bound to the revision's number and a
  digest of its identity.
- **Storage.** The part record (`vsift.transcript_part` version 1: the revision as it stands with only
  the segments it adds, its part number, first ordinal, merge-rules version and job); the store
  continues an open revision only from its own job and its own segments, refuses another transcript
  record while one is open, and seals it with `SealTranscript`, which retires its parts in the sealing
  generation; a session read rebuilds the open revision as the newest; a bundle is retained only once it
  is sealed. A stopped revision is a version-2 record with `published_until_us`. Fuzz target
  `transcript_part` (36 targets with the archive and OCR targets of `main`) with seeds pinned by `open_transcript_store`.
- **Engine and CLI.** An open newest revision is not a base: the revision it supersedes is, and only its
  own job continues it (another request is `TranscriptionRunning`). `transcript retranscribe` and
  `job resume` publish as they go (`PUBLICATION_INTERVAL`, 10 s); `transcript get --since` and
  `search --since`; `ReadGrowth` names the revision over what is pending.
- **Tests.** The cut matrix of the first pull request; every chain of `chain_tests` watched at every cut
  (`Watched::seal`: each open revision a prefix of the sealed one, ids included, covering nothing the
  sealed one does not, and its stopped revision keeping it); the job's publication, cancellation and
  resume over the in-memory store; the part record, the store and the stopped record
  (`open_transcript_store`); the contract (`growing_transcript_contract`, with the frozen examples
  `transcript-get.open.json` and `search.open.json`); and, opt-in with `FFmpeg`, `engine_growth`: a run
  read while it holds, followed with its token and sealed, and a run cancelled while it holds.
- **Accepted by the maintainer (2026-10-23): `INTEGRITY_FAILURE` for a resume under other merge rules.**
  A resume that finds an open revision written under another `MERGE_RULES_VERSION` seals it as stopped
  and fails with the existing code `INTEGRITY_FAILURE`; no new failure code is added. It can happen only
  across releases.
- **Not in this pull request:** the background runner, the status in seconds, the live line, the modes,
  the skill, the visual stream (sections 4 to 7); the crash campaign's workload still publishes at the
  seal only, and a workload with parts is added with the background runner.

## Implementation notes: the third pull request (2026-10-21)

The third pull request implements sections 5 and 6 for the transcript stream: `job start`, the `prepare`
job, the background runner, the status in seconds, cancellation and `session close`, the live line, the
quick and watched modes, D4 and the skill.

- **Domain.** `JobKind::Prepare`; `JobMode` (`quick`, `watched`) with `QUICK_LIMIT_MICROS` (ten minutes)
  and `for_duration` (quick under it, watched from it on and when the length is unknown); `PrepareStream`
  (`transcript`); `leaving_one_free` (D4: the root's capacity less one, when that leaves one);
  `MediaProgress` (where the finished chunks reach, the elapsed time, and what the fresh chunks advanced
  and took) with `percent`, `speed_millis` and `estimate_remaining_us` (D8: at least
  `ESTIMATE_MIN_FRESH_CHUNKS`, 2, and `ESTIMATE_MIN_FRESH_MICROS`, 20 s); `ProgressUpdate.media`.
- **Application.** `JobRequest::Prepare { mode }` and its keys (`vsift.prepare-request.v1`,
  `vsift.prepare-op.v1`: the session, the mode and the sealed base; `prepare_spec`); the `run_prepare` use
  case over a `PrepareStreams` port: it takes the job over, reconciles it, starts it, runs each stream and
  records the last stream's commit as its own (so a `prepare` job that died while committing is reconciled
  from the manifest chain like any job), or how a stream stopped; a stream that stops while the job is
  asked to cancel is cancelled too, so no open revision is left behind. `JobProgressLog`, the advisory
  progress record a runner writes. The run's `Stopwatch` moved to the ports (`StoppedStopwatch` for hosts
  that time nothing); `Publication` holds only its interval. `run_chunks` meters its chunks
  (`Meter`): the clock starts once the plan is made, after any admission wait, and a chunk taken from a
  checkpoint advances the position, not the speed.
- **Infrastructure.** The `prepare` record (`request.mode`, no range, no planned chunks; version 1 of the
  job record, decoded strictly); `progress.json` in the job's folder (`vsift.job_progress` version 1,
  rewritten without synchronisation, read strictly, a damaged one as none); `launch_detached`, which starts
  the runner as section 5 says and reports its `Detachment`.
- **Engine.** `job_start` (the mode and its basis, the job created, taken up, restarted, found running, or
  `done` when the newest revision is a succeeded `prepare` job's commit); `job_resume` of a `prepare` job
  runs it through `EngineStreams`: its transcript is `retranscribe` of the whole source, published as it
  goes and with `RecognitionShare::LeavingOneFree` in the watched mode, at the seal and whole in the quick
  mode, watched for the `prepare` job's own cancellation, while a loop records the stream's progress once
  a second. `job_status` adds the mode and the streams (the stream's job, its observed state, its figures
  and how far its open revision is published); `job_cancel` of a `prepare` job cancels its stream's job;
  `close_session` asks every live job of the session to cancel and waits up to `SESSION_CLOSE_JOB_WAIT`
  (10 s). The prepare path's futures are boxed: unboxed, they overflowed the main-thread stack of a debug
  build on Windows.
- **CLI and contract.** `job start [--mode quick|watched]` (`job.start`, `job-start-data.schema.json`);
  the runner's arguments are the CLI's (`job resume <job> --json`, `--session-root`,
  `--host-isolation`), and it answers once the runner holds the job or has ended it (within
  `RUNNER_HANDSHAKE`, 10 s; otherwise the runner is ended and the call fails `DEADLINE_EXCEEDED`, or
  `MISSING_CAPABILITY` when it could not start or ended first, naming the job and suggesting `job resume`).
  **Review RV-1 (2026-10-22):** the handshake counts only a live owner or an end the job was not at
  before the launch, so an interrupted job's state no longer passes for a runner that took it (finding 3);
  `job_start` reconciles the record it now owns before it chooses, so a job left `cancelling` by a dead
  runner restarts instead of waiting for a runner that refuses it; and a retranscription job records its
  `RecognitionShare` (`request.share`, written only when not `whole`, in the domain since), which
  `job_resume` uses again, since the threads are part of the job's key (finding 2). Two requests that
  differ only in the share name the same job when their keys agree (a root of one unit, a host's own
  recognizer) and open it.
  `job-data` gains `request.mode` and `streams` with `$defs.media_progress`, which progress events reuse
  (`media`). The live line is written by the task that owns the output, from the newest observation, at
  most once a second, and cleared before the result. The top-level help's example names `job start`.
- **Skill.** PREPARE starts the transcription with `job start` (a VSift before 0.3.0 refuses it, and the
  skill falls back to `transcript retranscribe`); `resume.md` gains "A transcription in the background" (go
  on at once, check `job status` between steps and report its progress, read the published part,
  `--since`, never call it complete before `succeeded`), and a running `prepare` job is no longer a reason
  to stop media work; `commands.md` classes `job start` `free` and allows the `--since` forms.
- **Tests.** The estimate's thresholds and the modes' threshold (domain); the `prepare` job's identity,
  commit, interruption and resume, cancellation of its stream and busy refusal, and the meter over fresh and
  reused chunks (application); the `prepare` record and the progress record read strictly (storage); the
  contract and its frozen examples (`job_start_contract`); through the binary, a watched `job start` whose
  runner takes the job before the call answers and ends it failed without whisper.cpp, a restart, and a
  quick one in the foreground (`job_cli_contract`); opt-in with `FFmpeg`, a watched job's progress and
  published part read from another engine while it holds, and `session close` stopping it
  (`engine_growth`).
- **Not in this pull request.** `--first` (section 4) and the visual stream come with the fourth pull
  request, whose `prepare` job has more than one step; the queued mode with the fifth. The crash
  campaign's workload still publishes at the seal only; a workload with parts needs its verifier to accept
  an open revision and is a follow-up. A runner whose session expires is not stopped at once: the expired
  session refuses its parts and its seal, and it ends when its run does (L-220).

## Implementation notes: the fourth pull request (2026-10-21)

The fourth pull request implements section 4 (`--first`, D5) and the visual stream of section 5 (D6). A
`prepare` job now runs three streams, in the order of `PrepareStream::ALL`: `first` (only with a window),
`visual`, `transcript`.

- **The first window.** `job start --first <from_us>:<to_us>` (at most `MAX_FIRST_WINDOW_MICROS`, 30
  minutes, which also bounds what is held for reuse); the window is part of the `prepare` job's identity and
  record (`request.first`). The runner probes the source's end, widens the window to the whole run's chunk
  grid (`align_to_chunk_grid`: every chunk of the widened range has the window of the whole run's chunk at
  that time) and retranscribes it as a range, sealed at once.
- **One recognition for the window (D5).** A run keeps the raw answer of every chunk it finishes in a
  `ChunkSeed`, under a seed key that is the recognition key without the replaced range (`seed_key`), and a
  later run takes the answer kept for a chunk of the same window instead of recognising it, stores it as
  its own checkpoint and treats it as reused. The engine's seed (`RecognitionSeed`) lives in the runner's
  memory, bounded to 96 answers; a runner that dies loses it, and the whole run then recognises the window
  again. The sealed revision is the one a run without the seed makes, because a recognizer hears only the
  chunk's audio (the property `a_whole_run_takes_the_first_windows_answers_and_makes_the_same_transcript`
  holds with a recognizer whose answer depends only on the window; L-200 bounds sameness to one machine,
  which a seed never leaves).
- **The visual stream (D6).** Batches of `VISUAL_BATCH_WINDOWS` (10) windows through the same analysis and
  commit as `candidates` (`analyse_visual_batch`), each an index revision, until every window is indexed;
  a busy root is asked again for about a minute; a source without video skips the stream. It has no job of
  its own: the progress record now holds each stream's phase (`running`, `done`, `skipped`), from which
  `job status` reports its state. Its `media` counts windows of video.
- **Status and contract.** The stream identifiers `first`, `visual` and `transcript`; the stream state
  `skipped`; `request.first` in `job-data`. A visual stop that a retry can get past leaves the `prepare` job
  interrupted (the windows it committed stay); one it cannot, such as missing media tools, fails it.
- **Skill.** PREPARE says to add `--first` when the user named a moment, and `resume.md` that `candidates`
  answers without analysing once the `visual` stream has succeeded.
- **Tests.** The grid alignment over sources, windows and lengths (domain); the seed taken by a whole run,
  which recognises only the chunks outside the window and makes the same transcript, and the `prepare`
  job's order of streams and recorded commit (application); the `prepare` record with its first window and
  the progress record's phase (storage); `--first` checked before anything runs and recorded
  (`job_cli_contract`); opt-in with `FFmpeg`, a watched job with a first window whose window is sealed
  first, whose video is indexed before the transcript (a warm `candidates` read), and whose whole run
  recognises the window's chunk not again, and a source without video skipping the visual stream
  (`engine_growth`).
- **Accepted by the maintainer (2026-10-23).** The first window's extra FFprobe (the runner needs the
  source's end before the window runs) and its extra transcript revision (the window is sealed as a range
  revision of its own), and the seed kept in the runner's memory (a runner that dies loses it, and the whole
  run then recognises the window again, which is correct but slower).
- **The sixth pull request** adds the crash campaign's workload with open revisions (its notes follow the
  fifth's). The visual stream's kill points are not covered yet.

## Implementation notes: the fifth pull request (2026-10-21)

The fifth pull request implements the queued mode of section 6.

- **The mode.** `JobMode::Queued` (`--mode queued`), never chosen by length. Its runner starts like a watched
  one; its transcripts run with `AdmissionWait::Queued`, at the whole capacity, publishing at the seal.
- **The queue.** `job-queue/` of the root: one entry a job, named by when it was entered and its id, so a
  listing in name order is first in, first out across sessions and survives a restart; `job start` enters
  a queued job (one already in keeps its place), and the job leaves the queue when it ends (and `job
  cancel` of one no process runs takes it out). One queued job of a root runs at a time: its runner holds
  `coordination/queue-turn.lock` for the whole job. A waiting job is still `queued` (a wait is no attempt):
  `run_prepare` asks `PrepareStreams::before_start` before it starts, and a cancellation meanwhile cancels
  it. `job status` reports `queue_position`, counting only entries whose job has not ended (an ended one's
  entry is removed as it is found). **Review RV-1 (2026-10-22):** the position, and so the turn, counts only
  the entries ahead whose job a runner holds or that were entered less than 30 s ago (their runner may
  still be starting); an entry nobody runs keeps its place without holding up the live jobs behind it
  (finding 1). An entry's name holds a stamp, the second times 10^9 raised past the newest entry's under
  `coordination/job-queue.lock`, so jobs entered in one second keep their arrival order (finding 8).
- **Fairness.** Queued work holds its permit marked by an exclusive lock on
  `coordination/admission-queued.lock`. An ordinary caller (an interactive command, a watched job) that
  finds the slots taken while that marker is held holds a shared lock on
  `coordination/admission-demand.lock` and waits up to `QUEUED_YIELD_WAIT` (120 s) for the slots, instead
  of being refused. Before every chunk but the first, the queued run's `ChunkGate` looks at the demand lock:
  while someone demands capacity it gives its permit back, polls every `QUEUED_ADMISSION_POLL`, and takes
  the capacity back once nobody does. A queued run never takes capacity someone waits for. Every lock is an
  operating-system lock, so a process that dies releases what it held.
- **The wait is shown** (the supervisor's condition of 2026-10-21 for accepting the 120 s). A recognition's
  own admission no longer waits inside the store: `RevisionStore::admit` takes the slots or answers `Busy` at
  once, and the run's `admit_waiting` waits for queued work itself, on its own timer (so no thread blocks),
  demanding the capacity through `RevisionStore::demand_capacity` and polling every `QUEUED_YIELD_POLL`
  (50 ms) until it is given back, it goes to someone else or `QUEUED_YIELD_WAIT` is spent, counted over the
  whole run, its retries included. When the wait starts, the progress port hears it once
  (`ProgressSink::queued_work_waiting`), and the engine's `AdmissionWaiting` carries the cause `QueuedWork`.
  The CLI writes a `lifecycle` `admission_waiting` event with `reason` `queued_work` under `--events jsonl`
  (`transcript retranscribe`, `job resume`, `job start`, and a worker's request) and shows `Waiting for
  queued work to make room (at most 2 min)` on the live line. A session write, an evidence capture, a visual
  window and a watched job's background runner still wait inside the store without saying so (L-223). The visual windows of a queued job
  take their capacity window by window, as `candidates` does, so they free it between windows anyway.
- **Accepted by the maintainer (2026-10-23): the other waits for queued work stay a residual for 0.3.0.** A session
  write, an evidence capture, a visual window and a watched job's background runner keep waiting inside the store
  without saying so (L-223). They are shown in 1.0 if trials show that people or agents are confused by them.
- **Not in 0.3.0:** priorities between queued jobs, and a host that runs the queue without a runner a job
  (1.0). After a restart a waiting job needs `job start` or `job resume` to run again (L-222).
- **Tests.** The queue's order across sessions, a job entered again keeping its place, a restart, one turn
  at a time, and an ordinary caller waiting for queued work and admitted when it yields (storage); a queued
  run yielding at a chunk boundary and committing what an ordinary run commits, and a waiting job staying
  queued, cancelled by a cancellation (application); a queued `job start` through the binary; opt-in with
  `FFmpeg`, a queued job holding the whole root and a `frame get` waiting for it and admitted at its next
  chunk (`engine_growth`).

## Decisions for the maintainer

All are taken (2026-10-21):

- **D1** One code path; a cancelled or unresumable run keeps what it published as a `stopped` revision.
  *Nothing that was shown can disappear.* (The maintainer.)
- **D2** Plain reads show the newest revision even while it is open, marked partial with what is pending.
  *You see the work as it happens.* (The maintainer.)
- **D3** `vsift job start` with a `prepare` job. *One command for the whole preparation.*
- **D4** A background recognition leaves one admission unit free. *Frames work during a long
  transcription, at some cost in speed.* (The maintainer.)
- **D5** `--first` as one job that recognises the window once. *The moment you asked about comes first,
  with no extra work.*
- **D6** Visual windows first, and the visual stream in 0.3.0. *Candidates are ready within minutes.*
- **D7** Publication at most every ~10 s, at most 64 parts a revision. *Fresh enough to follow, within
  the storage bounds.*
- **D8** The estimate from the cumulative measured speed. *A steady estimate, labelled as one.*
- **The three modes** (section 6). *Chosen by who is waiting.* (The maintainer.)
- **`session close` stops the session's running jobs** (section 5): it cancels them, waits up to 10 s, then closes.
  *Closing a session ends its work.* (The maintainer, 2026-10-21.)
- **The other waits for queued work** (section 6, L-223) stay a residual for 0.3.0 and are shown in 1.0 if trials
  show confusion. (The maintainer, 2026-10-23.)
- **The ten-minute threshold** (section 6) stands for now; the hosted `P14 real media` check on RM03 and RM09
  measures it and the estimate's error once the stack is on `main`, and a follow-up adjusts it if the data says
  so. (The maintainer, 2026-10-21.)

## Consequences

- A long transcription is visible while it runs, to a person (the live line) and to an agent (status,
  partial reads, `--since`), and the published part is evidence that can be cited before the run ends.
- A run that published parts and then ended early leaves a `stopped` revision; sessions written this way
  are refused by releases before 0.3.0 (L-221).
- A background revision differs in identity from a foreground one (its thread count), and it is somewhat
  slower on a small root (D4).
- The settled part lags the audio (L-219), and the runner's detachment depends on the caller's platform
  and job object (L-220).

## Alternatives

- **Provisional reads from checkpoints, nothing committed until the end.** Rejected: text read early could
  not be cited, and a live source would never commit.
- **A revision committed per batch of chunks, each a splice of the one before.** Rejected: the seams
  between batches would be range splices rather than the chunk-join merge, so the final transcript would
  differ from a whole run's; carried segments get new identities in every revision; and the bytes grow with
  the square of the transcript.
- **Ordinals in publication order, so a window could be published as an island first.** Rejected for
  0.3.0: it changes how every segment identity is derived and the sealed record's order; section 4 gets the
  window first without it.
- **`transcript retranscribe --background`.** Rejected: the visual and on-screen-text streams would need a
  second command.
- **The P11 worker request for desktop sessions.** Its ideas are reused (idempotent steps, records,
  resume); the command is not, because it runs only in operator workspaces with input roots and request
  files.
- **A long-running daemon now.** Deferred to the live-source work (section 8): a detached runner per job
  answers the recorded-file case with less to operate.

## Implementation notes: the sixth pull request, the crash campaign with open revisions (2026-10-22)

- **Workload.** `tools/p10-crash-campaign/src/grow.rs`. The stand-in speech is four chunks (105 s). Two new
  operations: `grow` (a whole-source retranscription with `Publication::AsItGoes` and no interval: three
  parts, then the seal) and `stop` (the same, with its job cancelled once its first part is acknowledged,
  as `job cancel` would; the run seals a stopped revision). Every part is acknowledged the moment its
  generation is committed: a `START` and an `ACK` of kind `part` with a sequence number of its own, the open
  revision's identity and no artifacts (the seal retires the part's file). A job operation on a session that
  a crash left with an open revision continues it: it resumes the job to its seal or stops it as `job
  cancel` of a job no process runs does (`cancel_job`, then `seal_stopped_now`), at random; a job that has
  ended, or has nearly spent its attempts, is stopped.
- **Verifier.** Besides the existing checks, a part's acknowledgement is held (its generation's manifest
  unchanged; its revision still the newest, or found by identity), and every session's newest revision is
  held to a seal-only run of the stand-ins rebuilt in memory, over the range the workload asked for (the
  whole stand-in source for a revision published as it goes; the request of the job that committed a
  sealed one), never the range the revision declares: a sealed revision equals it byte for byte; an open
  one, rebuilt from its parts after the crash, is a prefix of it with nothing left to come before the time
  it is published until; a stopped one equals, byte for byte, the stopped seal of the reference run's own
  cut after the chunks it reached (its published segments, the earlier text after them and its chunk
  outcomes). Parts are listed only while a revision is open, and at most 64. **Review RV-1
  (2026-10-22):** the range and the stopped check were the revision's own and a prefix before (findings 6
  and 5); unit tests feed the verifier a stopped seal that dropped its last published segment, one that
  added an unpublished one, and a revision whose run lost its last chunk.
- **Kill points (unit level, child processes, stand-in recogniser, an ordinary root):** each of
  `artifact-install`, `manifest-rename`, `pointer-rename`, `pointer-directory-sync` and `job-committing`, at
  every part's commit and at the seal (alternating with `chunk-recognised:1` to move the run on), and at the
  first part, the run's own stopped seal and the stopped seal of a later `job cancel`. After every kill the
  verifier finds nothing lost or damaged and what is published never goes back; the resumed run seals
  exactly what a seal-only run makes, and a stopped run keeps what it published.
- **Not yet run:** the hosted `P10 durability campaign` (layers A, B and C) with this workload. Its
  workflow is unchanged, but it can only be dispatched from `main`.
- **Accepted by the maintainer (2026-10-23).** The stand-in speech is four chunks (105 s) instead of three,
  which changes the existing operations' sequence for a given seed and lengthens each retranscription by one
  chunk. The maintainer also approved one full hosted `P10 durability campaign` (layers A, B and C) with this
  workload, and one more with the negative control, after this pull request merges; the coordinator
  dispatches both from `main`. No campaign runs on the development machine.

## Note, 2026-10-28: merge rules version 2 (#539)

`MERGE_RULES_VERSION` is 2: a segment that loops on its own words claims no time at a seam
([ADR 0017](0017-local-asr-through-whisper-cpp.md), the 2026-10-28 note). The digest test's sweep and the
prefix property (`every_cut_of_a_run_settles_a_prefix_of_its_merge`) now include such segments, and the
prefix claim holds with them. Version 1 shipped in no release, so the rule above (a resume under other
merge rules seals as stopped) affects no stored revision.

## Note, 2026-11-11: a run's own commits, the writer lock, and the queue (Fable 0.3.0 review, C-2 to C-9; H5)

The planner accepted these on 2026-11-11 (`story-handoffs/H5-design-rulings.md`).

- **A run's parts are committed under the run's own permit (C-2, C-3).** X-07 makes the permit cover
  everything an attempt runs, and before this note every part commit still took one more admission unit.
  A whole-share run (`transcript retranscribe` or `job resume` in the foreground, on a default 4-unit root
  with four or more cores) was then refused every part by its own reservation. The refusal was swallowed,
  so the revision appeared only at the seal (section 1's promise did not hold, and nothing said why). A
  watched run's parts took the one unit D4 leaves free for frames. `RevisionStore::publish_part` now takes
  a typed `CommitAdmission`: `UnderPermit(&permit)` from the run's gate, which takes no unit, or `Ordinary`.
  Only part commits use it. Evidence, generation and lifecycle commits, and the seal (made once the permit
  is given back), stay ordinary. The store covers a commit only with a permit it took on the same
  canonical root; any other permit falls back to an ordinary admission, never past it. Other processes
  never see more units held than the run reserved, the queued-work marker is not read or demanded by a
  covered commit, and the permit's slots and marker are only borrowed, so the marker stays last (section 6,
  #594). The swallow of a refused part stays: what is left to refuse a part is another commit of the
  session for a moment, or a generation another command moved, and the next chunk tries again.
- **A commit waits a moment for the session's writer lock (C-2).** A frame's evidence commit that met a
  part commit holding the session's writer lock was refused `BUSY` at once. Every commit now takes the
  writer lock through one helper, which polls it every 10 ms for up to 2 s, then answers `BUSY` as before.
  The writer lock is the innermost lock, and no code waits for any other lock while holding it, so the wait
  cannot form a cycle. On a worker thread of a multi-threaded runtime the pause runs in `block_in_place`,
  so the wait never holds up an async worker. The visible cost: a command that meets a held writer lock is
  refused after up to 2 s, not at once.
- **A resumed queued job waits as `queued` (C-4).** A job of the queued mode that was interrupted and is
  resumed is moved back to `queued` (the change `Requeue`, edge `interrupted -> queued`) by its owner,
  before it waits its turn. It keeps its epoch, attempts, failures and checkpoints. `job cancel` and
  `session close` then stop it as they stop any waiting job; before, `job cancel` answered `BUSY` for as
  long as it waited. A `queued` record with an attempt behind it is reported resumable as `interrupted`.
- **Section 5's close, corrected (C-6).** "Then closes ... a runner that missed it fails its next
  publication" was wrong: a runner holds a shared hold on the session's lifetime for as long as it lives,
  which the close needs exclusively. A close that still finds a runner after its 10 s wait answers `BUSY`,
  retryable, and closes nothing; the same command closes the session once the runner has ended. The lock
  protocol is unchanged.
- **The queue (C-5, C-7).** An entry is removed only on a positive finding that its job ended (no record,
  its session gone, or a terminal state). An entry whose job cannot be read for a moment keeps its place,
  and counts ahead only within its start grace. A stamp in the future (the clock stepped back) does not
  count as "entered recently". A closed session's entries are removed by the close.
- **Records (C-8, C-9).** L-220 says which clock a run checks expiry with; L-223's mitigation says a chunk
  can take minutes with a large model on a slow processor; the progress record's comment says its contents
  are synchronised and only its directory is not.
