# Speaker labels: the engine evaluation (0.3.0)

Status (2026-10-21, `0.3.0`): **step 1, the evaluation, is done, and the maintainer decided on 2026-10-21 (section 9).
Step 2 is built on the same branch (section 10, [ADR 0031](../decisions/0031-speaker-labels.md)), except the managed
install, which waits for the managed-install system's extension to three systems (another story).** Issue:
[#108](https://github.com/smormah/vsift/issues/108) (speakers for live meetings; this is the recorded-file version).
Register: [L-205](known-limits.md#l-205), [L-206](known-limits.md#l-206), [L-207](known-limits.md#l-207),
[L-217](known-limits.md#l-217) (the engine archives' provenance, recorded by the managed-install story),
[L-218](known-limits.md#l-218) (older releases and the stored record); the scope
exclusion [L-046](known-limits.md#l-046) ends for speakers when this ships.

## 1. The question

"Who said what": an opt-in step that labels a transcript revision's segments S1, S2, ... by voice, so an agent can
cite "S2 said the export stops at ninety percent", and a person can record that S2 is Alice. The engine is an adapter,
like whisper.cpp: a pinned external tool, optional, with no Python and nothing linked into VSift's crates. Voice data is
sensitive: **VSift keeps no voiceprint database; voice embeddings live in the engine's memory only; only anonymous labels
and the names a user verified are stored.**

Step 1 (this document) compares the engines VSift could run, on hosted runners only: licences, official binaries and
checksums, size, accuracy, speed, memory and security. Step 2 (section 10) builds the chosen one after the maintainer
decides.

## 2. The candidates

| Candidate | What it is | Kept? |
| --- | --- | --- |
| **sherpa-onnx `v1.13.8`** (k2-fsa, Xiaomi's next-gen Kaldi team) | C++ command-line tool `sherpa-onnx-offline-speaker-diarization` over ONNX Runtime: pyannote segmentation 3.0 (overlap-aware, 10 s windows), one speaker-embedding model of your choice, agglomerative clustering with a known count or a distance threshold | **Yes**, with four embedding models (below) |
| **polyvoice `v1.0.0`** (one maintainer) | A Rust command-line tool: pyannote-style powerset segmentation, WeSpeaker ResNet34 embeddings, VBx clustering with an automatic count, its own INT8 kernels (no ONNX Runtime) | **Yes** |
| **whisper.cpp tinydiarize** (`-tdrz`, `small.en-tdrz` model) | No new engine: whisper.cpp marks where the speaker changes; it does not say who | Yes, as a baseline |
| pyannote.audio, NVIDIA NeMo (Sortformer), diart, Senko | Python pipelines | No: Python runtime |
| Rev `reverb-diarization-v1`/`v2` (as sherpa-onnx segmentation models) | Rev's fine-tune of pyannote | No: the Rev Model Non-Production License forbids use "in the course of a commercial activity" |
| Picovoice Falcon | Proprietary SDK | No: closed, needs an account key and online activation |
| pyannote-rs, speakrs, sherpa-rs | Rust libraries over ONNX Runtime | No: an engine linked into VSift's crates, when an external tool will do |
| FluidAudio, the pyannote Core ML ports | Swift and Core ML | No: Apple only |

The sherpa-onnx embedding models tried: **NeMo TitaNet small** (`nemo_en_titanet_small.onnx`), **WeSpeaker ResNet34
VoxCeleb** (`wespeaker_en_voxceleb_resnet34.onnx`), **3D-Speaker CAM++ VoxCeleb** (the one sherpa-onnx's English
example names) and **3D-Speaker ERes2Net VoxCeleb**.

## 3. Licences

| Part | Licence | Note |
| --- | --- | --- |
| sherpa-onnx engine | Apache-2.0 | Bundles ONNX Runtime (MIT) as `libonnxruntime.so` / `.dll` / `.dylib` |
| pyannote segmentation 3.0 | MIT (CNRS) | The weights are gated on Hugging Face under pyannote's account; sherpa-onnx's ONNX export (`csukuangfj/sherpa-onnx-pyannote-segmentation-3-0`) carries the MIT `LICENSE` and is ungated |
| NeMo TitaNet small | Apache-2.0 (NGC: "covered by the license of the NeMo Toolkit") | NVIDIA's Hugging Face card for TitaNet **large** says CC-BY-4.0; treat as attribution-required. Trained on VoxCeleb, Fisher, Switchboard and LibriSpeech |
| WeSpeaker ResNet34 VoxCeleb | Hugging Face card: Apache-2.0; WeSpeaker's own `docs/pretrained.md`: VoxCeleb models follow **CC-BY-4.0** | Two statements; take the stricter (attribution) |
| 3D-Speaker CAM++ and ERes2Net VoxCeleb | Apache-2.0 (ModelScope) | |
| polyvoice | MIT (code) | Its models: the powerset INT8 file is a quantisation of the pyannote segmentation export (MIT); the ResNet34 INT8 file is a quantisation of WeSpeaker ResNet34 (its manifest says Apache-2.0, its README says "neural weights are MIT", WeSpeaker says CC-BY-4.0); the VBx PLDA parameters are **CC-BY-4.0** (derived from pyannote `speaker-diarization-community-1`) |
| tinydiarize `small.en-tdrz` | MIT | English only |

Every kept licence permits redistribution and commercial use. CC-BY-4.0 parts need attribution in the licence notice
VSift ships with a managed install, as whisper.cpp's model does today.

## 4. Official binaries, checksums and sizes

| | sherpa-onnx `v1.13.8` | polyvoice `v1.0.0` |
| --- | --- | --- |
| Linux x64 | `sherpa-onnx-v1.13.8-linux-x64-shared.tar.bz2`, 28,156,791 bytes; 86 MB unpacked (44 files); the diarization tool is 497,136 bytes and needs only `libonnxruntime.so` beside it (`RPATH $ORIGIN/../lib`) and the system's C and C++ libraries; run on Ubuntu 24.04 (glibc 2.39); the oldest glibc it accepts was not measured | `polyvoice-linux-x86_64`, 5,551,256 bytes, one file (libc, libm, libgcc only) |
| Linux arm64 | `...-linux-aarch64-shared-cpu.tar.bz2`, 28,091,778 bytes (not run) | `polyvoice-linux-aarch64`, 4,199,472 bytes (not run) |
| Windows x64 | `...-win-x64-shared-MT-Release.tar.bz2`, 24,805,859 bytes (run, section 6) | `polyvoice-windows-x86_64.exe`, 5,895,168 bytes (run) |
| macOS arm64 | `...-osx-arm64-shared.tar.bz2`, 20,314,448 bytes (run); `osx-x64` and `universal2` also exist | `polyvoice-macos-arm64`, 3,816,912 bytes (run); no Intel build |
| Checksums | **No publisher checksum file and no attestation** for the engine archives: GitHub's release-asset SHA-256 only. The embedding models have the release's own `checksum.txt` (SHA-256); the segmentation model has Hugging Face's LFS SHA-256 at a pinned revision | GitHub's release-asset SHA-256 for the binaries; no attestation. Its own manifest pins every model by SHA-256 **and a minisign signature** |
| Models | segmentation 5,992,913 bytes; TitaNet small 40,257,283; WeSpeaker ResNet34 26,534,365 (CAM++ 29,596,978; ERes2Net 26,485,263) | 8,414,314 bytes for the INT8 pair, plus about 267 KB of PLDA parameters |
| A managed install would hold | about 33 MB of tool and runtime plus 46 to 67 MB of models | about 14 MB |

Every pin is in `tools/p16_speakers_eval.py` (`SHERPA_ARCHIVES`, `SEGMENTATION`, `EMBEDDINGS`, `POLYVOICE`,
`TINYDIARIZE`), each by size and the digest its publisher gives. tinydiarize's model is 487,614,184 bytes.

## 5. How it was measured

All on hosted runners, by the workflow `P16 speakers evaluation` (`.github/workflows/p16-speakers-eval.yml`); nothing
was downloaded or run on a development machine.

- **Synthetic conversations with exact truth.** Five fixtures, spoken turn by turn by the P07 Kokoro recipe (same
  pinned revision and environment, four more of its voices) and laid on a timeline by sample positions:
  `SPK2-MF` (a support call, an American female and male voice, 19 turns, 75 s, two overlaps, two same-speaker
  pauses, short replies such as "Okay."), `SPK2-FF` (the same call with two similar American female voices),
  `SPK2-MF-N10` (`SPK2-MF` with the `office-v1` noise at 10 dB), `SPK3-MFM` (a planning meeting with three voices, 18
  turns, 73 s, one overlap), `SPK1-M` (one voice with pauses of up to 1.6 s, 45 s: a second speaker found here is
  invented). A turn's truth runs from its first to its last sample above -40 dBFS. The scripts are in the tool.
- **RM06** (NASA, a two-person space-to-ground call, 10:24, public domain) from the test-media release, held to its
  pin. Its reference is labelled by a person reading the transcript (who asks, who answers), at segment precision
  ([L-206](known-limits.md#l-206)).
- **RM11** (NASA, an in-flight interview from the space station, 18:36, public domain; added to the test-media release
  on 2026-10-21 for this evaluation): Mission Control Houston opens and closes the event, a television station's
  interviewer on the ground asks and the astronaut answers; the first 9:26 are music. Its reference is read the same
  way as RM06's (`tools/p16_speakers_rm11_reference.json`), and a user would give three speakers.
- **RM09** (NASA, 28:02, outdoor remarks, several speakers, English and Russian) for cost only: there is no reference.
- **Scores.** The diarization error rate (DER: missed speech, false alarm and confusion over true speech; 0.25 s collar,
  0.5 s on RM06 and RM11; overlap scored), the number of speakers found, and **the labelling VSift would show** (section 10):
  each whisper.cpp `base` segment gets the speaker who holds at least 70% of the speech in it, when speech covers at
  least half of it, and otherwise "no confident speaker". "Labelled" is the share of segments given a speaker; "wrong"
  is the share of all segments given the wrong one.
- **Cost.** Wall time as a real-time factor (RTF, 2 threads per engine on a 4-vCPU runner), the process's own CPU
  seconds and peak resident memory (`wait4`).
- **Platforms.** The same fixtures on hosted Windows Server 2025 and macOS 15 (Apple silicon) with the official builds.

## 6. Results

Runs: the full matrix **39518020351** (Ubuntu, AMD EPYC 9V74) and the shortlist **39527538316** (Ubuntu, Windows,
macOS; it printed RM06's labelled transcript and every candidate's RM06 turns, from which the reference of section 5 was
read and RM06 scored offline with the tool's own scorer), and **39567789710** (the shortlist again, scoring RM06 from the
committed reference with the same numbers, and the time and memory against length of section 6.4), and
**39595991957** (2026-10-21: RM11 only, with the recommended engine, two alternatives and polyvoice; it printed RM11's
labelled transcript and turns, from which its reference was read and RM11 scored offline with the tool's own scorer;
**39597641002** scored it from the committed reference with the same numbers). In the tables, "known" means the speaker count was given; "auto-t" means sherpa-onnx's distance threshold t.

### 6.1 Synthetic conversations (Ubuntu, run 39518020351)

| Candidate | SPK2-MF | SPK2-FF | SPK2-MF-N10 | SPK3-MFM | SPK1-M | RTF | Peak MiB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **polyvoice, known** | 0.0% | 44.5% (1 found) | 0.0% | 0.2% | 0.0% | 0.054 | 405 to 478 |
| **polyvoice, automatic** | 0.0% (2) | 44.5% (1) | 0.0% (2) | 0.2% (3) | 0.0% (1) | 0.054 | 406 to 493 |
| **sherpa-onnx + TitaNet small, known** | 0.0% | 44.5% | 0.0% | 9.1% | 0.0% | 0.095 | 216 to 310 |
| sherpa-onnx + TitaNet small, auto-0.5 | 0.0% (2) | 44.5% (1) | 0.0% (3) | 13.6% (5) | 0.0% (1) | 0.095 | 215 to 310 |
| **sherpa-onnx + TitaNet small, auto-0.7 and auto-0.9** (run 39527538316) | 0.0% (2) | 44.5% (1) | 0.0% (2) | 9.1% (3) | 0.0% (1) | 0.094 | 223 to 310 |
| **sherpa-onnx + WeSpeaker ResNet34, known** | 0.0% | 44.5% | 0.0% | 30.1% (2 of 3) | 0.0% | 0.216 | 166 to 178 |
| sherpa-onnx + WeSpeaker ResNet34, auto-0.5 | 0.0% (2) | 44.5% (1) | 0.0% (2) | 4.7% (3) | 0.0% (1) | 0.216 | 163 to 185 |
| sherpa-onnx + WeSpeaker ResNet34, auto-0.7 / 0.9 (run 39527538316) | 42.3% (1) / 42.3% (1) | 44.5% (1) | 42.7% (1) | 30.1% (2) / 60.2% (1) | 0.0% (1) | 0.216 | 165 to 185 |
| sherpa-onnx + ERes2Net, known | 0.0% | 44.5% | 0.0% | 9.1% | 0.0% | 0.206 | 226 to 324 |
| sherpa-onnx + CAM++, known | 42.0% | 34.9% | 29.3% | 50.3% (2 of 3) | 0.0% | 0.106 | 139 to 156 |
| sherpa-onnx + CAM++, auto 0.5 / 0.7 / 0.9 | 48.6 / 50.1 / 44.3% | 32.9 / 34.9 / 44.5% | 58.3 / 35.9 / 35.9% | 47.1 / 53.3 / 51.5% | 6.3 / 6.3 / 0.0% | 0.107 | 140 to 154 |
| sherpa-onnx (int8 segmentation) + CAM++, known | 27.1% | 42.3% | 35.0% | 66.1% | 0.0% | 0.105 | 132 to 154 |
| tinydiarize (turns, two labels) | 45.6% | 58.6% | 55.2% | n/a | n/a | 0.39 to 0.71 | 829 to 841 |

What the transcript would show (the labelling rule, Ubuntu): with polyvoice and with sherpa-onnx + TitaNet small,
**no segment of `SPK2-MF`, `SPK2-MF-N10`, `SPK3-MFM` or `SPK1-M` is given the wrong speaker**, and 95 to 100% of
segments get one (TitaNet's 9.1% on `SPK3-MFM` is time at turn edges, not a whole segment). WeSpeaker given three
speakers on `SPK3-MFM` hears two of them as one (25% of segments wrong); its automatic setting at 0.5 does not. On `SPK2-FF` **every candidate hears the two similar voices as one**, and the rule labels
every segment with confidence, 28.6% of them wrongly ([L-207](known-limits.md#l-207)).

### 6.2 RM06 and RM09 (cost: Ubuntu, run 39518020351)

| Candidate | RM06 (10:24): speakers found automatically; RTF; peak MiB | RM09 (28:02): speakers found; RTF; peak MiB |
| --- | --- | --- |
| polyvoice | 4; 0.053; 1,131 (known: 2; 1,131) | 19; 0.046; **2,273** |
| sherpa-onnx + TitaNet small | auto-0.5: 14; 0.105; 419 (known: 358) | auto-0.5: 40; 0.090; 969 |
| sherpa-onnx + WeSpeaker ResNet34 | auto-0.5: 6; 0.243; 220 (known: 203) | auto-0.5: 32; 0.201; 590 |
| sherpa-onnx + CAM++ | auto-0.5 / 0.9: 14 / 4; 0.112; 224 to 267 | auto-0.5 / 0.9: 45 / 17; 0.096; 394 to 660 |
| tinydiarize | 0.83; 928 | 0.71; 1,025 |

**RM06's accuracy against its read reference** (106 scored segments; the two unscored ranges and a 0.5 s collar left out;
scored offline from run 39527538316's logged turns with the tool's own scorer, and again by run 39567789710 from the committed reference, with the same numbers; the 5.7% missed speech in every row is
the pauses inside the reference's segment-long turns, so confusion and the segment columns are the measure):

| Candidate | Speakers found | Confusion | Segments labelled | Of those, right | Wrong, of all segments |
| --- | --- | --- | --- | --- | --- |
| **polyvoice, automatic** | 4 | **0.3%** | 98.1% | **99.0%** | **0.9%** |
| polyvoice, told 2 | 2 | 26.9% | 98.1% | 71.2% | 28.3% |
| **sherpa-onnx + TitaNet small, told 2** | 2 | **1.2%** | 98.1% | **98.1%** | **1.9%** |
| **sherpa-onnx + TitaNet small, auto-0.9** | 5 | **1.2%** | 98.1% | **98.1%** | **1.9%** |
| sherpa-onnx + TitaNet small, auto-0.7 / 0.5 | 6 / 14 | 3.6 / 16.1% | 97.2 / 94.3% | 94.2 / 86.0% | 5.7 / 13.2% |
| sherpa-onnx + WeSpeaker ResNet34, told 2 / auto-0.5 | 2 / 6 | 28.2 / 15.8% | 98.1 / 94.3% | 69.2 / 82.0% | 30.2 / 17.0% |
| sherpa-onnx + CAM++, told 2 | 2 | 40.8% | 87.7% | 59.1% | 35.8% |

On this call the caller and the astronaut are two men on a space-to-ground link; told two speakers, polyvoice and
WeSpeaker merge them for long stretches, while TitaNet does not. The automatic settings that found 4 or 5 speakers added
short extra labels (the closing controller is a real third voice).

**RM11's accuracy against its read reference** (run 39595991957; 145 scored segments; four unscored stretches (two short
interjections, a sentence the interviewer cut off, laughter) and a 0.5 s collar left out; scored offline from the logged
turns with the tool's own scorer, and again by run 39597641002 from the committed reference, with the same numbers):

| Candidate | Speakers found / given | DER | Confusion | Segments labelled | Of those, right | Wrong, of all segments | RTF; peak MiB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| polyvoice, automatic | 3 | 2.2% | 0.0% | 100% | 100% | 0.0% | 0.032; 1,688 |
| **sherpa-onnx + TitaNet small, auto-0.9** (the default) | 9 | **3.4%** | **1.3%** | 100% | **97.9%** | **2.1%** | 0.055; 427 |
| sherpa-onnx + TitaNet small, told 3 | 2 / 3 | 7.7% | 5.6% | 99.3% | 93.1% | 6.9% | 0.055; 394 |
| sherpa-onnx + WeSpeaker ResNet34, told 3 | 3 / 3 | 35.0% | 32.9% | 100% | 61.4% | 38.6% | 0.103; 242 |

With the automatic count, each person kept one main label (the controller, the interviewer, the astronaut); the six
extra labels hold 9.0 s of speech in all, and three segments went to them (the astronaut's first reply, one interjection
of the interviewer's, one of the controller's closing lines). **Told three, TitaNet found two**: it put the controller's
words under the interviewer's label, two voices on the ground side of one link, so a given count is not a guarantee
that the engine keeps that many voices apart ([L-207](known-limits.md#l-207)). The astronaut is the only voice on the
station side; NASA's description names a second crew member, who is not heard.

### 6.3 The same builds on three systems

In run 39527538316, **every shortlisted candidate gave the same numbers on Ubuntu, Windows Server 2025 and macOS 15
(Apple silicon) for every fixture**, with the official builds of each system. On macOS, sherpa-onnx + TitaNet small ran
at an RTF of 0.09 to 0.14 and peaked at 235 to 350 MiB, polyvoice at 0.03 to 0.04 and 415 to 588 MiB; the Windows job
measures time only (TitaNet 0.10 to 0.13, polyvoice 0.06).

**CAM++ is fragile.** In the first run (39517135665, whose Ubuntu job stopped early) CAM++ scored 12.1% on `SPK2-MF` on
Windows and macOS; in the next two runs it scored 42.0% on all three systems, while every other candidate's numbers were
the same in every run. The conversations are synthesized afresh in each run, and Kokoro's output can differ slightly
from one runner CPU to another ([L-200](known-limits.md#l-200)'s kind); a small change in the audio moved CAM++'s
clustering a long way. The workflow now prints each conversation's digest and the CPU, so runs are compared only on equal
audio.

### 6.4 Time and memory against length: the duration cap (run 39567789710)

sherpa-onnx + `TitaNet` small on RM09 and RM06 repeated end to end (real speech, so the clustering sees as many
embeddings as a real recording of that length would give it), two threads per model, on a hosted Ubuntu runner
(Intel Xeon Platinum 8573C, 16 GB):

| Length | Automatic count (0.9): wall time, RTF, peak | Told 2: wall time, RTF, peak |
| --- | --- | --- |
| 30 minutes | 2.3 min, 0.076, 620 MiB | 2.2 min, 0.074, 448 MiB |
| 1 hour | 4.7 min, 0.078, 1,021 MiB | 4.6 min, 0.076, 621 MiB |
| **2 hours** | **9.2 min, 0.076, 2,002 MiB** | **9.2 min, 0.077, 1,051 MiB** |
| 3 hours | 14.4 min, 0.080, 3,142 MiB | 13.9 min, 0.077, 1,584 MiB |

Time grows linearly (a twelfth of real time); memory grows by about 1 GiB an hour with the automatic count and half
that with a given count. **The cap is two hours** (the maintainer's aim, decision 5): 2.0 GiB at most leaves room on
an 8 GB machine. A longer recording is labelled a part at a time with `--from` and `--to`; labels of different parts
are not linked ([L-205](known-limits.md#l-205)).

## 7. Security

- **No network while diarizing.** Under `strace` (socket and connect calls), neither the sherpa-onnx tool nor
  polyvoice (with its models already in place) made a single socket call on `SPK2-MF` (run 39518020351).
  **polyvoice's binary can download**: `download-models`, and a `diarize` that finds a model missing, fetch from GitHub
  and Hugging Face and verify by SHA-256 and minisign. A VSift adapter would always pass `--models-cache` pointing at
  pinned files a managed install placed, and would treat a missing model as absent, never let the tool fetch.
- **Inputs.** VSift would hand either engine only a 16 kHz mono PCM WAV that its own FFmpeg wrote, never the source
  file, so the engine's own audio reader never sees untrusted containers. Models are fixed files pinned by SHA-256.
  ONNX Runtime (sherpa-onnx) parses the model protobufs; polyvoice parses its own model format. Both run under VSift's
  process supervisor (time limit, stream limits, kill on cancel), like whisper.cpp.
- **Supply chain.** sherpa-onnx: a large, active project (k2-fsa; about 15,400 stars; releases every one to two weeks;
  no published security advisories), but no checksum file or attestation for its engine archives: a pin rests on
  GitHub's digest and VSift's own review of the archive. polyvoice: one maintainer, a repository about five months old,
  `1.0.0` released on 2026-10-01, about 20 stars; better per-file integrity (SHA-256 plus minisign) but a bus factor
  of one and almost no outside use yet.
- **Memory.** polyvoice's peak grows with length: 1.1 GiB on 10 minutes and 2.3 GiB on 28 (sherpa-onnx + TitaNet:
  0.42 and 0.97 GiB). A one-hour recording would be several GiB unless VSift diarizes in bounded windows and links
  the labels across them, which costs accuracy at the window joins ([L-205](known-limits.md#l-205)).
- **Privacy.** Both engines keep embeddings in memory and write only start, end and label; neither keeps a voiceprint
  file. VSift would store only the labels, their coverage and the names a user verified.

## 8. Recommendation

**sherpa-onnx `v1.13.8` with pyannote segmentation 3.0 and NeMo TitaNet small**, run with the speaker count when the
user gives one (`--clustering.num-clusters`) and otherwise with the distance threshold 0.9.

- **Accuracy.** It is the only sherpa-onnx configuration that is right on both kinds of evidence: on RM06, 1.9% of
  segments wrong, told two speakers or not (and on RM11, read after the decision, 2.1% with the automatic count); on the synthetic set no wrong segment except the similar-voice case that
  defeats every candidate. polyvoice is a little better (RM06 0.9% wrong automatically; `SPK3-MFM` 0.2% against 9.1%),
  but only automatically: **told the speaker count, polyvoice merged the two men of RM06 (28% wrong)**, because its
  `--speakers` caps the clustering instead of setting it, and "I know there are two of us" is the user's most natural
  input.
- **Memory.** 377 MiB on RM06 and 588 MiB on RM09's 28 minutes, against polyvoice's 1.2 and 2.2 GiB; both grow with
  length ([L-205](known-limits.md#l-205)), sherpa-onnx far more slowly.
- **Supply chain.** A large, active Apache-2.0 project with official builds for every system VSift supports, against a
  five-month-old single-maintainer project; the engine and its models are separate files, so a model can be re-pinned
  without the engine, like whisper.cpp's.
- **The costs.** Twice polyvoice's time (RTF 0.10 against 0.05 on the 4-vCPU runner: about a minute for a ten-minute
  recording, beside several minutes of transcription), 33 MB of tool plus 46 MB of models, ONNX Runtime as a native library beside
  the tool, and no publisher checksum for the engine archives (VSift pins GitHub's digest after its own review, as it
  does for whisper.cpp's build archives).

polyvoice is the alternative to keep watching: re-evaluate it at `1.0.0` of VSift if it gains maintainers, a count
that is set rather than capped, and bounded memory. tinydiarize, CAM++, ERes2Net and WeSpeaker are not recommended
(section 6).

## 9. Decisions for the maintainer

**Decided by the maintainer on 2026-10-21** (relayed by the supervisor): (1) sherpa-onnx `v1.13.8` with pyannote
segmentation 3.0 and NeMo `TitaNet` small, (2) shipped with an attribution notice that covers both of `TitaNet`'s
licence statements, (3) managed installs on Windows, macOS and Linux, built on another story's extension of the
managed-install system, with the engine archives pinned by GitHub's digest and the provenance gap in the register (L-217, recorded by the
managed-install story),
(4) the automatic count at 0.9 when the user gives none and a given count passed through, (5) a duration cap with a
typed refusal and a remediation, about two hours if an 8 GB machine allows (section 6.4: it does), (6) labels
provisional ("S2, by voice"), the user's verified names the only identity, and the accuracy statement naming the
similar-voice case, (7) one more public-domain recording of two or three people for the test-media release, and the
maintainer listens to confirm RM06's reference and the new one (RM11, added and scored on 2026-10-21, section 6.2), with
[the listening sheet](speaker-listening-sheet.md).
The options as they were put:

1. **The engine.** sherpa-onnx + TitaNet small (recommended), polyvoice, or neither for `0.3.0` (speaker labels move
   to `1.0`).
2. **The embedding model's licence reading.** TitaNet small's NGC page puts it under the NeMo Toolkit's Apache-2.0,
   while NVIDIA's card for TitaNet large says CC-BY-4.0: ship it with an attribution notice either way (recommended),
   or ask NVIDIA.
3. **Managed on which systems.** Ubuntu x64 only, like whisper.cpp today, with Windows and macOS using a build the user
   points to (recommended for `0.3.0`); or managed on all three, since sherpa-onnx publishes official builds for each
   (more pins, more release checks).
4. **The automatic count's threshold.** 0.9 (recommended: it found the right count on every synthetic fixture but the
   similar-voice one, and RM06's best confusion) or require the user to give a count.
5. **Long recordings ([L-205](known-limits.md#l-205)).** A duration cap for one diarization (for example 2 hours, with
   a typed refusal beyond it) for `0.3.0`, or windowed diarization with labels linked across windows (more work, less
   accurate at the joins).
6. **What the labelling promises ([L-207](known-limits.md#l-207)).** The proposed rule labels a segment only when one
   speaker clearly holds it, but it cannot see two similar voices merged into one: every candidate labelled `SPK2-FF`
   confidently and 28.6% wrongly. Proposed: labels are always presented as provisional ("S2, by voice"), with the
   verified names as the only identity, and the accuracy statement names the similar-voice case.
7. **More evidence before the cut ([L-206](known-limits.md#l-206)).** RM06 is the only real recording with a reference,
   read from the transcript. Proposed: a person listens to RM06 to confirm the reference, and one more real two- or
   three-person item with different voices (a woman and a man, or accents) joins the test-media release.

## 10. Step 2, as built ([ADR 0031](../decisions/0031-speaker-labels.md))

What is built on the branch (the commands, JSON and failures are in `docs/contracts/cli-v1.md`, "Speaker labels"):
the domain's speaker record (labels derived from the turns on every read), the diarization port and use cases, the
strict stored record and its session sub-budget (128 records), the sherpa-onnx adapter, `transcript speakers`,
`transcript name-speaker`, `search --speaker`, the labels in `transcript get` and `search` (full and brief views),
the schemas and examples, the count shortfall (`count_shortfall` in the `speakers` block when a given count is not
kept, added on 2026-10-21 after RM11), the skill's command table, and an opt-in end-to-end test with the real engine (`P16
speakers end to end`). **Not built yet:** the managed install (it waits for the managed-install system's extension;
until then the tool is given with `--diarizer` or found on the filtered `PATH`, and the models with
`--segmentation-model` and `--embedding-model`) and a `setup check` row for the tool. The design as proposed:

- **Managed install and adapter.** A new runtime dependency and managed component for the chosen engine and its
  models, pinned by size and SHA-256 in the managed catalogue (Ubuntu x64 managed, like whisper.cpp; on Windows and
  macOS the user's own build through a path, unless the maintainer extends the managed install), and an adapter behind
  the process supervisor with a closed argument list. When the engine is absent, a speaker request refuses with the
  existing `missing_capability` failure and a remediation that names `setup install`; every other command works as now.
- **A side record, not a new transcript revision.** Revisions are immutable and the stored transcript record rejects
  unknown fields, so speaker labels are their own numbered, immutable record per transcript revision (the visual
  index is the precedent): engine and model identity, the speaker count asked for, the labels per segment id, and the
  coverage. A new diarization makes a new speaker record; a retranscription invalidates nothing silently, because the
  record names the revision it labels.
- **Assignment by segment overlap.** VSift has segment times, not word times, so a segment gets the speaker who holds
  at least 70% of the speech the engine found inside it, when that speech covers at least half the segment; otherwise
  it is "no confident speaker". The output says so in plain words: the precision is the segment, and a segment where
  two people speak is not split.
- **Names.** A command records a verified name for a label of one speaker record (for example S1 = "Alice"); the name
  is the user's statement, kept beside the label and shown as such, never inferred from a voice or the screen.
- **Speakers everywhere.** `transcript get` and search hits gain an additive speaker block (label, the verified name if
  any, the record it came from), distinct from the existing imported WebVTT `speaker` field; `search --speaker S1`
  filters, its cursor digest includes the filter, and its coverage says that only labelled segments were searched.
- **Coverage.** A speaker-coverage block in the pattern of `transcript_coverage`: labelled ranges, ranges with no
  confident speaker, and ranges never diarized, with a fixed warning when any part is unlabelled.
- **Seams.** The engine runs over the whole source audio, not per recognition chunk, so labels need no joining at
  the 30 s seams; they attach to the merged segments by time. A seam's kept duplicate copies and trimmed segments keep
  their own times, so each gets its own label by the same rule.

## 11. Reproduce

`gh workflow run p16-speakers-eval.yml --ref <branch>` once the workflow is on `main` (on a pull request it runs when
its own files change). `--reference RM06=tools/p16_speakers_rm06_reference.json` (and `RM11=...`) scores a read item;
`--candidate <name>` runs part of the matrix, as the RM11 job does. `python tools/p16_speakers_eval.py evaluate ... --everything` runs the full matrix of run
39518020351; without it, the shortlist. The tool's tests (`tools/test_p16_speakers_eval.py`) need no engine, model or
network.
