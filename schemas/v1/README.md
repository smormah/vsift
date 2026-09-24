# VSift JSON schemas v1

These files are the machine-readable public v1 boundary:

- `setup-check-response.schema.json` — backward-compatible setup diagnosis;
- `setup-plan.schema.json` — current read-only reviewed-catalogue plan and
  typed managed-unavailable states; a digest never authorizes installation by
  itself;
- `setup-plan-unqualified.schema.json` — historical P06 check-first response
  before catalogue acceptance, retained for v1 compatibility evidence;
- `operation-response.schema.json` — terminal result for new operations;
- `terminal-event.schema.json` — JSONL terminal wrapper, the last line of every
  `--events jsonl` stream (validate its `result` with
  `operation-response.schema.json` too);
- `evidence-event.schema.json` — one evidence record line of an `--events jsonl`
  stream: `record_type`, upsert `key` and the `record` itself (P07; today
  `transcript_segment`, whose record is `transcript-segment.schema.json`);
- `config.schema.json` — strict explicit configuration document reserved for P06;
- `ingest-data.schema.json` — the `data` member of a complete `ingest` result; its
  optional `transcript` member is present only when a supplied transcript was
  imported (P07);
- `transcript-get-data.schema.json` — the `data` member of a complete
  `transcript.get` result: one bounded page with its continuation cursor (P07);
- `transcript-revision.schema.json` — one transcript revision: identities,
  alignment origin and offset, sidecar identity and typed import warnings (P07);
- `transcript-get-stream-data.schema.json` — the `data` member of the terminal
  event that ends a `transcript.get --events jsonl` stream: the page without its
  items, with `record_count` and the continuation cursor (P07);
- `transcript-segment.schema.json` — one transcript segment, the first published
  evidence record (ADR 0016): self-describing identities, normalized source time,
  sanitized text, confidence, alignment and cue provenance (P07);
- `bundle-transcript-record.schema.json` — the content of a retained bundle's
  `transcript_record` artifact: one revision with all its segments, as stored. It is
  a storage record, not a response (its `schema_version` is the integer `1`), and
  its text is untrusted and unsanitized (P07).

The `--events jsonl` stream is a sequence of events with a contiguous `sequence`
from 0: for `transcript.get`, one `evidence` event per segment and then one
`terminal` event; for every other command, the terminal event alone. Dispatch on
`event`; the terminal event's `sequence` and, for `transcript.get`, its
`record_count` equal the number of evidence events before it. The exact consumer
rules (upsert keys, end of stream, paging) are in the CLI contract.

Data schemas describe the `data` member only; validate the surrounding envelope with
`operation-response.schema.json`. They reference each other by relative `$ref`
(`transcript-revision.schema.json`, `transcript-segment.schema.json`), resolved
against their `$id` under `https://vsift.dev/schemas/v1/`; the identifiers are
names, not download locations, so validators map them to these local files. The
examples `ingest.transcript.json`, `transcript-get.json` and
`transcript-rejected.json` describe the F10 fixture imported from
`fixtures/corpus/transcripts/F10.srt` with a +500 ms offset, as do
`transcript-get.events.jsonl` (the first `transcript-get.json` page as a JSON Lines
stream: two evidence events and the terminal event, one JSON value per line, checked
byte for byte by `vsift-contract`'s `evidence_stream_contract`) and
`bundle-transcript-record.json` (the stored record of the whole revision, checked
by `vsift-infrastructure`'s `transcript_store` tests, which also validate a real
retained bundle's record).
`media-tool-verification-failed.json` is the `ingest` failure an agent receives when
the automatic media-tool preflight fails (here FFmpeg selected as FFprobe, stopped at
the probe check); it is checked by `vsift-contract`'s `media_tool_preflight_contract`.
`storage-not-private.json` is the `setup configure` failure an agent receives when
the per-user configuration folder already exists and other accounts can access it;
it is checked by `vsift-contract`'s `storage_contract` and by the CLI's Windows
`private_storage_cli_contract`.

The Rust types that produce these documents live in the `vsift-contract` crate
(`crates/vsift-contract`), which every VSift host uses so they all emit identical JSON.
Its `schema_conformance` tests validate serialized values against these schemas and
compare them with the examples. They also prove that every `FailureCode` identifier
is in the envelope's `error.code` enum, and that every `CommandName` identifier
(`setup.configure-model`, `transcript.get`, ...) satisfies the `command` pattern of
both envelope schemas; the CLI's tests prove `CommandName` matches its commands.
Likewise every `EventKind` (`evidence`, `terminal`) is the `event` constant of
exactly one event schema, and every `EvidenceRecordType` is in the evidence event's
`record_type` enum with a branch that fixes its record schema, and nothing else is.

Every file under `examples/` is a frozen valid instance checked by the Rust contract
suites in `vsift-contract` (`schema_conformance`, `transcript_contract`) and `vsift-cli`. Response readers must tolerate additive fields within major v1. Strict request
and configuration readers reject unknown fields. Unknown major versions are rejected.

See the human-readable [CLI contract](../../docs/contracts/cli-v1.md) for limits,
exit codes, lifecycle semantics, and implementation status.
