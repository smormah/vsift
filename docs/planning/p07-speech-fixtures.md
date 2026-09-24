# P07 speech test fixtures

Status: tooling ready; the speech fixtures themselves are committed only after the
maintainer runs the generation workflow and reviews its artifact. Date: 2026-09-24.
Packet: P07 (local ASR needs real speech; the P04 corpus has tone sentinels only).

The maintainer approved generating P07 speech fixtures with the Kokoro text-to-speech
model **for testing only**. Kokoro, PyTorch and their Python dependencies are never a
VSift runtime dependency. They are never shipped or vendored, never named in a Cargo
manifest, and never installed on a maintainer or user machine. They run only on a
disposable GitHub Actions runner that has read-only repository access.

## Licence review

### Conclusion

The generated speech audio (utterance WAVs and the `*-speech` video variants) may be
committed under the repository's `MIT OR Apache-2.0` licence, like the rest of the
corpus. Nothing reviewed restricts the output, and every copyleft component is used only
as a generation tool on the runner. None is distributed.

1. **Model and voices: Apache-2.0.** The `hexgrad/Kokoro-82M` repository declares
   `license: apache-2.0` in its model card metadata. The card says "With Apache-licensed
   weights, Kokoro can be deployed anywhere from production environments to personal
   projects." The voice files are part of that same repository, and the card gives them
   no separate licence. Apache-2.0 places no condition on what the software produces.
   Its notice and attribution conditions apply only when the Work or a Derivative Work
   is redistributed. We distribute neither the weights nor the voice files. A synthetic
   reading of our own sentence does not reproduce the model's expression.
2. **Input text is project-owned.** Each script is the frozen `audio.script` in
   `fixtures/corpus/manifest.json`, which is already licensed `MIT OR Apache-2.0`.
3. **GPL tools create no obligation on the output.** Kokoro's grapheme-to-phoneme
   library, `misaki`, uses eSpeak NG (GPL-3.0-or-later) through `phonemizer-fork`
   (GPL-3.0-or-later). The eSpeak NG library comes bundled in the `espeakng-loader` wheel
   (the loader itself is MIT). Spanish uses eSpeak NG for all phonemization; English
   uses it only for words outside misaki's lexicon. Numbers are expanded by `num2words`
   (LGPL). The GNU GPL FAQ says a program's output is covered by the GPL only when the
   program copies part of itself into that output. Here the output is an IPA
   transcription of our own text, which Kokoro then turns into audio. No part of eSpeak
   NG's code or data is copied into the audio. These tools run on the disposable
   runner and are not redistributed, so their licences create no obligation for
   the repository. The provenance file records the IPA strings for our own sentences
   so they can be reviewed. That record is a mechanical transcription, not eSpeak NG
   material.
4. **Training data (residual, low).** The model card says Kokoro was trained on
   "permissive/non-copyrighted audio data". That includes public-domain audio,
   Apache/MIT-licensed audio, and synthetic audio from closed TTS providers. The card
   also says no synthetic audio came from open TTS models or "custom voice clones". It
   says a small amount of CC BY data was used: under 1 hour of Koniwa (Japanese) and
   under 11 hours of SIWIS (French). These claims come from the model's author and
   cannot be checked independently. Any claim about the training data would concern the
   weights, not a ten-second synthetic reading of our own text. The two voices used
   here are not the Japanese or French voices. Mitigation: the fixtures are test data
   only. They are regenerable, replaceable, and never part of a VSift release artifact.
5. **Likeness.** `af_heart` and `ef_dora` are synthetic voice embeddings. They are not
   presented as, and not claimed to imitate, any real person.

No component was unclear or restrictive enough to stop the work. Attribution is not
required. We still name Kokoro and the voices in `fixtures/corpus/README.md` and in the
provenance, because that is good practice.

### Components

