# CLI and JSON contract v1

Status: published v1 boundary. `setup check/plan/configure/configure-model`, foreground `ingest`
(including supplied-transcript import), the P05 `session` lifecycle, `transcript get`,
`transcript retranscribe` (local speech recognition), `search` (P08 transcript search),
`candidates` (P08 visual candidates) and `bundle validate` are operational. Other commands below
remain reserved and return `COMMAND_NOT_IMPLEMENTED` with exit 2. Reserving a
command does not claim its media, provisioning, or worker behavior is implemented.

## Command namespace

Global output options are `--json` for one terminal JSON document and
`--events jsonl` for a JSON Lines stream. They are mutually exclusive.
`--session-root <absolute-dir>` explicitly selects a private disposable
workspace; otherwise P05 uses the per-user application cache. The first command
that needs the root creates it. Commands that race to create it converge on one
root: the others wait at most five seconds for the creator and use the root only
after the full ownership and privacy checks, failing with `BUSY` if it is still
being created. An existing directory that VSift did not create is never adopted.

| Command | Contract purpose | Implementation packet |
| --- | --- | --- |
| `setup check` | Read-only dependency diagnosis | Implemented |
| `setup configure` | Persist an explicit user-managed executable path without running it | Partial P06 |
| `setup configure-model` | Persist an explicit user-managed model file path without parsing it | Partial P06 |
| `setup plan` | Read-only diagnosis plus exact reviewed Ubuntu 24.04 x86-64 catalogue actions and digest; manual guidance on unaccepted targets | Partial P06 |
| `setup install/repair/list/remove/rollback` | Explicit managed dependency lifecycle, still reserved | P13 ([ADR 0015](../decisions/0015-r0-delivery-replan.md)) |
| `ingest` | Open a disposable source-bound session; optionally import a supplied SRT/WebVTT transcript | Implemented in P05; transcript import in P07 increment 2 |
| `session list/status/close/renew/retain/clean` | Session and retention lifecycle | Implemented in P05 |
| `transcript get` | Bounded, pageable timestamped transcript segments | Implemented in P07 increment 2 |
| `transcript retranscribe` | New local-ASR transcript revision, whole source or one range | Implemented in P07 increment 3b |
| `search` | Bounded, ranked literal transcript search with honest coverage | Implemented in P08 PR 1 |
| `candidates` | Bounded visual-candidate retrieval with honest coverage; analyses missing windows first | Implemented in P08 PR 4 |
| `frame get/neighbours/burst`, `audio`, `crop` | Source-grounded evidence extraction | P09 |
| `bundle validate` | Bounded data-only bundle validation | Implemented in P05 |
| `job run/batch/status/resume/cancel` | Recoverable worker operations | P10/P11 |

### P05 disposable sessions and bundles

```console
vsift ingest ./recording.mp4 --json
vsift session list --json
vsift session status ses_0123456789abcdef --json
vsift session renew ses_0123456789abcdef --json
vsift session retain ses_0123456789abcdef --output ./evidence --json
vsift session retain ses_0123456789abcdef --output ./portable --include-source --json
vsift bundle validate ./portable --json
vsift session close ses_0123456789abcdef --json
vsift session clean --expired --dry-run --json
vsift session clean --expired --json
```

`ingest` stages and hashes one local source, returning its session/source IDs,
source bytes, committed generation, `process_crash_consistent` publication and
an RFC 3339 expiry. Without `--transcript` it does not start FFmpeg, setup,
transcription or indexing; supplied-transcript import is described in the next
section.
Default sessions expire after 24 idle hours; renewals cannot extend beyond seven
days from open. Close and cleanup return busy while active work holds the
session. Expiry becomes visible at the wall-clock boundary, but physical
cleanup requires a later `clean` invocation; no daemon or secure deletion is
promised.

`session list [--cursor 0..255]` and `session clean --expired
[--cursor 0..255] [--dry-run]` return one bounded hash-bucket page at a time.
`next_cursor` continues the scan; repeat until it is null. A page has at most
256 registrations. An initializing registration is visible as such. A corrupt
or busy item is reported with an item error and a partial page rather than
silently omitted or used as deletion authority.

`retain` requires a new absolute output directory and never overwrites one.
The output is owner-private and stays outside automatic cleanup. Both bundle
forms contain committed artifact bytes and a bounded v1 `bundle.json` written
last. Evidence-only exports omit the source and state that matching original
bytes are required for later re-extraction. `--include-source` copies and
verifies the private snapshot; it never moves or deletes the original. An
interrupted export can leave an incomplete private selected directory, which
`bundle validate` rejects. The retained lifecycle does not upgrade the
qualified publication guarantee; see [ADR 0013](../decisions/0013-retained-bundle-publication.md).

### P07 supplied transcripts

```console
vsift ingest ./recording.mp4 --transcript ./recording.srt --transcript-offset 500000 --json
vsift ingest ./recording.mp4 --transcript ./recording.vtt --transcript-offset=-250000 --json
vsift transcript get ses_0123456789abcdef --from 5000000 --to 9000000 --json
vsift transcript get ses_0123456789abcdef --from 0 --to 12000000 --limit 50 --cursor "<next_cursor>" --json
```

`ingest --transcript <file>` imports one `SubRip` (`.srt`) or `WebVTT` (`.vtt`)
sidecar with the new session. `--transcript-offset <signed microseconds>` (default
0, at most 24 hours either way, only with `--transcript`) is added to every sidecar
timestamp to reach source time; it is recorded with the revision and with every
segment. Use `--transcript-offset=-N` or `--transcript-offset -N` for a negative
offset. The format is detected from content: a `WEBVTT` signature selects WebVTT,
anything else must be valid SubRip.

Import never needs whisper.cpp or model weights (ADR 0014). It does need FFmpeg and
FFprobe, resolved like `setup check` (a `setup configure` selection first, then the
filtered `PATH`), because the staged video is probed with the P04 adapter to learn its
duration. Everything that can fail without touching the session root runs first:
offset bounds, reading and parsing the sidecar, locating the tools and the automatic
media-tool preflight described below. Then the
source is staged and probed, the cues are aligned, and the source binding and the
transcript revision are committed in **one** generation. A rejected import therefore
never leaves an open session. When the rejection comes after staging (the probe or
the alignment failed), the unactivated registration and its private source copy stay
in the owned root, listed as `initializing`, until `session clean` removes them as
abandoned after the idle interval. On success `data.transcript` (schema
[`transcript-revision.schema.json`](../../schemas/v1/transcript-revision.schema.json))
describes the revision; a plain ingest omits the member, so its output is unchanged.
The revision is stored as a `transcript_record` session artifact, counted by
`session status` and copied into retained bundles. Its content is published as
[`bundle-transcript-record.schema.json`](../../schemas/v1/bundle-transcript-record.schema.json)
(frozen example [`bundle-transcript-record.json`](../../schemas/v1/examples/bundle-transcript-record.json)).
`bundle validate` checks every transcript record's size and digest and then decodes
it strictly with the same rules as a session read (unknown fields or values, broken
import invariants such as a segment not at its cue timing plus the offset, or a
record naming another source): a non-conforming record fails the bundle with
`INTEGRITY_FAILURE`, and a newer record version with `UNSUPPORTED_SCHEMA`. Imports
write record version 1, the version the published schema describes. Version 2 is
reserved for revisions produced by local speech recognition; the reader already
decodes it with the same strictness (its run provenance and every segment's
provider times must reproduce the stored ranges, and every carried segment must match
its inherited provenance). `transcript retranscribe` writes it, and the bundle schema
describes both versions. Versions above 2 are `UNSUPPORTED_SCHEMA`.

**Malformed-data policy.** A rejection is a typed failure; nothing is imported.

