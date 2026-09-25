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

Maintainer decision D6 proposed the gates for the `base` default on 4 threads:
real-time factor at most 0.5, `whisper-cli` peak memory at most 400 MiB, word error
rate at most 10% on clean clips and at most 25% on F08, and every spoken critical
term except reviewed known misses. It also added a reviewed `base_q5_1` profile
(the q5_1 quantization of the same model). Measured on Windows 11, Xeon E5-2698 v4
([record](../planning/p07-asr-qualification.md)): `base` has a real-time factor of
0.388, 338 MiB peak memory, 3.25% pooled WER without noise and no unexpected
critical-term miss, but **61.5% WER on F08**, so it does not meet every gate;
`base_q5_1` also misses F08 (46.2%). `base` therefore is not yet qualified as the
default under the gates as written. It stays the pinned default in the reviewed
plan and the command until the maintainer decides between the options in the
record; this note records the outcome, not that decision.
