# VSift JSON schemas v1

These files are the machine-readable public v1 boundary:

- `setup-check-response.schema.json` â backward-compatible setup diagnosis. The
  additive `local_asr` object (P07 increment 3c) reports the registered model's
  identity (`not_selected`, `unreadable`, `unrecognised` or `known_pinned` with
  profile `base` or `base_q5_1`) and the local-ASR verification (`verified` from a
  `recorded` pass or `ran_now`, `failed` with a typed `check` and `reason`, or
  `not_run` with a `not_run_reason`); the constant `verification_scope` and
  `local_asr_model` fields are unchanged;
- `setup-plan.schema.json` â current read-only reviewed-catalogue plan and
  typed managed-unavailable states; a digest never authorizes installation by
  itself. Its availability for an accepted target is `catalogue_accepted` (renamed in
  place from `catalogue_accepted_install_pending` by P13 PR 4, before publication).
  Beside the digested intent it carries the observed state, which acceptance ignores:
  `readiness`, dependency and model statuses (`managed_current`), each action's
  `state` (`pending`, `current`) and `install_needed` (P13 PR 4);
- `setup-install.schema.json` â the `data` of a `setup.install` result (P13 PR 4):
  the catalogue revision, the source (`publisher` or `artifact_directory`) and every
  component of the accepted plan with its `status` (`activated`, `already_current`,
  `failed`), a failure's `step`, typed `reason`, `failure_code` and `smoke_check`, and
  what cleanup did with its stage. A failed install carries the same object as data
  beside its error (examples `setup-install.json` and `setup-install.failed.json`).
  The setup-check `lookup` gains `managed_version` and its remediation's
  `managed_install` takes the plan's availability values in the same change;
  since P13 PR 6 it also has `cleanup`: the stale stages swept before the transaction and
  each version the bounded cleanup after it handled;
- `setup-list.schema.json`, `setup-rollback.schema.json`, `setup-remove.schema.json` and
  `setup-repair.schema.json` — the `data` of the managed lifecycle commands (P13 PR 6):
  every component's selection and versions with whether each verifies; the version a
  rollback selected and the one it replaced; what a removal removed or kept (a failed
  removal carries the same object beside its error); and repair's read-only findings, each
  with its fix and the existing command that applies it (examples `setup-list.json`,
  `setup-rollback.json`, `setup-remove.json`, `setup-remove.failed.json`,
  `setup-repair.json`). The reserved-command examples `operation-error.json` and
  `terminal-event.json` (`setup.repair` answering `COMMAND_NOT_IMPLEMENTED`) stay as frozen
  v1 envelopes; since PR 6 no command answers that code;
- `setup-plan-unqualified.schema.json` â historical P06 check-first response
  before catalogue acceptance, retained for v1 compatibility evidence;
- `operation-response.schema.json` â terminal result for new operations;
- `terminal-event.schema.json` â JSONL terminal wrapper, the last line of every
  `--events jsonl` stream (validate its `result` with
  `operation-response.schema.json` too);
- `evidence-event.schema.json` â one evidence record line of an `--events jsonl`
  stream: `record_type`, upsert `key` and the `record` itself (P07:
  `transcript_segment`, whose record is `transcript-segment.schema.json`; P08:
  `visual_candidate`, keyed by `candidate_id`, whose record is
  `visual-candidate.schema.json`; P09: `frame_evidence` and `audio_evidence`, keyed by
  `evidence_id`, whose records are `frame-evidence.schema.json` and
  `audio-evidence.schema.json`);
- `progress-event.schema.json`, `lifecycle-event.schema.json` and
  `result-event.schema.json` â the P11 event kinds (ADR 0021): how far a long
  operation has come (`transcript.retranscribe` and `job.resume` since P11 PR 1; the
  worker commands later), a worker host's own state (started with its readiness,
  request admitted, waiting or finished, draining, stopped), and one worker request's
  `job-result`. Every string member is an enum or a bounded pattern and every line is
  at most 64 KiB;