| Component | Pinned version | Licence | Role | Distributed by VSift |
| --- | --- | --- | --- | --- |
| Kokoro-82M weights `kokoro-v1_0.pth` | HF revision `f3ff3571791e39611d31c381e3a41a3af07b4987`; 327,212,226 B; SHA-256 `496dba11…ad1e4` | Apache-2.0 | TTS model | No |
| Kokoro-82M `config.json` | same revision; 2,351 B; git blob SHA-1 `14a726ed…5b16a` | Apache-2.0 | Model configuration | No |
| Voice `af_heart` (American English, female, grade A) | same revision; 523,425 B; SHA-256 `0ab5709b…cb4ff` | Apache-2.0 | English voice | No |
| Voice `ef_dora` (Spanish, female) | same revision; 523,420 B; SHA-256 `d9d69b0f…35530` | Apache-2.0 | Spanish voice (F08 second sentence) | No |
| `kokoro` | 0.9.4 | Apache-2.0 | Inference pipeline | No |
| `misaki[en]` | 0.9.4 | Apache-2.0 | Grapheme-to-phoneme | No |
| `phonemizer-fork` | 3.3.2 | GPL-3.0-or-later | eSpeak NG binding | No |
| `espeakng-loader` (bundles eSpeak NG) | 0.2.4 | MIT loader; eSpeak NG GPL-3.0-or-later | Phonemizer backend | No |
| `num2words` | 0.5.14 | LGPL | Number expansion | No |
| `spacy` / `en_core_web_sm` | 3.8.7 / 3.8.0 | MIT / MIT | English tokenization | No |
| `torch` (CPU wheel) | 2.7.1+cpu | BSD-3-Clause | Tensor runtime | No |
| `transformers` / `huggingface-hub` / `tokenizers` | 5.17.0 / 1.33.0 / 0.23.2 | Apache-2.0 | Model classes / hub client / tokenizer | No |
| `numpy` | 2.2.6 | BSD-3-Clause | Arrays | No |
| FFmpeg (BtbN `n9.0.1-11-ge47273f4d9` LGPL build, as reviewed for P06) | archive SHA-256 pinned in `tools/p06_ubuntu_candidate_smoke.py` | LGPL-2.1-or-later | Mux and AAC/PCM encode | No |
| Generated utterances and speech variants | recorded in `speech-provenance.json` | MIT OR Apache-2.0 | Test fixtures | In the repository |

The full set of resolved distributions, including transitive dependencies, is
recorded in `speech-provenance.json` (`synthesis.resolved_distributions`). Each entry
comes from `pip install --report` and carries its download URL and SHA-256.

### Sources (read 2026-09-24)

- Kokoro model card and metadata at the pinned revision:
  <https://huggingface.co/hexgrad/Kokoro-82M/blob/f3ff3571791e39611d31c381e3a41a3af07b4987/README.md>
- Voice list with digests: <https://huggingface.co/hexgrad/Kokoro-82M/blob/f3ff3571791e39611d31c381e3a41a3af07b4987/VOICES.md>
- File sizes and LFS digests: <https://huggingface.co/api/models/hexgrad/Kokoro-82M/tree/f3ff3571791e39611d31c381e3a41a3af07b4987>
  and `/voices` under the same path
- `kokoro` package: <https://pypi.org/project/kokoro/0.9.4/>; source: <https://github.com/hexgrad/kokoro>
- `misaki` package: <https://pypi.org/project/misaki/0.9.4/>; source: <https://github.com/hexgrad/misaki>
- `phonemizer-fork`: <https://pypi.org/project/phonemizer-fork/3.3.2/>
- `espeakng-loader`: <https://github.com/thewh1teagle/espeakng-loader>
- eSpeak NG licence: <https://github.com/espeak-ng/espeak-ng> (README "License" section and `COPYING`)
- GNU GPL FAQ, output of GPL programs: <https://www.gnu.org/licenses/gpl-faq.html#WhatCaseIsOutputGPL>
- `num2words`: <https://pypi.org/project/num2words/0.5.14/>
- spaCy and `en_core_web_sm`: <https://pypi.org/project/spacy/3.8.7/>,
  <https://github.com/explosion/spacy-models/releases/tag/en_core_web_sm-3.8.0>
- `transformers`: <https://pypi.org/project/transformers/5.17.0/>; `huggingface-hub`:
  <https://pypi.org/project/huggingface-hub/1.33.0/>; `numpy`: <https://pypi.org/project/numpy/2.2.6/>
- Apache License 2.0: <https://www.apache.org/licenses/LICENSE-2.0>

## Recipe

`tools/generate_p07_speech.py` has three stages. Only `synthesize` needs PyTorch.

1. **`fetch`** (standard library only) downloads the pinned model, config and voice
   files and the spaCy English pipeline wheel. It uses HTTPS only and refuses
   credential-bearing or non-HTTPS redirects. Each download is bounded to its reviewed
   size. Model and voice files are checked by the published SHA-256; `config.json` is
   checked by its published git blob SHA-1. GitHub publishes no digest for the spaCy
   wheel, so that wheel is pinned by URL and exact size, and the stage records the
   SHA-256 it observed.
