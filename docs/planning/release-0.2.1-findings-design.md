# Release 0.2.1, second candidate: the design for the larger findings of the live testing

Status: a design for the maintainer to decide from, written after the decision of 2026-10-11 that #365, #366, #367, #368 and #370 (larger), #369, #371, #372 and #373 (small) and #374 (only if it can be verified) ride in `0.2.1` as a second candidate, `rc.2`. **It decides nothing and changes no code, skill or register entry.** The candidate's plan is [`release-0.2.1.md`](release-0.2.1.md).

**What it rests on.** The issues as filed (times, counts and sizes only; no recording was opened), the code at `cb8cb3a`, and reproductions with a locally built `vsift`, a scratch session root and the committed synthetic corpus: #369, #371, #373 and parts of #366. Nothing that needs a real recording, whisper.cpp, a packaged desktop client or a Windows shell wrapper was run; those statements are predictions or are listed in part F. #359 is another agent's and is not designed here.

**Terms.** S: one crate and its tests. M: two or three crates, a schema, an example and documents. L: core behaviour, an ADR note and a capacity review. "Additive" is ADR 0008's rule: a new option, member or word is allowed in v1; a published failure code, a field's meaning or an existing answer for an existing input is not. **The skill is frozen by digest:** it can use a new flag, form or word only through a skill change, which is a new digest and a re-run of agent batches 2 and 3 (about $9 and $1.31 for Claude Code, 9 to 10 and 1.6 million Codex input tokens: `release-0.2.1.md`). So all skill changes go in **one** commit, last.

## The short version

| Finding | I recommend | Code | Skill changes | Independent review |
| --- | --- | --- | --- | --- |
| #365 repeated fields | A brief view (`--view brief`) on `transcript get`, `search` and `candidates`: a page of 50 falls from about 54,000 to about 9,000 characters | M | yes | yes |
| #373 coverage | `transcript_coverage` and the envelope `coverage` on `transcript get`; `partial` over a gap, as `search` does | S, same pull request | yes | yes |
| #366 long report | No code. State a size, check a long report in two parts, measure the clients first; do not raise the 64 KiB cap | none | yes | no |
| #367 candidates | Skill: at most 5 minutes per call; the brief view for candidates; **no filter and no progress events until measured on a real recording** | in #365's pull request | yes | no |
| #368 whole recording | **Out of scope for this line**: description and a stated limit; first item of the next line (part A) | none | yes | no |
| #370 BUSY | Skill and a register entry; the cause is the root's capacity, not the session; frames during a transcription is a later design | none | yes | later: yes |
| #374 packaged client | Verify first (five minutes, a synthetic clip); documents only if confirmed | none | no | no |
| #369 missing file | A not-found remediation; the code stays | S | no | no |
| #371 `session list` | Skill: the cursor form and "until `next_cursor` is null"; page filling later | none | yes | no |
| #372 new ids | Skill: two sentences; one new test that a superseded id resolves | test only | yes | no |

## #365: the repeated fields of every item

**Problem.** An item of `transcript get` or `search` is about 1,050 to 1,100 characters; a page of 50 is about 54,000. A client saved it and cut its read at about 43,000: 9 items and the `next_cursor` were lost with no error from VSift. Reading 171 segments took seven calls.

**Today.** `TranscriptSegmentData::new` (`crates/vsift-contract/src/transcript.rs`) builds the item for `transcript get` (`TranscriptPageData`), for `search` (`SearchData::new` in `search.rs`, with a parallel `hits` list) and for the `--events jsonl` evidence record (`stream.rs`); a retained bundle holds the stored record, a different shape. On the committed example `schemas/v1/examples/transcript-get.asr.json` an item is 1,109 characters, and **483 of them (44%) repeat what the revision block of the same answer holds** (revision, source and source-segment ids, language, provider, two SHA-256 values, model and decoding profile). The issue's "about 850" is more than that example supports (about 550 with the fixed nulls counted). The text is there twice (`text`, `display_text`), the chunk times are 138 and `carried_from` is 122. A brief item (`segment_id`, `start_us`, `end_us`, `display_text`, speaker label) is 153. `--limit` is 1 to 100 (`crates/vsift-cli/src/command.rs`, `TYPICAL_INVESTIGATION_HELP`); lowering the maximum would refuse requests valid today, so "settle it" can only mean wording ("up to 100; the agent budgets use 20 and 50").

