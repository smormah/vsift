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
