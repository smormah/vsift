# VSift work record

Current-state handoff. Rewrite this file in every change and keep it within the
governance checker's size limit. History lives in git, `CHANGELOG.md`, the
qualification records and `docs/history/2026-09-09-to-23-delivery-log.md`.

## Now

Delivery was re-planned on 2026-09-23 ([ADR 0015](../docs/decisions/0015-r0-delivery-replan.md),
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md)). P00-P06 are
complete. P07 is in progress: three increments are done, the packet is not.

1. **Increment 1a (done, PR #126, `00e707f`):** `vsift-contract` owns the v1 wire types.
2. **Increment 1b (done, PR #127, `0e5a0cd`):** the `vsift` engine facade; the CLI is a
   thin host.
3. **Increment 2 (done, in review):** supplied-transcript path, the transcript stage of
   A-09. `ingest --transcript <srt|vtt> [--transcript-offset <us>]` and
   `transcript get <session> --from --to [--limit] [--cursor]`. Policies are in
   `docs/contracts/cli-v1.md` ("P07 supplied transcripts"). T-01/T-02 evidence is in
   `docs/planning/verification.md`; the opt-in `p07_transcript_e2e` stage passed.
4. **Increment 3 (next): local ASR.** Blocked on the maintainer's choice of
   speech-fixture source (F01-F09/F12 scripts need real speech audio; the corpus has
   tone sentinels only). Scope once unblocked:
   - whisper.cpp adapter over the process supervisor, output validated and offset to
     source time; imported and ASR revisions share `TranscriptRevision`;
   - bounded PCM chunking with overlap and deterministic duplicate removal at seams;
   - an automatic preflight calling `Engine::verify_media_tools` before the first
     media stage (including the transcript-import probe, which runs without it today);
   - whisper functional verification reported by `setup check`;
   - `transcript retranscribe` (still `COMMAND_NOT_IMPLEMENTED`) as a new revision;
   - T-03..T-06 and the `p07_local_asr` E2E stage.
5. **Still owed by P07 before the packet closes:**
   - the JSON Lines evidence stream decision (ADR 0016 decision 5): today
     `transcript get --events jsonl` is one terminal event;
   - `cargo-fuzz` targets for the SRT/VTT parsers (ADR 0016 decision 6) need a
     nightly-toolchain decision; `proptest` properties cover them now;
   - a published schema for the bundle's `transcript_record` artifact (today a strict,
     versioned internal storage record);
   - the packet completion record in the ledger once increment 3 merges.

## Open decisions (maintainer)

- Speech-fixture source for local ASR (blocks increment 3).
- Whether fuzzing may use a nightly toolchain in CI, or stays a scheduled job.
- Crate names are confirmed (`vsift` facade, `vsift-contract`); a crates.io
  availability check still precedes first publication.
- Minimum-supported-Rust-version policy before the library is first published.
- Whether and when to cut 0.x pre-releases after P09.
- Whether a local MCP adapter is wanted after P12. The CLI and skill stay primary.
- Removing leftover local worktrees and squash-merged `codex/*` branches.

## Known issues and gates

- Supplied-transcript success paths through the engine and the binary need real
  FFprobe, so they are opt-in (`--ignored`); hosted CI covers them only through the
  application, contract, store and parser tests. Rejections are covered everywhere.
- One full local `cargo test --workspace` run on Windows saw four
  `process_supervisor` tests fail (child exit status), then pass on rerun and alone.
  Not caused by increment 2; open an issue before treating a recurrence as noise.
- The unchanged P06 checkpoint test still uses application and infrastructure types,
  so the CLI keeps both as development dependencies.
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
- New features land in the engine (`crates/vsift`) once, never in a host.
- A new public command or failure code needs a `vsift_contract::CommandName` or
  `FailureCode::ALL` entry; conformance tests then require the v1 schemas to accept
  it (the fix for issue #125).
