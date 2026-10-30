# ADR 0037: The action: `vsift <video>` (explicitly `vsift watch <video>`), files beside the video

- Status: **Accepted by the planner, 2026-10-27**, at `44ca5102`, relayed by the coordinator:
  decisions 2 to 10 as recommended, with three conditions written into sections 2, 7 and 8 (marked
  "planner's condition"). Decision 1, the name, is the maintainer's: their ruling of 2026-10-27 keeps
  `watch`. The
  shape of the action (the name, the bare-argument rule, the default outputs, the two flags, progress
  and the finish screen) is the maintainer's design of 2026-10-27. It is built in up to three pull
  requests (section 13); until each lands, what it describes is a plan, not a feature.
- Date: 2026-10-27
- Refines: [ADR 0002](0002-disposable-investigation-sessions.md) (sessions are disposable; persistence
  needs explicit intent), [ADR 0008](0008-cli-and-json-contract.md) (the command namespace),
  [ADR 0013](0013-retained-bundle-publication.md) (the one export that exists today),
  [ADR 0022](0022-agent-skill-and-named-client-qualification.md) (the skill's command policy) and
  [ADR 0033](0033-progress-during-long-transcriptions.md) (the `prepare` job, its modes, status and
  live line). It meets ADR 0036 (the setup door, W6f, pull requests #537 and #538, both on `main`) at the
  `--include` vocabulary, the colour rules and the door's "Try it" line. Their text stays as accepted;
  section 12 says what this record changes in each.
- Tracking: the 0.3.0 story W6g, a late slice by the maintainer's ruling of 2026-10-27. Register
  entries [L-234](../planning/known-limits.md#l-234) (the frame bound),
  [L-235](../planning/known-limits.md#l-235) (screen changes for the first four hours only),
  [L-236](../planning/known-limits.md#l-236) (ids that name a closed session) and
  [L-237](../planning/known-limits.md#l-237) (a file held open on Windows), each added by the pull
  request that builds what it limits.

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
3. It names something that exists (by `symlink_metadata`, so a link is not followed here): a file, a
   folder or a link. The line is parsed as if `watch` stood before that argument, so `ingest`'s existing
   refusals answer: a folder is "a folder, not a video" (`SOURCE_IS_FOLDER`), a link `SOURCE_IS_LINK`
   (a folder: the planner's condition C, 2026-10-27, so that it is never "unknown command").
4. It does not exist but **looks like a path**: it contains a path separator (`/`, and `\` on Windows),
   or ends in a dot and one to five letters or digits (`meeting.mp4`, `talk.webm`). Parsed as `watch`
   too, so `ingest`'s existing source-not-found answer names the real mistake (decision 2).
5. Anything else: parsed as today, so `vsift serach` stays an unknown command with the parser's nearest
   suggestion, never "file not found".

**Implementation note, the first pull request (2026-10-28, the planner's ruling):** rule 4 never applies to
text that holds a control or hidden character; such an argument falls to rule 5 and stays the parser's to
reject, inert. SEC-T02's hostile-argument test of the readable output (`sec_t02_human_output`) found that its
sentinel, given as an unknown command, holds a terminal link to `https://...`, whose `/` would otherwise have
made it a `watch` of a missing file. A person never types a path with such a character. Pinned by the CLI
routing test `a_mistyped_command_stays_a_mistyped_command` and by `sec_t02_human_output` through the binary.

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
meeting.mp4: transcript by VSift 0.3.0, 2026-11-20 14:05 UTC (local speech recognition, model base.en).
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

- **The header's time is UTC**, to the minute, and says so: a file shared with colleagues in other
  places must not carry a time whose zone the reader has to guess.
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
at most **180** of them (decision 7, lowered from 200 by the first pull request's implementation note); above
that, the frames written are spread evenly over the video, the list marks each change whose frame was not
written, and the finish screen says how many were written. Each frame is a `frame get`, which takes two of
the session's 384 evidence slots (the PNG and its record), within its 512 artifacts: 180 frames take 360, and
the room is measured before the first frame, so a run never asks for more than the session holds.

**Correction, 2026-10-28 (the first pull request's "after" run).** This section said the bound of 200
kept the session "within its 512 artifacts". That was wrong: evidence has its own cap of 384 artifacts
(`MAX_EVIDENCE_ARTIFACTS`, ADR 0019 D4 and ADR 0020 D-2) within the 512, and each frame takes two of them,
so 200 frames needed 400 and RM03's run failed on its 193rd frame with `RESOURCE_LIMIT`. The bound is 180,
fitted to the measured room; should the session still refuse a frame for room, the frames stop there and
the run ends `partial` (`frames_stopped_for_room`), never failed. **For the third pull request:** if
`--include on-screen-text` stores evidence, it counts against the same 384, so that pull request reserves
its room before any work starts, beside the frames'. Frames are named by their time (`00-01-12.480.png`, which sorts and has no `:` for Windows),
and copied from the session's content-addressed artifact, then checked against its SHA-256.

Whether screen changes are cheap enough to run by default was a care point of the design note. By ADR
0033's measurement the visual stream runs at about 29 to 67 seconds of media a second, so a 48-minute
meeting takes one to two minutes before the transcription starts. The first pull request measures it,
and the frame count, on the hosted real-media workflow (section 13).

### 7. Files that grow while it runs (the second pull request)

So that nobody is left in the dark on a long video, `meeting.txt` and `meeting.srt` grow as the
revision is published, and a person can open them any time:

- **Each update is a whole snapshot**, written to a temporary file in the same folder and moved over
  the claimed file, so a reader or a crash only ever sees a complete, valid file. **The temporary file
  is created exclusively (create-new), with a random part in its name**
  (`.meeting.txt.<random>.vsift-partial`), and is never opened through a link: an existing name or a
  link there fails the update rather than writing through it. The replace targets the claimed name
  only, never a name read back from the folder (the planner's condition B, 2026-10-27; tested with a
  name taken in advance and with a link planted at the temporary name). Updates follow the published part (`--since` reads of the open revision), at
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
  reuses every finished chunk) and keeps writing to the same files. **If the session has expired, the
  files are refused as in section 5**, never replaced because their text carries the in-progress
  marker: anyone can write that text, and the decision must never depend on reading a file in the
  person's folder (the planner's condition A, 2026-10-27, the planner's recommendation of the two
  options it gave). The remediation then says these look like an unfinished VSift run, names the files
  to move away, and suggests `--output`.

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
  that record anticipates, in the first pull request here. ADR 0036's second pull request touches the
  same line; whichever lands second keeps the other's change.
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
- **CLI**: the bare-argument rule (each of section 2's five cases, an existing folder answering
  `SOURCE_IS_FOLDER`, global options before the video, `--name=value`, a file named like a command), golden files for the terminal views (complete, partial,
  refused, interrupted, no terminal, `NO_COLOR`), and the JSON contract test for `watch`.
- **Binary**: on the synthetic fixtures (the 14-second clip and the screen-change fixtures), on all
  three systems: `vsift <video>` writes the three outputs beside a copy of the fixture, a second run is
  refused, `--output` works, and no session is left open.
- **Policy**: the skill-policy test with `watch` in the never class (a skill text change: a draft freeze
  in the pull request, rewritten at the merge rebase, `v030-rules.md`).
- **Limitations to register:** L-234 the frame bound (section 6); L-235 screen changes only for the
  first four hours; L-236 ids that name a closed session (section 4); L-237 a file held open on Windows
  (section 7, with the second pull request).

## Decisions for the planner and the maintainer

Each has a recommendation. Decision 1 is the maintainer's; the others were proposed for the planner, none
changing an existing JSON v1 field, failure code or safety rule. **Outcome (the planner, 2026-10-27,
relayed by the coordinator): decisions 2 to 10 as recommended**, with conditions A to C (sections 8, 7
and 2). Decision 1 is the maintainer's ruling of 2026-10-27: keep `watch`.

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
7. **The frame bound** (section 6): **Recommendation: 200 frames** (180 since the first pull request's
   implementation note), **spread evenly over the video when
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

## Implementation note, 2026-10-28 (the first pull request: the action)

What the first pull request built, and where it differs from the sections above. Section 13's first slice is
complete in code; the "after" real-media run is its qualification evidence.

- **Where the code lives.** The domain holds the output names, the readable and SubRip times, the frame file
  names, the even spread and the SubRip cue-text rule (`crates/vsift-domain/src/watch.rs`, `MAX_WATCH_FRAMES` =
  200). The infrastructure holds `WatchClaim` (the exclusive claim, write-once handles that return each file's
  SHA-256, frame copies checked against their digest, and a release that removes only what it created, never a
  written file) and the three renderers (`crates/vsift-infrastructure/src/watch_output.rs`). **The orchestration
  lives in the engine** (`Engine::watch` in `crates/vsift/src/watch.rs`), composing the engine's own `ingest`,
  `job_start`, `job_resume`, transcript, candidate and frame methods and calling the folder writer directly:
  **there is no application use case and no `WatchOutputs` port** (section 3 proposed both). Every step it composes
  is already an application use case behind its own ports; the one new effect, writing the person's folder, has
  a single implementation, and the engine tests run it on real temporary folders. The second pull request, whose
  growing files need a snapshot-and-replace that tests want to fake, is where a port earns its place.
- **Frames are written after the job,** not while the transcription runs (section 3, step 5 moves to the second
  pull request with the growing files).
- **Ctrl-C** releases the claimed names and **closes** the session; continuing an interrupted run (section 8) is
  the second pull request's.
- **The envelope is always `complete`;** `data.outcome` says `partial` when a range was not transcribed, the
  video runs past four hours, or the frames stopped for room (`frames_stopped_for_room`). Deciding `partial`
  from those facts is done where the result is built.
- **A refusal names the output's kind** (`.txt`, `.srt`, `-vsift`) in fixed prose, never the file's name: a
  remediation never echoes text derived from the untrusted video name (L-071). The readable view does not name
  the file either, in this pull request.
- **The finish screen** has no elapsed time and no `Next: vsift setup` line (the missing-capability wording stays
  the existing remediation's). It says that each file names the video's SHA-256 and ends `More: vsift watch
  --help`; the "More" line names no extra until the third pull request.
- **A video whose name is not Unicode text is refused** before any work (`INVALID_ARGUMENT`, "Rename the video"),
  as is one with nothing before its extension: output names are derived from Unicode names only.
- **Rule 4 of section 2 never applies to text with a control or hidden character.** SEC-T02's hostile-argument
  test for the readable output (`sec_t02_human_output`) found that its sentinel, given as an unknown command, holds
  a terminal link to `https://...` among control and hidden characters, so its `/` made it look like a path and it
  would have become a `watch` of a missing file; such text now stays the parser's to reject, inert, as before. A
  person never types a path with a control or hidden character.
- **A source that is not a regular file goes to `ingest` first,** before the preflight, so condition C holds
  without FFmpeg configured: a folder is `SOURCE_IS_FOLDER`, a link `SOURCE_IS_LINK`, a missing file `ingest`'s
  "not found".
- **The header's provenance is "local speech recognition"**; the model's name is not written (section 4's example
  shows it), because the sealed revision's read does not carry it in a form fit for a person.
- **The door's "Try it" line** is `Try it: vsift ./your-video.mp4` (section 12), after the door's own verdict
  line `Setup complete: video and speech both work.`: both of ADR 0036's pull requests reached `main` first, so
  this one changed only the command and kept their wording.
- **The command's ordinal is 38,** after `setup` (37), and the skill's never list holds both.
- **Measured before the code** (the hosted `P14 real media` run 40595722898 on `e317271c`, main plus this record):
  RM03 (a 19:16 talk) has 576 screen changes and RM10 (a 3:21 screen recording) 106, about 30 a minute; finding
  them took about 1.7 s a minute of video against about 17 s a minute of transcription, which confirms section 6's
  care point (they are cheap enough to run by default). The frame bound is reached by a 19-minute talk (L-234).
  The visual-measurement step did not run, because RM03's advisory seam findings failed the campaign first.
- **The frame bound is 180, fitted to the session's room (the "after" run, 2026-10-28).** The "after" `P14
  real media` run 40677125255 on `67a058e4` failed RM03 with `RESOURCE_LIMIT` after 482.7 s; RM10 passed. Each
  frame is a `frame get`, which stores two evidence artifacts (the PNG and its record), and a session holds at
  most 384 (`MAX_EVIDENCE_ARTIFACTS`) within its 512 artifacts, so 200 frames needed 400 slots and the 193rd
  frame was refused. The bound is now 180 (360 slots), and `watch` measures the session's evidence room before
  the first frame and writes at most half of it (`frames_that_fit`), so it never asks for more than the session
  holds. The opt-in engine test `more_changes_than_the_session_has_room_for_still_finish` reproduced the failure
  without real media (a generated video with a new picture every two seconds) and passes with the fix; the test
  `the_frame_cap_fits_a_fresh_sessions_evidence_room` pins the relation.
- **Register:** L-234, L-235 and L-236 added by this pull request; L-237 comes with the second.

## Implementation note, 2026-10-28 (the second pull request: the use case)

The first pull request's departure "there is no application use case and no `WatchOutputs` port" is
**closed** (the planner's ruling of 2026-10-28: required for 0.3.0, and the first commit of the second
pull request). Section 3's layering now holds as written:

- **application** (`crates/vsift-application/src/watch.rs`): `run_watch`, the use case. It checks the source
  and the capabilities, derives the names, claims them, opens the session, runs the `prepare` job, states the
  untranscribed ranges as gaps in time order, measures the evidence room before the first frame and takes at
  most `frames_that_fit` frames spread evenly (a refusal for room ends the frames, not the run), finalises the
  files and closes the session; after the claim, a failure releases it. It reaches the existing operations
  through a second port, `WatchSteps`, which the engine supplies, as `PrepareStreams` supplies the `prepare`
  job's streams: one method per existing operation (the source's `ingest` refusal, the configuration check,
  `ingest`, the watched `prepare` job in this process, the transcript and candidate reads, the evidence room,
  `frame get`, the provenance, closing the session), none with a rule of its own. The port data and the
  outcome (`WatchOutcome`, `WatchFile`, `WatchFrame`, `WatchOutputKind`) moved here from the engine and the
  infrastructure; the engine re-exports them under the same names, so no host changes.
- **`WatchOutputs`** covers claim (which also resolves the folder: the video's own, or `--output`, created
  without its parents, canonicalised and never inside VSift's roots), copying a frame checked against its
  SHA-256, finalise and release. Writing a snapshot of a file that grows joins it with section 7.
- **infrastructure**: `FolderWatchOutputs`, the port's folder implementation over `WatchClaim` and the three
  renderers (`crates/vsift-infrastructure/src/watch_output.rs`).
- **engine**: `Engine::watch` is the facade: it makes the paths absolute, builds the two ports and maps the
  use case's error back to `EngineError`. The first pull request's `FrameTaker` seam is gone; the backstop is
  the use case's, tested over the fake steps.
- **Tests:** the use case over fakes of both ports (the order of the steps; a missing capability, a source
  that is not a file, an unusable name and a taken name each refused with nothing claimed or no session
  opened; a failure after the claim released and the session closed; a stated gap in time order; no subtitles
  without a sound track; 576 changes with 180 frames spread evenly and every change listed; the frames fitted
  to a small room; the backstop), the folder port on real temporary folders, and the engine's opt-in runs
  through `Engine::watch` unchanged.

## Amendment, 2026-10-28: the list holds settled screens (the second pull request, ruling 3)

**What changes.** Section 6 said every screen change the visual stream finds is listed. The first pull request's "before"
run (40595722898) found about 30 changes a minute on both RM03 (a 19-minute talk, mostly camera movement: 576 changes,
466 of them `visual_change` and 55 `motion_start`) and RM10 (a screen recording: 106), which is the per-window candidate
budget's ceiling: a list that long hides the screens a person would look at. From the second pull request:

- **Every change found is counted** (`changes_found`, as before), and **the list shows the settled screens** among them
  (`changes_listed`, additive to `watch-data`): a `first_frame` and each `settled_after_motion` always; a `visual_change`
  only when it comes at least 5 s after the last listed entry, whatever that entry's reason; a `motion_start` and
  `periodic_coverage` never, because a person reads the screen once it holds still. The rule is a pure domain function
  (`settled_screens`, `SETTLED_SCREEN_SPACING_MICROS`), applied by the use case before the frames.
- **Each line names its reason** in plain words (`first frame`, `settled after motion`, `visual change`), and the header
  says `N settled screens of M changes found` and states the rule when it left changes out. The finish screen says
  `N settled screens of M changes` in that case.
- **Frames** are taken for listed changes only: still at most 180, spread evenly over the listed ones and fitted to the
  measured evidence room (section 6 and its correction). **The 180 bound stays until the measurement is in.**
- No failure code changes; `changes_found` keeps its meaning.
- **The cost, stated (the planner's condition, 2026-10-28).** A screen shown for less than 5 s after a listed one is
  not listed unless motion settles on it: a dialog that flashes up 3 s after a click, for example. The header says so
  whenever the rule left changes out, and so does the guide's limits page (L-234, amended).
- **What was left out stays reachable (the planner's condition).** The run closes its session, so the changes it left
  out are kept nowhere. Whenever the rule left changes out, the header gives the commands that show every one:
  `vsift ingest <video>`, then `vsift candidates <session> --from 0 --to <the end the index covers, in microseconds>`
  (the number is written in), with the session id `ingest` gives. The guide's limits page says the same.

**Confirmed by the planner, 2026-10-28,** relayed by the coordinator: the five choices as written, with the two
conditions above and a third on the measurement: the run reports `changes_found`, `changes_listed` and the listed count
by reason for each item. If a talk like RM03 still lists more than about 100, or a screen recording like RM10 or RM01
drops a change a person would want, the numbers and a spacing proposal go back to the planner rather than a retune in
this pull request; no campaign threshold is loosened.

**The measurement** (the second pull request's one hosted `P14 real media` run, RM01, RM03, RM04 and RM10): the
campaign's `watchItem` records `changes_found` (today's rule), `changes_listed` (this one) and the listed changes by
reason for each item, and checks that the frames are min(listed, 180), that no in-progress marker or temporary file is
left, and that the session was closed.

**Measured, 2026-10-29: hosted run 40791259684** on `682920e5`. The pull request's later head, `adee6bc1`, differs from
it in records only. Every `vsift <video>` step exited 0, and every `watch` check passed. RM03 failed only the advisory
seam check, on the two rewording seams F14 left after #542.

| Item | Footage | Changes found (every change) | Listed (settled screens) | Listed by reason | Frames | Wall time (RTF) |
| --- | --- | --- | --- | --- | --- | --- |
| RM01 | screen recording | 26 | 15 | first frame 1, visual change 14 | 15 | 46 s (0.33) |
| RM03 | 19:16 talk, moving camera | 589 | 183 | first frame 1, visual change 128, settled after motion 54 | 180 | 479 s (0.41) |
| RM04 | screen recording | 138 | 48 | first frame 1, visual change 14, settled after motion 33 | 48 | 90 s (0.30) |
| RM10 | 3:21 screen recording | 108 | 31 | first frame 1, visual change 11, settled after motion 19 | 31 | 65 s (0.33) |

The campaign's own `candidates` step on the same run gives every change found by reason:

- RM01: 22 visual changes and no motion.
- RM03: 466 visual changes, 55 starts of motion, 54 settled.
- RM04: 62 visual changes, 37 starts of motion, 33 settled.
- RM10: 65 visual changes, 21 starts of motion, 19 settled.

**Reading.** The rule shortens every list by a half to three quarters.

- On RM04 and RM10, most of what it leaves out is either motion's start (its settled screen is listed) or a plain
  change within 5 s of a listed one.
- On RM01, a screen recording without motion, it leaves out 11 plain changes within 5 s of a listed one. That is the
  case this record names as the rule's cost: a screen that follows a listed one quickly, a dialog after a click say.
- The run's report holds counts, not the changes' times or frames. So whether a person would have wanted any of the
  changes left out on RM01 or RM10 is not shown by this run. It would take a look at those changes' frames.
- RM03, the talk, still lists 183. That is above the planner's mark of about 100, and above the 180-frame bound, so 3
  are listed without a frame. A spacing proposal goes to the planner rather than a retune here.
- The full list was not useful on footage like RM03: 589 entries in 19 minutes, one every two seconds, mostly camera
  movement.
- RM03's run was also `partial`, with one range not transcribed, which the transcript states in place.

**Delivery, amended 2026-10-29 (the planner's ruling, relayed by the coordinator).** Section 13's second pull request
is split. The second pull request holds the use case (the implementation note above) and the settled screens (this
amendment), with its own hosted measurement. Sections 7 to 9 (files that grow, frames during the transcription,
Ctrl-C keeping the session and a rerun continuing it, a file held open on Windows with L-237, and the bars) come in a
further pull request with its own hosted run. `--include on-screen-text` (section 10) stays separate and droppable at
the cut.

**The planner's ruling on the measurement (2026-10-29), relayed by the coordinator:** keep the 5 s rule as it stands.
"About 100" was a trigger for review, not a requirement. A spacing that grows with the video's length would thin long
screen recordings, where quick screens matter most. The bound that counts is the frame room: a listed change without a
frame is marked `no frame (evidence room)` on its own line (RM03's 3 of 183), and L-234 gives RM03 as its measured
example. Recording each found change's time and reason in the campaign's report, so that a later spacing review can be
simulated from one run, goes to the pull request for sections 7 to 9.

## Implementation note, 2026-10-29 (the third pull request: sections 7 to 9)

What the pull request for sections 7 to 9 built, under the planner's two rulings of 2026-10-29.

- **How the use case hears of the job (call 1, option (a)).** `WatchSteps::prepare` takes a `&dyn WatchProgress`,
  told `published()` after each chunk of speech (when a part may have been published) and `visual_indexed()` when the
  visual stream has indexed the whole video (its last batch reports every window, the `StreamEnd::Indexed` point).
  The callback only records, in a small signal; the use case polls a follower beside the job on the same task (a
  `std` join, no runtime in the application), which writes a snapshot at most every 10 s by the `Clock` port, after a
  `--since` read of the open revision, and the frames and `screen-changes.txt` once the visual stream is done. In the
  engine the job's progress observer must outlive the call, so it queues each notice on a channel, and the adapter
  hands it to the callback between the job's steps (a `select` on the same task; no thread, no poll).
- **Where the claimed names live (call 2).** A versioned record `watch-outputs.json` (format `vsift.watch_claim`, v1)
  beside the `prepare` job, in the session: the absolute folder and the three names, nothing else
  (`WatchClaimLog`). Checked in the storage code before it was written: no `VSift` reads a job directory's entries
  (only `job.json`, the locks, `chunks/` and, since #510, `progress.json`, by name), so an older one ignores it; the
  storage layout version did not move; session cleanup removes the session's whole tree, pruning removes a job's
  folder, and a retained bundle never holds `jobs/`. A record that fails any check is never read as a claim.
  **The planner's condition (2026-10-29):** the record is untrusted session content and only a key to find the
  session, never a place to write. A rerun canonicalises its own output folder again; the record must name exactly
  that folder and those names, and the run writes only to the paths it resolved itself. A record that does not parse,
  or names another folder, matches nothing, so the files in the way are refused by section 5 (the opt-in engine test
  `a_record_that_names_another_folder_is_refused`).
- **Growing files (section 7).** Every write of `<stem>.txt`, `<stem>.srt` and `screen-changes.txt` goes through
  `replace_claimed`: an exclusive temporary file `.<name>.<random>.vsift-partial` (random part from the identifier
  source), flushed, then moved over the claimed path. The claim now creates the two files empty and closes them, so
  no handle of this process stands in the way of a move on Windows. Tests: a temporary name taken in advance and a
  link planted there (Unix) each fail the update and write nothing through; a reader never sees a partial file
  (Unix); a handle held without delete sharing (Windows) skips the snapshot and fails the final write with `Held`
  after its wait, leaving the last snapshot (L-237).
- **Frames during the transcription (section 3, step 5).** Taken when `visual_indexed()` arrives. The room they fit is
  measured as if the session held no evidence (`evidence_slots_if_reused`): the session holds only this run's frames,
  and `frame get` gives a stored frame again without a new slot, so a run that continues an interrupted one spreads
  its frames over the same changes and the copies, already in the folder with the same digest, are accepted.
- **Ctrl-C (section 8).** The engine's run, cancelled by the caller, leaves the job `interrupted` with its chunks and
  the open revision as published (not sealed `stopped`, as section 8 put it: that is `job cancel`'s path, and an
  interrupted job is the one `job resume` continues). The use case writes one last snapshot, keeps the files and
  the session, and fails with `Stopped`, which the engine maps to `WatchFailure::Stopped` (`CANCELLED`). A rerun whose
  claim finds a name taken resolves the planned paths, then lists the open, unexpired sessions; for a job whose
  record names exactly those paths and which is not ended, it hashes the video (only then) and compares the
  session's source. A match is taken over (each output must still be a regular file or a folder, never a link) and
  its job resumed; anything else refuses as section 5 says. A release after a failure of a taken-over run removes
  nothing the earlier run created.
- **The bars (section 9)** are the CLI's (`human::watch_progress`, pure renders with golden files for the terminal,
  `NO_COLOR` and `TERM=dumb`; `watch_lines`, the board built from the `prepare` job's progress, a once-a-second gate
  tested with given times, and the drawing on standard error). Three differences from section 9's picture: the screen
  line says `done` without `14 found`, because the visual stream's progress counts windows, not changes; the copy line
  has no byte figures, because `ingest` reports no progress during the copy; and on Windows with standard output
  redirected, the console cannot take escape sequences, so the plain lines are shown.
- **Truthful markers (the planner's condition at review, 2026-10-29).** A snapshot carries a state
  (`SnapshotState`): `Running` writes section 7's markers; `Stopped`, written at Ctrl-C, starts `STOPPED: transcribed
  up to 00:12:30 of 00:48:00, where the run was stopped.` and says the same command continues while the private copy
  is kept, about a day unused; `Failed`, written best effort through `replace_claimed` when a run fails after this run
  wrote a snapshot or after it took over an interrupted run's files, starts `STOPPED WITH AN ERROR:` and says the file
  will not be finished. The subtitles' trailing cue says the same. The files are never removed and never left saying
  `VSift` is working; a held file stays as it is (L-237). Tests: the use case over fakes (Ctrl-C writes `Stopped`; a
  failure after a snapshot writes `Failed` for a fresh claim and for a taken-over one; none before any snapshot) and
  the render of each state.
- **Held or not writable; a leftover temporary file (the planner's two points).** Windows reports a target that is not
  writable as it reports a held one, so it too waits 30 s and fails `Held`; the remediation and L-237 name both. A hard
  stop between the temporary file and the move leaves a hidden `.<name>.<random>.vsift-partial`; the guide's limits
  page and L-237 say it is VSift's and safe to delete, and nothing cleans the person's folder automatically.
- **RM03's unplaced range** (123.8–125.0 s) went to intake with the seam work (#539, #474), not to the register.
- **The stopped line.** The failure's remediation starts `Stopped at 12:30 of 48:00. Run the same command again to
  continue.`, from the published time and the video's length (left out when no part had been published).
- **No new failure code and no JSON change.** The three "already exists" remediations now say how a leftover of an
  expired run is told apart; `WATCH_OUTPUT_HELD_REMEDIATION` and `WATCH_STOPPED_REMEDIATION` are new texts.
- **The campaign's watch report** records each found change's time and reason (from the item's own `candidates`
  step), and the speech measurement each unplaced range's start and end.
- **Register:** L-237 added.

**Measured, 2026-10-29: hosted run 40858083797** on `02f43a95` (RM01, RM03, RM04, RM10, `campaign`). Every
`vsift <video>` step and every `watch` check passed; RM03 failed only the two advisory rewording seams F14 left
(761–766 s and 825–830 s), as on the earlier runs. The counts are unchanged from #544's run:

| Item | Changes found | Listed | Frames | Wall time (RTF) | Outcome |
| --- | --- | --- | --- | --- | --- |
| RM01 | 26 | 15 | 15 | 45.3 s (0.33) | complete |
| RM03 | 589 | 183 | 180 | 426.3 s (0.37) | partial: one range not transcribed |
| RM04 | 138 | 48 | 48 | 83.8 s (0.28) | complete |
| RM10 | 108 | 31 | 31 | 62.4 s (0.31) | complete |

- **The report's new fields are present.** `measurements.watch.changes` lists 23, 576, 133 and 106 changes with their
  times and reasons. That is fewer than `changes_found`, because the campaign's `candidates` step leaves out
  `periodic_coverage`, which the settled-screen rule never lists, so a spacing review can still be simulated from it.
  `measurements.transcription.unplaced` holds each range's start and end.
- **RM03's one unplaced range** is 123.8 to 125.0 s (1.2 s), at the seam between chunks 4 and 5 (125 s). It is stated
  as not transcribed in the transcript and in `watch-data`. It is neither of the two seams the planner named: it is
  longer than L-156's 0.5 s tolerance and it is stated, while L-156's stretches are silent; and L-188's RM03 case is
  chunk 3 (75 to 100 s). It went to the planner through the coordinator.

## Amendment, 2026-10-30: `--keep <folder>`, keep a recording for later search (W6h)

**Status: accepted by the planner, 2026-10-30,** at `2678920a` with one change, which this text includes (a final write
that fails on a sealed job). The maintainer's word, 30 Oct: "Yes, add an option to the one-command action that
keeps the recording for later search, as a small follow-up once search has merged." Search (ADR 0035) reads retained
bundles only, and the action closes its session, so today nothing it did can be found later.

```console
vsift ./call.mp4 --keep ./kept-calls
vsift ./second-call.mp4 --keep ./kept-calls
vsift bundle search ./kept-calls --query "refund limit"
```

**What changes.** `vsift <video>` and `vsift watch <video>` take `--keep <folder>`. A run that finishes writes one
retained bundle (ADR 0013) to `<folder>/<name>`, after its outputs are finished and before its session closes. Section
11's "never keeps the session longer than the run needs" holds: the session still closes, and the bundle is the export
the person asked for by typing the option. Without `--keep`, nothing changes. Section 3's steps become: 1 check, 2 claim,
**2a check the keep target**, 3 to 6 as written, **6a write the bundle**, 7 close.

**The name.** `<name>` is the action's stem (section 5) with each `:` replaced by `-`, from a new domain method,
`WatchOutputNames::bundle()`. So `call.mp4` gives `kept-calls/call`, which is also what `bundle search` shows as the
bundle's name. The `:` matters: Linux allows it in a file name, and a recording can carry a clock time in its name
(`Screencast 10:30:00.webm`), but `session retain` and `bundle validate` refuse it in a bundle's folder name (the rule
of #232, `root_name_is_allowed`), so a bare stem would be refused for such a recording. A stem that rule refuses for any
other reason, a Windows device name such as `aux.v2`, is refused before any work with "Rename the video". The rule is
not widened, because it is what makes a name mean the same folder on every system.

**Refused before any work.** The keep target is checked after the output names are claimed and before the session is
opened; a refusal releases the claim, so nothing is left behind and no session exists. It comes after the claim so that
a run refused for a taken output never leaves a new keep folder. The check goes through a new port method,
`WatchOutputs::prepare_keep`, and a store check beside `retain_bundle`:

| What is wrong | Failure | Remediation says |
| --- | --- | --- |
| The name is taken under the folder, by anything, a link included | `INVALID_ARGUMENT` | A recording of that name is already kept there; VSift never replaces it. Keep this one in another folder, or rename the video. |
| Not a folder, or a new folder whose parent is missing, or a network path, which bundles refuse | `INVALID_ARGUMENT` | `--keep` takes a folder: an existing one, or a new one whose parent exists, on a local drive. |
| Inside VSift's session or managed root | `INVALID_ARGUMENT` | As `--output`'s. |
| The stem cannot name a bundle (above) | `INVALID_ARGUMENT` | Rename the video. |
| The folder refuses the write, or cannot hold an owner-private folder | `STORAGE_IO` | Choose a folder you can write to with `--keep <folder>`. |

A missing folder is created, with its parent required to exist, and with the ordinary permissions a person's folder has;
the bundle inside it is owner-private (ADR 0026), which is what `bundle search` checks. A folder created by a run the
check then refuses is removed again, if still empty.

**The name is checked, not held.** ADR 0013 has `retain_bundle` create its folder exclusively, so a folder claimed at
the start could not be used at the end. The check creates the real name privately, confirms it is private, and removes
it at once: that one step proves the name is free, the folder is writable, and it can hold a private folder, with no
random probe name. Nothing stands at the name during the run, so another program, or a second run with the same name in
the same folder, can take it meanwhile. Exclusive creation at the end then fails, nothing is replaced, and that run ends
as a failed write (below). An interrupted run leaves nothing in the keep folder, so nothing is left to clean up there.

**If the bundle cannot be written.** The outputs stand. The session is **left open**, not closed, so the work is not
lost. `data.outcome` is `partial`, `session_closed` is false, `kept.state` is `not_kept` with the failure's code, and the
envelope carries a fixed warning. The envelope's status stays `complete`, as the first pull request's implementation note
has it for every other partial: `data.outcome` is where `watch` says partial. The exit status is 0. The warning and the
finish screen give the command: `vsift session retain <session_id> --output <a new folder>`, with the real id in the
finish screen. They say the session lasts about a day unused. `retain_bundle` may have left an incomplete folder at the
name: ADR 0013 says VSift never deletes a caller-selected export, so it stays, `bundle validate` rejects it, `bundle
search` counts it in `skipped_entries`, and the words say it can be removed.

**A job that did not seal keeps nothing.** A run stopped by Ctrl-C, or failed before the job sealed its revision, writes
no bundle, and a failed run closes its session as it does today. A `partial` run does seal, so it is kept: the bundle
states its gaps, and search reports them. A Ctrl-C that arrives after the job has ended does not stop the bundle; the
write has no cancellation point, and takes seconds, because the video is hashed but not copied.

**With `--keep`, once the job has sealed, the session closes only after a bundle is committed (the planner's change,
2026-10-30).** The transcript exists and the person asked to keep it, so a close then would destroy the only copy. An
editor or viewer holding the growing `.txt` is an ordinary way to get here on Windows (L-237). So a failure after the
seal, a final write refused or a frame that failed, does not stop the bundle, which does not need the outputs:

- The bundle is written anyway. The failed output is reported as it is today, with its own code and remediation.
- The failure's `data` is new and small, `session_id`, `session_closed` and `kept`
  (`watch-failure-data.schema.json`), and the remediation's last words say what became of the bundle. Every other failed
  `watch` has `data` null, as before.
- The close follows the normal rule: closed when the bundle is committed; left open, with the `session retain` words,
  when the bundle fails too.

A failure before the seal stays as written above. Without `--keep`, nothing changes.

**Interrupted, and run again (section 8): the option belongs to the command line, not to the job.** It is not recorded.
`watch-outputs.json` keeps format `vsift.watch_claim` v1 with the same three names. The run that finishes is the one
that keeps, and it does what its own command line says:

| First run | Run again | What happens |
| --- | --- | --- |
| `--keep a`, stopped | `--keep a` | Continues the job, keeps in `a` |
| `--keep a`, stopped | no `--keep` | Continues the job, keeps nothing, closes the session |
| no `--keep`, stopped | `--keep a` | Continues the job, keeps in `a` |
| `--keep a`, stopped | `--keep b` | Continues the job, keeps in `b` |

The reason is the planner's condition of 2026-10-29: the job's record is untrusted session content, only a key to find
the session and never a place to write. A keep folder read back from it would be a place to write. The same condition is
why a rerun matches on the output names alone, so adding or dropping `--keep` never stops it finding its job. The
stopped line's "run the same command again" already includes the option. A rerun checks its keep target as any run does,
and a refusal then leaves the interrupted job and its session untouched.

**What the bundle holds.** What `session retain` writes by default: the transcript, the visual index, the frames and
their evidence records. The video is not copied in, and adding that is out of scope. On-screen text is not read, because
the action does not run `ocr`; `bundle search` says so per recording, and the help and the guide say it plainly. When
section 10's `--include on-screen-text` lands, its index is in the session, so the bundle carries it with no change here.

**The result (additive; `watch-data` has not shipped: the planner checked that `v0.2.3` has none).** One new required
member, null without `--keep`:

```json
"kept": {
  "state": "kept",
  "path": "/home/example/kept-calls/call",
  "name": "call",
  "failure_code": null
}
```

`state` is `kept` or `not_kept`; `path` is where the bundle is, or where it was to go, in the path form of `watch`'s
outputs; `failure_code` is the failure's public code when `not_kept`. The contract says it plainly: a script reads
`data.outcome` and `kept.state`, not only `status`. No new failure code, no prompt, no new command, no job format change. The finish screen
gains one line, `call/   kept for later search with vsift bundle search`, or, when `not_kept`, the words above.

**Layering.** `vsift-domain`: `WatchOutputNames::bundle()`. `vsift-application`: `run_watch` takes the keep folder,
calls `WatchOutputs::prepare_keep` and a new step `WatchSteps::keep_session`, one existing operation with no rule of its
own, and `WatchOutcome` gains `kept`. `vsift-infrastructure`: the folder work, and the check beside `retain_bundle`.
`vsift`: `WatchRequest::keep`, the step over `retain_session`, and `EngineError::WatchFailedAfterKeep`, which carries a
failure after the seal with what became of the bundle and answers with the inner failure's code. `vsift-contract`: the data, schema and remediations.
`vsift-cli`: the flag and the finish screen.

**No skill text changes.** The skill class stays **never** for `vsift watch` (section 11), and `--keep` is an option of
that command, so the skill does not mention it and the trial freeze does not move.

**Left as it is.** The header of `<stem>.txt` says the session was "closed when this run finished", and is written before
the bundle. In the rare `not_kept` case that is untrue for about a day; `session_closed` and the finish screen are right,
the warning says the private copy of the video stays until the session ends, and L-244 records it.

**Limit.** L-212 loses "a recording processed with `vsift <video>` leaves no bundle". L-244 records what `--keep` leaves:
the video is not in the bundle, on-screen text is not read, two videos with one stem cannot share a folder, the name is
not held during the run, what a failed write leaves, and the header's words after one.

**Tests.** Domain: `bundle()` for a plain stem, a `:`, a shortened stem. Application, over fakes: the order (bundle after
finalise and before close); close never called after a failed bundle, and the outcome `partial`; each refusal releases
the claim and opens no session; a stop writes no bundle; a failure before the seal writes none; a final write that fails
on a sealed job keeps, closes after the bundle, and reports the failed output; the same with the bundle failing too leaves
the session open; without `--keep` that failure closes as before; a rerun with, without and with another `--keep`, each
finding its job by the same names. Infrastructure: the keep check on real folders (taken by a folder, a
file and a link; a new folder; a missing parent; inside a root; the name rule; nothing left after a pass or a refusal).
Engine, opt-in and run by the `Nightly` workflow on Ubuntu from this change, which left `engine_watch` unguarded before
it (the planner's ruling, 2026-10-30; FFmpeg and a stand-in recogniser): a name already kept, the rerun table through a real job, and a final
write refused on a sealed job, whose bundle `bundle search` finds. CLI: the JSON contract with `kept`, the schema and
example, the failure's data, the finish screen's forms. End to end: the action with `--keep` on the synthetic clip, a
second recording into the same folder, then `bundle search` on the folder finds the words of both.
