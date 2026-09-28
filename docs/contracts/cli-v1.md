# CLI and JSON contract v1

Status: published v1 boundary. `setup check/plan/configure/configure-model`, foreground `ingest`
(including supplied-transcript import), the P05 `session` lifecycle, `transcript get`,
`transcript retranscribe` (local speech recognition), `search` (P08 transcript search),
`candidates` (P08 visual candidates), `frame get`, `frame neighbours`, `frame burst`, `crop`
and `audio` (P09 evidence navigation), `job status`, `job resume` and `job cancel` (P10
recoverable jobs), `job run` and `session init-workspace` (P11 worker host) and
`bundle validate` are operational. Other commands below
remain reserved and return `COMMAND_NOT_IMPLEMENTED` with exit 2. Reserving a
command does not claim its media, provisioning, or worker behavior is implemented.
The P11 batch contracts (batch summary and its events) are published ahead of `job
batch`; see "P11 worker contracts and events".

## Command namespace

Global output options are `--json` for one terminal JSON document and
`--events jsonl` for a JSON Lines stream. They are mutually exclusive.
`--session-root <absolute-dir>` explicitly selects a private disposable
workspace; otherwise P05 uses the per-user application cache. The first command
that needs the root creates it. Commands that race to create it converge on one
root: the others wait at most five seconds for the creator and use the root only
after the full ownership and privacy checks, failing with `BUSY` if it is still
being created. An existing directory that VSift did not create is never adopted.
`--host-isolation process-only|strict-linux` (P11 PR 2; default `process-only`)
selects the isolation a command runs under: `strict-linux` is accepted only when the
kernel attests the strict worker controls, and otherwise the command answers
`ISOLATION_UNAVAILABLE` (exit 2) before any work (see "P11 worker workspaces, admission
and isolation").

**Interruption (P10 PR 3).** While a long command runs (`ingest`, `transcript
retranscribe`, `candidates`, `frame get/neighbours/burst`, `crop`, `audio`, `job
resume`), the first `SIGINT` or `SIGTERM` (Unix) or console Ctrl-C or Ctrl-Break
(Windows) cancels it: it stops at its next boundary (a provider process is stopped, on
Unix with `SIGTERM` to its process group and a kill after 5 s, on Windows at once) and
writes its one documented terminal result: `CANCELLED` (exit 6), or the `partial`
result of `candidates` and the evidence commands, which commit what they finished. A
second interruption kills running providers without the graceful wait; either way the
process exits only after every provider it started has been reaped, within the
supervisor's 5 s + 5 s budget. An `ingest` copy is checked every 64 KiB; a
retranscription stops before its commit, and one already committing completes. Every
other command is short and keeps the operating system's default (an interruption ends
it; a commit is never half-visible). On Windows a console event reaches every process
on the console, and a process that inherited the "ignore Ctrl-C" attribute is never
told about a Ctrl-C (only Ctrl-Break); see L-053.

