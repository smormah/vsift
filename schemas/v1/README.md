# VSift JSON schemas v1

These files are the machine-readable public v1 boundary:

- `setup-check-response.schema.json` — backward-compatible setup diagnosis;
- `setup-plan.schema.json` — current read-only reviewed-catalogue plan and
  typed managed-unavailable states; a digest never authorizes installation by
  itself;
- `setup-plan-unqualified.schema.json` — historical P06 check-first response
  before catalogue acceptance, retained for v1 compatibility evidence;
- `operation-response.schema.json` — terminal result for new operations;
- `terminal-event.schema.json` — JSONL terminal wrapper (validate its `result` with
  `operation-response.schema.json` too);
- `config.schema.json` — strict explicit configuration document reserved for P06;
- `ingest-data.schema.json` — the `data` member of a complete `ingest` result; its
  optional `transcript` member is present only when a supplied transcript was
  imported (P07);
- `transcript-get-data.schema.json` — the `data` member of a complete
  `transcript.get` result: one bounded page with its continuation cursor (P07);
- `transcript-revision.schema.json` — one transcript revision: identities,
  alignment origin and offset, sidecar identity and typed import warnings (P07);
- `transcript-segment.schema.json` — one transcript segment, the first published
  evidence record (ADR 0016): self-describing identities, normalized source time,
  sanitized text, confidence, alignment and cue provenance (P07).

Data schemas describe the `data` member only; validate the surrounding envelope with
`operation-response.schema.json`. They reference each other by relative `$ref`
(`transcript-revision.schema.json`, `transcript-segment.schema.json`), resolved
against their `$id` under `https://vsift.dev/schemas/v1/`; the identifiers are
names, not download locations, so validators map them to these local files. The
examples `ingest.transcript.json`, `transcript-get.json` and
`transcript-rejected.json` describe the F10 fixture imported from
`fixtures/corpus/transcripts/F10.srt` with a +500 ms offset.

The Rust types that produce these documents live in the `vsift-contract` crate
(`crates/vsift-contract`), which every VSift host uses so they all emit identical JSON.
Its `schema_conformance` tests validate serialized values against these schemas and
compare them with the examples.

Every file under `examples/` is a frozen valid instance checked by the Rust contract
suites in `vsift-contract` (`schema_conformance`, `transcript_contract`) and `vsift-cli`. Response readers must tolerate additive fields within major v1. Strict request
and configuration readers reject unknown fields. Unknown major versions are rejected.

See the human-readable [CLI contract](../../docs/contracts/cli-v1.md) for limits,
exit codes, lifecycle semantics, and implementation status.
