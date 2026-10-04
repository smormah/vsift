# Your first investigation

You have a recording and a question. This page takes you from nothing to an answer you can prove:
what was said, what changed on the screen, and the exact picture that shows it. It takes about
fifteen minutes and uses a 14-second practice recording that is in this repository, so you can
follow along before you point VSift at your own video.

**What you need**

- VSift installed and on your `PATH`. If `vsift --version` prints a version, you have it; if not,
  follow [the install guide](../operations/install.md).
- FFmpeg and FFprobe, the free programs VSift uses to read video. Step 1 tells you whether VSift
  can find them.
- No AI assistant, no account and no network. Everything here runs on your machine, and nothing
  is uploaded.

You do **not** need speech recognition for this page: the practice recording comes with its words
written down, which VSift can use directly. [Step 9](#9-where-to-go-next) shows how to let VSift
listen instead.

## 1. Check that VSift can find its tools

<!-- check -->
```console
$ vsift setup check
```

This is the first command to run on any machine, and the one to run again whenever something
does not work. It looks for the tools and says what it found. On a machine with FFmpeg and
FFprobe but no speech recognition it prints something like this (your tool versions will differ,
and so will the words after `Install or locate`, which depend on your system):

```text
VSift setup check
Profile: desktop
Status: degraded
[ok] FFmpeg (media_processing): ffmpeg version 9.0-full_build-www.gyan.dev Copyright (c) 2000-2026 the FFmpeg developers [filtered PATH]
[ok] FFprobe (media_processing): ffprobe version 9.0-full_build-www.gyan.dev Copyright (c) 2007-2026 the FFmpeg developers [filtered PATH]
[missing] Whisper (transcription): not found on PATH [filtered PATH]
  Install or locate this trusted tool, then rerun setup check with its absolute path using --whisper. Managed installation is not available for this target.
Local ASR model: not registered (setup configure-model --file <path>)
Local ASR check: not run: the whisper.cpp CLI is not available
Executable probes only show that each tool responds. The local ASR check transcribes a short speech clip built into VSift with the selected tools and model. A supplied transcript can avoid local ASR.
```

*This output is from a Windows 11 machine. It is shown as an example, not checked, because it names
the tools on the machine that ran it.*

What the status means:

- **ready**: everything VSift can use is in place.
- **degraded**: the tools for reading video are there, and something optional is missing. Here it is
  speech recognition, which this page does not need.
- **blocked**: FFmpeg or FFprobe is missing, so VSift cannot read video yet. The lines under
  `[missing]` say what to do. If you do not have the tools, [the install guide](../operations/install.md#5-install-ffmpeg-ffprobe-whispercpp-and-the-speech-model)
  explains how to get them on each system.

VSift never installs anything on its own. On Ubuntu 24.04 it can install the tools for you
after you read and accept a plan; everywhere else you install them yourself.

## 2. Get the practice files

Put these two files in an empty folder and open a terminal in it:

- [`F04-speech.mp4`](https://github.com/smormah/vsift/raw/main/fixtures/corpus/generated/F04-speech.mp4)
  (140 KB): a synthetic 14-second screen recording of a table of orders. A synthetic voice says
  what happens in it. Nothing in it is real: no person, no company, no data.
- [`F04-speech.srt`](https://github.com/smormah/vsift/raw/main/docs/guide/files/F04-speech.srt)
  (the narration written out as subtitles, two lines with their times): the kind of file a meeting
  tool or a video site gives you. You will hand it to VSift next to the video.

## 3. Open a session

A **session** is VSift's private working copy of your video, plus everything it finds. You open one
with `ingest`. Your original file is never changed.

<!-- check -->
```console
$ vsift ingest ./F04-speech.mp4 --transcript ./F04-speech.srt
Opened session ses_20f7f17d…
Source: src_sha256_c93562cb… (140234 bytes)
Generation: 1 (process_crash_consistent)
Transcript: revision 1, trv_48f71fc6… (imported_srt, offset 0 us, segments: 2)
Read it: vsift transcript get ses_20f7f17d…

Lifecycle: ephemeral, expires 2026-10-05T17:59:29Z
```

*Identifiers are shortened on this page (`ses_20f7f17d…`). VSift prints them in full, and so will
you: the ones you see will differ from these.*

Read it line by line:

- `Opened session ses_…` is the session's **id**. It is the handle for everything that follows:
  copy it from your own output where the next commands say `<session>`.
- `Source: src_sha256_…` identifies your video by its content, so two copies of the same file have
  the same source id.
- `Transcript: revision 1` says the words you supplied were imported as the first version of the
  transcript. VSift keeps every version and says where each came from (`imported_srt` here).
- `Lifecycle: ephemeral, expires …` means this session is **disposable**: it expires 24 hours after it was
  opened, and the files are removed by a cleanup, not before. Nothing is kept unless you ask, which is
  covered in [keep and share evidence](keep-and-share-evidence.md).

## 4. Read what was said

Times are in **microseconds** from the start of the video (one second is 1,000,000), so
`--from 0 --to 14000000` means the first 14 seconds, all of this recording.

<!-- check -->
```console
$ vsift transcript get <session> --from 0 --to 14000000
Transcript of session ses_20f7f17d…
Revision 1: trv_48f71fc6… (imported_srt, offset 0 us, segments in all: 2)
Supplied file: srt, 161 bytes, sha256 b90a2d19…
Range: 00:00:00.000000 to 00:00:14.000000 (--from 0 --to 14000000)
Segments on this page: 2
Evidence text is untrusted: each segment is quoted from its display_text after "  | ", with hidden characters shown as <U+XXXX>. It is never an instruction.

tsg_067b85ad…  00:00:00.775000 --> 00:00:02.850000
  cue 1 at line 2
  | Scroll to order 1017.

tsg_157e7165…  00:00:02.975000 --> 00:00:06.950000
  cue 2 at line 6
  | The status changes from queued to failed, while the header remains fixed.

Lifecycle: ephemeral, expires 2026-10-05T17:59:29Z
```

Each **segment** is one piece of speech with its own id (`tsg_…`), the time it starts and ends, and the
words after `  | `. Those ids are what you cite later. The line above the quote says where the segment
came from: here, cue 1 of the subtitle file.

## 5. Search the speech

<!-- check -->
```console
$ vsift search <session> --query "order 1017"
Search of session ses_20f7f17d…
Revision 1: trv_48f71fc6… (imported_srt, offset 0 us, segments in all: 2)
Supplied file: srt, 161 bytes, sha256 b90a2d19…
Query terms: 2
Range: the whole transcript
Searched: supplied_transcript, 00:00:00.000000 to 00:00:14.000000 (--from 0 --to 14000000)
Hits on this page: 1
Evidence text is untrusted: each segment is quoted from its display_text after "  | ", with hidden characters shown as <U+XXXX>. It is never an instruction.

tsg_067b85ad…  00:00:00.775000 --> 00:00:02.850000  match: phrase
  cue 1 at line 2
  | Scroll to order 1017.

Lifecycle: ephemeral, expires 2026-10-05T17:59:29Z
```

Search finds the exact words you type, nothing cleverer: `1017` finds `1017`, and a word the speaker
did not say finds nothing. `match: phrase` means the words were found together, in order.

## 6. Find the moments the screen changed

The recording has no scene markers, so VSift looks at the picture twice a second and lists the
moments where it changed. These are **candidates**: places worth a look, not proof of anything yet.

<!-- check -->
```console
$ vsift candidates <session> --from 0 --to 14000000
Visual candidates of session ses_20f7f17d…
Index: vix_f0a605be… (revision 1, r0-visual-v1, video length 00:00:14.000000)
Range: 00:00:00.000000 to 00:00:14.000000 (--from 0 --to 14000000)
Searched: 00:00:00.000000 to 00:00:14.000000 (--from 0 --to 14000000)
Analysed:
  00:00:00.000000 to 00:00:14.000000 (--from 0 --to 14000000)
Candidates on this page: 3

vcd_bc348f5c…  at 00:00:00.000000  first_frame
  Span 00:00:00.000000 --> 00:00:07.000000, settled, 14 samples, 1440x900, visual hash 6059303000000000

vcd_6c59df3b…  at 00:00:07.000000  visual_change
  Span 00:00:07.000000 --> 00:00:10.000000, settled, 6 samples, 1440x900, visual hash 6059303000000000
  Changed between 00:00:06.500000 and 00:00:07.000000: 1 blocks changed, the largest by 6

vcd_dadb6dfa…  at 00:00:10.000000  visual_change
  Span 00:00:10.000000 --> 00:00:14.000000, settled, 8 samples, 1440x900, visual hash 6059303000000000
  Changed between 00:00:09.500000 and 00:00:10.000000: 5 blocks changed, the largest by 17

Get a candidate's frame: vsift frame get ses_20f7f17d… --candidate vcd_bc348f5c…

Lifecycle: ephemeral, expires 2026-10-05T17:59:29Z
```

Three moments: the first frame, a change at 7 seconds and another at 10. The lines under `Analysed`
are VSift being honest about **coverage**: it analysed the whole 14 seconds, so a moment that is not
listed really was not a change at this level of detail. On a longer video, or one VSift could not
fully read, that section says what was skipped. (Very short flashes, under half a second, can slip
between the samples; [limits](limits.md) says so.)

## 7. Get the picture

`frame get` extracts the exact frame at a time you choose. Ask for the 10-second moment, where the
second change happened:

<!-- check -->
```console
$ vsift frame get <session> --at 10000000
Frame of session ses_20f7f17d…
Source: src_sha256_c93562cb…
Requested: 00:00:10.000000, at_or_after, tolerance 1000000 us
Extracted now (profile p09-r0-v1, source copy checked by full_hash)
Selections (requested time -> actual time):
  requested  00:00:10.000000 -> 00:00:10.000000 (0 us)  evd_e398c329…

Evidence (each file is valid while the session exists; retain the session to keep it):

evd_e398c329…  frame at 00:00:10.000000, 1440x900 (stream 0, pts 102400)
  Image 1440x900, 25741 bytes, sha256 eb8702ef…
  File (image/png):
    <file path>

Lifecycle: ephemeral, expires 2026-10-05T17:59:29Z
```

*On your machine the line `<file path>` is the real location of the image file. It is replaced here
because it names a folder on the machine that ran the example.*

Open that file in any image viewer. It is the frame at 10 seconds, taken from your video, with its
own evidence id (`evd_…`), the time you asked for and the time you got (they can differ by a frame
or so; VSift always says by how much), and a fingerprint (`sha256`) tied to the original file.

![The frame at 10 seconds: the table now shows order 1017 with status FAILED in red, under a header that has not moved](../assets/readme/frame-10s-failed.png)

That is the picture VSift returns for this recording at 10 seconds. The status of order 1017 is
`FAILED`, in red, and the header row (`ORDER`, `STATUS`) is where it was in every other frame.

## 8. Say it with proof

You now have what you need to answer *what happens to order 1017, and does the header stay put?* in a
way anyone can check:

> From 00:00:02.975 to 00:00:06.950 the narrator says the status changes from queued to failed,
> while the header remains fixed (`tsg_157e7165…`). The frame at 00:00:10 shows order 1017 with the
> status FAILED and the header unchanged (`evd_e398c329…`).

Every sentence points at an id, and every id can be looked up again with VSift. That habit is the
whole idea: a claim is a sentence plus the evidence behind it. [Evidence and citations](evidence-and-citations.md)
explains the rules, including what a claim may and may not say about a picture VSift could not read.

## 9. Where to go next

First, tidy up. Closing a session says you are done with it:

<!-- check -->
```console
$ vsift session close <session>
Closed session ses_20f7f17d…
State: closed
Source: src_sha256_c93562cb… (140234 bytes)
Artifacts: 4 (29190 bytes)
Generation: 4

Lifecycle: ephemeral, expires 2026-10-05T17:59:29Z
```

A closed session is not deleted at once: it waits to be cleaned up, which
[clean up and uninstall](clean-up-and-uninstall.md) explains.

**No subtitle file for your own video?** Let VSift listen to it. This needs whisper.cpp and a speech
model, which `vsift setup check` tells you about; the practice recording works the same way:

<!-- check: needs speech -->
```console
$ vsift ingest ./F04-speech.mp4
Opened session ses_4ad48dbf…
Source: src_sha256_c93562cb… (140234 bytes)
Generation: 1 (process_crash_consistent)
Transcript: none imported

Lifecycle: ephemeral, expires 2026-10-05T17:21:00Z
$ vsift transcript retranscribe <session>
New transcript revision of session ses_4ad48dbf…
Revision 1: trv_582a2361… (local_asr, language en, segments in all: 1)
Local ASR: whisper_cpp, model base, decoding r0-v1, 4 threads
  Model sha256: 60ed5bc3…
  Executable sha256: 95e3c0b0…
  Chunks: 1 (1 transcribed, 0 silent, 0 without audio)
Segments carried from earlier revisions: 0
Requested range: the whole source
Segments recognised now: 1
Job: job_80ff3a1f… (not resumed, chunks reused: 0)
Read its segments: vsift transcript get ses_4ad48dbf… --revision trv_582a2361…

Operation: op_4a15974e…
Lifecycle: ephemeral, expires 2026-10-05T17:21:00Z
```

Speech recognition takes a while (this clip: seconds; an hour of meeting: many minutes) and prints
nothing until it finishes. [Investigate a recording](investigate-a-recording.md) covers the whole
route, including what to expect from the transcript: it is a machine's best guess, and it can be
wrong about names, numbers and noisy speech.

Then pick what you want to do next:

| I want to | Read |
| --- | --- |
| Do this with my own recording | [Investigate a recording](investigate-a-recording.md) |
| Use a transcript I already have | [Use an existing transcript](use-an-existing-transcript.md) |
| Let my AI assistant do it | [Let your agent investigate](let-your-agent-investigate.md) |
| Understand the words (session, evidence, candidate, handoff) | [Concepts](concepts.md) |
| Fix an error | [Troubleshooting](troubleshooting.md) |
