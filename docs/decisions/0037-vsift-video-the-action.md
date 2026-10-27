# ADR 0037: The action: `vsift <video>` (explicitly `vsift watch <video>`), files beside the video

- Status: **Proposed, 2026-10-27**, for the planner's review before any code. The shape of the action
  (the name, the bare-argument rule, the default outputs, the two flags, progress and the finish screen)
  is the maintainer's design of 2026-10-27; the name was ruled again the same day (decision 1). Every
  other choice is this record's proposal, listed with a recommendation under "Decisions for the planner
  and the maintainer". It is built in up to three pull requests (section 13); until each lands, what it
  describes is a plan, not a feature.
- Date: 2026-10-27
- Refines: [ADR 0002](0002-disposable-investigation-sessions.md) (sessions are disposable; persistence
  needs explicit intent), [ADR 0008](0008-cli-and-json-contract.md) (the command namespace),
  [ADR 0013](0013-retained-bundle-publication.md) (the one export that exists today),
  [ADR 0022](0022-agent-skill-and-named-client-qualification.md) (the skill's command policy) and
  [ADR 0033](0033-progress-during-long-transcriptions.md) (the `prepare` job, its modes, status and
  live line). It meets ADR 0036 (the setup door, W6f, pull request #537, not yet on `main`) at the
  `--include` vocabulary, the colour rules and the door's "Try it" line. Their text stays as accepted;
  section 12 says what this record changes in each.
- Tracking: the 0.3.0 story W6g, a late slice by the maintainer's ruling of 2026-10-27. Register
  entries: numbers to be assigned by the supervisor (section 14 lists the limitations).

## Context

A person with a recording, a meeting say, wants its text, perhaps to share with colleagues. Today that
takes four commands: `ingest`, then `job start` (or `transcript retranscribe`), then `transcript get
--from --to` with times in microseconds, then `search`. They copy a session id from one command to the
next, and everything is gone once the session expires after 24 idle hours. Those commands were designed
for an AI assistant and a script, and they stay. A person needs one action.

What exists (checked on 2026-10-27 in `main` at `f099ed5f`, by reading):

- **`ingest`** copies the video into a private session (the original is never changed) and names it by
  its content: the source id is `src_sha256_` and the video's SHA-256 (`cli-v1.md`, identities).
- **The `prepare` job** (ADR 0033, `crates/vsift/src/prepare.rs`) runs the visual stream first (the
  visual-candidate index, published in batches of ten 60-second windows), then a whole-source
  transcription that publishes as it goes in the watched mode. `Engine::job_start` records it and
  `Engine::job_resume` runs it in the calling process with a `ProgressObserver`; `job status` gives per
  stream the time done, the time published, the total, the speed and an estimate of the time left
  (null until two fresh chunks and 20 s have run). The quick mode (under ten minutes) publishes only at
  the seal; the watched mode publishes parts and leaves one admission unit free, so `frame get` works
  while it transcribes.
- **Reads of an open revision** (ADR 0033 section 3): `transcript get` shows the newest revision, open
  or not, marked partial with `pending_ranges`; `--since <token>` reads what was published after a
  token.
- **Identities and hashes.** A transcript revision is `trv_`, a segment `tsg_`, a visual index revision
  `vix_`, a screen change (visual candidate) `vcd_` with its `visual_hash`, a frame `evd_`. A frame's
  record carries its PNG's SHA-256, and its file is stored content-addressed in the session. Transcript
  and visual identities derive from content and the session; `evd_` from the session, source, stream,
  profile, the media tool's fingerprint and the exact frame timestamp.
- **The live line** (ADR 0033 section 5): one line on standard error, in a terminal only, updated at
  most once a second ("Transcribing 34:10 of 90:00 (38%), 2.1x real time, about 26 min left
  (estimate)").
- **The only export today** is `session retain` (ADR 0013): a new owner-private bundle directory at an
  absolute path the caller chose; an existing destination is refused.
- **The parser** (`crates/vsift-cli/src/command.rs`): clap, a closed set of top-level commands, `help`
  as a subcommand disabled. A rejected line answers `INVALID_ARGUMENT` with `command` `parse`, never
  echoing argument text in JSON (L-071, `parse_failure.rs`).
- **Bounds** that the action meets: the visual index covers at most four hours, a transcription plans
  at most 1,024 chunks, and a session holds at most 512 artifacts (ADR 0033 sections 1 and 8).

## Decision

### 1. The command and the name

`vsift watch <video> [--output <folder>] [--include <extra>]` is the explicit form. `vsift <video>` is
the same command, and the form people are shown (README, website, the door's "Try it" line, the finish
screen's examples). VSift itself is the verb.

**The name `watch` stays: the maintainer's ruling of 2026-10-27**, relayed by the planner. The options
and the reasons are recorded under decision 1, because the name tension in the messaging guide is real
and the identifier becomes a public, versioned name (`command: "watch"` in JSON v1) the day 0.3.0 ships.
How the word is used, so the command never reads as the product's promise:

- help describes what it gives, never "watches your video": `watch   Make a transcript, subtitles and
  the screen changes of a video, beside it (the same as  vsift <video>)`;
- public examples show the bare form; `watch` appears only where the explicit form is documented (the
  help and the command reference);
- the finish screen points to `vsift watch --help`, the help that exists today (`vsift help <command>`
  arrives with the help story, Part E3, which may change the line).

### 2. The bare-argument rule

Before the parser runs, the CLI looks at the first argument that is not a global option (`--json`,
`--events <format>`, `--session-root <path>`, `--host-isolation <value>`, in either `--name value` or
`--name=value` form, with the value-taking set read from the parser's own definition, not a copy):

1. It is a command name (any top-level subcommand, including `watch`): parsed as today. A command name
   always wins, so a file named `setup` in the current folder is watched with `vsift watch setup` or
   `vsift ./setup`.
2. It starts with `-`: parsed as today (an option, or `--help`/`--version`).
3. It names something that exists and is a file (by `symlink_metadata`, so a link is not followed here;
   `ingest` refuses it later with its existing `SOURCE_IS_LINK` remediation): the line is parsed as if
   `watch` stood before that argument.
4. It does not exist but **looks like a path**: it contains a path separator (`/`, and `\` on Windows),
   or ends in a dot and one to five letters or digits (`meeting.mp4`, `talk.webm`). Parsed as `watch`
   too, so `ingest`'s existing source-not-found answer names the real mistake (decision 2).
5. Anything else: parsed as today, so `vsift serach` stays an unknown command with the parser's nearest
   suggestion, never "file not found".

The rule only inserts the word; it never reorders, rewrites or echoes an argument, and JSON keeps
reporting a rejected line as `command` `parse`. A public video link joins rule 3 when online links ship
(ADR 0028, #407), and is out of scope here.

### 3. What it does, and the one engine path

`watch` is a front over the existing engine, like the setup door over `setup check`, `plan` and
`install`. In order:

1. **Checks what it needs, without probing tools** (design note C9). Video tools and speech recognition
   must be configured; the check reads the selected configuration and runs no tool, so it costs
   milliseconds, never `setup check`'s probe. Missing video or speech refuses before anything is copied
   or written: `MISSING_CAPABILITY`, the existing remediation, and in the readable view a last line
   `Next: vsift setup` (decision 6). A tool that is configured but broken is found by the run itself and
   reported with its existing code.
2. **Claims the output names** (section 5), before any work, so a name that is taken is refused at
   once, not after an hour.
3. **Ingests** the video into a new session (the private copy, as `ingest`).
4. **Starts the `prepare` job in the watched mode and runs it in this process**:
   `Engine::job_start` with `JobMode::Watched`, then `Engine::job_resume` with a progress observer.
   Watched rather than quick, whatever the length, because it publishes as it goes (the growing files
   of section 7) and leaves one admission unit free (the frames of step 5); in its own process rather
   than a detached runner, because the person is there and Ctrl-C must stop it. No second pipeline
   exists: the stream order, chunking, merge, admission and recovery are ADR 0033's.
5. **Writes the frames** of the screen changes as soon as the visual stream is done, while the
   transcription runs: one `frame get` per change at its representative time, copied out of the session
   (section 6).
6. **Writes the transcript files** as the revision grows (from the second pull request; before it, once
   at the seal), and finalises every file when the job ends.
7. **Closes the session** when everything is written (decision 4): the private copy of the video is
   removed then, not 24 hours later. An interrupted run keeps its session, so the same command continues
   it (section 8).

`watch` never prompts, so a script can use it; without a terminal it shows no bars (section 9).

**Where the code lives** (AGENTS.md's layering):

- **domain**: the output names derived from the video's name (section 5), and the readable time form
  (`00:12:34`), as values with their rules and tests;
- **application**: the `watch` use case, which orchestrates through ports: the existing session, job,
  transcript, candidate and frame use cases, and a new `WatchOutputs` port (claim, write a snapshot,
  finalise, release);
- **infrastructure**: the folder implementation of `WatchOutputs` (exclusive creation, same-folder
  temporary file and replace, frame copies) and the plain-text and SubRip writers, beside the existing
  SubRip reader in `transcript_sidecar`;
- **engine** (`vsift`): `Engine::watch(request, observer, cancellation)`, so a later host (a desktop
  app, R2's interfaces) gets the same action;
- **CLI**: the bare-argument rule, the flags, the terminal view and the JSON result. No rule lives
  there.

### 4. The default outputs

With no option, beside the video; with `--output <folder>`, in that folder. For `meeting.mp4`:

| File | What it holds |
| --- | --- |
| `meeting.txt` | The readable transcript: a short header, then one line per segment, `[00:12:34] text`. |
| `meeting.srt` | SubRip subtitles, which most video players pick up when the name matches the video's. |
| `meeting-vsift/` | The screen changes: one PNG per change and `screen-changes.txt`, their list. |

**The evidence character of the outputs (the planner's request).** Each file names what it came from,
so even the first run shows that VSift's output can be checked:

```
meeting.mp4: transcript by VSift 0.3.0, 2026-11-20 (local speech recognition, model base.en).
A machine transcription: check it against the recording before you rely on it.

Video SHA-256    3f9a0c...e21c   (all 64 characters in the file)
Transcript       trv_8c41f0...   in session ses_51d2a7... (closed when this run finished)

[00:00:04] Good morning, everyone. Let's start with the invoice numbers.
[00:00:11] The first one is 4407.
```

```
Screen changes in meeting.mp4, found by VSift 0.3.0: 14 changes, 14 frames.
Video SHA-256    3f9a0c...e21c
Visual index     vix_02b7e1...   in session ses_51d2a7...

Time        Frame                   Change          Frame evidence   Frame SHA-256
00:01:12    00-01-12.480.png        vcd_7f20c4...   evd_c19a3b...    9b1f2e...
00:04:55    00-04-55.040.png        vcd_a81d55...   evd_02f6de...    41c0a9...
```

- **Hashes outlive the session; ids do not.** The video's SHA-256 and each frame's SHA-256 can be
  checked by anyone, any time, with any hashing tool: the PNG beside the list is byte for byte the
  session's evidence artifact. The `trv_`, `vix_`, `vcd_` and `evd_` ids name items of the session,
  which the run closes; they say which run made the files, and the same video watched again gives a new
  session and new ids. The header says so in plain words, and nothing claims the ids can be checked
  after the run.
- **Segment ids are not on every line** of `meeting.txt` (decision 3): a person shares that file, and a
  `tsg_` on every line halves its readability. A segment is pinned by the revision and its time.
- **`meeting.srt` carries no header or ids:** a player shows every cue, and SubRip has no comments.
- **Text is written in its display form**, hidden and control characters made visible, as the readable
  views show it today: transcript text is untrusted (AGENTS.md). In SubRip a cue's text is one line,
  never holding a blank line or the `-->` arrow, so a segment can never forge a cue.
- **Gaps are stated, not hidden.** A part that was not transcribed appears in time order in
  `meeting.txt` as `[00:12:00-00:12:30] (not transcribed: <the reason in plain words>)`, and a video
  with no sound track says so in place of the lines (and writes no `.srt`). A video longer than the
  visual index's four hours lists screen changes for the first four hours and says so.

### 5. Names, and never overwriting

- **Names derive from the video's file name, which is untrusted.** The stem is the name without its
  last extension; the outputs are the stem plus a fixed suffix (`.txt`, `.srt`, `-vsift`). The stem is
  already a valid name in that folder, so nothing in it is rewritten, but the result must also be: if
  a name would exceed the system's component length, the stem is shortened at a character boundary
  (and the subtitles no longer share the video's name, which the finish screen says); a stem that ends
  in a dot or a space on Windows is trimmed of them. Printed names are written in their display form.
- **Never overwrite.** Each output is claimed by exclusive creation (create-new) before any work. If a
  name exists, nothing is written or copied: `INVALID_ARGUMENT` (the code `session retain` gives an
  existing destination; no new failure code), remediation "A file VSift would write already exists;
  VSift never replaces a file. Move it away, or write elsewhere with --output <folder>." The readable
  view names the file. The only exception is a run of the same video that this VSift left unfinished
  (section 8).
- **A folder that refuses** (read-only, no room, a cloud-synced folder that rejects the write): the
  claim fails before any work with `STORAGE_IO`, remediation "VSift could not write beside the video.
  Choose a folder you can write to with --output <folder>."
- **`--output <folder>`**: an existing folder, or a new one whose parent exists (created, never with
  its parents). It is canonicalised and refused if it lies inside VSift's session root or managed
  root, which VSift cleans automatically.
- **Permissions.** Unlike a retained bundle (ADR 0013, owner-private), the outputs take the folder's
  ordinary permissions, because a person writes them to share. Beside the video they are exactly as
  exposed as the video itself; `--output` chooses another exposure. The record says so, and so does the
  guide.

### 6. Screen changes and their frames

Every screen change the visual stream finds is listed in `screen-changes.txt`. Frames are written for
at most **200** of them (decision 7); above that, the 200 written are spread evenly over the video, the
list marks each change whose frame was not written, and the finish screen says "200 of 340 frames
written". The bound keeps the folder within a few hundred megabytes and the session within its 512
artifacts. Frames are named by their time (`00-01-12.480.png`, which sorts and has no `:` for Windows),
and copied from the session's content-addressed artifact, then checked against its SHA-256.

Whether screen changes are cheap enough to run by default was a care point of the design note. By ADR
0033's measurement the visual stream runs at about 29 to 67 seconds of media a second, so a 48-minute
meeting takes one to two minutes before the transcription starts. The first pull request measures it,
and the frame count, on the hosted real-media workflow (section 13).

### 7. Files that grow while it runs (the second pull request)

So that nobody is left in the dark on a long video, `meeting.txt` and `meeting.srt` grow as the
revision is published, and a person can open them any time:

- **Each update is a whole snapshot**, written to a temporary file in the same folder
  (`.meeting.txt.vsift-partial`) and moved over the claimed file, so a reader or a crash only ever sees
  a complete, valid file. Updates follow the published part (`--since` reads of the open revision), at
  most once every ten seconds.
- **While unfinished, each file says so.** `meeting.txt` starts with `IN PROGRESS: transcribed up to
  00:12:30 of 00:48:00. VSift is still working; this file is replaced when it finishes.`
  `meeting.srt` ends with one cue from the published time to the end of the video, reading `(VSift is
  still transcribing from here)`, so a person playing the video sees where the text stops. Both
  markers go at the final write.
- **A file held open by another program** (a Windows editor that locks it): the update is skipped and
  tried again at the next one. The final write tries for up to 30 seconds, then the run ends with
  `STORAGE_IO`, names the file, and leaves the last snapshot in place with its marker.
- The frames are written as soon as the visual stream is done (section 3, step 5), and
  `screen-changes.txt` once, when they are.

Before the second pull request, the files are written once, when the job ends, and the live line of
ADR 0033 is the progress.

### 8. Interrupted, and run again

- **Ctrl-C** cancels through the existing cancellation: the open revision is sealed `stopped` (ADR
  0033), the files keep their last snapshot and their in-progress marker, the session is kept, and the
  run ends with the `CANCELLED` exit status and the line `Stopped at 12:30 of 48:00. Run the same
  command again to continue.`
- **Run again** (decision 5): VSift hashes the video, as `ingest` does, and looks in its session root
  for an open session of that source with an unfinished `watch` job whose claimed outputs are exactly
  this run's names. The job keeps those names in its own record, in the session, so the decision never
  depends on reading a file in the person's folder. If one exists, it resumes that job (`job_resume`
  reuses every finished chunk) and keeps writing to the same files. If the session has expired, the
  unfinished files are replaced only when they still carry this VSift's in-progress marker; any other
  existing file is refused as in section 5.

### 9. What the person sees

**While it works** (in a terminal, on standard error, so standard output stays clean for a pipe): one
line per stream, from the `prepare` job's progress, redrawn at most once a second.

```
VSift  meeting.mp4  (48:00)

  Screen changes   ████████████████   done, 14 found
  Speech           ███████░░░░░░░░░   12:30 of 48:00   about 9 min left

  Writing as it goes: meeting.txt, meeting.srt (open them any time)
```

- The estimate is ADR 0033's (the measured speed of this run's fresh chunks), shown as "about N min
  left" and only once it exists; before that the line says "working out how long".
- Before the job starts, one line covers the private copy (`Copying meeting.mp4 into a private
  session`), which takes noticeable time for a large file.
- The first pull request shows ADR 0033's live line instead of the bars; the bars are the second.
- Symbols and colour follow ADR 0036 section 8 (and its implementation): one renderer, plain words for
  `NO_COLOR`, `TERM=dumb` and a non-terminal, and a status word always present.

**When it finishes** (standard output), the next step last:

```
Done in 11 min: meeting.mp4 (48:00)

  meeting.txt      readable transcript
  meeting.srt      subtitles (most video players pick them up)
  meeting-vsift/   14 screen changes with their frames

More: add on-screen text with  --include on-screen-text  ·  vsift watch --help
```

A partial result says what is missing in one line above the "More" line (`Not transcribed: 12:00 to
12:30, the speech tool failed there`). The "More" line names an extra only when that extra is
available here and not included.

**Without a terminal:** no bars; one plain line when each stream ends, then the same summary.

**With `--json`:** one result, command `watch` (additive to JSON v1: a schema, an example, contract
tests, `cli-v1.md` and the guide reference). Its data: `source_id`, `session_id` and whether it was
closed, `job_id`, `transcript_revision_id`, `visual_index_revision_id`, `outcome` (`complete` or
`partial`, with the not-transcribed ranges), and `outputs`: each written file's absolute path, kind and
SHA-256, and per frame its time, `candidate_id`, `evidence_id` and SHA-256. With `--events jsonl`, the
`prepare` job's existing progress records, then the terminal record. No existing command's JSON
changes, and **no new failure code** is introduced (each refusal above reuses one, with its own
remediation), so the failure-code freeze is untouched.

### 10. `--include <extra>`

The same word and the same person-facing names as `vsift setup --include` (ADR 0036), so there is one
vocabulary. For `watch` an extra adds evidence to the outputs:

- `on-screen-text`: reads the text on each written frame (ADR 0030's `ocr` over those frames' times) and
  adds it under the change in `screen-changes.txt`, quoted and in display form;
- `speakers`: once ADR 0031's group exists, speaker labels on the lines of `meeting.txt` and in the
  cues;
- `web-videos` is not an extra of `watch` (it is a kind of input) and is refused with a pointer to the
  bare-argument rule.

An extra that is not set up refuses before any work, `MISSING_CAPABILITY`, with `Next: vsift setup
--include <extra>` (design note C8). This is the third, separable pull request (decision 8).

### 11. What `watch` never does

- It never changes, moves or deletes the video, and never writes inside VSift's own folders.
- It never replaces a file it did not create in this run or an unfinished run of the same video
  (sections 5 and 8).
- It never prompts and never installs anything; a missing part points to `vsift setup`.
- It never keeps the session longer than the run needs: closed at the end, or left to expire after 24
  idle hours when interrupted.
- It never runs for an assistant: in 0.3.0 the skill keeps its session commands, and the command policy
  classes `vsift watch` (and so `vsift <video>`) as **never**, because it writes files into the
  person's folder (decision 9). Whether the skill may use it is a later skill decision (design note D2).

### 12. What this changes in earlier records

- **ADR 0002 and AGENTS.md (persistence needs explicit intent).** Typing the action is the explicit
  request for its files: the outputs persist because the person asked for them, while the session
  behind them stays disposable and is closed when the run ends. AGENTS.md's boundary holds as written;
  this record is where its application to `watch` is written down. The planner decides whether
  AGENTS.md gains a line pointing here.
- **ADR 0013.** A second kind of export: derived, readable files in a person's folder, with ordinary
  permissions, beside the owner-private evidence bundle of `session retain`, which is unchanged.
- **ADR 0008 and `cli-v1.md`.** A new top-level command, and a first argument that is an existing file
  (or looks like a path) is that command. `vsift serach` is unchanged.
- **ADR 0033.** The watched mode now also runs in the caller's own process, for `watch`. The engine
  allows it already (a runner is `job resume`); only the documentation of the modes gains the case.
- **ADR 0036.** The door's "Try it" line becomes `Setup complete. Try it: vsift ./your-video.mp4`, as
  that record anticipates, in the first pull request here.
- **ADR 0022.** One more never-class command.

### 13. Delivery, and the deadline rule

Each pull request stands on its own:

1. **The action**: the bare-argument rule, `watch` with `--output`, sections 3 to 6, the files written
   once at the end, ADR 0033's live line as progress, the finish screen, the JSON result, the skill
   policy and the door's "Try it" line, with the records. **This slice alone qualifies `vsift <video>`
   for 0.3.0**: the README switch at the 0.3.0 release records depends on it only.
2. **Growing files and the richer progress**: sections 7 and 8 and the bars of section 9.
3. **`--include on-screen-text`** (and `speakers` if ADR 0031's group has landed): section 10.

**Qualification evidence:** two dispatches of the hosted `P14 real media` workflow on the branch, before
and after: RM03 (19 minutes, speech) and RM01, RM04 or RM10 (screen recordings), running `vsift <video>`
by the built binary's absolute path and checking the outputs' presence, headers, hashes against the
files, frame counts and the visual stream's time. Nothing long runs on the development machine.

**If the first pull request is not qualified at the cut**, `watch` opens the next release and the
README keeps `vsift ingest`. If the second or third is not, it moves alone, and the first ships with
end-of-run files and the live line.

### 14. Tests

- **Domain**: names from untrusted stems (long names, a trailing dot or space, Unicode, a stem that
  is all extension), the readable time form, and the SubRip cue-text rule (no blank line, no arrow).
- **Application**, over fakes of the ports: the order of the steps; a refusal before any work (a
  missing capability, a name taken, a folder that refuses) leaves nothing written and no session; a
  failed transcription chunk becomes a stated gap; cancellation keeps the files and the session;
  continuing an unfinished run; the frame bound and its even spread.
- **Infrastructure**: exclusive claims, snapshot-and-replace (and a replace refused because the target
  is held open, on Windows), the writers against golden files, frames checked against their SHA-256.
- **CLI**: the bare-argument rule (each of section 2's five cases, global options before the video,
  `--name=value`, a file named like a command), golden files for the terminal views (complete, partial,
  refused, interrupted, no terminal, `NO_COLOR`), and the JSON contract test for `watch`.
- **Binary**: on the synthetic fixtures (the 14-second clip and the screen-change fixtures), on all
  three systems: `vsift <video>` writes the three outputs beside a copy of the fixture, a second run is
  refused, `--output` works, and no session is left open.
- **Policy**: the skill-policy test with `watch` in the never class (a skill text change: a draft freeze
  in the pull request, rewritten at the merge rebase, `v030-rules.md`).
- **Limitations to register** (numbers from the supervisor): the frame bound (section 6); screen changes
  only for the first four hours; ids that name a closed session (section 4); a file held open on Windows
  (section 7).

## Decisions for the planner and the maintainer

Each has a recommendation. Decision 1 is the maintainer's and already ruled; the others are proposed for
the planner, none changing an existing JSON v1 field, failure code or safety rule.

1. **The name `vsift watch`.** The messaging guide's tension: the group VSift must not be filed with
   includes plugins literally called `watch`, and "watch" is on its list of words to avoid as a lead.
   Options: (a) keep `watch`; (b) another explicit name before this record is accepted (`run`, `go`,
   `take`, `process`, `extract`, `evidence`; the design note already set aside `ingest`, `transcribe`,
   `look`, `view`, `review` and `sift`); (c) no explicit form at all, only `vsift <video>`. **Ruled by
   the maintainer on 2026-10-27, relayed by the planner: keep `vsift watch`.** **Recommendation: keep
   it**, with section 1's usage rules. The bare form dominates every public example; `watch` is the
   ordinary meaning of what a person asks for; and it is the natural name for ADR 0033 section 8's
   future push delivery from a live source (a `watch` that never ends, streaming `--events`), so it is
   one command growing, not a second meaning. The cost of changing it is nil now and a public-API
   change after 0.3.0.
2. **A missing file that looks like a path** (section 2, rule 4). Without it, `vsift meetng.mp4` (a typo)
   answers "unknown command". **Recommendation: treat it as `watch`,** so the answer is "the video was
   not found"; a bare word with no separator or extension (`serach`) stays an unknown command.
3. **Where the ids and hashes go** (section 4). (a) A header in `meeting.txt` (the video's SHA-256, the
   revision and session ids) and per-change ids and SHA-256 in `screen-changes.txt`; (b) also a `tsg_`
   on every transcript line; (c) ids only in `--json`. **Recommendation: (a)**, with every id and hash in
   the `--json` result as well; (b) costs the readability the file exists for.
4. **The session at the end** (section 3, step 7). Close it, or keep it for 24 idle hours so a person
   can explore it with the session commands. **Recommendation: close it** on success (the private copy
   of the video is freed at once, and exploring after the run is deferred, design note D7); keep it when
   interrupted, so the run can continue. A later `--keep-session` can come with the learning phase.
5. **Running it again** (section 8). (a) Always refuse existing files; (b) continue an unfinished run of
   the same video, found through VSift's own records, and replace only VSift's own unfinished files;
   (c) number new names (`meeting (2).txt`). **Recommendation: (b), in the second pull request**, since
   a long video is exactly the case that gets interrupted; until then, (a).
6. **Speech not set up.** (a) Refuse before any work, `Next: vsift setup`; (b) produce the screen changes
   only and say speech was skipped. **Recommendation: (a)** for 0.3.0: the person asked for the text,
   and a run that silently lacks it is worse than a clear "set up speech first". A screen-changes-only
   run can be a later flag if trial feedback asks for it.
7. **The frame bound** (section 6): **Recommendation: 200 frames, spread evenly over the video when
   there are more**, every change listed, the figure confirmed or changed by the first pull request's
   measurement on the screen recordings.
8. **`--include` for `watch`** (section 10): **Recommendation: the third pull request, droppable at the
   cut**, with `on-screen-text` first; the flag is refused as unknown until it lands, which is additive
   to add later.
9. **The skill's class for `watch`**: **Recommendation: never in 0.3.0** (section 11). It is a skill
   text change and so moves the trial freeze, handled under the 0.3.0 freeze rule.
10. **AGENTS.md.** **Recommendation: add one line** under "Product boundaries" pointing here: "A person's
    explicit `vsift <video>` writes derived files they asked for; the session behind it stays
    disposable (ADR 0037)." The rule itself is unchanged.

## Consequences

- A person gets a transcript, subtitles and the screen changes with one command and no ids to copy, and
  can open the text before a long video is done.
- The outputs show where they came from: the video's hash, the run's ids and every frame's hash, so the
  first thing a person sees from VSift is checkable.
- One engine path: `watch` adds no pipeline, only a use case over the existing session, job, transcript,
  candidate and frame use cases, and a writer of files.
- VSift writes outside its own folders for the first time without a path the caller typed. The claim,
  the never-overwrite rule and the refusal of VSift's own folders bound it.
- The first argument becomes slightly overloaded; command names always win, and the rule is tested case
  by case.

## Alternatives

- **`vsift ingest` printing a transcript.** Rejected: `ingest`'s JSON and behaviour are the assistant's,
  and a person needs files, not a session id.
- **A transcript only, no frames.** Set aside by the maintainer's design: the screen changes are what
  sets VSift apart from a plain transcriber, and by measurement they are cheap.
- **Writing into a bundle (`session retain`) by default.** Rejected: a bundle is for checking evidence
  later with VSift, not for reading or a video player; it stays the explicit export it is.
- **A background runner, as `job start` uses for a long recording.** Rejected for `watch`: the person is
  at the terminal and Ctrl-C must stop the work; the watched mode's behaviour is kept by running it in
  the caller's process.
- **Overwriting with a confirmation question.** Rejected: `watch` never prompts, and VSift never
  replaces a person's file.
