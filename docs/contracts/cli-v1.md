# CLI and JSON contract v1

Status: published v1 boundary. `setup check/plan/configure/configure-model`, foreground `ingest`
(including supplied-transcript import), the P05 `session` lifecycle, `transcript get`
and `bundle validate` are operational. Other commands below
remain reserved and return `COMMAND_NOT_IMPLEMENTED` with exit 2. Reserving a
command does not claim its media, provisioning, or worker behavior is implemented.

## Command namespace

Global output options are `--json` for one terminal JSON document and
`--events jsonl` for a JSON Lines stream. They are mutually exclusive.
`--session-root <absolute-dir>` explicitly selects a private disposable
workspace; otherwise P05 uses the per-user application cache.

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
| `transcript retranscribe` | New local-ASR transcript revision | P07 (local ASR increment) |
| `search`, `candidates` | Bounded text and visual-candidate retrieval | P08 |
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
`INTEGRITY_FAILURE`, and a newer record version with `UNSUPPORTED_SCHEMA`.

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
runs FFmpeg/FFprobe on user media (today only `ingest --transcript`; later local ASR,
frames and audio use the same hook), VSift proves the resolved pair works by running a
small reviewed test video built into VSift (F01) through the same probe, frame and
audio steps an investigation uses and comparing each result with its known answers
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
`probe`, `frame` or `audio` and `<reason>` is `process_failure`, `provider_rejected`,
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

`setup check` still reports only the fast executable probe
(`verification_scope: executable_probe_only`), so FFmpeg selected as FFprobe passes
`setup check` but fails this preflight at the `probe` step.

**Retrieval.** `transcript get <session> --from <us> --to <us> [--limit 1..100]
[--cursor <token>]` returns the segments of the session's latest revision that
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
`INVALID_ARGUMENT`. A session without a transcript, a closed or expired session, or an
empty range is `INVALID_ARGUMENT`. With `--events jsonl` the page is streamed as one
evidence event per segment followed by one terminal event (below). `transcript
retranscribe` remains `COMMAND_NOT_IMPLEMENTED` until local ASR ships.

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
  revision of the same transcript has new keys; VSift emits no delete or tombstone
  events yet, and a consumer that wants only the latest revision filters on
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
working transcription model. Those checks and verified
managed installation remain P06 work. Provider `detail` is not an instruction
channel. Paths are not echoed in the response.

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
`transcript get` evidence events precede it (see "Evidence stream" above), and every
other command writes the terminal event alone. stderr is
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
still reports `local_asr_model: not_checked`. Configuration does not authorize
downloads or establish model/provider compatibility.

Configuration writes use one private lock file. A held lock returns retryable
`BUSY` without changing the record; an OS lock failure returns `STORAGE_IO`
rather than claiming contention. Every VSift lock is released with an explicit
unlock rather than by closing its file, so a lock never stays held after its
owner lets go ([issue #66](https://github.com/smormah/vsift/issues/66)).

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
  ]
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
`tsg_` (transcript segment) and `sgm_` (source segment). Transcript and source-segment
identities are derived from content (session, sidecar digest, format, offset and
ordinal; source identity and segment index), so re-importing the same sidecar with the
same offset into the same session names them identically. Source and operation identities are
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

Candidate and transcript pages default to 20 items and accept 1 through 100.
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
| C-03 | page bounds and cursor scope/expiry/round trips, including transcript pages |
| C-04 | opaque identifier rejection of path, option, Unicode/control payloads |
| C-05 | bounded/sanitized output and broken stdout/stderr behavior |
| C-06 | strict bounded JSON decoding and schema/identifier rejection |
| C-07 | checked time/range/crop invariants and property tests |
| C-08 | schema examples and old-reader/additive-v1 compatibility |
| C-09 | legal job and cancellation terminal transitions |
| C-10 | unknown confidence, speaker metadata, time normalization, requested/actual timing, imported-transcript offset conversion |

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
