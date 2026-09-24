# VSift current status

As of 2026-09-24. Current-state document: rewrite it, don't append to it. Next
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
  offset, and return timestamped transcript segments for a time range, either as one
  page or as a JSON Lines stream of keyed evidence records an indexer can upsert;
- prove, automatically and once per tool pair, that FFmpeg/FFprobe really work
  before it runs them on a user's video;
- manage the session's lifetime and retention, and validate retained bundles,
  including the content of their transcript records;
- keep every folder it creates private to the user, whatever the parent folder grants.

Internally (P07 increment 3a) it can now also transcribe speech itself with
whisper.cpp, but no command exposes that yet: `transcript retranscribe` is the next
increment. Search and visuals are P08-P09.

## What works (public CLI)

- `setup check`, `setup configure`, `setup configure-model` and the read-only
  `setup plan` (`setup check` still reports only `executable_probe_only`). Its
  `detail` is now only a tool's version banner; whisper's output is never echoed and
  its detail says whether the executable is the reviewed v1.9.2 build (fix in 3a).
- `ingest <video>`: stages and hashes one local MP4/Matroska source into a disposable
  session (24 idle hours, at most 7 days). Runs no provider.
- `ingest <video> --transcript <file> [--transcript-offset <signed us>]`: parses the
  sidecar under a bounded strict policy, locates FFmpeg/FFprobe (configured first,
  then `PATH`; never Whisper), runs the media-tool preflight, probes the staged
  video's duration, keeps only cues wholly inside it, and commits source and
  transcript in one generation. Rejections carry a typed reason in fixed prose.
- `transcript get <session> --from --to [--limit 1..100] [--cursor]`: a bounded page
  of self-describing segment records; cursors bound to session, revision, range and
  expiry. With `--events jsonl` the page is an evidence stream (below).
- `session list/status/renew/close/retain/clean` and `bundle validate` (which now also
  decodes, strictly, the version-2 transcript record no command writes yet).
- Still `COMMAND_NOT_IMPLEMENTED`: `transcript retranscribe`, search, candidates,
  frame, audio, crop, job and setup install/repair/list/rollback/remove.

## Local ASR core (P07 increment 3a, internal)

- **Domain:** a revision's provenance is an import or one `AsrRun` (whisper build and
  model digests, decoding profile `r0-v1`, chunk plan, threads, audio stream, every
  chunk's outcome: transcribed, silent or no audio). Each segment names its origin;
  ASR segments re-derive their range from the chunk's observed decoded start plus
  provider times. `asr` module: R0 chunks (30 s windows, 5 s overlap), provider-output
  validation (reject/trim/fail rules of T-05/T-06), integer -50 dBFS silence test, and
  a deterministic seam merge (core ownership, edge-cut replacement, >= 2-word
  duplicate trimming, genuine repeats kept).
- **Application:** `SpeechAudioSource`/`SpeechRecognizer` ports, `transcribe_range`
  (identity checked before and after; typed `AsrFailure { stage, reason }`; nothing
  returned on failure or cancellation) and `build_asr_revision`.
- **Infrastructure:** `FfmpegMedia::speech_pcm` (<= 30 s, 1 MiB, 60 s, first decoded
  PTS), `FfmpegSpeechAudio`, `whisper_cli` (closed argv checked against v1.9.2
  `--help`, 120 s per chunk, stdout/stderr bounded and ignored, bounded no-follow read
  of the `-ojf` file), `WhisperSpeechRecognizer`, transcript record version 2.
- **Real run (opt-in, Windows 11, whisper.cpp v1.9.2 official build, base model):**
  F01, F05, F08 and F09 transcribed with required words present; F01 reads "The
  service status is healthy and the build is 2048." F09's audio starts at 0.75 s.
- Imports unchanged: identities and version-1 record bytes pinned against `9dad79e`.

## Private folders (fix, PR #141, `dadefbc`)

Every folder VSift creates (per-user configuration and its missing parents, session
root and its parent, retained bundles, managed data) is made private before use: a
protected DACL for the user, SYSTEM and Administrators on Windows (`windows-acl`, no
`unsafe`), 0o700 on Unix. Existing folders are never changed; a non-private one fails
`STORAGE_IO` naming its kind. Threat model SEC-18.

