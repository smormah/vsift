# ADR 0017: Local speech recognition through whisper.cpp

- Status: Accepted (maintainer, 2026-09-25)
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
   later. The engine's `Cancellation` is honoured between stages for library hosts.
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
- Known limits, measured on Windows 11 with the reviewed build: a short clip takes
  about 7.5 s end to end with verification cached (release build); the model is hashed
  three times per run (about 0.3 s each in release, so no identity cache was added).
  The base model starts a segment that follows leading silence at the start of its
  audio (F09's segment starts at 0.75 s, not 4.0 s); accuracy and timing are measured
  in 3c (T-04). Every chunk rehashes the session's source copy before FFmpeg reads it
  (the P04 check-before-use rule), which is linear in source size per chunk; long
  sources need a cheaper binding before the P14 load gates.

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
  verified against a download from that revision. Section 4 now reads "a reviewed
  pinned profile" for either; the profile is decided by file identity, recorded in
  `model_profile` and bound into the verification fingerprint, so a pass for one
  profile never stands in for the other. Only `base` is in the managed plan.
- **Seam merge fix (section 3).** A cut segment is replaced by a neighbour-chunk
  segment only when that segment spans the cut segment's midpoint; merely
  overlapping it (the previous sentence ending just inside) no longer counts. With
  `base_q5_1` the old rule dropped a whole sentence from both chunks without a
  warning; the regression test and the record describe it.
- **T-04.** Accuracy, timing and memory are measured by the opt-in
  `p07_asr_qualification` test ([record](../planning/p07-asr-qualification.md)):
  `base` meets the clean-speech, critical-term, real-time (0.388) and memory
  (338 MiB) gates and misses the F08 WER gate (61.5%). The default is a maintainer
  decision ([ADR 0005 note](0005-r0-scope-and-qualification-profiles.md)).
