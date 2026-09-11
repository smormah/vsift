# VSift project work record

## Current checkpoint

2026-09-11: P03 / issue #6 remains active on protected main
`c2b3829d77279a32b3487ab1170f820f1d68eeb5`. PR #33 accepted the narrower
storage profile: ephemeral NTFS/APFS P03 work may proceed, while strict Ubuntu/ext4
durability remains disabled until P10/P11/P14.

## Pending

- Implement and prove P03 handle-relative containment, stable locks, admission and
  process-crash-consistent ephemeral publication before completing the packet.
- FS-01: OS/storage crash qualification is missing. The default cap-std NTFS
  read-only directory handle fails synchronization; a safe writable-directory
  handle succeeds. Do not misreport this as Windows durability being impossible.
  ADR 0010 accepts the narrower desktop profile; do not treat this as durable evidence.
  Supply an owned disposable Ubuntu/ext4 fault environment before P10/P11 durable
  enablement and keep explicit durable requests fail-closed until then.
- Resolve baseline findings B-01..B-11 through their mapped implementation packets.
- P03 spike implementation: `5f15c7730607570663d739b7a4dd70a03947e500`, merged
  through PR #24 as `cbc531e80761078354be0b9942c52f00ddac05b0`.
  Local Windows validation: 92 tests, fmt, strict clippy, governance and rustdoc pass.
  Security review enabled cargo-deny's development licence/duplicate checks and
  recorded the existing borrow-or-share MIT-0 licence review. Follow-up
  `a262fa9bbabb4bebc6ebde581204c4dbe0a8186d` passed ten storage experiments each
  on NTFS, APFS and ext4 in CI run 34585520298; strengthened dependency checks pass
  in run 34585520299. Exact environments and the unresolved OS/storage crash gate
  are recorded in the P03 feasibility document. P03 remains incomplete.
- Preserve the complete R0 local-video-to-grounded-handoff journey. It must pass with
  supplied-transcript and local-ASR paths through named Codex and Claude Code clients;
  do not defer product usefulness to R1.
- Preserve managed indexing, optional enrichment, source-grounded reconstruction,
  industrial worker growth and integrated qualification as R1 packets P15..P20 under
  ADR 0011 and `docs/planning/r1-industrial-capability-expansion.md`. SQLite remains
  only a candidate explicit single-node adapter. Do not start P15 implementation before
  P14 or use R1 scope to expand the active P03 packet. GitHub milestone 2 contains
  future issues #25 through #30.

## Completed

- 2026-09-11: the narrower P03 storage qualification profile merged through PR #33
  as `c2b3829d77279a32b3487ab1170f820f1d68eeb5`. ADR 0010 permits
  process-crash-consistent ephemeral desktop publication, keeps explicit durable
  requests fail-closed, and assigns strict Ubuntu/ext4 durability qualification to
  P10/P11/P14. P03 implementation remains pending.
- 2026-09-11: the R1 managed industrial capability expansion merged through PR #31
  as `0cfdb407f805282995f326ca93c99bc7170eda04`. ADR 0011 preserves a complete R0
  video-to-grounded-handoff release gate and reserves R1 requirements R-15..R-20,
  packets P15..P20, threats SEC-26..SEC-35 and E/RC/I/H/Q tests. Milestone 2 and
  issues #25-#30 hold future R1 work; R0 issues #15/#17 carry the two-agent gate.
- 2026-09-09: foundation and GitHub governance published in `5289a2b`; support channel
  update in `d7a459e`.
- 2026-09-09: setup command rename merged through PR #1 as `df85f70`.
- 2026-09-09: source review of `df85f70` and comprehensive implementation proposal
  merged through PR #2 as `e3f8569`. It adds no runtime implementation.
- 2026-09-10: P00 merged through PR #18 as `924f6c5`. It accepted DEC-01..13,
  established milestone 1 and issues #3-#17, froze F01-F12 declarative fixture truth,
  and added a machine-validated ledger plus a required protected-branch Governance
  check. All PR checks passed; the workspace had 13 passing tests.
- 2026-09-10: P01 merged through PR #20 as `3d7a7d2`. It published the typed
  R0 CLI/config/JSON boundary, four v1 schemas, domain value contracts, stable
  errors/exits, bounded presentation and C-01..C-10 coverage. All protected checks
  passed across Ubuntu, Windows and macOS; the local workspace had 64 passing tests.
- 2026-09-11: P02 merged through PR #22 as `4e9ef08`. It added canonical provider
  resolution, shell-free process supervision, bounded concurrent pipes, one operation
  deadline, sticky cancellation, Windows Job Object/Unix process-group cleanup, and
  honest strict-isolation reporting. P-01..P-08 and C-05 passed across protected
  three-OS and strict Linux checks; the local Windows workspace had 82 passing tests.
