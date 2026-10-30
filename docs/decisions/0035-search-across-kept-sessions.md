# ADR 0035: Search across kept sessions

- Status: **Proposed**, 2026-10-30. Nothing it describes is built yet.
- Date: 2026-10-30
- Refines: [ADR 0013](0013-retained-bundle-publication.md) (retained bundles),
  [ADR 0018](0018-visual-candidate-index-and-transcript-search.md) (literal search and its ranking) and
  [ADR 0030](0030-on-screen-text.md) (on-screen text, decision 13). It meets
  [ADR 0029](0029-time-anchoring.md) (clock time). It is a first step towards P18's managed catalogue
  and decides nothing for it.
- Tracking: the 0.3.0 story W2. Register entry L-212.

## Context

Someone has kept several recordings, "the three customer calls from last week", and asks where
anyone mentioned the refund limit, or showed it on screen. `search` and `ocr search` work one session
at a time. The story asks for one question over every kept session, and only those, with hits that
each name their session, source, time and evidence id, and honest coverage for each session.

The first thing to settle is what "kept" means, because the code has no such state. Checked on `main`
at `8a35d1f4`, 2026-10-30, by reading the files named:

- **A session in the root is never kept.** It expires after 24 idle hours, and renewal stops seven
  days after it opened (`cli-v1.md`, P05). The store has no retained flag, and `session list` and
  `session status` have no such member. `session retain` changes nothing about the session.
- **What is kept is a retained bundle.** `session retain <id> --output <new-dir>`
  (`FilesystemSessionStore::retain_bundle`) writes a new, owner-private folder the user chose, outside
  automatic cleanup. VSift records nowhere that it did. So three calls from last week exist only as
  bundle folders: their sessions are gone.
- **A bundle holds what a search needs.** Its `bundle.json` lists every transcript, visual-index and
  screen-text record of the session. Each is verified by size and SHA-256 against that manifest and
  decoded by the same strict readers a session uses. The source is optional.
- **A bundle is not cheap to validate.** `bundle validate` hashes every artifact, and the whole source
  when it is included (up to 20 GiB), so a search cannot call it on every page.
- **A bundle holds no clock times.** Time anchors are not in bundles until time anchoring stage 2
  (L-210).

## Decision

### 1. "Kept" means a retained bundle that the caller names

The command reads the bundle folders named on its command line, and nothing else. It never opens the
session root. So a disposable session cannot be read, indexed or listed, by construction and not by a
filter, and nothing is searched that the user did not both keep and name.

The alternatives, and why not:

- **A kept flag on sessions in the root.** This would have sessions outlive their expiry, which changes
  "desktop investigations are disposable by default" for a feature that does not need it. A person who
  moved a bundle would also still have to name it.
- **A list of bundles in the root, written by `session retain`.** This is a small persistent catalogue:
  it goes stale when a folder is moved or deleted, and it stores paths the user never asked VSift to
  remember. Explicit creation, visible status and removal are P18's job (SEC-26, I-11). When P18 exists,
  it can answer "every kept session" with no folder named.

The cost, stated plainly: the user has to say where the bundles are. A folder of bundles is enough
(decision 2), so "my kept calls" is one path.

### 2. The command is `vsift bundle search`

```console
vsift bundle search ./kept-calls --query "refund limit" --json
vsift bundle search ./kept/call-1 ./kept/call-2 --query "E-409" --limit 50 --json
```

`bundle search <path>... --query <text> [--limit 1..100] [--cursor <token>] [--view full|brief]`, command
identifier `bundle.search`, data schema `bundle-search-data.schema.json`. It is a new command because
`search` requires a session and its data requires one `session_id` and one `revision`; making those
optional is not additive. `bundle` is already the noun for these folders (`bundle validate`).

Each `<path>` (at most 64) is one of two things:

- a bundle: a folder that holds a `bundle.json`;
- a folder of bundles: its immediate subfolders that hold a `bundle.json`. One level, never recursive,
  at most 4,096 entries looked at. A link is never followed.

A path resolving to no bundle at all fails the call with `INVALID_ARGUMENT` and a remediation that
names `session retain`. A wrong folder must never read as "nobody said it". Subfolders with no
`bundle.json` are counted in `skipped_entries`, because an interrupted `retain` leaves exactly that.

Left out of 0.3.0 because the story does not need them: `--from`/`--to` (a range of one video means
nothing across videos), `--events jsonl` and a filter by transcript or screen. Each can be added later.

### 3. What is read from each bundle, and what is trusted

Bundles are untrusted input that the user selected. For each one, in order, nothing being written:

