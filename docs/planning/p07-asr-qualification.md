# P07 local-ASR qualification record (T-04, decision D6)

Status: measured 2026-09-25 on one Windows machine (P07 increment 3c, branch
`p07/asr-setup-profiles`). **The base default does not meet every proposed D6 gate:
it fails the F08 word-error-rate gate (61.5% against at most 25%).** It meets the
other five. The default is a maintainer decision; a proposal is at the end.

## What was measured, and how

The opt-in test `crates/vsift-infrastructure/tests/p07_asr_qualification.rs`
(`local_asr_accuracy_timing_and_memory`, release build) runs each reviewed model
profile through VSift's real chain: the session's staged copy, FFprobe, 30 s chunks
decoded by FFmpeg, whisper.cpp v1.9.2 with decoding profile `r0-v1`, output
validation and the seam merge, exactly as `transcript retranscribe` does. Every
measurement uses **4 recognizer threads** (the D6 measurement point; the command
itself uses the machine's parallelism, at most 8).

- **Accuracy:** each committed speech clip (`fixtures/corpus/generated/*-speech.*`,
  F01-F09 and F12) is transcribed whole and scored by the test-only
  `asr_scoring` module against its frozen script in `fixtures/corpus/manifest.json`.
  Reference and hypothesis are normalised alike: lowercase; punctuation removed; a
  thousands comma removed; a hyphen between letters or digits joins an identifier
  (`E-409` is `e409`); a colon between digits separates two spoken numbers (`10:32`
  is `10 32`); decimals compare by value (`125.00` is `125`); number words are their
  value (`twelve` is `12`). WER is word edits over reference words. **Critical
  terms** are the manifest's `expected_terms` that are actually spoken in the script
  (F03's on-screen `G18` is excluded); a term is found when its words, joined
  without spaces, equal consecutive hypothesis words joined without spaces (`safe 12`
  finds `SAFE-12`; `407` never finds `4407`). Nothing is derived from a recognizer
  run. The reviewed known base-model misses (F04 "queued", F05 "4407", F08 "E-409")
  are reported but do not fail the gate.
- **Clean pool:** every clip without added noise (the `clean-synthetic` clips and
  F09, whose only difference is its offset audio stream), pooled: 123 reference
  words. **F08** (office noise at 10 dB SNR, one English and one Spanish sentence) is
  gated alone: 13 reference words.
- **Model load time:** whisper.cpp's own `load time` for the F01 utterance, run
  directly, median of three.
- **Real-time factor:** a 188.7 s clip concatenated at test time from the committed
  utterances (F01..F09, F12 cycled, 0.3 s gaps, over a black video; not committed),
  transcribed three times; the median elapsed time over the clip duration. It
  includes FFmpeg decoding of every chunk and the model identity check before and
  after the run.
- **Peak memory:** the largest `PeakWorkingSet64` of any `whisper-cli` process during
  those three runs, sampled every 100 ms by PowerShell `Get-Process` (on Linux the
  test reads `VmHWM` from `/proc/<pid>/status` instead).

Host: Windows 11 Pro 10.0.26200, Intel Xeon E5-2698 v4 @ 2.20 GHz (20 cores, 40
logical processors), 64 GiB RAM, NTFS; whisper.cpp v1.9.2 reviewed Windows build
(SHA-256 `95e3c0b0…2631d`); FFmpeg 9.0 (gyan.dev full build) on `PATH`.

## Results

| Measure | Gate (base) | `base` | `base_q5_1` |
| --- | --- | ---: | ---: |
| Model file | | 147,951,465 B | 59,707,625 B |
| Model load time (median of 3) | | 316 ms | 177 ms |
| Real-time factor, 188.7 s clip (median of 3) | <= 0.5 | **0.388** (73.2, 73.4, 74.6 s) | 0.409 (76.4, 77.2, 77.5 s) |
| Peak `whisper-cli` memory | <= 400 MiB | **338 MiB** | 250 MiB |
| Pooled WER, clips without noise (123 words) | <= 10% | **3.25%** (4 errors) | 4.06% (5 errors) |
| F08 WER (13 words) | <= 25% | **61.53% (8 errors) - not met** | 46.15% (6 errors) |
| Critical terms missed outside the known list | none | **none** | none |

Per clip (WER, critical terms missed):

| Clip | Words | `base` | `base_q5_1` |
| --- | ---: | --- | --- |
| F01 | 10 | 0% | 0% |
| F02 | 15 | 0% | 0% |
| F03 | 11 | 0% | 0% |
| F04 | 16 | 0% ("queued" heard) | 0% |
| F05 | 19 | 10.5%; "invoice 4407" (known: heard "Invoice407") | 15.8%; "invoice 4407" (heard "in Voice 40407") |
| F06 | 9 | 0% | 0% |
| F07 | 18 | 0% | 0% |
| F08 | 13 | 61.5%; "E-409" (known: heard "E4 and I") | 46.2%; "E-409" (heard "E4i9") |
| F09 | 5 | 0% | 0% |
| F12 | 20 | 10.0% ("SAFE-12" found as "safe 12", scored as two edits) | 10.0% |

F08's eight base errors: "AB-731" heard as "AB 731" (two edits; the term itself is
found), "E-409" as "E4 and I" (three), and the Spanish "AB-731" as "a vez 731"
(three). Even if a space inside an identifier were forgiven, F08 would be 6/13
(46%), still over the gate. The quantized profile is also over it on noise and is
slightly worse on clean speech. F04's "queued", on the known-miss list, was heard
correctly in both runs of this record; the list stays because the utterance alone
is heard as "Q".

All gates are met by `base` except F08. Load time is not gated. The whole test took
644 s. The first run of this record (same day, same host) gave the same accuracy
and 0.389/0.414 real-time factors; its memory sampler did not run (a multi-line
PowerShell script over standard input), which the recorded run fixes.

## Gaps

- **Accent and crosstalk are not in the corpus.** Every clip is one synthetic
  (Kokoro) voice per sentence; there is no human recording, regional accent or
  overlapping speech. T-04 stays open for those until fixtures exist.
- F08 is the only noisy clip and has 13 words, so its rate moves 7.7 points per word.
- Measured on one Windows machine. The opt-in `P07 local ASR` workflow
  (`.github/workflows/p07-local-asr.yml`) repeats this on hosted Ubuntu 24.04 and
  Windows Server 2025 runners with the reviewed Ubuntu and Windows builds; its reports
  are the Linux evidence once it has run.
- Long recordings: the timing clip is 3 minutes; the per-chunk source rehash (#148)
  makes cost grow with source size and is not measured here.

## Proposal (maintainer decides)

The base model meets the clean-speech, critical-term, real-time and memory gates
with margin, and misses only the noisy-bilingual F08 WER gate, which the quantized
profile also misses. Options:

1. **Keep `base` as the default and record F08 as a known limit** (proposed): state
   in the profile that noisy speech with spoken identifiers is not qualified at 25%
   and that critical identifiers in noise must be confirmed against the source
   (VSift already keeps confidence `provider_uncalibrated` and cites times). Either
   amend the F08 gate to what `base` does today, or leave it unmet and visible.
2. Qualify a larger pinned profile (for example multilingual `small`, about 488 MB)
   for noisy input before R0, and measure it with this test.
3. Keep `base` and add noisy fixtures before deciding, since one 13-word clip is a
   weak basis for a gate.

`base_q5_1` is not proposed as the default: it is smaller and loads faster, but
is slower end to end here and less accurate on clean speech. It stays a reviewed
alternative for machines where 338 MiB of peak memory is too much.

## Reproduce

```console
VSIFT_TEST_WHISPER_CLI=<absolute whisper-cli path>
VSIFT_TEST_WHISPER_MODEL=<absolute ggml-base.bin path>
VSIFT_TEST_WHISPER_MODEL_Q5_1=<absolute ggml-base-q5_1.bin path>
cargo test --release -p vsift-infrastructure --locked --test p07_asr_qualification -- --ignored --nocapture
```

The report is written to `.vsift/e2e-runs/p07-asr-qualification-<run>/report.json`.
The test fails when a base accuracy gate is not met, so today it fails on F08 by
design; the resource gates are reported, not enforced, because they measure the host.
