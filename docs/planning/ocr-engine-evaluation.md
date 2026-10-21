# OCR engine evaluation (0.3.0, P16 enrichment, R-15)

Status (2026-10-21, 0.3.0 development): **an evaluation for the maintainer's choice of engine; nothing ships.**

This is step 1 of the OCR story: on-screen text read from video frames, stored as evidence with its time and region,
and searchable together with the transcript. The adapter, the managed entry, the evidence records and search follow
the maintainer's choice. That is step 2, which brings ADR 0030 with it.

- **Pull request:** #499 (a draft until main opens for 0.3.0).
- **Requirement:** [R-15](r1-industrial-capability-expansion.md): optional, bounded, typed partial coverage, and the
  core path works without it.

## 1. The question

Which external OCR engine should VSift run as a pinned, optional tool on Windows, macOS and Linux, with no Python
runtime? The criteria:

- licence, of the engine and of its models;
- binary availability per platform, and how it would be pinned;
- size;
- accuracy on our real screen recordings (RM01, RM04 and RM10), and on text whose truth is exact;
- speed per frame;
- security posture, since the engine reads images whose pixels come from untrusted video.

## 2. How it was measured

**Where:** the workflow `P16 OCR evaluation` (`.github/workflows/p16-ocr-evaluation.yml`) on a hosted Ubuntu 24.04
runner with 4 logical CPUs. The tool is `tools/p16_ocr_evaluation.py`, with unit tests in
`tools/test_p16_ocr_evaluation.py`. Nothing ran on a development machine.

**The frames are VSift's own.** For each video, the commit's own `vsift` runs as a release build, with the pinned FFmpeg
of the managed catalogue:

- `ingest`;
- `candidates` over the whole video, periodic ones included;
- `frame get` for at most 40 candidates spread evenly.

Every engine reads the same native-size PNG files.

**Synthetic truth (exact).** The tool renders 16 screens of interface-like lines, each a label and a number:

- two columns, with one line at each of 8, 9, 10, 11, 12, 14, 16, 18 and 22 px, in DejaVu Sans or Liberation Sans;
- every fourth screen dark;
- English and Brazilian Portuguese labels, with accents;
- numbers of the shapes where a wrong digit is plausible: `4,731,145`, `15.2%`, `$67,512`, dates, times, `0.0371`,
  `v4.12.80`, `#4821`.

Each line's label and number shape come from a digest of its position, so neither follows its font size. That is 288
lines per video, holding 298 numbers (a few labels hold a digit, such as "Latency p95"). The screens are then read in four forms:

- **H.264:** encoded as RM10 is encoded (1280x720, 30 fps, 440 kbit/s);
- **H.264 jittered:** the screen shifted by up to 4 px every frame, at 300 kbit/s, so the encoder must code motion;
- **VP9:** encoded as RM04 is encoded (24 fps, 250 kbit/s);
- **the control:** the rendered screens themselves, lossless.

VSift takes the frame in the middle of each screen (`frame get --at`). Each line is scored on four measures:

- its character error rate: the least edit distance to any part of the frame's text, so that a line an engine joins
  to the next column still counts;