- `job-request.schema.json` â one strict worker request (`job run --request`, one line
  of `job batch --requests`): operation id, durability, optional deadline, an ingest
  or session target and at most 8 steps; paths relative to the operator's input root.
  The decoder (`vsift_contract::decode_work_request`) is authoritative and refuses
  more than the schema can say (`.`/`..` names, trailing dots or spaces, device names,
  the step order);
- `job-result.schema.json` â the answer to one request: derived status, replay flag,
  attempt, session, source, publication, lifecycle, the steps with typed outputs,
  coverage and failures, the request's failure and the controls it ran under
  (isolation, admission capacity, concurrency and, since P11 PR 2, where the resource
  limits come from and whether the free-space reserve was checked); at most 64 KiB, no
  path and no evidence text;
- `job-batch-data.schema.json` â the `data` of a `job.batch` result: counts per status,
  one item per processed line, the first line not started and why the batch stopped;
- `workspace-data.schema.json` â the `data` of `session.init-workspace` (implemented
  in P11 PR 2): the immutable policy of a worker workspace;
- `handoff-check-data.schema.json` â the `data` of a complete `handoff.check` result
  (P13 PR 5): `valid`, `handoff_version`, the `errors`, `warnings` and `case_notes`
  findings (each a pointer, a line, a closed `rule`, the schema's `allowed` values and
  fixed prose, never text from the draft), `truncated` and, with `--session`, how the
  session was used (`resolved`, a `gap`, `identities_checked`);
- `config.schema.json` â strict explicit configuration document reserved for P06;
- `ingest-data.schema.json` â the `data` member of a complete `ingest` result; its
  optional `transcript` member is present only when a supplied transcript was
  imported (P07);
- `transcript-get-data.schema.json` â the `data` member of a complete
  `transcript.get` result: one bounded page with its continuation cursor (P07);
- `transcript-revision.schema.json` â one transcript revision: identities,
  alignment origin and offset, sidecar identity and typed warnings (P07). A
  local-ASR revision (origin `local_asr`, P07 increment 3b) has `sidecar: null` and
  adds `local_asr` (the run), `supersedes`, `replaced_range` and
  `carried_segment_count`; an import never has those members;
- `transcript-retranscribe-data.schema.json` â the `data` member of a complete
  `transcript.retranscribe` result: the new local-ASR revision, the requested range,
  how many segments the run recognised (P07 increment 3b) and `job`, the recoverable
  job behind it (`job_id`, `resumed`, `chunks_reused`, `replayed`; P10 PR 2). Its
  envelope names the `operation_id` the result is recorded under;
- `job-data.schema.json` â the `data` member of a complete `job.status` or `job.cancel`
  result (P10 PR 3): one recoverable job's `job_id`, `session_id`, `kind`, `state`,
  `live_owner`, `resumable` and `resumable_reason`, the `operation_id` a retry should
  carry, `request.range`, `progress` (`chunks_total`, `chunks_checkpointed`),
  `attempts`, `result` (`revision_id`, `generation`) and `failure` (`code`,
  `retryable`). It never names a path, transcript text or provider output;
- `job-resume-data.schema.json` â the `data` member of a complete `job.resume` result
  (P10 PR 3): `job` (job data after the run) and `outcome` (exactly the
  `transcript.retranscribe` data);
- `transcript-get-stream-data.schema.json` â the `data` member of the terminal
  event that ends a `transcript.get --events jsonl` stream: the page without its
  items, with `record_count` and the continuation cursor (P07);
- `search-data.schema.json` â the `data` member of a complete or partial `search`
  result (P08): the matching segments as `items` (transcript segment records, in rank
  order), the parallel `hits` (`segment_id` and `match`: `phrase` or `all_terms`), the
  normalised `query.terms`, the requested `range`, the continuation cursor and
  `transcript_coverage` (basis, scope `transcript_text`, searched, transcribed,
  untranscribed and no-speech ranges). Its envelope `coverage` is `truncated`, and its
  status `partial`, exactly when part of the searched range has no transcript;