## Evidence stream and bundle record (P07 increment 2c)

- `transcript get --events jsonl`: one keyed evidence event per segment
  (`record_type: "transcript_segment"`, upsert `key` = `segment_id`, contiguous
  `sequence` from 0), then one terminal event carrying the page without `items`,
  `record_count` and `next_cursor`. Full rules: `cli-v1.md` ("Evidence stream").
- `bundle validate` decodes every `transcript_record` strictly
  (`bundle-transcript-record.schema.json` describes version 1).

## Speech fixtures and preflight

- Kokoro-spoken F01-F09/F12 clips (run 36052657304; test-only,
  `docs/planning/p07-speech-fixtures.md`); recorded whisper v1.9.2 `-ojf` outputs in
  `crates/vsift-infrastructure/tests/fixtures/whisper-1.9.2/`.
- `ensure_media_tools_verified` runs once per tool identity before `ingest
  --transcript` touches the session root; local ASR must call it too (3b).

## The engine library and contract

- **`crates/vsift`:** `Engine::new(EngineConfig, EnginePorts)`; setup, session,
  `transcript`, `validate_bundle`, `verify_media_tools`, `identify_model`. One typed
  `EngineError`; `failure_code()` is the single public-code mapping. It re-exports the
  provenance types (`TranscriptProvenance`, `SegmentOrigin`, `AsrRun`, ...). API is 0.x.
- **`crates/vsift-contract`:** every v1 wire type, the stream sequencing, fixed-prose
  warnings and remediation. Import-only fields are omitted for local-ASR revisions,
  so imports serialize exactly as before.
- **`vsift-cli`** is a thin host.

## What exists internally (not exposed by the CLI)

- **P02:** shell-free process supervision with containment, bounded output, deadline
  and cancellation. **P03:** private storage roots, locks, admission, immutable
  generations, process-crash recovery (ephemeral profile only; FS-01).
- **P04:** restricted FFprobe/FFmpeg metadata, frame, audio and now speech PCM.
- **P06:** model identification; its F01 verifier feeds the preflight.
- **Managed-installer foundations** (owned by P13).

## Packet status

| Packet | Status in plain terms |
| --- | --- |
| P00–P05 | Complete; merge commits and evidence are in the ledger |
| P06 | Complete: detect, select, verify and guide (PR #123, `b73df52`) |
| P07 | In progress: 1a-2c, speech fixtures and 3a (ASR core) done; 3b and 3c next |
| P08–P12, P14 | Not started |
| P13 | Not started; now also delivers managed dependency installation |

## Architecture snapshot

`vsift-domain` (values, no I/O) <- `vsift-application` (use cases and ports) <-
`vsift-infrastructure` (OS, processes, storage, providers, parsers) <- `vsift` (engine)
<- `vsift-cli` (parse, present). `vsift-contract` sits beside the engine and depends on
domain and application only. `tools/vsift-governance` checks the ledger and these
files' sizes. Largest modules: `filesystem_session_store.rs` and
`managed_artifact_store.rs` (about 3.8k and 3.6k lines with tests), then the domain
`transcript.rs` and `asr.rs`.

## Quality evidence

- Increment 3a, Windows 11: fmt, strict Clippy (pedantic as errors), `cargo test
  --workspace` (486 passed, 24 opt-in ignored), warning-denied rustdoc and the
  governance check pass. Opt-in `p07_local_asr` passed with the reviewed whisper build
  (SHA-256 `95e3c0b0...`, verified) and base model: about 8 s per clip in release,
  46 s in debug (mostly hashing the model twice); `setup check` against it reports
  `whisper.cpp v1.9.2 (reviewed build)`.
- Earlier: the P07 transcript E2E consumes each import as a JSONL stream, retains it
  and validates the bundle; fix #131 race tests 50/50; private-folder ACL tests.
- CI on every PR: Quality on Ubuntu, macOS and Windows; Documentation, Governance,
  strict worker boundary, dependency policy and CodeQL; squash merges to protected
  `main`. Qualification records are in `docs/planning/`; history in git,
  `CHANGELOG.md` and `docs/history/2026-09-09-to-23-delivery-log.md`.