| Command | Contract purpose | Implementation packet |
| --- | --- | --- |
| `setup check` | Read-only dependency diagnosis | Implemented |
| `setup configure` | Persist an explicit user-managed executable path without running it | Partial P06 |
| `setup configure-model` | Persist an explicit user-managed model file path without parsing it | Partial P06 |
| `setup plan` | Read-only diagnosis plus exact reviewed Ubuntu 24.04 x86-64 catalogue actions and digest; manual guidance on unaccepted targets | Partial P06 |
| `setup install/repair/list/remove/rollback` | Explicit managed dependency lifecycle, still reserved | P13 ([ADR 0015](../decisions/0015-r0-delivery-replan.md)) |
| `ingest` | Open a disposable source-bound session; optionally import a supplied SRT/WebVTT transcript | Implemented in P05; transcript import in P07 increment 2 |
| `session list/status/close/renew/retain/clean` | Session and retention lifecycle | Implemented in P05 |
| `session init-workspace` | Create a worker workspace: an explicit root with an immutable operator policy (durability, admission capacity, session retention) | Implemented in P11 PR 2 |
| `transcript get` | Bounded, pageable timestamped transcript segments | Implemented in P07 increment 2 |
| `transcript retranscribe` | New local-ASR transcript revision, whole source or one range; a recoverable job since P10 PR 2 | Implemented in P07 increment 3b |
| `search` | Bounded, ranked literal transcript search with honest coverage | Implemented in P08 PR 1 |
| `candidates` | Bounded visual-candidate retrieval with honest coverage; analyses missing windows first | Implemented in P08 PR 4 |
| `frame get/neighbours/burst` | Exact source frames, their neighbours and bursts, with requested and actual time, lineage and reuse | Implemented in P09 PR 3 |
| `crop`, `audio` | Native crops of frames and crops; bounded WAV clips of the source audio | Implemented in P09 PR 4 |
| `bundle validate` | Bounded data-only bundle validation | Implemented in P05 |
| `job status/resume/cancel` | Report, continue or cancel one recoverable job by its id | Implemented in P10 PR 3 |
| `job run` | One versioned worker request in a worker workspace, recorded under its operation id: replay, conflict, busy and continuation; two-stage shutdown | Implemented in P11 PR 3 |
| `job batch` | A finite JSON Lines file of worker requests, at most `--concurrency` at once, each line independent; one summary and the D5 exit | Implemented in P11 PR 4 |

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
source bytes, committed generation, `process_crash_consistent` publication (or
`os_crash_durable` in a durable worker workspace, P11) and an RFC 3339 expiry.
Without `--transcript` it does not start FFmpeg, setup, transcription or indexing;
supplied-transcript import is described in the next section.
Default sessions expire after 24 idle hours; renewals cannot extend beyond seven
days from open. A worker workspace sets its own retention (P11, below). Close and
cleanup return busy while active work holds the session. Expiry becomes visible at
the wall-clock boundary, but physical cleanup requires a later `clean` invocation;
no daemon or secure deletion is promised.

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
Since P09 PR 2 a bundle may also carry `evidence_record` artifacts
([`bundle-evidence-record.schema.json`](../../schemas/v1/bundle-evidence-record.schema.json))
with their `frame_png` and `audio_wav` files; `bundle validate` decodes each record
strictly and checks every item against its file (ADR 0013 note of 2026-09-26). The
frame commands write them since P09 PR 3 and `crop` and `audio` since PR 4 (see "P09
frames" and "P09 crops and audio clips" below).

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
- **Unknown events.** Within v1 new event kinds may appear before the terminal event
  (P11 added `progress`, `lifecycle` and `result`; see "P11 worker contracts and
  events" below). Dispatch on `event`, skip a kind you do not know and still count
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

`transcript retranscribe <session> [--from <us> --to <us>] [--operation-id op_...]` transcribes the session's
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
holds `session_id`, `requested_range` (null for the whole video), the new `revision`,
`recognised_segment_count` and, since P10 PR 2, `job` (below). A local-ASR revision summary has `alignment.origin`
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
writes its chunk progress (since P11 PR 1: `progress` events, stage
`recognising_speech` in `chunks`, 0 of the planned chunks once the plan is made, then
every chunk, reused ones included; at most one per second; example
[`transcript-retranscribe.events.jsonl`](../../schemas/v1/examples/transcript-retranscribe.events.jsonl))
and then its one terminal event, whose result is the `--json` result; stream the
records with `transcript get --revision <revision_id> --events jsonl`, page by page. A first Ctrl-C or `SIGTERM` (see
Interruption above) stops the run before its commit: nothing is committed, whisper.cpp
is stopped and reaped, the chunks it finished are kept for a resume (below), and the
failure is `CANCELLED` (exit 6) with the session and job in `error.affected_ids` and a
remediation whose `command` is `vsift job resume <job>`
([example](../../schemas/v1/examples/retranscribe-cancelled.json)). A run killed
outright (for example `SIGKILL`) leaves the same interrupted job; its work directory is
removed by the session's next run or cleanup.

**Recoverable runs (P10 PR 2, [ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)).**
Every run is a *job* whose identity (`job_...`) derives from the session and the
request's operation key: the source, audio stream, replaced range, chunk plan,
recognizer and model identity, local-ASR verification and the revision it supersedes.
Each chunk's raw recognizer output is kept as a private checkpoint inside the session
(never evidence, never in a bundle). Running the same command again after an
interruption (a crash, a failure, Ctrl-C) finds the same job and continues from its
finished chunks; a checkpoint that cannot be used (damaged, or not this run's) is
removed and its chunk recognised again. The committed revision is exactly the one an
uninterrupted run gives; nothing about the resume is written into it. `data.job`
reports `job_id`, `resumed`, `chunks_reused` and `replayed`, the envelope's
`operation_id` names the operation the result is recorded under, and the warnings
`resumed_from_checkpoint` and `checkpoint_discarded` (fixed prose, starting with the
identifier) say when checkpoints were used or discarded. A session that moved during
the run (a renewal, another revision) is followed instead of failing after all the
work: the commit is retried against the new head, or, if another revision replaced the
base, the range is widened over it again and the result spliced onto it when the range
is unchanged. A run that another process is running is `BUSY` with its job in
`error.affected_ids` and `error.retry_after_ms` 2000; a retry continues the job or
returns its result. Contention (a busy processing slot or writer, a moved generation)
is retried automatically at most twice with jittered backoff.

An operation id makes a retry safe (maintainer decision D-1): `--operation-id op_...`
(`op_` and 16 to 64 lowercase letters or digits, checked by the grammar before
anything is read; a malformed one is a `parse` failure) names the request. Repeating
the request with the id its first attempt used returns the committed revision again
(`replayed: true`, no new generation), even where the tools have since gone, because
a request with an id is answered from its record before any tool is resolved; the
same id with a different request (another range) fails `IDEMPOTENCY_CONFLICT` (exit
2, not retryable, the job in `affected_ids`) without changing anything; the same id
while its job runs elsewhere is `BUSY`. Operation ids are session-scoped and expire
with the session. Without one, every successful run is new work: after a success the
newest revision is the new base, so the same command commits another revision.

**Failures** use the existing codes, and `IDEMPOTENCY_CONFLICT` (new in P10 PR 2) for a
reused operation id. A failed run carries one fixed-prose remediation
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
| Interrupted by Ctrl-C/`SIGTERM` or a library cancellation before the commit (the session and job in `affected_ids`, `job resume <job>` suggested; the job stays resumable); or cancelled with `job cancel` while it ran (the job is cancelled) | `CANCELLED` |
| Work directory or the session's copy of the video unusable | `STORAGE_IO` |
| The same job is running in another process (`affected_ids` names it, `retry_after_ms` 2000); the session is held by cleanup; every processing slot or the writer stayed busy after two automatic retries; another revision changed the transcript around the range during the run so the recognised range no longer matches (rerun: it is widened again) | `BUSY` |
| The `--operation-id` was used earlier in the session for a different request (the job in `affected_ids`) | `IDEMPOTENCY_CONFLICT` |

A failed attempt leaves the job resumable: the next run of the same command continues
it. It fails for good only when one chunk failed three times with the same code, after
16 attempts, or when its range was superseded; the same command then starts it again
from nothing.

### P10 recoverable jobs

```console
vsift job status job_0123456789abcdef0123456789abcdef --json
vsift job resume job_0123456789abcdef0123456789abcdef --json
vsift job cancel job_0123456789abcdef0123456789abcdef --json
vsift session status ses_0123456789abcdef --json
```

Every `transcript retranscribe` runs as a recoverable job
([ADR 0020](../decisions/0020-recoverable-jobs-and-durable-publication.md)); its id is
in the result's `data.job.job_id`, in `error.affected_ids` of an interrupted or busy
run, and in `session status`. The job commands take the job id only: the job's session
is found through the session root's job index. They are grammar-checked first (one
`job_...` identity; a malformed one is a `parse` failure). `--events jsonl` writes the
one terminal event, like every command without evidence records; `job resume` first
writes the resumed run's chunk progress, as `transcript retranscribe` does (P11 PR 1).

- `job status <job>` reports one job
  ([`job-data.schema.json`](../../schemas/v1/job-data.schema.json), example
  [`job-status.json`](../../schemas/v1/examples/job-status.json)): `job_id`,
  `session_id`, `kind` (`retranscribe`), `state` (`queued`, `running`, `cancelling`,
  `committing`, `interrupted`, `succeeded`, `failed`, `cancelled`), `live_owner`
  (a process holds the job now), `resumable` and `resumable_reason` (`interrupted`,
  `not_started`, or why not: `live_owner`, `succeeded`, `failed`, `cancelled`,
  `session_not_open`), `operation_id` (the caller's first id, or a succeeded job's own
  commit id, which replays it), `request.range`, `progress` (`chunks_total`, `null` for a
  job created before P10 PR 3, and `chunks_checkpointed`, the checkpoints it keeps now),
  `attempts` in its current epoch, `result` (`revision_id` and `generation` of a
  succeeded job) and `failure` (`code` and `retryable` of the failure that ended the
  last attempt of an interrupted or failed job). A job whose process ended without
  recording it is reconciled first: `succeeded` when its commit is in the session's
  manifest chain, else `interrupted`; a live job is never taken over. No member names a
  path, transcript text or provider output.
- `job resume <job>` continues an interrupted (or never started) job from its
  checkpoints, under the operation id the caller first bound to it, and answers
  ([`job-resume-data.schema.json`](../../schemas/v1/job-resume-data.schema.json), example
  [`job-resume.json`](../../schemas/v1/examples/job-resume.json)) with `job` (the job
  afterwards) and `outcome` (exactly `transcript retranscribe`'s data), the envelope's
  `operation_id`, the session's lifecycle and the same warnings, including
  `resumed_from_checkpoint` and `checkpoint_discarded`. It is a long command:
  interruptible as above. Jobs never renew their session.
- `job cancel <job>` cancels a job and reports it afterwards (`job-data`, example
  [`job-cancel.json`](../../schemas/v1/examples/job-cancel.json)). Cancellation is
  serialized with the commit under the job's state lock: an interrupted or queued job
  no process owns is `cancelled` at once and its checkpoints are removed; a live job is
  marked `cancelling` and its owner notices within 250 ms, stops its provider and
  cancels the job before any commit (the owner's own command then fails `CANCELLED`); a
  job that is `committing` or `succeeded` keeps its result and the answer carries the
  warning `cancellation_too_late`; a failed or cancelled job is reported as it is.
  Repeating a cancel changes nothing. A cancelled or failed job is started afresh (a
  new epoch) by running the original command again.

`session status <session>` additionally lists the session's newest jobs, most recently
changed first: `jobs` (at most 16 items of `job_id`, `kind`, `state`, `live_owner`,
`resumable`, `resumable_reason`) and `jobs_truncated` (whether it holds more). The list
is read-only: a crashed job is shown as reconciliation would record it. Every other
member of the status is unchanged; `session list`, `renew` and `close` do not list jobs.

| Condition | Code |
| --- | --- |
| No session of the root holds the job (also a missing root) | `INVALID_ARGUMENT` (fixed remediation) |
| `job resume` of a succeeded, failed or cancelled job | `INVALID_ARGUMENT` (the job in `affected_ids`) |
| `job resume` of a job whose session is closed or expired | `INVALID_ARGUMENT` (the session and job in `affected_ids`; a closed or expired session cannot be renewed, so open a new session with `ingest` and run the request there) |
| `job resume` of a job a process owns now | `BUSY` (the job in `affected_ids`, `retry_after_ms` 2000) |
| `job resume` interrupted by Ctrl-C/`SIGTERM` | `CANCELLED` (exit 6; the session and job named, `job resume` suggested again) |
| `job cancel` while an owner appeared meanwhile, or the state lock stayed busy | `BUSY` |
| Every failure of `transcript retranscribe` (the resumed run) | as listed above |

### P11 `job run`

```console
vsift --session-root /srv/vsift/workspace job run --request request.json \
  --input-root /srv/vsift/inputs --bundle-root /srv/vsift/bundles --json
vsift --session-root /srv/vsift/workspace job run --request request.json \
  --input-root /srv/vsift/inputs --drain-timeout-ms 30000 --events jsonl
```

`job run` (P11 PR 3, [ADR 0021](../decisions/0021-worker-and-batch-host.md) sections 2,
4 and 6) runs one job request (below) in the worker workspace `--session-root` names.
Flags:

- `--request <file>` (required): one job-request v1 document of at most 64 KiB. It is
  read bounded and decoded strictly before anything else; a refused request is
  `INVALID_ARGUMENT` (or `UNSUPPORTED_SCHEMA`) with its fixed rejection's remediation,
  and a file that cannot be read `INVALID_ARGUMENT` with fixed remediation. Nothing of
  the input is echoed.
- `--input-root <dir>` (required): the absolute, existing, local directory every path of
  the request is relative to (contained inputs, below).
- `--bundle-root <dir>`: the absolute, existing directory a `retain` step writes
  `<bundle_name>` into; a request with a `retain` step and no usable bundle root is
  `INVALID_ARGUMENT` before anything runs.
- `--drain-timeout-ms 0..300000` (default 0): after a shutdown signal, how long the
  running step may finish before it is cancelled (D4).
- `--admission-wait-ms 0..60000` (default 60000): how long a step waits for admission
  capacity, retrying with full jitter, before it answers `BUSY` (`0`: at once).

Requests run only in a workspace created by `session init-workspace` (D1); a desktop
root is `INVALID_ARGUMENT` with fixed remediation. A durable request in an ephemeral
workspace is refused (`workspace_not_durable`); an ephemeral request in a durable
workspace opens a durable session (the workspace decides, ADR 0020 D-3).

**Steps.** The request's target, then each step, in order; each is one operation, run
at most once per operation id:

- *ingest* (an `ingest` target) copies the source (and imports a supplied transcript,
  as `ingest --transcript --transcript-offset` does) into a new session whose id is
  recorded before the copy starts. The files are opened inside `--input-root` one name
  at a time following no link: a link anywhere on the path is `path_outside_input_root`
  (L-062), a missing file `INVALID_ARGUMENT`, a directory, special file or file with
  several hard links `INVALID_SOURCE`.
- *retranscribe* runs `transcript retranscribe` of the range as a recoverable job under
  the operation id `op_` + 32 hex digits of SHA-256(`vsift.job-step.v1`, the request's
  operation id, the step's index): `job status <job>` reports it, and a redelivered
  request continues or replays it.
- *candidates* runs `candidates` over the range (the whole video without one) until no
  window is left unanalysed, at most 16 calls; what stays uncovered (`undecodable`,
  `no_decoded_frame`, a candidate budget) is its `coverage` and makes it `partial`.
- *retain* writes the bundle under a hidden staging name beside
  `<bundle-root>/<bundle_name>` and renames it once it validates; its outputs name the
  bundle and the SHA-256 of its manifest. A directory already under the name is
  accepted only when it validates as this session's bundle, else `INVALID_ARGUMENT`
  (L-064).
- *close* closes the session; a closed session is success.

**Result.** The `job-result` (below) is the `data` of every outcome, and the envelope's
`operation_id` is the request's. `complete` and `partial` (with the partial warning)
exit 0. A `failed` or `cancelled` request carries the error of the failure that ended it
(the failing step's, or the refusal's): its code, fixed remediation, `retry_after_ms`
(every `BUSY` has 2000) and the session in `affected_ids`; the exit status is its class
(D5: the failing step's; a shutdown is 6). The result names no path and carries no
evidence text; the session is named once it exists.

**Replay, conflict, busy, continue (ADR 0021 section 4).** Each request is recorded in
the workspace under its operation id:

| The workspace holds | Same request digest | Another digest |
| --- | --- | --- |
| nothing | run it (attempt 1) | run it |
| an ended request (complete, partial, or failed for good) | the recorded result again, `replayed: true`, exit as recorded, nothing run, even if the inputs are gone | `IDEMPOTENCY_CONFLICT`, exit 2, nothing changed |
| a request another process runs now | `BUSY`, exit 4, `retry_after_ms` 2000 | `IDEMPOTENCY_CONFLICT` |
| an unfinished request (interrupted, cancelled, or failed with `BUSY`, `DEADLINE_EXCEEDED`, `STORAGE_IO` or `CANCELLED`) | continue from the first unfinished step (`attempt` + 1) | `IDEMPOTENCY_CONFLICT` |

A redelivered request therefore commits once: one session per operation id, and every
delivery after the first recorded result returns exactly that result. Spacing, member
order and an omitted or `null` deadline do not change the digest. When an ended
request's result cannot be recorded, its work is committed but the result is not
acknowledged: `STORAGE_IO` (exit 7) with the result as `data`; delivering the request
again records and returns it. A workspace keeps at most 4,096 records; records of
sessions that are gone are pruned first, else `RESOURCE_LIMIT` (L-063).

**Retries and deadline (X-09).** A step that meets contention (`BUSY`: an admission
unit, a busy session or writer, the same job elsewhere) is tried again after a
full-jitter backoff within `--admission-wait-ms` and the deadline; nothing else is
retried. The deadline is the request's `deadline_ms`, or one day; a step never starts,
and a retry never waits, with less than one second of it left, and a step still
running at the deadline is cancelled at its next boundary: `DEADLINE_EXCEEDED` (exit
5), resumable. Deadlines and waits count per delivery (L-065).

**Shutdown (O-04, D4).** `job run` traps `SIGINT`/`SIGTERM` (console Ctrl-C/Ctrl-Break
on Windows) before it starts. The first signal stops the request before its next step
and, after `--drain-timeout-ms` (at once by default), cancels the running step at its
next boundary: a recognition is left interrupted with its checkpoints, candidates
keep the windows they committed, a commit in progress completes. The request ends
`cancelled` (exit 6), resumable: the same request continues it. A second signal
escalates (providers are killed without the graceful wait); providers are always
reaped before the process ends.

**Events.** With `--events jsonl`: `lifecycle` `started` (readiness: the workspace's
publication, the isolation, the admission capacity, concurrency 1), `request_admitted`,
`progress` (`running_request` in `steps`; a recognition's `recognising_speech` in
`chunks` with its `job_id`; each with `request_operation_id`), `admission_waiting` when a
step waits, `draining` (`reason` `shutdown`) when a shutdown begins, the `result` event
(`line` `null`), `request_finished` (status, code, rejection, dropped progress),
`stopped` (`reason` `end_of_input`, or `shutdown`) and the terminal event, whose result
is the `--json` response. A failure before the request is read or the workspace is
opened is the one terminal event.

### P11 `job batch`

```console
vsift --session-root /srv/vsift/workspace job batch --requests requests.jsonl   --input-root /srv/vsift/inputs --bundle-root /srv/vsift/bundles   --concurrency 4 --drain-timeout-ms 30000 --events jsonl
```

`job batch` (P11 PR 4, [ADR 0021](../decisions/0021-worker-and-batch-host.md) section 5)
runs the job requests of one file in the worker workspace `--session-root` names, each
exactly as `job run` runs one (steps, records, replay, conflict, busy, continuation,
retries and deadline). Flags:

- `--requests <file>` (required): a regular file of at most 1,000 lines, each one
  job-request v1 object of at most 64 KiB, or blank.
- `--input-root`, `--bundle-root`, `--admission-wait-ms 0..60000` (default 60000) and
  `--drain-timeout-ms 0..300000` (default 0): as for `job run`, for every request.
- `--concurrency 1..16` (default 1): requests run at once; above the workspace's
  admission capacity it is `INVALID_ARGUMENT` before any work.

**Reading.** The file is opened once and its lines counted through the same handle
before anything starts: more than 1,000 lines refuse the whole batch (`RESOURCE_LIMIT`,
exit 5, `termination_reason` `line_limit`, `not_started_from_line` 1, nothing run;
L-066), and a file that cannot be opened or read to its end, or is not a regular file,
is `STORAGE_IO` (exit 7, `input_error`). Then one line is read at a time, and the next
only when fewer than `--concurrency` requests run: the queue is never held in memory. A
reader that stops reading the stream holds the batch back.

**Isolation.** Each line is its own request with its own cancellation, deadline, record
and result. A blank line is skipped but counted. A line that is not a valid request, is
longer than 64 KiB (`request_too_large`) or reuses the operation id of an earlier line
of the same batch (`duplicate_operation_id`) is refused alone: it has a
`request_finished` (`rejected`, its code and rejection) and a summary item, but no
`result` event. A request the engine refuses against the workspace
(`workspace_not_durable`, `path_outside_input_root`) has its result, with the
rejection. With `--admission-wait-ms 0` a request may answer `BUSY` because another
request of the same batch holds an admission unit for a moment (L-067).

**Events.** With `--events jsonl`: `lifecycle` `started` (readiness, with the batch's
concurrency); for each line `request_admitted`, its `progress` and `admission_waiting`
events, its `result` event (with `line`) and `request_finished`, in the order lines
end; `draining` (`shutdown`) when a shutdown begins and `draining` (`drain_timeout`)
if requests still run when the drain time ends; `stopped` with the termination reason;
then the terminal event, whose result is the `--json` response. Progress of a request
always precedes its result. A failure before the workspace is opened, or a concurrency
above the capacity, is the one terminal event. `--json` prints the summary alone.

**Result and exit (D5).** The `data` of every outcome is the batch summary (below).
Every request complete: `complete`, exit 0; some partial: `partial` with the warning
"At least one request completed with a stated gap; see the coverage of its partial
steps.", exit 0. A shutdown that stopped the reading or a running request: `CANCELLED`,
exit 6, with `termination_reason` `shutdown` and the first line not started. Otherwise
the most severe failure among the lines and the file, in the order 7 > 1 > 5 > 3 > 2 >
(a request cancelled by `job cancel`, exit 6) > 4, with fixed remediation. A supervisor
tells a shutdown from a cancelled line by `termination_reason` (L-067).

**Shutdown (O-04, D4).** The first `SIGINT`/`SIGTERM` (console Ctrl-C/Ctrl-Break on
Windows) stops the reading and every running request before its next step; after
`--drain-timeout-ms` their running steps are cancelled at the next boundary. Every
started request stays resumable: delivering the file again continues the stopped
requests and replays the finished ones. A second signal escalates.

### P11 worker contracts and events

```console
vsift job run --request request.json --input-root /srv/vsift/inputs --json   # P11 PR 3
vsift job batch --requests requests.jsonl --events jsonl                    # P11 PR 4
```

P11 PR 1 publishes the worker contracts
([ADR 0021](../decisions/0021-worker-and-batch-host.md), maintainer decisions D1-D5
accepted 2026-09-28) before the commands that use them: `job run` implements them
since PR 3 and `job batch` since PR 4 (above). The
schemas and frozen examples are authoritative.

**Job request** ([`job-request.schema.json`](../../schemas/v1/job-request.schema.json),
example [`job-request.json`](../../schemas/v1/examples/job-request.json)). One JSON
object, at most 64 KiB and 16 levels deep; every member is required except
`deadline_ms`; unknown members are refused.

- `schema_version` `"1"` (a newer one is `UNSUPPORTED_SCHEMA`, whatever else it holds);
- `operation_id` (`op_` and 16-64 lowercase letters or digits): the idempotency key;
- `durability`: `durable` (only in a workspace initialised as durable) or `ephemeral`;
- `deadline_ms`: 1 to 86,400,000, or omitted or `null` for the host's limit;
- `target`: `{"ingest": {"source": <path>, "transcript": {"path": <path>, "offset_us":
  <signed us>} | null}}` or `{"session_id": "ses_..."}`;
- `steps` (at most 8, run in order after the target): `{"retranscribe": {"range":
  {"from_us", "to_us"} | null}}`, `{"candidates": {"range": ... | null}}`,
  `{"retain": {"bundle_name": "[a-z0-9][a-z0-9_-]{0,63}", "include_source": bool}}`
  and `{"close": {}}`. `retain` at most once and followed only by `close`; `close` at
  most once and last; a session target needs a step.

A path is relative to the operator's `--input-root`, with `/` between names: at most
1,024 bytes and 32 names, and no empty, `.` or `..` name, `\`, `:`, control
character, `<>"|?*`, name ending in a dot or a space, or Windows device name (`CON`,
`NUL`, `COM1`, ...). The request digest is SHA-256 over `vsift.job-request.v1`, a line
feed and the canonical form of the decoded request without its operation id: spacing,
member order and an omitted or `null` deadline do not change it. The same operation id
with another digest is `IDEMPOTENCY_CONFLICT` (exit 2).

A refused request is `INVALID_ARGUMENT` (or `UNSUPPORTED_SCHEMA`) with a fixed
rejection and fixed remediation, never an echo of the input: `request_too_large`,
`request_too_deep`, `malformed_request`, `unsupported_schema_version`,
`invalid_operation_id`, `invalid_session_id`, `invalid_path`,
`invalid_transcript_offset`, `invalid_deadline`, `invalid_range`, `invalid_bundle_name`,
`too_many_steps`, `step_order`, `no_work`, and, decided by the host,
`path_outside_input_root`, `duplicate_operation_id` (another line of the same batch)
and `workspace_not_durable`.

**Job result** ([`job-result.schema.json`](../../schemas/v1/job-result.schema.json);
examples [`job-run.json`](../../schemas/v1/examples/job-run.json),
[`job-run.replayed.json`](../../schemas/v1/examples/job-run.replayed.json),
[`job-run.partial.json`](../../schemas/v1/examples/job-run.partial.json), each the
`data` of a `job.run` result). At most 64 KiB; no path and no evidence text.
`operation_id`, `request_digest`, `status` (derived: `cancelled`, then `failed`, then
`partial` when a step left a stated gap, else `complete`), `replayed` (the recorded
result of an earlier call, without new work), `attempt`, `session_id`, `source_id`,
`publication`, `lifecycle`, `steps` (the ingest of an ingest target, then each
requested step: `kind`, `status` (`complete`, `partial`, `failed`, `cancelled`,
`not_started`), `elapsed_ms`, `admission_wait_ms`, `job_id`, typed `outputs`,
`coverage` and `failure`), `failure` (`code`, `retryable`, `retry_after_ms`, the
`step` that ended the request or the `rejection` that refused it) and `controls`
(`isolation` `process_only` or `strict_linux`, `admission_capacity`, `concurrency`,
`resource_limits` `host_cgroup` or `not_enforced`: the attested host cgroup's limits,
never VSift's own, and `free_space_reserve` `enforced` or `not_enforced`: whether the
workspace's free-space reserve was checked before the copy).
The outputs are: ingest `generation`, `revision_id` (an imported transcript or
`null`); retranscribe `revision_id`, `generation`, `chunks_reused`; candidates
`visual_index_id`, `generation`, `candidate_count` (the step analyses until no window
of its range is left unanalysed; what stays uncovered is its `coverage` and makes it
`partial`); retain `bundle_name`, `bundle_sha256`, `artifact_count`; close
`generation`. A `partial` `job.run` result carries the warning "The request completed
with a stated gap; see the coverage of each partial step."

**Batch summary** ([`job-batch-data.schema.json`](../../schemas/v1/job-batch-data.schema.json),
example [`job-batch.json`](../../schemas/v1/examples/job-batch.json) for the requests
of [`job-batch.requests.jsonl`](../../schemas/v1/examples/job-batch.requests.jsonl)).
The batch reads at most 1,000 lines of at most 64 KiB, one at a time, and isolates
every line. Its `data`: `counts` (`complete`, `partial`, `failed`, `cancelled`,
`rejected`), `items` (per processed line, in the order they finished: `line`,
`operation_id`, `status`, `code`, `rejection`), `not_started_from_line` and
`termination_reason` (`end_of_input`, `shutdown`, `line_limit`, `input_error`). Exit
status (D5): 0 when every request is complete or partial; 6 when a shutdown stopped
the batch; otherwise the most severe failure class in the order 7 > 1 > 5 > 3 > 2 >
4 (a request cancelled without a shutdown ranks between 2 and 4; an input error is 7,
a line limit 5). `--json` returns the summary only; the full per-request results are
the `result` events of `--events jsonl`.

**Workspace** ([`workspace-data.schema.json`](../../schemas/v1/workspace-data.schema.json),
example [`workspace-init.json`](../../schemas/v1/examples/workspace-init.json)): the
data of `session init-workspace` (P11 PR 2): `profile` `durable_workspace`,
`durability`, `publication` (`os_crash_durable` exactly when durable),
`admission_capacity` (1-64), `session_retention_seconds` (default 604,800, one hour to
2,592,000) and `outcome` (`created` or `already_initialized`). It never names the path.

**Events.** Three event kinds join `evidence` and `terminal`, with the same identity
members (`schema_version`, `event`, `sequence`, `command`, `operation_id`, which is
`null` for these kinds). Every string member is an enum or a bounded identifier; no
event names a path or carries text. Each line is at most 64 KiB.

- `progress` ([`progress-event.schema.json`](../../schemas/v1/progress-event.schema.json)):
  `request_operation_id` (the worker request, else `null`), `job_id`, `stage`
  (`copying_source` in `bytes`, `recognising_speech` in `chunks`, `analysing_video` in
  `windows`, `running_request` in `steps`), `completed`, `total` and
  `progress_dropped` (updates of this request dropped before this event). Advisory: at
  most one per second and 4,096 per request; an update inside the second replaces the
  held one, which is written with the next update or just before the terminal event;
  when stdout is slow, progress is dropped and counted rather than slowing the work.
- `lifecycle` ([`lifecycle-event.schema.json`](../../schemas/v1/lifecycle-event.schema.json)):
  `kind` `started` (with `readiness`: `publication`, `isolation`,
  `admission_capacity`, `concurrency`), `request_admitted`, `admission_waiting`
  (`reason` `admission_capacity`), `request_finished` (`status`, `code`, `rejection`,
  `progress_dropped`), `draining` (`reason` `shutdown` or `drain_timeout`) and
  `stopped` (`reason` `end_of_input`, `shutdown`, `line_limit` or `input_error`), with
  the request's `line` and `request_operation_id` where it concerns one.
- `result` ([`result-event.schema.json`](../../schemas/v1/result-event.schema.json)):
  one request's `job-result` and its `line`, written as soon as it ends.

Lifecycle, result and terminal events are never dropped. Example:
[`job-batch.events.jsonl`](../../schemas/v1/examples/job-batch.events.jsonl) (two
requests, one with chunk progress; its terminal result is `job-batch.json`).

### P11 worker workspaces, admission and isolation

```console
vsift --session-root /srv/vsift/workspace session init-workspace \
  --durability durable --admission-slots 8 --retention-hours 168 --json
vsift --session-root /srv/vsift/workspace ingest ./recording.mp4 --json
vsift --host-isolation strict-linux --session-root /srv/vsift/workspace session list --json
```

**Worker workspace (P11 PR 2, [ADR 0021](../decisions/0021-worker-and-batch-host.md)
section 3, D1, D2).** `session init-workspace` creates a worker workspace at
`--session-root`, which must be given explicitly, be absolute, not be the per-user
session cache, and have an existing parent (else `INVALID_ARGUMENT`). Its policy is
fixed when it is created and recorded in the root's ownership marker:

- `--durability durable|ephemeral`: `durable` is accepted only where OS-crash
  durability is qualified (Ubuntu 24.04 with the root on local ext4 keeping write
  barriers); anywhere else the command fails with `MISSING_CAPABILITY` and creates
  nothing. `ephemeral` works everywhere (development and CI).
- `--admission-slots 1..64`: the workspace's admission capacity, in weight units.
- `--retention-hours 1..720` (default 168): how long a session lives after it opens
  or is renewed; renewals never extend a session beyond 720 hours from its opening.

The data is `workspace-data` (above), with `outcome` `created`, or
`already_initialized` when the root already holds exactly this policy. Any other
policy, raised or lowered, and an existing desktop root are refused with
`INVALID_ARGUMENT` and fixed remediation; nothing is changed. Values out of range are
`INVALID_ARGUMENT` (`parse`).

A workspace is used with the ordinary commands through `--session-root`. `ingest
--session-root <workspace>` opens a session with the workspace's durability: in a
durable workspace every session reports `publication` `os_crash_durable` (the command
line's durable mode, ADR 0020 D-3); in an ephemeral one `process_crash_consistent`.
Every session of a workspace reports `lifecycle.mode` `durable_worker` with its
expiry, since the mode names whose rules bound its life; `session renew` extends it
by the retention, and `session clean --expired` removes it once expired, as for any
session. Before a source is copied into a workspace on Unix, the filesystem must have
the source's size and a 1 GiB reserve free, else `RESOURCE_LIMIT`; Windows does not
check (L-061).

**Weighted admission (X-07, ADR 0021 section 5a).** Every root (4 units for a desktop
root) limits the work it runs at once by weight: a visual-candidate window's `FFmpeg`
pass takes 2 units, a source copy, probe or evidence extraction 1, and a local
speech-recognition attempt its recognizer threads, which are the machine's
parallelism capped at 8 and at the root's capacity (the count is in the revision's
provenance, so a smaller root gives a different revision id, L-023). Work that needs
more than the root's whole capacity fails with `RESOURCE_LIMIT` (exit 5) and fixed
remediation before any tool runs (for example `candidates` on a one-unit workspace).
Contention is `BUSY` (exit 4) with `retry_after_ms`; interactive commands keep their
two bounded retries, and `job run` (P11 PR 3) waits a bounded, jittered time
(`--admission-wait-ms`, at most 60 s) before answering `BUSY`, reporting the wait as
`admission_wait_ms`. No
order is kept between processes sharing a root (L-060).

**Strict Linux isolation (ADR 0021 section 8).** `--host-isolation strict-linux` is
accepted only when the kernel attests that the process runs in a cgroup v2 with finite
`cpu.max`, `memory.max` and `pids.max` (on its cgroup or an ancestor), on a read-only
root filesystem, with no network interface but loopback. Otherwise every command
answers `ISOLATION_UNAVAILABLE` (exit 2) with fixed remediation before any work; the
missing controls are not named in the result. The limits are the host's; VSift
reports them (`controls.resource_limits` `host_cgroup`) and never claims to enforce
them.

**Contained inputs (ADR 0021 section 9; S-01, S-02).** A worker request's source and
transcript paths are opened inside the operator's `--input-root` (`job run`, P11 PR 3): the
root is canonicalised once and held; a path must pass the request path grammar and is
opened one name at a time following no link, so a link anywhere on the path (even one
inside the root) and a file with several hard links are refused
(`path_outside_input_root`, L-062).

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

### P09 frames

```console
vsift frame get ses_0123456789abcdef --at 1025000 --json
vsift frame get ses_0123456789abcdef --at 5970000 --select displayed-at --json
vsift frame get ses_0123456789abcdef --candidate vcd_0123456789abcdef --json
vsift frame neighbours ses_0123456789abcdef evd_0123456789abcdef --count 2 --json
vsift frame burst ses_0123456789abcdef --from 4000000 --to 8000000 --max-frames 12 --events jsonl
```

The frame commands hand an agent the evidence itself: the exact displayed frame at a
time, the frames around one, and the distinct frames over a stretch of time
([ADR 0019](../decisions/0019-evidence-navigation.md), decisions D1-D7). Every frame is
the video's own decoded pixels at native resolution (8-bit RGB PNG; nothing is scaled),
in its displayed orientation, and every result says which frame each requested time
resolved to, with the requested and actual time and their signed difference.

**Grammar (D6).**

- `frame get <session> (--at <us> | --candidate <vcd>) [--select at-or-after|displayed-at]
  [--tolerance-us <0..=10000000>]`. Exactly one of `--at` and `--candidate`. With
  `--at`, `at-or-after` (the default) takes the first frame whose time is at or after
  the request, so the pixels were never on screen before the moment named, and
  `displayed-at` the frame on screen at it (the last frame at or before it). The frame
  may lie at most the tolerance from the request, in the policy's direction: 1,000,000
  us (one second) unless `--tolerance-us` says otherwise. With `--candidate` (a
  `candidate_id` that `candidates` returned for the session) the frame is the
  candidate's `representative_us`, at-or-after with tolerance 0; `--select` and
  `--tolerance-us` are then parse errors. F01 (20 fps): 1,025,000 gives 1,050,000
  (delta 25,000); 5,970,000 is refused at-or-after (`after_final_frame`) and gives
  5,950,000 displayed-at (delta -20,000); 6,000,000, the end, is refused for both.
- `frame neighbours <session> <evidence> [--count 1..20]` (default 1) returns up to
  `count` consecutive displayed frames on each side of a whole frame that `frame get`,
  `frame neighbours` or `frame burst` returned in the session: consecutive frames,
  never time-spaced samples. `neighbours.before_stop` and `after_stop` say why a side
  holds fewer: `start_of_stream`, `end_of_stream` or `search_window` (the frames
  searched, up to 29 s either side, ended; more may exist).
- `frame burst <session> --from <us> --to <us> [--max-frames 1..100]` (default 12)
  spreads `max-frames` even target times over the half-open range (`from + k *
  duration / max-frames`), takes the first frame at or after each target and before the
  range's end, and returns each distinct frame once. The range is at most 60 s; one
  past the end of the video is clipped to it (`burst.extent`
  `clipped_at_end_of_stream`). `burst.targets` and `burst.distinct` say how many targets
  there were and how many distinct frames they named: a static screen returns one frame
  once, not many times. A range denser than one 1,200-frame listing (more than 20 s of
  60 fps video) is refused for now (`outside_listing`).

**Result.** `data` ([`frame-data.schema.json`](../../schemas/v1/frame-data.schema.json),
examples [`frame-get.json`](../../schemas/v1/examples/frame-get.json),
[`frame-neighbours.json`](../../schemas/v1/examples/frame-neighbours.json) and
[`frame-burst.partial.json`](../../schemas/v1/examples/frame-burst.partial.json)) holds
`session_id`, `source_id`, `operation` (`frame_get`, `frame_neighbours`,
`frame_burst`), the canonical `request` (with the defaults filled in), `request_key`,
`reused`, `profile` (`p09-r0-v1`), `tool_fingerprint`, `source_check`, `selections`,
`neighbours` and `burst` (the operation's detail, otherwise `null`), `items`, `files`
and `partial_reason`. Each selection has a `role` (`requested`; `before` and `after`
around an anchor; `target` in a burst), the `evidence_id` it resolved to,
`requested_us` (the requested time, the anchor's time or the target's time),
`actual_us` and `delta_us` (`actual_us - requested_us`). Each item is a published
`frame_evidence` record ([`frame-evidence.schema.json`](../../schemas/v1/frame-evidence.schema.json)):
`evidence_id`, `source_id`, `stream_index`, `kind` (`frame` or `crop`), `frame` (the
stream timestamp `pts` in the stream's time base, the normalized `time_us` and the
displayed frame's `width` and `height`), `crop` (`null` for a whole frame), `image`
(`image/png`, its size, SHA-256 and bytes), `profile` and `tool_fingerprint`.

**Identities and reuse (V-08).** An item's `evidence_id` digests only what fixes its
pixels: the session, source, stream, profile, provider fingerprint and the frame's exact
timestamp and time base. Two requests that resolve to one frame (1,025,000 and
1,040,000 on F01) therefore name one item and one file. `request_key` digests the whole
request with the profile and the media provider's fingerprint. Repeating a request with
the same key returns the committed result with `reused: true` after verifying every
file again: no FFmpeg runs and nothing is written (about 150 ms through the binary).
Replacing FFmpeg or FFprobe is another provider and so another key; a session copy
whose bytes changed is `INTEGRITY_FAILURE`. `source_check` says how the call that
committed the record checked the session's copy: `full_hash` the first time, then
`identity` (its on-disk identity was unchanged since that hash; decision D1, residual
in ADR 0012).

**Files (D2).** `files` delivers each item's image as `{evidence_id, media_type, path}`,
in item order. `path` is the absolute path of the committed artifact inside the
session (`<session root>/sessions/<session>/artifacts/artifact-<sha256>.png`). On
Windows the path may use the extended-length form `\\?\C:\...`, exactly as the engine
verified it: it is a valid, long-path-safe path that the Windows file APIs accept, and
hosts may display it as they wish (friendlier display is P13's human output); consumers
must not assume either form. It is valid while the session exists: until
`session close`, expiry and cleanup remove it. Read it; never write to it. To keep
evidence, `session retain` the session; retained bundles and evidence records never
contain a path. This is the only place public output names a local path. The frozen
examples write paths under the placeholder root `/vsift-session-root`.

**Partial results.** A call is bounded to 100 frames, 200 megapixels decoded, 256 MiB
of images (or the session's remaining space), the session's evidence slots and 120 s.
When a bound, the deadline or a cancellation stops a call after it extracted
something, what it extracted is committed and returned with status `partial` (exit 0),
a fixed warning chosen by `partial_reason` (`frame_budget`, `pixel_budget`,
`byte_budget`, `session_evidence_budget`, `deadline_exceeded`, `cancelled`). A partial
record is never reused: repeating the request runs again.

**Evidence stream.** `--events jsonl` writes one evidence event per item, in item
order, with `record_type` `frame_evidence` and key `evidence_id`, then one terminal
event whose data
([`frame-stream-data.schema.json`](../../schemas/v1/frame-stream-data.schema.json),
example [`frame-get.events.jsonl`](../../schemas/v1/examples/frame-get.events.jsonl))
is the result without its items plus `record_count`, with the same `status`. Stream
rules are those of `transcript get` (above); a burst streams at most 100 items. Because
an item's identity is fixed by its pixels, the same key always carries the same record,
whichever request named it.

**Storage.** Each extracting call commits its new images and one `evidence_record`
([`bundle-evidence-record.schema.json`](../../schemas/v1/bundle-evidence-record.schema.json))
in one generation. A session holds at most 384 evidence artifacts (images, clips and
records) within its 512 artifacts and 10 GiB (D4, raised by ADR 0020 D-2); a call that would not fit a record
and one image fails before any process.

**Failures** use existing codes, each with a fixed remediation:
`INVALID_ARGUMENT` when no frame satisfies the request (`at_or_after_end`,
`after_final_frame`, `before_first_frame`, `no_frame_within_tolerance`,
`outside_listing`; the remediation names the policy, tolerance or range to change), for
an unknown candidate or evidence identity, a parent that is not a whole frame
(neighbours of an audio clip or a crop), an empty or reversed range, a burst over 60 s
(remediation: find the moments with `candidates`, then burst around them), a video
with no video stream and a closed or expired session. `--count 0` or `21`,
`--max-frames 0` or `101`, `--tolerance-us` over 10,000,000, both or neither of `--at`
and `--candidate`, and `--select` or `--tolerance-us` with `--candidate` are parse
errors (command `parse`). `RESOURCE_LIMIT` when the session has no room for more
evidence (remediation: `session retain` it, then open a new session with `ingest`).
`MISSING_CAPABILITY` when FFmpeg or FFprobe is missing (Whisper is not needed), the
preflight fails, or FFmpeg cannot run or extract a frame it listed. `INVALID_SOURCE`
when the video cannot be decoded. `INTEGRITY_FAILURE` when the session's copy changed,
a candidate's frame is not at its time, or an earlier item no longer describes the
video. `STORAGE_IO` when a delivered path is not valid UTF-8 (remediation: use a
`--session-root` whose path is). `DEADLINE_EXCEEDED`, `BUSY` or `CANCELLED` only when
nothing was extracted.

Human output is the indented JSON result; readable terminal text is P13's.

### P09 crops and audio clips

```console
vsift crop ses_0123456789abcdef evd_0123456789abcdef --rect 850,420,280,70 --json
vsift audio ses_0123456789abcdef --from 0 --to 1000000 --json
vsift audio ses_0123456789abcdef --from 5000000 --to 8000000 --events jsonl
```

**`crop <session> <evidence> --rect x,y,w,h`** cuts a rectangle out of a frame or an
earlier crop that the session holds (D6, D7). The rectangle is in the parent image's
own displayed pixels (orientation applied): exactly four canonical unsigned decimals
(no sign, space or leading zero), with a positive width and height; `x + width` may
equal the parent's width, one pixel more is refused. Syntax is a parse error;
containment is checked against the parent before any tool runs (`INVALID_ARGUMENT`
with the rectangle rule as remediation). FFmpeg decodes the parent's exact frame again
and crops it, so the image is the source's pixels at native size; nothing is scaled,
sharpened or invented (on the rotated F01 variant every tested crop equals FFmpeg's own
decode of that region pixel for pixel). The result is `frame-data.schema.json` with
`operation` `crop` (example [`crop.json`](../../schemas/v1/examples/crop.json)): the
`request` names `parent_evidence_id` and `rect`, the item has `kind` `crop` and `crop`
with `x`, `y`, `width`, `height` in the parent's pixels and `frame_x`, `frame_y` in the
whole displayed frame's, so a crop of a crop still names source pixels. Its selection
has role `requested`, `requested_us` the parent frame's time and delta 0. A crop's
identity digests its frame, its parent and both rectangles, so repeating it is
`reused`; a crop of an audio clip is `INVALID_ARGUMENT` (kind remediation). The stream
writes the crop as one `frame_evidence` event.

**`audio <session> --from <us> --to <us>`** extracts a WAV clip (16 kHz mono signed
16-bit little-endian, D5) of the half-open range, at most 30 s. A range that runs past
the end of the source is clipped to it and says so (`range_clipped: true`); one that
starts at or after the end, an empty or reversed range and one over 30 s are
`INVALID_ARGUMENT` with a remediation, as is a source without an audio stream. `data`
([`audio-data.schema.json`](../../schemas/v1/audio-data.schema.json), example
[`audio.json`](../../schemas/v1/examples/audio.json)) has `operation` `audio`, the
`request` (`from_us`, `to_us`), `request_key`, `reused`, `profile`, `tool_fingerprint`,
`source_check`, one `requested` selection (`requested_us` the requested start,
`actual_us` the first decoded sample's time), `range_clipped`, one `audio_evidence`
item ([`audio-evidence.schema.json`](../../schemas/v1/audio-evidence.schema.json):
`evidence_id`, `source_id`, `stream_index`, the clipped `range`, `actual_start_us` and
`audio` with `audio/wav`, 16000, 1, `s16le`, SHA-256 and bytes), `files` (the clip's
absolute path, `audio/wav`, D2) and `partial_reason`. `actual_start_us` is when the
first sample really is: 64 ms into F01's audio-only variant (AAC priming), 750 ms into
F09 (its audio starts late). The clip's identity digests the stream and the clipped
range, so two requests clipped to the same range share one clip. `--events jsonl`
writes one `audio_evidence` event, then the terminal event
([`audio-stream-data.schema.json`](../../schemas/v1/audio-stream-data.schema.json)).
Whisper is not needed; clips are decoded by FFmpeg.

A damaged or cut-short part of a source is `INVALID_SOURCE` with nothing committed and
a remediation that names `candidates` for finding undecodable parts; the decodable
parts stay usable.

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
`transcript get`, `search`, `candidates`, the frame commands, `crop` and `audio` evidence
events precede it (see "Evidence stream", "P08 transcript search", "P08 visual
candidates", "P09 frames" and "P09 crops and audio clips" above);
`transcript retranscribe` and `job resume` write `progress` events before it, each as
it happens (P11 PR 1); every other command writes the terminal event alone. The
terminal event's `sequence` is always the number of events before it. stderr is
reserved for bounded, sanitized diagnostics and is never required to parse a result.

Output limits apply before writing:

- result: 1,048,576 bytes including the trailing newline;
- a `progress`, `lifecycle` or `result` event line: 65,536 bytes including the
  trailing newline (P11);
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
`evidence-event.schema.json`, and the P11 kinds against `progress-event.schema.json`,
`lifecycle-event.schema.json` and `result-event.schema.json` (whose `result` is
`job-result.schema.json`).

## Exit and error taxonomy

| Exit | Category | Representative codes |
| ---: | --- | --- |
| 0 | complete or supported partial/degraded result | none |
| 1 | unexpected internal failure | `INTERNAL` |
| 2 | usage, unsupported schema, missing capability, unavailable isolation, or an operation id reused for another request | `INVALID_ARGUMENT`, `UNSUPPORTED_SCHEMA`, `MISSING_CAPABILITY`, `ISOLATION_UNAVAILABLE`, `COMMAND_NOT_IMPLEMENTED`, `IDEMPOTENCY_CONFLICT` |
| 3 | invalid or unsupported source | `INVALID_SOURCE` |
| 4 | retryable condition | `BUSY` |
| 5 | deadline or resource limit | `DEADLINE_EXCEEDED`, `RESOURCE_LIMIT` |
| 6 | cancellation | `CANCELLED` |
| 7 | storage or output I/O/integrity failure | `STORAGE_IO`, `INTEGRITY_FAILURE` |

Every machine error includes a stable code, safe message, retryability, optional retry
delay, affected identifiers, and structured remediation. Evidence or provider text is
not interpolated into the public message. A remediation command (since P10 PR 3:
`vsift job resume <job>` after an interruption) is an executable plus argument array
of fixed words and validated identifiers, never shell text, and needs no authority.

## Identifiers, time, geometry, and confidence

Opaque IDs use a type prefix followed by 16 to 64 lowercase ASCII letters or digits:
`ses_`, `job_`, `op_`, `art_`, `evd_`, and, since P07, `trv_` (transcript revision),
`tsg_` (transcript segment) and `sgm_` (source segment), and since P08 `vix_` (visual
index revision) and `vcd_` (visual candidate); since P09 `evd_` names an evidence item
(a frame, crop or audio clip). Transcript and source-segment
identities are derived from content (session, sidecar digest, format, offset and
ordinal; source identity and segment index), so re-importing the same sidecar with the
same offset into the same session names them identically; visual identities derive from
the session, source, stream, analysis profile, window and representative time (and the
revision number for `vix_`), so a candidate keeps its identity in every later revision;
an `evd_` identity derives from the session, source, stream, adapter profile, provider
fingerprint and the exact frame timestamp (with a crop's region or a clip's range), so
every request that resolves to one frame names one item. Source and operation identities are
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
deeper than 64 containers (a P11 job request or batch line: 65,536 bytes and 16 levels).
Within major v1, readers must ignore additive response
fields; producers must not reinterpret or remove existing fields without a new major.

## Contract-test traceability

| Test ID | Executable evidence |
| --- | --- |
| C-01 | CLI hierarchy, help/version, parse errors, reserved-command failure; `session init-workspace` creation, idempotence, policy refusal, durable fail-closed and strict-isolation refusal before work (`workspace_cli_contract`) |
| C-02 | deterministic ready/degraded/blocked setup and terminal response states |
| C-03 | page bounds and cursor scope/expiry/round trips, including transcript pages, search pages (`search_cli_contract`, `engine_search`, the application's `search` tests with a no-gap/no-duplicate property) and candidate pages (`candidates_cli_contract`, `engine_candidates`, the application's `visual` tests with the property `any_range_and_limit_page_without_gaps_or_duplicates`) |
| C-04 | opaque identifier rejection of path, option, Unicode/control payloads |
| C-05 | bounded/sanitized output and broken stdout/stderr behavior |
| C-06 | strict bounded JSON decoding and schema/identifier rejection, including the P11 job request (`vsift-contract` `request` tests, `worker_contract`, fuzz targets `job_request` and `job_batch_line`) and the recorded steps and results of a request record (`recorded_steps_and_results_read_back_exactly`, fuzz target `request_record`) |
| C-07 | checked time/range/crop invariants and property tests |
| C-08 | schema examples and old-reader/additive-v1 compatibility, including the `setup check` `local_asr` object (`setup_local_asr_contract`, `engine_setup_local_asr`) and the P11 event kinds (`worker_events_contract`: a reader that knows only `evidence` and `terminal` skips `progress`, `lifecycle` and `result` and still sees a contiguous sequence) |
| C-09 | legal job and cancellation terminal transitions (`vsift-domain` `job` tests over the whole state graph; `job` use-case tests of cancellation serialized with the commit); the public job commands, `--operation-id` and interruptions through the binary (`job_cli_contract`, `interrupt_cli_contract`, the job examples in `local_asr_contract`); `job run` through the binary: results and events against their schemas, replay, conflict, busy, refusals, sentinels and shutdown (`job_run_cli_contract`), and the request path through the engine (`engine_worker`); `job batch` through the binary: events, summary, limits, O-02, O-03, X-08 and O-04 (`job_batch_cli_contract`), and the batch through the engine (`engine_batch`, opt-in `engine_batch_tools`) |
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
