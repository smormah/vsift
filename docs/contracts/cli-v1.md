# CLI and JSON contract v1

Status: published v1 boundary. `setup check`, foreground `ingest`, the P05
`session` lifecycle and `bundle validate` are operational. Other commands below
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
| `setup plan/install/repair/list/remove/rollback/configure` | Explicit managed dependency lifecycle | P06 |
| `ingest` | Open a disposable source-bound session; transcription remains P07 | Implemented in P05 |
| `session list/status/close/renew/retain/clean` | Session and retention lifecycle | Implemented in P05 |
| `transcript get/retranscribe` | Timestamped transcript evidence | P07 |
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
an RFC 3339 expiry. It does not start FFmpeg, setup, transcription or indexing.
`ingest --transcript` remains a P07 reservation and fails before mutation.
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

Running `vsift` or `vsift setup` without a leaf command prints help and performs no
dependency probe or mutation. `setup check` defaults to the `desktop` profile and a
five-second per-dependency deadline; `--profile worker` and
`--timeout-seconds 1..60` are explicit overrides.

## Output protocol

Human output is readable terminal text on stdout (P05 session operations use
indented JSON). In `--json` mode stdout contains
exactly one complete v1 result plus a newline. In `--events jsonl` mode each stdout
line is one bounded v1 event and exactly one terminal event ends the stream. stderr is
reserved for bounded, sanitized diagnostics and is never required to parse a result.

Output limits apply before writing:

- result: 1,048,576 bytes including the trailing newline;
- diagnostic: 4,096 bytes including the trailing newline;
- provider detail shown by `setup check`: 240 bytes.

ANSI, OSC, newlines, and other control characters from untrusted providers are
replaced in human diagnostics. A closed stdout is an I/O failure with exit 7; a
closed stderr cannot make an otherwise complete result fail.

The setup-check response preserves the existing v1 shape and adds the resolved
`profile`:

```json
{
  "schema_version": "1",
  "command": "setup.check",
  "profile": "desktop",
  "status": "blocked",
  "dependencies": [
    {"dependency": "ffmpeg", "capability": "media_processing", "status": "missing", "detail": null},
    {"dependency": "ffprobe", "capability": "media_processing", "status": "missing", "detail": null},
    {"dependency": "whisper", "capability": "transcription", "status": "missing", "detail": null}
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
against `operation-response.schema.json`.

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
`ses_`, `job_`, `op_`, `art_`, and `evd_`. Source and operation identities are
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

Candidate pages default to 20 items and accept 1 through 100. Continuation cursors are
opaque, local tokens of at most 512 bytes. They contain no filesystem paths or
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
| C-03 | page bounds and cursor scope/expiry/round trips |
| C-04 | opaque identifier rejection of path, option, Unicode/control payloads |
| C-05 | bounded/sanitized output and broken stdout/stderr behavior |
| C-06 | strict bounded JSON decoding and schema/identifier rejection |
| C-07 | checked time/range/crop invariants and property tests |
| C-08 | schema examples and old-reader/additive-v1 compatibility |
| C-09 | legal job and cancellation terminal transitions |
| C-10 | unknown confidence, speaker metadata, time normalization, requested/actual timing |

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
