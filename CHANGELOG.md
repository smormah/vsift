# Changelog

All notable changes to VSift will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Crops and audio clips from the command line (P09 PR 4, ADR 0019), completing
  evidence navigation: `vsift crop <session> <evd_...> --rect x,y,w,h` cuts a rectangle
  out of a frame or an earlier crop by decoding the frame again, at native size, in the
  displayed orientation, and records where it lies in the source frame, so a crop of a
  crop still names source pixels; `vsift audio <session> --from <us> --to <us>` returns
  a WAV clip of up to 30 seconds (16 kHz mono) that says when its first sample really
  starts and whether the range was clipped at the end of the source. Both deliver the
  committed file's absolute path, are reused when repeated, stream with
  `--events jsonl` (the new `audio_evidence` record for clips) and refuse a rectangle
  outside its parent, a clip of more than 30 seconds, a source without audio and
  damaged media with typed errors and remediation. New schemas `audio-data`,
  `audio-stream-data` and `audio-evidence`; frozen examples `crop.json` and
  `audio.json`. The P09 checkpoint now also checks crops pixel for pixel against
  FFmpeg's own decode, audio start times, damaged and cut-short media, every stream,
  retained bundles with evidence, and records performance; and it runs the mechanical
  journey from a video to cited evidence on both transcript paths (a supplied SubRip
  file and local speech recognition): search, candidates, the candidate's frame, a crop
  and the audio of the cited segment, all checked against frozen truth, then retain
  and validate the session; the results are in the
  [P09 qualification record](docs/planning/p09-evidence-navigation.md).
- Frames from the command line (P09 PR 3, ADR 0019):
  `vsift frame get <session> --at <us>` returns the first frame at or after a time
  (`--select displayed-at` for the frame on screen at it, `--tolerance-us` up to 10 s,
  1 s by default), `--candidate <vcd_...>` returns a visual candidate's own frame
  exactly; `vsift frame neighbours <session> <evd_...> [--count 1..20]` the
  consecutive frames on each side of an earlier frame, saying why a side stopped short
  (`start_of_stream`, `end_of_stream`, `search_window`); and
  `vsift frame burst <session> --from <us> --to <us> [--max-frames 1..100]` the
  distinct frames at evenly spaced times over up to 60 seconds (12 by default). Each
  result states which frame each requested time resolved to, with the requested and
  actual time and their difference, publishes every frame as a new `frame_evidence`
  record, and delivers the full-resolution PNG as the absolute path of the committed
  session file, valid while the session exists. Repeating a request returns the same
  result with `reused: true` in about 150 ms without running FFmpeg or writing
  anything. A call stopped by a budget returns what it extracted as `partial` with the
  reason; a full session (160 evidence files) is `RESOURCE_LIMIT` with the advice to
  retain it and open a new one, and a burst over 60 s points to `candidates`.
  `--events jsonl` streams the frames, then the rest. New schemas `frame-data`,
  `frame-stream-data` and `frame-evidence`; frozen examples `frame-get.json`,
  `frame-get.events.jsonl`, `frame-neighbours.json` and `frame-burst.partial.json`;
  the opt-in checkpoint `p09_evidence_e2e` checks F01, F09, the rotated variant and
  every visual candidate of the test videos against their frozen truth.
- The evidence core of evidence navigation (P09 PR 2, in the engine library, not yet
  reachable from the CLI; ADR 0019, now accepted with decisions D1-D7): exact frames at
  a time or of a visual candidate, the consecutive frames around one, an even burst
  over up to 60 seconds, a crop of a frame or of a crop (in source pixels) and a WAV
  clip of up to 30 seconds. Each call commits its images or clip and one
  `evidence_record` (a new strict, versioned session artifact with its published
  bundle schema) that says which frame each requested time resolved to, with the
  requested and actual time and their difference. Asking again for the same thing
  returns the committed evidence without running FFmpeg; two requests that land on the
  same frame share one item and one file; other tools are a new identity. Calls are
  bounded (100 frames, 200 megapixels, 256 MiB, 120 seconds) and return what they
  extracted, marked partial, when a bound stops them; a session holds at most 160
  evidence files and records. After one full hash, later evidence calls check the
  source copy by its file identity instead of hashing it again. `bundle validate`
  checks every evidence record against its images and clips. New fuzz targets
  `evidence_record` and `crop_rect`.
