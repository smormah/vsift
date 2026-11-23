# ADR 0017: Local speech recognition through whisper.cpp

- Status: Accepted (maintainer, 2026-09-25); decision 4 (Ctrl-C) superseded by [ADR 0020](0020-recoverable-jobs-and-durable-publication.md) in P10 PR 3 (2026-09-27)
- Date: 2026-09-25
- Tracking: [P07 / issue #10](https://github.com/smormah/vsift/issues/10), increment 3b
- Refines: [ADR 0005](0005-r0-scope-and-qualification-profiles.md) (the CPU speech
  baseline), [ADR 0014](0014-progressive-dependency-setup.md) and
  [ADR 0015](0015-r0-delivery-replan.md) (verification before use), and
  [ADR 0016](0016-embeddable-engine-and-evidence-contract.md) (the evidence contract)
- Implements maintainer decisions D1, D2, D3, D5, D7 and D8 of 2026-09-25. D4 (`setup
  check` reports local ASR) and D6 (a quantized profile and the measured default) are
  increment 3c (delivered; see the dated note at the end).

## Context

R0 must reach grounded evidence from a video with no transcript (journey A-08). P07
increment 3a built the core: chunking, a whisper.cpp adapter, provider-output
validation, silence and a seam merge, all internal. This increment makes it a public
capability, which fixes how it is requested, how its results relate to earlier
transcripts, what the public records say, and how it fails.

## Decision

### 1. Engine and request (D1)

- The provider is the whisper.cpp CLI (`whisper-cli`), with the builds reviewed in
  P06 recognised by executable SHA-256, and the pinned multilingual `base` model
  (ADR 0005). A faster-whisper adapter is backlog issue #147; whisper.cpp stays the
  default.
- Local ASR is requested only by `transcript retranscribe <session> [--from <us> --to
  <us>]`: both range flags or neither, neither meaning the whole source. `ingest`
  (with or without `--transcript`) and `transcript get` never resolve whisper.cpp or
  a model and never run ASR. `transcript get` on a session without a transcript keeps
  `INVALID_ARGUMENT` and now carries fixed remediation naming both routes.
- Tools resolve like `setup check`: a `setup configure` selection first, then the
  filtered `PATH` (`ffmpeg`, `ffprobe`, `whisper-cli`); the model is the one
  registered with `setup configure-model`. Nothing is resolved, hashed or run before
  the range is validated.
- The engine library (`Engine::retranscribe`) is the one implementation.
  `EnginePorts::with_speech_recognizer` lets a host supply its own recognizer and
  verifier, and `with_local_asr_verifier` replaces the reviewed fixture check; passes
  from either are recorded under a separate authority.

### 2. Invocation and output

- One supervised `whisper-cli` process per chunk, with a closed argument list checked
  against v1.9.2 `--help`: `-ojf -np -l auto -t <threads> -p 1 -bs 5 -bo 5 -sns -ng`
  (decoding profile `r0-v1`: beam 5, best-of 5, language detection, no translation,
  CPU only). Threads are the machine's parallelism, at most 8, and are recorded.
- Chunk audio is written as 16 kHz mono WAV in a **private work directory inside the
  session** (`sessions/<id>/work/asr-<16 hex>`), removed when the run ends; a
  directory left by a killed run is removed by the session's next run or by `session
  clean`. The run holds the session's shared lifetime lock, so `session close` and
  `session clean` return `BUSY` while it works, and one of the session root's
  admission slots for its whole recognition (each chunk's decoding takes another), so
  concurrent runs cannot oversubscribe the machine.
- Only the `-ojf` JSON file is read: size-checked, no-follow, strict, bounded (4 MiB,
  256 segments, 512 tokens each). stdout and stderr are bounded and discarded; paths,
  the model path and system information are never parsed into a value or shown.
- Validation is the domain's (3a): segments reversed, outside their chunk's decoded
  audio or the source are rejected and counted; an end up to one second past the audio
  is trimmed with the raw end kept; more than a quarter rejected, out-of-order
  segments or probabilities outside [0, 1] fail the chunk. Confidence is the mean
  text-token probability, `provider_uncalibrated`.
- Audio: the first audio stream whose codec the media adapter decodes. A source
  without one fails with `INVALID_ARGUMENT` before recognition.

### 3. Chunks and seams

R0 chunks are 30 s windows overlapping by 5 s. Every time is the chunk's observed first
decoded sample plus the provider's offset, never the requested window start; silent
chunks (every 20 ms frame below -50 dBFS) are recorded and not transcribed; the seam
merge keeps a sentence across a seam once, removes duplicates and keeps genuine
repeats (3a). The opt-in checkpoint builds a 44.6 s clip from six speech utterances
with one crossing the 25–30 s overlap and requires every checked word exactly once
(it recorded one `seam_duplicates_removed`). A segment cut at a chunk edge is replaced
by a neighbouring chunk's segment only when that segment spans the cut segment's
midpoint; merely overlapping it does not count. Measurement for increment 3c found the
looser 3a rule could drop a sentence that starts at a chunk's first sample from both
chunks without a warning; a regression test covers it.

### 4. Model profiles by identity (D5)

A model is identified by size and SHA-256 against the reviewed pins. Only a pinned
profile runs; any other file is refused before any work with `MISSING_CAPABILITY`
and remediation naming the reviewed model. The identity is read before the first
chunk and after the last; a change fails the run (`model_changed`) rather than mixing
outputs. The provider build and model digests, profile, decoding profile, chunk plan,
threads and audio stream are recorded in every run's provenance.

### 5. Revisions, splicing and citations (D3)

- Every run commits a new, immutable, **complete** revision, numbered after the
  session's newest. Without an earlier revision a bounded run covers only its range.
- With an earlier revision, a bounded range is first widened to whole segments of the
  newest revision: every segment it intersects, and every segment overlapping those,
  is replaced whole (`replaced_range`); a range in a gap is unchanged. Segments of the
  newest revision outside the replaced range are **carried** with their original
  text, timing, speaker, confidence and origin, and the run's segments fill the range.
  A whole-source run replaces everything (`replaced_range` is the source). One
  exception, added for #353 (see the 2026-10-10 note): text of the newest revision
  that reaches into a part of the range that the run could not read, and that none
  of the run's own text overlaps, is carried too.
- Carried segments get new identities in the new revision, so records already indexed
  under an older revision are never overwritten, and name their origin in
  `carried_from` (revision and segment identity). They always name the revision that
  first produced the text, never an intermediate copy. The revision stores
  `inherited`, the provenance of every revision it carries from, so every carried
  segment is still re-checked against the rule that produced it (an import's offset
  and cue timing, or a run's chunk audio and provider times), and must lie wholly
  outside the replaced range, or be kept by the rule of the 2026-10-10 note
  (reaching into a part of it that the revision's own run could not read, and not
  overlapped by the run's own text).
- The revision identity is derived from the session, revision number, `local_asr`,
  the run fingerprint (provider, model, profiles, plan, threads, stream), the covered
  range and the superseded revision; segment identities from the revision and ordinal.
- The newest revision is the default for `transcript get`, even over an import. Every
  revision stays readable with the additive `transcript get --revision <trv_id>`, so
  an older citation always resolves. Cursors were already bound to the revision.
- Stored as `transcript_record` version 2. A revision that carries nothing is written
  exactly as 3a wrote it; imports still write version 1 byte for byte.

### 6. Evidence stream (D7, D8)

- No tombstone events. A superseded revision is not deleted evidence: its records stay
  valid, immutable and resolvable, and consumers filter on `revision_id` against the
  revision they want (the newest is in every `transcript get` result).
- No progress events in this increment; readers already skip unknown event kinds.
- `transcript retranscribe --events jsonl` writes one terminal event, like every
  command but `transcript get`: a whole-video revision can hold far more records than
  one bounded stream carries. Its records are read page by page with `transcript get
  --revision <revision_id> --events jsonl`.

### 7. A run that hears no speech

It commits a revision with no new segment in its range and the typed warning
`no_speech_recognised`, keeping every chunk's outcome. Outside a bounded range the
earlier revision's segments are carried as usual. Discarding the attempt would hide
that the audio was examined; an empty revision records it. The domain now allows a
local-ASR revision without segments; an import still needs at least one cue.

### 8. Failure mapping

Existing failure codes only (`FailureCode::ALL` unchanged). A failed run is `AsrFailure
{ stage, reason }`, presented as fixed prose naming both identifiers and never a path:

| Cause | Code |
| --- | --- |
| whisper.cpp, FFmpeg or FFprobe missing; model not registered or unreadable; unpinned model; recognizer failed, unparseable or malformed output, invalid timestamps or scores; model changed mid-run; a recognizer process that cannot start | `MISSING_CAPABILITY` |
| No decodable audio stream; empty, reversed or out-of-source range; unknown `--revision` | `INVALID_ARGUMENT` |
| Audio stream present but undecodable | `INVALID_SOURCE` |
| Output bound, too many chunks (over 1,024) or segments, record over 24 MiB, abnormal recognizer termination (a signal or an NTSTATUS crash, usually memory) | `RESOURCE_LIMIT` |
| Deadline (120 s per chunk) | `DEADLINE_EXCEEDED` |
| Cancelled | `CANCELLED` |
| Work directory, or the session's source copy or decoded audio unreadable | `STORAGE_IO` |
| A renewal or another revision committed during the run (generation race); session held by cleanup; every processing slot of the session root in use (a run holds one for its whole recognition) | `BUSY` |
| Assembly invariant violated | `INTERNAL` |

### 9. Functional verification before use

Before the first media stage, after the media-tool preflight, the engine proves the
selected recognizer works: `FixtureAsrVerifier` embeds `F01-speech.mp4` (72,990 bytes,
SHA-256 `f8222a92…e881`), stages it in a private workspace, decodes its speech with
`speech_pcm`, runs the recognizer with the same profile and applies the same parsing,
validation and merge. It passes only if the normalised transcript contains `service
status`, `healthy` and `2048`, every segment starts within 0.5 s of the recorded speech
span (0.5–4.675 s) and ends no later than one second after it. The end bound is the
domain's provider end tolerance rather than the 0.5 s first proposed, because the
reviewed build ends F01's segment at 5.26 s, 0.585 s after the speech.

A pass is recorded in the per-user verification record (digests and times only), in
its own fingerprint domain binding the media-tool pair's fingerprint, the whisper
executable's canonical path and on-disk identity, the model's canonical path and
on-disk identity, the recognizer identity (executable and model SHA-256, profiles,
threads), the R0 chunk plan, the fixture digest, host isolation, the verifier's
authority, a verification profile version and the VSift version. Passes age out after
seven days like media-tool passes. A failure writes nothing and is typed
(`fixture_integrity`, `workspace`, `fixture_media`, `transcription` with its stage and
reason, `unexpected_transcript`).
`setup check` reports it (D4; see the increment 3c note below).

## Decisions recorded for maintainer confirmation

The maintainer confirmed all six on 2026-09-25.

These were not settled by D1–D8; each follows the existing contracts most closely.

1. A run that hears no speech commits an empty revision with `no_speech_recognised`
   (section 7) rather than failing.
2. The first decodable audio stream is transcribed; a source without one is
   `INVALID_ARGUMENT`, an undecodable one `INVALID_SOURCE`.
3. `retranscribe --events jsonl` emits only the terminal event (section 6).
4. The CLI does not trap Ctrl-C: the default handler ends the process, the supervisor
   kills whisper.cpp with it, nothing is committed, and the work directory is swept
   later (superseded by P10 PR 3; see the second 2026-09-27 note). The engine's `Cancellation` is honoured between stages for library hosts.
   Trapping signals needs Tokio's `signal` feature, a dependency change left for a
   separate review.
5. The verification's segment-end bound is one second (section 9).
6. A range reaching past the end of the source is `INVALID_ARGUMENT`, never clamped.

## Consequences

- A-08 is reachable: a plain ingest followed by `transcript retranscribe` yields
  citable, self-describing evidence with full provenance, and a bounded rerun never
  invalidates an earlier citation.
- The v1 transcript schemas are widened in place (D2); imports and their frozen
  examples are unchanged. Version-2 records are part of the published bundle schema.
- The first retranscription with a new tool, model or VSift version costs one
  fixture transcription (about 6 s on the reference machine) plus the media-tool check.
- whisper.cpp output is deterministic on one host but not across CPU backends: ggml
  picks an optimised backend for the CPU at load time (for example AVX2 or AVX-512),
  and their floating-point results differ slightly. On 2026-09-26 an Intel AVX-512
  hosted runner heard F05's "invoice" as "in voice" with `base`, where an AMD runner
  and the reference machine heard "invoice". Content-derived revision ids therefore
  differ between such hosts, as their transcripts do; the journey tests check only
  words every reviewed host hears, and T-04 accuracy is measured separately.
- Known limits, measured on Windows 11 with the reviewed build: a short clip takes
  about 7.5 s end to end with verification cached (release build); the model is hashed
  three times per run (about 0.3 s each in release, so no identity cache was added).
  The base model starts a segment that follows leading silence at the start of its
  audio (F09's segment starts at 0.75 s, not 4.0 s); accuracy and timing are measured
  in 3c (T-04). Every chunk rehashes the session's source copy before FFmpeg reads it
  (the P04 check-before-use rule), which is linear in source size per chunk; long
  sources need a cheaper binding before the P14 load gates. (Resolved on 2026-09-26,
  issue #148: a run now hashes the source copy twice in total; see the note at the
  end.)

## 2026-09-25 note: increment 3c delivered D4 and D6

- **D4.** `setup check` has an additive `local_asr` object: the registered model's
  identity (`not_selected`, `unreadable`, `unrecognised`, `known_pinned` with its
  profile) and the local-ASR verification of section 9 (`verified` from a `recorded`
  pass or `ran_now`, `failed` with a typed `check` and `reason`, or `not_run` with
  the first missing piece in resolution order). With no recorded pass it runs the
  media-tool and local-ASR preflights with the tools selected for the check, under
  its own 60 s budget (`DEFAULT_LOCAL_ASR_CHECK_BUDGET`, separate from
  `--timeout-seconds`); a run past it is cancelled and reported as `budget` /
  `budget_exceeded`. It writes only the verification record, only for a pass. The
  legacy `verification_scope` and `local_asr_model` constants and the exit status
  are unchanged. With a host-supplied recognizer the model is the one it reports.
- **D6.** A second reviewed profile, `base_q5_1` (`ggml-base-q5_1.bin`, 59,707,625
  bytes, SHA-256 `422f1ae4…a8898`). Hugging Face revision `80da2d8` (the base pin)
  has no quantized files, so this pins revision `5359861` of the same repository,
  where `ggml-base.bin` is byte-identical to the base pin; the q5_1 LFS SHA-256 was
  verified against a download from that revision. The maintainer accepted this pin
  revision on 2026-09-25. Section 4 applies to both profiles: the profile is decided
  by file identity, recorded in `model_profile` and bound into the verification
  fingerprint, so a pass for one profile never stands in for the other. Only `base`
  is in the managed plan.
- **T-04 and the default (decided 2026-09-25).** `base` stays the default. The
  opt-in `p07_asr_qualification` test ([record](../planning/p07-asr-qualification.md))
  enforces, for `base`, at most 10% pooled word error rate on clips without added
  noise and every spoken critical term (noisy F08 included) except the reviewed
  known misses. F08's word error rate is reported as a known limitation and not
  gated until a noisy-speech fixture set exists (issue #150); real-time factor and
  peak memory are reported. `base` passes these gates
  ([ADR 0005 note](0005-r0-scope-and-qualification-profiles.md)). The measurement
  also found the seam-merge case now described in section 3.

## 2026-09-26 note: the per-chunk source rehash is gone (issue #148)

A run no longer rehashes the session's source copy before every chunk. It binds the
copy for the whole run ([ADR 0012 note of 2026-09-26](0012-p04-source-media-profile.md)):
one full SHA-256 verification when the committed copy is opened, an on-disk identity
comparison (size, modification time, file identity and platform change fields)
before the probe and before each chunk's decode, and a second full verification
after recognition and before the revision is committed. The source is therefore
hashed exactly twice per run, whatever its length; on an 869 MB, 24-chunk source the
decoding stages took 25.5 s instead of 173.4 s. The local-ASR fixture check binds its
staged fixture the same way. Failure mapping (section 8) is unchanged: a copy that
changed is `STORAGE_IO` at a chunk's decode, as before, and `INTEGRITY_FAILURE` at the
new closing verification; nothing is committed. The source-integrity residual of ADR
0012 is unchanged.

## 2026-09-27 note: recoverable retranscription (P10) and decision 4

Since P10 PR 2 ([ADR 0020](0020-recoverable-jobs-and-durable-publication.md), accepted
2026-09-27) a retranscription is a recoverable job. Each chunk's raw recognizer output
is kept as a private checkpoint in the session, so running the same request again after
an interruption (a crash, a failure, a cancellation) continues from the finished chunks
and commits the revision an uninterrupted run would; resume information is never
written into the revision. The result's data names the job (`job`), and the envelope
the operation id it is recorded under.

Decision 4 still describes the command-line host in PR 2: Ctrl-C is not trapped, the
default handler ends the process and nothing is committed; the run's finished chunks
now survive it, and the next run of the same command resumes from them. P10 PR 3
supersedes decision 4: the CLI traps Ctrl-C and SIGTERM through Tokio's `signal`
feature and turns them into cancellation serialized with the commit (ADR 0020
section 5).

## 2026-09-27 note: decision 4 superseded (P10 PR 3)

Decision 4 is superseded by ADR 0020's PR 3 notes. The CLI now traps the first
`SIGINT`/`SIGTERM` (Unix) or console Ctrl-C/Ctrl-Break (Windows) during a long command
and cancels it: a retranscription stops at its next boundary (the supervisor stops
whisper.cpp gracefully, then kills it), commits nothing, keeps its job `interrupted`
with its finished chunks, and answers `CANCELLED` (exit 6) naming the session and job
and suggesting `vsift job resume <job>`. A second interruption kills providers without
the graceful wait; the process still exits only after they are reaped. The dependency
review of Tokio's `signal` feature is recorded in ADR 0020. The rest of this ADR is
unchanged.

## 2026-09-28 note: section 6 "no progress events" superseded (P11 PR 1)

The bullet of section 6 (D7) that deferred progress events is superseded by
[ADR 0021](0021-worker-and-batch-host.md) section 7. `transcript retranscribe
--events jsonl` (and `job resume`) now writes `progress` events before its one
terminal event: stage `recognising_speech` in `chunks`, 0 of the planned chunks once
the plan is made, then every chunk (reused ones included), at most one per second and
4,096 per run, advisory and dropped rather than blocking a slow reader. The terminal
event follows at the count of events before it; its result is unchanged. The records
of a revision are still read with `transcript get --revision <revision_id> --events
jsonl`. The rest of this record is unchanged.

## 2026-10-04 note: the one-second end tolerance superseded (P14 PR 7, #274)

The bullet of section 2 that trims "an end up to one second past the audio" and rejects a longer one is
**superseded**, and so is the sentence of the verification paragraph that took the provider end tolerance from the
same figure for chunks. The P14 load campaign found that a range cut mid-speech (`transcript retranscribe --from 0
--to 5000000` on three of the ten synthetic speech clips) failed as `MISSING_CAPABILITY`, `malformed_output`, with a
remediation to reinstall whisper.cpp. Reproduced here with the reviewed whisper.cpp v1.9.2 and the `base_q5_1`
model: the recogniser ended the last segment of a 5.001 s chunk at 7.000 s, 6.100 s and 6.000 s on three clips. A
recogniser's end timestamps are predicted tokens, quantised coarsely on a small model, and are not bounded by the
audio's length (a segment may end anywhere in the padded 30 s window); a second was a guess from one clip (F01's
reviewed-build end was 0.585 s past the speech). A long run hid it, because one rejected segment among many is under
the quarter threshold and is only counted; a short range has one segment, so one rejection is all of them and the
chunk failed.

**The rule now:** a segment that **starts inside** the chunk's decoded audio and ends past it is cut to the audio end,
as far as the padded 30 s window the recogniser works in, and counted under the existing `provider_end_trimmed` warning
with the raw end kept beside it (`end_trimmed`, `provider_end_us`). The window is the sanity bound (a recogniser's ends
may fall anywhere in it and nowhere beyond it): an end beyond 30 s from the chunk's start is not a time of this audio and
rejects the segment. The bound is never below one second past the audio, the figure 0.1.0 applied to every chunk, so a revision that
0.1.0 stored (a full window with an end a few hundred milliseconds past it) still rebuilds. A cut end is the audio's end, not evidence that speech continued there. A segment
that starts at or after the audio's end, runs backwards or is empty is still rejected and counted, and the quarter
rule is unchanged for those. Seam stitching is unchanged: a cut final segment of a middle chunk is a cut segment like
any other (the neighbour that heard the sentence whole supplies it; a multi-chunk test pins no repeat, no gap and no
step backwards). Field names and shapes of v1 do not change (the schema's description text does); the
fixture-verification tolerance of section 3 (`setup check`: a segment ends no later than one second after the
recorded speech) is a different check and is unchanged. The failure code of a chunk whose segments mostly do not fit
their audio stays `MISSING_CAPABILITY` (changing a published answer is not additive within v1), and it remains the only
signal of a recogniser answering with garbage for a whole run; its remediation now names the ways a segment is rejected,
says a range that ends mid-speech can cause it, and tells to retry with a larger range or the whole video, with the
reinstall step only if the whole video fails the same way. The `setup check` verification of the built-in clip, which has
no range to widen, keeps the reinstall remediation.

**Rolling back is not safe for a session that holds such a revision.** A trimmed end more than one second past its audio is
valid only from this version on: 0.1.0's `TranscriptRevision::new` refuses it as `AlignmentMismatch`, a load reports that as
`INTEGRITY_FAILURE`, and every command that reads the revision fails. The record is not damaged and nothing is migrated; use
the newer version, or discard the session (sessions are disposable unless persisted). The behaviour is recorded as
[L-130](../planning/known-limits.md#l-130).

## 2026-10-08 note: a floor of 100 ms before the recogniser and before the decode (P14, #322, #332)

Section 3 says a chunk is recorded and not transcribed when it is silent, and section 2 says a chunk with no audio is a
gap. Neither set a least amount of audio, and nothing upstream does: a requested range has no minimum, the last chunk of a
plan is what is left of its range, and the decode returns only what the audio track holds in a window. Two defects followed.

- **The recogniser was handed any length** (#322). The by-hand scan reading of 2026-10-07 found that the reviewed whisper.cpp
  v1.9.2 reads up to 200 samples past its buffer when it is given fewer than 201 (fixed upstream in v1.9.3) and, by a reading
  of its source, that its command line fails for 40 or fewer ([L-137](../planning/known-limits.md#l-137)). That a short
  audible range reached the recogniser is now shown, not only read: with the rules below taken out, an application test and an
  engine test over a real decode both show the recogniser called for it.
- **The decoder was handed any length** (#332, found while testing the first). FFmpeg cuts its output by a count of samples
  and takes a length that rounds to none for no limit at all (`libavfilter/trim.c`:
  `duration_tb = av_rescale_q(duration, AV_TIME_BASE_Q, tb)` rounds to the nearest, and 0 means no duration was given). What
  came back then was one whole filter frame instead of the range: with the decode's `asetnsamples=n=65536`, up to 65,536
  samples of the source, less when less audio is left (seen with FFmpeg 9.0 on a 6 s, 16 kHz clip: 65,536 samples from its
  start and from 1 s, and from 2 s the 64,000 that are left; about a second and a half of a 44.1 kHz track). So a
  `transcript retranscribe` range of a few microseconds was recognised like a long one, its segments lying after `--to`,
  and an `audio` clip of such a range held that frame.

**The rules now.**

1. **A planned chunk whose window is shorter than 100 ms is a gap and is not decoded** (`MIN_RECOGNITION_MICROS`,
   `PlannedChunk::is_below_recognition_floor`). It is recorded as `no_audio`: nothing was decoded, so there is no decoded
   range to record. With the R0 plan only a requested range under 100 ms gives such a chunk (a property test: the last window
   of a longer range is longer than the 5 s overlap).
2. **Decoded chunk audio of fewer than 1,600 samples (100 ms at 16 kHz), whatever its level, is a gap and is not given to
   the recogniser** (`MIN_RECOGNITION_SAMPLES`, `is_below_recognition_floor`). It is recorded as `silent`, with its decoded
   range. This is the case of a window of 100 ms or more of which the audio track covers a sliver.
3. **No audio decode is asked for a length that rounds to no sample** (`rounds_to_no_pcm_sample`: under half a sample at
   16 kHz, so 31 microseconds or less). The media adapter refuses it before FFmpeg runs, for a speech chunk and for an
   evidence clip alike, so a decode of zero length cannot be asked for. Rule 1 keeps a speech chunk from ever meeting that
   refusal; the `audio` command answers it as `INVALID_ARGUMENT` with its own remediation. The rule follows FFmpeg's
   rounding on purpose: FFmpeg cuts by the count of output samples rounded to the nearest, so a range of 32 to 62
   microseconds rounds to one sample, was answered with one, and stays answered. Refusing every range under one whole
   sample (63 microseconds) would not depend on where the tool rounds, and was not taken (the maintainer's decision of
   2026-10-08: refuse only what was wrong), because it would refuse requests that were answered correctly.

All three rules are the domain's, applied in the use cases and, for the third, again at the decode boundary. 100 ms is the
figure under which whisper.cpp itself decodes nothing (ten frames of spectrogram, "input is too short"), so no chunk that
could have produced a word is skipped; and the rules stand in front of every recogniser, so they also cover a recogniser a
user installed, which no review covers. A range that is all gaps is a run that heard nothing (section 7): it commits a
revision with no new segment and `no_speech_recognised`, and exits 0, where such a range could fail as `MISSING_CAPABILITY`
or be recognised as another range before.

**No new identifier.** The short chunks are stored under the two existing outcomes and counted under `silent_chunks_skipped`.
A separate outcome or warning would be more exact and was not taken: both are stored in the revision record, where every
earlier release decodes an unknown value as damage (`INTEGRITY_FAILURE`), so a session holding one short chunk could no
longer be read after a roll back (the cost L-130 already carries for one case); and nothing a caller does differs between a
quiet tenth of a second and a short one. The cost is that `silent` and `no_audio` each name two conditions, and the fixed
sentence of the warning ("no audible signal") is loose for a short loud chunk; the contract and the schemas' descriptions
say so. Field names and shapes of v1 do not change (three descriptions do).

**Not changed:** the chunk plan, the range flags of `transcript retranscribe` (no minimum is added: a short range is
answered, not refused), the silence rule, the decode's arguments, the pinned whisper.cpp (its re-pin stays with the FFmpeg
refresh, L-132) and the recognition key of a job.

**Checkpoints.** A resumed job judges the floor before anything it stored. For a window under 100 ms the only checkpoint
that is reused is the gap rule 1 gives; one that an earlier version stored with recognised output, or as silent with a
decoded range, is discarded and counted as such, and the chunk becomes the gap. Otherwise a job resumed across the upgrade
with a recogniser the user installed (whose output is not re-verified) would have brought back the answer the rule removes.

**What `audio` changes.** A range of 31 microseconds or less was answered with a clip that held a filter frame of
seconds and reported as a success; it is now refused. That is the one request whose answer was wrong. Where such a range
decoded to nothing (at the end of a clip, or on the audio-only fixture) its answer was `INVALID_SOURCE` and is
`INVALID_ARGUMENT` now: one published code replaced by another, for this request only. A range of 32 to 62 microseconds
keeps its answer, a clip of one sample. The cost of following the tool is that rule 3 is right only while FFmpeg rounds a
length to the nearest sample at the 16 kHz output. That was measured with FFmpeg 9.0 on Windows (the corpus clips and WAV
sources of 8 to 96 kHz: 31 microseconds is no limit, 32 is one sample, whatever the source's rate) and with the reviewed
managed build (FFmpeg `n9.0.1-11-ge47273f4d9-20260831`, hosted run 37719064583 of 2026-10-08: the three opt-in tests passed). The opt-in tests that run a real
FFmpeg pin 31, 32 and 62 microseconds; they are the only check of it and the one to run when FFmpeg is refreshed (L-132). A bound
after the decode (no more samples than the range and one) would make the rule independent of the tool; it is a follow-up,
not part of the third candidate (L-141).

## 2026-10-10 note: a chunk whose answer cannot be used is a recorded gap, not a failed run (#353)

**Proposed for the maintainer to confirm by merging this change.** It supersedes the part of section 2 that failed a chunk when more than a quarter
of its segments were rejected and the part of section 8 that made any such chunk fail the run, and it amends the 2026-10-04 note's last
sentence (the failure of "a chunk whose segments mostly do not fit their audio ... remains the only signal of a recogniser answering with
garbage for a whole run").

### What happened

(Issue [#353](https://github.com/smormah/vsift/issues/353); known limit L-145, deleted from the register when `0.2.1` was published.) The first real recording tried with the stable `0.2.0` (a 34:36 English screencast of slides and live coding, one speaker, long pauses while
the speaker types) failed as `MISSING_CAPABILITY`, step `output_validation`, reason `malformed_output`, after 65 of 83 chunks were saved.
Nothing was committed. The cause was one 30-second chunk, 27:05 to 27:35, which held a few segments of which one was rejected. Three
defects stacked:

1. **One unusable chunk failed the whole run.** The 82 usable chunks were thrown away, and because the chunk fails the same way every time, a resume
   could never get past it.
2. **The quarter rule weighed nothing in a sparse chunk.** With three or fewer text segments, one rejected segment is already more than a quarter,
   so a chunk was failed for having a single rejection beside any number of kept segments. The note of 2026-10-04 says a long run hides a
   rejection "because one rejected segment among many is under the quarter threshold"; that holds for dense speech and not for a recording with pauses,
   where whether a rejection is a warning or the loss of the transcript depended only on where the chunk boundaries fell.
3. **The failure did not say what failed.** The code is `MISSING_CAPABILITY` and the capability was present; the remediation was the #274 text (retry with a
   larger range, then reinstall whisper.cpp), which cannot help; the failure named no job, chunk or time; and the job stayed `resumable: true`.

The same audio transcribed when the chunk was cut five seconds earlier or later, and the rest of the recording, about 150 other chunk windows, transcribed.

### The decisions

**1. A chunk whose answer cannot be used is a recorded gap, with an outcome of its own.** `AsrChunkOutcome::Unusable`, stored and presented as `unusable`.
A chunk becomes one when the output rules refuse its answer as a whole: every text segment rejected, two or more rejected of fewer than four, more than a quarter
of at least four rejected (decision 3), or a structural fault (out-of-order segments, a score outside 0 to 1, oversized output). It is a decoded chunk, so it records its decoded range like a
silent one. It is never recorded as `silent` or `no_audio`: those say nothing was there to recognise, which would be false of speech the
recogniser could not place, and a search would report the window as "no speech" instead of untranscribed. The merge treats it as a chunk that heard nothing, so
speech that a neighbour heard in the overlap is kept. The run counts such chunks under a new warning, `provider_chunks_rejected` (count of chunks, first chunk
ordinal).

**2. A run fails only when most of the chunks the recogniser answered are unusable.** `unusable_chunks_end_the_run`: more than half
(`UNUSABLE_CHUNK_SHARE_DENOMINATOR = 2`). It was chosen like this.

- *More than half,* because what the rule is for (a recogniser that answers with garbage, such as one run on audio it cannot read) fails most chunks, and a real
  recording with pauses fails few: 1 of 83 here. A broken recogniser is also caught by the check VSift runs on a reviewed clip before it touches the user's video.
- *Chunks that were never given to the recogniser do not count.* Silent chunks, chunks under 100 ms and chunks with no audio say nothing about it. A recording that is
  mostly quiet is judged by the few chunks that had speech.
- *A strict majority.* One bad chunk of two is a partial run: the other chunk was answered by the same recogniser, build and model on audio of the same recording, which
  shows it works, and the one unproven chunk is reported as a gap. Every answered chunk unusable always fails, so a lone chunk that cannot be placed (a short range)
  still fails as it always did, and the failure says why.
- *The verdict is known as soon as it cannot change,* so a recogniser that answers with garbage is not run to the end of a long recording: the run stops when the
  unusable chunks are more than half of the chunks answered plus all that remain. The result is the verdict the full run would give, reached sooner.
- *Consecutive failures were considered and not taken as a second rule.* A stretch of typing or music can have several unusable chunks in a row, and chunks overlap by
  five seconds, so adjacent chunks are not independent samples; only the share of answered chunks separates a broken recogniser from such a stretch.

**3. The quarter rule judges a share only where a share means something, and the cut-over is monotonic.** `MIN_SEGMENTS_FOR_REJECTION_RATIO = 4`. With four or more text
segments the rule is exactly what it was (more than a quarter rejected makes the chunk unusable), so a chunk of dense speech is judged as before and every chunk that
passed before passes the same way. With one to three text segments **at most one rejected segment is tolerated**: it is dropped and counted under
`provider_segments_rejected`, and the others are kept; two or more rejected, or every text segment rejected whatever the count, make the chunk unusable. The allowance of one
is what keeps the rule monotonic across the minimum. Tolerating any number of rejections below four (failing only a chunk of which all were rejected) would keep two rejected
segments among three and fail the same chunk when a valid fourth segment was added (two of four is over a quarter): more evidence would make the chunk worse. "At most one,
then a quarter" has no such inversion: for every segment count from one to sixteen, a usable chunk stays usable when a valid segment is added, and an unusable one stays
unusable when another segment is rejected or a rejected one is added. A property test checks every pair of counts up to sixteen against the rule in closed form. *Why four:*
below it a share means nothing (one rejection beside two kept segments is a third), and four is the smallest count from which the quarter rule alone agrees with the
allowance for every count above it (one rejection of four is exactly a quarter and is tolerated; a search over the counts in a test finds that four, and no smaller number, is
that point). A segment with only whitespace or a marker such as `[BLANK_AUDIO]` is removed
and counted as a marker, not a rejection, and is not a text segment. The reasons a segment is rejected (empty or reversed range, start at or after the audio's end, an end
beyond the padded window, outside the source) are unchanged.

**4. A run with gaps is `partial`.** The revision is committed. `transcript retranscribe` and `job resume` answer `status: partial` (exit 0, as for a search with gaps),
the envelope's `coverage` lists the gaps as `<from_us>-<to_us>` with the reason `untranscribed_range`, and the data gain `untranscribed_ranges` and
`revision.local_asr.unusable_chunks`. A gap is the part of an unusable chunk's window that no neighbour's window transcribed or found silent. The result lists the whole gap,
because it states what **this run** did not transcribe. A later `search` of the revision lists less where text was kept in a gap (decision 7): the coverage rule is shared with
the search and never counts an earlier revision's windows inside a window this run examined, so a replaced part is never reported as covered, but it does count text the revision
kept as transcribed. The result's list and the search's list therefore differ exactly where kept text lies, and the warning says so. A run with no gap is presented exactly as
before: status `complete`, `coverage` null, none of the new members.

*Why the reason is the published `untranscribed_range` and not a new value.* It is honest (the range has no transcript), it is the reason a search gives for the same
range, and the agent skill copies reasons from results into its handoff from a closed list, so a new reason would break an agent that follows the skill as released. What is
specific to this cause is in the data, the chunk outcome and the warning.

**5. The failure says what is true, and a job that cannot succeed is not resumable.** When a run fails because most answers were unusable, the code stays
`MISSING_CAPABILITY` (a published code does not change within v1; the remediation carries the fix). The remediation keeps the first sentence ("failed at the
`output_validation` step (`malformed_output`)"), then names the reason (`too_many_rejected_segments` or the structural one), how many chunks of how many had been answered
when the run stopped, and the first by position, by `H:MM:SS` and in microseconds, the unit of `--from` and `--to`; the envelope's `affected_ids` names the job.
For rejected segments it says the tool works (VSift's own check passed before the run) and then says only what the numbers establish. With **more than three** chunks
answered (`FEW_ANSWERED_CHUNKS = 3`; most of many answers unusable is a statement about most of the speech the run covered, not about one stretch; the run's range, not the recording, is what it can speak of) it says that most of the speech the run covered could not be
transcribed reliably and tells to use a transcript the user has with `ingest --transcript`. With **three or fewer** answered (the run cannot tell a bad stretch from a bad
recording, and a different range cuts the audio at other points, which changes what a chunk contains) it says that this stretch of the recording could not be transcribed
and tells to try a slightly different range with `--from` and `--to`, or the same transcript. Neither text says that another range will not help (no run has established
that) and the second says only that it may. For a structural fault the recogniser itself is the suspect and the reinstall step
stays. The job ends as `failed` at once, so `job status` says `resumable: false` (`resumable_reason: failed`) and `job resume` refuses; the same command run again starts
the job anew. Before, it stayed resumable until the poison rule's third identical failure, which a resume could only repeat. The setup check's verification of the built-in
clip has its own text for every reason, which names no chunk, range or `--from`/`--to` (the clip is whole and reviewed; the tool is the suspect) and ends with the reinstall step.
The model identity is checked before the verdict on the answers, so a recogniser swapped during the run is reported as `model_changed` (which a retry can fix) and not as
unusable answers (the answers of two models are not the evidence of either).

**6. Checkpoints keep a verdict, not the refused answer.** A job's checkpoint for an unusable chunk is a new kind, `unusable` (decoded range and reason), so a resume neither
asks the recogniser about the chunk again nor brings a refused answer back. A `recognised` checkpoint is unchanged: stored output that the rules now reject is still damaged
or forged and is discarded and recognised again (S-08). A checkpoint of the new kind that a release before this one finds is an unusable checkpoint to it, which it discards
and redoes, so it is safe to roll back.

**7. An unreadable part keeps the text the session already had there (refined after a second review).** (A decision for the maintainer to confirm.) A range retranscription replaces whole segments of the
newest revision with what the run transcribes. When the run could not read part of its range, replacing the old segments there would delete text the session had, and the
user who asked for a better transcript would end up with none for that stretch: a bounded retranscription of 10 to 65 s over an imported transcript with cues at 5, 12, 40
and 60 s, whose first chunk (10 to 40 s) is unusable and whose second (35 to 65 s) is read, would lose the cue at 12 s. **The rule, in one sentence** (`EarlierTextRule`): a segment of the
superseded revision that lies in the replaced range is **kept**, whole and with its original provenance and `carried_from`, when it reaches into a part of the range the run could not read (a gap, decision 4)
and none of the text the run itself wrote overlaps it; every other segment in the range is replaced, and a segment outside the range is carried as always. So the cue at 12 s is kept, the cues at 40 and
60 s (inside the part the second chunk read) are replaced by what it heard, and the cue at 5 s was never in the range.

*What this decides, and why.* The first version of the rule kept only a segment wholly inside a gap, and a review found that a cue that crossed the gap's edge by a single microsecond was deleted, nearly fifteen
seconds of it in audio nobody read again. The edge of a gap is the edge of a neighbouring chunk's window, so a sentence there is read only in part, and a run replaces what it read in full and no more. Three cases follow.
(i) The run's own text overlaps the segment: the run has heard the same words, and its text replaces the segment, as it replaces any other (the overlap is strict: text that only touches the segment does not replace it).
(ii) The run read the audio past the edge and found it quiet or empty, and wrote nothing over the segment: the segment is **kept whole**. The alternatives were to drop it, which deletes text on the strength of the edge of a
window beside an unread one (the chunk overlap exists so that a neighbour answers for a sentence at an edge, and here the neighbour is the unread one), or to keep it by a fraction or a tolerance, which is a number with no derivation and a
cliff at it. Keeping errs towards the text the session had; the cost is that earlier text may remain over audio a run read as empty, which only a run that reads the whole of it can settle. (iii) The segment lies wholly where the run
read: it is replaced, by text or by silence, as before. The rule has one known cost: where the run's own text overlaps a segment that is mostly in the gap (a fragment of its tail heard by the neighbour), the
head of the sentence is lost with it; the alternative, keeping both, repeats the words.

The rule is the domain's, applied by the use case that builds the revision, by the revision's validation (so a stored record cannot place a carried segment where the rule would not, and cannot drop one where it would
keep it), and by the coverage; and it applies to a whole-source retranscription over an earlier revision as to a bounded one. What is said about it: the result lists the **whole** gap as not
transcribed by this run (`untranscribed_ranges`, the envelope's `coverage`), the warning says what the rule is (earlier text that reaches into the gap is kept whole, unless this run's own text overlaps it; any other
words said there cannot be found), and a `search` of the revision counts the kept text as transcribed (so for the example above it lists 10 to 12 s and 14 to 35 s, not 10 to 35 s). A run that could not read a chunk also no
longer claims `no_speech_recognised`: that statement is about the whole range, and the run has not heard that there was none.

**8. A part some run could not read stays untranscribed through every later run, and the revision records it where that matters.** (A decision for the maintainer to confirm; it adds an optional member to the stored record, so it is
stated here as a change to the record and not only to a search.) A revision keeps the provenance only of the runs whose text it carries (`inherited`). A second review showed what follows: after a run leaves 10 to 35 s unread, a
later run over 0 to 8 s carries the earlier text from the whole-source run (or from the imported file), whose windows, or the file's whole source, were then counted over the part the intermediate run had left, so a search
said the whole source was transcribed. The run that left the part is not in the later revision (it may have no text to carry), so nothing in the record said so. **The rule** (the module documentation of the coverage): a revision
covers an instant only where it holds a segment or a run it still records read the audio, and an earlier run's window is never counted inside a range a later run replaced, nor inside a part the superseded revision did
not itself cover. To make the second half computable, **a spliced revision records the parts its superseded revision did not cover, outside the range it replaced, that a window or file it carries would otherwise be counted over**
(`TranscriptRevision::carried_untranscribed`, the optional record member `carried_untranscribed`, at most 1,024 ranges; where there would be more, the nearest on each side of the replaced range are joined, never across it, which marks
more untranscribed and never less).

*Only what is needed is recorded* (a third review found that the first version recorded all that the superseded revision left untranscribed, including parts no run ever examined, and so wrote the member for chains that never had an unusable
chunk, such as one range retranscription after another with a gap between them; the previous release reads such a record as damaged, which the rollback statement below denied). The member is a mask over what the carried windows and files claim, so
a part that nothing carried claims needs none: a window claims only what a run examined, and a part some run examined and the revision does not cover was left by a run that could not read it (an unusable chunk), or is masked by a record
that exists for such a part. A chain that began with a range, or whose runs are apart, or in which every chunk was read, records nothing and is written as it always was. A test checks both halves over every chain of up to three retranscriptions from a grid of
ranges and chunk answers from five starting points (two that began with a range, one whole-source run with an unusable chunk, one without, one imported file): a chain with no unusable chunk never has the member, and a search says the same as it would
with everything recorded.

It is validated when a record is read (in order, merged, inside the source, outside the replaced range). Records written before this change have none, and are read as they were, with the coverage they had: the derivation never trusts an absent
list to mean "covered". The coverage can still understate (a run none of whose text a revision carries is not counted), never overstate; a test over those chains checks by an independent count that no second is claimed that no recorded run read.
**Rolling back:** a record with the member is read as damaged by `0.2.0` and earlier, and only a session whose chain holds an `unusable` chunk has one. *(Amended 2026-10-11, #359: also a session whose chain holds a stretch of words that no segment holds, because the member is written for such a stretch under a window or file a later revision carries; see the note of that date.)*


**A swapped recogniser and a resumed job.** The recogniser's identity is read before the first chunk and after the last (and, now, before a failure on the answers is reported), not before each chunk, so a swap during a run is
detected when the run ends. Checkpoints written after the swap are stored under the recognition key of the model the run started with, so if the original model is put back and the job resumed, they are reused: a
`recognised` output of the swapped model would be committed under the original model's provenance (a limit of the checkpoints from before this change), and an `unusable` verdict of it makes the resumed run fail as
unusable answers, ending the job as failed, after which the same command starts it afresh and works. Not changed here: a per-chunk identity check hashes the model for every chunk, and ending the job on `model_changed` changes
a published behaviour; it is listed for the register. *(Superseded 2026-10-17, #361: the identity is now read after every answer; see the note of that date.)*

### What does not change

The chunk plan, the 100 ms floor, the silence rule, the other warnings, the field names and shapes of v1 (this adds optional members), the failure codes, and the
verification of the built-in clip.

**What a worker supervisor sees (`job run`, `job batch`).** A `retranscribe` step reports the revision it committed and the job behind it, and nothing about that revision's
gaps: its status is `complete` and its `coverage` is null even when some chunks of its run were unusable, and so is the request's. This is deliberate for 0.2.1, to keep the
worker schema unchanged, and it means a supervisor that reads only `job run` is **not told** that part of a recording was not transcribed. It can tell by reading the
revision the step names: `transcript get --revision <revision_id>` shows the warning `provider_chunks_rejected` and the count of chunks with an unusable answer
(`revision.local_asr.unusable_chunks`), and a `search` of the session lists the ranges. `transcript retranscribe`, `job resume` and the other direct commands do answer
`partial`. A test pins the step's shape (`worker_contract`), and the destructuring in it stops compiling if a member for the gaps is added, so the change is made on purpose,
with the schema and these words, in a later release. *(Amended 2026-10-17, #362, for `0.2.2`: the step now uses the members it already had: it is `partial`, and its
`coverage` is the envelope `coverage` `transcript retranscribe` gives for the same revision; the test above was replaced. ADR 0021, note of that date, has the
detail and the rolling-back statement.)*

### Rolling back

A revision that holds an unusable chunk stores the outcome `unusable` and the warning `provider_chunks_rejected`, which every release before this one decodes as damage
(`INTEGRITY_FAILURE`), as L-130 records for a trimmed end, and a revision spliced from one that left a part some run could not read, under a window or file it carries, stores the member
`carried_untranscribed`, which those releases read as damage too. **Only a session whose chain holds an `unusable` chunk has such a revision** *(amended 2026-10-11, #359: or a stretch of words that no segment holds, which a revision also records as `unplaced_ranges`; see the note of that date, whose rollback statement is the one to read)*: the member is written only for a part such a chunk left (decision 8), and a chain that
never had one, such as one range retranscription after another, is written byte for byte as before and is read by the release before. Every other revision is written as before. A run that had such a chunk would have
failed on the release before. Use the newer version, or discard the session.

### Evidence, and what is not proven

Tests at the lowest layer that shows each rule: the domain (segment-count boundaries and the monotonic rule for every count up to sixteen, empty text as a marker, the
strict-majority threshold and its edges, a run's gaps, `EarlierTextRule` over a table of earlier segments (wholly in the gap, a microsecond past its edge, across it over no
text, overlapped by the run's text by a microsecond, touching it, wholly in what was read, outside the range, across the range's edge), coverage of an unusable window and of a
replaced one, text kept across a gap's edge and covered, an earlier window not counted over what the superseded revision left untranscribed and the record's validation, the
span arithmetic), the application (one chunk of ten, exactly half and more than half, early stop, silent chunks not counted, one of two, the job failing at once and not resumable,
a resume that does not ask again, the review scenario of text kept in an unreadable part, no `no_speech_recognised` beside an unusable chunk, a model swapped during the run, and
chains of runs: the review's whole-source chain and the same chain over an imported file, three runs with different unread parts, a later run that reads the unread part, a chain
with no unread part, the microsecond straddler, the run's own text over a straddler, and every chain of up to three retranscriptions from a grid of ranges and chunk answers from
three starting points against an independent count of what was read), the checkpoint and revision records (round trips, damaged records, a record of kept text and the same record
with the chunk changed, a record of a chain with its list of untranscribed parts and the forged forms of it), the contract (the partial result, the kept text listed as not
transcribed by the run, the unchanged complete result, the failure's prose on both sides of the three-answer boundary and for the setup check, a worker step pinned as
`complete`, every schema; the failure and partial examples are produced by the application's own run over a stand-in recording, not written by hand), and the human output with
snapshots.

**Not proven:** no real recording was used (the recording is commercial and cannot be shared); the tests build the shape from recorded-style provider output. Which kind of
segment whisper.cpp returned for 27:05 to 27:35 (empty, reversed, or at or after the audio's end) is unknown, and the fix does not depend on it: with a single rejected
segment beside kept ones it is kept out, and with only rejected segments the chunk is a gap. The three constants (`MIN_SEGMENTS_FOR_REJECTION_RATIO`,
`UNUSABLE_CHUNK_SHARE_DENOMINATOR`, `FEW_ANSWERED_CHUNKS`) are proposals, chosen as above, not measured on a corpus of real recordings (the follow-up issue on real media in
the test set stands). The kept-text rule has no test over a real decode: the engine's opt-in tests that decode real speech do not exercise it. The chain test's count of what was read is the author's
reading of what a run read, independent of the code it checks but not of the rules; and no session written by `0.2.0` with several revisions was read by this code, only the rule that a record with no
`carried_untranscribed` is read as before. The rule's choice for text over audio the run read as empty (kept) is a judgement, not a measurement.

## 2026-10-11 note: merging chunks can no longer drop speech at a seam, and words that no segment holds are reported (#359)

**Proposed for the maintainer to confirm by merging this change.** It supersedes the claim, in the 2026-09-25 note and in the merge's own comment, that a segment cut by a chunk edge may be
replaced by a neighbour's segment that spans its midpoint "so speech is never lost" (a segment that spans the midpoint can end before the cut one does), and it adds to section 3 (the seam
merge). It is meant for the second candidate of `0.2.1`; `0.2.0` and `0.2.1-rc.1` are affected. *Revised in the same change after two independent reviews: the guard is on the union of the
segments and not on each, the claims about "nothing lost merging as before" are replaced by what is true, an empty range no longer marks a stretch, and the stretches are bounded fast and printed bounded;
each is said where it comes. A second round added: a stretch is cut to its chunk's window by the one definition that a stored revision also applies (a run whose audio began a frame before its window no longer fails at the end), the repeat rule can drop a copy that the guard exposed to it, the exact rollback reading, and the disclosure that the simulation's figures are not reproducible from the repository.*

### What happened

(Issue #359.) On two real recordings, which stay on the maintainer's machine (a 17:34 spoken walkthrough, 42 chunks, 171 segments, and the 34:36 lecture of #353),
a sentence broke off at the end of one chunk's last kept segment and resumed seconds later in mid-phrase, with no warning, while `search` reported the whole video transcribed. In the first row of the
report: the earlier chunk (window 0 to 30 s) ends with a segment 22.32 to 29.50 s, which ends exactly 0.5 s before its window does, so the edge margin (`SEAM_EDGE_MICROS`, 0.5 s) does not call it cut; the later
chunk (window 25 to 55 s) starts with 25.00 to 32.44 s, which starts at its window's edge and so is cut. Rule 2 asks whether the earlier chunk has an uncut segment that spans the cut one's midpoint
(28.72 s); it has, so the later segment was dropped whole and the earlier one kept, and the 2.94 s from 29.50 to 32.44 s, past the end of the earlier segment and in audio the earlier chunk never heard
beyond 30 s, were in neither chunk's output. The mirror at a right edge is the same with the roles swapped. Rule 1 could do the same (a copy is dropped when its owner has *any* segment that overlaps it, however little).
**The cause is inferred** from the timings in the report and from the code: the raw output of the chunks concerned is not available. It is reproduced, though: the issue's own timings, run through the merge as it was, give
exactly the loss the report names (the new test fails on the old rules), so the mechanism is established even if the recogniser behaviour that produced those timings is not.

A second thing in the same report: rejected segments (`provider_segments_rejected`: 3 of 264 and 8 of about 179 recognised segments on the two recordings) are dropped with a count and no time, so each leaves a
possible hole that no result reports.

### The decisions

**1. A guard on rules 1 and 2: dropped segments are restored until the union of all the validated segments has no hole beyond the tolerance.** The maintainer's notes recommended, for a patch release, Option A (a neighbour segment replaces
a cut one only when it also reaches the cut segment's far end within a small tolerance, with rule 3 trimming the repeated leading words) over Option B (decide ownership by time and keep only the later
segment's words after the earlier one's end). **Option B was not taken because the data it needs are not kept**: a `ProviderToken` holds a token's kind and probability and no time, so the words after a
given instant cannot be told apart in a segment (whisper.cpp's own output has an offset for each token, which the adapter does not read and the checkpoint does not store; reading and storing them is a larger change to an
adapter and a stored checkpoint, and trusting per-token times a larger risk, for a patch release). **Option A is realised in its global form.** Its local form (look at the one neighbour segment that was chosen) would still have to be
backed by a check of everything kept, since that neighbour may not be all that covers the cut segment, and rule 1 drops by a different test altogether.

*The guarantee is on the union, and not on each dropped segment* (the first version of this change judged each dropped segment, and a review showed that the tolerance then stacks). Two dropped copies that each leave the
tolerance uncovered, at ends that face each other, leave twice of it: chunk 0's 20 to 27 s kept and 27.5 to 29 s dropped, chunk 1's 25 to 27.5 s dropped and 28 to 33 s kept, leave 27 to 28 s uncovered, a hole of 1 s in speech that
both chunks wrote words for, and every segment passes a per-segment test. A guarantee on the union cannot stack, and it implies the per-segment one (a segment lies inside the union). So: the stretches of the union that no kept
segment covers (the holes) are taken left to right; for a hole longer than the tolerance the dropped segments that overlap it are taken the most overlapping first (then by start, end, chunk and position), each restored if it
overlaps a remaining piece of the hole that is still longer than the tolerance, until none is left (`restore_lost_speech`). The dropped segments over a hole cover it entirely, so the loop always ends within the tolerance, and
it restores whole segments and as few as that order needs (two copies of the same speech are not both restored). Its properties, each held by a test:

- **No stretch of the union longer than the tolerance is covered by no kept segment**, and so none of any one segment. Over generated layouts (up to five chunks of the R0 plan, segments hugging the window edges, rejected segments of
  every kind among them) it holds, the merge invents nothing and keeps start order (`no_layout_of_chunks_loses_speech_at_a_seam`; run 40 times with fresh seeds in this review round, 40 of 40 passed). A fixed sweep over the same
  layouts (`the_generated_layouts_reach_the_guard`) shows that the guard acts in a real share of them and leaves a real share alone. The stacking layout above is a test of its own
  (`the_tolerance_does_not_stack_into_a_longer_hole`), and four mutations of the guard (a restoration not counted as covering, the order reversed, the list not sorted, the clip to the source of a stretch removed) each fail a test.
- **Against a frozen copy of the merge as it was** (`asr/previous_merge.rs`, rules 1 to 3 as at `52bc325`), the property is: where that merge left no hole beyond the tolerance, this one gives the same segments and warnings; where
  it did, and the repeat rule (rule 3) does not act, this one keeps every segment that one kept, in the same order, and only adds. **Where rule 3 acts that is not so** (the guard can expose a segment to it that the old merge kept: the point on rule 3 below). The generator of that property gives every segment words of its own, so rule 3 never acts in it; a second property, with a vocabulary of two words so that it does, checks the true and weaker statement: before rule 3 no stretch of the union longer than the tolerance is in no kept segment, rule 3 drops only a segment whose words a kept segment repeats, and so every stretch longer than the tolerance that is left in no kept segment after it is time covered by segments that it dropped as repeats (`with_repeated_words_every_hole_is_time_whose_words_a_kept_segment_repeats`; `the_repeated_vocabulary_layouts_reach_the_repeat_rule` shows that its layouts reach rule 3). (The first version of this change claimed "a run that lost nothing merges exactly as before" and tested it against itself, which proved nothing.)
- **It judges by time and not by words, and that costs repeats.** A copy whose words the kept neighbour also wrote can be restored because its time is not covered, and then the same words stand twice where nothing was lost
  (`a_copy_that_the_kept_neighbour_also_wrote_can_stand_twice`: the two chunks disagree by a second on where "the file and save" ends). A reviewer's word-level simulation of synthetic speech (1,500 runs of its mild setting, exact words with pauses in 12 per cent of the speech, run again on
  this code; **the simulation is not in the repository and not reproducible from it, so these figures are the reviewer's and not a measurement of real speech**) gives lost words 5,931 before and 97 now, and duplicated words 516 before and 1,923 now; in the 451 runs where the earlier merge lost no word the output still differed in 74 (16 per cent), and in its noisy setting (5 per cent of the words misrecognised,
  timing jitter; lost words 6,351 and 118, duplicated words 2,275 and 7,066) in 158 of 409 such runs (39 per cent); the per-segment version of the guard gave 2,261 duplicated words in the mild case. **The trade is made on purpose: a repeat is visible, a hole is not.**
- **Rule 3 limits what it can undo.** Rule 3 compares only the last kept segment of the earlier chunk with the first kept segment of the later one, so a restored segment outside that pair is never trimmed, a repeat that differs by
  a word is never found (the issue's point 3, not done), and its containment test can drop a restored copy that carries new time (`the_repeat_rule_can_still_drop_a_restored_copy_that_carries_new_time`: "well yes yes okay" from 22.32 to
  29.5 s beside "yes yes" from 25.0 to 32.44 s loses 29.5 to 32.44 s; the merge before lost it too). **The guard can also expose a segment to rule 3 that the old merge kept** (`the_guard_can_expose_a_segment_to_the_repeat_rule_that_the_old_merge_kept`): chunk 0 (window 0 to 30 s) has 20.0 to 26.9 s "one two three" and 27.0 to 29.5 s "alpha beta gamma", chunk 1 (25 to 55 s) has 28.0 to 33.0 s "beta gamma"; the old merge keeps 20.0 to 26.9 s and 28.0 to 33.0 s and leaves 27 to 28 s in no segment; the guard restores "alpha beta gamma", and rule 3 then drops "beta gamma" because its words are inside it, so the hole becomes 29.5 to 33.0 s (3.5 s) although no word is lost. The time hole can therefore grow where no word is lost, and "only adds" is a statement about where rule 3 does not act. Extending rule 3 to every overlapping pair that involves a restored segment is **not done here**; it is registered, and decided after
  the real-recording check.

**The tolerance, `SEAM_COVERAGE_TOLERANCE_MICROS`, is 0.5 s, the edge margin.** (The constant's rustdoc has the derivation, which is an upper bound for one configuration and not a proof that the value is right for real speech.) The earlier chunk's
segment that stands for a cut one is not itself cut, so it ends at least one margin before its window's end `W`; whatever the dropped segment carries beyond it is left for at most the tolerance, so it ends no later than
`W - margin + tolerance`. That is at or before `W`, the audio the earlier chunk heard too, exactly when the tolerance is at most the margin; one microsecond more would let a segment that runs past `W`, into audio only the later chunk
heard, lose the part beyond it. So for this configuration the margin is the largest safe tolerance. A smaller value would be as safe at the window's edge and would restore more copies that differ by timestamp noise; **whether half a second
is the right amount of that noise for real speech has not been measured.** A test finds the largest tolerance for which two chunk layouts lose nothing beyond a window's edge by binary search over the R0 plan, at both edges
(`the_tolerance_is_the_largest_that_never_loses_speech_beyond_a_window_edge`); it checks that the code's arithmetic agrees with the derivation, and a change to the margin moves it, but it does not check the choice. A second test
holds the boundary (exactly the tolerance is left, a microsecond more is restored). The overlap (5 s) does not enter the bound. **What it costs:** up to half a second at the end or start of a dropped copy can be words the kept segment's
recogniser did not write down (one word at a normal speaking rate), with no sign; the old rules lost that and everything after the neighbour's end with it. The 0.58 s hole of recording A (one missing word, which the screen showed) is of
that size, which is why the tolerance is stated as the size of what is knowingly left and not as zero: zero would restore nearly every copy, because two recognisers' ends for one sentence are rarely equal, and the repeats would swamp the reader.

**2. Words that no segment holds are reported with their time, without calling the run partial.** The options weighed (the contract is additive only and the frozen skill copies gap reasons from a closed list, so
the published `untranscribed_range` is reused and no new reason word exists):

- **(a) Every uncovered rejected stretch is a coverage gap and the run is `partial`.** One channel, the same as #353, and the skill already handles it. But rejected segments with words are routine on real
  speech (both recordings had some), so `partial` would be the usual answer to a transcription and a status that is nearly always set says little; and a stretch is an upper bound (below), so the alarm is out of proportion. A re-run gives the same.
- **(b) The run stays `complete`; the stretches are listed with their times in a new optional member, and a search never counts them as transcribed.** The user of `retranscribe` is told where, once; the
  place where an agent reads "no hit" as "not said" (`search`) is where the stretch is untranscribed and the result `partial`. **This is what is implemented.**
- **(c) A duration threshold.** Used inside (b): a stretch no longer than the tolerance is not recorded. The figure is the one the merge already leaves uncovered at the end of a kept copy (it would be inconsistent to report half a second
  of rejected words while leaving that much of a kept segment silently), so no second constant has to be defended; it is a convention for consistency with the guard and **not a measurement of how long junk is**. It removes the stretches of
  a few hundred milliseconds that make up most of what "junk" would be, and not longer ones; a lost word of that length is not reported.

*What a stretch is.* A rejected segment names a time its audio cannot hold, so it cannot say how long its words took. The recogniser wrote its segments in order, so words that start at the segment's start end before the next segment it
wrote begins: the stretch runs from the segment's own start (never before it, whatever its end says) to the next segment's start in provider order, a marker or another rejected segment included. It is an upper bound, not a measurement. It is
listed only when the segment has a letter or digit in its text (punctuation is not a word), has a range that is not empty, starts inside its chunk's audio and is followed there by another segment. **An empty range (start equal to end)
is count-only** (the first version marked a stretch for it, and a review showed what that does: recognisers are known to write text for silence, and a stretch for such text at the start of a chunk that speaks only from 20 s would list 20 s as
untranscribed and turn the search `partial`; **whether whisper.cpp writes it with an empty range has not been checked on real output**, so this is a decision made to avoid that alarm, and its cost (words in such a segment are dropped with no sign beyond the count) stays registered, L-158; an empty range says nothing about where its words lie). **A reversed range (end before start) keeps its stretch, from its start**: a segment that gave a start and a faulty end is placed by the start it
gave, never earlier (the first version took the earlier of the two times, and a rejected segment starting at 12.5 s with an end of 0.0 s then marked 0 to 5 s and 7 to 20 s, the first before its own start). Other rejected kinds with
a positive range (an end beyond the padded window, a range outside the source) keep the stretch, clipped to the source and to the chunk's window. The kinds that stay counted only (no word, an empty range, a start at or after the audio's end because the
recogniser works in a window padded to 30 s and a time after the audio's end is a time in the padding, nothing after it inside the audio) are **a limit and not a claim that nothing was lost**. Two limits follow from "up to the next thing
the recogniser wrote": a chunk's last segment has nothing to bound it, and a stretch to the audio's end would list a silent tail as untranscribed, so it is count-only, though what lies at a chunk's end is also in the next chunk's first seconds
**only when there is a next chunk and it was not silent or unusable** (not for the last chunk of a run); and the bound has a cliff (a next segment 0.1 s before the audio's end gives a stretch of almost all that is left of the chunk, one at or
after its end gives none). Each kind, and the cliff, is a test (`each_kind_of_rejected_segment_leaves_the_stretch_it_can_or_none`, `what_follows_a_rejected_segment_bounds_its_words`, `a_rejected_segment_near_the_audios_end_is_bounded_only_by_what_starts_inside_it`, `a_stretch_never_leaves_the_source`).

*What is recorded and where.* `merge_chunks` returns the stretches, less what any kept segment of any chunk covers, where what is left is longer than the tolerance (`MergedTranscript::unplaced`, at most 1,024; where there would be more
the nearest are joined, which marks more and never less, with a join that sorts the gaps and does not search the whole list for each join, because the first version's took minutes at tens of thousands of stretches: 5,000 took 1 s, 20,000 took 17 s and 65,000 took
185 s in a test build; it is tested against the search it replaced and by a time-bounded test over 200,000 spans and a merge over about 1,900 raw stretches). They are stored on the revision (`TranscriptRevision::unplaced`, the record
member `unplaced_ranges`), validated on reading (in order, merged, inside the source and inside the range the run covered; an import has none; **producer and check share one definition of where a stretch may lie**, `unplaced_limits`: inside the chunk's window and the source when a chunk makes it, inside the range the run covered and the source when a stored revision is checked, so what a run makes a stored revision accepts by construction. The first version clipped to the source only: the decoder's first sample may be at an earlier frame boundary than the window asked for, a kept segment from such audio is not checked against the window, and a run over 10 to 40 s whose audio began 20 ms early listed 9.98 to 12.98 s, which the check refused, so a run in which every chunk had been transcribed failed at the end; found by a reviewer, and held by a domain test, an application test (`a_run_whose_audio_began_before_its_window_still_builds_its_revision`) and a property that a stored revision accepts the stretches of every generated layout, with audio up to 40 ms early and runs that start away from 0), presented in the revision summary (`local_asr.unplaced_ranges`, every stretch, at most 1,024) and in the
human text (the first 100 and how many more, as a search prints at most 100 of each kind of range; each called an upper bound), and **counted by coverage**: a stretch is not covered by the window the run read, only by a segment of the revision
(its own words, or earlier text it kept) or by a window the run found silent (words written over audio with no signal are the recogniser's, not a loss). The rule of #353 that keeps a part untranscribed through later runs
(`carried_untranscribed`) needs nothing new: a later revision records, as before, every part that its superseded revision lists as untranscribed and that a window or file it carries would be counted over, and that now includes these stretches. A test
(`no_chain_of_runs_makes_search_claim_what_no_run_read`, now with a chunk that writes such words) checks it for every chain of up to three retranscriptions against an independent count.

*What to do about a stretch.* The published search warning ("Run transcript retranscribe to transcribe them") is unchanged and **does not fit this cause**: transcribing just the stretch again cuts the audio at other points and may cover it, but it
may give the same stretch, because the recogniser may write the same words with the same fault. **Supplying a transcript** the user has (`ingest --transcript`) is the sure remedy. The guide, the contract and the register say so.

*The recommendation* is (b) with (c), as implemented. (a) is a small change if the maintainer prefers a single channel at the cost of an almost always `partial` answer: `finish_retranscription` would add the stretches to the gaps.
**The weakest points of (b):** a `search` over a whole recording lists every such stretch as untranscribed; how many real recordings have how many stretches is **not known** (no recording was used), so the alarm in `search` may be larger than
hoped; and with the empty and last-segment kinds left out it may report little.

**3. Compatibility and the rollback, stated exactly.** A revision whose run left no stretch is stored and presented as before, though one in which only the merge's guard acted may hold more segments (the property above, against a frozen copy, is the
statement of when the merge is unchanged). The new record member `unplaced_ranges` is optional, written only when non-empty and validated on read; the presentation's `local_asr.unplaced_ranges` is written only when non-empty; the
`provider_segments_rejected` warning has longer fixed prose in the envelope only for a revision that lists a stretch. `ChunkSegments`, `MergedTranscript`, `ValidatedChunk`, `AsrTranscription`, `TranscriptRevisionParts` and
`TranscriptRevision` each gained a member (types of the internal crates, which the `vsift` engine does not re-export; the published JSON contract is additive). **Rolling back.** `0.2.0` and earlier, **and `0.2.1-rc.1`** (all read a stored record
strictly), read a record that has `unplaced_ranges` as damaged (`INTEGRITY_FAILURE`); `0.2.0` and earlier read `carried_untranscribed` so too, which `0.2.1-rc.1` knows, so a later revision that holds only that member is read by `0.2.1-rc.1` and the earlier one it carries from is not. Such a release reads newest first and stops at the first revision it needs, so **if the newest revision holds `unplaced_ranges` nothing of the session can be read, and if only an older revision holds it the newest reads normally and the older ones are refused**, when asked for by id, by `handoff check --session` and by `bundle validate`. The record decoder of this version is more lenient than the schema, on purpose: it reads an empty `unplaced_ranges` list as no stretch (the schema asks for at least one), as it does for `carried_untranscribed`, because a reader should not refuse a record whose only fault is a list that says nothing; what a list may hold is checked by the revision's own validation. Unlike the `unusable` chunk of #353, which failed the run on the release before, **a rejected segment with words and a bounded stretch
succeeded on `0.2.0`**: so a session written by this version may hold the member without anything having gone wrong. Nothing already written is rewritten. Use the newer version, or discard the session. This is the one stored-format
addition of the change; a design with none (the stretches derived when read) was looked for and does not exist, because the times of a rejected segment are in no stored record. The statements of the 2026-10-10 note that only an `unusable` chunk
causes `carried_untranscribed` are amended in place.

### What does not change

The chunk plan, the output rules and which segments are rejected (a rejected segment is still counted exactly as before), the quarter rule, the unusable-chunk channel, the failure codes, the published gap reason
(`untranscribed_range`) and the published search warning, the agent skill (frozen), every warning kind, code and count, the checkpoint format, and the status of a run (`complete` unless a chunk was unusable). Rules 1 to 3 are unchanged; the guard is a step
between 2 and 3.

### Evidence, and what is not proven

Tests at the lowest layer that shows each rule: the domain (the report's first row with its own timings, its mirror at a right edge, a chain of three chunks, a segment longer than the overlap that starts at a window edge and one 0.4 s and 0.6 s after it,
the edge margin at 0.4 s and 0.6 s, the tolerance by search and at its boundary, the guard's order and its stop, the stacking layout, the property over generated layouts against a frozen copy of the old merge and its sweep, the cost of the guard
the limit of rule 3 and the guard's exposure of a segment to it stated as tests, the guard's skip of a segment inside one it has restored and its order against plain start order (each caught by a mutation), the repeated-vocabulary property, the early-audio cases of the stretch, the span set and the join against the searches they replace, every kind of rejected segment and the cases that mark nothing, the clip to the source, the merge's reporting and its bound over about 1,900
stretches, the revision's validation and its coverage), the application (chains of runs), the record (round trip, schema, forged forms, the fuzz seed), the contract (the new example, produced by the application's own run over a stand-in recording; a `search` that is
`partial` over a stretch; the unchanged frozen examples) and the human text with snapshots, one of them for more than 100 stretches. The opt-in real-tool stage `p07_local_asr_multi_chunk_seam` holds the 9.25 s sentence that starts at the second chunk's
window to the rule, for a binary from the second candidate on (an older published binary is not held to it).

**Not proven:** no real recording was used, and **the opt-in real-tool stage was not run in the session that wrote this** (it needs ffmpeg, whisper.cpp and a model). The cause is a reading of the timings, reproduced in the merge; that it is what happened on the
two recordings is not shown, and a listener's check of the holes on them (the issue's last item) has not been done and stays with the maintainer. The tolerance is derived for one configuration and not measured; how many repeats real recordings gain from the guard
is known only from a synthetic simulation by a reviewer that is not in the repository (the several-fold above); the share of real recordings that have unplaced stretches, and their sizes, are unknown, and taking "to the next segment" as the stretch is a reading of what the data can say, not
an observation of what the recogniser does. The guard's cost was not measured on large input beyond a test that merges 128 chunks of dense speech (about 32,000 segments, about 0.7 s in an unoptimised build); the work is meant to be linear in the segments with a logarithmic factor (the covered stretches are kept in a tree), and nothing was measured on real output.

## 2026-10-17 note: a speech chunk is decoded from a second before it (#334)

The decode of a chunk (and of an `audio` clip; it is one function) no longer hands FFmpeg the chunk's start as an input seek and its
length as `-t`. In a source without a video stream the seek landed on the audio stream and the decoder dropped the first frame after
it, so a chunk's audio began up to one codec frame late (64 ms for AAC at 16 kHz; even a chunk at 0 lost its first 64 ms), and a very
short `audio` range decoded to nothing. The decode now seeks one second earlier (not at all within the first second), resamples to
16 kHz and cuts the chunk out with `atrim`, which keeps #332's rounding at the 16 kHz output. The bound after the decode that the
2026-10-08 note proposes is not made: `atrim` cuts by timestamps, and a source whose audio timestamps overlap (a clip looped with
`-stream_loop` and copied) decodes 30.032 s for a 30 s chunk, as it always did. What a recogniser hears: on a 16 kHz source with video, the same bytes as before; on other rates, the same samples except the
resampler's first few milliseconds, which now fall before the chunk; on an audio-only source, the chunk from its true start. The 100 ms
floor, the chunk plan and every recorded field are unchanged. Register: L-141.

## 2026-10-17 note: the identity is read after every answer, so a checkpoint names the model that answered (#361)

Supersedes the paragraph "A swapped recogniser and a resumed job" of the 2026-10-10 note. A run reads the recogniser's identity before its first chunk and
**after every chunk the recogniser answered, before the answer is kept or stored**; it must equal the identity the run's recognition key was built from. A
swap therefore ends the run at the first answer given after it, as `model_changed` (still retryable; the job stays resumable), and that answer is not
checkpointed, so a resume with the original model put back reuses only answers the original model gave. The checks before the verdict on the answers and
after the last chunk are removed as redundant (every answer was followed by one), so a swap that makes the recogniser answer with garbage is still
reported as `model_changed` first. The cost is one hash of the model per recognised chunk (about 0.3 s in a release build against seconds of recognition
per 30 s chunk), the per-chunk read this ADR's earlier notes left out for its cost. Ending the job on `model_changed` was the other option and was not
taken: it changes a published, retryable failure. Left: a swap undone between the two reads around one chunk, and checkpoints written by earlier releases
(register L-146).

## 2026-10-18 note: the reviewed whisper.cpp is v1.9.5 (#322, #399)

Changes no decision of this ADR. The reviewed build moves from v1.9.2 to **v1.9.5** (upstream's binaries of build release `b5454`,
commit `d1be6fde11ac`; the review is in [`p06-ubuntu-artifact-candidate.md`](../planning/p06-ubuntu-artifact-candidate.md#2026-10-18-re-pin-whispercpp-v195-build-b5454)),
which holds the memory-safety fixes the note of 2026-10-08 lists and the fix of CVE-2026-110810 ([L-137](../planning/known-limits.md#l-137)).
The adapter is unchanged: v1.9.5's command-line source was compared with v1.9.2's, and no flag of decoding profile `r0-v1`, no default and no
line of the `-ojf` writer changed, so the parser and the v1.9.2 fixtures stand. v1.9.5 exits 3 for a missing model and 11 when an input or
output file failed, both read as a failed provider like any non-zero exit. The 100 ms floor of 2026-10-08 stays: it guards any whisper.cpp a
user registers. Hosted accuracy (T-04) is identical to v1.9.2's on the same runner CPUs. The build is part of the recogniser's identity, so a
transcript made with v1.9.2 keeps that provenance, and recognition with v1.9.5 is recognition by another build (a resume across the change
is the `model_changed` case of the 2026-10-17 note). `setup check` names the new bytes `whisper.cpp v1.9.5 (reviewed build)` and still names
v1.9.2's `whisper.cpp v1.9.2 (reviewed build)`, which a managed install made by `0.2.2` or earlier keeps until `setup install` runs again.

## 2026-10-19 note: every rejected segment with a word is placed, and a search says what a stretch is (#383, #384)

Amends the note of 2026-10-11 (#359), section 2, "which kinds of rejected segment have a stretch", and its paragraph *What to do about a
stretch*; nothing else of it changes.

**What was seen.** Two real recordings (0.2.1, #383) each had one stretch in mid-sentence, 5.12 s and 6.16 s, at the end of a chunk,
with the next chunk's first segment at its window start, a count of rejected segments, no `unplaced_ranges`, and coverage that called
the stretch transcribed; on one of them an earlier release had kept the end of the sentence there. That matches the kind "nothing after it
inside its audio", which was only counted. The raw recogniser output was not captured, so the match is the strongest evidence there is, not
proof.

**The decision: every rejected segment with a letter or digit is placed** (`unplaced_stretch` in `crates/vsift-domain/src/asr.rs`). Its
words lie after the start of what the recogniser wrote before them and before the start of what it wrote after them, so the stretch runs
from the segment's own start (never before it, whatever its end says; an empty range included) or, for a start at or after the audio's end
(a time in the padding of the 30 s window), from the start of the latest segment before it that starts inside the audio (or the audio's
start); and **until** the next segment's start, or the audio's end when no segment after it starts inside the audio. The merge still takes
out what any kept segment of any chunk covers (a chunk's tail is heard by the next chunk's first seconds) and drops what is left at half a
second or less. Only text with no letter or digit has no stretch. The cliff of the earlier rule (a next segment 0.1 s before the audio's
end gave almost the whole rest of the chunk, one at its end gave none) is gone. **The cost, accepted for the rule that coverage is stated
truthfully:** a pause or silent tail over which the recogniser wrote words that cannot be placed is now listed as untranscribed, so more
searches are `partial` and each stretch can be longer than what was lost ([L-159](../planning/known-limits.md#l-159)). How much more, on
real speech, is what the hosted campaign measures: on RM06, RM09 and RM10 it added no stretch (`P14 real media` runs 39217761933 and
39220841090).

**A search's warning fits the cause (#384).** When a gap of a `search` overlaps a stretch the revision lists, the warning is
`UNPLACED_WORDS_SEARCH_WARNING` (`vsift-contract`): it says the stretch is an upper bound, that transcribing the range again may fill it but
may give it back, and that a transcript file is the sure way. Every other partial search keeps the published text word for word. The
skill's own rule already says to retranscribe once and then offer a transcript file, so it does not change. **Decided by the maintainer on
2026-10-19:** no bounded list for now; the revision block keeps every stretch, at most 1,024 (L-159). L-158 lowered to low, also the
maintainer's decision of 2026-10-19.

**Compatibility.** No stored member, field, code or reason is new: more revisions hold `unplaced_ranges` than before, which `0.2.0` and
earlier and `0.2.1-rc.1` read as damage ([L-160](../planning/known-limits.md#l-160)). A checkpoint holds the recogniser's raw answer, so a
job started by an earlier version and resumed by this one places its rejected segments by this rule. **Tests:** `each_kind_of_rejected_segment_leaves_the_stretch_it_can_or_none`,
`what_follows_a_rejected_segment_bounds_its_words` and `a_rejected_segment_near_the_audios_end_is_bounded_by_the_audios_end` (the 2026-10-11 note's `..._is_bounded_only_by_what_starts_inside_it`, renamed; each failed
on the earlier rule), the contract test `words_that_no_segment_holds_are_listed_and_the_run_stays_complete` and
`a_search_over_words_that_no_segment_holds_is_partial_and_names_the_stretch`.
## 2026-10-19 note: the repeat rule compares every overlapping pair and keeps a copy that carries time (#380, #382)

Amends rule 3 of `merge_chunks` as the note of 2026-10-11 (#359) left it; the guard, rules 1 and 2, the tolerance and the chunk plan
are unchanged.

**What was wrong.** Rule 3 compared one pair per seam (the last kept segment of the earlier chunk and the first of the later one),
wanted every compared word equal, and dropped a later copy whose words all occur in the earlier one whatever its time. So a copy the guard
restored anywhere else stood twice (#380: on two real recordings of `0.2.1`, 13 and 8 strict repeat pairs, restored copies that differ
from the other chunk's text by a word, and two segments wholly inside the one before them), and a contained copy that carried new time was
dropped, leaving a stretch with no segment that a search called transcribed, a stretch the guard had sometimes just covered (#382, L-157).

**The decisions.**

1. **Every pair** of kept segments of consecutive chunks whose times overlap is compared, in start order, the earlier chunk's first. A
   repeat inside one chunk, or between chunks that are not consecutive, is not looked for: two chunks that are not neighbours share no audio.
2. **One inner word may differ among four or more** compared words (`MIN_FUZZY_SEAM_WORDS`); the first and the last must match, and runs
   of two or three must match exactly, as before. Why four: one different word in a shorter run leaves one or two in common, which the
   rule already refuses to call a repeat (`MIN_SEAM_DUPLICATE_WORDS`). Why not the first or the last: that is where a copy meets the words
   around it, so a difference there is more likely another word than the same word heard differently. The cost: a trim can take a word
   only one chunk wrote, keeping the earlier chunk's version of the same audio.
3. **A contained copy is dropped only when what else is kept covers its time to within the tolerance**; otherwise it is kept whole.
   This is the guard's trade applied to rule 3 (a repeat is visible, a hole is not), and the alternative that the issue named. Two other
   designs were considered: listing the time a dropped copy leaves as `unplaced_ranges` (no repeat, but a `partial` search wherever two
   chunks' ends for one sentence differ by more than half a second, and a member whose meaning is "words no segment holds" stretched to
   words a segment holds at another time), and running the guard again after rule 3 (the same result as this, by a second pass). So
   **the guard's guarantee now holds after the whole merge**: no stretch of the union of the chunks' segments longer than the tolerance
   is covered by no kept segment. A trim never changes a segment's time.

**What it costs and what it changes.** Two chunks that end the same sentence more than half a second apart, where rule 1 kept both,
now keep both copies (until now the later one was dropped). How often that happens on real speech, against the repeats the wider rule
removes, is what the hosted campaign measures: repeated word triples at seams went from 13 to 6 on RM06, 79 to 80 on RM09 and 7 to 7 on
RM10 (`P14 real media` runs 39217761933 and 39224192022, which recognised the same words).
`seam_duplicates_removed` counts each trim or drop, so it can be larger than one per seam. No field, code, warning text or stored member
changes. The frozen copy of the earlier merge (`asr/previous_merge.rs`) is unchanged: where it lost nothing, with unique words, the
merge still gives its result, because rule 3 does not act on unique words. **Tests** (each failed on the rule before):
`a_copy_the_guard_restored_is_removed_when_what_is_kept_covers_its_time`, `a_repeat_beyond_the_first_pair_of_a_seam_is_trimmed`,
`a_repeat_that_differs_by_one_inner_word_is_trimmed`, `a_contained_copy_that_carries_time_beyond_the_tolerance_is_kept`,
`the_repeat_rule_keeps_a_restored_copy_that_carries_new_time`, `the_repeat_rule_no_longer_undoes_what_the_guard_restored`, the property
`with_repeated_words_no_hole_is_left_and_only_repeats_are_dropped` and its sweep `the_repeated_vocabulary_layouts_reach_the_repeat_rule`.
The 2026-10-11 note's `a_copy_that_the_kept_neighbour_also_wrote_can_stand_twice`,
`the_repeat_rule_can_still_drop_a_restored_copy_that_carries_new_time` and `the_guard_can_expose_a_segment_to_the_repeat_rule_that_the_old_merge_kept`
are the earlier forms of the first and the last two of these, renamed. L-157 is deleted; L-155 says what repeats are left. **The
maintainer decided on 2026-10-19:** keep a copy that carries time as a visible repeat, the one-word allowance at four words, and L-157
deleted.
## 2026-10-19 note: the seam tolerance, measured on real speech, stays half a second (#381)

Changes no decision. The note of 2026-10-11 derived `SEAM_COVERAGE_TOLERANCE_MICROS` (half a second, the edge margin) as an upper
bound and said it was not measured. The real-media campaign now measures what it leaves (`seam_joins`): the gaps where one chunk's
kept segment meets the next chunk's at a seam, against the gaps between one chunk's own segments, which whisper.cpp writes touching.
On RM06, RM09 and RM10 (hosted run 39224705780, both chunk plans; [`real-media-set.md`](../planning/real-media-set.md) section 14.2),
15 of 187 seam joins had a gap of up to half a second (4 up to 0.1 s, 9 up to 0.25 s), against 2 of 558 joins inside a chunk; two plans'
copies of the same text differ by a median of 40 ms. **The value stays:** a smaller one would restore a copy for 6 (at 0.25 s) or 11
(at 0.1 s) of the 15 stretches, each kept even where its words repeat (the note on #382), and with nothing known about what the stretches
hold no smaller value can be shown to lose less than it repeats. **The maintainer decided on 2026-10-19 that it stays.** The campaign lists
where each stretch starts, for a listener; if they hold words, 0.1 s is the figure to try. L-156 has the figures.

## 2026-10-20 note: a segment that repeats the one before it in its chunk claims no time (#485)

Amends rules 1 and 2 of `merge_chunks`; the guard, rule 3 and the tolerance are unchanged. On RM09 (hosted runs 39466921335 and
39486605862, `real-media-set.md` section 14.5) chunk 36 wrote one sentence twice, and rule 1 dropped chunk 37's segment over
925.00 to 927.96 s because the repeated copy owned that time; the guard saw no hole because the copy covered it, so the words were in
no segment and coverage called the stretch transcribed (L-201). **The decision (the maintainer's, 2026-10-20, the narrowest of three
options):** a segment whose normalised words repeat those of the segment just before it in the same chunk (at least two words,
compared as rule 3 compares them, so one inner word may differ among four or more) claims no time: it does not make its chunk count
as having a segment at that time in rule 1, and it does not stand in for a neighbour's cut segment in rule 2. It is kept by its own
chunk as any segment is, so the rule only lets another chunk's words be kept as well and removes nothing; a sentence a speaker says
twice keeps both segments. The broader options, restoring any dropped segment whose words no kept segment holds, or treating the
repetition in recognition (with #474), were not taken. After the fix RM09's chunk 37 segment is kept, and the seam's missing words
went from 16 to 9 (the rest are the language limit, L-188). Tests: `a_repeated_copy_in_one_chunk_no_longer_hides_the_next_chunks_words`
(it failed before), `a_sentence_said_twice_is_kept_twice`, `only_a_segment_with_the_same_words_as_the_one_before_is_a_repeat`;
the properties against the frozen earlier merge, which use unique words, are unchanged.

## Note, 2026-10-21 (0.3.0, ADR 0033): a revision is published as its run goes

This record says a run commits its revision once, when it ends, and that a failed or cancelled run
commits nothing. Since 0.3.0 that holds only for a run that publishes at its seal (a worker's step).
`transcript retranscribe` and `job resume` publish the settled part of their merge while they run, as
parts of an open revision with its final identity; the merge rules of this record are unchanged, and
the sealed revision is the one a run that published nothing seals. A run that is cancelled, or fails in
a way it cannot resume from, after it published part of its revision seals it as stopped, keeping every
published segment ([ADR 0033](0033-progress-during-long-transcriptions.md), D1). The text above is
kept as written.

## 2026-10-28 note: a segment that loops on its own words claims no time (#539)

Amends rules 1 and 2 of `merge_chunks` again, beside the 2026-10-20 note; the guard, rule 3 and the tolerance are unchanged. On RM03
(hosted runs 40654763827 and 40595722898, `real-media-set.md` section 14.6) chunk 3 heard English as Turkish and wrote one segment over
its whole window, 75 to 105 s, one word repeated fifty times. The loop owned the time of chunk 2's last words in the overlap, so rule 1
dropped them, and the guard saw no hole because the loop covered it: 15 words at the seam at 75 to 80 s were in no segment and coverage
called the stretch transcribed (L-201; the loop itself is L-188's limit). The #485 rule did not see it, because the loop is inside one
segment. **The decision (the maintainer's, 2026-10-28, the narrow merge rule; the language design of #474 stays in `1.0.0`):** a
segment in which a unit of one to four normalised words is said at least eight times in a row, the repeats making up at least half of
its words, also claims no time in rules 1 and 2. As with #485 it is still kept by its own chunk, so the rule removes nothing; where it
errs on speech that really is repetitive, the cost is a neighbour's copy kept as well, a visible repeat. The thresholds keep a word said
a few times for emphasis, and a loop that is a small part of a longer segment, out of the rule. `MERGE_RULES_VERSION` goes to 2
(ADR 0033); no release carried version 1, so no stored open revision is affected. Tests:
`a_segment_that_loops_on_its_own_words_no_longer_hides_the_chunk_before` (it failed before),
`only_a_segment_mostly_made_of_one_repeated_unit_loops`, the merge-rules digest sweep and ADR 0033's prefix property, both now with
looping segments; the properties against the frozen earlier merge, which use unique words, are unchanged. The hosted run on the fix is
in `real-media-set.md` section 14.7.
