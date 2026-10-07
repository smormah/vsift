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
  A whole-source run replaces everything (`replaced_range` is the source).
- Carried segments get new identities in the new revision, so records already indexed
  under an older revision are never overwritten, and name their origin in
  `carried_from` (revision and segment identity). They always name the revision that
  first produced the text, never an intermediate copy. The revision stores
  `inherited`, the provenance of every revision it carries from, so every carried
  segment is still re-checked against the rule that produced it (an import's offset
  and cue timing, or a run's chunk audio and provider times), and must lie wholly
  outside the replaced range.
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
