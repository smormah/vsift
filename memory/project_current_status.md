# VSift current status

As of 2026-09-25. Current-state document: rewrite it, don't append to it. Next
actions and open decisions are in `memory/TODO.md`.

## In plain English

VSift is a Rust command-line tool that gives AI coding agents local,
source-grounded access to the evidence in a video. Under
[ADR 0016](../docs/decisions/0016-embeddable-engine-and-evidence-contract.md) it is
also an embeddable engine library (`vsift`) that the CLI, and later other hosts, use.
Today it can:
- check and register its dependencies and show a read-only setup plan;
- copy a video into a private, disposable session;
- import an existing SRT or WebVTT transcript with the video, aligned by an explicit
  offset;
- **transcribe the video's speech itself with whisper.cpp** (`transcript
  retranscribe`, P07 increment 3b, in review): the whole video or one range, into a new
  revision that keeps earlier citations valid;
- return timestamped transcript segments for a time range, from the newest revision
  or any earlier one, as one page or as a JSON Lines stream of keyed evidence records;
- prove, automatically and once per setup, that FFmpeg/FFprobe and the speech
  recognizer really work before running them on a user's video;
- manage the session's lifetime and retention, and validate retained bundles,
  including the content of their transcript records;
- keep every folder it creates private to the user, whatever the parent folder grants.

Increment 3b is complete (PR #149, ADR 0017 accepted 2026-09-25). The
P07 packet is not complete: increment 3c (setup-check reporting and the model
profile gates) and the packet record remain. Search and visuals are P08-P09.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model` and the read-only
  `setup plan` (`setup check` still reports only `executable_probe_only`).
- `ingest <video>`: stages and hashes one local MP4/Matroska source into a disposable
  session (24 idle hours, at most 7 days). Runs no provider.
- `ingest <video> --transcript <file> [--transcript-offset <signed us>]`: parses the
  sidecar, runs the media-tool preflight, probes the video, keeps only cues wholly
  inside it, and commits source and transcript in one generation. Never needs whisper.
- `transcript retranscribe <session> [--from <us> --to <us>]` (3b): resolves FFmpeg,
  FFprobe, `whisper-cli` and the registered model, refuses any model that is not the
  pinned base profile, runs the media-tool and local-ASR preflights, then decodes
  30 s chunks of the session's committed copy into a private work directory inside
  the session and commits one version-2 record. A range widens to whole segments of
  the newest revision; segments outside it are carried under new identities naming
  their origin (`carried_from`). No speech still commits a revision
  (`no_speech_recognised`). Existing failure codes, fixed-prose remediation.
- `transcript get <session> --from --to [--limit 1..100] [--cursor] [--revision]`: the
  newest revision by default, any revision by identity; `--events jsonl` streams it.
  A session without a transcript gets remediation naming both routes.
- `session list/status/renew/close/retain/clean` and `bundle validate` (records v1
  and v2, decoded strictly).
- Still `COMMAND_NOT_IMPLEMENTED`: search, candidates, frame, audio, crop, job and
  setup install/repair/list/rollback/remove.

## Local ASR (P07 increments 3a and 3b)

- **Domain:** a revision's provenance is an import or one `AsrRun`; a spliced revision
  also holds `inherited` provenance, and every segment (own or carried) is re-checked
  against the rule that produced it. `snap_to_segments` widens bounded ranges. R0
  chunks: 30 s windows, 5 s overlap; validation, -50 dBFS silence and seam merge.
- **Application:** `transcribe_range` (refuses unpinned models, identity checked before
  and after), `build_asr_revision` (splicing), `LocalAsrVerifier` port and
  `preflight_local_asr` (shares the verification record, own fingerprint domain).
- **Infrastructure:** whisper CLI adapter (abnormal exits are `RESOURCE_LIMIT`),
  `FixtureAsrVerifier` over the embedded F01 speech clip and its fingerprint,
  `SourceSnapshot::open_committed`, `session_work_directory`, reads by revision
  identity, record version 2 with carried segments.
- **Engine:** `Engine::retranscribe`; `EnginePorts::with_local_asr_verifier` and
  `with_speech_recognizer` for hosts and tests.
- **Contract:** widened `transcript-segment`, `transcript-revision` and
  `bundle-transcript-record` schemas (D2), new `transcript-retranscribe-data`, frozen
  F01 speech examples; imports serialize exactly as before.

## Evidence stream and private folders

- `transcript get --events jsonl`: one keyed evidence event per segment, then one
  terminal event. No tombstones (D8): superseded revisions stay valid and readable;
  consumers filter on `revision_id`. `retranscribe --events jsonl` writes only its
  terminal event.
- Every folder VSift creates is made private before use; a non-private existing one
  fails `STORAGE_IO` naming its kind. Threat model SEC-18.

## Fuzzing (P07, PR #146)

- `fuzz/` has six `cargo-fuzz` targets over published parsers (SRT, WebVTT, whisper
  `-ojf`, transcript records v1/v2, FFprobe metadata, cursors); weekly pinned-nightly
  `Fuzz` workflow, per-PR stable seed replay. 3b added the spliced record as a seed.

## The engine library and contract

- **`crates/vsift`:** `Engine::new(EngineConfig, EnginePorts)`; setup, session,
  transcript, retranscribe, `validate_bundle`, `verify_media_tools`, `identify_model`.
  One typed `EngineError`; `failure_code()` is the single public-code mapping. API 0.x.
- **`crates/vsift-contract`:** every v1 wire type, stream sequencing, fixed-prose
  warnings and remediation. **`vsift-cli`** is a thin host.

## What exists internally (not exposed by the CLI)

- **P02:** shell-free process supervision. **P03:** private storage roots, locks,
  admission, immutable generations, process-crash recovery (ephemeral profile; FS-01).
- **P04:** restricted FFprobe/FFmpeg metadata, frame, audio and speech PCM.
- **P06:** model identification. **Managed-installer foundations** (owned by P13).

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | In progress: 1a-3a and fuzz merged; 3b merged (ADR 0017 accepted); 3c next |
| P08–P12, P14 | Not started |
| P13 | Not started; now also delivers managed dependency installation |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present). `vsift-contract` sits beside the engine and depends on
domain and application only. `tools/vsift-governance` checks the ledger and these
files' sizes; `fuzz/` is the fuzz harness. Largest modules: `filesystem_session_store.rs`
and `managed_artifact_store.rs`, then the domain `transcript.rs` and `asr.rs`.

## Quality evidence

- 3b, Windows 11: fmt, strict Clippy, rustdoc with warnings denied, governance and the
  fuzz seed replay pass; `cargo test --workspace` passes apart from the known #128
  process-supervisor flake under load (passes alone). Opt-in with the reviewed
  whisper.cpp v1.9.2 and pinned base model (release build): `p07_local_asr` passed
  all nine stages in 64 s, about 7.5 s per short clip once verified, 16 s for the
  first run (both preflights); the real two-chunk seam kept every word exactly once.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance, fuzz
  harness replay, strict worker boundary, dependency policy and CodeQL; squash merges to
  protected `main`. Qualification records are in `docs/planning/`; history in git,
  `CHANGELOG.md` and `docs/history/2026-09-09-to-23-delivery-log.md`.
