# VSift JSON schemas v1

These files are the machine-readable public v1 boundary:

- `setup-check-response.schema.json` — backward-compatible setup diagnosis. The
  additive `local_asr` object (P07 increment 3c) reports the registered model's
  identity (`not_selected`, `unreadable`, `unrecognised` or `known_pinned` with
  profile `base` or `base_q5_1`) and the local-ASR verification (`verified` from a
  `recorded` pass or `ran_now`, `failed` with a typed `check` and `reason`, or
  `not_run` with a `not_run_reason`); the constant `verification_scope` and
  `local_asr_model` fields are unchanged;
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
  alignment origin and offset, sidecar identity and typed warnings (P07). A
  local-ASR revision (origin `local_asr`, P07 increment 3b) has `sidecar: null` and
  adds `local_asr` (the run), `supersedes`, `replaced_range` and
  `carried_segment_count`; an import never has those members;
- `transcript-retranscribe-data.schema.json` — the `data` member of a complete
  `transcript.retranscribe` result: the new local-ASR revision, the requested range and
  how many segments the run recognised (P07 increment 3b);
- `transcript-get-stream-data.schema.json` — the `data` member of the terminal
  event that ends a `transcript.get --events jsonl` stream: the page without its
  items, with `record_count` and the continuation cursor (P07);
- `search-data.schema.json` — the `data` member of a complete or partial `search`
  result (P08): the matching segments as `items` (transcript segment records, in rank
  order), the parallel `hits` (`segment_id` and `match`: `phrase` or `all_terms`), the
  normalised `query.terms`, the requested `range`, the continuation cursor and
  `transcript_coverage` (basis, scope `transcript_text`, searched, transcribed,
  untranscribed and no-speech ranges). Its envelope `coverage` is `truncated`, and its
  status `partial`, exactly when part of the searched range has no transcript;
- `search-stream-data.schema.json` — the `data` member of the terminal event that ends
  a `search --events jsonl` stream: the page without its items, with `hits`,
  `record_count` and the cursor (P08);
- `transcript-segment.schema.json` — one transcript segment, the first published
  evidence record (ADR 0016): self-describing identities, normalized source time,
  sanitized text, confidence, alignment and cue provenance (P07). A local-ASR segment
  has alignment origin `local_asr` with its chunk, provider times and recognizer, and
  `cue: null`; a segment carried into a spliced revision adds `carried_from`;
- `bundle-transcript-record.schema.json` — the content of a retained bundle's
  `transcript_record` artifact: one revision with all its segments, as stored. It is
  a storage record, not a response: its `schema_version` is the integer record
  version, `1` for an import and `2` for a local-ASR revision (with its run, what it
  superseded, inherited provenance and carried segments), and its text is untrusted
  and unsanitized (P07).

The `--events jsonl` stream is a sequence of events with a contiguous `sequence`
from 0: for `transcript.get`, one `evidence` event per segment and then one
`terminal` event; for `search`, one `evidence` event per matching segment (the same
`transcript_segment` records, in rank order) and then one `terminal` event; for every
other command, the terminal event alone. Dispatch on `event`; the terminal event's
`sequence` and, for `transcript.get` and `search`, its `record_count` equal the number
of evidence events before it. The exact consumer
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
`transcript-retranscribe.json`, `transcript-get.asr.json` and
`bundle-transcript-record.asr.json` describe the F01 speech clip
(`fixtures/corpus/generated/F01-speech.mp4`): revision 1 transcribed the whole clip
with the reviewed whisper.cpp v1.9.2 build and pinned base model (from its recorded
output), and revision 2 retranscribed 5.5-6 s, where the clip is silent, so it
carries revision 1's segment (`carried_from`) and records a silent chunk and
`no_speech_recognised`. The first two are checked by `vsift-contract`'s
`local_asr_contract`, the record by `vsift-infrastructure`'s `local_asr_store`.
`setup-check.local-asr.json` is a ready `setup check` whose local-ASR check ran now
and passed with the pinned base model, and `setup-check.blocked.json` the check with
nothing installed (local ASR `not_run`, `media_tools_unavailable`); both are checked
by `vsift-contract`'s `setup_local_asr_contract` and `schema_conformance`, and the
blocked one against the binary's output by the CLI's `schema_contract`. Local-ASR
provenance names the model profile `base` or `base_q5_1` (P07 increment 3c).
`search.json` and `search.events.jsonl` search that F10 import for `R-17`: one phrase
hit, the segment F10-E01 cites, with complete coverage from the supplied transcript,
as a result and as a stream (one evidence event and the terminal event); both are
checked by `vsift-contract`'s `search_contract`, the stream byte for byte.
`search-stream-data.schema.json` references definitions of `search-data.schema.json`
by JSON pointer (`search-data.schema.json#/$defs/hit`).
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
