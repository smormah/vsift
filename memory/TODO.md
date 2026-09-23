# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

Delivery was re-planned on 2026-09-23 ([ADR 0015](../docs/decisions/0015-r0-delivery-replan.md),
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md)) when work
moved to attended implementation with maintainer review. Nothing public can yet read
a video's content; this order gets there:

1. **Close P06 (narrowed by ADR 0015).** Every P06 increment has landed or is in
   review: detection, BYO selection, read-only plans, typed guidance, engine
   verification of the selected FFmpeg/FFprobe (`FixtureMediaToolVerifier`) and
   model identity (`verify_model_file`). The final increment audits D-01, D-07
   (unavailable-target guidance), D-09 and D-10 case by case, closes the gaps
   (denied configuration storage, read-only tool install, saved-plan revalidation
   through the binary, manual guidance on every unavailable target) and adds the
   opt-in P06 E2E stage (`p06_setup_e2e`), which passed locally on Windows 11.
   By design there is no `setup verify` command (ADR 0015 note). Remaining:
   - Merge the final increment once protected checks pass.
   - Then the one-off ledger completion record and an issue #9 update. P06 is
     not complete until that record lands.
2. **P07 increment 1:** extract the embeddable engine facade and contract crate
   with no behaviour change (ADR 0016).
3. **P07 transcription:** segment-first, with published transcript schemas and
   SRT/VTT fuzz targets.

## Open decisions (maintainer)

- Final crate names for the facade and contract crate (provisional `vsift`,
  `vsift-contract`), after a crates.io availability check.
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.
- Removing leftover local worktrees and squash-merged `codex/*` branches.

## Known issues and gates

- #66 is closed. A recurrence of lock `Busy` is a new finding, not a re-run candidate.
- FS-01: strict OS/storage-crash durability is unqualified. Durable requests fail
  closed until P10/P11/P14 run the Ubuntu/ext4 campaign (ADR 0010).
- Baseline findings B-01..B-11 close through their mapped packets.

## Parked: managed installation (now P13)

Resume order recorded when P06 was parked on 2026-09-23:
1. Production smoke executor over the digest-bound policy.
2. Failure cleanup before activation.
3. The guarded download/stage/smoke/publish transaction.
4. Public install, rollback and uninstall.
5. Bounded version cleanup.
6. Process-kill and power-loss qualification.
7. D-02..D-08 and the managed-install E2E stage.

The reviewed Ubuntu 24.04 x86-64 catalogue stops new plans on 2028-08-01 and relies
on the publisher keeping the month-end FFmpeg asset; revalidate or replace it
through a reviewed catalogue revision. Never resolve a live "latest".

## Guardrails

- R0 ships only when a coding agent goes from a local video to a grounded handoff
  through both the supplied-transcript and local-ASR paths, in named Codex and
  Claude Code trials (A-08/A-09).
- R1 packets P15..P20 (milestone 2, issues #25..#30) start only after P14.
- Live capture ([#107](https://github.com/smormah/vsift/issues/107),
  [#108](https://github.com/smormah/vsift/issues/108)) waits for the finite-video journey.