- Groundwork for evidence navigation (P09 PR 1, not yet reachable from the CLI; ADR
  0019): the media adapter can list a stretch of a video's actual frame
  times, extract up to eight frames by their exact timestamps as full-resolution PNG
  images, crop a rectangle of a frame in its displayed orientation, and cut a WAV clip
  of up to 30 seconds (16 kHz mono) that says when its first sample really starts.
  Frames are chosen by integer timestamps, so a visual candidate's time now extracts
  exactly that frame (all 29 candidates of the test videos at a difference of 0). The
  rules that choose frames for a time (at-or-after by default, or the frame on screen),
  the frames around one, and an evenly spread burst are pure, property-tested domain
  code. New fuzz targets `frame_showinfo`, `frame_listing` and `png_sequence`.
- The automatic FFmpeg/FFprobe check now also lists frame times, extracts a frame by
  its exact timestamp and crops it (verification profile 3, still reported as the
  `frame` check), so every recorded pass is verified once more.

- Visual candidates: `vsift candidates <session> --from <us> --to <us>` lists the
  moments where the video's screen changed, plus a sample at least every 10 seconds so
  a static screen is still represented, 20 per page (`--limit 1..100`, `--cursor` to
  continue). Each candidate is a new `visual_candidate` evidence record with the time
  of the actual decoded frame that shows it (which `frame get` will extract), the span
  of time it stands for, why it was proposed, whether the screen was settled,
  transient or moving, the size of the change (uncalibrated numbers for ordering only)
  and a similarity hash that shows when a screen repeats. The first call over a range
  analyses its 60-second windows with FFmpeg, at most 30 minutes of video per call, at
  up to two frames per second as tiny grey thumbnails that are never kept; the rest is
  reported as `not_analyzed` and the next call continues it. Later calls over analysed
  time need no tool and take about 100 ms. Every result lists what is not analysed or
  could not be (`not_analyzed`, `deadline_exceeded`, `undecodable`,
  `no_decoded_frame`, `candidate_budget_exhausted`); a result with gaps is `partial`
  (exit 0) with the gaps in the envelope `coverage`. `--events jsonl` streams the
  records, then the coverage. The index is stored in the session as validated records
  that travel into retained bundles. On the synthetic test videos every stable screen
  of at least a second is found, with no false changes. New schemas `candidates-data`,
  `candidates-stream-data`, `visual-candidate` and `bundle-visual-index-record` with
  frozen examples; ADR 0018 (accepted 2026-09-26) records the design. A video without a video
  stream is `INVALID_ARGUMENT` with fixed remediation; no new failure code.
- The automatic FFmpeg/FFprobe check now also proves that visual sampling works
  (verification profile 2, check `visual_sampling`), so every recorded pass is
  verified once more after upgrading.
- Transcript search: `vsift search <session> --query <text>` finds a literal query in
  the session's newest transcript (or `--revision <trv_id>`), optionally within
  `--from/--to`, 20 hits per page (`--limit 1..100`, `--cursor` to continue). Spelling
  differences such as `R-17` and "dialog r 17", `2,048` and `2048`, or `twelve` and `12`
  still match. Whole-phrase matches come first, then segments containing every word;
  a phrase split across two segments is not found, and accents are not folded. Each
  hit is the same transcript segment record `transcript get` returns, and `--events
  jsonl` streams those records followed by the hit list. Every result says which parts
  of the searched range have no transcript and where local recognition found no
  speech; when part is untranscribed, the result is `partial` (still exit 0) and the
  envelope `coverage` lists the gaps. On-screen text is not searched. Search reads the
  stored transcript on every call and writes nothing; a 20,000-segment transcript
  pages in about 150 ms. A rejected query (`empty`, `too_long` over 256 bytes,
  `too_many_terms` over 16 words, `control_character`) is `INVALID_ARGUMENT` with
  fixed remediation naming the reason. New schemas `search-data` and
  `search-stream-data` with frozen examples; ADR 0018 (accepted 2026-09-26) records the design.
