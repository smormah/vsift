# ADR 0031: Speaker labels ("who said what") by an external diarization engine

- Status: **Accepted, 2026-10-21**, by the maintainer's decisions on the engine evaluation
  ([`speaker-labels-evaluation.md`](../planning/speaker-labels-evaluation.md) section 9), relayed
  by the supervisor the same day. The design below follows them; where it names a detail the
  decisions left open, it says so.
- Date: 2026-10-21
- Tracking: [#108](https://github.com/smormah/vsift/issues/108) (speakers in live meetings; this
  ADR is its recorded-file part), the `0.3.0` scope of [ADR 0027](0027-the-road-to-1-0.md)
- Refines: R-15 (P16's diarization, [ADR 0011](0011-r1-industrial-capability-expansion.md)),
  moved into `0.3.0` by ADR 0027; ends the speaker part of the scope exclusion
  [L-046](../planning/known-limits.md#l-046) when step 2 ships; register
  [L-205](../planning/known-limits.md#l-205) to [L-207](../planning/known-limits.md#l-207).

## Context

An agent asked about a recorded call or meeting needs "who said what": which voice spoke each
cited segment. VSift's transcript segments carry a `speaker` field today only for WebVTT voice
tags of a supplied transcript; a transcript VSift made itself has none. The evaluation compared
external engines with no Python on synthetic conversations with exact truth and on a real
two-person call, and the maintainer chose on 2026-10-21:

1. **sherpa-onnx `v1.13.8`** (`sherpa-onnx-offline-speaker-diarization`, Apache-2.0) with
   **pyannote segmentation 3.0** (MIT) and **NeMo TitaNet small**, shipped with an attribution
   notice that covers both of TitaNet's licence statements (Apache-2.0 on NVIDIA's NGC page,
   CC-BY-4.0 on NVIDIA's card for the large model).
2. **Managed installs on Windows, macOS and Linux.** Another story extends the managed-install
   system to all three; this one adds sherpa-onnx and the two models as components on top of it.
   The engine archives have only GitHub's release-asset digest: they are pinned by that SHA-256,
   and the register states the provenance gap ([L-217](../planning/known-limits.md#l-217), recorded by the
   managed-install story).
3. **The count:** automatic at the distance threshold 0.9 when the user gives none; a given count
   is passed through.
4. **Labels are provisional** ("S2, by voice"). The user's verified names are the only identity.
   The accuracy statement names the similar-voice case (L-207).
5. **Long recordings:** a duration cap with a typed refusal and a remediation, set by measurement,
   aiming at about two hours if an 8 GB machine allows.
6. **More evidence:** a second real recording of two or three people joins the test-media
   release; the maintainer listens to confirm RM06's read reference and the new one.
7. **Privacy:** no voiceprints are stored.

## Decision

### The engine is an adapter

- A new runtime dependency (the diarization tool) and two model files, resolved like
  whisper.cpp and its model: an explicit path, the configured path, the managed store (opened by
  digest), and never a name looked up on `PATH` for the models. The adapter runs the tool under
  the process supervisor with a closed argument list (`--segmentation.pyannote-model`,
  `--embedding.model`, the thread counts, `--clustering.num-clusters=N` or
  `--clustering.cluster-threshold=0.9`, and the WAV path), in its own executable's folder, with a
  deadline, stream bounds and cancellation, and parses only its `start -- end speaker_NN` lines.
- The engine is given a 16 kHz mono 16-bit PCM WAV that VSift writes in the operation's private
  work folder from the bound source copy, through the restricted FFmpeg adapter's speech decode,
  window after window; it never sees the source file. The WAV is removed when the run ends,
  whatever the outcome.
- **Absent engine:** a speaker request refuses with the existing `missing_capability` failure and
  a remediation naming `setup install`; every other command works as before.

### The managed install and its notice

Each managed system gets three components: the sherpa-onnx tool (with its ONNX Runtime library beside it) and the two
model files, pinned by URL, size and SHA-256 (the models by the publishers' published digests: Hugging Face's LFS
SHA-256 for the segmentation model at revision `9403a690`, sherpa-onnx's `checksum.txt` for `TitaNet` small; the engine
archives by GitHub's release-asset digest, the only one published, a provenance gap the register states as L-217). The install
ships this attribution notice with the models, which covers both of `TitaNet`'s licence statements:

> Speaker diarization uses the pyannote segmentation 3.0 model (Copyright (c) 2022 CNRS, MIT licence; ONNX export by
> the sherpa-onnx project) and NVIDIA's NeMo `TitaNet` small speaker-embedding model (Copyright NVIDIA Corporation;
> distributed under the NeMo Toolkit's Apache License 2.0, and attributed here as the Creative Commons Attribution 4.0
> licence of NVIDIA's `TitaNet` model card requires), run by sherpa-onnx (Apache License 2.0) with ONNX Runtime (MIT
> licence).

### A speaker record per transcript revision, derived on every read

- Transcript revisions stay immutable, and the stored transcript record keeps rejecting unknown
  fields. Speaker labels are their own numbered, immutable **speaker record** (`spk_...`) per
  session, naming the source and the revision it labels; the visual-candidate index is the
  precedent. The record stores the engine's turns (times and cluster numbers), what was asked
  (the count), the labelling profile, the diarized range and the engine's and models' SHA-256.
- **Labels are derived, not stored.** Clusters are renumbered `S1`, `S2`, ... by when each is
  first heard, and each segment's label is re-derived from the turns and the revision on every
  read (`speaker-labels-v1`: the speech the engine heard covers at least half of the segment, and
  one label holds at least 70% of it; otherwise "no confident speaker"). A stored record cannot
  claim a label its turns do not give.
- **Segment precision, said plainly.** VSift has segment times, not word times; a segment in
  which two people speak is not split. Every output that shows a label says so.
- **Names.** A user records a verified name for a label of one record; a name is the user's
  statement, kept beside the label and shown as such. Recording or clearing a name writes a new
  numbered speaker record with the same turns (so an earlier citation stays resolvable); a name is
  never inferred from a voice or from text on the screen.
- **Coverage.** Each output that shows speakers says which segments have a label, which have no
  confident speaker, and which lie outside the audio the engine heard (the cap), with a fixed
  warning when any part is unlabelled.

### Privacy

The engine computes its voice embeddings in its own process memory and returns times and cluster
numbers only; VSift never reads, receives or writes an embedding or any audio-derived vector. The
record holds turns, labels and the user's names, under the session's own lifetime and retention
(ADR 0002): disposable by default, kept only in a bundle the user makes. There is no cross-session
voice database, and labels of two sessions are never linked.

### Public JSON v1, additively

New commands for diarizing a revision and for recording a name; an additive speaker block on
`transcript get` segments and `search` hits (label, the provisional wording, the verified name if
any, the record), distinct from the existing imported WebVTT `speaker` field; `search --speaker`
filters by label or name and its cursor digest includes the filter; a speaker-coverage block in
the pattern of `transcript_coverage`. Schemas, contract tests, `cli-v1.md` and the guide
reference change with them.

## Consequences

- A second external engine joins the managed catalogue on three systems, with its own re-pin
  cadence (sherpa-onnx releases every one to two weeks) and its own supersession list.
- Diarization holds the whole diarized range in the engine's memory; the duration cap bounds it
  (L-205), and audio beyond the cap is reported as not diarized, never silently dropped.
- Two similar voices can share one label with nothing in the output to say so (L-207); the
  provisional wording and the accuracy statement carry that limit.
- The evidence for accuracy is synthetic speech and a few real recordings (L-206).
- A release before `0.3.0` reads a session or bundle that holds a speaker record as damaged and refuses
  the whole session (L-218, an accepted residual, as for every new stored kind).

## Alternatives considered

polyvoice (slightly more accurate automatically, but its count caps rather than sets, its memory
grew to 2.2 GiB on 28 minutes, and it is a young single-maintainer project), whisper.cpp's
tinydiarize (turn marks only, slow), other sherpa-onnx embedding models, and Python pipelines,
proprietary SDKs, non-production-licence models and engines linked into VSift's crates: section 2
and 6 of the evaluation.