2. **`synthesize`** first refuses to run unless every pinned distribution is installed
   at its exact version. It then loads the model and voices from local files only (the
   workflow sets `HF_HUB_OFFLINE=1`). Each speech-bearing fixture's frozen script is
   spoken with speed 1.0, one CPU thread, deterministic algorithms, and
   `torch.manual_seed(731)` reset before every segment. The model runs in `eval()`
   mode, so dropout is off. Each fixture gets one clean utterance,
   `speech/<id>-utterance.wav` (mono, 16-bit, 24 kHz, Kokoro's native rate). Samples
   are quantized as `round(x * 32767)` and any clipping is counted. The stage records
   the phonemes and Kokoro's word timings. If an utterance would not fit its window, it
   fails before writing anything.
3. **`assemble`** (standard library plus an explicit FFmpeg) re-reads the committed
   utterances and places each one on its fixture's stream-local audio timeline. It adds
   noise where the mode requires it. It then muxes a new `<id>-speech` variant: the P04
   video stream is copied packet for packet, and the placed track is encoded exactly as
   P04 did its tone (MP4: AAC 64 kbit/s; F09 Matroska: PCM s16le), mono at 16 kHz.
   FFmpeg runs with explicit arguments, `-n` (never overwrite) and `+bitexact`. Each P04
   source is checked against `generated/provenance.json` before it is read and is never
   written.

### What is spoken, and where

| Manifest audio mode | Fixtures | Speech variant |
| --- | --- | --- |
| `clean-synthetic` | F01-F07, F12 | Utterance over digital silence |
| `noisy-synthetic` | F08 | Utterance over deterministic office noise |
| `offset-synthetic` | F09 | Utterance on the 750 ms delayed audio stream of the Matroska VFR clip |
| `none` | F10, F11 | None (no audio by truth) |

The generator maps modes exhaustively, so a new mode fails generation until it is
reviewed. Scripts are split into sentences only at whitespace after `.`, `!` or `?`,
which keeps decimals such as `2.5` and `125.00` intact. The split must be lossless.
Sentences are spoken in American English (`af_heart`). The one exception is a
reviewed per-sentence language list: F08 is `en-US` then `es` (`ef_dora`). The
generator checks that list against the manifest's sentence count. Language segments
are joined by a fixed 400 ms pause.

Placement:

- **With a manifest speech event** (F08-E01 1.0-17.0 s; F09-E02 4.0-6.0 s): the
  utterance starts exactly at the event start and must end inside the event.
- **Without one** (the clean fixtures): the utterance starts at a fixed 500 ms lead-in
  and must end at least 250 ms before the clip ends. This is a generation policy, not
  truth. Where speech falls relative to the visual events of clean fixtures is not
  asserted anywhere.
- **Offsets:** F09's audio stream begins at `offset_us` (750 ms) after the container
  origin, as in P04. The utterance therefore starts 3.25 s into the stream and 4.0 s
  on the normalized timeline. The 2 s container origin is taken from the reviewed
  P04 command record, and the verifier checks it against the P04 file.
- All placement times must fall exactly on the 24 kHz sample grid.

### Noise recipe `office-v1` (F08)

Pink noise is Paul Kellet's economy filter applied to uniform white noise. Sparse
keyboard-like clicks are added: 10 ms bursts decaying by 0.99 per sample, spaced
150-600 ms apart. Everything comes from `random.Random(408).random()`, a sequence Python
guarantees to be stable across versions. The noise covers the whole audio track. It is
scaled so that the RMS of the clean utterance over the RMS of the noise equals a power
ratio of 10 (10 dB SNR). If the mix would exceed full scale, a single gain is applied.
Only `+`, `*`, `/` and `sqrt` are used. These are correctly rounded IEEE operations, so
the placed track is bit-identical on every platform. A unit test pins a golden digest
of the noise.

## Determinism

- The fetch, assembly and verification stages are deterministic for a given FFmpeg
  build. The placed track's SHA-256 is recorded, and a second assembly produced
  identical bytes in the local test suite.
- Kokoro's decoder draws random noise and phase at inference. The recipe fixes the
  seed, the thread count and deterministic algorithms. The workflow synthesizes a
  second time on the same runner and requires byte-identical output. Bit-identical
  output across different CPUs or PyTorch builds is **not** guaranteed. The committed
  bytes and their recorded digests are authoritative. The verifier checks committed
  files against the record; it never compares them with a fresh regeneration.

## Keeping truth independent

- `manifest.json` stays the only truth source. The generator reads scripts, modes,
  offsets and speech windows from it and never writes to it.
- `speech_start_us` and `speech_end_us` are exact by construction: they are the
  samples where the generator placed the utterance. They are never measured from the
  output.
- `tts_word_timings` come from Kokoro's own duration predictor (12.5 ms steps; English
  segments only, because Kokoro reports no timings for eSpeak-phonemized Spanish). They
  are generation facts, labelled as engine-reported, not independent truth. ASR tests
  must allow tolerance around them. No expected timestamp is ever produced by running
  an ASR on the output.
