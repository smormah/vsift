# P05 disposable-session and bundle qualification

Status: implementation review in progress. This record describes the P05 branch;
the [delivery ledger](delivery-ledger.json) remains authoritative for protected
completion.

## Implemented boundary

`ingest <local-file>` now opens a foreground ephemeral session without
transcription or setup. The application preflights the requested publication
guarantee before registration, stages a no-follow private source snapshot, and
commits source identity and expiry in the P03 immutable generation chain.
`ingest --transcript` remains reserved for P07 and fails before mutation.
The default root is a private per-user cache directory; `--session-root`
selects an explicit absolute private root for a local invocation.

The idle TTL is 24 hours from open/renew, capped at seven days from the first
open. A wall-clock timestamp alone cannot steal a live OS lock. Status discloses
open, closed, or expired state; renew rejects expired/closed sessions. Close
claims exclusive lifetime access before committing the closed state. There is
no hidden daemon, exact-time erasure, or secure-erasure claim.

The temporary index is root-local and capability-scoped. Its SHA-256 buckets
contain at most 256 marker files each, so each list/clean page scans at most one
bucket and supplies a numeric continuation cursor through 256 buckets.
Registration precedes initialization; its held marker lock protects a suspended
opener. A crash leaves a marker that becomes abandoned-cleanup eligible after
24 hours. Closed, expired, and abandoned owned sessions are cleaned only after
exclusive claim, contained tree validation, and quarantine. An active operation,
corrupt marker/manifest, link escape, or excessive tree is skipped or rejected
without deleting original media. Retained output sits outside the disposable
root and is never targeted by this cleaner.

P04 frame/audio bytes can be published as bounded immutable content-addressed
artifacts in a later session generation. `session retain` creates a fresh
private user-selected bundle. Evidence-only bundles contain committed artifact
bytes and a source hash/size; later re-extraction requires original media with
that hash. `--include-source` copies and verifies the private source snapshot.
`bundle validate` checks version, exact generated names, entry count, byte
limits, no-follow/single-link regular files, and SHA-256 without executing
content. The export manifest is the last file written and an incomplete
selected directory never validates. Exports report
`process_crash_consistent`, not strict OS/storage-crash durability, under
[ADR 0013](../decisions/0013-retained-bundle-publication.md).

## Named verification

| ID | P05 evidence |
| --- | --- |
| S-04 | Concurrent renew/close commits one generation; live shared reader and source hold exclude close/clean, including a killed child process. P03 already covers two readers/two writers and writer fencing. |
| S-05 | Domain expiry boundary, backward clock, renewal cap; suspended cross-process registration/read locks are never stolen by an advanced injected clock. PID identity is not used as lock authority. |
| S-06 | Closed/expired and abandoned marker cleanup, corrupt root ownership and external hard-link refusal; original and retained source bytes survive. |
| S-07/S-08 | Existing P03 manifest/pointer fault and corruption suites continue to pass with optional P05 lifecycle fields. P05 bundle future version, malicious path, changed source, and missing/changed file cases fail explicitly. |
| S-09 | A second export to an existing path conflicts; incomplete/malicious bundle metadata fails validation; only new private output is written. |
| S-10 | Evidence-only and source-inclusive bundles validate after move; source inclusion and matching-original requirement are explicit; duplicate destination never overwrites. |
| S-11 | Each indexed bucket is capped at 256; a 257th marker fails as capacity, with bucket continuation and no root-wide loading. |

The public [CLI contract](../contracts/cli-v1.md) is exercised from separate
processes against the v1 JSON schema. The opt-in cumulative P05 checkpoint is:

```console
cargo test -p vsift-infrastructure --locked --test p05_session_e2e -- --ignored --nocapture
```

On 2026-09-12 it passed locally on Windows x86-64 with native FFmpeg/FFprobe
9.0 and the project-owned F01 fixture. The report records fixture manifest
SHA-256 `dfae6419dba8b8bbbf00e18185c7580e1a9b935a61e9abbb82c9c549481e8187`,
source SHA-256 `65cec002d7dd8747e8ceb76f25270d35f38bfe354292e3c07bfa6169e2445070`,
actual frame time 750,000 μs, two retained bundle modes, original preservation,
environment/tool versions, authorization and coverage gaps. The run was on a
dirty pre-merge checkout; its filesystem was uninspected, so it is development
evidence only. P06-P14 and the complete video-to-grounded-handoff journey
remain `not_implemented`.

Dependency review: `getrandom` 0.3.4 was already present in the lockfile and is
used directly for unpredictable session/operation IDs. `time` 0.3.55 supplies
RFC 3339 formatting at the CLI boundary. Both declare MIT or Apache-2.0;
`time` requires Rust 1.88, below this repository's 1.98 MSRV. Both upstream
repositories have recent releases ([time](https://github.com/time-rs/time/releases),
[getrandom](https://github.com/rust-random/getrandom/blob/master/CHANGELOG.md)).
The direct `getrandom` 0.3 use avoids introducing a second major version while
the existing dependency graph still uses 0.3; revisit with a consolidated
0.4 upgrade. The new lockfile closure passed `cargo deny check`
for advisories, bans, licences and sources; its duplicate-version warnings are
pre-existing dependency-tree cases.

## Limits and follow-up ownership

- The P05 CLI creates no durable workspace or managed catalogue. Explicit
  durable requests still fail before mutation. P10/P11/P14 own strict
  Ubuntu/ext4 OS/storage-crash qualification.
- A selected export may remain incomplete after interruption. It is private,
  fails validation, and requires explicit caller inspection/removal. VSift
  never treats it as a completed bundle or deletes it automatically.
- The source snapshot is at most 20 GiB. Committed evidence is at most 10 GiB
  across 256 artifacts; a generation chain is capped at 4,096. Cleanup
  refuses more than 100,000 contained entries, 30 GiB, or five nested levels.
- The P05 checkpoint exercises one real-media fixture; the P04 checkpoint
  continues to exercise seven media scenarios. Future packet stages attach
  their own production behavior rather than inheriting a pass from this run.
- Desktop provider isolation retains the [ADR 0012](../decisions/0012-p04-source-media-profile.md)
  limits. A local same-user attacker who can rewrite private state is outside
  the desktop trust boundary.

Hosted macOS and Windows runners exposed a test-fixture collision: wall-clock
nanoseconds alone did not distinguish parallel roots within one process. The
P04/P05 temporary fixtures now add a process-local atomic sequence; no
production session identity depends on wall-clock resolution.