**Options.**
- **(a) Skill only: 20 per page.** No surface. A page is about 22,000 to 25,000 characters; 171 segments take nine calls and about 190,000 characters. S. Does not fix the repeated bytes.
- **(b) Drop only the revision constants.** A page of 50 falls to about 30,000, still near where one client cut. Not recommended.
- **(c) A brief view.** One option on up to three commands (`full`, the default, and `brief`; the name is the implementer's), a `view` member in `data` only when brief (like rc.1's `unusable_chunks`, so the default answer stays byte for byte as before), two or three new data schemas and examples. Refused with `--events jsonl`: those records are published evidence that an indexer upserts by key (`cli-v1.md`, "Evidence stream"), and a reduced record under the same key would corrupt an index. Older releases: an unknown option is a parse failure (`INVALID_ARGUMENT`); default output is byte for byte unchanged; nothing stored differs. M; risk low (presentation only; engine untouched). Does not fix `display_text` growing up to four times for hidden characters (ADR 0008).

**Recommendation.** (c) on all three commands in one change, with (a)'s 20 as the rule for the full view. The repeated fields are most of what an agent pays to read a transcript: 171 segments of 90 characters are about 190,000 characters in the full form and about 33,000 in the brief form. (a) alone doubles the calls.

**To close.** Contract tests: both views validate against their schemas; every brief field equals the same field of the full item (the views cannot drift); the brief view with `--events jsonl` is refused and stream and bundle records are byte-identical to today's; a brief page is under a quarter of the full page on the example (the issue's "measured page in both forms"). Only a real run shows a client now reading a page of 50 with its cursor: the 17:34 case once, counts only.

**The skill says.** The option in the four `transcript get` and `search` forms of `commands.md` and in step 3; "copy ids and times from the item, the revision's id from the revision block"; the page rule in `budgets.md`.

## #373: coverage that `transcript get` does not return

**Problem.** Step 3 tells the agent to check `coverage` and `transcript_coverage` after reading windows; `transcript get` returns neither (reproduced).

**Today.** `transcript_get` (`crates/vsift-cli/src/session.rs`) adds only the lifecycle. `search_response` (`crates/vsift-contract/src/search.rs`) adds `transcript_coverage` and the envelope `coverage`, and makes the status `partial` when part of the searched range has no transcript. The rule is `SearchCoverage::of(revision, window)` (`crates/vsift-domain/src/search/coverage.rs`), a pure function of a revision and a range that `transcript get` can reuse.

**Options.** (a) Skill only: "coverage comes from `search`"; awkward, since a search needs a query. (b) Add `transcript_coverage`, always present, as `cli-v1.md` ("Configuration and compatibility") requires of an added member, and in the terminal data of `--events jsonl`; `coverage` stays null, status `complete`. (c) (b) plus the envelope `coverage`, and `partial` exactly when the range has untranscribed parts, as `search` does. All are additive and older releases ignore the member; (c) changes the status of an existing answer, only for a revision with an uncovered range. Before rc.1 that arose from a range-only transcription of a video with no earlier transcript, or from a splice that replaced every segment of an earlier run (coverage then understates, never overstates: `coverage.rs`); `search` already reports both as `partial`. The unusable-chunk gaps exist only in the candidate. S.

**Recommendation.** (c): one rule for both commands, and the envelope's `coverage` is what a generic consumer reads. Without it a read over a gap looks complete, the failure "a gap is never taken as absence" exists to prevent; hence the independent review.

**To close.** The domain function is reused. Contract tests: a complete session's bytes are unchanged except the new member; a gap session answers `partial` with the gaps; the stream's terminal data carries the member; schema, example, `cli-v1.md`. Stand-ins are enough (`retranscribe-unusable-output*.json`). **The skill says:** the existing sentence becomes true; a `partial` read is usable and its gaps are gaps.

## #366: a long report cannot be checked on Windows

**Problem.** Both of the skill's forms hand the report to a shell on the client's command line. In Git Bash a 15,000-character report was cut (an unexpected end of file about 70 of 150 lines in) and a 5,500-character one passed; in PowerShell 15,000 passed and 70,000 failed before `vsift` started (`ENAMETOOLONG`).