- `tools/verify_p07_speech.py` shares no code with the generator. It derives the
  speech-bearing fixtures, offsets and speech windows from the manifest itself. It
  checks every hash, byte count and WAV header. It checks that the spoken segment text
  joins back into the frozen script, that each variant has exactly one video and one
  audio stream, and that the video packets hash identically to the P04 source. It also
  checks the codec, rate, channel count, container origin, audio offset and duration.
  Finally it decodes the audio. In clean tracks, every speech-active 20 ms window must
  lie inside the recorded span (with 100 ms tolerance for AAC framing), and that span
  must lie inside the manifest window. In F08, noise must cover the track, and the
  speech span must stand at least 4 dB above the noise-only region.

## Regenerating

1. Dispatch **P07 speech fixtures** (`.github/workflows/p07-speech-fixtures.yml`) on
   `main`. It runs the model-free tests. It downloads the P06-pinned FFmpeg and reruns
   the tests with FFmpeg against it. It fetches and pins the model, installs the pinned
   environment (PyTorch from the CPU index), then synthesizes, assembles and verifies.
   It uploads two artifacts: `p07-speech-fixtures` and `p07-speech-environment` (pip
   reports, `pip freeze`, fetch record). As a last step it regenerates and requires
   identical bytes.
2. Use the artifact only if the whole job is green, including the repeatability step.
3. Copy the contents of `p07-speech-fixtures` into `fixtures/corpus/generated/`:
   `speech/*.wav`, `*-speech.mp4`, `F09-speech.mkv`, `speech-provenance.json` and
   `speech-verification.json`.
4. Review before committing:
   - listen to each clip;
   - read the `phonemes` in `speech-provenance.json` for identifiers such as
     `AB-731`, `E-409`, `p95`, `10:32` and `SAFE-12`;
   - confirm `clipped_samples` is zero or explained.

   A mispronunciation is fixed by a reviewed recipe change, never by editing the
   manifest.
5. Locally, run `python tools/verify_p07_speech.py --ffmpeg <ffmpeg> --ffprobe <ffprobe>`.
   This needs no PyTorch. It rewrites `speech-verification.json` with this machine's
   independent result.
6. Commit the files with the workflow run URL in the pull request. After review, pin
   the spaCy wheel's SHA-256 recorded under `synthesis.environment.spacy_model_wheel`
   in `SPACY_MODEL_WHEEL`, so later runs check a digest rather than a size.

If an utterance does not fit its window, generation stops. F09's is the tightest: 2 s
for "Marker beta is visible now." Changing the speed or placement for one fixture is a
reviewed recipe change recorded here.

## Known limits

- Direct dependencies are pinned by exact version. The transitive set is resolved by
  pip and recorded with digests; it is not installed from a hash-locked file.
- The spaCy wheel is pinned by size until its first recorded digest is reviewed and
  pinned (step 6 above).
- Clean fixtures' speech is not aligned with their visual events. Cross-modal timing
  truth for those fixtures does not exist in the manifest and is not implied.
- The Spanish segment has no word timings.

## 2026-09-24 note: patched transformers

GitHub dependency review flagged `transformers` 4.51.3 (high-severity code-execution and
path-traversal advisories, plus ReDoS). All are fixed only from 5.10.0, so the generator now
pins `transformers` 5.17.0 with `huggingface-hub` 1.33.0 and `tokenizers` 0.23.2 (required by
transformers 5.x). Kokoro 0.9.4 declares `transformers` without a version bound; its
compatibility with 5.x is proven only by the first workflow run, which must pass the
repeatability step before any clip is committed.