| Input | Policy | Code |
| --- | --- | --- |
| File over 8 MiB, line over 4,096 bytes, cue text over 4,096 bytes, more than 20,000 timed cues, or a stored revision record over 24 MiB | Rejected | `RESOURCE_LIMIT` |
| UTF-8 byte-order mark | Removed, then parsed | none |
| UTF-16/UTF-32 byte-order mark, invalid UTF-8 | Rejected; text is never lossily repaired | `INVALID_SOURCE` |
| LF, CRLF or CR line endings | Accepted and equivalent | none |
| Control characters other than tab (including C1 and U+2028/U+2029) | Rejected at their line | `INVALID_SOURCE` |
| Malformed timestamp (SubRip `H:MM:SS,mmm`, WebVTT `[HH:]MM:SS.mmm`), minutes or seconds over 59, missing `-->`, text after a SubRip end time, missing SubRip cue number, bad `WEBVTT` signature | Rejected at their line | `INVALID_SOURCE` |
| Cue ending at or before its start | Rejected | `INVALID_SOURCE` |
| Cue starting before the previous cue | Rejected (`out_of_order`); cues are never re-sorted | `INVALID_SOURCE` |
| Overlapping cues | Kept, each with its own timing; warning `overlapping_cues` | none |
| Text without cue timing, or a SubRip cue continued after a blank line | Rejected (`untimed_text`): untimed text cannot support a timestamp citation | `INVALID_SOURCE` |
| Timed cue with no text | Skipped; warning `empty_cues_skipped` | none |
| Recognised markup (WebVTT spans, timestamps and character references; SubRip `<i>`/`<b>`/`<u>`/`<font>` and `{\...}` blocks) | Removed from `text`; the payload as written is kept in `original_text`; warning `markup_removed` | none |
| WebVTT `<v Name>` naming exactly one voice | Provider speaker label (`imported_webvtt_voice`); an invalid or ambiguous voice is dropped with warning `speaker_label_discarded`. SubRip `Name:` prefixes stay text. | none |
| WebVTT `NOTE`, `STYLE`, `REGION` blocks and cue settings | Ignored (no cue text) | none |
| WebVTT `Language:` header with a well-formed tag | Revision language | none |
| No timed cue with text | Rejected (`no_cues`) | `INVALID_SOURCE` |

**Offset and alignment policy (T-02).** Only cues that lie wholly inside the probed
source timeline `[0, duration]` after the offset are imported. A cue entirely before
zero or after the end is left out with warning `cues_outside_source`; a cue crossing
either boundary is left out with warning `cues_crossing_source_boundary`. No cue is
ever clamped, trimmed or shifted, because that would cite text at a time it was not
written for. If nothing remains, the import is rejected with `no_cues_within_source`
(`INVALID_ARGUMENT`: check the offset and that the transcript belongs to the video).
An offset beyond 24 hours is `offset_out_of_range` (`INVALID_ARGUMENT`). Each warning
reports its count and first cue ordinal, and says whether it excluded cues; the
envelope `warnings` carry fixed prose for each kind. A result with warnings is still
`complete` (exit 0).

Rejections carry one `remediation` item with a fixed-prose summary naming the typed
reason and, where known, the line, for example `The supplied transcript was rejected
(invalid_timestamp) at line 6. ...`. It never contains transcript text, suggests no
command and requires no authority. See
[`transcript-rejected.json`](../../schemas/v1/examples/transcript-rejected.json). A
missing FFmpeg/FFprobe is `MISSING_CAPABILITY` with a remediation explaining that
import needs them but not Whisper; an unreadable sidecar is `STORAGE_IO`, and a
non-regular or unsafe sidecar path is `INVALID_SOURCE`, as for source media.

**Automatic media-tool preflight.** Before the first media stage of an operation that
runs FFmpeg/FFprobe on user media (today `ingest --transcript` and `transcript
retranscribe`; later frames and audio use the same hook), VSift proves the resolved pair works by running a
small reviewed test video built into VSift (F01) through the same probe, frame,
audio and visual-sampling steps an investigation uses and comparing each result with its known answers
(ADR 0015). It runs after the sidecar is parsed and the tools are located, and before
the session root is created or touched, so a failure writes nothing. There is no
separate command. Plain `ingest`, `setup` commands and `transcript get` never run it,
and it never needs Whisper or a model.

The first media operation with a given pair takes about 1–2 seconds longer (measured
on Windows 11 with FFmpeg 9.0: 2.5 s for the first F10 import, 0.7 s for the next).
A pass is recorded and reused while all of these stay the same: the canonical paths
of both executables and their size and modification time (plus device, inode, mode,
owner and change time on Unix, creation time and attributes on Windows), the
reviewed compatibility policy and test-video digest, the adapter profile, the
verification profile and the VSift version. Reinstalling, upgrading or reselecting
either tool therefore triggers one new check, and every pass ages out after seven
days. Executable contents are not hashed: hashing two ~100 MiB static builds would
cost every operation what the record saves, and would still miss shared libraries.

The record is `media-tool-verification/verified-v1.json` inside the private per-user
VSift directory that also holds `setup configure` selections (`%LOCALAPPDATA%\vsift`
on Windows, `~/Library/Application Support/vsift` on macOS,
`${XDG_CONFIG_HOME:-~/.config}/vsift` elsewhere). It holds at most eight
`{fingerprint, verified_at_unix_seconds}` entries of SHA-256 digests and times, never
paths, tool output or media, and is at most 4 KiB. The same directory is the parent of
the short-lived private workspace each check creates and removes. A check that is
killed cannot remove its workspace, so every preflight removes leftovers: only
directories named exactly `vsift-tool-verification-<16 hex>`, unchanged for at least
an hour and not locked by a running check, at most eight at a time, never following
links and never touching anything else. The record is an
optimisation, never an authority: a missing, corrupt, oversized, linked or
foreign-version record reads as "not verified", the check runs, and the record is
replaced. Writers use a non-blocking lock; a process that finds it held verifies
without recording rather than waiting. Failures are never recorded, and no record
problem can fail an operation. Deleting the directory is always safe.

A failed check is a failed operation (`status: "failed"`, `data: null`) with one
`remediation` item (`required_authority: "none"`, `command: null`) whose summary
begins with the fixed sentence `The selected FFmpeg and FFprobe failed VSift's
media-tool check at the <check> step (<reason>).`, where `<check>` is `preparation`,
`probe`, `frame`, `audio` or `visual_sampling` (verification profile 2, P08; profile 3, P09, adds frame listing, exact-timestamp extraction and a crop to the `frame` check) and `<reason>` is `process_failure`, `provider_rejected`,
`output_limit`, `unexpected_result`, `deadline`, `workspace`, `cancelled` or
`fixture_integrity`. Fixed prose for the reason and the next step follows; no path or
tool output is ever included. See
[`media-tool-verification-failed.json`](../../schemas/v1/examples/media-tool-verification-failed.json).

| Reason | Code | Next step in the remediation |
| --- | --- | --- |
| `process_failure`, `provider_rejected`, `output_limit`, `unexpected_result` | `MISSING_CAPABILITY` | Reinstall FFmpeg/FFprobe from a trusted build or register a working pair with `setup configure ffmpeg\|ffprobe --executable <path>`, then retry |
| `deadline` | `DEADLINE_EXCEEDED` | Retry when the machine is less busy; otherwise register a different pair |
| `workspace` (no private place to run the check) | `STORAGE_IO` | Make the per-user VSift directory private, writable and not full, then retry |
| `cancelled` | `CANCELLED` | Retry |
| `fixture_integrity` (the built-in test video is corrupt) | `INTERNAL` | Reinstall VSift |