- `search-stream-data.schema.json` â the `data` member of the terminal event that ends
  a `search --events jsonl` stream: the page without its items, with `hits`,
  `record_count` and the cursor (P08);
- `candidates-data.schema.json` â the `data` member of a complete or partial
  `candidates` result (P08 PR 4): the requested `range`, the `index` revision and its
  fixed sampling grid, `coverage` (the searched range, the `analyzed` ranges and every
  typed gap: `not_analyzed`, `deadline_exceeded`, `undecodable`, `no_decoded_frame`,
  `candidate_budget_exhausted`), the candidates as `items` in time order and the
  continuation cursor. Its envelope `coverage` is `truncated`, and its status
  `partial`, exactly when there is a gap;
- `candidates-stream-data.schema.json` â the `data` member of the terminal event that
  ends a `candidates --events jsonl` stream: the page without its items, with
  `record_count` (P08);
- `visual-candidate.schema.json` â one visual candidate, the second published evidence
  record (P08): identities, window, the actual decoded frame time
  `representative_us`, `span`, `change_window`, `reasons`, `stability`, the
  uncalibrated `change` sizes, `visual_hash`, `sample_count`, displayed dimensions
  and the analysis profile;
- `frame-data.schema.json` â the `data` member of a complete or partial `frame.get`,
  `frame.neighbours`, `frame.burst` (P09 PR 3) or `crop` (PR 4) result: the `operation`, its canonical
  `request`, `request_key`, `reused`, `profile`, `tool_fingerprint`, `source_check`, the
  `selections` (role, `evidence_id`, requested and actual time and their signed
  delta), the operation's `neighbours` side stops or `burst` plan, the items, the
  delivered `files` (`evidence_id`, `media_type` and the absolute path of the committed
  session artifact, valid while the session exists) and `partial_reason`. Its status is
  `partial` exactly when `partial_reason` is set;
- `frame-stream-data.schema.json` â the `data` member of the terminal event that ends a
  frame command's `--events jsonl` stream: the result without its items, with
  `record_count` (P09 PR 3);
- `frame-evidence.schema.json` â one frame or crop item, the third published evidence
  record (P09 PR 3): `evidence_id`, `source_id`, `stream_index`, `kind`, the displayed
  `frame` (timestamp, time base, time and size), `crop` (`null` for a whole frame),
  `image` (`image/png`, size, SHA-256 and bytes), `profile` and `tool_fingerprint`.
  Request facts and paths are never in it, so one key always carries one record;
- `audio-data.schema.json` â the `data` member of a complete or partial `audio` result
  (P09 PR 4): the requested range, `request_key`, `reused`, `source_check`, one
  `requested` selection (requested start, first decoded sample, delta),
  `range_clipped`, the one `audio_evidence` item, the delivered `files` and
  `partial_reason`;
- `audio-stream-data.schema.json` â the terminal data of an `audio --events jsonl`
  stream: the result without its item, with `record_count` (P09 PR 4);
- `audio-evidence.schema.json` â one audio clip, the fourth published evidence record
  (P09 PR 4): `evidence_id`, `source_id`, `stream_index`, the clipped `range`,
  `actual_start_us` and `audio` (`audio/wav`, 16 kHz, mono, `s16le`, SHA-256 and bytes);
- `bundle-visual-index-record.schema.json` â the content of a session or retained
  bundle's `visual_index_record` artifact: one revision of the visual index with every
  recorded window and candidate, as stored (P08). A storage record, not a response:
  its `schema_version` is the integer record version `1`;
- `bundle-evidence-record.schema.json` â the content of a session or retained bundle's
  `evidence_record` artifact: the lineage of one evidence call (request key and
  parameters, selections with requested and actual time, items with their media
  digests, provider fingerprint and source check, and the partial reason), as stored
  (P09 PR 2). A storage record, not a response: its `schema_version` is the integer
  record version `1`; it never holds a path;
