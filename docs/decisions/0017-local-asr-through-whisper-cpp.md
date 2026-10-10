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
  exception, added for #353 (see the 2026-10-10 note): text the newest revision had
  wholly inside a part of the range that the run could not read is carried too.
- Carried segments get new identities in the new revision, so records already indexed
  under an older revision are never overwritten, and name their origin in
  `carried_from` (revision and segment identity). They always name the revision that
  first produced the text, never an intermediate copy. The revision stores
  `inherited`, the provenance of every revision it carries from, so every carried
  segment is still re-checked against the rule that produced it (an import's offset
  and cue timing, or a run's chunk audio and provider times), and must lie wholly
  outside the replaced range, or wholly inside a part of it that the revision's own
  run could not read (the 2026-10-10 note).
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

(Known limit [L-145](../planning/known-limits.md#l-145).) The first real recording tried with the stable `0.2.0` (a 34:36 English screencast of slides and live coding, one speaker, long pauses while
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
answered (`FEW_ANSWERED_CHUNKS = 3`; most of many answers unusable is a statement about the recording's speech) it says that this recording's speech could not be
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

**7. An unreadable part keeps the text the session already had there.** (A decision for the maintainer to confirm.) A range retranscription replaces whole segments of the
newest revision with what the run transcribes. When the run could not read part of its range, replacing the old segments there would delete text the session had, and the
user who asked for a better transcript would end up with none for that stretch: a bounded retranscription of 10 to 65 s over an imported transcript with cues at 5, 12, 40
and 60 s, whose first chunk (10 to 40 s) is unusable and whose second (35 to 65 s) is read, would lose the cue at 12 s. Instead the new revision **carries** a segment of the
superseded revision that lies wholly inside a gap of the run (decision 4), as it was, with its original provenance and `carried_from`; the cue at 12 s is kept, the cues at 40 and
60 s (inside the part the second chunk read) are replaced by what it heard, and the cue at 5 s was never in the range. Only the whole segment inside a gap is kept: a segment
that reaches out of the gap lies partly in a part the run read, and the run's own text replaces it. The rule is the domain's (`AsrRun::unusable_gaps`, applied by the use case
that builds the revision, by the revision's validation, and again when a stored record is read, so a forged record cannot place a carried segment in a part the run read, and by
the coverage), and it applies to a whole-source retranscription over an earlier revision as to a bounded one. What is said about it:
the result lists the **whole** gap as not transcribed by this run (`untranscribed_ranges`, the envelope's `coverage`), the warning says that earlier text inside it is kept as it
was and that any other words said there cannot be found, and a `search` of the revision counts the kept text as transcribed (so for the example above it lists 10 to 12 s and 14
to 35 s, not 10 to 35 s). A run that could not read a chunk also no longer claims `no_speech_recognised`: that statement is about the whole range, and the run has not heard that
there was none. No schema member is added by this decision.

### What does not change

The chunk plan, the 100 ms floor, the silence rule, the other warnings, the field names and shapes of v1 (this adds optional members), the failure codes, and the
verification of the built-in clip.

**What a worker supervisor sees (`job run`, `job batch`).** A `retranscribe` step reports the revision it committed and the job behind it, and nothing about that revision's
gaps: its status is `complete` and its `coverage` is null even when some chunks of its run were unusable, and so is the request's. This is deliberate for 0.2.1, to keep the
worker schema unchanged, and it means a supervisor that reads only `job run` is **not told** that part of a recording was not transcribed. It can tell by reading the
revision the step names: `transcript get --revision <revision_id>` shows the warning `provider_chunks_rejected` and the count of chunks with an unusable answer
(`revision.local_asr.unusable_chunks`), and a `search` of the session lists the ranges. `transcript retranscribe`, `job resume` and the other direct commands do answer
`partial`. A test pins the step's shape (`worker_contract`), and the destructuring in it stops compiling if a member for the gaps is added, so the change is made on purpose,
with the schema and these words, in a later release.

### Rolling back

A revision that holds an unusable chunk stores the outcome `unusable` and the warning `provider_chunks_rejected`, which every release before this one decodes as damage
(`INTEGRITY_FAILURE`), as L-130 records for a trimmed end. Only a session that holds such a revision is affected: every other revision is written byte for byte as before.
A run that had such a chunk would have failed on the release before. Use the newer version, or discard the session.

### Evidence, and what is not proven

Tests at the lowest layer that shows each rule: the domain (segment-count boundaries and the monotonic rule for every count up to sixteen, empty text as a marker, the
strict-majority threshold and its edges, a run's gaps, coverage of an unusable window and of a replaced one, text kept inside a gap and refused anywhere else in the replaced
range), the application (one chunk of ten, exactly half and more than half, early stop, silent chunks not counted, one of two, the job failing at once and not resumable, a
resume that does not ask again, the review scenario of text kept in an unreadable part and no `no_speech_recognised` beside an unusable chunk, a model swapped during the run),
the checkpoint and revision records (round trips, damaged records, a record of kept text and the same record with the chunk changed), the contract (the partial result, the
kept text listed as not transcribed by the run, the unchanged complete result, the failure's prose on both sides of the three-answer boundary and for the setup check, a
worker step pinned as `complete`, every schema; the failure and partial examples are produced by the application's own run over a stand-in recording, not written by hand), and
the human output with snapshots.

**Not proven:** no real recording was used (the recording is commercial and cannot be shared); the tests build the shape from recorded-style provider output. Which kind of
segment whisper.cpp returned for 27:05 to 27:35 (empty, reversed, or at or after the audio's end) is unknown, and the fix does not depend on it: with a single rejected
segment beside kept ones it is kept out, and with only rejected segments the chunk is a gap. The three constants (`MIN_SEGMENTS_FOR_REJECTION_RATIO`,
`UNUSABLE_CHUNK_SHARE_DENOMINATOR`, `FEW_ANSWERED_CHUNKS`) are proposals, chosen as above, not measured on a corpus of real recordings (the follow-up issue on real media in
the test set stands). The kept-text rule has no test over a real decode: the engine's opt-in tests that decode real speech do not exercise it.
