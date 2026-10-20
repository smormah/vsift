# ADR 0005: R0 scope and qualification profiles

- Status: Accepted
- Date: 2026-09-10
- Resolves: DEC-01, DEC-02, DEC-03, DEC-04, DEC-11, DEC-12

## Context

The first functional release must serve a disposable desktop investigation while
forming a reliable processing core for backend workers. Broad platform and enrichment
claims would make that release impossible to qualify rigorously.

## Decision

R0 includes bounded single-host job and batch execution. Distributed queues,
multi-tenant authorization, remote object storage and a network API remain host or R2
concerns. The first strict worker profile is Ubuntu 24.04 LTS x86-64 on ext4 under a
supervisor with cgroup v2/container limits. Desktop qualification targets are Windows
11 25H2 x64 on NTFS and macOS 15 arm64 on APFS. These are qualification targets until
the release matrix passes; they are not current support claims.

The CPU speech baseline is a pinned whisper.cpp provider with the multilingual `base`
model as the first candidate. It becomes the default only if P07 accuracy and resource
gates pass; larger models remain optional. Desktop sessions use a 24-hour idle expiry
and seven-day absolute lifetime by default. Explicitly retained work does not expire
automatically.

R0 does not require diarization, OCR, embeddings, a vision model or scroll stitching.
It preserves visual sequences and lets an agent request denser source evidence.

## Consequences

Other systems may work but are unqualified until added by evidence and an updated
matrix. Limits and platform versions can change through an ADR when qualification
findings justify it. Optional enrichment cannot become a hidden install or correctness
dependency for core investigation.

## 2026-09-25 note: measured default outcome (P07 increment 3c)

**The measured default is the multilingual `base` model.** Maintainer decision D6
proposed gates for it on 4 recognizer threads, and the maintainer settled the noise
gate on 2026-09-25 after measurement ([record](../planning/p07-asr-qualification.md),
Windows 11, Xeon E5-2698 v4):

- Enforced: at most 10% pooled word error rate on clips without added noise
  (measured 3.25%), and every spoken critical term found in every clip, noisy F08
  included, except the reviewed known misses (none unexpected).
- Reported, not enforced (they measure the host): real-time factor at most 0.5
  (0.388, and 0.489 on a busier run) and `whisper-cli` peak memory at most 400 MiB
  (338 MiB).
- **Deferred:** a word-error-rate gate for noisy speech. F08 (61.5%, one 13-word
  clip) is reported as a known limitation. Issue #150 builds a noisy-speech fixture
  set, with accent and crosstalk, before any such gate is set.

A second reviewed profile, `base_q5_1` (its q5_1 quantization, pinned at the
accepted revision `5359861`), is an optional alternative measured alongside; it is
not the default.

## 2026-10-20 note: the noisy-speech set exists, and noise is gated on it

The set issue #150 asked for is in `fixtures/corpus/noisy-speech/` (twelve English and
Spanish passages, 469 words per level, clean and synthetic `office-v1` noise at 20, 10 and
5 dB) and is measured by the same qualification test
([record](../planning/p07-asr-qualification.md), section of 2026-10-20): `base` 5.1% clean,
5.3%, 5.1% and 7.2% at 20, 10 and 5 dB on hosted Ubuntu, within half a point of that on
Windows. **The maintainer decided on 2026-10-20** (recorded on #150) to gate it for
`base`: at most 10% word error rate at 20 dB, 10% at 10 dB and 15% at 5 dB, enforced by the
test. This supersedes the 2026-09-25 deferral above for synthetic noise only: F08 stays
gated on critical terms, and real noise is not measured. The set's utterance WAVs are
assets of the test-media release, fetched by digest; the committed clips are what the test
reads.
Accent and crosstalk still need human or licensed recordings
([L-198](../planning/known-limits.md#l-198)).