- `transcript-segment.schema.json` â one transcript segment, the first published
  evidence record (ADR 0016): self-describing identities, normalized source time,
  sanitized text, confidence, alignment and cue provenance (P07). A local-ASR segment
  has alignment origin `local_asr` with its chunk, provider times and recognizer, and
  `cue: null`; a segment carried into a spliced revision adds `carried_from`. The
  additive `display_text` (2026-09-29) is `text` with every hidden character (Unicode
  `Cf`, `Default_Ignorable_Code_Point`, U+2028 and U+2029) written as `<U+XXXX>`, the
  form to quote; `text` and `original_text` keep the characters raw. A speaker object
  adds `display_label` by the same rule;
- `bundle-transcript-record.schema.json` â the content of a retained bundle's
  `transcript_record` artifact: one revision with all its segments, as stored. It is
  a storage record, not a response: its `schema_version` is the integer record
  version, `1` for an import and `2` for a local-ASR revision (with its run, what it
  superseded, inherited provenance and carried segments), and its text is untrusted
  and unsanitized (P07).

The `--events jsonl` stream is a sequence of events with a contiguous `sequence`
from 0: for `transcript.get`, one `evidence` event per segment and then one
`terminal` event; for `search`, one `evidence` event per matching segment (the same
`transcript_segment` records, in rank order) and then one `terminal` event; for
`candidates`, one `evidence` event per candidate (`visual_candidate` records, in time
order) and then one `terminal` event; for `frame.get`, `frame.neighbours`,
`frame.burst` and `crop`, one `evidence` event per item (`frame_evidence` records, in the
result's item order) and then one `terminal` event; for `audio`, one `audio_evidence`
event and then one `terminal` event; for `transcript.retranscribe` and `job.resume`,
`progress` events (P11) and then one `terminal` event; for every other command, the
terminal event alone. Dispatch on `event`; the terminal event's `sequence` and, for `transcript.get`,
`search`, `candidates` and the frame commands, its `record_count` equal the number of evidence events
before it. The exact consumer
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
`parse-failure.json` (P13 PR 1, L-071) is the failure of a rejected command line,
`vsift crop <session> <evidence> --rect 10 --json`: `command` `parse`,
`INVALID_ARGUMENT` and one remediation whose summary carries the rejection
(`invalid_value`) and names only the grammar's `--rect` and `crop`, with the help
`vsift crop --help` as its `command`; the CLI's `schema_contract` checks it against the
binary's output.
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
`job-status.json`, `job-cancel.json`, `job-resume.json` and
`retranscribe-cancelled.json` (P10 PR 3) describe the job of the F01 retranscription of
`transcript-retranscribe.json`: its status after a Ctrl-C stopped a run (interrupted,
resumable, last failure `CANCELLED`), a `job cancel` that came too late (succeeded,
warning `cancellation_too_late`), the `job resume` that committed revision 2 from its
checkpoint (the job and the retranscription), and the interrupted run's failure
(`CANCELLED`, the session and job in `affected_ids`, a remediation whose `command` is
`vsift job resume <job>`). All four are checked by `vsift-contract`'s
`local_asr_contract`. `session status` (whose data has no separate schema) adds, since
P10 PR 3, `jobs` (at most 16 of `job_id`, `kind`, `state`, `live_owner`, `resumable`,
`resumable_reason`, the members of `job-data.schema.json` with the same meaning) and
`jobs_truncated`.
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
by JSON pointer (`search-data.schema.json#/$defs/hit`), and
`candidates-stream-data.schema.json` those of `candidates-data.schema.json`.
`candidates.json` and `candidates.events.jsonl` page the F02 fixture (three slides,
A-B-A) indexed from the samples FFmpeg 9.0 decoded from it
(`crates/vsift-infrastructure/tests/data/visual_samples/F02.json`): four candidates
(first frame, two slide changes and the periodic sample of the last slide), complete
coverage, as a result and as a stream; `candidates.partial.json` pages a synthetic
150 s video whose first window is analysed, second undecodable and third not analysed
yet, so it is `partial` with both gaps. All three are checked by `vsift-contract`'s
`candidates_contract`, the stream byte for byte. `bundle-visual-index-record.json` is
F02's stored index record, checked by `vsift-infrastructure`'s `visual_index_store`
tests, which also validate the records of a real retained bundle.
`bundle-evidence-record.json` is the record of a `frame get` at 1.025 s over a fake
20 fps stream, checked by `vsift-infrastructure`'s `evidence_store` tests, which also
validate every operation's record and the evidence of retained bundles.
`frame-get.json` and `frame-get.events.jsonl` are `frame get --at 1025000` over F01 (the
frame at 1.05 s, delta 25,000 us), as a result and as a stream; `frame-neighbours.json`
takes two neighbours of F01's first frame (the before side stops at
`start_of_stream`); `frame-burst.partial.json` bursts 4-8 s of the 6 s video with 12
targets (clipped to 4-6 s) in a session with room for only four more images, so it is
`partial` with `session_evidence_budget`. `crop.json` crops the status line
(150,235,550,55) of that 1.05 s frame, and `audio.json` clips 0-1 s of the source, whose
first decoded sample is 64 ms in (as in F01's audio-only variant). They are built by the application's
extraction use cases over a stand-in of F01's streams (its real 1/10240 time base and
1280x720), with stand-in image and clip bytes and a synthetic provider fingerprint, and checked
by `vsift-contract`'s `navigation_contract`, the stream byte for byte. Their delivered paths
are written under the placeholder root `/vsift-session-root` (with the real
`sessions/<session>/artifacts/artifact-<sha256>.png` or `.wav` layout below it), never a real
user's folder; a real result names the absolute path on the machine that ran it.
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
Likewise every `EventKind` (`evidence`, `terminal`, `progress`, `lifecycle`, `result`)
is the `event` constant of exactly one event schema, and every `EvidenceRecordType` is
in the evidence event's `record_type` enum with a branch that fixes its record schema,
and nothing else is.

