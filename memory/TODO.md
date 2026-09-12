# VSift project work record

## Current checkpoint

2026-09-13: The maintainer confirmed the R0 setup journey in ADR 0014:
detect existing/partly installed components first, explicitly plan/install
reviewed missing ones, and always provide typed manual/BYO guidance if managed
installation is unavailable, denied or fails. Script-installed off-PATH tools
must be selectable. Agents need rich remediation but cannot infer install
authority from video inspection. The BYO-only proposal PR #49 closed unmerged;
P06 source-review PR #48 was carried forward and closed as superseded by PR #50.
The source assessment and clarification commits are `b1c271d` and `6ab6ef3` on
that review branch. P06 remains planned, with its immutable provider/model
catalogue still a gate for each managed target. This changes acceptance, not
runtime behavior; no protected merge or implementation evidence yet.

2026-09-12: P06 source review started from protected main
`777bc3e56788d43cd9a647dba54c287390c2998d`; P05's evidence follow-up is
merged and issue #8 is closed. A reviewed cross-target provider/model artifact
catalog is absent. P06 implementation is gated by the source and licence decision
recorded in `docs/planning/p06-provisioning-source-review.md`.

2026-09-12: P05 implementation from protected main
`8741f57dfc5b45c8cb4d3f1f7791d0f1d143c627` merged through protected PR #46 as
`c3f9313f8df0871d17ab80ad5bb142be6421b36f`. The ledger now records
completion and the passed three-OS quality, governance, documentation,
security and local checkpoint evidence. Its evidence follow-up has since merged
and issue #8 is closed. P06 is the next eligible packet and remains planned.

2026-09-12: P04 source/media primitives completed through protected PR #44 as
`4fc859b3344bd47c254dd9da9cac72f5ad3d61d5`. This evidence-only follow-up
records its completed ledger status and final test evidence. Issue #7 closed after
that record merged, making P05 eligible at this checkpoint.

2026-09-11: P03's implementation completed through protected PR #42 as
`3eef9b7ac3bcfe092d82137ccf2aa9aa084aca4f`. Its private provisioning and ACL
checks, weighted admission, lifetime holds, fenced generations and process-crash
recovery remain internal; no storage/session command is exposed. This evidence-only
follow-up records the completed ledger state; issue #6 is closed. P04 is now the next
eligible packet but remains planned.

## Pending

- Merge the ADR 0014 progressive-setup clarification through protected checks
  without claiming P06 completion. Issue #9 now describes detect/install/guide.
- Resolve PR #48's reviewed per-target FFmpeg/FFprobe, whisper.cpp and model
  source matrix before managed activation. Qualify at least one complete
  managed-install target; on every named R0 target, test the typed manual/BYO
  fallback, permission denial and off-PATH selection. D-01..D-10 and the P06
  E2E stage remain pending.

- P06 source gate (2026-09-12): select and review immutable per-target
  FFmpeg/FFprobe, whisper.cpp CLI and multilingual `base` model artifacts before
  managed download/activation. The current source matrix cannot justify a
  three-target installer. See `docs/planning/p06-provisioning-source-review.md`.
  P06 remains planned; D-01..D-10, its E2E stage, ledger completion and issue #9
  closure remain pending.
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
  are recorded in the P03 feasibility document. P03's narrower ephemeral profile is
  complete; this durable qualification remains P10/P11/P14 work.
- Preserve the complete R0 local-video-to-grounded-handoff journey. It must pass with
  supplied-transcript and local-ASR paths through named Codex and Claude Code clients;
  do not defer product usefulness to R1.
- Track the cumulative opt-in E2E spine in `docs/planning/e2e-test-spine.md` and issue
  #40. P04 and P05 have real-media checkpoints, with P06-P14 and the complete
  journey explicitly `not_implemented`. Every later packet must attach its production
  stage or explicitly record why none applies; P12/P14 cannot substitute for missing
  earlier integration.
