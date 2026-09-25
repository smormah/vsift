# ADR 0016: Embeddable engine and published evidence contract

- Status: Accepted
- Date: 2026-09-23
- Tracking: [P07 / issue #10](https://github.com/smormah/vsift/issues/10), with
  follow-through in P08, P09, P12 and P13
- Refines: [ADR 0001](0001-rust-native-cli.md),
  [ADR 0003](0003-external-runtime-adapters.md) and
  [ADR 0008](0008-cli-and-json-contract.md)

## Context

VSift is meant to be a component others plug in: coding agents, other agents,
server workers, indexing services and, later, a desktop application. Today the
CLI is its only public surface. The layering is sound (domain ← application ←
infrastructure ← CLI), but the CLI crate holds orchestration that every host
would need:
- session-root resolution
- ingest and session flows
- setup-plan evaluation and acceptance
- identifier and clock generation
- the v1 JSON response types

An embedding host would have to copy that code or shell out to the CLI.

The evidence model is already specified (architecture and contracts §2 and
§10). It defines transcript segments, visual candidates, evidence artifacts,
coverage, and the retained bundle as the handoff to index consumers. However,
no evidence-record schema is published yet, and `--events jsonl` carries only
progress and terminal records. Live capture ([#107](https://github.com/smormah/vsift/issues/107))
is a planned later source mode. Fuzzing is scheduled only at P14 qualification,
and nothing has been released.

## Decision

1. **Library first.** VSift's engine becomes a public Rust library that the
   CLI, a future MCP adapter, a worker host and a desktop application all use.
   - A facade crate (provisional name `vsift`) exposes composed use cases.
     Hosts construct the engine with explicit adapters and configuration, with
     no global state or service locator.
   - A contract crate (provisional name `vsift-contract`) owns the versioned
     wire types and their schema conformance tests, so every host emits
     identical JSON.
   - `vsift-cli` becomes a thin host: argument parsing, presentation, exit
     codes and composition only.
   - Identifier generation and the clock become injected ports.
   - The domain, application and infrastructure crates remain implementation
     detail behind the facade.
   - Crate names are confirmed by an availability check before first
     publication, as ADR 0009 requires for the npm name.
2. **Refactor before new features.** P07's first increment extracts the facade
   and contract crate without changing behaviour. The existing CLI contract and
   schema tests must pass unchanged. Transcription then lands on the library.
3. **Stability is explicit per surface.** The CLI JSON v1 contract stays the
   stable public API. The library API is 0.x and unstable until declared
   otherwise. `cargo-semver-checks` joins CI when the facade is first published.
   A minimum-supported-Rust-version policy is set before that publication; the
   current MSRV equals the latest stable release.
4. **Segment-first evidence pipeline.**
   - P07–P09 process a source as ordered, immutable, identified segments; a
     finite file is a closed stream of one or more segments.
   - Transcript revisions, candidates and evidence references carry segment
     identity as well as normalized source time.
   - Live capture later adds a capture adapter that produces segments. The
     pipeline does not change, and ADR 0012's file-only R0 source profile is
     unaffected.
5. **The evidence contract is published as it is built.**
   - Each of P07, P08 and P09 adds its record schemas and conformance examples
     to `schemas/v1`, plus the bundle manifest fields it produces.
   - Evidence records are also available as a JSON Lines stream, so a pipeline
     or indexer can consume them without reading bundle files. P07 fixes the
     exact CLI surface through its contract tests.
6. **Verification others can see.**
   - A `cargo-fuzz` target lands with each parser of untrusted input: bundle
     validation, strict JSON input and FFprobe metadata now, SRT/VTT with P07.
     CI runs short fuzz passes; long campaigns remain P14 gates.
   - P08/P09 publish a reproducible benchmark over the F01–F12 ground truth:
     candidate recall against known events, timestamp error and throughput.
7. **Integration surfaces.**
   - The CLI plus agent skill remain the primary agent integration (maintainer
     decision, 2026-09-09).
   - MCP remains an optional thin adapter over the same library and contract,
     scheduled after P12 if wanted. It is local and single-user, not the
     multi-tenant service reserved for R2.
8. **Pre-releases.** After P09 the maintainer may cut 0.x pre-releases whose
   notes state the stability of each surface. R0 remains the first release
   that claims full qualification.

## Consequences

- P07 grows by one refactor increment, and later packets are cheaper: new
  features land once in the engine and every host inherits them.
- The workspace gains two crates. Dependencies still point inward, and the
  governance checker is unaffected.
- The architecture documents are updated in the refactor PR itself, once the
  facade exists.
- Index consumers, the desktop application and future adapters depend only on
  the facade and published schemas, never on infrastructure internals.
- Fuzz targets and the benchmark add CI time. Short fuzz passes are bounded;
  long runs are scheduled, not per-PR.

## 2026-09-23 implementation note: contract crate extracted

- The maintainer confirmed the crate names: `vsift` for the engine facade and
  `vsift-contract` for the wire contract. A crates.io availability check still
  precedes first publication.
- P07 increment 1a added `crates/vsift-contract`. It owns the v1 envelope,
  terminal event, setup check, setup plan, strict saved-plan input, session,
  bundle and frozen evidence-metadata types, plus the mapping into them from
  domain and application values. It depends on `vsift-domain`,
  `vsift-application`, `serde` and `serde_json` only.
- Values the contract cannot own without an infrastructure or host dependency
  are passed in typed form: the dependency lookup provenance, the bundle source
  inclusion and the clean outcome are contract enums mapped at the CLI edge, and
  RFC 3339 timestamps are formatted by the host, which owns the clock.
- The CLI contract and schema suites passed unchanged. The contract crate's own
  tests validate its values against `schemas/v1` and match the frozen examples.
- Increment 1b adds the `vsift` facade: engine use cases with injected clock and
  identifier ports, leaving `vsift-cli` a thin host.

## 2026-09-23 implementation note: engine facade extracted

- P07 increment 1b added `crates/vsift`, the embeddable engine. A host builds an
  `Engine` from an explicit `EngineConfig` (session-root location, per-user
  configuration location, host isolation) and `EnginePorts` (clock and identifier
  source). Construction performs no I/O; each location is resolved by the operation
  that first needs it, so every failure keeps its previous timing and code.
- Operations: `check_setup`, `plan_setup` with `EvaluatedSetupPlan::validate_acceptance`
  for saved-plan revalidation, `configure_executable`, `configure_model`, `ingest`,
  `list_sessions`, `session_status`, `renew_session`, `close_session`,
  `retain_session`, `clean_sessions` and `validate_bundle`. `verify_media_tools`
  (the P06 fixture verifier) and `identify_model` are exposed for later hosts and the
  P07 preflight; no CLI command calls them.
- Results are typed engine values (`SessionSnapshot`, `SessionPage`, `CleanPage`,
  `BundleSummary`, `SetupCheckReport`, `EvaluatedSetupPlan`) or re-exported
  application values. Every operation fails with one `EngineError` that keeps its
  typed cause; `EngineError::failure_code` is the single mapping to public codes.
  Infrastructure errors are mirrored, not re-exported.
- New application ports `Clock` and `IdentifierSource`, with `SystemClock` and
  `RandomIdentifierSource` in infrastructure. Session-root location and first-use
  provisioning moved from the CLI into infrastructure (`platform_session_root`,
  `open_session_root`).
- `vsift-cli` is now a thin host. It keeps clap parsing, effective-configuration
  precedence, presentation through `vsift-contract`, RFC 3339 formatting, exit codes,
  the bounded writer and reading the saved-plan file. Its normal dependencies are
  `vsift` and `vsift-contract` (plus clap, serde, serde_json, time and tokio); it no
  longer depends on `vsift-application`, `vsift-domain` or `vsift-infrastructure`.
  The unchanged opt-in P06 checkpoint test still uses application and infrastructure
  types, so both remain CLI development dependencies.
- The `vsift` library API is 0.x and unstable, as decision 3 states. The CLI binary
  target is excluded from rustdoc because it shares the `vsift` name with the library.
- No behaviour change: the CLI contract and schema suites passed unchanged, and a
  differential run of 86 CLI invocations against `00e707f` gave identical output and
  exit codes after normalizing random identifiers, timestamps and index placement.

## 2026-09-24 implementation note: supplied transcripts, the first evidence record

- P07 increment 2 imports SubRip and WebVTT sidecars (`ingest --transcript`,
  `--transcript-offset`) and pages them (`transcript get`). It follows decision 4: a
  finite file is one closed source segment with a content-derived `sgm_` identity, and
  every transcript revision (`trv_`) and segment (`tsg_`) carries source and
  source-segment identity beside normalized time, so live capture can add segments
  later without changing the records.
- Decision 5 in part: `transcript-segment.schema.json` is the first published
  evidence record, with `transcript-revision`, `transcript-get-data` and `ingest-data`
  schemas and frozen examples. The retained bundle gains the manifest artifact kind
  `transcript_record` (a versioned storage record, validated strictly on read). The
  JSON Lines evidence stream is still undecided: `--events jsonl` returns a page as
  one terminal event, and the per-record stream is left for a later P07 increment.
- Decision 6 in part: both parsers are bounded, hand-written and covered by
  `proptest` properties (arbitrary bytes, structured near-miss input, generated
  round trips). The `cargo-fuzz` targets need a nightly toolchain and are deferred to
  a separate decision; they are not in this increment.
- The engine's `ingest` now returns `IngestOutcome` (session plus optional revision)
  and takes an optional `SuppliedTranscriptRequest`; `Engine::transcript` pages a
  revision. The CLI keeps its thin-host shape and gained `CommandFailure`, which
  carries a typed fixed-prose remediation for transcript rejections.

## 2026-09-24 implementation note: the JSON Lines evidence stream

This note fixes the CLI surface decision 5 left to P07.

- `transcript get --events jsonl` writes one evidence event per segment of the page,
  then exactly one terminal event. `--json` and human output are unchanged, and every
  other command still writes its terminal event alone.
- An evidence event (`evidence-event.schema.json`) carries the envelope's version,
  `event: "evidence"`, a `sequence` that is contiguous from 0 across the whole stream,
  `command` and `operation_id`. It adds `record_type`, an upsert `key` and the `record`
  itself. For `transcript_segment` the key is the record's `segment_id` and the record
  is exactly the published transcript segment.
- The terminal event ends the stream. Its sequence equals the number of records, and
  its data (`transcript-get-stream-data.schema.json`) is the page without `items`:
  `session_id`, `revision`, `range`, `record_count` and `next_cursor`.
- A distinct terminal data schema was chosen over repeating `items` or sending an
  empty `items` array. Repeating would double the output. An empty array would let a
  reader that ignores the event kind conclude, silently, that the page was empty.
  This way such a reader fails validation instead.
- Records are immutable and their identities are derived from content, so upserting
  by (`record_type`, `key`) is idempotent. A stream is complete only once its terminal
  event is read; a gap in `sequence` or a missing terminal event means it is
  incomplete. No delete or tombstone events exist yet; they arrive with the first
  operation that supersedes evidence.
- No progress events: the read is local and bounded. Readers skip unknown event kinds
  within v1, so progress can be added later without a new major version.
- The whole stream is bounded by the page limit (at most 100 records plus the
  terminal event, each line within the 1 MiB result budget). It is assembled before
  its first byte is written, so a line over budget writes nothing.
- The sequencing lives in `vsift-contract` (`TranscriptEvidenceStream`), so every host
  emits the same stream. `EventKind::ALL` and `EvidenceRecordType::ALL` are guarded
  against drift from the published schemas, as `CommandName` and `FailureCode::ALL` are.
- The retained bundle's `transcript_record` artifact now has a published schema,
  `bundle-transcript-record.schema.json`, with a frozen example, and `bundle validate`
  decodes every transcript record strictly (see the ADR 0013 note of the same date).

## 2026-09-24 implementation note: one revision type for imports and local ASR

- P07 increment 3a keeps decision 4's single `TranscriptRevision` for both paths. Its
  provenance is either an import (format, sidecar identity, offset) or one local ASR
  run (provider and model digests, decoding profile, chunk plan, threads, audio
  stream, every chunk's outcome), and each segment carries its own origin, so the
  revision constructor re-derives every range on every read. Optional `supersedes`
  and `replaced_range` are reserved for retranscription; imports never set them.
- Imports are unaffected: their `trv_`/`tsg_`/`sgm_` identities and their version-1
  `transcript_record` bytes are pinned by tests against the pre-change importer.
  Local-ASR revisions use record version 2, which readers (including `bundle
  validate`) already decode strictly; no command writes it yet.
- No public command, schema or event kind changed. Retranscription, its schemas,
  the stream's treatment of superseded revisions and the next ADR follow in
  increment 3b.

## 2026-09-24 implementation note: cargo-fuzz targets (decision 6)

- Maintainer decision of 2026-09-24: `cargo-fuzz` runs on a pinned nightly toolchain
  (`nightly-2026-09-01`) in the scheduled `Fuzz` workflow, weekly and on manual
  dispatch (seconds per target, default 300, at most 1200), not on every pull request.
  The repository's stable toolchain and MSRV are unchanged. Every pull request instead
  runs `Fuzz harness replay` on stable: formatting, strict Clippy and the same target
  bodies over every committed seed. Long campaigns remain P14 gates.
- The harness is `fuzz/`, its own package with its own committed lockfile, excluded
  from the workspace. The libFuzzer entry points need its `libfuzzer` feature, so
  stable builds never compile libFuzzer. Each target body is a plain function that
  reports a broken invariant as a typed `Violation`; the entry point aborts on one,
  which libFuzzer records as a crash. Nothing in the harness panics or unwraps.
- Six targets, all through published APIs: `transcript_srt` and `transcript_webvtt`
  (`parse_supplied_transcript`: cue count, positive and ordered timing, bounded
  non-blank text, text never larger than the sidecar); `whisper_full_json`
  (`parse_whisper_full_json`, then the domain's chunk validation against a fixed 30 s
  chunk: segment and token bounds, segments inside the chunk); `transcript_record`
  (`decode_transcript_record`, versions 1 and 2, as `bundle validate` reads records:
  an accepted record re-encodes and decodes to the same revision);
  `ffprobe_metadata` (`parse_ffprobe_metadata` for both containers: positive
  duration, unique stream indexes, dimensions exactly on video streams); and
  `transcript_cursor` (`CursorToken::parse`, the untrusted `--cursor` value: an
  accepted token survives encode and parse).
- The one product change: the FFprobe metadata parser is now public as
  `vsift_infrastructure::parse_ffprobe_metadata`, beside the other pure provider
  parsers, because this decision names FFprobe metadata and the parser could not be
  reached without running FFprobe. Its behaviour is unchanged.
- Seeds are small and committed: copies of the existing transcript, whisper,
  bundle-record and F11 probe fixtures, the inline probe and cursor documents from
  existing tests and examples, and one version-2 record re-derived from the recorded
  F01 whisper output. The replay tests fail if a seed drifts from its origin.
- Not fuzzed yet, with the reason: the bundle manifest and metadata records and the
  storage ownership marker (private, decoded only inside capability-scoped directory
  reads; reachable publicly only through `validate_bundle` on a real directory tree);
  the media-tool verification record and the user dependency configuration (private,
  read only after ownership and link checks on private storage); the CLI's strict
  JSON request decoder (crate-private and unused until P11 admits request files); the
  FFmpeg `showinfo` diagnostic parser (private); and the managed-installation archive
  inventories (P13, and their input is digest-verified first). Each is a strict
  serde decoder plus field checks. Making them public only for fuzzing would widen
  the published API without a consumer, so they wait for a natural entry point or a
  filesystem-backed P14 harness.
- `unsafe`: no VSift crate changed, and the fuzz crate forbids `unsafe_code` too.
  `libfuzzer-sys` 0.4.13's `fuzz_target!` expands to two `#[no_mangle] extern "C"`
  functions and no `unsafe` block; the lint is not reported for code expanded from
  another crate's macro, so the entry points compile under `forbid` (a locally
  written `#[no_mangle]` is rejected, which confirms the lint is active). The FFI glue
  and libFuzzer's C++ runtime are inside `libfuzzer-sys`, like any dependency's
  internals. No ADR exception was needed.
- Dependencies: `libfuzzer-sys` 0.4.13 (rust-fuzz project, released 2026-06-04 after
  releases in February 2026 and July 2025; vendors LLVM libFuzzer) and `arbitrary`
  1.4.2 (MIT OR Apache-2.0), both only in the unpublished harness; `cargo-fuzz` 0.13.2
  (MIT OR Apache-2.0, released 2026-06-09) is installed in CI with `--locked`. `libfuzzer-sys`
  declares `(MIT OR Apache-2.0) AND NCSA`: its vendored libFuzzer files carry
  `Apache-2.0 WITH LLVM-exception` headers, but the declared NCSA term is not in
  `deny.toml`'s allow list. The maintainer approved (2026-09-25) an NCSA exception
  for `libfuzzer-sys` alone, and the `Dependency policy` job now also checks the fuzz
  lockfile; advisories, sources, bans and licences pass, and it adds no duplicate
  version.