The executable probes of `setup check` stay fast (`verification_scope:
executable_probe_only`), so FFmpeg selected as FFprobe passes those probes. `setup
check` runs this preflight only as part of its local-ASR verification (below), when
whisper.cpp and a reviewed model are also registered; otherwise the first media
command reports the failure at the `probe` step.

**Retrieval.** `transcript get <session> --from <us> --to <us> [--limit 1..100]
[--cursor <token>] [--revision <trv_id>]` returns the segments of the session's newest
revision (or, with `--revision`, of that revision) that
intersect the half-open range (a segment that started earlier but is still running
is included), in start order, 20 per page by default. `data`
([`transcript-get-data.schema.json`](../../schemas/v1/transcript-get-data.schema.json))
holds the revision summary, the range, the `items` and `next_cursor`. Each item is a
self-contained transcript evidence record
([`transcript-segment.schema.json`](../../schemas/v1/transcript-segment.schema.json)):
segment, revision, source and source-segment identities, `start_us`/`end_us`,
sanitized `text` (lines joined with `\n`), `original_text` when markup was removed,
`markup`, `speaker`, `confidence` (always `null` with origin `unavailable` for
imported text), `language`, `alignment` (origin, offset and the cue timing as
written) and `cue` (its ordinal and line in the file). `--limit` and `--cursor` are
additions to the reserved grammar, needed because overlapping cues make time-based
continuation lose or repeat segments. Pass `next_cursor` back with the same session
and range; it is bound to the revision (not the storage generation, so a renewal
keeps it valid), the range and the session expiry, and any other use is
`INVALID_ARGUMENT`. A session without a transcript, a closed or expired session, an
empty range, or a `--revision` the session does not hold is `INVALID_ARGUMENT`; the
first carries a fixed remediation naming `ingest --transcript` and `transcript
retranscribe`, the last one naming where revision identities come from. A malformed
`--revision` is a parse error. `transcript get` never runs a provider. With `--events
jsonl` the page is streamed as one evidence event per segment followed by one
terminal event (below).

**Evidence stream (`--events jsonl`).** ADR 0016 decision 5 makes evidence records
available as JSON Lines, so a pipeline or indexer can consume them without reading a
page or a bundle. `vsift transcript get <session> --from <us> --to <us> [--limit]
[--cursor] --events jsonl` writes, in order:

1. one **evidence event** per segment of the page, in start order
   ([`evidence-event.schema.json`](../../schemas/v1/evidence-event.schema.json));
2. exactly one **terminal event**
   ([`terminal-event.schema.json`](../../schemas/v1/terminal-event.schema.json)) whose
   complete `result.data` is the page without its items
   ([`transcript-get-stream-data.schema.json`](../../schemas/v1/transcript-get-stream-data.schema.json)):
   `session_id`, `revision`, `range`, `record_count` and `next_cursor`.

Every line is one complete JSON object and a newline, with `schema_version: "1"`, its
`event` kind, `sequence` (0, 1, 2, ... across the whole stream; the terminal event's
`sequence` equals `record_count`), `command` and `operation_id`. An evidence event adds
`record_type` (`transcript_segment`), `key` (the upsert key; for a transcript segment,
its `segment_id`) and `record`, which is exactly the transcript segment of
[`transcript-segment.schema.json`](../../schemas/v1/transcript-segment.schema.json) that
`--json` returns in `items`. The first page of F10 with `--limit 2`, from the frozen
example [`transcript-get.events.jsonl`](../../schemas/v1/examples/transcript-get.events.jsonl)
(records shortened here with `...`):

```json
{"schema_version":"1","event":"evidence","sequence":0,"command":"transcript.get","operation_id":null,"record_type":"transcript_segment","key":"tsg_e88330d57e140d6e7e2ed4447950c5b8","record":{"segment_id":"tsg_e88330d57e140d6e7e2ed4447950c5b8","revision_id":"trv_663ae41bbedc740b651fe61999d395b0","start_us":1000000,"end_us":4000000,"text":"This synthetic sidecar is aligned with\nan explicit 500 millisecond offset.",...}}
{"schema_version":"1","event":"evidence","sequence":1,"command":"transcript.get","operation_id":null,"record_type":"transcript_segment","key":"tsg_f9644c630cd9023d38e2eb89741c9b7e","record":{"segment_id":"tsg_f9644c630cd9023d38e2eb89741c9b7e","revision_id":"trv_663ae41bbedc740b651fe61999d395b0","start_us":5000000,"end_us":9000000,"text":"Dialog R-17 is displayed now.",...}}
{"schema_version":"1","event":"terminal","sequence":2,"command":"transcript.get","operation_id":null,"result":{"schema_version":"1","command":"transcript.get","operation_id":null,"status":"complete","data":{"next_cursor":"v1|ses_0123456789abcdef0123456789abcdef|1|06ff...9b04|2|1790294400000000","range":{"from_us":0,"to_us":12000000},"record_count":2,"revision":{...},"session_id":"ses_0123456789abcdef0123456789abcdef"},"warnings":[],"error":null,"coverage":null,"lifecycle":{"mode":"ephemeral","expires_at":"2026-09-25T00:00:00Z"}}}
```

How a consumer reads it:

- **Upsert by key.** Evidence records are immutable and their identities are derived
  from content (the session, the revision and the segment ordinal), so the same
  (`record_type`, `key`) always carries the same record. Upserting by it is idempotent:
  re-reading a page, overlapping pages or a retry never duplicate evidence. A new
  revision of the same transcript has new keys, including the segments it carries
  unchanged from an older revision (they name their origin in `carried_from`).
  VSift emits no delete or tombstone events (ADR 0017): a superseded revision is not
  deleted evidence, its records stay valid, immutable and readable with
  `--revision`, and a consumer that wants only one revision filters on
  `record.revision_id` against the terminal event's `revision.revision_id`.
- **End of stream.** The stream is complete only when its terminal event has been
  read. Its `result.status` says whether the operation succeeded, and on success
  `record_count` must equal the number of evidence events received and the terminal
  `sequence`; a gap in `sequence` or a missing terminal event (for example a closed
  pipe) means the stream is incomplete. Records already upserted remain valid because
  they are immutable.
- **Paging.** `next_cursor` in the terminal data continues the stream: pass it back
  with `--cursor` and the same session and range; `null` means the range is exhausted.
  Cursor rules are those of `--json`.
- **Failures and empty pages.** A request that fails (bad range, cursor or session,
  no transcript) writes only one terminal failure event with `sequence: 0`, exactly as
  before. A valid range that no segment intersects writes one complete terminal event
  with `record_count: 0`.
- **Unknown events.** Within v1 new event kinds (such as progress) may appear before
  the terminal event. Dispatch on `event`, skip a kind you do not know and still count
  its `sequence`. `transcript get` emits no progress events: the read is bounded and
  local.

The stream is bounded by the page limit: at most `--limit` evidence events (1 to 100,
default 20) and one terminal event, each line within the 1 MiB result budget. The
whole stream is assembled before its first byte is written, so a line over budget
fails the command like an oversized `--json` result: exit 7 and a diagnostic on
stderr, with nothing on stdout. `--json` and human output are unchanged: one result with the page's `items`.

### P07 local speech recognition

```console
vsift ingest ./recording.mp4 --json
vsift transcript retranscribe ses_0123456789abcdef --json
vsift transcript retranscribe ses_0123456789abcdef --from 12000000 --to 18000000 --json
vsift transcript get ses_0123456789abcdef --from 0 --to 30000000 --json
vsift transcript get ses_0123456789abcdef --from 0 --to 30000000 --revision trv_0123456789abcdef --json
```

`transcript retranscribe <session> [--from <us> --to <us>]` transcribes the session's
speech locally with whisper.cpp and commits a new transcript revision
([ADR 0017](../decisions/0017-local-asr-through-whisper-cpp.md)). Give both range
flags or neither; neither transcribes the whole video. It is the only command that
runs speech recognition: `ingest`, with or without `--transcript`, and `transcript
get` never look for whisper.cpp or a model.

