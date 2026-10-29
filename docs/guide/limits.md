# Limits: what VSift cannot do, in plain words

This page lists the limits a user is likely to meet, so you can judge what to trust. Each one links to its
entry in the [known-limits register](../planning/known-limits.md), which is the full record and the
authority. This page describes VSift 0.2.3 as it is, not as it will be.

The short version: **VSift is tested on synthetic videos and a synthetic voice only.** Nobody has yet run it on
real meetings, accents or noisy rooms, so every accuracy figure below says what was measured, on what.

## What VSift can read

- **Containers and codecs.** MP4 and MOV files and Matroska and WebM files; video in H.264, H.265, VP8, VP9, AV1
  or MPEG-4; audio in AAC, MP3, Opus, Vorbis, FLAC or uncompressed PCM. Anything else (AVI, FLV, WMV) is
  refused, not guessed at: re-save it first.
- **Size.** Up to 4 hours and 20 GiB, 32 streams and 16 megapixels per picture. A bigger file is refused.
- **Ten minutes to open a video.** Opening a video copies it into VSift's own folder, and the copy is stopped after
  ten minutes, however far it got. A disk in your computer is nearly always fast enough. A big video on
  a network drive, a memory stick, an SD card or a folder that downloads from the cloud when first read may not be:
  copy it to a disk in your computer first and open that copy
  ([what the message looks like](troubleshooting.md#invalid_source-the-video-or-the-transcript-cannot-be-used)).
  Nothing raises the ten minutes, and trying again starts the copy from the beginning.
- **Per command.** A page of results is 20 by default and 100 at most (`--limit`); a burst of frames is 12 by
  default and 100 at most (`--max-frames`); an audio clip is 30 seconds at most and is a WAV at 16 kHz mono;
  a session keeps at most 512 stored items; one result is at most 1 MiB. Bigger asks fail with
  `RESOURCE_LIMIT`: ask for less.
- **A supplied transcript.** SRT or WebVTT in UTF-8, at most 20,000 cues, 4,096 bytes per cue and 8 MiB.
- **Clock times (from 0.3.0).** A result's real-world time is only as good as its source: the time you gave
  with `--recorded-at`, the recorder's own note, or the file's creation time, which is often the time it was
  converted. A recording that was paused or cut in the middle has clock times that are wrong after the cut. The
  file's own tags are read only after `vsift setup check` (or any command that looks at the picture or sound)
  has checked the media tools once, and a kept bundle does not keep the clock times
  ([how it works](evidence-and-citations.md#when-it-happened-by-the-clock)).

## The words: speech recognition

- **Tested only on a synthetic voice** ([L-022](../planning/known-limits.md#l-022)). The recogniser is
  whisper.cpp with its `base` model. On clean synthetic speech it got about 3% of the words wrong. No human
  voice, regional accent, overlapping speakers or long recording has been measured.
- **Noise makes it much worse** ([L-020](../planning/known-limits.md#l-020)). On the one noisy test clip it got
  more than half the words wrong. A transcript of a noisy recording can be substantially wrong.
- **Names and numbers are the usual casualties** ([L-021](../planning/known-limits.md#l-021)). In the tests,
  "queued", "invoice 4407" and "E-409" were misheard. Search will not bridge the difference: `407` never finds
  `4407`. Check any number you rely on against the audio clip.
- **The confidence figure is the recogniser's own** and is not a probability. VSift marks it as uncalibrated.
- **Speech recognition prints nothing until it is finished** ([L-017](../planning/known-limits.md#l-017)), and a long
  recording takes many minutes. Press Ctrl-C to stop; running it again continues.
- **A stretch can be left without a transcript (0.2.1 and later; 0.2.0 fails the whole run instead,
  [#353](https://github.com/smormah/vsift/issues/353)).** When the recogniser's answer for a 30-second stretch cannot be
  placed in that stretch of audio, VSift leaves the stretch out, keeps the rest and says so: the result is
  `partial` and lists the stretch as not transcribed by that run, and it stays listed through every later run, however
  far from it, until a run reads it. A `transcript get` and a `search` over a range that includes the stretch are `partial`
  too and list it, so an empty page there is never a silence. A recording with long pauses can have a few. When you were re-transcribing a recording
  that already had a transcript, the words it had that reach into such a stretch are kept whole (unless the new words
  overlap them); a word said there that no transcript has cannot be found. Transcribing just that stretch again may cover it (the audio is
  then cut at other points), and a transcript file you supply can. If most stretches fail, the command fails
  instead ([troubleshooting](troubleshooting.md#missing_capability-a-tool-or-model-is-missing)). A worker `job run`
  or `job batch` request reports it too since `0.2.2`: its retranscribe step is `partial` and lists the same stretches
  in its `coverage` (in `0.2.1` the step was `complete` either way).
- **A transcript segment can start seconds before its first word ([L-024](../planning/known-limits.md#l-024)).** The
  recogniser often starts the first segment of a 30-second chunk at the chunk's first instant, whatever the audio holds, so
  after a pause, noise or a silent stretch a cited start can come before the words: on real recordings by 1 to 4 seconds
  where it could be compared, and by about 15 seconds after a long silent lead. Such a segment's
  `alignment.provider_start_us` is 0. The words are inside the segment; to place them, listen to the segment's clip
  (`vsift audio`) or read the frames around it. Such a segment also counts as transcribed over the time it claims.
  `0.2.3` says so and changes nothing in the times; word-level times are to be evaluated in R1.
- **A chunk heard as another language can lose speech, or show it in the wrong language, on a recording in two languages
  and on one in one language spoken with an accent ([L-188](../planning/known-limits.md#l-188)).** The recogniser decides the
  language of each 30-second chunk on its own. On an English briefing with answers in Russian, one chunk heard as Russian
  wrote nothing for about 18 seconds of English, and another wrote a 25-second Russian segment over an English question. On
  a talk in English with a Turkish accent, one chunk heard as Turkish wrote a single 30-second segment, mostly one Turkish
  word repeated, so about 20 seconds of English has no English text (until `0.3.0` about 25: it also hid 15 words the chunk
  before it had heard, which are now kept beside it). Where
  the chunks are cut decides which stretches are affected, and `transcript get` and `search` call them transcribed: VSift
  does not yet flag such a chunk (the revision's `language` is empty when its chunks disagree). If a recording mixes
  languages or is spoken with an accent, check stretches where the text is in another language or repeats one word, listen
  there (`vsift audio`) before relying on "not found", or supply a transcript (`ingest --transcript`). A fix is planned for
  1.0; it is a plan, not a promise.
- **Where two chunks meet, a word or two can still be missing or repeated (0.2.1 and later, [L-155](../planning/known-limits.md#l-155) and [L-156](../planning/known-limits.md#l-156)).** The
  recogniser works on 30-second chunks that overlap by five seconds, and VSift joins them. Before this version it could
  drop the end of a sentence at a join, with no sign (found on two real recordings; it is in `0.2.0` and `0.2.1-rc.1`).
  In `0.2.1`, VSift keeps a piece of speech again whenever leaving it out would leave more than half a
  second of what the chunks heard in no segment, so up to about one word at the end of a copy can still be missing (with no
  sign), and where the two chunks wrote the overlap differently the same words can appear twice. Since `0.2.3` VSift looks
  for a repeat in every pair of overlapping pieces of two neighbouring chunks, and lets one word differ among four or more
  (earlier versions looked at one pair per join and wanted every word equal). It removes a repeated piece only when what else
  is kept covers its time, so a piece that runs on past the one it repeats stays, and its words appear twice; earlier versions
  dropped it, and the stretch it covered then had no segment. VSift judges by time, not by words, so it can still keep a copy
  twice where nothing was lost. **More can be missing where a chunk loops** ([L-201](../planning/known-limits.md#l-201)):
  since `0.2.3` a piece that repeats the piece just before it no longer hides the next chunk's words, and since `0.3.0` nor
  does a piece that loops on its own words (one to four words said eight times or more in a row, most of the piece; this kept
  15 words at one join of a real talk). A loop of another kind still can.
  **The cost of the fix is mostly repeats, not holes; both are in the transcript.** The cause was worked out from the
  times in a report, not from the recogniser's own output of the recordings, and the fix has not been tried on them.
- **Words the recogniser wrote that VSift could not place are listed, not found (0.2.1 and later, [L-158](../planning/known-limits.md#l-158) and [L-159](../planning/known-limits.md#l-159)).** When
  the recogniser writes words whose times do not fit its audio (an empty or backwards range, or one that runs past the
  chunk), VSift drops them and counts them (`provider_segments_rejected`). If such a segment has a word in it, the revision
  lists the stretch from its start to the next segment's start, or to the end of the chunk's audio when nothing follows it
  there (`local_asr.unplaced_ranges`), as where the words may lie, less what kept words cover, if that is more than half a second.
  Since `0.2.3` that includes a segment with an empty range, one placed after its audio ends and the last of a chunk,
  which earlier releases only counted, so a loss there was not listed; a pause where the recogniser wrote words can now be
  listed too. Only a rejected segment with no letter or digit is just counted.
  The run stays `complete` (it read everything), and `search` lists the stretch as not transcribed: a word said there cannot
  be found. A `transcript get` over the stretch answers `partial` and lists it too: it shows no words there, which is not a silence. Each stretch is an upper bound: the words took only part of it, so `search` can call more time untranscribed than
  was lost. Retranscribing just the stretch may give the same stretch; a transcript file you supply (`ingest --transcript`)
  is the sure remedy. The list is stored in the session, and **`0.2.0` and earlier, and `0.2.1-rc.1`, read a revision that holds it as
  damaged** (`INTEGRITY_FAILURE`). They read the newest revision first: if that one holds it they can read nothing of the
  session; if only an older one does, they read the newest normally and refuse the older ones (by id, in `handoff check --session`
  and in `bundle validate`). Use the newer version, or discard the session.
- **A stretch can be listed as not transcribed that the first run did read** ([L-033](../planning/known-limits.md#l-033)).
  When a re-transcription of a range follows a first run with few or no words (a recording with one spoken stretch, say, or
  one in which the first run heard only silence), the new transcript carries nothing from the first run and no longer
  remembers that it had read the rest of the recording, so a `transcript get` or a `search` of the whole recording lists that
  rest as not transcribed, and the silence the first run found is not listed as silence any more. The stretch is listed
  wrongly and is never reported as silence: the safe error. Transcribing the stretch again, or the whole recording, clears it.

## The pictures: screen changes and frames

- **Small changes are often not listed** ([L-028](../planning/known-limits.md#l-028)). The size of a change VSift
  counts was set on synthetic screens. On a real, heavily compressed screen recording it listed no false change, and
  it saw nearly every new line of text, caption or page. But it saw only about half of the changed words, and few
  cursor moves or single changed characters. Look at the frames themselves when a small edit matters.
- **It looks twice a second** ([L-029](../planning/known-limits.md#l-029)). A change shorter than half a second
  can fall between looks and is not reported, and the coverage lines cannot say so. A burst of frames around a
  moment finds it if you suspect one.
- **No real motion has been measured** ([L-030](../planning/known-limits.md#l-030)). The practice recordings
  draw no scrolling, cursor movement or loading animation, so how well VSift sees those is unknown.
- **VSift reads text in a picture only when you ask** (`vsift ocr`, new in 0.3.0, with the optional OCR runner). It
  then reads Latin-script text off each candidate's frame and one frame every 10 seconds, and `ocr search` finds it with
  the frame, time and place on the screen, even in a recording with no speech. It reads only those frames, and no other script
  ([L-202](../planning/known-limits.md#l-202)); a text that appears and goes between them is not found. What it read
  is the engine's reading: check the frame before you rely on a value. Without `ocr`, VSift gives you the frame and
  you (or an assistant that can open images) read it. A crop at native size helps with small text. **Small numbers can be misread** ([L-018](../planning/known-limits.md#l-018)):
  in a compressed recording, a frame taken while the screen moves can turn a digit into a different one, and
  enlarging the picture does not bring it back. Read numbers from a frame where the screen has settled, and check them
  against a neighbouring frame (`frame neighbours`) or the transcript.
- **Candidates are places to look, not findings.**

## The files `vsift <video>` writes (new in 0.3.0)

- **Settled screens, and frames for at most 180 of them** ([L-234](../planning/known-limits.md#l-234)).
  `<name>-vsift/screen-changes.txt` lists the settled screens: the first frame, each screen once motion settles, and a
  change at least 5 s after the one listed before it, each with its reason; its header says how many changes were
  found in all. With more than 180 listed, the frames are spread evenly over them and the others are listed with
  `no frame (evidence room)`: a 19-minute talk lists 183 settled screens of 589 changes, so 3 have no frame. The cost: a screen shown for less than 5 s after a listed one, a dialog that flashes up 3 s after a click say, is
  not listed unless motion settles on it, and the run closes its session, so the changes it left out are kept
  nowhere. To see every change, keep a session of the video: `vsift ingest <video>`, then
  `vsift candidates <session> --from 0 --to <microseconds>`, with the session id `ingest` gives; the list's header
  writes the exact `--to` in. The changes counted are the ones `candidates` would give you, so a change its
  per-window limit leaves out is not counted either.
- **Screen changes in the first four hours only** ([L-235](../planning/known-limits.md#l-235)). The transcript covers
  the whole video; the list and the finish screen say where the screen changes stop.
- **The ids in the files name a closed session** ([L-236](../planning/known-limits.md#l-236)). They say which run made
  the files, but the run closes its session, so no command can look them up afterwards. The SHA-256 of the video and
  of every frame can be checked by anyone, any time, with any hashing tool.
- **The files take the folder's ordinary permissions**, because you write them to share. Beside the video they are as
  exposed as the video itself; `--output <folder>` writes them elsewhere.

## Where and how it was tested

- **Windows 11, macOS 15 on Apple silicon, Ubuntu 24.04 on x64.** VSift is built for these. The published 0.1.0
  was installed and run on hosted test machines of each, which are not clean machines, and nobody has run it on a Mac
  ([L-035](../planning/known-limits.md#l-035), the [support matrix](../planning/support-and-resource-profiles.md)).
  Another Linux may work; nobody has tried. Windows on Arm, Intel Macs and Windows 10 are refused with a clear message.
- **Managed tool installation is Ubuntu 24.04 only** ([L-037](../planning/known-limits.md#l-037)). Everywhere else you
  install FFmpeg, FFprobe and whisper.cpp yourself. On macOS the journeys were tried with Homebrew's builds, which
  VSift does not review ([L-114](../planning/known-limits.md#l-114)).
- **The programs are not signed** ([L-098](../planning/known-limits.md#l-098)). Windows SmartScreen or Smart App
  Control and macOS Gatekeeper may warn about or stop a downloaded file; nobody has yet seen them react to a
  VSift file. [Install guide, section 4](../operations/install.md#4-windows-and-macos-smartscreen-gatekeeper-and-what-to-check-instead).
- **Going back to an older VSift does not work reliably:** it may refuse a session a newer one wrote, so finish or
  close your sessions first ([L-044](../planning/known-limits.md#l-044)).

## Safety and your data

- **FFmpeg, FFprobe and whisper.cpp are not sandboxed on a desktop** ([L-004](../planning/known-limits.md#l-004)).
  VSift starts them without a shell and limits their time and output (and, for FFmpeg, each memory allocation), but
  a flaw in one of them, triggered by a crafted video, would run with your rights. Open videos you would be willing to open in any player.
- **Nothing is uploaded by VSift.** An assistant that reads its output may send what it reads to its own service;
  that is up to the assistant and how you set it up.
- **Text in a video can try to give orders to an assistant** ([L-007](../planning/known-limits.md#l-007)). VSift
  labels it untrusted and an assistant using the skill refuses to act on it, but no tool can make every model immune.
- **One local copy.** Sessions and bundles live on your disk. If the disk or the machine is lost, so is the evidence
  ([L-057](../planning/known-limits.md#l-057)); a bundle copied elsewhere survives. Cleanup deletes files the ordinary
  way, not securely.
- **Durability across a power cut** is shown only on Ubuntu 24.04 with a local ext4 disk
  ([L-008](../planning/known-limits.md#l-008)), and only as far as the disk honours its flushes
  ([L-056](../planning/known-limits.md#l-056)). Elsewhere a crash of VSift itself is survived; a power cut during a
  write may not be.

## AI assistants

- **Two assistants have been tried**, Claude Code and Codex, each with two models, on synthetic videos, with the skill.
  Other assistants and models are untested; two models were tried and fell short of the bar
  ([L-082](../planning/known-limits.md#l-082), [L-084](../planning/known-limits.md#l-084)).
  Codex's own Windows sandbox cannot run VSift ([L-076](../planning/known-limits.md#l-076)).
  [The skill guide](../agents/skill.md#models-and-clients-trialled) has the figures.
- **A report can still be wrong.** The trial reports were right most of the time, not always; the cited evidence is
  what lets you check. One known slip: stating the content of a blurred region as seen in the pixels when only the
  narration said it ([L-095](../planning/known-limits.md#l-095)).
- **The words in a transcript keep hidden characters in `text`.** Quote `display_text`, which shows them as
  `<U+XXXX>` ([L-083](../planning/known-limits.md#l-083)).

## Behaviour that may change

- **The readable text VSift prints without `--json` may change in any release.** Programs should read `--json`,
  whose shape is versioned and only ever grows ([CLI contract](../contracts/cli-v1.md)).
- **The library behind the command line is not a published interface yet**
  ([L-043](../planning/known-limits.md#l-043)).