1. The folder checks of `bundle validate`: absolute path, a real folder and no link, and **private to
   the current user** (`validate_platform_root_permissions`: owner and mode on Unix, the access list on
   Windows). That check is the rule "never another user's". A bundle copied in with ordinary
   permissions is reported `not_private`, as `bundle validate` already refuses it.
2. The manifest, bounded to 128 KiB and strictly parsed.
3. Only the newest transcript record, the newest screen-text index record and the newest visual-index
   record, each checked for size and SHA-256 against the manifest and decoded by the session readers
   (`read_transcript_artifact`, `read_screen_text_index_artifact`, `read_visual_index_artifact`). The
   newest revision is the one `search` would read.

The source, frames, audio and evidence records are not read or hashed, and no tool runs. A hit is
therefore as trustworthy as the record it comes from, and a damaged frame is `bundle validate`'s to
find. Text is published twice, as read and with hidden characters shown, as everywhere.

A bundle that fails any step **does not fail the search**. It is listed `unavailable` with a reason
(`not_private`, `integrity_failure`, `unsupported_version`, `unreadable`), and the result is `partial`.
A bundle whose transcript or index alone cannot be read still gives what it can (ADR 0030 decision 10,
RV-2 finding 2).

### 4. Bounds

| Bound | Value | Why |
| --- | --- | --- |
| Paths named | 64 | Keeps a command line and its parse bounded |
| Bundles searched per call | 64 | Every page reads them all, and each gets a coverage entry |
| Record bytes read per call | 1 GiB | Worst case for one bundle is 80 MiB (24 + 48 + 8); a typical bundle is a few MiB |
| Hits per page | 20 by default, 1 to 100 | As `search` |
| Coverage entries | every searched or unavailable bundle | Never cut: a gap is never hidden |

**Time is bounded by bytes, not by a clock.** The declared sizes are in the manifest before anything is
read, so which bundles fit does not depend on how busy the machine is. A clock would make the same
request give different pages and would break the cursor. Bundles are taken in rank order (decision 5);
those over a bound are `not_searched`, with `bundle_limit` or `read_budget`, listed by name (at most 64,
with a count), and the result is `partial`. To search them, the caller names them in another call.

The records are read one bundle at a time and dropped. Only the page's hits are kept, in a bounded
heap, so memory is about one bundle plus one page.

### 5. Ranking across sessions

Search stays literal and deterministic, with no scoring. The order is:

1. tier: `phrase` before `all_terms`, as both single-session searches rank;
2. bundle: by folder name, then manifest digest, so the order does not depend on the order of the
   paths;
3. time within the bundle: a segment's start or a line's first frame;
4. transcript before on-screen text at the same time.

So every phrase hit comes before any all-terms hit, and hits of one recording stay together inside each
tier. Matching and on-screen grouping are the domain's existing rules, unchanged.

The same bundle found twice, the same `manifest_sha256`, is searched once and counted in `duplicates`.
Two different bundles of one session or one source are both searched: they can hold different
revisions. Each hit says which it came from.

### 6. The cursor

A `next_cursor` is the existing `CursorToken`, bound to:

- the digest of the ordered manifest digests and the normalised query words;
- the first bundle's session and the bundle count;
- the rank position of the page's last hit, `<tier>-<bundle rank>-<rank within that bundle>`;
- an expiry 24 hours after it was issued.

Any change to the set of bundles, including a bundle added, removed, renamed or re-retained, rejects the
cursor with `INVALID_ARGUMENT` rather than silently restarting. Pages never repeat or skip a hit.
`total_hits` is exact on every page, because every page reads every bundle.

### 7. The result

`data` holds `query`, `hits`, `total_hits`, `next_cursor`, a `bundles` summary (`found`, `searched`,
`unavailable`, `not_searched`, `duplicates`, `skipped_entries`), `sessions` (one entry per bundle) and
`not_searched`.

Every hit is:

```json
{
  "kind": "transcript",
  "match": "phrase",
  "bundle": "call-2",
  "manifest_sha256": "...",
  "session_id": "ses_...",
  "source_id": "src_sha256_...",
  "evidence_id": "tsg_...",
  "time_us": 5000000,
  "segment": { "the transcript segment, as transcript get returns it": "..." }
}
```

For `kind` `screen_text`, `evidence_id` is the line's `line_id` and the object is `line`, the hit as
`ocr search` returns it. Both reuse published shapes, so a reader has one parser for each; `--view brief`
uses the brief segment. `bundle` is the folder's own name, sanitised; **no absolute path is ever
returned**.

`sessions` is the coverage the story asks for, on every page:

```json
{
  "bundle": "call-2",
  "manifest_sha256": "...",
  "session_id": "ses_...",
  "source_id": "src_sha256_...",
  "source_included": false,
  "state": "searched",
  "hits": 3,
  "transcript": {
    "state": "partial",
    "revision_id": "trv_...",
    "basis": "local_asr",
    "untranscribed_ranges": 2,
    "untranscribed_us": 14000000
  },
  "screen_text": {
    "state": "not_run",
    "index_id": null,
    "planned": 0,
    "read": 0,
    "failed": 0,
    "pending": 0,
    "candidates_complete": false
  }
}
```

Transcript states are `complete`, `partial`, `none` and `unavailable`; on-screen states are those of
`search`'s `screen_text` member. Coverage here is a summary, not the range lists, because 64 sets of
ranges would not fit the CLI's 1 MiB result limit.

**The envelope.** The status is `partial`, with a fixed warning, unless every bundle named was searched
and both of its parts are complete. So an ordinary result over bundles that never ran `ocr` is
`partial`. That is correct: the question asked about the screen, and the answer cannot speak for it.
`coverage.gaps` is empty, because a time range means nothing across videos, and `coverage.reasons` holds
distinct identifiers (`no_transcript`, `untranscribed_range`, `not_run`, `frames_pending`, `frames_failed`,
`candidates_incomplete`, `bundle_unavailable`, `bundle_limit`, `read_budget`). `lifecycle` is `retained`,
as for `bundle validate`.

The failures use existing codes, so no new failure code is added.

### 8. Layering

- `vsift-domain`: the rank key and the rule of decision 5; a way to get every hit of a revision, which
  `search_revision` is built on, because `search_revision` stops at one page.
- `vsift-application`: `bundle_search`, a pure accumulator that takes each bundle's records and keeps the
  bounded page, the totals and the coverage. It is tested without a filesystem. ADR 0030 decision 13
  expected this to call `page_screen_text` once per session; that use case pages with a per-session
  cursor, so the accumulator calls the same domain function under it, `screen_text_occurrences`, instead.
- `vsift-infrastructure`: the bundle's open checks, shared with `validate_bundle`, and a read of the
  three records.
- `vsift`: `Engine::search_bundles`, which finds the bundles and drives the accumulator.
- `vsift-contract`: the data, the schema, the `CommandName` entry.
- `vsift-cli`: arguments and the human renderer. The skill classes the command `explicit`, in one place
  (`references/commands.md`), and the trial freeze is rewritten at the merge rebase (`v030-rules.md`).

### 9. No persistent index in 0.3.0

Nothing is stored. Every call reads the records it needs, so there is no index to build, keep in step,
delete or leak, and deleting a bundle removes it from search. That also keeps ADR 0013's line: a bundle
is a data folder, and VSift keeps no memory of it.

The estimate, not yet measured: reading and decoding a few MiB per bundle costs tens of milliseconds, so
64 ordinary bundles take a few seconds per page and the worst case stays within the 1 GiB budget. An
index would save that per-page cost for a set that a person chose and usually keeps small. If the hosted
speed check shows otherwise, P18 is where an index belongs, and this ADR gets a note with the numbers.

## Consequences

- A person or an agent can ask one question of a folder of kept recordings, with no setup and no
  stored state, and is told exactly what could not be seen.
- It works on every bundle a 0.3.0 reader accepts, including bundles retained by earlier releases.
- **Known limits (L-212):** a person must name the folders. A hit has no frame at hand, and no clock time
  until bundles carry anchors (L-210); to look at a moment, open the source again with `ingest` (the
  bundle's `source.media`, or the original, whose SHA-256 the `source_id` names). A hit's ids are not
  those of that new session. At most 64 bundles and 1 GiB are read per call. A bundle copied without its
  private permissions is `not_private`. A record's digest is only as good as the manifest, which is not
  signed (as for `bundle validate`).
- Sessions still open in the root are not searched, even if they were retained: the bundle is.

## Tests

- **Domain and application:** the ranking across bundles, the page and cursor over a mixed set, a cursor
  refused after the set changes, pages that neither repeat nor skip, the budget's admission order, and
  coverage for each state.
- **Infrastructure:** a bundle not private, a link, a changed record, a manifest over its bound, a bundle
  with a part, an old bundle from `published_v0_1_0_records`.
- **CLI contract:** the schema and example, the session root not created or opened, a wrong folder
  refused, a folder of bundles, duplicates, one damaged bundle beside good ones, no absolute path in any
  output, the human renderer's snapshot.
- **Speed (load, never on the development machine):** a hosted or LOKI run with 64 bundles.