It needs FFmpeg, FFprobe and `whisper-cli`, resolved like `setup check` (a `setup
configure` selection first, then the filtered `PATH`), and a model registered with
`setup configure-model`. Only a model identified by size and SHA-256 as a reviewed
pinned profile runs, and its identity decides the profile (no configuration field
does): `base`, the multilingual whisper.cpp base model (`ggml-base.bin`, 147,951,465
bytes, the default), or `base_q5_1`, its 5-bit quantization (`ggml-base-q5_1.bin`,
59,707,625 bytes, SHA-256 `422f1ae4…a8898`, from the same Hugging Face repository at
revision `5359861`, a pin the maintainer accepted on 2026-09-25). Any other file is refused before any work with
`MISSING_CAPABILITY`. The profile is recorded in every revision's `local_asr`
provenance (`model_profile`). The order is part of the contract: the range is checked, the
tools and model are resolved and identified, the session is checked (it must be open
and unexpired), the media-tool preflight runs, then the **local-ASR preflight**, and
only then is the session's committed copy of the video decoded. The local-ASR
preflight transcribes a short reviewed speech clip built into VSift (F01, "The
service status is healthy and the build is 2048.") with the selected recognizer and
model and requires those words inside the clip's speech window; a pass is recorded
like the media-tool pass (same private record, its own fingerprint over the tools,
recognizer and model identities, profile and VSift version) and reused for seven
days. The first run with a new setup therefore takes several seconds longer.

Chunk audio is written only to a private work directory inside the session, removed
when the run ends; while the run works, `session close` and `session clean` return
`BUSY`. The video is decoded in 30-second chunks overlapping by 5 seconds; silent
chunks are recorded and skipped; times are anchored at each chunk's first decoded
sample; text heard twice where chunks overlap is kept once. Each chunk may take at
most 120 seconds, and a run at most 1,024 chunks.

**Revisions.** Every run commits one new, immutable, complete revision, numbered after
the newest, which becomes the default for `transcript get`. With a range, and an
earlier revision, the range is first widened to whole segments of the newest revision
(every segment it cuts, and every segment overlapping those), recorded as
`replaced_range`. Segments outside it are carried with their original text, timing,
confidence and provenance, under new `segment_id`s and with `carried_from` naming the
revision and segment that first produced them; the recognised segments fill the range.
Every earlier revision stays readable with `transcript get --revision`, so an older
citation always resolves. A run that recognises no speech still commits a revision
(no new segment, warning `no_speech_recognised`) so the attempt is on record.

`data`
([`transcript-retranscribe-data.schema.json`](../../schemas/v1/transcript-retranscribe-data.schema.json),
example [`transcript-retranscribe.json`](../../schemas/v1/examples/transcript-retranscribe.json))
holds `session_id`, `requested_range` (null for the whole video), the new `revision`
and `recognised_segment_count`. A local-ASR revision summary has `alignment.origin`
`local_asr`, `sidecar: null`, `local_asr` (provider and model by SHA-256, the pinned
profile, decoding profile `r0-v1`, chunk plan, threads, audio stream, covered range and
chunk counts), `supersedes`, `replaced_range` and `carried_segment_count`. A local-ASR
segment has `alignment` with its chunk, the chunk's decoded audio range, the provider's
chunk-relative times, whether the end was trimmed, and the recognizer; `cue` is `null`
and confidence is the mean token probability, `provider_uncalibrated` (example
[`transcript-get.asr.json`](../../schemas/v1/examples/transcript-get.asr.json)).
Warnings use the envelope's fixed prose and the revision's typed codes; for local-ASR
codes `first_cue` is the first affected chunk. Segments are read with `transcript get`
(the new revision is the default). With `--events jsonl`, `transcript retranscribe`
writes its terminal event only; stream the records with `transcript get --revision
<revision_id> --events jsonl`, page by page. The command-line host does not trap
Ctrl-C: interrupting it commits nothing, and the work directory is removed by the
session's next run or cleanup.

**Failures** use existing codes. A failed run carries one fixed-prose remediation
starting `Local speech recognition failed at the <stage> step (<reason>).`, a failed
preflight one starting `VSift's local speech-recognition check ... failed (<kind>)`;
neither contains a path, provider output or transcript text.

| Condition | Code |
| --- | --- |
| FFmpeg, FFprobe or whisper-cli not found; no model registered; model unreadable; model not a reviewed pinned profile; whisper-cli failed, produced unparseable or malformed output (times outside its audio, invalid scores) or could not start; the model or executable changed during the run; the preflight transcript missed its words | `MISSING_CAPABILITY` |
| Empty or reversed range; range past the end of the video; video with no decodable audio stream; closed or expired session | `INVALID_ARGUMENT` |
| Audio stream present but undecodable | `INVALID_SOURCE` |
| Output over its bounds, more than 1,024 chunks, a record over 24 MiB, or whisper-cli ended abnormally (usually out of memory) | `RESOURCE_LIMIT` |
| A chunk exceeded its 120 s deadline | `DEADLINE_EXCEEDED` |
| Cancelled (library hosts) | `CANCELLED` |
| Work directory or the session's copy of the video unusable | `STORAGE_IO` |
| The session changed during the run (a renewal or another revision), is held by cleanup, or every processing slot of the session root is in use; retry | `BUSY` |

### P08 transcript search

```console
vsift search ses_0123456789abcdef --query "R-17" --json
vsift search ses_0123456789abcdef --query "dialog r 17" --from 0 --to 30000000 --limit 50 --json
vsift search ses_0123456789abcdef --query "invoice 4407" --revision trv_0123456789abcdef --events jsonl
```

`search <session> --query <text> [--from <us> --to <us>] [--limit 1..100] [--cursor
<token>] [--revision <trv_id>]` finds a literal query in the segments of the session's
newest transcript revision, or of the revision `--revision` names
([ADR 0018](../decisions/0018-visual-candidate-index-and-transcript-search.md)). The
query is data only, never a pattern or regular expression; a query that starts with a
hyphen is written `--query=-17`. `--from` and `--to` go together and restrict the search
to segments intersecting the half-open range; without them the whole revision is
searched. It is computed from the immutable revision on each call, never runs a
provider and writes nothing. It searches transcript text only, never on-screen text.

**Matching.** Query and segment text are normalised the same way: lowercase; a comma
thousands separator is removed (`2,048` is `2048`); a hyphen between letters or digits
joins them (`E-409` is `e409`); a colon between digits separates numbers (`10:32` is
`10 32`); a decimal compares by value (`125.00` is `125`); `zero` to `twenty` and the
tens are digits; any other punctuation separates words. There is no Unicode
normalisation or accent folding. A segment matches as a `phrase` when the query words
joined without spaces equal consecutive segment words joined without spaces (`AB 731`
finds `AB-731`, `dialog r 17` finds `Dialog R-17`, `407` never finds `4407`), or by
`all_terms` when every query word is one of its words. A phrase that continues into the
next segment is not found. Hits are ranked phrase first, then by segment start. A query
is at most 256 bytes and 1 to 16 words; an empty query, a longer one, more words or a
control character (tab and newline included) is `INVALID_ARGUMENT` with one fixed
remediation `The search query was rejected (<reason>). ...`, where `<reason>` is `empty`,
`too_long`, `too_many_terms` or `control_character`; the query is never repeated.

**Result.** `data` ([`search-data.schema.json`](../../schemas/v1/search-data.schema.json),
example [`search.json`](../../schemas/v1/examples/search.json)) holds `session_id`, the
searched `revision` summary, `query.terms` (the normalised words), `range` (null for
the whole revision), `items`, `hits`, `next_cursor` and `transcript_coverage`. `items`
are the matching segments as published transcript evidence records, exactly as
`transcript get` returns them, in rank order; `hits` lists each item's `segment_id`
and `match` (`phrase` or `all_terms`) in the same order. 20 hits per page by default,
1 to 100 with `--limit`.

**Coverage.** A search can only find words a transcript holds, so every result says
what it could not see. `transcript_coverage` holds the `basis` (`supplied_transcript`:
a supplied file, taken to cover the whole video but not verified complete; `local_asr`:
what local recognition examined; `mixed`: local recognition spliced into supplied
text), `scope: "transcript_text"`, the `searched_range` (the request clipped to the
video, null when wholly outside it), and `transcribed_ranges`, `untranscribed_ranges`
and `no_speech_ranges` (transcribed parts where recognition found no audible signal or
no audio), each merged, in start order and at most 100 long (`ranges_truncated` says
when one was cut). The envelope `coverage` member, frozen since v1 was published, is
filled by `search`: `truncated` is true exactly when part of the searched range has no
transcript, `gaps` lists those parts as `"<from_us>-<to_us>"` (merged, at most 100)
and `reasons` holds distinct identifiers (`untranscribed_range`, plus
`gap_list_truncated` beyond 100 gaps). Such a result has status `partial`, the fixed
warning `Part of the searched range has no transcript, so words said there cannot be
found; ...`, and exits 0 like any supported partial result. A complete search has
`coverage: {"truncated": false, "gaps": [], "reasons": []}`.

**Paging.** Pass `next_cursor` back with the same session, query (any spelling that
normalises to the same words), range and revision. It is bound to the revision, the
normalised query and range, the rank position of the page's last hit and the session
expiry when it was issued; any other use is `INVALID_ARGUMENT`. Pages never repeat or
skip a hit.

**Evidence stream.** `search --events jsonl` writes one evidence event per hit, in rank
order, whose `record_type` is `transcript_segment` and whose record and key are exactly
those `transcript get` streams, so an indexer upserts records it may already hold, then
one terminal event whose data
([`search-stream-data.schema.json`](../../schemas/v1/search-stream-data.schema.json),
example [`search.events.jsonl`](../../schemas/v1/examples/search.events.jsonl)) is the
page without its items: `hits`, `record_count`, `transcript_coverage`, the cursor and
the rest, with the same `status` and envelope `coverage` as `--json`. Stream rules are
those of `transcript get` (above). The F10 example (records shortened with `...`):

```json
{"schema_version":"1","event":"evidence","sequence":0,"command":"search","operation_id":null,"record_type":"transcript_segment","key":"tsg_f9644c630cd9023d38e2eb89741c9b7e","record":{"segment_id":"tsg_f9644c630cd9023d38e2eb89741c9b7e","start_us":5000000,"end_us":9000000,"text":"Dialog R-17 is displayed now.",...}}
{"schema_version":"1","event":"terminal","sequence":1,"command":"search","operation_id":null,"result":{"schema_version":"1","command":"search","operation_id":null,"status":"complete","data":{"hits":[{"match":"phrase","segment_id":"tsg_f9644c630cd9023d38e2eb89741c9b7e"}],"next_cursor":null,"query":{"terms":["r17"]},"range":null,"record_count":1,"revision":{...},"session_id":"ses_0123456789abcdef0123456789abcdef","transcript_coverage":{"basis":"supplied_transcript","scope":"transcript_text",...}},"warnings":[],"error":null,"coverage":{"truncated":false,"gaps":[],"reasons":[]},"lifecycle":{"mode":"ephemeral","expires_at":"2026-09-25T00:00:00Z"}}}
```

**Failures** use existing codes: `INVALID_ARGUMENT` for a rejected query, an empty
range, a rejected cursor, a `--revision` the session does not hold (the remediation
naming where revision identities come from), a session without a transcript (the
remediation naming `ingest --transcript` and `transcript retranscribe`) and a closed or
expired session; `--limit 0`, `--limit 101`, a lone `--from` or `--to` and a malformed
`--revision` are parse errors. A stored record that fails verification is
`INTEGRITY_FAILURE` or `UNSUPPORTED_SCHEMA`, as for every read.

### P08 visual candidates

```console
vsift candidates ses_0123456789abcdef --from 0 --to 12000000 --json
vsift candidates ses_0123456789abcdef --from 0 --to 1800000000 --limit 100 --json
vsift candidates ses_0123456789abcdef --from 60000000 --to 120000000 --events jsonl
```

`candidates <session> --from <us> --to <us> [--limit 1..100] [--cursor <token>]` pages
the visual candidates of the session's video whose representative time lies in the
half-open range, 20 per page by default
([ADR 0018](../decisions/0018-visual-candidate-index-and-transcript-search.md),
decisions 9-14). A candidate is a moment at which the screen changed, or a periodic
sample of a screen that did not, proposed so an agent need not look at every frame; it
is a shortlist, not evidence, and never an image (P09 `frame get` extracts the frame).
A range that runs past the end of the video is clipped to it; one that starts at or
after the end is `INVALID_ARGUMENT`.

**Analysis inside the call.** The video is cut into fixed 60 s windows. A call first
analyses the missing windows of its range in ascending order, at most 30 (30 minutes of
video) per call, then commits them as a new revision of the session's visual index;
the remaining windows are reported as `not_analyzed` and the next call for the range
continues them. Analysis decodes actual frames at most twice a second as 128x72 grey
samples that are never kept, runs the automatic FFmpeg/FFprobe preflight first (now
including a `visual_sampling` check), and verifies the session's source copy before
committing. A range whose windows are all analysed, and every call with `--cursor`, is a
warm read: no tool is needed or run and nothing is written. On Windows 11 with FFmpeg
9.0, analysis ran at 29 media seconds per second on 1440x900 20 fps video and 67 on
640x360 10 fps video; a warm page took about 150 ms through the binary on a 30-minute
session, and about 100 ms in the engine on the largest index a session can hold
([recall record](../planning/p08-candidate-recall.md)).

**Result.** `data` ([`candidates-data.schema.json`](../../schemas/v1/candidates-data.schema.json),
example [`candidates.json`](../../schemas/v1/examples/candidates.json)) holds
`session_id`, the requested `range`, `index` (the revision `index_id`, its `number`,
`profile` `r0-visual-v1`, the video's `duration_us` and the fixed `window_us`,
`sample_interval_us` and `coverage_interval_us`), `coverage`, `items` and
`next_cursor`. Each item is a published `visual_candidate` evidence record
([`visual-candidate.schema.json`](../../schemas/v1/visual-candidate.schema.json)):
`candidate_id`, `source_id`, `source_segment_id`, `stream_index`, its 60 s `window`,
`representative_us` (the actual decoded frame's time), the `span` of time it stands
for, `change_window` (the change happened after `from_us` and at or before `to_us`; null
for a first frame or periodic coverage), `reasons` (`first_frame`, `visual_change`,
`motion_start`, `settled_after_motion`, `periodic_coverage`), `stability` (`settled`,
`transient`, `in_motion`, `open_at_window_end`), `change` (`changed_blocks`,
`max_block_delta`: uncalibrated integers for ordering only, never a confidence),
`visual_hash` (a screen that reappears later is a separate candidate with the same
hash), `sample_count`, `displayed_dimensions` and `analysis`. Every 10 s of analysed
video with a decoded frame has a candidate.

**Coverage.** `coverage.searched_range` is the range clipped to the video;
`coverage.analyzed` lists the analysed parts (merged); `coverage.gaps` lists every gap
with its `reason` and `dropped_candidates`: `not_analyzed` (call again),
`deadline_exceeded` (the window this call timed out on; call again), `undecodable` (the
media tools could not decode it; not retried), `no_decoded_frame` (a 10 s cell with no
frame) and `candidate_budget_exhausted` (a window that changed more often than its 32
candidates, with the number dropped); `ranges_truncated` says a list was cut at 100.
The envelope `coverage` is `truncated` exactly when there is a gap, `gaps` lists them
merged as `"<from_us>-<to_us>"` (at most 100) and `reasons` the distinct gap reasons (and
`gap_list_truncated`). Such a result has status `partial`, the fixed warning `Part of
the requested range has no visual candidates because it is not analysed yet or could
not be analysed; ...`, and exits 0
([`candidates.partial.json`](../../schemas/v1/examples/candidates.partial.json)).

**Paging.** Pass `next_cursor` back with the same session and range. It is bound to the
source, stream, profile and range, the state of every window the range touches, the
page's last candidate and the session expiry when it was issued. Analysing windows
outside the range leaves it valid; once a later call analyses a window inside the
range, or for any other session or range, after expiry or when forged, it is
`INVALID_ARGUMENT` rather than a silent restart. Pages never repeat or skip a
candidate; the page size may change between pages.

**Evidence stream.** `candidates --events jsonl` writes one evidence event per candidate,
in time order, with `record_type` `visual_candidate` and key `candidate_id`, then one
terminal event whose data
([`candidates-stream-data.schema.json`](../../schemas/v1/candidates-stream-data.schema.json),
example [`candidates.events.jsonl`](../../schemas/v1/examples/candidates.events.jsonl))
is the page without its items plus `record_count`, with the same `status` and envelope
`coverage` as `--json`. Stream rules are those of `transcript get` (above).

**Storage.** Each call that analyses anything commits one `visual_index_record`
artifact (strict versioned JSON, at most 8 MiB, at most 64 per session; the shape is
[`bundle-visual-index-record.schema.json`](../../schemas/v1/bundle-visual-index-record.schema.json)).
It is removed with the session and carried by `session retain`; `bundle validate`
re-checks every record's windows, spans, change rule, coverage and identities.

**Failures** use existing codes: `INVALID_ARGUMENT` for an empty or reversed range, a
range starting at or after the video's end, a rejected cursor, a cursor for a session
with no visual candidates yet (remediation: run without `--cursor` first), a video with
no video stream (remediation naming `transcript get` and `search`) and a closed or
expired session; `--limit 0`, `--limit 101` and a missing `--from` or `--to` are parse
errors. `INVALID_SOURCE` when no video stream can be decoded or the probe rejects the
copy. `MISSING_CAPABILITY` when FFmpeg or FFprobe is missing (only when windows must be
analysed; the remediation says Whisper is not needed), the preflight fails or FFmpeg
cannot run on a window. `DEADLINE_EXCEEDED`, `BUSY` or `CANCELLED` only when the call
analysed nothing and nothing of the range was analysed before; otherwise the result is
`partial`. `RESOURCE_LIMIT` for a record over 8 MiB or a 65th index record.
`INTEGRITY_FAILURE` or `UNSUPPORTED_SCHEMA` for a stored record that fails verification
or a source copy that changed during the call.

Running `vsift` or `vsift setup` without a leaf command prints help and performs no
dependency probe or mutation. `setup check` defaults to the `desktop` profile and a
five-second total operation deadline; `--profile worker` and
`--timeout-seconds 1..60` are explicit overrides. `--ffmpeg`, `--ffprobe` and
`--whisper` select existing absolute executables for this check only. An invalid
explicit selection does not fall back to `PATH` or persist configuration.

P06 will distinguish an already suitable provider, an installable missing provider,
and one requiring user-managed installation or explicit configuration. `setup plan`
is read-only; `setup install` requires acceptance of its unchanged plan. On missing
rights, offline/unqualified target or failed installation, headless calls return
typed bounded manual remediation and never prompt, elevate or silently retry with
broader authority. An agent's request to inspect a video does not authorize setup.
The first P06 increment checks explicit paths or filtered `PATH` and provides
typed manual/BYO remediation for missing, unhealthy and timed-out tools. The legacy aggregate
`status` reflects only executable probe results. `verification_scope` is
`executable_probe_only`, and `local_asr_model` is `not_checked`: a successful
`--version`/`--help` response does **not** prove provider compatibility or a
working transcription model. Both fields keep these constant values for v1
compatibility; the additive `local_asr` object below reports the model and a real
transcription check. Verified managed installation remains P13 work.

**Local ASR in `setup check`** (P07 increment 3c, maintainer decision D4). The
response carries a `local_asr` object:

```json
"local_asr": {
  "model": {"status": "known_pinned", "profile": "base"},
  "verification": {"status": "verified", "source": "ran_now", "not_run_reason": null, "check": null, "reason": null}
}
```

- `model.status` is `not_selected` (nothing registered with `setup configure-model`),
  `unreadable`, `unrecognised` (readable but not a reviewed pin) or `known_pinned`,
  with `profile` `base` or `base_q5_1` (null otherwise). The file is identified by
  size and SHA-256 on every check; its path is never shown.
- `verification.status` is `verified`, `failed` or `not_run`. Every detail field is
  present and null unless it applies. `verified` has `source` `recorded` (a
  still-valid pass for exactly this setup is on record; nothing ran) or `ran_now`.
  `failed` has `check` (`preparation`, `fixture_probe`, a transcription stage —
  `planning`, `recognizer_identity`, `audio_extraction`, `recognition`,
  `output_validation`, `assembly` — `transcript` or `budget`) and `reason` (the
  local-ASR verification's typed reason, such as `fixture_media`,
  `unexpected_transcript`, `deadline` or `abnormal_termination`, or
  `budget_exceeded`). `not_run` has `not_run_reason`, the first that applies in the
  order a retranscription resolves its dependencies: `media_tools_unavailable`
  (FFmpeg or FFprobe missing, rejected or failing its own check),
  `whisper_unavailable`, `model_not_selected`, `model_not_pinned`.
- When no pass is recorded and everything is present, `setup check` runs the same
  media-tool and local-ASR preflights as `transcript retranscribe` (the built-in F01
  speech clip, with the tools selected for this check: per call, configured, then
  `PATH`) within its own **60-second budget**, separate from `--timeout-seconds`. A
  run past the budget is stopped and reported as `failed` / `budget` /
  `budget_exceeded`. It writes only the per-user verification record, and only for a
  pass, so the next check or retranscription with the same setup reuses it for up to
  seven days. It never creates a session.
- The exit status still reflects only the executable probes: a local-ASR check that
  did not pass is reported, not a failed `setup check`, because media inspection and
  supplied transcripts need no speech recognition.

Schema [`setup-check-response.schema.json`](../../schemas/v1/setup-check-response.schema.json)
(the object is optional for responses from earlier builds); examples
[`setup-check.local-asr.json`](../../schemas/v1/examples/setup-check.local-asr.json)
and [`setup-check.blocked.json`](../../schemas/v1/examples/setup-check.blocked.json). Provider `detail` is not an instruction
channel. Paths are not echoed in the response. `detail` for FFmpeg and FFprobe is
only their `ffmpeg version ...` / `ffprobe version ...` banner line, or `detected`
when none is safe to show; whisper output is never echoed, and its `detail` is
`whisper.cpp v1.9.2 (reviewed build)` when the executable is byte-identical to a
build reviewed in P06, otherwise `whisper-cli (build not recognised)`. No line that
looks like a path or a ggml loader log is ever shown.

`setup plan --profile <desktop|worker>` probes configured executables or filtered
`PATH` like `setup check`, without per-call path options. On **Ubuntu 24.04
x86-64** it consults the reviewed, pinned catalogue for the paired FFmpeg and
FFprobe build, whisper.cpp CLI and multilingual `base` model. It returns only
actions needed by the current probe and configured-model presence, with exact
publisher URL, sizes, SHA-256, archive inventory, installed files, licence and
notice/source links, known trust limits, private destination and permissions.
Its lowercase SHA-256 `plan_digest` binds profile, exact target, catalogue
revision, current probe results, configured selections, all planned actions and
the revision's reviewed compatibility policy. For the current Ubuntu revision,
that policy fixes the exact F01 fixture identity, expected FFmpeg/FFprobe build
identities, per-stream and generated-file limits, media and inference deadlines,
and 16-kHz mono audio contract. The policy remains an installer safety constraint rather than a claim
that compatibility execution has passed.
The same unchanged observations produce the same digest. Any changed catalogue
or observed selection requires a fresh plan and acceptance. No configured paths
are echoed in the response. The plan is **read-only**: `setup install` still
returns `COMMAND_NOT_IMPLEMENTED` and cannot apply it yet. The availability
value `catalogue_accepted_install_pending` states that distinction explicitly.
For a readable `--plan` document, the reserved install path now decodes the
complete strict response within the JSON byte/nesting budgets, rebuilds the
current plan, requires the saved presentation to match it exactly, and checks
`--accept-plan` against the current digest before returning the reserved-command
result. Malformed, unknown-field, changed-state or mismatched-digest documents
fail with `INVALID_ARGUMENT` before any transfer or managed-root mutation. An
unreadable plan retains the reserved `COMMAND_NOT_IMPLEMENTED` behavior until the
installer can provide its complete storage-failure contract.

Windows x86-64, macOS ARM64, other hosts and expired or invalid catalogue
entries return typed `unavailable_*` status, no actions or digest, and manual
BYO guidance. Missing, unhealthy and timed-out tools receive
`manual_selection_required` unless a reviewed managed action applies. A
responding executable remains `existing_executable_probe_only`; a configured
model file remains `configured_model_probe_only`. Neither response proves
provider or model compatibility. `readiness` remains the executable-probe
aggregate. The model status in a plan is presence-only and is separate from
the still-unverified `setup check` model status. The new strict response schema
is [`setup-plan.schema.json`](../../schemas/v1/setup-plan.schema.json); the
older [`unqualified` example](../../schemas/v1/examples/setup-plan.unqualified.json)
is retained as historical v1 evidence. An abbreviated current response follows.

```json
{
  "schema_version": "1",
  "command": "setup.plan",
  "status": "complete",
  "data": {
    "profile": "desktop",
    "readiness": "blocked",
    "verification_scope": "executable_probe_and_reviewed_catalogue",
    "target": "ubuntu_24_04_x86_64",
    "local_asr_model": {"status": "missing", "disposition": "managed_install", "required_authority": "user", "next_step": "Review the exact managed model action and its digest. Setup install remains unavailable until the complete installer qualifies."},
    "managed_install": "catalogue_accepted_install_pending",
    "catalogue_revision": "ubuntu-24.04-x86_64-2026-09-22-r2",
    "stop_new_plans_at": "2028-08-01T00:00:00Z",
    "plan_digest": "<64 lowercase hex characters>",
    "actions": ["<exact reviewed artifact actions>"],
    "dependencies": [
      {"dependency": "ffmpeg", "status": "missing", "disposition": "managed_install", "required_authority": "user", "next_step": "Review the exact managed action and its digest. Setup install remains unavailable until the complete installer qualifies."}
    ]
  }
}
```

## Output protocol

Human output is readable terminal text on stdout (P05 session operations use
indented JSON). In `--json` mode stdout contains
exactly one complete v1 result plus a newline. In `--events jsonl` mode each stdout
line is one bounded v1 event and exactly one terminal event ends the stream; for
`transcript get`, `search` and `candidates` evidence events precede it (see "Evidence
stream", "P08 transcript search" and "P08 visual candidates" above), and every other
command, including `transcript retranscribe`, writes the terminal event alone. stderr is
reserved for bounded, sanitized diagnostics and is never required to parse a result.

Output limits apply before writing:

- result: 1,048,576 bytes including the trailing newline;
- diagnostic: 4,096 bytes including the trailing newline;
- provider detail shown by `setup check`: 240 bytes.

ANSI, OSC, newlines, and other control characters from untrusted providers are
replaced in human diagnostics. A closed stdout is an I/O failure with exit 7; a
closed stderr cannot make an otherwise complete result fail.

The setup-check response preserves its existing v1 fields and adds lookup,
verification and typed remediation metadata. The complete frozen example is
[`setup-check.blocked.json`](../../schemas/v1/examples/setup-check.blocked.json);
an abbreviated response is:

`lookup` is `explicit_path` for a path passed to this check,
`configured_user_path` for an explicit per-user registration, or `filtered_path`
for safe ambient discovery. `setup configure <ffmpeg|ffprobe|whisper>
--executable <absolute-path>` persistently registers a canonical user-managed
file without executing it. It creates private per-user configuration under
`LOCALAPPDATA/vsift` on Windows, `~/Library/Application Support/vsift` on macOS,
or `${XDG_CONFIG_HOME:-~/.config}/vsift` on Linux. It rejects unknown record
keys/versions and unsafe storage; `setup check` revalidates and probes the
selection on each call. A per-call path takes precedence.
`setup configure-model --file <absolute-path>` stores a canonical nonempty
user-managed model path in
the same private record. Registration does not parse model bytes; `setup check`
identifies them in its `local_asr` object (the legacy `local_asr_model` field stays
`not_checked`). Configuration does not authorize downloads or establish
model/provider compatibility.

Configuration writes use one private lock file. A held lock returns retryable
`BUSY` without changing the record; an OS lock failure returns `STORAGE_IO`
rather than claiming contention. Every VSift lock is released with an explicit
unlock rather than by closing its file, so a lock never stays held after its
owner lets go ([issue #66](https://github.com/smormah/vsift/issues/66)).

### Private per-user folders

Every folder VSift creates for itself is private to the current user whatever its
parent grants: the per-user configuration folder and any missing parent of it, the
session root and its created parent, each retained bundle, and the managed-data
folder. On Windows each gets its own protected DACL (no inheritance from the parent)
granting only the current user, SYSTEM and Administrators; on Unix it is created
owner-only (0o700). This happens before anything is written into the folder, and the
result is still validated.

An existing folder is never changed. When the per-user configuration folder or the
session root exists but another account can access it (on Unix: group or other mode
bits, or another owner), the command fails before using it with `STORAGE_IO`
(exit 7, not retryable) and one `remediation` item (`required_authority: "none"`,
`command: null`). Its summary begins with the fixed sentence `The VSift <kind> folder
is accessible to other accounts, so VSift did not use it and changed nothing.`, where
`<kind>` is `user_configuration` or `session_root`, followed by fixed prose saying
where that folder is by default and to delete it and retry (or remove the other
accounts' access). The folder's path is never included. Every command that reads the
configuration (`setup check`, `setup plan`, `setup configure`, `setup
configure-model`) or opens the session root (`ingest`, `session ...`,
`transcript get`) reports it this way. Before 2026-09-24 the configuration case was
`STORAGE_IO` without remediation and the session-root case was `INVALID_ARGUMENT`.
See [`storage-not-private.json`](../../schemas/v1/examples/storage-not-private.json).
Storage that is a link, unreadable or not writable remains `STORAGE_IO` without this
remediation. A folder another VSift process created a moment ago can briefly look
non-private while that process restricts it, so a folder that is still empty and was
created within the last 10 seconds is checked again for up to 2 seconds (the session
root keeps its existing 5-second wait for a creator) before it is refused.

```json
{
  "schema_version": "1",
  "command": "setup.check",
  "profile": "desktop",
  "status": "blocked",
  "verification_scope": "executable_probe_only",
  "local_asr_model": "not_checked",
  "dependencies": [
    {"dependency": "ffmpeg", "capability": "media_processing", "status": "missing", "detail": null, "lookup": "filtered_path", "validation": "not_validated", "remediation": {"reason": "missing", "managed_install": "unavailable_unqualified", "required_authority": "user", "next_step": "Install or locate a trusted FFmpeg executable, then rerun setup check.", "explicit_path_option": "--ffmpeg"}}
  ],
  "local_asr": {
    "model": {"status": "not_selected", "profile": null},
    "verification": {"status": "not_run", "source": null, "not_run_reason": "media_tools_unavailable", "check": null, "reason": null}
  }
}
```

New operations use these required terminal fields:

```json
{
  "schema_version": "1",
  "command": "session.list",
  "operation_id": null,
  "status": "failed",
  "data": null,
  "warnings": [],
  "error": {
    "code": "COMMAND_NOT_IMPLEMENTED",
    "message": "This reserved R0 command is not implemented in the current build.",
    "retryable": false,
    "retry_after_ms": null,
    "affected_ids": [],
    "remediation": []
  },
  "coverage": null,
  "lifecycle": null
}
```

The authoritative field definitions and complete examples are in
[`schemas/v1`](../../schemas/v1/README.md). Consumers of a terminal event validate
the event wrapper against `terminal-event.schema.json` and its `result` member
against `operation-response.schema.json`; evidence events validate against
`evidence-event.schema.json`.

## Exit and error taxonomy

| Exit | Category | Representative codes |
| ---: | --- | --- |
| 0 | complete or supported partial/degraded result | none |
| 1 | unexpected internal failure | `INTERNAL` |
| 2 | usage, unsupported schema, missing capability, or unavailable isolation | `INVALID_ARGUMENT`, `UNSUPPORTED_SCHEMA`, `MISSING_CAPABILITY`, `ISOLATION_UNAVAILABLE`, `COMMAND_NOT_IMPLEMENTED` |
| 3 | invalid or unsupported source | `INVALID_SOURCE` |
| 4 | retryable condition | `BUSY` |
| 5 | deadline or resource limit | `DEADLINE_EXCEEDED`, `RESOURCE_LIMIT` |
| 6 | cancellation | `CANCELLED` |
| 7 | storage or output I/O/integrity failure | `STORAGE_IO`, `INTEGRITY_FAILURE` |

Every machine error includes a stable code, safe message, retryability, optional retry
delay, affected identifiers, and structured remediation. Evidence or provider text is
not interpolated into the public message. Remediation commands, when introduced, are
an executable plus argument array and never shell text.

## Identifiers, time, geometry, and confidence

Opaque IDs use a type prefix followed by 16 to 64 lowercase ASCII letters or digits:
`ses_`, `job_`, `op_`, `art_`, `evd_`, and, since P07, `trv_` (transcript revision),
`tsg_` (transcript segment) and `sgm_` (source segment), and since P08 `vix_` (visual
index revision) and `vcd_` (visual candidate). Transcript and source-segment
identities are derived from content (session, sidecar digest, format, offset and
ordinal; source identity and segment index), so re-importing the same sidecar with the
same offset into the same session names them identically; visual identities derive from
the session, source, stream, analysis profile, window and representative time (and the
revision number for `vix_`), so a candidate keeps its identity in every later revision. Source and operation identities are
`src_sha256_` or `opk_sha256_` followed by exactly 64 lowercase hexadecimal digits.
They cannot contain paths, options, whitespace, or control characters.

All public media positions are non-negative source-timeline microseconds. Ranges are
half-open `[from, to)` and require `from < to`. Provider stream time bases are
normalized with checked rational arithmetic. Frame results keep requested and actual
times separate and expose their signed delta.

A crop is positive `x,y,width,height`, uses orientation-correct displayed pixels, and
must be wholly contained by positive frame dimensions. Checked arithmetic rejects
zero, overflow, and out-of-bounds rectangles before I/O.

Confidence is optional. Unknown confidence stays `null` with origin `unavailable`;
provider values state whether they are calibrated. A speaker label is bounded provider
metadata, not verified human identity.

## Pagination

Candidate, transcript and search pages default to 20 items and accept 1 through 100.
A candidate cursor's generation is the state of the windows its own range touches, not
the index revision, so analysing other windows does not invalidate it.
Continuation cursors are opaque, local tokens of at most 512 bytes. They contain no filesystem paths or
credentials and are bound to session, canonical-query digest, immutable generation,
last item, and expiry. A cursor from another query/session/generation or an expired
cursor is rejected rather than silently restarted.

## Configuration and compatibility

Effective configuration is immutable for one operation. Precedence is:

1. validated explicit flags;
2. an explicitly selected configuration;
3. per-user configuration;
4. built-in defaults;
5. host policy, which constrains every preceding layer.

P01 implements this resolver but does not load configuration files; `setup check`
currently supplies only explicit flags and defaults. Project-local configuration is
never discovered from the current directory. P06 will add explicit selected/user
configuration loading under this frozen precedence and strict
[`config.schema.json`](../../schemas/v1/config.schema.json).

Unknown schema majors are rejected. Request/config documents are strict and reject
unknown or missing fields, invalid enums, more than 1,048,576 input bytes, and nesting
deeper than 64 containers. Within major v1, readers must ignore additive response
fields; producers must not reinterpret or remove existing fields without a new major.

## Contract-test traceability

| Test ID | Executable evidence |
| --- | --- |
| C-01 | CLI hierarchy, help/version, parse errors, reserved-command failure |
| C-02 | deterministic ready/degraded/blocked setup and terminal response states |
| C-03 | page bounds and cursor scope/expiry/round trips, including transcript pages, search pages (`search_cli_contract`, `engine_search`, the application's `search` tests with a no-gap/no-duplicate property) and candidate pages (`candidates_cli_contract`, `engine_candidates`, the application's `visual` tests with the property `any_range_and_limit_page_without_gaps_or_duplicates`) |
| C-04 | opaque identifier rejection of path, option, Unicode/control payloads |
| C-05 | bounded/sanitized output and broken stdout/stderr behavior |
| C-06 | strict bounded JSON decoding and schema/identifier rejection |
| C-07 | checked time/range/crop invariants and property tests |
| C-08 | schema examples and old-reader/additive-v1 compatibility, including the `setup check` `local_asr` object (`setup_local_asr_contract`, `engine_setup_local_asr`) |
| C-09 | legal job and cancellation terminal transitions |
| C-10 | unknown confidence, speaker metadata, time normalization, requested/actual timing, imported-transcript offset conversion, local-ASR provenance and carried segments (`local_asr_contract`, `local_asr_store`) |

These tests establish the public boundary only. Provider execution, filesystem
durability, media correctness, concurrency, and load guarantees belong to later
packets and require their own evidence.

## P01 threat review

| Threat | P01 control | Residual owning packet |
| --- | --- | --- |
| SEC-01 | typed allowlisted CLI grammar, opaque identifiers, literal queries, separate path values, and no new process execution | P02 proves exact provider argument/process policy |
| SEC-03 | bounded complete output, safe diagnostics, terminal-control replacement, and broken-pipe outcomes | P02 adds concurrent capped provider-pipe draining and termination |
| SEC-16 | evidence cannot select configuration or install authority; remediation is typed data rather than shell text | P12 qualifies the agent procedure against hostile evidence |
| SEC-21 | strict bounded/depth-limited data decoding, closed schemas, no reference retrieval, and no executing deserializer | P05 validates bundle containment, counts, sizes, paths, and hashes |

These are deliberately partial controls where the threatened provider, bundle, or
agent feature does not exist yet. P01 does not close a later packet's security gate.