- whether it was read exactly;
- whether its number was read exactly;
- whether its number was **misread**: another number, within two edits of it, was written instead. This is the
  plausible wrong digit that [#173](small-text-real-media.md) found to be the worst case.

**Real recordings (relative).** RM01, RM04 and RM10 come from the test-media release, held to their pins. No person
has labelled their on-screen text, and no frame of theirs leaves the runner. So they are scored against **interface
labels whose presence a person can check in the product's own interface**, listed as `LEXICON` in the tool and
written from the products, not from the recordings:

- RM01: MediaWiki's 2013 Vector skin;
- RM04: Wikimedia Commons in Brazilian Portuguese, with English fallbacks;
- RM10: data.census.gov.

A label counts when an engine reads it as whole words on any frame. Some labels may never be on a recognised frame,
so each engine's share is given of the labels at least one engine found ("reachable"). The report also counts:

- **corroborated words:** the share of an engine's words of four letters or more that an engine of another family
  also read in the same frame, a precision proxy;
- lines per frame;
- the engine's median line confidence;
- wall and CPU seconds per frame. CPU time sums every thread.

**What leaves the runner:** the report, the engines' provenance, and our own synthetic text with each engine's reading
of it. The real recordings' frames and recognised text stay on the runner.

**The runs:**

- [39518099402](https://github.com/smormah/vsift/actions/runs/39518099402): the three-platform build;
- [39527698439](https://github.com/smormah/vsift/actions/runs/39527698439): every figure in section 4, synthetic
  and real. Run 39524331429, the run before it, gave the same synthetic figures; it matched labels as substrings and
  had no sparse-text mode.

The earlier runs on the pull request found two defects of the tool, fixed before these runs. In one, every video's
frames shared one scratch name, so Tesseract read another video's frames. In the other, a row's label followed its
font size.

## 3. The candidates

| | **Tesseract 5.5.3** (LSTM) | **PP-OCR** (PaddleOCR's models on ONNX Runtime) | **ocrs 0.13.1** |
| --- | --- | --- | --- |
| Engine licence | Apache-2.0; Leptonica 1.87.0 under its own BSD-2-Clause-style licence | Models Apache-2.0 (PaddleOCR); ONNX Runtime MIT; no runner exists (below) | MIT or Apache-2.0 |
| Model licence | `tessdata_fast` and `tessdata_best`: Apache-2.0 | Apache-2.0 | CC BY-SA 4.0 (trained on HierText) |
| Maintenance | Active: 5.5.3 of 2026-07-24, upstream pushes this week | Models active (PP-OCRv6 in 2026); ONNX Runtime 1.31.0 of 2026-10-09 | One maintainer, active (0.13.1 of 2026-09-13) |
| Official binaries | Source only. A Windows installer since 5.5.3 (NSIS `.exe`, 26.6 MB, GitHub digest). No Linux or macOS binary; distribution packages pull many shared libraries | ONNX Runtime: official archives for win-x64, osx-arm64, linux-x64 and aarch64, each with a GitHub SHA-256. Models: PaddlePaddle's own bundles, ModelScope (with RapidOCR's SHA-256 list) and copies with digests | Source on crates.io only |
| How VSift would pin it | **Built in our CI from the release tag, commit checked** (built on all three platforms: run 39518099402); `tessdata` pinned by commit with Git's blob SHA-1 checked, then SHA-256 | ONNX Runtime from Microsoft's release by URL, size and SHA-256; models by URL, size and SHA-256; **a runner VSift writes and builds** | Built in our CI with `--locked` (built on all three platforms); models from mutable S3 URLs, so they would have to be mirrored |
| Size | Binary 4.1 MB (Windows, macOS), 5.4 MB (Linux; 4.8 MB stripped), with no image library; `fast` models eng 4.1 MB and por 2.0 MB; `best` 15.4 MB and 8.2 MB | PP-OCRv5 mobile: detector 4.8 MB and Latin recogniser 7.9 MB; PP-OCRv6 small: 9.9 MB and 21.2 MB; ONNX Runtime library 29 MB on Linux; a runner of a few MB | Binary 8.6 to 12.2 MB; models 2.5 MB and 9.7 MB |
| Scripts | Per-language models (eng, por and 100 more) | Latin model (Portuguese, Spanish and others); PP-OCRv6 one multilingual model | Latin letters without diacritics: "Página" is read "P?gina" |
| Confidence | Per word | Per line | **None** |

**Considered and left out:**

- Windows OCR (`Windows.Media.Ocr`) and Apple's Vision: one platform each, behaviour that differs by OS version;
- RapidOcrOnnx: the last binaries are of 2023 (and 2024 for Windows), with PP-OCRv3-era models;
- PaddleOCR-json: last released in 2024, about 100 MB, no macOS build;
- oar-ocr's CLI: not released; it carries PDF rendering and vision-language models;
- EasyOCR, docTR and Surya: Python.

## 4. Results

### 4.1 Synthetic text with exact truth (H.264 like RM10)

The jittered H.264, the VP9 and the lossless control give the same exact-line counts within five lines of 288 for
every engine but ocrs (within seven). At these rates, in these synthetic screens, compression is not what limits the
engines (section 8).

The size columns give the share of lines of that size read exactly.

| Engine | Lines read exactly | Numbers read exactly | Numbers misread (of 298) | 8 px | 9 px | 10 px | 12 px | Wall s/frame | CPU s/frame |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Tesseract `fast` | 74% | 81% | 34 | 0% | 41% | 78% | 91% | 0.22 | 0.22 |
| Tesseract `best` | 77% | 81% | 29 | 3% | 41% | 72% | 100% | 0.54 | 0.54 |
| Tesseract `fast`, enlarged 2x | 92% | 96% | 11 | 69% | 94% | 97% | 88% | 0.43 | 0.43 |
| Tesseract `best`, enlarged 2x | 92% | 98% | 5 | 59% | 97% | 94% | 91% | 0.82 | 0.82 |
| Tesseract `fast`, enlarged 2x, sparse text | 92% | 98% | 7 | 72% | 91% | 97% | 91% | 0.43 | 0.43 |
| Tesseract `best`, enlarged 2x, sparse text | 91% | 98% | 6 | 63% | 97% | 94% | 84% | 0.85 | 0.85 |
| **PP-OCRv5 mobile, Latin** | **99%** | **100%** | **0** | 94% | 100% | 100% | 100% | 0.49 (4 threads) | 1.97 |
| PP-OCRv5 mobile, Latin, 1 thread | 99% | 100% | 0 | 94% | 100% | 100% | 100% | 0.75 | 0.76 |
| PP-OCRv6 small | 98% | 100% | 0 | 88% | 97% | 100% | 100% | 0.63 (4 threads) | 2.51 |
| ocrs | 58% | 81% | 46 | 38% | 50% | 44% | 63% | 0.64 | 1.24 |

From 11 px up, PP-OCR read every line exactly, and Tesseract enlarged 2x read 84 to 100% of them. Across the four
forms:

- PP-OCRv5 misread 0 numbers, or 1 in VP9;
- Tesseract enlarged 2x misread 5 to 11;
- Tesseract at native size misread 24 to 35;
- ocrs misread 41 to 55, and wrote "?" for every accented letter.

### 4.2 The real screen recordings

Each cell gives RM01, RM04 and RM10, in that order. Labels are counted of those some engine found: 29 of RM01's 34,
14 of RM04's 20 and 25 of RM10's 29. The frames were 26 (every candidate), 40 of 138 and 40 of 108.

| Engine | Labels found | Corroborated words | Lines per frame | Median line confidence | CPU s/frame (wall) |
| --- | --- | --- | --- | --- | --- |
| Tesseract `fast` | 28, 9, 23 | 93%, 86%, 78% | 49, 12, 21 | 0.90, 0.87, 0.85 | 0.57, 0.26, 0.33 |
| Tesseract `best` | 29, 9, 23 | 90%, 85%, 79% | 49, 12, 21 | 0.88, 0.88, 0.82 | 1.40, 0.60, 0.71 |
| Tesseract `fast`, enlarged 2x | 29, 10, 25 | 93%, 92%, 80% | 48, 12, 28 | 0.96, 0.91, 0.91 | 1.14, 0.57, 0.76 |
| Tesseract `best`, enlarged 2x | 29, 10, 25 | 93%, 91%, 84% | 48, 12, 28 | 0.95, 0.91, 0.90 | 2.40, 1.19, 1.54 |
| Tesseract `fast`, enlarged 2x, sparse text | 29, 11, 22 | 93%, 84%, 74% | 59, 43, 97 | 0.96, 0.92, 0.90 | 1.11, 0.69, 0.81 |
| **PP-OCRv5 mobile, Latin** | **29, 13, 25** | 96%, 86%, 88% | 58, 26, 69 | 0.99, 0.99, 0.99 | 6.77, 3.65, 6.62 (1.71, 0.93, 1.66) |
| PP-OCRv5 mobile, Latin, 1 thread | 29, 13, 25 | 96%, 86%, 88% | 58, 26, 69 | 0.99, 0.99, 0.99 | 3.02, 1.56, 3.09 |
| PP-OCRv6 small | 29, 13, 25 | 95%, 86%, 89% | 61, 24, 75 | 1.00, 1.00, 1.00 | 8.56, 4.26, 9.31 (2.17, 1.07, 2.34) |
| ocrs | 28, 11, 25 | 84%, 64%, 79% | 55, 24, 52 | none | 3.66, 1.74, 3.64 |

**PP-OCR found the most interface labels:** every reachable one on RM01 and RM10, and 13 of 14 on RM04.

- **The labels only PP-OCR read** (ocrs read some of them too):
  - on RM04, "Pesquisar", "Retrata" and "Wikidata";
  - on RM10, "Census Bureau" and "Download" at native size; Tesseract read these two only when enlarged.
- **Tesseract at its default page mode finds far fewer lines on the 720p recordings:** 12 and 28 lines a frame on
  RM04 and RM10, where PP-OCR finds 26 and 69. What it reads, it mostly reads well: 78 to 92% of its words are
  corroborated.
- **Tesseract's sparse-text mode finds more lines** (43 and 97), but fewer of its words are corroborated (84% and
  74%), and on RM10 it lost three labels.
- **No engine read** RM04's Portuguese navigation labels ("Página principal", "Mudanças recentes", "Ver histórico")
  or RM01's toolbox links. They were probably not on any recognised frame, or too small; only a person's look at
  the frames can tell, and no frame left the runner.
- **The confidence scores are not comparable across engines.** PP-OCR's median of 0.99 says nothing about its errors.
  Tesseract's per-word confidence was 0.82 to 0.96.

### 4.3 What the candidates reach

In an earlier version of the synthetic screens, each screen kept its labels and layout and only the numbers
changed. On it, `candidates` reached **9 of the 16 screens**: the first frame, the switches between light and dark,
and two others (run 39519243719, H.264 and VP9 alike). No periodic candidate filled the gaps, which were shorter than
the 10-second period.

When labels change as well, every screen is reached (16 of 16). This is [L-028](known-limits.md#l-028): a changed
digit or word is often not a candidate. It matters for OCR, because a pass over candidates alone will not read a
value that changes in place.

On the real recordings, RM04 and RM10 each had six windows whose candidate budget ran out
([L-029](known-limits.md#l-029)), so a pass over candidates skips frames there too. The coverage that OCR reports
must say both.

## 5. Security posture

Every engine runs as an external process under VSift's process supervisor, with its time, memory and output bounded,
like whisper.cpp. Its input is a frame that VSift's own FFmpeg decoded and wrote: the container, the codec and the
file format are never the engine's to parse. The pixels, and so the text, are still untrusted.

- **Tesseract with Leptonica (C and C++).** Both are in OSS-Fuzz. Leptonica's fuzzing has produced memory-safety CVEs
  in pixel processing as well as in image decoders: CVE-2020-36277 to CVE-2020-36281 cover over-reads in
  border-following, raster operations, colour quantisation and the TIFF reader, all fixed in 1.80.0. The evaluated
  build removes every image library, the network, archives and the legacy engine, and Leptonica reads only PNM, which
  VSift writes. Layout analysis of attacker-chosen pixels remains, in C and C++.
- **PP-OCR on ONNX Runtime.**
  - The models are fixed, pinned files; ONNX Runtime's known weaknesses are mostly in model parsing, which a pinned
    model does not expose.
  - The runner would decode VSift's PNG in Rust and hand the runtime tensors of a fixed shape.
  - The pre- and post-processing (resizing, thresholding the detection map, decoding the recogniser's output) would
    be VSift's own Rust.
  - The attack surface is then the runtime's numeric kernels on tensors of bounded size.
  - The cost: the runtime is a 29 MB C++ library that VSift does not build.
- **ocrs (Rust).** It is memory-safe apart from the `unsafe` blocks of its numeric kernels (`rten`), and decodes with
  the `image` crate. Its lock file pins a yanked crate (`spin` 0.9.8), and its models come from unversioned URLs.

**Text read off the screen is untrusted evidence.** It aids search and is never promoted to source truth. A
recognised instruction is a hint, not a command ([SEC-28](security-threat-model.md)). This holds for every engine.

## 6. Recommendation

**PP-OCRv5 mobile with the Latin recogniser, run by a small standalone runner that VSift builds, on ONNX Runtime from
Microsoft's release. Opt-in in 0.3.0.**

- **Why PP-OCR:**
  - It is the most accurate on the measure that matters most, numbers. It misread 0 of 298 numbers in each of the
    lossless control, H.264 and the jittered H.264, and 1 in VP9, against 5 to 11 for Tesseract enlarged twice and
    24 to 35 for Tesseract at native size.
  - It read 94% of the 8 px lines exactly.
  - On the real recordings it finds the most text: every reachable interface label on RM01 and RM10 and 13 of 14 on
    RM04, with twice Tesseract's lines per frame on the 720p recordings (26 and 69 against 12 and 28).
  - Its licences are Apache-2.0 and MIT.
  - PP-OCRv5 mobile is as accurate as PP-OCRv6 small and a fifth faster.
- **Its cost:**
  - No maintained standalone runner exists, so VSift writes one: Rust, loading ONNX Runtime and the two models, PNG
    in, JSON lines out.
  - VSift then builds and publishes it as its own pinned binary for the three platforms. This is the first engine
    binary VSift itself would publish.
  - CPU: about 0.7 to 3 CPU seconds a frame on one core (1.5 to 3.1 s on the real recordings), two to four times
    Tesseract's.
- **Why opt-in:**
  - A 5-minute recording such as RM04 has 138 candidates, about 3.5 CPU minutes of OCR.
  - It is an enrichment under R-15: the core path must not depend on it.
  - The maintainer decides (section 7).
- **The fallback, if the maintainer prefers not to own a runner:** Tesseract 5.5.3 with `tessdata_fast`, its frames
  enlarged twice for recognition only.
  - Its figures: 92% of lines exact, 96% of numbers, 8 to 11 misread of 298, and two to four times cheaper in CPU.
  - Its binary is still ours to build on Linux and macOS (proved in run 39518099402); upstream ships source and a
    Windows installer.
  - It reads fewer lines on the 720p recordings (section 4.2).
- **Not ocrs:** the lowest accuracy, no confidence, no accented letters, share-alike models at mutable URLs.

## 7. For the maintainer's decision

1. **The engine:** PP-OCRv5 mobile (Latin) through a runner VSift builds (recommended), or Tesseract 5.5.3 enlarged
   2x, built by VSift.
2. **The supply chain:** either way VSift builds and publishes an engine binary, which it has not done before (FFmpeg
   comes from BtbN's builds, whisper.cpp from ggml-org's releases). The questions:
   - where it lives: a release of this repository, or of a separate repository for the runner;
   - how it is attested;
   - whether the managed install covers Windows and macOS for OCR, or Ubuntu x86-64 only, as the managed catalogue
     does today, with `setup configure` elsewhere.
3. **Opt-in or by default:** recommended opt-in for 0.3.0, as an explicit command over a session's candidates,
   bounded per call and resumable. By default would add minutes of CPU to an investigation.
4. **Which frames:** candidates only, or candidates plus a fixed period for screens whose values change in place
   (section 4.3).
5. **Languages:** the Latin model covers English, Portuguese, Spanish and most European languages. Should other
   scripts (Cyrillic for RM09, CJK) be added later as separate models?
6. **ADR 0030** records the decision when step 2 starts.

## 8. Limits of this evaluation

- **Synthetic screens are cleaner than real ones.** The synthetic screens hold black or near-white text on flat
  backgrounds. At the rates tried, compression barely touched them: the encoder refines a still screen, and the jitter
  moved whole pixels. The stressed, smeared digits of [`small-text-real-media.md`](small-text-real-media.md) section 3
  were not reproduced, so the misread figures are a floor, not a rate for real recordings.
- **Real recordings have no text truth.** Label recall is relative to what some engine found and depends on the
  labels listed. Corroboration measures agreement between engine families, not correctness. A person's labels of RM10's
  and RM01's on-screen text would turn these into rates.
- **One runner type.** Speeds are from a 4-CPU hosted runner; Windows and macOS were built, not timed.
- **PP-OCR ran through RapidOCR's Python pipeline,** to measure the models. A Rust runner must reproduce its pre- and
  post-processing, and its accuracy must be measured again on the same frames before release.
- **Three screen recordings,** in English and Portuguese. Dark-mode interfaces are synthetic only.