The P11 worker examples (checked by `vsift-contract`'s `worker_contract` and
`worker_events_contract`): `job-request.json` ingests F01 into a durable workspace
and retranscribes it, pages its candidates and retains it as `f01-review`;
`job-run.json` is its complete `job.run` result and `job-run.replayed.json` the same
result replayed by operation id; `job-run.partial.json` is a candidates request over
a 150 s range with an undecodable minute, so it is `partial`. Since P11 PR 3 `job run`
emits these results: a `failed` or `cancelled` `job.run` response carries the job
result as its `data` beside its `error`, and a result whose record could not be
written is presented as `STORAGE_IO` with the result as data.
`job-batch.requests.jsonl` holds two requests (an F10 ingest with its supplied
transcript, then candidates and close; a 5.5-6 s retranscription of an existing F01
session and close); `job-batch.events.jsonl` is their `--events jsonl` batch (started,
two admissions, the retranscription's chunk progress, a result and a
`request_finished` per request, stopped, then the terminal event, byte for byte) and
`job-batch.json` its `--json` summary. `workspace-init.json` initialises a durable
workspace with capacity 8 and the default 168-hour retention.
`transcript-retranscribe.events.jsonl` is the `--events jsonl` form of
`transcript-retranscribe.json`: the job's chunk progress (0 of 1, 1 of 1), then that
result as the terminal event (checked by `local_asr_contract`).

Every file under `examples/` is a frozen valid instance checked by the Rust contract
suites in `vsift-contract` (`schema_conformance`, `transcript_contract`) and `vsift-cli`. Response readers must tolerate additive fields within major v1. Strict request
and configuration readers reject unknown fields. Unknown major versions are rejected.

See the human-readable [CLI contract](../../docs/contracts/cli-v1.md) for limits,
exit codes, lifecycle semantics, and implementation status.