- Preserve managed indexing, optional enrichment, source-grounded reconstruction,
  industrial worker growth and integrated qualification as R1 packets P15..P20 under
  ADR 0011 and `docs/planning/r1-industrial-capability-expansion.md`. SQLite remains
  only a candidate explicit single-node adapter. Do not start P15 implementation before
  P14 or use R1 scope to expand the active P03 packet. GitHub milestone 2 contains
  future issues #25 through #30.

## Completed

- 2026-09-12: P05 implementation commit
  `1238580a6a86a433019fb2ecabfeec3eef56ea43` and fixture correction
  `8e7232c86d1d324b8ccd7e8e5ba271d813544ee5` merged through protected
  PR #46 as `c3f9313f8df0871d17ab80ad5bb142be6421b36f`.
  Foreground ephemeral ingestion, typed lifecycle, bounded list/cleanup,
  source-safe retention and data-only bundle validation passed local gates,
  native FFmpeg/FFprobe P05 checkpoint, three-OS Quality, Governance,
  Documentation, strict-worker, dependency policy/review, CodeQL and Rust
  analysis. The bundle publication limit is recorded in ADR 0013.

- 2026-09-12: P04 merged through protected PR #44 as
  `4fc859b3344bd47c254dd9da9cac72f5ad3d61d5`. Internal no-follow source
  snapshots, typed restricted FFprobe/FFmpeg metadata/frame/audio operations,
  deterministic project-owned fixtures, independent pixel/timestamp verification and
  the opt-in seven-scenario real-media checkpoint landed. Local fmt, strict Clippy,
  workspace tests, warning-denied rustdoc, governance and cargo-deny passed. The
  generator reproduced the same provenance SHA-256 on a second same-build run; the
  verifier checked 11 source clips and seven malformed variants. Protected Quality
  passed on Ubuntu, Windows and macOS; Governance, Documentation, strict-worker,
  dependency policy/review, CodeQL and Rust analysis passed. Native FFmpeg 9.0 on
  Windows/NTFS is development evidence, not a release support claim. ADR 0012
  records the remaining desktop decoder isolation limits; P05-P14 and the complete
  E2E journey remain `not_implemented` at this checkpoint.

- 2026-09-11: P03 completed in protected PR #42 as
  `3eef9b7ac3bcfe092d82137ccf2aa9aa084aca4f`. It added owned private-root
  provisioning, Unix owner/mode and Windows DACL validation, immutable root-wide
  weighted admission, cross-process shared/exclusive lifetime coordination, fenced
  immutable generation publication, bounded linked-manifest verification and recovery
  at every manifest/pointer write, flush and rename boundary. Durable initialization
  and publication fail before mutation. The local workspace passed 128 tests (three
  child-process entries intentionally ignored and launched by parent tests), fmt,
  strict Clippy, rustdoc, governance and cargo-deny. All protected three-OS quality,
  governance/documentation, strict-worker, dependency and CodeQL/Rust checks passed.
  The adapter remains internal, strict OS/storage crash durability remains assigned to
  P10/P11/P14, and P04 remains planned until separately started.
  PR #43's first Windows run also exposed a timing-only concurrency-test weakness;
  its bounded monotonic retry fix passed ten consecutive targeted runs and the full
  local gate set before protected checks were rerun.

- 2026-09-11: P03 implementation increments PR #35
  (`9ee3c048e1460008cd4f6c3e16dc23f78115ad0d`) and PR #36
  (`65fe00c3d43405a6ed5c8bda8ae50a2896e80d65`) established typed storage
  guarantees plus the internal capability-scoped generation-zero initializer. The
  second increment passed 105 local tests and all protected cross-platform/security
  checks after its Unix lint regression was corrected. A later Windows run exposed
  time-only temporary-root naming as intermittently non-unique under parallel tests;
  PR #38 (`3fd29247bb241e4aa6f5561e410a06ed5d1cf839`) added a process-local
  sequence and a same-timestamp regression test. The corrected suite passes 106 local
  tests and every protected job. These increments do not complete P03 or expose a
  storage/session command.
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
