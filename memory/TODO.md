# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

Delivery was re-planned on 2026-09-23 ([ADR 0015](../docs/decisions/0015-r0-delivery-replan.md),
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md)). P00-P06 are
complete; P06 closed on detect, select, verify and guide (PR #123, `b73df52`). P07 is in
progress. Nothing public can yet read a video's content; P07 starts that:

1. **P07 increment 1a (done, in review):** the `vsift-contract` crate now owns the v1
   JSON wire types and their mapping from domain and application values. No behaviour
   change: the CLI contract and schema suites pass unchanged, and a differential run of
   47 CLI invocations against `7f57f69` produced identical output.
2. **P07 increment 1b (next):** the `vsift` engine facade. Move the engine use cases
   out of `vsift-cli` (session-root resolution, ingest and session flows, setup-plan
   evaluation and acceptance), inject clock and identifier ports, and leave the CLI a
   thin host: parsing, presentation, exit codes and composition. Still no behaviour
   change; the same suites must pass unchanged.
3. **P07 transcription:** segment-first, with published transcript schemas and
   SRT/VTT fuzz targets. Includes the preflight that calls
   `FixtureMediaToolVerifier` before the first media stage, and whisper.cpp
   functional verification.

## Open decisions (maintainer)

- Crate names are confirmed (`vsift` facade, `vsift-contract`); a crates.io
  availability check still precedes first publication.
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.
- Removing leftover local worktrees and squash-merged `codex/*` branches.

## Known issues and gates

- Schema drift found during 1a, not yet tracked in an issue (no schema change was
  allowed in 1a): `operation-response.schema.json` has no `ISOLATION_UNAVAILABLE`
  error code although `FailureCode::IsolationUnavailable` exists, and its `command`
  pattern rejects `setup.configure-model`, which the CLI already emits.
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
