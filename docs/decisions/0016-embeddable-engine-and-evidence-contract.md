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
