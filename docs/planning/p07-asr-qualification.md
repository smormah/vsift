# P07 local-ASR qualification record (T-04, decision D6)

Status: measured 2026-09-25 on one Windows machine (P07 increment 3c, branch
`p07/asr-setup-profiles`). **Decided (maintainer, 2026-09-25): `base` is the measured
default and passes the gates below.** Noisy speech is gated on critical terms only;
F08's word error rate is a reported known limitation until the noisy-speech fixture
set of [issue #150](https://github.com/smormah/vsift/issues/150) exists. The
`base_q5_1` pin at Hugging Face revision `5359861` was accepted the same day.

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
  are reported but do not fail the gate. Critical terms are gated in every clip,
  the noisy F08 included.
- **Clean pool:** every clip without added noise (the `clean-synthetic` clips and
  F09, whose only difference is its offset audio stream), pooled: 123 reference
  words. **F08** (office noise at 10 dB SNR, one English and one Spanish sentence,
  13 reference words) is reported alone; its word error rate is not gated.
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

## Gates (decided 2026-09-25)

| Gate for `base`, 4 threads | Enforced? |
| --- | --- |
| Pooled WER of clips without added noise <= 10% | Yes: the test fails |
| Every spoken critical term found in every clip, noisy included, except the reviewed known misses | Yes: the test fails |
| F08 (noisy) WER | No: reported as a known limitation, `"gated": false` with the reason in the report; a noise WER gate waits for #150 |
| Real-time factor <= 0.5 | No: reported, it measures the host |
| Peak `whisper-cli` memory <= 400 MiB | No: reported, it measures the host |

`base_q5_1` is measured and reported only.

## Results

Two full runs on the same host and day. Accuracy was identical in both. Run A had
the machine to itself. Run B is the run of record for the decided gates, and the
machine was busier during it, so its timings are slower.

| Measure | Gate (base) | `base`, run A | `base`, run B | `base_q5_1`, run A | `base_q5_1`, run B |
| --- | --- | ---: | ---: | ---: | ---: |
| Model file | | 147,951,465 B | | 59,707,625 B | |
| Model load time (median of 3) | reported | 316 ms | 413 ms | 177 ms | 184 ms |
| Real-time factor, 188.7 s clip (median of 3) | <= 0.5, reported | 0.388 (73.2, 73.4, 74.6 s) | 0.489 (92.4, 89.6, 97.1 s) | 0.409 | 0.432 (81.6, 71.9, 130.1 s) |
| Peak `whisper-cli` memory | <= 400 MiB, reported | 338 MiB | 337 MiB | 250 MiB | 251 MiB |
| Pooled WER, clips without noise (123 words) | **<= 10%, enforced** | **3.25%** (4 errors) | **3.25%** | 4.06% (5 errors) | 4.06% |
| Critical terms missed outside the known list | **none, enforced** | **none** | **none** | none | none |
| F08 WER (13 words) | reported (#150) | 61.53% (8 errors) | 61.53% | 46.15% (6 errors) | 46.15% |

**Outcome: `base` meets every enforced gate and both reported resource gates.** In
run B the test passed (`test result: ok`, 809 s) and its report says, for F08,
`"gated": false` with the reason.

Per clip (WER, critical terms missed; the same in both runs):

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
(three). Its critical terms "AB-731", "2.5 seconds" and "identificador" were heard;
"E-409" is a reviewed known miss. F04's "queued", also on the known-miss list, was
heard correctly in these runs; the list keeps it because the utterance alone is
heard as "Q". An earlier run the same day gave the same accuracy and real-time
factors of 0.389 and 0.414; its memory sampler did not run (a multi-line PowerShell
script over standard input), which the recorded runs fix.

## Hosted runners (added 2026-09-26)

The opt-in `P07 local ASR` workflow stages the pinned toolchain on disposable GitHub
runners (4 vCPUs, 4 recognizer threads, release build). Accuracy was identical to the
reference machine on every runner.

| Run | Runner | `base` RTF | `base` peak | `base_q5_1` RTF | `base_q5_1` peak |
| --- | --- | ---: | ---: | ---: | ---: |
| [36175016465](https://github.com/smormah/vsift/actions/runs/36175016465) | windows-2025, AMD | 0.284 | 335 MiB | 0.338 | 249 MiB |
| [36175016465](https://github.com/smormah/vsift/actions/runs/36175016465) | ubuntu-24.04, generic CPU backend only | 3.245 | 322 MiB | 2.919 | 235 MiB |
| [36199691655](https://github.com/smormah/vsift/actions/runs/36199691655) | ubuntu-24.04, optimised backends (#153) | 0.244 | 319 MiB | 0.298 | 232 MiB |
| [36199691655](https://github.com/smormah/vsift/actions/runs/36199691655) | windows-2025, AMD | 0.264 | 336 MiB | 0.342 | 249 MiB |

The first Ubuntu figures are from before #153, when the reviewed Ubuntu file set
held only ggml's generic `libggml-cpu-x64.so`. With the optimised backends pinned,
`base` meets the 0.5 real-time-factor limit on both hosted platforms. Output differs
slightly between CPU backends. An Intel AVX-512 Windows runner (run
[36198903762](https://github.com/smormah/vsift/actions/runs/36198903762)) heard F05's
"invoice" as "in voice" with `base`, a reviewed known miss, so journey tests check only
words every reviewed host hears (ADR 0017).

## Seam finding and fix

Running the local-ASR checkpoint with `base_q5_1` lost F07's whole sentence from the
two-chunk seam clip, with no warning. Chunk 1 heard the sentence from its first
sample (25.0-34.0 s); chunk 0's previous sentence ended at 25.24 s, just inside it.
The seam merge (rule 2, increment 3a) treated any overlapping segment across the
edge as a copy, so it dropped chunk 1's sentence as "covered", while chunk 0's cut
copy was dropped because chunk 1 owned its midpoint. whisper-cli itself output the
sentence in both chunks. The fix, found here and merged with increment 3b (PR #149,
`57a72d4`; ADR 0017 section 3), makes a neighbour segment a copy only when it spans
the cut segment's midpoint, with the regression test
`a_sentence_starting_at_a_window_is_not_covered_by_the_previous_sentence`. With the
fix the `base_q5_1` seam stage passes and the `base` checkpoint and adapter test still
pass. The accuracy figures above come from single-chunk clips and the timing from a
multi-chunk clip, so neither depends on the fix.

The checkpoint's whole-file stage still fails with `base_q5_1`, because it hears F05's
"invoice" as "in voice" (a genuine recognition difference, visible in the table). The
checkpoint's word checks are written for the default profile, so the workflow runs it
with `base` only and measures `base_q5_1` here.

## Gaps

All three are tracked in [issue #150](https://github.com/smormah/vsift/issues/150):

- **Noisy speech:** F08 is the only noisy clip and has 13 words, so its rate moves 7.7
  points per word; no noise word-error gate is set until a noisy fixture set exists.
- **Accent and crosstalk are not in the corpus.** Every clip is one synthetic
  (Kokoro) voice per sentence; there is no human recording, regional accent or
  overlapping speech. T-04 stays open for those until fixtures exist.
- Measured on one Windows machine. The opt-in `P07 local ASR` workflow
  (`.github/workflows/p07-local-asr.yml`) repeats this on hosted Ubuntu 24.04 and
  Windows Server 2025 runners with the reviewed Ubuntu and Windows builds; its reports
  are the Linux evidence once it has run.
- Long recordings: the timing clip is 3 minutes; the per-chunk source rehash (#148)
  makes cost grow with source size and is not measured here. (2026-09-26: #148 is
  fixed in P08; a run now hashes the source twice in total, see ADR 0012's note.)

## Decision (maintainer, 2026-09-25)

1. **`base` stays the default.** It meets the clean-speech and critical-term gates
   with margin, and the resource gates.
2. **Noisy speech is gated on critical terms only**, except the reviewed known
   misses. F08's word error rate is reported as a known limitation and not gated; an
   agent must confirm critical identifiers heard in noise against the source (VSift
   keeps confidence `provider_uncalibrated` and cites times). Issue #150 builds a
   noisy-speech fixture set, with accent and crosstalk, before any noise gate is set.
3. **The `base_q5_1` pin at revision `5359861` is accepted.** It stays an optional
   reviewed alternative for machines where 338 MiB of peak memory is too much. It is
   smaller and loads faster, but was not faster end to end here and is slightly
   less accurate on clean speech.

After 3c merges, the `P07 local ASR` workflow runs on `main` as qualification
evidence for Ubuntu 24.04 and Windows Server 2025.

## Reproduce

```console
VSIFT_TEST_WHISPER_CLI=<absolute whisper-cli path>
VSIFT_TEST_WHISPER_MODEL=<absolute ggml-base.bin path>
VSIFT_TEST_WHISPER_MODEL_Q5_1=<absolute ggml-base-q5_1.bin path>
cargo test --release -p vsift-infrastructure --locked --test p07_asr_qualification -- --ignored --nocapture
```

The report is written to `.vsift/e2e-runs/p07-asr-qualification-<run>/report.json`.
The test fails only when an enforced base gate (clean WER, critical terms) is not
met. F08's word error rate and the resource gates are reported, not enforced.