- Opt-in P08 checkpoints (`p08_search_e2e`, `p08_candidates_e2e`) and the
  `search_query`, `visual_samples` and `visual_index_record` fuzz targets.

- `setup check` now reports local speech recognition in a new `local_asr` object:
  whether the registered model is a reviewed pinned model and which profile, and
  whether whisper.cpp, the model and FFmpeg/FFprobe really transcribe a short speech
  clip built into VSift. A pass already on record is reported as `recorded`;
  otherwise the check runs it within its own 60-second budget and records a pass, so
  the first `transcript retranscribe` afterwards starts straight away. When it cannot
  run, the reason says what is missing first (media tools, whisper.cpp, a model, or a
  reviewed model). The existing fields and the exit status are unchanged; no path is
  shown.
- A second reviewed model profile, `base_q5_1`: the 5-bit quantization of the
  multilingual base model (`ggml-base-q5_1.bin`, 59,707,625 bytes, SHA-256
  `422f1ae4…a8898`), about 40% of the base model's size. Register it with `setup
  configure-model` like the base model; its identity selects the profile, and every
  revision records which one ran. The base model stays the default and the only model
  in the managed setup plan.
- Measured speech accuracy, speed and memory for both models, recorded in
  `docs/planning/p07-asr-qualification.md`. The base model is confirmed as the
  default: on clean speech it gets 3.25% of words wrong and hears every key term,
  runs at 0.39 times real time on 4 threads and peaks at 338 MiB. On noisy speech only
  the key terms are checked; its overall word accuracy there (61.5% errors on one
  short noisy clip) is a known limitation until a larger noisy test set exists
  (issue #150).
- An opt-in `P07 local ASR` workflow (manual and weekly) runs the local-ASR
  checkpoint and the new accuracy, timing and memory qualification on Ubuntu 24.04
  and Windows with the reviewed whisper.cpp v1.9.2 builds and both pinned models,
  each download checked against its pinned size and SHA-256, and uploads the reports.
- Local speech recognition: `vsift transcript retranscribe <session> [--from <us> --to
  <us>]` transcribes a session's speech with whisper.cpp into a new transcript
  revision, for the whole video or one range. It needs FFmpeg, FFprobe and
  `whisper-cli` (registered with `setup configure` or on `PATH`) and the reviewed
  multilingual base model registered with `setup configure-model`; any other model file
  is refused with `MISSING_CAPABILITY`. Before touching the video it checks, once per
  setup, that the recognizer really transcribes a short speech clip built into VSift.
  A range is widened to whole segments of the newest revision, and the new revision
  keeps every segment outside it unchanged (new identities, naming the segment they
  came from), so earlier citations stay valid. The newest revision is what `transcript
  get` returns; `transcript get --revision <trv_id>` reads any earlier one. A run that
  hears no speech is still recorded, with the warning `no_speech_recognised`. Failures
  use existing codes with fixed-prose remediation that names the failed step and
  reason. `ingest` and `transcript get` still never look for whisper.cpp or a model.
- Published contract for local ASR: the v1 `transcript-segment`,
  `transcript-revision` and `bundle-transcript-record` schemas now also describe
  local-ASR revisions (version-2 records), and a new `transcript-retranscribe-data`
  schema describes the command's result, with frozen examples. Imported transcripts
  produce exactly the same output as before.
- Fuzzing: `cargo-fuzz` targets in `fuzz/` for the parsers of untrusted input, namely
  SRT and WebVTT sidecars, whisper.cpp `-ojf` output, stored transcript records,
  FFprobe metadata and `transcript get --cursor` tokens. The new `Fuzz` workflow runs
  each for five minutes a week (or on demand) on a pinned nightly toolchain, and every
  pull request replays them over their committed seeds on the normal stable toolchain.
  No command or output changes. For library users, the FFprobe metadata parser is now
  public as `vsift_infrastructure::parse_ffprobe_metadata`, unchanged in behaviour.
- Internal local speech recognition core (P07 increment 3a), reached through
  `transcript retranscribe` from increment 3b onward. The engine library can now cut a range into overlapping
  30-second chunks, decode each with FFmpeg, recognise it with whisper.cpp v1.9.2,
  check every reported time against the audio actually decoded, skip silent chunks,
  and merge the chunks back into one transcript without dropping or doubling speech
  at the seams. Each result records exactly which whisper build, model file and
  settings produced it, and a run whose model changes part-way fails instead of
  mixing outputs. Supplied-transcript imports are unchanged: same identities, and
  the same stored record byte for byte. `bundle validate` now also accepts, and
  checks strictly, the version-2 transcript record that local recognition writes.
- Test fixtures tooling: `tools/generate_p07_speech.py` and the manually dispatched
  `P07 speech fixtures` workflow generate speech variants of the synthetic corpus
  videos from their frozen scripts, using the Kokoro text-to-speech model on a
  disposable CI runner, and `tools/verify_p07_speech.py` checks them independently.
  Kokoro is used only to make test data and is not a VSift dependency.
- Evidence stream: `vsift transcript get ... --events jsonl` now writes one line per
  transcript segment, each a self-describing evidence event with the segment record
  and an upsert key (its `segment_id`), followed by exactly one terminal event that
  carries the paging cursor and the number of records sent. An indexer can upsert
  records by key and knows the stream is complete when the terminal event arrives.
  Previously this mode returned the whole page as a single terminal event. `--json`
  and human output are unchanged. New v1 schemas `evidence-event` and
  `transcript-get-stream-data`, with a frozen example stream.
- The transcript record stored in retained bundles now has a published v1 schema,
  `bundle-transcript-record`, with a frozen example. `bundle validate` now decodes
  every transcript record and rejects a bundle whose record does not conform, even
  when its size and digest match the manifest. Bundles made by `session retain` are
  unaffected.
- Automatic media-tool check: before `ingest --transcript` measures the video,
  VSift runs its small built-in test video through the selected FFmpeg and FFprobe
  and checks the results. It runs once per tool pair (about 1–2 seconds the first
  time) and is repeated only when a tool is reinstalled, upgraded or reselected,
  when VSift is updated, or after seven days. A pair that fails, such as FFmpeg
  selected as FFprobe, stops the import before anything is written with
  `MISSING_CAPABILITY` (or another typed code) and a remediation that names the
  failed check and reason and says how to select working tools. The pass is kept
  in the private per-user VSift directory as digests and times only; there is no
  new command. Plain `ingest` and `setup` commands are unaffected.
- Supplied transcript import: `vsift ingest <video> --transcript <file.srt|file.vtt>
  [--transcript-offset <signed microseconds>]` imports an existing SubRip or WebVTT
  transcript into the new disposable session. The video is measured with FFprobe and
  only cues that lie wholly inside it after the offset are imported; nothing is
  clamped or shifted, and anything left out is reported with a typed warning.
  Malformed transcripts are rejected with a typed reason and line number before any
  session is opened. Whisper and model weights are not needed. The transcript is
  kept with the session and copied into retained bundles.
- `vsift transcript get <session> --from <us> --to <us> [--limit 1..100]
  [--cursor <token>]` returns a bounded page of timestamped transcript segments,
  each a self-describing evidence record with its alignment and provenance, plus a
  continuation cursor.
- New v1 schemas: `ingest-data`, `transcript-get-data`, `transcript-revision` and
  `transcript-segment`, with frozen examples. Plain `ingest` output is unchanged.
- F10 sidecar transcripts (`fixtures/corpus/transcripts/F10.srt` and `F10.vtt`) and
  an opt-in P07 end-to-end stage that imports them and cites F10's truth window.

- The engine can now prove that the selected FFmpeg and FFprobe actually work.
  It runs a small reviewed test video, built into VSift, through the same
  metadata, frame and audio steps an investigation uses and checks each result
  against the video's known answers. It can also identify whether a registered
  Whisper model is the reviewed pinned model. It now runs automatically before
  the first media operation (see the media-tool check above).

### Changed

- `transcript retranscribe` now checks the session's copy of the video twice per run
  instead of before every 30-second chunk: it verifies the copy's SHA-256 when the
  run starts and again before the new revision is saved, and before each chunk only
  compares the file's size, modification time and file identity. Long videos no
  longer pay a full read of the copy per chunk (on an 869 MB, 24-chunk video,
  decoding took 25.5 s instead of 173.4 s). The integrity guarantee is the same as
  before (ADR 0012, issue #148); results and schemas are unchanged and no failure code
  was added.
- Delivery is re-planned by ADR 0015. Managed dependency installation
  (`setup install` and its repair, list, rollback and remove lifecycle) moves from
  P06 to P13 and remains an R0 release requirement. P06 now closes on detection,
  bring-your-own selection, verification of the selected tools and manual guidance.
  Behaviour is unchanged: `setup install` still returns `COMMAND_NOT_IMPLEMENTED`.
- ADR 0016 commits VSift to an embeddable engine library and a published evidence
  contract, starting at P07.
- Internal reorganisation with no behaviour change: the v1 JSON response types moved
  from the CLI into a new `vsift-contract` crate that every future host will share.
  Command output, exit codes and schemas are unchanged.
- Internal reorganisation with no behaviour change: VSift's engine is now a Rust
  library, the `vsift` crate, and the command-line tool is a thin layer over it.
  Future hosts such as a worker or a desktop app will use the same library. Command
  output, exit codes and schemas are unchanged; the library API is not yet stable.
- The governance check now keeps the two session handoff files to a current-state
  size. The earlier day-by-day log is archived in `docs/history/`.

### Fixed

- **Security (SEC-17):** the media adapter could report a time taken from a video's
  own metadata instead of what FFmpeg decoded. FFmpeg repeats a file's metadata (for
  example its title) in the same diagnostic output VSift reads frame and audio times
  from, and two readers accepted any line that merely contained the filter's name, so
  a crafted file could shift the times `transcript retranscribe` gave its own
  transcript segments. Readers now accept only lines the filter itself wrote, require
  a complete, consistent sequence of frames, and check the time base against the
  probed stream; anything else is rejected. No failure code or schema changed.
- `transcript retranscribe` could save a revision even if the session's copy of the
  video changed after the last chunk was decoded. The copy is now verified again
  before the revision is saved; if it changed, the run fails with
  `INTEGRITY_FAILURE` and saves nothing.
- The reviewed Ubuntu x64 whisper.cpp v1.9.2 file set now includes ggml's 13
  optimised CPU backends (`sse42` through `zen4`) from the same pinned archive,
  each pinned by size and SHA-256. Before, only the generic `libggml-cpu-x64.so`
  was selected, so local speech recognition on Ubuntu ran about 11 times slower
  than on Windows (#153). The Ubuntu `setup plan` whisper action now lists 25
  files instead of 12.
- Local speech recognition could drop a whole sentence that started exactly
  where a 30-second chunk begins, when the previous sentence ended just after that
  point, without any warning. The sentence is now kept once.
- `setup check` no longer echoes a provider's first output line as `detail`. With
  whisper.cpp v1.9.2 that line was a library-loader log naming an absolute folder,
  which broke the promise that paths are not echoed. FFmpeg and FFprobe now report
  only their `ffmpeg version ...` / `ffprobe version ...` banner line (or
  `detected`). Whisper's output is never echoed: `detail` is
  `whisper.cpp v1.9.2 (reviewed build)` when the executable's bytes match a build
  reviewed for P06, otherwise `whisper-cli (build not recognised)`. As a second
  guard, no line that looks like a path or a ggml loader log is ever shown.
- On a Windows profile whose `%LOCALAPPDATA%` gives other accounts access to new
  folders (for example a sandbox group or an app-container capability), `setup
  configure`, `setup configure-model` and `setup check` failed with `STORAGE_IO`
  and no explanation, because the folder VSift had just created inherited that
  access and VSift then correctly refused it. Every folder VSift creates for itself
  (the per-user configuration folder and its missing parents, the session folder
  and its parent, retained bundles, the managed-data folder) now gets its own
  permissions before anything is written: only you, SYSTEM and Administrators, with
  inheritance from the parent turned off. On Linux and macOS these folders were
  already created owner-only; a missing parent of a private folder is now owner-only
  too. A folder that already exists is never changed: if other accounts can access
  it, the command still stops, now with a remediation naming the folder
  (`user_configuration` or `session_root`) and how to fix it. A session folder in
  that state is now `STORAGE_IO` instead of `INVALID_ARGUMENT`. A folder another
  VSift process has only just created is given a moment to become private before
  it is judged, so commands started together do not trip over each other.
- Media-tool check record: a reader that caught another process replacing the
  record could mistake the replacement for an unsafe record (issue #136). On
  Windows this made a concurrency test fail in about half of its runs. The read
  is now retried and otherwise counts as "not verified"; a record with more than
  one link is still refused. A failed flush of a new record no longer discards the
  pass, since a record lost to a crash already just means one more check. Taking
  the record's write lock now retries brief failures a few times instead of
  skipping the pass (seen on macOS); a linked or non-regular lock file is still
  refused and is never reported as busy.
- Media-tool check workspaces left behind when VSift was killed during a check are
  now removed by a later check, once they are an hour old and no running check
  holds them (issue #132). Only exactly named VSift workspaces in the private
  per-user state directory are removed, and links are never followed.
- Several VSift commands started at the same moment on a machine that has no
  session directory yet no longer fail with `INTEGRITY_FAILURE` ("ownership marker
  is invalid") (issue #131). One of them creates the session directory; the others
  wait for it to finish, for at most five seconds, and then use it only after the
  usual ownership and privacy checks. If it is still being created after five
  seconds they fail with the retryable `BUSY`. A directory VSift did not create is
  still refused at once.
- The published v1 schemas now accept `ISOLATION_UNAVAILABLE` and
  `setup.configure-model`, which the CLI already emitted (issue #125).
- Locks are now always released explicitly instead of by closing their file
  (issue #66). On Linux and macOS a child process started by another thread
  briefly holds copies of every open file, so a lock released only by closing
  could stay held for a moment and make an immediate retry report `BUSY`. This
  caused the intermittent CI failures and would have affected a busy worker.
  It applies to session, registration, admission, root-initialization,
  configuration, managed-install and managed-version locks.
- Per-user dependency configuration now reports `BUSY` only when the OS says
  another handle holds its lock. Other lock acquisition failures surface as
  storage I/O; an intermittent hosted `BUSY` test symptom remains under review.
- Registration explicitly releases its short-lived root initialization lock
  before returning the long-lived marker hold, preventing a duplicated file
  descriptor from prolonging root contention during an immediate bucket scan.

### Added

- P06 now fixes the Ubuntu managed candidate's compatibility policy in the
  reviewed catalogue: the exact checked-in F01 fixture, expected FFmpeg build
  and FFprobe identities, 64-KiB per-stream and transcript limits, 256-KiB
  generated-audio limit, 60-second media deadline, 180-second inference
  deadline, and 16-kHz mono audio contract. Catalogue
  completeness and the accepted plan digest bind these values; an invalid or
  changed policy cannot reuse prior acceptance. Production smoke execution and
  activation remain pending.
- P06 can now strictly and boundedly decode a saved `setup plan --json`
  document, rebuild the plan from current target, catalogue, configuration,
  probes and time, require the entire presentation to remain unchanged, and
  verify the separately supplied acceptance digest. Malformed or stale readable
  plans fail before transfer or filesystem mutation. A valid plan still ends in
  `COMMAND_NOT_IMPLEMENTED`; compatibility smoke and the installer transaction
  remain pending.
- P06's pinned multilingual `base` model now uses the same accepted-action
  authority as the Ubuntu archives. Exact model bytes can be copied into a
  private unactivated payload and runtime with bounded size/SHA-256 rechecks;
  unsafe names or mismatched review fail before mutation. The disposable
  Ubuntu workflow exercises the path against fresh publisher bytes. Provider
  compatibility and managed activation remain pending.
- P06 accepted Ubuntu actions can now be rebound to exact reviewed publisher
  source and passed through the owned archive/payload/runtime preparation path
  using the catalogue inventory itself. Changed action fields or mismatched
  staged bytes fail closed. This remains unactivated; raw-model staging,
  compatibility smoke and the public installer are still pending.
- P06 now records an exact Ubuntu 24.04 x86-64 reviewed catalogue for the
  pinned month-end FFmpeg/FFprobe build, whisper.cpp v1.9.2 CLI and multilingual
  `base` model. `setup plan` emits only currently needed actions with direct
  publisher URLs, pinned bytes/hashes, archive and installed-file inventories,
  licence/source disclosures, trust limits, private destination, and a
  deterministic state-bound acceptance digest. It stops new plans on 2028-08-01
  and returns typed managed-unavailable guidance elsewhere. Installation and
  compatibility preflight remain unavailable; the plan makes no legal-clearance
  claim.
- P06 published runtimes now hold shared per-version OS locks. A guarded
  transaction can atomically select an older published version for rollback and
  remove only an unselected version after obtaining its exclusive lock. Selected
  or live-held versions remain intact; a private tombstone makes interrupted
  exact-file removal retryable through metadata deletion and a lost response.
  A native child-process test proves a live hold blocks removal and abrupt
  process exit releases it. Public install, rollback and uninstall commands
  remain unavailable.
- P06 can now publish a fully rechecked prepared runtime under a canonical
  component/version identity and atomically select it with a hashed pointer while
  holding the root installation guard. Published versions retain exact manifests,
  regular-file identity, private modes and SHA-256 checks; interrupted pointer
  replacement is retryable and prior versions remain readable. This infrastructure
  primitive carries no catalogue, compatibility or plan-acceptance authority.
- P06 now has a root-wide managed installation guard backed by a private,
  single-link OS-locked file. Concurrent writers receive typed `Busy` without
  waiting or retrying; linked or incorrectly permissioned lock files fail
  closed. The guard serializes future transactions but grants no install authority.
- The opt-in disposable Ubuntu P06 qualification workflow now sends a freshly
  bounded and SHA-256-verified whisper.cpp archive through the production Rust
  owned-runtime layout check before running the separate candidate compatibility
  smoke. It still grants no catalogue, plan, activation or install authority.
- P06 can now copy a verified payload into a fresh private, unactivated
  `runtime.pending` directory with only reviewed regular-file aliases and
  selected Unix owner-executable modes. Every copy is bounded and rechecked;
  failed preparation removes only its owned runtime files. A pinned Ubuntu
  whisper.cpp archive passed this layout stage without binary execution.
- P06 `setup plan --profile` now performs a read-only configured/PATH executable
  diagnosis. Until a per-target managed artifact is qualified, its v1 result
  reports an unavailable managed path, no install actions or acceptance digest,
  and typed manual BYO steps. `setup install` remains reserved.
- P06 now composes a verified managed artifact with bounded raw tar, gzip/tar
  or XZ/tar selected-file staging under a fresh private payload directory.
  Selected files are rechecked before use; changed, linked or unexpected files
  block opening and cleanup removes only the reviewed selection. The payload
  remains unactivated and managed installation remains unavailable.
- P06 can transfer an exact reviewed publisher artifact over direct HTTPS into
  the private unactivated stage. Immutable GitHub release and Hugging Face
  model routes admit only their reviewed CDN redirect, with bounded deadlines,
  cancellation, whole-artifact size/SHA-256 verification and no resume.
  Managed installation remains unavailable.
- P06 now has a positively marked private per-user managed root and one-artifact
  staging transaction. It verifies exact reviewed bytes on import and again
  before archive use, removes its own stage after failed import, and refuses
  unmarked roots or unexpected staging entries. This is an infrastructure
  boundary; managed installation remains unavailable.
- P06 bounded archive adapters can stage an exact reviewed regular-file
  selection into an empty private directory capability. Staging uses portable
  flat names, create-new/no-follow writes and private modes, ignores archive
  links/directories/modes, and removes files it created when any later archive
  or compression check fails. This infrastructure primitive does not activate
  managed installation.
- P05 foreground disposable `ingest`, session list/status/renew/close/clean,
  explicit evidence-only or source-inclusive retain, and data-only bundle
  validation. Source and frame/audio artifacts use P03's private
  capability-scoped generations; a bounded index and held OS locks protect
  active or abandoned sessions during cleanup. Retained output reports
  process-crash-consistent publication under ADR 0013. The opt-in P05
  checkpoint runs real media through artifact publication, both export modes
  and source-preserving cleanup.
- P04 internal source snapshot and bounded FFprobe/FFmpeg media adapter with typed
  stream metadata, actual frame/audio timestamps, source identity and an opt-in
  real-media checkpoint. Project-owned synthetic fixtures include VFR, rotation,
  audio-track and malformed variants with independent provenance verification.
- Accepted ADR 0011 and scoped R1 as the managed industrial capability expansion:
  optional enrichment, source-grounded composition, explicit catalogue lifecycle,
  industrial worker growth and integrated qualification in P15-P20. R0 now has an
  explicit two-agent end-to-end release gate and MCP remains a later adapter.
- P03 native filesystem/lock feasibility experiments and a recorded OS/storage
  crash-qualification blocker. ADR 0010 now accepts ephemeral NTFS/APFS desktop
  qualification for P03 and defers strict Ubuntu/ext4 durable enablement to the
  P10/P11/P14 fault campaign.
- Began P03 implementation with typed durability requirements, qualified publication
  guarantees, non-wrapping storage generations, and an application gate that rejects
  unsupported durable requests before invoking the mutating session-store port.
- Added the first internal capability-scoped filesystem session-store adapter: it
  validates an existing owned root, serializes initialization with a stable OS lock,
  publishes an immutable checksummed generation zero, and verifies it before reuse.
  The adapter is not yet composed into a public command.
- Completed the internal P03 storage/coordination boundary in PR #42 with
  owned private-root provisioning, Unix owner/mode and Windows DACL validation,
  immutable root-wide weighted admission, shared/exclusive lifetime holds,
  generation-fenced publication, bounded integrity-chain recovery, and deterministic
  error/process-crash tests at every manifest and pointer boundary. Durable requests
  remain rejected before mutation and no session command is exposed. PR #43 also
  makes concurrent lock-contention tests wait against a bounded monotonic deadline
  instead of assuming a fixed number of scheduler yields.

- Initial Rust workspace and architectural boundaries.
- Read-only `vsift setup check` runtime diagnostic with versioned JSON output.
- Contributor, security, governance, and automation foundations.
- Detailed proposed implementation blueprint, source baseline review, threat model,
  verification matrix and work packets for desktop and server-worker execution.
- Accepted R0 architecture decisions, qualification/resource profiles, synthetic
  fixture truth, GitHub packet backlog and CI-enforced anti-drift delivery ledger.
- Published the typed v1 R0 command namespace, JSON and JSONL terminal envelopes,
  stable errors/exits, configuration precedence, schemas, and compatibility examples.
- Added domain contracts for identifiers, source time/ranges, crops, paging cursors,
  confidence/provenance metadata, and legal job terminal transitions.
- Replaced environment-dependent CLI assertions with deterministic contract,
  compatibility, boundary, and property tests. Reserved operations fail explicitly
  without claiming their later implementation.
- Routed external setup probes through a shell-free process supervisor with canonical
  executable provenance, an allowlisted environment, bounded concurrent output,
  shared deadlines, caller cancellation, descendant cleanup, and truthful reporting
  of process containment versus strict worker isolation.
