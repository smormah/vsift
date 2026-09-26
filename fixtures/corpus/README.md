# VSift synthetic R0 corpus

This directory defines the public, rights-safe ground truth used to qualify the media
pipeline and agent workflow. `manifest.json` is the reviewed truth source;
`manifest.schema.json` validates its structure. P04 fixture media is generated from
deterministic project-owned recipes in `tools/generate_p04_fixtures.py`; generated
hashes and command provenance are recorded in `generated/provenance.json`. Run the
independent `tools/verify_p04_fixtures.py` before using generated media as evidence.

The manifest deliberately separates expected evidence from future selection code.
An implementation may not generate expected timestamps by asking its own algorithm
where events occurred. Changes to truth require review and a reason, especially when
a failing benchmark would become easier.

Rules:

- Use synthetic names, speech, screens and values. Never include real meetings,
  credentials, private source code or third-party media.
- Keep generation inputs editable and licence them under the repository's dual licence.
- Record actual generated-file hashes, generator version and FFmpeg invocation before
  using a fixture as release evidence.
- Treat F11 malformed variants as hostile. Generate/run them only in disposable,
  resource-limited test environments.
- F12 contains synthetic prompt-injection text; it is test data, never an instruction.
- Every expected event uses integer microseconds on the normalized presentation timeline.

P00 defines truth. P04 generates F01-F10/F12 visual media, F11 malformed variants,
tone-based audio sentinels and rotated/audio-track variants. The verifier independently
checks generated hashes, timestamps and selected decoded pixels. The source recipes
are reproducible with the same FFmpeg build; `generated/provenance.json` records that
build's version and hash. P07 adds speech variants of the speech-bearing fixtures
(below); the tone fixtures stay unchanged. P08/P12 use the frozen corpus for measured retrieval and agent evaluation. See the
[P04 qualification record](../../docs/planning/p04-media-qualification.md).

## Speech variants (P07)

Local-ASR tests need real speech; the P04 tone sentinels stay as they are, byte for
byte. `tools/generate_p07_speech.py` speaks each speech-bearing fixture's frozen
`audio.script` with the [Kokoro](https://huggingface.co/hexgrad/Kokoro-82M) text-to-speech
model (Apache-2.0; voices `af_heart` and, for F08's Spanish sentence, `ef_dora`).
**Kokoro is test tooling only**: it runs in the manually dispatched
`.github/workflows/p07-speech-fixtures.yml` on a disposable runner, is never a VSift
dependency, and is never shipped. The licence review and the full recipe are in
[p07-speech-fixtures.md](../../docs/planning/p07-speech-fixtures.md).

| Files | Content |
| --- | --- |
| `generated/speech/<id>-utterance.wav` | Clean Kokoro utterance, mono 16-bit 24 kHz |
| `generated/<id>-speech.mp4` (F01-F08, F12) | P04 video stream copied unchanged plus the placed speech as AAC, mono 16 kHz |
| `generated/F09-speech.mkv` | P04 VFR video, 2 s origin, speech as PCM on the 750 ms delayed audio stream |
| `generated/speech-provenance.json` | Model revision, voices, pinned environment, parameters, FFmpeg build, placement, noise, hashes and sizes |
| `generated/speech-verification.json` | Result of the independent `tools/verify_p07_speech.py` |

The manifest remains the truth source. Speech starts at the manifest speech event where
one exists (F08, F09) and must end inside it; otherwise it starts after a fixed 500 ms
lead-in, which is a generation policy and not timing truth. F08 carries deterministic
office noise at 10 dB SNR. The provenance records where each utterance was placed
(exact by construction) and Kokoro's own word timings, labelled as engine-reported; no
expected time is ever derived by running an ASR on the output.

To regenerate, dispatch the workflow, check that the job (including its repeatability
step) is green, copy the `p07-speech-fixtures` artifact into `generated/`, listen to the
clips and review the recorded phonemes, then run the verifier locally, which needs no
PyTorch:

```console
python tools/verify_p07_speech.py --ffmpeg /absolute/path/to/ffmpeg --ffprobe /absolute/path/to/ffprobe
```

`python -m unittest discover -s tools -p test_p07_speech.py` tests the recipe and the
verifier without the model (the assembly tests also need FFmpeg and FFprobe).

## Recorded visual samples (P08)

The visual-candidate index is gated on what FFmpeg actually decodes from F01-F10 and
F12. `crates/vsift-infrastructure/tests/data/visual_samples/<id>.json` records, per
fixture, every sample of every 60 s window (time, 16x9 block means, 64-bit difference
hash; no pixels) as `FfmpegMedia::visual_samples` decoded it, with the FFmpeg version,
the sampling profile and the date. The always-run recall test scores candidates built
from them against this manifest only; see `docs/development.md` to re-record them.

Finding (2026-09-26): three events are drawn with exactly the pixels of the state
before them, because `scene()` in `tools/generate_p04_fixtures.py` renders no
difference for them: F04-E02 (the scroll: the table still shows row 1001 until F04-E03
starts), F05-E02 (no loading indicator is drawn) and F12-E02 (the defect code screen
is drawn from the start). No visual method can see these events begin, so the recall
report lists them as corpus limitations (re-verified from the samples on every run)
instead of changing the truth. F12-E02 is still hit by periodic coverage. Regenerating
the motion fixtures with rendered scrolling and a loading state is a separate,
reviewed truth change.

## Supplied-transcript sidecars (P07)

`transcripts/F10.srt` and `transcripts/F10.vtt` are hand-written, rights-safe synthetic
sidecars for F10 under the repository licence. They are equivalent cue for cue and are
written 500 ms early, so the explicit offset `--transcript-offset 500000` places the cue
"Dialog R-17 is displayed now." exactly on F10-E01's frozen truth window
(5,000,000-9,000,000 us); the other two cues (one names the 500 millisecond offset) lie
inside F10's 12 s duration. They do not change `manifest.json`. The repository stores
them with LF line endings, which fixes their digests:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `transcripts/F10.srt` | 245 | `2a4ee37d826754eac43da03e7cc63f718b16aed4d3ce163c380ebd8ab9630847` |
| `transcripts/F10.vtt` | 346 | `daaeb39fde45b449e2204e6b9ef99f4428267a6e073052723c997f82560c3611` |

Small malformed T-01 variants live beside their tests in
`crates/vsift-infrastructure/tests/data/transcripts`; byte-level variants (encodings,
byte-order marks, line endings, control characters) are written inline in the tests.