**Today.** `vsift handoff check` reads **standard input** (`check` in `crates/vsift-cli/src/handoff.rs`); the limits are the clients'. The Git Bash cut is about 7,000 characters if lines are even, near the 8,191 of a `cmd.exe` command line: arithmetic, not a measurement. Four things the issue did not have:
1. **The check refuses a draft over 64 KiB itself** (`MAX_HANDOFF_REPORT_BYTES`; `INVALID_ARGUMENT`, "larger than 64 KiB; shorten it"; reproduced with 70,858 bytes). The issue's closing test, a report of 70,000 characters or more passing, cannot succeed whatever the shell does.
2. `--file <absolute path>` exists, but the skill forbids writing a file (rule 8) and the trial grader counts any write or edit tool as unauthorized (`docs/agents/trials.md`, "The command policy").
3. **The skill's forms never pass `--session`, so the check compares no id with the session**: `data.session` is null by design (`cli-v1.md`, "P13 `handoff check`"). It checks the schema, the rules and the text. That answers the issue's "not known".
4. A prose half answers `handoff_block_missing` plus any text finding (a leaked path gave `local_path`); a block alone answers the schema and rules; reproduced. So checking in two parts already works with the two literal forms.

**Options.** (a) **Skill:** the whole message is at most 64 KiB; a size known to pass both forms (5,500 characters passed in both; more waits for a measurement); a longer report is checked as prose and block in two calls, the prose half's `handoff_block_missing` being expected. No surface; S; does not fix reports over 64 KiB. (b) A third form with `--file`: needs a file write, a grader change and a new freeze; against ADR 0023 G. (c) Raise the cap: the shells fail first; a contract constant and the fuzz bound change for no gain. (d) Shorter reports, the real fix: a whole-recording report is about 70,000 characters because it is one (part A); a find-the-moment report is 4,000 to 6,000.

**Recommendation.** (a), after one measurement through the pinned Claude Code (`2.1.284`): a synthetic heredoc and here-string of about 4, 8, 12, 16, 32 and 60 thousand characters in each shell, ten to twelve runs that need the maintainer's go because they spend allowance. Restate the closing evidence as "a report at the stated size passes both forms through the pinned client". `--session` in the forms is a grader change and is not recommended for `0.2.1`.

**To close.** The skill text and freeze; the measurement in the pull request; a CLI test that a prose-only and a block-only draft give the documented answers (none exists).

## #367: `candidates` on a long recording

**Problem.** The first call for a 17:34 recording ran over 120 seconds with no output; its 25 candidates reached only 01:39 (about 15 a minute, most `periodic_coverage`, one `visual_hash` on 7 of 25); on a 34:36 recording 13 ranges ended `candidate_budget_exhausted`.

**Today.** Source time is cut into 60-second windows sampled at 2 Hz. A call analyses the missing windows of the range **one after another**, at most 30 (`extend_visual_index_within`, `crates/vsift-application/src/visual.rs`; `Engine::candidates`, `crates/vsift/src/candidates.rs`), each one FFmpeg decode with two threads and admission weight 2 of the root's 4. **The windows are committed once, at the end of the call**: a deadline or Ctrl-C keeps those done, a client that kills the process keeps nothing. Nothing is printed meanwhile; the progress stage `analysing_video` is defined (`crates/vsift-domain/src/progress.rs`) but nothing emits it. P08 measured 29 to 67 media seconds per second on synthetic clips; the issue's call ran under 8.8 on a real file of unstated resolution and frame rate. A window keeps at most 32 candidates (`MAX_WINDOW_CANDIDATES`); the rest are dropped and counted, and no filter brings them back. `periodic_coverage` is added only for a 10-second cell where nothing changed.

**Options.**
- **(a) Skill: bound the call.** At most 5 minutes at a time (each call then commits its own windows: checkpointing at no code cost); with a transcript, derive frame times from it, as both agents already did. No surface. At the issue's rate 5 minutes is 34 seconds or more; one data point supports the figure.
- **(b) The brief view for candidates** (#365's mechanism): an item falls from 580 to 660 characters to 215 to 235 (measured on `candidates.json`); a page of 50 from about 31,000 to about 11,000.
- **(c) A filter** (`--reasons`, or one entry per distinct hash). M; the cursor is bound to the query (`candidate_query_digest`), so it must carry the filter. **Be sceptical of the hash form:** `visual_hash` is a 9 by 8 difference hash of a 128 by 72 picture (`difference_hash`, `crates/vsift-domain/src/visual.rs`) while changes are found on a 16 by 9 block grid, so typing can change the screen and not the hash; collapsing equal hashes would hide what a timeline wants. Dropping `periodic_coverage` loses the representative of a long static screen. Neither is known to help on a real recording.
- **(d) Progress events** for `candidates --events jsonl`. Additive, S to M. It shows a person that the call is alive; it does not lengthen a client's limit, and the skill's forms (`--json`, or the last line of `--events jsonl`) never show it to an agent.
- **(e) Parallel windows** (the root's 4 units allow two): up to half the wall time. L: core concurrency, capacity review, independent review.

**Recommendation.** (a) and (b). No (c), (d) or (e) until the maintainer has counted, on the two recordings, candidates per minute by reason and seconds per minute of video (counts and times, nothing from content). Register the open parts: analysis speed on real recordings; dropped candidates.

**To close.** (b) as in #365; (a) is text. The issue's calls-and-characters before and after for a 15 to 30 minute recording can come only from the maintainer's recordings; the pull request carries synthetic figures. **The skill says:** step 4's "page candidates over the whole video" becomes the 5-minute and transcript-first rules; `not_analyzed` still means "call again".

## #370: `frame get` is `BUSY` during a transcription

**Problem.** A frame asked for during a ten-minute range transcription was refused `BUSY`; in another session a frame succeeded during a `candidates` call. With rc.1's rule (wait, retry once, report a gap) a frame during a transcription always becomes a gap.

**Today.** The cause is predicted to be **the root's capacity, not the session**. A desktop root has 4 admission units (`DEFAULT_ADMISSION_CAPACITY`). A recognition reserves its threads, `min(machine threads, 8, capacity)` (`recognizer_threads`, `crates/vsift-domain/src/admission.rs`), so any machine with four or more threads gives it all 4 for the whole run. A frame needs 1 for its probe and 1 for its extraction (`SINGLE_STAGE_WEIGHT`, `FfmpegMedia::admit`), a `candidates` window 2 (`support-and-resource-profiles.md`, "Heavy-stage admission"). That fits both observations: 4 of 4 refuses a frame, 2 of 4 leaves room for one. I found no session lock a transcription holds exclusively for its run: it takes the session's shared hold, and the writer lock only to commit (ADR 0020, "Locks"). A test: start a transcription of session A and ask for a frame of session B in the same root; `BUSY` means the root. The answer already says a transcription "can hold the capacity for minutes" (`MEDIA_BUSY_REMEDIATION`). L-041 (a slow root creator) and L-131 (a try-only lock when sessions open) are other mechanisms and do not explain this.

**Options.**
- **(a) Skill and register.** After a client times out a transcription (the usual way an agent runs anything else during it), a `BUSY` frame is not retried: read `job status` once; if the job runs, stop visual work, write the partial report with the resume card (`next_command`: `job status <job>`) and tell the user it is still running. A real gap, honestly named. S.
- **(b) Name the job.** On contention look up the session's jobs (`Engine::session_jobs`) and put the live job in `error.affected_ids` with a specific remediation. Additive, S. It cannot name another session's transcription, and the answer already says the essential thing.
- **(c) Let frames run during a transcription.** (c1) The recognition leaves one unit: thread count is part of every run's provenance and revision identity (L-023), so recognition keys change, an unfinished job of an earlier release cannot resume, and a 4-thread machine is up to a quarter slower. (c2) A fifth unit for new roots only: capacity is immutable per root, existing roots do not change, and the recognizer's cap moves with it. (c3) A reserved evidence slot outside the capacity: a changed ceiling, which `support-and-resource-profiles.md` says needs "security and capacity review plus an ADR update". L; independent review mandatory.

**Recommendation.** (a); (b) only if the maintainer wants the job named. Register (c) as the reason; design it after the release, (c3) likeliest.

**To close.** The issue asks for a test that pins the decided behaviour. For (a), the existing `BUSY` contract test plus the skill text, and **one engine test** (a recognition holding all units makes a frame and a `candidates` window `BUSY`; one unit left lets the frame through) so the register entry rests on a test. **The skill says:** the `BUSY` row of `commands.md` (+2 lines) and a sentence in `resume.md`.

## #374: sessions made in a packaged desktop client (not verified)

**Problem.** Every image path VSift returned in the Claude desktop app on Windows lay under `%LOCALAPPDATA%\Packages\<package>\LocalCache\Local\VSift-sessions\`, not in the documented `%LOCALAPPDATA%\VSift-sessions`. Whether `session list` and `session clean` in an ordinary terminal see those sessions is not known.

**Today.** The root comes from the `LOCALAPPDATA` variable alone (`platform_session_root`, `crates/vsift-infrastructure/src/session_root.rs`). Returned paths come from the root **as VSift canonicalised it** (`fs::canonicalize` in `root.rs`, then `delivered_path.rs`), so they show the location the file system reports. The repository says nothing about packaged clients. From general knowledge of Windows, not from this repository and not checked: a packaged process and its children can have writes to `%LOCALAPPDATA%` redirected into the package's `LocalCache`, which would fit the paths and hide the sessions from a normal terminal. If the client instead sets `LOCALAPPDATA` itself, the effect for a user is the same.

**Cheapest verification** (maintainer, five minutes, no private video). In the desktop client have the agent ingest `fixtures/corpus/generated/F01.mp4` (76 KB, synthetic) and run `vsift session list --json`; note the id. In an ordinary terminal run `vsift session list --json` and look in `%LOCALAPPDATA%\VSift-sessions` and `%LOCALAPPDATA%\Packages\*\LocalCache\Local\VSift-sessions`. Not listed and present in the package folder: confirmed. Listed: nothing to do.

**Options if confirmed.** (a) **Documents only:** the install guide's uninstall table and the cleanup page say that a client installed as a package may keep sessions in its own cache, where to look, and that `vsift session clean` run from inside that client removes them. S. (b) A path-free member in `setup check` or `session list` (`session_root`: `per_user_cache`, `explicit`, `redirected`), from comparing the canonical and the configured root without `unsafe` (the workspace forbids it); JSON never names the root's path today. Additive, M. (c) Move the default root: a documented location changes and existing sessions are orphaned. L; not for `0.2.1`.

**Recommendation.** Verify; if confirmed, (a) now and (b) later. Either way a register entry that says what was and was not shown (a hosted runner cannot show packaging). No skill change.

## #369: `ingest` of a path that names no file

**Problem.** `vsift ingest <a name that is not a file> --json` answers exit 7, `STORAGE_IO`, `remediation: []` (reproduced). The skill says to stop and report the code, so the commonest first mistake ends an investigation.

**Today.** `open_source` (`crates/vsift-infrastructure/src/source_snapshot.rs`) fails with `Io(NotFound)`; `open_session_error` turns every `Io` that is not "out of room" into `OpenSessionError::SourceIo` (`crates/vsift-application/src/session.rs`), which `open_session_failure_code` maps to `STORAGE_IO` (`crates/vsift/src/error.rs`) with no remediation. The link and no-room cases show the pattern: their own variant, the same code, a fixed-prose remediation (`SOURCE_IS_LINK_REMEDIATION`, `crates/vsift-cli/src/lib.rs`). Also reproduced: the failed call **still creates the session root**, because the engine provisions it before it looks at the source; harmless, but a typo is the first write to the profile.

**Recommendation.** A `SourceNotFound` variant for a missing file or folder, the code and exit status unchanged (an older release gives the same answer without the text), and a remediation that says no file is at that path, that a relative path is read from the folder the command runs in, and to check the name or give the full path (no path echoed). S. Reordering to check the source before provisioning changes a documented order: leave it. **To close:** a contract test for the remediation, the code and exit status, a human-output snapshot, a frozen example. The skill's `STORAGE_IO` row already says to read the remediation.

## #371: `session list` pages

**Problem.** Three sessions took four calls, one item a page, and the skill's allowed forms have no `--cursor` for `session list` (or for `session clean --expired --dry-run`).

**Today.** The index has 256 hash buckets; a page is the first non-empty bucket and the cursor is the next bucket number (`list_sessions`, `first_occupied_page` in `crates/vsift/src/sessions.rs`; `scan_session_bucket` in `crates/vsift-infrastructure/src/filesystem_session_store/index.rs`). With sessions in random buckets, N sessions take about N calls plus one.

**Options.** (a) Skill: both cursor forms and "continue until `next_cursor` is null", as `lifecycle.md` already says for clean. S. (b) Fill a page: scan on to at least 20 entries or the end, the cursor staying a bucket number. Additive, but it changes `cli-v1.md`'s "a page has at most 256 registrations", also applies to `session clean`, and touches the store scan: M, independent review. **Recommendation:** (a); (b) only if people keep many sessions. **To close:** the skill text and freeze, and a CLI contract test with sessions in distinct buckets that reads every page to null (none exists).

## #372: new ids after a range retranscription

After `transcript retranscribe --from --to` every carried segment has a new `segment_id` with `carried_from` naming the old (`cli-v1.md`, "Revisions"). **An older id still resolves:** the contract says so, `handoff_session_records` (`crates/vsift/src/handoff.rs`) adds every segment of every committed revision, and `retain_bundle` copies the session's whole artifact list; so `handoff check --session` and a retained bundle accept it. No test pins that. **The risk is meaning, not acceptance:** for a segment the run replaced, the old id points at the text just judged wrong. **Recommendation:** skill only, two sentences (step 6 and `handoff.md`, Citations): after a range retranscription read the range again and cite the newest revision's ids. **To close:** the text, and a test with two revisions in which `--session` resolves an id of the first.

## A. #368 is a scope decision: whole-recording questions

rc.1's description (not yet published) says the skill is for "a cited summary, timeline or handoff"; its budgets, the report's limits and its sections are those of "find the moment". The published `0.2.0` description named what a recording shows or says, where something happens, or a bug report; rc.1 widened it.

**What "in scope" would require.**

| Part | What | Size |
| --- | --- | --- |
| Procedure | A path in a new `references/survey.md` and a pointer in `SKILL.md`, overriding "always search first" and "never page the whole transcript" for this question only: read all pages in the brief view (4 calls for 171 segments), cut chapters, one frame per chapter (a frame is two tool calls, the command and the open), `candidates` only for ranges of interest | M |
| Budget | No new profile: `standard` (80 calls, 24 images) holds it; the issue's run used all 24 images on nine screens; I estimate 40 to 50 calls for 17 minutes. `compact` (6 images) cannot | text |
| Handoff limits | `handoff.schema.json` allows 50 claims, 100 citations, 10 per claim; the run used 48 claims and cited 77 of 171 segments, and a 34-minute lecture needs more. Additive options: a `transcript_range` citation type (one entry for a run of segments, resolvable at both ends) or higher maxima. An older binary embeds the old schema and refuses such a report (only if a user kept an older `vsift` with a new skill). The check's 64 KiB cap and the shells' limits (#366) mean a whole-recording report must stay near 20,000 characters, about 25 to 30 claims | M |
| Report shape | A prose template "Overview, Timeline, Evidence, Gaps, Untrusted, Lifecycle" instead of the fault sections (`handoff check` does not enforce the eight headings, so this is a skill convention); `claims[].section` stays `context` or gains `timeline` (an additive word) | S |
| Resume | The card holds 4 findings and 8 ids; a cut-short survey cannot say what it covered | note |
| Measurement | **The harness cannot do it today:** the corpus is synthetic and short (the longest trial video is F02 looped to 8:14), real recordings are private, a scenario names truth by manifest events. Either (i) a generated 15-minute narrated fixture with about ten scripted screens (the Kokoro tooling of `fixtures/corpus/README.md`), a manifest, a scenario, a new freeze and probably `-AllowGraderChange`: L; or (ii) the maintainer's manual runs on private recordings, a count-only record (status, valid check, calls, images, time) with the maintainer judging correctness: never committed evidence; or (iii) #363's free recordings, "the longest perhaps 10 to 15 minutes", which may fall short of 15 | L |

**Cost of a run (arithmetic, not measured).** Reading 171 segments is about 190,000 characters (about 48,000 tokens) in the full form and about 33,000 (about 8,000 tokens) in the brief form: the brief view is what makes the question affordable at all. The observed 510,000 tokens are cumulative over a 99-minute session.

**What "not in scope" would require.** The `description` (limit 1,024 characters) says what the skill is for and that a summary or timeline of a whole recording is beyond its budgets; a three-line limit in `SKILL.md` and `budgets.md`: for such a question say so, offer the stretch the user names, and stop; `docs/agents/skill.md`. S. It restores what `0.2.0` users already have.

**Recommendation: out for this line, in for the next.** In costs a new procedure, a schema change and a fixture that does not exist, in a release whose purpose is a high-severity fix, and the new text would ship with no scenario that exercises it (batches 2 and 3 re-run on the old scenarios only). Out costs a stated limit: the question people ask first is narrowed. The trial run did not mislead (it reported `partial`, with a resume card, though everything had been read); it was badly shaped and expensive. The brief view and `transcript_coverage` of this release are the foundation of the next line's survey path, so nothing built now is wasted.

## B. Ordering and splitting

| Pull request | Findings | Files it owns | Parallel | Independent adversarial review |
| --- | --- | --- | --- | --- |
| 1 Reads | #365, #373, brief candidates of #367 | `vsift-contract` (`transcript.rs`, `search.rs`, `candidates.rs`), `vsift-cli` (`command.rs`, `session.rs`, `search.rs`, `candidates.rs`), `schemas/v1`, `cli-v1.md` | yes | **yes**: #373 decides what an agent believes about gaps, and the brief view must never reach a stream or bundle |
| 2 Answers | #369, #370's text if wanted, #374 documents | `vsift-application/src/session.rs`, `source_snapshot.rs`, `vsift/src/error.rs`, `vsift-cli/src/lib.rs`, a remediation constant, a snapshot, `install.md`, the cleanup guide | yes | no: text and one error variant, no core behaviour |
| 3 (only if built) | #371 page filling, #367 filter | `sessions.rs`, `index.rs`; `visual.rs` paging | yes | yes: both change an existing command's pages and cursor |
| 4 The cut | the skill, one freeze, the version, the plan, runbook 6.15, changelog | `skills/vsift`, `fuzz/seeds/handoff_check/SKILL.md` (a byte copy), `docs/agents/skill.md`, `committed_freeze` and the freeze files | **after 1 to 3** | an independent read of the skill diff against the CLI; `freeze check` |

Every pull request also touches `cli-v1.md`, the `[Unreleased]` changelog, `known-limits.md` and the two memory files (AGENTS.md: each change rewrites them), so rebases conflict on those four, not on code. The skill cannot go first: its guard (`skill_contract`) fails any command the CLI does not parse. Prepare it on a branch while 1 and 2 are reviewed, as `skill-0.2.1` was, and apply it in the cut.

**Rebuilds.** `main` requires an up-to-date branch, so each merge restarts every other open pull request's CI (about 40 minutes). K merges in sequence cost K(K+1)/2 runs and at least K × 40 minutes whatever the order: 1, 2 and the cut give 6 runs and 120 minutes; five pull requests give 15 and 200. The lever is fewer pull requests, not an order. CI runs only for pull requests into `main` (`ci.yml`), so an integration branch would give review without CI until one final pull request. I recommend 1 and 2 in parallel, merged as each goes green, then the cut, and a closed list: anything not agreed now goes to rc.3 or `0.2.2`.

**Register entries to write with the code** (next free number L-149 at `cb8cb3a`): the capacity cause of #370; shell limits and the check's cap (#366); analysis speed and dropped candidates on real recordings (#367); whole-recording scope (#368); the packaged-client facts (#374); each with what was and was not shown.

## C. The version number

**Facts.** ADR 0024 decision A makes `0.2.0` the R0 version and promises "the command-line grammar, exit codes and v1 JSON (additive-only since 0.1.0) and nothing else"; it does not separate a minor from a patch for an additive change, and `cli-v1.md` ("Configuration and compatibility") says only that within v1 readers ignore additive members and producers do not reinterpret or remove one "without a new major". `SECURITY.md` names "the newest `0.2.x`" and says fixes arrive as "a new patch version of the line (`0.2.1` and so on)". `CHANGELOG.md` says the project follows Semantic Versioning, under which `0.y.z` may change at any time and, from 1.0, backward-compatible additions are a minor. **`0.2.1-rc.1` already adds optional members (`unusable_chunks`, `untranscribed_ranges`, a warning code) and a stored checkpoint kind, and a session that holds a gap cannot be read by `0.2.0`:** additive growth was already accepted as a patch, once. This set adds one option on up to three commands, two optional members and two or three schemas; the default answer of every command for every input is unchanged except `transcript get` over a revision with an uncovered range. A `0.3.0` would start a new candidate series (`0.3.0-rc.1`) and change every document that names `0.2.1` or "the newest `0.2.x`" (the plan, runbooks 6.14 and 6.15, the `SECURITY.md` row, the guide's marker, the changelog); the evidence re-runs do not depend on the number. I did not check how the four npm packages pin one another, which decides whether a caret range on `0.2` would reach `0.3.0`.

**Recommendation (not a decision).** Stay `0.2.1`: the project's rule treats additive as one line, rc.1 set the precedent, and a new number costs documents and tells a user nothing they need. **I would change my mind** if the set changes an existing command's answer for an existing input beyond #373: a new default, a page filling that moves a documented bound (#371 (b)), a removed word.

## D. What changes in the skill overall (the skill agent's brief)

One commit, last, with one new freeze, on the code that merged. `SKILL.md` is **exactly 300 lines** (`MAX_SKILL_LINES`); the references have no cap.

| # | Change | Where | Net lines in `SKILL.md` |
| --- | --- | --- | --- |
| 1 | Description and limit per the decision on #368 (out: wording and a 3-line limit) | front matter, REPORT or rules | 0 to +3 |
| 2 | The brief view in the `transcript get` and `search` forms; page rule 20 for the full view, 50 for brief | step 3, `commands.md` forms, `budgets.md` | +1 |
| 3 | Coverage sentence true for both commands; a `partial` read is usable | step 3 | 0 |
| 4 | `candidates` in ranges of at most 5 minutes; with a transcript derive frame times from it; brief view | step 4 (replaces the "whole video" bullet) | 0 to +2 |
| 5 | After a timed-out transcription a `BUSY` frame is not retried: `job status`, then a partial report with the card | `commands.md` `BUSY` row, `resume.md`, a clause in step 2 | +1 |
| 6 | Cite the newest revision's ids after a range retranscription | step 6, `handoff.md` | +2 |
| 7 | Report size: at most 64 KiB, a size known to pass, two parts for a longer one (literal forms unchanged) | REPORT, `commands.md` "Checking the draft" | +2 |
| 8 | `session list --cursor` and `session clean --expired --dry-run --cursor` forms; "until null" | `commands.md`, `lifecycle.md` | 0 |
| 9 | The copy of `SKILL.md` in `fuzz/seeds/handoff_check/`; the "change for 0.2.1" section of `docs/agents/skill.md`; `freeze write`, the `committed_freeze` pin and its comment | outside the skill | 0 |

**Size.** About +6 to +11 lines gross against 0 of room. Candidates to compact without dropping a rule: the REPORT "Before you send" checklist (nine lines that `safety.md` and `handoff.md` repeat), the handoff-check paragraph (eight lines that `commands.md` repeats) and the PowerShell quoting tail of "Reading results and errors" (three lines that belong in `commands.md`). Each compaction risks the trial results, so it needs the batches. If #368 were in scope: about 6 more lines in `SKILL.md`, a `references/survey.md` of about 70 lines and about 25 in `handoff.md`; that probably exceeds what can be recovered, which makes the 300-line bound itself a decision.

**What "closed" means for a skill-only finding.** AGENTS.md closes a finding on implementation plus a regression test. For #366, #367, #370, #371 and #372 the implementation is text: the guard (`skill_contract`) tests that its commands parse and its vocabulary matches, and batches 2 and 3 re-run, but no scenario exercises a long report, a long recording, a `BUSY` frame, a session list or a range retranscription. I would close them as "text shipped, behaviour unmeasured" and leave the register entry open until the maintainer's own real run shows an agent following the sentence.

## E. Open questions, in the order they block work

1. **#368: in scope for this line, or out?** (in / out) Blocks the skill brief, #366's remedy and any schema change.
2. **Brief view: build it for `transcript get`, `search` and `candidates`?** (yes / no) Blocks pull request 1.
3. **`transcript get` over a gap: `partial` like `search`, or only the new member?** (partial / member) Blocks pull request 1's contract.
4. **How many code pull requests?** (two / each) Two means 1 and 2 above.
5. **Version: `0.2.1` or `0.3.0`?** (0.2.1 / 0.3.0) Blocks the cut's documents, not the code.
6. **Memory files: rewritten in each pull request as AGENTS.md says, or once in the cut?** (each / cut) Decides how many rebases conflict.
7. **#366: restate the closing evidence, and go for the ten to twelve measurement runs?** (restate / keep) and (go / wait).
8. **#367: measure on your recordings first, before any filter or parallel windows?** (measure / build)
9. **#370: frames during a transcription after the release?** (later / now) "Now" is an L change with a capacity review.
10. **#371: fill pages?** (skip / build)
11. **#374: will you run the five-minute check before the cut?** (yes / no)
12. **If #368 is in: keep the 300-line bound?** (keep / raise)

## F. What I could not determine, and what it would take

- **The clients' real limits** for the two forms: a measurement through the pinned client, or by hand in each.
- **Why `candidates` is slow on the real recordings**, and their candidates per minute by reason: a time and a count by the maintainer on a 5-minute range of each; no content is read. The 34:36 recording's segment count (about 336) is extrapolated from 171 in 17:34.
- **Whether a packaged client redirects the session root** (#374): the five-minute check; a hosted runner cannot show it.
- **Whether the capacity prediction for #370 is right:** it needs whisper.cpp and a second session; I could not run a transcription.
- **Whether an agent reads a brief page of 50 to its cursor**, and how it follows the new text: one real run, counts only.
- **What the trial grader needs for a long, no-truth or `complete`-within-budget scenario:** I read the scenario format and `trials.md`, not the grader source; a long fixture may need `-AllowGraderChange`.
- **Whether `0.2.1-rc.1` is tagged and published:** the plan on `main` says it is cut and not tagged; the brief for this design calls the next candidate `rc.2`. Numbering and freeze history depend on it.
- **That a retained bundle holds every revision:** read from `retain_bundle`, not run; one test closes it (#372).
- **How the fix for #359 changes coverage:** if it records new gaps in the revision, `SearchCoverage` (and so #373) reports them; not designed here.
- **Sizes are from one committed example per command** (a 52-character segment; an imported transcript's items are 640 to 730 characters), not from real transcripts; real page sizes need the maintainer's two recordings.
